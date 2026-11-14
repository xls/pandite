//! Nikon NEF decoder, written from the clean-room specification
//! `specs/raw/nef.md` and the standards it cites (TIFF 6.0, ITU-T T.81,
//! Exif 2.32).
//!
//! Variants V1 to V8 are developed from sensor data (R-NEF-1). Every other
//! file (X1 to X7), and every file that fails a check, shows its embedded
//! preview (R-NEF-2, R-NEF-56). The TIFF structure is read with the Samsung
//! decoder's reader, which handles both byte orders.
//!
//! The levels, demosaicing, color matrix, crop, orientation and output
//! encoding are the Sony decoder's ([`crate::raw::arw`]): the whole raster
//! becomes a plane whose CFA phase is that of the stored raster, and the
//! plan is handed to the shared renderer, which demosaics the plane and
//! crops afterwards (R-NEF-48). The developed image is linear sRGB
//! (R-NEF-44) inside the pipeline; the provider delivers pandite's default
//! display rendering of it, like the Sony decoder. [`Developed::origin`]
//! reports whether the sensor data or a preview was delivered (R-NEF-57).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-NEF-68); a partly decoded raw image is dropped with
//! the error and never delivered (R-NEF-69).

#![forbid(unsafe_code)]

mod color;
mod huffman;
mod meta;
mod preview;
mod sensor;
mod tiff;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Class, CropSource, Relative, Variant, WbSource};

use crate::raw::arw::{self, Output, Rect, Rendering};
use crate::raw::tone::Tone;
use crate::util::check_size;
use meta::tag;
use pandite_core::{
    CancelToken, DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result,
};
use std::fmt::Display;

/// Results inside the NEF decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type NefResult<T> = Result<T>;

/// A decoding failure (R-NEF-68: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: nef: {msg}"))
}

/// Which image was delivered (R-NEF-57).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-NEF-44).
    Sensor(Variant),
    /// An embedded JPEG preview: sRGB-encoded values, not linear.
    Preview,
    /// The IFD0 RGB thumbnail: sRGB-encoded values, not linear.
    Thumbnail,
}

/// Facts about the raw data of a developed file. Read by the tests.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Info {
    /// Stored raster W x H.
    pub raster: (usize, usize),
    /// BitsPerSample.
    pub bits: u32,
    /// The final image area (R-NEF-40).
    pub crop: Rect,
    /// The rule that gave the crop.
    pub crop_source: CropSource,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// White level (R-NEF-36).
    pub white: u32,
    /// Black levels of CFA positions 0 to 3 (R-NEF-37).
    pub black: [f64; 4],
    /// WB multipliers R, G, B (R-NEF-38).
    pub wb: [f64; 3],
    /// Source of the WB multipliers.
    pub wb_source: WbSource,
    /// True when the color matrix came from the model table.
    pub matrix_from_table: bool,
    /// The code table of V6 to V8.
    pub table: Option<&'static str>,
    /// Bits used and bits held by the V6 to V8 stream.
    pub stream_bits: Option<(u64, u64)>,
}

/// A delivered image and where it came from (R-NEF-57).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Developed {
    /// The image, upright.
    pub image: ImageBuf,
    /// Sensor data or preview.
    pub origin: Origin,
    /// The classification of the file.
    pub class: Option<Class>,
    /// Raw facts when the sensor data were developed.
    pub info: Option<Info>,
}

/// This format's entry in the developer table of [`crate::raw`].
pub(crate) const DEVELOPER: crate::raw::Developer = crate::raw::Developer {
    name: "Nikon NEF",
    detect: is_nef,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as NEF: the `.nef`
/// extension, or a TIFF (R-NEF-4) whose IFD0 Make starts with "NIKON"
/// (M1: "NIKON CORPORATION") and that has a CFA raw IFD (R-NEF-15).
pub(crate) fn is_nef(data: &[u8], extension: Option<&str>) -> bool {
    if extension == Some("nef") {
        return true;
    }
    let Ok(c) = meta::open(data) else {
        return false;
    };
    let nikon = c
        .ifd0()
        .and_then(|d| d.ascii(0x010F).ok().flatten())
        .is_some_and(|m| m.starts_with(b"NIKON"));
    nikon && c.raw.is_some()
}

/// True when the file carries an embedded JPEG preview (R-NEF-52): a
/// reduced decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(c) if !preview::collect(&c).is_empty())
}

/// The stored size of the largest JPEG candidate (R-NEF-40 rule 2).
fn largest(candidates: &[arw::preview::Candidate]) -> Option<(u32, u32)> {
    candidates.first().map(|p| (p.width, p.height))
}

/// The output size of a plan: the crop, turned for orientations 5 to 8
/// (R-NEF-51).
fn output_size(plan: &meta::Plan) -> (usize, usize) {
    arw::render::output_size(plan.crop, plan.orientation)
}

/// Size and orientation of the image [`decode`] delivers at full size. An
/// error for data that are not a TIFF (R-NEF-4) or without any image.
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let c = meta::open(data)?;
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    let candidates = preview::collect(&c);
    if let Ok(Class::Sensor(v)) = meta::classify(&c) {
        if let Ok(plan) = meta::plan(&c, v, largest(&candidates)) {
            let (w, h) = output_size(&plan);
            return Ok(Some((w as u32, h as u32, orientation)));
        }
    }
    if let Some(p) = candidates.first() {
        let (w, h) = p.displayed(o);
        return Ok(Some((w, h, orientation)));
    }
    let (_, w, h) = preview::thumbnail(&c).ok_or_else(|| fail("no decodable image"))?;
    let (w, h) = preview::thumbnail_displayed(w, h, o);
    Ok(Some((w, h, orientation)))
}

/// The storage of the sensor data by class, for the facts panel.
fn compression_name(class: Class) -> Option<&'static str> {
    Some(match class {
        Class::Sensor(Variant::V1) => "Uncompressed",
        Class::Sensor(Variant::V2) => "Uncompressed, 12-bit packed",
        Class::Sensor(Variant::V3) => "Uncompressed, 12-bit packed (D100)",
        Class::Sensor(Variant::V4) => "Uncompressed, 12-bit packed",
        Class::Sensor(Variant::V5) => "Uncompressed, 14-bit packed",
        Class::Sensor(Variant::V6) => "Nikon lossless compressed",
        Class::Sensor(Variant::V7) => "Nikon lossy compressed (type 1)",
        Class::Sensor(Variant::V8) => "Nikon lossy compressed (type 2)",
        Class::OutOfScope(Relative::X1) => "Nikon High Efficiency",
        Class::OutOfScope(Relative::X2) => "Nikon small raw",
        Class::OutOfScope(Relative::X3) => "Nikon lossy compressed",
        Class::OutOfScope(_) => return None,
    })
}

/// Header facts of a NEF file: the storage of the sensor data (from the
/// classification, R-NEF-3), the CFA layout and sample depth of the raw
/// IFD, and the camera settings of IFD0 and its Exif IFD. Reads tags only;
/// no sensor data. `None` when the data are not a TIFF.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok()?;
    let mut f = ImageFacts::named("Nikon NEF");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    let Some(raw) = &c.raw else {
        return Some(f);
    };
    f.compression = meta::classify(&c)
        .ok()
        .and_then(compression_name)
        .map(str::to_string);
    f.bits_per_channel = raw
        .int(tag::BITS_PER_SAMPLE)
        .ok()
        .flatten()
        .and_then(|b| u8::try_from(b).ok())
        .filter(|&b| b > 0);
    // The CFA layout when R-NEF-18 accepts it: a 2 x 2 repeat and a Bayer
    // pattern (0, 1, 1, 2 when the tag is absent).
    let dim_ok = !raw.has(tag::CFA_REPEAT_PATTERN_DIM)
        || matches!(raw.ints(tag::CFA_REPEAT_PATTERN_DIM), Ok(Some(d)) if d == [2, 2]);
    let pattern: Option<Vec<u32>> = if raw.has(tag::CFA_PATTERN) {
        match raw.entry(tag::CFA_PATTERN).map(|e| e.typ) {
            Some(7) => raw
                .bytes(tag::CFA_PATTERN)
                .ok()
                .flatten()
                .map(|b| b.iter().map(|&v| u32::from(v)).collect()),
            _ => raw.ints(tag::CFA_PATTERN).ok().flatten(),
        }
    } else {
        Some(vec![0, 1, 1, 2])
    };
    let bayer = pattern.filter(|p| {
        let count = |c: u32| p.iter().filter(|&&x| x == c).count();
        p.len() == 4 && count(0) == 1 && count(1) == 2 && count(2) == 1
    });
    if let (true, Some(p)) = (dim_ok, bayer) {
        let names = ["R", "G", "B"];
        let p: String = p.iter().map(|&c| names[c as usize]).collect();
        f.channels = Some(format!("Bayer {p}"));
        f.channel_count = Some(1);
    }
    Some(f)
}

/// Decode a NEF file to an upright image with the default display
/// rendering.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-NEF-57).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// The plan of the shared renderer for the plane of the whole raster
/// (R-NEF-39, R-NEF-43, R-NEF-47 to R-NEF-49, R-NEF-51).
fn carrier(plan: &meta::Plan) -> arw::meta::Plan {
    // The plane holds s * 2^(14 - bps) (see `sensor`): black levels are the
    // tag values on the 14-bit scale and the white level is scaled alike,
    // so v = clip((s - b) / (w - b) * m, 0, 1) is unchanged (R-NEF-39).
    let k = sensor::shift(plan);
    arw::meta::Plan {
        // The renderer's CFA path (any variant but its YCbCr one).
        variant: arw::Variant::V1,
        width: plan.width,
        height: plan.height,
        // R-NEF-47: the whole raster is the valid area of the demosaicing.
        valid: (plan.width, plan.height),
        // R-NEF-40, R-NEF-48, R-NEF-49: crop after demosaicing and color.
        crop: plan.crop,
        orientation: plan.orientation,
        // R-NEF-21: the pattern origin at stored pixel (0, 0).
        cfa: plan.cfa,
        white: plan.white << k,
        black: plan.black14,
        wb: plan.wb,
        wb_source: if plan.wb_source.as_shot() {
            arw::WbSource::AsShot
        } else if plan.wb_source == WbSource::Table {
            arw::WbSource::Table
        } else {
            arw::WbSource::Generic
        },
        // R-NEF-43: the normalized matrix, output clipped to 0 to 1.
        matrix: plan.matrix,
        matrix_from_table: plan.matrix_from_table,
        curve: [0; 4],
        ycc: arw::meta::Ycc {
            luma: [0.299, 0.587, 0.114],
            zero: [0.0; 2],
            factors: [1.0; 3],
            black: 0.0,
            subsampling: None,
        },
        layout: arw::meta::Layout::Strips {
            offsets: Vec::new(),
            rows_per_strip: 0,
            row_bytes: 0,
        },
        model: plan.model.clone(),
    }
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-NEF-57). The steps are those of the
/// specification's Decoding procedure.
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    options.cancel.check()?;
    // Steps 1 and 2: size, header, IFD chain, Exif IFD and maker note.
    // R-NEF-4: another start is "not a NEF file", without a preview.
    let c = meta::open(data)?;
    let o = c.orientation();
    // Step 3: collect the preview candidates first, so every later failure
    // can fall back (R-NEF-52 to R-NEF-55).
    let candidates = preview::collect(&c);
    // Steps 4 and 5: raw IFD, NEFCompression and classification (R-NEF-3).
    // X1 to X7 use the preview path and their sensor data are never read
    // (R-NEF-2).
    let (class, mut last) = match meta::classify(&c) {
        Ok(k) => (Some(k), None),
        Err(e) => (None, Some(e)),
    };
    // Steps 6 and 7: the plan, with every check made before allocation.
    let plan = match class {
        Some(Class::Sensor(v)) => match meta::plan(&c, v, largest(&candidates)) {
            Ok(p) => Some(p),
            Err(e) => {
                last = Some(e);
                None
            }
        },
        _ => None,
    };

    // A preview is enough when the caller wants a smaller image.
    if let Some((tw, th)) = options.target_size {
        let full_w = match &plan {
            Some(p) => output_size(p).0 as u64,
            None => candidates
                .first()
                .map_or(0, |p| u64::from(p.displayed(o).0)),
        };
        let fits = |p: &&arw::preview::Candidate| {
            let (w, h) = p.displayed(o);
            w >= tw || h >= th || (full_w > 0 && u64::from(w) * 20 >= full_w * 19)
        };
        if let Some(p) = candidates.iter().filter(fits).min_by_key(|p| p.area()) {
            match arw::preview::decode(data, p, o, options) {
                Ok(image) => {
                    return Ok(Some(Developed {
                        image,
                        origin: Origin::Preview,
                        class,
                        info: None,
                    }))
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
    }

    // Steps 8 to 14 for V1 to V8.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop(data, plan, &options.cancel, output) {
            Ok((image, stream_bits)) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        bits: plan.bits,
                        crop: plan.crop,
                        crop_source: plan.crop_source,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        white: plan.white,
                        black: [0, 1, 2, 3].map(|i| plan.black(i)),
                        wb: plan.wb,
                        wb_source: plan.wb_source,
                        matrix_from_table: plan.matrix_from_table,
                        table: plan.coding.as_ref().map(|k| k.table.name),
                        stream_bits,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-NEF-69: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-NEF-2, R-NEF-56: the preview path, largest candidate first, the
    // next one when a candidate fails (R-NEF-55).
    for p in &candidates {
        options.cancel.check()?;
        match arw::preview::decode(data, p, o, options) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Preview,
                    class,
                    info: None,
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => last = Some(e),
        }
    }
    // R-NEF-56: no JPEG decodes: the IFD0 thumbnail, else an error.
    if let Some(t) = preview::thumbnail(&c) {
        match preview::decode_thumbnail(data, t, o) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Thumbnail,
                    class,
                    info: None,
                }))
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| fail("no decodable image")))
}

/// Develop the sensor data to an upright sRGB image (steps 8 to 14) in the
/// requested encoding, through the Sony decoder's renderer. Also returns
/// the bits used and held by a V6 to V8 stream.
fn develop(
    data: &[u8],
    plan: &meta::Plan,
    cancel: &CancelToken,
    output: Output,
) -> NefResult<(ImageBuf, Option<(u64, u64)>)> {
    let (w, h) = output_size(plan);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    let shared = carrier(plan);
    let (plane, stream_bits) = sensor::decode(data, plan, cancel)?;
    let sensor = arw::render::Sensor::Cfa(plane);
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => {
            arw::render::render(&shared, &sensor, cancel, &arw::Colorimetric16)?
        }
        Output::Display16 => arw::render::render(&shared, &sensor, cancel, &arw::Display16(tone))?,
        Output::Display8 => arw::render::render(&shared, &sensor, cancel, &arw::Display8(tone))?,
    };
    Ok((ImageBuf::new(w, h, format, bytes)?, stream_bits))
}

//! Panasonic RW2 decoder, also for the older Panasonic RAW files of the
//! same container, written from the clean-room specification
//! `specs/raw/rw2.md` and the standards it cites (TIFF 6.0, ITU-T T.81).
//!
//! Variants V1 to V8 are developed from sensor data (R-RW2-1). Every other
//! file (X1 to X5), and every file that fails a check, shows its embedded
//! JPEG preview (R-RW2-2, R-RW2-49). The decoder never reads the Exif IFD,
//! the CameraIFD or the maker note to decode the sensor data (R-RW2-13);
//! the CameraIFD is never read as an IFD (R-RW2-63), and the Exif data of a
//! preview stream are removed before it is decoded (R-RW2-51).
//!
//! The levels, demosaicing, color matrix, orientation and output encoding
//! are the Sony decoder's ([`crate::raw::arw`]): the active area becomes a
//! plane whose CFA phase is that of the stored raster, and the plan is
//! handed to the shared renderer. The developed image is linear sRGB
//! (R-RW2-39) inside the pipeline; the provider delivers pandite's default
//! display rendering of it, like the Sony decoder. [`Developed::origin`]
//! reports whether the sensor data or a preview was delivered (R-RW2-50).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-RW2-64); a partly decoded raw image is dropped and
//! never delivered (R-RW2-65).

#![forbid(unsafe_code)]

mod color;
mod meta;
mod preview;
mod sensor;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Active, Class, Relative, Variant, WbSource};

use crate::raw::arw::{self, Output, Rendering};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{
    CancelToken, DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result,
};
use std::fmt::Display;

/// Results inside the RW2 decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type Rw2Result<T> = Result<T>;

/// A decoding failure (R-RW2-64: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: rw2: {msg}"))
}

/// Which image was delivered (R-RW2-50).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-RW2-39).
    Sensor(Variant),
    /// An embedded preview: sRGB-encoded JPEG values, not linear.
    Preview,
}

/// Facts about the raw data of a developed file. Read by the tests.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Info {
    /// Stored raster W x H.
    pub raster: (usize, usize),
    /// The active area.
    pub active: Active,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// Source of the white balance multipliers.
    pub wb: WbSource,
    /// True when the color matrix came from the model table.
    pub matrix_from_table: bool,
}

/// A delivered image and where it came from (R-RW2-50).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Developed {
    /// The image, upright.
    pub image: ImageBuf,
    /// Sensor data or preview.
    pub origin: Origin,
    /// The classification of the file.
    pub class: Class,
    /// Raw facts when the sensor data were developed.
    pub info: Option<Info>,
}

/// This format's entry in the developer table of [`crate::raw`].
pub(crate) const DEVELOPER: crate::raw::Developer = crate::raw::Developer {
    name: "Panasonic RW2",
    detect: is_rw2,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as RW2: the `.rw2`
/// extension, or the header bytes 49 49 55 00 that every RW2 file and every
/// older Panasonic RAW file starts with (R-RW2-5, M1).
pub(crate) fn is_rw2(data: &[u8], extension: Option<&str>) -> bool {
    extension == Some("rw2") || data.starts_with(&meta::MAGIC)
}

/// True when the file carries an embedded preview (R-RW2-45): a reduced
/// decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(Some(c)) if !preview::collect(&c).is_empty())
}

/// Size and orientation of the image [`decode`] delivers at full size, or
/// `Ok(None)` when the header is not 49 49 55 00 (the caller then uses its
/// generic preview search).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let Some(c) = meta::open(data)? else {
        return Ok(None);
    };
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    if let Class::Sensor(v) = meta::classify(&c) {
        if let Ok(plan) = meta::plan(&c, v) {
            let (w, h) = output_size(&plan);
            return Ok(Some((w as u32, h as u32, orientation)));
        }
    }
    let p = preview::collect(&c)
        .into_iter()
        .next()
        .ok_or_else(|| fail("no decodable image"))?;
    let (w, h) = p.displayed(o);
    Ok(Some((w, h, orientation)))
}

/// The storage of the sensor data by variant, for the facts panel.
fn compression_name(c: &meta::Container<'_>, class: Class) -> Option<&'static str> {
    Some(match class {
        Class::Sensor(Variant::V1) => "Panasonic 12-bit block code (RawFormat 4)",
        Class::Sensor(Variant::V2) => "Panasonic 12-bit packed (RawFormat 5)",
        Class::Sensor(Variant::V3) => "Panasonic 14-bit packed (RawFormat 5)",
        Class::Sensor(Variant::V4) => "Panasonic 12-bit block code (RawFormat 6)",
        Class::Sensor(Variant::V5) => "Panasonic 14-bit block code (RawFormat 6)",
        Class::Sensor(Variant::V6) => "Panasonic 14-bit packed (RawFormat 7)",
        Class::Sensor(Variant::V7) => "Panasonic 12-bit packed (RawFormat 2)",
        Class::Sensor(Variant::V8) => "Uncompressed, 16-bit containers",
        Class::OutOfScope(Relative::X1) => "Panasonic variable-length (RawFormat 8)",
        // The Compression names of source S3.
        Class::OutOfScope(_) => match c.uint(meta::tag::COMPRESSION)? {
            34316 => "Panasonic RAW 1",
            34826 => "Panasonic RAW 2",
            34828 => "Panasonic RAW 3",
            34830 => "Panasonic RAW 4",
            _ => return None,
        },
    })
}

/// Header facts of an RW2 file: the storage of the sensor data (from the
/// classification, R-RW2-3), the CFA layout and sample depth of IFD0, and
/// the camera settings of IFD0 and its Exif IFD. Reads tags only; no
/// sensor data. `None` when the header is not 49 49 55 00.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok().flatten()?;
    let mut f = ImageFacts::named("Panasonic RW2");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    f.compression = compression_name(&c, meta::classify(&c)).map(str::to_string);
    f.bits_per_channel = c
        .uint(meta::tag::BITS_PER_SAMPLE)
        .and_then(|b| u8::try_from(b).ok())
        .filter(|&b| b > 0);
    if let Some(colors) = c.uint(meta::tag::CFA_PATTERN).and_then(meta::cfa_colors) {
        let names = ["R", "G", "B"];
        let pattern: String = colors.iter().map(|&k| names[usize::from(k)]).collect();
        f.channels = Some(format!("Bayer {pattern}"));
        f.channel_count = Some(1);
    }
    Some(f)
}

/// Decode an RW2 file to an upright image with the default display
/// rendering, or `Ok(None)` when the header is not 49 49 55 00.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-RW2-50).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// The output size of a plan: the active area, turned for orientations 5
/// to 8 (R-RW2-44).
fn output_size(plan: &meta::Plan) -> (usize, usize) {
    arw::render::output_size(carrier_crop(plan), plan.orientation)
}

/// The whole active area as the crop of the shared renderer (R-RW2-17).
fn carrier_crop(plan: &meta::Plan) -> arw::Rect {
    let (w, h) = plan.active.size();
    arw::Rect { x: 0, y: 0, w, h }
}

/// The plan of the shared renderer for the plane of the active area
/// (R-RW2-35, R-RW2-40 to R-RW2-42, R-RW2-38, R-RW2-44).
fn carrier(plan: &meta::Plan) -> arw::meta::Plan {
    let crop = carrier_crop(plan);
    let (top, left) = (plan.active.top, plan.active.left);
    // R-RW2-18, R-RW2-42: plane pixel (x, y) is stored pixel (left + x,
    // top + y), so plane CFA position k (x = k mod 2, y = floor(k / 2)) has
    // the color of stored CFA position 2 * ((top + y) mod 2) + (left + x)
    // mod 2. The renderer demosaics inside the plane, which is the active
    // area (R-RW2-41).
    let mut cfa = [0u8; 4];
    let mut black = [0u32; 4];
    for (k, (color, b)) in cfa.iter_mut().zip(black.iter_mut()).enumerate() {
        let (x, y) = (left + (k & 1), top + (k >> 1));
        *color = plan.colors[2 * (y % 2) + (x % 2)];
        *b = plan.black[usize::from(*color)];
    }
    // R-RW2-35, R-RW2-61: u = clip((v - b_c) / (w_c - b_c) * m_c, 0, 1),
    // and the output is clipped to 0 to 1 after the matrix. The renderer
    // takes one white level; with F as that level the per-color white goes
    // into the multiplier: m_c * (F - b_c) / (w_c - b_c), divided by
    // F - b_c, is m_c / (w_c - b_c). R-RW2-62: w_c - b_c > 255 (R-RW2-33).
    let full = plan.variant.full_scale();
    let wb = [0, 1, 2].map(|c| {
        let span = f64::from(plan.white[c]) - f64::from(plan.black[c]);
        plan.wb[c] * (f64::from(full) - f64::from(plan.black[c])) / span.max(1.0)
    });
    arw::meta::Plan {
        // The renderer's CFA path (any variant but its YCbCr one).
        variant: arw::Variant::V1,
        width: crop.w,
        height: crop.h,
        valid: (crop.w, crop.h),
        crop,
        orientation: plan.orientation,
        cfa,
        white: full,
        black,
        wb,
        wb_source: match plan.wb_source {
            WbSource::AsShot => arw::WbSource::AsShot,
            WbSource::Table => arw::WbSource::Table,
            WbSource::Generic => arw::WbSource::Generic,
        },
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
/// origin of the delivered image (R-RW2-50).
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    // Steps 1 and 2 of the decoding procedure: header and IFD0.
    let Some(c) = meta::open(data)? else {
        // X4: the caller's generic preview search (R-RW2-2).
        return Ok(None);
    };
    let o = c.orientation();
    // Step 3: collect the preview candidates first, so every later failure
    // can fall back (R-RW2-45, R-RW2-46).
    let candidates = preview::collect(&c);
    // Step 4: classification (R-RW2-3, R-RW2-4); X1 to X5 use the preview
    // path (R-RW2-2).
    let class = meta::classify(&c);
    let mut last = None;
    // Steps 5 and 6: the plan, with every check made before allocation.
    let plan = match class {
        Class::Sensor(v) => match meta::plan(&c, v) {
            Ok(p) => Some(p),
            Err(e) => {
                last = Some(e);
                None
            }
        },
        Class::OutOfScope(_) => None,
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
            match preview::decode(data, p, o, options) {
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

    // Steps 7 to 12 for the specified variants.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop(data, plan, &options.cancel, output) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        active: plan.active,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        wb: plan.wb_source,
                        matrix_from_table: plan.matrix_from_table,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-RW2-65: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-RW2-2, R-RW2-49: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
    // R-RW2-47, R-RW2-48 and R-RW2-51 are applied by `preview::decode`.
    for p in &candidates {
        options.cancel.check()?;
        match preview::decode(data, p, o, options) {
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
    // R-RW2-66: a RAW file (V7, V8) without a preview candidate reports
    // its decoding failure as an error, because no fallback exists.
    Err(last.unwrap_or_else(|| fail("no decodable image")))
}

/// Develop the sensor data to an upright sRGB image (steps 7 to 12) in the
/// requested encoding, through the Sony decoder's renderer.
fn develop(
    data: &[u8],
    plan: &meta::Plan,
    cancel: &CancelToken,
    output: Output,
) -> Rw2Result<ImageBuf> {
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
    let sensor = arw::render::Sensor::Cfa(sensor::decode(data, plan, cancel)?);
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => {
            arw::render::render(&shared, &sensor, cancel, &arw::Colorimetric16)?
        }
        Output::Display16 => arw::render::render(&shared, &sensor, cancel, &arw::Display16(tone))?,
        Output::Display8 => arw::render::render(&shared, &sensor, cancel, &arw::Display8(tone))?,
    };
    ImageBuf::new(w, h, format, bytes)
}

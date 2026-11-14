//! Pentax PEF decoder, written from the clean-room specification
//! `specs/raw/pef.md` and the standards it cites (TIFF 6.0, ITU-T T.81).
//!
//! Variants P1 (16-bit samples), P2 (12-bit packed), P3 (Pentax Huffman
//! coding, CFA) and P4 (Pentax Huffman coding, monochrome) are developed
//! from sensor data (R-PEF-1). Every other file, and every file that fails
//! a check, shows its embedded JPEG preview (R-PEF-2, R-PEF-57). A file with
//! several frames (Pixel Shift, HDR) is developed from its IFD0 frame alone
//! (R-PEF-52).
//!
//! The container, the maker note, the classification, the Huffman decoder
//! and the levels are this module's own. The rest is shared with the Sony
//! decoder ([`crate::raw::arw`]): the demosaicing, the color matrix and the
//! orientation of the renderer, the display rendering and its encoders,
//! and the preview decoding. The developed image is linear sRGB (R-PEF-46)
//! inside the pipeline; the provider delivers pandite's default display
//! rendering of it, like the Sony decoder. [`Developed::origin`] reports
//! whether the sensor data or a preview was delivered (R-PEF-58).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-PEF-68); a partly decoded raw image is dropped with
//! the error and never delivered (R-PEF-69).

#![forbid(unsafe_code)]

mod color;
mod huffman;
mod meta;
mod preview;
mod render;
mod sensor;
mod tiff;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Class, Variant};

use crate::raw::arw::preview::{self as shared_preview, Candidate};
use crate::raw::arw::{Colorimetric16, Display16, Display8, Output, Rect, Rendering, WbSource};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result};
use std::fmt::Display;

/// Results inside the PEF decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type PefResult<T> = Result<T>;

/// A decoding failure (R-PEF-68: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: pef: {msg}"))
}

/// Which image was delivered (R-PEF-58).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-PEF-46).
    Sensor(Variant),
    /// An embedded preview: sRGB-encoded JPEG values, `Rgba8`.
    Preview,
}

/// Facts about the raw data of a developed file. Read by the tests.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Info {
    /// Stored raster W x H.
    pub raster: (usize, usize),
    /// The crop rectangle.
    pub crop: Rect,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// Source of the white balance multipliers.
    pub wb: WbSource,
    /// True when the color matrix came from the model table.
    pub matrix_from_table: bool,
    /// True when IFD0 has SubIFDs (decoded as IFD0 alone).
    pub multi_frame: bool,
}

/// A delivered image and where it came from (R-PEF-58). The provider
/// passes on the image only; the tests read the report.
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
    name: "Pentax PEF",
    detect: is_pef,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as PEF: the `.pef`
/// extension, or a TIFF file whose IFD0 holds CFA or linear raw data
/// (PhotometricInterpretation 32803 or 34892) and whose maker note has the
/// AOC or PENTAX form.
pub(crate) fn is_pef(data: &[u8], extension: Option<&str>) -> bool {
    if extension == Some("pef") {
        return true;
    }
    let Ok(c) = meta::open(data) else {
        return false;
    };
    c.maker.is_some()
        && matches!(
            c.ifd0().uint(meta::tag::PHOTOMETRIC),
            Ok(Some(32803 | 34892))
        )
}

/// True when the file carries an embedded preview (R-PEF-53, R-PEF-54): a
/// reduced decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(c) if !preview::collect(&c).is_empty())
}

/// Size and orientation of the image [`decode`] delivers at full size.
/// Fails for a file that is not a TIFF file (R-PEF-4).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let c = meta::open(data)?;
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    if let Ok(Class::Sensor(v)) = meta::classify(&c) {
        if let Ok(plan) = meta::plan(&c, v) {
            let (w, h) = render::output_size(&plan);
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

/// Header facts of a PEF file: the storage of the sensor data (from the
/// classification, R-PEF-3), the CFA layout and sample depth of IFD0, and
/// the camera settings of IFD0 and its Exif IFD. Reads tags only; no sensor
/// data. `None` when the data are not a TIFF file.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok()?;
    let mut f = ImageFacts::named("Pentax PEF");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    let ifd0 = c.ifd0();
    let compression = ifd0.uint(meta::tag::COMPRESSION).ok().flatten();
    f.compression = match meta::classify(&c).ok() {
        Some(Class::Sensor(Variant::P1)) => Some("Uncompressed"),
        Some(Class::Sensor(Variant::P2)) => Some("Pentax 12-bit packed"),
        Some(Class::Sensor(Variant::P3 | Variant::P4)) => Some("Pentax Huffman"),
        _ => match compression {
            Some(1) => Some("Uncompressed"),
            Some(65535) => Some("Pentax compressed"),
            _ => None,
        },
    }
    .map(str::to_string);
    let bits = ifd0
        .uint(meta::tag::BITS_PER_SAMPLE)
        .ok()
        .flatten()
        .and_then(|b| u8::try_from(b).ok())
        .filter(|&b| b > 0);
    f.bits_per_channel = bits;
    match ifd0.uint(meta::tag::PHOTOMETRIC).ok().flatten() {
        Some(32803) => {
            let names = ["R", "G", "B"];
            let pattern: Option<String> = c
                .exif
                .as_ref()
                .and_then(|e| e.byte_array(meta::tag::CFA_PATTERN).ok().flatten())
                .and_then(meta::bayer)
                .map(|p| p.iter().map(|&k| names[usize::from(k)]).collect());
            f.channels = Some(match pattern {
                Some(p) => format!("Bayer {p}"),
                None => "Bayer".to_string(),
            });
            f.channel_count = Some(1);
        }
        Some(34892) => {
            f.channels = Some("Gray".to_string());
            f.channel_count = Some(1);
        }
        _ => {}
    }
    Some(f)
}

/// Decode a PEF file to an upright image with the default display
/// rendering. Fails for a file that is not a TIFF file (R-PEF-4).
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-PEF-58).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-PEF-58).
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    options.cancel.check()?;
    // Steps 1 and 2 of the decoding procedure: header and IFD chain
    // (R-PEF-4: any other start is an error; R-PEF-11: so is an invalid
    // IFD0).
    let c = meta::open(data)?;
    let o = c.orientation();
    // Step 3: collect the preview candidates first, so every later failure
    // can fall back (R-PEF-53, R-PEF-54).
    let candidates = preview::collect(&c);
    // Steps 4 and 5: maker note and classification (R-PEF-3).
    let (class_ok, mut last) = match meta::classify(&c) {
        Ok(k) => (Some(k), None),
        Err(e) => (None, Some(e)),
    };
    // Steps 5 to 7: the plan, with every check made before allocation.
    // R-PEF-2: X2 and X3 files have no plan; their sensor data are never
    // interpreted.
    let plan = match class_ok {
        Some(Class::Sensor(v)) => match meta::plan(&c, v) {
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
            Some(p) => render::output_size(p).0 as u64,
            None => candidates
                .first()
                .map_or(0, |p| u64::from(p.displayed(o).0)),
        };
        let fits = |p: &&Candidate| {
            let (w, h) = p.displayed(o);
            w >= tw || h >= th || (full_w > 0 && u64::from(w) * 20 >= full_w * 19)
        };
        if let Some(p) = candidates.iter().filter(fits).min_by_key(|p| p.area()) {
            match shared_preview::decode(data, p, o, options) {
                Ok(image) => {
                    return Ok(Some(Developed {
                        image,
                        origin: Origin::Preview,
                        class: class_ok,
                        info: None,
                    }))
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
    }

    // Steps 8 to 13 for the specified variants.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop(data, plan, options, output) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class: class_ok,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        crop: plan.crop,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        wb: plan.wb_source,
                        matrix_from_table: plan.matrix_from_table,
                        multi_frame: plan.multi_frame,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-PEF-69: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-PEF-2, R-PEF-57: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
    // R-PEF-55 and R-PEF-56 are applied by the shared preview decoder.
    for p in &candidates {
        options.cancel.check()?;
        match shared_preview::decode(data, p, o, options) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Preview,
                    class: class_ok,
                    info: None,
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| fail("no decodable image")))
}

/// Develop the sensor data to an upright sRGB image (steps 8 to 13) in the
/// requested encoding. Full precision is kept until the last step.
fn develop(
    data: &[u8],
    plan: &meta::Plan,
    options: &DecodeOptions,
    output: Output,
) -> PefResult<ImageBuf> {
    let (w, h) = render::output_size(plan);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    // Step 8: the sensor data of the crop rectangle.
    let plane = sensor::decode(data, plan, &options.cancel)?;
    // Steps 9 to 12 and the encoding of step 13.
    let tone = Tone::unit();
    let (colorimetric, display16, display8) = (Colorimetric16, Display16(tone), Display8(tone));
    let encoder: &dyn crate::raw::arw::render::RowEncoder = match output {
        Output::Colorimetric16 => &colorimetric,
        Output::Display16 => &display16,
        Output::Display8 => &display8,
    };
    let bytes = render::render(plan, plane, &options.cancel, encoder)?;
    ImageBuf::new(w, h, format, bytes)
}

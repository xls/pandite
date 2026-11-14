//! Olympus and OM System ORF decoder, written from the clean-room
//! specification `specs/raw/orf.md` and the standards it cites (TIFF 6.0,
//! Exif 2.32, ITU-T T.81).
//!
//! Variants V1 (16-bit samples), V2 (10 samples in 16 bytes), V3 (12-bit
//! pairs in two fields), V4 (12-bit values in 32-bit words), V5 and V6
//! (Olympus lossless compression, 12 and 14 bits) are developed from
//! sensor data (R-ORF-1). Files that match no rule (X1), big-endian files
//! of another layout (X2) and every file that fails a check show their
//! embedded JPEG preview (R-ORF-2, R-ORF-53).
//!
//! The developed image is linear sRGB (R-ORF-43) inside the pipeline, and
//! it goes through the rendering of the Sony decoder: the colorimetric
//! result as [`Rendering::Colorimetric`] (`Rgba16` with the sRGB transfer
//! function), and pandite's default display rendering as
//! [`Rendering::Display`], which the provider delivers. A preview arrives
//! as `Rgba8`. [`Developed::origin`] reports which one was delivered
//! (R-ORF-54).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-ORF-64); a partly decoded raw image is dropped with
//! the error and never delivered (R-ORF-65).

#![forbid(unsafe_code)]

mod develop;
mod lossless;
mod meta;
mod preview;
mod sensor;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Class, Source, Style, Variant};

use crate::raw::arw::{Output, Rect, Rendering};
use pandite_core::{DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, Result};
use std::fmt::Display;

/// Results inside the ORF decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type OrfResult<T> = Result<T>;

/// A decoding failure (R-ORF-64: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: orf: {msg}"))
}

/// Which image was delivered (R-ORF-54).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-ORF-43).
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
    /// Final image area (R-ORF-17).
    pub crop: Rect,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// The maker-note style.
    pub style: Style,
    /// Bit depth B (R-ORF-33, R-ORF-34).
    pub bits: u32,
    /// Black levels of CFA positions 0 to 3 (R-ORF-36).
    pub black: [u32; 4],
    /// Source of the black levels.
    pub black_source: Source,
    /// WB multipliers mR, mG, mB (R-ORF-38).
    pub wb: [f64; 3],
    /// Source of the white balance multipliers.
    pub wb_source: Source,
    /// Source of the color matrix.
    pub matrix_source: Source,
    /// V5 and V6: the strip bits after the last code (R-ORF-26).
    pub unused_bits: Option<u64>,
}

/// A delivered image and where it came from (R-ORF-54). The provider
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
    name: "Olympus ORF",
    detect: is_orf,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as ORF: the `.orf`
/// extension, or one of the three ORF header starts of R-ORF-4, which no
/// other format uses. ORI files (X3) are recognized by their content.
pub(crate) fn is_orf(data: &[u8], extension: Option<&str>) -> bool {
    extension == Some("orf")
        || data.starts_with(b"IIRO")
        || data.starts_with(b"IIRS")
        || data.starts_with(b"MMOR")
}

/// True when the file carries an embedded preview (R-ORF-48, R-ORF-49): a
/// reduced decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(c) if !preview::collect(&c).is_empty())
}

/// Size and orientation of the image [`decode`] delivers at full size. An
/// error for data that are not an ORF file (R-ORF-4) or without any image.
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let c = meta::open(data)?;
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    if let Ok(Class::Sensor(v)) = meta::classify(&c) {
        if let Ok(plan) = meta::plan(&c, v) {
            let (w, h) = develop::output_size(&plan);
            return Ok(Some((w as u32, h as u32, orientation)));
        }
    }
    let p = preview::collect(&c)
        .into_iter()
        .next()
        .ok_or_else(|| fail("no decodable image"))?;
    let (w, h) = preview::displayed(&p, o);
    Ok(Some((w, h, orientation)))
}

/// Header facts of an ORF file: the storage of the sensor data (from the
/// classification, R-ORF-3), the bit depth B of the decoded values
/// (R-ORF-33, R-ORF-34), the CFA layout of the Exif CFAPattern (R-ORF-15),
/// and the camera settings of IFD0 and its Exif IFD. Reads tags only; no
/// sensor data. `None` when the data are not an ORF file.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok()?;
    let mut f = ImageFacts::named("Olympus ORF");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    if let Ok(Class::Sensor(v)) = meta::classify(&c) {
        f.compression = Some(
            match v {
                Variant::V1 => "Uncompressed",
                Variant::V2 => "Olympus 12-bit packed",
                Variant::V3 => "Olympus 12-bit packed, two fields",
                Variant::V4 => "Olympus 12-bit packed in 32-bit words",
                Variant::V5 | Variant::V6 => "Olympus lossless",
            }
            .to_string(),
        );
        f.bits_per_channel = meta::bit_depth(c.note.as_ref(), v)
            .ok()
            .and_then(|b| u8::try_from(b).ok());
    }
    if let Some(cfa) = c
        .exif
        .as_ref()
        .and_then(|e| meta::cfa_pattern(e, c.le).ok())
    {
        let names = ["R", "G", "B"];
        let p: String = cfa.iter().map(|&k| names[usize::from(k)]).collect();
        f.channels = Some(format!("Bayer {p}"));
        f.channel_count = Some(1);
    }
    Some(f)
}

/// Decode an ORF file to an upright image with the default display
/// rendering.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-ORF-54).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-ORF-54). The steps are those of the
/// specification's Decoding procedure.
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    // Steps 1 to 3: size, header, IFD chain, Exif IFD and maker note.
    // R-ORF-4: another start or an invalid IFD0 is an error without a
    // preview.
    let c = meta::open(data)?;
    let o = c.orientation();
    // Step 4: collect the preview candidates first, so every later failure
    // can fall back (R-ORF-48 to R-ORF-50).
    let candidates = preview::collect(&c);
    // Step 5: classification (R-ORF-3). X1 and X2 use the preview path and
    // their sensor data are never read (R-ORF-2).
    let (class, mut last) = match meta::classify(&c) {
        Ok(k) => (Some(k), None),
        Err(e) => (None, Some(e)),
    };
    // Steps 5 to 7: the plan, with every check made before allocation.
    let plan = match class {
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
            Some(p) => develop::output_size(p).0 as u64,
            None => candidates
                .first()
                .map_or(0, |p| u64::from(preview::displayed(p, o).0)),
        };
        let fits = |p: &&crate::raw::arw::preview::Candidate| {
            let (w, h) = preview::displayed(p, o);
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

    // Steps 8 to 14 for V1 to V6.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop::develop(data, plan, &options.cancel, output) {
            Ok((image, unused_bits)) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        crop: plan.crop,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        style: plan.style,
                        bits: plan.bits,
                        black: plan.black,
                        black_source: plan.black_source,
                        wb: plan.wb,
                        wb_source: plan.wb_source,
                        matrix_source: plan.matrix_source,
                        unused_bits,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-ORF-65: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-ORF-2, R-ORF-53: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
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
    Err(last.unwrap_or_else(|| fail("no decodable image")))
}

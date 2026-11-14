//! Samsung SRW decoder, written from the clean-room specification
//! `specs/raw/srw.md` and the standards it cites (TIFF 6.0, Exif 2.32).
//!
//! Variants V1 (16-bit samples) and V2 (12-bit packed) are developed from
//! sensor data (R-SRW-1). The compressed codings (X1, X2, X3), any other
//! combination (X4), files without a raw IFD (X5) and every file that
//! fails a check show their embedded JPEG preview (R-SRW-2, R-SRW-48).
//!
//! The developed image is linear sRGB (R-SRW-38) inside the pipeline, and
//! it goes through the rendering of the Sony decoder: the colorimetric
//! result as [`Rendering::Colorimetric`] (`Rgba16` with the sRGB transfer
//! function), and pandite's default display rendering as
//! [`Rendering::Display`], which the provider delivers. A preview arrives
//! as `Rgba8`. [`Developed::origin`] reports which one was delivered
//! (R-SRW-49).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-SRW-57); a partly decoded raw image is dropped with
//! the error and never delivered (R-SRW-58).

#![forbid(unsafe_code)]

mod color;
mod develop;
pub(crate) mod ifd;
mod meta;
mod preview;
mod sensor;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Class, MatrixSource, Relative, Variant, WbSource};

use crate::raw::arw::{self, Output, Rect, Rendering};
use meta::tag;
use pandite_core::{DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, Result};
use std::fmt::Display;

/// Results inside the SRW decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type SrwResult<T> = Result<T>;

/// A decoding failure (R-SRW-57: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: srw: {msg}"))
}

/// Which image was delivered (R-SRW-49).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-SRW-38).
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
    /// Valid area (R-SRW-22).
    pub valid: Rect,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// White level (R-SRW-31).
    pub white: u32,
    /// Black levels R, G, B (R-SRW-32).
    pub black: [f64; 3],
    /// Source of the white balance multipliers.
    pub wb: WbSource,
    /// Source of the color matrix.
    pub matrix: MatrixSource,
    /// True when the raw data byte order is little-endian (R-SRW-25).
    pub data_le: bool,
}

/// A delivered image and where it came from (R-SRW-49). The provider
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
    name: "Samsung SRW",
    detect: is_srw,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as SRW: the `.srw`
/// extension, or a TIFF (R-SRW-4) whose IFD0 Make is "SAMSUNG" (M1) and
/// whose raw IFD (R-SRW-12) has one of the SRW Compression values of the
/// specification (32769, 32770, 32772, 32773), which plain TIFF files do
/// not use.
pub(crate) fn is_srw(data: &[u8], extension: Option<&str>) -> bool {
    if extension == Some("srw") {
        return true;
    }
    let Ok(c) = meta::open(data) else {
        return false;
    };
    let samsung = c
        .ifd0()
        .is_some_and(|d| matches!(d.ascii(tag::MAKE), Ok(Some(b"SAMSUNG"))));
    samsung
        && c.raw.as_ref().is_some_and(|r| {
            matches!(
                r.uint(tag::COMPRESSION),
                Ok(Some(32769 | 32770 | 32772 | 32773))
            )
        })
}

/// True when the file carries an embedded preview (R-SRW-44): a reduced
/// decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(c) if !preview::collect(&c).is_empty())
}

/// Size and orientation of the image [`decode`] delivers at full size. An
/// error for data that are not a TIFF (R-SRW-4) or without any image.
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
    let (w, h) = p.displayed(o);
    Ok(Some((w, h, orientation)))
}

/// Header facts of an SRW file: the storage of the sensor data (from the
/// classification, R-SRW-3), the CFA layout and sample depth of the raw
/// IFD, and the camera settings of IFD0 and its Exif IFD. Reads tags only;
/// no sensor data. `None` when the data are not a TIFF.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok()?;
    let mut f = ImageFacts::named("Samsung SRW");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    let Some(raw) = &c.raw else {
        return Some(f);
    };
    f.compression = match meta::classify(&c).ok() {
        Some(Class::Sensor(Variant::V1)) => Some("Uncompressed"),
        Some(Class::Sensor(Variant::V2)) => Some("Samsung 12-bit packed"),
        Some(Class::OutOfScope(Relative::X1)) => Some("Samsung SRW Compressed"),
        Some(Class::OutOfScope(Relative::X2)) => Some("Samsung SRW Compressed 2"),
        Some(Class::OutOfScope(Relative::X3)) => Some("Samsung compressed"),
        _ => None,
    }
    .map(str::to_string);
    f.bits_per_channel = raw
        .uint(tag::BITS_PER_SAMPLE)
        .ok()
        .flatten()
        .and_then(|b| u8::try_from(b).ok())
        .filter(|&b| b > 0);
    // The CFA layout when R-SRW-17 accepts it: a 2 x 2 repeat and a Bayer
    // pattern (0, 1, 1, 2 when the tag is absent).
    let dim_ok = !raw.has(tag::CFA_REPEAT_PATTERN_DIM)
        || matches!(raw.uints(tag::CFA_REPEAT_PATTERN_DIM), Ok(Some(d)) if d == [2, 2]);
    let pattern: Option<Vec<u8>> = if raw.has(tag::CFA_PATTERN) {
        raw.byte_values(tag::CFA_PATTERN)
            .ok()
            .flatten()
            .map(<[u8]>::to_vec)
    } else {
        Some(vec![0, 1, 1, 2])
    };
    let bayer = pattern.filter(|p| {
        let count = |c: u8| p.iter().filter(|&&x| x == c).count();
        p.len() == 4 && count(0) == 1 && count(1) == 2 && count(2) == 1
    });
    if let (true, Some(p)) = (dim_ok, bayer) {
        let names = ["R", "G", "B"];
        let p: String = p.iter().map(|&c| names[usize::from(c)]).collect();
        f.channels = Some(format!("Bayer {p}"));
        f.channel_count = Some(1);
    }
    Some(f)
}

/// Decode an SRW file to an upright image with the default display
/// rendering.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-SRW-49).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-SRW-49). The steps are those of the
/// specification's Decoding procedure.
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    // Steps 1 and 2: size, header and IFD chain. R-SRW-4: another start is
    // an error without a preview.
    let c = meta::open(data)?;
    let o = c.orientation();
    // Step 3: collect the preview candidates first, so every later failure
    // can fall back (R-SRW-44, R-SRW-45).
    let candidates = preview::collect(&c);
    // Step 4: raw IFD and classification (R-SRW-3). X1 to X5 use the
    // preview path and their strip is never read (R-SRW-2, R-SRW-30).
    let (class, mut last) = match meta::classify(&c) {
        Ok(k) => (Some(k), None),
        Err(e) => (None, Some(e)),
    };
    // Steps 5 to 8: the plan, with every check made before allocation.
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

    // Steps 9 to 13 for V1 and V2.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop::develop(data, plan, &options.cancel, output) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        valid: plan.valid,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        white: plan.white,
                        black: plan.black,
                        wb: plan.wb_source,
                        matrix: plan.matrix_source,
                        data_le: plan.data_le,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-SRW-58: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-SRW-2, R-SRW-48: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
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
    Err(last.unwrap_or_else(|| fail("no decodable image")))
}

//! Fujifilm RAF decoder, written from the clean-room specification
//! `specs/raw/raf.md` and the standards it cites (TIFF 6.0, ITU-T T.81,
//! Exif 2.32).
//!
//! Variants V1 (12-bit packed), V2 (14-bit packed) and V3 (16-bit
//! containers) with the X-Trans (C1) or Bayer (C2) layout are developed
//! from sensor data (R-RAF-1). Every other file, and every file that fails
//! a check, shows its embedded JPEG preview (R-RAF-2, R-RAF-52).
//!
//! The shared parts of the Sony decoder are called, not copied: the TIFF
//! IFD reader (the CFA container is a small TIFF structure), the sample
//! plane, the preview frame size and decoding, the orientation table, the
//! row encoders (color matrix, default display rendering, sRGB encoding)
//! and the vector kernels. The X-Trans demosaic is this module's own.
//!
//! The developed image is linear sRGB (R-RAF-38) inside the pipeline and is
//! delivered like the Sony decoder's: [`Rendering::Display`] (pandite's
//! default display rendering, `Rgba16`, or `Rgba8` when the caller prefers
//! 8 bits) through the provider, [`Rendering::Colorimetric`] (`Rgba16` with
//! the sRGB transfer function) for the tests. A preview arrives as `Rgba8`.
//! [`Developed::origin`] reports which one was delivered (R-RAF-53).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-RAF-63); a partly decoded raw image is dropped with
//! its error and never delivered (R-RAF-64).

#![forbid(unsafe_code)]

mod container;
mod preview;
mod render;
mod sensor;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use container::{Class, Layout, Relative, Variant, WbSource};

use crate::raw::arw::{self, render::output_size, Output, Rect};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{
    CancelToken, DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result,
};
use std::fmt::Display;

/// Results inside the RAF decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type RafResult<T> = Result<T>;

/// A decoding failure (R-RAF-63: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: raf: {msg}"))
}

/// Which image was delivered (R-RAF-53).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-RAF-38).
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
    /// Final image area.
    pub crop: Rect,
    /// Exif orientation 1 to 8.
    pub orientation: u8,
    /// Color filter layout.
    pub layout: Layout,
    /// BitsPerSample.
    pub bits: u32,
    /// White level.
    pub white: u32,
    /// Black level.
    pub black: f64,
    /// WB multipliers R, G, B.
    pub wb: [f64; 3],
    /// Source of the WB multipliers.
    pub wb_source: WbSource,
}

/// A delivered image and where it came from (R-RAF-53). The provider
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
    name: "Fujifilm RAF",
    detect: is_raf,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as RAF: the `.raf`
/// extension, or the 16-byte magic of R-RAF-4.
pub(crate) fn is_raf(data: &[u8], extension: Option<&str>) -> bool {
    extension == Some("raf") || data.starts_with(container::MAGIC)
}

/// True when the file carries a preview candidate (R-RAF-47, R-RAF-48):
/// a reduced decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(container::open(data), Ok(Some(raf)) if !preview::collect(&raf).candidates.is_empty())
}

/// Size and orientation of the image [`decode`] delivers at full size, or
/// `Ok(None)` when the data are not a RAF file (R-RAF-4).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let Some(raf) = container::open(data)? else {
        return Ok(None);
    };
    let previews = preview::collect(&raf);
    let o = previews.orientation;
    let orientation = Orientation::from_exif(u16::from(o));
    if let Ok(a) = container::analyze(&raf) {
        if let Some(plan) = a.tags.and_then(|t| container::plan(&raf, &t, o).ok()) {
            let (w, h) = output_size(plan.crop, o);
            return Ok(Some((w as u32, h as u32, orientation)));
        }
    }
    let p = previews
        .candidates
        .first()
        .ok_or_else(|| fail("no decodable image"))?;
    let (w, h) = p.displayed(o);
    Ok(Some((w, h, orientation)))
}

/// Header facts of a RAF file: the storage of the sensor data (from the
/// classification, R-RAF-3), the color filter layout and sample depth of
/// the FujiIFD, and the camera settings of the preview's Exif segment.
/// Reads tags only; no sensor data. `None` when the data are not RAF.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let raf = container::open(data).ok().flatten()?;
    let mut f = ImageFacts::named("Fujifilm RAF");
    if let Some(jpeg) = raf.part(raf.jpeg) {
        if let Some(j) = crate::jpeg::header_facts(jpeg) {
            f.camera = j.camera;
        }
    }
    let Ok(a) = container::analyze(&raf) else {
        return Some(f);
    };
    f.compression = match a.class {
        Class::Sensor(Variant::V1, _) => Some("Uncompressed, 12-bit packed"),
        Class::Sensor(Variant::V2, _) => Some("Uncompressed, 14-bit packed"),
        Class::Sensor(Variant::V3, _) => Some("Uncompressed"),
        Class::OutOfScope(Relative::X1) => Some("Fujifilm lossless compressed"),
        Class::OutOfScope(Relative::X2) => Some("Fujifilm lossy compressed"),
        Class::OutOfScope(_) => None,
    }
    .map(str::to_string);
    if let (Class::Sensor(_, layout), Some(t)) = (a.class, a.tags) {
        let name = match layout {
            Layout::XTrans => "X-Trans",
            Layout::Bayer => "Bayer RGGB",
        };
        f.channels = Some(name.to_string());
        f.channel_count = Some(1);
        f.bits_per_channel = u8::try_from(t.bits).ok();
    }
    Some(f)
}

/// How developed sensor data are delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rendering {
    /// The colorimetric result of the specification: linear sRGB
    /// (R-RAF-38) with the sRGB transfer function, `Rgba16`.
    #[cfg_attr(not(test), allow(dead_code))]
    Colorimetric,
    /// pandite's default display rendering ([`crate::raw::tone`]) on top
    /// of the colorimetric result: `Rgba8` when the caller prefers 8 bits,
    /// else `Rgba16`.
    Display,
}

/// Decode a RAF file to an upright image with the default display
/// rendering, or `Ok(None)` when the data are not a RAF file.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-RAF-53).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-RAF-53).
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    options.cancel.check()?;
    // Step 1: file size, magic and header (R-RAF-54, R-RAF-4, R-RAF-5).
    let Some(raf) = container::open(data)? else {
        return Ok(None);
    };
    // Step 2: the preview candidates and the orientation first, so every
    // later failure can fall back (R-RAF-43, R-RAF-47).
    let previews = preview::collect(&raf);
    let o = previews.orientation;
    let candidates = &previews.candidates;
    // Steps 3 to 5: meta container, CFA container, classification.
    let (class, tags, mut last) = match container::analyze(&raf) {
        Ok(a) => (Some(a.class), a.tags, None),
        Err(e) => (None, None, Some(e)),
    };
    // Steps 6 and 7: the plan, with every check made before allocation.
    let plan = match tags {
        Some(t) => match container::plan(&raf, &t, o) {
            Ok(p) => Some(p),
            Err(e) => {
                last = Some(e);
                None
            }
        },
        None => None,
    };

    // A preview is enough when the caller wants a smaller image.
    if let Some((tw, th)) = options.target_size {
        let full_w = match &plan {
            Some(p) => output_size(p.crop, o).0 as u64,
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

    // Steps 8 to 13 for the specified variants.
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
                        crop: plan.crop,
                        orientation: plan.orientation,
                        layout: plan.layout,
                        bits: plan.bits,
                        white: plan.white,
                        black: plan.black,
                        wb: plan.wb,
                        wb_source: plan.wb_source,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-RAF-64: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-RAF-2, R-RAF-52: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
    for p in candidates {
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

/// Develop the sensor data to an upright sRGB image (steps 8 to 13) in
/// the requested encoding, through the Sony decoder's row encoders.
fn develop(
    data: &[u8],
    plan: &container::Plan,
    cancel: &CancelToken,
    output: Output,
) -> RafResult<ImageBuf> {
    let (w, h) = output_size(plan.crop, plan.orientation);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    let plane = sensor::decode(data, plan, cancel)?;
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => render::render(plan, &plane, cancel, &arw::Colorimetric16)?,
        Output::Display16 => render::render(plan, &plane, cancel, &arw::Display16(tone))?,
        Output::Display8 => render::render(plan, &plane, cancel, &arw::Display8(tone))?,
    };
    ImageBuf::new(w, h, format, bytes)
}

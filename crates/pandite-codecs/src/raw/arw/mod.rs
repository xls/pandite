//! Sony ARW decoder, written from the clean-room specification
//! `specs/raw/arw.md` and the standards it cites (TIFF 6.0, ITU-T T.81).
//!
//! Variants V1 (uncompressed), V2 (12-bit packed), V3 (lossy 128-bit
//! blocks), V4 (lossless CFA tiles) and V5 (lossless YCbCr tiles) are
//! developed from sensor data (R-ARW-1). Every other file, and every file
//! that fails a check, shows its embedded JPEG preview (R-ARW-2, R-ARW-63).
//! The specification never depends on the maker note or the encrypted
//! SR2 structures, and neither does this code (R-ARW-15).
//!
//! The developed image is linear sRGB (R-ARW-53) inside the pipeline. The
//! provider delivers it as `PixelFormat::Rgba16` with the sRGB transfer
//! function, as the DNG path does, because that is pandite's pixel
//! contract for 16-bit images; a preview arrives as `PixelFormat::Rgba8`.
//! [`Origin`] reports which one was delivered (R-ARW-64).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-ARW-79); a partly decoded raw image is dropped and
//! never delivered (R-ARW-80).

#![forbid(unsafe_code)]

mod color;
mod ifd;
mod lj92;
mod meta;
mod plane;
mod pool;
mod preview;
mod render;
mod strips;
mod tiles;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Class, Rect, Variant, WbSource};

use crate::util::check_size;
use pandite_core::{CancelToken, DecodeOptions, Error, ImageBuf, Orientation, PixelFormat, Result};
use std::fmt::Display;

/// Results inside the ARW decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type ArwResult<T> = Result<T>;

/// A decoding failure (R-ARW-79: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: arw: {msg}"))
}

/// Which image was delivered (R-ARW-64).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-ARW-53), delivered with
    /// the sRGB transfer function as `Rgba16`.
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
    /// Valid area.
    pub valid: (usize, usize),
    /// Final image area.
    pub crop: Rect,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
    /// Source of the white balance multipliers.
    pub wb: WbSource,
    /// True when the color matrix came from the model table.
    pub matrix_from_table: bool,
}

/// A delivered image and where it came from (R-ARW-64). The provider
/// passes on the image only; its pixel format tells the two origins apart
/// (`Rgba16` developed, `Rgba8` preview). The tests read the report.
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

/// True when this provider should treat the data as ARW: the `.arw`
/// extension, or a little-endian TIFF whose IFD0 has Make "SONY", SubIFDs
/// and SR2Private (all measured ARW and SR2 files, M1; a TIFF exported by
/// other software lacks SR2Private).
pub(crate) fn is_arw(data: &[u8], extension: Option<&str>) -> bool {
    if extension == Some("arw") {
        return true;
    }
    let Ok(Some(first)) = ifd::header(data) else {
        return false;
    };
    let Some(ifd0) = ifd::dir(data, u64::from(first)) else {
        return false;
    };
    matches!(ifd0.ascii(0x010F), Ok(Some(b"SONY"))) && ifd0.has(0x014A) && ifd0.has(0xC634)
}

/// Size and orientation of the image [`decode`] delivers at full size, or
/// `Ok(None)` when the data are not a little-endian TIFF (the caller then
/// uses its generic preview search).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let Some(c) = meta::open(data)? else {
        return Ok(None);
    };
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    if let Ok(Class::Sensor(v)) = meta::classify(&c) {
        if let Ok(plan) = meta::plan(&c, v) {
            let (w, h) = render::output_size(plan.crop, o);
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

/// Decode an ARW file to an upright image, or `Ok(None)` when the data
/// are not a little-endian TIFF.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_report(data, options)?.map(|d| d.image))
}

/// [`decode`], reporting the origin of the delivered image (R-ARW-64).
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    // Steps 1 and 2 of the decoding procedure: header and IFD chain.
    let c = match meta::open(data) {
        Ok(Some(c)) => c,
        // R-ARW-4: a big-endian file (X6) or other data: preview path.
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };
    let o = c.orientation();
    // Step 3: collect the preview candidates first, so every later failure
    // can fall back (R-ARW-59, R-ARW-60).
    let candidates = preview::collect(&c);
    // Steps 4 and 5: raw IFD and classification (R-ARW-3).
    let (class_ok, mut last) = match meta::classify(&c) {
        Ok(k) => (Some(k), None),
        Err(e) => (None, Some(e)),
    };
    // Steps 6 and 7: the plan, with every check made before allocation.
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
            Some(p) => render::output_size(p.crop, o).0 as u64,
            None => candidates
                .first()
                .map_or(0, |p| u64::from(p.displayed(o).0)),
        };
        let fits = |p: &&preview::Candidate| {
            let (w, h) = p.displayed(o);
            w >= tw || h >= th || (full_w > 0 && u64::from(w) * 20 >= full_w * 19)
        };
        if let Some(p) = candidates.iter().filter(fits).min_by_key(|p| p.area()) {
            match preview::decode(data, p, o, options) {
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

    // Steps 8 to 14 for the specified variants.
    if let Some(plan) = &plan {
        match develop(data, plan, &options.cancel) {
            Ok(image) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor(plan.variant),
                    class: class_ok,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        valid: plan.valid,
                        crop: plan.crop,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                        wb: plan.wb_source,
                        matrix_from_table: plan.matrix_from_table,
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-ARW-80: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-ARW-2, R-ARW-63: the preview path, largest candidate first, the
    // next one when a candidate fails; an error only when none decodes.
    for p in &candidates {
        options.cancel.check()?;
        match preview::decode(data, p, o, options) {
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

/// The sensor data of a plan, decoded (step 8).
fn decode_sensor(
    data: &[u8],
    plan: &meta::Plan,
    cancel: &CancelToken,
) -> ArwResult<render::Sensor> {
    Ok(match plan.variant {
        Variant::V1 | Variant::V2 | Variant::V3 => {
            render::Sensor::Cfa(strips::decode(data, plan, cancel)?)
        }
        Variant::V4 => render::Sensor::Cfa(tiles::decode_cfa(data, plan, cancel)?),
        Variant::V5 => render::Sensor::Ycc(tiles::decode_ycc(data, plan, cancel)?),
    })
}

/// Develop the sensor data to an upright 16-bit sRGB image (steps 8 to
/// 14).
fn develop(data: &[u8], plan: &meta::Plan, cancel: &CancelToken) -> ArwResult<ImageBuf> {
    let (w, h) = render::output_size(plan.crop, plan.orientation);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    check_size(w, h, PixelFormat::Rgba16)?;
    let sensor = decode_sensor(data, plan, cancel)?;
    let table = render::srgb_table();
    let bytes = render::render::<8, _>(plan, &sensor, cancel, |rgb| {
        let mut px = [0u8; 8];
        for (k, v) in rgb.iter().enumerate() {
            // v is clipped to 0..1 (R-ARW-52); NaN cannot occur (R-ARW-78).
            let i = ((v * 65535.0 + 0.5) as usize).min(65535);
            px[2 * k..2 * k + 2].copy_from_slice(&table[i].to_ne_bytes());
        }
        px[6..8].copy_from_slice(&u16::MAX.to_ne_bytes());
        px
    })?;
    ImageBuf::new(w, h, PixelFormat::Rgba16, bytes)
}

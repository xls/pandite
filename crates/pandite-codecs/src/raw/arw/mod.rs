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
//! specification's result is available as [`Rendering::Colorimetric`]:
//! `PixelFormat::Rgba16` with the sRGB transfer function (pandite's pixel
//! contract for 16-bit images). The provider delivers
//! [`Rendering::Display`]: pandite's default display rendering
//! ([`crate::raw::tone`], not part of the specification) applied to that
//! linear result, as `Rgba16`, or as `Rgba8` in one pass when the caller
//! prefers 8 bits. A preview arrives as `Rgba8`. [`Developed::origin`]
//! reports which one was delivered (R-ARW-64); the pixel format alone no
//! longer tells them apart when 8 bits are preferred.
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
pub(crate) mod preview;
mod render;
mod strips;
mod tiles;

#[cfg(test)]
pub(crate) mod fixtures;
#[cfg(test)]
pub(crate) mod tests;

pub(crate) use meta::{Class, Rect, Variant, WbSource};

use crate::pool;
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{
    CancelToken, DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result,
};
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
    name: "Sony ARW",
    detect: is_arw,
    has_preview,
    probe,
    decode,
    facts,
};

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

/// True when the file carries an embedded preview (R-ARW-59, R-ARW-60):
/// a reduced decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(Some(c)) if !preview::collect(&c).is_empty())
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

/// Header facts of an ARW file: the storage of the sensor data (from the
/// classification, R-ARW-3), the CFA layout and sample depth of the raw
/// IFD, and the camera settings of IFD0 and its EXIF IFD. Reads tags only;
/// no sensor data. `None` when the data are not a little-endian TIFF.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok().flatten()?;
    let mut f = ImageFacts::named("Sony ARW");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    let Some(raw) = &c.raw else {
        return Some(f);
    };
    let compression = raw.uint(0x0103).ok().flatten();
    f.compression = match meta::classify(&c).ok() {
        Some(Class::Sensor(Variant::V1)) => Some("Uncompressed"),
        Some(Class::Sensor(Variant::V2)) => Some("Sony 12-bit packed"),
        Some(Class::Sensor(Variant::V3)) => Some("Sony lossy, 8 bits per pixel"),
        Some(Class::Sensor(Variant::V4)) => Some("Lossless JPEG (ARW)"),
        Some(Class::Sensor(Variant::V5)) => Some("Lossless JPEG YCbCr (ARW)"),
        _ => match compression {
            Some(1) => Some("Uncompressed"),
            Some(7) => Some("Lossless JPEG (ARW)"),
            Some(32767) => Some("Sony compressed"),
            _ => None,
        },
    }
    .map(str::to_string);
    let bits = raw
        .uint(0x0102)
        .ok()
        .flatten()
        .and_then(|b| u8::try_from(b).ok())
        .filter(|&b| b > 0);
    match raw.uint(0x0106).ok().flatten() {
        Some(32803) => {
            let names = ["R", "G", "B"];
            let pattern: Option<String> = raw
                .uints(0x828E)
                .ok()
                .flatten()
                .filter(|p| p.len() == 4)
                .and_then(|p| p.iter().map(|&c| names.get(c as usize).copied()).collect());
            f.channels = Some(match pattern {
                Some(p) => format!("Bayer {p}"),
                None => "Bayer".to_string(),
            });
            f.channel_count = Some(1);
            f.bits_per_channel = bits;
        }
        Some(6) => {
            f.channels = Some("YCbCr".to_string());
            f.channel_count = Some(3);
            f.bits_per_channel = bits;
            if let Some(s) = raw.uints(0x0212).ok().flatten().filter(|s| s.len() >= 2) {
                f.subsampling = u8::try_from(s[0])
                    .ok()
                    .zip(u8::try_from(s[1]).ok())
                    .and_then(|(h, v)| crate::facts::subsampling(h, v))
                    .map(str::to_string);
            }
        }
        _ => f.bits_per_channel = bits,
    }
    Some(f)
}

/// How developed sensor data are delivered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rendering {
    /// The colorimetric result of the specification: linear sRGB
    /// (R-ARW-53) with the sRGB transfer function, `Rgba16`.
    Colorimetric,
    /// pandite's default display rendering ([`crate::raw::tone`]) on top
    /// of the colorimetric result: `Rgba8` when the caller prefers 8 bits,
    /// else `Rgba16`.
    Display,
}

/// Decode an ARW file to an upright image with the default display
/// rendering, or `Ok(None)` when the data are not a little-endian TIFF.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-ARW-64).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-ARW-64).
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
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

/// The pixel encoding of a developed image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Output {
    /// Colorimetric linear sRGB with the sRGB transfer, 16 bits.
    Colorimetric16,
    /// Default display rendering, 16 bits.
    Display16,
    /// Default display rendering, 8 bits.
    Display8,
}

/// Linear sRGB to 16-bit sRGB codes, colorimetric.
struct Colorimetric16;

impl render::RowEncoder for Colorimetric16 {
    fn pixel_bytes(&self) -> usize {
        8
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        let table = render::srgb_table();
        for (k, px) in out.chunks_exact_mut(8).enumerate() {
            let v = render::to_srgb(m, [cam[0][k], cam[1][k], cam[2][k]]);
            for k in 0..3 {
                // v is clipped to 0..1 (R-ARW-52); NaN cannot occur (R-ARW-78).
                let i = ((v[k] * 65535.0 + 0.5) as usize).min(65535);
                px[2 * k..2 * k + 2].copy_from_slice(&table[i].to_ne_bytes());
            }
            px[6..8].copy_from_slice(&u16::MAX.to_ne_bytes());
        }
    }
}

/// The default display rendering to 16-bit sRGB codes.
struct Display16(&'static Tone);

impl render::RowEncoder for Display16 {
    fn pixel_bytes(&self) -> usize {
        8
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        let table = render::srgb_table();
        for (k, px) in out.chunks_exact_mut(8).enumerate() {
            let d = self
                .0
                .apply(render::to_srgb(m, [cam[0][k], cam[1][k], cam[2][k]]));
            for k in 0..3 {
                let i = ((d[k] * 65535.0 + 0.5) as usize).min(65535);
                px[2 * k..2 * k + 2].copy_from_slice(&table[i].to_ne_bytes());
            }
            px[6..8].copy_from_slice(&u16::MAX.to_ne_bytes());
        }
    }
}

/// The default display rendering to 8-bit sRGB in one pass.
struct Display8(&'static Tone);

impl render::RowEncoder for Display8 {
    fn pixel_bytes(&self) -> usize {
        4
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        crate::simd::matrix_tone_rgba8(m, self.0, cam, out);
    }
}

/// Develop the sensor data to an upright sRGB image (steps 8 to 14) in
/// the requested encoding. Full precision is kept until the last step.
fn develop(
    data: &[u8],
    plan: &meta::Plan,
    cancel: &CancelToken,
    output: Output,
) -> ArwResult<ImageBuf> {
    let (w, h) = render::output_size(plan.crop, plan.orientation);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    let sensor = decode_sensor(data, plan, cancel)?;
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => render::render(plan, &sensor, cancel, &Colorimetric16)?,
        Output::Display16 => render::render(plan, &sensor, cancel, &Display16(tone))?,
        Output::Display8 => render::render(plan, &sensor, cancel, &Display8(tone))?,
    };
    ImageBuf::new(w, h, format, bytes)
}

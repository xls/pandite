//! Canon CR2 decoder, written from the clean-room specification
//! `specs/raw/cr2.md` and the standards it cites (TIFF 6.0, ITU-T T.81).
//!
//! Variant V1 (RAW: one lossless JPEG stream of 2 or 4 components in frame
//! layouts A, B and C) is developed from sensor data (R-CR2-1). sRAW and
//! mRAW (X1), any other stream (X2) and every file that fails a check show
//! the embedded JPEG preview (R-CR2-2, R-CR2-41). Data that are not a CR2
//! file (X3) go back to the caller, which reports them through its generic
//! path.
//!
//! The decoder calls the shared parts of the Sony decoder: TIFF IFD
//! reading, the T.81 lossless JPEG decoder (with the T.81 line-start rule
//! for every line), the rendering (normalization, bilinear demosaicing,
//! color matrix, orientation, display tone) and the preview decoder.
//!
//! The developed image is linear sRGB (R-CR2-33) inside the pipeline. The
//! provider delivers pandite's default display rendering of it
//! ([`crate::raw::tone`], as for ARW), as `Rgba16`, or as `Rgba8` when the
//! caller prefers 8 bits. A preview arrives as `Rgba8`.
//! [`Developed::origin`] reports which one was delivered (R-CR2-42).
//!
//! Failures inside the decoder are recoverable `Error` values, never
//! process aborts (R-CR2-52); a partly decoded raw image is dropped and
//! never delivered (R-CR2-53).

#![forbid(unsafe_code)]

mod color;
mod meta;
mod preview;
mod sensor;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use meta::{Area, Class, Layout};
pub(crate) use sensor::Levels;

use crate::raw::arw::{self, meta as arw_meta, render, Output, Rendering, WbSource};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{
    CancelToken, DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, PixelFormat, Result,
};
use std::fmt::Display;

/// Results inside the CR2 decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type Cr2Result<T> = Result<T>;

/// A decoding failure (R-CR2-52: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: cr2: {msg}"))
}

/// Which image was delivered (R-CR2-42).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The developed sensor data: linear sRGB (R-CR2-33).
    Sensor,
    /// An embedded preview: sRGB-encoded JPEG values, not linear.
    Preview,
}

/// Facts about the raw data of a developed file. Read by the tests.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Info {
    /// Raster W x H.
    pub raster: (usize, usize),
    /// The visible area.
    pub visible: Area,
    /// Frame X, Y, Nf.
    pub frame: (usize, usize, usize),
    /// Sample precision P.
    pub precision: u32,
    /// Frame layout A, B or C.
    pub layout: Option<Layout>,
    /// Tag 0xC640 n, a, b when n >= 1.
    pub slices: Option<(usize, usize, usize)>,
    /// Tag 0xC5E0.
    pub cfa_value: u32,
    /// Black and white levels.
    pub levels: Levels,
    /// WB multipliers R, G, B.
    pub wb: [f64; 3],
    /// Source of the white balance multipliers.
    pub wb_source: WbSource,
    /// True when the color matrix came from the model table.
    pub matrix_from_table: bool,
    /// IFD0 orientation 1 to 8.
    pub orientation: u8,
    /// IFD0 Model.
    pub model: String,
}

/// A delivered image and where it came from (R-CR2-42). The provider
/// passes on the image only; the tests read the report.
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
    name: "Canon CR2",
    detect: is_cr2,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this provider should treat the data as CR2: the `.cr2`
/// extension, or the CR2 signature of R-CR2-4 (49 49 2A 00 in bytes 0 to
/// 3, 43 52 02 in bytes 8 to 10).
pub(crate) fn is_cr2(data: &[u8], extension: Option<&str>) -> bool {
    extension == Some("cr2")
        || (data.get(..4) == Some(&[0x49, 0x49, 0x2A, 0x00][..])
            && data.get(8..11) == Some(&[0x43, 0x52, 0x02][..]))
}

/// True when the file carries an embedded preview (R-CR2-37): a reduced
/// decode is then cheap.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    matches!(meta::open(data), Ok(Some(c)) if !preview::collect(&c).is_empty())
}

/// The visible area as a crop rectangle of the visible-area plane.
fn crop_of(plan: &meta::Plan) -> arw::Rect {
    arw::Rect {
        x: 0,
        y: 0,
        w: plan.visible.width(),
        h: plan.visible.height(),
    }
}

/// Size and orientation of the image [`decode`] delivers at full size, or
/// `Ok(None)` when the data are not a CR2 file (X3).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let Some(c) = meta::open(data)? else {
        return Ok(None);
    };
    let o = c.orientation();
    let orientation = Orientation::from_exif(u16::from(o));
    if meta::classify(&c) == Class::V1 {
        if let Ok(plan) = meta::plan(&c) {
            let (w, h) = render::output_size(crop_of(&plan), o);
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

/// The CFA arrangement of tag 0xC5E0 as letters, R-CR2-19.
fn pattern_name(value: u32) -> Option<&'static str> {
    match value {
        1 => Some("RGGB"),
        2 => Some("BGGR"),
        3 => Some("GBRG"),
        4 => Some("GRBG"),
        _ => None,
    }
}

/// Header facts of a CR2 file: the storage of the sensor data (from the
/// classification, R-CR2-3, and the LJ92 frame header), the CFA layout and
/// sample precision, and the camera settings of IFD0 and its Exif IFD.
/// Reads tags and stream headers only; no sensor data. `None` when the
/// data are not a CR2 file.
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let c = meta::open(data).ok().flatten()?;
    let mut f = ImageFacts::named("Canon CR2");
    if let Some(t) = crate::exif::Tiff::new(data) {
        f.camera = crate::facts::camera(&t);
    }
    // The stream headers come before the entropy data: a file head is
    // enough.
    let Some(s) = c.stream_head() else {
        return Some(f);
    };
    let class = meta::classify_with(&c, Some(&s));
    let bits = u8::try_from(s.precision).ok().filter(|&b| b > 0);
    match class {
        Class::V1 => {
            f.compression = Some("Lossless JPEG".to_string());
            let value = c
                .raw
                .as_ref()
                .and_then(|r| r.uint(meta::tag::CFA_PATTERN).ok().flatten())
                .unwrap_or(1);
            f.channels = Some(match pattern_name(value) {
                Some(p) => format!("Bayer {p}"),
                None => "Bayer".to_string(),
            });
            f.channel_count = Some(1);
            f.bits_per_channel = bits;
        }
        Class::X1 => {
            f.compression = Some("Lossless JPEG (sRAW)".to_string());
            if s.components.len() == 3 {
                f.channels = Some("YCbCr".to_string());
                f.channel_count = Some(3);
                f.bits_per_channel = bits;
                let luma = s.components[0];
                f.subsampling = u8::try_from(luma.h)
                    .ok()
                    .zip(u8::try_from(luma.v).ok())
                    .and_then(|(h, v)| crate::facts::subsampling(h, v))
                    .map(str::to_string);
            }
        }
        Class::X2 => {
            f.compression = Some("Lossless JPEG".to_string());
            f.bits_per_channel = bits;
        }
    }
    Some(f)
}

/// Decode a CR2 file to an upright image with the default display
/// rendering, or `Ok(None)` when the data are not a CR2 file (X3).
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    Ok(decode_with(data, options, Rendering::Display)?.map(|d| d.image))
}

/// The colorimetric decode of the specification, reporting the origin of
/// the delivered image (R-CR2-42).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    decode_with(data, options, Rendering::Colorimetric)
}

/// Decode with the given rendering of the sensor data, reporting the
/// origin of the delivered image (R-CR2-42).
pub(crate) fn decode_with(
    data: &[u8],
    options: &DecodeOptions,
    rendering: Rendering,
) -> Result<Option<Developed>> {
    // Steps 1 and 2 of the decoding procedure: size, header, IFDs.
    let c = match meta::open(data) {
        Ok(Some(c)) => c,
        // R-CR2-2: X3 is not a CR2 file; the caller reports it.
        Ok(None) => return Ok(None),
        Err(e) => return Err(e),
    };
    let o = c.orientation();
    // Step 3: the preview candidates first, so every later failure can fall
    // back (R-CR2-37, R-CR2-38).
    let candidates = preview::collect(&c);
    // Steps 4 and 5: raw IFD and classification (R-CR2-3). X1 and X2 use
    // the preview path without reading their sensor data (R-CR2-2).
    let class = meta::classify(&c);
    let mut last = None;
    // Steps 6 and 7: the plan, with every check made before allocation.
    let plan = match class {
        Class::V1 => match meta::plan(&c) {
            Ok(p) => Some(p),
            Err(e) => {
                last = Some(e);
                None
            }
        },
        Class::X1 | Class::X2 => None,
    };

    // A preview is enough when the caller wants a smaller image.
    if let Some((tw, th)) = options.target_size {
        let full_w = match &plan {
            Some(p) => render::output_size(crop_of(p), o).0 as u64,
            None => candidates
                .first()
                .map_or(0, |p| u64::from(preview::displayed(p, o).0)),
        };
        let fits = |p: &&arw::preview::Candidate| {
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

    // Steps 8 to 14 for V1.
    if let Some(plan) = &plan {
        let output = match rendering {
            Rendering::Colorimetric => Output::Colorimetric16,
            Rendering::Display if options.prefer_8bit => Output::Display8,
            Rendering::Display => Output::Display16,
        };
        match develop(data, plan, &options.cancel, output) {
            Ok((image, levels)) => {
                return Ok(Some(Developed {
                    image,
                    origin: Origin::Sensor,
                    class,
                    info: Some(Info {
                        raster: (plan.width, plan.height),
                        visible: plan.visible,
                        frame: plan.frame,
                        precision: plan.precision,
                        layout: plan.layout,
                        slices: plan.slices,
                        cfa_value: plan.cfa_value,
                        levels,
                        wb: plan.wb,
                        wb_source: plan.wb_source,
                        matrix_from_table: plan.matrix_from_table,
                        orientation: plan.orientation,
                        model: plan.model.clone(),
                    }),
                }))
            }
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // R-CR2-53: the partly decoded image was dropped with the error.
            Err(e) => last = Some(e),
        }
    }

    // R-CR2-41: the preview path, largest candidate first, the next one
    // when a candidate fails; an error only when none decodes.
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

/// The shared rendering's plan for the visible area of a V1 file: a CFA
/// plane of the visible area alone, so the bilinear demosaicing sees only
/// neighbors inside it (R-CR2-34), with the CFA colors and black levels
/// turned to the phase of the visible area origin (R-CR2-35). The fields
/// the rendering does not read for CFA data (tone curve, YCbCr parameters,
/// strip layout) hold neutral values.
fn shared_plan(plan: &meta::Plan, levels: Levels) -> arw_meta::Plan {
    let crop = crop_of(plan);
    let v = plan.visible;
    arw_meta::Plan {
        variant: arw::Variant::V1,
        width: crop.w,
        height: crop.h,
        valid: (crop.w, crop.h),
        crop,
        orientation: plan.orientation,
        cfa: sensor::phase(plan.cfa, v),
        white: levels.white,
        black: sensor::phase(levels.black, v),
        wb: plan.wb,
        wb_source: plan.wb_source,
        matrix: plan.matrix,
        matrix_from_table: plan.matrix_from_table,
        curve: [0; 4],
        ycc: arw_meta::Ycc {
            luma: [0.299, 0.587, 0.114],
            zero: [16384.0; 2],
            factors: [1.0; 3],
            black: 0.0,
            subsampling: None,
        },
        layout: arw_meta::Layout::Strips {
            offsets: Vec::new(),
            rows_per_strip: 0,
            row_bytes: 0,
        },
        model: plan.model.clone(),
    }
}

/// Develop a V1 file (steps 8 to 13) to an upright image in the requested
/// encoding, with the levels it measured.
fn develop(
    data: &[u8],
    plan: &meta::Plan,
    cancel: &CancelToken,
    output: Output,
) -> Cr2Result<(ImageBuf, Levels)> {
    cancel.check()?;
    let (w, h) = render::output_size(crop_of(plan), plan.orientation);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    // Step 8: decode and reassemble the raster.
    let raster = sensor::decode(data, plan, cancel)?;
    let tone = Tone::unit();
    let (bytes, levels) = match output {
        Output::Colorimetric16 => develop_raster(raster, plan, cancel, &arw::Colorimetric16)?,
        Output::Display16 => develop_raster(raster, plan, cancel, &arw::Display16(tone))?,
        Output::Display8 => develop_raster(raster, plan, cancel, &arw::Display8(tone))?,
    };
    Ok((ImageBuf::new(w, h, format, bytes)?, levels))
}

/// Steps 9 to 13 on a decoded raster, through `encoder`: the bytes of the
/// upright image and the levels.
fn develop_raster(
    raster: sensor::Raster,
    plan: &meta::Plan,
    cancel: &CancelToken,
    encoder: &dyn render::RowEncoder,
) -> Cr2Result<(Vec<u8>, Levels)> {
    // Step 9: black levels (R-CR2-24, R-CR2-25) and white level (R-CR2-26).
    // The WB multipliers are in the plan (R-CR2-27, R-CR2-28).
    let black = sensor::black_levels(&raster, plan.black_area, cancel)?;
    let white = sensor::white_level(&raster, plan, black)?;
    let levels = Levels { black, white };
    // Steps 10 to 13 by the shared rendering: R-CR2-29 normalization (each
    // CFA position's black level and the multiplier of its color, clipped
    // to 0 to 1), R-CR2-34 bilinear demosaicing inside the visible area
    // (R-ARW-54, R-ARW-55), R-CR2-32 matrix and clip to linear sRGB
    // (R-CR2-33), R-CR2-36 orientation (R-ARW-58).
    let shared = shared_plan(plan, levels);
    let sensor = render::Sensor::Cfa(sensor::visible_plane(raster, plan.visible)?);
    let bytes = render::render(&shared, &sensor, cancel, encoder)?;
    Ok((bytes, levels))
}

//! Canon CR3 decoder, written from the clean-room specification
//! `specs/raw/cr3.md` and the standards it cites (QuickTime File Format,
//! TIFF 6.0, ITU-T T.81, JFIF).
//!
//! The specification has no in-scope sensor-data variant: the Canon CRX
//! codec of the sensor data (X1 lossless, X2 lossy, X3 Dual Pixel RAW) is
//! not specified, and neither are HEVC previews (X5). The decoder never
//! reads a CRX or HEVC sample (R-CR3-1, R-CR3-28). It reads the container,
//! the metadata and the orientation, and delivers the largest embedded JPEG
//! preview, upright, for every CR3 file that has one (R-CR3-1, R-CR3-37).
//! A file without a JPEG preview gets the error "no supported preview"
//! with its variant (R-CR3-37). CTBO, CCTP, CNCV and the XMP box are never
//! used to locate data (R-CR3-21).
//!
//! Failures are recoverable `Error` values, never process aborts, and no
//! read leaves its buffer (R-CR3-53).

#![forbid(unsafe_code)]

mod boxes;
mod meta;
mod preview;

#[cfg(test)]
mod fixtures;
#[cfg(test)]
mod tests;

pub(crate) use boxes::Variant;
pub(crate) use meta::Meta;
pub(crate) use preview::PreviewSource;

use boxes::Container;
use pandite_core::{DecodeOptions, Error, ImageBuf, ImageFacts, Orientation, Result};
use std::fmt::Display;

/// Results inside the CR3 decoder; every error means "fail" in the sense
/// of the specification's Error handling section.
pub(crate) type Cr3Result<T> = Result<T>;

/// A decoding failure (R-CR3-53: a recoverable error value).
pub(crate) fn fail(msg: impl Display) -> Error {
    Error::Corrupt(format!("raw: cr3: {msg}"))
}

/// The error text of R-CR3-37.
pub(crate) const NO_PREVIEW: &str = "no supported preview";

/// A delivered image and its report (R-CR3-41). Every delivered image is
/// an embedded preview: sRGB-encoded JPEG values, never decoded sensor
/// data. The provider passes on the image only; the tests read the report.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Developed {
    /// The image, upright.
    pub image: ImageBuf,
    /// The preview it came from.
    pub source: PreviewSource,
    /// The stored size of that preview (from its frame header).
    pub stored: (u32, u32),
    /// The CMT2 ColorSpace value, 1 when absent (R-CR3-35).
    pub color_space: u16,
    /// The variant of the file (R-CR3-2).
    pub variant: Variant,
    /// True for Dual Pixel RAW (R-CR3-2, X3).
    pub dual_pixel: bool,
    /// The metadata (R-CR3-31 to R-CR3-34, R-CR3-38).
    pub meta: Meta,
}

/// This format's entry in the developer table of [`crate::raw`].
pub(crate) const DEVELOPER: crate::raw::Developer = crate::raw::Developer {
    name: "Canon CR3",
    detect,
    has_preview,
    probe,
    decode,
    facts,
};

/// True when this module reads the data: the `.cr3` extension, or the file
/// type box of R-CR3-4.
pub(crate) fn detect(data: &[u8], extension: Option<&str>) -> bool {
    extension == Some("cr3") || boxes::is_cr3(data)
}

/// R-CR3-50: a file whose "moov" is missing and whose preview holder box
/// holds no valid PRVW candidate fails.
fn readable(c: &Container<'_>, candidates: &[preview::Candidate]) -> bool {
    c.moov.is_some() || candidates.iter().any(|p| p.source == PreviewSource::P2)
}

/// The container and its candidates when this module handles the file.
/// `Ok(None)` for data that are not a CR3 file (R-CR3-4) and for a CR3
/// file that fails R-CR3-50: the provider then uses its generic preview
/// search, as it did for every CR3 file before this module.
fn handled(data: &[u8]) -> Result<Option<(Container<'_>, Vec<preview::Candidate>)>> {
    let Some(c) = boxes::open(data)? else {
        return Ok(None);
    };
    let candidates = preview::collect(&c);
    if !readable(&c, &candidates) {
        return Ok(None);
    }
    Ok(Some((c, candidates)))
}

/// True when the file carries a JPEG preview: a reduced decode is then
/// cheap. For a file this module declines, the provider's generic search
/// answers.
pub(crate) fn has_preview(data: &[u8]) -> bool {
    match handled(data) {
        Ok(Some((_, candidates))) => !candidates.is_empty(),
        Ok(None) => !crate::raw::previews(data).is_empty(),
        Err(_) => false,
    }
}

/// Size and orientation of the image [`decode`] delivers at full size:
/// the largest candidate, oriented (R-CR3-37 to R-CR3-39).
pub(crate) fn probe(data: &[u8]) -> Result<Option<(u32, u32, Orientation)>> {
    let Some((c, candidates)) = handled(data)? else {
        return Ok(None);
    };
    let o = meta::orientation(&c);
    let best = candidates.first().ok_or_else(|| no_preview(&c))?;
    let (w, h) = best.displayed(o);
    Ok(Some((w, h, Orientation::from_exif(u16::from(o)))))
}

/// The R-CR3-37 error, naming the variant.
fn no_preview(c: &Container<'_>) -> Error {
    fail(format!("{NO_PREVIEW} (variant {:?})", c.variant()))
}

/// Header facts: the container, the coding of the sensor data by CMP1
/// byte 26 of the main CRX track and Dual Pixel RAW (R-CR3-2), the color
/// space of the previews (R-CR3-35), and the camera of CMT1. The
/// specification gives no bit depth or color filter layout (OQ-8, OQ-9).
pub(crate) fn facts(data: &[u8]) -> Option<ImageFacts> {
    let (c, _) = handled(data).ok().flatten()?;
    let mut f = ImageFacts::named("Canon CR3");
    if let Some(b) = meta::cmt(&c, b"CMT1") {
        f.camera = crate::facts::camera(&b.tiff);
    }
    let coding = match c.crx_mode() {
        Some(Variant::X1) => Some("Canon CRX lossless (RAW)"),
        Some(_) => Some("Canon CRX lossy (C-RAW)"),
        None => None,
    };
    f.compression = coding.map(|s| {
        if c.dual_pixel() {
            format!("{s}, Dual Pixel RAW")
        } else {
            s.to_string()
        }
    });
    // S9: 1 is sRGB, 0xFFFF is Uncalibrated (OQ-11).
    f.color_space = match meta::color_space(&c) {
        1 => Some("sRGB".to_string()),
        0xFFFF => Some("Uncalibrated".to_string()),
        _ => None,
    };
    Some(f)
}

/// Decode a CR3 file to its upright preview, or `Ok(None)` when the
/// module declines the file (see [`handled`]).
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Option<ImageBuf>> {
    let Some((c, candidates)) = handled(data)? else {
        return Ok(None);
    };
    Ok(Some(deliver(&c, &candidates, options)?.image))
}

/// The decoding procedure of the specification with its report. Fails for
/// a file shorter than 24 bytes (R-CR3-49) and for a file without "moov"
/// and without a PRVW candidate (R-CR3-50); `Ok(None)` for a file that is
/// not a CR3 file (R-CR3-4).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn decode_report(data: &[u8], options: &DecodeOptions) -> Result<Option<Developed>> {
    let Some(c) = boxes::open(data)? else {
        return Ok(None);
    };
    let candidates = preview::collect(&c);
    if !readable(&c, &candidates) {
        return Err(fail("no \"moov\" box and no PRVW preview"));
    }
    deliver(&c, &candidates, options).map(Some)
}

/// Steps 5 to 10 of the decoding procedure.
fn deliver(
    c: &Container<'_>,
    candidates: &[preview::Candidate],
    options: &DecodeOptions,
) -> Result<Developed> {
    options.cancel.check()?;
    let m = meta::read(c);
    let o = m.orientation;
    let report = |image: ImageBuf, p: &preview::Candidate| Developed {
        image,
        source: p.source,
        stored: (p.stream.width, p.stream.height),
        color_space: m.color_space,
        variant: c.variant(),
        dual_pixel: c.dual_pixel(),
        meta: m.clone(),
    };
    // R-CR3-37: no candidate left.
    if candidates.is_empty() {
        return Err(no_preview(c));
    }

    // A smaller preview is enough when the caller wants a smaller image
    // (pandite's reduced decode, not part of the specification): the
    // smallest candidate that covers the target. The thumbnail (P3) is
    // left out (OQ-16: delivered only when no larger candidate decodes).
    if let Some((tw, th)) = options.target_size {
        let full_w = u64::from(candidates[0].displayed(o).0);
        let fits = |p: &&preview::Candidate| {
            let (w, h) = p.displayed(o);
            p.source != PreviewSource::P3
                && (w >= tw || h >= th || u64::from(w) * 20 >= full_w * 19)
        };
        if let Some(p) = candidates.iter().filter(fits).min_by_key(|p| p.area()) {
            match preview::decode(c.data, p, o, options) {
                Ok(image) => return Ok(report(image, p)),
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
    }

    // Steps 8 to 10: the largest candidate first, the next one when a
    // candidate fails, an error only when none decodes (R-CR3-40). A
    // failed decode is dropped with its error (R-CR3-54).
    let mut last = None;
    for p in candidates {
        options.cancel.check()?;
        match preview::decode(c.data, p, o, options) {
            Ok(image) => return Ok(report(image, p)),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| no_preview(c)))
}

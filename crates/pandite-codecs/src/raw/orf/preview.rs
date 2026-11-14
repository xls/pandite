//! The embedded JPEG previews of an ORF file (section Embedded preview).
//! Candidates are collected here. Decoding uses the Sony decoder's preview
//! code (which accepts the streams of R-ORF-51, as R-ARW-61 asks), without
//! its orientation exception: R-ORF-52 rotates every preview.

use super::meta::{tag, Container};
use super::OrfResult;
use crate::raw::arw::preview::{self, frame_size, Candidate};
use crate::{jpeg, orient};
use pandite_core::{DecodeOptions, ImageBuf, Orientation};

/// R-ORF-48 to R-ORF-50: the preview candidates, largest first, the
/// earlier of the R-ORF-48 order first among equals.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    let mut consider = |offset: u64, len: u64| {
        // R-ORF-49: offset + length inside the file (64-bit sum, R-ORF-55),
        // length at least 4, first bytes FF D8.
        if offset + len > size || len < 4 {
            return;
        }
        let (offset, len) = (offset as usize, len as usize);
        let stream = &c.data[offset..offset + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == offset) {
            return;
        }
        // R-ORF-50: the size from the first SOF0, SOF1 or SOF2 segment,
        // found by walking the marker segments; a stream without one is
        // dropped. PreviewImageValid is not read.
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset,
                len,
                width,
                height,
            });
        }
    };
    // R-ORF-48, in this order: the CameraSettings pair with the offset
    // origin of the note style (R-ORF-12) ...
    if let Some(note) = &c.note {
        if let Some(cs) = &note.camera_settings {
            if let (Ok(Some(start)), Ok(Some(len))) = (
                cs.uint(tag::PREVIEW_IMAGE_START),
                cs.uint(tag::PREVIEW_IMAGE_LENGTH),
            ) {
                consider(note.origin + u64::from(start), u64::from(len));
            }
        }
        // ... the main maker-note tag 0x0100 (offset = its value offset,
        // length = its count) ...
        if let Some(e) = note
            .main
            .as_ref()
            .and_then(|m| m.entry(tag::THUMBNAIL_IMAGE))
        {
            consider(e.at, u64::from(e.count));
        }
    }
    // ... and the pairs (0x0201, 0x0202) of every IFD of the chain, with
    // file offsets.
    for d in &c.chain {
        if let (Ok(Some(off)), Ok(Some(len))) = (d.uint(tag::JPEG_OFFSET), d.uint(tag::JPEG_LENGTH))
        {
            consider(u64::from(off), u64::from(len));
        }
    }
    // R-ORF-50: the largest width * height first; the sort is stable, so
    // equal sizes keep the R-ORF-48 order.
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// The size of a candidate as delivered: R-ORF-52 applies orientations 5
/// to 8 to every preview, so they swap its axes.
pub(super) fn displayed(p: &Candidate, orientation: u8) -> (u32, u32) {
    if orientation >= 5 {
        (p.height, p.width)
    } else {
        (p.width, p.height)
    }
}

/// Decode one candidate and apply the IFD0 orientation (R-ORF-52, all
/// values 1 to 8). A stream with bytes after its EOI marker, or without
/// an EOI marker after its last scan, is accepted (R-ORF-51).
pub(super) fn decode(
    data: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> OrfResult<ImageBuf> {
    let o = Orientation::from_exif(u16::from(orientation));
    // The target is in display orientation; the JPEG decoder scales the
    // stored axes.
    let opts = DecodeOptions {
        target_size: options.target_size.map(|(w, h)| jpeg::oriented(w, h, o)),
        first_frame_only: true,
        prefer_8bit: true,
        cancel: options.cancel.clone(),
    };
    // The Sony code with orientation 1 decodes the stream unrotated.
    let img = preview::decode(data, p, 1, &opts)?;
    // The previews carry no orientation of their own (M19); one that does
    // was already turned upright by the JPEG decoder, as in the Sony code.
    let own = data
        .get(p.offset..p.offset + p.len)
        .and_then(jpeg::scan_header)
        .and_then(|h| h.exif)
        .map(crate::exif::orientation_from_exif)
        .unwrap_or_default();
    if own == Orientation::Normal {
        orient::apply(img, o)
    } else {
        Ok(img)
    }
}

//! The embedded JPEG previews of a CR3 file (section Embedded preview).
//!
//! The candidates are collected here; the frame size, the decoding, the
//! acceptance of streams with bytes after EOI or without EOI, and the
//! orientation with its exception for tall previews are the shared preview
//! code of the Sony decoder ([`crate::raw::arw::preview`]), whose rules
//! R-ARW-60 to R-ARW-62 are the same as R-CR3-37, R-CR3-38, R-CR3-39 and
//! R-CR3-42 of this document.

use super::boxes::{be32, range, Bx, Container, Role};
use super::Cr3Result;
use crate::raw::arw::preview as shared;
use pandite_core::{DecodeOptions, ImageBuf};

/// R-CR3-46: longest candidate stream (256 MiB).
pub(crate) const MAX_STREAM: u64 = 1 << 28;
/// R-CR3-46: largest X * Y of a candidate.
pub(crate) const MAX_AREA: u64 = 1 << 28;

/// Where a preview comes from (section Embedded preview, R-CR3-41).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PreviewSource {
    /// The first sample of a JPEG track: the full-size preview.
    P1,
    /// The PRVW box of the preview holder box.
    P2,
    /// The THMB box of the Canon metadata box.
    P3,
}

/// One JPEG candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// Offset, length and stored size of the stream.
    pub stream: shared::Candidate,
    /// Where it comes from.
    pub source: PreviewSource,
}

impl Candidate {
    /// X * Y of the frame header.
    pub(crate) fn area(&self) -> u64 {
        self.stream.area()
    }

    /// The size as delivered for an orientation (R-CR3-38, R-CR3-39).
    pub(crate) fn displayed(&self, orientation: u8) -> (u32, u32) {
        self.stream.displayed(orientation)
    }
}

/// Keep a stream as a candidate when it lies inside the file, is at least
/// 4 bytes long and starts with FF D8 (R-CR3-36), is at most 256 MiB long,
/// and has a frame size of at most 2^28 pixels (R-CR3-46). The size comes
/// from the first SOF0, SOF1 or SOF2 segment (R-CR3-37); a stream without
/// one fails (R-CR3-51). X and Y are 16-bit fields, so they never exceed
/// 65535, and a 0 gives no size.
fn push(out: &mut Vec<Candidate>, d: &[u8], offset: u64, len: u64, source: PreviewSource) {
    if !(4..=MAX_STREAM).contains(&len) {
        return;
    }
    let Some(stream) = offset
        .checked_add(len)
        .and_then(|end| range(d, offset, end))
    else {
        return;
    };
    if stream[..2] != [0xFF, 0xD8] {
        return;
    }
    let Some((width, height)) = shared::frame_size(stream) else {
        return;
    };
    if u64::from(width) * u64::from(height) > MAX_AREA {
        return;
    }
    out.push(Candidate {
        stream: shared::Candidate {
            offset: offset as usize,
            len: len as usize,
            width,
            height,
        },
        source,
    });
}

/// R-CR3-19 and R-CR3-20: the JPEG stream of a THMB box (length at payload
/// offset 8) or PRVW box (length at payload offset 12), at payload offset
/// 16, when the version byte is 0 and 16 + L lies inside the payload.
pub(crate) fn boxed_stream(d: &[u8], b: &Bx, length_at: u64) -> Option<(u64, u64)> {
    if b.byte(d, 0)? != 0 || b.payload_len() < 16 {
        return None;
    }
    let len = u64::from(be32(d, b.body + length_at)?);
    if 16u64.checked_add(len)? > b.payload_len() {
        return None;
    }
    Some((b.body + 16, len))
}

/// R-CR3-36: the candidates in collection order (the first sample of each
/// JPEG track in "moov" order, each PRVW box, the THMB box), then ordered
/// by decreasing X * Y with the earlier candidate first on a tie
/// (R-CR3-37, R-CR3-40: a stable sort).
pub(crate) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let d = c.data;
    let mut out = Vec::new();
    for t in c.tracks.iter().filter(|t| t.role == Role::Jpeg) {
        if let Some((off, len)) = t.sample {
            push(&mut out, d, off, len, PreviewSource::P1);
        }
    }
    for b in &c.prvw {
        if let Some((off, len)) = boxed_stream(d, b, 12) {
            push(&mut out, d, off, len, PreviewSource::P2);
        }
    }
    if let Some(b) = c.canon_box(b"THMB") {
        if let Some((off, len)) = boxed_stream(d, &b, 8) {
            push(&mut out, d, off, len, PreviewSource::P3);
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// Decode one candidate to upright 8-bit sRGB (R-CR3-29, R-CR3-30,
/// R-CR3-35): the shared JPEG decoder converts YCbCr with the JFIF
/// formulas and expands gray, the shared preview code accepts bytes after
/// EOI or a missing EOI (R-CR3-42) and applies the orientation with the
/// exception for previews taller than wide (R-CR3-38, R-CR3-39). A failed
/// decode delivers nothing (R-CR3-54).
pub(crate) fn decode(
    d: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> Cr3Result<ImageBuf> {
    shared::decode(d, &p.stream, orientation, options)
}

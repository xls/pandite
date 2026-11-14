//! The embedded JPEG previews of a CR2 file (section Embedded preview):
//! candidates collected by R-CR2-37 and R-CR2-38, decoded by the shared
//! preview decoder with the IFD0 orientation (R-CR2-39, R-CR2-40).

use super::meta::{tag, Container};
use super::Cr2Result;
use crate::raw::arw::preview::{self as shared, Candidate};
use pandite_core::{DecodeOptions, ImageBuf, Orientation};

/// R-CR2-37 and R-CR2-38: the IFD0 strip (Compression 6, one StripOffsets
/// and one StripByteCounts value) and the pair 0x0201, 0x0202 of every IFD
/// of the chain, each kept only when its byte range lies inside the file,
/// is at least 4 bytes long, starts with FF D8 and has a size from its
/// first SOF0, SOF1 or SOF2 segment. Largest width * height first.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    if let Some(ifd0) = c.ifd0() {
        let jpeg = matches!(ifd0.uint(tag::COMPRESSION), Ok(Some(6)));
        if let (true, Ok(Some(o)), Ok(Some(l))) = (
            jpeg,
            ifd0.uints(tag::STRIP_OFFSETS),
            ifd0.uints(tag::STRIP_BYTE_COUNTS),
        ) {
            if o.len() == 1 && l.len() == 1 {
                ranges.push((o[0], l[0]));
            }
        }
    }
    for dir in &c.chain {
        if let (Ok(Some(o)), Ok(Some(l))) = (dir.uint(tag::JPEG_OFFSET), dir.uint(tag::JPEG_LENGTH))
        {
            ranges.push((o, l));
        }
    }
    let mut out: Vec<Candidate> = Vec::new();
    for (off, len) in ranges {
        // R-CR2-37, R-CR2-46: inside the file (64-bit sum, R-CR2-43), at
        // least 4 bytes, FF D8 first.
        if u64::from(off) + u64::from(len) > size || len < 4 {
            continue;
        }
        let (off, len) = (off as usize, len as usize);
        let stream = &c.data[off..off + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == off) {
            continue;
        }
        // R-CR2-38: the size from the first SOF0, SOF1 or SOF2 segment. A
        // candidate without one has no size and is not kept.
        if let Some((width, height)) = shared::frame_size(stream) {
            out.push(Candidate {
                offset: off,
                len,
                width,
                height,
            });
        }
    }
    // R-CR2-38: the largest width * height first; R-CR2-41 tries the others
    // in this order.
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// The size of a candidate as delivered: R-CR2-40 applies the IFD0
/// orientation, so values 5 to 8 swap width and height.
pub(super) fn displayed(p: &Candidate, orientation: u8) -> (u32, u32) {
    if orientation >= 5 {
        (p.height, p.width)
    } else {
        (p.width, p.height)
    }
}

/// Decode one candidate (R-CR2-39: bytes after EOI, or no EOI after the
/// last scan, are accepted, R-ARW-61) and apply the IFD0 orientation
/// (R-CR2-40).
pub(super) fn decode(
    data: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> Cr2Result<ImageBuf> {
    let o = Orientation::from_exif(u16::from(orientation));
    shared::decode_as(data, p, o, options)
}

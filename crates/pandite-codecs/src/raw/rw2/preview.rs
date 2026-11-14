//! The embedded JPEG previews of an RW2 file (section Embedded preview):
//! IFD0 tags 0x002E (JpgFromRaw) and 0x0127 (JpgFromRaw2). Their Exif
//! segments are removed (R-RW2-51); decoding and orientation are then
//! shared with the Sony decoder.

use super::meta::{tag, Container};
use super::{fail, Rw2Result};
use crate::raw::arw::preview::{self as shared, frame_size, Candidate};
use pandite_core::{DecodeOptions, ImageBuf};

/// R-RW2-45 and R-RW2-46: the preview candidates, largest first.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    for t in [tag::JPG_FROM_RAW, tag::JPG_FROM_RAW2] {
        // A count of 0 counts as absent (S18 has 0x0127 with count 0).
        let Some(e) = c.ifd0.entry(t) else {
            continue;
        };
        // R-RW2-45: count >= 4, offset + count inside the file (64-bit
        // sum, R-RW2-52), and the stream starts with FF D8. The value bytes
        // are located by R-RW2-8 (the value field holds the offset when the
        // stream is longer than 4 bytes).
        if e.count < 4 || e.len > size || e.at > size - e.len {
            continue;
        }
        let (at, len) = (e.at as usize, e.len as usize);
        let stream = &c.data[at..at + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == at) {
            continue;
        }
        // R-RW2-46: the size from the first SOF0, SOF1 or SOF2 segment. A
        // candidate without one has no size and is not kept (the document
        // does not say; the Sony decoder does the same).
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset: at,
                len,
                width,
                height,
            });
        }
    }
    // R-RW2-46: the largest width * height first (R-RW2-49: then the next
    // largest).
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// The preview stream without its APP1 segments (Exif), so that neither the
/// orientation tag nor the thumbnail inside it is used (R-RW2-51). The
/// marker segments before the first SOS are walked by ITU-T T.81 B.1.1: a
/// marker is FF followed by its code (fill bytes FF may precede it), and a
/// segment length counts its own two bytes. Everything from SOS on, and
/// anything that does not parse as a segment, is copied unchanged for the
/// JPEG decoder to judge.
pub(super) fn without_exif(stream: &[u8]) -> Rw2Result<Vec<u8>> {
    let mut out = Vec::new();
    out.try_reserve_exact(stream.len())
        .map_err(|_| fail("not enough memory for the preview"))?;
    let head = stream.len().min(2);
    out.extend_from_slice(&stream[..head]);
    let mut pos = head;
    while pos < stream.len() {
        let rest = &stream[pos..];
        if rest.len() < 4 || rest[0] != 0xFF {
            break;
        }
        let marker = rest[1];
        if marker == 0xFF {
            // A fill byte.
            out.push(0xFF);
            pos += 1;
            continue;
        }
        // SOS, EOI and the markers without a length end the walk.
        if matches!(marker, 0xDA | 0xD9 | 0x01 | 0xD0..=0xD7) {
            break;
        }
        let len = usize::from(u16::from_be_bytes([rest[2], rest[3]]));
        if len < 2 || 2 + len > rest.len() {
            break;
        }
        if marker != 0xE1 {
            out.extend_from_slice(&rest[..2 + len]);
        }
        pos += 2 + len;
    }
    out.extend_from_slice(&stream[pos..]);
    Ok(out)
}

/// Decode one candidate and orient it by the IFD0 orientation: R-RW2-51
/// (its Exif data are removed first), then the shared preview decoder,
/// which applies R-RW2-47 (bytes after EOI, no EOI) and R-RW2-48 (values 5
/// to 8 only on a preview that is not taller than wide).
pub(super) fn decode(
    data: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> Rw2Result<ImageBuf> {
    let stream = data
        .get(p.offset..p.offset.saturating_add(p.len))
        .ok_or_else(|| fail("preview outside the file"))?;
    let clean = without_exif(stream)?;
    let local = Candidate {
        offset: 0,
        len: clean.len(),
        ..*p
    };
    shared::decode(&clean, &local, orientation, options)
}

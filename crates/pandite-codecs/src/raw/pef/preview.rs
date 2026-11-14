//! The embedded JPEG previews of a PEF file (section Embedded preview).
//! Candidates are found here; sizing, decoding and orientation use the
//! shared preview code of the Sony decoder.

use super::meta::Container;
use crate::raw::arw::preview::{frame_size, Candidate};

/// JPEGInterchangeFormat and JPEGInterchangeFormatLength.
const OFFSET_TAG: u16 = 0x0201;
const LENGTH_TAG: u16 = 0x0202;

/// R-PEF-53 and R-PEF-54: the preview candidates of every IFD of the chain,
/// largest first.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    for dir in &c.chain {
        let (Ok(Some(off)), Ok(Some(len))) = (dir.uint(OFFSET_TAG), dir.uint(LENGTH_TAG)) else {
            continue;
        };
        // R-PEF-53: offset + length inside the file (64-bit sum), length at
        // least 4, and the stream starts with FF D8.
        if u64::from(off) + u64::from(len) > size || len < 4 {
            continue;
        }
        let (off, len) = (off as usize, len as usize);
        let stream = &c.data[off..off + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == off) {
            continue;
        }
        // R-PEF-54: the size from the first SOF0, SOF1 or SOF2 segment, not
        // from ImageWidth and ImageLength. A candidate without one has no
        // size and is not kept (specification gap: the document does not
        // say).
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset: off,
                len,
                width,
                height,
            });
        }
    }
    // R-PEF-54: the largest width * height first.
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

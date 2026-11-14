//! The embedded JPEG previews of an SRW file (section Embedded preview).
//! Candidates are collected here; decoding and orientation use the Sony
//! decoder's preview code, whose rules R-ARW-60 to R-ARW-62 are the rules
//! R-SRW-45 to R-SRW-47.

use super::ifd::Dir;
use super::meta::{self, tag, Container};
use crate::raw::arw::preview::{frame_size, Candidate};

/// R-SRW-44 and R-SRW-45: the preview candidates of every IFD of the main
/// chain and of every valid SubIFD of these IFDs, largest first. The
/// maker-note preview of tag 0x0035 is not used.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    let mut consider = |d: &Dir<'_>| {
        let (Ok(Some(off)), Ok(Some(len))) = (d.uint(tag::JPEG_OFFSET), d.uint(tag::JPEG_LENGTH))
        else {
            return;
        };
        // R-SRW-44: offset + length inside the file (64-bit sum, R-SRW-50),
        // length at least 4, and the stream starts with FF D8.
        if u64::from(off) + u64::from(len) > size || len < 4 {
            return;
        }
        let (off, len) = (off as usize, len as usize);
        let stream = &c.data[off..off + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == off) {
            return;
        }
        // R-SRW-45: the size from the first SOF0, SOF1 or SOF2 segment. A
        // stream without one has no size and is not kept (specification
        // gap: the document does not say).
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset: off,
                len,
                width,
                height,
            });
        }
    };
    for d in &c.chain {
        consider(d);
        for s in meta::sub_ifds(c.data, c.le, d) {
            consider(&s);
        }
    }
    // R-SRW-45: the largest width * height first.
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

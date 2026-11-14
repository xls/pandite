//! The embedded previews of a NEF file (section Embedded preview): the
//! JPEG candidates of the IFD chain, the SubIFDs and the maker-note
//! PreviewIFD, and the uncompressed RGB thumbnail of IFD0. JPEG decoding
//! and orientation use the Sony decoder's preview code, whose rules
//! R-ARW-60 to R-ARW-62 are the rules R-NEF-53, R-NEF-54 and R-NEF-57.

use super::meta::{self, tag, Container};
use super::tiff::Ifd;
use super::{fail, NefResult};
use crate::raw::arw::preview::{frame_size, Candidate};
use crate::{orient, util};
use pandite_core::{ImageBuf, Orientation};

/// R-NEF-52 and R-NEF-53: the JPEG candidates of every IFD of the chain,
/// of every valid SubIFD of these IFDs and of the maker-note PreviewIFD,
/// largest width * height first (R-NEF-55).
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    // `base` is added to the stored offset: 0 in the main structure, the
    // maker-note origin in the PreviewIFD (R-NEF-14).
    let mut consider = |d: &Ifd<'_>, base: u64| {
        let (Ok(Some(off)), Ok(Some(len))) = (d.int(tag::JPEG_OFFSET), d.int(tag::JPEG_LENGTH))
        else {
            return;
        };
        // R-NEF-52, R-NEF-61: offset + length inside the file (64-bit sum,
        // R-NEF-58), length at least 4, and the stream starts with FF D8.
        let at = base + u64::from(off);
        if at + u64::from(len) > size || len < 4 {
            return;
        }
        let (at, len) = (at as usize, len as usize);
        let stream = &c.data[at..at + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == at) {
            return;
        }
        // R-NEF-53: the size from the first SOF0, SOF1 or SOF2 segment. A
        // stream without one has no size and is not kept (the document
        // does not say; the Sony decoder does the same).
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset: at,
                len,
                width,
                height,
            });
        }
    };
    for d in &c.chain {
        consider(d, 0);
        for s in meta::sub_ifds(c.data, c.le, d) {
            consider(&s, 0);
        }
    }
    // The maker-note PreviewIFD: its offset and the offsets inside it count
    // from the maker-note origin.
    if let Some(m) = &c.maker {
        if let Ok(Some(off)) = m.ifd.int(tag::PREVIEW_IFD) {
            if let Some(p) = Ifd::read(c.data, m.le, m.origin + u64::from(off), m.origin) {
                consider(&p, m.origin);
            }
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// The IFD0 thumbnail of R-NEF-56: Compression 1, PhotometricInterpretation
/// 2, SamplesPerPixel 3, BitsPerSample 8 8 8 and one strip of exactly
/// 3 * width * height bytes inside the file. Returns (offset, width,
/// height).
pub(super) fn thumbnail(c: &Container<'_>) -> Option<(usize, u32, u32)> {
    let d = c.ifd0()?;
    let int = |t: u16| d.int(t).ok().flatten();
    if int(tag::COMPRESSION)? != 1
        || int(tag::PHOTOMETRIC)? != 2
        || int(tag::SAMPLES_PER_PIXEL)? != 3
    {
        return None;
    }
    if d.ints(tag::BITS_PER_SAMPLE).ok().flatten()?.as_slice() != [8, 8, 8] {
        return None;
    }
    let (w, h) = (int(tag::IMAGE_WIDTH)?, int(tag::IMAGE_LENGTH)?);
    let offsets = d.ints(tag::STRIP_OFFSETS).ok().flatten()?;
    let counts = d.ints(tag::STRIP_BYTE_COUNTS).ok().flatten()?;
    let ([off], [count]) = (offsets.as_slice(), counts.as_slice()) else {
        return None;
    };
    let need = 3 * u64::from(w) * u64::from(h);
    // R-NEF-61: the strip lies inside the file.
    if w == 0 || h == 0 || u64::from(*count) != need || u64::from(*off) + need > c.data.len() as u64
    {
        return None;
    }
    Some((*off as usize, w, h))
}

/// The size of the thumbnail as delivered (R-NEF-57).
pub(super) fn thumbnail_displayed(w: u32, h: u32, orientation: u8) -> (u32, u32) {
    if orientation >= 5 && w >= h {
        (h, w)
    } else {
        (w, h)
    }
}

/// Decode the thumbnail (pixels in row order, red, green, blue) to RGBA8
/// and orient it by R-NEF-57: the IFD0 orientation, except values 5 to 8
/// on a picture taller than wide.
pub(super) fn decode_thumbnail(
    data: &[u8],
    (off, w, h): (usize, u32, u32),
    orientation: u8,
) -> NefResult<ImageBuf> {
    let need = 3 * w as usize * h as usize;
    let src = off
        .checked_add(need)
        .and_then(|end| data.get(off..end))
        .ok_or_else(|| fail("thumbnail outside the file"))?;
    let mut px = util::alloc_rgba8(w, h)?;
    util::expand_to_rgba8(src, 3, &mut px);
    let img = util::rgba8_image(w, h, px)?;
    let o = if orientation >= 5 && h > w {
        Orientation::Normal
    } else {
        Orientation::from_exif(u16::from(orientation))
    };
    orient::apply(img, o)
}

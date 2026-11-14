//! The orientation (R-RAF-43, R-RAF-44) and the embedded JPEG previews of a
//! RAF file (section Embedded preview).

use super::container::Raf;
use super::RafResult;
use crate::exif::Tiff;
use crate::raw::arw::preview::{self as shared, frame_size, Candidate};
use crate::{jpeg, orient, util};
use pandite_core::{DecodeOptions, ImageBuf, Orientation};

/// Exif Orientation in IFD0.
const ORIENTATION_TAG: u16 = 0x0112;
/// JPEGInterchangeFormat and JPEGInterchangeFormatLength in IFD1.
const OFFSET_TAG: u16 = 0x0201;
const LENGTH_TAG: u16 = 0x0202;

/// The preview candidates, largest first, and the orientation.
pub(super) struct Previews {
    /// Candidates by decreasing width * height (R-RAF-49).
    pub candidates: Vec<Candidate>,
    /// Exif orientation 1 to 8 (R-RAF-43).
    pub orientation: u8,
}

/// The TIFF structure of the first APP1 Exif segment of a JPEG stream
/// among the marker segments before its first SOS marker (R-RAF-43), and
/// its offset in the stream. The slice ends with the segment, so every
/// offset read inside it stays in the segment (R-RAF-44).
pub(super) fn exif(s: &[u8]) -> Option<(usize, &[u8])> {
    if s.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut pos = 2usize;
    loop {
        if *s.get(pos)? != 0xFF {
            return None;
        }
        // Fill bytes before a marker.
        while *s.get(pos + 1)? == 0xFF {
            pos += 1;
        }
        let marker = *s.get(pos + 1)?;
        match marker {
            // Standalone markers have no length.
            0x01 | 0xD0..=0xD7 => {
                pos += 2;
                continue;
            }
            // The first SOS, or the end of the image.
            0xDA | 0xD9 => return None,
            _ => {}
        }
        let len = usize::from(util::be16(s, pos + 2)?);
        if len < 2 {
            return None;
        }
        let end = pos + 2 + len;
        let payload = s.get(pos + 4..end)?;
        if marker == 0xE1 {
            if let Some(tiff) = payload.strip_prefix(b"Exif\0\0") {
                return Some((pos + 4 + 6, tiff));
            }
        }
        pos = end;
    }
}

/// A TIFF header (S1 section 2): byte order 49 49 or 4D 4D and 42.
fn tiff(t: &[u8]) -> Option<Tiff<'_>> {
    let tiff = Tiff::new(t)?;
    (tiff.u16(2)? == 42).then_some(tiff)
}

/// R-RAF-43: tag 0x0112 of IFD0, 1 when it is absent or outside 1 to 8.
pub(super) fn orientation(t: &[u8]) -> u8 {
    let ifd0 = tiff(t).and_then(|tf| tf.ifd(tf.first_ifd()?).map(|(e, _)| (tf, e)));
    match ifd0.and_then(|(tf, e)| tf.get(&e, ORIENTATION_TAG)) {
        Some(v @ 1..=8) => v as u8,
        _ => 1,
    }
}

/// R-RAF-47, R-RAF-48: the IFD1 thumbnail (offset and length in the TIFF
/// slice), when it lies inside the APP1 segment.
fn thumbnail(t: &[u8]) -> Option<(usize, usize)> {
    let tf = tiff(t)?;
    let (_, next) = tf.ifd(tf.first_ifd()?)?;
    if next == 0 {
        return None;
    }
    let (ifd1, _) = tf.ifd(next)?;
    let off = tf.get(&ifd1, OFFSET_TAG)? as usize;
    let len = tf.get(&ifd1, LENGTH_TAG)? as usize;
    (off.checked_add(len)? <= t.len()).then_some((off, len))
}

/// R-RAF-48 and R-RAF-49: keep a candidate that lies inside the file, is
/// at least 4 bytes long, starts with FF D8 and has a SOF0, SOF1 or SOF2
/// frame size.
fn push(out: &mut Vec<Candidate>, data: &[u8], offset: usize, len: usize) {
    let Some(s) = offset
        .checked_add(len)
        .and_then(|end| data.get(offset..end))
    else {
        return;
    };
    if len < 4 || s[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == offset) {
        return;
    }
    // A candidate without a frame size (the fixtures' Exif stub) has no
    // size by R-RAF-49 and cannot be decoded; it is not kept.
    if let Some((width, height)) = frame_size(s) {
        out.push(Candidate {
            offset,
            len,
            width,
            height,
        });
    }
}

/// Step 2 of the decoding procedure: the orientation (R-RAF-43, R-RAF-44,
/// also when the JPEG cannot be decoded) and the preview candidates: the
/// main JPEG of the header and the IFD1 thumbnail of its Exif segment
/// (R-RAF-47), largest first (R-RAF-49).
pub(super) fn collect(raf: &Raf<'_>) -> Previews {
    let mut candidates = Vec::new();
    let mut o = 1;
    // R-RAF-6: the JPEG part is present.
    if let (Some(p), Some(stream)) = (raf.jpeg, raf.part(raf.jpeg)) {
        push(&mut candidates, raf.data, p.offset, p.len);
        if let Some((at, t)) = exif(stream) {
            o = orientation(t);
            if let Some((off, len)) = thumbnail(t) {
                // Offsets of IFD1 count from the TIFF header of the segment.
                push(&mut candidates, raf.data, p.offset + at + off, len);
            }
        }
    }
    // The main JPEG stays first among equal sizes.
    candidates.sort_by_key(|p| std::cmp::Reverse(p.area()));
    Previews {
        candidates,
        orientation: o,
    }
}

/// The orientation that undoes `o`.
fn inverse(o: Orientation) -> Orientation {
    match o {
        Orientation::Rotate90 => Orientation::Rotate270,
        Orientation::Rotate270 => Orientation::Rotate90,
        other => other,
    }
}

/// Decode one candidate and orient it: R-RAF-51, the orientation of
/// R-RAF-43 by the table of R-RAF-45, except values 5 to 8 on a preview
/// taller than wide. R-RAF-50: bytes after EOI and a missing EOI are
/// accepted by the Sony decoder's preview decoding.
pub(super) fn decode(
    data: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> RafResult<ImageBuf> {
    let img = shared::decode(data, p, orientation, options)?;
    // The JPEG decoder turns a stream by its own Exif orientation (the main
    // JPEG carries the orientation of R-RAF-43) and the shared decoding
    // then leaves it; otherwise the shared decoding applied R-RAF-51.
    let stream = data.get(p.offset..p.offset + p.len).unwrap_or(&[]);
    let own = jpeg::scan_header(stream)
        .and_then(|h| h.exif)
        .map(crate::exif::orientation_from_exif)
        .unwrap_or_default();
    let wanted = if p.rotates(orientation) {
        Orientation::from_exif(u16::from(orientation))
    } else {
        Orientation::Normal
    };
    let applied = if own == Orientation::Normal {
        wanted
    } else {
        own
    };
    if applied == wanted {
        return Ok(img);
    }
    orient::apply(orient::apply(img, inverse(applied))?, wanted)
}

//! The embedded JPEG previews of an ARW file (section Embedded preview).

use super::meta::Container;
use super::{fail, ArwResult};
use crate::{jpeg, orient};
use pandite_core::{DecodeOptions, Error, ImageBuf, Orientation};

/// JPEGInterchangeFormat and JPEGInterchangeFormatLength.
const OFFSET_TAG: u16 = 0x0201;
const LENGTH_TAG: u16 = 0x0202;

/// One preview candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Candidate {
    /// File offset of the JPEG stream.
    pub offset: usize,
    /// Stream length.
    pub len: usize,
    /// Stored width from the first SOF0, SOF1 or SOF2 segment.
    pub width: u32,
    /// Stored height.
    pub height: u32,
}

impl Candidate {
    /// Width times height.
    pub(super) fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// True when the IFD0 orientation is applied to this preview: always,
    /// except values 5 to 8 on a preview taller than wide (R-ARW-62).
    pub(super) fn rotates(&self, orientation: u8) -> bool {
        !(orientation >= 5 && self.height > self.width)
    }

    /// The size as delivered, after R-ARW-62.
    pub(super) fn displayed(&self, orientation: u8) -> (u32, u32) {
        if orientation >= 5 && self.rotates(orientation) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// R-ARW-60: the size from the first SOF0, SOF1 or SOF2 marker segment.
fn frame_size(s: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2usize;
    loop {
        if *s.get(pos)? != 0xFF {
            return None;
        }
        while *s.get(pos + 1)? == 0xFF {
            pos += 1;
        }
        let marker = *s.get(pos + 1)?;
        match marker {
            // Standalone markers.
            0x01 | 0xD0..=0xD7 => {
                pos += 2;
                continue;
            }
            // Start of scan or end of image before any frame header.
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let len = usize::from(u16::from_be_bytes([*s.get(pos + 2)?, *s.get(pos + 3)?]));
        if len < 2 {
            return None;
        }
        if matches!(marker, 0xC0..=0xC2) {
            let b = s.get(pos + 4..pos + 9)?;
            let h = u32::from(u16::from_be_bytes([b[1], b[2]]));
            let w = u32::from(u16::from_be_bytes([b[3], b[4]]));
            return (w > 0 && h > 0).then_some((w, h));
        }
        pos += 2 + len;
    }
}

/// R-ARW-59 and R-ARW-60: the preview candidates of IFD0 and every IFD
/// of the chain, largest first.
pub(super) fn collect(c: &Container<'_>) -> Vec<Candidate> {
    let size = c.data.len() as u64;
    let mut out: Vec<Candidate> = Vec::new();
    for dir in &c.chain {
        let (Ok(Some(off)), Ok(Some(len))) = (dir.uint(OFFSET_TAG), dir.uint(LENGTH_TAG)) else {
            continue;
        };
        // R-ARW-59: offset + length inside the file (64-bit sum), length at
        // least 4, and the stream starts with FF D8.
        if u64::from(off) + u64::from(len) > size || len < 4 {
            continue;
        }
        let (off, len) = (off as usize, len as usize);
        let stream = &c.data[off..off + len];
        if stream[..2] != [0xFF, 0xD8] || out.iter().any(|p| p.offset == off) {
            continue;
        }
        // A candidate without SOF0, SOF1 or SOF2 has no size by R-ARW-60
        // and is not kept (specification gap: the document does not say).
        if let Some((width, height)) = frame_size(stream) {
            out.push(Candidate {
                offset: off,
                len,
                width,
                height,
            });
        }
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.area()));
    out
}

/// Decode one candidate and orient it (R-ARW-61, R-ARW-62).
pub(super) fn decode(
    data: &[u8],
    p: &Candidate,
    orientation: u8,
    options: &DecodeOptions,
) -> ArwResult<ImageBuf> {
    let stream = data
        .get(p.offset..p.offset + p.len)
        .ok_or_else(|| fail("preview outside the file"))?;
    let rotate = p.rotates(orientation);
    let o = if rotate {
        Orientation::from_exif(u16::from(orientation))
    } else {
        Orientation::Normal
    };
    // The target is in display orientation; the JPEG decoder scales the
    // stored axes.
    let opts = DecodeOptions {
        target_size: options.target_size.map(|(w, h)| jpeg::oriented(w, h, o)),
        first_frame_only: true,
        prefer_8bit: true,
        cancel: options.cancel.clone(),
    };
    let img = match jpeg::decode_image(stream, &opts) {
        Ok(img) => img,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        // R-ARW-61: a stream that ends without EOI after its last scan is
        // accepted: retry with an EOI marker appended. Bytes after EOI are
        // ignored by the JPEG decoder.
        Err(e) if !stream.ends_with(&[0xFF, 0xD9]) => {
            let mut fixed = Vec::new();
            fixed.try_reserve_exact(stream.len() + 2).map_err(|_| e)?;
            fixed.extend_from_slice(stream);
            fixed.extend_from_slice(&[0xFF, 0xD9]);
            jpeg::decode_image(&fixed, &opts)?
        }
        Err(e) => return Err(e),
    };
    // The previews carry no orientation of their own (M13); one that does
    // was already turned upright by the JPEG decoder.
    let own = jpeg::scan_header(stream)
        .and_then(|h| h.exif)
        .map(crate::exif::orientation_from_exif)
        .unwrap_or_default();
    if own == Orientation::Normal {
        orient::apply(img, o)
    } else {
        Ok(img)
    }
}

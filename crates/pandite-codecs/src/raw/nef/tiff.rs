//! Typed tag access for NEF on top of the TIFF reader of the Samsung
//! decoder (`srw::ifd`), which reads IFDs in either byte order with value
//! offsets from any origin. That reader already applies R-NEF-6 (inline
//! values up to 4 bytes), R-NEF-7 (the first entry of a tag), R-NEF-9
//! (entries with a type outside 1 to 13 are skipped), R-NEF-11 (the IFD
//! chain) and R-NEF-12 (invalid IFDs). This layer adds the integer types of
//! R-NEF-8 (BYTE as well), the size rules R-NEF-60 and R-NEF-61, and reads
//! numbers inside UNDEFINED values in the byte order of the IFD (R-NEF-5).

use super::{fail, NefResult};
use crate::raw::arw::ifd::Entry;
use crate::raw::srw::ifd::{self, Dir};

/// One IFD, the byte order of its values and the file it lies in.
#[derive(Clone, Debug)]
pub(super) struct Ifd<'a> {
    dir: Dir<'a>,
    /// Byte order of the multi-byte fields of this IFD (R-NEF-5): the file
    /// byte order, or the maker-note byte order inside the maker note.
    pub le: bool,
    data: &'a [u8],
}

impl<'a> Ifd<'a> {
    /// The IFD at file offset `offset`, or `None` when it is invalid
    /// (R-NEF-12). Out-of-line value offsets count from `base`: 0 in the
    /// main structure, the maker-note origin inside the maker note
    /// (R-NEF-14).
    pub(super) fn read(data: &'a [u8], le: bool, offset: u64, base: u64) -> Option<Self> {
        ifd::dir(data, le, offset, base).map(|dir| Ifd { dir, le, data })
    }

    /// IFD0 and the IFDs of its chain (R-NEF-11).
    pub(super) fn chain(data: &'a [u8], le: bool, first: u32) -> Vec<Self> {
        ifd::chain(data, le, first)
            .into_iter()
            .map(|dir| Ifd { dir, le, data })
            .collect()
    }

    /// The first entry of `tag` with at least one value (R-NEF-7, R-NEF-16).
    pub(super) fn entry(&self, tag: u16) -> Option<&Entry> {
        self.dir.entry(tag)
    }

    /// True when `tag` is present with at least one value.
    pub(super) fn has(&self, tag: u16) -> bool {
        self.dir.has(tag)
    }

    /// R-NEF-60: a value whose count * type size exceeds the file size
    /// fails before anything is allocated for it.
    fn check_len(&self, e: &Entry) -> NefResult<()> {
        if e.len > self.data.len() as u64 {
            return Err(fail(format!("tag 0x{:04X} is larger than the file", e.tag)));
        }
        Ok(())
    }

    /// The value bytes of `tag`, whatever its type. R-NEF-61: a value that
    /// does not lie completely inside the file is not read; the tag is
    /// then invalid and counts as absent (its default applies, and a
    /// required tag fails by R-NEF-16).
    pub(super) fn bytes(&self, tag: u16) -> NefResult<Option<&'a [u8]>> {
        let Some(e) = self.dir.entry(tag) else {
            return Ok(None);
        };
        self.check_len(e)?;
        Ok(self.dir.bytes(e).ok().flatten())
    }

    /// All values of an integer tag of type BYTE, SHORT, LONG or IFD
    /// (R-NEF-8). `Ok(None)` when absent, of another type or invalid.
    pub(super) fn ints(&self, tag: u16) -> NefResult<Option<Vec<u32>>> {
        let Some(e) = self.dir.entry(tag) else {
            return Ok(None);
        };
        self.check_len(e)?;
        if e.typ == 1 {
            return Ok(self
                .dir
                .bytes(e)
                .ok()
                .flatten()
                .map(|b| b.iter().map(|&v| u32::from(v)).collect()));
        }
        Ok(self.dir.uints(tag).ok().flatten())
    }

    /// The first value of an integer tag (R-NEF-8).
    pub(super) fn int(&self, tag: u16) -> NefResult<Option<u32>> {
        Ok(self.ints(tag)?.and_then(|v| v.first().copied()))
    }

    /// An ASCII value up to its first zero byte, trailing spaces removed
    /// (R-NEF-10). `Ok(None)` when absent, not ASCII or invalid.
    pub(super) fn ascii(&self, tag: u16) -> NefResult<Option<&'a [u8]>> {
        let Some(e) = self.dir.entry(tag) else {
            return Ok(None);
        };
        self.check_len(e)?;
        Ok(self.dir.ascii(tag).ok().flatten())
    }

    /// The (numerator, denominator) pairs of a RATIONAL tag. `Ok(None)`
    /// when absent, of another type or invalid.
    pub(super) fn rationals(&self, tag: u16) -> NefResult<Option<Vec<(u32, u32)>>> {
        let Some(e) = self.dir.entry(tag) else {
            return Ok(None);
        };
        if e.typ != 5 {
            return Ok(None);
        }
        let le = self.le;
        Ok(self.bytes(tag)?.map(|b| {
            b.chunks_exact(8)
                .map(|c| (word(&c[..4], le), word(&c[4..], le)))
                .collect()
        }))
    }
}

/// The 16-bit number at byte `at` of a value, in the given byte order
/// (R-NEF-5). `None` past the end of the value.
pub(super) fn u16_at(b: &[u8], at: usize, le: bool) -> Option<u16> {
    let p = b.get(at..at.checked_add(2)?)?;
    Some(if le {
        u16::from_le_bytes([p[0], p[1]])
    } else {
        u16::from_be_bytes([p[0], p[1]])
    })
}

/// A 32-bit number in the given byte order (R-NEF-5).
pub(super) fn word(c: &[u8], le: bool) -> u32 {
    let b = [c[0], c[1], c[2], c[3]];
    if le {
        u32::from_le_bytes(b)
    } else {
        u32::from_be_bytes(b)
    }
}

/// The 32-bit number at file offset `at` (R-NEF-5).
pub(super) fn read32(data: &[u8], at: u64, le: bool) -> Option<u32> {
    ifd::read32(data, at, le)
}

//! The TIFF structure of an SRW file: header, IFD chain, entries and typed
//! values in either byte order, and the maker-note IFD whose value offsets
//! count from its own first byte. Every offset and length is checked
//! against the file in 64-bit arithmetic.
//!
//! The Sony reader (`arw::ifd`) reads little-endian files only, and most
//! SRW files are big-endian (M1), so this module reads the structure
//! itself. It shares the entry record and the field type sizes with the
//! Sony reader.

use super::{fail, SrwResult};
use crate::raw::arw::ifd::{type_size, Entry};

/// Most IFDs followed along the main chain (R-SRW-10).
const MAX_CHAIN: usize = 16;
/// Largest entry count of a valid IFD (R-SRW-11).
const MAX_ENTRIES: u64 = 1024;

/// Read a u16 in the given byte order (R-SRW-5).
pub(super) fn read16(d: &[u8], at: u64, le: bool) -> Option<u16> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(2)?)?;
    Some(if le {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    })
}

/// Read a u32 in the given byte order (R-SRW-5).
pub(crate) fn read32(d: &[u8], at: u64, le: bool) -> Option<u32> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(4)?)?;
    Some(if le {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    })
}

/// Check the size and the header and return the byte order (true for
/// little-endian) and the IFD0 offset. A file shorter than 8 bytes
/// (R-SRW-50) or with any other start than 4D 4D 00 2A or 49 49 2A 00
/// (R-SRW-4) is an error without a preview.
pub(super) fn header(data: &[u8]) -> SrwResult<(bool, u32)> {
    // R-SRW-50: shorter than 8 bytes.
    if data.len() < 8 {
        return Err(fail("file shorter than 8 bytes"));
    }
    // R-SRW-4: only the two TIFF starts are accepted.
    let le = match data[..4] {
        [0x4D, 0x4D, 0x00, 0x2A] => false,
        [0x49, 0x49, 0x2A, 0x00] => true,
        _ => return Err(fail("not a TIFF header")),
    };
    // R-SRW-10: IFD0 at the offset in header bytes 4 to 7.
    let first = read32(data, 4, le).unwrap_or(0);
    Ok((le, first))
}

/// One IFD: its entries in file order and the next-IFD offset.
#[derive(Clone, Debug)]
pub(crate) struct Dir<'a> {
    data: &'a [u8],
    /// Byte order of the file (R-SRW-5).
    le: bool,
    /// True for the maker-note IFD: a value outside the file makes its tag
    /// absent instead of failing (R-SRW-59).
    lenient: bool,
    /// The entries with a field type from 1 to 13, in file order.
    pub entries: Vec<Entry>,
    /// The offset of the next IFD of the chain, 0 at the end.
    pub next: u32,
}

/// Read the IFD at file offset `offset`, or `None` when it is invalid
/// (R-SRW-11). Out-of-line value offsets count from `base`: 0 for the
/// TIFF structure, P for the maker note (R-SRW-14).
pub(crate) fn dir(data: &[u8], le: bool, offset: u64, base: u64) -> Option<Dir<'_>> {
    let size = data.len() as u64;
    // R-SRW-11 case 1: the offset lies outside the file.
    if offset >= size {
        return None;
    }
    let n = u64::from(read16(data, offset, le)?);
    // R-SRW-11 case 2: N is 0 or greater than 1024.
    if n == 0 || n > MAX_ENTRIES {
        return None;
    }
    // R-SRW-11 case 3: the 2 + 12 * N + 4 bytes lie inside the file
    // (64-bit arithmetic, R-SRW-50).
    if offset + 2 + 12 * n + 4 > size {
        return None;
    }
    let mut entries = Vec::with_capacity(n as usize);
    for i in 0..n {
        let at = offset + 2 + 12 * i;
        let tag = read16(data, at, le)?;
        let typ = read16(data, at + 2, le)?;
        let count = read32(data, at + 4, le)?;
        // R-SRW-7: skip entries whose type is outside 1 to 13, for example
        // the all-zero entry of M1.
        let Some(unit) = type_size(typ) else {
            continue;
        };
        let len = u64::from(count) * unit;
        // R-SRW-6: inline when count * size <= 4, else at the offset in
        // the value field; for the maker note P plus that offset
        // (R-SRW-14), in 64-bit arithmetic (R-SRW-50).
        let value_at = if len <= 4 {
            at + 8
        } else {
            base + u64::from(read32(data, at + 8, le)?)
        };
        entries.push(Entry {
            tag,
            typ,
            count,
            at: value_at,
            len,
        });
    }
    let next = read32(data, offset + 2 + 12 * n, le).unwrap_or(0);
    Some(Dir {
        data,
        le,
        lenient: false,
        entries,
        next,
    })
}

/// The maker-note IFD at P (R-SRW-13, R-SRW-14, R-SRW-59): R-SRW-11 applies
/// at P, its value offsets count from P, and a value outside the file
/// makes its tag absent.
pub(super) fn maker_dir(data: &[u8], le: bool, p: u64) -> Option<Dir<'_>> {
    let mut d = dir(data, le, p, p)?;
    d.lenient = true;
    Some(d)
}

/// IFD0 and the IFDs that follow it through the next-IFD offsets
/// (R-SRW-10). Empty when IFD0 itself is invalid.
pub(crate) fn chain(data: &[u8], le: bool, first: u32) -> Vec<Dir<'_>> {
    let mut out: Vec<Dir<'_>> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let mut at = first;
    // R-SRW-10: stop at offset 0, an invalid IFD, a repeated offset or
    // after 16 IFDs.
    while at != 0 && out.len() < MAX_CHAIN && !seen.contains(&at) {
        seen.push(at);
        let Some(d) = dir(data, le, u64::from(at), 0) else {
            break;
        };
        at = d.next;
        out.push(d);
    }
    out
}

impl<'a> Dir<'a> {
    /// The first entry of `tag` (R-SRW-7), any entry order. An entry with
    /// count 0 counts as absent (R-SRW-16).
    pub(crate) fn entry(&self, tag: u16) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.tag == tag)
            .filter(|e| e.count > 0)
    }

    /// True when `tag` is present with at least one value.
    pub(crate) fn has(&self, tag: u16) -> bool {
        self.entry(tag).is_some()
    }

    /// The value bytes of an entry. R-SRW-52: count * type size beyond the
    /// file size fails before any allocation. R-SRW-53: the range must lie
    /// inside the file, else the read fails; in the maker note the tag is
    /// absent instead (R-SRW-59).
    pub(crate) fn bytes(&self, e: &Entry) -> SrwResult<Option<&'a [u8]>> {
        let size = self.data.len() as u64;
        if e.len > size {
            return Err(fail("tag value larger than the file"));
        }
        if e.at > size - e.len {
            if self.lenient {
                return Ok(None);
            }
            return Err(fail("tag value outside the file"));
        }
        Ok(Some(&self.data[e.at as usize..(e.at + e.len) as usize]))
    }

    /// The value bytes of `tag`, `Ok(None)` when absent.
    fn tag_bytes(&self, tag: u16) -> SrwResult<Option<(&Entry, &'a [u8])>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        Ok(self.bytes(e)?.map(|b| (e, b)))
    }

    /// All values of an unsigned integer tag: SHORT, LONG or type 13
    /// (R-SRW-8). `Ok(None)` when absent or of another type.
    pub(crate) fn uints(&self, tag: u16) -> SrwResult<Option<Vec<u32>>> {
        let Some((e, b)) = self.tag_bytes(tag)? else {
            return Ok(None);
        };
        let le = self.le;
        let v = match e.typ {
            3 => b
                .chunks_exact(2)
                .map(|c| {
                    u32::from(if le {
                        u16::from_le_bytes([c[0], c[1]])
                    } else {
                        u16::from_be_bytes([c[0], c[1]])
                    })
                })
                .collect(),
            4 | 13 => b.chunks_exact(4).map(|c| word(c, le)).collect(),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// The first value of an unsigned integer tag.
    pub(super) fn uint(&self, tag: u16) -> SrwResult<Option<u32>> {
        Ok(self.uints(tag)?.and_then(|v| v.first().copied()))
    }

    /// The values of a BYTE or UNDEFINED tag such as CFAPattern (R-SRW-8).
    pub(super) fn byte_values(&self, tag: u16) -> SrwResult<Option<&'a [u8]>> {
        let Some((e, b)) = self.tag_bytes(tag)? else {
            return Ok(None);
        };
        Ok(matches!(e.typ, 1 | 7).then_some(b))
    }

    /// The values of a LONG or SLONG maker-note tag (R-SRW-8), each as
    /// read by its type: LONG unsigned, SLONG signed.
    pub(super) fn longs(&self, tag: u16) -> SrwResult<Option<Vec<i64>>> {
        let Some((e, b)) = self.tag_bytes(tag)? else {
            return Ok(None);
        };
        let le = self.le;
        let v = match e.typ {
            4 => b.chunks_exact(4).map(|c| i64::from(word(c, le))).collect(),
            9 => b
                .chunks_exact(4)
                .map(|c| i64::from(word(c, le) as i32))
                .collect(),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// The stored elements of an encrypted maker-note tag, each read as a
    /// signed 32-bit value, also for type LONG (R-SRW-20).
    pub(super) fn signed(&self, tag: u16) -> SrwResult<Option<Vec<i64>>> {
        let Some((e, b)) = self.tag_bytes(tag)? else {
            return Ok(None);
        };
        if !matches!(e.typ, 4 | 9) {
            return Ok(None);
        }
        let le = self.le;
        Ok(Some(
            b.chunks_exact(4)
                .map(|c| i64::from(word(c, le) as i32))
                .collect(),
        ))
    }

    /// An ASCII value compared by its bytes before the first zero byte,
    /// after trailing spaces are removed (R-SRW-9).
    pub(crate) fn ascii(&self, tag: u16) -> SrwResult<Option<&'a [u8]>> {
        let Some((e, b)) = self.tag_bytes(tag)? else {
            return Ok(None);
        };
        if e.typ != 2 {
            return Ok(None);
        }
        let mut s = b.split(|&c| c == 0).next().unwrap_or(&[]);
        while let [rest @ .., b' '] = s {
            s = rest;
        }
        Ok(Some(s))
    }
}

/// A 32-bit word of a value in the given byte order (R-SRW-5).
fn word(c: &[u8], le: bool) -> u32 {
    let b = [c[0], c[1], c[2], c[3]];
    if le {
        u32::from_le_bytes(b)
    } else {
        u32::from_be_bytes(b)
    }
}

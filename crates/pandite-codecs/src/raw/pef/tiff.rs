//! The TIFF structure of a PEF file in either byte order: header, IFD
//! chain, entries and typed values, with every offset and length checked
//! against the file (sections Container layout and Error handling).

use super::{fail, PefResult};

/// Most IFDs followed along the main chain (R-PEF-11).
const MAX_CHAIN: usize = 16;
/// Largest entry count of a valid IFD (R-PEF-12).
const MAX_ENTRIES: u64 = 1024;

/// Byte size of one value of TIFF field type `typ` (TIFF 6.0 section 2).
/// `None` for types outside 1 to 12 (R-PEF-9).
fn type_size(typ: u16) -> Option<u64> {
    match typ {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// A 16-bit value at `at`, little-endian when `le` (R-PEF-5).
pub(super) fn u16_at(d: &[u8], at: u64, le: bool) -> Option<u16> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(2)?)?;
    Some(if le {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    })
}

/// A 32-bit value at `at`, little-endian when `le` (R-PEF-5).
pub(super) fn u32_at(d: &[u8], at: u64, le: bool) -> Option<u32> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(4)?)?;
    Some(if le {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    })
}

/// 16-bit values of a byte array in the given byte order.
pub(super) fn u16s(b: &[u8], le: bool) -> Vec<u16> {
    b.chunks_exact(2)
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect()
}

/// Check the file size and the header. Returns the byte order (true for
/// little-endian) and the IFD0 offset.
pub(super) fn header(data: &[u8]) -> PefResult<(bool, u32)> {
    // R-PEF-59: shorter than 8 bytes or longer than 2^32 - 1 bytes fails.
    if data.len() < 8 || data.len() as u64 > u64::from(u32::MAX) {
        return Err(fail("file size outside 8 bytes to 4 GiB"));
    }
    // R-PEF-4: only 4D 4D 00 2A and 49 49 2A 00 are accepted.
    let le = match data[..4] {
        [0x4D, 0x4D, 0x00, 0x2A] => false,
        [0x49, 0x49, 0x2A, 0x00] => true,
        _ => return Err(fail("not a TIFF header")),
    };
    Ok((le, u32_at(data, 4, le).unwrap_or(0)))
}

/// One IFD entry with the location of its value bytes resolved.
#[derive(Clone, Copy, Debug)]
pub(super) struct Entry {
    /// Tag number.
    pub tag: u16,
    /// Field type, 1 to 12.
    pub typ: u16,
    /// Number of values.
    pub count: u32,
    /// File offset of the value bytes.
    pub at: u64,
    /// Length of the value bytes: count times the type size.
    pub len: u64,
}

/// One IFD: its entries in file order, its byte order and the next-IFD
/// offset.
#[derive(Clone, Debug)]
pub(super) struct Dir<'a> {
    data: &'a [u8],
    /// True for a little-endian IFD.
    pub le: bool,
    /// The entries with a field type from 1 to 12, in file order.
    pub entries: Vec<Entry>,
    /// The offset of the next IFD, 0 at the end of the chain.
    pub next: u32,
}

/// Read the IFD at `offset` in the byte order `le`, with out-of-line value
/// offsets counted from `origin` (R-PEF-6, R-PEF-16). `None` when the IFD
/// is invalid (R-PEF-12).
pub(super) fn dir(data: &[u8], offset: u64, le: bool, origin: u64) -> Option<Dir<'_>> {
    let size = data.len() as u64;
    // R-PEF-12 case 1: the offset lies outside the file.
    if offset >= size {
        return None;
    }
    let n = u64::from(u16_at(data, offset, le)?);
    // R-PEF-12 case 2: N is 0 or greater than 1024.
    if n == 0 || n > MAX_ENTRIES {
        return None;
    }
    // R-PEF-12 case 3: the 2 + 12 * N + 4 bytes lie inside the file. All
    // sums are 64-bit (R-PEF-59).
    if offset + 2 + 12 * n + 4 > size {
        return None;
    }
    let mut entries = Vec::with_capacity(n as usize);
    for i in 0..n {
        let at = offset + 2 + 12 * i;
        let tag = u16_at(data, at, le)?;
        let typ = u16_at(data, at + 2, le)?;
        let count = u32_at(data, at + 4, le)?;
        // R-PEF-9: skip entries whose type is outside 1 to 12.
        let Some(unit) = type_size(typ) else {
            continue;
        };
        let len = u64::from(count) * unit;
        // R-PEF-6: inline when count * size <= 4, else at the offset in the
        // value field, counted from the origin of this IFD.
        let value_at = if len <= 4 {
            at + 8
        } else {
            origin + u64::from(u32_at(data, at + 8, le)?)
        };
        entries.push(Entry {
            tag,
            typ,
            count,
            at: value_at,
            len,
        });
    }
    let next = u32_at(data, offset + 2 + 12 * n, le).unwrap_or(0);
    Some(Dir {
        data,
        le,
        entries,
        next,
    })
}

/// IFD0 and the IFDs that follow it through the next-IFD offsets
/// (R-PEF-11). Empty when IFD0 itself is invalid.
pub(super) fn chain(data: &[u8], first: u32, le: bool) -> Vec<Dir<'_>> {
    let mut out: Vec<Dir<'_>> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let mut at = first;
    // R-PEF-11: stop at offset 0, an invalid IFD, a repeated offset or
    // after 16 IFDs.
    while at != 0 && out.len() < MAX_CHAIN && !seen.contains(&at) {
        seen.push(at);
        let Some(d) = dir(data, u64::from(at), le, 0) else {
            break;
        };
        at = d.next;
        out.push(d);
    }
    out
}

impl<'a> Dir<'a> {
    /// The first entry of `tag`, in any entry order (R-PEF-7). An entry
    /// with a count of 0 counts as absent (R-PEF-18).
    pub(super) fn entry(&self, tag: u16) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.tag == tag)
            .filter(|e| e.count > 0)
    }

    /// True when `tag` is present with at least one value.
    pub(super) fn has(&self, tag: u16) -> bool {
        self.entry(tag).is_some()
    }

    /// True when the value bytes of `e` lie completely inside the file
    /// (R-PEF-62).
    pub(super) fn inside(&self, e: &Entry) -> bool {
        let size = self.data.len() as u64;
        e.len <= size && e.at <= size - e.len
    }

    /// The value bytes of an entry. Fails when the value is larger than the
    /// file (R-PEF-61), before any allocation. `None` when the value lies
    /// outside the file (R-PEF-62): it is not read, and its tag counts as
    /// invalid.
    pub(super) fn bytes(&self, e: &Entry) -> PefResult<Option<&'a [u8]>> {
        let size = self.data.len() as u64;
        if e.len > size {
            return Err(fail("tag value larger than the file"));
        }
        if !self.inside(e) {
            return Ok(None);
        }
        Ok(Some(&self.data[e.at as usize..(e.at + e.len) as usize]))
    }

    /// All values of an integer tag, SHORT or LONG (R-PEF-8). `Ok(None)`
    /// when absent, of another type or outside the file.
    pub(super) fn uints(&self, tag: u16) -> PefResult<Option<Vec<u32>>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        if !matches!(e.typ, 3 | 4) {
            return Ok(None);
        }
        let Some(b) = self.bytes(e)? else {
            return Ok(None);
        };
        Ok(Some(if e.typ == 3 {
            u16s(b, self.le).into_iter().map(u32::from).collect()
        } else {
            b.chunks_exact(4)
                .map(|c| {
                    let w = [c[0], c[1], c[2], c[3]];
                    if self.le {
                        u32::from_le_bytes(w)
                    } else {
                        u32::from_be_bytes(w)
                    }
                })
                .collect()
        }))
    }

    /// The first value of an integer tag (R-PEF-70).
    pub(super) fn uint(&self, tag: u16) -> PefResult<Option<u32>> {
        Ok(self.uints(tag)?.and_then(|v| v.first().copied()))
    }

    /// The value bytes of a byte-array tag, BYTE or UNDEFINED (R-PEF-8).
    /// `Ok(None)` when absent, of another type or outside the file.
    pub(super) fn byte_array(&self, tag: u16) -> PefResult<Option<&'a [u8]>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        if !matches!(e.typ, 1 | 7) {
            return Ok(None);
        }
        self.bytes(e)
    }

    /// An ASCII value with all trailing zero bytes and spaces removed
    /// (R-PEF-10).
    pub(super) fn ascii(&self, tag: u16) -> PefResult<Option<&'a [u8]>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        if e.typ != 2 {
            return Ok(None);
        }
        let Some(mut b) = self.bytes(e)? else {
            return Ok(None);
        };
        while let [rest @ .., 0 | b' '] = b {
            b = rest;
        }
        Ok(Some(b))
    }
}

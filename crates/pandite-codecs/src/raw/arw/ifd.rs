//! The TIFF structure of an ARW file: header, IFD chain, entries and
//! typed values, with every offset and length checked against the file.

use super::{fail, ArwResult};

/// Most IFDs followed along the main chain (R-ARW-12).
const MAX_CHAIN: usize = 16;
/// Largest entry count of a valid IFD (R-ARW-14).
const MAX_ENTRIES: u64 = 1024;

/// Byte size of one value of TIFF field type `typ` (TIFF 6.0 section 2;
/// type 13 is the IFD type of R-ARW-8). `None` for types outside 1 to 13.
fn type_size(typ: u16) -> Option<u64> {
    match typ {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 | 13 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// One IFD entry with the location of its value bytes resolved.
#[derive(Clone, Copy, Debug)]
pub(in crate::raw) struct Entry {
    /// Tag number.
    pub tag: u16,
    /// Field type, 1 to 13.
    pub typ: u16,
    /// Number of values.
    pub count: u32,
    /// File offset of the value bytes.
    pub at: u64,
    /// Length of the value bytes: count times the type size.
    pub len: u64,
}

/// One IFD: its entries in file order and the next-IFD offset.
#[derive(Clone, Debug)]
pub(in crate::raw) struct Dir<'a> {
    data: &'a [u8],
    /// The entries, in file order.
    pub entries: Vec<Entry>,
    /// The offset of the next IFD of the chain, 0 at the end.
    pub next: u32,
}

/// Check the header (R-ARW-65, R-ARW-4) and return the IFD0 offset
/// (R-ARW-11). `Ok(None)` means "not a little-endian TIFF": a big-endian
/// file (X6) or other data.
pub(super) fn header(data: &[u8]) -> ArwResult<Option<u32>> {
    // R-ARW-65: shorter than 8 bytes or longer than 2^32 - 1 bytes fails.
    if data.len() < 8 || data.len() as u64 > u64::from(u32::MAX) {
        return Err(fail("file size outside 8 bytes to 4 GiB"));
    }
    // R-ARW-4: only 49 49 2A 00 is accepted; 4D 4D 00 2A is X6.
    if data[..4] != [0x49, 0x49, 0x2A, 0x00] {
        return Ok(None);
    }
    Ok(Some(le32(data, 4).unwrap_or(0)))
}

/// Read a little-endian u16 (R-ARW-5).
pub(super) fn le16(d: &[u8], at: u64) -> Option<u16> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

/// Read a little-endian u32 (R-ARW-5).
pub(in crate::raw) fn le32(d: &[u8], at: u64) -> Option<u32> {
    let at = usize::try_from(at).ok()?;
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read the IFD at `offset`, or `None` when it is invalid (R-ARW-14).
pub(in crate::raw) fn dir(data: &[u8], offset: u64) -> Option<Dir<'_>> {
    let size = data.len() as u64;
    // R-ARW-14 case 1: the offset lies outside the file.
    if offset >= size {
        return None;
    }
    let n = u64::from(le16(data, offset)?);
    // R-ARW-14 case 2: N is 0 or greater than 1024.
    if n == 0 || n > MAX_ENTRIES {
        return None;
    }
    // R-ARW-14 case 3: the 2 + 12 * N + 4 bytes lie inside the file (64-bit
    // arithmetic, R-ARW-65).
    if offset + 2 + 12 * n + 4 > size {
        return None;
    }
    let mut entries = Vec::with_capacity(n as usize);
    for i in 0..n {
        let at = offset + 2 + 12 * i;
        let tag = le16(data, at)?;
        let typ = le16(data, at + 2)?;
        let count = le32(data, at + 4)?;
        // R-ARW-9: skip entries whose type is outside 1 to 13.
        let Some(unit) = type_size(typ) else {
            continue;
        };
        let len = u64::from(count) * unit;
        // R-ARW-6: inline when count * size <= 4, else at the offset in
        // the value field.
        let value_at = if len <= 4 {
            at + 8
        } else {
            u64::from(le32(data, at + 8)?)
        };
        entries.push(Entry {
            tag,
            typ,
            count,
            at: value_at,
            len,
        });
    }
    let next = le32(data, offset + 2 + 12 * n).unwrap_or(0);
    Some(Dir {
        data,
        entries,
        next,
    })
}

/// IFD0 and the IFDs that follow it through the next-IFD offsets
/// (R-ARW-12). Empty when IFD0 itself is invalid.
pub(in crate::raw) fn chain(data: &[u8], first: u32) -> Vec<Dir<'_>> {
    let mut out: Vec<Dir<'_>> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let mut at = first;
    // R-ARW-12: stop at offset 0, an invalid IFD, a repeated offset or
    // after 16 IFDs.
    while at != 0 && out.len() < MAX_CHAIN && !seen.contains(&at) {
        seen.push(at);
        let Some(d) = dir(data, u64::from(at)) else {
            break;
        };
        at = d.next;
        out.push(d);
    }
    out
}

impl<'a> Dir<'a> {
    /// The entry of `tag` with at least one value, in any entry order
    /// (R-ARW-7). A count of 0 counts as absent (R-ARW-16).
    pub(in crate::raw) fn entry(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag && e.count > 0)
    }

    /// True when `tag` is present with at least one value.
    pub(in crate::raw) fn has(&self, tag: u16) -> bool {
        self.entry(tag).is_some()
    }

    /// The value bytes of an entry, after checking that the whole range
    /// lies inside the file (R-ARW-67, R-ARW-68). Fails otherwise.
    pub(in crate::raw) fn bytes(&self, e: &Entry) -> ArwResult<&'a [u8]> {
        let size = self.data.len() as u64;
        // R-ARW-67: count * type size beyond the file size fails before
        // any allocation; R-ARW-68: the range must lie inside the file.
        if e.len > size || e.at > size - e.len {
            return Err(fail("tag value outside the file"));
        }
        Ok(&self.data[e.at as usize..(e.at + e.len) as usize])
    }

    /// All values of an unsigned integer tag. TIFF 6.0 readers accept
    /// BYTE, SHORT and LONG for unsigned integer fields; R-ARW-8 adds the
    /// IFD type. `Ok(None)` when absent or of another type.
    pub(in crate::raw) fn uints(&self, tag: u16) -> ArwResult<Option<Vec<u32>>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        let b = self.bytes(e)?;
        let v = match e.typ {
            1 => b.iter().map(|&x| u32::from(x)).collect(),
            3 => b
                .chunks_exact(2)
                .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
                .collect(),
            4 | 13 => b
                .chunks_exact(4)
                .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect(),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// The first value of an unsigned integer tag.
    pub(in crate::raw) fn uint(&self, tag: u16) -> ArwResult<Option<u32>> {
        Ok(self.uints(tag)?.and_then(|v| v.first().copied()))
    }

    /// The values of a signed 16-bit tag such as WB_RGGBLevels (SSHORT,
    /// also accepted as SHORT).
    pub(super) fn sshorts(&self, tag: u16) -> ArwResult<Option<Vec<i32>>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        let b = self.bytes(e)?;
        let v = match e.typ {
            8 => b
                .chunks_exact(2)
                .map(|c| i32::from(i16::from_le_bytes([c[0], c[1]])))
                .collect(),
            3 => b
                .chunks_exact(2)
                .map(|c| i32::from(u16::from_le_bytes([c[0], c[1]])))
                .collect(),
            _ => return Ok(None),
        };
        Ok(Some(v))
    }

    /// The values of a RATIONAL tag, or of a tag that may be SHORT, LONG or
    /// RATIONAL. A RATIONAL with denominator 0 is `None` (invalid,
    /// R-ARW-17).
    pub(super) fn reals(&self, tag: u16) -> ArwResult<Option<Vec<Option<f64>>>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        let b = self.bytes(e)?;
        let v = match e.typ {
            5 => b
                .chunks_exact(8)
                .map(|c| {
                    let n = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let d = u32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    (d != 0).then(|| f64::from(n) / f64::from(d))
                })
                .collect(),
            10 => b
                .chunks_exact(8)
                .map(|c| {
                    let n = i32::from_le_bytes([c[0], c[1], c[2], c[3]]);
                    let d = i32::from_le_bytes([c[4], c[5], c[6], c[7]]);
                    (d != 0).then(|| f64::from(n) / f64::from(d))
                })
                .collect(),
            _ => match self.uints(tag)? {
                Some(u) => u.into_iter().map(|x| Some(f64::from(x))).collect(),
                None => return Ok(None),
            },
        };
        Ok(Some(v))
    }

    /// An ASCII value with all trailing zero bytes and spaces removed
    /// (R-ARW-10).
    pub(in crate::raw) fn ascii(&self, tag: u16) -> ArwResult<Option<&'a [u8]>> {
        let Some(e) = self.entry(tag) else {
            return Ok(None);
        };
        if e.typ != 2 {
            return Ok(None);
        }
        let mut b = self.bytes(e)?;
        while let [rest @ .., 0 | b' '] = b {
            b = rest;
        }
        Ok(Some(b))
    }
}

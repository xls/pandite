//! A small, bounds-checked TIFF structure reader.
//!
//! It serves three jobs: reading the EXIF orientation tag from JPEG, PNG,
//! WebP and other containers, telling camera RAW files apart from plain TIFF
//! files, and finding embedded JPEG previews in RAW files. It never
//! allocates more than the entries it returns and never follows an offset
//! outside the data.

use pandite_core::Orientation;

/// EXIF tag numbers used by the codecs.
pub(crate) mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 254;
    pub const IMAGE_WIDTH: u16 = 256;
    pub const IMAGE_LENGTH: u16 = 257;
    pub const COMPRESSION: u16 = 259;
    pub const PHOTOMETRIC: u16 = 262;
    pub const STRIP_OFFSETS: u16 = 273;
    pub const ORIENTATION: u16 = 274;
    pub const STRIP_BYTE_COUNTS: u16 = 279;
    pub const SUB_IFDS: u16 = 330;
    pub const EXIF_IFD: u16 = 34665;
    pub const DNG_VERSION: u16 = 50706;
}

/// Most IFDs the walker visits in one file. Stops reference loops.
const MAX_IFDS: usize = 64;
/// Most entries read from one IFD.
const MAX_ENTRIES: usize = 4096;

/// One IFD entry with its value location resolved.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Entry {
    pub tag: u16,
    pub typ: u16,
    pub count: u32,
    /// Absolute offset of the value bytes in the data.
    pub value_at: usize,
}

/// A TIFF byte stream: byte order plus the data it lives in.
#[derive(Clone, Copy)]
pub(crate) struct Tiff<'a> {
    data: &'a [u8],
    le: bool,
}

impl<'a> Tiff<'a> {
    /// Parse a TIFF header at the start of `data`. Accepts the standard
    /// magic 42 and the variants used by Olympus (`RO`, `RS`, `OR`) and
    /// Panasonic (`U`) raw files.
    pub(crate) fn new(data: &'a [u8]) -> Option<Self> {
        let le = match data.get(..2)? {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let t = Tiff { data, le };
        let magic = t.u16(2)?;
        match magic {
            42 | 0x4f52 | 0x5352 | 0x0055 => Some(t),
            _ => None,
        }
    }

    /// Parse EXIF payload, with or without the `Exif\0\0` prefix.
    pub(crate) fn from_exif(data: &'a [u8]) -> Option<Self> {
        let data = data.strip_prefix(b"Exif\0\0").unwrap_or(data);
        Self::new(data)
    }

    /// The underlying bytes.
    pub(crate) fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Read a u16 in file byte order.
    pub(crate) fn u16(&self, pos: usize) -> Option<u16> {
        let b = self.data.get(pos..pos.checked_add(2)?)?;
        Some(if self.le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    /// Read a u32 in file byte order.
    pub(crate) fn u32(&self, pos: usize) -> Option<u32> {
        let b = self.data.get(pos..pos.checked_add(4)?)?;
        Some(if self.le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    /// Offset of the first IFD.
    pub(crate) fn first_ifd(&self) -> Option<usize> {
        let off = self.u32(4)? as usize;
        (off >= 8 && off < self.data.len()).then_some(off)
    }

    /// The entries of the IFD at `offset` and the offset of the next IFD
    /// (zero when none).
    pub(crate) fn ifd(&self, offset: usize) -> Option<(Vec<Entry>, usize)> {
        let n = usize::from(self.u16(offset)?);
        if n == 0 || n > MAX_ENTRIES {
            return None;
        }
        let mut entries = Vec::with_capacity(n);
        for i in 0..n {
            let at = offset + 2 + i * 12;
            let tag = self.u16(at)?;
            let typ = self.u16(at + 2)?;
            let count = self.u32(at + 4)?;
            let size = type_size(typ).checked_mul(count as usize)?;
            let value_at = if size <= 4 {
                at + 8
            } else {
                self.u32(at + 8)? as usize
            };
            if value_at.checked_add(size)? > self.data.len() {
                continue;
            }
            entries.push(Entry {
                tag,
                typ,
                count,
                value_at,
            });
        }
        let next = self.u32(offset + 2 + n * 12).unwrap_or(0) as usize;
        let next = if next >= 8 && next < self.data.len() && next != offset {
            next
        } else {
            0
        };
        Some((entries, next))
    }

    /// Value `index` of an integer entry (BYTE, SHORT, LONG and signed kinds).
    pub(crate) fn value(&self, e: &Entry, index: u32) -> Option<u32> {
        if index >= e.count {
            return None;
        }
        let i = index as usize;
        match e.typ {
            1 | 6 | 7 => self.data.get(e.value_at + i).map(|&b| u32::from(b)),
            3 | 8 => self.u16(e.value_at + 2 * i).map(u32::from),
            4 | 9 | 13 => self.u32(e.value_at + 4 * i),
            _ => None,
        }
    }

    /// The first value of `tag` in `entries`.
    pub(crate) fn get(&self, entries: &[Entry], tag: u16) -> Option<u32> {
        let e = entries.iter().find(|e| e.tag == tag)?;
        self.value(e, 0)
    }

    /// All integer values of `tag`, at most `limit` of them.
    pub(crate) fn values(&self, entries: &[Entry], tag: u16, limit: usize) -> Vec<u32> {
        let Some(e) = entries.iter().find(|e| e.tag == tag) else {
            return Vec::new();
        };
        (0..e.count.min(limit as u32))
            .filter_map(|i| self.value(e, i))
            .collect()
    }

    /// The bytes of an entry's value.
    pub(crate) fn bytes(&self, e: &Entry) -> Option<&'a [u8]> {
        let size = type_size(e.typ).checked_mul(e.count as usize)?;
        self.data.get(e.value_at..e.value_at.checked_add(size)?)
    }

    /// Visit IFD0, the chained IFDs, SubIFDs and the EXIF IFD, depth first.
    /// The callback receives each IFD's entries.
    pub(crate) fn walk(&self, mut visit: impl FnMut(&[Entry])) {
        let mut stack: Vec<usize> = Vec::new();
        let mut seen: Vec<usize> = Vec::new();
        if let Some(first) = self.first_ifd() {
            stack.push(first);
        }
        while let Some(off) = stack.pop() {
            if seen.contains(&off) || seen.len() >= MAX_IFDS {
                continue;
            }
            seen.push(off);
            let Some((entries, next)) = self.ifd(off) else {
                continue;
            };
            visit(&entries);
            if next != 0 {
                stack.push(next);
            }
            for t in [tag::SUB_IFDS, tag::EXIF_IFD] {
                for sub in self.values(&entries, t, 16) {
                    let sub = sub as usize;
                    if sub >= 8 && sub < self.data.len() {
                        stack.push(sub);
                    }
                }
            }
        }
    }

    /// The orientation tag of IFD0.
    pub(crate) fn orientation(&self) -> Orientation {
        let Some(first) = self.first_ifd() else {
            return Orientation::Normal;
        };
        let Some((entries, _)) = self.ifd(first) else {
            return Orientation::Normal;
        };
        self.get(&entries, tag::ORIENTATION)
            .map(|v| Orientation::from_exif(v as u16))
            .unwrap_or_default()
    }
}

/// Byte size of one value of a TIFF field type. Unknown types count as 1.
fn type_size(typ: u16) -> usize {
    match typ {
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        16..=18 => 8,
        _ => 1,
    }
}

/// The EXIF orientation stored in an EXIF payload (with or without the
/// `Exif\0\0` prefix). `Normal` when absent or unreadable.
pub(crate) fn orientation_from_exif(data: &[u8]) -> Orientation {
    Tiff::from_exif(data)
        .map(|t| t.orientation())
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A minimal little-endian EXIF block holding only an orientation tag.
    pub(crate) fn exif_with_orientation(value: u16, prefix: bool) -> Vec<u8> {
        let mut v = Vec::new();
        if prefix {
            v.extend_from_slice(b"Exif\0\0");
        }
        v.extend_from_slice(b"II*\0");
        v.extend_from_slice(&8u32.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&tag::ORIENTATION.to_le_bytes());
        v.extend_from_slice(&3u16.to_le_bytes());
        v.extend_from_slice(&1u32.to_le_bytes());
        v.extend_from_slice(&value.to_le_bytes());
        v.extend_from_slice(&[0, 0]);
        v.extend_from_slice(&0u32.to_le_bytes());
        v
    }

    #[test]
    fn reads_orientation_in_both_byte_orders() {
        assert_eq!(
            orientation_from_exif(&exif_with_orientation(6, true)),
            Orientation::Rotate90
        );
        assert_eq!(
            orientation_from_exif(&exif_with_orientation(3, false)),
            Orientation::Rotate180
        );
        let mut be = Vec::new();
        be.extend_from_slice(b"MM\0*");
        be.extend_from_slice(&8u32.to_be_bytes());
        be.extend_from_slice(&1u16.to_be_bytes());
        be.extend_from_slice(&tag::ORIENTATION.to_be_bytes());
        be.extend_from_slice(&3u16.to_be_bytes());
        be.extend_from_slice(&1u32.to_be_bytes());
        be.extend_from_slice(&8u16.to_be_bytes());
        be.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        assert_eq!(orientation_from_exif(&be), Orientation::Rotate270);
    }

    #[test]
    fn hostile_exif_is_harmless() {
        assert_eq!(orientation_from_exif(b""), Orientation::Normal);
        assert_eq!(
            orientation_from_exif(b"II*\0\xff\xff\xff\xff"),
            Orientation::Normal
        );
        let mut looped = exif_with_orientation(6, false);
        let n = looped.len();
        looped[n - 4..].copy_from_slice(&8u32.to_le_bytes());
        let t = Tiff::new(&looped).unwrap();
        let mut visits = 0;
        t.walk(|_| visits += 1);
        assert_eq!(visits, 1);
        for cut in 0..looped.len() {
            let _ = orientation_from_exif(&looped[..cut]);
        }
    }
}

//! Synthetic RW2 and Panasonic RAW files for the tests: the
//! specification's fixtures F1 to F8 (section Test vectors, Fixture
//! recipes) and a builder that follows the recipe layout.

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};

/// One IFD0 value for the builder.
#[derive(Clone, Debug)]
pub(super) enum Val {
    /// SHORT, count 1.
    Short(u16),
    /// LONG, count 1.
    Long(u32),
    /// ASCII with a terminating zero byte.
    Ascii(&'static str),
    /// UNDEFINED, count = length, the bytes placed after the IFD values.
    Undefined(Vec<u8>),
    /// LONG: the file offset of the sensor data.
    DataOffset,
    /// An entry given by its raw fields: type, count, value field.
    Raw(u16, u32, u32),
}

/// A synthetic file following the recipe layout: the header, IFD0 at
/// `ifd_at`, the out-of-line values (ASCII first, then the UNDEFINED
/// streams at multiples of 16), then the sensor data at offset 512 or the
/// next multiple of 16 after the values.
#[derive(Clone, Debug)]
pub(super) struct Rw2 {
    /// 24 for RW2 files, 8 for RAW files.
    pub ifd_at: usize,
    /// IFD0 entries; written in ascending tag order.
    pub entries: Vec<(u16, Val)>,
    /// The sensor data.
    pub data: Vec<u8>,
}

impl Rw2 {
    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let mut sorted: Vec<&(u16, Val)> = self.entries.iter().collect();
        sorted.sort_by_key(|e| e.0);
        let n = sorted.len();
        let mut at = self.ifd_at + 2 + 12 * n + 4;
        // Places of the out-of-line values, ASCII strings first.
        let mut place = vec![0usize; n];
        for (i, (_, v)) in sorted.iter().enumerate() {
            if let Val::Ascii(s) = v {
                if s.len() + 1 > 4 {
                    place[i] = at;
                    at += (s.len() + 1).next_multiple_of(2);
                }
            }
        }
        for (i, (_, v)) in sorted.iter().enumerate() {
            if let Val::Undefined(b) = v {
                if b.len() > 4 {
                    at = at.next_multiple_of(16);
                    place[i] = at;
                    at += b.len();
                }
            }
        }
        let data_at = at.next_multiple_of(16).max(512);
        let mut out = vec![0u8; data_at];
        out[..4].copy_from_slice(&[0x49, 0x49, 0x55, 0x00]);
        out[4..8].copy_from_slice(&(self.ifd_at as u32).to_le_bytes());
        let mut e = self.ifd_at;
        out[e..e + 2].copy_from_slice(&(n as u16).to_le_bytes());
        e += 2;
        for (i, (tag, v)) in sorted.iter().enumerate() {
            let (typ, count, value): (u16, u32, [u8; 4]) = match v {
                Val::Short(x) => {
                    let b = x.to_le_bytes();
                    (3, 1, [b[0], b[1], 0, 0])
                }
                Val::Long(x) => (4, 1, x.to_le_bytes()),
                Val::DataOffset => (4, 1, (data_at as u32).to_le_bytes()),
                Val::Raw(t, c, x) => (*t, *c, x.to_le_bytes()),
                Val::Ascii(s) => {
                    let mut bytes = s.as_bytes().to_vec();
                    bytes.push(0);
                    if bytes.len() > 4 {
                        out[place[i]..place[i] + bytes.len()].copy_from_slice(&bytes);
                        (2, bytes.len() as u32, (place[i] as u32).to_le_bytes())
                    } else {
                        bytes.resize(4, 0);
                        (
                            2,
                            s.len() as u32 + 1,
                            [bytes[0], bytes[1], bytes[2], bytes[3]],
                        )
                    }
                }
                Val::Undefined(b) => {
                    if b.len() > 4 {
                        out[place[i]..place[i] + b.len()].copy_from_slice(b);
                        (7, b.len() as u32, (place[i] as u32).to_le_bytes())
                    } else {
                        let mut q = b.clone();
                        q.resize(4, 0);
                        (7, b.len() as u32, [q[0], q[1], q[2], q[3]])
                    }
                }
            };
            out[e..e + 2].copy_from_slice(&tag.to_le_bytes());
            out[e + 2..e + 4].copy_from_slice(&typ.to_le_bytes());
            out[e + 4..e + 8].copy_from_slice(&count.to_le_bytes());
            out[e + 8..e + 12].copy_from_slice(&value);
            e += 12;
        }
        // The next-IFD offset 0 is already zero.
        out.extend_from_slice(&self.data);
        out
    }
}

/// The 22 IFD0 entries common to F1 to F6, with W, H, D, K and RawFormat
/// (F = 2^D - 1).
pub(super) fn common(w: u16, h: u16, d: u16, k: u16, raw_format: u16) -> Vec<(u16, Val)> {
    let f = (1u16 << d) - 1;
    vec![
        (0x0002, Val::Short(w)),
        (0x0003, Val::Short(h)),
        (0x0004, Val::Short(0)),
        (0x0005, Val::Short(0)),
        (0x0006, Val::Short(h)),
        (0x0007, Val::Short(w)),
        (0x0009, Val::Short(1)),
        (0x000A, Val::Short(d)),
        (0x000B, Val::Short(34316)),
        (0x000E, Val::Short(f)),
        (0x000F, Val::Short(f)),
        (0x0010, Val::Short(f)),
        (0x001C, Val::Short(k)),
        (0x001D, Val::Short(k)),
        (0x001E, Val::Short(k)),
        (0x0024, Val::Short(600)),
        (0x0025, Val::Short(300)),
        (0x0026, Val::Short(450)),
        (0x002D, Val::Short(raw_format)),
        (0x0110, Val::Ascii("DC-GH5")),
        (0x0112, Val::Short(1)),
        (0x0118, Val::DataOffset),
    ]
}

/// Replace or add an entry.
pub(super) fn with(mut e: Vec<(u16, Val)>, tag: u16, v: Val) -> Vec<(u16, Val)> {
    e.retain(|x| x.0 != tag);
    e.push((tag, v));
    e
}

/// Remove an entry.
pub(super) fn without(mut e: Vec<(u16, Val)>, tag: u16) -> Vec<(u16, Val)> {
    e.retain(|x| x.0 != tag);
    e
}

/// Store a logical stream in whole chunks of 16384 bytes, rotated: logical
/// byte i of a chunk at chunk offset (i + 8184) mod 16384 (section Chunk
/// rotation). The rest of the last chunk is 0.
pub(super) fn rotate(logical: &[u8]) -> Vec<u8> {
    let chunks = logical.len().div_ceil(16384).max(1);
    let mut out = vec![0u8; chunks * 16384];
    for (l, &b) in logical.iter().enumerate() {
        let (c, i) = (l / 16384, l % 16384);
        out[c * 16384 + (i + 8184) % 16384] = b;
    }
    out
}

/// F1 header, IFD0 and Model string (bytes 0x000 to 0x12F), as printed in
/// the specification.
pub(super) const F1_HEAD_HEX: &str = "
0000: 49 49 55 00 18 00 00 00 00 00 00 00 00 00 00 00
0010: 00 00 00 00 00 00 00 00 16 00 02 00 03 00 01 00
0020: 00 00 1C 00 00 00 03 00 03 00 01 00 00 00 02 00
0030: 00 00 04 00 03 00 01 00 00 00 00 00 00 00 05 00
0040: 03 00 01 00 00 00 00 00 00 00 06 00 03 00 01 00
0050: 00 00 02 00 00 00 07 00 03 00 01 00 00 00 1C 00
0060: 00 00 09 00 03 00 01 00 00 00 01 00 00 00 0A 00
0070: 03 00 01 00 00 00 0C 00 00 00 0B 00 03 00 01 00
0080: 00 00 0C 86 00 00 0E 00 03 00 01 00 00 00 FF 0F
0090: 00 00 0F 00 03 00 01 00 00 00 FF 0F 00 00 10 00
00A0: 03 00 01 00 00 00 FF 0F 00 00 1C 00 03 00 01 00
00B0: 00 00 80 00 00 00 1D 00 03 00 01 00 00 00 80 00
00C0: 00 00 1E 00 03 00 01 00 00 00 80 00 00 00 24 00
00D0: 03 00 01 00 00 00 58 02 00 00 25 00 03 00 01 00
00E0: 00 00 2C 01 00 00 26 00 03 00 01 00 00 00 C2 01
00F0: 00 00 2D 00 03 00 01 00 00 00 04 00 00 00 10 01
0100: 02 00 07 00 00 00 26 01 00 00 12 01 03 00 01 00
0110: 00 00 01 00 00 00 18 01 04 00 01 00 00 00 00 02
0120: 00 00 00 00 00 00 44 43 2D 47 48 35 00 00 00 00
";

/// F1 logical bytes 0 to 63, at file offsets 8696 to 8759, as printed.
pub(super) const F1_DATA_HEX: &str = "
21F8: 80 80 80 00 02 02 02 08 08 08 20 20 20 47 B8 46
2208: 80 80 80 00 02 02 02 08 08 08 20 20 20 47 B8 46
2218: 80 80 80 00 02 02 02 08 08 08 20 20 20 6B 74 84
2228: 80 80 80 00 02 02 02 08 08 08 20 20 20 6B 74 84
";

/// The four blocks of F1 (and F5): rows 0 and 1, two blocks each.
pub(super) fn f1_blocks() -> Vec<u8> {
    hex(F1_DATA_HEX)
}

/// The four blocks of F3 (and F7): (1131, 2119) five times in row 0,
/// (2119, 1131) five times in row 1.
pub(super) fn f3_blocks() -> Vec<u8> {
    let row0 = hex("6B 74 84 6B 74 84 6B 74 84 6B 74 84 6B 74 84 00");
    let row1 = hex("47 B8 46 47 B8 46 47 B8 46 47 B8 46 47 B8 46 00");
    [row0.clone(), row0, row1.clone(), row1].concat()
}

/// The four blocks of F2 (and F6).
pub(super) fn f2_blocks() -> Vec<u8> {
    hex("80 11 40 08 18 01 84 80 11 40 08 18 01 84 80 11
         00 21 60 04 10 02 46 00 21 60 04 10 02 46 00 21
         00 21 60 04 10 02 46 00 21 60 04 10 02 46 00 21
         80 11 40 08 18 01 84 80 11 40 08 18 01 84 80 11")
}

/// The four blocks of F4.
pub(super) fn f4_blocks() -> Vec<u8> {
    hex("00 20 80 00 02 20 80 00 02 20 80 00 02 10 02 46
         00 20 80 00 02 20 80 00 02 20 80 00 02 18 01 84
         00 20 80 00 02 20 80 00 02 20 80 00 02 18 01 84
         00 20 80 00 02 20 80 00 02 20 80 00 02 10 02 46")
}

/// The F8 sensor data: row 0 R G R G, row 1 G B G B.
pub(super) fn f8_data() -> Vec<u8> {
    hex("B0 46 70 84 B0 46 70 84 70 84 B0 46 70 84 B0 46")
}

/// An RW2 file (IFD0 at 24) with the given entries and data.
pub(super) fn rw2(entries: Vec<(u16, Val)>, data: Vec<u8>) -> Vec<u8> {
    Rw2 {
        ifd_at: 24,
        entries,
        data,
    }
    .build()
}

/// A RAW file (IFD0 at 8) with the given entries and data.
pub(super) fn raw(entries: Vec<(u16, Val)>, data: Vec<u8>) -> Vec<u8> {
    Rw2 {
        ifd_at: 8,
        entries,
        data,
    }
    .build()
}

/// F1 entries: V1, W 28, H 2, D 12, K 128, RawFormat 4.
pub(super) fn f1_entries() -> Vec<(u16, Val)> {
    common(28, 2, 12, 128, 4)
}

/// F1 (V1): one rotated chunk, file length 16896.
pub(super) fn f1() -> Vec<u8> {
    rw2(f1_entries(), rotate(&f1_blocks()))
}

/// F2 entries: V3, W 18, D 14, K 497, RawFormat 5.
pub(super) fn f2_entries() -> Vec<(u16, Val)> {
    common(18, 2, 14, 497, 5)
}

/// F2 (V3): one rotated chunk, file length 16896.
pub(super) fn f2() -> Vec<u8> {
    rw2(f2_entries(), rotate(&f2_blocks()))
}

/// F3 (V2): W 20, RawFormat 5, one rotated chunk, file length 16896.
pub(super) fn f3() -> Vec<u8> {
    rw2(common(20, 2, 12, 128, 5), rotate(&f3_blocks()))
}

/// F4 (V5): W 22, D 14, K 497, RawFormat 6, no rotation, length 576.
pub(super) fn f4() -> Vec<u8> {
    rw2(common(22, 2, 14, 497, 6), f4_blocks())
}

/// F5 (V4): the F1 IFD0 with RawFormat 6, the F1 blocks without
/// rotation, length 576.
pub(super) fn f5() -> Vec<u8> {
    rw2(common(28, 2, 12, 128, 6), f1_blocks())
}

/// F6 (V6): the F2 IFD0 with RawFormat 7, the F2 blocks without
/// rotation, length 576.
pub(super) fn f6() -> Vec<u8> {
    rw2(common(18, 2, 14, 497, 7), f2_blocks())
}

/// F7 entries: the F1 entries with W 20, K 0, RawFormat 2, StripOffsets in
/// place of RawDataOffset, and StripByteCounts 64.
pub(super) fn f7_entries() -> Vec<(u16, Val)> {
    let e = without(common(20, 2, 12, 0, 2), 0x0118);
    let e = with(e, 0x0111, Val::DataOffset);
    with(e, 0x0117, Val::Long(64))
}

/// F7 (V7, RAW): IFD0 at 8 with 23 entries, the F3 blocks, length 576.
pub(super) fn f7() -> Vec<u8> {
    raw(f7_entries(), f3_blocks())
}

/// F8 entries: as F7 with W 4, Compression 34828, no RawFormat and
/// StripByteCounts 16.
pub(super) fn f8_entries() -> Vec<(u16, Val)> {
    let e = without(f7_entries(), 0x002D);
    let e = with(e, 0x0002, Val::Short(4));
    let e = with(e, 0x0007, Val::Short(4));
    let e = with(e, 0x000B, Val::Short(34828));
    with(e, 0x0117, Val::Long(16))
}

/// F8 (V8, RAW): IFD0 at 8 with 22 entries, 16 data bytes, length 528.
pub(super) fn f8() -> Vec<u8> {
    raw(f8_entries(), f8_data())
}

/// Add JpgFromRaw (0x002E) and JpgFromRaw2 (0x0127) streams.
pub(super) fn with_previews(e: Vec<(u16, Val)>, a: Vec<u8>, b: Vec<u8>) -> Vec<(u16, Val)> {
    let e = with(e, 0x002E, Val::Undefined(a));
    with(e, 0x0127, Val::Undefined(b))
}

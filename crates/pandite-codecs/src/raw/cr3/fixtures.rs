//! Synthetic CR3 files for the tests: the fixtures F1 to F3 of the
//! specification (section Fixture recipes), the one-component JPEG streams
//! of TV-14, and a builder for other layouts.

pub(crate) use crate::raw::arw::fixtures::hex;

use super::boxes::{CANON_UUID, PREVIEW_UUID};

/// The file type box of TV-1.
pub(crate) const FTYP: [u8; 24] = [
    0x00, 0x00, 0x00, 0x18, 0x66, 0x74, 0x79, 0x70, 0x63, 0x72, 0x78, 0x20, 0x00, 0x00, 0x00, 0x01,
    0x63, 0x72, 0x78, 0x20, 0x69, 0x73, 0x6F, 0x6D,
];

/// The Canon field of a CRAW entry (entry offset 86, M4).
pub(crate) const JPEG_FIELD: [u8; 4] = [0, 3, 0, 0];
pub(crate) const CRX_FIELD: [u8; 4] = [0, 1, 0, 1];
pub(crate) const HEVC_FIELD: [u8; 4] = [0, 4, 0, 0];

/// Fixture F1, the complete 928-byte file of the specification.
pub(crate) const F1_HEX: &str = "
0000: 00 00 00 18 66 74 79 70 63 72 78 20 00 00 00 01
0010: 63 72 78 20 69 73 6F 6D 00 00 02 20 6D 6F 6F 76
0020: 00 00 01 31 75 75 69 64 85 C0 B6 87 82 0F 11 E0
0030: 81 11 F4 CE 46 2B 6A 48 00 00 00 26 43 4E 43 56
0040: 43 61 6E 6F 6E 43 52 33 5F 30 30 31 2F 30 30 2E
0050: 31 30 2E 30 30 2F 30 30 2E 30 30 2E 30 30 00 00
0060: 00 4E 43 4D 54 31 49 49 2A 00 08 00 00 00 03 00
0070: 0F 01 02 00 06 00 00 00 32 00 00 00 10 01 02 00
0080: 0D 00 00 00 38 00 00 00 12 01 03 00 01 00 00 00
0090: 06 00 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61
00A0: 6E 6F 6E 20 45 4F 53 20 52 36 00 00 00 00 00 A5
00B0: 54 48 4D 42 00 00 00 00 00 08 00 08 00 00 00 8D
00C0: 00 01 00 00 FF D8 FF DB 00 43 00 01 01 01 01 01
00D0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
00E0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
00F0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0100: 01 01 01 01 01 01 01 01 01 01 01 FF C0 00 0B 08
0110: 00 08 00 08 01 01 11 00 FF C4 00 14 00 01 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 FF C4
0130: 00 14 10 01 00 00 00 00 00 00 00 00 00 00 00 00
0140: 00 00 00 00 FF DA 00 08 01 01 00 00 3F 00 3F FF
0150: D9 00 00 00 E7 74 72 61 6B 00 00 00 DF 6D 64 69
0160: 61 00 00 00 21 68 64 6C 72 00 00 00 00 00 00 00
0170: 00 76 69 64 65 00 00 00 00 00 00 00 00 00 00 00
0180: 00 00 00 00 00 B6 6D 69 6E 66 00 00 00 AE 73 74
0190: 62 6C 00 00 00 76 73 74 73 64 00 00 00 00 00 00
01A0: 00 01 00 00 00 66 43 52 41 57 00 00 00 00 00 00
01B0: 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01C0: 00 00 00 20 00 10 00 48 00 00 00 48 00 00 00 00
01D0: 00 00 00 01 00 00 00 00 00 00 00 00 00 00 00 00
01E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01F0: 00 00 00 00 00 18 FF FF 00 03 00 00 00 00 00 0C
0200: 4A 50 45 47 00 00 00 00 00 00 00 18 73 74 73 7A
0210: 00 00 00 00 00 00 00 00 00 00 00 01 00 00 00 91
0220: 00 00 00 18 63 6F 36 34 00 00 00 00 00 00 00 01
0230: 00 00 00 00 00 00 03 0F 00 00 00 C7 75 75 69 64
0240: EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16
0250: 00 00 00 00 00 00 00 01 00 00 00 A7 50 52 56 57
0260: 00 00 00 00 00 01 00 10 00 08 00 01 00 00 00 8F
0270: FF D8 FF DB 00 43 00 01 01 01 01 01 01 01 01 01
0280: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0290: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
02A0: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
02B0: 01 01 01 01 01 01 01 FF C0 00 0B 08 00 08 00 10
02C0: 01 01 11 00 FF C4 00 15 00 01 01 00 00 00 00 00
02D0: 00 00 00 00 00 00 00 00 00 00 09 FF C4 00 14 10
02E0: 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
02F0: 00 FF DA 00 08 01 01 00 00 3F 00 9F E3 FF D9 00
0300: 00 00 01 6D 64 61 74 00 00 00 00 00 00 00 A1 FF
0310: D8 FF DB 00 43 00 01 01 01 01 01 01 01 01 01 01
0320: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0330: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0340: 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
0350: 01 01 01 01 01 01 FF C0 00 0B 08 00 10 00 20 01
0360: 01 11 00 FF C4 00 15 00 01 01 00 00 00 00 00 00
0370: 00 00 00 00 00 00 00 00 00 0A FF C4 00 14 10 01
0380: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0390: FF DA 00 08 01 01 00 00 3F 00 A4 00 00 1F FF D9
";

/// The 141-byte stream of TV-14.
pub(crate) const TV14_HEX: &str = "
FF D8 FF DB 00 43 00
01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
01 01 01 01 01 01 01 01 01 01 01 01 01 01 01 01
FF C0 00 0B 08 00 08 00 08 01 01 11 00
FF C4 00 14 00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
FF C4 00 14 10 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
FF DA 00 08 01 01 00 00 3F 00 3F FF D9
";

/// Concatenate byte strings.
pub(crate) fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.iter().flat_map(|p| p.iter().copied()).collect()
}

/// A box with an 8-byte header.
pub(crate) fn bx(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = u32::try_from(8 + payload.len()).expect("box size");
    cat(&[&size.to_be_bytes(), typ, payload])
}

/// A box with the size field 1 and a 64-bit size (16-byte header).
pub(crate) fn bx64(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let size = 16 + payload.len() as u64;
    cat(&[&1u32.to_be_bytes(), typ, &size.to_be_bytes(), payload])
}

/// A uuid box with the extended type `ext`.
pub(crate) fn uuid(ext: &[u8; 16], payload: &[u8]) -> Vec<u8> {
    bx(b"uuid", &cat(&[ext, payload]))
}

/// A one-component baseline JPEG of `w` x `h` pixels of one gray value, in
/// the form of the fixture recipes: DQT with all values 1, a DC table with
/// the code 0 for category 0 and the code 10 for the category of the first
/// block's difference, an AC table with only the end-of-block code. Only
/// the first 8 x 8 block codes a DC difference.
pub(crate) fn gray_jpeg(w: u16, h: u16, gray: u8) -> Vec<u8> {
    let diff = (i32::from(gray) - 128) * 8;
    let cat_of = |v: i32| 32 - v.unsigned_abs().leading_zeros();
    let c = cat_of(diff);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
    out.extend([1u8; 64]);
    out.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    out.extend(h.to_be_bytes());
    out.extend(w.to_be_bytes());
    out.extend([0x01, 0x01, 0x11, 0x00]);
    // DC table 0: "0" -> category 0, and "10" -> category c.
    let mut bits = [0u8; 16];
    bits[0] = 1;
    let mut vals = vec![0u8];
    if diff != 0 {
        bits[1] = 1;
        vals.push(c as u8);
    }
    out.extend([0xFF, 0xC4]);
    out.extend(((2 + 1 + 16 + vals.len()) as u16).to_be_bytes());
    out.push(0x00);
    out.extend(bits);
    out.extend(&vals);
    // AC table 0: "0" -> end of block.
    out.extend([0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01]);
    out.extend([0u8; 15]);
    out.push(0x00);
    out.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    let blocks = usize::from(w).div_ceil(8) * usize::from(h).div_ceil(8);
    let mut bw = BitWriter::default();
    for k in 0..blocks {
        if k == 0 && diff != 0 {
            bw.put(0b10, 2);
            let v = if diff < 0 { diff + (1 << c) - 1 } else { diff };
            bw.put(v as u32, c);
        } else {
            bw.put(0, 1);
        }
        bw.put(0, 1);
    }
    out.extend(bw.finish());
    out.extend([0xFF, 0xD9]);
    out
}

/// A three-component baseline JPEG of one color: each component's first
/// block codes its DC difference, all others none. `samp` holds the
/// sampling factors (H, V) of the three components. Quantization values
/// are 1, the DC table gives every needed category a 4-bit code, the AC
/// table has only the end-of-block code "0".
pub(crate) fn ycc_jpeg(w: u16, h: u16, ycc: [u8; 3], samp: [(u8, u8); 3]) -> Vec<u8> {
    let diffs = ycc.map(|v| (i32::from(v) - 128) * 8);
    let cat_of = |v: i32| 32 - v.unsigned_abs().leading_zeros();
    let mut cats: Vec<u8> = diffs.iter().map(|&d| cat_of(d) as u8).collect();
    cats.push(0);
    cats.sort_unstable();
    cats.dedup();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
    out.extend([1u8; 64]);
    out.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]);
    out.extend(h.to_be_bytes());
    out.extend(w.to_be_bytes());
    out.push(3);
    for (i, (sh, sv)) in samp.iter().enumerate() {
        out.extend([i as u8 + 1, (sh << 4) | sv, 0]);
    }
    let mut bits = [0u8; 16];
    bits[3] = cats.len() as u8;
    out.extend([0xFF, 0xC4]);
    out.extend(((2 + 1 + 16 + cats.len()) as u16).to_be_bytes());
    out.push(0x00);
    out.extend(bits);
    out.extend(&cats);
    out.extend([0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01]);
    out.extend([0u8; 15]);
    out.push(0x00);
    out.extend([0xFF, 0xDA, 0x00, 0x0C, 0x03]);
    for i in 1..=3u8 {
        out.extend([i, 0x00]);
    }
    out.extend([0x00, 0x3F, 0x00]);
    let hmax = samp.iter().map(|s| s.0).max().unwrap_or(1);
    let vmax = samp.iter().map(|s| s.1).max().unwrap_or(1);
    let across = usize::from(w).div_ceil(8 * usize::from(hmax));
    let down = usize::from(h).div_ceil(8 * usize::from(vmax));
    let code = |c: u32| cats.iter().position(|&x| u32::from(x) == c).unwrap() as u32;
    let mut first = [true; 3];
    let mut bw = BitWriter::default();
    for _ in 0..across * down {
        for k in 0..3 {
            for _ in 0..usize::from(samp[k].0) * usize::from(samp[k].1) {
                let diff = if first[k] { diffs[k] } else { 0 };
                first[k] = false;
                let c = cat_of(diff);
                bw.put(code(c), 4);
                if c > 0 {
                    let v = if diff < 0 { diff + (1 << c) - 1 } else { diff };
                    bw.put(v as u32, c);
                }
                bw.put(0, 1);
            }
        }
    }
    out.extend(bw.finish());
    out.extend([0xFF, 0xD9]);
    out
}

/// A one-component baseline JPEG of `cols` x `rows` flat 8 x 8 blocks with
/// the gray values `vals` in row order. Every needed DC category has a
/// 4-bit code; the AC table has only the end-of-block code "0".
pub(crate) fn blocks_jpeg(cols: u16, rows: u16, vals: &[u8]) -> Vec<u8> {
    let cat_of = |v: i32| 32 - v.unsigned_abs().leading_zeros();
    let mut prev = 0i32;
    let diffs: Vec<i32> = vals
        .iter()
        .map(|&v| {
            let dc = (i32::from(v) - 128) * 8;
            let d = dc - prev;
            prev = dc;
            d
        })
        .collect();
    let mut cats: Vec<u8> = diffs.iter().map(|&d| cat_of(d) as u8).collect();
    cats.sort_unstable();
    cats.dedup();
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xDB, 0x00, 0x43, 0x00];
    out.extend([1u8; 64]);
    out.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08]);
    out.extend((rows * 8).to_be_bytes());
    out.extend((cols * 8).to_be_bytes());
    out.extend([0x01, 0x01, 0x11, 0x00]);
    let mut bits = [0u8; 16];
    bits[3] = cats.len() as u8;
    out.extend([0xFF, 0xC4]);
    out.extend(((2 + 1 + 16 + cats.len()) as u16).to_be_bytes());
    out.push(0x00);
    out.extend(bits);
    out.extend(&cats);
    out.extend([0xFF, 0xC4, 0x00, 0x14, 0x10, 0x01]);
    out.extend([0u8; 15]);
    out.push(0x00);
    out.extend([0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00]);
    let mut bw = BitWriter::default();
    for &d in &diffs {
        let c = cat_of(d);
        let code = cats.iter().position(|&x| u32::from(x) == c).unwrap() as u32;
        bw.put(code, 4);
        if c > 0 {
            let v = if d < 0 { d + (1 << c) - 1 } else { d };
            bw.put(v as u32, c);
        }
        bw.put(0, 1);
    }
    out.extend(bw.finish());
    out.extend([0xFF, 0xD9]);
    out
}

/// An entropy-coded segment writer with byte stuffing (T.81 F.1.2.3).
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

impl BitWriter {
    fn put(&mut self, v: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((v >> i) & 1);
            self.n += 1;
            if self.n == 8 {
                self.byte();
            }
        }
    }

    fn byte(&mut self) {
        let b = self.acc as u8;
        self.out.push(b);
        if b == 0xFF {
            self.out.push(0x00);
        }
        self.acc = 0;
        self.n = 0;
    }

    /// Pad the last byte with 1-bits.
    fn finish(mut self) -> Vec<u8> {
        while self.n != 0 {
            self.put(1, 1);
        }
        self.out
    }
}

/// A TIFF field value for [`tiff`].
#[derive(Clone, Debug)]
pub(crate) enum Val {
    /// ASCII with a terminating zero added.
    Ascii(&'static str),
    /// SHORT values.
    Short(Vec<u16>),
    /// LONG values.
    Long(Vec<u32>),
    /// Any type: type, count and the value bytes in the block's order.
    Raw(u16, u32, Vec<u8>),
}

/// A TIFF block (header, IFD0, next-IFD offset 0) with values over 4
/// bytes after the IFD in entry order, each padded to an even length.
pub(crate) fn tiff(le: bool, entries: &[(u16, Val)]) -> Vec<u8> {
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut out = if le {
        vec![0x49, 0x49, 0x2A, 0x00]
    } else {
        vec![0x4D, 0x4D, 0x00, 0x2A]
    };
    out.extend(u32b(8));
    let mut ifd = u16b(entries.len() as u16).to_vec();
    let mut data = Vec::new();
    let data_at = 8 + 2 + 12 * entries.len() + 4;
    for (tag, v) in entries {
        let (typ, count, bytes): (u16, u32, Vec<u8>) = match v {
            Val::Ascii(s) => {
                let mut b = s.as_bytes().to_vec();
                b.push(0);
                (2, b.len() as u32, b)
            }
            Val::Short(s) => (3, s.len() as u32, s.iter().flat_map(|&x| u16b(x)).collect()),
            Val::Long(s) => (4, s.len() as u32, s.iter().flat_map(|&x| u32b(x)).collect()),
            Val::Raw(t, c, b) => (*t, *c, b.clone()),
        };
        ifd.extend(u16b(*tag));
        ifd.extend(u16b(typ));
        ifd.extend(u32b(count));
        if bytes.len() <= 4 {
            let mut field = bytes.clone();
            field.resize(4, 0);
            ifd.extend(field);
        } else {
            ifd.extend(u32b((data_at + data.len()) as u32));
            data.extend(&bytes);
            if data.len() % 2 == 1 {
                data.push(0);
            }
        }
    }
    out.extend(ifd);
    out.extend(u32b(0));
    out.extend(data);
    out
}

/// A CRAW sample entry: the 90 bytes of M4 and the child boxes.
pub(crate) fn craw_entry(w: u16, h: u16, field: [u8; 4], children: &[u8]) -> Vec<u8> {
    let size = (90 + children.len()) as u32;
    let mut e = size.to_be_bytes().to_vec();
    e.extend(b"CRAW");
    e.extend([0u8; 6]);
    e.extend([0, 1]);
    e.extend([0u8; 16]);
    e.extend(w.to_be_bytes());
    e.extend(h.to_be_bytes());
    e.extend([0, 0x48, 0, 0, 0, 0x48, 0, 0]);
    e.extend([0u8; 4]);
    e.extend([0, 1]);
    e.extend([0u8; 32]);
    e.extend([0, 0x18, 0xFF, 0xFF]);
    e.extend(field);
    e.extend(children);
    e
}

/// The CRAW entry of a JPEG track (child "JPEG" with payload 0).
pub(crate) fn jpeg_entry(w: u16, h: u16) -> Vec<u8> {
    craw_entry(w, h, JPEG_FIELD, &bx(b"JPEG", &[0, 0, 0, 0]))
}

/// A CMP1 payload of 52 bytes like TV-13: FF 00 (or FF 10), width and
/// height at bytes 8 and 12, byte 26 = `mode`.
pub(crate) fn cmp1(w: u32, h: u32, mode: u8, ff10: bool) -> Vec<u8> {
    let mut p = vec![0u8; 52];
    p[0] = 0xFF;
    p[1] = if ff10 { 0x10 } else { 0x00 };
    p[8..12].copy_from_slice(&w.to_be_bytes());
    p[12..16].copy_from_slice(&h.to_be_bytes());
    p[26] = mode;
    p
}

/// The CRAW entry of a CRX track (children "CMP1" and "free").
pub(crate) fn crx_entry(w: u32, h: u32, mode: u8) -> Vec<u8> {
    let kids = cat(&[
        &bx(b"CMP1", &cmp1(w, h, mode, mode != 0)),
        &bx(b"free", &[0, 0]),
    ]);
    craw_entry(0, 0, CRX_FIELD, &kids)
}

/// The CRAW entry of an HEVC track (children "HEVC" and "free").
pub(crate) fn hevc_entry(w: u16, h: u16) -> Vec<u8> {
    let kids = cat(&[
        &bx(b"HEVC", &bx(b"hvcC", &[1, 2, 3])),
        &bx(b"free", &[0, 0]),
    ]);
    craw_entry(w, h, HEVC_FIELD, &kids)
}

/// The CTMD sample entry of F3.
pub(crate) fn ctmd_entry() -> Vec<u8> {
    hex("00 00 00 10 43 54 4D 44 00 00 00 00 00 00 00 01")
}

/// THMB box with version 0: 160 x 120 header fields set to `w` x `h`.
pub(crate) fn thmb(w: u16, h: u16, jpeg: &[u8]) -> Vec<u8> {
    let p = cat(&[
        &[0, 0, 0, 0],
        &w.to_be_bytes(),
        &h.to_be_bytes(),
        &(jpeg.len() as u32).to_be_bytes(),
        &[0, 1, 0, 0],
        jpeg,
    ]);
    bx(b"THMB", &p)
}

/// PRVW box with version 0.
pub(crate) fn prvw(w: u16, h: u16, jpeg: &[u8]) -> Vec<u8> {
    let p = cat(&[
        &[0, 0, 0, 0, 0, 1],
        &w.to_be_bytes(),
        &h.to_be_bytes(),
        &[0, 1],
        &(jpeg.len() as u32).to_be_bytes(),
        jpeg,
    ]);
    bx(b"PRVW", &p)
}

/// The sample of a track.
#[derive(Clone, Debug)]
pub(crate) enum Sample {
    /// Bytes stored in "mdat".
    Data(Vec<u8>),
    /// An explicit offset and size (nothing stored).
    At(u64, u32),
}

/// One track of a [`Cr3`] file.
#[derive(Clone, Debug)]
pub(crate) struct TrackSpec {
    /// Handler type.
    pub handler: [u8; 4],
    /// The whole first sample entry.
    pub entry: Vec<u8>,
    /// The first sample.
    pub sample: Sample,
    /// True: "stsz" with a sample size and no table (as in F3); false: a
    /// table of `count` sizes.
    pub fixed: bool,
    /// The "stsz" number of entries.
    pub count: u32,
}

impl TrackSpec {
    /// A JPEG track with its sample in "mdat".
    pub(crate) fn jpeg(w: u16, h: u16, stream: Vec<u8>) -> Self {
        TrackSpec {
            handler: *b"vide",
            entry: jpeg_entry(w, h),
            sample: Sample::Data(stream),
            fixed: false,
            count: 1,
        }
    }

    /// A CRX track whose sample is `data`.
    pub(crate) fn crx(w: u32, h: u32, mode: u8, data: Vec<u8>) -> Self {
        TrackSpec {
            handler: *b"vide",
            entry: crx_entry(w, h, mode),
            sample: Sample::Data(data),
            fixed: false,
            count: 1,
        }
    }

    /// A metadata track with the CTMD sample `data`.
    pub(crate) fn ctmd(data: Vec<u8>) -> Self {
        TrackSpec {
            handler: *b"meta",
            entry: ctmd_entry(),
            sample: Sample::Data(data),
            fixed: true,
            count: 1,
        }
    }

    /// The "trak" box, with the sample at `offset` (for a stored sample).
    pub(crate) fn bytes(&self, offset: u64) -> Vec<u8> {
        let size = match &self.sample {
            Sample::Data(d) => d.len() as u32,
            Sample::At(_, s) => *s,
        };
        let offset = match &self.sample {
            Sample::Data(_) => offset,
            Sample::At(o, _) => *o,
        };
        let hdlr = bx(b"hdlr", &cat(&[&[0u8; 8], &self.handler, &[0u8; 13]]));
        let stsd = bx(b"stsd", &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &self.entry]));
        let stsz = if self.fixed {
            bx(
                b"stsz",
                &cat(&[&[0u8; 4], &size.to_be_bytes(), &self.count.to_be_bytes()]),
            )
        } else {
            let mut p = cat(&[&[0u8; 4], &[0u8; 4], &self.count.to_be_bytes()]);
            for _ in 0..self.count {
                p.extend(size.to_be_bytes());
            }
            bx(b"stsz", &p)
        };
        let co64 = bx(
            b"co64",
            &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &offset.to_be_bytes()]),
        );
        let stbl = bx(b"stbl", &cat(&[&stsd, &stsz, &co64]));
        let mdia = bx(b"mdia", &cat(&[&hdlr, &bx(b"minf", &stbl)]));
        bx(b"trak", &mdia)
    }
}

/// A CR3 file: "ftyp", "moov" (the Canon metadata box and the tracks),
/// the preview holder box, "mdat" with the samples stored in it.
#[derive(Clone, Debug)]
pub(crate) struct Cr3 {
    /// The child boxes of the Canon metadata box.
    pub canon: Vec<Vec<u8>>,
    /// The tracks in "moov" order.
    pub tracks: Vec<TrackSpec>,
    /// The child boxes of the preview holder box (after its 8 bytes), or
    /// no preview holder box.
    pub holder: Option<Vec<Vec<u8>>>,
}

impl Cr3 {
    /// The file bytes.
    pub(crate) fn build(&self) -> Vec<u8> {
        let canon = uuid(&CANON_UUID, &self.canon.concat());
        let holder = self
            .holder
            .as_ref()
            .map(|kids| {
                uuid(
                    &PREVIEW_UUID,
                    &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &kids.concat()]),
                )
            })
            .unwrap_or_default();
        let moov_of = |offsets: &[u64]| {
            let traks: Vec<u8> = self
                .tracks
                .iter()
                .zip(offsets)
                .flat_map(|(t, &o)| t.bytes(o))
                .collect();
            bx(b"moov", &cat(&[&canon, &traks]))
        };
        let zeros = vec![0u64; self.tracks.len()];
        let mdat_payload = (FTYP.len() + moov_of(&zeros).len() + holder.len() + 16) as u64;
        let mut offsets = Vec::new();
        let mut payload = Vec::new();
        for t in &self.tracks {
            offsets.push(mdat_payload + payload.len() as u64);
            if let Sample::Data(d) = &t.sample {
                payload.extend(d);
            }
        }
        cat(&[&FTYP, &moov_of(&offsets), &holder, &bx64(b"mdat", &payload)])
    }
}

/// The CNCV box of F1.
pub(crate) fn cncv() -> Vec<u8> {
    bx(b"CNCV", b"CanonCR3_001/00.10.00/00.00.00")
}

/// The CMT1 box of F1 with the given orientation.
pub(crate) fn cmt1(orientation: u16) -> Vec<u8> {
    bx(
        b"CMT1",
        &tiff(
            true,
            &[
                (0x010F, Val::Ascii("Canon")),
                (0x0110, Val::Ascii("Canon EOS R6")),
                (0x0112, Val::Short(vec![orientation])),
            ],
        ),
    )
}

/// F1 built from its recipe.
pub(crate) fn f1_spec(orientation: u16) -> Cr3 {
    Cr3 {
        canon: vec![cncv(), cmt1(orientation), thmb(8, 8, &gray_jpeg(8, 8, 128))],
        tracks: vec![TrackSpec::jpeg(32, 16, gray_jpeg(32, 16, 200))],
        holder: Some(vec![prvw(16, 8, &gray_jpeg(16, 8, 96))]),
    }
}

/// Fixture F1.
pub(crate) fn f1() -> Vec<u8> {
    hex(F1_HEX)
}

/// Fixture F2: F1 with the byte at 0x30F changed from FF to 00.
pub(crate) fn f2() -> Vec<u8> {
    let mut f = f1();
    f[0x30F] = 0x00;
    f
}

/// ColorData of F3: 813 SHORT values, 0 except c[0] = 33, c[85] to c[89],
/// c[343] to c[346] and c[810] to c[812].
pub(crate) fn f3_color_data() -> Vec<u16> {
    let mut c = vec![0u16; 813];
    c[0] = 33;
    c[85..90].copy_from_slice(&[1856, 1024, 1024, 1670, 4965]);
    c[343..347].copy_from_slice(&[512; 4]);
    c[810..813].copy_from_slice(&[12735, 14008, 12735]);
    c
}

/// A CTMD sample of one record of `typ` holding one block per (tag,
/// TIFF bytes).
pub(crate) fn ctmd_record(typ: u16, blocks: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (tag, t) in blocks {
        body.extend(((8 + t.len()) as u32).to_le_bytes());
        body.extend(tag.to_le_bytes());
        body.extend(t);
    }
    let mut r = ((12 + body.len()) as u32).to_le_bytes().to_vec();
    r.extend(typ.to_le_bytes());
    r.extend([1, 1, 1, 1, 0xFF, 0xFF]);
    r.extend(body);
    r
}

/// The 1672-byte metadata sample of F3.
pub(crate) fn f3_sample() -> Vec<u8> {
    let note = tiff(true, &[(0x4001, Val::Short(f3_color_data()))]);
    ctmd_record(8, &[(0x927C, note)])
}

/// F3 built from its recipe.
pub(crate) fn f3_spec() -> Cr3 {
    let mut s = f1_spec(1);
    s.tracks.push(TrackSpec::ctmd(f3_sample()));
    s
}

/// Fixture F3.
pub(crate) fn f3() -> Vec<u8> {
    f3_spec().build()
}

/// A file with every kind of track: P1 (`p1` x 2/3 size), two CRX tracks
/// (track 2 small, track 3 main with CMP1 byte 26 = `mode`), the metadata
/// track of F3, an optional Dual Pixel track 5, PRVW and THMB. The CRX
/// samples are `crx` (never read by the decoder).
pub(crate) fn full_spec(mode: u8, dual: bool, crx: Vec<u8>) -> Cr3 {
    let mut tracks = vec![
        TrackSpec::jpeg(48, 32, gray_jpeg(48, 32, 200)),
        TrackSpec::crx(24, 16, mode, crx.clone()),
        TrackSpec::crx(64, 40, mode, crx.clone()),
        TrackSpec::ctmd(f3_sample()),
    ];
    if dual {
        tracks.push(TrackSpec::crx(64, 40, mode, crx));
    }
    Cr3 {
        canon: vec![
            cncv(),
            cmt1(1),
            bx(b"CMT2", &tiff(true, &[(0xA001, Val::Short(vec![1]))])),
            bx(
                b"CMT3",
                &tiff(
                    true,
                    &[(
                        0x00E0,
                        Val::Short(vec![34, 64, 40, 1, 1, 8, 4, 55, 35, 0, 0, 0, 0, 0, 0, 0, 0]),
                    )],
                ),
            ),
            thmb(8, 8, &gray_jpeg(8, 8, 128)),
        ],
        tracks,
        holder: Some(vec![prvw(16, 8, &gray_jpeg(16, 8, 96))]),
    }
}

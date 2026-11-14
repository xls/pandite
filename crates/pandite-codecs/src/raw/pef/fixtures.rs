//! Synthetic PEF files for the tests: the specification's fixtures F1 to
//! F5 (section Test vectors, Fixture recipes), a builder that follows the
//! recipe layout in either byte order with either maker-note form, and a
//! Pentax Huffman encoder written from the rules of section Compression.

use super::meta::Form;

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};

/// One IFD value for the builder.
#[derive(Clone, Debug)]
pub(super) enum Val {
    Byte(Vec<u8>),
    Undef(Vec<u8>),
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    /// LONG offset of a blob (by index).
    Blob(usize),
    /// LONG length of a blob (by index).
    BlobLen(usize),
    /// Any field type and count with these value bytes, as given.
    Raw(u16, u32, Vec<u8>),
    /// The maker note (UNDEFINED), placed after every other value.
    MakerNote(Vec<u8>),
}

/// The maker note of a builder.
#[derive(Clone, Debug)]
pub(super) struct Maker {
    /// AOC or PENTAX form.
    pub form: Form,
    /// Byte order of the maker note.
    pub le: bool,
    /// The entries.
    pub entries: Vec<(u16, Val)>,
}

/// A synthetic PEF file following the recipe layout: header, IFD0 at 8
/// with its values, the Exif IFD with its values and the maker note last,
/// further chain IFDs (IFD1, IFD2), then blobs at multiples of 16.
#[derive(Clone, Debug)]
pub(super) struct Builder {
    /// Little-endian file.
    pub le: bool,
    /// IFD0 entries; the Exif IFD pointer is added when `exif` is set.
    pub ifd0: Vec<(u16, Val)>,
    /// Exif IFD entries; the MakerNote entry is added when `maker` is set.
    pub exif: Option<Vec<(u16, Val)>>,
    /// The maker note.
    pub maker: Option<Maker>,
    /// IFDs chained after IFD0.
    pub chain: Vec<Vec<(u16, Val)>>,
    /// Data blobs: blob 0 is the sensor strip, the others previews.
    pub blobs: Vec<Vec<u8>>,
}

fn even(n: usize) -> usize {
    n + n % 2
}

/// Field type, count and value bytes of a value.
fn payload(v: &Val, le: bool, blob_at: &[usize], blobs: &[Vec<u8>]) -> (u16, u32, Vec<u8>) {
    let w16 = |x: u16| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    let w32 = |x: u32| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    match v {
        Val::Byte(b) => (1, b.len() as u32, b.clone()),
        Val::Undef(b) | Val::MakerNote(b) => (7, b.len() as u32, b.clone()),
        Val::Ascii(s) => {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            (2, b.len() as u32, b)
        }
        Val::Short(x) => (3, x.len() as u32, x.iter().flat_map(|&v| w16(v)).collect()),
        Val::Long(x) => (4, x.len() as u32, x.iter().flat_map(|&v| w32(v)).collect()),
        Val::Blob(k) => (
            4,
            1,
            w32(blob_at.get(*k).copied().unwrap_or(0) as u32).to_vec(),
        ),
        Val::BlobLen(k) => (4, 1, w32(blobs.get(*k).map_or(0, Vec::len) as u32).to_vec()),
        Val::Raw(t, n, b) => (*t, *n, b.clone()),
    }
}

/// One IFD at file offset `at` (entries sorted by tag) with its
/// out-of-line values after it, each at an even file offset, the maker
/// note last. Stored value offsets count from `origin`. No padding after
/// the last value.
fn ifd_bytes(
    entries: &[(u16, Val)],
    at: usize,
    le: bool,
    origin: usize,
    next: usize,
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) -> Vec<u8> {
    let mut sorted: Vec<&(u16, Val)> = entries.iter().collect();
    sorted.sort_by_key(|e| e.0);
    let w16 = |x: u16| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    let w32 = |x: u32| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    // Value positions: in tag order, the maker note after all others.
    let values_at = at + 2 + 12 * sorted.len() + 4;
    let mut order: Vec<usize> = (0..sorted.len()).collect();
    order.sort_by_key(|&i| matches!(sorted[i].1, Val::MakerNote(_)));
    let mut pos = vec![0usize; sorted.len()];
    let mut end = values_at;
    for &i in &order {
        let (_, _, p) = payload(&sorted[i].1, le, blob_at, blobs);
        if p.len() > 4 {
            let start = even(end);
            pos[i] = start;
            end = start + p.len();
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(&w16(sorted.len() as u16));
    for (i, (tag, v)) in sorted.iter().enumerate() {
        let (typ, count, p) = payload(v, le, blob_at, blobs);
        out.extend_from_slice(&w16(*tag));
        out.extend_from_slice(&w16(typ));
        out.extend_from_slice(&w32(count));
        if p.len() <= 4 {
            let mut q = p.clone();
            q.resize(4, 0);
            out.extend_from_slice(&q);
        } else {
            out.extend_from_slice(&w32((pos[i] - origin) as u32));
        }
    }
    out.extend_from_slice(&w32(next as u32));
    out.resize(end - at, 0);
    for (i, (_, v)) in sorted.iter().enumerate() {
        let (_, _, p) = payload(v, le, blob_at, blobs);
        if p.len() > 4 {
            out[pos[i] - at..pos[i] - at + p.len()].copy_from_slice(&p);
        }
    }
    out
}

impl Maker {
    /// The maker-note bytes when it starts at file offset `m`.
    fn bytes(&self, m: usize) -> Vec<u8> {
        let (mut out, origin) = match self.form {
            Form::Aoc => (b"AOC\0".to_vec(), 0),
            Form::Pentax => (b"PENTAX \0".to_vec(), m),
        };
        out.extend_from_slice(if self.le { b"II" } else { b"MM" });
        let at = m + out.len();
        out.extend(ifd_bytes(&self.entries, at, self.le, origin, 0, &[], &[]));
        out
    }
}

impl Builder {
    /// A builder with the given byte order and nothing in it.
    pub(super) fn new(le: bool) -> Self {
        Builder {
            le,
            ifd0: Vec::new(),
            exif: None,
            maker: None,
            chain: Vec::new(),
            blobs: Vec::new(),
        }
    }

    /// Serialize with the blobs at `blob_at`.
    fn write(&self, blob_at: &[usize]) -> (Vec<u8>, usize) {
        let le = self.le;
        let mut out = if le {
            vec![0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]
        } else {
            vec![0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]
        };
        // The Exif IFD offset and the maker note do not change the sizes
        // of IFD0 and the Exif IFD: lay out with placeholders first.
        let mut ifd0 = self.ifd0.clone();
        if self.exif.is_some() {
            ifd0.push((0x8769, Val::Long(vec![0])));
        }
        let size0 = ifd_bytes(&ifd0, 8, le, 0, 0, blob_at, &self.blobs).len();
        let exif_at = even(8 + size0);
        let mut exif_bytes = Vec::new();
        let mut after = exif_at;
        if let Some(exif) = &self.exif {
            let mut entries = exif.clone();
            let maker_len = self.maker.as_ref().map(|m| m.bytes(0).len());
            if let Some(n) = maker_len {
                entries.push((0x927C, Val::MakerNote(vec![0; n])));
            }
            let probe = ifd_bytes(&entries, exif_at, le, 0, 0, blob_at, &self.blobs);
            if let Some(mk) = &self.maker {
                // The maker note is the last value: M is where it starts.
                let m = exif_at + probe.len() - maker_len.unwrap_or(0);
                entries.retain(|e| e.0 != 0x927C);
                entries.push((0x927C, Val::MakerNote(mk.bytes(m))));
            }
            exif_bytes = ifd_bytes(&entries, exif_at, le, 0, 0, blob_at, &self.blobs);
            after = exif_at + exif_bytes.len();
            ifd0.retain(|e| e.0 != 0x8769);
            ifd0.push((0x8769, Val::Long(vec![exif_at as u32])));
        }
        let mut chain_at = Vec::new();
        for c in &self.chain {
            let at = even(after);
            chain_at.push(at);
            after = at + ifd_bytes(c, at, le, 0, 0, blob_at, &self.blobs).len();
        }
        let first_next = chain_at.first().copied().unwrap_or(0);
        out.extend(ifd_bytes(&ifd0, 8, le, 0, first_next, blob_at, &self.blobs));
        if self.exif.is_some() {
            out.resize(exif_at, 0);
            out.extend(exif_bytes);
        }
        for (i, (&at, c)) in chain_at.iter().zip(&self.chain).enumerate() {
            let next = chain_at.get(i + 1).copied().unwrap_or(0);
            out.resize(at, 0);
            out.extend(ifd_bytes(c, at, le, 0, next, blob_at, &self.blobs));
        }
        (out, after)
    }

    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        // The blob positions follow the structures, whose sizes do not
        // depend on them.
        let placeholder = vec![0usize; self.blobs.len()];
        let (_, mut at) = self.write(&placeholder);
        let mut blob_at = Vec::new();
        for b in &self.blobs {
            at = at.div_ceil(16) * 16;
            blob_at.push(at);
            at += b.len();
        }
        let (mut out, _) = self.write(&blob_at);
        for (b, &a) in self.blobs.iter().zip(&blob_at) {
            out.resize(a, 0);
            out.extend_from_slice(b);
        }
        out
    }
}

/// The IFD0 entries of the fixture recipes: W, H, BPS, Compression,
/// PhotometricInterpretation, Model, the strip in blob 0, Orientation,
/// SamplesPerPixel, RowsPerStrip (the Exif pointer is added by the
/// builder).
pub(super) fn common_ifd0(
    (w, h): (u32, u32),
    bps: u16,
    compression: u16,
    photometric: u16,
    model: &str,
    orientation: u16,
) -> Vec<(u16, Val)> {
    vec![
        (0x0100, Val::Long(vec![w])),
        (0x0101, Val::Long(vec![h])),
        (0x0102, Val::Short(vec![bps])),
        (0x0103, Val::Short(vec![compression])),
        (0x0106, Val::Short(vec![photometric])),
        (0x0110, Val::Ascii(model.to_string())),
        (0x0111, Val::Blob(0)),
        (0x0112, Val::Short(vec![orientation])),
        (0x0115, Val::Short(vec![1])),
        (0x0116, Val::Long(vec![h])),
        (0x0117, Val::BlobLen(0)),
    ]
}

/// The CFA pattern of the recipes: RGGB.
pub(super) const RGGB: [u8; 8] = [0, 2, 0, 2, 0, 1, 1, 2];

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

/// The 53-byte table of TV-5 (K10D, 12-bit, big-endian).
pub(super) const TV5: &str =
    "00 01 00 00 00 16 00 28 00 49 00 23 00 4D 0F 00 0C 00 08 00 00 00 04 00 \
     0A 00 0E 00 0F 80 0F C0 0F E0 0F F0 0F F8 0F FC 05 03 03 02 02 03 04 06 07 08 09 0A 0A";

/// The 59-byte table of TV-6 (14-bit, little-endian).
pub(super) const TV6: &str =
    "03 00 00 00 0C 00 00 00 00 00 00 00 00 00 E0 0F C0 0F 80 0F 00 0F 00 0E \
     00 0A 00 0C 00 08 00 00 00 04 F0 0F F8 0F FC 0F FE 0F FF 0F 08 07 06 05 04 03 03 03 02 02 \
     09 0A 0B 0C 0C";

/// Bit writer, most significant bit first, no byte stuffing.
struct Bits {
    acc: u64,
    n: u32,
    out: Vec<u8>,
}

impl Bits {
    fn put(&mut self, v: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | u64::from(v & ((1u32 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            self.n -= 8;
            self.out.push((self.acc >> self.n) as u8);
        }
    }
}

/// Encode a `w` x `h` raster with the codes (code, length) of a table: the
/// inverse of R-PEF-29 to R-PEF-34. The last byte is padded with 1-bits.
pub(super) fn encode(codes: &[(u16, u8)], w: usize, h: usize, values: &[u16]) -> Vec<u8> {
    let mut b = Bits {
        acc: 0,
        n: 0,
        out: Vec::new(),
    };
    for y in 0..h {
        for x in 0..w {
            let v = |x: usize, y: usize| i32::from(values[y * w + x]);
            let p = if x >= 2 {
                v(x - 2, y)
            } else if y >= 2 {
                v(x, y - 2)
            } else {
                0
            };
            let d = v(x, y) - p;
            let s = 32 - d.unsigned_abs().leading_zeros();
            let e = if d >= 0 { d } else { d + (1 << s) - 1 };
            let (c, l) = codes[s as usize];
            b.put(u32::from(c), u32::from(l));
            b.put(e as u32, s);
        }
    }
    let pad = (8 - b.n % 8) % 8;
    if pad > 0 {
        b.put((1 << pad) - 1, pad);
    }
    b.out
}

/// Fixture F1 (P1, 4 x 2, big-endian, AOC maker note), the complete
/// 288-byte file of the specification.
pub(super) const F1_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 0C 01 00 00 04 00 00
0010: 00 01 00 00 00 04 01 01 00 04 00 00 00 01 00 00
0020: 00 02 01 02 00 03 00 00 00 01 00 0C 00 00 01 03
0030: 00 03 00 00 00 01 00 01 00 00 01 06 00 03 00 00
0040: 00 01 80 23 00 00 01 10 00 02 00 00 00 0C 00 00
0050: 00 9E 01 11 00 04 00 00 00 01 00 00 01 10 01 12
0060: 00 03 00 00 00 01 00 01 00 00 01 15 00 03 00 00
0070: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0080: 00 02 01 17 00 04 00 00 00 01 00 00 00 10 87 69
0090: 00 04 00 00 00 01 00 00 00 AA 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 31 30 44 00 00 02 92 7C 00 07
00B0: 00 00 00 34 00 00 00 D0 A3 02 00 07 00 00 00 08
00C0: 00 00 00 C8 00 00 00 00 00 02 00 02 00 01 01 02
00D0: 41 4F 43 00 4D 4D 00 02 02 00 00 03 00 00 00 04
00E0: 00 00 00 F4 02 01 00 03 00 00 00 04 00 00 00 FC
00F0: 00 00 00 00 00 80 00 80 00 80 00 80 35 C0 20 00
0100: 20 00 24 E0 00 00 00 00 00 00 00 00 00 00 00 00
0110: 04 68 08 50 04 68 08 50 08 50 04 04 08 50 04 04
";

/// Fixture F3 (P3, 8 x 4, big-endian, AOC maker note, 12-bit, crop
/// (1, 0, 4, 4)), the complete 406-byte file of the specification.
pub(super) const F3_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 0C 01 00 00 04 00 00
0010: 00 01 00 00 00 08 01 01 00 04 00 00 00 01 00 00
0020: 00 04 01 02 00 03 00 00 00 01 00 0C 00 00 01 03
0030: 00 03 00 00 00 01 FF FF 00 00 01 06 00 03 00 00
0040: 00 01 80 23 00 00 01 10 00 02 00 00 00 0C 00 00
0050: 00 9E 01 11 00 04 00 00 00 01 00 00 01 60 01 12
0060: 00 03 00 00 00 01 00 01 00 00 01 15 00 03 00 00
0070: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0080: 00 04 01 17 00 04 00 00 00 01 00 00 00 36 87 69
0090: 00 04 00 00 00 01 00 00 00 AA 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 31 30 44 00 00 02 92 7C 00 07
00B0: 00 00 00 8D 00 00 00 D0 A3 02 00 07 00 00 00 08
00C0: 00 00 00 C8 00 00 00 00 00 02 00 02 00 01 01 02
00D0: 41 4F 43 00 4D 4D 00 05 00 38 00 03 00 00 00 02
00E0: 00 01 00 00 00 39 00 03 00 00 00 02 00 04 00 04
00F0: 02 00 00 03 00 00 00 04 00 00 01 18 02 01 00 03
0100: 00 00 00 04 00 00 01 20 02 20 00 07 00 00 00 35
0110: 00 00 01 28 00 00 00 00 00 80 00 80 00 80 00 80
0120: 35 C0 20 00 20 00 24 E0 00 01 00 00 00 16 00 28
0130: 00 49 00 23 00 4D 0F 00 0C 00 08 00 00 00 04 00
0140: 0A 00 0E 00 0F 80 0F C0 0F E0 0F F0 0F F8 0F FC
0150: 05 03 03 02 02 03 04 06 07 08 09 0A 0A 00 00 00
0160: FF FF FF FF 85 0F FD 1A 3D EF FB D7 FF F7 2F EF
0170: FF FF FF E8 09 FF 05 0F 7B FF BF BF FB D7 FB DE
0180: FF D1 A3 DE FF BD 7F FF 72 FE F7 BF E0 A1 EF 7F
0190: F7 F7 FF 7A FF 7F
";

/// Fixture F4 (P3, 8 x 4, little-endian, PENTAX maker note, 14-bit, crop
/// (2, 1, 4, 2)), the complete 429-byte file of the specification.
pub(super) const F4_HEX: &str = "
0000: 49 49 2A 00 08 00 00 00 0C 00 00 01 04 00 01 00
0010: 00 00 08 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 0E 00 00 00 03 01
0030: 03 00 01 00 00 00 FF FF 00 00 06 01 03 00 01 00
0040: 00 00 23 80 00 00 10 01 02 00 14 00 00 00 9E 00
0050: 00 00 11 01 04 00 01 00 00 00 80 01 00 00 12 01
0060: 03 00 01 00 00 00 01 00 00 00 15 01 03 00 01 00
0070: 00 00 01 00 00 00 16 01 04 00 01 00 00 00 04 00
0080: 00 00 17 01 04 00 01 00 00 00 2D 00 00 00 69 87
0090: 04 00 01 00 00 00 B2 00 00 00 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 2D 33 20 4D 61 72 6B 20 49 49
00B0: 49 00 02 00 7C 92 07 00 A0 00 00 00 D8 00 00 00
00C0: 02 A3 07 00 08 00 00 00 D0 00 00 00 00 00 00 00
00D0: 00 02 00 02 00 01 01 02 50 45 4E 54 41 58 20 00
00E0: 49 49 05 00 7E 00 04 00 01 00 00 00 F8 3F 00 00
00F0: 00 02 03 00 04 00 00 00 4C 00 00 00 01 02 03 00
0100: 04 00 00 00 54 00 00 00 20 02 07 00 3B 00 00 00
0110: 5C 00 00 00 3E 02 07 00 08 00 00 00 98 00 00 00
0120: 00 00 00 00 00 01 00 01 00 01 00 01 80 4F 00 20
0130: 00 20 40 3A 03 00 00 00 0C 00 00 00 00 00 00 00
0140: 00 00 E0 0F C0 0F 80 0F 00 0F 00 0E 00 0A 00 0C
0150: 00 08 00 00 00 04 F0 0F F8 0F FC 0F FE 0F FF 0F
0160: 08 07 06 05 04 03 03 03 02 02 09 0A 0B 0C 0C 00
0170: 02 00 01 00 04 00 02 00 00 00 00 00 00 00 00 00
0180: FE FE FE FE FE FE FE FE FF F8 10 3F F7 88 7F 7F
0190: 7F 7F 7F 7F 7F F4 28 3F FE 04 0F EF EF EF EF EF
01A0: EF FE FD FF FF E1 DF FD FD FD FD FD FD
";

/// Fixture F5 (P4, 8 x 2, little-endian, PENTAX maker note, monochrome),
/// the complete 375-byte file of the specification.
pub(super) const F5_HEX: &str = "
0000: 49 49 2A 00 08 00 00 00 0C 00 00 01 04 00 01 00
0010: 00 00 08 00 00 00 01 01 04 00 01 00 00 00 02 00
0020: 00 00 02 01 03 00 01 00 00 00 0E 00 00 00 03 01
0030: 03 00 01 00 00 00 FF FF 00 00 06 01 03 00 01 00
0040: 00 00 4C 88 00 00 10 01 02 00 1F 00 00 00 9E 00
0050: 00 00 11 01 04 00 01 00 00 00 50 01 00 00 12 01
0060: 03 00 01 00 00 00 01 00 00 00 15 01 03 00 01 00
0070: 00 00 01 00 00 00 16 01 04 00 01 00 00 00 02 00
0080: 00 00 17 01 04 00 01 00 00 00 27 00 00 00 69 87
0090: 04 00 01 00 00 00 BE 00 00 00 00 00 00 00 50 45
00A0: 4E 54 41 58 20 4B 2D 33 20 4D 61 72 6B 20 49 49
00B0: 49 20 4D 6F 6E 6F 63 68 72 6F 6D 65 00 00 01 00
00C0: 7C 92 07 00 77 00 00 00 D0 00 00 00 00 00 00 00
00D0: 50 45 4E 54 41 58 20 00 49 49 03 00 7E 00 04 00
00E0: 01 00 00 00 FA 3F 00 00 00 02 03 00 04 00 00 00
00F0: 34 00 00 00 20 02 07 00 3B 00 00 00 3C 00 00 00
0100: 00 00 00 00 40 00 40 00 40 00 40 00 03 00 00 00
0110: 0C 00 00 00 00 00 00 00 00 00 E0 0F C0 0F 80 0F
0120: 00 0F 00 0E 00 0A 00 0C 00 08 00 00 00 04 F0 0F
0130: F8 0F FC 0F FE 0F FF 0F 08 07 06 05 04 03 03 03
0140: 02 02 09 0A 0B 0C 0C 00 00 00 00 00 00 00 00 00
0150: 90 3F E8 51 FF 7D 0F FB E8 7F DF 43 FE FA 1F F7
0160: D0 FF FB 0C BF FF FF AF FF 80 77 FF C0 45 FF E0
0170: 23 11 06 41 90 64 7F
";

/// F1 from its hex dump.
pub(super) fn f1() -> Vec<u8> {
    hex(F1_HEX)
}

/// F2 (P2): F1 with Compression 32773, StripByteCounts 12 and the 12 data
/// bytes of the recipe (284 bytes).
pub(super) fn f2() -> Vec<u8> {
    let mut f = f1();
    f[0x36] = 0x80;
    f[0x37] = 0x05;
    f[0x8D] = 0x0C;
    f.truncate(0x110);
    f.extend_from_slice(&hex("46 88 50 46 88 50 85 04 04 85 04 04"));
    f
}

/// F3 from its hex dump.
pub(super) fn f3() -> Vec<u8> {
    hex(F3_HEX)
}

/// F4 from its hex dump.
pub(super) fn f4() -> Vec<u8> {
    hex(F4_HEX)
}

/// F5 from its hex dump.
pub(super) fn f5() -> Vec<u8> {
    hex(F5_HEX)
}

/// The decoded raster of F3 (8 x 4, 12-bit).
pub(super) fn f3_raster() -> Vec<u16> {
    let r0 = [4095, 2128, 1128, 2128, 1128, 4095, 4095, 4095];
    let r1 = [4095, 1028, 2128, 1028, 2128, 4095, 4095, 4095];
    [r0, r1, r0, r1].concat()
}

/// The decoded raster of F4 (8 x 4, 14-bit).
pub(super) fn f4_raster() -> Vec<u16> {
    let r1 = [8256, 3856, 8256, 3856, 8256, 3856, 8256, 3856];
    let r2 = [4256, 8256, 4256, 8256, 4256, 8256, 4256, 8256];
    [[0; 8], r1, r2, [16383; 8]].concat()
}

/// The decoded raster of F5 (8 x 2, 14-bit).
pub(super) fn f5_raster() -> Vec<u16> {
    vec![
        64, 1064, 2064, 3064, 4064, 5064, 6064, 16378, 16378, 8221, 64, 100, 200, 300, 400, 500,
    ]
}

/// The (code, length) pairs of a table given as hex bytes.
pub(super) fn codes(table_hex: &str, bps: u32, le: bool) -> Vec<(u16, u8)> {
    super::huffman::Table::parse(&hex(table_hex), bps, le)
        .expect("valid table")
        .codes
}

/// The F1 recipe through the builder; the strip in blob 0.
pub(super) fn f1_builder() -> Builder {
    let strip = hex("04 68 08 50 04 68 08 50 08 50 04 04 08 50 04 04");
    Builder {
        ifd0: common_ifd0((4, 2), 12, 1, 32803, "PENTAX K10D", 1),
        exif: Some(vec![(0xA302, Val::Undef(RGGB.to_vec()))]),
        maker: Some(Maker {
            form: Form::Aoc,
            le: false,
            entries: vec![
                (0x0200, Val::Short(vec![128; 4])),
                (0x0201, Val::Short(vec![13760, 8192, 8192, 9440])),
            ],
        }),
        blobs: vec![strip],
        ..Builder::new(false)
    }
}

/// The F3 recipe through the builder, its strip encoded from the raster.
pub(super) fn f3_builder() -> Builder {
    let strip = encode(&codes(TV5, 12, false), 8, 4, &f3_raster());
    Builder {
        ifd0: common_ifd0((8, 4), 12, 65535, 32803, "PENTAX K10D", 1),
        exif: Some(vec![(0xA302, Val::Undef(RGGB.to_vec()))]),
        maker: Some(Maker {
            form: Form::Aoc,
            le: false,
            entries: vec![
                (0x0038, Val::Short(vec![1, 0])),
                (0x0039, Val::Short(vec![4, 4])),
                (0x0200, Val::Short(vec![128; 4])),
                (0x0201, Val::Short(vec![13760, 8192, 8192, 9440])),
                (0x0220, Val::Undef(hex(TV5))),
            ],
        }),
        blobs: vec![strip],
        ..Builder::new(false)
    }
}

/// The F4 recipe through the builder, its strip encoded from the raster.
pub(super) fn f4_builder() -> Builder {
    let strip = encode(&codes(TV6, 14, true), 8, 4, &f4_raster());
    Builder {
        ifd0: common_ifd0((8, 4), 14, 65535, 32803, "PENTAX K-3 Mark III", 1),
        exif: Some(vec![(0xA302, Val::Undef(RGGB.to_vec()))]),
        maker: Some(Maker {
            form: Form::Pentax,
            le: true,
            entries: vec![
                (0x007E, Val::Long(vec![16376])),
                (0x0200, Val::Short(vec![256; 4])),
                (0x0201, Val::Short(vec![20352, 8192, 8192, 14912])),
                (0x0220, Val::Undef(hex(TV6))),
                (0x023E, Val::Undef(hex("02 00 01 00 04 00 02 00"))),
            ],
        }),
        blobs: vec![strip],
        ..Builder::new(true)
    }
}

/// The F5 recipe through the builder, its strip encoded from the raster.
pub(super) fn f5_builder() -> Builder {
    let strip = encode(&codes(TV6, 14, true), 8, 2, &f5_raster());
    Builder {
        ifd0: common_ifd0(
            (8, 2),
            14,
            65535,
            34892,
            "PENTAX K-3 Mark III Monochrome",
            1,
        ),
        exif: Some(Vec::new()),
        maker: Some(Maker {
            form: Form::Pentax,
            le: true,
            entries: vec![
                (0x007E, Val::Long(vec![16378])),
                (0x0200, Val::Short(vec![64; 4])),
                (0x0220, Val::Undef(hex(TV6))),
            ],
        }),
        blobs: vec![strip],
        ..Builder::new(true)
    }
}

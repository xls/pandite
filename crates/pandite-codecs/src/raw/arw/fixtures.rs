//! Synthetic ARW files for the tests: the specification's fixtures F1 to
//! F5 (section Test vectors, Fixture recipes), a builder that follows the
//! recipe layout, a lossless JPEG encoder written from T.81, and small
//! preview JPEGs.

/// One IFD value for the builder.
#[derive(Clone, Debug)]
pub(super) enum Val {
    Byte(Vec<u8>),
    Ascii(&'static str),
    Short(Vec<u16>),
    Long(Vec<u32>),
    SShort(Vec<i16>),
    Rational(Vec<(u32, u32)>),
    /// LONG offsets of blobs (by index).
    Blobs(Vec<usize>),
    /// LONG lengths of blobs (by index).
    BlobLens(Vec<usize>),
}

/// A synthetic ARW file following the recipe layout: header, IFD0 at 8
/// with its values, the raw IFD with its values, extra chain IFDs, then
/// blobs at multiples of 16.
#[derive(Clone, Debug, Default)]
pub(super) struct Builder {
    /// IFD0 entries; SubIFDs is added when `sub_ifds` is set.
    pub ifd0: Vec<(u16, Val)>,
    /// Raw IFD entries.
    pub raw: Vec<(u16, Val)>,
    /// IFDs chained after IFD0 (IFD1, IFD2, ...).
    pub chain: Vec<Vec<(u16, Val)>>,
    /// Data blobs: sensor strips or tiles, previews.
    pub blobs: Vec<Vec<u8>>,
    /// Add SubIFDs to IFD0 pointing at the raw IFD.
    pub sub_ifds: bool,
}

fn payload(v: &Val, blob_at: &[usize], blobs: &[Vec<u8>]) -> (u16, u32, Vec<u8>) {
    match v {
        Val::Byte(b) => (1, b.len() as u32, b.clone()),
        Val::Ascii(s) => {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            (2, b.len() as u32, b)
        }
        Val::Short(x) => (
            3,
            x.len() as u32,
            x.iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        Val::Long(x) => (
            4,
            x.len() as u32,
            x.iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        Val::SShort(x) => (
            8,
            x.len() as u32,
            x.iter().flat_map(|v| v.to_le_bytes()).collect(),
        ),
        Val::Rational(x) => (
            5,
            x.len() as u32,
            x.iter()
                .flat_map(|(n, d)| [n.to_le_bytes(), d.to_le_bytes()].concat())
                .collect(),
        ),
        Val::Blobs(i) => (
            4,
            i.len() as u32,
            i.iter()
                .flat_map(|&k| (blob_at[k] as u32).to_le_bytes())
                .collect(),
        ),
        Val::BlobLens(i) => (
            4,
            i.len() as u32,
            i.iter()
                .flat_map(|&k| (blobs[k].len() as u32).to_le_bytes())
                .collect(),
        ),
    }
}

/// Size of an IFD with its out-of-line values (each at an even offset).
fn ifd_size(entries: &[(u16, Val)], blobs: &[Vec<u8>]) -> usize {
    let zero = vec![0usize; blobs.len()];
    let mut n = 2 + 12 * entries.len() + 4;
    for (_, v) in entries {
        let (_, _, p) = payload(v, &zero, blobs);
        if p.len() > 4 {
            n += p.len() + p.len() % 2;
        }
    }
    n
}

/// Write one IFD at `at` (entries sorted by tag) with its values after it.
fn write_ifd(
    out: &mut Vec<u8>,
    entries: &[(u16, Val)],
    next: usize,
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) {
    let at = out.len();
    let mut sorted: Vec<&(u16, Val)> = entries.iter().collect();
    sorted.sort_by_key(|e| e.0);
    let mut values: Vec<u8> = Vec::new();
    let values_at = at + 2 + 12 * sorted.len() + 4;
    out.extend_from_slice(&(sorted.len() as u16).to_le_bytes());
    for (tag, v) in sorted {
        let (typ, count, p) = payload(v, blob_at, blobs);
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&typ.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        if p.len() <= 4 {
            let mut q = p.clone();
            q.resize(4, 0);
            out.extend_from_slice(&q);
        } else {
            out.extend_from_slice(&((values_at + values.len()) as u32).to_le_bytes());
            values.extend_from_slice(&p);
            if values.len() % 2 == 1 {
                values.push(0);
            }
        }
    }
    out.extend_from_slice(&(next as u32).to_le_bytes());
    out.extend(values);
}

impl Builder {
    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let mut ifd0 = self.ifd0.clone();
        let size0 = ifd_size(&ifd0, &self.blobs) + if self.sub_ifds { 12 } else { 0 };
        let raw_at = 8 + size0;
        if self.sub_ifds {
            ifd0.push((0x014A, Val::Long(vec![raw_at as u32])));
        }
        let raw_size = if self.raw.is_empty() {
            0
        } else {
            ifd_size(&self.raw, &self.blobs)
        };
        let mut chain_at = Vec::new();
        let mut at = raw_at + raw_size;
        for c in &self.chain {
            chain_at.push(at);
            at += ifd_size(c, &self.blobs);
        }
        let mut blob_at = Vec::new();
        for b in &self.blobs {
            at = at.div_ceil(16) * 16;
            blob_at.push(at);
            at += b.len();
        }
        let mut out = vec![0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00];
        write_ifd(
            &mut out,
            &ifd0,
            chain_at.first().copied().unwrap_or(0),
            &blob_at,
            &self.blobs,
        );
        if !self.raw.is_empty() {
            write_ifd(&mut out, &self.raw, 0, &blob_at, &self.blobs);
        }
        for (i, c) in self.chain.iter().enumerate() {
            let next = chain_at.get(i + 1).copied().unwrap_or(0);
            write_ifd(&mut out, c, next, &blob_at, &self.blobs);
        }
        for (b, &a) in self.blobs.iter().zip(&blob_at) {
            out.resize(a, 0);
            out.extend_from_slice(b);
        }
        out
    }
}

/// The common IFD0 of the fixtures: Make, Model, Orientation (SubIFDs is
/// added by the builder).
pub(super) fn common_ifd0(model: &'static str, orientation: u16) -> Vec<(u16, Val)> {
    vec![
        (0x010F, Val::Ascii("SONY")),
        (0x0110, Val::Ascii(model)),
        (0x0112, Val::Short(vec![orientation])),
    ]
}

/// The five common raw tags of F1 to F4.
pub(super) fn common_raw() -> Vec<(u16, Val)> {
    vec![
        (0x7310, Val::Short(vec![512; 4])),
        (0x7313, Val::SShort(vec![2429, 1024, 1024, 1783])),
        (0x828D, Val::Short(vec![2, 2])),
        (0x828E, Val::Byte(vec![0, 1, 1, 2])),
        (0xC61D, Val::Short(vec![16383])),
    ]
}

/// Raw IFD entries of a strip file: W, H, bits, compression, CFA, one
/// strip of `rows` rows in blob 0.
pub(super) fn strip_raw(w: u32, h: u32, bits: u16, compression: u16, rows: u32) -> Vec<(u16, Val)> {
    let mut r = vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Short(vec![w as u16])),
        (0x0101, Val::Short(vec![h as u16])),
        (0x0102, Val::Short(vec![bits])),
        (0x0103, Val::Short(vec![compression])),
        (0x0106, Val::Short(vec![32803])),
        (0x0111, Val::Blobs(vec![0])),
        (0x0115, Val::Short(vec![1])),
        (0x0116, Val::Short(vec![rows as u16])),
        (0x0117, Val::BlobLens(vec![0])),
    ];
    r.extend(common_raw());
    r
}

/// Parse a hex dump with optional "0000:" address prefixes.
pub(super) fn hex(s: &str) -> Vec<u8> {
    s.lines()
        .flat_map(|l| {
            let l = l.split_once(':').map_or(l, |(_, rest)| rest);
            l.split_whitespace()
                .map(|t| u8::from_str_radix(t, 16).expect("hex byte"))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Fixture F1 (V1, 4 x 2), the complete 304-byte file of the
/// specification.
pub(super) const F1_HEX: &str = "
0000: 49 49 2A 00 08 00 00 00 04 00 0F 01 02 00 05 00
0010: 00 00 3E 00 00 00 10 01 02 00 09 00 00 00 44 00
0020: 00 00 12 01 03 00 01 00 00 00 01 00 00 00 4A 01
0030: 04 00 01 00 00 00 4E 00 00 00 00 00 00 00 53 4F
0040: 4E 59 00 00 49 4C 43 45 2D 37 4D 34 00 00 0F 00
0050: FE 00 04 00 01 00 00 00 00 00 00 00 00 01 03 00
0060: 01 00 00 00 04 00 00 00 01 01 03 00 01 00 00 00
0070: 02 00 00 00 02 01 03 00 01 00 00 00 0E 00 00 00
0080: 03 01 03 00 01 00 00 00 01 00 00 00 06 01 03 00
0090: 01 00 00 00 23 80 00 00 11 01 04 00 01 00 00 00
00A0: 20 01 00 00 15 01 03 00 01 00 00 00 01 00 00 00
00B0: 16 01 03 00 01 00 00 00 02 00 00 00 17 01 04 00
00C0: 01 00 00 00 10 00 00 00 10 73 03 00 04 00 00 00
00D0: 08 01 00 00 13 73 08 00 04 00 00 00 10 01 00 00
00E0: 8D 82 03 00 02 00 00 00 02 00 02 00 8E 82 01 00
00F0: 04 00 00 00 00 01 01 02 1D C6 03 00 01 00 00 00
0100: FF 3F 00 00 00 00 00 00 00 02 00 02 00 02 00 02
0110: 7D 09 00 04 00 04 F7 06 00 00 00 00 00 00 00 00
0120: D0 09 A0 11 D0 09 A0 11 A0 11 E8 05 A0 11 E8 05
";

/// Fixture F3 (V3, 32 x 2), the complete 368-byte file of the
/// specification.
pub(super) const F3_HEX: &str = "
0000: 49 49 2A 00 08 00 00 00 04 00 0F 01 02 00 05 00
0010: 00 00 3E 00 00 00 10 01 02 00 09 00 00 00 44 00
0020: 00 00 12 01 03 00 01 00 00 00 01 00 00 00 4A 01
0030: 04 00 01 00 00 00 4E 00 00 00 00 00 00 00 53 4F
0040: 4E 59 00 00 49 4C 43 45 2D 37 4D 34 00 00 10 00
0050: FE 00 04 00 01 00 00 00 00 00 00 00 00 01 03 00
0060: 01 00 00 00 20 00 00 00 01 01 03 00 01 00 00 00
0070: 02 00 00 00 02 01 03 00 01 00 00 00 0E 00 00 00
0080: 03 01 03 00 01 00 00 00 FF 7F 00 00 06 01 03 00
0090: 01 00 00 00 23 80 00 00 11 01 04 00 01 00 00 00
00A0: 30 01 00 00 15 01 03 00 01 00 00 00 01 00 00 00
00B0: 16 01 03 00 01 00 00 00 02 00 00 00 17 01 04 00
00C0: 01 00 00 00 40 00 00 00 10 70 03 00 04 00 00 00
00D0: 14 01 00 00 10 73 03 00 04 00 00 00 1C 01 00 00
00E0: 13 73 08 00 04 00 00 00 24 01 00 00 8D 82 03 00
00F0: 02 00 00 00 02 00 02 00 8E 82 01 00 04 00 00 00
0100: 00 01 01 02 1D C6 03 00 01 00 00 00 FF 3F 00 00
0110: 00 00 00 00 40 1F A0 28 64 32 14 37 00 02 00 02
0120: 00 02 00 02 7D 09 00 04 00 04 F7 06 00 00 00 00
0130: 68 44 23 04 00 00 00 00 00 00 00 00 00 00 00 00
0140: B8 C5 2D 04 00 00 00 00 00 00 00 00 00 00 00 00
0150: B8 C5 2D 04 00 00 00 00 00 00 00 00 00 00 00 00
0160: F4 A2 17 04 00 00 00 00 00 00 00 00 00 00 00 00
";

/// F1 from its hex dump.
pub(super) fn f1() -> Vec<u8> {
    hex(F1_HEX)
}

/// F2: F1 with BitsPerSample 12, Compression 32767, StripByteCounts 12
/// and the 12 data bytes of the recipe (300 bytes).
pub(super) fn f2() -> Vec<u8> {
    let mut f = f1();
    f[0x7C] = 0x0C;
    f[0x88] = 0xFF;
    f[0x89] = 0x7F;
    f[0xC4] = 0x0C;
    f.truncate(0x120);
    f.extend_from_slice(&hex("74 82 46 74 82 46 68 A4 17 68 A4 17"));
    f
}

/// F3 from its hex dump.
pub(super) fn f3() -> Vec<u8> {
    hex(F3_HEX)
}

/// The F4 tile (32841 bytes).
pub(super) fn f4_tile() -> Vec<u8> {
    let mut t = hex(
        "FF D8 FF C3 00 14 0E 01 00 01 00 04 01 11 00 02 11 00 03 11 00 04 11 00 FF C4 00 16 00 \
         01 01 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0C 0D FF DA 00 0E 04 01 00 02 00 03 \
         00 04 00 01 00 00",
    );
    t.extend_from_slice(&hex("C9 CF 86 7E 19 FC 5E 70"));
    t.extend(std::iter::repeat_n(0u8, 32767));
    t.extend_from_slice(&[0xFF, 0xD9]);
    t
}

/// F4 (V4, one 512 x 512 tile): 17 raw entries.
pub(super) fn f4() -> Vec<u8> {
    let mut raw = vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Short(vec![512])),
        (0x0101, Val::Short(vec![512])),
        (0x0102, Val::Short(vec![14])),
        (0x0103, Val::Short(vec![7])),
        (0x0106, Val::Short(vec![32803])),
        (0x0115, Val::Short(vec![1])),
        (0x0142, Val::Short(vec![512])),
        (0x0143, Val::Short(vec![512])),
        (0x0144, Val::Blobs(vec![0])),
        (0x0145, Val::Long(vec![32841])),
        (0x7038, Val::Long(vec![512, 512])),
    ];
    raw.extend(common_raw());
    Builder {
        ifd0: common_ifd0("ILCE-7M4", 1),
        raw,
        blobs: vec![f4_tile()],
        sub_ifds: true,
        ..Builder::default()
    }
    .build()
}

/// The F5 tile (49220 bytes).
pub(super) fn f5_tile() -> Vec<u8> {
    let mut t = hex(
        "FF D8 FF C3 00 11 10 02 00 02 00 03 01 22 00 02 11 00 03 11 00 FF C4 00 15 00 01 01 \
         00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0F FF DA 00 0C 03 01 00 02 00 03 00 01 \
         00 00",
    );
    t.extend_from_slice(&hex("89 C3 89 FF 00 FC FF 00 FC"));
    t.extend(std::iter::repeat_n(0u8, 49151));
    t.extend_from_slice(&[0xFF, 0xD9]);
    t
}

/// The F5 raw IFD entries (19), tile in blob 0.
pub(super) fn f5_raw() -> Vec<(u16, Val)> {
    vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Short(vec![512])),
        (0x0101, Val::Short(vec![512])),
        (0x0102, Val::Short(vec![15, 15, 15])),
        (0x0103, Val::Short(vec![7])),
        (0x0106, Val::Short(vec![6])),
        (0x0115, Val::Short(vec![3])),
        (0x0142, Val::Short(vec![512])),
        (0x0143, Val::Short(vec![512])),
        (0x0144, Val::Blobs(vec![0])),
        (0x0145, Val::Long(vec![49220])),
        (
            0x0211,
            Val::Rational(vec![(19595, 65536), (38470, 65536), (7471, 65536)]),
        ),
        (0x0212, Val::Short(vec![2, 2])),
        (
            0x0214,
            Val::Rational(vec![
                (0, 1),
                (16383, 1),
                (16384, 1),
                (16384, 1),
                (16384, 1),
                (16384, 1),
            ]),
        ),
        (0x7038, Val::Long(vec![512, 512])),
        (0x7310, Val::Short(vec![512; 4])),
        (0x7313, Val::SShort(vec![2429, 1024, 1024, 1783])),
        (0x828D, Val::Short(vec![2, 2])),
        (0x828E, Val::Byte(vec![255; 4])),
    ]
}

/// F5 (V5, one 512 x 512 tile, size M).
pub(super) fn f5() -> Vec<u8> {
    Builder {
        ifd0: common_ifd0("ILCE-7M4", 1),
        raw: f5_raw(),
        blobs: vec![f5_tile()],
        sub_ifds: true,
        ..Builder::default()
    }
    .build()
}

/// One encoded component for [`lj92`]: sampling factors, samples
/// row-major with `width` columns (whole MCUs).
pub(super) struct Comp<'a> {
    pub h: usize,
    pub v: usize,
    pub samples: &'a [u16],
    pub width: usize,
}

/// Bit writer with FF stuffing.
struct Writer {
    acc: u64,
    n: u32,
    out: Vec<u8>,
}

impl Writer {
    fn put(&mut self, v: u32, len: u32) {
        if len == 0 {
            return;
        }
        self.acc = (self.acc << len) | u64::from(v & ((1u32 << len) - 1));
        self.n += len;
        while self.n >= 8 {
            self.n -= 8;
            let b = (self.acc >> self.n) as u8;
            self.out.push(b);
            if b == 0xFF {
                self.out.push(0);
            }
        }
    }

    /// Pad the last byte with 1-bits.
    fn flush(&mut self) {
        let pad = (8 - self.n % 8) % 8;
        if pad > 0 {
            self.put((1 << pad) - 1, pad);
        }
    }
}

/// Encode a lossless JPEG with predictor 1, one Huffman table of 17
/// five-bit codes, optional restart interval (in MCUs) and the Sony
/// line-start rule when `sony` is set. Written from T.81 Annex H.
pub(super) fn lj92(
    precision: u8,
    x: usize,
    y: usize,
    comps: &[Comp<'_>],
    restart: usize,
    sony: bool,
) -> Vec<u8> {
    let mut out = vec![0xFF, 0xD8];
    let mut sof = vec![precision];
    sof.extend_from_slice(&(y as u16).to_be_bytes());
    sof.extend_from_slice(&(x as u16).to_be_bytes());
    sof.push(comps.len() as u8);
    for (i, c) in comps.iter().enumerate() {
        sof.extend_from_slice(&[i as u8 + 1, ((c.h << 4) | c.v) as u8, 0]);
    }
    out.extend_from_slice(&[0xFF, 0xC3]);
    out.extend_from_slice(&((sof.len() + 2) as u16).to_be_bytes());
    out.extend(sof);
    let mut dht = vec![0x00];
    let mut bits = [0u8; 16];
    bits[4] = 17;
    dht.extend_from_slice(&bits);
    dht.extend(0u8..17);
    out.extend_from_slice(&[0xFF, 0xC4]);
    out.extend_from_slice(&((dht.len() + 2) as u16).to_be_bytes());
    out.extend(dht);
    if restart > 0 {
        out.extend_from_slice(&[0xFF, 0xDD, 0x00, 0x04]);
        out.extend_from_slice(&(restart as u16).to_be_bytes());
    }
    let mut sos = vec![comps.len() as u8];
    for i in 0..comps.len() {
        sos.extend_from_slice(&[i as u8 + 1, 0x00]);
    }
    sos.extend_from_slice(&[1, 0, 0]);
    out.extend_from_slice(&[0xFF, 0xDA]);
    out.extend_from_slice(&((sos.len() + 2) as u16).to_be_bytes());
    out.extend(sos);

    let hmax = comps.iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = comps.iter().map(|c| c.v).max().unwrap_or(1);
    let (mcux, mcuy) = (x.div_ceil(hmax), y.div_ceil(vmax));
    let mut w = Writer {
        acc: 0,
        n: 0,
        out: Vec::new(),
    };
    let initial = 1i32 << (precision - 1);
    let rows_per_interval = if restart > 0 { restart / mcux } else { 0 };
    let mut first = vec![0usize; comps.len()];
    let mut rst = 0u8;
    for my in 0..mcuy {
        if rows_per_interval > 0 && my > 0 && my % rows_per_interval == 0 {
            w.flush();
            w.out.extend_from_slice(&[0xFF, 0xD0 + (rst & 7)]);
            rst = rst.wrapping_add(1);
            for (f, c) in first.iter_mut().zip(comps) {
                *f = my * c.v;
            }
        }
        for mx in 0..mcux {
            for (ci, c) in comps.iter().enumerate() {
                for dv in 0..c.v {
                    let yy = my * c.v + dv;
                    for dh in 0..c.h {
                        let xx = mx * c.h + dh;
                        let s = |px: usize, py: usize| i32::from(c.samples[py * c.width + px]);
                        let pred = if yy == first[ci] {
                            if xx == 0 {
                                initial
                            } else {
                                s(xx - 1, yy)
                            }
                        } else if xx == 0 {
                            if sony && c.v >= 2 && yy % c.v == 0 {
                                s(0, yy - c.v)
                            } else {
                                s(0, yy - 1)
                            }
                        } else {
                            s(xx - 1, yy)
                        };
                        let mut d = (s(xx, yy) - pred) & 0xFFFF;
                        if d >= 32768 {
                            d -= 65536;
                        }
                        if d == -32768 {
                            w.put(16, 5);
                        } else {
                            let ssss = 32 - d.unsigned_abs().leading_zeros();
                            w.put(ssss, 5);
                            let extra = if d >= 0 { d } else { d + (1 << ssss) - 1 };
                            w.put(extra as u32, ssss);
                        }
                    }
                }
            }
        }
    }
    w.flush();
    out.extend(w.out);
    out.extend_from_slice(&[0xFF, 0xD9]);
    out
}

/// A V3 block whose 16 pixels all have code value `c`: MAX = MIN = c,
/// IMAX 0, IMIN 1, all deltas 0 (as in fixture F3).
pub(super) fn flat_block(c: u16) -> [u8; 16] {
    let b = u128::from(c) | (u128::from(c) << 11) | (1u128 << 26);
    b.to_le_bytes()
}

/// A baseline JPEG of `w` x `h` pixels in one color.
pub(super) fn jpeg(w: u16, h: u16, rgb: [u8; 3]) -> Vec<u8> {
    let px: Vec<u8> = (0..usize::from(w) * usize::from(h))
        .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
        .collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 95)
        .encode(&px, w, h, jpeg_encoder::ColorType::Rgba)
        .expect("encode preview");
    out
}

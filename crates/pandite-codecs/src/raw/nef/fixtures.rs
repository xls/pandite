//! Synthetic NEF files for the tests: the specification's fixtures F1 to
//! F10 (section Test vectors, Fixture recipes), built by a builder that
//! follows the recipe layout in either byte order, with optional previews.

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};

/// One IFD value for the builder.
#[derive(Clone, Debug)]
pub(super) enum Val {
    Byte(Vec<u8>),
    Undef(Vec<u8>),
    Ascii(&'static str),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rational(Vec<(u32, u32)>),
    /// LONG file offsets of blobs, by index.
    Blobs(Vec<usize>),
    /// LONG lengths of blobs, by index.
    BlobLens(Vec<usize>),
    /// Any field type and count with its value bytes in file order.
    Raw(u16, u32, Vec<u8>),
}

pub(super) fn u16b(v: u16, le: bool) -> [u8; 2] {
    if le {
        v.to_le_bytes()
    } else {
        v.to_be_bytes()
    }
}

pub(super) fn u32b(v: u32, le: bool) -> [u8; 4] {
    if le {
        v.to_le_bytes()
    } else {
        v.to_be_bytes()
    }
}

/// Field type, count and value bytes of a value.
fn payload(v: &Val, le: bool, blob_at: &[usize], blobs: &[Vec<u8>]) -> (u16, u32, Vec<u8>) {
    match v {
        Val::Byte(b) => (1, b.len() as u32, b.clone()),
        Val::Undef(b) => (7, b.len() as u32, b.clone()),
        Val::Ascii(s) => {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            (2, b.len() as u32, b)
        }
        Val::Short(x) => (
            3,
            x.len() as u32,
            x.iter().flat_map(|&v| u16b(v, le)).collect(),
        ),
        Val::Long(x) => (
            4,
            x.len() as u32,
            x.iter().flat_map(|&v| u32b(v, le)).collect(),
        ),
        Val::Rational(x) => (
            5,
            x.len() as u32,
            x.iter()
                .flat_map(|&(n, d)| [u32b(n, le), u32b(d, le)].concat())
                .collect(),
        ),
        Val::Blobs(i) => (
            4,
            i.len() as u32,
            i.iter()
                .flat_map(|&k| u32b(blob_at.get(k).copied().unwrap_or(0) as u32, le))
                .collect(),
        ),
        Val::BlobLens(i) => (
            4,
            i.len() as u32,
            i.iter()
                .flat_map(|&k| u32b(blobs[k].len() as u32, le))
                .collect(),
        ),
        Val::Raw(t, n, b) => (*t, *n, b.clone()),
    }
}

/// Size of an IFD with its out-of-line values (each at an even offset).
fn ifd_size(entries: &[(u16, Val)], blobs: &[Vec<u8>]) -> usize {
    let mut n = 2 + 12 * entries.len() + 4;
    for (_, v) in entries {
        let (_, _, p) = payload(v, false, &[], blobs);
        if p.len() > 4 {
            n += p.len() + p.len() % 2;
        }
    }
    n
}

/// Write one IFD at the end of `out` (entries in ascending tag order) with
/// its values after it, each at an even offset. Value offsets count from
/// `base`.
fn write_ifd(
    out: &mut Vec<u8>,
    entries: &[(u16, Val)],
    next: usize,
    base: usize,
    le: bool,
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) {
    let at = out.len();
    let mut sorted: Vec<&(u16, Val)> = entries.iter().collect();
    sorted.sort_by_key(|e| e.0);
    let mut values: Vec<u8> = Vec::new();
    let values_at = at + 2 + 12 * sorted.len() + 4;
    out.extend_from_slice(&u16b(sorted.len() as u16, le));
    for (tag, v) in sorted {
        let (typ, count, p) = payload(v, le, blob_at, blobs);
        out.extend_from_slice(&u16b(*tag, le));
        out.extend_from_slice(&u16b(typ, le));
        out.extend_from_slice(&u32b(count, le));
        if p.len() <= 4 {
            let mut q = p.clone();
            q.resize(4, 0);
            out.extend_from_slice(&q);
        } else {
            out.extend_from_slice(&u32b((values_at + values.len() - base) as u32, le));
            values.extend_from_slice(&p);
            if values.len() % 2 == 1 {
                values.push(0);
            }
        }
    }
    out.extend_from_slice(&u32b(next as u32, le));
    out.extend(values);
}

/// The TIFF header of a byte order with the first IFD at 8.
pub(super) fn tiff_header(le: bool) -> Vec<u8> {
    if le {
        vec![0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]
    } else {
        vec![0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]
    }
}

/// A synthetic NEF file following the recipe layout: header, IFD0 with
/// its values, the Exif IFD with the maker note as its value, the raw IFD,
/// further SubIFDs, further IFDs of the chain, then the blobs at multiples
/// of 16.
#[derive(Clone, Debug, Default)]
pub(super) struct Builder {
    /// Little-endian TIFF structure and maker note.
    pub le: bool,
    /// IFD0 entries; SubIFDs and the Exif pointer are added.
    pub ifd0: Vec<(u16, Val)>,
    /// Maker-note entries (offsets count from the maker-note origin); no
    /// Exif IFD when `None`.
    pub maker: Option<Vec<(u16, Val)>>,
    /// A JPEG in a maker-note PreviewIFD (tag 0x0011), stored inside the
    /// maker note.
    pub maker_jpeg: Option<Vec<u8>>,
    /// Raw IFD entries; no raw IFD when empty.
    pub raw: Vec<(u16, Val)>,
    /// SubIFDs listed before the raw IFD in tag 0x014A.
    pub before: Vec<Vec<(u16, Val)>>,
    /// SubIFDs listed after the raw IFD in tag 0x014A.
    pub after: Vec<Vec<(u16, Val)>>,
    /// IFDs chained after IFD0 (IFD1, IFD2, ...).
    pub chain: Vec<Vec<(u16, Val)>>,
    /// Data blobs: sensor strips, previews, thumbnails.
    pub blobs: Vec<Vec<u8>>,
}

/// The entries of an IFD that holds JPEG blob `k`.
pub(super) fn jpeg_ifd(k: usize) -> Vec<(u16, Val)> {
    vec![
        (0x00FE, Val::Long(vec![1])),
        (0x0103, Val::Short(vec![6])),
        (0x0201, Val::Blobs(vec![k])),
        (0x0202, Val::BlobLens(vec![k])),
    ]
}

impl Builder {
    /// The maker-note bytes: the Nikon header, a TIFF header in the same
    /// byte order with IFD offset 8, the maker-note IFD and its values,
    /// then the PreviewIFD and its JPEG when present.
    fn maker_note(&self, entries: &[(u16, Val)]) -> Vec<u8> {
        let le = self.le;
        let mut m = vec![0x4E, 0x69, 0x6B, 0x6F, 0x6E, 0x00, 0x02, 0x10, 0x00, 0x00];
        m.extend(tiff_header(le));
        let mut entries = entries.to_vec();
        if self.maker_jpeg.is_some() {
            entries.push((0x0011, Val::Long(vec![0])));
        }
        // Offsets count from the origin, maker-note byte 10.
        let pifd = 8 + ifd_size(&entries, &[]);
        if let Some(j) = &self.maker_jpeg {
            entries.retain(|e| e.0 != 0x0011);
            entries.push((0x0011, Val::Long(vec![pifd as u32])));
            write_ifd(&mut m, &entries, 0, 10, le, &[], &[]);
            let jpeg_at = pifd + 2 + 24 + 4;
            let p = vec![
                (0x0201, Val::Long(vec![jpeg_at as u32])),
                (0x0202, Val::Long(vec![j.len() as u32])),
            ];
            write_ifd(&mut m, &p, 0, 10, le, &[], &[]);
            m.extend_from_slice(j);
        } else {
            write_ifd(&mut m, &entries, 0, 10, le, &[], &[]);
        }
        m
    }

    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let le = self.le;
        let blobs = &self.blobs;
        let maker = self.maker.as_ref().map(|e| self.maker_note(e));
        let exif = maker
            .as_ref()
            .map(|m| vec![(0x927C, Val::Undef(m.clone()))]);
        let mut subs: Vec<&Vec<(u16, Val)>> = self.before.iter().collect();
        if !self.raw.is_empty() {
            subs.push(&self.raw);
        }
        subs.extend(self.after.iter());
        let ifd0_with = |sub_at: Vec<u32>, exif_at: u32| {
            let mut e = self.ifd0.clone();
            if !sub_at.is_empty() {
                e.push((0x014A, Val::Long(sub_at)));
            }
            if exif.is_some() {
                e.push((0x8769, Val::Long(vec![exif_at])));
            }
            e
        };
        // First pass: positions (sizes do not depend on offset values).
        let mut at = 8 + ifd_size(&ifd0_with(vec![0; subs.len()], 0), blobs);
        let exif_at = at;
        if let Some(x) = &exif {
            at += ifd_size(x, blobs);
        }
        // Physical order: the raw IFD first, then the other SubIFDs.
        let mut physical: Vec<&Vec<(u16, Val)>> = Vec::new();
        if !self.raw.is_empty() {
            physical.push(&self.raw);
        }
        physical.extend(self.before.iter());
        physical.extend(self.after.iter());
        let mut phys_at = Vec::new();
        for s in &physical {
            phys_at.push(at);
            at += ifd_size(s, blobs);
        }
        let mut chain_at = Vec::new();
        for c in &self.chain {
            chain_at.push(at);
            at += ifd_size(c, blobs);
        }
        let mut blob_at = Vec::new();
        for b in blobs {
            at = at.div_ceil(16) * 16;
            blob_at.push(at);
            at += b.len();
        }
        // The SubIFD offsets in list order.
        let pos_of = |s: &Vec<(u16, Val)>| -> u32 {
            let i = physical
                .iter()
                .position(|p| std::ptr::eq(*p, s))
                .expect("SubIFD");
            phys_at[i] as u32
        };
        let sub_at: Vec<u32> = subs.iter().map(|s| pos_of(s)).collect();
        // Second pass: write.
        let mut out = tiff_header(le);
        let next0 = chain_at.first().copied().unwrap_or(0);
        write_ifd(
            &mut out,
            &ifd0_with(sub_at, exif_at as u32),
            next0,
            0,
            le,
            &blob_at,
            blobs,
        );
        if let Some(x) = &exif {
            assert_eq!(out.len(), exif_at);
            write_ifd(&mut out, x, 0, 0, le, &blob_at, blobs);
        }
        for (s, &a) in physical.iter().zip(&phys_at) {
            assert_eq!(out.len(), a);
            write_ifd(&mut out, s, 0, 0, le, &blob_at, blobs);
        }
        for (i, c) in self.chain.iter().enumerate() {
            assert_eq!(out.len(), chain_at[i]);
            let next = chain_at.get(i + 1).copied().unwrap_or(0);
            write_ifd(&mut out, c, next, 0, le, &blob_at, blobs);
        }
        for (b, &a) in blobs.iter().zip(&blob_at) {
            out.resize(a, 0);
            out.extend_from_slice(b);
        }
        out
    }
}

/// The IFD0 entries of the recipe: Make, Model, Orientation.
pub(super) fn ifd0(model: &'static str, orientation: u16) -> Vec<(u16, Val)> {
    vec![
        (0x010F, Val::Ascii("NIKON CORPORATION")),
        (0x0110, Val::Ascii(model)),
        (0x0112, Val::Short(vec![orientation])),
    ]
}

/// The 12 raw IFD entries of the recipe: W x H, bits, compression, one
/// strip of H rows in blob 0, the CFA pattern.
pub(super) fn raw_ifd(
    w: u32,
    h: u32,
    bits: u16,
    compression: u16,
    pattern: [u8; 4],
) -> Vec<(u16, Val)> {
    vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Long(vec![w])),
        (0x0101, Val::Long(vec![h])),
        (0x0102, Val::Short(vec![bits])),
        (0x0103, Val::Short(vec![compression])),
        (0x0106, Val::Short(vec![32803])),
        (0x0111, Val::Blobs(vec![0])),
        (0x0115, Val::Short(vec![1])),
        (0x0116, Val::Long(vec![h])),
        (0x0117, Val::BlobLens(vec![0])),
        (0x828D, Val::Short(vec![2, 2])),
        (0x828E, Val::Byte(pattern.to_vec())),
    ]
}

/// The standard Bayer pattern of the recipes.
pub(super) const RGGB: [u8; 4] = [0, 1, 1, 2];

/// One fixture: byte order, Model, bps, Compression, pattern, maker-note
/// entries and strip bytes.
pub(super) fn recipe(
    le: bool,
    model: &'static str,
    bits: u16,
    compression: u16,
    pattern: [u8; 4],
    maker: Vec<(u16, Val)>,
    strip: Vec<u8>,
) -> Builder {
    Builder {
        le,
        ifd0: ifd0(model, 1),
        maker: Some(maker),
        raw: raw_ifd(4, 2, bits, compression, pattern),
        blobs: vec![strip],
        ..Builder::default()
    }
}

/// Maker-note tag 0x000C with the four rationals n / d.
pub(super) fn wb_tag(r: u32, b: u32, d: u32) -> (u16, Val) {
    (0x000C, Val::Rational(vec![(r, d), (b, d), (d, d), (d, d)]))
}

/// Maker-note tag 0x0093 NEFCompression.
pub(super) fn nefc(v: u16) -> (u16, Val) {
    (0x0093, Val::Short(vec![v]))
}

/// The maker-note entries of F1: WB_RBLevels and BlackLevel 400.
pub(super) fn f1_maker() -> Vec<(u16, Val)> {
    vec![wb_tag(3892, 2817, 2048), (0x003D, Val::Short(vec![400; 4]))]
}

/// The maker-note entries of F4: WB_RBLevels, BlackLevel 1008 and tag
/// 0x0051 with NEFCompression `c` at bytes 10 and 11 (little-endian).
pub(super) fn f4_maker(c: u8) -> Vec<(u16, Val)> {
    let mut m51 = b"01000400".to_vec();
    m51.extend([0x00, 0x00, c, 0x00]);
    m51.extend([0u8; 12]);
    vec![
        wb_tag(609, 1085, 512),
        (0x003D, Val::Short(vec![1008; 4])),
        (0x0051, Val::Undef(m51)),
    ]
}

/// A tag 0x0096 value: version, four predictors, NC and 16-bit values in
/// the byte order, then `pad` zero bytes.
pub(super) fn lin_tag(
    le: bool,
    version: [u8; 2],
    predictor: u16,
    nc: u16,
    values: &[u16],
    pad: usize,
) -> (u16, Val) {
    let mut b = version.to_vec();
    for _ in 0..4 {
        b.extend(u16b(predictor, le));
    }
    b.extend(u16b(nc, le));
    for &v in values {
        b.extend(u16b(v, le));
    }
    b.extend(vec![0u8; pad]);
    (0x0096, Val::Undef(b))
}

/// The 46 bytes of tag 0x0096 of S13 2254 (TV-2).
pub(super) const TV2_LOSSLESS: &str = "46 30 02 00 02 00 02 00 02 00 00 20 B7 28 AA 9D E0 72 5E 90
    1A F6 1E 99 65 82 F0 AF BF 20 D2 D0 2D C8 C7 0C A1 84 C7 58 2D 50 D3 AB 00 00";

/// The F1 recipe: V1, little-endian, NIKON D850, 14 bits.
pub(super) fn f1_builder() -> Builder {
    recipe(
        true,
        "NIKON D850",
        14,
        1,
        RGGB,
        f1_maker(),
        hex("60 0B 30 13 60 0B 30 13 30 13 78 07 30 13 78 07"),
    )
}

/// F1 through the builder.
pub(super) fn f1() -> Vec<u8> {
    f1_builder().build()
}

/// The F2 recipe: V2, big-endian, NIKON D700, 12 bits.
pub(super) fn f2_builder() -> Builder {
    recipe(
        false,
        "NIKON D700",
        12,
        1,
        RGGB,
        vec![wb_tag(461, 338, 256)],
        hex("27 44 68 27 44 68 46 81 7A 46 81 7A"),
    )
}

/// F2 through the builder.
pub(super) fn f2() -> Vec<u8> {
    f2_builder().build()
}

/// The F3 recipe: V4, little-endian, NIKON D850, 12 bits, row stride 16.
pub(super) fn f3_builder() -> Builder {
    let mut m = f1_maker();
    m.push(nefc(6));
    let mut strip = hex("D8 C2 4C D8 C2 4C");
    strip.extend([0u8; 10]);
    strip.extend(hex("CC E4 1D CC E4 1D"));
    strip.extend([0u8; 10]);
    recipe(true, "NIKON D850", 12, 34713, RGGB, m, strip)
}

/// F3 through the builder.
pub(super) fn f3() -> Vec<u8> {
    f3_builder().build()
}

/// The F4 recipe: V5, little-endian, NIKON Z 6, 14 bits, row stride 16.
pub(super) fn f4_builder() -> Builder {
    let mut strip = hex("C0 0D 64 05 DC 40 56");
    strip.extend([0u8; 9]);
    strip.extend(hex("90 15 76 02 59 61 27"));
    strip.extend([0u8; 9]);
    recipe(true, "NIKON Z 6", 14, 34713, RGGB, f4_maker(10), strip)
}

/// F4 through the builder.
pub(super) fn f4() -> Vec<u8> {
    f4_builder().build()
}

/// The F5 recipe: V3, big-endian, NIKON D100, 12 bits, pattern 1 0 2 1.
pub(super) fn f5_builder() -> Builder {
    let mut strip = hex("46 82 74 46 82 74");
    strip.extend([0u8; 10]);
    strip.extend(hex("17 A4 68 17 A4 68"));
    strip.extend([0u8; 10]);
    recipe(
        false,
        "NIKON D100",
        12,
        34713,
        [1, 0, 2, 1],
        vec![nefc(2)],
        strip,
    )
}

/// F5 through the builder.
pub(super) fn f5() -> Vec<u8> {
    f5_builder().build()
}

/// The F6 recipe: V6 table L12, big-endian, NIKON D700.
pub(super) fn f6_builder() -> Builder {
    recipe(
        false,
        "NIKON D700",
        12,
        34713,
        RGGB,
        vec![
            wb_tag(461, 338, 256),
            nefc(3),
            (0x0096, Val::Undef(hex(TV2_LOSSLESS))),
        ],
        hex("BD 3E 9A 3D EF A6 8D 79 F7 80"),
    )
}

/// F6 through the builder.
pub(super) fn f6() -> Vec<u8> {
    f6_builder().build()
}

/// The F7 recipe: V6 table L14, little-endian, NIKON D850.
pub(super) fn f7_builder() -> Builder {
    let mut m = f1_maker();
    m.push(nefc(3));
    m.push(lin_tag(true, [0x46, 0x30], 2048, 34, &[], 34));
    recipe(
        true,
        "NIKON D850",
        14,
        34713,
        RGGB,
        m,
        hex("DD 83 CB 30 FB EF 2C C1 BB FD F0"),
    )
}

/// F7 through the builder.
pub(super) fn f7() -> Vec<u8> {
    f7_builder().build()
}

/// The F8 curve: C(i) = i below 256, 256 + 6 * (i - 256) up to 682.
pub(super) fn f8_curve() -> Vec<u16> {
    (0..683u16)
        .map(|i| if i < 256 { i } else { 256 + 6 * (i - 256) })
        .collect()
}

/// The F8 recipe: V7 table Y12, big-endian, NIKON D70, pattern 2 1 1 0.
pub(super) fn f8_builder() -> Builder {
    let m = vec![
        nefc(1),
        lin_tag(false, [0x44, 0x10], 328, 683, &f8_curve(), 34),
    ];
    recipe(
        false,
        "NIKON D70",
        12,
        34713,
        [2, 1, 1, 0],
        m,
        hex("85 E9 5E F6 94 97 DE"),
    )
}

/// F8 through the builder.
pub(super) fn f8() -> Vec<u8> {
    f8_builder().build()
}

/// The F9 knots: K(j) = 16 j up to 16, then min(4095, 256 + 48 (j - 16)).
pub(super) fn f9_knots() -> Vec<u16> {
    (0..257u32)
        .map(|j| {
            if j <= 16 {
                16 * j
            } else {
                (256 + 48 * (j - 16)).min(4095)
            }
        } as u16)
        .collect()
}

/// The F9 recipe: V8 table Y12 (version 0x44 0x20), big-endian, NIKON D3.
pub(super) fn f9_builder() -> Builder {
    let m = vec![
        wb_tag(432, 375, 256),
        nefc(4),
        lin_tag(false, [0x44, 0x20], 328, 257, &f9_knots(), 98),
    ];
    recipe(
        false,
        "NIKON D3",
        12,
        34713,
        RGGB,
        m,
        hex("9A 7D B7 EF 7D B6 03 DE"),
    )
}

/// F9 through the builder.
pub(super) fn f9() -> Vec<u8> {
    f9_builder().build()
}

/// The F10 knots: K(j) = 16 j up to 64, then min(16383, 1024 + 64 (j -
/// 64)).
pub(super) fn f10_knots() -> Vec<u16> {
    (0..257u32)
        .map(|j| {
            if j <= 64 {
                16 * j
            } else {
                (1024 + 64 * (j - 64)).min(16383)
            }
        } as u16)
        .collect()
}

/// The F10 recipe: V8 table Y14 (version 0x44 0x40), little-endian, NIKON Z 6,
/// NEFCompression 4 in tag 0x0051.
pub(super) fn f10_builder() -> Builder {
    let mut m = f4_maker(4);
    m.push(lin_tag(true, [0x44, 0x40], 1312, 257, &f10_knots(), 98));
    recipe(
        true,
        "NIKON Z 6",
        14,
        34713,
        RGGB,
        m,
        hex("DA 87 ED 13 EF BF 68 92 B7 DF 00"),
    )
}

/// F10 through the builder.
pub(super) fn f10() -> Vec<u8> {
    f10_builder().build()
}

/// Fixture F2 (V2), the complete 364-byte file of the specification.
pub(super) const F2_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 12 00 00 00 4A 01 10 00 02 00 00 00 0B 00 00
0020: 00 5C 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 00 BE 87 69 00 04 00 00
0040: 00 01 00 00 00 68 00 00 00 00 4E 49 4B 4F 4E 20
0050: 43 4F 52 50 4F 52 41 54 49 4F 4E 00 4E 49 4B 4F
0060: 4E 20 44 37 30 30 00 00 00 01 92 7C 00 07 00 00
0070: 00 44 00 00 00 7A 00 00 00 00 4E 69 6B 6F 6E 00
0080: 02 10 00 00 4D 4D 00 2A 00 00 00 08 00 01 00 0C
0090: 00 05 00 00 00 04 00 00 00 1A 00 00 00 00 00 00
00A0: 01 CD 00 00 01 00 00 00 01 52 00 00 01 00 00 00
00B0: 01 00 00 00 01 00 00 00 01 00 00 00 01 00 00 0C
00C0: 00 FE 00 04 00 00 00 01 00 00 00 00 01 00 00 04
00D0: 00 00 00 01 00 00 00 04 01 01 00 04 00 00 00 01
00E0: 00 00 00 02 01 02 00 03 00 00 00 01 00 0C 00 00
00F0: 01 03 00 03 00 00 00 01 00 01 00 00 01 06 00 03
0100: 00 00 00 01 80 23 00 00 01 11 00 04 00 00 00 01
0110: 00 00 01 60 01 15 00 03 00 00 00 01 00 01 00 00
0120: 01 16 00 04 00 00 00 01 00 00 00 02 01 17 00 04
0130: 00 00 00 01 00 00 00 0C 82 8D 00 03 00 00 00 02
0140: 00 02 00 02 82 8E 00 01 00 00 00 04 00 01 01 02
0150: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0160: 27 44 68 27 44 68 46 81 7A 46 81 7A
";

/// Fixture F6 (V6, table L12), the complete 426-byte file of the
/// specification.
pub(super) const F6_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 12 00 00 00 4A 01 10 00 02 00 00 00 0B 00 00
0020: 00 5C 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 04 87 69 00 04 00 00
0040: 00 01 00 00 00 68 00 00 00 00 4E 49 4B 4F 4E 20
0050: 43 4F 52 50 4F 52 41 54 49 4F 4E 00 4E 49 4B 4F
0060: 4E 20 44 37 30 30 00 00 00 01 92 7C 00 07 00 00
0070: 00 8A 00 00 00 7A 00 00 00 00 4E 69 6B 6F 6E 00
0080: 02 10 00 00 4D 4D 00 2A 00 00 00 08 00 03 00 0C
0090: 00 05 00 00 00 04 00 00 00 32 00 93 00 03 00 00
00A0: 00 01 00 03 00 00 00 96 00 07 00 00 00 2E 00 00
00B0: 00 52 00 00 00 00 00 00 01 CD 00 00 01 00 00 00
00C0: 01 52 00 00 01 00 00 00 01 00 00 00 01 00 00 00
00D0: 01 00 00 00 01 00 46 30 02 00 02 00 02 00 02 00
00E0: 00 20 B7 28 AA 9D E0 72 5E 90 1A F6 1E 99 65 82
00F0: F0 AF BF 20 D2 D0 2D C8 C7 0C A1 84 C7 58 2D 50
0100: D3 AB 00 00 00 0C 00 FE 00 04 00 00 00 01 00 00
0110: 00 00 01 00 00 04 00 00 00 01 00 00 00 04 01 01
0120: 00 04 00 00 00 01 00 00 00 02 01 02 00 03 00 00
0130: 00 01 00 0C 00 00 01 03 00 03 00 00 00 01 87 99
0140: 00 00 01 06 00 03 00 00 00 01 80 23 00 00 01 11
0150: 00 04 00 00 00 01 00 00 01 A0 01 15 00 03 00 00
0160: 00 01 00 01 00 00 01 16 00 04 00 00 00 01 00 00
0170: 00 02 01 17 00 04 00 00 00 01 00 00 00 0A 82 8D
0180: 00 03 00 00 00 02 00 02 00 02 82 8E 00 01 00 00
0190: 00 04 00 01 01 02 00 00 00 00 00 00 00 00 00 00
01A0: BD 3E 9A 3D EF A6 8D 79 F7 80
";

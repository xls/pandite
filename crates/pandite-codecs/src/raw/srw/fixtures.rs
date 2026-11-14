//! Synthetic SRW files for the tests: the specification's fixtures F1 to
//! F5 (section Test vectors, Fixture recipes), and a builder that follows
//! the recipe layout in either byte order, with optional previews.

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};

/// One IFD value for the builder.
#[derive(Clone, Debug)]
pub(super) enum Val {
    Byte(Vec<u8>),
    Undef(Vec<u8>),
    Ascii(&'static str),
    Short(Vec<u16>),
    Long(Vec<u32>),
    SLong(Vec<i32>),
    /// LONG file offsets of blobs, by index.
    Blobs(Vec<usize>),
    /// LONG lengths of blobs, by index.
    BlobLens(Vec<usize>),
    /// Any field type and count with its value bytes in file order.
    Raw(u16, u32, Vec<u8>),
}

fn u16b(v: u16, le: bool) -> [u8; 2] {
    if le {
        v.to_le_bytes()
    } else {
        v.to_be_bytes()
    }
}

fn u32b(v: u32, le: bool) -> [u8; 4] {
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
        Val::SLong(x) => (
            9,
            x.len() as u32,
            x.iter().flat_map(|&v| u32b(v as u32, le)).collect(),
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

/// Write one IFD at the end of `out` with its values after it. Value
/// offsets count from `base`. Entries are sorted by tag unless `reverse`
/// asks for descending order.
#[allow(clippy::too_many_arguments)]
fn write_ifd(
    out: &mut Vec<u8>,
    entries: &[(u16, Val)],
    next: usize,
    base: usize,
    le: bool,
    reverse: bool,
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) {
    let at = out.len();
    let mut sorted: Vec<&(u16, Val)> = entries.iter().collect();
    sorted.sort_by_key(|e| e.0);
    if reverse {
        sorted.reverse();
    }
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

/// A synthetic SRW file following the recipe layout: header, IFD0 with its
/// values, the Exif IFD with the maker note as its value, the preview
/// SubIFD, the raw IFD, IFD1 with the thumbnail SubIFD, then the blobs at
/// multiples of 16.
#[derive(Clone, Debug, Default)]
pub(super) struct Builder {
    /// Little-endian TIFF structure.
    pub le: bool,
    /// IFD0 entries; SubIFDs and the Exif pointer are added.
    pub ifd0: Vec<(u16, Val)>,
    /// IFD0 entries in descending tag order.
    pub reverse_ifd0: bool,
    /// Maker-note entries (offsets count from P); no Exif IFD when empty.
    pub maker: Vec<(u16, Val)>,
    /// Raw IFD entries; no raw IFD when empty.
    pub raw: Vec<(u16, Val)>,
    /// Blob index of the full-size preview (first SubIFD of IFD0).
    pub preview: Option<usize>,
    /// Blob index of the thumbnail (SubIFD of IFD1).
    pub thumb: Option<usize>,
    /// Data blobs: sensor strips, previews.
    pub blobs: Vec<Vec<u8>>,
}

impl Builder {
    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let le = self.le;
        let blobs = &self.blobs;
        // The maker note does not depend on its position: its offsets count
        // from its first byte.
        let maker = if self.maker.is_empty() {
            Vec::new()
        } else {
            let mut m = Vec::new();
            write_ifd(&mut m, &self.maker, 0, 0, le, false, &[], blobs);
            m
        };
        let exif = vec![(0x927C, Val::Undef(maker.clone()))];
        let jpeg_ifd = |k: usize| {
            vec![
                (0x0201, Val::Blobs(vec![k])),
                (0x0202, Val::BlobLens(vec![k])),
            ]
        };
        let preview = self.preview.map(jpeg_ifd);
        let thumb = self.thumb.map(jpeg_ifd);
        let sub_count = usize::from(preview.is_some()) + usize::from(!self.raw.is_empty());
        let ifd0_with = |subs: Vec<u32>, exif_at: u32| {
            let mut e = self.ifd0.clone();
            if sub_count > 0 {
                e.push((0x014A, Val::Long(subs)));
            }
            if !maker.is_empty() {
                e.push((0x8769, Val::Long(vec![exif_at])));
            }
            e
        };
        // First pass: positions (sizes do not depend on offset values).
        let ifd0_at = 8;
        let mut at = ifd0_at + ifd_size(&ifd0_with(vec![0; sub_count], 0), blobs);
        let exif_at = at;
        if !maker.is_empty() {
            at += ifd_size(&exif, blobs);
        }
        let preview_at = at;
        if let Some(p) = &preview {
            at += ifd_size(p, blobs);
        }
        let raw_at = at;
        if !self.raw.is_empty() {
            at += ifd_size(&self.raw, blobs);
        }
        let ifd1 = vec![(0x014A, Val::Long(vec![0]))];
        let ifd1_at = at;
        let thumb_at = ifd1_at + ifd_size(&ifd1, blobs);
        if let Some(t) = &thumb {
            at = thumb_at + ifd_size(t, blobs);
        }
        let mut blob_at = Vec::new();
        for b in blobs {
            at = at.div_ceil(16) * 16;
            blob_at.push(at);
            at += b.len();
        }
        // Second pass: write.
        let mut out = if le {
            vec![0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]
        } else {
            vec![0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]
        };
        let mut subs = Vec::new();
        if preview.is_some() {
            subs.push(preview_at as u32);
        }
        if !self.raw.is_empty() {
            subs.push(raw_at as u32);
        }
        let next0 = if thumb.is_some() { ifd1_at } else { 0 };
        let ifd0 = ifd0_with(subs, exif_at as u32);
        write_ifd(
            &mut out,
            &ifd0,
            next0,
            0,
            le,
            self.reverse_ifd0,
            &blob_at,
            blobs,
        );
        if !maker.is_empty() {
            assert_eq!(out.len(), exif_at);
            write_ifd(&mut out, &exif, 0, 0, le, false, &blob_at, blobs);
        }
        if let Some(p) = &preview {
            assert_eq!(out.len(), preview_at);
            write_ifd(&mut out, p, 0, 0, le, false, &blob_at, blobs);
        }
        if !self.raw.is_empty() {
            assert_eq!(out.len(), raw_at);
            write_ifd(&mut out, &self.raw, 0, 0, le, false, &blob_at, blobs);
        }
        if let Some(t) = &thumb {
            assert_eq!(out.len(), ifd1_at);
            let ifd1 = vec![(0x014A, Val::Long(vec![thumb_at as u32]))];
            write_ifd(&mut out, &ifd1, 0, 0, le, false, &blob_at, blobs);
            write_ifd(&mut out, t, 0, 0, le, false, &blob_at, blobs);
        }
        for (b, &a) in blobs.iter().zip(&blob_at) {
            out.resize(a, 0);
            out.extend_from_slice(b);
        }
        out
    }
}

/// The key of TV-3.
pub(super) const KEY: [u32; 11] = [305, 72, 737, 456, 282, 307, 519, 724, 13, 505, 193];

/// The stored value of real value `v` at element `i` of an encrypted tag
/// with sign `s` and start index `k0` (the decryption rule inverted).
pub(super) fn encrypt(real: &[i64], s: i64, k0: usize) -> Vec<i32> {
    real.iter()
        .enumerate()
        .map(|(i, &v)| (v - s * i64::from(KEY[(k0 + i) % 11])) as i32)
        .collect()
}

/// The maker-note entries of the recipe with the given real levels:
/// byte order tag (0x0041 unused or 0x0040), SensorAreas, key, WB levels,
/// black levels, ColorMatrixSRGB and white levels.
pub(super) fn maker(
    order: Option<u32>,
    areas: [u32; 8],
    wb: [i64; 4],
    black: [i64; 4],
    white: i64,
) -> Vec<(u16, Val)> {
    let mut m = vec![
        match order {
            Some(v) => (0x0040, Val::Long(vec![v])),
            None => (0x0041, Val::Long(vec![0])),
        },
        (0xA010, Val::Long(areas.to_vec())),
        (0xA020, Val::Long(KEY.to_vec())),
        (
            0xA021,
            Val::Long(encrypt(&wb, -1, 0).iter().map(|&v| v as u32).collect()),
        ),
        (0xA028, Val::SLong(encrypt(&black, -1, 0))),
        (
            0xA031,
            Val::SLong(encrypt(
                &[430, -152, -22, -36, 352, -60, 4, -128, 380],
                1,
                0,
            )),
        ),
    ];
    let mut levels = vec![0i64; 4];
    levels.extend([white; 8]);
    m.push((
        0xA048,
        Val::Long(encrypt(&levels, 1, 0).iter().map(|&v| v as u32).collect()),
    ));
    m
}

/// The raw IFD entries of the recipe: W x H, bits, compression, one strip
/// of `rows` rows per strip in blob `blob`, CFA 1 0 2 1.
pub(super) fn raw_ifd(w: u16, h: u16, bits: u16, compression: u16, blob: usize) -> Vec<(u16, Val)> {
    vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Short(vec![w])),
        (0x0101, Val::Short(vec![h])),
        (0x0102, Val::Short(vec![bits])),
        (0x0103, Val::Short(vec![compression])),
        (0x0111, Val::Blobs(vec![blob])),
        (0x0115, Val::Short(vec![3])),
        (0x0116, Val::Short(vec![h])),
        (0x0117, Val::BlobLens(vec![blob])),
        (0x828D, Val::Short(vec![2, 2])),
        (0x828E, Val::Byte(vec![1, 0, 2, 1])),
    ]
}

/// The IFD0 entries of the recipe: Make, Model, Orientation.
pub(super) fn ifd0(model: &'static str, orientation: u16) -> Vec<(u16, Val)> {
    vec![
        (0x010F, Val::Ascii("SAMSUNG")),
        (0x0110, Val::Ascii(model)),
        (0x0112, Val::Short(vec![orientation])),
    ]
}

/// V2 rows of 12-bit values, big-endian or little-endian groups (the
/// inverse of R-SRW-27 and R-SRW-28).
pub(super) fn pack12(values: &[u16], le: bool) -> Vec<u8> {
    values
        .chunks_exact(2)
        .flat_map(|p| {
            let (a, b) = (u32::from(p[0]), u32::from(p[1]));
            if le {
                let g = a | (b << 12);
                [g as u8, (g >> 8) as u8, (g >> 16) as u8]
            } else {
                [(a >> 4) as u8, (((a & 15) << 4) | (b >> 8)) as u8, b as u8]
            }
        })
        .collect()
}

/// V1 samples in the given byte order (the inverse of R-SRW-26).
pub(super) fn pack16(values: &[u16], le: bool) -> Vec<u8> {
    values
        .iter()
        .flat_map(|&v| if le { v.to_le_bytes() } else { v.to_be_bytes() })
        .collect()
}

/// The pixel values of the fixture raster: red, green and blue inside the
/// valid area (columns 1 to 6, rows 1 to 4) with pattern 1 0 2 1, and
/// `outside` everywhere else.
pub(super) fn fixture_values(rgb: [u16; 3], outside: u16) -> Vec<u16> {
    let cfa = [1usize, 0, 2, 1];
    let mut v = Vec::new();
    for y in 0..6 {
        for x in 0..8 {
            let inside = (1..7).contains(&x) && (1..5).contains(&y);
            v.push(if inside {
                rgb[cfa[2 * (y % 2) + x % 2]]
            } else {
                outside
            });
        }
    }
    v
}

/// The recipe of F1 to F3 through the builder: `v1` selects F3 (V1,
/// little-endian 16-bit samples, black 12, white 16383), else F1 (V2
/// big-endian groups) or F2 (`order` 0: little-endian groups).
pub(super) fn recipe(le_tiff: bool, v1: bool, order: Option<u32>) -> Builder {
    let (black, white) = if v1 { (12, 16383) } else { (3, 4095) };
    let values = if v1 {
        fixture_values([4000, 6000, 2400], 16383)
    } else {
        fixture_values([1000, 1500, 600], 4095)
    };
    let data_le = match order {
        Some(0) => true,
        Some(_) => false,
        None => le_tiff,
    };
    let strip = if v1 {
        pack16(&values, data_le)
    } else {
        pack12(&values, data_le)
    };
    let (bits, compression) = if v1 { (14, 32769) } else { (12, 32770) };
    Builder {
        le: le_tiff,
        ifd0: ifd0("NX210", 1),
        maker: maker(
            order,
            [0, 0, 8, 6, 1, 1, 7, 5],
            [9776, 4096, 4096, 7232],
            [black; 4],
            white,
        ),
        raw: raw_ifd(8, 6, bits, compression, 0),
        blobs: vec![strip],
        ..Builder::default()
    }
}

/// Fixture F1 (V2, big-endian groups), the complete 600-byte file of the
/// specification.
pub(super) const F1_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 08 00 00 00 4A 01 10 00 02 00 00 00 06 00 00
0020: 00 52 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 84 87 69 00 04 00 00
0040: 00 01 00 00 00 58 00 00 00 00 53 41 4D 53 55 4E
0050: 47 00 4E 58 32 31 30 00 00 01 92 7C 00 07 00 00
0060: 01 1A 00 00 00 6A 00 00 00 00 00 07 00 41 00 04
0070: 00 00 00 01 00 00 00 00 A0 10 00 04 00 00 00 08
0080: 00 00 00 5A A0 20 00 04 00 00 00 0B 00 00 00 7A
0090: A0 21 00 04 00 00 00 04 00 00 00 A6 A0 28 00 09
00A0: 00 00 00 04 00 00 00 B6 A0 31 00 09 00 00 00 09
00B0: 00 00 00 C6 A0 48 00 04 00 00 00 0C 00 00 00 EA
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08
00D0: 00 00 00 06 00 00 00 01 00 00 00 01 00 00 00 07
00E0: 00 00 00 05 00 00 01 31 00 00 00 48 00 00 02 E1
00F0: 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07
0100: 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1
0110: 00 00 27 61 00 00 10 48 00 00 12 E1 00 00 1E 08
0120: 00 00 01 34 00 00 00 4B 00 00 02 E4 00 00 01 CB
0130: 00 00 00 7D FF FF FF 20 FF FF FD 09 FF FF FE 14
0140: 00 00 00 46 FF FF FE 91 FF FF FD FD FF FF FC AC
0150: 00 00 01 6F FF FF FE CF FF FF FF B8 FF FF FD 1F
0160: FF FF FE 38 00 00 0E E5 00 00 0E CC 00 00 0D F8
0170: 00 00 0D 2B 00 00 0F F2 00 00 0E 06 00 00 0F 3E
0180: 00 00 0E CE 00 0B 00 FE 00 04 00 00 00 01 00 00
0190: 00 00 01 00 00 03 00 00 00 01 00 08 00 00 01 01
01A0: 00 03 00 00 00 01 00 06 00 00 01 02 00 03 00 00
01B0: 00 01 00 0C 00 00 01 03 00 03 00 00 00 01 80 02
01C0: 00 00 01 11 00 04 00 00 00 01 00 00 02 10 01 15
01D0: 00 03 00 00 00 01 00 03 00 00 01 16 00 03 00 00
01E0: 00 01 00 06 00 00 01 17 00 04 00 00 00 01 00 00
01F0: 00 48 82 8D 00 03 00 00 00 02 00 02 00 02 82 8E
0200: 00 01 00 00 00 04 01 00 02 01 00 00 00 00 00 00
0210: FF FF FF FF FF FF FF FF FF FF FF FF FF F5 DC 25
0220: 85 DC 25 85 DC 25 8F FF FF F3 E8 5D C3 E8 5D C3
0230: E8 5D CF FF FF F5 DC 25 85 DC 25 85 DC 25 8F FF
0240: FF F3 E8 5D C3 E8 5D C3 E8 5D CF FF FF FF FF FF
0250: FF FF FF FF FF FF FF FF
";

/// Fixture F3 (V1, little-endian 16-bit samples), the complete 624-byte
/// file of the specification.
pub(super) const F3_HEX: &str = "
0000: 4D 4D 00 2A 00 00 00 08 00 05 01 0F 00 02 00 00
0010: 00 08 00 00 00 4A 01 10 00 02 00 00 00 06 00 00
0020: 00 52 01 12 00 03 00 00 00 01 00 01 00 00 01 4A
0030: 00 04 00 00 00 01 00 00 01 84 87 69 00 04 00 00
0040: 00 01 00 00 00 58 00 00 00 00 53 41 4D 53 55 4E
0050: 47 00 4E 58 32 31 30 00 00 01 92 7C 00 07 00 00
0060: 01 1A 00 00 00 6A 00 00 00 00 00 07 00 40 00 04
0070: 00 00 00 01 00 00 00 00 A0 10 00 04 00 00 00 08
0080: 00 00 00 5A A0 20 00 04 00 00 00 0B 00 00 00 7A
0090: A0 21 00 04 00 00 00 04 00 00 00 A6 A0 28 00 09
00A0: 00 00 00 04 00 00 00 B6 A0 31 00 09 00 00 00 09
00B0: 00 00 00 C6 A0 48 00 04 00 00 00 0C 00 00 00 EA
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08
00D0: 00 00 00 06 00 00 00 01 00 00 00 01 00 00 00 07
00E0: 00 00 00 05 00 00 01 31 00 00 00 48 00 00 02 E1
00F0: 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07
0100: 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1
0110: 00 00 27 61 00 00 10 48 00 00 12 E1 00 00 1E 08
0120: 00 00 01 3D 00 00 00 54 00 00 02 ED 00 00 01 D4
0130: 00 00 00 7D FF FF FF 20 FF FF FD 09 FF FF FE 14
0140: 00 00 00 46 FF FF FE 91 FF FF FD FD FF FF FC AC
0150: 00 00 01 6F FF FF FE CF FF FF FF B8 FF FF FD 1F
0160: FF FF FE 38 00 00 3E E5 00 00 3E CC 00 00 3D F8
0170: 00 00 3D 2B 00 00 3F F2 00 00 3E 06 00 00 3F 3E
0180: 00 00 3E CE 00 0B 00 FE 00 04 00 00 00 01 00 00
0190: 00 00 01 00 00 03 00 00 00 01 00 08 00 00 01 01
01A0: 00 03 00 00 00 01 00 06 00 00 01 02 00 03 00 00
01B0: 00 01 00 0E 00 00 01 03 00 03 00 00 00 01 80 01
01C0: 00 00 01 11 00 04 00 00 00 01 00 00 02 10 01 15
01D0: 00 03 00 00 00 01 00 03 00 00 01 16 00 03 00 00
01E0: 00 01 00 06 00 00 01 17 00 04 00 00 00 01 00 00
01F0: 00 60 82 8D 00 03 00 00 00 02 00 02 00 02 82 8E
0200: 00 01 00 00 00 04 01 00 02 01 00 00 00 00 00 00
0210: FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F
0220: FF 3F 70 17 60 09 70 17 60 09 70 17 60 09 FF 3F
0230: FF 3F A0 0F 70 17 A0 0F 70 17 A0 0F 70 17 FF 3F
0240: FF 3F 70 17 60 09 70 17 60 09 70 17 60 09 FF 3F
0250: FF 3F A0 0F 70 17 A0 0F 70 17 A0 0F 70 17 FF 3F
0260: FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F FF 3F
";

/// F1 from its hex dump.
pub(super) fn f1() -> Vec<u8> {
    hex(F1_HEX)
}

/// F2: F1 with maker-note tag 0x0040 = 0 (byte 0x006D) and the six
/// little-endian rows of the recipe.
pub(super) fn f2() -> Vec<u8> {
    let mut f = f1();
    f[0x006D] = 0x40;
    f.truncate(0x210);
    f.extend(hex("
        FF FF FF FF FF FF FF FF FF FF FF FF
        FF CF 5D 58 C2 5D 58 C2 5D 58 F2 FF
        FF 8F 3E DC 85 3E DC 85 3E DC F5 FF
        FF CF 5D 58 C2 5D 58 C2 5D 58 F2 FF
        FF 8F 3E DC 85 3E DC 85 3E DC F5 FF
        FF FF FF FF FF FF FF FF FF FF FF FF"));
    f
}

/// F3 from its hex dump.
pub(super) fn f3() -> Vec<u8> {
    hex(F3_HEX)
}

/// F4 (X2): F1 with Compression 32772 (byte 0x01BF).
pub(super) fn f4() -> Vec<u8> {
    let mut f = f1();
    f[0x01BF] = 0x04;
    f
}

/// F5 (X1): F1 with StripByteCounts 71 (byte 0x01F1).
pub(super) fn f5() -> Vec<u8> {
    let mut f = f1();
    f[0x01F1] = 0x47;
    f
}

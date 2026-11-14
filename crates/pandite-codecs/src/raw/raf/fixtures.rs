//! Synthetic RAF files: the fixtures F1 to F4 of the specification as hex
//! dumps, and a builder that follows the recipe layout for variations.

use crate::raw::arw::fixtures::{hex, jpeg};

/// Fixture F1 (V3, C1, 8 x 8 raster, crop 6 x 6 at (1, 1), X-T3 layout),
/// the complete 672-byte file of the specification.
pub(super) const F1_HEX: &str = "
0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 4C 00 00 01 10 00 00 01 90 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 01 00 00 00 00 00 00 00 FF D9 00 00 00 05
00C0: 01 00 00 04 00 08 00 08 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 06 00 06 01 30 00 04 0C 0C 0C 0C
00E0: 01 31 00 24 01 00 02 01 02 00 02 01 01 00 01 01
00F0: 00 01 01 02 01 01 01 02 00 01 00 02 00 01 01 02
0100: 01 01 02 01 01 00 01 01 00 00 00 00 00 00 00 00
0110: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
0120: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0130: 01 00 00 00 08 00 00 00 02 F0 04 00 01 00 00 00
0140: 08 00 00 00 03 F0 04 00 01 00 00 00 0E 00 00 00
0150: 07 F0 04 00 01 00 00 00 10 01 00 00 08 F0 04 00
0160: 01 00 00 00 80 00 00 00 0A F0 04 00 24 00 00 00
0170: 74 00 00 00 0E F0 04 00 03 00 00 00 04 01 00 00
0180: 00 00 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0190: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01A0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01B0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01C0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01D0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01E0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
01F0: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0200: FE 03 00 00 FE 03 00 00 FE 03 00 00 FE 03 00 00
0210: FE 03 00 00 2E 01 00 00 47 02 00 00 48 02 00 00
0220: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0230: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13
0240: E6 07 CE 0B 9E 13 CE 0B E6 07 9E 13 E6 07 CE 0B
0250: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13
0260: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0270: CE 0B E6 07 9E 13 E6 07 CE 0B 9E 13 CE 0B E6 07
0280: 9E 13 9E 13 CE 0B 9E 13 9E 13 E6 07 9E 13 9E 13
0290: 9E 13 9E 13 E6 07 9E 13 9E 13 CE 0B 9E 13 9E 13
";

/// Fixture F2 (V1, C2, 8 x 4 raster, crop 6 x 2 at (1, 1), orientation 6),
/// the complete 416-byte file of the specification.
pub(super) const F2_HEX: &str = "
0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 24 00 00 00 E0 00 00 00 C0 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 06 00 00 00 00 00 00 00 FF D9 00 00 00 04
00C0: 01 00 00 04 00 04 00 08 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 02 00 06 01 30 00 04 0A 0B 09 08
00E0: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
00F0: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0100: 01 00 00 00 08 00 00 00 02 F0 04 00 01 00 00 00
0110: 04 00 00 00 03 F0 04 00 01 00 00 00 0C 00 00 00
0120: 07 F0 04 00 01 00 00 00 90 00 00 00 08 F0 04 00
0130: 01 00 00 00 30 00 00 00 0A F0 04 00 04 00 00 00
0140: 74 00 00 00 0E F0 04 00 03 00 00 00 84 00 00 00
0150: 00 00 00 00 FE 00 00 00 FF 00 00 00 FF 00 00 00
0160: FE 00 00 00 2E 01 00 00 D2 01 00 00 B8 01 00 00
0170: E7 B4 6D E7 B4 6D E7 B4 6D E7 B4 6D DB F6 41 DB
0180: F6 41 DB F6 41 DB F6 41 E7 B4 6D E7 B4 6D E7 B4
0190: 6D E7 B4 6D DB F6 41 DB F6 41 DB F6 41 DB F6 41
";

/// Fixture F3 (V2, C2, 16 x 4 raster, crop 14 x 2 at (1, 1)), the
/// complete 480-byte file of the specification.
pub(super) const F3_HEX: &str = "
0000: 46 55 4A 49 46 49 4C 4D 43 43 44 2D 52 41 57 20
0010: 30 32 30 31 46 46 30 30 30 30 30 31 46 49 58 54
0020: 55 52 45 00 00 00 00 00 00 00 00 00 00 00 00 00
0030: 00 00 00 00 00 00 00 00 00 00 00 00 30 31 30 30
0040: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0050: 00 00 00 00 00 00 00 94 00 00 00 28 00 00 00 BC
0060: 00 00 00 24 00 00 00 E0 00 00 01 00 00 00 00 00
0070: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0080: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0090: 00 00 00 00 FF D8 FF E1 00 22 45 78 69 66 00 00
00A0: 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00
00B0: 00 00 01 00 00 00 00 00 00 00 FF D9 00 00 00 04
00C0: 01 00 00 04 00 04 00 10 01 10 00 04 00 01 00 01
00D0: 01 11 00 04 00 02 00 0E 01 30 00 04 0A 0B 09 08
00E0: 49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00
00F0: 00 00 1A 00 00 00 00 00 00 00 07 00 01 F0 04 00
0100: 01 00 00 00 10 00 00 00 02 F0 04 00 01 00 00 00
0110: 04 00 00 00 03 F0 04 00 01 00 00 00 0E 00 00 00
0120: 07 F0 04 00 01 00 00 00 90 00 00 00 08 F0 04 00
0130: 01 00 00 00 70 00 00 00 0A F0 04 00 04 00 00 00
0140: 74 00 00 00 0E F0 04 00 03 00 00 00 84 00 00 00
0150: 00 00 00 00 00 04 00 00 00 04 00 00 00 04 00 00
0160: 00 04 00 00 00 01 00 00 A5 01 00 00 FE 01 00 00
0170: 02 3A 41 2F 2F A0 13 F4 F4 02 3A 41 41 2F A0 13
0180: 13 F4 02 3A 3A 41 2F A0 A0 13 F4 02 84 7E 80 4E
0190: 4E E8 07 E8 E8 84 7E 80 80 4E E8 07 07 E8 84 7E
01A0: 7E 80 4E E8 E8 07 E8 84 02 3A 41 2F 2F A0 13 F4
01B0: F4 02 3A 41 41 2F A0 13 13 F4 02 3A 3A 41 2F A0
01C0: A0 13 F4 02 84 7E 80 4E 4E E8 07 E8 E8 84 7E 80
01D0: 80 4E E8 07 07 E8 84 7E 7E 80 4E E8 E8 07 E8 84
";

/// F1 from its hex dump.
pub(super) fn f1() -> Vec<u8> {
    hex(F1_HEX)
}

/// F2 from its hex dump.
pub(super) fn f2() -> Vec<u8> {
    hex(F2_HEX)
}

/// F3 from its hex dump.
pub(super) fn f3() -> Vec<u8> {
    hex(F3_HEX)
}

/// F4: F1 with RAFCompression 2 at byte 0x6F (X1).
pub(super) fn f4() -> Vec<u8> {
    let mut f = f1();
    f[0x6F] = 2;
    f
}

/// Record 0x0131 of the X-T3 (TV-4).
pub(super) const XT3: [u8; 36] = [
    1, 0, 2, 1, 2, 0, 2, 1, 1, 0, 1, 1, 0, 1, 1, 2, 1, 1, 1, 2, 0, 1, 0, 2, 0, 1, 1, 2, 1, 1, 2, 1,
    1, 0, 1, 1,
];

/// Record 0x0131 of the X-T1 (TV-4).
pub(super) const XT1: [u8; 36] = [
    2, 1, 1, 0, 1, 1, 0, 1, 1, 2, 1, 1, 1, 2, 0, 1, 0, 2, 0, 1, 1, 2, 1, 1, 2, 1, 1, 0, 1, 1, 1, 0,
    2, 1, 2, 0,
];

/// The FujiLayout record of the Bayer layout.
pub(super) const BAYER: [u8; 4] = [0x0A, 0x0B, 0x09, 0x08];

/// Record 0x0131 for a pattern given as rows of colors (inverse of
/// R-RAF-18: byte 35 - (6y + x) holds the color of (x, y)).
pub(super) fn xtrans_record(rows: [[u8; 6]; 6]) -> Vec<u8> {
    let mut r = vec![0u8; 36];
    for (y, row) in rows.iter().enumerate() {
        for (x, &c) in row.iter().enumerate() {
            r[35 - (6 * y + x)] = c;
        }
    }
    r
}

/// The 40-byte JPEG stub of the recipes: SOI, an Exif segment with only
/// an orientation, EOI.
pub(super) fn stub(orientation: u16) -> Vec<u8> {
    let mut s = hex("FF D8 FF E1 00 22 45 78 69 66 00 00 49 49 2A 00 08 00 00 00 01 00 12 01 03 00 01 00 00 00 00 00 00 00 00 00 00 00 FF D9");
    s[30..32].copy_from_slice(&orientation.to_le_bytes());
    s
}

/// A baseline JPEG of `w` x `h` pixels in one color with an Exif segment
/// right after SOI: IFD0 with the orientation, and an IFD1 thumbnail when
/// `thumb` is given (stored inside the segment).
pub(super) fn jpeg_exif(
    w: u16,
    h: u16,
    rgb: [u8; 3],
    orientation: u16,
    thumb: Option<Vec<u8>>,
) -> Vec<u8> {
    let base = jpeg(w, h, rgb);
    let mut t = b"II*\0".to_vec();
    t.extend_from_slice(&8u32.to_le_bytes());
    // IFD0 at 8: one entry, next IFD at 26 when a thumbnail follows.
    t.extend_from_slice(&1u16.to_le_bytes());
    t.extend_from_slice(&0x0112u16.to_le_bytes());
    t.extend_from_slice(&3u16.to_le_bytes());
    t.extend_from_slice(&1u32.to_le_bytes());
    t.extend_from_slice(&u32::from(orientation).to_le_bytes());
    let next: u32 = if thumb.is_some() { 26 } else { 0 };
    t.extend_from_slice(&next.to_le_bytes());
    if let Some(th) = thumb {
        // IFD1 at 26: offset and length, next 0, the thumbnail at 56.
        t.extend_from_slice(&2u16.to_le_bytes());
        for (tag, v) in [(0x0201u16, 56u32), (0x0202, th.len() as u32)] {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&4u16.to_le_bytes());
            t.extend_from_slice(&1u32.to_le_bytes());
            t.extend_from_slice(&v.to_le_bytes());
        }
        t.extend_from_slice(&0u32.to_le_bytes());
        t.extend_from_slice(&th);
    }
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((t.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&t);
    out.extend_from_slice(&base[2..]);
    out
}

/// Pack 12-bit values as V1 rows (inverse of R-RAF-23).
pub(super) fn pack_v1(values: &[u16]) -> Vec<u8> {
    values
        .chunks_exact(2)
        .flat_map(|p| {
            let (a, b) = (p[0] & 0xFFF, p[1] & 0xFFF);
            [
                (a & 0xFF) as u8,
                ((a >> 8) | ((b & 0xF) << 4)) as u8,
                (b >> 4) as u8,
            ]
        })
        .collect()
}

/// Pack 14-bit values as V2 rows of `w` pixels (inverse of R-RAF-25).
pub(super) fn pack_v2(values: &[u16], w: usize) -> Vec<u8> {
    let mut out = Vec::new();
    for row in values.chunks_exact(w) {
        let (mut acc, mut n) = (0u64, 0u32);
        for &v in row {
            acc = (acc << 14) | u64::from(v & 0x3FFF);
            n += 14;
            while n >= 32 {
                n -= 32;
                out.extend_from_slice(&((acc >> n) as u32).to_le_bytes());
            }
            acc &= (1u64 << n) - 1;
        }
    }
    out
}

/// LE16 values as a V3 strip.
pub(super) fn pack_v3(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Value of tags 0xF007 and 0xF008 that the builder fills in.
pub(super) const AUTO: u32 = u32::MAX;

/// A RAF file following the recipe layout: header, JPEG at 0x94, meta
/// container after it, CFA container at the next multiple of 16 with IFD0
/// at 8, the FujiIFD after IFD0, its out-of-line values, then the strip at
/// the next multiple of 16 inside the container.
#[derive(Clone, Debug)]
pub(super) struct Builder {
    /// Bytes 16 to 19.
    pub version: [u8; 4],
    /// RAFCompression at 108.
    pub compression: u32,
    /// BE32 at 128.
    pub second_cfa: u32,
    /// The embedded JPEG.
    pub jpeg: Vec<u8>,
    /// Meta records in order.
    pub records: Vec<(u16, Vec<u8>)>,
    /// Record count written instead of the number of records.
    pub record_count: Option<u32>,
    /// IFD0 entries (tag, type, count, value); `None` writes the entry
    /// 0xF000 type 13 count 1 with the FujiIFD offset.
    pub ifd0: Option<Vec<(u16, u16, u32, u32)>>,
    /// FujiIFD entries (tag, type, values). Types 3 and 4 store 2 and 4
    /// bytes per value, any other type 4 bytes.
    pub fuji: Vec<(u16, u16, Vec<u32>)>,
    /// The sensor data.
    pub strip: Vec<u8>,
}

impl Builder {
    /// A file of the recipes' common part: `layout` is record 0x0131 (36
    /// bytes, with 0x0130 = 0C 0C 0C 0C) or empty for Bayer (0x0130 =
    /// 0A 0B 09 08); crop (left, top, width, height); black levels; WB
    /// levels G, R, B.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        w: u32,
        h: u32,
        bits: u32,
        layout: &[u8],
        crop: (u16, u16, u16, u16),
        black: Vec<u32>,
        wb: [u32; 3],
        orientation: u16,
        strip: Vec<u8>,
    ) -> Self {
        let pair = |a: u16, b: u16| [a.to_be_bytes(), b.to_be_bytes()].concat();
        let mut records = vec![
            (0x0100, pair(h as u16, w as u16)),
            (0x0110, pair(crop.1, crop.0)),
            (0x0111, pair(crop.3, crop.2)),
        ];
        if layout.is_empty() {
            records.push((0x0130, BAYER.to_vec()));
        } else {
            records.push((0x0130, vec![0x0C; 4]));
            records.push((0x0131, layout.to_vec()));
        }
        Builder {
            version: *b"0201",
            compression: 0,
            second_cfa: 0,
            jpeg: stub(orientation),
            records,
            record_count: None,
            ifd0: None,
            fuji: vec![
                (0xF001, 4, vec![w]),
                (0xF002, 4, vec![h]),
                (0xF003, 4, vec![bits]),
                (0xF007, 4, vec![AUTO]),
                (0xF008, 4, vec![AUTO]),
                (0xF00A, 4, black),
                (0xF00E, 4, wb.to_vec()),
            ],
            strip,
        }
    }

    /// Replace or add a FujiIFD entry.
    pub(super) fn tag(mut self, tag: u16, typ: u16, values: Vec<u32>) -> Self {
        match self.fuji.iter_mut().find(|e| e.0 == tag) {
            Some(e) => *e = (tag, typ, values),
            None => self.fuji.push((tag, typ, values)),
        }
        self
    }

    /// Remove a FujiIFD entry.
    pub(super) fn untag(mut self, tag: u16) -> Self {
        self.fuji.retain(|e| e.0 != tag);
        self
    }

    /// Replace, add (`Some`) or remove (`None`) a meta record.
    pub(super) fn record(mut self, tag: u16, data: Option<Vec<u8>>) -> Self {
        match (self.records.iter().position(|r| r.0 == tag), data) {
            (Some(i), Some(d)) => self.records[i].1 = d,
            (None, Some(d)) => self.records.push((tag, d)),
            (Some(i), None) => {
                self.records.remove(i);
            }
            (None, None) => {}
        }
        self
    }

    /// The file bytes.
    pub(super) fn build(&self) -> Vec<u8> {
        let mut meta = self
            .record_count
            .unwrap_or(self.records.len() as u32)
            .to_be_bytes()
            .to_vec();
        for (tag, d) in &self.records {
            meta.extend_from_slice(&tag.to_be_bytes());
            meta.extend_from_slice(&(d.len() as u16).to_be_bytes());
            meta.extend_from_slice(d);
        }
        let jpeg_at = 0x94usize;
        let meta_at = jpeg_at + self.jpeg.len();
        let cfa_at = (meta_at + meta.len()).next_multiple_of(16);

        // The CFA container: TIFF header, IFD0, FujiIFD, values, strip.
        let n0 = self.ifd0.as_ref().map_or(1, Vec::len);
        let fuji_at = 8 + 2 + 12 * n0 + 4;
        let n = self.fuji.len();
        let mut values_at = fuji_at + 2 + 12 * n + 4;
        let size = |typ: u16| if typ == 3 { 2 } else { 4 };
        let mut blobs = Vec::new();
        let mut fields = Vec::new();
        for (tag, typ, vals) in &self.fuji {
            let bytes: Vec<u8> = vals
                .iter()
                .flat_map(|&v| {
                    if *typ == 3 {
                        (v as u16).to_le_bytes().to_vec()
                    } else {
                        v.to_le_bytes().to_vec()
                    }
                })
                .collect();
            let count = (bytes.len() / size(*typ)) as u32;
            let field = if bytes.len() <= 4 {
                let mut f = [0u8; 4];
                f[..bytes.len()].copy_from_slice(&bytes);
                f
            } else {
                let at = values_at as u32;
                values_at += bytes.len();
                blobs.extend_from_slice(&bytes);
                at.to_le_bytes()
            };
            fields.push((*tag, *typ, count, field));
        }
        let strip_at = values_at.next_multiple_of(16);
        for f in &mut fields {
            if f.0 == 0xF007 && f.3 == AUTO.to_le_bytes() {
                f.3 = (strip_at as u32).to_le_bytes();
            }
            if f.0 == 0xF008 && f.3 == AUTO.to_le_bytes() {
                f.3 = (self.strip.len() as u32).to_le_bytes();
            }
        }
        let mut c = vec![0x49, 0x49, 0x2A, 0x00, 8, 0, 0, 0];
        let ifd0 = self
            .ifd0
            .clone()
            .unwrap_or_else(|| vec![(0xF000, 13, 1, fuji_at as u32)]);
        c.extend_from_slice(&(ifd0.len() as u16).to_le_bytes());
        for (tag, typ, count, value) in ifd0 {
            c.extend_from_slice(&tag.to_le_bytes());
            c.extend_from_slice(&typ.to_le_bytes());
            c.extend_from_slice(&count.to_le_bytes());
            c.extend_from_slice(&value.to_le_bytes());
        }
        c.extend_from_slice(&[0; 4]);
        c.extend_from_slice(&(n as u16).to_le_bytes());
        for (tag, typ, count, field) in fields {
            c.extend_from_slice(&tag.to_le_bytes());
            c.extend_from_slice(&typ.to_le_bytes());
            c.extend_from_slice(&count.to_le_bytes());
            c.extend_from_slice(&field);
        }
        c.extend_from_slice(&[0; 4]);
        c.extend_from_slice(&blobs);
        c.resize(strip_at, 0);
        c.extend_from_slice(&self.strip);

        let mut f = Vec::new();
        f.extend_from_slice(b"FUJIFILMCCD-RAW ");
        f.extend_from_slice(&self.version);
        f.extend_from_slice(b"FF000001");
        let mut model = b"FIXTURE".to_vec();
        model.resize(32, 0);
        f.extend_from_slice(&model);
        f.extend_from_slice(b"0100");
        f.resize(84, 0);
        for v in [
            jpeg_at as u32,
            self.jpeg.len() as u32,
            meta_at as u32,
            meta.len() as u32,
            cfa_at as u32,
            c.len() as u32,
            self.compression,
        ] {
            f.extend_from_slice(&v.to_be_bytes());
        }
        f.resize(128, 0);
        f.extend_from_slice(&self.second_cfa.to_be_bytes());
        f.resize(jpeg_at, 0);
        f.extend_from_slice(&self.jpeg);
        f.extend_from_slice(&meta);
        f.resize(cfa_at, 0);
        f.extend_from_slice(&c);
        f
    }
}

/// F1 by the builder: V3, C1, 8 x 8, X-T3 layout, flat field R 3022,
/// G 5022, B 2022.
pub(super) fn f1_builder() -> Builder {
    let pattern = pattern_of(&XT3);
    let values: Vec<u16> = (0..64)
        .map(|i| [3022u16, 5022, 2022][usize::from(pattern[6 * ((i / 8) % 6) + (i % 8) % 6])])
        .collect();
    Builder::new(
        8,
        8,
        14,
        &XT3,
        (1, 1, 6, 6),
        vec![1022; 36],
        [302, 583, 584],
        1,
        pack_v3(&values),
    )
}

/// F2 by the builder: V1, C2, 8 x 4, orientation 6.
pub(super) fn f2_builder() -> Builder {
    let values: Vec<u16> = (0..32)
        .map(|i| match ((i % 8) % 2, (i / 8) % 2) {
            (0, 0) => 1255,
            (1, 1) => 1055,
            _ => 1755,
        })
        .collect();
    Builder::new(
        8,
        4,
        12,
        &[],
        (1, 1, 6, 2),
        vec![254, 255, 255, 254],
        [302, 466, 440],
        6,
        pack_v1(&values),
    )
}

/// F3 by the builder: V2, C2, 16 x 4.
pub(super) fn f3_builder() -> Builder {
    let values: Vec<u16> = (0..64)
        .map(|i| match ((i % 16) % 2, (i / 16) % 2) {
            (0, 0) => 3024,
            (1, 1) => 2024,
            _ => 5024,
        })
        .collect();
    Builder::new(
        16,
        4,
        14,
        &[],
        (1, 1, 14, 2),
        vec![1024; 4],
        [256, 421, 510],
        1,
        pack_v2(&values, 16),
    )
}

/// The pattern of a 0x0131 record by R-RAF-18 (index 6 * y + x).
pub(super) fn pattern_of(record: &[u8; 36]) -> [u8; 36] {
    std::array::from_fn(|i| record[35 - i])
}

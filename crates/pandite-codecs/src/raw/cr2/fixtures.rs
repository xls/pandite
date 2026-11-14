//! Synthetic CR2 files for the tests: the specification's fixtures F1 to
//! F4 (section Test vectors, Fixture recipes), and a builder that lays
//! out complete CR2 files (header, IFD0 with the preview strip, IFD1 with
//! the thumbnail, Exif IFD, MakerNote, raw IFD, LJ92 stream) from a raster,
//! with the stream written by the T.81 encoder of the ARW test fixtures.

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};
use crate::raw::arw::fixtures::{lj92, Comp};

/// One IFD value for the builder.
#[derive(Clone, Debug)]
pub(super) enum V {
    Short(Vec<u16>),
    Long(Vec<u32>),
    SShort(Vec<i16>),
    Ascii(&'static str),
    /// UNDEFINED bytes.
    Undef(Vec<u8>),
    /// LONG file offset of blob k.
    Blob(usize),
    /// LONG length of blob k.
    BlobLen(usize),
    /// LONG file offset of IFD k.
    Ifd(usize),
    /// UNDEFINED value that is IFD k itself (the MakerNote): count = its
    /// size, value offset = its file offset.
    Inner(usize),
}

/// Type, count and either the value bytes or a value offset.
type Payload = (u16, u32, Result<Vec<u8>, u32>);

fn payload(
    v: &V,
    ifd_at: &[usize],
    ifd_size: &[usize],
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) -> Payload {
    let at = |list: &[usize], k: usize| list.get(k).copied().unwrap_or(0) as u32;
    let le16 = |x: &[u16]| x.iter().flat_map(|v| v.to_le_bytes()).collect();
    match v {
        V::Short(x) => (3, x.len() as u32, Ok(le16(x))),
        V::Long(x) => (
            4,
            x.len() as u32,
            Ok(x.iter().flat_map(|v| v.to_le_bytes()).collect()),
        ),
        V::SShort(x) => (
            8,
            x.len() as u32,
            Ok(x.iter().flat_map(|v| v.to_le_bytes()).collect()),
        ),
        V::Ascii(s) => {
            let mut b = s.as_bytes().to_vec();
            b.push(0);
            (2, b.len() as u32, Ok(b))
        }
        V::Undef(b) => (7, b.len() as u32, Ok(b.clone())),
        V::Blob(k) => (4, 1, Ok(at(blob_at, *k).to_le_bytes().to_vec())),
        V::BlobLen(k) => {
            let len = blobs.get(*k).map_or(0, Vec::len) as u32;
            (4, 1, Ok(len.to_le_bytes().to_vec()))
        }
        V::Ifd(k) => (4, 1, Ok(at(ifd_at, *k).to_le_bytes().to_vec())),
        V::Inner(k) => (7, at(ifd_size, *k), Err(at(ifd_at, *k))),
    }
}

/// Size of an IFD with its out-of-line values (each at an even offset).
fn ifd_size(entries: &[(u16, V)], blobs: &[Vec<u8>]) -> usize {
    let mut n = 2 + 12 * entries.len() + 4;
    for (_, v) in entries {
        if let (_, _, Ok(p)) = payload(v, &[], &[], &[], blobs) {
            if p.len() > 4 {
                n += p.len() + p.len() % 2;
            }
        }
    }
    n
}

/// A CR2 file: IFDs in file order after the 16-byte header, then blobs at
/// multiples of 16.
#[derive(Clone, Debug, Default)]
pub(super) struct Builder {
    /// The IFDs, in file order.
    pub ifds: Vec<Vec<(u16, V)>>,
    /// The main chain by IFD index: IFD0 first.
    pub chain: Vec<usize>,
    /// The IFD whose offset goes into header bytes 12 to 15 (None: 0).
    pub raw: Option<usize>,
    /// Data blobs: previews and the raw strip.
    pub blobs: Vec<Vec<u8>>,
    /// Write blob 0 after all other blobs.
    pub strip_last: bool,
}

impl Builder {
    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let sizes: Vec<usize> = self.ifds.iter().map(|e| ifd_size(e, &self.blobs)).collect();
        let mut ifd_at = Vec::new();
        let mut at = 16;
        for s in &sizes {
            ifd_at.push(at);
            at += s;
        }
        let mut blob_at = vec![0; self.blobs.len()];
        let mut order: Vec<usize> = (0..self.blobs.len()).collect();
        if self.strip_last && !order.is_empty() {
            order.rotate_left(1);
        }
        for &k in &order {
            at = at.div_ceil(16) * 16;
            blob_at[k] = at;
            at += self.blobs[k].len();
        }
        let first = self.chain.first().map_or(0, |&k| ifd_at[k] as u32);
        let raw = self.raw.map_or(0, |k| ifd_at[k] as u32);
        let mut out = vec![0x49, 0x49, 0x2A, 0x00];
        out.extend_from_slice(&first.to_le_bytes());
        out.extend_from_slice(&[0x43, 0x52, 0x02, 0x00]);
        out.extend_from_slice(&raw.to_le_bytes());
        for (i, entries) in self.ifds.iter().enumerate() {
            let next = self
                .chain
                .iter()
                .position(|&k| k == i)
                .and_then(|p| self.chain.get(p + 1))
                .map_or(0, |&k| ifd_at[k]);
            let mut sorted: Vec<&(u16, V)> = entries.iter().collect();
            sorted.sort_by_key(|e| e.0);
            let values_at = ifd_at[i] + 2 + 12 * sorted.len() + 4;
            let mut values: Vec<u8> = Vec::new();
            out.extend_from_slice(&(sorted.len() as u16).to_le_bytes());
            for (tag, v) in sorted {
                let (typ, count, p) = payload(v, &ifd_at, &sizes, &blob_at, &self.blobs);
                out.extend_from_slice(&tag.to_le_bytes());
                out.extend_from_slice(&typ.to_le_bytes());
                out.extend_from_slice(&count.to_le_bytes());
                match p {
                    Err(offset) => out.extend_from_slice(&offset.to_le_bytes()),
                    Ok(p) if p.len() <= 4 => {
                        let mut q = p;
                        q.resize(4, 0);
                        out.extend_from_slice(&q);
                    }
                    Ok(p) => {
                        let o = (values_at + values.len()) as u32;
                        out.extend_from_slice(&o.to_le_bytes());
                        values.extend_from_slice(&p);
                        if values.len() % 2 == 1 {
                            values.push(0);
                        }
                    }
                }
            }
            out.extend_from_slice(&(next as u32).to_le_bytes());
            out.extend(values);
        }
        for &k in &order {
            out.resize(blob_at[k], 0);
            out.extend_from_slice(&self.blobs[k]);
        }
        out
    }
}

/// R-CR2-17 as the specification writes it: the raster (x, y) of sample
/// index s for slices n, a, b (n >= 1) of a raster of height h, or one
/// slice of width w.
pub(super) fn slice_position(
    s: usize,
    slices: Option<(usize, usize, usize)>,
    w: usize,
    h: usize,
) -> (usize, usize) {
    match slices {
        None => (s % w, s / w),
        Some((n, a, b)) => {
            let j = n.min(s / (a * h));
            let q = s - j * a * h;
            let sw = if j < n { a } else { b };
            (j * a + q % sw, q / sw)
        }
    }
}

/// SensorInfo of 17 values for a W x H raster with visible area L, T, R,
/// B and the black rectangle BL, BT, BR, BB.
pub(super) fn sensor_info(w: usize, h: usize, v: [usize; 4], mask: [usize; 4]) -> Vec<u16> {
    let mut si = vec![34, w as u16, h as u16, 1, 1];
    si.extend(v.iter().map(|&x| x as u16));
    si.extend(mask.iter().map(|&x| x as u16));
    si.extend([0u16; 4]);
    si
}

/// ColorData of 68 values with version `version` (index 0) and the WB
/// values at index 63 (ColorData3 to 8 layout).
pub(super) fn color_data_63(version: i16, wb: [i16; 4]) -> V {
    let mut v = vec![0i16; 68];
    v[0] = version;
    v[63..67].copy_from_slice(&wb);
    V::SShort(v)
}

/// A synthetic V1 file: a raster encoded in a frame of the chosen layout,
/// with the tags of the fixture recipes, optional previews, and hooks to
/// replace or drop entries.
#[derive(Clone, Debug)]
pub(super) struct Recipe {
    /// Raster W x H, row-major.
    pub raster: Vec<u16>,
    /// Raster width W.
    pub w: usize,
    /// Raster height H.
    pub h: usize,
    /// Components Nf.
    pub nf: usize,
    /// Precision P.
    pub precision: u8,
    /// Layout C: one frame line holds two raster rows.
    pub layout_c: bool,
    /// Slices n, a, b used by the encoding (n >= 1).
    pub slices: Option<(usize, usize, usize)>,
    /// The values of tag 0xC640 written (None: no tag).
    pub slice_tag: Option<Vec<u16>>,
    /// Restart interval in MCUs (0: none).
    pub restart: usize,
    /// SensorInfo values.
    pub sensor_info: Vec<u16>,
    /// Raw IFD entries added or replacing the defaults.
    pub raw_extra: Vec<(u16, V)>,
    /// Raw IFD tags removed.
    pub raw_drop: Vec<u16>,
    /// MakerNote entries added or replacing the defaults.
    pub maker_extra: Vec<(u16, V)>,
    /// MakerNote tags removed.
    pub maker_drop: Vec<u16>,
    /// IFD0 Model.
    pub model: &'static str,
    /// IFD0 Orientation.
    pub orientation: u16,
    /// The IFD0 strip preview.
    pub preview: Option<Vec<u8>>,
    /// The IFD1 thumbnail.
    pub thumbnail: Option<Vec<u8>>,
    /// A stream that replaces the encoded one.
    pub stream: Option<Vec<u8>>,
    /// Leave the MakerNote out of the Exif IFD.
    pub no_maker_note: bool,
    /// IFDs chained after the raw IFD (IFD4 of dual pixel files).
    pub after_raw: Vec<Vec<(u16, V)>>,
}

impl Recipe {
    /// A W x H raster with visible area `v` (L, T, R, B), the value
    /// `black` outside it and `f(x, y)` inside; Nf, P and the default tags
    /// of fixture F1 (CanonModelID 0x80000285, ColorData version 10 with
    /// WB 2429 1024 1024 1783, CFA value 1, one slice).
    pub(super) fn new(
        (w, h): (usize, usize),
        nf: usize,
        precision: u8,
        v: [usize; 4],
        black: u16,
        f: impl Fn(usize, usize) -> u16,
    ) -> Self {
        let mut raster = vec![black; w * h];
        for y in v[1]..=v[3] {
            for x in v[0]..=v[2] {
                raster[y * w + x] = f(x, y);
            }
        }
        Recipe {
            raster,
            w,
            h,
            nf,
            precision,
            layout_c: false,
            slices: None,
            slice_tag: None,
            restart: 0,
            sensor_info: sensor_info(w, h, v, [0; 4]),
            raw_extra: Vec::new(),
            raw_drop: Vec::new(),
            maker_extra: Vec::new(),
            maker_drop: Vec::new(),
            model: "Canon EOS 5D Mark III",
            orientation: 1,
            preview: None,
            thumbnail: None,
            stream: None,
            no_maker_note: false,
            after_raw: Vec::new(),
        }
    }

    /// Frame X and Y of the layout.
    pub(super) fn frame(&self) -> (usize, usize) {
        if self.layout_c {
            (2 * self.w / self.nf, self.h / 2)
        } else {
            (self.w / self.nf, self.h)
        }
    }

    /// The LJ92 stream of the raster: the sample sequence by the slice
    /// mapping, split into Nf component planes (R-CR2-22).
    pub(super) fn encode(&self) -> Vec<u8> {
        let (x, y) = self.frame();
        let mut planes = vec![vec![0u16; x * y]; self.nf];
        for s in 0..self.w * self.h {
            let (px, py) = slice_position(s, self.slices, self.w, self.h);
            planes[s % self.nf][s / self.nf] = self.raster[py * self.w + px];
        }
        let comps: Vec<Comp<'_>> = planes
            .iter()
            .map(|p| Comp {
                h: 1,
                v: 1,
                samples: p,
                width: x,
            })
            .collect();
        lj92(self.precision, x, y, &comps, self.restart, false)
    }

    /// The complete file.
    pub(super) fn build(&self) -> Vec<u8> {
        let stream = self.stream.clone().unwrap_or_else(|| self.encode());
        // IFD indices: 0 IFD0, 1 Exif, 2 MakerNote, 3 raw, 4 IFD1. Blob 0
        // is the raw strip, written last in the file as in real files (M1).
        let mut blobs = vec![stream];
        let mut ifd0 = vec![
            (0x010F, V::Ascii("Canon")),
            (0x0110, V::Ascii(self.model)),
            (0x0112, V::Short(vec![self.orientation])),
            (0x8769, V::Ifd(1)),
        ];
        if let Some(p) = &self.preview {
            blobs.push(p.clone());
            let k = blobs.len() - 1;
            ifd0.push((0x0103, V::Short(vec![6])));
            ifd0.push((0x0111, V::Blob(k)));
            ifd0.push((0x0117, V::BlobLen(k)));
        }
        let mut chain = vec![0];
        let exif = if self.no_maker_note {
            vec![(0x9000, V::Undef(b"0230".to_vec()))]
        } else {
            vec![(0x927C, V::Inner(2))]
        };
        let mut ifds = vec![ifd0, exif];
        let mut maker = vec![
            (0x0010, V::Long(vec![0x8000_0285])),
            (0x00E0, V::Short(self.sensor_info.clone())),
            (0x4001, color_data_63(10, [2429, 1024, 1024, 1783])),
        ];
        edit(&mut maker, &self.maker_extra, &self.maker_drop);
        ifds.push(maker);
        let mut raw = vec![
            (0x0103, V::Short(vec![6])),
            (0x0111, V::Blob(0)),
            (0x0117, V::BlobLen(0)),
            (0xC5D8, V::Long(vec![1])),
            (0xC5E0, V::Long(vec![1])),
            (0xC6C5, V::Long(vec![1])),
        ];
        if let Some(t) = &self.slice_tag {
            raw.push((0xC640, V::Short(t.clone())));
        }
        edit(&mut raw, &self.raw_extra, &self.raw_drop);
        ifds.push(raw);
        if let Some(t) = &self.thumbnail {
            blobs.push(t.clone());
            let k = blobs.len() - 1;
            ifds.push(vec![(0x0201, V::Blob(k)), (0x0202, V::BlobLen(k))]);
            chain.push(4);
        }
        // Real files chain the raw IFD last (M1), dual pixel files have
        // one more IFD after it (M11).
        chain.push(3);
        for extra in &self.after_raw {
            ifds.push(extra.clone());
            chain.push(ifds.len() - 1);
        }
        Builder {
            ifds,
            chain,
            raw: Some(3),
            blobs,
            strip_last: true,
        }
        .build()
    }
}

/// Replace or add `extra` entries and remove `drop` tags.
fn edit(entries: &mut Vec<(u16, V)>, extra: &[(u16, V)], drop: &[u16]) {
    entries.retain(|e| !drop.contains(&e.0) && !extra.iter().any(|x| x.0 == e.0));
    entries.extend(extra.iter().cloned());
}

/// The CFA image of the fixtures: value `rgb[c]` for the color c of each
/// raster position under CFA value `cfa` (R-CR2-19).
pub(super) fn flat_cfa(cfa: u32, rgb: [u16; 3]) -> impl Fn(usize, usize) -> u16 {
    let colors: [usize; 4] = match cfa {
        2 => [2, 1, 1, 0],
        3 => [1, 2, 0, 1],
        4 => [1, 0, 2, 1],
        _ => [0, 1, 1, 2],
    };
    move |x, y| rgb[colors[2 * (y & 1) + (x & 1)]]
}

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

/// F4 from its hex dump.
pub(super) fn f4() -> Vec<u8> {
    hex(F4_HEX)
}

/// Fixture F1 (layout A, 20 x 8, two slices), the complete 652-byte file
/// of the specification.
pub(super) const F1_HEX: &str = "
0000: 49 49 2A 00 10 00 00 00 43 52 02 00 48 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 16 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 62 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 35 44 20 4D 61 72 6B 20 49 49
0060: 49 00 01 00 7C 92 07 00 D4 00 00 00 74 00 00 00
0070: 00 00 00 00 03 00 10 00 04 00 01 00 00 00 85 02
0080: 00 80 E0 00 03 00 11 00 00 00 9E 00 00 00 01 40
0090: 03 00 44 00 00 00 C0 00 00 00 00 00 00 00 22 00
00A0: 14 00 08 00 01 00 01 00 08 00 02 00 13 00 07 00
00B0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00C0: 0A 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 7D 09
0140: 00 04 00 04 F7 06 50 14 05 00 03 01 03 00 01 00
0150: 00 00 06 00 00 00 11 01 04 00 01 00 00 00 90 01
0160: 00 00 17 01 04 00 01 00 00 00 FC 00 00 00 E0 C5
0170: 04 00 01 00 00 00 01 00 00 00 40 C6 03 00 03 00
0180: 00 00 8A 01 00 00 00 00 00 00 01 00 0A 00 0A 00
0190: FF D8 FF C4 00 42 00 00 01 04 02 03 01 01 01 01
01A0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01B0: 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01
01C0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01D0: 02 01 0C 0B 0D 0E FF C3 00 0E 0E 00 08 00 0A 02
01E0: 01 11 00 02 11 00 FF DA 00 0A 02 01 00 02 10 01
01F0: 00 00 FF 00 1F FF 00 FC 7F FE 73 9C E7 39 CE 73
0200: 9C E7 39 CE 73 9C E7 39 CE 73 FB E8 7E FA 0F E0
0210: 5F F8 17 F9 CE 73 9C FD F4 1D FA 39 CE 73 9C E7
0220: 3F BE 87 EF A0 FE 05 FF 00 81 7F 9C E7 39 CF DF
0230: 41 DF A3 9C E7 39 CE 73 FB E8 7E FA 0F E0 5F F8
0240: 17 F9 CE 73 9C FD F4 1D FA 39 CE 73 9C E7 39 CE
0250: 73 9C E7 39 CE 73 FB E8 7E FA 0E 73 9C E7 39 CF
0260: EF A1 F9 11 F9 CE 73 9C E7 39 CE 73 9C E7 39 CF
0270: EF A1 F9 11 F9 CE 73 9C E7 39 CE 73 9C E7 39 CF
0280: EF A1 F9 11 F9 CE 73 9C E7 3F FF D9
";

/// Fixture F2 (layout C, 16 x 8: each frame line of 32 samples holds raster
/// rows 2v and 2v + 1), the complete 697-byte file of the specification.
pub(super) const F2_HEX: &str = "
0000: 49 49 2A 00 10 00 00 00 43 52 02 00 48 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 15 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 62 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 36 44 20 4D 61 72 6B 20 49 49
0060: 00 00 01 00 7C 92 07 00 D4 00 00 00 74 00 00 00
0070: 00 00 00 00 03 00 10 00 04 00 01 00 00 00 06 04
0080: 00 80 E0 00 03 00 11 00 00 00 9E 00 00 00 01 40
0090: 03 00 44 00 00 00 C0 00 00 00 00 00 00 00 22 00
00A0: 10 00 08 00 01 00 01 00 08 00 02 00 0F 00 07 00
00B0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00C0: 0F 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 25 08
0140: 00 04 00 04 19 07 B4 12 05 00 03 01 03 00 01 00
0150: 00 00 06 00 00 00 11 01 04 00 01 00 00 00 90 01
0160: 00 00 17 01 04 00 01 00 00 00 29 01 00 00 E0 C5
0170: 04 00 01 00 00 00 01 00 00 00 40 C6 03 00 03 00
0180: 00 00 8A 01 00 00 00 00 00 00 00 00 10 00 10 00
0190: FF D8 FF C4 00 42 00 00 01 04 02 03 01 01 01 01
01A0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01B0: 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01
01C0: 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A
01D0: 02 01 0C 0B 0D 0E FF C3 00 14 0E 00 04 00 08 04
01E0: 01 11 00 02 11 00 03 11 00 04 11 00 FF DA 00 0E
01F0: 04 01 00 02 10 03 00 04 10 01 00 00 FF 00 07 FF
0200: 00 FC 1F FF 00 F0 7F FF 00 C1 FF 00 E7 39 CE 73
0210: 9C E7 39 CE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C
0220: E7 39 CF F5 5C 3F EE 26 3F D5 70 FF 00 B8 98 E7
0230: 39 CF F2 A3 FF 00 E1 D9 FF 00 CA 8F FF 00 87 67
0240: E7 39 CF FB 89 8F F6 56 3F EE 26 3F D9 58 E7 39
0250: CE 73 9C E7 39 CF F5 5C 3F EE 26 3F D5 70 FF 00
0260: B8 98 E7 39 CF F2 A3 FF 00 E1 D9 FF 00 CA 8F FF
0270: 00 87 67 E7 39 CF FB 89 8F F6 56 3F EE 26 3F D9
0280: 58 E7 39 CE 73 9C E7 39 CF F5 5C 3F EE 26 3F D5
0290: 70 FF 00 B8 98 E7 39 CF F2 A3 FF 00 E1 D9 FF 00
02A0: CA 8F FF 00 87 67 E7 39 CF FB 89 8F F6 56 3F EE
02B0: 26 3F D9 58 E7 39 CF FF D9
";

/// Fixture F3 (layout B, 20 x 6, three slices, CFA value 3, unknown model),
/// the complete 638-byte file of the specification.
pub(super) const F3_HEX: &str = "
0000: 49 49 2A 00 10 00 00 00 43 52 02 00 42 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 0F 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 01 00 00 00 69 87 04 00 01 00 00 00 5C 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 45 4F 53 20 54 45 53 54 00 00 01 00 7C 92
0060: 07 00 D4 00 00 00 6E 00 00 00 00 00 00 00 03 00
0070: 10 00 04 00 01 00 00 00 99 99 00 80 E0 00 03 00
0080: 11 00 00 00 98 00 00 00 01 40 03 00 44 00 00 00
0090: BA 00 00 00 00 00 00 00 22 00 14 00 06 00 01 00
00A0: 01 00 08 00 02 00 13 00 05 00 00 00 00 00 00 00
00B0: 00 00 00 00 00 00 00 00 00 00 63 00 00 00 00 00
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 7D 09 00 04 00 04 F7 06
0140: 50 14 05 00 03 01 03 00 01 00 00 00 06 00 00 00
0150: 11 01 04 00 01 00 00 00 8A 01 00 00 17 01 04 00
0160: 01 00 00 00 F4 00 00 00 E0 C5 04 00 01 00 00 00
0170: 03 00 00 00 40 C6 03 00 03 00 00 00 84 01 00 00
0180: 00 00 00 00 02 00 07 00 06 00 FF D8 FF C4 00 42
0190: 00 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00
01A0: 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
01B0: 01 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00
01C0: 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
01D0: FF C3 00 14 0C 00 06 00 05 04 01 11 00 02 11 00
01E0: 03 11 00 04 11 00 FF DA 00 0E 04 01 00 02 10 03
01F0: 00 04 10 01 00 00 FE 1F FF 00 C3 FF 00 F8 7F FF
0200: 00 0F FE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C E7
0210: 39 CE 73 9C E7 39 CE 73 9C E7 39 CE 73 9C E7 39
0220: CE 73 9C E7 F5 DC DF A7 F5 DC DF A7 F5 DC DF A7
0230: 37 E9 CD FA 7F 5D CE 73 A0 BF 34 17 CD FA 73 9F
0240: C8 8F 7E 9C DF A7 37 E9 FD 77 3A 0B FF 00 22 3E
0250: 82 FF 00 C8 8F 9C E7 39 CE 73 FA EE 6F D3 FA EE
0260: 6F D3 7E 9D 05 F9 CD 05 F7 E8 E7 37 E9 D0 5F 7E
0270: 9D 05 F9 CD 05 F7 E8 D0 5F 7E 8E 73 FF D9
";

/// Fixture F4 (layout A without slice tag, CFA value 4, ColorData type 7,
/// black rectangle right of the visible area, orientation 6), the complete
/// 571-byte file of the specification.
pub(super) const F4_HEX: &str = "
0000: 49 49 2A 00 10 00 00 00 43 52 02 00 56 01 00 00
0010: 04 00 0F 01 02 00 06 00 00 00 46 00 00 00 10 01
0020: 02 00 14 00 00 00 4C 00 00 00 12 01 03 00 01 00
0030: 00 00 06 00 00 00 69 87 04 00 01 00 00 00 60 00
0040: 00 00 00 00 00 00 43 61 6E 6F 6E 00 43 61 6E 6F
0050: 6E 20 50 6F 77 65 72 53 68 6F 74 20 47 31 32 00
0060: 01 00 7C 92 07 00 E4 00 00 00 72 00 00 00 00 00
0070: 00 00 03 00 10 00 04 00 01 00 00 00 00 00 92 02
0080: E0 00 03 00 11 00 00 00 9C 00 00 00 01 40 07 00
0090: 98 00 00 00 BE 00 00 00 00 00 00 00 22 00 0C 00
00A0: 06 00 01 00 01 00 02 00 02 00 09 00 05 00 0A 00
00B0: 02 00 0B 00 05 00 00 00 00 00 00 00 00 00 FD FF
00C0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00D0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00E0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00F0: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0100: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0110: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0120: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0130: 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0140: 00 00 00 00 00 00 00 00 00 00 00 00 CB 05 79 03
0150: 79 03 82 07 50 14 04 00 03 01 03 00 01 00 00 00
0160: 06 00 00 00 11 01 04 00 01 00 00 00 8C 01 00 00
0170: 17 01 04 00 01 00 00 00 AF 00 00 00 E0 C5 04 00
0180: 01 00 00 00 04 00 00 00 00 00 00 00 FF D8 FF C4
0190: 00 42 00 00 01 04 02 03 01 01 01 01 01 00 00 00
01A0: 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B
01B0: 0D 0E 01 00 01 04 02 03 01 01 01 01 01 00 00 00
01C0: 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B
01D0: 0D 0E FF C3 00 0E 0C 00 06 00 06 02 01 11 00 02
01E0: 11 00 FF DA 00 0A 02 01 00 02 10 01 00 00 FE 0F
01F0: FF 00 C1 FF 00 9C E7 39 CE 73 9C E7 39 CE 73 9C
0200: E7 39 CE 73 FA EE 77 E8 E7 39 CE 73 F9 11 F4 17
0210: E7 37 E9 FD 77 39 CE 73 9C D0 5F F2 23 E7 3F AE
0220: E7 7E 8E 73 9C E7 3F 91 1F 41 7E 73 7E 9F D7 73
0230: 9C E7 39 CD 05 FF 00 22 3F FF D9
";

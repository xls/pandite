//! The metadata of a CR3 file (sections Metadata tags and Levels and
//! white balance): the TIFF blocks CMT1 to CMT4, the CTMD records of the
//! metadata track, ColorData and SensorInfo.
//!
//! The TIFF blocks are read with pandite's shared TIFF reader
//! ([`crate::exif::Tiff`]) on the bytes of the block, so every offset
//! counts from the first byte of the block (R-CR3-22). The checks of this
//! document that the shared reader does not make are made here: the value
//! 42 in the header, field types 1 to 12, and IFD0 only.

use super::boxes::{range, Container, Role};
use crate::exif::{Entry, Tiff};

/// R-CR3-47: largest metadata sample read.
pub(crate) const MAX_CTMD: u64 = 16 << 20;
/// R-CR3-48: most records or blocks walked.
pub(crate) const MAX_WALK: usize = 65536;

/// A TIFF block with its IFD0 entries.
pub(crate) struct Block<'a> {
    /// The reader over the block bytes.
    pub tiff: Tiff<'a>,
    /// The IFD0 entries with a field type of 1 to 12 whose value bytes lie
    /// inside the block (R-CR3-23).
    pub entries: Vec<Entry>,
}

/// R-CR3-22 to R-CR3-24: read IFD0 of a TIFF block. `None` when the
/// header is not II or MM with 42, or IFD0 is not readable (including more
/// than 4096 entries, R-CR3-47).
pub(crate) fn block(bytes: &[u8]) -> Option<Block<'_>> {
    // R-CR3-22: the byte order of the header and the value 42.
    let tiff = Tiff::new(bytes)?;
    if tiff.u16(2)? != 42 {
        return None;
    }
    // R-CR3-24: IFD0 only; the next-IFD offset is never followed. The
    // shared reader refuses an IFD of more than 4096 entries (R-CR3-47)
    // and skips entries whose value bytes leave the block (R-CR3-23).
    let (all, _next) = tiff.ifd(tiff.first_ifd()?)?;
    // R-CR3-23: field types outside 1 to 12 are ignored.
    let entries = all
        .into_iter()
        .filter(|e| (1..=12).contains(&e.typ))
        .collect();
    Some(Block { tiff, entries })
}

impl Block<'_> {
    /// The entry of `tag`, in any entry order (R-CR3-24).
    pub(crate) fn entry(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|e| e.tag == tag)
    }

    /// Integer value `i` of `tag` (BYTE, SHORT, LONG and their signed
    /// kinds).
    pub(crate) fn uint(&self, tag: u16, i: u32) -> Option<u32> {
        self.tiff.value(self.entry(tag)?, i)
    }

    /// An ASCII value without its trailing zero bytes and spaces
    /// (R-CR3-24).
    pub(crate) fn ascii(&self, tag: u16) -> Option<String> {
        let e = self.entry(tag)?;
        if e.typ != 2 {
            return None;
        }
        let mut b = self.tiff.bytes(e)?;
        while let [rest @ .., 0 | b' '] = b {
            b = rest;
        }
        Some(String::from_utf8_lossy(b).into_owned())
    }
}

/// SensorInfo of CMT3 (R-CR3-34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SensorInfo {
    /// Values 1 and 2: the sensor width and height.
    pub size: (u32, u32),
    /// Values 5 to 8: left, top, right and bottom of the camera crop,
    /// inclusive.
    pub crop: (u32, u32, u32, u32),
}

impl SensorInfo {
    /// Width and height of the camera crop.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn crop_size(&self) -> (u32, u32) {
        let (l, t, r, b) = self.crop;
        ((r - l).saturating_add(1), (b - t).saturating_add(1))
    }
}

/// The values read from ColorData (R-CR3-31 to R-CR3-33).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ColorData {
    /// c[0], the ColorDataVersion.
    pub version: i16,
    /// The ColorData count.
    pub count: u32,
    /// WB_RGGBLevelsAsShot (signed), when known (R-CR3-31).
    pub wb_levels: Option<[i16; 4]>,
    /// The as-shot multipliers R, G, B (R-CR3-32).
    pub multipliers: Option<[f64; 3]>,
    /// ColorTempAsShot, read as the signed value of step 4.
    pub color_temp: Option<i16>,
    /// PerChannelBlackLevel, in stored order (R-CR3-33, OQ-8).
    pub black: Option<[u16; 4]>,
    /// NormalWhiteLevel (R-CR3-33, OQ-7).
    pub normal_white: Option<u16>,
    /// SpecularWhiteLevel (R-CR3-33, OQ-7).
    pub specular_white: Option<u16>,
    /// LinearityUpperMargin (OQ-7).
    pub linearity_upper_margin: Option<u16>,
}

/// The metadata the decoder reports.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Meta {
    /// CMT1 Make.
    pub make: Option<String>,
    /// CMT1 Model.
    pub model: Option<String>,
    /// CMT1 Orientation, 1 to 8 (R-CR3-38).
    pub orientation: u8,
    /// CMT2 ColorSpace, 1 when absent (R-CR3-35).
    pub color_space: u16,
    /// CMT3 SensorInfo (R-CR3-34).
    pub sensor: Option<SensorInfo>,
    /// ColorData of the metadata track (R-CR3-27).
    pub color: Option<ColorData>,
}

/// The TIFF block of a CMT box of the Canon metadata box.
pub(crate) fn cmt<'a>(c: &Container<'a>, t: &[u8; 4]) -> Option<Block<'a>> {
    let b = c.canon_box(t)?;
    block(b.payload(c.data))
}

/// R-CR3-38 and R-CR3-52: CMT1 Orientation, 1 when absent or outside 1
/// to 8.
pub(crate) fn orientation(c: &Container<'_>) -> u8 {
    cmt(c, b"CMT1")
        .and_then(|b| b.uint(0x0112, 0))
        .filter(|v| (1..=8).contains(v))
        .map_or(1, |v| v as u8)
}

/// R-CR3-35: CMT2 ColorSpace, 1 when the tag is absent.
pub(crate) fn color_space(c: &Container<'_>) -> u16 {
    cmt(c, b"CMT2")
        .and_then(|b| b.uint(0xA001, 0))
        .and_then(|v| u16::try_from(v).ok())
        .unwrap_or(1)
}

/// R-CR3-34 and R-CR3-52: SensorInfo when CMT3 holds tag 0x00E0 with at
/// least 9 values, value 7 >= value 5 and value 8 >= value 6.
pub(crate) fn sensor_info(c: &Container<'_>) -> Option<SensorInfo> {
    let b = cmt(c, b"CMT3")?;
    let e = b.entry(0x00E0)?;
    if e.count < 9 {
        return None;
    }
    let v: Vec<u32> = (0..9).map(|i| b.tiff.value(e, i)).collect::<Option<_>>()?;
    if v[7] < v[5] || v[8] < v[6] {
        return None;
    }
    Some(SensorInfo {
        size: (v[1], v[2]),
        crop: (v[5], v[6], v[7], v[8]),
    })
}

/// A little-endian u32 of the CTMD layout (M10).
fn le32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// R-CR3-25: the first sample of the metadata track, when it is at most
/// 16 MiB (R-CR3-47).
pub(crate) fn ctmd_sample<'a>(c: &Container<'a>) -> Option<&'a [u8]> {
    let t = c.tracks.iter().find(|t| t.role == Role::Metadata)?;
    let (off, size) = t.sample?;
    if size > MAX_CTMD {
        return None;
    }
    range(c.data, off, off.checked_add(size)?)
}

/// R-CR3-25: the records of a metadata sample as (type, payload), up to
/// the first record whose size is less than 12 or passes the end of the
/// sample, and at most 65536 records (R-CR3-48).
pub(crate) fn records(s: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < s.len() && out.len() < MAX_WALK {
        let Some(size) = le32(s, at) else {
            break;
        };
        let size = size as usize;
        let Some(end) = at.checked_add(size) else {
            break;
        };
        if size < 12 || end > s.len() {
            break;
        }
        let typ = u16::from_le_bytes([s[at + 4], s[at + 5]]);
        out.push((typ, &s[at + 12..end]));
        at = end;
    }
    out
}

/// R-CR3-26: the blocks of a record payload of type 7, 8 or 9 as (tag,
/// TIFF bytes), up to the first block whose size is less than 8 or passes
/// the end of the record, and at most 65536 blocks (R-CR3-48). Only the
/// record of type 8 holds a value this document uses (R-CR3-27).
pub(crate) fn blocks(r: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < r.len() && out.len() < MAX_WALK {
        let (Some(size), Some(tag)) = (le32(r, at), le32(r, at + 4)) else {
            break;
        };
        let size = size as usize;
        let Some(end) = at.checked_add(size) else {
            break;
        };
        if size < 8 || end > r.len() {
            break;
        }
        out.push((tag, &r[at + 8..end]));
        at = end;
    }
    out
}

/// The ColorData indices of one layout: WB_RGGBLevelsAsShot,
/// ColorTempAsShot, PerChannelBlackLevel, NormalWhiteLevel,
/// SpecularWhiteLevel and LinearityUpperMargin (section Location).
fn indices(version: i16) -> Option<[u32; 6]> {
    Some(match version {
        16..=19 => [71, 75, 329, 796, 797, 798],
        32 | 33 => [85, 89, 343, 810, 811, 812],
        34 | 48 => [105, 109, 363, 640, 641, 642],
        64..=67 => [105, 109, 383, 660, 661, 662],
        _ => return None,
    })
}

/// R-CR3-32: the multipliers from R, G1, G2, B; unknown when one is 0 or
/// negative (R-CR3-55: the divisor is checked).
pub(crate) fn multipliers(l: [i16; 4]) -> Option<[f64; 3]> {
    if l.iter().any(|&v| v <= 0) {
        return None;
    }
    let g = (f64::from(l[1]) + f64::from(l[2])) / 2.0;
    Some([f64::from(l[0]) / g, 1.0, f64::from(l[3]) / g])
}

/// Steps 3 to 5 of section Location: the values of a ColorData entry
/// (field type SHORT or SSHORT only, R-CR3-27).
pub(crate) fn color_data(b: &Block<'_>) -> Option<ColorData> {
    let e = b.entry(0x4001)?;
    if !matches!(e.typ, 3 | 8) || e.count == 0 {
        return None;
    }
    let count = e.count;
    // Step 4: c[i] as a 16-bit value in the byte order of the block; a
    // value outside the count is ignored (R-CR3-47).
    let raw = |i: u32| -> Option<u16> {
        if i >= count {
            return None;
        }
        b.tiff.u16(e.value_at.checked_add(2 * i as usize)?)
    };
    let signed = |i: u32| raw(i).map(|v| v as i16);
    let version = signed(0)?;
    let mut out = ColorData {
        version,
        count,
        wb_levels: None,
        multipliers: None,
        color_temp: None,
        black: None,
        normal_white: None,
        specular_white: None,
        linearity_upper_margin: None,
    };
    // OQ-12: a version outside the table leaves everything unknown.
    let Some([wb, temp, black, normal, specular, margin]) = indices(version) else {
        return Some(out);
    };
    // R-CR3-31: the 4 levels inside the count.
    out.wb_levels = (|| {
        Some([
            signed(wb)?,
            signed(wb + 1)?,
            signed(wb + 2)?,
            signed(wb + 3)?,
        ])
    })();
    out.multipliers = out.wb_levels.and_then(multipliers);
    out.color_temp = signed(temp);
    // R-CR3-33: unsigned, each only when its index is inside the count.
    out.black = (|| {
        Some([
            raw(black)?,
            raw(black + 1)?,
            raw(black + 2)?,
            raw(black + 3)?,
        ])
    })();
    out.normal_white = raw(normal);
    out.specular_white = raw(specular);
    out.linearity_upper_margin = raw(margin);
    Some(out)
}

/// R-CR3-27: ColorData from tag 0x4001 of the first block with the tag
/// 0x927C in the first record of type 8 of the metadata sample.
pub(crate) fn find_color_data(c: &Container<'_>) -> Option<ColorData> {
    let s = ctmd_sample(c)?;
    let (_, record) = records(s).into_iter().find(|(t, _)| *t == 8)?;
    let (_, tiff) = blocks(record).into_iter().find(|(t, _)| *t == 0x927C)?;
    color_data(&block(tiff)?)
}

/// Read every reported value (decoding procedure, steps 3 and 6).
pub(crate) fn read(c: &Container<'_>) -> Meta {
    let cmt1 = cmt(c, b"CMT1");
    Meta {
        make: cmt1.as_ref().and_then(|b| b.ascii(0x010F)),
        model: cmt1.as_ref().and_then(|b| b.ascii(0x0110)),
        orientation: orientation(c),
        color_space: color_space(c),
        sensor: sensor_info(c),
        color: find_color_data(c),
    }
}

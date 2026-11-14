//! Classification of an SRW file (R-SRW-3) and the decode plan: sizes,
//! strip layout, valid area, raw data byte order, levels, white balance,
//! color matrix and orientation, all validated before any large
//! allocation.

use super::color;
use super::ifd::{self, Dir};
use super::{fail, SrwResult};
use crate::raw::arw::Rect;

/// Tag numbers of the specification's Metadata tags tables.
pub(super) mod tag {
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const BITS_PER_SAMPLE: u16 = 0x0102;
    pub const COMPRESSION: u16 = 0x0103;
    pub const MAKE: u16 = 0x010F;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const ROWS_PER_STRIP: u16 = 0x0116;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const SUB_IFDS: u16 = 0x014A;
    pub const JPEG_OFFSET: u16 = 0x0201;
    pub const JPEG_LENGTH: u16 = 0x0202;
    pub const CFA_REPEAT_PATTERN_DIM: u16 = 0x828D;
    pub const CFA_PATTERN: u16 = 0x828E;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const MAKER_NOTE: u16 = 0x927C;
    // Maker-note tags.
    pub const RAW_DATA_BYTE_ORDER: u16 = 0x0040;
    pub const SENSOR_AREAS: u16 = 0xA010;
    pub const ENCRYPTION_KEY: u16 = 0xA020;
    pub const WB_LEVELS_UNCORRECTED: u16 = 0xA021;
    pub const WB_LEVELS_BLACK: u16 = 0xA028;
    pub const COLOR_MATRIX: u16 = 0xA030;
    pub const COLOR_MATRIX_SRGB: u16 = 0xA031;
    pub const WHITE_LEVELS: u16 = 0xA048;
}

/// The storage variants this decoder develops from sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Compression 32769: 16-bit samples in the raw data byte order.
    V1,
    /// Compression 32770: 12-bit packed, two pixels in each 24-bit group.
    V2,
}

/// The out-of-scope classes that use the preview path (R-SRW-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// Compression 32770 with another strip size (Samsung compressed).
    X1,
    /// Compression 32772.
    X2,
    /// Compression 32773.
    X3,
    /// Any other combination.
    X4,
    /// No raw IFD.
    X5,
}

/// The result of classification (R-SRW-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A class that uses the preview path.
    OutOfScope(Relative),
}

/// Where the white balance multipliers came from (R-SRW-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WbSource {
    /// Decrypted maker-note tag 0xA021.
    AsShot,
    /// The fallback multipliers of R-SRW-33.
    Fallback,
}

/// Where the color matrix came from (R-SRW-35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MatrixSource {
    /// Decrypted maker-note tag 0xA031 (ColorMatrixSRGB).
    Srgb,
    /// Decrypted maker-note tag 0xA030 (ColorMatrix).
    Camera,
    /// The table row of the Model.
    Table,
    /// The generic table row.
    Generic,
}

/// Most SubIFD offsets examined in one IFD. The specification sets no
/// bound; real files have two, and each candidate costs an IFD parse.
pub(super) const MAX_SUB_IFDS: usize = 256;

/// The parsed container: the IFD chain, the raw IFD and the maker note.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// True when the TIFF header is little-endian.
    pub le: bool,
    /// IFD0, IFD1 and later IFDs of the main chain (R-SRW-10). Empty when
    /// IFD0 is invalid.
    pub chain: Vec<Dir<'a>>,
    /// The raw IFD, when one qualifies (R-SRW-12).
    pub raw: Option<Dir<'a>>,
    /// The maker-note IFD, when there is a valid one (R-SRW-13).
    pub maker: Option<Dir<'a>>,
}

/// Parse the header, the IFD chain, the raw IFD and the maker note. An
/// error means "not an SRW file": no preview (R-SRW-4).
pub(super) fn open(data: &[u8]) -> SrwResult<Container<'_>> {
    let (le, first) = ifd::header(data)?;
    let chain = ifd::chain(data, le, first);
    let raw = chain.first().and_then(|ifd0| raw_ifd(data, le, ifd0));
    let maker = chain.first().and_then(|ifd0| maker_note(data, le, ifd0));
    Ok(Container {
        data,
        le,
        chain,
        raw,
        maker,
    })
}

/// The valid SubIFDs of an IFD, in the order of the offsets of tag 0x014A.
pub(super) fn sub_ifds<'a>(data: &'a [u8], le: bool, d: &Dir<'a>) -> Vec<Dir<'a>> {
    d.uints(tag::SUB_IFDS)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_SUB_IFDS)
        .filter_map(|off| ifd::dir(data, le, u64::from(off), 0))
        .collect()
}

/// R-SRW-12: the first SubIFD of IFD0, in the order of the offsets, that is
/// a valid IFD and holds the tags 0x0100, 0x0101, 0x0103, 0x0111 and
/// 0x0117.
fn raw_ifd<'a>(data: &'a [u8], le: bool, ifd0: &Dir<'a>) -> Option<Dir<'a>> {
    const NEEDED: [u16; 5] = [
        tag::IMAGE_WIDTH,
        tag::IMAGE_LENGTH,
        tag::COMPRESSION,
        tag::STRIP_OFFSETS,
        tag::STRIP_BYTE_COUNTS,
    ];
    sub_ifds(data, le, ifd0)
        .into_iter()
        .find(|d| NEEDED.iter().all(|&t| d.has(t)))
}

/// R-SRW-13: the value of tag 0x927C of the Exif IFD (IFD0 tag 0x8769),
/// read as an IFD at P. `None` when the Exif IFD or the maker-note IFD is
/// invalid (R-SRW-11, R-SRW-59), or when the MakerNote value does not lie
/// inside the file (R-SRW-53).
fn maker_note<'a>(data: &'a [u8], le: bool, ifd0: &Dir<'a>) -> Option<Dir<'a>> {
    let exif_at = ifd0.uint(tag::EXIF_IFD).ok().flatten()?;
    let exif = ifd::dir(data, le, u64::from(exif_at), 0)?;
    let e = exif.entry(tag::MAKER_NOTE)?;
    exif.bytes(e).ok().flatten()?;
    // P is the file offset of the first byte of the MakerNote value.
    ifd::maker_dir(data, le, e.at)
}

impl<'a> Container<'a> {
    /// IFD0, when valid.
    pub(super) fn ifd0(&self) -> Option<&Dir<'a>> {
        self.chain.first()
    }

    /// The IFD0 Orientation: 1 when absent or outside 1 to 8 (R-SRW-42).
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().map(|d| d.uint(tag::ORIENTATION)) {
            Some(Ok(Some(v @ 1..=8))) => v as u8,
            _ => 1,
        }
    }
}

/// Classify the file by the rules of R-SRW-3, in this order.
pub(super) fn classify(c: &Container<'_>) -> SrwResult<Class> {
    use Class::{OutOfScope, Sensor};
    // Rule 1: no raw IFD (R-SRW-12).
    let Some(raw) = &c.raw else {
        return Ok(OutOfScope(Relative::X5));
    };
    let compression = raw.uint(tag::COMPRESSION)?;
    // BitsPerSample is the first value of tag 0x0102.
    let bits = raw.uint(tag::BITS_PER_SAMPLE)?;
    let size = match (raw.uint(tag::IMAGE_WIDTH)?, raw.uint(tag::IMAGE_LENGTH)?) {
        (Some(w), Some(h)) => Some((u64::from(w), u64::from(h))),
        _ => None,
    };
    // "Total strip bytes": the sum of all StripByteCounts values (64-bit,
    // R-SRW-50).
    let total: u64 = raw
        .uints(tag::STRIP_BYTE_COUNTS)?
        .unwrap_or_default()
        .iter()
        .map(|&v| u64::from(v))
        .sum();
    if let Some((w, h)) = size {
        // Rule 2.
        if compression == Some(32769)
            && matches!(bits, None | Some(12) | Some(14))
            && total >= 2 * w * h
        {
            return Ok(Sensor(Variant::V1));
        }
        // Rule 3.
        if compression == Some(32770)
            && matches!(bits, None | Some(12))
            && w % 2 == 0
            && total == 3 * w * h / 2
        {
            return Ok(Sensor(Variant::V2));
        }
    }
    Ok(match compression {
        // Rule 4.
        Some(32770) => OutOfScope(Relative::X1),
        // Rule 5.
        Some(32772) => OutOfScope(Relative::X2),
        // Rule 6.
        Some(32773) => OutOfScope(Relative::X3),
        // Rule 7.
        _ => OutOfScope(Relative::X4),
    })
}

/// Everything needed to develop the sensor data.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    /// The storage variant.
    pub variant: Variant,
    /// Stored raster width W.
    pub width: usize,
    /// Stored raster height H.
    pub height: usize,
    /// The valid area in stored raster coordinates (R-SRW-22).
    pub valid: Rect,
    /// IFD0 orientation 1 to 8 (R-SRW-42).
    pub orientation: u8,
    /// Colors of CFA positions 0 to 3, origin at stored pixel (0, 0)
    /// (R-SRW-17, R-SRW-23).
    pub cfa: [u8; 4],
    /// True when the raw data byte order is little-endian (R-SRW-25).
    pub data_le: bool,
    /// White level w (R-SRW-31).
    pub white: u32,
    /// Black levels bR, bG, bB (R-SRW-32).
    pub black: [f64; 3],
    /// WB multipliers mR, mG, mB (R-SRW-33).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// The camera-to-sRGB matrix, each row divided by its sum (R-SRW-36).
    pub matrix: [[f64; 3]; 3],
    /// Where the matrix came from (R-SRW-35).
    pub matrix_source: MatrixSource,
    /// File offset of each strip (R-SRW-24).
    pub strips: Vec<usize>,
    /// Rows in each strip except possibly the last (R-SRW-18).
    pub rows_per_strip: usize,
    /// Bytes per row: R = 2 * W (V1) or 3 * W / 2 (V2).
    pub row_bytes: usize,
    /// The IFD0 Model string.
    pub model: String,
}

/// Largest W * H accepted (R-SRW-51).
const MAX_PIXELS: u64 = 1 << 27;
/// The fallback multipliers R, G, B of R-SRW-33.
pub(super) const FALLBACK_WB: [f64; 3] = [2.1137, 1.0, 1.5737];

fn require<T>(v: Option<T>, what: &str) -> SrwResult<T> {
    // R-SRW-16: a required tag absent or with count 0 uses the preview path.
    v.ok_or_else(|| fail(format!("required tag {what} missing")))
}

/// The real values of an encrypted maker-note tag (decryption rule):
/// V(i) = S(i) + s * K((k0 + i) mod 11), in 64-bit signed arithmetic
/// (R-SRW-20). `Ok(None)` when the tag or a valid key is absent (R-SRW-19).
pub(super) fn decrypt(
    maker: &Dir<'_>,
    key: Option<&[i64; 11]>,
    tag: u16,
    sign: i64,
    k0: usize,
) -> SrwResult<Option<Vec<i64>>> {
    let Some(key) = key else {
        return Ok(None);
    };
    let Some(stored) = maker.signed(tag)? else {
        return Ok(None);
    };
    Ok(Some(
        stored
            .iter()
            .enumerate()
            .map(|(i, &s)| s + sign * key[(k0 + i) % 11])
            .collect(),
    ))
}

/// R-SRW-19: the key, tag 0xA020, when it has exactly 11 values.
pub(super) fn key(maker: &Dir<'_>) -> SrwResult<Option<[i64; 11]>> {
    Ok(maker
        .longs(tag::ENCRYPTION_KEY)?
        .and_then(|v| <[i64; 11]>::try_from(v).ok()))
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check.
pub(super) fn plan(c: &Container<'_>, variant: Variant) -> SrwResult<Plan> {
    let data_len = c.data.len() as u64;
    // R-SRW-50: a file longer than 2^32 - 1 bytes fails.
    if data_len > u64::from(u32::MAX) {
        return Err(fail("file longer than 2^32 - 1 bytes"));
    }
    let raw = c.raw.as_ref().ok_or_else(|| fail("no raw IFD"))?;
    let ifd0 = c.ifd0().ok_or_else(|| fail("no IFD0"))?;
    // R-SRW-16: the required tags of the raw IFD and IFD0. R-SRW-15: tags
    // 0x00FE, 0x0115 and 0x0106 are never read.
    let w = require(raw.uint(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(raw.uint(tag::IMAGE_LENGTH)?, "ImageLength")?;
    require(raw.uint(tag::COMPRESSION)?, "Compression")?;
    require(ifd0.uint(tag::SUB_IFDS)?, "SubIFDs")?;
    require(ifd0.uint(tag::EXIF_IFD)?, "Exif IFD pointer")?;
    // R-SRW-51: limits on W, H and W * H.
    if w == 0 || h == 0 || w > 65535 || h > 65535 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    let (wu, hu) = (w as usize, h as usize);
    let bits = raw.uint(tag::BITS_PER_SAMPLE)?;
    let model_bytes = ifd0.ascii(tag::MODEL)?.unwrap_or_default().to_vec();
    let model = String::from_utf8_lossy(&model_bytes).into_owned();
    // R-SRW-42: Orientation 1 when absent or outside 1 to 8.
    let orientation = match ifd0.uint(tag::ORIENTATION)? {
        Some(v @ 1..=8) => v as u8,
        _ => 1,
    };

    // R-SRW-17: a 2 x 2 repeat (or no dimension tag) and one 0, two 1 and
    // one 2; otherwise the preview path. R-SRW-21: the pattern comes from
    // tag 0x828E alone; maker-note tag 0x0050 is never read.
    let dim = raw.uints(tag::CFA_REPEAT_PATTERN_DIM)?;
    if raw.has(tag::CFA_REPEAT_PATTERN_DIM) && dim.as_deref() != Some(&[2, 2]) {
        return Err(fail("CFA repeat pattern is not 2 x 2"));
    }
    let mut cfa = [0u8, 1, 1, 2];
    if raw.has(tag::CFA_PATTERN) {
        let p = raw
            .byte_values(tag::CFA_PATTERN)?
            .ok_or_else(|| fail("CFAPattern is not BYTE or UNDEFINED"))?;
        let count = |c: u8| p.iter().filter(|&&x| x == c).count();
        if p.len() != 4 || count(0) != 1 || count(1) != 2 || count(2) != 1 {
            return Err(fail("CFA pattern is not a Bayer pattern"));
        }
        cfa.copy_from_slice(p);
    }

    let (strips, rows_per_strip, row_bytes) = strips(raw, variant, wu, hu, data_len)?;

    // R-SRW-16: the maker note and its tag 0xA010 are required.
    let maker = c.maker.as_ref().ok_or_else(|| fail("no maker note"))?;
    // R-SRW-22: the valid area from elements 4 to 7 of tag 0xA010 (not
    // encrypted). The table gives 8 values; another count is treated as
    // absent.
    let areas = maker
        .longs(tag::SENSOR_AREAS)?
        .filter(|v| v.len() == 8)
        .ok_or_else(|| fail("maker-note tag 0xA010 missing"))?;
    let (vx0, vy0, vx1, vy1) = (areas[4], areas[5], areas[6], areas[7]);
    let (wi, hi) = (i64::from(w), i64::from(h));
    if !(0 <= vx0 && vx0 + 2 <= vx1 && vx1 <= wi && 0 <= vy0 && vy0 + 2 <= vy1 && vy1 <= hi) {
        return Err(fail("valid area outside the raster"));
    }
    let valid = Rect {
        x: vx0 as usize,
        y: vy0 as usize,
        w: (vx1 - vx0) as usize,
        h: (vy1 - vy0) as usize,
    };

    // R-SRW-25: the raw data byte order from maker-note tag 0x0040.
    let data_le = match maker.longs(tag::RAW_DATA_BYTE_ORDER)? {
        None => c.le,
        Some(v) => match v.first() {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(fail("raw data byte order is neither 0 nor 1")),
        },
    };

    // R-SRW-19, R-SRW-20: the real values of the encrypted tags, with the
    // (s, k0) of the table.
    let key = key(maker)?;
    let key = key.as_ref();
    let wb_levels = decrypt(maker, key, tag::WB_LEVELS_UNCORRECTED, -1, 0)?;
    let black_levels = decrypt(maker, key, tag::WB_LEVELS_BLACK, -1, 0)?;
    let matrix_camera = decrypt(maker, key, tag::COLOR_MATRIX, 1, 0)?;
    let matrix_srgb = decrypt(maker, key, tag::COLOR_MATRIX_SRGB, 1, 0)?;
    let white_levels = decrypt(maker, key, tag::WHITE_LEVELS, 1, 0)?;

    let white = white_level(&model_bytes, white_levels.as_deref(), variant, bits);
    let black = black_level(black_levels.as_deref(), white);
    let (wb, wb_source) = multipliers(wb_levels.as_deref(), black);
    let (matrix, matrix_source) = matrix(
        matrix_srgb.as_deref(),
        matrix_camera.as_deref(),
        &model_bytes,
    );

    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        valid,
        orientation,
        cfa,
        data_le,
        white,
        black,
        wb,
        wb_source,
        matrix,
        matrix_source,
        strips,
        rows_per_strip,
        row_bytes,
        model,
    })
}

/// R-SRW-31: the white level from the first rule that applies.
pub(super) fn white_level(
    model: &[u8],
    levels: Option<&[i64]>,
    variant: Variant,
    bits: Option<u32>,
) -> u32 {
    // Rule 1: the model table.
    if let Some(w) = color::table_white(model) {
        return w;
    }
    // Rule 2: element 4 of decrypted tag 0xA048 with 12 elements, from 256
    // to 65535.
    if let Some(v) = levels.filter(|v| v.len() == 12).map(|v| v[4]) {
        if (256..=65535).contains(&v) {
            return v as u32;
        }
    }
    // Rule 3.
    match variant {
        Variant::V2 => 4095,
        Variant::V1 if bits == Some(12) => 4095,
        Variant::V1 => 16383,
    }
}

/// R-SRW-32: bR, bG = (V(1) + V(2)) / 2 and bB from decrypted tag 0xA028
/// with 4 elements, each from 0 to w - 256; else 0 for every color.
pub(super) fn black_level(levels: Option<&[i64]>, white: u32) -> [f64; 3] {
    let limit = i64::from(white) - 256;
    match levels {
        Some(v) if v.len() == 4 && v.iter().all(|&b| (0..=limit).contains(&b)) => {
            [v[0] as f64, (v[1] + v[2]) as f64 / 2.0, v[3] as f64]
        }
        _ => [0.0; 3],
    }
}

/// R-SRW-33: mR = (R - bR) / (G - bG), mG = 1, mB = (B - bB) / (G - bG)
/// with G = (G1 + G2) / 2 from decrypted tag 0xA021, or the fallback when
/// the tag is absent, a difference is not greater than 0 (R-SRW-56) or a
/// multiplier lies outside 0.1 to 10. The table gives the tag 4 values;
/// another count is treated as absent.
pub(super) fn multipliers(levels: Option<&[i64]>, black: [f64; 3]) -> ([f64; 3], WbSource) {
    let as_shot = levels.filter(|v| v.len() == 4).and_then(|v| {
        let g = (v[1] + v[2]) as f64 / 2.0;
        let (dr, dg, db) = (v[0] as f64 - black[0], g - black[1], v[3] as f64 - black[2]);
        if dg <= 0.0 || dr <= 0.0 || db <= 0.0 {
            return None;
        }
        let (mr, mb) = (dr / dg, db / dg);
        ((0.1..=10.0).contains(&mr) && (0.1..=10.0).contains(&mb)).then_some([mr, 1.0, mb])
    });
    match as_shot {
        Some(m) => (m, WbSource::AsShot),
        None => (FALLBACK_WB, WbSource::Fallback),
    }
}

/// R-SRW-36: a tag matrix is valid with 9 elements whose rows each sum to
/// 128 to 512.
fn tag_matrix(v: Option<&[i64]>) -> Option<[f64; 9]> {
    let v = v.filter(|v| v.len() == 9)?;
    if !v
        .chunks_exact(3)
        .all(|r| (128..=512).contains(&r.iter().sum::<i64>()))
    {
        return None;
    }
    let mut m = [0.0; 9];
    for (o, &x) in m.iter_mut().zip(v) {
        *o = x as f64;
    }
    Some(m)
}

/// R-SRW-35: the first valid source of decrypted 0xA031, decrypted 0xA030,
/// the table row of the Model and the generic row. R-SRW-36: each element
/// is divided by the sum of its row.
pub(super) fn matrix(
    srgb: Option<&[i64]>,
    camera: Option<&[i64]>,
    model: &[u8],
) -> ([[f64; 3]; 3], MatrixSource) {
    let (m, source) = if let Some(m) = tag_matrix(srgb) {
        (m, MatrixSource::Srgb)
    } else if let Some(m) = tag_matrix(camera) {
        (m, MatrixSource::Camera)
    } else if let Some(m) = color::table_matrix(model) {
        (m, MatrixSource::Table)
    } else {
        (color::GENERIC, MatrixSource::Generic)
    };
    let mut out = [[0.0; 3]; 3];
    for (r, o) in out.iter_mut().enumerate() {
        let row = &m[3 * r..3 * r + 3];
        // R-SRW-56: every row sum is from 128 to 512 (tags) or 256 (table),
        // so the divisor is greater than 0.
        let sum: f64 = row.iter().sum();
        for (c, v) in o.iter_mut().enumerate() {
            *v = row[c] / sum;
        }
    }
    (out, source)
}

/// Strip layout (R-SRW-18, R-SRW-24, R-SRW-53, R-SRW-54, R-SRW-55):
/// strip offsets, rows per strip and the row size R.
fn strips(
    raw: &Dir<'_>,
    variant: Variant,
    w: usize,
    h: usize,
    data_len: u64,
) -> SrwResult<(Vec<usize>, usize, usize)> {
    let offsets = require(raw.uints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(raw.uints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    // R-SRW-24: the row size R of the variant (W is even for V2, R-SRW-3).
    let row_bytes = match variant {
        Variant::V1 => 2 * w,
        Variant::V2 => 3 * w / 2,
    };
    // R-SRW-18: RowsPerStrip is H when absent, 0 or greater than H.
    let rps = match raw.uint(tag::ROWS_PER_STRIP)? {
        Some(r) if r > 0 && (r as usize) <= h => r as usize,
        _ => h,
    };
    // R-SRW-54 case 2: equal counts.
    if offsets.len() != counts.len() {
        return Err(fail("StripOffsets and StripByteCounts differ in count"));
    }
    // R-SRW-54 case 3: ceil(H / RowsPerStrip) strips.
    if offsets.len() != h.div_ceil(rps) {
        return Err(fail("strip count does not match RowsPerStrip"));
    }
    // R-SRW-54 case 1: total bytes at least R * H (64-bit, R-SRW-50).
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total < row_bytes as u64 * h as u64 {
        return Err(fail("strips hold fewer bytes than the raster"));
    }
    let mut starts = Vec::with_capacity(offsets.len());
    for (s, (&off, &count)) in offsets.iter().zip(&counts).enumerate() {
        let rows = rps.min(h - s * rps);
        // R-SRW-54 case 4: each strip holds R times its rows.
        if u64::from(count) < row_bytes as u64 * rows as u64 {
            return Err(fail("a strip is shorter than its rows"));
        }
        // R-SRW-53: the strip byte range lies inside the file.
        if u64::from(off) + u64::from(count) > data_len {
            return Err(fail("a strip lies outside the file"));
        }
        starts.push(off as usize);
    }
    // R-SRW-55: the decoder reads exactly R bytes of each of at most H
    // rows, so never more than 2 * W * H (V1) or 3 * W * H / 2 (V2) bytes,
    // also when StripByteCounts is larger.
    Ok((starts, rps, row_bytes))
}

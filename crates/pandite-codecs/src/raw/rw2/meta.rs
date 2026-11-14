//! The container of an RW2 or Panasonic RAW file (header and IFD0), the
//! classification (R-RW2-3) and the decode plan: sizes, active area, CFA
//! colors, levels, white balance, color matrix, orientation and the data
//! range, all validated before any large allocation.

use super::color;
use super::{fail, Rw2Result};
use crate::raw::arw::ifd::{self, Dir};

/// Tag numbers of IFD0 (section Metadata tags).
pub(super) mod tag {
    pub const SENSOR_WIDTH: u16 = 0x0002;
    pub const SENSOR_HEIGHT: u16 = 0x0003;
    pub const TOP_BORDER: u16 = 0x0004;
    pub const LEFT_BORDER: u16 = 0x0005;
    pub const BOTTOM_BORDER: u16 = 0x0006;
    pub const RIGHT_BORDER: u16 = 0x0007;
    pub const CFA_PATTERN: u16 = 0x0009;
    pub const BITS_PER_SAMPLE: u16 = 0x000A;
    pub const COMPRESSION: u16 = 0x000B;
    /// LinearityLimitRed, Green, Blue.
    pub const LINEARITY_LIMIT: [u16; 3] = [0x000E, 0x000F, 0x0010];
    /// BlackLevelRed, Green, Blue.
    pub const BLACK_LEVEL: [u16; 3] = [0x001C, 0x001D, 0x001E];
    /// WBRedLevel, WBGreenLevel, WBBlueLevel.
    pub const WB_LEVEL: [u16; 3] = [0x0024, 0x0025, 0x0026];
    pub const RAW_FORMAT: u16 = 0x002D;
    pub const JPG_FROM_RAW: u16 = 0x002E;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const RAW_DATA_OFFSET: u16 = 0x0118;
    pub const JPG_FROM_RAW2: u16 = 0x0127;
}

/// Header bytes 0 to 3 of every accepted file (R-RW2-5).
pub(super) const MAGIC: [u8; 4] = [0x49, 0x49, 0x55, 0x00];

/// The storage variants developed from sensor data (section Scope and
/// variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// RawFormat 4, 12 bits: block code with 8-bit codes, chunk rotation.
    V1,
    /// RawFormat 5, 12 bits: packed, chunk rotation.
    V2,
    /// RawFormat 5, 14 bits: packed, chunk rotation.
    V3,
    /// RawFormat 6, 12 bits: block code with 8-bit codes.
    V4,
    /// RawFormat 6, 14 bits: block code with 10-bit codes.
    V5,
    /// RawFormat 7, 14 bits: packed.
    V6,
    /// RawFormat 2, 12 bits (.RAW): packed.
    V7,
    /// No RawFormat, Compression 34828 (.RAW): 16-bit containers.
    V8,
}

/// How the 16-byte blocks of a variant hold their pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Coding {
    /// Two initial values and zones of coded differences (V1, V4, V5).
    BlockCode,
    /// Plain D-bit values (V2, V3, V6, V7).
    Packed,
    /// One 16-bit container per pixel (V8).
    Containers,
}

impl Variant {
    /// D, the sample bit depth.
    pub(super) fn depth(self) -> u32 {
        match self {
            Variant::V3 | Variant::V5 | Variant::V6 => 14,
            _ => 12,
        }
    }

    /// F = 2^D - 1, the full-scale value (R-RW2-30).
    pub(super) fn full_scale(self) -> u32 {
        (1 << self.depth()) - 1
    }

    /// p, the pixels per 16-byte block (1 for the 2-byte containers of V8).
    pub(super) fn per_block(self) -> usize {
        match self {
            Variant::V1 | Variant::V4 => 14,
            Variant::V2 | Variant::V7 => 10,
            Variant::V3 | Variant::V6 => 9,
            Variant::V5 => 11,
            Variant::V8 => 1,
        }
    }

    /// True for the variants stored with the chunk rotation (V1, V2, V3).
    pub(super) fn rotated(self) -> bool {
        matches!(self, Variant::V1 | Variant::V2 | Variant::V3)
    }

    /// The coding of the sensor data.
    pub(super) fn coding(self) -> Coding {
        match self {
            Variant::V1 | Variant::V4 | Variant::V5 => Coding::BlockCode,
            Variant::V8 => Coding::Containers,
            _ => Coding::Packed,
        }
    }

    /// True for the variants of the older .RAW files, whose data start is
    /// StripOffsets and whose black level has no offset.
    pub(super) fn legacy(self) -> bool {
        matches!(self, Variant::V7 | Variant::V8)
    }
}

/// The out-of-scope relatives that use the preview path (R-RW2-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// RawFormat 8: variable-length coding (OQ-1).
    X1,
    /// Another RawFormat value, or a pair (RawFormat, BitsPerSample) that
    /// is not in the variant table.
    X2,
    /// No RawFormat and the rule of V8 does not match.
    X3,
    /// Big-endian files or another magic number.
    X4,
    /// A raster width that is not a multiple of p.
    X5,
}

/// The result of classification (R-RW2-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A relative that uses the preview path.
    OutOfScope(Relative),
}

/// The parsed container: the file and its IFD0.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// IFD0, without entries of a type outside 1 to 12 (R-RW2-10).
    pub ifd0: Dir<'a>,
    /// The file offset just after IFD0, after its next-IFD offset field.
    pub ifd0_end: u64,
}

/// Parse the header and IFD0. `Ok(None)` for a file whose bytes 0 to 3 are
/// not 49 49 55 00 (X4): the caller then uses its generic preview search.
pub(super) fn open(data: &[u8]) -> Rw2Result<Option<Container<'_>>> {
    // R-RW2-52: shorter than 8 bytes or longer than 2^32 - 1 bytes fails.
    if data.len() < 8 || data.len() as u64 > u64::from(u32::MAX) {
        return Err(fail("file size outside 8 bytes to 4 GiB"));
    }
    // R-RW2-5: only 49 49 55 00 is accepted (little-endian, magic 85).
    if data[..4] != MAGIC {
        return Ok(None);
    }
    // R-RW2-7: IFD0 at the offset in bytes 4 to 7 (R-RW2-6: little-endian);
    // bytes 8 to 23 are not read.
    let first = u64::from(u32::from_le_bytes([data[4], data[5], data[6], data[7]]));
    // R-RW2-11: an offset outside the file, N of 0 or above 1024, or an IFD
    // that does not lie completely inside the file is invalid; there is no
    // preview without IFD0, so this is an error.
    let Some(mut ifd0) = ifd::dir(data, first) else {
        return Err(fail("IFD0 is not a valid IFD"));
    };
    // R-RW2-10: entries with a type outside 1 to 12 are skipped. The IFD
    // reader already skips types outside 1 to 13; type 13 goes here.
    ifd0.entries.retain(|e| (1..=12).contains(&e.typ));
    // R-RW2-8 is applied by the IFD reader: values of count * size <= 4 bytes
    // are read from the value field, others from the offset it holds.
    let n = u64::from(u16::from_le_bytes([
        data[first as usize],
        data[first as usize + 1],
    ]));
    // The 2 + 12 * N + 4 bytes of IFD0 end here (R-RW2-11, R-RW2-57).
    let ifd0_end = first + 2 + 12 * n + 4;
    Ok(Some(Container {
        data,
        ifd0,
        ifd0_end,
    }))
}

impl Container<'_> {
    /// The first value of an integer tag (R-RW2-9: found by tag number;
    /// R-RW2-10: SHORT or LONG, first value). `None` when the tag is absent,
    /// has a count of 0, has another type, or its values lie outside the
    /// file (invalid). R-RW2-54: the IFD reader refuses a count * type size
    /// beyond the file before it allocates the values.
    pub(super) fn uint(&self, tag: u16) -> Option<u32> {
        self.ifd0.uint(tag).ok().flatten()
    }

    /// A required integer tag: absent, count 0 or unreadable fails
    /// (R-RW2-14, preview path).
    fn required(&self, tag: u16, name: &str) -> Rw2Result<u32> {
        self.ifd0
            .uint(tag)?
            .ok_or_else(|| fail(format!("required tag {name} missing")))
    }

    /// The IFD0 Model string with trailing zero bytes and spaces removed
    /// (R-RW2-12); empty when absent.
    pub(super) fn model(&self) -> Vec<u8> {
        self.ifd0
            .ascii(tag::MODEL)
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_vec()
    }

    /// The IFD0 Orientation: 1 when absent or outside 1 to 8 (R-RW2-43).
    pub(super) fn orientation(&self) -> u8 {
        match self.uint(tag::ORIENTATION) {
            Some(v @ 1..=8) => v as u8,
            _ => 1,
        }
    }
}

/// Classify the file by the rules of R-RW2-3, in this order. A file that
/// matches no rule is X2 (RawFormat present) or X3 (RawFormat absent).
/// R-RW2-58: the variant comes from the pair (RawFormat, BitsPerSample),
/// never from the file size alone.
pub(super) fn classify(c: &Container<'_>) -> Class {
    use Class::{OutOfScope, Sensor};
    // Rule 1: bytes 0 to 3 are not 49 49 55 00 (X4). `open` declines such
    // files before classification; the rule is kept for completeness.
    if c.data.get(..4) != Some(&MAGIC[..]) {
        return OutOfScope(Relative::X4);
    }
    let raw_format_present = c.ifd0.has(tag::RAW_FORMAT);
    let raw_format = c.uint(tag::RAW_FORMAT);
    let bits = c.uint(tag::BITS_PER_SAMPLE);
    // Rule 2: RawFormat 8 is X1.
    if raw_format == Some(8) {
        return OutOfScope(Relative::X1);
    }
    // Rules 3 to 9.
    let mut variant = match (raw_format, bits) {
        (Some(4), Some(12)) => Some(Variant::V1),
        (Some(5), Some(12)) => Some(Variant::V2),
        (Some(5), Some(14)) => Some(Variant::V3),
        (Some(6), Some(12)) => Some(Variant::V4),
        (Some(6), Some(14)) => Some(Variant::V5),
        (Some(7), Some(14)) => Some(Variant::V6),
        (Some(2), Some(12)) => Some(Variant::V7),
        _ => None,
    };
    // Rule 10: RawFormat absent, Compression 34828, BitsPerSample 12 and
    // StripByteCounts at least 2 * W * H.
    if variant.is_none() && !raw_format_present {
        let w = c.uint(tag::SENSOR_WIDTH).map(u64::from);
        let h = c.uint(tag::SENSOR_HEIGHT).map(u64::from);
        let count = c.uint(tag::STRIP_BYTE_COUNTS).map(u64::from);
        if let (Some(w), Some(h), Some(count)) = (w, h, count) {
            if c.uint(tag::COMPRESSION) == Some(34828) && bits == Some(12) && count >= 2 * w * h {
                variant = Some(Variant::V8);
            }
        }
    }
    match variant {
        Some(v) => {
            // R-RW2-4: W mod p must be 0 (X5). A missing or zero W fails
            // later by R-RW2-14.
            match c.uint(tag::SENSOR_WIDTH) {
                Some(w) if w > 0 && !(w as usize).is_multiple_of(v.per_block()) => {
                    OutOfScope(Relative::X5)
                }
                _ => Sensor(v),
            }
        }
        None if raw_format_present => OutOfScope(Relative::X2),
        None => OutOfScope(Relative::X3),
    }
}

/// The active area: rows `top..bottom`, columns `left..right` of the stored
/// raster.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Active {
    /// First row.
    pub top: usize,
    /// First column.
    pub left: usize,
    /// Row after the last row.
    pub bottom: usize,
    /// Column after the last column.
    pub right: usize,
}

impl Active {
    /// Width and height.
    pub(crate) fn size(&self) -> (usize, usize) {
        (self.right - self.left, self.bottom - self.top)
    }
}

/// Where the white balance multipliers came from (R-RW2-34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WbSource {
    /// Tags 0x0024 to 0x0026 (as shot).
    AsShot,
    /// The D50 scales of the model table.
    Table,
    /// The generic scales.
    Generic,
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
    /// The active area (R-RW2-15, R-RW2-17).
    pub active: Active,
    /// Colors (0 red, 1 green, 2 blue) of the CFA positions 0 to 3 of the
    /// stored raster (R-RW2-18).
    pub colors: [u8; 4],
    /// White level w_c of red, green and blue (R-RW2-31).
    pub white: [u32; 3],
    /// Black level b_c of red, green and blue (R-RW2-32).
    pub black: [u32; 3],
    /// WB multipliers mR, mG, mB (R-RW2-34).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// Normalized camera-to-sRGB matrix (R-RW2-36, R-RW2-37).
    pub matrix: [[f64; 3]; 3],
    /// True when the matrix came from the model table.
    pub matrix_from_table: bool,
    /// IFD0 orientation 1 to 8 (R-RW2-43).
    pub orientation: u8,
    /// File offset S of the sensor data (R-RW2-19).
    pub start: u64,
    /// The IFD0 Model string.
    pub model: String,
}

/// Largest W * H accepted (R-RW2-53).
const MAX_PIXELS: u64 = 1 << 27;
/// Bytes of one chunk of V1 to V3 (section Chunk rotation).
pub(super) const CHUNK: u64 = 16384;

/// The colors of CFA positions 0 to 3 for CFAPattern 1 to 4 (R-RW2-18),
/// `None` for another value (R-RW2-16).
pub(super) fn cfa_colors(pattern: u32) -> Option<[u8; 4]> {
    const R: u8 = 0;
    const G: u8 = 1;
    const B: u8 = 2;
    match pattern {
        1 => Some([R, G, G, B]),
        2 => Some([G, R, B, G]),
        3 => Some([G, B, R, G]),
        4 => Some([B, G, G, R]),
        _ => None,
    }
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check.
pub(super) fn plan(c: &Container<'_>, variant: Variant) -> Rw2Result<Plan> {
    let size = c.data.len() as u64;
    // R-RW2-14: the tags required by every variant.
    let w = c.required(tag::SENSOR_WIDTH, "SensorWidth")?;
    let h = c.required(tag::SENSOR_HEIGHT, "SensorHeight")?;
    let pattern = c.required(tag::CFA_PATTERN, "CFAPattern")?;
    c.required(tag::BITS_PER_SAMPLE, "BitsPerSample")?;
    // R-RW2-14: the tags required by some variants.
    if variant != Variant::V8 {
        c.required(tag::RAW_FORMAT, "RawFormat")?;
    }
    if variant == Variant::V8 {
        c.required(tag::COMPRESSION, "Compression")?;
        c.required(tag::STRIP_BYTE_COUNTS, "StripByteCounts")?;
    }
    // R-RW2-19: the data start from 0x0118 for V1 to V6, 0x0111 for V7, V8.
    let start = if variant.legacy() {
        c.required(tag::STRIP_OFFSETS, "StripOffsets")?
    } else {
        c.required(tag::RAW_DATA_OFFSET, "RawDataOffset")?
    };
    // R-RW2-14: W or H of 0. R-RW2-53: W or H above 65535, W * H above 2^27.
    if w == 0 || h == 0 {
        return Err(fail("raster width or height is 0"));
    }
    if w > 65535 || h > 65535 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    let (wu, hu) = (w as usize, h as usize);
    // R-RW2-4 again, so a plan never exists for X5.
    if !wu.is_multiple_of(variant.per_block()) {
        return Err(fail("raster width is not a multiple of the block pixels"));
    }

    // R-RW2-15: the border tags (defaults 0, 0, H, W) only when they form a
    // rectangle of at least 16 x 16 inside the raster; otherwise the whole
    // raster. R-RW2-17: the crop tags 0x002F to 0x0032 are never read.
    let top = c.uint(tag::TOP_BORDER).unwrap_or(0);
    let left = c.uint(tag::LEFT_BORDER).unwrap_or(0);
    let bottom = c.uint(tag::BOTTOM_BORDER).unwrap_or(h);
    let right = c.uint(tag::RIGHT_BORDER).unwrap_or(w);
    let borders_ok = top < bottom
        && bottom <= h
        && left < right
        && right <= w
        && bottom - top >= 16
        && right - left >= 16;
    let active = if borders_ok {
        Active {
            top: top as usize,
            left: left as usize,
            bottom: bottom as usize,
            right: right as usize,
        }
    } else {
        Active {
            top: 0,
            left: 0,
            bottom: hu,
            right: wu,
        }
    };

    // R-RW2-16: CFAPattern 1 to 4, else the preview path.
    let colors = cfa_colors(pattern).ok_or_else(|| fail("CFAPattern is not 1 to 4"))?;

    // R-RW2-30: F from the variant. R-RW2-31: w_c from LinearityLimit when
    // 0.9 * F <= value <= F, else F.
    let full = variant.full_scale();
    let white = tag::LINEARITY_LIMIT.map(|t| match c.uint(t) {
        Some(v) if 10 * u64::from(v) >= 9 * u64::from(full) && v <= full => v,
        _ => full,
    });
    // R-RW2-32: b_c = tag + 15 for V1 to V6, the tag for V7 and V8.
    // R-RW2-33: a missing tag counts as 0 before the offset.
    let offset = if variant.legacy() { 0 } else { 15 };
    let black = tag::BLACK_LEVEL.map(|t| c.uint(t).unwrap_or(0).saturating_add(offset));
    // R-RW2-33: the preview path when b_c >= w_c - 255.
    for k in 0..3 {
        if u64::from(black[k]) + 255 >= u64::from(white[k]) {
            return Err(fail("black level too close to the white level"));
        }
    }

    // R-RW2-34: the WB multipliers from the first available source.
    let model_bytes = c.model();
    let model = String::from_utf8_lossy(&model_bytes).into_owned();
    let levels = tag::WB_LEVEL.map(|t| c.uint(t));
    let (wb, wb_source) = match levels {
        // Source 1, all three greater than 0 (R-RW2-62: the divisor
        // WBGreenLevel is greater than 0).
        [Some(r), Some(g), Some(b)] if r > 0 && g > 0 && b > 0 => (
            [
                f64::from(r) / f64::from(g),
                1.0,
                f64::from(b) / f64::from(g),
            ],
            WbSource::AsShot,
        ),
        _ => match color::table_wb(&model_bytes) {
            // Source 2: the D50 scales of the model.
            Some((r, b)) => ([r, 1.0, b], WbSource::Table),
            // Source 3: the generic scales.
            None => (
                [color::GENERIC_WB.0, 1.0, color::GENERIC_WB.1],
                WbSource::Generic,
            ),
        },
    };
    let (matrix, matrix_from_table) = color::matrix(&model_bytes);

    // R-RW2-57: the data start must not be 0xFFFFFFFF, must not lie before
    // the end of IFD0 and must lie inside the file.
    let start = u64::from(start);
    if start == 0xFFFF_FFFF || start < c.ifd0_end || start >= size {
        return Err(fail("sensor data start outside the file"));
    }
    // R-RW2-20, R-RW2-22, R-RW2-55: the whole data range inside the file,
    // in 64-bit arithmetic (R-RW2-52).
    let pixels = u64::from(w) * u64::from(h);
    let blocks = pixels / variant.per_block() as u64;
    let needed = match variant {
        // R-RW2-22: whole chunks of 16384 bytes.
        Variant::V1 | Variant::V2 | Variant::V3 => (16 * blocks).div_ceil(CHUNK) * CHUNK,
        // R-RW2-20: 2 * W * H bytes for V8.
        Variant::V8 => 2 * pixels,
        // R-RW2-20: 16 * NB bytes for V4 to V7.
        _ => 16 * blocks,
    };
    if start + needed > size {
        return Err(fail("sensor data extend beyond the end of the file"));
    }

    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        active,
        colors,
        white,
        black,
        wb,
        wb_source,
        matrix,
        matrix_from_table,
        orientation: c.orientation(),
        start,
        model,
    })
}

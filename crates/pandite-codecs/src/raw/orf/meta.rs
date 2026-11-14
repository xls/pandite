//! The container of an ORF file (header, IFD chain, Exif IFD, maker note
//! and its sub-IFDs), the classification of R-ORF-3 and the decode plan:
//! sizes, strip layout, bit depth, levels, white balance, color matrix,
//! crop and orientation, all validated before any large allocation.
//!
//! The TIFF structure is read with the Samsung decoder's reader
//! (`srw::ifd`): IFDs in either byte order (R-ORF-5), entries of field
//! types 1 to 13 with inline or out-of-line values (R-ARW-6, R-ARW-9), the
//! first entry of a tag (R-ORF-6), the validity rules of R-ARW-14, the IFD
//! chain of R-ARW-12, and value offsets counted from any origin, which the
//! maker note needs (R-ORF-12). Every value range is checked against the
//! file before it is read, and a count * type size beyond the file size
//! fails before any allocation (R-ORF-57).

use super::{fail, OrfResult};
use crate::raw::arw::Rect;
use crate::raw::srw::ifd::{self, Dir};

/// Tag numbers of the specification's Metadata tags tables.
pub(super) mod tag {
    // IFD0.
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const BITS_PER_SAMPLE: u16 = 0x0102;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const JPEG_OFFSET: u16 = 0x0201;
    pub const JPEG_LENGTH: u16 = 0x0202;
    // Exif IFD.
    pub const CFA_PATTERN: u16 = 0xA302;
    pub const MAKER_NOTE: u16 = 0x927C;
    // Main maker-note IFD.
    pub const THUMBNAIL_IMAGE: u16 = 0x0100;
    pub const MAIN_COLOR_MATRIX: u16 = 0x1011;
    pub const MAIN_BLACK_LEVEL: u16 = 0x1012;
    pub const RED_BALANCE: u16 = 0x1017;
    pub const BLUE_BALANCE: u16 = 0x1018;
    pub const MAIN_VALID_BITS: u16 = 0x102C;
    pub const CAMERA_SETTINGS: u16 = 0x2020;
    pub const IMAGE_PROCESSING: u16 = 0x2040;
    pub const RAW_INFO: u16 = 0x3000;
    // ImageProcessing and RawInfo IFDs.
    pub const WB_RB_LEVELS: u16 = 0x0100;
    pub const COLOR_MATRIX: u16 = 0x0200;
    pub const BLACK_LEVEL2: u16 = 0x0600;
    pub const GAIN_BASE: u16 = 0x0610;
    pub const VALID_BITS: u16 = 0x0611;
    pub const CROP_LEFT: u16 = 0x0612;
    pub const CROP_TOP: u16 = 0x0613;
    pub const CROP_WIDTH: u16 = 0x0614;
    pub const CROP_HEIGHT: u16 = 0x0615;
    // CameraSettings IFD.
    pub const PREVIEW_IMAGE_START: u16 = 0x0101;
    pub const PREVIEW_IMAGE_LENGTH: u16 = 0x0102;
}

/// The storage variants of the sensor data (section Scope and variants).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// 16-bit samples in the file byte order, B bits in the high part.
    V1,
    /// 12-bit samples, 10 samples in 15 bytes plus 1 pad byte.
    V2,
    /// 12-bit pairs, MSB-first, rows stored as two fields.
    V3,
    /// 12-bit values, MSB-first bit string in 32-bit little-endian words.
    V4,
    /// Olympus lossless compression, B = 12.
    V5,
    /// Olympus lossless compression, B = 14.
    V6,
}

/// The out-of-scope relatives that use the preview path (R-ORF-2). X3
/// (ORI files) has no detection of its own: such a file is classified by
/// its content like any other file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// A file that matches no rule of R-ORF-3.
    X1,
    /// A big-endian file with a layout other than V1.
    X2,
}

/// The result of classification (R-ORF-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A relative that uses the preview path.
    OutOfScope(Relative),
}

/// The maker-note header forms of section Container layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Style {
    /// "OLYMP", 00 and 2 version bytes: offsets count from the file start.
    Olymp,
    /// "OLYMPUS", 00, byte order and 2 version bytes.
    Olympus,
    /// "OM SYSTEM", 00 00 00, byte order and 2 version bytes.
    OmSystem,
}

/// The source of a level, a multiplier pair or the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// The ImageProcessing IFD (main maker-note tag 0x2040).
    ImageProcessing,
    /// The RawInfo IFD (main maker-note tag 0x3000).
    RawInfo,
    /// The main maker-note IFD.
    Main,
    /// The generic values of the specification.
    Generic,
}

/// The maker note: its style, origin, byte order and IFDs.
pub(super) struct Note<'a> {
    /// The header form.
    pub style: Style,
    /// File offset of the first maker-note byte.
    #[cfg_attr(not(test), allow(dead_code))]
    pub start: u64,
    /// The origin of every offset inside the maker note (R-ORF-12).
    pub origin: u64,
    /// True when the maker note is little-endian.
    pub le: bool,
    /// The main maker-note IFD, when valid.
    pub main: Option<Dir<'a>>,
    /// The CameraSettings IFD (tag 0x2020), when present and valid.
    pub camera_settings: Option<Dir<'a>>,
    /// The ImageProcessing IFD (tag 0x2040), when present and valid.
    pub image_processing: Option<Dir<'a>>,
    /// The RawInfo IFD (tag 0x3000), when present and valid.
    pub raw_info: Option<Dir<'a>>,
}

/// The parsed container.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// True when header bytes 0 and 1 are 49 49 (R-ORF-5).
    pub le: bool,
    /// True when header bytes 2 and 3 are 52 53.
    pub rs: bool,
    /// IFD0 and the IFDs of its chain (R-ORF-9), never empty.
    pub chain: Vec<Dir<'a>>,
    /// The Exif IFD (R-ORF-10), when valid.
    pub exif: Option<Dir<'a>>,
    /// The maker note (R-ORF-11), when it has one of the three headers.
    pub note: Option<Note<'a>>,
}

/// Stream header of V5 (R-ORF-26).
pub(super) const V5_HEADER: [u8; 7] = [0, 0, 0, 0, 1, 0, 0];
/// Stream header of V6 (R-ORF-26).
pub(super) const V6_HEADER: [u8; 8] = [0, 0, 0, 0, 0, 1, 0, 0];
/// The generic multipliers R, G, B of R-ORF-38 source 4.
pub(super) const GENERIC_WB: [f64; 3] = [1.944, 1.0, 1.801];
/// The generic matrix of section Color conversion (rows sRGB R, G, B;
/// columns camera R, G, B).
pub(super) const GENERIC_MATRIX: [[f64; 3]; 3] = [
    [1.418, -0.317, -0.101],
    [-0.200, 1.407, -0.207],
    [0.030, -0.419, 1.389],
];
/// Largest W * H accepted (R-ORF-56).
const MAX_PIXELS: u64 = 1 << 27;
/// Largest W and H accepted (R-ORF-56).
const MAX_SIDE: u32 = 65535;
/// Bits of the shortest code of V5 and V6: the sign bit, 2 low bits, a
/// unary part of at least 1 bit and k >= 2 remainder bits (R-ORF-28,
/// R-ORF-29).
const MIN_CODE_BITS: u64 = 6;

/// Parse the header, the IFD chain, the Exif IFD and the maker note. An
/// error means "no raw image and no preview" (R-ORF-4).
pub(super) fn open(data: &[u8]) -> OrfResult<Container<'_>> {
    // R-ORF-55: a file shorter than 8 bytes has no header.
    if data.len() < 8 {
        return Err(fail("file shorter than 8 bytes"));
    }
    // R-ORF-4: only the three ORF starts. R-ORF-5: bytes 0 and 1 give the
    // byte order of the TIFF structure.
    let le = match data[..4] {
        [0x49, 0x49, 0x52, 0x4F] | [0x49, 0x49, 0x52, 0x53] => true,
        [0x4D, 0x4D, 0x4F, 0x52] => false,
        _ => return Err(fail("not an ORF header")),
    };
    // R-ORF-8: IFD0 at the offset in header bytes 4 to 7 is the raw IFD.
    let first = ifd::read32(data, 4, le).unwrap_or(0);
    // R-ORF-6, R-ORF-9: IFD0 and the IFDs of its next-IFD chain.
    let chain = ifd::chain(data, le, first);
    // R-ORF-4: an invalid IFD0 (R-ARW-14) is an error without a preview.
    let Some(ifd0) = chain.first() else {
        return Err(fail("IFD0 is not a valid IFD"));
    };
    let exif = exif_ifd(data, le, ifd0);
    let note = exif.as_ref().and_then(|e| maker_note(data, le, e));
    Ok(Container {
        data,
        le,
        rs: data[2..4] == [0x52, 0x53],
        chain,
        exif,
        note,
    })
}

/// R-ORF-10: the Exif IFD at the offset in IFD0 tag 0x8769 (LONG or IFD,
/// R-ORF-7), read with the file byte order and file offsets.
fn exif_ifd<'a>(data: &'a [u8], le: bool, ifd0: &Dir<'a>) -> Option<Dir<'a>> {
    let at = ifd0.uint(tag::EXIF_IFD).ok().flatten()?;
    ifd::dir(data, le, u64::from(at), 0)
}

/// R-ORF-10 to R-ORF-13: the maker note, the value of Exif tag 0x927C.
/// `None` when the tag is absent, its value does not lie inside the file
/// (R-ORF-14), it is shorter than its header or it has another header
/// (R-ORF-11).
fn maker_note<'a>(data: &'a [u8], le: bool, exif: &Dir<'a>) -> Option<Note<'a>> {
    let e = exif.entry(tag::MAKER_NOTE)?;
    let bytes = exif.bytes(e).ok().flatten()?;
    let start = e.at;
    let order = |b: &[u8]| match b {
        b"II" => Some(true),
        b"MM" => Some(false),
        _ => None,
    };
    // R-ORF-11: the style from the header bytes, without the version
    // bytes. R-ORF-12: the byte order of the style ("OLYMPUS" starts with
    // "OLYMP" but has no 00 at its sixth byte).
    let (style, header, note_le) = if bytes.len() >= 8 && bytes.starts_with(b"OLYMP\0") {
        (Style::Olymp, 8, le)
    } else if bytes.len() >= 12 && bytes.starts_with(b"OLYMPUS\0") {
        (Style::Olympus, 12, order(&bytes[8..10])?)
    } else if bytes.len() >= 16 && bytes.starts_with(b"OM SYSTEM\0\0\0") {
        (Style::OmSystem, 16, order(&bytes[12..14])?)
    } else {
        return None;
    };
    // R-ORF-12: offsets inside the maker note count from the file start
    // (OLYMP) or from the maker-note start (OLYMPUS, OM SYSTEM). The main
    // IFD follows the header.
    let origin = if style == Style::Olymp { 0 } else { start };
    let main = ifd::dir(data, note_le, start + header, origin);
    let sub = |t: u16| {
        main.as_ref()
            .and_then(|m| sub_ifd(data, note_le, origin, m, t))
    };
    Some(Note {
        style,
        start,
        origin,
        le: note_le,
        camera_settings: sub(tag::CAMERA_SETTINGS),
        image_processing: sub(tag::IMAGE_PROCESSING),
        raw_info: sub(tag::RAW_INFO),
        main,
    })
}

/// R-ORF-13: the sub-IFD of main maker-note tag `t`: at origin + value for
/// type 13 or 4 with count 1, at the value offset for type 7 with a count
/// greater than 4, absent in every other case and when invalid
/// (R-ARW-14). Its own offsets count from the origin of the style.
fn sub_ifd<'a>(data: &'a [u8], le: bool, origin: u64, main: &Dir<'a>, t: u16) -> Option<Dir<'a>> {
    let e = main.entry(t)?;
    let at = match (e.typ, e.count) {
        (4 | 13, 1) => origin + u64::from(main.uint(t).ok()??),
        (7, n) if n > 4 => e.at,
        _ => return None,
    };
    ifd::dir(data, le, at, origin)
}

impl<'a> Container<'a> {
    /// IFD0, the raw IFD (R-ORF-8).
    pub(super) fn ifd0(&self) -> &Dir<'a> {
        &self.chain[0]
    }

    /// R-ORF-46: the IFD0 Orientation, 1 when absent or outside 1 to 8.
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().uint(tag::ORIENTATION) {
            Ok(Some(v @ 1..=8)) => v as u8,
            _ => 1,
        }
    }

    /// The IFD0 Model, for reports only (not used for decoding).
    pub(super) fn model(&self) -> String {
        let m = self.ifd0().ascii(tag::MODEL).ok().flatten();
        String::from_utf8_lossy(m.unwrap_or_default()).into_owned()
    }
}

fn require<T>(v: Option<T>, what: &str) -> OrfResult<T> {
    // R-ORF-14: a required tag that is absent or has a count of 0 (the
    // reader treats count 0 as absent) uses the preview path.
    v.ok_or_else(|| fail(format!("required tag {what} missing")))
}

/// The first 8 bytes of the first strip, when the strip lies inside the
/// file and holds at least 8 bytes (otherwise rules 6 and 7 of R-ORF-3 do
/// not match).
fn strip_head(data: &[u8], offset: u32, count: Option<u32>) -> Option<&[u8]> {
    let (offset, count) = (u64::from(offset), u64::from(count?));
    if count < 8 || offset + count > data.len() as u64 {
        return None;
    }
    data.get(offset as usize..offset as usize + 8)
}

/// Classify the file by the rules of R-ORF-3, in this order, after the
/// check of the required IFD0 tags (R-ORF-14).
pub(super) fn classify(c: &Container<'_>) -> OrfResult<Class> {
    use Class::{OutOfScope, Sensor};
    let ifd0 = c.ifd0();
    let w = require(ifd0.uint(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(ifd0.uint(tag::IMAGE_LENGTH)?, "ImageLength")?;
    // BPS is the first value of tag 0x0102.
    let bps = require(ifd0.uint(tag::BITS_PER_SAMPLE)?, "BitsPerSample")?;
    let offsets = require(ifd0.uints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(ifd0.uints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    require(ifd0.uint(tag::EXIF_IFD)?, "ExifIFD")?;
    // n = W * H and T, the total of all StripByteCounts values, in 64-bit
    // arithmetic (R-ORF-55).
    let w = u64::from(w);
    let n = w * u64::from(h);
    let t: u64 = counts.iter().map(|&v| u64::from(v)).sum();
    // Rule 1: T = 2 * n.
    if t == 2 * n {
        return Ok(Sensor(Variant::V1));
    }
    // Rule 2: header bytes 0 and 1 are 4D 4D.
    if !c.le {
        return Ok(OutOfScope(Relative::X2));
    }
    // Rule 3: W mod 10 = 0 and T = 16 * n / 10.
    if w % 10 == 0 && 10 * t == 16 * n {
        return Ok(Sensor(Variant::V2));
    }
    // Rule 4: BPS 12, header bytes 2 and 3 are 52 53, W even, T = 3 n / 2.
    if bps == 12 && c.rs && w % 2 == 0 && 2 * t == 3 * n {
        return Ok(Sensor(Variant::V3));
    }
    // Rule 5: BPS 12, header bytes 2 and 3 are 52 4F, W mod 8 = 0, T = 3 n
    // / 2.
    if bps == 12 && !c.rs && w % 8 == 0 && 2 * t == 3 * n {
        return Ok(Sensor(Variant::V4));
    }
    // Rules 6 and 7: one strip, W even, and the stream header.
    if offsets.len() == 1 && w % 2 == 0 {
        if let Some(head) = strip_head(c.data, offsets[0], counts.first().copied()) {
            if head[..7] == V5_HEADER {
                return Ok(Sensor(Variant::V5));
            }
            if head == V6_HEADER {
                return Ok(Sensor(Variant::V6));
            }
        }
    }
    // Rule 8.
    Ok(OutOfScope(Relative::X1))
}

/// Where the sensor data lie in the file.
#[derive(Clone, Debug)]
pub(super) enum Layout {
    /// V1 to V4: the file offset of every stored row, in stored order (H
    /// rows), and the bytes per stored row RB.
    Rows {
        /// File offset of each stored row.
        starts: Vec<usize>,
        /// RB of section Sensor data layout.
        row_bytes: usize,
    },
    /// V5 and V6: the code bits, from the first byte after the stream
    /// header to the end of the strip.
    Stream {
        /// File offset of the first code byte (R-ORF-26).
        start: usize,
        /// File offset after the last strip byte.
        end: usize,
    },
}

/// Everything needed to develop the sensor data.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    /// The storage variant.
    pub variant: Variant,
    /// Stored width W.
    pub width: usize,
    /// Stored height H.
    pub height: usize,
    /// The final image area (R-ORF-17).
    pub crop: Rect,
    /// IFD0 orientation 1 to 8 (R-ORF-46).
    pub orientation: u8,
    /// Colors of CFA positions 0 to 3, origin at raster pixel (0, 0)
    /// (R-ORF-19).
    pub cfa: [u8; 4],
    /// True when the file is little-endian (V1 samples, R-ORF-5).
    pub le: bool,
    /// Bit depth B (R-ORF-33, R-ORF-34).
    pub bits: u32,
    /// White level 2^B - 1 (R-ORF-35).
    pub white: u32,
    /// Black levels of CFA positions 0 to 3 (R-ORF-36, R-ORF-37).
    pub black: [u32; 4],
    /// Where the black levels came from.
    pub black_source: Source,
    /// WB multipliers mR, mG, mB (R-ORF-38).
    pub wb: [f64; 3],
    /// Where the multipliers came from.
    pub wb_source: Source,
    /// The camera-to-sRGB matrix, each row divided by its sum (R-ORF-40,
    /// R-ORF-41).
    pub matrix: [[f64; 3]; 3],
    /// Where the matrix came from.
    pub matrix_source: Source,
    /// Where the sensor data lie.
    pub layout: Layout,
    /// The maker-note style.
    pub style: Style,
    /// The IFD0 Model string.
    pub model: String,
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check, before any
/// large allocation (decoding procedure steps 5 to 7).
pub(super) fn plan(c: &Container<'_>, variant: Variant) -> OrfResult<Plan> {
    let data_len = c.data.len() as u64;
    // R-ORF-55: a file longer than 2^32 - 1 bytes fails.
    if data_len > u64::from(u32::MAX) {
        return Err(fail("file longer than 2^32 - 1 bytes"));
    }
    let ifd0 = c.ifd0();
    // R-ORF-14: the required IFD0 tags.
    let w = require(ifd0.uint(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(ifd0.uint(tag::IMAGE_LENGTH)?, "ImageLength")?;
    require(ifd0.uint(tag::BITS_PER_SAMPLE)?, "BitsPerSample")?;
    let offsets = require(ifd0.uints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(ifd0.uints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    let exif_at = require(ifd0.uint(tag::EXIF_IFD)?, "ExifIFD")?;
    // R-ORF-56: limits on W, H and W * H.
    if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || u64::from(w) * u64::from(h) > MAX_PIXELS
    {
        return Err(fail("raster size outside the limits"));
    }
    let (wu, hu) = (w as usize, h as usize);
    // R-ORF-14: the Exif IFD offset inside the file; an invalid Exif IFD
    // holds none of its required tags.
    if u64::from(exif_at) >= data_len {
        return Err(fail("Exif IFD outside the file"));
    }
    let exif = c
        .exif
        .as_ref()
        .ok_or_else(|| fail("Exif IFD is not a valid IFD"))?;
    // R-ORF-15, R-ORF-19.
    let cfa = cfa_pattern(exif, c.le)?;
    // R-ORF-11, R-ORF-14: the maker note is required.
    let note = c
        .note
        .as_ref()
        .ok_or_else(|| fail("maker note absent, outside the file or of another style"))?;
    let layout = layout(variant, &offsets, &counts, wu, hu, data_len)?;
    let bits = bit_depth(Some(note), variant)?;
    // R-ORF-35: w = 2^B - 1.
    let white = (1u32 << bits) - 1;
    let (black, black_source) = black_levels(note, bits, white)?;
    let (wb, wb_source) = multipliers(note)?;
    let (matrix, matrix_source) = color_matrix(note)?;
    let crop = crop(note, w, h)?;
    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        crop,
        orientation: c.orientation(),
        cfa,
        le: c.le,
        bits,
        white,
        black,
        black_source,
        wb,
        wb_source,
        matrix,
        matrix_source,
        layout,
        style: note.style,
        model: c.model(),
    })
}

/// R-ORF-15: the CFA pattern of Exif tag 0xA302: 8 bytes, the SHORT
/// repeat values 2 and 2 in the file byte order, then the colors of CFA
/// positions 0 to 3 with exactly one 0, two 1 and one 2. Otherwise the
/// preview path. R-ORF-19: the pattern origin is raster pixel (0, 0).
pub(super) fn cfa_pattern(exif: &Dir<'_>, le: bool) -> OrfResult<[u8; 4]> {
    let p = require(exif.byte_values(tag::CFA_PATTERN)?, "CFAPattern")?;
    if p.len() != 8 {
        return Err(fail("CFAPattern does not have 8 bytes"));
    }
    if ifd::read16(p, 0, le) != Some(2) || ifd::read16(p, 2, le) != Some(2) {
        return Err(fail("CFA repeat is not 2 x 2"));
    }
    let colors = [p[4], p[5], p[6], p[7]];
    let count = |v: u8| colors.iter().filter(|&&x| x == v).count();
    if count(0) != 1 || count(1) != 2 || count(2) != 1 {
        return Err(fail("CFA pattern is not a Bayer pattern"));
    }
    Ok(colors)
}

/// The strip layout of R-ORF-20 and R-ORF-26 with the checks of R-ORF-14,
/// R-ORF-57, R-ORF-58 and R-ORF-59.
fn layout(
    variant: Variant,
    offsets: &[u32],
    counts: &[u32],
    w: usize,
    h: usize,
    data_len: u64,
) -> OrfResult<Layout> {
    // R-ORF-58 case 1: equal counts.
    if offsets.len() != counts.len() {
        return Err(fail("StripOffsets and StripByteCounts differ in count"));
    }
    // R-ORF-14, R-ORF-57: every strip byte range lies inside the file
    // (64-bit sums, R-ORF-55).
    for (&o, &n) in offsets.iter().zip(counts) {
        if u64::from(o) + u64::from(n) > data_len {
            return Err(fail("a strip lies outside the file"));
        }
    }
    // RB of section Sensor data layout. R-ORF-3 makes W a multiple of 10
    // for V2, of 2 for V3 and of 8 for V4.
    let row_bytes = match variant {
        Variant::V1 => 2 * w,
        Variant::V2 => 16 * w / 10,
        Variant::V3 | Variant::V4 => 3 * w / 2,
        Variant::V5 | Variant::V6 => {
            let header = if variant == Variant::V5 {
                V5_HEADER.len()
            } else {
                V6_HEADER.len()
            } as u64;
            if offsets.len() != 1 {
                return Err(fail("compressed data in more than one strip"));
            }
            let (start, len) = (u64::from(offsets[0]), u64::from(counts[0]));
            // R-ORF-58 case 3: the stream header plus 1 byte.
            if len < header + 1 {
                return Err(fail("strip shorter than its stream header plus 1 byte"));
            }
            // Every code holds at least 6 bits, so a strip with fewer than
            // 6 * W * H code bits fails by R-ORF-60; this is checked before
            // the raster is allocated, which bounds the allocation by the
            // strip size.
            if (len - header) * 8 < MIN_CODE_BITS * (w as u64) * (h as u64) {
                return Err(fail("strip too short for the raster"));
            }
            return Ok(Layout::Stream {
                start: (start + header) as usize,
                end: (start + len) as usize,
            });
        }
    };
    // R-ORF-20: StripByteCounts / RB rows from each strip, in the order of
    // the strips, at its offset, until H rows; strips need not follow each
    // other or hold RowsPerStrip rows.
    let mut starts = Vec::with_capacity(h);
    for (&o, &n) in offsets.iter().zip(counts) {
        let n = n as usize;
        // R-ORF-58 case 2: a whole number of rows in each strip.
        if !n.is_multiple_of(row_bytes) {
            return Err(fail("a strip does not hold a whole number of rows"));
        }
        let take = (n / row_bytes).min(h - starts.len());
        starts.extend((0..take).map(|r| o as usize + r * row_bytes));
    }
    // R-ORF-58 case 2: the strips hold at least H rows. R-ORF-59: exactly
    // H rows of RB bytes are read, never more.
    if starts.len() < h {
        return Err(fail("the strips hold fewer than H rows"));
    }
    Ok(Layout::Rows { starts, row_bytes })
}

/// R-ORF-33 and R-ORF-34: the bit depth B. For V1 the first value of the
/// first present tag of ImageProcessing 0x0611, RawInfo 0x0611 and main
/// maker-note 0x102C; 12 when none is present or when that value lies
/// outside 10 to 16 (the next tag is not tried).
pub(super) fn bit_depth(note: Option<&Note<'_>>, variant: Variant) -> OrfResult<u32> {
    Ok(match variant {
        Variant::V2 | Variant::V3 | Variant::V4 | Variant::V5 => 12,
        Variant::V6 => 14,
        Variant::V1 => {
            let list = note.map_or([None, None, None], |n| {
                [
                    n.image_processing.as_ref().map(|d| (d, tag::VALID_BITS)),
                    n.raw_info.as_ref().map(|d| (d, tag::VALID_BITS)),
                    n.main.as_ref().map(|d| (d, tag::MAIN_VALID_BITS)),
                ]
            });
            match list.into_iter().flatten().find(|(d, t)| d.has(*t)) {
                Some((d, t)) => d.uint(t)?.filter(|b| (10..=16).contains(b)).unwrap_or(12),
                None => 12,
            }
        }
    })
}

/// R-ORF-36: the black levels of CFA positions 0 to 3 from the first
/// source that applies and whose tag has exactly 4 values with every
/// scaled value at most w / 4; the preview path when none qualifies.
pub(super) fn black_levels(
    note: &Note<'_>,
    bits: u32,
    white: u32,
) -> OrfResult<([u32; 4], Source)> {
    let sources = [
        // Source 1, on the 12-bit scale: b = value * 2^(B - 12), only for
        // B >= 12.
        (
            note.image_processing.as_ref().filter(|_| bits >= 12),
            tag::BLACK_LEVEL2,
            bits.saturating_sub(12),
            Source::ImageProcessing,
        ),
        // Sources 2 and 3, on the data scale.
        (
            note.raw_info.as_ref(),
            tag::BLACK_LEVEL2,
            0,
            Source::RawInfo,
        ),
        (note.main.as_ref(), tag::MAIN_BLACK_LEVEL, 0, Source::Main),
    ];
    for (dir, t, shift, source) in sources {
        let Some(d) = dir else {
            continue;
        };
        let Some(v) = d.uints(t)? else {
            continue;
        };
        if v.len() != 4 {
            continue;
        }
        let b = [v[0], v[1], v[2], v[3]].map(|x| u64::from(x) << shift);
        // b <= w / 4, in integers: 4 * b <= w.
        if b.iter().all(|&b| 4 * b <= u64::from(white)) {
            return Ok((b.map(|b| b as u32), source));
        }
    }
    Err(fail("no black level source qualifies"))
}

/// R-ORF-38: mR and mB from the first source whose tags are present and
/// give two multipliers from 0.0625 to 16; mG = 1.
pub(super) fn multipliers(note: &Note<'_>) -> OrfResult<([f64; 3], Source)> {
    let pair = |r: f64, b: f64| {
        let ok = |m: f64| (0.0625..=16.0).contains(&m);
        (ok(r) && ok(b)).then_some([r, 1.0, b])
    };
    // Source 1: WB_RBLevels values 1 and 2 over GainBase G0 (64 to 4096,
    // else 256: the divisor is never 0, R-ORF-63).
    if let Some(ip) = &note.image_processing {
        if let Some(v) = ip.uints(tag::WB_RB_LEVELS)?.filter(|v| v.len() >= 2) {
            let g0 = match ip.uint(tag::GAIN_BASE)? {
                Some(g @ 64..=4096) => f64::from(g),
                _ => 256.0,
            };
            if let Some(m) = pair(f64::from(v[0]) / g0, f64::from(v[1]) / g0) {
                return Ok((m, Source::ImageProcessing));
            }
        }
    }
    // Source 2: WB_RBLevelsUsed over 256.
    if let Some(ri) = &note.raw_info {
        if let Some(v) = ri.uints(tag::WB_RB_LEVELS)?.filter(|v| v.len() >= 2) {
            if let Some(m) = pair(f64::from(v[0]) / 256.0, f64::from(v[1]) / 256.0) {
                return Ok((m, Source::RawInfo));
            }
        }
    }
    // Source 3: RedBalance and BlueBalance, both present, first values.
    if let Some(main) = &note.main {
        if let (Some(r), Some(b)) = (main.uint(tag::RED_BALANCE)?, main.uint(tag::BLUE_BALANCE)?) {
            if let Some(m) = pair(f64::from(r) / 256.0, f64::from(b) / 256.0) {
                return Ok((m, Source::Main));
            }
        }
    }
    // Source 4.
    Ok((GENERIC_WB, Source::Generic))
}

/// R-ORF-16, R-ORF-7: the values of a SHORT tag marked "signed" or of an
/// SSHORT tag as signed 16-bit integers (v >= 32768 means v - 65536).
pub(super) fn signed16(d: &Dir<'_>, le: bool, t: u16) -> OrfResult<Option<Vec<i32>>> {
    let Some(e) = d.entry(t) else {
        return Ok(None);
    };
    if !matches!(e.typ, 3 | 8) {
        return Ok(None);
    }
    let Some(b) = d.bytes(e)? else {
        return Ok(None);
    };
    Ok(Some(
        b.chunks_exact(2)
            .map(|c| {
                let v = if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                };
                i32::from(v as i16)
            })
            .collect(),
    ))
}

/// R-ORF-41: each row divided by its sum, when all three row sums are
/// greater than 0 (R-ORF-63: no division by 0 or a negative sum).
fn normalized(m: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let sums = m.map(|r| r.iter().sum::<f64>());
    sums.iter()
        .all(|&s| s > 0.0)
        .then(|| [0, 1, 2].map(|r| m[r].map(|v| v / sums[r])))
}

/// R-ORF-40, R-ORF-41: the matrix of the first present tag of
/// ImageProcessing 0x0200, RawInfo 0x0200 and main maker-note 0x1011 that
/// has 9 values, divided by 256 and row-normalized; the generic matrix when
/// no tag qualifies or a row sum is not greater than 0.
pub(super) fn color_matrix(note: &Note<'_>) -> OrfResult<([[f64; 3]; 3], Source)> {
    let list = [
        (
            note.image_processing.as_ref(),
            tag::COLOR_MATRIX,
            Source::ImageProcessing,
        ),
        (note.raw_info.as_ref(), tag::COLOR_MATRIX, Source::RawInfo),
        (note.main.as_ref(), tag::MAIN_COLOR_MATRIX, Source::Main),
    ];
    let generic = (
        normalized(GENERIC_MATRIX).unwrap_or(GENERIC_MATRIX),
        Source::Generic,
    );
    for (dir, t, source) in list {
        let Some(d) = dir else {
            continue;
        };
        let Some(v) = signed16(d, note.le, t)?.filter(|v| v.len() == 9) else {
            continue;
        };
        let m = [0, 1, 2].map(|r| [0, 1, 2].map(|k| f64::from(v[3 * r + k]) / 256.0));
        return Ok(normalized(m).map_or(generic, |m| (m, source)));
    }
    Ok(generic)
}

/// R-ORF-17: the crop rectangle of the ImageProcessing IFD when its four
/// tags are present with CW >= 2, CH >= 2, CX + CW <= W and CY + CH <= H,
/// else that of the RawInfo IFD under the same conditions, else the whole
/// raster. R-ORF-18: AspectFrame (0x1113) is never read.
pub(super) fn crop(note: &Note<'_>, w: u32, h: u32) -> OrfResult<Rect> {
    for d in [note.image_processing.as_ref(), note.raw_info.as_ref()]
        .into_iter()
        .flatten()
    {
        let (Some(x), Some(y), Some(cw), Some(ch)) = (
            d.uint(tag::CROP_LEFT)?,
            d.uint(tag::CROP_TOP)?,
            d.uint(tag::CROP_WIDTH)?,
            d.uint(tag::CROP_HEIGHT)?,
        ) else {
            continue;
        };
        let (x, y, cw, ch) = (u64::from(x), u64::from(y), u64::from(cw), u64::from(ch));
        if cw >= 2 && ch >= 2 && x + cw <= u64::from(w) && y + ch <= u64::from(h) {
            return Ok(Rect {
                x: x as usize,
                y: y as usize,
                w: cw as usize,
                h: ch as usize,
            });
        }
    }
    Ok(Rect {
        x: 0,
        y: 0,
        w: w as usize,
        h: h as usize,
    })
}

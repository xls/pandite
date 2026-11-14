//! The container of a NEF file (IFD chain, SubIFDs, Exif IFD and maker
//! note), the classification of R-NEF-3 and the decode plan: sizes, strip
//! layout, code table and curve, levels, white balance, color matrix, crop
//! and orientation, all validated before any large allocation.

use super::color;
use super::huffman::{self, Spec};
use super::tiff::{read32, u16_at, Ifd};
use super::{fail, NefResult};
use crate::raw::arw::Rect;

/// Tag numbers of the specification's Metadata tags tables.
pub(super) mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 0x00FE;
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const BITS_PER_SAMPLE: u16 = 0x0102;
    pub const COMPRESSION: u16 = 0x0103;
    pub const PHOTOMETRIC: u16 = 0x0106;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const SAMPLES_PER_PIXEL: u16 = 0x0115;
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
    pub const WB_RB_LEVELS: u16 = 0x000C;
    pub const PREVIEW_IFD: u16 = 0x0011;
    pub const BLACK_LEVEL: u16 = 0x003D;
    pub const CROP_AREA: u16 = 0x0045;
    pub const MAKER_NOTES_0X51: u16 = 0x0051;
    pub const NEF_COMPRESSION: u16 = 0x0093;
    pub const LINEARIZATION: u16 = 0x0096;
    pub const COLOR_BALANCE: u16 = 0x0097;
}

/// The storage variants this decoder develops from sensor data (R-NEF-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Uncompressed 16-bit samples in the file byte order.
    V1,
    /// Uncompressed 12-bit packed, most significant bits first.
    V2,
    /// D100: 12-bit packed in 16-byte groups of 10 pixels.
    V3,
    /// Uncompressed 12-bit packed, least significant bits first.
    V4,
    /// Uncompressed 14-bit packed, least significant bits first.
    V5,
    /// Lossless compressed (tables L12, L14), no curve.
    V6,
    /// Lossy compressed type 1 (table Y12), curve stored directly.
    V7,
    /// Lossy compressed type 2 (tables Y12, Y14), curve of 257 knots.
    V8,
}

impl Variant {
    /// True for the Huffman variants V6, V7 and V8.
    pub(crate) fn huffman(self) -> bool {
        matches!(self, Variant::V6 | Variant::V7 | Variant::V8)
    }
}

/// The out-of-scope relatives that use the preview path (R-NEF-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// High Efficiency and High Efficiency* (NEFCompression 13 or 14).
    X1,
    /// Small raw (NEFCompression 8).
    X2,
    /// Lossy compressed files of the D1 family (NEFCompression 1, no tag
    /// 0x0096).
    X3,
    /// D1X (non-square pixels).
    X4,
    /// Coolpix E-series NEF.
    X5,
    /// No CFA raw IFD.
    X6,
    /// Any other combination.
    X7,
}

/// The result of classification (R-NEF-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A relative that uses the preview path.
    OutOfScope(Relative),
}

/// Where the white balance multipliers came from (R-NEF-38).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WbSource {
    /// Source 1: maker-note tag 0x000C WB_RBLevels (as shot).
    RbLevels,
    /// Source 2: maker-note tag 0x0097 version "0102" (as shot).
    ColorBalance0102,
    /// Source 3: maker-note tag 0x0097 version "0103" (as shot).
    ColorBalance0103,
    /// Source 4: the D50 scales of the model table.
    Table,
    /// Source 5: the generic scales.
    Generic,
}

impl WbSource {
    /// True for the sources that hold the camera's as-shot balance.
    pub(crate) fn as_shot(self) -> bool {
        !matches!(self, WbSource::Table | WbSource::Generic)
    }
}

/// Which rule of R-NEF-40 gave the crop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CropSource {
    /// Rule 1: maker-note tag 0x0045 CropArea.
    CropArea,
    /// Rule 2: the largest JPEG preview, centered.
    Preview,
    /// Rule 3: the whole raster.
    Whole,
}

/// Most SubIFD offsets examined in one IFD. The specification measured 1 to
/// 6; the bound only limits the work on a hostile file.
const MAX_SUB_IFDS: usize = 256;

/// The maker note (R-NEF-13, R-NEF-14).
#[derive(Clone, Debug)]
pub(super) struct Maker<'a> {
    /// The first maker-note IFD.
    pub ifd: Ifd<'a>,
    /// File offset of the maker-note origin: maker-note byte 10, the first
    /// byte of its TIFF header. Every offset inside the maker note counts
    /// from here.
    pub origin: u64,
    /// The maker-note byte order (R-NEF-5).
    pub le: bool,
}

/// The parsed container.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// The file byte order (R-NEF-5).
    pub le: bool,
    /// IFD0 and the IFDs of its chain (R-NEF-11). Empty when IFD0 is not a
    /// valid IFD.
    pub chain: Vec<Ifd<'a>>,
    /// The raw IFD, when a SubIFD qualifies (R-NEF-15).
    pub raw: Option<Ifd<'a>>,
    /// The maker note, when it is valid (R-NEF-14).
    pub maker: Option<Maker<'a>>,
}

/// R-NEF-58 and R-NEF-4: check the size and the header and return the file
/// byte order (true for little-endian) and the IFD0 offset. A file shorter
/// than 8 bytes fails; any other start than 4D 4D 00 2A or 49 49 2A 00 is
/// "not a NEF file".
pub(super) fn header(data: &[u8]) -> NefResult<(bool, u32)> {
    // R-NEF-58: shorter than 8 bytes.
    if data.len() < 8 {
        return Err(fail("file shorter than 8 bytes"));
    }
    // R-NEF-4: only the two TIFF starts.
    let le = match data[..4] {
        [0x4D, 0x4D, 0x00, 0x2A] => false,
        [0x49, 0x49, 0x2A, 0x00] => true,
        _ => return Err(fail("not a NEF file")),
    };
    // R-NEF-11: IFD0 at the offset in header bytes 4 to 7.
    Ok((le, read32(data, 4, le).unwrap_or(0)))
}

/// Parse the header, the IFD chain, the raw IFD and the maker note. An
/// error means "not a NEF file" (R-NEF-4) or a file shorter than 8 bytes
/// (R-NEF-58): there is no preview then.
pub(super) fn open(data: &[u8]) -> NefResult<Container<'_>> {
    let (le, first) = header(data)?;
    let chain = Ifd::chain(data, le, first);
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

/// The valid SubIFDs of an IFD, in the order of the offsets of tag 0x014A
/// (types LONG or IFD, R-NEF-8).
pub(super) fn sub_ifds<'a>(data: &'a [u8], le: bool, d: &Ifd<'a>) -> Vec<Ifd<'a>> {
    d.ints(tag::SUB_IFDS)
        .ok()
        .flatten()
        .unwrap_or_default()
        .into_iter()
        .take(MAX_SUB_IFDS)
        .filter_map(|off| Ifd::read(data, le, u64::from(off), 0))
        .collect()
}

/// R-NEF-15: the first SubIFD of IFD0, in the order of the offsets, that is
/// a valid IFD, has NewSubfileType 0 or no NewSubfileType, has
/// PhotometricInterpretation 32803 and the tags 0x0100, 0x0101, 0x0103,
/// 0x0111 and 0x0117.
fn raw_ifd<'a>(data: &'a [u8], le: bool, ifd0: &Ifd<'a>) -> Option<Ifd<'a>> {
    const NEEDED: [u16; 5] = [
        tag::IMAGE_WIDTH,
        tag::IMAGE_LENGTH,
        tag::COMPRESSION,
        tag::STRIP_OFFSETS,
        tag::STRIP_BYTE_COUNTS,
    ];
    sub_ifds(data, le, ifd0).into_iter().find(|d| {
        let subfile = if d.has(tag::NEW_SUBFILE_TYPE) {
            d.int(tag::NEW_SUBFILE_TYPE).ok().flatten()
        } else {
            Some(0)
        };
        subfile == Some(0)
            && matches!(d.int(tag::PHOTOMETRIC), Ok(Some(32803)))
            && NEEDED.iter().all(|&t| d.has(t))
    })
}

/// R-NEF-13 and R-NEF-14: the maker note of Exif tag 0x927C of the Exif
/// IFD (IFD0 tag 0x8769). Valid when it starts with "Nikon" and a zero
/// byte, its bytes 10 to 17 are a TIFF header, and the IFD at the header
/// offset (counted from the origin, maker-note byte 10) is valid.
fn maker_note<'a>(data: &'a [u8], le: bool, ifd0: &Ifd<'a>) -> Option<Maker<'a>> {
    let exif_at = ifd0.int(tag::EXIF_IFD).ok().flatten()?;
    let exif = Ifd::read(data, le, u64::from(exif_at), 0)?;
    let start = exif.entry(tag::MAKER_NOTE)?.at;
    // R-NEF-61: the maker-note value lies inside the file.
    let note = exif.bytes(tag::MAKER_NOTE).ok().flatten()?;
    if note.len() < 18 || note[..6] != *b"Nikon\0" {
        return None;
    }
    let mle = match note[10..14] {
        [0x4D, 0x4D, 0x00, 0x2A] => false,
        [0x49, 0x49, 0x2A, 0x00] => true,
        _ => return None,
    };
    let origin = start.checked_add(10)?;
    let first = read32(data, origin + 4, mle)?;
    let ifd = Ifd::read(data, mle, origin + u64::from(first), origin)?;
    Some(Maker {
        ifd,
        origin,
        le: mle,
    })
}

impl<'a> Container<'a> {
    /// IFD0, when valid.
    pub(super) fn ifd0(&self) -> Option<&Ifd<'a>> {
        self.chain.first()
    }

    /// The IFD0 Model string after R-NEF-10, empty when absent.
    pub(super) fn model(&self) -> Vec<u8> {
        self.ifd0()
            .and_then(|d| d.ascii(tag::MODEL).ok().flatten())
            .unwrap_or_default()
            .to_vec()
    }

    /// R-NEF-50: the IFD0 Orientation, 1 when absent or outside 1 to 8.
    /// The Orientation of the raw IFD is never used.
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().map(|d| d.int(tag::ORIENTATION)) {
            Some(Ok(Some(v @ 1..=8))) => v as u8,
            _ => 1,
        }
    }

    /// R-NEF-20: NEFCompression from the first value of maker-note tag
    /// 0x0093 when it is readable as an integer, else from bytes 10 and 11
    /// (maker-note byte order) of maker-note tag 0x0051 when that tag has
    /// at least 12 bytes, else absent. Absent without a valid maker note
    /// (R-NEF-14).
    pub(super) fn nef_compression(&self) -> NefResult<Option<u32>> {
        let Some(m) = &self.maker else {
            return Ok(None);
        };
        if let Some(v) = m.ifd.int(tag::NEF_COMPRESSION)? {
            return Ok(Some(v));
        }
        Ok(m.ifd
            .bytes(tag::MAKER_NOTES_0X51)?
            .filter(|b| b.len() >= 12)
            .and_then(|b| u16_at(b, 10, m.le))
            .map(u32::from))
    }

    /// The value bytes of maker-note tag 0x0096 (linearization tag), when
    /// present and at least 12 bytes long (R-NEF-3, R-NEF-16).
    pub(super) fn linearization(&self) -> NefResult<Option<&'a [u8]>> {
        let Some(m) = &self.maker else {
            return Ok(None);
        };
        Ok(m.ifd.bytes(tag::LINEARIZATION)?.filter(|b| b.len() >= 12))
    }
}

/// The raw-IFD numbers that classification and the plan share.
struct RawFacts {
    compression: Option<u32>,
    bits: Option<u32>,
    size: Option<(u64, u64)>,
    /// The sum of all StripByteCounts values (64-bit, R-NEF-58).
    total: u64,
}

fn raw_facts(raw: &Ifd<'_>) -> NefResult<RawFacts> {
    let size = match (raw.int(tag::IMAGE_WIDTH)?, raw.int(tag::IMAGE_LENGTH)?) {
        (Some(w), Some(h)) => Some((u64::from(w), u64::from(h))),
        _ => None,
    };
    Ok(RawFacts {
        compression: raw.int(tag::COMPRESSION)?,
        bits: raw.int(tag::BITS_PER_SAMPLE)?,
        size,
        total: raw
            .ints(tag::STRIP_BYTE_COUNTS)?
            .unwrap_or_default()
            .iter()
            .map(|&v| u64::from(v))
            .sum(),
    })
}

/// True when `total` is an exact multiple of `h` and the row stride
/// total / h lies from ceil(n * W / 8) to ceil(n * W / 8) + 15 (rules 9 and
/// 10 of R-NEF-3, R-NEF-27).
fn packed_stride_ok(total: u128, w: u128, h: u128, n: u128) -> bool {
    if h == 0 || !total.is_multiple_of(h) {
        return false;
    }
    let need = (n * w).div_ceil(8);
    let stride = total / h;
    stride >= need && stride <= need + 15
}

/// Classify the file by the rules of R-NEF-3, in this order. A file that
/// matches no rule is X7.
pub(super) fn classify(c: &Container<'_>) -> NefResult<Class> {
    use Class::{OutOfScope, Sensor};
    let model = c.model();
    // Rule 1.
    if model == b"NIKON D1X" {
        return Ok(OutOfScope(Relative::X4));
    }
    // Rule 2: "E" followed by four digits 0 to 9.
    if model.len() == 5 && model[0] == b'E' && model[1..].iter().all(u8::is_ascii_digit) {
        return Ok(OutOfScope(Relative::X5));
    }
    // Rule 3: no raw IFD (R-NEF-15).
    let Some(raw) = &c.raw else {
        return Ok(OutOfScope(Relative::X6));
    };
    let f = raw_facts(raw)?;
    let bits = f.bits;
    // The sizes in 128-bit arithmetic: W * H of two 32-bit values times a
    // byte count cannot overflow (R-NEF-58: at least 64 bits).
    let total = u128::from(f.total);
    let size = f.size.map(|(w, h)| (u128::from(w), u128::from(h)));
    if f.compression == Some(1) {
        if let Some((w, h)) = size {
            // Rule 4.
            if matches!(bits, Some(12 | 14)) && total >= 2 * w * h {
                return Ok(Sensor(Variant::V1));
            }
            // Rule 5.
            if bits == Some(12) && w % 2 == 0 && total == 3 * w * h / 2 {
                return Ok(Sensor(Variant::V2));
            }
        }
        return Ok(OutOfScope(Relative::X7));
    }
    if f.compression != Some(34713) {
        return Ok(OutOfScope(Relative::X7));
    }
    // Without a valid maker note NEFCompression is absent and there is no
    // linearization tag (R-NEF-14), so rules 6 to 14 cannot match.
    let nefc = c.nef_compression()?;
    // Rule 6.
    if matches!(nefc, Some(13 | 14)) {
        return Ok(OutOfScope(Relative::X1));
    }
    // Rule 7.
    if nefc == Some(8) {
        return Ok(OutOfScope(Relative::X2));
    }
    if let Some((w, h)) = size {
        // Rule 8.
        if nefc == Some(2) && bits == Some(12) && total == 16 * w.div_ceil(10) * h {
            return Ok(Sensor(Variant::V3));
        }
        // Rule 9.
        if nefc == Some(6) && bits == Some(12) && packed_stride_ok(total, w, h, 12) {
            return Ok(Sensor(Variant::V4));
        }
        // Rule 10.
        if nefc == Some(10) && bits == Some(14) && packed_stride_ok(total, w, h, 14) {
            return Ok(Sensor(Variant::V5));
        }
    }
    // Rules 11 to 13: the first two bytes of tag 0x0096, at least 12 bytes
    // long.
    let lin = c.linearization()?;
    let version = lin.map(|b| [b[0], b[1]]);
    // Rule 11.
    if version == Some([0x46, 0x30])
        && matches!(nefc, None | Some(3))
        && matches!(bits, Some(12 | 14))
    {
        return Ok(Sensor(Variant::V6));
    }
    // Rule 12.
    if version == Some([0x44, 0x10]) && matches!(nefc, None | Some(1)) && bits == Some(12) {
        return Ok(Sensor(Variant::V7));
    }
    // Rule 13.
    if matches!(version, Some([0x44, 0x20] | [0x44, 0x40]))
        && matches!(nefc, None | Some(4))
        && matches!(bits, Some(12 | 14))
    {
        return Ok(Sensor(Variant::V8));
    }
    // Rule 14: no tag 0x0096 at all (a tag shorter than 12 bytes is
    // present and matches no rule: X7). Both use the preview path.
    let has_lin = c
        .maker
        .as_ref()
        .is_some_and(|m| m.ifd.has(tag::LINEARIZATION));
    if nefc == Some(1) && !has_lin {
        return Ok(OutOfScope(Relative::X3));
    }
    Ok(OutOfScope(Relative::X7))
}

/// Where the sensor data lie in the file.
#[derive(Clone, Debug)]
pub(crate) enum Layout {
    /// V1, V2: strips of `rows_per_strip` rows of `row_bytes` bytes, each
    /// at its own offset (R-NEF-22).
    Strips {
        /// File offset of each strip.
        offsets: Vec<usize>,
        /// Rows in each strip except possibly the last.
        rows_per_strip: usize,
        /// Row stride R.
        row_bytes: usize,
    },
    /// V3 to V8: one byte sequence (R-NEF-23).
    Stream {
        /// File offset of the first byte.
        offset: usize,
        /// Bytes in the sequence.
        len: usize,
        /// Row stride R (V3, V4, V5); 0 for the bit stream of V6 to V8.
        row_bytes: usize,
    },
}

/// The Huffman coding of V6, V7 and V8.
#[derive(Clone, Debug)]
pub(crate) struct Coding {
    /// The code table (R-NEF-30).
    pub table: &'static Spec,
    /// Start predictors P0 to P3 (bytes 2 to 9 of tag 0x0096).
    pub predictors: [i32; 4],
    /// The curve C of V7 and V8 (R-NEF-34, R-NEF-35); `None` for V6.
    pub curve: Option<Vec<u16>>,
}

impl Coding {
    /// Code values must be less than this (R-NEF-33): 2^bps for V6, the
    /// curve length L for V7 and V8.
    pub(crate) fn limit(&self, bits: u32) -> i32 {
        match &self.curve {
            Some(c) => c.len() as i32,
            None => 1 << bits,
        }
    }
}

/// Everything needed to develop the sensor data.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    /// The storage variant.
    pub variant: Variant,
    /// Stored raster width W.
    pub width: usize,
    /// Stored raster height H.
    pub height: usize,
    /// BitsPerSample: 12 or 14.
    pub bits: u32,
    /// True for a little-endian file (V1 samples, R-NEF-25).
    pub le: bool,
    /// The final image area (R-NEF-40).
    pub crop: Rect,
    /// The rule that gave the crop.
    pub crop_source: CropSource,
    /// IFD0 orientation 1 to 8 (R-NEF-50).
    pub orientation: u8,
    /// Colors of CFA positions 0 to 3, origin at stored pixel (0, 0)
    /// (R-NEF-18, R-NEF-21).
    pub cfa: [u8; 4],
    /// White level w = 2^bps - 1 (R-NEF-36).
    pub white: u32,
    /// Tag 0x003D values of CFA positions 0 to 3 on the 14-bit scale when
    /// R-NEF-37 accepts them, else 0. The black level of position i is
    /// `black14[i] / 2^(14 - bps)`.
    pub black14: [u32; 4],
    /// WB multipliers mR, mG, mB (R-NEF-38).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// The camera-to-sRGB matrix, rows divided by their sums (R-NEF-42).
    pub matrix: [[f64; 3]; 3],
    /// True when the matrix came from the model table (R-NEF-41).
    pub matrix_from_table: bool,
    /// Where the sensor data lie.
    pub layout: Layout,
    /// The Huffman coding of V6 to V8.
    pub coding: Option<Coding>,
    /// The IFD0 Model string.
    pub model: String,
}

impl Plan {
    /// The black level of CFA position `i` as the real number of R-NEF-37.
    pub(crate) fn black(&self, i: usize) -> f64 {
        f64::from(self.black14[i]) / f64::from(1u32 << (14 - self.bits))
    }
}

/// Largest W * H accepted (R-NEF-59).
const MAX_PIXELS: u64 = 1 << 27;

fn require<T>(v: Option<T>, what: &str) -> NefResult<T> {
    // R-NEF-16: a required tag absent, with count 0 or unreadable uses the
    // preview path.
    v.ok_or_else(|| fail(format!("required tag {what} missing")))
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check. `preview`
/// is the stored size of the largest JPEG candidate with a parsable SOF
/// segment (R-NEF-40 rule 2).
pub(super) fn plan(
    c: &Container<'_>,
    variant: Variant,
    preview: Option<(u32, u32)>,
) -> NefResult<Plan> {
    // R-NEF-58: a file longer than 2^32 - 1 bytes fails.
    let data_len = c.data.len() as u64;
    if data_len > u64::from(u32::MAX) {
        return Err(fail("file longer than 2^32 - 1 bytes"));
    }
    let raw = c.raw.as_ref().ok_or_else(|| fail("no raw IFD"))?;
    let ifd0 = c.ifd0().ok_or_else(|| fail("no IFD0"))?;
    // R-NEF-16: the required tags of the raw IFD and IFD0.
    let w = require(raw.int(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(raw.int(tag::IMAGE_LENGTH)?, "ImageLength")?;
    let bits = require(raw.int(tag::BITS_PER_SAMPLE)?, "BitsPerSample")?;
    require(raw.int(tag::COMPRESSION)?, "Compression")?;
    require(raw.int(tag::PHOTOMETRIC)?, "PhotometricInterpretation")?;
    require(ifd0.int(tag::SUB_IFDS)?, "SubIFDs")?;
    // R-NEF-59: limits on W, H and W * H.
    if w == 0 || h == 0 || w > 65535 || h > 65535 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    // Every variant has 12 or 14 bits (R-NEF-3); the scales below rely on it.
    if bits != 12 && bits != 14 {
        return Err(fail("BitsPerSample is neither 12 nor 14"));
    }
    let (wu, hu) = (w as usize, h as usize);
    // R-NEF-65: two columns and two rows for the start predictors.
    if variant.huffman() && (wu < 2 || hu < 2) {
        return Err(fail("a Huffman raster needs at least 2 x 2 pixels"));
    }
    let model_bytes = c.model();
    let model = String::from_utf8_lossy(&model_bytes).into_owned();
    let orientation = c.orientation();
    let cfa = cfa_pattern(raw)?;
    let layout = layout(raw, variant, wu, hu, data_len)?;
    let coding = if variant.huffman() {
        Some(coding(c, variant, bits)?)
    } else {
        None
    };
    // R-NEF-36: the white level.
    let white = (1u32 << bits) - 1;
    let black14 = black_levels(c, bits)?;
    let (wb, wb_source) = multipliers(c, &model_bytes)?;
    let (matrix, matrix_from_table) = color::matrix(&model_bytes);
    let (crop, crop_source) = crop(c, wu, hu, preview)?;
    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        bits,
        le: c.le,
        crop,
        crop_source,
        orientation,
        cfa,
        white,
        black14,
        wb,
        wb_source,
        matrix,
        matrix_from_table,
        layout,
        coding,
        model,
    })
}

/// R-NEF-18: a 2 x 2 repeat (or no dimension tag) and a pattern of one 0,
/// two 1 and one 2 (0, 1, 1, 2 when CFAPattern is absent); otherwise the
/// preview path. R-NEF-21: the pattern origin is stored pixel (0, 0).
fn cfa_pattern(raw: &Ifd<'_>) -> NefResult<[u8; 4]> {
    if raw.has(tag::CFA_REPEAT_PATTERN_DIM)
        && raw.ints(tag::CFA_REPEAT_PATTERN_DIM)?.as_deref() != Some(&[2, 2])
    {
        return Err(fail("CFA repeat pattern is not 2 x 2"));
    }
    if !raw.has(tag::CFA_PATTERN) {
        return Ok([0, 1, 1, 2]);
    }
    // BYTE in the table; any integer type is accepted (R-NEF-8), and the
    // UNDEFINED bytes of some writers too.
    let values: Vec<u32> = match raw.entry(tag::CFA_PATTERN).map(|e| e.typ) {
        Some(7) => raw
            .bytes(tag::CFA_PATTERN)?
            .unwrap_or_default()
            .iter()
            .map(|&b| u32::from(b))
            .collect(),
        _ => raw.ints(tag::CFA_PATTERN)?.unwrap_or_default(),
    };
    let count = |c: u32| values.iter().filter(|&&x| x == c).count();
    if values.len() != 4 || count(0) != 1 || count(1) != 2 || count(2) != 1 {
        return Err(fail("CFA pattern is not a Bayer pattern"));
    }
    Ok([0, 1, 2, 3].map(|i| values[i] as u8))
}

/// The strip layout of the variant (R-NEF-19, R-NEF-22, R-NEF-23, R-NEF-27,
/// R-NEF-61 to R-NEF-63).
fn layout(raw: &Ifd<'_>, variant: Variant, w: usize, h: usize, data_len: u64) -> NefResult<Layout> {
    let offsets = require(raw.ints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(raw.ints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    if offsets.len() != counts.len() {
        // R-NEF-62 case 1 (V1, V2); for V3 to V8 the strips cannot be
        // joined either without a length for each (R-NEF-23).
        return Err(fail("StripOffsets and StripByteCounts differ in count"));
    }
    // R-NEF-61: every strip lies inside the file (64-bit sums, R-NEF-58).
    for (&off, &count) in offsets.iter().zip(&counts) {
        if u64::from(off) + u64::from(count) > data_len {
            return Err(fail("a strip lies outside the file"));
        }
    }
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    match variant {
        Variant::V1 | Variant::V2 => {
            // The row stride R of the stride table (W is even for V2).
            let row_bytes = match variant {
                Variant::V1 => 2 * w,
                _ => 3 * w / 2,
            };
            // R-NEF-19: RowsPerStrip is H when absent, 0 or greater than H.
            let rps = match raw.int(tag::ROWS_PER_STRIP)? {
                Some(r) if r > 0 && (r as usize) <= h => r as usize,
                _ => h,
            };
            // R-NEF-62 case 2: ceil(H / RowsPerStrip) strips.
            if offsets.len() != h.div_ceil(rps) {
                return Err(fail("strip count does not match RowsPerStrip"));
            }
            for (s, &count) in counts.iter().enumerate() {
                // R-NEF-22: the last strip can hold fewer rows.
                let rows = rps.min(h - s * rps);
                // R-NEF-62 case 3: each strip holds R times its rows.
                if u64::from(count) < row_bytes as u64 * rows as u64 {
                    return Err(fail("a strip is shorter than its rows"));
                }
            }
            Ok(Layout::Strips {
                offsets: offsets.iter().map(|&o| o as usize).collect(),
                rows_per_strip: rps,
                row_bytes,
            })
        }
        _ => {
            // R-NEF-23: one strip, or strips that each start where the
            // previous one ends.
            for k in 1..offsets.len() {
                if u64::from(offsets[k]) != u64::from(offsets[k - 1]) + u64::from(counts[k - 1]) {
                    return Err(fail("the strips are not one byte sequence"));
                }
            }
            let offset = *offsets.first().ok_or_else(|| fail("no strip"))? as usize;
            let len = usize::try_from(total).map_err(|_| fail("strip size"))?;
            let row_bytes = match variant {
                // R-NEF-45: 16 * ceil(W / 10) bytes per row; rule 8 of
                // R-NEF-3 fixed the total at H rows (R-NEF-63).
                Variant::V3 => 16 * w.div_ceil(10),
                // R-NEF-27: total / H, exact and inside the range of rules 9
                // and 10, already checked by the classification.
                Variant::V4 | Variant::V5 => {
                    let n = if variant == Variant::V4 { 12 } else { 14 };
                    if !packed_stride_ok(u128::from(total), w as u128, h as u128, n) {
                        return Err(fail("row stride outside the specified range"));
                    }
                    len / h
                }
                _ => 0,
            };
            if row_bytes as u64 * h as u64 > total {
                return Err(fail("the strip is shorter than its rows"));
            }
            Ok(Layout::Stream {
                offset,
                len,
                row_bytes,
            })
        }
    }
}

/// The table, start predictors and curve of V6, V7 and V8 (R-NEF-16,
/// R-NEF-30, R-NEF-34, R-NEF-35).
fn coding(c: &Container<'_>, variant: Variant, bits: u32) -> NefResult<Coding> {
    let m = c.maker.as_ref().ok_or_else(|| fail("no maker note"))?;
    // R-NEF-16: tag 0x0096 is required and at least 12 bytes long.
    let lin = c
        .linearization()?
        .ok_or_else(|| fail("required tag 0x0096 missing or too short"))?;
    let le = m.le;
    let at = |i: usize| u16_at(lin, i, le);
    let mut predictors = [0i32; 4];
    for (k, p) in predictors.iter_mut().enumerate() {
        *p = i32::from(at(2 + 2 * k).ok_or_else(|| fail("tag 0x0096 too short"))?);
    }
    // R-NEF-30: the code table of the variant and bps.
    let table: &'static Spec = match (variant, bits) {
        (Variant::V6, 12) => &huffman::L12,
        (Variant::V6, _) => &huffman::L14,
        (Variant::V7, _) => &huffman::Y12,
        (_, 12) => &huffman::Y12,
        _ => &huffman::Y14,
    };
    let nc = usize::from(at(10).ok_or_else(|| fail("tag 0x0096 too short"))?);
    let max = (1u32 << bits) - 1;
    let curve = match variant {
        Variant::V6 => None,
        Variant::V7 => {
            // R-NEF-34: 2 <= NC <= 2^bps, 12 + 2 * NC bytes, C
            // non-decreasing and at most 2^bps - 1; L = NC.
            if nc < 2 || nc > (1usize << bits) || lin.len() < 12 + 2 * nc {
                return Err(fail("V7 curve length"));
            }
            let c: Vec<u16> = (0..nc).filter_map(|i| at(12 + 2 * i)).collect();
            if c.len() != nc
                || c.windows(2).any(|p| p[1] < p[0])
                || c.iter().any(|&v| u32::from(v) > max)
            {
                return Err(fail("V7 curve is not valid"));
            }
            Some(c)
        }
        _ => {
            // R-NEF-35: NC = 257, at least 526 bytes, K non-decreasing and
            // at most 2^bps - 1.
            if nc != 257 || lin.len() < 526 {
                return Err(fail("V8 curve length"));
            }
            let k: Vec<u16> = (0..257).filter_map(|i| at(12 + 2 * i)).collect();
            let Ok(k) = <[u16; 257]>::try_from(k) else {
                return Err(fail("V8 knots missing"));
            };
            if k.windows(2).any(|p| p[1] < p[0]) || k.iter().any(|&v| u32::from(v) > max) {
                return Err(fail("V8 knots are not valid"));
            }
            // The step S: 2^bps / 256 for version 0x44 0x20, 2^bps / 1024
            // for 0x44 0x40.
            let step = if lin[1] == 0x40 {
                (1usize << bits) / 1024
            } else {
                (1usize << bits) / 256
            };
            Some(expand(&k, step))
        }
    };
    Ok(Coding {
        table,
        predictors,
        curve,
    })
}

/// The V8 curve from 257 knots K and the step S (section Curves): L =
/// 256 S + 1 entries, C(i) = K(j) + floor((K(j + 1) - K(j)) f / S) with
/// j = floor(i / S) and f = i mod S, and C(256 S) = K(256).
pub(crate) fn expand(k: &[u16; 257], step: usize) -> Vec<u16> {
    let step = step.max(1);
    let s = step as i64;
    let mut c = Vec::with_capacity(256 * step + 1);
    for i in 0..256 * step {
        let (j, f) = (i / step, (i % step) as i64);
        let (a, b) = (i64::from(k[j]), i64::from(k[j + 1]));
        // floor() of the specification; R-NEF-35 makes K non-decreasing, so
        // the result lies from K(j) to K(j + 1).
        c.push((a + ((b - a) * f).div_euclid(s)) as u16);
    }
    c.push(k[256]);
    c
}

/// R-NEF-37: tag 0x003D on the 14-bit scale per CFA position, accepted
/// when it has 4 values and each scaled value BlackLevel[i] / 2^(14 - bps)
/// is less than w - 255; otherwise 0 for every position.
fn black_levels(c: &Container<'_>, bits: u32) -> NefResult<[u32; 4]> {
    let Some(m) = &c.maker else {
        return Ok([0; 4]);
    };
    let Some(v) = m.ifd.ints(tag::BLACK_LEVEL)? else {
        return Ok([0; 4]);
    };
    let scale = f64::from(1u32 << (14 - bits));
    let w = f64::from((1u32 << bits) - 1);
    if v.len() == 4 && v.iter().all(|&b| f64::from(b) / scale < w - 255.0) {
        Ok([v[0], v[1], v[2], v[3]])
    } else {
        Ok([0; 4])
    }
}

/// True for a multiplier from 0.25 to 8 (R-NEF-38).
fn in_range(m: f64) -> bool {
    (0.25..=8.0).contains(&m)
}

/// R-NEF-38: the WB multipliers from the first valid source.
fn multipliers(c: &Container<'_>, model: &[u8]) -> NefResult<([f64; 3], WbSource)> {
    if let Some(m) = &c.maker {
        // Source 1: tag 0x000C with at least 2 rationals whose first two
        // denominators are not 0 (R-NEF-17), both from 0.25 to 8.
        if let Some(r) = m.ifd.rationals(tag::WB_RB_LEVELS)? {
            if r.len() >= 2 && r[0].1 != 0 && r[1].1 != 0 {
                let mr = f64::from(r[0].0) / f64::from(r[0].1);
                let mb = f64::from(r[1].0) / f64::from(r[1].1);
                if in_range(mr) && in_range(mb) {
                    return Ok(([mr, 1.0, mb], WbSource::RbLevels));
                }
            }
        }
        // Sources 2 and 3: tag 0x0097 versions "0102" and "0103".
        if let Some(b) = m.ifd.bytes(tag::COLOR_BALANCE)? {
            let le = m.le;
            let levels = if b.starts_with(b"0102") && b.len() >= 18 {
                // R, G1, G2, B at bytes 10, 12, 14, 16.
                let v = [10, 12, 14, 16].map(|i| f64::from(u16_at(b, i, le).unwrap_or(0)));
                Some((v[0], v[1], v[2], v[3], WbSource::ColorBalance0102))
            } else if b.starts_with(b"0103") && b.len() >= 28 {
                // R, G1, B, G2 at bytes 20, 22, 24, 26.
                let v = [20, 22, 24, 26].map(|i| f64::from(u16_at(b, i, le).unwrap_or(0)));
                Some((v[0], v[1], v[3], v[2], WbSource::ColorBalance0103))
            } else {
                None
            };
            if let Some((r, g1, g2, bl, source)) = levels {
                let g = (g1 + g2) / 2.0;
                // R-NEF-67: G > 0 before the division.
                if g > 0.0 && in_range(r / g) && in_range(bl / g) {
                    return Ok(([r / g, 1.0, bl / g], source));
                }
            }
        }
    }
    // Source 4: the D50 scales of the model table.
    if let Some((r, b)) = color::table_wb(model) {
        return Ok(([r, 1.0, b], WbSource::Table));
    }
    // Source 5: the generic scales.
    let (r, b) = color::GENERIC_WB;
    Ok(([r, 1.0, b], WbSource::Generic))
}

/// R-NEF-40: the final image area.
fn crop(
    c: &Container<'_>,
    w: usize,
    h: usize,
    preview: Option<(u32, u32)>,
) -> NefResult<(Rect, CropSource)> {
    // Rule 1: tag 0x0045 with 4 values, width and height at least 16,
    // inside the raster.
    if let Some(m) = &c.maker {
        if let Some(v) = m.ifd.ints(tag::CROP_AREA)? {
            if v.len() == 4 {
                let [l, t, cw, ch] = [0, 1, 2, 3].map(|i| u64::from(v[i]));
                if cw >= 16 && ch >= 16 && l + cw <= w as u64 && t + ch <= h as u64 {
                    let r = Rect {
                        x: l as usize,
                        y: t as usize,
                        w: cw as usize,
                        h: ch as usize,
                    };
                    return Ok((r, CropSource::CropArea));
                }
            }
        }
    }
    // Rule 2: the largest JPEG candidate with 0.9 * W <= PW <= W and
    // 0.9 * H <= PH <= H, centered.
    if let Some((pw, ph)) = preview {
        let (pw, ph) = (pw as usize, ph as usize);
        if 10 * pw >= 9 * w && pw <= w && 10 * ph >= 9 * h && ph <= h {
            let r = Rect {
                x: (w - pw) / 2,
                y: (h - ph) / 2,
                w: pw,
                h: ph,
            };
            return Ok((r, CropSource::Preview));
        }
    }
    // Rule 3: the whole raster.
    Ok((Rect { x: 0, y: 0, w, h }, CropSource::Whole))
}

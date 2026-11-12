//! Classification of an ARW file (R-ARW-3) and the decode plan: sizes,
//! layout, levels, white balance, color matrix and orientation, all
//! validated before any large allocation.

use super::color;
use super::ifd::{self, Dir};
use super::{fail, ArwResult};

/// Tag numbers of the specification's Metadata tags table.
mod tag {
    pub const NEW_SUBFILE_TYPE: u16 = 0x00FE;
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const COMPRESSION: u16 = 0x0103;
    pub const PHOTOMETRIC: u16 = 0x0106;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const SAMPLES_PER_PIXEL: u16 = 0x0115;
    pub const ROWS_PER_STRIP: u16 = 0x0116;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const SUB_IFDS: u16 = 0x014A;
    pub const TILE_WIDTH: u16 = 0x0142;
    pub const TILE_LENGTH: u16 = 0x0143;
    pub const TILE_OFFSETS: u16 = 0x0144;
    pub const TILE_BYTE_COUNTS: u16 = 0x0145;
    pub const YCBCR_COEFFICIENTS: u16 = 0x0211;
    pub const YCBCR_SUBSAMPLING: u16 = 0x0212;
    pub const REFERENCE_BLACK_WHITE: u16 = 0x0214;
    pub const SONY_TONE_CURVE: u16 = 0x7010;
    pub const SONY_RAW_IMAGE_SIZE: u16 = 0x7038;
    pub const WB_CORRECTION: u16 = 0x7039;
    pub const BLACK_LEVEL: u16 = 0x7310;
    pub const WB_RGGB_LEVELS: u16 = 0x7313;
    pub const CFA_REPEAT_PATTERN_DIM: u16 = 0x828D;
    pub const CFA_PATTERN: u16 = 0x828E;
    pub const WHITE_LEVEL: u16 = 0xC61D;
    pub const DEFAULT_CROP_ORIGIN: u16 = 0xC61F;
    pub const DEFAULT_CROP_SIZE: u16 = 0xC620;
}

/// The storage variants this decoder develops from sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Uncompressed, 16-bit little-endian samples.
    V1,
    /// 12-bit packed (DSLR-A850 and A900).
    V2,
    /// 128-bit lossy blocks with a tone curve.
    V3,
    /// Lossless JPEG tiles, four CFA components.
    V4,
    /// Lossless JPEG tiles, YCbCr.
    V5,
}

/// The out-of-scope relatives that use the preview path (R-ARW-2). The
/// sixth, X6 (big-endian files), is recognized from the header before
/// classification: [`open`] returns `Ok(None)` for it (R-ARW-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// SRF: IFD0 has no SubIFDs tag.
    X1,
    /// SR2 (DSC-R1).
    X2,
    /// ARW 1.0: no SubIFD is a valid raw IFD.
    X3,
    /// ARW 2.x variable-length coding.
    X4,
    /// Any other combination.
    X5,
}

/// The result of classification (R-ARW-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A relative that uses the preview path.
    OutOfScope(Relative),
}

/// Most SubIFD offsets examined for the raw IFD.
const MAX_SUB_IFDS: usize = 256;

/// The parsed container: the IFD chain and the raw IFD.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// IFD0, IFD1, IFD2 and later IFDs of the chain.
    pub chain: Vec<Dir<'a>>,
    /// The raw IFD, when one qualifies (R-ARW-13).
    pub raw: Option<Dir<'a>>,
    /// True when IFD0 has tag 0x014A.
    pub has_sub_ifds: bool,
}

/// Parse the header and the IFD chain. `Ok(None)` for a file that is not
/// a little-endian TIFF (X6 and other data).
pub(super) fn open(data: &[u8]) -> ArwResult<Option<Container<'_>>> {
    let Some(first) = ifd::header(data)? else {
        return Ok(None);
    };
    let chain = ifd::chain(data, first);
    let Some(ifd0) = chain.first() else {
        return Err(fail("IFD0 is not a valid IFD"));
    };
    let has_sub_ifds = ifd0.has(tag::SUB_IFDS);
    // R-ARW-13: the first SubIFD, in the order of the offsets, that is a
    // valid IFD with NewSubfileType 0 or without that tag. At most the
    // first 256 offsets are examined (real files have one); the document
    // sets no bound, and each candidate costs an IFD parse.
    let mut raw = None;
    for off in ifd0
        .uints(tag::SUB_IFDS)?
        .unwrap_or_default()
        .into_iter()
        .take(MAX_SUB_IFDS)
    {
        let Some(d) = ifd::dir(data, u64::from(off)) else {
            continue;
        };
        if d.uint(tag::NEW_SUBFILE_TYPE)?.unwrap_or(0) == 0 {
            raw = Some(d);
            break;
        }
    }
    Ok(Some(Container {
        data,
        chain,
        raw,
        has_sub_ifds,
    }))
}

impl Container<'_> {
    /// IFD0.
    pub(super) fn ifd0(&self) -> &Dir<'_> {
        &self.chain[0]
    }

    /// The IFD0 Model string, trimmed (R-ARW-10).
    pub(super) fn model(&self) -> Vec<u8> {
        self.ifd0()
            .ascii(tag::MODEL)
            .ok()
            .flatten()
            .unwrap_or_default()
            .to_vec()
    }

    /// The IFD0 Orientation: 1 when absent or outside 1 to 8 (R-ARW-57).
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().uint(tag::ORIENTATION) {
            Ok(Some(v @ 1..=8)) => v as u8,
            _ => 1,
        }
    }
}

/// Classify the file by the rules of R-ARW-3, in order. A file that
/// matches no rule is X5.
pub(super) fn classify(c: &Container<'_>) -> ArwResult<Class> {
    use Class::{OutOfScope, Sensor};
    // Rule 1: IFD0 has no tag 0x014A.
    if !c.has_sub_ifds {
        return Ok(OutOfScope(Relative::X1));
    }
    // Rule 2: no SubIFD qualifies as raw IFD (R-ARW-13).
    let Some(raw) = &c.raw else {
        return Ok(OutOfScope(Relative::X3));
    };
    // Rule 3: Model "DSC-R1", compared by R-ARW-10.
    if c.model() == b"DSC-R1" {
        return Ok(OutOfScope(Relative::X2));
    }
    let compression = raw.uint(tag::COMPRESSION)?;
    let photometric = raw.uint(tag::PHOTOMETRIC)?;
    // SamplesPerPixel is 1 when the tag is absent.
    let spp = raw.uint(tag::SAMPLES_PER_PIXEL)?.unwrap_or(1);
    let size = match (raw.uint(tag::IMAGE_WIDTH)?, raw.uint(tag::IMAGE_LENGTH)?) {
        (Some(w), Some(h)) => Some((u64::from(w), u64::from(h))),
        _ => None,
    };
    // "Total strip bytes": the sum of all StripByteCounts values.
    let total: u64 = raw
        .uints(tag::STRIP_BYTE_COUNTS)?
        .unwrap_or_default()
        .iter()
        .map(|&v| u64::from(v))
        .sum();
    let tiles = [
        tag::TILE_WIDTH,
        tag::TILE_LENGTH,
        tag::TILE_OFFSETS,
        tag::TILE_BYTE_COUNTS,
    ]
    .iter()
    .all(|&t| raw.has(t));
    let cfa = photometric == Some(32803);
    if let Some((w, h)) = size {
        // Rule 4.
        if compression == Some(1) && cfa && spp == 1 && total >= 2 * w * h {
            return Ok(Sensor(Variant::V1));
        }
        // Rule 5 (R-ARW-70: exact byte counts).
        if compression == Some(32767) && cfa && w % 2 == 0 && total == 3 * w * h / 2 {
            return Ok(Sensor(Variant::V2));
        }
        // Rule 6 (R-ARW-70: exact byte counts; R-ARW-29: W mod 32 = 0).
        if compression == Some(32767) && cfa && w % 32 == 0 && total == w * h {
            return Ok(Sensor(Variant::V3));
        }
    }
    // Rule 7.
    if compression == Some(32767) {
        return Ok(OutOfScope(Relative::X4));
    }
    // Rule 8.
    if compression == Some(7) && cfa && tiles {
        return Ok(Sensor(Variant::V4));
    }
    // Rule 9.
    if compression == Some(7) && photometric == Some(6) && spp == 3 && tiles {
        return Ok(Sensor(Variant::V5));
    }
    Ok(OutOfScope(Relative::X5))
}

/// A rectangle in raw raster coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    /// Left column.
    pub x: usize,
    /// Top row.
    pub y: usize,
    /// Width.
    pub w: usize,
    /// Height.
    pub h: usize,
}

/// Where the sensor data lie in the file.
#[derive(Clone, Debug)]
pub(super) enum Layout {
    /// V1, V2, V3: strips of `rows_per_strip` rows of `row_bytes` bytes.
    Strips {
        /// File offset of each strip.
        offsets: Vec<usize>,
        /// Rows in each strip except possibly the last.
        rows_per_strip: usize,
        /// Bytes per row: R of R-ARW-24.
        row_bytes: usize,
    },
    /// V4, V5: 512 x 512 tiles in tile index order.
    Tiles {
        /// (file offset, byte count) of each tile.
        tiles: Vec<(usize, usize)>,
        /// Tiles per tile row.
        across: usize,
    },
}

/// Where the white balance multipliers came from (R-ARW-46).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WbSource {
    /// Tag 0x7313 WB_RGGBLevels (as shot).
    AsShot,
    /// The D50 scales of the model table.
    Table,
    /// The generic scales.
    Generic,
    /// Not used: V5 data are already white balanced (R-ARW-49).
    NotApplied,
}

/// The V5 conversion parameters (R-ARW-42, R-ARW-48).
#[derive(Clone, Copy, Debug)]
pub(super) struct Ycc {
    /// LumaRed, LumaGreen, LumaBlue.
    pub luma: [f64; 3],
    /// Chroma zero levels of Cb and Cr.
    pub zero: [f64; 2],
    /// Tag 0x7039 correction factors R, G, B.
    pub factors: [f64; 3],
    /// BlackLevel position 0.
    pub black: f64,
    /// YCbCrSubSampling, when present.
    pub subsampling: Option<(u32, u32)>,
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
    /// Valid area width and height (R-ARW-20).
    pub valid: (usize, usize),
    /// Final image area (R-ARW-21).
    pub crop: Rect,
    /// IFD0 orientation 1 to 8 (R-ARW-57).
    pub orientation: u8,
    /// Colors of CFA positions 0 to 3 (R-ARW-18, R-ARW-23).
    pub cfa: [u8; 4],
    /// White level w (R-ARW-43).
    pub white: u32,
    /// Black levels of CFA positions 0 to 3 (R-ARW-44, R-ARW-45).
    pub black: [u32; 4],
    /// WB multipliers R, G, B (R-ARW-46).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// Normalized camera-to-sRGB matrix (R-ARW-50, R-ARW-51).
    pub matrix: [[f64; 3]; 3],
    /// True when the matrix came from the model table.
    pub matrix_from_table: bool,
    /// V3 tone curve knots T1 to T4 (R-ARW-34).
    pub curve: [u32; 4],
    /// V5 conversion parameters.
    pub ycc: Ycc,
    /// Where the sensor data lie.
    pub layout: Layout,
    /// The IFD0 Model string.
    pub model: String,
}

/// Largest W * H accepted (R-ARW-66).
const MAX_PIXELS: u64 = 1 << 27;
/// Required tile size (R-ARW-25).
pub(super) const TILE: usize = 512;

fn require<T>(v: Option<T>, what: &str) -> ArwResult<T> {
    // R-ARW-16: a required tag absent or with count 0 uses the preview path.
    v.ok_or_else(|| fail(format!("required tag {what} missing")))
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check.
pub(super) fn plan(c: &Container<'_>, variant: Variant) -> ArwResult<Plan> {
    let raw = c.raw.as_ref().ok_or_else(|| fail("no raw IFD"))?;
    let data_len = c.data.len() as u64;
    let w = require(raw.uint(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(raw.uint(tag::IMAGE_LENGTH)?, "ImageLength")?;
    require(raw.uint(tag::COMPRESSION)?, "Compression")?;
    require(raw.uint(tag::PHOTOMETRIC)?, "PhotometricInterpretation")?;
    // R-ARW-66: limits on W, H and W * H.
    if w == 0 || h == 0 || w > 65535 || h > 65535 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    let (wu, hu) = (w as usize, h as usize);
    let model_bytes = c.model();
    let model = String::from_utf8_lossy(&model_bytes).into_owned();

    // R-ARW-20: valid area from 0x7038 when 1 <= VW <= W and 1 <= VH <= H.
    let valid = match raw.uints(tag::SONY_RAW_IMAGE_SIZE)? {
        Some(v) if v.len() >= 2 && (1..=w).contains(&v[0]) && (1..=h).contains(&v[1]) => {
            (v[0] as usize, v[1] as usize)
        }
        _ => (wu, hu),
    };

    // R-ARW-21: the DefaultCrop rectangle when both tags are present and it
    // fits the valid area; otherwise the whole valid area. R-ARW-17: an
    // invalid RATIONAL takes the default of its tag. A value that is not a
    // whole number is treated as invalid as well (specification gap: the
    // document allows RATIONAL but does not say how to round).
    let whole = |v: Option<f64>| v.filter(|x| x.fract() == 0.0 && *x >= 0.0 && *x <= 65535.0);
    let pair =
        |vals: Option<Vec<Option<f64>>>, default: (usize, usize)| -> Option<(usize, usize)> {
            let v = vals?;
            if v.len() < 2 {
                return None;
            }
            match (whole(v[0]), whole(v[1])) {
                (Some(a), Some(b)) => Some((a as usize, b as usize)),
                _ => Some(default),
            }
        };
    let origin = pair(raw.reals(tag::DEFAULT_CROP_ORIGIN)?, (0, 0));
    let size = pair(raw.reals(tag::DEFAULT_CROP_SIZE)?, valid);
    let crop = match (origin, size) {
        (Some((cx, cy)), Some((cw, ch)))
            if cw >= 16 && ch >= 16 && cx + cw <= valid.0 && cy + ch <= valid.1 =>
        {
            Rect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
            }
        }
        _ => Rect {
            x: 0,
            y: 0,
            w: valid.0,
            h: valid.1,
        },
    };
    // R-ARW-22: SonyCropTopLeft (0x74C7) and SonyCropSize (0x74C8) are not
    // read at all.

    // R-ARW-18: for V1 to V4 the CFA must be 2 x 2 with one 0, two 1 and
    // one 2; otherwise the preview path.
    let mut cfa = [0u8, 1, 1, 2];
    if variant != Variant::V5 {
        if let Some(dim) = raw.uints(tag::CFA_REPEAT_PATTERN_DIM)? {
            if dim != [2, 2] {
                return Err(fail("CFA repeat pattern is not 2 x 2"));
            }
        }
        if let Some(p) = raw.uints(tag::CFA_PATTERN)? {
            let count = |c: u32| p.iter().filter(|&&x| x == c).count();
            if p.len() != 4 || count(0) != 1 || count(1) != 2 || count(2) != 1 {
                return Err(fail("CFA pattern is not a Bayer pattern"));
            }
            for (o, v) in cfa.iter_mut().zip(&p) {
                *o = *v as u8;
            }
        }
    }

    // R-ARW-43: white level from 0xC61D when 4096 to 65535, else 16383.
    let white = raw
        .uint(tag::WHITE_LEVEL)?
        .filter(|v| (4096..=65535).contains(v))
        .unwrap_or(16383);
    // R-ARW-44: four BlackLevel values, each less than w - 255. R-ARW-45:
    // otherwise 800 for DSC-RX0 and DSC-RX10 models, else 512.
    let tagged_black = match raw.uints(tag::BLACK_LEVEL)? {
        Some(v) if v.len() == 4 && v.iter().all(|&b| b < white - 255) => {
            Some([v[0], v[1], v[2], v[3]])
        }
        _ => None,
    };
    let black = tagged_black.unwrap_or_else(|| {
        let b = if model_bytes.starts_with(b"DSC-RX0") || model_bytes.starts_with(b"DSC-RX10") {
            800
        } else {
            512
        };
        [b; 4]
    });

    // R-ARW-46: WB multipliers from the first available source.
    let (wb, wb_source) = if variant == Variant::V5 {
        // R-ARW-49: no WB multipliers on V5 data.
        ([1.0; 3], WbSource::NotApplied)
    } else {
        match raw.sshorts(tag::WB_RGGB_LEVELS)? {
            Some(v) if v.len() >= 4 && v[..4].iter().all(|&x| x > 0) => {
                let g = f64::from(v[1] + v[2]) / 2.0;
                (
                    [f64::from(v[0]) / g, 1.0, f64::from(v[3]) / g],
                    WbSource::AsShot,
                )
            }
            _ => match color::table_wb(&model_bytes) {
                Some((r, b)) => ([r, 1.0, b], WbSource::Table),
                None => (
                    [color::GENERIC_WB.0, 1.0, color::GENERIC_WB.1],
                    WbSource::Generic,
                ),
            },
        }
    };
    let (matrix, matrix_from_table) = color::matrix(&model_bytes);

    // R-ARW-34: tone curve knots from 0x7010 when 4 values are ascending
    // with T4 > 0, else the standard curve.
    let curve = match raw.uints(tag::SONY_TONE_CURVE)? {
        Some(t) if t.len() == 4 && t[0] <= t[1] && t[1] <= t[2] && t[2] <= t[3] && t[3] > 0 => {
            [t[0], t[1], t[2], t[3]]
        }
        _ => [8000, 10400, 12900, 14100],
    };

    let ycc = if variant == Variant::V5 {
        ycc_params(raw, tagged_black.map_or(512, |b| b[0]))?
    } else {
        Ycc {
            luma: [0.299, 0.587, 0.114],
            zero: [16384.0; 2],
            factors: [1.0; 3],
            black: 512.0,
            subsampling: None,
        }
    };

    let layout = match variant {
        Variant::V1 | Variant::V2 | Variant::V3 => strips(raw, variant, wu, hu, data_len)?,
        Variant::V4 | Variant::V5 => tiles(raw, wu, hu, data_len)?,
    };

    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        valid,
        crop,
        orientation: c.orientation(),
        cfa,
        white,
        black,
        wb,
        wb_source,
        matrix,
        matrix_from_table,
        curve,
        ycc,
        layout,
        model,
    })
}

/// V5 parameters: R-ARW-42 coefficients and chroma zero, R-ARW-48 factors
/// and black level, R-ARW-40 subsampling.
fn ycc_params(raw: &Dir<'_>, black0: u32) -> ArwResult<Ycc> {
    // 0x0211: defaults 0.299, 0.587, 0.114 (TIFF 6.0). R-ARW-17: an invalid
    // RATIONAL takes the tag default; R-ARW-78: LumaGreen is a divisor and
    // must be greater than 0.
    let mut luma = [0.299, 0.587, 0.114];
    if let Some(v) = raw.reals(tag::YCBCR_COEFFICIENTS)? {
        if v.len() >= 3 && v[..3].iter().all(Option::is_some) {
            let l = [
                v[0].unwrap_or(0.0),
                v[1].unwrap_or(0.0),
                v[2].unwrap_or(0.0),
            ];
            if l[1] > 0.0 {
                luma = l;
            }
        }
    }
    // 0x0214: entries 3 and 5 (counted from 1, the reference black of Cb and
    // Cr in TIFF 6.0) are the chroma zero levels; default 16384.
    let mut zero = [16384.0; 2];
    if let Some(v) = raw.reals(tag::REFERENCE_BLACK_WHITE)? {
        if v.len() >= 6 && v[..6].iter().all(Option::is_some) {
            zero = [v[2].unwrap_or(16384.0), v[4].unwrap_or(16384.0)];
        }
    }
    // 0x7039: ratios R, G, B; R-ARW-17 an invalid RATIONAL takes the tag
    // default (1, 1, 1); R-ARW-48 a ratio outside 0.5 to 2 counts as 1.
    let mut factors = [1.0; 3];
    if let Some(v) = raw.reals(tag::WB_CORRECTION)? {
        if v.len() >= 3 && v[..3].iter().all(Option::is_some) {
            for (f, r) in factors.iter_mut().zip(&v[..3]) {
                let r = r.unwrap_or(1.0);
                *f = if (0.5..=2.0).contains(&r) { r } else { 1.0 };
            }
        }
    }
    // R-ARW-48: b is BlackLevel position 0, default 512. R-ARW-78: the
    // divisor 16383 - b must be greater than 0, else b takes its default.
    let black = if black0 < 16383 {
        f64::from(black0)
    } else {
        512.0
    };
    let subsampling = match raw.uints(tag::YCBCR_SUBSAMPLING)? {
        Some(v) if v.len() >= 2 => Some((v[0], v[1])),
        Some(_) => return Err(fail("YCbCrSubSampling has fewer than 2 values")),
        None => None,
    };
    Ok(Ycc {
        luma,
        zero,
        factors,
        black,
        subsampling,
    })
}

/// Strip layout of V1 to V3 (R-ARW-19, R-ARW-24, R-ARW-68, R-ARW-69,
/// R-ARW-70).
fn strips(raw: &Dir<'_>, variant: Variant, w: usize, h: usize, data_len: u64) -> ArwResult<Layout> {
    let offsets = require(raw.uints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(raw.uints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    // R-ARW-24: the row size R of the variant.
    let row_bytes = match variant {
        Variant::V1 => 2 * w,
        Variant::V2 => 3 * w / 2,
        _ => w,
    };
    // R-ARW-19: RowsPerStrip is H when absent, 0 or greater than H.
    let rps = match raw.uint(tag::ROWS_PER_STRIP)? {
        Some(r) if r > 0 && (r as usize) <= h => r as usize,
        _ => h,
    };
    // R-ARW-69 case 2: equal counts.
    if offsets.len() != counts.len() {
        return Err(fail("StripOffsets and StripByteCounts differ in count"));
    }
    // R-ARW-69 case 3: ceil(H / RowsPerStrip) strips.
    if offsets.len() != h.div_ceil(rps) {
        return Err(fail("strip count does not match RowsPerStrip"));
    }
    // R-ARW-69 case 1: total bytes at least R * H.
    let total: u64 = counts.iter().map(|&c| u64::from(c)).sum();
    if total < row_bytes as u64 * h as u64 {
        return Err(fail("strips hold fewer bytes than the raster"));
    }
    let mut starts = Vec::with_capacity(offsets.len());
    for (s, (&off, &count)) in offsets.iter().zip(&counts).enumerate() {
        let rows = rps.min(h - s * rps);
        // R-ARW-69 case 4: each strip holds R times its rows.
        if u64::from(count) < (row_bytes * rows) as u64 {
            return Err(fail("a strip is shorter than its rows"));
        }
        // R-ARW-68: the strip byte range lies inside the file (64-bit sums,
        // R-ARW-65).
        if u64::from(off) + u64::from(count) > data_len {
            return Err(fail("a strip lies outside the file"));
        }
        starts.push(off as usize);
    }
    // R-ARW-70: V2 and V3 never read more than 3 * W * H / 2 or W * H bytes:
    // the decoder reads exactly R * H bytes, R = 3 * W / 2 or W.
    Ok(Layout::Strips {
        offsets: starts,
        rows_per_strip: rps,
        row_bytes,
    })
}

/// Tile layout of V4 and V5 (R-ARW-25, R-ARW-26, R-ARW-68, R-ARW-71).
fn tiles(raw: &Dir<'_>, w: usize, h: usize, data_len: u64) -> ArwResult<Layout> {
    let tw = require(raw.uint(tag::TILE_WIDTH)?, "TileWidth")? as usize;
    let tl = require(raw.uint(tag::TILE_LENGTH)?, "TileLength")? as usize;
    let offsets = require(raw.uints(tag::TILE_OFFSETS)?, "TileOffsets")?;
    let counts = require(raw.uints(tag::TILE_BYTE_COUNTS)?, "TileByteCounts")?;
    // R-ARW-25: 512 x 512 tiles and W, H multiples of 512.
    if tw != TILE || tl != TILE || !w.is_multiple_of(TILE) || !h.is_multiple_of(TILE) {
        return Err(fail("tile size is not 512 x 512"));
    }
    let across = w / TILE;
    let down = h / TILE;
    // R-ARW-25, R-ARW-71: exactly TilesAcross * TilesDown entries.
    if offsets.len() != across * down || counts.len() != across * down {
        return Err(fail("tile count does not match the raster"));
    }
    let mut tiles = Vec::with_capacity(offsets.len());
    for (&off, &count) in offsets.iter().zip(&counts) {
        // R-ARW-68: each tile byte range lies inside the file.
        if u64::from(off) + u64::from(count) > data_len {
            return Err(fail("a tile lies outside the file"));
        }
        tiles.push((off as usize, count as usize));
    }
    // R-ARW-26: entry t is tile column t mod TilesAcross, tile row
    // floor(t / TilesAcross); the offsets need not be in file order.
    Ok(Layout::Tiles { tiles, across })
}

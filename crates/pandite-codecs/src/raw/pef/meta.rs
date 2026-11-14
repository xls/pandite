//! The container of a PEF file (IFD chain, Exif IFD, maker note), its
//! classification (R-PEF-3) and the decode plan: sizes, crop, CFA, levels,
//! white balance, color matrix and orientation, all validated before any
//! large allocation.

use super::color;
use super::huffman::Table;
use super::tiff::{self, Dir};
use super::{fail, PefResult};
use crate::raw::arw::{Rect, WbSource};

/// Tag numbers of the specification's Metadata tags tables.
pub(super) mod tag {
    pub const IMAGE_WIDTH: u16 = 0x0100;
    pub const IMAGE_LENGTH: u16 = 0x0101;
    pub const BITS_PER_SAMPLE: u16 = 0x0102;
    pub const COMPRESSION: u16 = 0x0103;
    pub const PHOTOMETRIC: u16 = 0x0106;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const SAMPLES_PER_PIXEL: u16 = 0x0115;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const SUB_IFDS: u16 = 0x014A;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const MAKER_NOTE: u16 = 0x927C;
    pub const CFA_PATTERN: u16 = 0xA302;
    // Maker note.
    pub const IMAGE_AREA_OFFSET: u16 = 0x0038;
    pub const RAW_IMAGE_SIZE: u16 = 0x0039;
    pub const WHITE_LEVEL: u16 = 0x007E;
    pub const BLACK_POINT: u16 = 0x0200;
    pub const WHITE_POINT: u16 = 0x0201;
    pub const HUFFMAN_TABLE: u16 = 0x0220;
    pub const CROP: u16 = 0x023E;
}

/// The maker-note tags this decoder reads.
const MAKER_TAGS: [u16; 7] = [
    tag::IMAGE_AREA_OFFSET,
    tag::RAW_IMAGE_SIZE,
    tag::WHITE_LEVEL,
    tag::BLACK_POINT,
    tag::WHITE_POINT,
    tag::HUFFMAN_TABLE,
    tag::CROP,
];

/// The storage variants this decoder develops from sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// 16-bit samples, 12-bit values.
    P1,
    /// 12-bit values packed into 3 bytes per 2 pixels.
    P2,
    /// Pentax Huffman coding, CFA.
    P3,
    /// Pentax Huffman coding, monochrome.
    P4,
}

/// The out-of-scope cases that use the preview path (R-PEF-2). X1 (several
/// frames) is decoded as its IFD0 frame (R-PEF-52); X4 is not PEF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// Compression 65535 that no Huffman variant matches.
    X2,
    /// Any other storage, or a maker note that cannot be parsed.
    X3,
}

/// The result of classification (R-PEF-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant.
    Sensor(Variant),
    /// A case that uses the preview path.
    OutOfScope(Relative),
}

/// The two maker-note layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    /// "AOC", value offsets from the file start.
    Aoc,
    /// "PENTAX ", value offsets from the maker-note start M.
    Pentax,
}

/// The parsed maker note.
pub(super) struct MakerNote<'a> {
    /// The layout.
    #[cfg_attr(not(test), allow(dead_code))]
    pub form: Form,
    /// M, the file offset of the maker note.
    #[cfg_attr(not(test), allow(dead_code))]
    pub at: u64,
    /// The maker-note IFD.
    pub dir: Dir<'a>,
}

/// The parsed container.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// True for a little-endian file.
    pub le: bool,
    /// IFD0, IFD1, IFD2 and later IFDs of the chain.
    pub chain: Vec<Dir<'a>>,
    /// The Exif IFD, when valid.
    pub exif: Option<Dir<'a>>,
    /// The maker note, when it can be parsed.
    pub maker: Option<MakerNote<'a>>,
}

/// Parse the header, the IFD chain, the Exif IFD and the maker note.
/// Fails for a file that is not a TIFF file or whose IFD0 is invalid.
pub(super) fn open(data: &[u8]) -> PefResult<Container<'_>> {
    // R-PEF-59, R-PEF-4.
    let (le, first) = tiff::header(data)?;
    // R-PEF-11: IFD0 and the chain; an invalid IFD0 is an error.
    let chain = tiff::chain(data, first, le);
    let Some(ifd0) = chain.first() else {
        return Err(fail("IFD0 is not a valid IFD"));
    };
    // R-PEF-14: the Exif IFD at the offset in IFD0 tag 0x8769, valid by
    // R-PEF-12.
    let exif = match ifd0.uint(tag::EXIF_IFD) {
        Ok(Some(off)) => tiff::dir(data, u64::from(off), le, 0),
        _ => None,
    };
    let maker = exif.as_ref().and_then(|e| maker_note(data, e));
    Ok(Container {
        data,
        le,
        chain,
        exif,
        maker,
    })
}

/// R-PEF-14 to R-PEF-17 and R-PEF-71: the maker note, or `None` when it
/// cannot be parsed (R-PEF-3 rule 1).
fn maker_note<'a>(data: &'a [u8], exif: &Dir<'a>) -> Option<MakerNote<'a>> {
    // R-PEF-71: tag 0x927C (BYTE or UNDEFINED, R-PEF-8) with a count of at
    // least 14. Its count exceeds 4, so the value field holds the offset M
    // (R-PEF-14).
    let e = exif.entry(tag::MAKER_NOTE)?;
    if !matches!(e.typ, 1 | 7) || e.count < 14 {
        return None;
    }
    let m = e.at;
    // R-PEF-71: M + 14 inside the file. The maker note need not lie inside
    // the 0x927C byte count.
    if m + 14 > data.len() as u64 {
        return None;
    }
    let head = &data[m as usize..m as usize + 14];
    // R-PEF-15: the form from the bytes at M; R-PEF-16: the origin of the
    // value offsets.
    let (form, order, ifd_at, origin) = if head[..4] == *b"AOC\0" {
        (Form::Aoc, &head[4..6], m + 6, 0)
    } else if head[..8] == *b"PENTAX \0" {
        (Form::Pentax, &head[8..10], m + 10, m)
    } else {
        return None;
    };
    let le = match order {
        [0x4D, 0x4D] => false,
        [0x49, 0x49] => true,
        _ => return None,
    };
    // R-PEF-17: a valid IFD at M + 6 or M + 10.
    let dir = tiff::dir(data, ifd_at, le, origin)?;
    // R-PEF-71 and R-PEF-62: the values this decoder reads lie inside the
    // file.
    for t in MAKER_TAGS {
        if let Some(e) = dir.entry(t) {
            if !dir.inside(e) {
                return None;
            }
        }
    }
    Some(MakerNote { form, at: m, dir })
}

impl Container<'_> {
    /// IFD0.
    pub(super) fn ifd0(&self) -> &Dir<'_> {
        &self.chain[0]
    }

    /// The IFD0 Orientation: 1 when absent or outside 1 to 8 (R-PEF-50).
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().uint(tag::ORIENTATION) {
            Ok(Some(v @ 1..=8)) => v as u8,
            _ => 1,
        }
    }

    /// The Huffman table of tag 0x0220 for `bps`-bit data, when valid
    /// (R-PEF-27, R-PEF-28).
    pub(super) fn table(&self, bps: u32) -> PefResult<Option<Table>> {
        let Some(mn) = &self.maker else {
            return Ok(None);
        };
        Ok(mn
            .dir
            .byte_array(tag::HUFFMAN_TABLE)?
            .and_then(|b| Table::parse(b, bps, mn.dir.le)))
    }
}

/// Classify the file by the rules of R-PEF-3, in order. A file that
/// matches no rule is X3.
pub(super) fn classify(c: &Container<'_>) -> PefResult<Class> {
    use Class::{OutOfScope, Sensor};
    // Rule 1: the maker note cannot be parsed.
    if c.maker.is_none() {
        return Ok(OutOfScope(Relative::X3));
    }
    // R-PEF-13: the image tags of IFD0; R-PEF-70: the first value.
    let ifd0 = c.ifd0();
    let compression = ifd0.uint(tag::COMPRESSION)?;
    let photometric = ifd0.uint(tag::PHOTOMETRIC)?;
    // SamplesPerPixel is 1 when the tag is absent.
    let spp = ifd0.uint(tag::SAMPLES_PER_PIXEL)?.unwrap_or(1);
    let bps = ifd0.uint(tag::BITS_PER_SAMPLE)?;
    // W * H of two 32-bit values fits 64 bits; the multiples are checked.
    let size = match (ifd0.uint(tag::IMAGE_WIDTH)?, ifd0.uint(tag::IMAGE_LENGTH)?) {
        (Some(w), Some(h)) => Some((w, u64::from(w) * u64::from(h))),
        _ => None,
    };
    let strip_bytes = ifd0.uint(tag::STRIP_BYTE_COUNTS)?.map(u64::from);
    match compression {
        Some(65535) => {
            let huffman = match bps {
                Some(b @ (12 | 14)) => spp == 1 && c.table(b)?.is_some(),
                _ => false,
            };
            Ok(match photometric {
                // Rule 2.
                Some(32803) if huffman => Sensor(Variant::P3),
                // Rule 3.
                Some(34892) if huffman => Sensor(Variant::P4),
                // Rule 4.
                _ => OutOfScope(Relative::X2),
            })
        }
        // Rule 5.
        Some(1) if photometric == Some(32803) && spp == 1 && bps == Some(12) => {
            match (size, strip_bytes) {
                (Some((_, px)), Some(n)) if px.checked_mul(2).is_some_and(|b| n >= b) => {
                    Ok(Sensor(Variant::P1))
                }
                _ => Ok(OutOfScope(Relative::X3)),
            }
        }
        // Rule 6.
        Some(32773) if photometric == Some(32803) && spp == 1 && bps == Some(12) => {
            match (size, strip_bytes) {
                (Some((w, px)), Some(n))
                    if w % 2 == 0 && px.checked_mul(3).is_some_and(|b| n == b / 2) =>
                {
                    Ok(Sensor(Variant::P2))
                }
                _ => Ok(OutOfScope(Relative::X3)),
            }
        }
        _ => Ok(OutOfScope(Relative::X3)),
    }
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
    /// BitsPerSample.
    pub bps: u32,
    /// True for a little-endian file (P1 samples, R-PEF-5).
    pub le: bool,
    /// File offset and byte count of the strip.
    pub strip: (usize, usize),
    /// The Huffman table (P3, P4).
    pub table: Option<Table>,
    /// The crop rectangle (R-PEF-22, R-PEF-23).
    pub crop: Rect,
    /// IFD0 orientation 1 to 8 (R-PEF-50).
    pub orientation: u8,
    /// Colors (0 red, 1 green, 2 blue) of CFA positions 0 to 3, origin at
    /// stored pixel (0, 0) (R-PEF-21). Unused for P4.
    pub cfa: [u8; 4],
    /// White level w (R-PEF-37).
    pub white: u32,
    /// The four values of tag 0x0200 when valid (R-PEF-38).
    #[cfg_attr(not(test), allow(dead_code))]
    pub black_tag: Option<[u32; 4]>,
    /// Twice the black level of red, green and blue (R-PEF-38, R-PEF-39):
    /// 2 * B0, B1 + B2, 2 * B3, so the mean of the greens stays whole.
    pub black2: [u32; 3],
    /// The P4 black level: the mean of the four values (R-PEF-42).
    pub gray_black: f64,
    /// WB multipliers R, G, B (R-PEF-40).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// Normalized camera-to-sRGB matrix (R-PEF-43, R-PEF-44).
    pub matrix: [[f64; 3]; 3],
    /// True when the matrix came from the model table.
    pub matrix_from_table: bool,
    /// The IFD0 Model string.
    pub model: String,
    /// True when IFD0 has SubIFDs (several frames, decoded as IFD0 alone).
    pub multi_frame: bool,
}

/// Largest W * H accepted (R-PEF-60).
const MAX_PIXELS: u64 = 1 << 27;

fn require<T>(v: Option<T>, what: &str) -> PefResult<T> {
    // R-PEF-18: a required tag absent, with a count of 0 or with its value
    // outside the file uses the preview path.
    v.ok_or_else(|| fail(format!("required tag {what} missing")))
}

/// R-PEF-19: the colors of CFA positions 0 to 3 of a valid CFAPattern.
pub(super) fn bayer(p: &[u8]) -> Option<[u8; 4]> {
    if p.len() != 8 || (p[..4] != [0, 2, 0, 2] && p[..4] != [2, 0, 2, 0]) {
        return None;
    }
    let c = [p[4], p[5], p[6], p[7]];
    let count = |v: u8| c.iter().filter(|&&x| x == v).count();
    let greens = (c[1] == 1 && c[2] == 1) || (c[0] == 1 && c[3] == 1);
    (count(0) == 1 && count(1) == 2 && count(2) == 1 && greens).then_some(c)
}

/// Build and validate the plan for a file of a specified variant. Fails
/// (preview path) on any missing required tag or failed check.
pub(super) fn plan(c: &Container<'_>, variant: Variant) -> PefResult<Plan> {
    let mn = c.maker.as_ref().ok_or_else(|| fail("no maker note"))?;
    let ifd0 = c.ifd0();
    let size = c.data.len() as u64;
    // R-PEF-13, R-PEF-18: the required tags of IFD0.
    let w = require(ifd0.uint(tag::IMAGE_WIDTH)?, "ImageWidth")?;
    let h = require(ifd0.uint(tag::IMAGE_LENGTH)?, "ImageLength")?;
    let bps = require(ifd0.uint(tag::BITS_PER_SAMPLE)?, "BitsPerSample")?;
    require(ifd0.uint(tag::COMPRESSION)?, "Compression")?;
    require(ifd0.uint(tag::PHOTOMETRIC)?, "PhotometricInterpretation")?;
    if !matches!(bps, 12 | 14) {
        return Err(fail("BitsPerSample is not 12 or 14"));
    }
    // R-PEF-60: limits on W, H and W * H.
    let px = u64::from(w) * u64::from(h);
    if w == 0 || h == 0 || w > 65535 || h > 65535 || px > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    // R-PEF-20: exactly one strip, completely inside the file.
    let offsets = require(ifd0.uints(tag::STRIP_OFFSETS)?, "StripOffsets")?;
    let counts = require(ifd0.uints(tag::STRIP_BYTE_COUNTS)?, "StripByteCounts")?;
    if offsets.len() != 1 || counts.len() != 1 {
        return Err(fail("not exactly one strip"));
    }
    let (off, len) = (u64::from(offsets[0]), u64::from(counts[0]));
    if off + len > size {
        return Err(fail("the strip lies outside the file"));
    }
    // R-PEF-63, R-PEF-64: the strip holds the raster.
    let fits = match variant {
        Variant::P1 => len >= 2 * px,
        Variant::P2 => w % 2 == 0 && len == 3 * px / 2,
        Variant::P3 | Variant::P4 => len >= px.div_ceil(8),
    };
    if !fits {
        return Err(fail("the strip does not hold the raster"));
    }
    let table = match variant {
        Variant::P3 | Variant::P4 => Some(require(c.table(bps)?, "Huffman table")?),
        Variant::P1 | Variant::P2 => None,
    };
    let (wu, hu) = (w as usize, h as usize);
    // Model and Orientation are optional: absent or outside the file means
    // the default, a value larger than the file fails (R-PEF-61).
    let model_bytes = ifd0.ascii(tag::MODEL)?.unwrap_or_default().to_vec();
    ifd0.uint(tag::ORIENTATION)?;
    let model = String::from_utf8_lossy(&model_bytes).into_owned();
    let md = &mn.dir;

    // R-PEF-22: tag 0x023E (BYTE or UNDEFINED, count 8), else tags 0x0038
    // and 0x0039 with 2 values each, else the whole raster.
    let from_23e = match md.entry(tag::CROP) {
        Some(e) if matches!(e.typ, 1 | 7) && e.count == 8 => md.bytes(e)?.map(|b| {
            let v = tiff::u16s(b, md.le);
            [v[0], v[1], v[2], v[3]].map(u32::from)
        }),
        _ => None,
    };
    let rect = match from_23e {
        Some(r) => Some(r),
        None => match (
            md.uints(tag::IMAGE_AREA_OFFSET)?,
            md.uints(tag::RAW_IMAGE_SIZE)?,
        ) {
            (Some(o), Some(s)) if o.len() == 2 && s.len() == 2 => Some([o[0], o[1], s[0], s[1]]),
            _ => None,
        },
    };
    // R-PEF-23: the whole raster when the rectangle is empty or does not
    // fit (64-bit sums).
    let crop = match rect {
        Some([x0, y0, cw, ch])
            if cw >= 1
                && ch >= 1
                && u64::from(x0) + u64::from(cw) <= u64::from(w)
                && u64::from(y0) + u64::from(ch) <= u64::from(h) =>
        {
            Rect {
                x: x0 as usize,
                y: y0 as usize,
                w: cw as usize,
                h: ch as usize,
            }
        }
        _ => Rect {
            x: 0,
            y: 0,
            w: wu,
            h: hu,
        },
    };

    // R-PEF-19, R-PEF-21: the CFA of P1 to P3 from Exif tag 0xA302.
    let cfa = if variant == Variant::P4 {
        [1; 4]
    } else {
        let exif = c.exif.as_ref().ok_or_else(|| fail("no Exif IFD"))?;
        let p = require(exif.byte_array(tag::CFA_PATTERN)?, "CFAPattern")?;
        bayer(p).ok_or_else(|| fail("CFAPattern is not a 2 x 2 Bayer pattern"))?
    };

    // R-PEF-37: the white level from tag 0x007E when 256 to 2^BPS - 1.
    let maxv = (1u32 << bps) - 1;
    let white = match md.uint(tag::WHITE_LEVEL)? {
        Some(v) if (256..=maxv).contains(&v) => v,
        _ => maxv,
    };
    // R-PEF-38: four values, each less than w - 255; R-PEF-39: else 0.
    let black_tag = match md.uints(tag::BLACK_POINT)? {
        Some(v) if v.len() == 4 && v.iter().all(|&b| b < white - 255) => {
            Some([v[0], v[1], v[2], v[3]])
        }
        _ => None,
    };
    let black2 = black_tag.map_or([0; 3], |b| [2 * b[0], b[1] + b[2], 2 * b[3]]);
    let gray_black = black_tag.map_or(0.0, |b| b.iter().map(|&v| f64::from(v)).sum::<f64>() / 4.0);

    // R-PEF-40: WB multipliers of P1 to P3 from the first available
    // source; R-PEF-42: none for P4.
    let (wb, wb_source) = if variant == Variant::P4 {
        ([1.0; 3], WbSource::NotApplied)
    } else {
        match md.uints(tag::WHITE_POINT)? {
            Some(v) if v.len() == 4 && v.iter().all(|&x| x > 0) => {
                // R-PEF-67: G > 0 because both greens are.
                let g = (f64::from(v[1]) + f64::from(v[2])) / 2.0;
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
    // R-PEF-43, R-PEF-44: the normalized matrix; R-PEF-47: none for P4.
    let (matrix, matrix_from_table) = if variant == Variant::P4 {
        ([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], false)
    } else {
        color::matrix(&model_bytes)
    };

    Ok(Plan {
        variant,
        width: wu,
        height: hu,
        bps,
        le: c.le,
        strip: (off as usize, len as usize),
        table,
        crop,
        orientation: c.orientation(),
        cfa,
        white,
        black_tag,
        black2,
        gray_black,
        wb,
        wb_source,
        matrix,
        matrix_from_table,
        model,
        // R-PEF-52: the SubIFDs are never read.
        multi_frame: ifd0.has(tag::SUB_IFDS),
    })
}

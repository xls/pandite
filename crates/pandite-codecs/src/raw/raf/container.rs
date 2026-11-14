//! The RAF container: the header (R-RAF-4 to R-RAF-6), the meta container
//! (R-RAF-7, R-RAF-8), the CFA container and its FujiIFD (R-RAF-9 to
//! R-RAF-15), the classification (R-RAF-3) and the decode plan, all
//! validated before any large allocation.

use super::{fail, RafResult};
use crate::raw::arw::ifd::{self, Dir, Entry};
use crate::raw::arw::Rect;
use crate::util;

/// Bytes 0 to 15 of every RAF file (R-RAF-4).
pub(crate) const MAGIC: &[u8; 16] = b"FUJIFILMCCD-RAW ";
/// Header length of format version "0201" (R-RAF-54: shorter files are
/// rejected).
pub(super) const HEADER_LEN: usize = 148;
/// Most meta container records (R-RAF-56).
const MAX_RECORDS: u32 = 4096;
/// Largest W * H accepted (R-RAF-55).
pub(super) const MAX_PIXELS: u64 = 1 << 27;

/// Tag numbers of the specification's Metadata tags section.
pub(super) mod tag {
    /// Meta record RawImageFullSize: H, W.
    pub const FULL_SIZE: u16 = 0x0100;
    /// Meta record RawImageCropTopLeft: top, left.
    pub const CROP_TOP_LEFT: u16 = 0x0110;
    /// Meta record RawImageCroppedSize: height, width.
    pub const CROPPED_SIZE: u16 = 0x0111;
    /// Meta record FujiLayout.
    pub const FUJI_LAYOUT: u16 = 0x0130;
    /// Meta record XTransLayout.
    pub const XTRANS_LAYOUT: u16 = 0x0131;
    /// IFD0 entry of the CFA container that points to the FujiIFD.
    pub const FUJI_IFD: u16 = 0xF000;
    /// FujiIFD RawImageFullWidth W.
    pub const WIDTH: u16 = 0xF001;
    /// FujiIFD RawImageFullHeight H.
    pub const HEIGHT: u16 = 0xF002;
    /// FujiIFD BitsPerSample.
    pub const BITS: u16 = 0xF003;
    /// FujiIFD StripOffsets.
    pub const STRIP_OFFSETS: u16 = 0xF007;
    /// FujiIFD StripByteCounts.
    pub const STRIP_BYTE_COUNTS: u16 = 0xF008;
    /// FujiIFD BlackLevel.
    pub const BLACK_LEVEL: u16 = 0xF00A;
    /// FujiIFD WB_GRBLevelsAuto.
    pub const WB_AUTO: u16 = 0xF00D;
    /// FujiIFD WB_GRBLevels.
    pub const WB: u16 = 0xF00E;
}

/// The FujiLayout value of the Bayer layout C2 (R-RAF-19).
const BAYER_LAYOUT: [u8; 4] = [0x0A, 0x0B, 0x09, 0x08];

/// The storage variants this decoder develops from sensor data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// 12-bit packed, 2 pixels in 3 bytes.
    V1,
    /// 14-bit packed, 16 pixels in 7 LE32 words.
    V2,
    /// One LE16 value per pixel.
    V3,
}

/// The color filter layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// C1: the 6x6 X-Trans pattern of meta record 0x0131.
    XTrans,
    /// C2: 2x2 Bayer RGGB.
    Bayer,
}

/// The out-of-scope relatives that use the preview path (R-RAF-2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Relative {
    /// Lossless compressed (RAFCompression 2).
    X1,
    /// Lossy compressed (RAFCompression 3).
    X2,
    /// Legacy container.
    X3,
    /// Two-image file.
    X4,
    /// Other color filter layout.
    X5,
    /// Any other structure.
    X6,
}

/// The result of classification (R-RAF-3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// Sensor data in a specified variant and layout.
    Sensor(Variant, Layout),
    /// A relative that uses the preview path.
    OutOfScope(Relative),
}

/// Where the white balance multipliers came from (R-RAF-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WbSource {
    /// FujiIFD 0xF00E WB_GRBLevels.
    AsShot,
    /// FujiIFD 0xF00D WB_GRBLevelsAuto.
    Auto,
    /// The generic multipliers.
    Generic,
}

/// One part of the file named by the header: offset and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Part {
    /// File offset.
    pub offset: usize,
    /// Length in bytes.
    pub len: usize,
}

/// The header of a RAF file.
pub(super) struct Raf<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// Format version, bytes 16 to 19.
    pub version: [u8; 4],
    /// The embedded JPEG, when present (R-RAF-6).
    pub jpeg: Option<Part>,
    /// The meta container, when present.
    pub meta: Option<Part>,
    /// The CFA container, when present.
    pub cfa: Option<Part>,
    /// RAFCompression, offset 108.
    pub compression: u32,
    /// The BE32 value at offset 128 (second CFA container offset).
    pub second_cfa: u32,
}

impl<'a> Raf<'a> {
    /// The bytes of a present part.
    pub(super) fn part(&self, p: Option<Part>) -> Option<&'a [u8]> {
        let p = p?;
        self.data.get(p.offset..p.offset.checked_add(p.len)?)
    }
}

/// R-RAF-6: a part is present only when its length is greater than 0 and
/// offset + length (64-bit sum, R-RAF-54) is at most the file size.
fn part(size: u64, offset: u32, len: u32) -> Option<Part> {
    (len > 0 && u64::from(offset) + u64::from(len) <= size).then_some(Part {
        offset: offset as usize,
        len: len as usize,
    })
}

/// Check the size and the magic and read the header. `Ok(None)` when the
/// data are not a RAF file (R-RAF-4).
pub(super) fn open(data: &[u8]) -> RafResult<Option<Raf<'_>>> {
    // R-RAF-4: bytes 0 to 15 are the magic, else "not a RAF file".
    if data.get(..16) != Some(&MAGIC[..]) {
        return Ok(None);
    }
    // R-RAF-54: a file shorter than the header is rejected.
    if data.len() < HEADER_LEN {
        return Err(fail("file shorter than the 148-byte header"));
    }
    // R-RAF-5: the header fields at offsets 84 to 128 are BE32 values; the
    // file holds at least 148 bytes, so every one of them can be read.
    let be = |at: usize| util::be32(data, at).unwrap_or(0);
    let size = data.len() as u64;
    let mut version = [0u8; 4];
    version.copy_from_slice(&data[16..20]);
    Ok(Some(Raf {
        data,
        version,
        jpeg: part(size, be(84), be(88)),
        meta: part(size, be(92), be(96)),
        cfa: part(size, be(100), be(104)),
        compression: be(108),
        second_cfa: be(128),
    }))
}

/// The meta container records, in file order.
#[derive(Default)]
pub(super) struct Records<'a> {
    list: Vec<(u16, &'a [u8])>,
}

impl<'a> Records<'a> {
    /// R-RAF-8: the data of the first record of `tag`.
    pub(super) fn get(&self, tag: u16) -> Option<&'a [u8]> {
        self.list.iter().find(|r| r.0 == tag).map(|r| r.1)
    }

    /// Two BE16 values of a record with at least 4 bytes.
    fn pair(&self, tag: u16) -> Option<(u16, u16)> {
        let r = self.get(tag)?;
        Some((util::be16(r, 0)?, util::be16(r, 2)?))
    }
}

/// Read the meta container (R-RAF-7, R-RAF-56). No records when it is
/// absent or too short for its count.
pub(super) fn records<'a>(raf: &Raf<'a>) -> RafResult<Records<'a>> {
    let Some(m) = raf.part(raf.meta) else {
        return Ok(Records::default());
    };
    let Some(n) = util::be32(m, 0) else {
        return Ok(Records::default());
    };
    // R-RAF-56: more than 4096 records fail.
    if n > MAX_RECORDS {
        return Err(fail("meta container has more than 4096 records"));
    }
    // R-RAF-7: in order, stopping at the first record whose head or data
    // leave the container; R-RAF-56: a record is stored only after its
    // bytes are checked, so memory never grows with N or a record size.
    let mut list = Vec::new();
    let mut pos = 4usize;
    for _ in 0..n {
        let (Some(tag), Some(size)) = (util::be16(m, pos), util::be16(m, pos + 2)) else {
            break;
        };
        let start = pos + 4;
        let end = start + usize::from(size);
        let Some(bytes) = m.get(start..end) else {
            break;
        };
        list.push((tag, bytes));
        pos = end;
    }
    Ok(Records { list })
}

/// The FujiIFD inside the CFA container. Offsets of its entries count from
/// the first byte of the container (R-RAF-11): the IFD reader works on the
/// container's bytes, so every offset is checked against the container.
pub(super) struct Fuji<'a> {
    dir: Dir<'a>,
}

impl<'a> Fuji<'a> {
    /// R-RAF-12: the first entry of `tag` in any entry order whose type is
    /// SHORT or LONG; entries of other types are skipped. R-RAF-14: an
    /// entry with no value counts as absent.
    fn entry(&self, tag: u16) -> Option<&Entry> {
        self.dir
            .entries
            .iter()
            .find(|e| e.tag == tag && matches!(e.typ, 3 | 4) && e.count >= 1)
    }

    /// The value bytes of an integer tag and its type. R-RAF-57: a value
    /// range larger than the container fails before anything is read;
    /// R-RAF-58: the range must lie inside the container.
    fn raw(&self, tag: u16) -> RafResult<Option<(&'a [u8], u16)>> {
        match self.entry(tag) {
            Some(e) => Ok(Some((self.dir.bytes(e)?, e.typ))),
            None => Ok(None),
        }
    }

    /// The values of an integer tag, read one by one without allocation.
    fn each(bytes: &[u8], typ: u16) -> impl Iterator<Item = u32> + '_ {
        let unit = if typ == 3 { 2 } else { 4 };
        bytes.chunks_exact(unit).map(move |c| {
            if unit == 2 {
                u32::from(u16::from_le_bytes([c[0], c[1]]))
            } else {
                u32::from_le_bytes([c[0], c[1], c[2], c[3]])
            }
        })
    }

    /// The first value of an integer tag.
    fn first(&self, tag: u16) -> RafResult<Option<u32>> {
        Ok(self
            .raw(tag)?
            .and_then(|(b, typ)| Self::each(b, typ).next()))
    }

    /// The first three values of an integer tag with at least three.
    fn three(&self, tag: u16) -> RafResult<Option<[u32; 3]>> {
        let Some((b, typ)) = self.raw(tag)? else {
            return Ok(None);
        };
        let mut it = Self::each(b, typ);
        Ok(match (it.next(), it.next(), it.next()) {
            (Some(a), Some(b), Some(c)) => Some([a, b, c]),
            _ => None,
        })
    }

    /// R-RAF-29: the arithmetic mean of all BlackLevel values.
    fn black_mean(&self) -> RafResult<Option<f64>> {
        let Some((b, typ)) = self.raw(tag::BLACK_LEVEL)? else {
            return Ok(None);
        };
        let (mut sum, mut n) = (0f64, 0u64);
        for v in Self::each(b, typ) {
            sum += f64::from(v);
            n += 1;
        }
        // R-RAF-62: the divisor n is checked to be greater than 0.
        Ok((n > 0).then(|| sum / n as f64))
    }
}

/// The CFA container bytes, its TIFF header (R-RAF-9), IFD0 and the FujiIFD
/// (R-RAF-10, R-RAF-13). `None` when one of them is missing or invalid.
fn fuji<'a>(raf: &Raf<'a>) -> Option<Fuji<'a>> {
    let c = raf.part(raf.cfa)?;
    // R-RAF-9: the first 8 bytes are 49 49 2A 00 and a LE32 IFD0 offset;
    // every TIFF field of the container is little-endian.
    if c.len() < 8 || c[..4] != [0x49, 0x49, 0x2A, 0x00] {
        return None;
    }
    let ifd0_at = ifd::le32(c, 4)?;
    // R-RAF-13: an IFD outside the container, with N of 0 or above 1024,
    // or with its 2 + 12 * N + 4 bytes not inside the container is invalid.
    let ifd0 = ifd::dir(c, u64::from(ifd0_at))?;
    // R-RAF-10: the entry 0xF000 of type 13 or 4 with count 1.
    let e = ifd0
        .entries
        .iter()
        .find(|e| e.tag == tag::FUJI_IFD && matches!(e.typ, 13 | 4) && e.count == 1)?;
    // R-RAF-11: one 4-byte value sits in the value field itself.
    let at = ifd::le32(c, e.at)?;
    Some(Fuji {
        dir: ifd::dir(c, u64::from(at))?,
    })
}

/// R-RAF-17 and R-RAF-18 (C1) or R-RAF-19 (C2): the layout and the color
/// (0 red, 1 green, 2 blue) of every position (x mod 6, y mod 6) at index
/// 6 * (y mod 6) + (x mod 6). R-RAF-20: the origin of both patterns is
/// stored pixel (0, 0). `None` when neither layout applies (X5).
pub(super) fn layout(records: &Records<'_>) -> Option<(Layout, [u8; 36])> {
    let mut pattern = [0u8; 36];
    if let Some(x) = records.get(tag::XTRANS_LAYOUT) {
        // R-RAF-17: exactly 36 bytes of 0, 1 or 2: 8 red, 20 green, 8 blue.
        let count = |v: u8| x.iter().filter(|&&b| b == v).count();
        if x.len() != 36 || count(0) != 8 || count(1) != 20 || count(2) != 8 {
            return None;
        }
        // R-RAF-18: pixel (x, y) has the color of byte
        // 35 - (6 * (y mod 6) + (x mod 6)).
        for (i, p) in pattern.iter_mut().enumerate() {
            *p = x[35 - i];
        }
        return Some((Layout::XTrans, pattern));
    }
    // R-RAF-19: 0x0131 absent and 0x0130 = 0A 0B 09 08: red at (even x,
    // even y), blue at (odd x, odd y), green otherwise.
    if records.get(tag::FUJI_LAYOUT) != Some(&BAYER_LAYOUT[..]) {
        return None;
    }
    for (i, p) in pattern.iter_mut().enumerate() {
        let (x, y) = (i % 6, i / 6);
        *p = match (x % 2, y % 2) {
            (0, 0) => 0,
            (1, 1) => 2,
            _ => 1,
        };
    }
    Some((Layout::Bayer, pattern))
}

/// The tag values of a file classified as a sensor variant.
#[derive(Clone, Debug)]
pub(super) struct Tags {
    /// Storage variant.
    pub variant: Variant,
    /// Color filter layout.
    pub layout: Layout,
    /// Colors of the 36 pattern positions.
    pub pattern: [u8; 36],
    /// Raster width W.
    pub w: u32,
    /// Raster height H.
    pub h: u32,
    /// BitsPerSample.
    pub bits: u32,
    /// StripOffsets, from the start of the CFA container.
    pub strip_offset: u32,
    /// StripByteCounts.
    pub strip_len: u32,
    /// Black level b (R-RAF-29).
    pub black: f64,
    /// 0xF00E G, R, B.
    pub wb: Option<[u32; 3]>,
    /// 0xF00D G, R, B.
    pub wb_auto: Option<[u32; 3]>,
    /// Meta record 0x0110: top, left.
    pub crop_top_left: Option<(u16, u16)>,
    /// Meta record 0x0111: height, width.
    pub crop_size: Option<(u16, u16)>,
}

/// The classification and, for a sensor variant, its tag values.
pub(super) struct Analysis {
    /// The class (R-RAF-3).
    pub class: Class,
    /// The tag values when the class is a sensor variant.
    pub tags: Option<Tags>,
}

impl From<Relative> for Analysis {
    fn from(r: Relative) -> Self {
        Analysis {
            class: Class::OutOfScope(r),
            tags: None,
        }
    }
}

/// White level w = 2^b - 1 (R-RAF-28).
pub(super) fn white_level(bits: u32) -> u32 {
    ((1u64 << bits.min(16)) - 1) as u32
}

/// Classify the file by the rules of R-RAF-3, in order, and the further
/// rules that make a file X6 (R-RAF-15, R-RAF-22, R-RAF-24, R-RAF-26,
/// R-RAF-30). A file that matches no rule is X6. An error means "fail"
/// (R-RAF-54, R-RAF-56, R-RAF-57, R-RAF-58).
pub(super) fn analyze(raf: &Raf<'_>) -> RafResult<Analysis> {
    use Relative::{X1, X2, X3, X4, X5, X6};
    // R-RAF-54: a file longer than 2^32 - 1 bytes fails.
    if raf.data.len() as u64 > u64::from(u32::MAX) {
        return Err(fail("file longer than 4 GiB"));
    }
    // Rule 1: format version not "0201".
    if raf.version != *b"0201" {
        return Ok(X3.into());
    }
    // Rule 2: RAFCompression.
    match raf.compression {
        0 => {}
        2 => return Ok(X1.into()),
        3 => return Ok(X2.into()),
        _ => return Ok(X6.into()),
    }
    // Rule 3: the CFA container is absent (R-RAF-6).
    let Some(cfa) = raf.part(raf.cfa) else {
        return Ok(X6.into());
    };
    // Rule 4: no little-endian TIFF header at the CFA container (OQ-12: a
    // big-endian container is X3 as well).
    if !cfa.starts_with(&[0x49, 0x49, 0x2A, 0x00]) {
        return Ok(X3.into());
    }
    // Rule 5: a second CFA container.
    if raf.second_cfa != 0 {
        return Ok(X4.into());
    }
    // Rule 6: the FujiIFD missing or invalid (R-RAF-10, R-RAF-13), or a
    // required tag absent (R-RAF-14).
    let Some(fuji) = fuji(raf) else {
        return Ok(X6.into());
    };
    let required = [
        tag::WIDTH,
        tag::HEIGHT,
        tag::BITS,
        tag::STRIP_OFFSETS,
        tag::STRIP_BYTE_COUNTS,
        tag::BLACK_LEVEL,
    ];
    if required.iter().any(|&t| fuji.entry(t).is_none()) {
        return Ok(X6.into());
    }
    let need = |t: u16| -> RafResult<u32> {
        fuji.first(t)?
            .ok_or_else(|| fail(format!("FujiIFD tag {t:#06X} has no value")))
    };
    let (w, h, bits) = (need(tag::WIDTH)?, need(tag::HEIGHT)?, need(tag::BITS)?);
    let (strip_offset, strip_len) = (need(tag::STRIP_OFFSETS)?, need(tag::STRIP_BYTE_COUNTS)?);
    let records = records(raf)?;
    // R-RAF-15: record 0x0100 with 4 or more bytes must hold H and W.
    if let Some((rh, rw)) = records.pair(tag::FULL_SIZE) {
        if u32::from(rh) != h || u32::from(rw) != w {
            return Ok(X6.into());
        }
    }
    // Rule 7: neither C1 nor C2.
    let Some((layout, pattern)) = self::layout(&records) else {
        return Ok(X5.into());
    };
    // Rules 8 to 10, exact integer products (R-RAF-59: exact byte counts).
    let (wl, hl, sl) = (u128::from(w), u128::from(h), u128::from(strip_len));
    let variant = if bits == 12 && 2 * sl == 3 * wl * hl {
        Variant::V1
    } else if bits == 14 && 4 * sl == 7 * wl * hl {
        Variant::V2
    } else if (bits == 14 || bits == 16) && sl == 2 * wl * hl {
        Variant::V3
    } else {
        return Ok(X6.into());
    };
    // R-RAF-24: V1 needs an even W. R-RAF-26: V2 needs W mod 16 = 0.
    if (variant == Variant::V1 && w % 2 != 0) || (variant == Variant::V2 && w % 16 != 0) {
        return Ok(X6.into());
    }
    // R-RAF-22, R-RAF-60: the strip lies inside the CFA container, which
    // lies inside the file (R-RAF-6).
    if u64::from(strip_offset) + u64::from(strip_len) > cfa.len() as u64 {
        return Ok(X6.into());
    }
    // R-RAF-29, R-RAF-30: the black level must be less than w - 255.
    let Some(black) = fuji.black_mean()? else {
        return Ok(X6.into());
    };
    if black >= f64::from(white_level(bits)) - 255.0 {
        return Ok(X6.into());
    }
    // R-RAF-32: 0xF00E and 0xF00D are G, R, B.
    let wb = fuji.three(tag::WB)?;
    let wb_auto = fuji.three(tag::WB_AUTO)?;
    Ok(Analysis {
        class: Class::Sensor(variant, layout),
        tags: Some(Tags {
            variant,
            layout,
            pattern,
            w,
            h,
            bits,
            strip_offset,
            strip_len,
            black,
            wb,
            wb_auto,
            crop_top_left: records.pair(tag::CROP_TOP_LEFT),
            crop_size: records.pair(tag::CROPPED_SIZE),
        }),
    })
}

/// The classification alone (R-RAF-3).
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn classify(raf: &Raf<'_>) -> RafResult<Class> {
    Ok(analyze(raf)?.class)
}

/// The generic WB multipliers R and B (R-RAF-33 source 3).
pub(super) const GENERIC_WB: (f64, f64) = (1.830, 1.849);

/// The camera-to-sRGB matrix of the specification (R-RAF-35): rows are
/// sRGB output channels, columns camera channels.
pub(super) const MATRIX: [[f64; 3]; 3] = [
    [1.400_839_3, -0.208_572_2, -0.192_267_1],
    [-0.204_991_3, 1.711_312_8, -0.506_321_5],
    [0.019_079_1, -0.529_412_4, 1.510_333_3],
];

/// R-RAF-36: the matrix with every row divided by its sum. R-RAF-62: a
/// row sum that is not greater than 0 leaves the row as it is (the fixed
/// matrix's rows all sum to 1).
pub(super) fn matrix() -> [[f64; 3]; 3] {
    MATRIX.map(|row| {
        let sum: f64 = row.iter().sum();
        if sum > 0.0 {
            row.map(|v| v / sum)
        } else {
            row
        }
    })
}

/// R-RAF-33: WB multipliers R, G, B from the first available source.
pub(super) fn white_balance(t: &Tags) -> ([f64; 3], WbSource) {
    // R-RAF-32, R-RAF-62: G, R, B with every value greater than 0.
    let ratios = |v: Option<[u32; 3]>| {
        let [g, r, b] = v?;
        (g > 0 && r > 0 && b > 0).then(|| {
            let g = f64::from(g);
            [f64::from(r) / g, 1.0, f64::from(b) / g]
        })
    };
    if let Some(m) = ratios(t.wb) {
        (m, WbSource::AsShot)
    } else if let Some(m) = ratios(t.wb_auto) {
        (m, WbSource::Auto)
    } else {
        ([GENERIC_WB.0, 1.0, GENERIC_WB.1], WbSource::Generic)
    }
}

/// R-RAF-55: W and H from 1 to 65535 and W * H at most 2^27.
pub(super) fn check_limits(w: u32, h: u32) -> RafResult<()> {
    if w == 0 || h == 0 || w > 65535 || h > 65535 || u64::from(w) * u64::from(h) > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    Ok(())
}

/// R-RAF-16: the crop rectangle when records 0x0110 and 0x0111 are both
/// present with 4 bytes or more, CW and CH are at least 16 and the
/// rectangle fits the raster; otherwise the whole raster.
pub(super) fn crop(t: &Tags) -> Rect {
    let (w, h) = (t.w as usize, t.h as usize);
    if let (Some((top, left)), Some((ch, cw))) = (t.crop_top_left, t.crop_size) {
        let (cx, cy, cw, ch) = (
            usize::from(left),
            usize::from(top),
            usize::from(cw),
            usize::from(ch),
        );
        if cw >= 16 && ch >= 16 && cx + cw <= w && cy + ch <= h {
            return Rect {
                x: cx,
                y: cy,
                w: cw,
                h: ch,
            };
        }
    }
    Rect { x: 0, y: 0, w, h }
}

/// Everything needed to develop the sensor data.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    /// The storage variant.
    pub variant: Variant,
    /// The color filter layout.
    pub layout: Layout,
    /// Color of position (x mod 6, y mod 6) at 6 * (y mod 6) + (x mod 6).
    pub pattern: [u8; 36],
    /// Raster width W.
    pub width: usize,
    /// Raster height H.
    pub height: usize,
    /// Final image area (R-RAF-16).
    pub crop: Rect,
    /// Exif orientation 1 to 8 (R-RAF-43).
    pub orientation: u8,
    /// BitsPerSample.
    pub bits: u32,
    /// White level w (R-RAF-28).
    pub white: u32,
    /// Black level b (R-RAF-29, R-RAF-31).
    pub black: f64,
    /// WB multipliers R, G, B (R-RAF-33).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// The normalized camera-to-sRGB matrix (R-RAF-35, R-RAF-36).
    pub matrix: [[f64; 3]; 3],
    /// File offset D of the strip (R-RAF-21).
    pub strip: usize,
    /// Row size R in bytes (R-RAF-21).
    pub row_bytes: usize,
}

/// Build and validate the plan of a sensor variant (step 6 and 7). Fails
/// (preview path) on any failed check.
pub(super) fn plan(raf: &Raf<'_>, t: &Tags, orientation: u8) -> RafResult<Plan> {
    check_limits(t.w, t.h)?;
    // R-RAF-61: BitsPerSample 12, 14 or 16, matching the variant.
    let bits_ok = match t.variant {
        Variant::V1 => t.bits == 12,
        Variant::V2 => t.bits == 14,
        Variant::V3 => t.bits == 14 || t.bits == 16,
    };
    if !bits_ok {
        return Err(fail("BitsPerSample does not match the variant"));
    }
    let (w, h) = (t.w as usize, t.h as usize);
    // R-RAF-21: R = 3W/2 (V1), 7W/4 (V2), 2W (V3).
    let row_bytes = match t.variant {
        Variant::V1 => 3 * w / 2,
        Variant::V2 => 7 * w / 4,
        Variant::V3 => 2 * w,
    };
    let cfa = raf.cfa.ok_or_else(|| fail("no CFA container"))?;
    // R-RAF-59: the rows read are exactly StripByteCounts bytes.
    if (row_bytes as u64) * (h as u64) != u64::from(t.strip_len) {
        return Err(fail("strip size does not match the raster"));
    }
    // R-RAF-60: StripOffsets + StripByteCounts inside the container.
    if u64::from(t.strip_offset) + u64::from(t.strip_len) > cfa.len as u64 {
        return Err(fail("strip outside the CFA container"));
    }
    // R-RAF-21, R-RAF-58: D = CFA container offset + StripOffsets, and the
    // strip lies inside the file.
    let strip = cfa
        .offset
        .checked_add(t.strip_offset as usize)
        .ok_or_else(|| fail("strip offset"))?;
    if strip as u64 + u64::from(t.strip_len) > raf.data.len() as u64 {
        return Err(fail("strip outside the file"));
    }
    let white = white_level(t.bits);
    // R-RAF-30, R-RAF-62: w - b > 255 > 0, so the normalization divisor is
    // positive.
    if t.black >= f64::from(white) - 255.0 || !t.black.is_finite() {
        return Err(fail("black level not below the white level"));
    }
    let (wb, wb_source) = white_balance(t);
    Ok(Plan {
        variant: t.variant,
        layout: t.layout,
        pattern: t.pattern,
        width: w,
        height: h,
        crop: crop(t),
        orientation,
        bits: t.bits,
        white,
        black: t.black,
        wb,
        wb_source,
        matrix: matrix(),
        strip,
        row_bytes,
    })
}

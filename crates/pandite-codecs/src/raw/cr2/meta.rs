//! The container of a CR2 file (header, IFD chain, raw IFD, Exif IFD and
//! MakerNote), its classification (R-CR2-3) and the decode plan: sizes,
//! slices, visible area, CFA, black rectangle, white balance, color matrix
//! and orientation, all validated before any large allocation.

use super::color;
use super::{fail, Cr2Result};
use crate::raw::arw::ifd::{self, Dir};
use crate::raw::arw::lj92::{self, Stream};
use crate::raw::arw::WbSource;

/// Tag numbers of the specification's Metadata tags section.
pub(super) mod tag {
    pub const COMPRESSION: u16 = 0x0103;
    pub const MODEL: u16 = 0x0110;
    pub const STRIP_OFFSETS: u16 = 0x0111;
    pub const ORIENTATION: u16 = 0x0112;
    pub const STRIP_BYTE_COUNTS: u16 = 0x0117;
    pub const JPEG_OFFSET: u16 = 0x0201;
    pub const JPEG_LENGTH: u16 = 0x0202;
    pub const EXIF_IFD: u16 = 0x8769;
    pub const MAKER_NOTE: u16 = 0x927C;
    pub const CFA_PATTERN: u16 = 0xC5E0;
    pub const SLICES: u16 = 0xC640;
    pub const SRAW_TYPE: u16 = 0xC6C5;
    pub const MODEL_ID: u16 = 0x0010;
    pub const SENSOR_INFO: u16 = 0x00E0;
    pub const COLOR_DATA: u16 = 0x4001;
}

/// The result of classification (R-CR2-3). X3 (not a CR2 file) is
/// recognized from the header before classification: [`open`] returns
/// `Ok(None)` for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Class {
    /// RAW: one CFA sample per raster pixel, in scope.
    V1,
    /// sRAW1 (M-RAW) and sRAW2 (S-RAW): YCbCr samples, preview path.
    X1,
    /// Any other stream or a broken raw IFD: preview path.
    X2,
}

/// The frame layouts of V1 (section Scope and variants). They only matter
/// for the tests and reports: the decoding is the same for all three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    /// Nf = 2, X * Nf = W.
    A,
    /// Nf = 4, X * Nf = W.
    B,
    /// Nf = 4, X * Nf = 2 * W: one frame line holds two raster rows.
    C,
}

/// The parsed container.
pub(super) struct Container<'a> {
    /// The file bytes.
    pub data: &'a [u8],
    /// IFD0 and the IFDs that follow it (R-CR2-9).
    pub chain: Vec<Dir<'a>>,
    /// The raw IFD at the offset in header bytes 12 to 15, when valid
    /// (R-CR2-8, R-CR2-7).
    pub raw: Option<Dir<'a>>,
    /// The MakerNote IFD, when found (R-CR2-10).
    pub maker: Option<Dir<'a>>,
}

/// Parse the header and the IFDs. `Ok(None)` for a file that is not a CR2
/// file (X3, R-CR2-2): the caller then uses its generic preview search.
pub(super) fn open(data: &[u8]) -> Cr2Result<Option<Container<'_>>> {
    // R-CR2-4, R-CR2-3 rule 1: bytes 0 to 3 are 49 49 2A 00 and bytes 8 to
    // 10 are 43 52 02; any byte 11. Big-endian files are X3 (R-CR2-5).
    let signature = data.get(..4) == Some(&[0x49, 0x49, 0x2A, 0x00][..])
        && data.get(8..11) == Some(&[0x43, 0x52, 0x02][..]);
    if !signature {
        return Ok(None);
    }
    // R-CR2-43: shorter than 16 bytes or longer than 2^32 - 1 bytes fails.
    if data.len() < 16 || data.len() as u64 > u64::from(u32::MAX) {
        return Err(fail("file size outside 16 bytes to 4 GiB"));
    }
    // R-CR2-5: every multi-byte field is little-endian.
    let first = ifd::le32(data, 4).unwrap_or(0);
    let raw_at = ifd::le32(data, 12).unwrap_or(0);
    // R-CR2-9: IFD0 and the next-IFD offsets, stopping at 0, at an invalid
    // IFD, at a repeated offset or after 16 IFDs (the rules of R-ARW-12).
    let mut chain = ifd::chain(data, first);
    // R-CR2-11: the next-IFD field of the raw IFD is not followed, so the
    // chain ends at the raw IFD when it reaches it (IFD3 in real files).
    let mut at = first;
    let mut keep = chain.len();
    for (i, d) in chain.iter().enumerate() {
        if at == raw_at {
            keep = i + 1;
            break;
        }
        at = d.next;
    }
    chain.truncate(keep);
    // R-CR2-8: the raw IFD is the IFD at the offset in bytes 12 to 15,
    // whether or not the chain reaches it. R-CR2-7: invalid IFDs are None.
    let raw = ifd::dir(data, u64::from(raw_at));
    let maker = chain.first().and_then(|ifd0| maker_note(data, ifd0));
    Ok(Some(Container {
        data,
        chain,
        raw,
        maker,
    }))
}

/// R-CR2-10: the Exif IFD at the offset in IFD0 tag 0x8769, then the IFD
/// at the value offset of its tag 0x927C. Its value offsets count from
/// byte 0 of the file like all others; R-CR2-11: its next-IFD field is
/// never used.
fn maker_note<'a>(data: &'a [u8], ifd0: &Dir<'a>) -> Option<Dir<'a>> {
    let exif_at = ifd0.uint(tag::EXIF_IFD).ok().flatten()?;
    let exif = ifd::dir(data, u64::from(exif_at))?;
    let e = exif.entry(tag::MAKER_NOTE)?;
    // A value of 4 bytes or less is inline (R-ARW-6) and has no value
    // offset; it could not hold an IFD either.
    if e.len <= 4 {
        return None;
    }
    ifd::dir(data, e.at)
}

impl Container<'_> {
    /// IFD0, when valid.
    pub(super) fn ifd0(&self) -> Option<&Dir<'_>> {
        self.chain.first()
    }

    /// The IFD0 Model string, trimmed (R-CR2-6, R-ARW-10). Diagnostic only.
    pub(super) fn model(&self) -> String {
        let m = self
            .ifd0()
            .and_then(|d| d.ascii(tag::MODEL).ok().flatten())
            .unwrap_or_default();
        String::from_utf8_lossy(m).into_owned()
    }

    /// R-CR2-36: the IFD0 Orientation, 1 when absent or outside 1 to 8.
    pub(super) fn orientation(&self) -> u8 {
        match self.ifd0().map(|d| d.uint(tag::ORIENTATION)) {
            Some(Ok(Some(v @ 1..=8))) => v as u8,
            _ => 1,
        }
    }

    /// The byte range of the raw strip: one StripOffsets value and one
    /// StripByteCounts value (R-CR2-12), inside the file (R-CR2-46, 64-bit
    /// sum by R-CR2-43).
    pub(super) fn strip(&self) -> Cr2Result<(usize, usize)> {
        let raw = self.raw.as_ref().ok_or_else(|| fail("no valid raw IFD"))?;
        let offsets = raw
            .uints(tag::STRIP_OFFSETS)?
            .ok_or_else(|| fail("required tag StripOffsets missing"))?;
        let counts = raw
            .uints(tag::STRIP_BYTE_COUNTS)?
            .ok_or_else(|| fail("required tag StripByteCounts missing"))?;
        if offsets.len() != 1 || counts.len() != 1 {
            return Err(fail("the raw IFD does not have exactly one strip"));
        }
        let (off, len) = (u64::from(offsets[0]), u64::from(counts[0]));
        if off + len > self.data.len() as u64 {
            return Err(fail("the raw strip lies outside the file"));
        }
        Ok((off as usize, len as usize))
    }

    /// The LJ92 frame and scan headers of the raw strip.
    pub(super) fn stream(&self) -> Cr2Result<Stream<'_>> {
        let (off, len) = self.strip()?;
        lj92::parse(&self.data[off..off + len])
    }

    /// The LJ92 headers from the part of the raw strip that lies inside
    /// the data: header facts of a file head, never used for decoding.
    pub(super) fn stream_head(&self) -> Option<Stream<'_>> {
        let raw = self.raw.as_ref()?;
        let offsets = raw.uints(tag::STRIP_OFFSETS).ok()??;
        let counts = raw.uints(tag::STRIP_BYTE_COUNTS).ok()??;
        if offsets.len() != 1 || counts.len() != 1 {
            return None;
        }
        let end = (u64::from(offsets[0]) + u64::from(counts[0])).min(self.data.len() as u64);
        let bytes = self.data.get(offsets[0] as usize..end as usize)?;
        lj92::parse(bytes).ok()
    }
}

/// R-CR2-21: the frame and scan of a V1 stream.
pub(super) fn is_v1_stream(s: &Stream<'_>) -> bool {
    let nf = s.components.len();
    // Case 1: P 12 or 14, Nf 2 or 4, H = V = 1. One SOF3 header: the
    // parser fails on a second one.
    (s.precision == 12 || s.precision == 14)
        && (nf == 2 || nf == 4)
        && s.components.iter().all(|c| c.h == 1 && c.v == 1)
        // Case 2: one scan with every component once (the parser refuses a
        // repeated or unknown component), Ss = 1, Se = 0, Ah = 0, Al = 0.
        && s.scan.len() == nf
        && s.predictor == 1
        && s.se == 0
        && s.ah == 0
        && s.pt == 0
        // Case 3: X >= 1 and Y >= 1.
        && s.samples >= 1
        && s.lines >= 1
}

/// Classify the file by the rules of R-CR2-3, in order (rule 1, X3, is
/// applied by [`open`]).
pub(super) fn classify(c: &Container<'_>) -> Class {
    classify_with(c, c.stream().ok().as_ref())
}

/// [`classify`] with the stream headers already parsed (`None` when they
/// cannot be parsed).
pub(super) fn classify_with(c: &Container<'_>, stream: Option<&Stream<'_>>) -> Class {
    // Rule 2: the raw IFD is invalid, lacks StripOffsets or
    // StripByteCounts, or Compression is not 6.
    let Some(raw) = &c.raw else {
        return Class::X2;
    };
    let compression = raw.uint(tag::COMPRESSION).ok().flatten();
    if compression != Some(6) || !raw.has(tag::STRIP_OFFSETS) || !raw.has(tag::STRIP_BYTE_COUNTS) {
        return Class::X2;
    }
    // Rule 3: tag 0xC6C5 present with a value other than 1. A value that
    // cannot be read makes the raw IFD invalid (rule 2).
    match raw.uint(tag::SRAW_TYPE) {
        Ok(None | Some(1)) => {}
        Ok(Some(_)) => return Class::X1,
        Err(_) => return Class::X2,
    }
    // Rules 4 to 6 read the frame header. A stream whose headers cannot be
    // parsed meets neither rule 4 nor rule 5: rule 6.
    let Some(s) = stream else {
        return Class::X2;
    };
    // Rule 4.
    if s.components.len() == 3 || s.components.iter().any(|k| k.h != 1 || k.v != 1) {
        return Class::X1;
    }
    // Rules 5 and 6.
    if is_v1_stream(s) {
        Class::V1
    } else {
        Class::X2
    }
}

/// A rectangle of raster coordinates, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    /// First column.
    pub left: usize,
    /// First row.
    pub top: usize,
    /// Last column.
    pub right: usize,
    /// Last row.
    pub bottom: usize,
}

impl Area {
    /// Width: right - left + 1.
    pub(crate) fn width(&self) -> usize {
        self.right - self.left + 1
    }

    /// Height: bottom - top + 1.
    pub(crate) fn height(&self) -> usize {
        self.bottom - self.top + 1
    }
}

/// Everything needed to decode and develop a V1 file.
#[derive(Clone, Debug)]
pub(super) struct Plan {
    /// Byte range of the raw strip.
    pub strip: (usize, usize),
    /// Sample precision P.
    pub precision: u32,
    /// Samples per frame line X, frame lines Y, components Nf.
    pub frame: (usize, usize, usize),
    /// Raster width W (SensorInfo index 1).
    pub width: usize,
    /// Raster height H (SensorInfo index 2).
    pub height: usize,
    /// Tag 0xC640 n, a, b when n >= 1; `None` for one slice of width W.
    pub slices: Option<(usize, usize, usize)>,
    /// The visible area L, T, R, B (R-CR2-18).
    pub visible: Area,
    /// Tag 0xC5E0 (1 to 4).
    pub cfa_value: u32,
    /// Colors (0 red, 1 green, 2 blue) of CFA positions 0 to 3 at raster
    /// coordinates (R-CR2-19).
    pub cfa: [u8; 4],
    /// The black rectangle (R-CR2-24).
    pub black_area: Area,
    /// WB multipliers R, G, B (R-CR2-28).
    pub wb: [f64; 3],
    /// Where `wb` came from.
    pub wb_source: WbSource,
    /// Normalized camera-to-sRGB matrix (R-CR2-30, R-CR2-31).
    pub matrix: [[f64; 3]; 3],
    /// True when the matrix came from the model table.
    pub matrix_from_table: bool,
    /// IFD0 orientation 1 to 8 (R-CR2-36).
    pub orientation: u8,
    /// The IFD0 Model string.
    pub model: String,
    /// The frame layout, when it is one of A, B and C.
    pub layout: Option<Layout>,
}

/// Largest W * H accepted (R-CR2-44).
const MAX_PIXELS: u64 = 1 << 27;

/// R-CR2-13: ColorData as signed 16-bit little-endian values: count values
/// for SHORT or SSHORT, floor(count / 2) for UNDEFINED, absent for other
/// types. A value range outside the file fails (R-CR2-45).
pub(super) fn color_data(maker: &Dir<'_>) -> Cr2Result<Option<Vec<i32>>> {
    let Some(e) = maker.entry(tag::COLOR_DATA) else {
        return Ok(None);
    };
    let n = match e.typ {
        3 | 8 => e.count as usize,
        7 => e.count as usize / 2,
        _ => return Ok(None),
    };
    // R-CR2-45: the byte range is checked against the file before the
    // values are collected.
    let bytes = maker.bytes(e)?;
    Ok(Some(
        bytes
            .chunks_exact(2)
            .take(n)
            .map(|c| i32::from(i16::from_le_bytes([c[0], c[1]])))
            .collect(),
    ))
}

/// The frame layout of a V1 frame.
pub(super) fn layout(x: usize, nf: usize, w: usize) -> Option<Layout> {
    match (nf, x.checked_mul(nf)?) {
        (2, xn) if xn == w => Some(Layout::A),
        (4, xn) if xn == w => Some(Layout::B),
        (4, xn) if Some(xn) == w.checked_mul(2) => Some(Layout::C),
        _ => None,
    }
}

/// R-CR2-16: the slices of tag 0xC640 (`None` when absent) for raster
/// width `w`: n, a, b when n >= 1, `None` for one slice of width W.
pub(super) fn slices(tag: Option<&[u32]>, w: u64) -> Cr2Result<Option<(usize, usize, usize)>> {
    match tag {
        // Absent, or n = 0 (b is not used then, M3): one slice.
        None => Ok(None),
        Some(v) if v.first().is_none_or(|&n| n == 0) => Ok(None),
        Some(&[n, a, b, ..]) => {
            let (n, a, b) = (u64::from(n), u64::from(a), u64::from(b));
            // n * a + b = W with a >= 1 and b >= 1, else the preview path.
            let sum = n.checked_mul(a).and_then(|v| v.checked_add(b));
            if a < 1 || b < 1 || sum != Some(w) {
                return Err(fail("slice widths do not add up to the raster width"));
            }
            Ok(Some((n as usize, a as usize, b as usize)))
        }
        // n >= 1 with fewer than 3 values: the specification gives no
        // slice widths for it; preview path.
        Some(_) => Err(fail("slice tag with n >= 1 and fewer than 3 values")),
    }
}

/// R-CR2-19: the colors (0 red, 1 green, 2 blue) of CFA positions 0 to 3
/// for tag 0xC5E0; any value outside 1 to 4 fails (preview path).
pub(super) fn cfa_colors(value: u32) -> Cr2Result<[u8; 4]> {
    match value {
        1 => Ok([0, 1, 1, 2]),
        2 => Ok([2, 1, 1, 0]),
        3 => Ok([1, 2, 0, 1]),
        4 => Ok([1, 0, 2, 1]),
        _ => Err(fail("CR2CFAPattern outside 1 to 4")),
    }
}

/// The visible area (R-CR2-18) and the black rectangle (R-CR2-24) from
/// SensorInfo `si` (at least 9 values) of a W x H raster.
pub(super) fn areas(si: &[u32], w: usize, h: usize) -> Cr2Result<(Area, Area)> {
    if si.len() < 9 {
        return Err(fail("SensorInfo has fewer than 9 values"));
    }
    // R-CR2-18: 0 <= L <= R < W, 0 <= T <= B < H, at least 2 x 2.
    let (l, t, r, b) = (
        si[5] as usize,
        si[6] as usize,
        si[7] as usize,
        si[8] as usize,
    );
    if !(l <= r && r < w && t <= b && b < h && r - l + 1 >= 2 && b - t + 1 >= 2) {
        return Err(fail("visible area outside the raster"));
    }
    let visible = Area {
        left: l,
        top: t,
        right: r,
        bottom: b,
    };
    // R-CR2-14: indices 9 to 12 only with at least 13 values, else 0.
    let mask = if si.len() >= 13 {
        [si[9], si[10], si[11], si[12]].map(|v| v as usize)
    } else {
        [0; 4]
    };
    // R-CR2-24 rule 1: the SensorInfo rectangle when not all 0, with
    // 0 <= BL < BR < W and 0 <= BT < BB < H, and not overlapping the
    // visible area.
    let [bl, bt, br, bb] = mask;
    let overlaps = bl <= r && br >= l && bt <= b && bb >= t;
    let black = if mask != [0; 4] && bl < br && br < w && bt < bb && bb < h && !overlaps {
        Area {
            left: bl,
            top: bt,
            right: br,
            bottom: bb,
        }
    } else if l >= 8 {
        // Rule 2: columns 0 to floor(L / 2) - 1, rows T to B.
        Area {
            left: 0,
            top: t,
            right: l / 2 - 1,
            bottom: b,
        }
    } else {
        // R-CR2-24: rule 1 does not apply and L < 8.
        return Err(fail("no black rectangle: rule 1 does not apply and L < 8"));
    };
    Ok((visible, black))
}

/// Build and validate the plan of a V1 file. Fails (preview path) on any
/// missing required tag or failed check.
pub(super) fn plan(c: &Container<'_>) -> Cr2Result<Plan> {
    let raw = c.raw.as_ref().ok_or_else(|| fail("no valid raw IFD"))?;
    // R-CR2-12: Compression, StripOffsets and StripByteCounts present, one
    // strip.
    if raw.uint(tag::COMPRESSION)? != Some(6) {
        return Err(fail("raw IFD Compression is not 6"));
    }
    let strip = c.strip()?;
    let s = c.stream()?;
    if !is_v1_stream(&s) {
        return Err(fail("the stream differs from R-CR2-21"));
    }
    let (x, y, nf) = (s.samples, s.lines, s.components.len());
    // R-CR2-12: the MakerNote and SensorInfo with at least 9 values.
    let maker = c
        .maker
        .as_ref()
        .ok_or_else(|| fail("MakerNote not found"))?;
    // R-CR2-14: SensorInfo as unsigned 16-bit values.
    let si = maker
        .uints(tag::SENSOR_INFO)?
        .ok_or_else(|| fail("SensorInfo missing"))?;
    if si.len() < 9 {
        return Err(fail("SensorInfo has fewer than 9 values"));
    }
    // R-CR2-15: W and H from SensorInfo indices 1 and 2.
    let (w, h) = (u64::from(si[1]), u64::from(si[2]));
    // R-CR2-44: limits on W, H and W * H.
    if w == 0 || h == 0 || w > 65535 || h > 65535 || w * h > MAX_PIXELS {
        return Err(fail("raster size outside the limits"));
    }
    // R-CR2-15: W * H equals X * Y * Nf of the frame.
    let samples = (x as u64) * (y as u64) * (nf as u64);
    if w * h != samples {
        return Err(fail("SensorInfo raster size differs from the frame"));
    }
    // Every sample takes at least one bit (each Huffman code is 1 to 16
    // bits), so a strip with fewer bits than samples cannot decode (R-CR2-50)
    // and is refused before the raster is allocated: the allocation is
    // bounded by the strip size.
    if samples > 8 * strip.1 as u64 {
        return Err(fail("the raw strip is too short for the raster"));
    }
    let (wu, hu) = (w as usize, h as usize);

    let slices = slices(raw.uints(tag::SLICES)?.as_deref(), w)?;

    let (visible, black_area) = areas(&si, wu, hu)?;
    let cfa_value = raw.uint(tag::CFA_PATTERN)?.unwrap_or(1);
    let cfa = cfa_colors(cfa_value)?;

    // R-CR2-13, R-CR2-27, R-CR2-28: white balance.
    let cd = color_data(maker)?;
    // R-CR2-30: CanonModelID.
    let model_id = maker.uint(tag::MODEL_ID)?;
    let (wb, wb_source) = color::white_balance(cd.as_deref(), model_id);
    let (matrix, matrix_from_table) = color::matrix(model_id);

    Ok(Plan {
        strip,
        precision: s.precision,
        frame: (x, y, nf),
        width: wu,
        height: hu,
        slices,
        visible,
        cfa_value,
        cfa,
        black_area,
        wb,
        wb_source,
        matrix,
        matrix_from_table,
        orientation: c.orientation(),
        model: c.model(),
        layout: layout(x, nf, wu),
    })
}

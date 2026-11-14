//! Step 8 to 10 of the decoding procedure: the LJ92 stream decoded by the
//! shared T.81 decoder into the sample sequence, the slices put together
//! into the raster, the black and white levels measured in the raster,
//! and the visible area cut out for the shared rendering.

use super::meta::{is_v1_stream, Area, Plan};
use super::{fail, Cr2Result};
use crate::raw::arw::lj92::{self, LineStart, Stream, Target};
use crate::raw::arw::plane::{self, Plane};
use pandite_core::CancelToken;

/// The reassembled raster: W x H sensor values, row-major.
pub(super) struct Raster {
    /// The values.
    pub data: Vec<u16>,
    /// Raster width W.
    pub width: usize,
    /// Raster height H.
    pub height: usize,
}

impl Raster {
    /// The value at raster (x, y).
    #[inline]
    pub(super) fn at(&self, x: usize, y: usize) -> u16 {
        self.data[y * self.width + x]
    }
}

/// Decode the raw strip and place every sample in the raster (R-CR2-20,
/// R-CR2-22, R-CR2-23, R-CR2-17).
pub(super) fn decode(data: &[u8], plan: &Plan, cancel: &CancelToken) -> Cr2Result<Raster> {
    cancel.check()?;
    let (off, len) = plan.strip;
    // R-CR2-50: the decoder reads nothing outside the strip.
    let bytes = data
        .get(off..off.saturating_add(len))
        .ok_or_else(|| fail("the raw strip lies outside the file"))?;
    let s = lj92::parse(bytes)?;
    // The stream must still be the one the plan was made for (R-CR2-21),
    // whose X * Y * Nf equals W * H (R-CR2-15) within the limits of
    // R-CR2-44: that bounds the allocation of the sample sequence.
    if !is_v1_stream(&s) || (s.samples, s.lines, s.components.len()) != plan.frame {
        return Err(fail("the stream differs from the plan"));
    }
    let seq = sequence(&s, cancel)?;
    if plan.width.checked_mul(plan.height) != Some(seq.len()) {
        return Err(fail("the frame differs from the raster size"));
    }
    // R-CR2-17: the sample sequence fills the slices one after the other.
    let data = match plan.slices {
        None => seq,
        Some((n, a, b)) => reassemble(&seq, plan.width, plan.height, (n, a, b), cancel)?,
    };
    Ok(Raster {
        data,
        width: plan.width,
        height: plan.height,
    })
}

/// R-CR2-22: where the shared decoder writes each frame component (the
/// targets are indexed like the frame components): sample (u, v) of the
/// component at scan position k (0-based) goes to sequence index
/// s = (v * X + u) * Nf + k, that is base k, Nf between MCUs and X * Nf
/// between frame lines.
pub(super) fn targets(s: &Stream<'_>) -> Cr2Result<Vec<Target>> {
    let nf = s.components.len();
    let line = s
        .samples
        .checked_mul(nf)
        .ok_or_else(|| fail("frame line size"))?;
    let mut targets = vec![
        Target {
            buffer: 0,
            base: 0,
            xs: nf,
            ys: line,
        };
        nf
    ];
    for (k, &(fi, _)) in s.scan.iter().enumerate() {
        let t = targets.get_mut(fi).ok_or_else(|| fail("scan component"))?;
        t.base = k;
    }
    Ok(targets)
}

/// Decode a V1 stream into its sample sequence of X * Y * Nf values
/// (R-CR2-22). R-CR2-20: T.81 Annex H with the line-start rule of H.1.2.1
/// for every line (no Sony exception), reconstruction modulo 2^16,
/// shifted left by Pt. R-CR2-23: DRI and RST markers are handled by the
/// shared decoder, which resets the predictions at each restart interval.
/// The shared parser and decoder fail on the stream faults of R-CR2-47
/// (no SOI, a missing or short SOF3, DHT or SOS, an undefined table),
/// R-CR2-48 (DHT counts above 256 or the bytes left, overflowing codes),
/// R-CR2-49 (a code not in the table, SSSS above 16, a marker other than
/// RST before the last MCU) and R-CR2-50 (bits needed beyond the data,
/// while unconsumed look-ahead bits never fail). The caller bounds
/// X * Y * Nf before calling.
pub(super) fn sequence(s: &Stream<'_>, cancel: &CancelToken) -> Cr2Result<Vec<u16>> {
    let total = s
        .samples
        .checked_mul(s.lines)
        .and_then(|v| v.checked_mul(s.components.len()))
        .ok_or_else(|| fail("frame size"))?;
    let targets = targets(s)?;
    let mut seq = plane::zeroed(total)?;
    s.decode_cancellable(&mut [&mut seq[..]], &targets, LineStart::T81, Some(cancel))?;
    Ok(seq)
}

/// R-CR2-17 for n >= 1: sample s goes to slice j = min(n, floor(s / (a *
/// H))), index q = s - j * a * H inside it, slice width w (a for j < n, b
/// for j = n), raster (j * a + q mod w, floor(q / w)). Slice j holds the
/// samples j * a * H to j * a * H + w * H - 1, row by row.
pub(super) fn reassemble(
    seq: &[u16],
    w: usize,
    h: usize,
    (n, a, b): (usize, usize, usize),
    cancel: &CancelToken,
) -> Cr2Result<Vec<u16>> {
    // The plan checked n * a + b = W, a >= 1 and b >= 1 (R-CR2-16).
    let check = n
        .checked_mul(a)
        .and_then(|v| v.checked_add(b))
        .and_then(|v| v.checked_mul(h));
    if check != Some(seq.len()) || w.checked_mul(h) != Some(seq.len()) {
        return Err(fail("slices do not cover the raster"));
    }
    let mut out = plane::zeroed(seq.len())?;
    let mut start = 0usize;
    for j in 0..=n {
        cancel.check()?;
        let sw = if j < n { a } else { b };
        let x0 = j * a;
        for (y, src) in seq[start..start + sw * h].chunks_exact(sw).enumerate() {
            let at = y * w + x0;
            out[at..at + sw].copy_from_slice(src);
        }
        start += sw * h;
    }
    Ok(out)
}

/// The levels measured in the raster (R-CR2-25, R-CR2-26).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Levels {
    /// Black level of CFA positions 0 to 3 at raster coordinates.
    pub black: [u32; 4],
    /// White level w.
    pub white: u32,
}

/// R-CR2-25: the black level of each CFA position: the lower median (rank
/// floor((n - 1) / 2) of the ascending values) of the raster samples of
/// that position inside the black rectangle. Counted in a histogram, so
/// no copy of the rectangle is held.
pub(super) fn black_levels(r: &Raster, area: Area, cancel: &CancelToken) -> Cr2Result<[u32; 4]> {
    if area.right >= r.width || area.bottom >= r.height {
        return Err(fail("black rectangle outside the raster"));
    }
    let mut hist = vec![[0u32; 4]; 1 << 16];
    let mut count = [0u64; 4];
    for y in area.top..=area.bottom {
        if (y - area.top).is_multiple_of(256) {
            cancel.check()?;
        }
        for x in area.left..=area.right {
            let p = 2 * (y & 1) + (x & 1);
            hist[usize::from(r.at(x, y))][p] += 1;
            count[p] += 1;
        }
    }
    let mut out = [0u32; 4];
    for p in 0..4 {
        if count[p] == 0 {
            return Err(fail("a CFA position has no sample in the black rectangle"));
        }
        let rank = (count[p] - 1) / 2;
        let mut seen = 0u64;
        for (v, bins) in hist.iter().enumerate() {
            seen += u64::from(bins[p]);
            if seen > rank {
                out[p] = v as u32;
                break;
            }
        }
    }
    Ok(out)
}

/// R-CR2-26: the white level from the visible area: w = M when at least
/// 16 visible samples equal the largest value M and M >= 2^(P - 1), else
/// 2^P - 1. Fails (preview path) when w <= max(b) + 255.
pub(super) fn white_level(r: &Raster, plan: &Plan, black: [u32; 4]) -> Cr2Result<u32> {
    let v = plan.visible;
    if v.right >= r.width || v.bottom >= r.height {
        return Err(fail("visible area outside the raster"));
    }
    let (mut m, mut c) = (0u16, 0u64);
    for y in v.top..=v.bottom {
        let row = &r.data[y * r.width + v.left..=y * r.width + v.right];
        for &s in row {
            if s > m {
                m = s;
                c = 1;
            } else if s == m {
                c += 1;
            }
        }
    }
    let p = plan.precision;
    let w = if c >= 16 && u32::from(m) >= 1 << (p - 1) {
        u32::from(m)
    } else {
        (1 << p) - 1
    };
    let max_black = black.iter().copied().max().unwrap_or(0);
    if w <= max_black + 255 {
        return Err(fail("white level not above the black level plus 255"));
    }
    Ok(w)
}

/// Cut the visible area out of the raster in place (R-CR2-18: output
/// pixel (i, j) is raster (L + i, T + j)) and return it as a row-major
/// plane for the shared rendering.
pub(super) fn visible_plane(r: Raster, v: Area) -> Cr2Result<Plane> {
    let (vw, vh) = (v.width(), v.height());
    if v.right >= r.width || v.bottom >= r.height {
        return Err(fail("visible area outside the raster"));
    }
    let w = r.width;
    let mut data = r.data;
    // Row j moves from (T + j) * W + L to j * VW: never forward, and never
    // onto a row that is still to be moved, because VW <= W.
    for j in 0..vh {
        let src = (v.top + j) * w + v.left;
        data.copy_within(src..src + vw, j * vw);
    }
    data.truncate(vw * vh);
    Ok(Plane {
        data,
        tw: vw,
        th: vh,
        across: 1,
    })
}

/// A 2 x 2 table of raster CFA positions seen from the visible area
/// origin (L, T): entry 2 * j + i is the raster position of visible pixel
/// (i, j). R-CR2-35: the CFA phase inside the visible area follows from L
/// and T.
pub(super) fn phase<T: Copy>(table: [T; 4], v: Area) -> [T; 4] {
    let mut out = table;
    for j in 0..2 {
        for i in 0..2 {
            out[2 * j + i] = table[2 * ((v.top + j) & 1) + ((v.left + i) & 1)];
        }
    }
    out
}

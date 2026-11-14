//! From decoded sensor values to the final image: normalization and white
//! balance (R-ARW-47, R-ARW-48), bilinear demosaicing inside the valid
//! area (R-ARW-54, R-ARW-55, R-ARW-56), the V5 color conversion (R-ARW-41,
//! R-ARW-42), the color matrix (R-ARW-52), the crop (R-ARW-21) and the
//! orientation (R-ARW-57, R-ARW-58).
//!
//! The work is split into bands of output rows. Each band maps back to a
//! rectangle of the cropped source, which it walks row by row with a
//! three-row window of normalized values, so every output byte is written
//! by exactly one thread and no full-size intermediate is held.

use super::meta::{Plan, Rect, Variant, Ycc};
use super::plane::Plane;
use super::tiles::YccPlanes;
use super::{fail, pool, ArwResult};
use pandite_core::CancelToken;
use std::sync::OnceLock;

/// Decoded sensor values.
pub(crate) enum Sensor {
    /// One CFA value per pixel on the 14-bit scale (V1 to V4).
    Cfa(Plane),
    /// Y, Cb and Cr planes (V5).
    Ycc(YccPlanes),
}

/// The output size for a crop and an orientation: W' x H' for 1 to 4,
/// H' x W' for 5 to 8 (R-ARW-58).
pub(crate) fn output_size(crop: Rect, orientation: u8) -> (usize, usize) {
    if orientation >= 5 {
        (crop.h, crop.w)
    } else {
        (crop.w, crop.h)
    }
}

/// R-ARW-58: the output pixel of stored pixel (x, y) of a W' x H' image.
#[inline]
fn map(o: u8, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        8 => (y, w - 1 - x),
        _ => (x, y),
    }
}

/// The 16-bit sRGB code of every linear value k / 65535.
pub(super) fn srgb_table() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|v| (crate::util::linear_to_srgb(f32::from(v) / 65535.0) * 65535.0 + 0.5) as u16)
            .collect()
    })
}

/// R-ARW-42: camera RGB from Y, Cb and Cr with the coefficients LR, LG,
/// LB and the chroma zero level Z (unit chroma scale).
#[inline]
pub(super) fn ycc_rgb(y: f64, cb: f64, cr: f64, p: &Ycc) -> [f64; 3] {
    let [lr, lg, lb] = p.luma;
    let cb = cb - p.zero[0];
    let cr = cr - p.zero[1];
    let r = y + (2.0 - 2.0 * lr) * cr;
    let b = y + (2.0 - 2.0 * lb) * cb;
    // R-ARW-78: LG > 0 is checked when the plan is made.
    let g = (y - lb * b - lr * r) / lg;
    [r, g, b]
}

/// R-ARW-48: v = clip((c - 1024) / (16383 - b) * f, 0, 1) per channel,
/// without WB multipliers (R-ARW-49).
#[inline]
pub(super) fn v5_levels(rgb: [f64; 3], p: &Ycc) -> [f32; 3] {
    // R-ARW-78: 16383 - b > 0 is checked when the plan is made.
    let span = 16383.0 - p.black;
    let mut v = [0f32; 3];
    for c in 0..3 {
        v[c] = ((rgb[c] - 1024.0) / span * p.factors[c]).clamp(0.0, 1.0) as f32;
    }
    v
}

/// Precomputed per-file constants.
struct Ctx<'a> {
    sensor: &'a Sensor,
    /// Valid area width and height.
    vw: usize,
    vh: usize,
    crop: Rect,
    orientation: u8,
    /// Color (0 red, 1 green, 2 blue) of CFA positions 0 to 3.
    cfa: [usize; 4],
    /// The two greens are diagonal neighbors (all real Bayer sensors):
    /// the fixed-neighbor formulas apply inside the valid area.
    diagonal: bool,
    /// Black level of each CFA position.
    black: [f32; 4],
    /// m / (w - b) of each CFA position.
    scale: [f32; 4],
    /// The normalized matrix (R-ARW-51).
    m: [[f32; 3]; 3],
    /// V5 conversion parameters.
    ycc: Ycc,
}

impl<'a> Ctx<'a> {
    fn new(plan: &Plan, sensor: &'a Sensor) -> Self {
        let cfa = plan.cfa.map(usize::from);
        let diagonal = (cfa[0] == 1 && cfa[3] == 1) || (cfa[1] == 1 && cfa[2] == 1);
        let w = f64::from(plan.white);
        let mut black = [0f32; 4];
        let mut scale = [0f32; 4];
        for p in 0..4 {
            let b = f64::from(plan.black[p]);
            black[p] = b as f32;
            // R-ARW-47: (s - b) / (w - b) * m. R-ARW-78: w - b > 0 holds
            // because b < w - 255 (R-ARW-44, R-ARW-45).
            scale[p] = (plan.wb[cfa[p].min(2)] / (w - b).max(1.0)) as f32;
        }
        Ctx {
            sensor,
            vw: plan.valid.0,
            vh: plan.valid.1,
            crop: plan.crop,
            orientation: plan.orientation,
            cfa,
            diagonal,
            black,
            scale,
            m: plan.matrix.map(|r| r.map(|v| v as f32)),
            ycc: plan.ycc,
        }
    }

    /// R-ARW-47: normalized, white-balanced, clipped values of plane row
    /// `y`, columns `lo..hi`, into `out[0..hi - lo]`.
    fn normalize(&self, plane: &Plane, y: usize, lo: usize, hi: usize, out: &mut [f32]) {
        let par = 2 * (y & 1);
        let (b0, k0, b1, k1) = (
            self.black[par],
            self.scale[par],
            self.black[par + 1],
            self.scale[par + 1],
        );
        let mut x = lo;
        while x < hi {
            let tc = x / plane.tw;
            let row = plane.row(y, tc);
            let tx = tc * plane.tw;
            let end = hi.min(tx + plane.tw);
            let src = row.get(x - tx..end - tx).unwrap_or(&[]);
            let dst = &mut out[x - lo..end - lo];
            // R-ARW-77: normalized values are clipped to 0 to 1. Column
            // parity picks the black level and scale of each sample.
            crate::simd::normalize_row(src, x & 1 == 1, [b0, b1], [k0, k1], dst);
            x = end;
        }
    }

    /// CFA color of absolute pixel (x, y): R-ARW-23, pattern origin at
    /// stored pixel (0, 0), independent of the crop.
    #[inline]
    fn color(&self, x: usize, y: usize) -> usize {
        self.cfa[2 * (y & 1) + (x & 1)]
    }

    /// R-ARW-54 and R-ARW-55 by definition: the own color, and for each
    /// missing color the mean of the neighbors of that color among the 8
    /// neighbors inside the valid area, 0 when there are none. `rows` hold
    /// the normalized rows y - 1, y, y + 1 (None outside the valid area),
    /// indexed by column - `base`.
    fn demosaic_general(
        &self,
        x: usize,
        y: usize,
        rows: &[Option<&[f32]>; 3],
        base: usize,
    ) -> [f32; 3] {
        let own = self.color(x, y);
        let mut sum = [0f32; 3];
        let mut cnt = [0u32; 3];
        for (dy, row) in rows.iter().enumerate() {
            let Some(row) = row else {
                continue;
            };
            let yy = y + dy - 1;
            for xx in x.saturating_sub(1)..=(x + 1).min(self.vw - 1) {
                if xx == x && dy == 1 {
                    continue;
                }
                let c = self.color(xx, yy);
                if c != own {
                    sum[c] += row.get(xx.wrapping_sub(base)).copied().unwrap_or(0.0);
                    cnt[c] += 1;
                }
            }
        }
        let mut out = [0f32; 3];
        for c in 0..3 {
            out[c] = if c == own {
                rows[1]
                    .and_then(|r| r.get(x - base).copied())
                    .unwrap_or(0.0)
            } else if cnt[c] > 0 {
                sum[c] / cnt[c] as f32
            } else {
                0.0
            };
        }
        out
    }

    /// Camera RGB of absolute pixels (x0..x1, y) for CFA data. `win` holds
    /// normalized rows indexed by column - base, base = max(x0, 1) - 1.
    fn cfa_row(&self, y: usize, x0: usize, x1: usize, win: &Window, out: &mut RowRgb) {
        let base = win.base;
        let rows = [
            if y > 0 { win.get(y - 1) } else { None },
            win.get(y),
            if y + 1 < self.vh {
                win.get(y + 1)
            } else {
                None
            },
        ];
        let inner_y = y >= 1 && y + 1 < self.vh && self.diagonal;
        // The fast range: neighbors on every side inside the valid area.
        let (fa, fb) = if inner_y {
            (x0.max(1), x1.min(self.vw - 1).max(x0.max(1)))
        } else {
            (x1, x1)
        };
        for x in x0..fa.min(x1) {
            out.set(x - x0, self.demosaic_general(x, y, &rows, base));
        }
        if let (Some(a), Some(b), Some(c)) = (rows[0], rows[1], rows[2]) {
            if fb > fa {
                // Fixed neighbor sets of a Bayer pattern with diagonal
                // greens, equal to the general rule of R-ARW-54: own
                // value, horizontal and vertical pairs for green pixels,
                // the cross and the diagonals for the others.
                let green_first = self.color(fa, y) == 1;
                let q = if green_first {
                    self.color(fa + 1, y)
                } else {
                    self.color(fa, y)
                };
                let [r, g, bl] = &mut out.p;
                let planes = [
                    &mut r[fa - x0..fb - x0],
                    &mut g[fa - x0..fb - x0],
                    &mut bl[fa - x0..fb - x0],
                ];
                crate::simd::bayer_row(a, b, c, fa - base, fb - fa, green_first, q, planes);
            }
        }
        for x in fb.max(fa.min(x1))..x1 {
            out.set(x - x0, self.demosaic_general(x, y, &rows, base));
        }
    }

    /// Camera RGB of absolute pixels (x0..x1, y) for V5 data: R-ARW-41
    /// chroma placement, R-ARW-42 conversion, R-ARW-48 levels.
    fn ycc_row(&self, p: &YccPlanes, y: usize, x0: usize, x1: usize, out: &mut RowRgb) {
        let mut x = x0;
        while x < x1 {
            let tc = x / p.y.tw;
            let tx = tc * p.y.tw;
            let end = x1.min(tx + p.y.tw);
            let yr = p.y.row(y, tc);
            let cbr = p.cb.row(y / p.v, tc);
            let crr = p.cr.row(y / p.v, tc);
            for xx in x..end {
                let lx = xx - tx;
                let luma = f64::from(yr.get(lx).copied().unwrap_or(0));
                // R-ARW-41: chroma sample (floor(x / 2), floor(y / V)) of
                // the same tile.
                let cb = f64::from(cbr.get(lx / 2).copied().unwrap_or(0));
                let cr = f64::from(crr.get(lx / 2).copied().unwrap_or(0));
                let rgb = ycc_rgb(luma, cb, cr, &self.ycc);
                out.set(xx - x0, v5_levels(rgb, &self.ycc));
            }
            x = end;
        }
    }
}

/// One row of camera RGB as three planes, so the vector kernels load and
/// store whole runs of one channel.
pub(super) struct RowRgb {
    /// Red, green and blue.
    pub p: [Vec<f32>; 3],
}

impl RowRgb {
    fn new(n: usize) -> Self {
        RowRgb {
            p: [vec![0.0; n], vec![0.0; n], vec![0.0; n]],
        }
    }

    #[inline]
    fn set(&mut self, k: usize, v: [f32; 3]) {
        self.p[0][k] = v[0];
        self.p[1][k] = v[1];
        self.p[2][k] = v[2];
    }

    fn planes(&self) -> [&[f32]; 3] {
        [&self.p[0], &self.p[1], &self.p[2]]
    }
}

/// R-ARW-52: the matrix product of camera RGB `v`, clipped to 0 to 1:
/// linear sRGB (R-ARW-53). The clip takes the lower bound first, so a NaN
/// (which R-ARW-78 rules out) would become 0 the same way in every path.
#[inline]
pub(crate) fn to_srgb(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    crate::raw::tone::matrix_clip(m, v)
}

/// Normalized CFA rows y - 1, y, y + 1 for one band, columns base..limit.
struct Window {
    rows: [Vec<f32>; 3],
    tags: [usize; 3],
    base: usize,
    limit: usize,
}

impl Window {
    fn new(base: usize, limit: usize) -> Self {
        let n = limit - base;
        Window {
            rows: [vec![0.0; n], vec![0.0; n], vec![0.0; n]],
            tags: [usize::MAX; 3],
            base,
            limit,
        }
    }

    /// Make row `y` available, normalizing it when it is not cached.
    fn load(&mut self, ctx: &Ctx<'_>, plane: &Plane, y: usize) {
        let slot = y % 3;
        if self.tags[slot] != y {
            let (base, limit) = (self.base, self.limit);
            ctx.normalize(plane, y, base, limit, &mut self.rows[slot]);
            self.tags[slot] = y;
        }
    }

    fn get(&self, y: usize) -> Option<&[f32]> {
        let slot = y % 3;
        (self.tags[slot] == y).then(|| &self.rows[slot][..])
    }
}

/// Turns rows of camera RGB into output bytes: the color matrix (R-ARW-52,
/// see [`to_srgb`]) to linear sRGB (R-ARW-53), then the encoding.
pub(crate) trait RowEncoder: Sync {
    /// Bytes of one output pixel.
    fn pixel_bytes(&self) -> usize;
    /// Encode `cam` (camera RGB planes, white balanced, 0..1) through
    /// matrix `m` into `out`, which is `pixels * pixel_bytes()` long.
    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]);
}

/// Render the image through `encoder`. Returns the bytes of the oriented,
/// cropped image.
pub(crate) fn render(
    plan: &Plan,
    sensor: &Sensor,
    cancel: &CancelToken,
    encoder: &dyn RowEncoder,
) -> ArwResult<Vec<u8>> {
    match (sensor, plan.variant) {
        (Sensor::Ycc(_), Variant::V5) => {}
        (Sensor::Cfa(_), v) if v != Variant::V5 => {}
        _ => return Err(fail("sensor data do not match the variant")),
    }
    let ctx = Ctx::new(plan, sensor);
    let crop = ctx.crop;
    if crop.w == 0 || crop.h == 0 || crop.x + crop.w > ctx.vw || crop.y + crop.h > ctx.vh {
        return Err(fail("crop outside the valid area"));
    }
    let (ow, oh) = output_size(crop, ctx.orientation);
    let n = encoder.pixel_bytes();
    let row_bytes = ow * n;
    let len = row_bytes
        .checked_mul(oh)
        .ok_or_else(|| fail("output size"))?;
    let mut out = crate::util::alloc(len, ow as u32, oh as u32)?;
    let band = (oh / (pool::threads() * 4)).clamp(16, 256);
    let work: Vec<(usize, &mut [u8])> = out.chunks_mut(row_bytes * band).enumerate().collect();
    pool::run(work, cancel, |(i, rows)| {
        render_band(&ctx, i * band, rows.len() / row_bytes, ow, rows, encoder);
        Ok(())
    })?;
    Ok(out)
}

/// Render output rows `r0..r0 + n` into `dst`.
fn render_band(
    ctx: &Ctx<'_>,
    r0: usize,
    n: usize,
    ow: usize,
    dst: &mut [u8],
    encoder: &dyn RowEncoder,
) {
    let px = encoder.pixel_bytes();
    let (cw, ch, o) = (ctx.crop.w, ctx.crop.h, ctx.orientation);
    // The source rectangle (crop coordinates) that maps to these rows.
    let (xa, xb, ya, yb) = match o {
        1 | 2 => (0, cw, r0, r0 + n),
        3 | 4 => (0, cw, ch - r0 - n, ch - r0),
        5 | 6 => (r0, r0 + n, 0, ch),
        _ => (cw - r0 - n, cw - r0, 0, ch),
    };
    // R-ARW-56: demosaic in absolute raster coordinates (the CFA phase of
    // the stored raster) and keep only the crop.
    let (x0, x1) = (ctx.crop.x + xa, ctx.crop.x + xb);
    let mut cam = RowRgb::new(x1 - x0);
    let mut encoded = vec![0u8; (x1 - x0) * px];
    let mut window = match ctx.sensor {
        Sensor::Cfa(_) => Some(Window::new(x0.saturating_sub(1), (x1 + 1).min(ctx.vw))),
        Sensor::Ycc(_) => None,
    };
    for yc in ya..yb {
        let y = ctx.crop.y + yc;
        match (ctx.sensor, window.as_mut()) {
            (Sensor::Cfa(plane), Some(win)) => {
                for yy in y.saturating_sub(1)..=(y + 1).min(ctx.vh - 1) {
                    win.load(ctx, plane, yy);
                }
                ctx.cfa_row(y, x0, x1, win, &mut cam);
            }
            (Sensor::Ycc(p), _) => ctx.ycc_row(p, y, x0, x1, &mut cam),
            _ => {}
        }
        encoder.encode(&ctx.m, cam.planes(), &mut encoded);
        if o == 1 {
            // Upright: the source row is one output row.
            let at = (yc - r0) * ow * px;
            if let Some(row) = dst.get_mut(at..at + encoded.len()) {
                row.copy_from_slice(&encoded);
            }
            continue;
        }
        for (i, src) in encoded.chunks_exact(px).enumerate() {
            let (ox, oy) = map(o, xa + i, yc, cw, ch);
            let at = ((oy - r0) * ow + ox) * px;
            if let Some(d) = dst.get_mut(at..at + px) {
                d.copy_from_slice(src);
            }
        }
    }
}

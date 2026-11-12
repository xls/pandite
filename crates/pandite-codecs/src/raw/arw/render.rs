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
pub(super) enum Sensor {
    /// One CFA value per pixel on the 14-bit scale (V1 to V4).
    Cfa(Plane),
    /// Y, Cb and Cr planes (V5).
    Ycc(YccPlanes),
}

/// The output size for a crop and an orientation: W' x H' for 1 to 4,
/// H' x W' for 5 to 8 (R-ARW-58).
pub(super) fn output_size(crop: Rect, orientation: u8) -> (usize, usize) {
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
            for (i, (o, &s)) in dst.iter_mut().zip(src).enumerate() {
                let (b, k) = if (x + i) & 1 == 0 { (b0, k0) } else { (b1, k1) };
                // R-ARW-77: normalized values are clipped to 0 to 1.
                *o = ((f32::from(s) - b) * k).clamp(0.0, 1.0);
            }
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
    fn cfa_row(&self, y: usize, x0: usize, x1: usize, win: &Window, out: &mut [[f32; 3]]) {
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
            out[x - x0] = self.demosaic_general(x, y, &rows, base);
        }
        if let (Some(a), Some(b), Some(c)) = (rows[0], rows[1], rows[2]) {
            for x in fa..fb {
                let i = x - base;
                let own = self.color(x, y);
                let mut p = [0f32; 3];
                // Fixed neighbor sets of a Bayer pattern with diagonal
                // greens; equal to the general rule of R-ARW-54.
                if own == 1 {
                    let h = self.color(x + 1, y);
                    let v = self.color(x, y + 1);
                    p[1] = b[i];
                    p[h] = (b[i - 1] + b[i + 1]) * 0.5;
                    p[v] = (a[i] + c[i]) * 0.5;
                } else {
                    p[own] = b[i];
                    p[1] = (b[i - 1] + b[i + 1] + a[i] + c[i]) * 0.25;
                    p[2 - own] = (a[i - 1] + a[i + 1] + c[i - 1] + c[i + 1]) * 0.25;
                }
                out[x - x0] = p;
            }
        }
        for x in fb.max(fa.min(x1))..x1 {
            out[x - x0] = self.demosaic_general(x, y, &rows, base);
        }
    }

    /// Camera RGB of absolute pixels (x0..x1, y) for V5 data: R-ARW-41
    /// chroma placement, R-ARW-42 conversion, R-ARW-48 levels.
    fn ycc_row(&self, p: &YccPlanes, y: usize, x0: usize, x1: usize, out: &mut [[f32; 3]]) {
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
                out[xx - x0] = v5_levels(rgb, &self.ycc);
            }
            x = end;
        }
    }

    /// R-ARW-52: the matrix product, clipped to 0 to 1.
    #[inline]
    fn to_srgb(&self, v: [f32; 3]) -> [f32; 3] {
        let m = &self.m;
        let mut o = [0f32; 3];
        for r in 0..3 {
            o[r] = (m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]).clamp(0.0, 1.0);
        }
        o
    }
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

/// Render the image. `encode` turns one linear sRGB pixel (R-ARW-53) into
/// `N` output bytes. Returns the bytes of the oriented, cropped image.
pub(super) fn render<const N: usize, E>(
    plan: &Plan,
    sensor: &Sensor,
    cancel: &CancelToken,
    encode: E,
) -> ArwResult<Vec<u8>>
where
    E: Fn([f32; 3]) -> [u8; N] + Sync,
{
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
    let row_bytes = ow * N;
    let len = row_bytes
        .checked_mul(oh)
        .ok_or_else(|| fail("output size"))?;
    let mut out = crate::util::alloc(len, ow as u32, oh as u32)?;
    let band = (oh / (pool::threads() * 4)).clamp(16, 256);
    let work: Vec<(usize, &mut [u8])> = out.chunks_mut(row_bytes * band).enumerate().collect();
    pool::run(work, cancel, |(i, rows)| {
        render_band(&ctx, i * band, rows.len() / row_bytes, ow, rows, &encode);
        Ok(())
    })?;
    Ok(out)
}

/// Render output rows `r0..r0 + n` into `dst`.
fn render_band<const N: usize, E>(
    ctx: &Ctx<'_>,
    r0: usize,
    n: usize,
    ow: usize,
    dst: &mut [u8],
    encode: &E,
) where
    E: Fn([f32; 3]) -> [u8; N],
{
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
    let mut cam = vec![[0f32; 3]; x1 - x0];
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
        for (i, &v) in cam.iter().enumerate() {
            let (ox, oy) = map(o, xa + i, yc, cw, ch);
            let at = ((oy - r0) * ow + ox) * N;
            if let Some(px) = dst.get_mut(at..at + N) {
                px.copy_from_slice(&encode(ctx.to_srgb(v)));
            }
        }
    }
}

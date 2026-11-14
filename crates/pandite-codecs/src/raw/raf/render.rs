//! From decoded sensor values to the final image: normalization and white
//! balance (R-RAF-34), bilinear demosaicing of the X-Trans and Bayer
//! patterns on the stored raster (R-RAF-39 to R-RAF-42), the color matrix
//! (R-RAF-37), the crop (R-RAF-16) and the orientation (R-RAF-45,
//! R-RAF-46).
//!
//! The work is split into bands of output rows. Each band maps back to a
//! rectangle of the crop, which it walks row by row with a five-row window
//! of normalized values (the 5 x 5 rule of R-RAF-40 reaches two rows and
//! columns out), so every output byte is written by exactly one thread and
//! no full-size intermediate is held.

use super::container::{Layout, Plan};
use super::{fail, RafResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use crate::raw::arw::render::{map, output_size, RowEncoder};
use crate::raw::arw::Rect;
use pandite_core::CancelToken;

/// Rows of the window: y - 2 to y + 2.
const SPAN: usize = 5;

/// One neighbor of an interior pixel: window row (0 for y - 2 to 4 for
/// y + 2) and column offset.
type Neighbor = (usize, isize);

/// Precomputed per-file constants.
struct Ctx<'a> {
    plane: &'a Plane,
    /// Raster width W and height H.
    w: usize,
    h: usize,
    crop: Rect,
    orientation: u8,
    layout: Layout,
    /// Color of position (x mod 6, y mod 6) at 6 * (y mod 6) + (x mod 6).
    pattern: [u8; 36],
    /// Black level b (R-RAF-31: one level for every pixel).
    black: f32,
    /// m / (w - b) of each color (R-RAF-34).
    scale: [f32; 3],
    /// The normalized matrix (R-RAF-36).
    m: [[f32; 3]; 3],
    /// For each pattern position and color: the neighbors an interior
    /// pixel averages for that missing color (R-RAF-39, else R-RAF-40).
    lists: Vec<[Vec<Neighbor>; 3]>,
}

impl<'a> Ctx<'a> {
    fn new(plan: &Plan, plane: &'a Plane) -> Self {
        let b = plan.black;
        // R-RAF-62: w - b > 255 (R-RAF-30), so the divisor is positive.
        let span = (f64::from(plan.white) - b).max(1.0);
        let scale = plan.wb.map(|m| (m / span) as f32);
        let color =
            |x: isize, y: isize| plan.pattern[(6 * y.rem_euclid(6) + x.rem_euclid(6)) as usize];
        let mut lists = Vec::with_capacity(36);
        for p in 0..36isize {
            let (px, py) = (p % 6, p / 6);
            let own = color(px, py);
            let mut per: [Vec<Neighbor>; 3] = Default::default();
            for (c, list) in per.iter_mut().enumerate() {
                if c == usize::from(own) {
                    continue;
                }
                for radius in [1isize, 2] {
                    for dy in -radius..=radius {
                        for dx in -radius..=radius {
                            if (dx, dy) != (0, 0) && usize::from(color(px + dx, py + dy)) == c {
                                list.push(((dy + 2) as usize, dx));
                            }
                        }
                    }
                    if !list.is_empty() {
                        break;
                    }
                }
            }
            lists.push(per);
        }
        Ctx {
            plane,
            w: plan.width,
            h: plan.height,
            crop: plan.crop,
            orientation: plan.orientation,
            layout: plan.layout,
            pattern: plan.pattern,
            black: b as f32,
            scale,
            m: plan.matrix.map(|r| r.map(|v| v as f32)),
            lists,
        }
    }

    /// R-RAF-18, R-RAF-19, R-RAF-20, R-RAF-42: the color of stored pixel
    /// (x, y), the pattern anchored at stored pixel (0, 0).
    #[inline]
    fn color(&self, x: usize, y: usize) -> usize {
        usize::from(self.pattern[6 * (y % 6) + x % 6])
    }

    /// R-RAF-34: v = clip((s - b) / (w - b) * m, 0, 1) of raster row `y`,
    /// columns `lo..hi`, into `out[0..hi - lo]`.
    #[allow(clippy::manual_clamp)]
    fn normalize(&self, y: usize, lo: usize, hi: usize, out: &mut [f32]) {
        let src = self.plane.row(y, 0).get(lo..hi).unwrap_or(&[]);
        let out = &mut out[..src.len()];
        match self.layout {
            Layout::Bayer => {
                // Columns alternate between two colors: the Sony
                // decoder's kernel with one black level.
                let k = [self.scale[self.color(0, y)], self.scale[self.color(1, y)]];
                crate::simd::normalize_row(src, lo & 1 == 1, [self.black; 2], k, out);
            }
            Layout::XTrans => {
                let k: [f32; 6] = std::array::from_fn(|i| self.scale[self.color(i, y)]);
                for (i, (o, &s)) in out.iter_mut().zip(src).enumerate() {
                    // Lower bound first: NaN would become 0 (R-RAF-62).
                    *o = ((f32::from(s) - self.black) * k[(lo + i) % 6])
                        .max(0.0)
                        .min(1.0);
                }
            }
        }
    }

    /// R-RAF-39 and R-RAF-40 by definition, for any pixel: the own color,
    /// and for each missing color the mean of the pixels of that color
    /// among the 8 neighbors inside the raster; when there are none, among
    /// the 24 pixels of the 5 x 5 neighborhood inside the raster; else 0.
    fn general(&self, x: usize, y: usize, win: &Window) -> [f32; 3] {
        let v = |xx: usize, yy: usize| {
            win.get(yy)
                .and_then(|r| r.get(xx - win.base))
                .copied()
                .unwrap_or(0.0)
        };
        let own = self.color(x, y);
        let mut out = [0f32; 3];
        out[own] = v(x, y);
        for (c, o) in out.iter_mut().enumerate() {
            if c == own {
                continue;
            }
            for radius in [1usize, 2] {
                let (mut sum, mut n) = (0f32, 0u32);
                for yy in y.saturating_sub(radius)..=(y + radius).min(self.h - 1) {
                    for xx in x.saturating_sub(radius)..=(x + radius).min(self.w - 1) {
                        if (xx, yy) != (x, y) && self.color(xx, yy) == c {
                            sum += v(xx, yy);
                            n += 1;
                        }
                    }
                }
                // R-RAF-62: a mean only over a non-empty set.
                if n > 0 {
                    *o = sum / n as f32;
                    break;
                }
            }
        }
        out
    }

    /// Camera RGB of raster pixels (x0..x1, y), demosaiced on the stored
    /// raster with neighbors outside the crop (R-RAF-41). `win` holds the
    /// normalized rows y - 2 to y + 2 inside the raster.
    fn row(&self, y: usize, x0: usize, x1: usize, win: &Window, out: &mut [Vec<f32>; 3]) {
        // The interior: every neighbor the fixed lists name lies inside the
        // raster (1 pixel for the Bayer kernel, 2 for the X-Trans lists).
        let margin = match self.layout {
            Layout::Bayer => 1,
            Layout::XTrans => 2,
        };
        let inner_y = y >= margin && y + margin < self.h;
        let (fa, fb) = if inner_y {
            let fa = x0.max(margin).min(x1);
            (fa, x1.min(self.w.saturating_sub(margin)).max(fa))
        } else {
            (x1, x1)
        };
        for x in (x0..fa).chain(fb..x1) {
            let p = self.general(x, y, win);
            for c in 0..3 {
                out[c][x - x0] = p[c];
            }
        }
        if fb <= fa {
            return;
        }
        let base = win.base;
        match self.layout {
            Layout::Bayer => {
                let (Some(a), Some(b), Some(c)) = (win.get(y - 1), win.get(y), win.get(y + 1))
                else {
                    return;
                };
                // The Sony decoder's kernel for a Bayer pattern with
                // diagonal greens: equal to R-RAF-39 inside the raster.
                let green_first = self.color(fa, y) == 1;
                let q = if green_first {
                    self.color(fa + 1, y)
                } else {
                    self.color(fa, y)
                };
                let [r, g, bl] = out;
                let planes = [
                    &mut r[fa - x0..fb - x0],
                    &mut g[fa - x0..fb - x0],
                    &mut bl[fa - x0..fb - x0],
                ];
                crate::simd::bayer_row(a, b, c, fa - base, fb - fa, green_first, q, planes);
            }
            Layout::XTrans => {
                let rows: [&[f32]; SPAN] =
                    std::array::from_fn(|k| win.get(y + k - 2).unwrap_or(&[]));
                for x in fa..fb {
                    let p = 6 * (y % 6) + x % 6;
                    let own = usize::from(self.pattern[p]);
                    let i = x - base;
                    for (c, (plane, list)) in out.iter_mut().zip(&self.lists[p]).enumerate() {
                        plane[x - x0] = if c == own {
                            rows[2].get(i).copied().unwrap_or(0.0)
                        } else {
                            let mut sum = 0f32;
                            for &(r, dx) in list {
                                let at = i.wrapping_add_signed(dx);
                                sum += rows[r].get(at).copied().unwrap_or(0.0);
                            }
                            // R-RAF-40: 0 when even the 5 x 5 set is empty.
                            if list.is_empty() {
                                0.0
                            } else {
                                sum / list.len() as f32
                            }
                        };
                    }
                }
            }
        }
    }
}

/// Normalized rows y - 2 to y + 2 of one band, columns base..limit.
struct Window {
    rows: [Vec<f32>; SPAN],
    tags: [usize; SPAN],
    base: usize,
    limit: usize,
}

impl Window {
    fn new(base: usize, limit: usize) -> Self {
        let n = limit - base;
        Window {
            rows: std::array::from_fn(|_| vec![0.0; n]),
            tags: [usize::MAX; SPAN],
            base,
            limit,
        }
    }

    /// Make row `y` available, normalizing it when it is not cached.
    fn load(&mut self, ctx: &Ctx<'_>, y: usize) {
        let slot = y % SPAN;
        if self.tags[slot] != y {
            ctx.normalize(y, self.base, self.limit, &mut self.rows[slot]);
            self.tags[slot] = y;
        }
    }

    fn get(&self, y: usize) -> Option<&[f32]> {
        let slot = y % SPAN;
        (self.tags[slot] == y).then(|| &self.rows[slot][..])
    }
}

/// Render the image through `encoder`, which applies the color matrix
/// (R-RAF-37) and the output encoding. Returns the bytes of the oriented,
/// cropped image.
pub(super) fn render(
    plan: &Plan,
    plane: &Plane,
    cancel: &CancelToken,
    encoder: &dyn RowEncoder,
) -> RafResult<Vec<u8>> {
    let ctx = Ctx::new(plan, plane);
    if plane.data.len() != ctx.w * ctx.h || plane.tw != ctx.w {
        return Err(fail("sensor data do not match the raster"));
    }
    let crop = ctx.crop;
    if crop.w == 0 || crop.h == 0 || crop.x + crop.w > ctx.w || crop.y + crop.h > ctx.h {
        return Err(fail("crop outside the raster"));
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
    // The source rectangle (crop coordinates) that maps to these rows by
    // the table of R-RAF-45.
    let (xa, xb, ya, yb) = match o {
        1 | 2 => (0, cw, r0, r0 + n),
        3 | 4 => (0, cw, ch - r0 - n, ch - r0),
        5 | 6 => (r0, r0 + n, 0, ch),
        _ => (cw - r0 - n, cw - r0, 0, ch),
    };
    // R-RAF-41, R-RAF-42: demosaic in raster coordinates with the stored
    // pattern, using neighbors outside the crop, and keep only the crop.
    let (x0, x1) = (ctx.crop.x + xa, ctx.crop.x + xb);
    let mut cam: [Vec<f32>; 3] = std::array::from_fn(|_| vec![0.0; x1 - x0]);
    let mut encoded = vec![0u8; (x1 - x0) * px];
    let mut win = Window::new(x0.saturating_sub(2), (x1 + 2).min(ctx.w));
    for yc in ya..yb {
        let y = ctx.crop.y + yc;
        for yy in y.saturating_sub(2)..=(y + 2).min(ctx.h - 1) {
            win.load(ctx, yy);
        }
        ctx.row(y, x0, x1, &win, &mut cam);
        encoder.encode(&ctx.m, [&cam[0], &cam[1], &cam[2]], &mut encoded);
        if o == 1 {
            // Upright: the source row is one output row.
            let at = (yc - r0) * ow * px;
            if let Some(row) = dst.get_mut(at..at + encoded.len()) {
                row.copy_from_slice(&encoded);
            }
            continue;
        }
        // R-RAF-45, R-RAF-46: the output pixel of stored pixel (x, y).
        for (i, src) in encoded.chunks_exact(px).enumerate() {
            let (ox, oy) = map(o, xa + i, yc, cw, ch);
            let at = ((oy - r0) * ow + ox) * px;
            if let Some(d) = dst.get_mut(at..at + px) {
                d.copy_from_slice(src);
            }
        }
    }
}

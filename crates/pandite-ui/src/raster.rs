//! A tiny antialiased vector rasterizer for icons.
//!
//! Shapes are described on a 24 x 24 design grid and evaluated as signed
//! distance fields: each pixel's coverage is `0.5 - distance` (in pixels),
//! clamped to 0..1. That gives a one-pixel antialiasing ramp at any output
//! size, so the same description renders crisp at 16, 20, 40 or 80 pixels.

/// A point on the 24 x 24 design grid.
pub(crate) type P = (f32, f32);

/// Size of the design grid.
pub(crate) const GRID: f32 = 24.0;

/// One primitive. Lengths are design units.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Shape {
    /// A stroked open or closed polyline with round caps and joins.
    Path {
        pts: &'static [P],
        width: f32,
        closed: bool,
    },
    /// A filled polygon (even-odd rule).
    Fill { pts: &'static [P] },
    /// A filled circle.
    Disc { c: P, r: f32 },
    /// A circle outline.
    Ring { c: P, r: f32, width: f32 },
    /// A circular arc with round caps. Angles are degrees, growing
    /// clockwise on screen (y points down), 0 at three o'clock.
    Arc {
        c: P,
        r: f32,
        start: f32,
        sweep: f32,
        width: f32,
    },
    /// A filled rounded rectangle.
    Box {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
    },
    /// A rounded rectangle outline centered on the box edge.
    Frame {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        width: f32,
    },
    /// The outline of the union of `parts`, centered on its edge.
    Outline { parts: &'static [Shape], width: f32 },
}

/// Draw or erase one shape.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// Union with the shape.
    Add(Shape),
    /// Erase the shape grown by the given margin (design units).
    Cut(Shape, f32),
}

/// A coordinate transform applied before evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transform {
    /// Draw as described.
    Identity,
    /// Mirror left to right.
    MirrorX,
    /// Swap the axes (mirror across the main diagonal).
    Transpose,
}

fn len(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

fn dist_segment(p: P, a: P, b: P) -> f32 {
    let (px, py) = (p.0 - a.0, p.1 - a.1);
    let (bx, by) = (b.0 - a.0, b.1 - a.1);
    let ll = bx * bx + by * by;
    let t = if ll > 0.0 {
        ((px * bx + py * by) / ll).clamp(0.0, 1.0)
    } else {
        0.0
    };
    len(px - bx * t, py - by * t)
}

fn sd_rounded_box(p: P, x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    let qx = (p.0 - (x + w * 0.5)).abs() - (w * 0.5 - r);
    let qy = (p.1 - (y + h * 0.5)).abs() - (h * 0.5 - r);
    len(qx.max(0.0), qy.max(0.0)) + qx.max(qy).min(0.0) - r
}

fn sd_polygon(p: P, pts: &[P]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return f32::INFINITY;
    }
    let mut d = f32::INFINITY;
    let mut inside = false;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        d = d.min(dist_segment(p, a, b));
        if (a.1 > p.1) != (b.1 > p.1) {
            let x = (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0;
            if p.0 < x {
                inside = !inside;
            }
        }
    }
    if inside {
        -d
    } else {
        d
    }
}

impl Shape {
    /// Signed distance from `p` to the shape edge, negative inside.
    pub(crate) fn distance(&self, p: P) -> f32 {
        match *self {
            Shape::Path { pts, width, closed } => {
                let mut d = f32::INFINITY;
                let n = pts.len();
                if n == 1 {
                    d = len(p.0 - pts[0].0, p.1 - pts[0].1);
                }
                for i in 1..n {
                    d = d.min(dist_segment(p, pts[i - 1], pts[i]));
                }
                if closed && n > 2 {
                    d = d.min(dist_segment(p, pts[n - 1], pts[0]));
                }
                d - width * 0.5
            }
            Shape::Fill { pts } => sd_polygon(p, pts),
            Shape::Disc { c, r } => len(p.0 - c.0, p.1 - c.1) - r,
            Shape::Ring { c, r, width } => (len(p.0 - c.0, p.1 - c.1) - r).abs() - width * 0.5,
            Shape::Arc {
                c,
                r,
                start,
                sweep,
                width,
            } => {
                let (vx, vy) = (p.0 - c.0, p.1 - c.1);
                let angle = vy.atan2(vx).to_degrees();
                let rel = (angle - start).rem_euclid(360.0);
                let d = if rel <= sweep {
                    (len(vx, vy) - r).abs()
                } else {
                    let end = |deg: f32| {
                        let rad = deg.to_radians();
                        (c.0 + r * rad.cos(), c.1 + r * rad.sin())
                    };
                    let a = end(start);
                    let b = end(start + sweep);
                    len(p.0 - a.0, p.1 - a.1).min(len(p.0 - b.0, p.1 - b.1))
                };
                d - width * 0.5
            }
            Shape::Box { x, y, w, h, r } => sd_rounded_box(p, x, y, w, h, r),
            Shape::Frame {
                x,
                y,
                w,
                h,
                r,
                width,
            } => sd_rounded_box(p, x, y, w, h, r).abs() - width * 0.5,
            Shape::Outline { parts, width } => {
                let union = parts
                    .iter()
                    .map(|s| s.distance(p))
                    .fold(f32::INFINITY, f32::min);
                union.abs() - width * 0.5
            }
        }
    }
}

/// Signed distance of a whole icon description at `p`.
pub(crate) fn distance(ops: &[Op], p: P) -> f32 {
    let mut d = f32::INFINITY;
    for op in ops {
        match op {
            Op::Add(shape) => d = d.min(shape.distance(p)),
            Op::Cut(shape, grow) => d = d.max(-(shape.distance(p) - grow)),
        }
    }
    d
}

/// Rasterize `ops` into a `size` x `size` coverage mask (0 to 255, row
/// major, top row first).
pub(crate) fn rasterize(ops: &[Op], transform: Transform, size: u32) -> Vec<u8> {
    let size = size.max(1);
    let units_per_px = GRID / size as f32;
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let gx = (x as f32 + 0.5) * units_per_px;
            let gy = (y as f32 + 0.5) * units_per_px;
            let p = match transform {
                Transform::Identity => (gx, gy),
                Transform::MirrorX => (GRID - gx, gy),
                Transform::Transpose => (gy, gx),
            };
            let d_px = distance(ops, p) / units_per_px;
            let coverage = (0.5 - d_px).clamp(0.0, 1.0);
            out.push((coverage * 255.0).round() as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_has_solid_center_soft_edge_and_empty_corners() {
        let ops = [Op::Add(Shape::Disc {
            c: (12.0, 12.0),
            r: 8.0,
        })];
        let mask = rasterize(&ops, Transform::Identity, 24);
        assert_eq!(mask[12 * 24 + 12], 255);
        assert_eq!(mask[0], 0);
        let partial = mask.iter().filter(|&&v| v > 0 && v < 255).count();
        assert!(partial > 8, "edge pixels are antialiased");
    }

    #[test]
    fn cut_removes_coverage() {
        let ops = [
            Op::Add(Shape::Box {
                x: 2.0,
                y: 2.0,
                w: 20.0,
                h: 20.0,
                r: 0.0,
            }),
            Op::Cut(
                Shape::Disc {
                    c: (12.0, 12.0),
                    r: 4.0,
                },
                0.0,
            ),
        ];
        let mask = rasterize(&ops, Transform::Identity, 24);
        assert_eq!(mask[12 * 24 + 12], 0);
        assert_eq!(mask[4 * 24 + 4], 255);
    }

    #[test]
    fn arc_covers_only_its_sweep() {
        let ops = [Op::Add(Shape::Arc {
            c: (12.0, 12.0),
            r: 8.0,
            start: 0.0,
            sweep: 90.0,
            width: 2.0,
        })];
        // 0 to 90 degrees clockwise is the lower right quarter.
        assert!(
            distance(
                &ops,
                (
                    12.0 + 8.0 * std::f32::consts::FRAC_1_SQRT_2,
                    12.0 + 8.0 * std::f32::consts::FRAC_1_SQRT_2
                )
            ) < 0.0
        );
        assert!(
            distance(
                &ops,
                (
                    12.0 - 8.0 * std::f32::consts::FRAC_1_SQRT_2,
                    12.0 - 8.0 * std::f32::consts::FRAC_1_SQRT_2
                )
            ) > 3.0
        );
    }

    #[test]
    fn transforms_mirror_and_transpose() {
        let ops = [Op::Add(Shape::Disc {
            c: (4.0, 12.0),
            r: 2.0,
        })];
        let plain = rasterize(&ops, Transform::Identity, 24);
        let mirrored = rasterize(&ops, Transform::MirrorX, 24);
        let transposed = rasterize(&ops, Transform::Transpose, 24);
        assert_eq!(plain[12 * 24 + 3], 255);
        assert_eq!(mirrored[12 * 24 + 20], 255);
        assert_eq!(transposed[3 * 24 + 12], 255);
    }
}

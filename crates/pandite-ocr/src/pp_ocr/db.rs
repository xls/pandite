//! Text boxes from the detection model's probability map, the way the
//! reference does it (Differentiable Binarization post-processing):
//! threshold the map, dilate it, take every outline (`cv2.findContours`
//! with `RETR_LIST`), fit the smallest rotated rectangle around each,
//! score it by the mean probability inside, grow it by the unclip
//! distance (`pyclipper` with round joins), fit a rectangle again, scale
//! it to the image and put the boxes in reading order.

/// A point in pixels.
pub(crate) type Pt = [f64; 2];

/// Pixels with a probability above this are text.
const THRESH: f32 = 0.3;
/// Boxes with a lower mean probability are dropped.
const BOX_THRESH: f64 = 0.5;
/// At most this many outlines are looked at.
const MAX_CANDIDATES: usize = 1000;
/// How far a box grows: area times this, over the perimeter.
const UNCLIP_RATIO: f64 = 1.6;
/// Rectangles with a shorter side are dropped (before the unclip; after
/// it the limit is 2 more).
const MIN_SIZE: f64 = 3.0;
/// Boxes whose tops differ by less than this share a line in the order.
const LINE_GAP: f64 = 10.0;

/// The text boxes of a `width` x `height` probability map, scaled to a
/// `dest` (width, height) image: integer corners in the order top-left,
/// top-right, bottom-right, bottom-left, boxes in reading order.
pub(crate) fn boxes(
    pred: &[f32],
    width: usize,
    height: usize,
    dest: (usize, usize),
) -> Vec<[Pt; 4]> {
    let mask = dilate(&threshold(pred), width, height);
    let mut found = Vec::new();
    for points in outlines(&mask, width, height)
        .into_iter()
        .take(MAX_CANDIDATES)
    {
        let Some((rect, side)) = min_area_rect(&points) else {
            continue;
        };
        if side < MIN_SIZE {
            continue;
        }
        let rect = mini_box(rect);
        if score(pred, width, height, &rect) < BOX_THRESH {
            continue;
        }
        let grown = unclip(&rect);
        let Some((rect, side)) = min_area_rect(&grown) else {
            continue;
        };
        if side < MIN_SIZE + 2.0 {
            continue;
        }
        let scale = |v: f64, from: usize, to: usize| {
            // The reference does this in 32-bit floats.
            let v = (v as f32 / from as f32 * to as f32).round_ties_even();
            f64::from(v.clamp(0.0, to as f32)).trunc()
        };
        let rect = mini_box(rect).map(|[x, y]| [scale(x, width, dest.0), scale(y, height, dest.1)]);
        let rect = clockwise(rect).map(|[x, y]| {
            [
                x.clamp(0.0, dest.0 as f64 - 1.0),
                y.clamp(0.0, dest.1 as f64 - 1.0),
            ]
        });
        let side = |a: Pt, b: Pt| ((a[0] - b[0]).hypot(a[1] - b[1])).trunc();
        if side(rect[0], rect[1]) <= 3.0 || side(rect[0], rect[3]) <= 3.0 {
            continue;
        }
        found.push(rect);
    }
    reading_order(found)
}

fn threshold(pred: &[f32]) -> Vec<u8> {
    pred.iter().map(|&p| u8::from(p > THRESH)).collect()
}

/// Dilation with a 2 x 2 kernel anchored at its bottom right
/// (`cv2.dilate` with `[[1, 1], [1, 1]]`): a pixel is set when it or its
/// left, upper or upper-left neighbor is.
fn dilate(mask: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = mask.to_vec();
    for y in 0..height {
        for x in 0..width {
            let at =
                |dx: usize, dy: usize| x >= dx && y >= dy && mask[(y - dy) * width + x - dx] != 0;
            if at(1, 0) || at(0, 1) || at(1, 1) {
                out[y * width + x] = 1;
            }
        }
    }
    out
}

/// The points of every outline of `mask`, as `cv2.findContours` with
/// `RETR_LIST` lists them: the outer border of every 8-connected group of
/// set pixels and the border around every 4-connected hole in one. Only
/// the convex hull of an outline matters downstream, so each outline is
/// given by the leftmost and rightmost border pixel of each of its rows.
/// Outlines come in the order their first pixel is met, row by row.
fn outlines(mask: &[u8], width: usize, height: usize) -> Vec<Vec<Pt>> {
    const NONE: u32 = u32::MAX;
    let n = width * height;
    let mut label = vec![NONE; n];
    // (first pixel, row extremes) per outline.
    let mut found: Vec<(usize, Vec<Pt>)> = Vec::new();
    let mut stack = Vec::new();
    let mut pixels = Vec::new();
    for start in 0..n {
        if label[start] != NONE {
            continue;
        }
        let set = mask[start] != 0;
        let id = found.len() as u32;
        label[start] = id;
        stack.push(start);
        pixels.clear();
        let mut touches_edge = false;
        while let Some(i) = stack.pop() {
            pixels.push(i);
            let (x, y) = ((i % width) as i64, (i / width) as i64);
            if x == 0 || y == 0 || x as usize == width - 1 || y as usize == height - 1 {
                touches_edge = true;
            }
            for (dx, dy) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                // Set pixels join diagonally, holes only side by side.
                if !set && dx != 0 && dy != 0 {
                    continue;
                }
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx as usize >= width || ny as usize >= height {
                    continue;
                }
                let j = ny as usize * width + nx as usize;
                if label[j] == NONE && (mask[j] != 0) == set {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
        if set {
            found.push((start, row_extremes(pixels.iter().copied(), width)));
        } else if !touches_edge {
            // A hole: its border is the set pixels beside it.
            let border = pixels.iter().flat_map(|&i| {
                let (x, y) = (i % width, i / width);
                [
                    (x > 0).then(|| i - 1),
                    (x + 1 < width).then(|| i + 1),
                    (y > 0).then(|| i - width),
                    (y + 1 < height).then(|| i + width),
                ]
                .into_iter()
                .flatten()
                .filter(|&j| mask[j] != 0)
            });
            found.push((start, row_extremes(border, width)));
        } else {
            // The background around the groups: no outline of its own.
            found.push((start, Vec::new()));
        }
    }
    found
        .into_iter()
        .filter(|(_, points)| !points.is_empty())
        .map(|(_, points)| points)
        .collect()
}

/// The leftmost and rightmost of `pixels` on each row, as points.
fn row_extremes(pixels: impl Iterator<Item = usize>, width: usize) -> Vec<Pt> {
    let mut rows: Vec<(usize, usize, usize)> = Vec::new();
    for i in pixels {
        rows.push((i / width, i % width, i % width));
    }
    rows.sort_unstable();
    let mut out: Vec<Pt> = Vec::new();
    let mut k = 0;
    while k < rows.len() {
        let y = rows[k].0;
        let (mut lo, mut hi) = (rows[k].1, rows[k].2);
        while k < rows.len() && rows[k].0 == y {
            lo = lo.min(rows[k].1);
            hi = hi.max(rows[k].2);
            k += 1;
        }
        out.push([lo as f64, y as f64]);
        if hi != lo {
            out.push([hi as f64, y as f64]);
        }
    }
    out
}

/// The convex hull of `points`, counter-clockwise on screen, without
/// collinear points (Andrew's monotone chain).
fn hull(points: &[Pt]) -> Vec<Pt> {
    let mut p: Vec<Pt> = points.to_vec();
    p.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Pt, a: Pt, b: Pt| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut lower: Vec<Pt> = Vec::new();
    for &q in &p {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], q) <= 0.0 {
            lower.pop();
        }
        lower.push(q);
    }
    let mut upper: Vec<Pt> = Vec::new();
    for &q in p.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], q) <= 0.0 {
            upper.pop();
        }
        upper.push(q);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// The smallest rotated rectangle around `points` (`cv2.minAreaRect`) as
/// its four corners in turn, and its shorter side. None without points.
pub(crate) fn min_area_rect(points: &[Pt]) -> Option<([Pt; 4], f64)> {
    let h = hull(points);
    match h.len() {
        0 => return None,
        1 => return Some(([h[0]; 4], 0.0)),
        2 => return Some(([h[0], h[1], h[1], h[0]], 0.0)),
        _ => {}
    }
    let mut best: Option<(f64, [Pt; 4], f64)> = None;
    for i in 0..h.len() {
        let (a, b) = (h[i], h[(i + 1) % h.len()]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len == 0.0 {
            continue;
        }
        let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let v = [-u[1], u[0]];
        let (mut u0, mut u1, mut v0, mut v1) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for p in &h {
            let pu = p[0] * u[0] + p[1] * u[1];
            let pv = p[0] * v[0] + p[1] * v[1];
            u0 = u0.min(pu);
            u1 = u1.max(pu);
            v0 = v0.min(pv);
            v1 = v1.max(pv);
        }
        let area = (u1 - u0) * (v1 - v0);
        if best.as_ref().is_none_or(|(a, ..)| area < *a) {
            let at = |s: f64, t: f64| [s * u[0] + t * v[0], s * u[1] + t * v[1]];
            let corners = [at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)];
            best = Some((area, corners, (u1 - u0).min(v1 - v0)));
        }
    }
    best.map(|(_, corners, side)| (corners, side))
}

/// Rectangle corners in the reference's order (`get_mini_boxes`): sorted
/// by x, the left pair split by y into top-left and bottom-left, the
/// right pair into top-right and bottom-right.
fn mini_box(corners: [Pt; 4]) -> [Pt; 4] {
    let mut p = corners;
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (tl, bl) = if p[1][1] > p[0][1] {
        (p[0], p[1])
    } else {
        (p[1], p[0])
    };
    let (tr, br) = if p[3][1] > p[2][1] {
        (p[2], p[3])
    } else {
        (p[3], p[2])
    };
    [tl, tr, br, bl]
}

/// Corners in clockwise order on screen from the top-left
/// (`order_points_clockwise`): the two leftmost by y, then the two
/// rightmost by y.
fn clockwise(corners: [Pt; 4]) -> [Pt; 4] {
    let mut p = corners;
    p.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let (mut left, mut right) = ([p[0], p[1]], [p[2], p[3]]);
    left.sort_by(|a, b| a[1].total_cmp(&b[1]));
    right.sort_by(|a, b| a[1].total_cmp(&b[1]));
    [left[0], right[0], right[1], left[1]]
}

/// The mean probability inside `rect` (`box_score_fast`): the rectangle
/// is filled like `cv2.fillPoly` over the block of pixels it spans.
fn score(pred: &[f32], width: usize, height: usize, rect: &[Pt; 4]) -> f64 {
    let (w, h) = (width as f64, height as f64);
    let lo = |v: f64, n: f64| v.floor().clamp(0.0, n - 1.0);
    let hi = |v: f64, n: f64| v.ceil().clamp(0.0, n - 1.0);
    let xs = rect.map(|p| p[0]);
    let ys = rect.map(|p| p[1]);
    let x0 = lo(xs.iter().copied().fold(f64::MAX, f64::min), w);
    let x1 = hi(xs.iter().copied().fold(f64::MIN, f64::max), w);
    let y0 = lo(ys.iter().copied().fold(f64::MAX, f64::min), h);
    let y1 = hi(ys.iter().copied().fold(f64::MIN, f64::max), h);
    // Corners relative to the block, truncated to whole pixels.
    let poly = rect.map(|p| [(p[0] - x0).trunc(), (p[1] - y0).trunc()]);
    let (bw, bh) = ((x1 - x0) as usize + 1, (y1 - y0) as usize + 1);
    let (mut sum, mut count) = (0f64, 0usize);
    for r in 0..bh {
        let Some((a, b)) = span(&poly, r as f64) else {
            continue;
        };
        // Edge pixels count, as the outline `cv2.fillPoly` draws.
        let (a, b) = ((a + 0.5).floor().max(0.0), (b + 0.5).floor());
        if b < a {
            continue;
        }
        let row = (y0 as usize + r) * width + x0 as usize;
        for x in a as usize..=(b as usize).min(bw - 1) {
            sum += f64::from(pred[row + x]);
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        sum / count as f64
    }
}

/// The x range a convex polygon covers on row `y`, when it reaches it.
fn span(poly: &[Pt; 4], y: f64) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::MAX, f64::MIN);
    for i in 0..4 {
        let (a, b) = (poly[i], poly[(i + 1) % 4]);
        let (ya, yb) = (a[1].min(b[1]), a[1].max(b[1]));
        if y < ya || y > yb {
            continue;
        }
        if a[1] == b[1] {
            lo = lo.min(a[0].min(b[0]));
            hi = hi.max(a[0].max(b[0]));
        } else {
            let x = a[0] + (y - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// The rectangle grown by `area * UNCLIP_RATIO / perimeter` with round
/// corners, as `pyclipper` offsets it: the corners are truncated to whole
/// pixels first and the result has whole-pixel vertices.
pub(crate) fn unclip(rect: &[Pt; 4]) -> Vec<Pt> {
    let mut area = 0.0;
    let mut perimeter = 0.0;
    for i in 0..4 {
        let (a, b) = (rect[i], rect[(i + 1) % 4]);
        area += a[0] * b[1] - b[0] * a[1];
        perimeter += (b[0] - a[0]).hypot(b[1] - a[1]);
    }
    if perimeter <= 0.0 {
        return Vec::new();
    }
    let delta = (area * 0.5).abs() * UNCLIP_RATIO / perimeter;
    let path: Vec<[i64; 2]> = rect.iter().map(|p| [p[0] as i64, p[1] as i64]).collect();
    offset_round(&path, delta)
        .into_iter()
        .map(|[x, y]| [x as f64, y as f64])
        .collect()
}

/// Clipper 6.4 `ClipperOffset` of one closed polygon by `delta` with
/// round joins and the default arc tolerance of 0.25.
fn offset_round(path: &[[i64; 2]], delta: f64) -> Vec<[i64; 2]> {
    // Drop repeated points, also a last point equal to the first.
    let mut src: Vec<[i64; 2]> = Vec::with_capacity(path.len());
    for &p in path {
        if src.last() != Some(&p) {
            src.push(p);
        }
    }
    while src.len() > 1 && src.first() == src.last() {
        src.pop();
    }
    if src.len() < 3 {
        return Vec::new();
    }
    // Clipper's orientation: a negative area turns the path around so
    // that the normals point outward.
    let mut a = 0f64;
    for i in 0..src.len() {
        let j = (i + src.len() - 1) % src.len();
        a += (src[j][0] + src[i][0]) as f64 * (src[j][1] - src[i][1]) as f64;
    }
    if -a * 0.5 < 0.0 {
        src.reverse();
    }
    if delta.abs() < 1e-20 {
        return src;
    }
    let round = |v: f64| {
        if v < 0.0 {
            (v - 0.5) as i64
        } else {
            (v + 0.5) as i64
        }
    };
    let tolerance = if 0.25 > delta.abs() * 0.25 {
        delta.abs() * 0.25
    } else {
        0.25
    };
    let mut steps = std::f64::consts::PI / (1.0 - tolerance / delta.abs()).acos();
    if steps > delta.abs() * std::f64::consts::PI {
        steps = delta.abs() * std::f64::consts::PI;
    }
    let (mut sin, cos) = (std::f64::consts::TAU / steps).sin_cos();
    let steps_per_rad = steps / std::f64::consts::TAU;
    if delta < 0.0 {
        sin = -sin;
    }
    let len = src.len();
    let normals: Vec<[f64; 2]> = (0..len)
        .map(|j| {
            let (p, q) = (src[j], src[(j + 1) % len]);
            if p == q {
                return [0.0, 0.0];
            }
            let (dx, dy) = ((q[0] - p[0]) as f64, (q[1] - p[1]) as f64);
            let f = 1.0 / dx.hypot(dy);
            [dy * f, -dx * f]
        })
        .collect();
    let mut out = Vec::new();
    let at = |p: [i64; 2], n: [f64; 2]| {
        [
            round(p[0] as f64 + n[0] * delta),
            round(p[1] as f64 + n[1] * delta),
        ]
    };
    let mut k = len - 1;
    for j in 0..len {
        let (nk, nj, p) = (normals[k], normals[j], src[j]);
        let mut sin_a = nk[0] * nj[1] - nj[0] * nk[1];
        let cos_a = nk[0] * nj[0] + nj[1] * nk[1];
        if (sin_a * delta).abs() < 1.0 {
            if cos_a > 0.0 {
                // Nearly straight on: one point, and k stays (as in Clipper).
                out.push(at(p, nk));
                continue;
            }
        } else {
            sin_a = sin_a.clamp(-1.0, 1.0);
        }
        if sin_a * delta < 0.0 {
            out.push(at(p, nk));
            out.push(p);
            out.push(at(p, nj));
        } else {
            let turn = sin_a.atan2(cos_a);
            let n = (round(steps_per_rad * turn.abs())).max(1);
            let (mut x, mut y) = (nk[0], nk[1]);
            for _ in 0..n {
                out.push([
                    round(p[0] as f64 + x * delta),
                    round(p[1] as f64 + y * delta),
                ]);
                let x2 = x;
                x = x * cos - sin * y;
                y = x2 * sin + y * cos;
            }
            out.push(at(p, nj));
        }
        k = j;
    }
    out
}

/// Boxes top to bottom, and left to right among boxes whose tops are
/// within [`LINE_GAP`] of the box above (`sorted_boxes`).
fn reading_order(mut boxes: Vec<[Pt; 4]>) -> Vec<[Pt; 4]> {
    boxes.sort_by(|a, b| a[0][1].total_cmp(&b[0][1]));
    let mut line = 0usize;
    let mut keyed: Vec<(usize, [Pt; 4])> = Vec::with_capacity(boxes.len());
    for (i, b) in boxes.iter().enumerate() {
        if i > 0 && b[0][1] - boxes[i - 1][0][1] >= LINE_GAP {
            line += 1;
        }
        keyed.push((line, *b));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then(a.1[0][0].total_cmp(&b.1[0][0])));
    keyed.into_iter().map(|(_, b)| b).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w` x `h` map with probability `p` inside each (x0, y0, x1, y1)
    /// block (ends exclusive) and 0.05 elsewhere.
    fn map(w: usize, h: usize, blocks: &[(usize, usize, usize, usize, f32)]) -> Vec<f32> {
        let mut m = vec![0.05f32; w * h];
        for &(x0, y0, x1, y1, p) in blocks {
            for y in y0..y1 {
                for x in x0..x1 {
                    m[y * w + x] = p;
                }
            }
        }
        m
    }

    #[test]
    fn a_block_gives_one_unclipped_box() {
        // A 40 x 8 block at (10, 6). The mask is dilated one pixel right
        // and down: pixels 10..=50 x 6..=14, a 40 x 8 rectangle of pixel
        // centers. Unclip distance: 320 * 1.6 / 96 = 5.33, so the grown
        // rectangle spans 5..=55 x 1..=19 (vertices rounded to whole
        // pixels by Clipper).
        let m = map(64, 24, &[(10, 6, 50, 14, 0.9)]);
        let b = boxes(&m, 64, 24, (64, 24));
        assert_eq!(
            b,
            vec![[[5.0, 1.0], [55.0, 1.0], [55.0, 19.0], [5.0, 19.0]]]
        );
        // Scaled to an image twice as large, rounded to whole pixels.
        let b = boxes(&m, 64, 24, (128, 48));
        assert_eq!(
            b,
            vec![[[10.0, 2.0], [110.0, 2.0], [110.0, 38.0], [10.0, 38.0]]]
        );
    }

    #[test]
    fn weak_small_and_thin_regions_are_dropped() {
        let m = map(
            80,
            40,
            &[
                // Above the pixel threshold but below the box threshold.
                (5, 5, 45, 15, 0.4),
                // Too thin: one row, two with the dilation.
                (5, 25, 45, 26, 0.95),
                // A dot.
                (60, 30, 61, 31, 0.95),
            ],
        );
        assert!(boxes(&m, 80, 40, (80, 40)).is_empty());
    }

    #[test]
    fn boxes_come_in_reading_order() {
        // Two boxes on one line (tops 3 pixels apart, the right one
        // higher) and one below; given bottom first.
        let m = map(
            200,
            60,
            &[
                (10, 40, 90, 50, 0.9),
                (110, 9, 190, 19, 0.9),
                (10, 12, 90, 22, 0.9),
            ],
        );
        let b = boxes(&m, 200, 60, (200, 60));
        let lefts: Vec<(f64, f64)> = b.iter().map(|q| (q[0][0], q[0][1])).collect();
        assert_eq!(lefts.len(), 3, "{b:?}");
        assert!(lefts[0].0 < 20.0 && lefts[0].1 < 20.0, "{lefts:?}");
        assert!(lefts[1].0 > 100.0, "{lefts:?}");
        assert!(lefts[2].1 > 30.0, "{lefts:?}");
    }

    #[test]
    fn a_slanted_block_gives_a_turned_box() {
        // A band 8 pixels high rising 1 pixel every 4 to the right.
        let (w, h) = (120, 60);
        let mut m = vec![0.0f32; w * h];
        for x in 10..110 {
            let top = 40 - x / 4;
            for y in top..top + 8 {
                m[y * w + x] = 0.9;
            }
        }
        let b = boxes(&m, w, h, (w, h));
        assert_eq!(b.len(), 1, "{b:?}");
        let [tl, tr, br, bl] = b[0];
        let slope = (tr[1] - tl[1]) / (tr[0] - tl[0]);
        assert!((slope + 0.25).abs() < 0.03, "{slope} {:?}", b[0]);
        assert!((br[1] - bl[1]) / (br[0] - bl[0]) < -0.2);
        assert!(tl[0] < bl[0] + 6.0 && tl[1] < bl[1]);
    }

    #[test]
    fn holes_are_outlines_of_their_own() {
        // A ring: the hole inside is an outline too, with its own
        // rectangle (scored low here, so only the ring's box is kept).
        let mut m = map(60, 40, &[(10, 5, 50, 35, 0.9)]);
        for y in 12..28 {
            for x in 20..40 {
                m[y * 60 + x] = 0.0;
            }
        }
        let mask = dilate(&threshold(&m), 60, 40);
        assert_eq!(outlines(&mask, 60, 40).len(), 2);
        assert_eq!(boxes(&m, 60, 40, (60, 40)).len(), 1);
    }

    #[test]
    fn min_area_rect_of_a_turned_square() {
        let pts = [[5.0, 0.0], [10.0, 5.0], [5.0, 10.0], [0.0, 5.0]];
        let (rect, side) = min_area_rect(&pts).unwrap();
        assert!((side - 50f64.sqrt()).abs() < 1e-9, "{side}");
        for c in rect {
            assert!(pts
                .iter()
                .any(|p| (p[0] - c[0]).abs() < 1e-9 && (p[1] - c[1]).abs() < 1e-9));
        }
        assert!(min_area_rect(&[]).is_none());
        assert_eq!(min_area_rect(&[[1.0, 1.0], [4.0, 1.0]]).unwrap().1, 0.0);
    }

    #[test]
    fn unclip_matches_clipper() {
        // A 10 x 4 rectangle: distance 40 * 1.6 / 28 = 2.29. The straight
        // edges move out by that, rounded: 2 pixels.
        let grown = unclip(&[[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [0.0, 4.0]]);
        let xs: Vec<f64> = grown.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = grown.iter().map(|p| p[1]).collect();
        let (x0, x1) = (
            xs.iter().copied().fold(f64::MAX, f64::min),
            xs.iter().copied().fold(f64::MIN, f64::max),
        );
        let (y0, y1) = (
            ys.iter().copied().fold(f64::MAX, f64::min),
            ys.iter().copied().fold(f64::MIN, f64::max),
        );
        assert_eq!((x0, x1, y0, y1), (-2.0, 12.0, -2.0, 6.0));
        // Round corners: arcs add vertices.
        assert!(grown.len() > 4, "{grown:?}");
        // Corners are truncated before the offset, either winding grows.
        let other = unclip(&[[0.9, 0.9], [0.9, 4.9], [10.9, 4.9], [10.9, 0.9]]);
        let (rect, _) = min_area_rect(&other).unwrap();
        let (r, _) = min_area_rect(&grown).unwrap();
        let sort = |mut v: [Pt; 4]| {
            v.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
            v
        };
        let near = sort(rect)
            .iter()
            .zip(sort(r).iter())
            .all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9);
        assert!(near, "{rect:?} {r:?}");
    }
}

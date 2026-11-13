//! The Rgba8 box filter in exact integer arithmetic.
//!
//! Along an axis that shrinks from `s` to `d` pixels (`d <= s`), measure
//! in units where a source pixel is `d` long and a destination pixel `s`
//! long. Every overlap is then an integer: a source pixel adds to a
//! destination pixel with an integer weight in `1..=d`, and the weights of
//! one destination pixel sum to `s`. A destination pixel's weights over
//! both axes therefore sum to `sw * sh`.
//!
//! 1. Vertical pass: for each destination row, the source rows that cover
//!    it are added into one `u32` sum per source column and channel, each
//!    row times its integer weight, up to [`simd::MAX_BATCH`] rows of equal
//!    weight per pass ([`simd::accumulate`]). Color is premultiplied
//!    (`C * A`, alpha stays `A`). On opaque rows the plain mode adds `C`
//!    instead, with no alpha work. A destination row starts in plain mode
//!    when the one above was opaque; the kernels report whether every
//!    alpha byte they read was 255, and a translucent pass in plain mode
//!    restarts the destination row premultiplied. A sum is at most
//!    `255 * 255 * sh`, below `2^32` because `sh <= 65535`.
//! 2. Horizontal pass: each destination pixel adds its column sums times
//!    their integer weights in `u64` (at most `255 * 255 * sw * sh`, below
//!    `2^46`).
//! 3. Rounding: with color sums `Sc`, alpha sum `Sa` and `n = sw * sh`,
//!    color is `Sc / Sa` (premultiplied) or `Sc / n` (opaque) and alpha is
//!    `Sa / n`, each rounded to the nearest integer, halves up
//!    ([`Divider`]). On opaque rows the premultiplied sums are exactly 255
//!    times the plain ones and `Sa = 255 * n`, so both forms are the same
//!    rational number and give the same byte.
//!
//! These are the exact values of the `f64` reference's formula, rounded
//! correctly. The reference rounds its float sums, so the two can differ
//! by one step where the exact average lies within float error of a half.

use super::simd::{self, Level};
use pandite_core::{ImageBuf, PixelFormat};

/// The source pixels that cover one destination pixel along an axis: a
/// run of `count` pixels from `first`. The first and the last may be
/// partial; every pixel between them has the full weight `d`.
#[derive(Clone, Copy, Debug)]
struct Cover {
    first: usize,
    count: usize,
    first_weight: u32,
    last_weight: u32,
}

impl Cover {
    /// The weight of the `k`-th source pixel of the run (`full` = `d`).
    fn weight(&self, k: usize, full: u32) -> u32 {
        if k == 0 {
            self.first_weight
        } else if k + 1 == self.count {
            self.last_weight
        } else {
            full
        }
    }
}

/// The covers of an `src` to `dst` reduction (`1 <= dst <= src`).
fn axis_plan(src: u32, dst: u32) -> Vec<Cover> {
    let (s, d) = (u64::from(src), u64::from(dst));
    (0..d)
        .map(|j| {
            let start = j * s;
            let end = start + s;
            let first = start / d;
            let last = (end - 1) / d;
            Cover {
                first: first as usize,
                count: (last - first + 1) as usize,
                first_weight: (((first + 1) * d).min(end) - start) as u32,
                last_weight: (end - (last * d).max(start)) as u32,
            }
        })
        .collect()
}

/// Shrink an Rgba8 image to `dw x dh` (each side at most the source side,
/// at least 1).
pub(super) fn downscale(level: Level, image: &ImageBuf, dw: u32, dh: u32) -> ImageBuf {
    let (sw, sh) = (image.width(), image.height());
    let level = if level.supported() {
        level
    } else {
        Level::Scalar
    };
    let cols = axis_plan(sw, dw);
    // The full-weight columns between each destination pixel's first and
    // last column.
    let inner: Vec<(usize, usize)> = cols
        .iter()
        .map(|c| (c.first + 1, c.count.saturating_sub(2)))
        .collect();
    let rows = axis_plan(sh, dh);
    let per_total = Divider::new(u64::from(sw) * u64::from(sh));
    let mut acc = vec![0u32; sw as usize * 4];
    let mut inner_sums = vec![[0u64; 4]; dw as usize];
    let mut batch: Vec<&[u8]> = Vec::with_capacity(simd::MAX_BATCH);
    let mut data = vec![0u8; dw as usize * dh as usize * 4];
    // Start each destination row in the mode the previous one needed.
    let mut premultiplied = false;
    for (cover, out) in rows.iter().zip(data.chunks_exact_mut(dw as usize * 4)) {
        acc.fill(0);
        let mut opaque = true;
        let mut k = 0;
        while k < cover.count {
            // One pass adds consecutive rows of the same weight.
            let weight = cover.weight(k, dh);
            batch.clear();
            while k < cover.count && batch.len() < simd::MAX_BATCH && cover.weight(k, dh) == weight
            {
                batch.push(image.row((cover.first + k) as u32));
                k += 1;
            }
            if !simd::accumulate(level, &batch, weight, premultiplied, &mut acc) {
                opaque = false;
                if !premultiplied {
                    // Plain sums only hold opaque pixels: start this
                    // destination row again, premultiplied.
                    premultiplied = true;
                    acc.fill(0);
                    k = 0;
                }
            }
        }
        simd::sum_runs(level, &acc, &inner, &mut inner_sums);
        finish_row(&acc, &cols, &inner_sums, dw, &per_total, premultiplied, out);
        premultiplied = !opaque;
    }
    ImageBuf::new(dw, dh, PixelFormat::Rgba8, data).unwrap_or_else(|_| image.clone())
}

/// The horizontal pass and the rounding for one destination row:
/// `inner_sums` holds the totals of each destination pixel's full-weight
/// columns, `full` is their weight.
fn finish_row(
    acc: &[u32],
    cols: &[Cover],
    inner_sums: &[[u64; 4]],
    full: u32,
    per_total: &Divider,
    premultiplied: bool,
    out: &mut [u8],
) {
    let full = u64::from(full);
    for (j, cover) in cols.iter().enumerate() {
        let inner = &inner_sums[j];
        let px = &mut out[j * 4..j * 4 + 4];
        let head = &acc[cover.first * 4..cover.first * 4 + 4];
        let first_weight = u64::from(cover.first_weight);
        let sum: [u64; 4] = if cover.count == 1 {
            std::array::from_fn(|c| u64::from(head[c]) * first_weight)
        } else {
            let last = (cover.first + cover.count - 1) * 4;
            let tail = &acc[last..last + 4];
            let last_weight = u64::from(cover.last_weight);
            std::array::from_fn(|c| {
                u64::from(head[c]) * first_weight
                    + inner[c] * full
                    + u64::from(tail[c]) * last_weight
            })
        };
        finish(sum, per_total, premultiplied, px);
    }
}

/// Write one pixel from its weighted sums (see the module documentation).
/// `per_total` divides by `sw * sh`.
fn finish(sum: [u64; 4], per_total: &Divider, premultiplied: bool, px: &mut [u8]) {
    if premultiplied && sum[3] == 0 {
        px.fill(0);
        return;
    }
    let per_alpha;
    let color = if premultiplied {
        per_alpha = Divider::new(sum[3]);
        &per_alpha
    } else {
        per_total
    };
    for (o, &s) in px[..3].iter_mut().zip(&sum[..3]) {
        *o = color.round(s);
    }
    px[3] = per_total.round(sum[3]);
}

/// Division by a fixed `d` (1 to `2^38`: at most `255 * sw * sh`),
/// rounded to the nearest integer with halves up, for quotients up to
/// 255: a multiply by a rounded-up reciprocal, then an exact correction.
struct Divider {
    /// `2 * d`.
    twice: u64,
    /// `ceil(2^64 / (2 * d))`.
    magic: u64,
}

impl Divider {
    fn new(d: u64) -> Divider {
        let twice = 2 * d;
        Divider {
            twice,
            magic: u64::MAX / twice + 1,
        }
    }

    /// `n / d` rounded, halves up: `floor(num / 2d)` with `num = 2n + d`.
    /// Here `n <= 255 * d`, so `num <= 511 * d`. The estimate
    /// `num * magic / 2^64` is at least `num / 2d` and exceeds it by less
    /// than `num / 2^64 < 2^-17`: it is the quotient or one more.
    fn round(&self, n: u64) -> u8 {
        let num = 2 * n + self.twice / 2;
        let q = ((u128::from(num) * u128::from(self.magic)) >> 64) as u64;
        (q - u64::from(q * self.twice > num)) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_plan_weights_sum_to_the_source_size() {
        for (s, d) in [
            (1u32, 1u32),
            (7, 7),
            (3, 2),
            (257, 256),
            (1000, 7),
            (65535, 1),
        ] {
            let plan = axis_plan(s, d);
            assert_eq!(plan.len(), d as usize);
            let mut next = 0usize;
            for c in &plan {
                let sum: u64 = (0..c.count).map(|k| u64::from(c.weight(k, d))).sum();
                assert_eq!(sum, u64::from(s), "{s} -> {d}");
                assert!(c.first_weight >= 1 && c.first_weight <= d);
                assert!(c.last_weight >= 1 && c.last_weight <= d);
                // Runs touch or share one partial pixel, in order.
                assert!(c.first == next || c.first + 1 == next, "{s} -> {d}");
                next = c.first + c.count;
            }
            assert_eq!(next, s as usize);
        }
    }

    #[test]
    fn divider_rounds_halves_up_exactly() {
        let exact = |n: u64, d: u64| ((2 * n + d) / (2 * d)) as u8;
        let check = |n: u64, d: u64| assert_eq!(Divider::new(d).round(n), exact(n, d), "{n} / {d}");
        check(1, 2);
        check(255, 2);
        check(254, 2);
        check(3, 4);
        check(1, 4);
        check(255 * 7, 7);
        // Every quotient and the values around each half, for small and
        // for the largest divisors.
        let big = 1u64 << 46;
        for d in [
            1u64,
            2,
            3,
            7,
            255,
            65_025,
            1 << 30,
            big / 255 - 1,
            big / 255,
        ] {
            for q in 0..=255u64 {
                let half = q * d + d / 2;
                for n in [q * d, half.saturating_sub(1), half, half + 1] {
                    if n <= 255 * d {
                        check(n, d);
                    }
                }
            }
        }
        let mut s = 0x1234_5678u64;
        for _ in 0..100_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let d = 1 + s % (big / 255);
            check(s % (255 * d + 1), d);
        }
    }
}

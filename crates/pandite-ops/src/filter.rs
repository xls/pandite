//! Resampling filters and the per-axis weight tables built from them.
//!
//! Resizing is separable: one horizontal pass and one vertical pass. Each
//! pass is driven by a weight table that lists, for every output sample, the
//! run of source samples it reads and the weight of each.
//!
//! Anti-aliasing: when an axis shrinks by a ratio `r > 1`, the kernel is
//! stretched by `r` (see `Weights::new`). Its support then covers
//! `support * r` source pixels on each side of the output pixel center and it
//! low-pass filters at the new, lower sampling rate. A 10x reduction with
//! Lanczos3 therefore reads about 60 source pixels per output pixel instead
//! of 6, so fine detail averages out instead of aliasing. When an axis grows,
//! the kernel keeps its natural width and interpolates.
//!
//! Edges: taps that fall outside the image are mirrored back inside around
//! the first and last pixel centers (whole-sample symmetric reflection, the
//! `gfedcb|abcdefgh|gfedcba` rule). The kernel keeps its full symmetric shape
//! at the borders, so flat areas stay flat and nothing darkens or smears at
//! the edge.

use std::f64::consts::PI;

/// A resampling filter for [`crate::resize()`].
///
/// Every filter except [`Filter::Nearest`] widens with the reduction ratio
/// when downscaling, so all of them are safe against aliasing; they differ in
/// sharpness and ringing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Filter {
    /// Copy the nearest source pixel. Keeps hard pixel edges (pixel art,
    /// screenshots at integer zoom) but aliases when downscaling.
    Nearest,
    /// Area average: every output pixel is the exact coverage-weighted mean
    /// of the source pixels under it. Never rings. Shrinking is exact at
    /// integer ratios; at fractional ratios very fine patterns leave a faint
    /// residue. Enlarging gives sharp pixel blocks whose boundaries blend
    /// ("pixel mixing"), good for pixel art at non-integer scales.
    Box,
    /// Linear interpolation (tent filter). Soft, never rings.
    Triangle,
    /// Catmull-Rom cubic (B = 0, C = 0.5). Sharp with slight ringing.
    CatmullRom,
    /// Mitchell-Netravali cubic (B = C = 1/3). Balanced, very little ringing.
    Mitchell,
    /// Lanczos windowed sinc with three lobes. The sharpest; the default.
    #[default]
    Lanczos3,
}

impl Filter {
    /// Every filter, in menu order from fastest to sharpest.
    pub const ALL: [Filter; 6] = [
        Filter::Nearest,
        Filter::Box,
        Filter::Triangle,
        Filter::CatmullRom,
        Filter::Mitchell,
        Filter::Lanczos3,
    ];

    /// A short lowercase name for settings files, for example `"lanczos3"`.
    pub const fn name(self) -> &'static str {
        match self {
            Filter::Nearest => "nearest",
            Filter::Box => "box",
            Filter::Triangle => "triangle",
            Filter::CatmullRom => "catmullrom",
            Filter::Mitchell => "mitchell",
            Filter::Lanczos3 => "lanczos3",
        }
    }

    /// The filter whose [`Filter::name`] matches `name`, ignoring case.
    pub fn from_name(name: &str) -> Option<Filter> {
        Filter::ALL
            .into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
    }

    /// Kernel radius in source pixels at 1:1 scale. When downscaling by a
    /// ratio `r`, the radius used is `support() * r`.
    pub const fn support(self) -> f64 {
        match self {
            Filter::Nearest | Filter::Box => 0.5,
            Filter::Triangle => 1.0,
            Filter::CatmullRom | Filter::Mitchell => 2.0,
            Filter::Lanczos3 => 3.0,
        }
    }

    /// The kernel value at distance `x`, measured in (stretched) pixels.
    fn kernel(self, x: f64) -> f64 {
        let x = x.abs();
        match self {
            Filter::Nearest | Filter::Box => {
                if x < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Filter::Triangle => (1.0 - x).max(0.0),
            Filter::CatmullRom => cubic(x, 0.0, 0.5),
            Filter::Mitchell => cubic(x, 1.0 / 3.0, 1.0 / 3.0),
            Filter::Lanczos3 => {
                if x < 3.0 {
                    sinc(x) * sinc(x / 3.0)
                } else {
                    0.0
                }
            }
        }
    }
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 {
        1.0
    } else {
        let a = PI * x;
        a.sin() / a
    }
}

/// The Mitchell-Netravali family of cubics for `x >= 0`.
fn cubic(x: f64, b: f64, c: f64) -> f64 {
    let (x2, x3) = (x * x, x * x * x);
    if x < 1.0 {
        ((12.0 - 9.0 * b - 6.0 * c) * x3 + (-18.0 + 12.0 * b + 6.0 * c) * x2 + (6.0 - 2.0 * b))
            / 6.0
    } else if x < 2.0 {
        ((-b - 6.0 * c) * x3
            + (6.0 * b + 30.0 * c) * x2
            + (-12.0 * b - 48.0 * c) * x
            + (8.0 * b + 24.0 * c))
            / 6.0
    } else {
        0.0
    }
}

/// Map any integer sample index into `0..n` by whole-sample symmetric
/// reflection: `-1 -> 1`, `-2 -> 2`, `n -> n - 2`. Repeats for indices far
/// outside, so it works for kernels wider than the image.
fn reflect(j: i64, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let n = n as i64;
    let period = 2 * (n - 1);
    let m = j.rem_euclid(period);
    (if m >= n { period - m } else { m }) as usize
}

/// The weight of source pixel `j` for an output pixel centered at `center`
/// (source coordinates, pixel `j` spans `j..j + 1`). `ratio` is source
/// pixels per output pixel; `stretch = max(1, ratio)` is the kernel widening
/// factor.
fn tap(filter: Filter, j: i64, center: f64, ratio: f64, stretch: f64) -> f64 {
    let left = j as f64;
    match filter {
        // Exact area coverage of pixel j by the output pixel's footprint,
        // `ratio` source pixels wide, when shrinking and when growing.
        Filter::Box => {
            let half = 0.5 * ratio;
            ((left + 1.0).min(center + half) - left.max(center - half)).max(0.0)
        }
        _ => filter.kernel((left + 0.5 - center) / stretch),
    }
}

/// The weight table for one axis: for output sample `i`, source samples
/// `start(i)..end(i)` with weights `taps(i).1`. Weights of each output
/// sample sum to 1. Starts and ends never decrease from one output sample to
/// the next, which the streaming vertical pass relies on.
#[derive(Debug)]
pub(crate) struct Weights {
    starts: Vec<usize>,
    bounds: Vec<usize>,
    coeffs: Vec<f32>,
    max_len: usize,
    max_open: usize,
    identity: bool,
}

impl Weights {
    /// Build the table that resamples `src` samples to `dst` samples.
    ///
    /// This is where the filter support scales with the reduction ratio:
    /// `stretch = max(1, src / dst)` widens the kernel, so the window covers
    /// `filter.support() * stretch` source pixels on each side.
    pub(crate) fn new(src: usize, dst: usize, filter: Filter) -> Self {
        assert!(src > 0 && dst > 0, "weight table needs non-empty axes");
        if src == dst {
            return Self::identity(src);
        }
        let ratio = src as f64 / dst as f64;
        // Downscaling by `ratio` stretches the kernel by the same factor.
        let stretch = ratio.max(1.0);
        let support = filter.support() * stretch;
        // Per output sample: (first source index, tap count, offset in `raw`).
        let mut spans: Vec<(usize, usize, usize)> = Vec::with_capacity(dst);
        let mut raw: Vec<f32> = Vec::new();
        let mut acc: Vec<f64> = Vec::new();
        for i in 0..dst {
            let center = (i as f64 + 0.5) * ratio;
            let nearest = (center.floor() as usize).min(src - 1);
            let offset = raw.len();
            if filter == Filter::Nearest {
                raw.push(1.0);
                spans.push((nearest, 1, offset));
                continue;
            }
            // Every pixel whose center lies within `support` of `center`.
            let lo = (center - support - 0.5).floor() as i64;
            let hi = (center + support - 0.5).ceil() as i64;
            let (mut fmin, mut fmax) = (usize::MAX, 0);
            for j in lo..=hi {
                let f = reflect(j, src);
                fmin = fmin.min(f);
                fmax = fmax.max(f);
            }
            acc.clear();
            acc.resize(fmax - fmin + 1, 0.0);
            for j in lo..=hi {
                acc[reflect(j, src) - fmin] += tap(filter, j, center, ratio, stretch);
            }
            let sum: f64 = acc.iter().sum();
            let first = acc.iter().position(|&w| w != 0.0);
            let last = acc.iter().rposition(|&w| w != 0.0);
            match (first, last) {
                (Some(a), Some(b)) if sum.abs() > 1e-9 && sum.is_finite() => {
                    raw.extend(acc[a..=b].iter().map(|&w| (w / sum) as f32));
                    spans.push((fmin + a, b - a + 1, offset));
                }
                // Cannot happen for the built-in kernels; stay well defined.
                _ => {
                    raw.push(1.0);
                    spans.push((nearest, 1, offset));
                }
            }
        }
        Self::assemble(&spans, &raw)
    }

    /// The table for an axis that keeps its size: every sample copies itself.
    fn identity(n: usize) -> Self {
        Self {
            starts: (0..n).collect(),
            bounds: (0..=n).collect(),
            coeffs: vec![1.0; n],
            max_len: 1,
            max_open: 1,
            identity: true,
        }
    }

    /// Pad the windows with zero weights where needed so starts and ends
    /// never decrease, then lay them out flat.
    fn assemble(spans: &[(usize, usize, usize)], raw: &[f32]) -> Self {
        let mut windows: Vec<(usize, usize)> =
            spans.iter().map(|&(s, len, _)| (s, s + len)).collect();
        let mut next_start = usize::MAX;
        for w in windows.iter_mut().rev() {
            w.0 = w.0.min(next_start);
            next_start = w.0;
        }
        let mut prev_end = 0;
        for w in windows.iter_mut() {
            w.1 = w.1.max(prev_end);
            prev_end = w.1;
        }
        let mut starts = Vec::with_capacity(spans.len());
        let mut bounds = Vec::with_capacity(spans.len() + 1);
        let mut coeffs = Vec::with_capacity(raw.len());
        bounds.push(0);
        for (&(start, end), &(s, len, offset)) in windows.iter().zip(spans) {
            starts.push(start);
            coeffs.resize(coeffs.len() + (s - start), 0.0);
            coeffs.extend_from_slice(&raw[offset..offset + len]);
            coeffs.resize(coeffs.len() + (end - s - len), 0.0);
            bounds.push(coeffs.len());
        }
        let max_len = windows.iter().map(|&(s, e)| e - s).max().unwrap_or(1);
        // Largest number of windows that contain one source sample. With
        // monotone windows this is a two-pointer sweep.
        let mut max_open = 1;
        let mut closed = 0;
        for (i, &(start, _)) in windows.iter().enumerate() {
            while windows[closed].1 <= start {
                closed += 1;
            }
            max_open = max_open.max(i + 1 - closed);
        }
        Self {
            starts,
            bounds,
            coeffs,
            max_len,
            max_open,
            identity: false,
        }
    }

    /// First source sample and weights for output sample `i`.
    pub(crate) fn taps(&self, i: usize) -> (usize, &[f32]) {
        (
            self.starts[i],
            &self.coeffs[self.bounds[i]..self.bounds[i + 1]],
        )
    }

    /// First source sample read by output sample `i`.
    pub(crate) fn start(&self, i: usize) -> usize {
        self.starts[i]
    }

    /// One past the last source sample read by output sample `i`.
    pub(crate) fn end(&self, i: usize) -> usize {
        self.starts[i] + self.bounds[i + 1] - self.bounds[i]
    }

    /// Longest window, in source samples.
    pub(crate) fn max_len(&self) -> usize {
        self.max_len
    }

    /// Most output samples whose windows share one source sample.
    pub(crate) fn max_open(&self) -> usize {
        self.max_open
    }

    /// Resample one row of RGBA float pixels (4 floats each) from `src` into
    /// `dst`, which holds one pixel per output sample.
    pub(crate) fn apply(&self, src: &[f32], dst: &mut [f32]) {
        if self.identity {
            dst.copy_from_slice(&src[..dst.len()]);
            return;
        }
        for (i, out) in dst.chunks_exact_mut(4).enumerate() {
            let (start, weights) = self.taps(i);
            let pixels = &src[start * 4..(start + weights.len()) * 4];
            let mut acc = [0.0f32; 4];
            for (p, &w) in pixels.chunks_exact(4).zip(weights) {
                acc[0] += p[0] * w;
                acc[1] += p[1] * w;
                acc[2] += p[2] * w;
                acc[3] += p[3] * w;
            }
            out.copy_from_slice(&acc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [(usize, usize); 12] = [
        (1, 1),
        (1, 7),
        (7, 1),
        (2, 3),
        (3, 2),
        (10, 3),
        (37, 256),
        (256, 37),
        (1000, 3),
        (3, 1000),
        (640, 480),
        (65_535, 1),
    ];

    #[test]
    fn weights_are_normalized_and_monotone() {
        for filter in Filter::ALL {
            for (src, dst) in SIZES {
                let w = Weights::new(src, dst, filter);
                let mut prev = (0, 0);
                for i in 0..dst {
                    let (start, taps) = w.taps(i);
                    assert!(!taps.is_empty());
                    assert!(start + taps.len() <= src, "{filter:?} {src}->{dst} row {i}");
                    let sum: f64 = taps.iter().map(|&t| f64::from(t)).sum();
                    assert!((sum - 1.0).abs() < 1e-5, "{filter:?} {src}->{dst}: {sum}");
                    assert!(w.start(i) >= prev.0 && w.end(i) >= prev.1);
                    assert!(w.end(i) - w.start(i) <= w.max_len());
                    prev = (w.start(i), w.end(i));
                }
                assert!(w.max_open() >= 1);
            }
        }
    }

    #[test]
    fn support_grows_with_the_reduction_ratio() {
        // Lanczos3 reads about 2 * 3 * ratio source pixels per output pixel.
        let mild = Weights::new(200, 100, Filter::Lanczos3);
        let heavy = Weights::new(200, 10, Filter::Lanczos3);
        let (_, mild_taps) = mild.taps(50);
        let (_, heavy_taps) = heavy.taps(5);
        assert!((11..=13).contains(&mild_taps.len()), "{}", mild_taps.len());
        assert!(
            (119..=121).contains(&heavy_taps.len()),
            "{}",
            heavy_taps.len()
        );
        // Upscaling keeps the natural width.
        let up = Weights::new(10, 100, Filter::Lanczos3);
        assert!(up.taps(50).1.len() <= 7);
        // Box at an integer ratio averages exactly that many pixels.
        let boxed = Weights::new(12, 3, Filter::Box);
        let (start, taps) = boxed.taps(1);
        assert_eq!(start, 4);
        assert_eq!(taps, &[0.25, 0.25, 0.25, 0.25]);
    }

    #[test]
    fn box_uses_fractional_coverage() {
        // 3 -> 2: each output covers 1.5 source pixels.
        let w = Weights::new(3, 2, Filter::Box);
        let (start, taps) = w.taps(0);
        assert_eq!(start, 0);
        assert!((taps[0] - 2.0 / 3.0).abs() < 1e-6 && (taps[1] - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn box_enlarges_into_blocks_with_blended_boundaries() {
        // 2 -> 4: every output footprint lies inside one source pixel.
        let w = Weights::new(2, 4, Filter::Box);
        let picks: Vec<(usize, Vec<f32>)> = (0..4)
            .map(|i| (w.taps(i).0, w.taps(i).1.to_vec()))
            .collect();
        assert_eq!(
            picks,
            vec![
                (0, vec![1.0]),
                (0, vec![1.0]),
                (1, vec![1.0]),
                (1, vec![1.0])
            ]
        );
        // 2 -> 3: the middle output straddles the boundary and blends 50/50.
        let w = Weights::new(2, 3, Filter::Box);
        assert_eq!(w.taps(0), (0, &[1.0f32][..]));
        let (start, mid) = w.taps(1);
        assert_eq!(start, 0);
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[1] - 0.5).abs() < 1e-6);
        // Unlike a tent filter, which blends every enlarged pixel.
        let tent = Weights::new(2, 4, Filter::Triangle);
        assert_eq!(tent.taps(1).1.len(), 2);
    }

    #[test]
    fn reflection_mirrors_around_edge_pixels() {
        let got: Vec<usize> = (-4..9).map(|j| reflect(j, 5)).collect();
        assert_eq!(got, vec![4, 3, 2, 1, 0, 1, 2, 3, 4, 3, 2, 1, 0]);
        assert_eq!(reflect(-7, 1), 0);
        assert_eq!(reflect(3, 2), 1);
    }

    #[test]
    fn kernels_have_expected_shape() {
        for filter in Filter::ALL {
            // Mitchell is not interpolating: its peak is (6 - 2B) / 6.
            let peak = if filter == Filter::Mitchell {
                8.0 / 9.0
            } else {
                1.0
            };
            assert!((filter.kernel(0.0) - peak).abs() < 1e-12, "{filter:?}");
            assert_eq!(filter.kernel(filter.support() + 0.01), 0.0);
        }
        // Interpolating kernels vanish at integer offsets.
        assert!(Filter::CatmullRom.kernel(1.0).abs() < 1e-12);
        assert!(Filter::Lanczos3.kernel(2.0).abs() < 1e-12);
        assert!((Filter::Mitchell.kernel(1.0) - 1.0 / 18.0).abs() < 1e-12);
        assert_eq!(Filter::from_name("LANCZOS3"), Some(Filter::Lanczos3));
        assert_eq!(Filter::from_name("bogus"), None);
        assert_eq!(Filter::default(), Filter::Lanczos3);
    }

    #[test]
    fn apply_resamples_a_row() {
        let w = Weights::new(4, 2, Filter::Box);
        let src = [
            0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5,
        ];
        let mut dst = [0.0f32; 8];
        w.apply(&src, &mut dst);
        assert_eq!(dst, [0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5]);
    }
}

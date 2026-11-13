//! The vector kernels against their scalar references (gate G3).

use super::*;
use crate::raw::tone::Tone;

/// A deterministic pseudo-random sequence (xorshift32).
pub(super) struct Rng(u32);

impl Rng {
    pub(super) fn new(seed: u32) -> Self {
        Rng(seed.max(1))
    }

    pub(super) fn next(&mut self) -> u32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s
    }

    /// A float in `lo..hi`.
    pub(super) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
}

/// Every level this CPU can run, besides scalar.
fn vector_levels() -> Vec<Level> {
    [Level::Avx2, Level::Neon]
        .into_iter()
        .filter(|&l| supported(l))
        .collect()
}

/// Edge values for one channel: bounds, just outside, tiny, huge, NaN.
const EDGES: [f32; 14] = [
    0.0,
    -0.0,
    1.0,
    -1e-7,
    1.000_000_1,
    1e-38,
    -1e-38,
    0.5,
    0.999_999,
    2.0,
    -3.5,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NAN,
];

fn matrices() -> Vec<[[f32; 3]; 3]> {
    vec![
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        // ILCE-7RM5 row-normalized (R-ARW-51 shape).
        [
            [1.95 / 1.0, -0.87 / 1.0, -0.08 / 1.0],
            [-0.15, 1.58, -0.43],
            [0.03, -0.45, 1.41],
        ],
        [[2.5, -1.2, -0.3], [-0.4, 1.9, -0.5], [0.1, -0.9, 1.8]],
    ]
}

fn planes(cam: &[[f32; 3]]) -> [Vec<f32>; 3] {
    [0, 1, 2].map(|c| cam.iter().map(|p| p[c]).collect())
}

fn run_both(level: Level, m: &[[f32; 3]; 3], tone: &Tone, cam: &[[f32; 3]]) -> (Vec<u8>, Vec<u8>) {
    let p = planes(cam);
    let view = [&p[0][..], &p[1][..], &p[2][..]];
    let mut want = vec![0u8; cam.len() * 4];
    let mut got = vec![0xAAu8; cam.len() * 4];
    scalar::matrix_tone_rgba8(m, tone, view, &mut want);
    matrix_tone_rgba8_with(level, m, tone, view, &mut got);
    (want, got)
}

/// Bit patterns of a float slice, so NaN compares equal to itself.
fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

/// The Bayer kernel at `level` and in scalar on the same rows.
#[allow(clippy::too_many_arguments)]
fn bayer_both(
    level: Level,
    rows: &[Vec<f32>; 3],
    i0: usize,
    n: usize,
    green_first: bool,
    q: usize,
) -> ([Vec<u32>; 3], [Vec<u32>; 3]) {
    let mut want = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    let mut got = [vec![-9f32; n], vec![-9f32; n], vec![-9f32; n]];
    {
        let [a, b, c] = &mut want;
        scalar::bayer_row(
            &rows[0],
            &rows[1],
            &rows[2],
            i0,
            n,
            green_first,
            q,
            [a, b, c],
        );
    }
    {
        let [a, b, c] = &mut got;
        bayer_row_with(
            level,
            &rows[0],
            &rows[1],
            &rows[2],
            i0,
            n,
            green_first,
            q,
            [a, b, c],
        );
    }
    (want.map(|p| bits(&p)), got.map(|p| bits(&p)))
}

fn normalize_both(
    level: Level,
    src: &[u16],
    odd: bool,
    black: [f32; 2],
    scale: [f32; 2],
) -> (Vec<u32>, Vec<u32>) {
    let mut want = vec![0f32; src.len()];
    let mut got = vec![-1f32; src.len()];
    scalar::normalize_row(src, odd, black, scale, &mut want);
    normalize_row_with(level, src, odd, black, scale, &mut got);
    (bits(&want), bits(&got))
}

/// Gate G3: every vector routine gives the same bytes as its scalar
/// version on random rows, on edge values (bounds, values just outside,
/// denormals, infinities, NaN, out-of-gamut colors) and on every row
/// length from 0 to 40, so each vector tail size and each row end is
/// covered.
#[test]
fn simd_matches_scalar_exactly() {
    let levels = vector_levels();
    let tones = [Tone::unit().clone(), Tone::new(2.0), Tone::new(0.7)];
    let mut rng = Rng::new(0x5EED);
    let mut compared = 0usize;
    for &level in &levels {
        for m in matrices() {
            for tone in &tones {
                // Every length, random values over and around 0..1.
                for len in 0..=40usize {
                    let cam: Vec<[f32; 3]> = (0..len)
                        .map(|_| [0, 1, 2].map(|_| rng.range(-0.2, 1.3)))
                        .collect();
                    let (want, got) = run_both(level, &m, tone, &cam);
                    assert_eq!(want, got, "{level:?} random row of {len}");
                    compared += len;
                }
                // Long rows with mostly in-range values (the real case).
                let cam: Vec<[f32; 3]> = (0..9_504 + 5)
                    .map(|_| [0, 1, 2].map(|_| rng.range(0.0, 1.0).powi(3)))
                    .collect();
                let (want, got) = run_both(level, &m, tone, &cam);
                assert_eq!(want, got, "{level:?} long row");
                compared += cam.len();
                // Every combination of edge values in every lane position.
                let mut cam = Vec::new();
                for &a in &EDGES {
                    for &b in &EDGES {
                        for &c in &EDGES {
                            cam.push([a, b, c]);
                        }
                    }
                }
                for shift in 0..8 {
                    let (want, got) = run_both(level, &m, tone, &cam[shift..]);
                    assert_eq!(want, got, "{level:?} edge values, shift {shift}");
                    compared += cam.len() - shift;
                }
                // Values on the table steps and between them.
                let cam: Vec<[f32; 3]> = (0..4096 * 2)
                    .map(|i| {
                        let v = i as f32 / (2.0 * 4095.0);
                        [v, v * 0.5, (1.0 - v).max(0.0)]
                    })
                    .collect();
                let (want, got) = run_both(level, &m, tone, &cam);
                assert_eq!(want, got, "{level:?} table steps");
                compared += cam.len();
            }
        }
    }
    // The Bayer interior kernel: every length and start parity, both
    // row colors, random rows and rows of edge values.
    let mut bayer = 0usize;
    for &level in &levels {
        for len in 3..=44usize {
            let rows: [Vec<f32>; 3] =
                [0, 1, 2].map(|_| (0..len).map(|_| rng.range(0.0, 1.0)).collect());
            for i0 in 1..len.min(4) {
                for n in 0..len - i0 {
                    for green_first in [false, true] {
                        for q in [0usize, 2] {
                            let (want, got) = bayer_both(level, &rows, i0, n, green_first, q);
                            assert_eq!(
                                want, got,
                                "{level:?} bayer len {len} i0 {i0} n {n} {green_first} {q}"
                            );
                            bayer += n;
                        }
                    }
                }
            }
        }
        let edge_row: Vec<f32> = EDGES.iter().cycle().take(9_504 + 3).copied().collect();
        let rows = [
            edge_row.clone(),
            edge_row.iter().rev().copied().collect(),
            edge_row,
        ];
        for green_first in [false, true] {
            let (want, got) = bayer_both(level, &rows, 1, 9_504, green_first, 2);
            assert_eq!(want, got, "{level:?} bayer edge values");
            bayer += 9_504;
        }
    }
    // The normalization kernel: every length and parity, random and edge
    // samples, real and extreme constants.
    let mut norm = 0usize;
    for &level in &levels {
        let consts = [
            ([512.0f32, 512.0], [2.372 / 15_871.0, 1.0 / 15_871.0]),
            ([0.0, 800.0], [1.0, 0.5]),
            ([65_535.0, 0.0], [1e-9, 1e9]),
        ];
        for (black, scale) in consts {
            for len in 0..=40usize {
                let src: Vec<u16> = (0..len).map(|_| rng.next() as u16).collect();
                for odd in [false, true] {
                    let (want, got) = normalize_both(level, &src, odd, black, scale);
                    assert_eq!(want, got, "{level:?} normalize len {len} odd {odd}");
                    norm += len;
                }
            }
            let src: Vec<u16> = [0u16, 1, 511, 512, 513, 16_383, 16_384, 65_534, 65_535]
                .iter()
                .cycle()
                .take(9_504 + 7)
                .copied()
                .collect();
            for odd in [false, true] {
                let (want, got) = normalize_both(level, &src, odd, black, scale);
                assert_eq!(want, got, "{level:?} normalize edge samples");
                norm += src.len();
            }
        }
    }
    println!(
        "vector levels on this CPU: {levels:?}; compared with the scalar reference: {compared} pixels of matrix and tone, {bayer} of Bayer interpolation, {norm} samples of normalization"
    );
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        assert!(levels.contains(&Level::Avx2), "AVX2 path tested");
    }
    #[cfg(target_arch = "aarch64")]
    assert!(levels.contains(&Level::Neon), "NEON path tested");
}

/// Gate G3: without the CPU feature the scalar path runs. The level
/// choice for a CPU without AVX2 or NEON is scalar; dispatching at the
/// scalar level, or at a level this CPU lacks, runs the scalar reference
/// (counted), and the detected level runs the vector path without
/// calling it for whole vectors.
#[test]
fn scalar_path_used_without_cpu_feature() {
    assert_eq!(select(Features::default()), Level::Scalar);
    assert_eq!(
        select(Features {
            avx2: true,
            neon: false
        }),
        Level::Avx2
    );
    assert_eq!(
        select(Features {
            avx2: false,
            neon: true
        }),
        Level::Neon
    );
    let m = matrices()[1];
    let tone = Tone::unit();
    let mut rng = Rng::new(7);
    // A multiple of every vector width: a vector path needs no scalar tail.
    let cam: Vec<[f32; 3]> = (0..64)
        .map(|_| [0, 1, 2].map(|_| rng.range(0.0, 1.0)))
        .collect();
    let p = planes(&cam);
    let cam = [&p[0][..], &p[1][..], &p[2][..]];
    let mut reference = vec![0u8; 64 * 4];
    scalar::matrix_tone_rgba8(&m, tone, cam, &mut reference);
    let calls = || scalar::CALLS.with(std::cell::Cell::get);
    let mut out = vec![0u8; 64 * 4];
    let before = calls();
    matrix_tone_rgba8_with(Level::Scalar, &m, tone, cam, &mut out);
    assert_eq!(calls(), before + 1, "the scalar level runs the scalar code");
    assert_eq!(out, reference);
    // A level whose feature this CPU lacks falls back to scalar.
    let missing = [Level::Avx2, Level::Neon]
        .into_iter()
        .find(|&l| !supported(l))
        .expect("no CPU has both AVX2 and NEON");
    let before = calls();
    let mut out = vec![0u8; 64 * 4];
    matrix_tone_rgba8_with(missing, &m, tone, cam, &mut out);
    assert_eq!(calls(), before + 1, "{missing:?} is absent: scalar ran");
    assert_eq!(out, reference);
    // The detected vector level does not call the scalar code at all.
    if let Some(&level) = vector_levels().first() {
        let before = calls();
        let mut out = vec![0u8; 64 * 4];
        matrix_tone_rgba8_with(level, &m, tone, cam, &mut out);
        assert_eq!(calls(), before, "{level:?} ran the vector code");
        assert_eq!(out, reference);
    }
    // The Bayer and normalization kernels fall back the same way.
    let rows: [Vec<f32>; 3] = [0, 1, 2].map(|_| (0..66).map(|_| rng.range(0.0, 1.0)).collect());
    let before = calls();
    let (want, got) = bayer_both(missing, &rows, 1, 64, true, 0);
    assert_eq!(calls(), before + 2, "scalar reference plus the fallback");
    assert_eq!(want, got);
    let src: Vec<u16> = (0..64).map(|_| rng.next() as u16).collect();
    let before = calls();
    let (want, got) = normalize_both(missing, &src, false, [512.0, 512.0], [1e-4, 2e-4]);
    assert_eq!(calls(), before + 2, "scalar reference plus the fallback");
    assert_eq!(want, got);
    println!("detected level {:?} (features {:?})", level(), detect());
}

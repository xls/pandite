//! The vector kernels against their scalar references.

use super::*;

/// A deterministic pseudo-random sequence (xorshift32).
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

/// Every level this CPU can run, besides scalar.
fn vector_levels() -> Vec<Level> {
    Level::available()
        .into_iter()
        .filter(|&l| l != Level::Scalar)
        .collect()
}

/// Rows of `len` pixels: random, all 255, opaque with random color,
/// alternating black and white, and opaque but for one alpha of 254.
fn rows_of(len: usize, rng: &mut Rng) -> Vec<Vec<u8>> {
    let mut one_translucent = vec![255u8; len * 4];
    if len > 0 {
        one_translucent[(rng.next() as usize % len) * 4 + 3] = 254;
    }
    vec![
        (0..len * 4).map(|_| rng.byte()).collect(),
        vec![255; len * 4],
        (0..len * 4)
            .map(|i| if i % 4 == 3 { 255 } else { rng.byte() })
            .collect(),
        (0..len * 4).map(|i| [0, 255][(i / 4 + i) % 2]).collect(),
        one_translucent,
    ]
}

#[test]
fn downscale_kernels_match_scalar() {
    let levels = vector_levels();
    let mut rng = Rng(0x0D0E_5CA1);
    let mut compared = 0usize;
    for &level in &levels {
        // Every row length from 0 to 40 pixels covers each vector tail.
        for len in 0..=40usize {
            let rows = rows_of(len, &mut rng);
            // Batches of 1 to MAX_BATCH rows; the largest weight a batch
            // can have, a small one and a random one.
            for count in 1..=MAX_BATCH {
                for pick in 0..rows.len() {
                    let batch: Vec<&[u8]> = (0..count)
                        .map(|i| &rows[(i * pick + pick) % rows.len()][..])
                        .collect();
                    let largest = 65_535 / count as u32;
                    for weight in [largest, 1, 1 + rng.next() % largest] {
                        for premultiply in [false, true] {
                            let start: Vec<u32> = (0..len * 4).map(|_| rng.next() % 1000).collect();
                            let mut want = start.clone();
                            let mut got = start;
                            let want_opaque =
                                scalar::accumulate(&batch, weight, premultiply, &mut want);
                            let got_opaque =
                                accumulate(level, &batch, weight, premultiply, &mut got);
                            let what = format!(
                                "{level:?} {len} pixels, {count} rows, pick {pick}, weight {weight}, premultiply {premultiply}"
                            );
                            assert_eq!(want, got, "{what}");
                            assert_eq!(want_opaque, got_opaque, "{what}: opacity");
                            compared += len * count;
                        }
                    }
                }
            }
        }
        // A translucent pixel in each position of a vector-sized row.
        for at in 0..16 {
            let mut row = vec![255u8; 16 * 4];
            row[at * 4 + 3] = 254;
            let mut acc = vec![0u32; 16 * 4];
            assert!(
                !accumulate(level, &[&row], 1, false, &mut acc),
                "{level:?} alpha at {at}"
            );
        }
        // Runs of every length and start over random sums, the largest too.
        let mut acc: Vec<u32> = (0..64 * 4).map(|_| rng.next()).collect();
        acc[..8].fill(u32::MAX);
        let runs: Vec<(usize, usize)> = (0..40)
            .flat_map(|count| [(0, count), (3, count), (64 - count, count)])
            .collect();
        let mut want = vec![[0u64; 4]; runs.len()];
        let mut got = vec![[1u64; 4]; runs.len()];
        scalar::sum_runs(&acc, &runs, &mut want);
        sum_runs(level, &acc, &runs, &mut got);
        assert_eq!(want, got, "{level:?} run sums");
    }
    println!("vector levels on this CPU: {levels:?}; {compared} pixel rows compared");
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        assert!(levels.contains(&Level::Avx2), "AVX2 path tested");
    }
    #[cfg(target_arch = "aarch64")]
    assert!(levels.contains(&Level::Neon), "NEON path tested");
}

#[test]
fn downscale_scalar_kernels_run_without_cpu_feature() {
    let calls = || scalar::CALLS.with(std::cell::Cell::get);
    // A multiple of every vector width: a vector path needs no scalar tail.
    let mut rng = Rng(99);
    let row: Vec<u8> = (0..64 * 4).map(|_| rng.byte()).collect();
    let batch = [&row[..], &row[..]];
    let mut reference = vec![0u32; 64 * 4];
    assert!(!scalar::accumulate(&batch, 77, true, &mut reference));
    // The scalar level runs the scalar code.
    let before = calls();
    let mut out = vec![0u32; 64 * 4];
    assert!(!accumulate(Level::Scalar, &batch, 77, true, &mut out));
    assert_eq!(calls(), before + 1);
    assert_eq!(out, reference);
    // A level whose feature this CPU lacks falls back to scalar.
    let missing = [Level::Avx2, Level::Neon]
        .into_iter()
        .find(|l| !l.supported())
        .expect("no CPU has both AVX2 and NEON");
    let before = calls();
    let mut out = vec![0u32; 64 * 4];
    assert!(!accumulate(missing, &batch, 77, true, &mut out));
    assert_eq!(calls(), before + 1, "{missing:?} is absent: scalar ran");
    assert_eq!(out, reference);
    // The detected vector level does not call the scalar code at all.
    if let Some(&level) = vector_levels().first() {
        let before = calls();
        let mut out = vec![0u32; 64 * 4];
        assert!(!accumulate(level, &batch, 77, true, &mut out));
        let mut runs = [[0u64; 4]; 1];
        sum_runs(level, &out, &[(0, 64)], &mut runs);
        assert_eq!(calls(), before, "{level:?} ran the vector code");
        assert_eq!(out, reference);
    }
    println!("detected level {:?}", Level::detected());
}

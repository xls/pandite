//! Vector kernels of the Rgba8 downscaler, chosen at run time.
//!
//! Every kernel has a scalar reference in [`scalar`] and vector versions
//! that produce exactly the same integer sums: x86_64 with AVX2 (detected
//! with `is_x86_feature_detected!`; without it the scalar kernels run),
//! aarch64 with NEON (part of the aarch64 baseline). [`Level::detected`]
//! picks the level once; setting the environment variable `PANDITE_SIMD`
//! to `scalar` forces the scalar kernels, for comparisons.
//!
//! This module and its children are the only place in the crate that uses
//! `unsafe`: intrinsics and unaligned loads and stores, each block with a
//! SAFETY comment, each target-feature function called only after the
//! feature was detected.

#![allow(unsafe_code)]

pub(crate) mod scalar;

#[cfg(target_arch = "x86_64")]
mod x86;

#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(test)]
mod tests;

use std::sync::OnceLock;

/// The instruction set the Rgba8 downscaler runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Level {
    /// Plain Rust integer code, any CPU.
    Scalar,
    /// x86_64 with AVX2.
    Avx2,
    /// aarch64 Advanced SIMD.
    Neon,
}

impl Level {
    /// Every level, scalar first.
    pub const ALL: [Level; 3] = [Level::Scalar, Level::Avx2, Level::Neon];

    /// The level [`super::box_downscale`] uses in this process: the best
    /// one this CPU has, or scalar when the environment variable
    /// `PANDITE_SIMD` is `scalar`. Detected once.
    pub fn detected() -> Level {
        static LEVEL: OnceLock<Level> = OnceLock::new();
        *LEVEL.get_or_init(|| {
            let forced =
                std::env::var("PANDITE_SIMD").is_ok_and(|v| v.eq_ignore_ascii_case("scalar"));
            if forced {
                Level::Scalar
            } else {
                Level::ALL
                    .into_iter()
                    .rev()
                    .find(|l| l.supported())
                    .unwrap_or(Level::Scalar)
            }
        })
    }

    /// True when this CPU can run the level.
    pub fn supported(self) -> bool {
        match self {
            Level::Scalar => true,
            #[cfg(target_arch = "x86_64")]
            Level::Avx2 => std::arch::is_x86_feature_detected!("avx2"),
            #[cfg(target_arch = "aarch64")]
            Level::Neon => std::arch::is_aarch64_feature_detected!("neon"),
            _ => false,
        }
    }

    /// Every level this CPU can run, scalar first.
    pub fn available() -> Vec<Level> {
        Level::ALL.into_iter().filter(|l| l.supported()).collect()
    }

    /// The lowercase name: `scalar`, `avx2` or `neon`.
    pub fn name(self) -> &'static str {
        match self {
            Level::Scalar => "scalar",
            Level::Avx2 => "avx2",
            Level::Neon => "neon",
        }
    }
}

/// Rows that one [`accumulate`] call adds at most. The vector kernels sum
/// plain bytes of up to 257 rows in 16 bits.
pub(super) const MAX_BATCH: usize = 8;

/// Add RGBA8 rows times one `weight` to the column sums `acc` (4 per
/// pixel) and tell whether every alpha byte added was 255: see
/// [`scalar::accumulate`]. A level the CPU lacks runs the scalar kernel.
pub(super) fn accumulate(
    level: Level,
    rows: &[&[u8]],
    weight: u32,
    premultiply: bool,
    acc: &mut [u32],
) -> bool {
    match level {
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 if Level::Avx2.supported() => {
            // SAFETY: AVX2 was detected on this CPU just above.
            unsafe {
                if premultiply {
                    x86::accumulate::<true>(rows, weight, acc)
                } else {
                    x86::accumulate::<false>(rows, weight, acc)
                }
            }
        }
        #[cfg(target_arch = "aarch64")]
        Level::Neon if Level::Neon.supported() => {
            // SAFETY: NEON was detected on this CPU just above.
            unsafe {
                if premultiply {
                    neon::accumulate::<true>(rows, weight, acc)
                } else {
                    neon::accumulate::<false>(rows, weight, acc)
                }
            }
        }
        _ => scalar::accumulate(rows, weight, premultiply, acc),
    }
}

/// The channel totals of runs of column sums: see [`scalar::sum_runs`].
/// A level the CPU lacks runs the scalar kernel.
pub(super) fn sum_runs(level: Level, acc: &[u32], runs: &[(usize, usize)], out: &mut [[u64; 4]]) {
    match level {
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 if Level::Avx2.supported() => {
            // SAFETY: AVX2 was detected on this CPU just above.
            unsafe { x86::sum_runs(acc, runs, out) }
        }
        #[cfg(target_arch = "aarch64")]
        Level::Neon if Level::Neon.supported() => {
            // SAFETY: NEON was detected on this CPU just above.
            unsafe { neon::sum_runs(acc, runs, out) }
        }
        _ => scalar::sum_runs(acc, runs, out),
    }
}

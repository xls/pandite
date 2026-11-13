//! Vector kernels for the decoders' hot loops, chosen at run time.
//!
//! Every kernel has a scalar reference implementation in [`scalar`] and
//! vector versions that must produce exactly the same output: x86_64 with
//! AVX2 (SSE2 is the x86_64 baseline), aarch64 with NEON. [`level`]
//! detects the CPU once (`is_x86_feature_detected!`; NEON is part of the
//! aarch64 baseline); setting the environment variable `PANDITE_SIMD` to
//! `scalar` forces the scalar path, for comparisons.
//!
//! This module and its children are the only place in the crate's
//! decoders that uses `unsafe` (the operating system codec modules aside):
//! intrinsics and unaligned loads and stores, each block with a SAFETY
//! comment, each target-feature function called only after the feature
//! was detected.

#![allow(unsafe_code)]

pub(crate) mod scalar;

#[cfg(target_arch = "x86_64")]
mod x86;

#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(test)]
mod tests;

use crate::raw::tone::Tone;
use std::sync::OnceLock;

/// The instruction set a kernel runs with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    /// Plain Rust, any CPU.
    Scalar,
    /// x86_64 with AVX2 and FMA-free 256-bit integer and float vectors.
    Avx2,
    /// aarch64 Advanced SIMD.
    Neon,
}

/// CPU features that decide the level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Features {
    /// x86_64 AVX2.
    pub avx2: bool,
    /// aarch64 NEON.
    pub neon: bool,
}

/// The level for a feature set: the best one the CPU has, scalar when it
/// has none of the vector extensions.
pub(crate) fn select(f: Features) -> Level {
    if f.avx2 {
        Level::Avx2
    } else if f.neon {
        Level::Neon
    } else {
        Level::Scalar
    }
}

/// The features of this CPU.
pub(crate) fn detect() -> Features {
    #[cfg(target_arch = "x86_64")]
    {
        Features {
            avx2: std::arch::is_x86_feature_detected!("avx2"),
            neon: false,
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        Features {
            avx2: false,
            neon: std::arch::is_aarch64_feature_detected!("neon"),
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        Features::default()
    }
}

/// The level every kernel uses in this process (detected once).
pub(crate) fn level() -> Level {
    static LEVEL: OnceLock<Level> = OnceLock::new();
    *LEVEL.get_or_init(|| {
        let forced = std::env::var("PANDITE_SIMD").is_ok_and(|v| v.eq_ignore_ascii_case("scalar"));
        if forced {
            Level::Scalar
        } else {
            select(detect())
        }
    })
}

/// True when `level` can run on this CPU.
pub(crate) fn supported(level: Level) -> bool {
    let f = detect();
    match level {
        Level::Scalar => true,
        Level::Avx2 => f.avx2,
        Level::Neon => f.neon,
    }
}

/// Camera RGB (three planes) through color matrix `m` (white balance
/// already applied), clipped to 0..=1, then the display tone curve and the
/// sRGB transfer to RGBA8 with alpha 255: `out` holds 4 bytes per pixel.
pub(crate) fn matrix_tone_rgba8(m: &[[f32; 3]; 3], tone: &Tone, cam: [&[f32]; 3], out: &mut [u8]) {
    matrix_tone_rgba8_with(level(), m, tone, cam, out);
}

/// [`matrix_tone_rgba8`] at a given level. A level the CPU lacks falls
/// back to the scalar path.
pub(crate) fn matrix_tone_rgba8_with(
    level: Level,
    m: &[[f32; 3]; 3],
    tone: &Tone,
    cam: [&[f32]; 3],
    out: &mut [u8],
) {
    match level {
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 if supported(Level::Avx2) => {
            // SAFETY: AVX2 was detected on this CPU just above.
            unsafe { x86::matrix_tone_rgba8(m, tone, cam, out) }
        }
        #[cfg(target_arch = "aarch64")]
        Level::Neon if supported(Level::Neon) => {
            // SAFETY: NEON was detected on this CPU just above.
            unsafe { neon::matrix_tone_rgba8(m, tone, cam, out) }
        }
        _ => scalar::matrix_tone_rgba8(m, tone, cam, out),
    }
}

/// Bilinear demosaic of the interior of one Bayer row: see
/// [`scalar::bayer_row`] for the arguments.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bayer_row(
    a: &[f32],
    b: &[f32],
    c: &[f32],
    i0: usize,
    n: usize,
    green_first: bool,
    q: usize,
    out: scalar::Planes<'_>,
) {
    bayer_row_with(level(), a, b, c, i0, n, green_first, q, out);
}

/// [`bayer_row`] at a given level. A level the CPU lacks falls back to
/// the scalar path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bayer_row_with(
    level: Level,
    a: &[f32],
    b: &[f32],
    c: &[f32],
    i0: usize,
    n: usize,
    green_first: bool,
    q: usize,
    out: scalar::Planes<'_>,
) {
    match level {
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 if supported(Level::Avx2) => {
            // SAFETY: AVX2 was detected on this CPU just above.
            unsafe { x86::bayer_row(a, b, c, i0, n, green_first, q, out) }
        }
        #[cfg(target_arch = "aarch64")]
        Level::Neon if supported(Level::Neon) => {
            // SAFETY: NEON was detected on this CPU just above.
            unsafe { neon::bayer_row(a, b, c, i0, n, green_first, q, out) }
        }
        _ => scalar::bayer_row(a, b, c, i0, n, green_first, q, out),
    }
}

/// Normalize a run of raw samples (R-ARW-47): see
/// [`scalar::normalize_row`].
pub(crate) fn normalize_row(
    src: &[u16],
    odd_first: bool,
    black: [f32; 2],
    scale: [f32; 2],
    out: &mut [f32],
) {
    normalize_row_with(level(), src, odd_first, black, scale, out);
}

/// [`normalize_row`] at a given level. A level the CPU lacks falls back
/// to the scalar path.
pub(crate) fn normalize_row_with(
    level: Level,
    src: &[u16],
    odd_first: bool,
    black: [f32; 2],
    scale: [f32; 2],
    out: &mut [f32],
) {
    match level {
        #[cfg(target_arch = "x86_64")]
        Level::Avx2 if supported(Level::Avx2) => {
            // SAFETY: AVX2 was detected on this CPU just above.
            unsafe { x86::normalize_row(src, odd_first, black, scale, out) }
        }
        #[cfg(target_arch = "aarch64")]
        Level::Neon if supported(Level::Neon) => {
            // SAFETY: NEON was detected on this CPU just above.
            unsafe { neon::normalize_row(src, odd_first, black, scale, out) }
        }
        _ => scalar::normalize_row(src, odd_first, black, scale, out),
    }
}

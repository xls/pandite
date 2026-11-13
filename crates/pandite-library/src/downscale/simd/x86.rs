//! AVX2 kernels for x86_64. Every function here requires AVX2; callers
//! check [`super::Level::supported`] first. Results equal the scalar
//! reference in [`super::scalar`] exactly: the same integer products and
//! sums (wrapping never happens within the callers' bounds), eight pixels
//! per step. Row ends that do not fill a vector go to the scalar
//! reference.
//!
//! [`super::neon`] is the line-by-line twin at 128-bit width. Two steps
//! have no counterpart there: the lane fix-up (AVX2 unpacks work within
//! 128-bit lanes, so the sums come out as pixels 0 | 4, 1 | 5, 2 | 6,
//! 3 | 7 and a cross-lane permute puts them in order) and the prefetch
//! hint, which does not change any result.

use super::{scalar, MAX_BATCH};
use std::arch::x86_64::*;

/// Pixels per vector step.
const LANES: usize = 8;

/// Bytes ahead of each load that the kernel asks the CPU to fetch. The
/// rows of a batch are separate streams; without the hint the fused pass
/// reads memory at about half the speed the hardware can deliver.
const PREFETCH: usize = 1024;

/// AVX2 version of [`scalar::accumulate`], with `premultiply` fixed at
/// compile time.
///
/// Plain bytes are summed over the rows in 16 bits (at most `255 * 8`)
/// and then multiplied by the weight into 32 bits; premultiplied products
/// are summed in 32 bits and then multiplied by the weight. Every loaded
/// vector is also ANDed into `all` for the opacity result.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn accumulate<const PREMULTIPLY: bool>(
    rows: &[&[u8]],
    weight: u32,
    acc: &mut [u32],
) -> bool {
    let n = acc.len() / 4;
    // The weight is a 16-bit multiplier, and every row must hold n pixels.
    if rows.len() > MAX_BATCH || weight > 0xFFFF || rows.iter().any(|r| r.len() < n * 4) {
        return scalar::accumulate(rows, weight, PREMULTIPLY, acc);
    }
    let done = n / LANES * LANES;
    let zero = _mm256_setzero_si256();
    let mut all = _mm256_set1_epi8(-1);
    let w16 = _mm256_set1_epi16(weight as u16 as i16);
    let w32 = _mm256_set1_epi32(weight as i32);
    // Each pixel's alpha byte to its red, green and blue bytes, zero (an
    // index with bit 7 set) to its alpha byte; then 1 into the alpha byte.
    let spread = _mm256_broadcastsi128_si256(_mm_setr_epi8(
        3, 3, 3, -1, 7, 7, 7, -1, 11, 11, 11, -1, 15, 15, 15, -1,
    ));
    let one = _mm256_set1_epi32(0x0100_0000);
    // Full 32-bit products of 16-bit samples and the weight.
    let widen_mul = |x: __m256i| {
        let lo = _mm256_mullo_epi16(x, w16);
        let hi = _mm256_mulhi_epu16(x, w16);
        (_mm256_unpacklo_epi16(lo, hi), _mm256_unpackhi_epi16(lo, hi))
    };
    let dst = acc.as_mut_ptr();
    for k in (0..done).step_by(LANES) {
        // 32-bit sums of pixels 0 | 4, 1 | 5, 2 | 6, 3 | 7.
        let [x0, x1, y0, y1] = if PREMULTIPLY {
            let mut s = [zero; 4];
            for row in rows {
                // SAFETY: k + 8 <= n and the row holds n pixels: bytes
                // 4k .. 4k + 32 are inside it. A prefetch never faults, at
                // any address.
                let v = unsafe {
                    _mm_prefetch::<_MM_HINT_T0>(
                        row.as_ptr().wrapping_add(k * 4 + PREFETCH).cast::<i8>(),
                    );
                    _mm256_loadu_si256(row.as_ptr().add(k * 4).cast::<__m256i>())
                };
                all = _mm256_and_si256(all, v);
                // Multipliers A, A, A, 1: a product is at most 255 * 255.
                let m = _mm256_or_si256(_mm256_shuffle_epi8(v, spread), one);
                // 16-bit products of pixels 0, 1 | 4, 5 and 2, 3 | 6, 7.
                let a = _mm256_mullo_epi16(
                    _mm256_unpacklo_epi8(v, zero),
                    _mm256_unpacklo_epi8(m, zero),
                );
                let b = _mm256_mullo_epi16(
                    _mm256_unpackhi_epi8(v, zero),
                    _mm256_unpackhi_epi8(m, zero),
                );
                s[0] = _mm256_add_epi32(s[0], _mm256_unpacklo_epi16(a, zero));
                s[1] = _mm256_add_epi32(s[1], _mm256_unpackhi_epi16(a, zero));
                s[2] = _mm256_add_epi32(s[2], _mm256_unpacklo_epi16(b, zero));
                s[3] = _mm256_add_epi32(s[3], _mm256_unpackhi_epi16(b, zero));
            }
            [
                _mm256_mullo_epi32(s[0], w32),
                _mm256_mullo_epi32(s[1], w32),
                _mm256_mullo_epi32(s[2], w32),
                _mm256_mullo_epi32(s[3], w32),
            ]
        } else {
            let (mut a, mut b) = (zero, zero);
            for row in rows {
                // SAFETY: k + 8 <= n and the row holds n pixels: bytes
                // 4k .. 4k + 32 are inside it. A prefetch never faults, at
                // any address.
                let v = unsafe {
                    _mm_prefetch::<_MM_HINT_T0>(
                        row.as_ptr().wrapping_add(k * 4 + PREFETCH).cast::<i8>(),
                    );
                    _mm256_loadu_si256(row.as_ptr().add(k * 4).cast::<__m256i>())
                };
                all = _mm256_and_si256(all, v);
                // 16-bit sums of pixels 0, 1 | 4, 5 and 2, 3 | 6, 7.
                a = _mm256_add_epi16(a, _mm256_unpacklo_epi8(v, zero));
                b = _mm256_add_epi16(b, _mm256_unpackhi_epi8(v, zero));
            }
            let (x0, x1) = widen_mul(a);
            let (y0, y1) = widen_mul(b);
            [x0, x1, y0, y1]
        };
        // Lane fix-up: pixels 0, 1 | 2, 3 | 4, 5 | 6, 7.
        let s0 = _mm256_permute2x128_si256::<0x20>(x0, x1);
        let s1 = _mm256_permute2x128_si256::<0x20>(y0, y1);
        let s2 = _mm256_permute2x128_si256::<0x31>(x0, x1);
        let s3 = _mm256_permute2x128_si256::<0x31>(y0, y1);
        // SAFETY: k + 8 <= n: sums 4k .. 4k + 32 are inside `acc`.
        unsafe {
            let p = dst.add(k * 4).cast::<__m256i>();
            _mm256_storeu_si256(p, _mm256_add_epi32(_mm256_loadu_si256(p), s0));
            let p = p.add(1);
            _mm256_storeu_si256(p, _mm256_add_epi32(_mm256_loadu_si256(p), s1));
            let p = p.add(1);
            _mm256_storeu_si256(p, _mm256_add_epi32(_mm256_loadu_si256(p), s2));
            let p = p.add(1);
            _mm256_storeu_si256(p, _mm256_add_epi32(_mm256_loadu_si256(p), s3));
        }
    }
    // Every alpha bit set: (!all & alpha) == 0.
    let alpha = _mm256_set1_epi32(0xFF00_0000u32 as i32);
    let opaque = _mm256_testc_si256(all, alpha) == 1;
    if done < n {
        let mut tails: [&[u8]; MAX_BATCH] = [&[]; MAX_BATCH];
        for (t, row) in tails.iter_mut().zip(rows) {
            *t = &row[done * 4..];
        }
        let tails = &tails[..rows.len()];
        return scalar::accumulate(tails, weight, PREMULTIPLY, &mut acc[done * 4..]) && opaque;
    }
    opaque
}

/// AVX2 version of [`scalar::sum_runs`]: two pixels per step, one 64-bit
/// total per channel and pixel.
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
pub(super) unsafe fn sum_runs(acc: &[u32], runs: &[(usize, usize)], out: &mut [[u64; 4]]) {
    for (&(first, count), total) in runs.iter().zip(out.iter_mut()) {
        let run = &acc[first * 4..(first + count) * 4];
        let p = run.as_ptr();
        let (mut s0, mut s1) = (_mm256_setzero_si256(), _mm256_setzero_si256());
        let pairs = count / 2 * 2;
        for i in (0..pairs).step_by(2) {
            // SAFETY: i + 2 <= count: sums 4i .. 4i + 8 are inside `run`.
            let (u, v) = unsafe {
                (
                    _mm_loadu_si128(p.add(i * 4).cast::<__m128i>()),
                    _mm_loadu_si128(p.add(i * 4 + 4).cast::<__m128i>()),
                )
            };
            s0 = _mm256_add_epi64(s0, _mm256_cvtepu32_epi64(u));
            s1 = _mm256_add_epi64(s1, _mm256_cvtepu32_epi64(v));
        }
        if pairs < count {
            // SAFETY: pairs + 1 == count: sums 4 pairs .. 4 count are inside `run`.
            let u = unsafe { _mm_loadu_si128(p.add(pairs * 4).cast::<__m128i>()) };
            s0 = _mm256_add_epi64(s0, _mm256_cvtepu32_epi64(u));
        }
        // SAFETY: `total` holds four u64, 32 bytes.
        unsafe {
            _mm256_storeu_si256(
                total.as_mut_ptr().cast::<__m256i>(),
                _mm256_add_epi64(s0, s1),
            );
        }
    }
}

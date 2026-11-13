//! NEON kernels for aarch64. Every function here requires NEON; callers
//! check [`super::Level::supported`] first. Results equal the scalar
//! reference in [`super::scalar`] exactly: the same integer products and
//! sums (wrapping never happens within the callers' bounds), four pixels
//! per step. Row ends that do not fill a vector go to the scalar
//! reference.
//!
//! This is the line-by-line twin of [`super::x86`] at 128-bit width. NEON
//! widens whole vectors, so the sums come out in pixel order and the AVX2
//! lane fix-up has no counterpart here; `vmull` forms the 32-bit products
//! that AVX2 builds from `mullo`, `mulhi` and an unpack. The AVX2 prefetch
//! hint is left out: stable Rust has no aarch64 prefetch intrinsic, and the
//! hint changes no result. Constants are loaded from byte arrays, so lane
//! `i` is byte `i` in memory as in the pixel loads.

use super::{scalar, MAX_BATCH};
use std::arch::aarch64::*;

/// Pixels per vector step.
const LANES: usize = 4;

/// NEON version of [`scalar::accumulate`], with `premultiply` fixed at
/// compile time.
///
/// Plain bytes are summed over the rows in 16 bits (at most `255 * 8`)
/// and then multiplied by the weight into 32 bits; premultiplied products
/// are summed in 32 bits and then multiplied by the weight. Every loaded
/// vector is also ANDed into `all` for the opacity result.
///
/// # Safety
///
/// The CPU must support NEON.
#[target_feature(enable = "neon")]
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
    let zero = vdupq_n_u32(0);
    let mut all = vdupq_n_u8(0xFF);
    let w16 = vdupq_n_u16(weight as u16);
    let w32 = vdupq_n_u32(weight);
    // Each pixel's alpha byte to its red, green and blue bytes, zero (an
    // index of 16 or more) to its alpha byte; then 1 into the alpha byte.
    let spread_bytes: [u8; 16] = [
        3, 3, 3, 0xFF, 7, 7, 7, 0xFF, 11, 11, 11, 0xFF, 15, 15, 15, 0xFF,
    ];
    let one_bytes: [u8; 16] = [0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1];
    // SAFETY: both arrays hold 16 bytes.
    let (spread, one) = unsafe {
        (
            vld1q_u8(spread_bytes.as_ptr()),
            vld1q_u8(one_bytes.as_ptr()),
        )
    };
    // Full 32-bit products of 16-bit samples and the weight.
    let widen_mul = |x: uint16x8_t| {
        (
            vmull_u16(vget_low_u16(x), vget_low_u16(w16)),
            vmull_high_u16(x, w16),
        )
    };
    let dst = acc.as_mut_ptr();
    for k in (0..done).step_by(LANES) {
        // 32-bit sums of pixels 0, 1, 2, 3.
        let [x0, x1, y0, y1] = if PREMULTIPLY {
            let mut s = [zero; 4];
            for row in rows {
                // SAFETY: k + 4 <= n and the row holds n pixels: bytes
                // 4k .. 4k + 16 are inside it.
                let v = unsafe { vld1q_u8(row.as_ptr().add(k * 4)) };
                all = vandq_u8(all, v);
                // Multipliers A, A, A, 1: a product is at most 255 * 255.
                let m = vorrq_u8(vqtbl1q_u8(v, spread), one);
                // 16-bit products of pixels 0, 1 and 2, 3.
                let a = vmulq_u16(vmovl_u8(vget_low_u8(v)), vmovl_u8(vget_low_u8(m)));
                let b = vmulq_u16(vmovl_high_u8(v), vmovl_high_u8(m));
                s[0] = vaddq_u32(s[0], vmovl_u16(vget_low_u16(a)));
                s[1] = vaddq_u32(s[1], vmovl_high_u16(a));
                s[2] = vaddq_u32(s[2], vmovl_u16(vget_low_u16(b)));
                s[3] = vaddq_u32(s[3], vmovl_high_u16(b));
            }
            [
                vmulq_u32(s[0], w32),
                vmulq_u32(s[1], w32),
                vmulq_u32(s[2], w32),
                vmulq_u32(s[3], w32),
            ]
        } else {
            let (mut a, mut b) = (vdupq_n_u16(0), vdupq_n_u16(0));
            for row in rows {
                // SAFETY: k + 4 <= n and the row holds n pixels: bytes
                // 4k .. 4k + 16 are inside it.
                let v = unsafe { vld1q_u8(row.as_ptr().add(k * 4)) };
                all = vandq_u8(all, v);
                // 16-bit sums of pixels 0, 1 and 2, 3.
                a = vaddq_u16(a, vmovl_u8(vget_low_u8(v)));
                b = vaddq_u16(b, vmovl_high_u8(v));
            }
            let (x0, x1) = widen_mul(a);
            let (y0, y1) = widen_mul(b);
            [x0, x1, y0, y1]
        };
        // Already in pixel order: no lane fix-up.
        // SAFETY: k + 4 <= n: sums 4k .. 4k + 16 are inside `acc`.
        unsafe {
            let p = dst.add(k * 4);
            vst1q_u32(p, vaddq_u32(vld1q_u32(p), x0));
            let p = p.add(4);
            vst1q_u32(p, vaddq_u32(vld1q_u32(p), x1));
            let p = p.add(4);
            vst1q_u32(p, vaddq_u32(vld1q_u32(p), y0));
            let p = p.add(4);
            vst1q_u32(p, vaddq_u32(vld1q_u32(p), y1));
        }
    }
    // Every alpha byte 255: with the color bytes set too, the smallest
    // byte is 255.
    let color_bytes: [u8; 16] = [
        0xFF, 0xFF, 0xFF, 0, 0xFF, 0xFF, 0xFF, 0, 0xFF, 0xFF, 0xFF, 0, 0xFF, 0xFF, 0xFF, 0,
    ];
    // SAFETY: the array holds 16 bytes.
    let color = unsafe { vld1q_u8(color_bytes.as_ptr()) };
    let opaque = vminvq_u8(vorrq_u8(all, color)) == 0xFF;
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

/// NEON version of [`scalar::sum_runs`]: two pixels per step, one 64-bit
/// total per channel and pixel (each in a low and a high half).
///
/// # Safety
///
/// The CPU must support NEON.
#[target_feature(enable = "neon")]
pub(super) unsafe fn sum_runs(acc: &[u32], runs: &[(usize, usize)], out: &mut [[u64; 4]]) {
    for (&(first, count), total) in runs.iter().zip(out.iter_mut()) {
        let run = &acc[first * 4..(first + count) * 4];
        let p = run.as_ptr();
        let (mut s0, mut s1) = ([vdupq_n_u64(0); 2], [vdupq_n_u64(0); 2]);
        let pairs = count / 2 * 2;
        for i in (0..pairs).step_by(2) {
            // SAFETY: i + 2 <= count: sums 4i .. 4i + 8 are inside `run`.
            let (u, v) = unsafe { (vld1q_u32(p.add(i * 4)), vld1q_u32(p.add(i * 4 + 4))) };
            s0 = [vaddw_u32(s0[0], vget_low_u32(u)), vaddw_high_u32(s0[1], u)];
            s1 = [vaddw_u32(s1[0], vget_low_u32(v)), vaddw_high_u32(s1[1], v)];
        }
        if pairs < count {
            // SAFETY: pairs + 1 == count: sums 4 pairs .. 4 count are inside `run`.
            let u = unsafe { vld1q_u32(p.add(pairs * 4)) };
            s0 = [vaddw_u32(s0[0], vget_low_u32(u)), vaddw_high_u32(s0[1], u)];
        }
        // SAFETY: `total` holds four u64: two at index 0 and two at index 2.
        unsafe {
            vst1q_u64(total.as_mut_ptr(), vaddq_u64(s0[0], s1[0]));
            vst1q_u64(total.as_mut_ptr().add(2), vaddq_u64(s0[1], s1[1]));
        }
    }
}

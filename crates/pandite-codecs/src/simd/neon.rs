//! NEON kernels for aarch64. Every function here requires NEON; callers
//! check [`super::supported`] first. Results equal the scalar reference
//! in [`super::scalar`] bit for bit: the same operations in the same
//! order, no fused multiply-add, truncating conversions, and the
//! `maxnm`/`minnm` forms whose NaN handling matches `f32::max` and
//! `f32::min`. NEON has no gather, so table lookups read lanes one by
//! one. Row ends that do not fill a vector go to the scalar reference.

use super::scalar;
use crate::raw::tone::{srgb8_table, Tone, LUMA, SCALE_STEPS, SRGB8_STEPS};
use std::arch::aarch64::*;

/// Pixels per vector.
const LANES: usize = 4;

/// NEON version of [`scalar::matrix_tone_rgba8`].
///
/// # Safety
///
/// The CPU must support NEON.
#[target_feature(enable = "neon")]
pub(super) unsafe fn matrix_tone_rgba8(
    m: &[[f32; 3]; 3],
    tone: &Tone,
    cam: [&[f32]; 3],
    out: &mut [u8],
) {
    let n = cam[0]
        .len()
        .min(cam[1].len())
        .min(cam[2].len())
        .min(out.len() / 4);
    let scale = tone.table();
    let srgb = srgb8_table();
    let done = if scale.len() > SCALE_STEPS && srgb.len() >= SRGB8_STEPS {
        n / LANES * LANES
    } else {
        0
    };
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    let half = vdupq_n_f32(0.5);
    let steps = vdupq_n_f32((SCALE_STEPS - 1) as f32);
    let codes = vdupq_n_f32((SRGB8_STEPS - 1) as f32);
    let mv = m.map(|r| r.map(|v| vdupq_n_f32(v)));
    let luma = LUMA.map(|v| vdupq_n_f32(v));
    let clip = |v: float32x4_t| vminnmq_f32(vmaxnmq_f32(v, zero), one);
    for k in (0..done).step_by(LANES) {
        // SAFETY: k + 4 <= n, which is within every plane.
        let (r, g, b) = unsafe {
            (
                vld1q_f32(cam[0].as_ptr().add(k)),
                vld1q_f32(cam[1].as_ptr().add(k)),
                vld1q_f32(cam[2].as_ptr().add(k)),
            )
        };
        let row = |c: usize| {
            clip(vaddq_f32(
                vaddq_f32(vmulq_f32(mv[c][0], r), vmulq_f32(mv[c][1], g)),
                vmulq_f32(mv[c][2], b),
            ))
        };
        let (lr, lg, lb) = (row(0), row(1), row(2));
        let l = vaddq_f32(
            vaddq_f32(vmulq_f32(luma[0], lr), vmulq_f32(luma[1], lg)),
            vmulq_f32(luma[2], lb),
        );
        let pos = vmulq_f32(clip(l), steps);
        let i = vcvtq_s32_f32(pos);
        let t = vsubq_f32(pos, vcvtq_f32_s32(i));
        let mut idx = [0i32; 4];
        // SAFETY: `idx` has room for 4 lanes.
        unsafe { vst1q_s32(idx.as_mut_ptr(), i) };
        // 0 <= pos <= SCALE_STEPS - 1: idx and idx + 1 are inside `scale`.
        let av = idx.map(|j| scale[j as usize]);
        let bv = idx.map(|j| scale[j as usize + 1]);
        // SAFETY: both arrays hold 4 floats.
        let (a, bb) = unsafe { (vld1q_f32(av.as_ptr()), vld1q_f32(bv.as_ptr())) };
        let s = vaddq_f32(a, vmulq_f32(vsubq_f32(bb, a), t));
        let mut xr = vmulq_f32(lr, s);
        let mut xg = vmulq_f32(lg, s);
        let mut xb = vmulq_f32(lb, s);
        let mx = vmaxnmq_f32(vmaxnmq_f32(xr, xg), xb);
        let over = vcgtq_f32(mx, one);
        if vmaxvq_u32(over) != 0 {
            let y = vminnmq_f32(vmulq_f32(l, s), one);
            let f = vdivq_f32(vsubq_f32(one, y), vsubq_f32(mx, y));
            let pull = |c: float32x4_t| vaddq_f32(y, vmulq_f32(vsubq_f32(c, y), f));
            xr = vbslq_f32(over, pull(xr), xr);
            xg = vbslq_f32(over, pull(xg), xg);
            xb = vbslq_f32(over, pull(xb), xb);
        }
        let index = |c: float32x4_t| vcvtq_s32_f32(vaddq_f32(vmulq_f32(clip(c), codes), half));
        let mut ir = [0i32; 4];
        let mut ig = [0i32; 4];
        let mut ib = [0i32; 4];
        // SAFETY: each array has room for 4 lanes.
        unsafe {
            vst1q_s32(ir.as_mut_ptr(), index(xr));
            vst1q_s32(ig.as_mut_ptr(), index(xg));
            vst1q_s32(ib.as_mut_ptr(), index(xb));
        }
        let dst = &mut out[k * 4..k * 4 + 16];
        for lane in 0..LANES {
            let d = &mut dst[lane * 4..lane * 4 + 4];
            d[0] = srgb[ir[lane] as usize];
            d[1] = srgb[ig[lane] as usize];
            d[2] = srgb[ib[lane] as usize];
            d[3] = 255;
        }
    }
    if done < n {
        let tail = [&cam[0][done..n], &cam[1][done..n], &cam[2][done..n]];
        scalar::matrix_tone_rgba8(m, tone, tail, &mut out[done * 4..n * 4]);
    }
}

/// NEON version of [`scalar::bayer_row`].
///
/// # Safety
///
/// The CPU must support NEON.
#[target_feature(enable = "neon")]
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn bayer_row(
    a: &[f32],
    b: &[f32],
    c: &[f32],
    i0: usize,
    n: usize,
    green_first: bool,
    q: usize,
    out: scalar::Planes<'_>,
) {
    let len = a.len().min(b.len()).min(c.len());
    let n = n.min(out[0].len()).min(out[1].len()).min(out[2].len());
    let ok = i0 >= 1 && i0 + n < len && (q == 0 || q == 2);
    let done = if ok { n / LANES * LANES } else { 0 };
    let o = 2 - q.min(2);
    let halfv = vdupq_n_f32(0.5);
    let quarter = vdupq_n_f32(0.25);
    let pattern: [u32; 4] = if green_first {
        [u32::MAX, 0, u32::MAX, 0]
    } else {
        [0, u32::MAX, 0, u32::MAX]
    };
    // SAFETY: `pattern` holds 4 lanes.
    let green = unsafe { vld1q_u32(pattern.as_ptr()) };
    let [p0, p1, p2] = out;
    let mut planes = [p0, p1, p2];
    for k in (0..done).step_by(LANES) {
        let i = i0 + k;
        // SAFETY: i - 1 >= 0 and i + 5 <= i0 + n + 1 <= len, so every
        // 4-float load stays inside a, b and c.
        let (am, ac, ap, bm, bc, bp, cm, cc, cp) = unsafe {
            (
                vld1q_f32(a.as_ptr().add(i - 1)),
                vld1q_f32(a.as_ptr().add(i)),
                vld1q_f32(a.as_ptr().add(i + 1)),
                vld1q_f32(b.as_ptr().add(i - 1)),
                vld1q_f32(b.as_ptr().add(i)),
                vld1q_f32(b.as_ptr().add(i + 1)),
                vld1q_f32(c.as_ptr().add(i - 1)),
                vld1q_f32(c.as_ptr().add(i)),
                vld1q_f32(c.as_ptr().add(i + 1)),
            )
        };
        let h = vaddq_f32(bm, bp);
        let horiz = vmulq_f32(h, halfv);
        let vert = vmulq_f32(vaddq_f32(ac, cc), halfv);
        let cross = vmulq_f32(vaddq_f32(vaddq_f32(h, ac), cc), quarter);
        let diag = vmulq_f32(vaddq_f32(vaddq_f32(vaddq_f32(am, ap), cm), cp), quarter);
        let vq = vbslq_f32(green, horiz, bc);
        let vg = vbslq_f32(green, bc, cross);
        let vo = vbslq_f32(green, vert, diag);
        // SAFETY: k + 4 <= n <= every plane's length.
        unsafe {
            vst1q_f32(planes[q].as_mut_ptr().add(k), vq);
            vst1q_f32(planes[1].as_mut_ptr().add(k), vg);
            vst1q_f32(planes[o].as_mut_ptr().add(k), vo);
        }
    }
    if done < n {
        let [p0, p1, p2] = &mut planes;
        let tail = [&mut p0[done..n], &mut p1[done..n], &mut p2[done..n]];
        // `done` is a multiple of 4: the tail starts on the same parity.
        scalar::bayer_row(a, b, c, i0 + done, n - done, green_first, q, tail);
    }
}

/// NEON version of [`scalar::normalize_row`].
///
/// # Safety
///
/// The CPU must support NEON.
#[target_feature(enable = "neon")]
pub(super) unsafe fn normalize_row(
    src: &[u16],
    odd_first: bool,
    black: [f32; 2],
    scale: [f32; 2],
    out: &mut [f32],
) {
    let n = src.len().min(out.len());
    let done = n / LANES * LANES;
    let (b0, b1, k0, k1) = if odd_first {
        (black[1], black[0], scale[1], scale[0])
    } else {
        (black[0], black[1], scale[0], scale[1])
    };
    let bs = [b0, b1, b0, b1];
    let ks = [k0, k1, k0, k1];
    // SAFETY: both arrays hold 4 floats.
    let (bv, kv) = unsafe { (vld1q_f32(bs.as_ptr()), vld1q_f32(ks.as_ptr())) };
    let zero = vdupq_n_f32(0.0);
    let one = vdupq_n_f32(1.0);
    for k in (0..done).step_by(LANES) {
        // SAFETY: k + 4 <= n: 4 samples of `src` are in bounds.
        let raw = unsafe { vld1_u16(src.as_ptr().add(k)) };
        let v = vcvtq_f32_u32(vmovl_u16(raw));
        let r = vminnmq_f32(vmaxnmq_f32(vmulq_f32(vsubq_f32(v, bv), kv), zero), one);
        // SAFETY: k + 4 <= n: 4 floats of `out` are in bounds.
        unsafe { vst1q_f32(out.as_mut_ptr().add(k), r) };
    }
    if done < n {
        scalar::normalize_row(&src[done..n], odd_first, black, scale, &mut out[done..n]);
    }
}

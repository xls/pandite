//! AVX2 kernels for x86_64. Every function here requires AVX2; callers
//! check [`super::supported`] first. Results equal the scalar reference
//! in [`super::scalar`] bit for bit: the same operations in the same
//! order, no fused multiply-add, truncating float to integer conversion,
//! and lower-bound-first clipping (`max` then `min`, which turns NaN into
//! the bound the way the scalar `f32::max` does). Row ends that do not
//! fill a vector go to the scalar reference.

use super::scalar;
use crate::raw::tone::{srgb8_table, Tone, LUMA, SCALE_STEPS, SRGB8_STEPS, TABLE_PAD};
use std::arch::x86_64::*;

/// Pixels per vector.
const LANES: usize = 8;

/// AVX2 version of [`scalar::matrix_tone_rgba8`].
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
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
    let tables_ok = scale.len() > SCALE_STEPS && srgb.len() >= SRGB8_STEPS + TABLE_PAD;
    let done = if tables_ok { n / LANES * LANES } else { 0 };
    let zero = _mm256_setzero_ps();
    let one = _mm256_set1_ps(1.0);
    let half = _mm256_set1_ps(0.5);
    let mv = m.map(|r| r.map(|v| _mm256_set1_ps(v)));
    let luma = LUMA.map(|v| _mm256_set1_ps(v));
    let steps = _mm256_set1_ps((SCALE_STEPS - 1) as f32);
    let codes = _mm256_set1_ps((SRGB8_STEPS - 1) as f32);
    let byte = _mm256_set1_epi32(0xFF);
    let alpha = _mm256_set1_epi32(0xFF00_0000u32 as i32);
    let ione = _mm256_set1_epi32(1);
    let scale_ptr = scale.as_ptr();
    let srgb_ptr = srgb.as_ptr().cast::<i32>();
    let clip = |v: __m256| _mm256_min_ps(_mm256_max_ps(v, zero), one);
    for k in (0..done).step_by(LANES) {
        // SAFETY: k + 8 <= n, which is within every plane.
        let (r, g, b) = unsafe {
            (
                _mm256_loadu_ps(cam[0].as_ptr().add(k)),
                _mm256_loadu_ps(cam[1].as_ptr().add(k)),
                _mm256_loadu_ps(cam[2].as_ptr().add(k)),
            )
        };
        let row = |c: usize| {
            clip(_mm256_add_ps(
                _mm256_add_ps(_mm256_mul_ps(mv[c][0], r), _mm256_mul_ps(mv[c][1], g)),
                _mm256_mul_ps(mv[c][2], b),
            ))
        };
        let (lr, lg, lb) = (row(0), row(1), row(2));
        let l = _mm256_add_ps(
            _mm256_add_ps(_mm256_mul_ps(luma[0], lr), _mm256_mul_ps(luma[1], lg)),
            _mm256_mul_ps(luma[2], lb),
        );
        let pos = _mm256_mul_ps(clip(l), steps);
        let i = _mm256_cvttps_epi32(pos);
        let t = _mm256_sub_ps(pos, _mm256_cvtepi32_ps(i));
        // SAFETY: 0 <= pos <= SCALE_STEPS - 1, so i and i + 1 index
        // inside `scale`, which has SCALE_STEPS + 1 entries (tables_ok).
        let (a, bb) = unsafe {
            (
                _mm256_i32gather_ps::<4>(scale_ptr, i),
                _mm256_i32gather_ps::<4>(scale_ptr, _mm256_add_epi32(i, ione)),
            )
        };
        let s = _mm256_add_ps(a, _mm256_mul_ps(_mm256_sub_ps(bb, a), t));
        let mut xr = _mm256_mul_ps(lr, s);
        let mut xg = _mm256_mul_ps(lg, s);
        let mut xb = _mm256_mul_ps(lb, s);
        let mx = _mm256_max_ps(_mm256_max_ps(xr, xg), xb);
        let over = _mm256_cmp_ps::<_CMP_GT_OQ>(mx, one);
        if _mm256_movemask_ps(over) != 0 {
            let y = _mm256_min_ps(_mm256_mul_ps(l, s), one);
            let f = _mm256_div_ps(_mm256_sub_ps(one, y), _mm256_sub_ps(mx, y));
            let pull = |c: __m256| _mm256_add_ps(y, _mm256_mul_ps(_mm256_sub_ps(c, y), f));
            xr = _mm256_blendv_ps(xr, pull(xr), over);
            xg = _mm256_blendv_ps(xg, pull(xg), over);
            xb = _mm256_blendv_ps(xb, pull(xb), over);
        }
        let index =
            |c: __m256| _mm256_cvttps_epi32(_mm256_add_ps(_mm256_mul_ps(clip(c), codes), half));
        // SAFETY: each index is in 0..=SRGB8_STEPS - 1 and the table has
        // TABLE_PAD more bytes (tables_ok), so the 4-byte loads stay in it.
        let (cr, cg, cb) = unsafe {
            (
                _mm256_and_si256(_mm256_i32gather_epi32::<1>(srgb_ptr, index(xr)), byte),
                _mm256_and_si256(_mm256_i32gather_epi32::<1>(srgb_ptr, index(xg)), byte),
                _mm256_and_si256(_mm256_i32gather_epi32::<1>(srgb_ptr, index(xb)), byte),
            )
        };
        // Little endian: R, G, B, A in memory order.
        let px = _mm256_or_si256(
            _mm256_or_si256(cr, _mm256_slli_epi32::<8>(cg)),
            _mm256_or_si256(_mm256_slli_epi32::<16>(cb), alpha),
        );
        // SAFETY: bytes 4k .. 4k + 32 are inside `out` (k + 8 <= n).
        unsafe { _mm256_storeu_si256(out.as_mut_ptr().add(k * 4).cast::<__m256i>(), px) };
    }
    if done < n {
        let tail = [&cam[0][done..n], &cam[1][done..n], &cam[2][done..n]];
        scalar::matrix_tone_rgba8(m, tone, tail, &mut out[done * 4..n * 4]);
    }
}

/// AVX2 version of [`scalar::bayer_row`].
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
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
    // The vector loop reads i - 1 ..= i + 8 for i = i0 + k.
    let ok = i0 >= 1 && i0 + n < len && (q == 0 || q == 2);
    let done = if ok { n / LANES * LANES } else { 0 };
    let o = 2 - q.min(2);
    let halfv = _mm256_set1_ps(0.5);
    let quarter = _mm256_set1_ps(0.25);
    // Lanes 0, 2, 4, 6 hold even k: green when green_first.
    let even = _mm256_castsi256_ps(_mm256_setr_epi32(-1, 0, -1, 0, -1, 0, -1, 0));
    let green = if green_first {
        even
    } else {
        _mm256_xor_ps(even, _mm256_castsi256_ps(_mm256_set1_epi32(-1)))
    };
    let [p0, p1, p2] = out;
    let mut planes = [p0, p1, p2];
    for k in (0..done).step_by(LANES) {
        let i = i0 + k;
        // SAFETY: i - 1 >= 0 and i + 9 <= i0 + n + 1 <= len, so every
        // 8-float load below stays inside a, b and c.
        let (am, ac, ap, bm, bc, bp, cm, cc, cp) = unsafe {
            (
                _mm256_loadu_ps(a.as_ptr().add(i - 1)),
                _mm256_loadu_ps(a.as_ptr().add(i)),
                _mm256_loadu_ps(a.as_ptr().add(i + 1)),
                _mm256_loadu_ps(b.as_ptr().add(i - 1)),
                _mm256_loadu_ps(b.as_ptr().add(i)),
                _mm256_loadu_ps(b.as_ptr().add(i + 1)),
                _mm256_loadu_ps(c.as_ptr().add(i - 1)),
                _mm256_loadu_ps(c.as_ptr().add(i)),
                _mm256_loadu_ps(c.as_ptr().add(i + 1)),
            )
        };
        let h = _mm256_add_ps(bm, bp);
        let horiz = _mm256_mul_ps(h, halfv);
        let vert = _mm256_mul_ps(_mm256_add_ps(ac, cc), halfv);
        let cross = _mm256_mul_ps(_mm256_add_ps(_mm256_add_ps(h, ac), cc), quarter);
        let diag = _mm256_mul_ps(
            _mm256_add_ps(_mm256_add_ps(_mm256_add_ps(am, ap), cm), cp),
            quarter,
        );
        // Green pixels: G own, q horizontal, other vertical. Others: q
        // own, G from the cross, other from the diagonals.
        let vq = _mm256_blendv_ps(bc, horiz, green);
        let vg = _mm256_blendv_ps(cross, bc, green);
        let vo = _mm256_blendv_ps(diag, vert, green);
        // SAFETY: k + 8 <= n <= every plane's length.
        unsafe {
            _mm256_storeu_ps(planes[q].as_mut_ptr().add(k), vq);
            _mm256_storeu_ps(planes[1].as_mut_ptr().add(k), vg);
            _mm256_storeu_ps(planes[o].as_mut_ptr().add(k), vo);
        }
    }
    if done < n {
        let [p0, p1, p2] = &mut planes;
        let tail = [&mut p0[done..n], &mut p1[done..n], &mut p2[done..n]];
        // `done` is a multiple of 8: the tail starts on the same parity.
        scalar::bayer_row(a, b, c, i0 + done, n - done, green_first, q, tail);
    }
}

/// AVX2 version of [`scalar::normalize_row`].
///
/// # Safety
///
/// The CPU must support AVX2.
#[target_feature(enable = "avx2")]
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
    let bv = _mm256_setr_ps(b0, b1, b0, b1, b0, b1, b0, b1);
    let kv = _mm256_setr_ps(k0, k1, k0, k1, k0, k1, k0, k1);
    let zero = _mm256_setzero_ps();
    let one = _mm256_set1_ps(1.0);
    for k in (0..done).step_by(LANES) {
        // SAFETY: k + 8 <= n: 8 samples of `src` (16 bytes) are in bounds.
        let raw = unsafe { _mm_loadu_si128(src.as_ptr().add(k).cast::<__m128i>()) };
        let v = _mm256_cvtepi32_ps(_mm256_cvtepu16_epi32(raw));
        let r = _mm256_min_ps(
            _mm256_max_ps(_mm256_mul_ps(_mm256_sub_ps(v, bv), kv), zero),
            one,
        );
        // SAFETY: k + 8 <= n: 8 floats of `out` are in bounds.
        unsafe { _mm256_storeu_ps(out.as_mut_ptr().add(k), r) };
    }
    if done < n {
        // `done` is even, so the tail starts on the same parity.
        scalar::normalize_row(&src[done..n], odd_first, black, scale, &mut out[done..n]);
    }
}

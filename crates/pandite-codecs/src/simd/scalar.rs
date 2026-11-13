//! Scalar reference implementations of the vector kernels. The vector
//! versions must match these bit for bit.

use crate::raw::tone::{matrix_clip, srgb8_index, srgb8_table, Tone};

#[cfg(test)]
thread_local! {
    /// Calls of the scalar kernels on this thread, for the dispatch test.
    pub(crate) static CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one scalar kernel call (tests only).
#[inline]
fn counted() {
    #[cfg(test)]
    CALLS.with(|c| c.set(c.get() + 1));
}

/// Camera RGB (three planes) through the color matrix `m` (clipped to
/// 0..=1), the display tone curve and the sRGB transfer to RGBA8 (alpha
/// 255).
pub(crate) fn matrix_tone_rgba8(m: &[[f32; 3]; 3], tone: &Tone, cam: [&[f32]; 3], out: &mut [u8]) {
    counted();
    let table = srgb8_table();
    let [r, g, b] = cam;
    for (k, px) in out
        .chunks_exact_mut(4)
        .enumerate()
        .take(r.len().min(g.len()).min(b.len()))
    {
        let d = tone.apply(matrix_clip(m, [r[k], g[k], b[k]]));
        px[0] = table[srgb8_index(d[0])];
        px[1] = table[srgb8_index(d[1])];
        px[2] = table[srgb8_index(d[2])];
        px[3] = 255;
    }
}

/// One row of camera RGB as three planes.
pub(crate) type Planes<'a> = [&'a mut [f32]; 3];

/// Bilinear demosaic of the interior of one Bayer row with diagonal
/// greens (R-ARW-54, R-ARW-55 fixed-neighbor form). `a`, `b`, `c` are the
/// normalized rows above, at and below; pixel `k` of the output (k in
/// 0..n) sits at index `i0 + k` of those rows, and `i0 >= 1`,
/// `i0 + n < len` for all three. `green_first`: pixel 0 is green. `q`: the
/// other color on this row (0 red or 2 blue); the rows above and below
/// carry `2 - q`.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
pub(crate) fn bayer_row(
    a: &[f32],
    b: &[f32],
    c: &[f32],
    i0: usize,
    n: usize,
    green_first: bool,
    q: usize,
    out: Planes<'_>,
) {
    counted();
    let [o0, o1, o2] = out;
    let planes = [o0, o1, o2];
    let o = 2 - q;
    for k in 0..n {
        let i = i0 + k;
        let h = b[i - 1] + b[i + 1];
        let green = (k & 1 == 0) == green_first;
        if green {
            planes[1][k] = b[i];
            planes[q][k] = h * 0.5;
            planes[o][k] = (a[i] + c[i]) * 0.5;
        } else {
            planes[q][k] = b[i];
            planes[1][k] = (h + a[i] + c[i]) * 0.25;
            planes[o][k] = (a[i - 1] + a[i + 1] + c[i - 1] + c[i + 1]) * 0.25;
        }
    }
}

/// R-ARW-47 for one run of raw samples: `((s - black) * scale)` clipped
/// to 0..=1 (lower bound first), with the even and odd constants of the
/// row's CFA positions; `odd_first` when sample 0 sits in an odd column.
// Lower bound first, not `clamp`: NaN becomes 0 the same way in the vector
// paths.
#[allow(clippy::manual_clamp)]
pub(crate) fn normalize_row(
    src: &[u16],
    odd_first: bool,
    black: [f32; 2],
    scale: [f32; 2],
    out: &mut [f32],
) {
    counted();
    let first = usize::from(odd_first);
    for (k, (o, &s)) in out.iter_mut().zip(src).enumerate() {
        let p = (k + first) & 1;
        *o = ((f32::from(s) - black[p]) * scale[p]).max(0.0).min(1.0);
    }
}

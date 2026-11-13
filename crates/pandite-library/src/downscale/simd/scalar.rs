//! Scalar reference kernels. The vector versions must produce exactly the
//! same results.

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

/// Add the RGBA8 pixels of every row in `rows` times `weight` to the sums
/// `acc`, four per pixel, as many pixels as `acc` holds (every row holds
/// at least as many). `premultiply`: red, green and blue are first
/// multiplied by the pixel's alpha (alpha itself is added as is);
/// otherwise all four bytes are added as they are. Returns true when
/// every alpha byte added was 255.
///
/// The caller keeps every sum below `2^32`: a byte product is at most
/// `255 * 255`, and `rows.len() * weight` plus the weights already in a
/// sum stay at most 65535.
pub(crate) fn accumulate(rows: &[&[u8]], weight: u32, premultiply: bool, acc: &mut [u32]) -> bool {
    counted();
    let mut all = 0xFF;
    for row in rows {
        let pixels = row.chunks_exact(4).zip(acc.chunks_exact_mut(4));
        if premultiply {
            for (px, sum) in pixels {
                let a = u32::from(px[3]);
                sum[0] += u32::from(px[0]) * a * weight;
                sum[1] += u32::from(px[1]) * a * weight;
                sum[2] += u32::from(px[2]) * a * weight;
                sum[3] += a * weight;
                all &= px[3];
            }
        } else {
            for (px, sum) in pixels {
                sum[0] += u32::from(px[0]) * weight;
                sum[1] += u32::from(px[1]) * weight;
                sum[2] += u32::from(px[2]) * weight;
                sum[3] += u32::from(px[3]) * weight;
                all &= px[3];
            }
        }
    }
    all == 0xFF
}

/// For each run `(first, count)` of pixels of `acc` (four sums per pixel),
/// the four channel totals of the run, into `out`.
pub(crate) fn sum_runs(acc: &[u32], runs: &[(usize, usize)], out: &mut [[u64; 4]]) {
    counted();
    for (&(first, count), total) in runs.iter().zip(out.iter_mut()) {
        *total = [0; 4];
        for px in acc[first * 4..(first + count) * 4].chunks_exact(4) {
            total[0] += u64::from(px[0]);
            total[1] += u64::from(px[1]);
            total[2] += u64::from(px[2]);
            total[3] += u64::from(px[3]);
        }
    }
}

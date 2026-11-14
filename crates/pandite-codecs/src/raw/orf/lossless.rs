//! V5 and V6: the Olympus lossless compression (section Compression,
//! subsection V5 and V6: Olympus lossless compression).
//!
//! The code bits form one MSB-first bit string for the whole raster
//! (R-ORF-26). Each pixel is one code: a sign bit, 2 low bits, a unary part
//! (or its escape) and k remainder bits (R-ORF-29). The prediction uses the
//! decoded values of the same CFA color two columns and two rows away
//! (R-ORF-31). The rows depend on each other through the bit string, so the
//! decode is sequential.

use super::{fail, OrfResult};
use pandite_core::CancelToken;

/// Zero bits of the unary part after which the escape follows (step 4).
const ZERO_LIMIT: u32 = 12;
/// The escape reads 15 - k bits as the quotient (step 4).
const ESCAPE_BITS: u32 = 15;
/// The averaging threshold of the prediction table.
const THRESHOLD: i32 = 32;
/// Rows decoded between two checks of the cancel token.
const CANCEL_ROWS: usize = 16;

/// An MSB-first reader of the code bits that never reads past the end of
/// its bytes: a read that needs more bits than remain fails (R-ORF-60).
pub(super) struct Bits<'a> {
    data: &'a [u8],
    /// Next byte to load.
    pos: usize,
    /// The next `n` bits of the string in the high bits, zeros below.
    buf: u64,
    n: u32,
}

impl<'a> Bits<'a> {
    pub(super) fn new(data: &'a [u8]) -> Self {
        Bits {
            data,
            pos: 0,
            buf: 0,
            n: 0,
        }
    }

    /// Load bytes until at least 57 bits are buffered or the bytes end.
    /// One code holds at most 31 bits, so one refill serves a whole pixel.
    #[inline]
    fn refill(&mut self) {
        while self.n <= 56 {
            let Some(&b) = self.data.get(self.pos) else {
                break;
            };
            self.buf |= u64::from(b) << (56 - self.n);
            self.n += 8;
            self.pos += 1;
        }
    }

    /// Read `k` bits as an unsigned integer, the first bit most
    /// significant; k = 0 gives 0 (R-ORF-26).
    #[inline]
    fn take(&mut self, k: u32) -> Option<u32> {
        if k == 0 {
            return Some(0);
        }
        if self.n < k {
            return None;
        }
        let v = (self.buf >> (64 - k)) as u32;
        self.buf <<= k;
        self.n -= k;
        Some(v)
    }

    /// Step 4: read single bits until a 1 bit or until 12 zero bits. The
    /// number of zero bits; 12 means the escape follows.
    #[inline]
    fn unary(&mut self) -> Option<u32> {
        let avail = self.n.min(ZERO_LIMIT);
        let z = self.buf.leading_zeros();
        if z < avail {
            // A 1 bit ends the run after z zero bits.
            self.buf <<= z + 1;
            self.n -= z + 1;
            Some(z)
        } else if avail == ZERO_LIMIT {
            self.buf <<= ZERO_LIMIT;
            self.n -= ZERO_LIMIT;
            Some(ZERO_LIMIT)
        } else {
            // Fewer than 12 bits remain and all of them are zero.
            None
        }
    }

    /// The number of bits read so far.
    pub(super) fn consumed(&self) -> u64 {
        self.pos as u64 * 8 - u64::from(self.n)
    }
}

/// R-ORF-31: the prediction P of pixel x of the current row from the
/// values of the same color: Ra = s(x - 2, y), Rb = s(x, y - 2) and Rc =
/// s(x - 2, y - 2). `above` is row y - 2, `None` for y < 2.
#[inline]
pub(super) fn predict(row: &[u16], above: Option<&[u16]>, x: usize) -> i32 {
    match (x >= 2, above) {
        (false, None) => 0,
        (true, None) => i32::from(row[x - 2]),
        (false, Some(up)) => i32::from(up[x]),
        (true, Some(up)) => {
            let (ra, rb, rc) = (
                i32::from(row[x - 2]),
                i32::from(up[x]),
                i32::from(up[x - 2]),
            );
            let (da, db) = ((ra - rc).abs(), (rb - rc).abs());
            if (ra < rc && rc < rb) || (rb < rc && rc < ra) {
                // Rc strictly between Ra and Rb.
                if da > THRESHOLD || db > THRESHOLD {
                    ra + rb - rc
                } else {
                    // floor((Ra + Rb) / 2) of non-negative values.
                    (ra + rb) / 2
                }
            } else if da > db {
                ra
            } else {
                rb
            }
        }
    }
}

/// Decode the W x H raster of `out` (row-major) from the code bits
/// `stream` (the strip after its stream header, R-ORF-26), with values
/// from 0 to 2^`bits` - 1. Returns the number of code bits read; the bits
/// after the last pixel are ignored. Fails on a read beyond the end of the
/// strip (R-ORF-60) and on the first value outside the range (R-ORF-32,
/// R-ORF-61).
pub(super) fn decode(
    stream: &[u8],
    w: usize,
    h: usize,
    bits: u32,
    out: &mut [u16],
    cancel: &CancelToken,
) -> OrfResult<u64> {
    if out.len() != w * h {
        return Err(fail("raster size"));
    }
    let max = (1i32 << bits) - 1;
    let end = || fail("compressed data end before the last pixel");
    let mut r = Bits::new(stream);
    for y in 0..h {
        if y % CANCEL_ROWS == 0 {
            cancel.check()?;
        }
        let (done, rest) = out.split_at_mut(y * w);
        let row = &mut rest[..w];
        let above = (y >= 2).then(|| &done[(y - 2) * w..(y - 1) * w]);
        // R-ORF-27: A, N and C per parity, 0 at the start of every row.
        let mut a = [0u32; 2];
        let mut runs = [0u32; 2];
        let mut carry = [0i32; 2];
        for x in 0..w {
            let p = x & 1;
            // R-ORF-28 (step 1): the code length k.
            let len = 32 - a[p].leading_zeros();
            let k = if runs[p] >= 3 {
                len.max(2)
            } else {
                len.saturating_sub(2).max(4)
            };
            // The Bounds paragraph: k <= 13 (A < 2^15 after every value
            // that passed R-ORF-32), so 15 - k >= 2. Never true; kept so
            // that no shift can overflow.
            if k > 13 {
                return Err(fail("code length"));
            }
            r.refill();
            // R-ORF-29 (steps 2 to 5): sign, low bits, unary part with its
            // escape, remainder.
            let sign = r.take(1).ok_or_else(end)?;
            let low = r.take(2).ok_or_else(end)?;
            let z = r.unary().ok_or_else(end)?;
            let q = if z < ZERO_LIMIT {
                z
            } else {
                let q = r.take(ESCAPE_BITS - k).ok_or_else(end)?;
                // The bit after an escaped Q is read and ignored (OQ-2).
                r.take(1).ok_or_else(end)?;
                q
            };
            let rem = r.take(k).ok_or_else(end)?;
            // M = Q * 2^k + R: below 2^15 after an escape, below 12 * 2^13
            // otherwise; 32-bit signed arithmetic from here (R-ORF-61).
            let m = ((q << k) | rem) as i32;
            // R-ORF-30 (steps 6 to 8).
            let e = if sign == 0 { m } else { -m - 1 };
            let d = e + carry[p];
            let err = 4 * d + low as i32;
            // Step 9 with R-ORF-31.
            let s = predict(row, above, x) + err;
            // R-ORF-32, R-ORF-61: the first value outside 0 to 2^B - 1
            // fails; the partly decoded raster is dropped by the caller.
            if !(0..=max).contains(&s) {
                return Err(fail("compressed value outside the range"));
            }
            row[x] = s as u16;
            // R-ORF-30 (step 10): C with the old C, floor division.
            carry[p] = (3 * d + carry[p]).div_euclid(32);
            a[p] = m as u32;
            runs[p] = if m <= 16 { runs[p] + 1 } else { 0 };
        }
    }
    Ok(r.consumed())
}

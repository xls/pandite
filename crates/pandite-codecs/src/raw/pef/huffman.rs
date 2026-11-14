//! Pentax Huffman coding of variants P3 and P4 (section Compression): the
//! table of maker-note tag 0x0220, the bit stream, the difference and the
//! prediction.

use super::{fail, PefResult};
use pandite_core::CancelToken;

/// Entries of the code lookup table: one per 12-bit stream prefix.
const LUT_SIZE: usize = 1 << 12;

/// A validated table (R-PEF-27, R-PEF-28).
#[derive(Clone, Debug)]
pub(crate) struct Table {
    /// For each SSSS value s: the code as an L(s)-bit number and L(s).
    #[cfg_attr(not(test), allow(dead_code))]
    pub codes: Vec<(u16, u8)>,
    /// For each 12-bit prefix of the stream: `(L << 8) | s` of the entry
    /// whose code begins the prefix, 0 when no code matches. A structure of
    /// fixed size (R-PEF-65).
    lut: Vec<u16>,
}

impl Table {
    /// Parse and validate the value bytes of tag 0x0220 for `bps`-bit data;
    /// `le` is the byte order of the maker note. `None` when the table is
    /// not valid.
    pub(crate) fn parse(bytes: &[u8], bps: u32, le: bool) -> Option<Table> {
        if !matches!(bps, 12 | 14) {
            return None;
        }
        // N = BPS + 1 entries.
        let n = bps as usize + 1;
        // R-PEF-27: the count is 14 + 3 * N and V equals N - 12.
        if bytes.len() != 14 + 3 * n {
            return None;
        }
        let v = super::tiff::u16_at(bytes, 0, le)?;
        if usize::from(v) != n - 12 {
            return None;
        }
        let words = super::tiff::u16s(&bytes[14..14 + 2 * n], le);
        let lengths = &bytes[14 + 2 * n..14 + 3 * n];
        let mut codes = Vec::with_capacity(n);
        for (&c, &l) in words.iter().zip(lengths) {
            // R-PEF-28: 1 <= L(s) <= 12, C(s) <= 4095 and the low 12 - L(s)
            // bits of C(s) are zero.
            if !(1..=12).contains(&l) || c > 4095 || c % (1 << (12 - l)) != 0 {
                return None;
            }
            codes.push((c >> (12 - l), l));
        }
        // R-PEF-28: no two codes are equal or one the beginning of another.
        for (i, &(a, la)) in codes.iter().enumerate() {
            for &(b, lb) in &codes[i + 1..] {
                let (short, ls, long, ll) = if la <= lb {
                    (a, la, b, lb)
                } else {
                    (b, lb, a, la)
                };
                if long >> (ll - ls) == short {
                    return None;
                }
            }
        }
        let mut lut = vec![0u16; LUT_SIZE];
        for (s, &(c, l)) in codes.iter().enumerate() {
            let lo = usize::from(c) << (12 - l);
            let hi = (usize::from(c) + 1) << (12 - l);
            for e in &mut lut[lo..hi] {
                *e = (u16::from(l) << 8) | s as u16;
            }
        }
        Some(Table { codes, lut })
    }
}

/// The 32 stream bits that start at bit `pos`, most significant bit first
/// (R-PEF-29). Bits beyond the last byte of the strip read as 0: only the
/// look-ahead reaches them (R-PEF-35).
#[inline]
fn peek32(strip: &[u8], pos: u64) -> u32 {
    let byte = (pos >> 3) as usize;
    let mut b = [0u8; 8];
    match strip.get(byte..byte.saturating_add(8)) {
        Some(s) => b.copy_from_slice(s),
        None => {
            let tail = strip.get(byte..).unwrap_or(&[]);
            b[..tail.len()].copy_from_slice(tail);
        }
    }
    ((u64::from_be_bytes(b) << (pos & 7)) >> 32) as u32
}

/// Decode the `w` x `h` raster of `bps`-bit values coded in `strip` with
/// `table`. Calls `put(x, y, value)` for each pixel in raster order and
/// returns the number of coded bits. Fails when a code word or its extra
/// bits do not fit the strip, or no code matches. Holds no row: the
/// predictions need only the last two values of the row and the first two
/// values of the two rows above.
pub(crate) fn decode(
    strip: &[u8],
    table: &Table,
    (w, h): (usize, usize),
    bps: u32,
    cancel: &CancelToken,
    mut put: impl FnMut(usize, usize, u16),
) -> PefResult<u64> {
    let maxv = (1i32 << bps) - 1;
    let end = strip.len() as u64 * 8;
    let mut pos = 0u64;
    // R-PEF-65: no allocation here at all, so never memory in proportion
    // to the decoded bits. The values of columns 0 and 1 of the last row
    // of each row parity predict the row two rows below.
    let mut starts = [[0u16; 2]; 2];
    for y in 0..h {
        if y % 16 == 0 {
            cancel.check()?;
        }
        // The values of columns x - 2 and x - 1 of this row.
        let mut last = [0u16; 2];
        // R-PEF-30: the pixels in raster order; R-PEF-29: the rows follow
        // each other without alignment.
        for x in 0..w {
            let bits = peek32(strip, pos);
            // R-PEF-31: the entry whose L(s)-bit code equals the next L(s)
            // bits of the stream; fail when none matches.
            let e = table.lut[(bits >> 20) as usize];
            let len = u32::from(e >> 8);
            if len == 0 {
                return Err(fail("no Huffman code matches the stream"));
            }
            let s = u32::from(e & 0xFF);
            // R-PEF-35: the code word and its extra bits lie inside the
            // strip.
            let used = u64::from(len + s);
            if pos + used > end {
                return Err(fail("Huffman stream ends inside a pixel"));
            }
            // R-PEF-32: the difference D from SSSS s and the extra bits E.
            let d = if s == 0 {
                0
            } else {
                let ev = ((bits << len) >> (32 - s)) as i32;
                if ev >= 1 << (s - 1) {
                    ev
                } else {
                    ev - (1 << s) + 1
                }
            };
            pos += used;
            // R-PEF-33: the prediction from x - 2, from y - 2 at the row
            // start, 0 in the first two pixels of the first two rows.
            let start = &mut starts[y & 1];
            let p = match x {
                0 | 1 if y >= 2 => start[x & 1],
                0 | 1 => 0,
                _ => last[0],
            };
            // R-PEF-34: the clipped sum is stored and predicts later pixels.
            let v = (i32::from(p) + d).clamp(0, maxv) as u16;
            if x < 2 {
                start[x & 1] = v;
            }
            last = [last[1], v];
            put(x, y, v);
        }
    }
    // R-PEF-35: bits after the last coded pixel are ignored.
    Ok(pos)
}

//! The prefix-coded differences of V6, V7 and V8 (section Compression,
//! subsections Bit stream, Code tables, Differences and prediction).
//!
//! The four code tables are given as BITS and HUFFVAL (ITU-T T.81 Annex
//! C); the codes follow from them by the procedure of figures C.1 and C.2.
//! Codes that the specification marks "not established" are kept as codes
//! without a category: reading one ends the sensor path (R-NEF-31), and
//! the file shows its preview. The tables are not completed in any other
//! way.

use super::{fail, NefResult};
use pandite_core::CancelToken;

/// One code table of the specification.
#[derive(Debug)]
pub(crate) struct Spec {
    /// The table name, for example "L12".
    pub name: &'static str,
    /// Numbers of codes of lengths 1 to 16 (BITS).
    bits: [u8; 16],
    /// Categories in code order (HUFFVAL); `None` marks a code that is
    /// not established.
    values: &'static [Option<u8>],
    /// Code subtrees that are not established: (code, length). Every bit
    /// sequence that starts with such a code is not established.
    open: &'static [(u16, u8)],
}

/// Table L12 (V6, bps 12).
pub(crate) const L12: Spec = Spec {
    name: "L12",
    bits: [0, 1, 4, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    values: &[
        Some(5),
        Some(4),
        Some(6),
        Some(3),
        Some(7),
        Some(2),
        Some(8),
        Some(1),
        Some(9),
        Some(0),
        Some(10),
        Some(11),
        Some(12),
    ],
    open: &[],
};

/// Table L14 (V6, bps 14).
pub(crate) const L14: Spec = Spec {
    name: "L14",
    bits: [0, 1, 4, 2, 2, 3, 1, 2, 0, 0, 0, 0, 0, 0, 0, 0],
    values: &[
        Some(7),
        Some(6),
        Some(8),
        Some(5),
        Some(9),
        Some(4),
        Some(10),
        Some(3),
        Some(11),
        Some(12),
        Some(2),
        Some(0),
        Some(1),
        Some(13),
        Some(14),
    ],
    open: &[],
};

/// Table Y12 (V7, and V8 with bps 12): the codes 11111110 and 111111111
/// are not established (OQ-1).
pub(crate) const Y12: Spec = Spec {
    name: "Y12",
    bits: [0, 1, 5, 1, 1, 1, 1, 1, 2, 0, 0, 0, 0, 0, 0, 0],
    values: &[
        Some(5),
        Some(4),
        Some(3),
        Some(6),
        Some(2),
        Some(7),
        Some(1),
        Some(0),
        Some(8),
        Some(9),
        None,
        Some(10),
        None,
    ],
    open: &[],
};

/// Table Y14 (V8 with bps 14): the subtree 111111111 is not established
/// (OQ-1).
pub(crate) const Y14: Spec = Spec {
    name: "Y14",
    bits: [0, 1, 4, 3, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    values: &[
        Some(5),
        Some(6),
        Some(4),
        Some(7),
        Some(8),
        Some(3),
        Some(9),
        Some(2),
        Some(1),
        Some(0),
        Some(10),
        Some(11),
        Some(12),
    ],
    open: &[(0b1_1111_1111, 9)],
};

/// What a code means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Meaning {
    /// Category s: s extra bits follow.
    Category(u8),
    /// A code marked "not established" (R-NEF-31).
    NotEstablished,
}

impl Spec {
    /// The codes of the table by S2 Annex C (figures C.1 and C.2): the
    /// first code of the shortest length is all zeros, each next code of
    /// the same length is the previous code plus 1, and each step to a
    /// longer length shifts the next code left by one bit. Then the
    /// subtrees that are not established. Each item is (code, length,
    /// meaning).
    pub(crate) fn codes(&self) -> Vec<(u32, u8, Meaning)> {
        let mut out = Vec::new();
        let mut code = 0u32;
        let mut k = 0usize;
        for (i, &n) in self.bits.iter().enumerate() {
            let len = i as u8 + 1;
            for _ in 0..n {
                let meaning = match self.values.get(k).copied().flatten() {
                    Some(s) => Meaning::Category(s),
                    None => Meaning::NotEstablished,
                };
                out.push((code, len, meaning));
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        for &(c, len) in self.open {
            out.push((u32::from(c), len, Meaning::NotEstablished));
        }
        out
    }
}

/// A lookup table: the meaning and length of the code that starts every
/// bit sequence of `width` bits.
pub(crate) struct Table {
    /// The table name, for reports.
    pub name: &'static str,
    /// Lookup width: the longest code length.
    width: u32,
    /// Entry per `width`-bit prefix: 0 = no code, else length << 8 |
    /// category, category 0xFF = not established.
    lut: Vec<u16>,
    /// Fewest bits one pixel can consume: the smallest code length plus
    /// category over the established codes.
    pub min_bits: u32,
}

/// The category byte of a code that is not established.
const OPEN: u16 = 0xFF;

impl Table {
    /// Build the lookup table of a specification table (R-NEF-30).
    pub(crate) fn new(spec: &Spec) -> Table {
        let codes = spec.codes();
        let width = codes.iter().map(|c| u32::from(c.1)).max().unwrap_or(1);
        let mut lut = vec![0u16; 1 << width];
        let mut min_bits = u32::MAX;
        for &(code, len, meaning) in &codes {
            let len32 = u32::from(len);
            let shift = width - len32;
            let first = (code << shift) as usize;
            let last = ((code + 1) << shift) as usize;
            let cat = match meaning {
                Meaning::Category(s) => {
                    min_bits = min_bits.min(len32 + u32::from(s));
                    u16::from(s)
                }
                Meaning::NotEstablished => OPEN,
            };
            for e in &mut lut[first..last.min(1 << width)] {
                *e = (u16::from(len) << 8) | cat;
            }
        }
        Table {
            name: spec.name,
            width,
            lut,
            min_bits: min_bits.max(1),
        }
    }
}

/// A most-significant-bit-first reader over the strip (R-NEF-29) that
/// supplies zero bits past the end and counts the bits it consumed
/// (R-NEF-64).
struct Reader<'a> {
    data: &'a [u8],
    /// The next byte to load.
    pos: usize,
    /// The next bits, left-aligned.
    acc: u64,
    /// Valid bits in `acc`.
    n: u32,
    /// Bits consumed so far.
    used: u64,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader {
            data,
            pos: 0,
            acc: 0,
            n: 0,
            used: 0,
        }
    }

    /// Load bytes until more than 56 bits are buffered; zero bytes past the
    /// end of the strip (R-NEF-64: a look-ahead never fails by itself).
    #[inline]
    fn refill(&mut self) {
        while self.n <= 56 {
            let b = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos = self.pos.saturating_add(1);
            self.acc |= u64::from(b) << (56 - self.n);
            self.n += 8;
        }
    }

    /// The next `k` bits (1 to 32) as a number, without consuming them.
    #[inline]
    fn peek(&self, k: u32) -> u32 {
        (self.acc >> (64 - k)) as u32
    }

    /// Consume `k` bits (at most the buffered ones).
    #[inline]
    fn skip(&mut self, k: u32) {
        if k == 0 {
            return;
        }
        self.acc = if k >= 64 { 0 } else { self.acc << k };
        self.n -= k;
        self.used += u64::from(k);
    }
}

/// The difference of S2 F.2.2.1 (procedure EXTEND) for category `s` and
/// extra bits `e` (R-NEF-32): 0 for s = 0, e when e >= 2^(s - 1), else
/// e - 2^s + 1.
#[inline]
pub(crate) fn extend(s: u32, e: u32) -> i32 {
    if s == 0 {
        return 0;
    }
    let half = 1i32 << (s - 1);
    let e = e as i32;
    if e >= half {
        e
    } else {
        e - (1i32 << s) + 1
    }
}

/// The parameters of one decode.
pub(crate) struct Job<'a> {
    /// The strip bytes: one bit stream for all rows.
    pub stream: &'a [u8],
    /// Pixels per row W.
    pub width: usize,
    /// Rows H.
    pub height: usize,
    /// The code table.
    pub table: &'a Table,
    /// Start predictors P0 to P3 (tag 0x0096 bytes 2 to 9).
    pub predictors: [i32; 4],
    /// Code values must lie in 0 to `limit` - 1 (R-NEF-33).
    pub limit: i32,
}

/// Decode `job.height` rows of code values (R-NEF-29 to R-NEF-33). `row`
/// receives each row in raster order. Returns the number of bits consumed;
/// the bits after the last pixel of row H - 1 are never checked
/// (R-NEF-29).
pub(crate) fn decode(
    job: &Job<'_>,
    cancel: &CancelToken,
    mut row: impl FnMut(usize, &[i32]) -> NefResult<()>,
) -> NefResult<u64> {
    let (w, h, table) = (job.width, job.height, job.table);
    // R-NEF-65: the start predictors need two columns and two rows. The
    // rows are a precondition of the plan; the decoder itself only needs
    // the columns for its prediction state.
    if w < 2 {
        return Err(fail("fewer than 2 columns"));
    }
    let total = (job.stream.len() as u64).saturating_mul(8);
    let mut bits = Reader::new(job.stream);
    // Code values of pixels (0, y - 2) and (1, y - 2) by row parity; for
    // rows 0 and 1 the start predictors P(2 * y + x).
    let p = job.predictors;
    let mut above = [[p[0], p[1]], [p[2], p[3]]];
    let mut values = vec![0i32; w];
    for y in 0..h {
        if y % 64 == 0 {
            cancel.check()?;
        }
        let parity = y & 1;
        // R-NEF-32: x = 0 and 1 predict from two rows above (or P); x >= 2
        // from the pixel two columns to the left.
        let mut last = above[parity];
        for (x, out) in values.iter_mut().enumerate() {
            if bits.n < 32 {
                bits.refill();
            }
            let entry = table.lut[bits.peek(table.width) as usize];
            let len = u32::from(entry >> 8);
            let cat = entry & 0xFF;
            // R-NEF-31: no listed code matches, or a code that is not
            // established.
            if len == 0 {
                return Err(fail(format!("no code of table {} matches", table.name)));
            }
            if cat == OPEN {
                return Err(fail(format!(
                    "a code of table {} that is not established",
                    table.name
                )));
            }
            bits.skip(len);
            let s = u32::from(cat);
            let e = if s > 0 {
                let e = bits.peek(s);
                bits.skip(s);
                e
            } else {
                0
            };
            // R-NEF-64: the code and its extra bits lie inside the strip.
            if bits.used > total {
                return Err(fail("the codes run past the end of the strip"));
            }
            // R-NEF-66: 32-bit signed arithmetic; |d| < 2^14 and the
            // prediction lies in 0 to limit - 1, so the sum cannot
            // overflow before the range check.
            let v = last[x & 1] + extend(s, e);
            // R-NEF-33: the code value range.
            if v < 0 || v >= job.limit {
                return Err(fail(format!(
                    "code value {v} outside 0 to {}",
                    job.limit - 1
                )));
            }
            last[x & 1] = v;
            *out = v;
        }
        above[parity] = [values[0], values[1]];
        row(y, &values)?;
    }
    Ok(bits.used)
}

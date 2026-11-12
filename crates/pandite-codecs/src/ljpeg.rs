//! Lossless JPEG (ITU T.81 process 14, SOF3) decoder, as used inside DNG
//! files. Supports 2 to 16-bit precision, 1 to 4 interleaved components,
//! predictors 1 to 7, point transform and restart intervals.

use crate::util::{be16, corrupt, unsupported};
use pandite_core::Result;

const FORMAT: &str = "lossless jpeg";

/// A canonical Huffman table for DC-style symbols.
#[derive(Clone, Default)]
struct Huffman {
    /// (code length, code, symbol), sorted by length then code.
    lookup: Vec<(u8, u16, u8)>,
    /// Fast table indexed by the next 9 bits: (length, symbol), length 0 = slow path.
    fast: Vec<(u8, u8)>,
}

impl Huffman {
    fn new(counts: &[u8; 16], symbols: &[u8]) -> Result<Self> {
        let mut lookup = Vec::with_capacity(symbols.len());
        let mut code: u32 = 0;
        let mut k = 0;
        for (i, &n) in counts.iter().enumerate() {
            let len = i as u8 + 1;
            for _ in 0..n {
                let sym = *symbols
                    .get(k)
                    .ok_or_else(|| corrupt(FORMAT, "short table"))?;
                if code >= 1 << len {
                    return Err(corrupt(FORMAT, "bad Huffman table"));
                }
                lookup.push((len, code as u16, sym));
                code += 1;
                k += 1;
            }
            code <<= 1;
        }
        let mut fast = vec![(0u8, 0u8); 512];
        for &(len, code, sym) in &lookup {
            if len <= 9 {
                let shift = 9 - len;
                let start = (code as usize) << shift;
                for e in &mut fast[start..start + (1 << shift)] {
                    *e = (len, sym);
                }
            }
        }
        Ok(Huffman { lookup, fast })
    }
}

/// Bit reader over entropy-coded data with 0xFF00 unstuffing.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
    marker: bool,
}

impl<'a> Bits<'a> {
    fn fill(&mut self) {
        while self.n <= 56 {
            let mut b = 0u8;
            if !self.marker && self.pos < self.d.len() {
                b = self.d[self.pos];
                if b == 0xFF {
                    let next = self.d.get(self.pos + 1).copied().unwrap_or(0);
                    if next == 0 {
                        self.pos += 2;
                    } else {
                        // A marker: feed zeros from here on.
                        self.marker = true;
                        b = 0;
                    }
                } else {
                    self.pos += 1;
                }
            }
            self.acc |= u64::from(b) << (56 - self.n);
            self.n += 8;
        }
    }

    fn peek(&mut self, bits: u32) -> u32 {
        self.fill();
        (self.acc >> (64 - bits)) as u32
    }

    fn skip(&mut self, bits: u32) {
        self.acc <<= bits;
        self.n -= bits;
    }

    fn get(&mut self, bits: u32) -> u32 {
        if bits == 0 {
            return 0;
        }
        let v = self.peek(bits);
        self.skip(bits);
        v
    }

    fn symbol(&mut self, h: &Huffman) -> Result<u8> {
        let (len, sym) = h.fast[self.peek(9) as usize];
        if len > 0 {
            self.skip(u32::from(len));
            return Ok(sym);
        }
        let code16 = self.peek(16);
        for &(len, code, sym) in &h.lookup {
            if len > 9 && (code16 >> (16 - len)) as u16 == code {
                self.skip(u32::from(len));
                return Ok(sym);
            }
        }
        Err(corrupt(FORMAT, "bad Huffman code"))
    }

    /// Skip to the byte after the next RSTn marker.
    fn restart(&mut self) -> Result<()> {
        self.acc = 0;
        self.n = 0;
        self.marker = false;
        while self.pos + 1 < self.d.len() {
            if self.d[self.pos] == 0xFF && (0xD0..=0xD7).contains(&self.d[self.pos + 1]) {
                self.pos += 2;
                return Ok(());
            }
            self.pos += 1;
        }
        Err(corrupt(FORMAT, "missing restart marker"))
    }
}

/// A decoded lossless JPEG: samples interleaved per pixel.
pub(crate) struct Decoded {
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub samples: Vec<u16>,
}

/// Decode a lossless JPEG stream.
pub(crate) fn decode(data: &[u8]) -> Result<Decoded> {
    if data.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(corrupt(FORMAT, "missing SOI"));
    }
    let mut tables: [Option<Huffman>; 4] = Default::default();
    let mut frame: Option<(u8, usize, usize, Vec<u8>)> = None;
    let mut restart = 0usize;
    let mut pos = 2;
    loop {
        while data.get(pos) == Some(&0xFF) && data.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        if data.get(pos) != Some(&0xFF) {
            return Err(corrupt(FORMAT, "expected a marker"));
        }
        let marker = *data
            .get(pos + 1)
            .ok_or_else(|| corrupt(FORMAT, "truncated"))?;
        let len = usize::from(be16(data, pos + 2).ok_or_else(|| corrupt(FORMAT, "truncated"))?);
        let body = data
            .get(pos + 4..pos + 2 + len)
            .ok_or_else(|| corrupt(FORMAT, "truncated segment"))?;
        match marker {
            0xC4 => {
                let mut p = 0;
                while p + 17 <= body.len() {
                    let id = usize::from(body[p] & 3);
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&body[p + 1..p + 17]);
                    let total: usize = counts.iter().map(|&c| usize::from(c)).sum();
                    let syms = body
                        .get(p + 17..p + 17 + total)
                        .ok_or_else(|| corrupt(FORMAT, "short table"))?;
                    tables[id] = Some(Huffman::new(&counts, syms)?);
                    p += 17 + total;
                }
            }
            0xC3 => {
                let precision = *body.first().ok_or_else(|| corrupt(FORMAT, "short SOF"))?;
                let h = usize::from(be16(body, 1).unwrap_or(0));
                let w = usize::from(be16(body, 3).unwrap_or(0));
                let n = usize::from(*body.get(5).unwrap_or(&0));
                let ids: Vec<u8> = (0..n)
                    .filter_map(|i| body.get(6 + 3 * i).copied())
                    .collect();
                if w == 0
                    || h == 0
                    || n == 0
                    || n > 4
                    || ids.len() != n
                    || !(2..=16).contains(&precision)
                {
                    return Err(corrupt(FORMAT, "bad frame header"));
                }
                frame = Some((precision, w, h, ids));
            }
            0xC0..=0xC2 | 0xC5..=0xCF if marker != 0xC8 && marker != 0xCC => {
                return Err(unsupported(FORMAT, "not a lossless JPEG"));
            }
            0xDD => restart = usize::from(be16(body, 0).unwrap_or(0)),
            0xDA => {
                let (precision, w, h, ids) = frame
                    .clone()
                    .ok_or_else(|| corrupt(FORMAT, "scan before frame"))?;
                let ns = usize::from(*body.first().unwrap_or(&0));
                if ns != ids.len() {
                    return Err(unsupported(FORMAT, "non-interleaved scans"));
                }
                let mut comp_tables = Vec::with_capacity(ns);
                for i in 0..ns {
                    let td = usize::from(body.get(2 + 2 * i).copied().unwrap_or(0) >> 4) & 3;
                    comp_tables.push(
                        tables[td]
                            .clone()
                            .ok_or_else(|| corrupt(FORMAT, "missing Huffman table"))?,
                    );
                }
                let predictor = *body.get(1 + 2 * ns).unwrap_or(&1);
                let pt = u32::from(body.get(3 + 2 * ns).copied().unwrap_or(0) & 15);
                let start = pos + 2 + len;
                let samples = scan(
                    &data[start.min(data.len())..],
                    &comp_tables,
                    w,
                    h,
                    u32::from(precision),
                    predictor,
                    pt,
                    restart,
                )?;
                return Ok(Decoded {
                    width: w,
                    height: h,
                    components: ns,
                    samples,
                });
            }
            0xD9 => return Err(corrupt(FORMAT, "no scan")),
            _ => {}
        }
        pos += 2 + len;
    }
}

#[allow(clippy::too_many_arguments)]
fn scan(
    d: &[u8],
    tables: &[Huffman],
    w: usize,
    h: usize,
    precision: u32,
    predictor: u8,
    pt: u32,
    restart: usize,
) -> Result<Vec<u16>> {
    if !(1..=7).contains(&predictor) {
        return Err(unsupported(FORMAT, format!("predictor {predictor}")));
    }
    let n = tables.len();
    let row_len = w * n;
    let total = row_len
        .checked_mul(h)
        .ok_or_else(|| corrupt(FORMAT, "image too large"))?;
    // Reject streams far too short for the claimed size (each sample costs
    // at least one bit).
    if d.len().saturating_mul(8) < total {
        return Err(corrupt(FORMAT, "truncated scan"));
    }
    let mut out = vec![0u16; total];
    let mut bits = Bits {
        d,
        pos: 0,
        acc: 0,
        n: 0,
        marker: false,
    };
    let default = 1i32 << (precision - pt - 1);
    let mask = (1i32 << precision) - 1;
    let mut since_restart = 0usize;
    let mut first_line = true;
    for y in 0..h {
        for x in 0..w {
            if restart > 0 && since_restart == restart {
                bits.restart()?;
                since_restart = 0;
                first_line = true;
            }
            let reset = restart > 0 && since_restart == 0;
            for (c, table) in tables.iter().enumerate() {
                let i = y * row_len + x * n + c;
                let ra = |o: &Vec<u16>| i32::from(o[i - n]) >> pt;
                let rb = |o: &Vec<u16>| i32::from(o[i - row_len]) >> pt;
                let rc = |o: &Vec<u16>| i32::from(o[i - row_len - n]) >> pt;
                let pred = if (y == 0 && x == 0) || (reset && x == 0 && first_line) {
                    default
                } else if first_line || y == 0 {
                    if x == 0 {
                        default
                    } else {
                        ra(&out)
                    }
                } else if x == 0 {
                    rb(&out)
                } else {
                    match predictor {
                        1 => ra(&out),
                        2 => rb(&out),
                        3 => rc(&out),
                        4 => ra(&out) + rb(&out) - rc(&out),
                        5 => ra(&out) + ((rb(&out) - rc(&out)) >> 1),
                        6 => rb(&out) + ((ra(&out) - rc(&out)) >> 1),
                        _ => (ra(&out) + rb(&out)) >> 1,
                    }
                };
                let s = u32::from(bits.symbol(table)?);
                let diff = match s {
                    0 => 0,
                    16 => 32768,
                    s if s < 16 => {
                        let v = bits.get(s) as i32;
                        if v < (1 << (s - 1)) {
                            v - (1 << s) + 1
                        } else {
                            v
                        }
                    }
                    _ => return Err(corrupt(FORMAT, "bad difference category")),
                };
                let v = (pred + diff) & mask;
                out[i] = (v << pt) as u16;
            }
            since_restart += 1;
        }
        first_line = false;
    }
    Ok(out)
}

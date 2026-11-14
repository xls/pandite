//! Lossless JPEG (ITU-T T.81 Annex H, Huffman coding, SOF3) for ARW tiles.
//!
//! The decoder follows T.81: Huffman tables by Annex C, the DECODE,
//! RECEIVE and EXTEND procedures of F.2.2, predictors by Table H.1,
//! difference categories by Table H.2, reconstruction modulo 2^16, output
//! multiplied by 2^Pt (H.2.2), restart intervals by E.2.4 and H.1.1. One
//! departure is optional: [`LineStart::Sony`] applies R-ARW-39.
//!
//! Every sample is written through a [`Target`]: a base index and strides
//! into an output buffer, so V4 components land directly in their 2 x 2
//! CFA positions and V5 components in their own planes.

use super::{fail, ArwResult};
use pandite_core::CancelToken;

/// Bits looked up at once in the Huffman fast table.
const FAST_BITS: u32 = 9;

/// How the first sample of a line is predicted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::raw) enum LineStart {
    /// T.81 H.1.2.1: from the sample above (Rb). Used by the tests that
    /// show what the Sony rule changes.
    #[cfg_attr(not(test), allow(dead_code))]
    T81,
    /// R-ARW-39: for a component with V >= 2, line y with y mod V = 0 and
    /// y >= V predicts from (0, y - V); all other lines as T.81.
    Sony,
}

/// A Huffman table built from BITS and HUFFVAL (T.81 Annex C, F.2.2.3).
#[derive(Clone)]
struct Huffman {
    /// Indexed by the next `FAST_BITS` bits: (code length << 8) | symbol,
    /// 0 when no code of length <= `FAST_BITS` matches.
    fast: Vec<u16>,
    /// MAXCODE(I) for I = 1 to 16 (-1 when no code has length I).
    maxcode: [i32; 17],
    /// MINCODE(I).
    mincode: [i32; 17],
    /// VALPTR(I).
    valptr: [usize; 17],
    /// HUFFVAL.
    values: Vec<u8>,
}

impl Huffman {
    /// Build the table. R-ARW-73: fails when the codes overflow the code
    /// space of their length.
    fn new(bits: &[u8; 16], values: &[u8]) -> ArwResult<Self> {
        let mut maxcode = [-1i32; 17];
        let mut mincode = [0i32; 17];
        let mut valptr = [0usize; 17];
        let mut fast = vec![0u16; 1 << FAST_BITS];
        // Annex C, Figures C.1 and C.2: codes of each length are
        // consecutive; CODE shifts left by one at each new length.
        let mut code: u32 = 0;
        let mut k = 0usize;
        for len in 1..=16usize {
            let n = u32::from(bits[len - 1]);
            if n > 0 {
                if code + n > (1u32 << len) {
                    return Err(fail("Huffman codes overflow their length"));
                }
                valptr[len] = k;
                mincode[len] = code as i32;
                maxcode[len] = (code + n - 1) as i32;
                for c in code..code + n {
                    let sym = *values.get(k).ok_or_else(|| fail("short HUFFVAL"))?;
                    if len as u32 <= FAST_BITS {
                        let shift = FAST_BITS - len as u32;
                        let start = (c << shift) as usize;
                        for e in &mut fast[start..start + (1usize << shift)] {
                            *e = ((len as u16) << 8) | u16::from(sym);
                        }
                    }
                    k += 1;
                }
                code += n;
            }
            code <<= 1;
        }
        Ok(Huffman {
            fast,
            maxcode,
            mincode,
            valptr,
            values: values.to_vec(),
        })
    }
}

/// Bit reader over entropy-coded data (T.81 F.2.2.5). It removes stuffed
/// zero bytes, stops at any marker, and never reads outside its slice
/// (R-ARW-75). Look-ahead past the end yields zero bits; consuming them
/// fails.
struct Bits<'a> {
    d: &'a [u8],
    pos: usize,
    acc: u64,
    /// Valid bits in `acc`, left-aligned.
    n: u32,
    /// A marker or the end of the slice was reached.
    end: bool,
}

impl<'a> Bits<'a> {
    fn new(d: &'a [u8]) -> Self {
        Bits {
            d,
            pos: 0,
            acc: 0,
            n: 0,
            end: false,
        }
    }

    /// Fill the accumulator to at least 57 bits, or to the data end.
    #[inline]
    fn fill(&mut self) {
        while self.n <= 56 && !self.end {
            let Some(&b) = self.d.get(self.pos) else {
                self.end = true;
                break;
            };
            if b == 0xFF {
                // FF 00 is a stuffed FF; FF followed by anything else (or
                // nothing) is a marker that ends the data.
                if self.d.get(self.pos + 1) == Some(&0) {
                    self.pos += 2;
                } else {
                    self.end = true;
                    break;
                }
            } else {
                self.pos += 1;
            }
            self.acc |= u64::from(b) << (56 - self.n);
            self.n += 8;
        }
    }

    /// The next `k` bits (1 to 16) without consuming them.
    #[inline]
    fn peek(&self, k: u32) -> u32 {
        (self.acc >> (64 - k)) as u32
    }

    /// Consume `k` bits. R-ARW-75: fails when they lie beyond the data.
    #[inline]
    fn skip(&mut self, k: u32) -> ArwResult<()> {
        if k > self.n {
            return Err(fail("entropy data ended inside a code"));
        }
        self.acc <<= k;
        self.n -= k;
        Ok(())
    }

    /// DECODE (F.2.2.3): one Huffman symbol. R-ARW-74: a code that is not
    /// in the table fails.
    #[inline]
    fn symbol(&mut self, h: &Huffman) -> ArwResult<u8> {
        let e = h.fast[self.peek(FAST_BITS) as usize];
        if e != 0 {
            self.skip(u32::from(e >> 8))?;
            return Ok(e as u8);
        }
        for len in FAST_BITS as usize + 1..=16 {
            let code = self.peek(len as u32) as i32;
            if code <= h.maxcode[len] {
                if code < h.mincode[len] {
                    break;
                }
                let i = h.valptr[len] + (code - h.mincode[len]) as usize;
                self.skip(len as u32)?;
                return h
                    .values
                    .get(i)
                    .copied()
                    .ok_or_else(|| fail("HUFFVAL index"));
            }
        }
        Err(fail("Huffman code not in the table"))
    }

    /// One difference: DECODE, RECEIVE and EXTEND (F.2.2.1), with the
    /// lossless category 16 = 32768 (Table H.2).
    #[inline]
    fn diff(&mut self, h: &Huffman) -> ArwResult<i32> {
        self.fill();
        let s = u32::from(self.symbol(h)?);
        match s {
            0 => Ok(0),
            1..=15 => {
                let v = self.peek(s) as i32;
                self.skip(s)?;
                // EXTEND (Figure F.12).
                Ok(if v < (1 << (s - 1)) {
                    v - (1 << s) + 1
                } else {
                    v
                })
            }
            16 => Ok(32768),
            // R-ARW-74: SSSS greater than 16 fails.
            _ => Err(fail("difference category above 16")),
        }
    }

    /// End a restart interval (E.2.4): drop the remaining bits, find the
    /// next marker, require RSTm with the expected m (R-ARW-74, R-ARW-76).
    fn restart(&mut self, m: u8) -> ArwResult<()> {
        self.acc = 0;
        self.n = 0;
        // Locate the marker: skip data bytes and stuffed pairs.
        loop {
            match (self.d.get(self.pos), self.d.get(self.pos + 1)) {
                (Some(0xFF), Some(0)) => self.pos += 2,
                (Some(0xFF), Some(0xFF)) => self.pos += 1,
                (Some(0xFF), Some(&marker)) => {
                    if marker != 0xD0 + (m & 7) {
                        return Err(fail("expected a restart marker"));
                    }
                    self.pos += 2;
                    self.end = false;
                    return Ok(());
                }
                (Some(_), _) => self.pos += 1,
                (None, _) => return Err(fail("restart marker missing")),
            }
        }
    }
}

/// One frame component (T.81 B.2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::raw) struct Component {
    /// Component identifier Ci.
    pub id: u8,
    /// Horizontal sampling factor Hi.
    pub h: usize,
    /// Vertical sampling factor Vi.
    pub v: usize,
}

/// A parsed lossless JPEG stream, up to the start of its entropy data.
pub(in crate::raw) struct Stream<'a> {
    /// Sample precision P.
    pub precision: u32,
    /// Number of lines Y.
    pub lines: usize,
    /// Samples per line X.
    pub samples: usize,
    /// Frame components in frame order.
    pub components: Vec<Component>,
    /// Scan components in scan order: (frame index, Huffman table).
    pub scan: Vec<(usize, usize)>,
    /// Predictor selection value Ss.
    pub predictor: u8,
    /// End of spectral selection Se (T.81 B.2.3), read for callers that
    /// check it; not used by the decoding.
    pub se: u8,
    /// Successive approximation bit position high Ah (T.81 B.2.3), read
    /// for callers that check it; not used by the decoding.
    pub ah: u8,
    /// Point transform Pt.
    pub pt: u32,
    /// Restart interval Ri in MCUs, 0 when none.
    pub restart: usize,
    tables: [Option<Huffman>; 4],
    /// The entropy-coded data up to the end of the stream's byte range.
    data: &'a [u8],
}

/// Where the samples of one component go: sample (x, y) is written at
/// `buffers[buffer][base + x * xs + y * ys]`.
#[derive(Clone, Copy, Debug)]
pub(in crate::raw) struct Target {
    /// Index of the output buffer.
    pub buffer: usize,
    /// Index of sample (0, 0).
    pub base: usize,
    /// Step between columns.
    pub xs: usize,
    /// Step between lines.
    pub ys: usize,
}

fn be16(d: &[u8], at: usize) -> ArwResult<usize> {
    d.get(at..at + 2)
        .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
        .ok_or_else(|| fail("truncated marker segment"))
}

/// Parse markers from SOI to the end of the SOS header (R-ARW-72,
/// R-ARW-73). The stream must start with FF D8, and SOF3, DHT and SOS must
/// come before the entropy data.
pub(in crate::raw) fn parse(stream: &[u8]) -> ArwResult<Stream<'_>> {
    // R-ARW-72 case 1: the stream starts with FF D8.
    if stream.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err(fail("tile does not start with SOI"));
    }
    let mut tables: [Option<Huffman>; 4] = Default::default();
    let mut frame: Option<(u32, usize, usize, Vec<Component>)> = None;
    let mut restart = 0usize;
    let mut pos = 2usize;
    loop {
        // Fill bytes FF before a marker are allowed (T.81 B.1.1.2).
        if stream.get(pos) != Some(&0xFF) {
            return Err(fail("expected a marker"));
        }
        while stream.get(pos + 1) == Some(&0xFF) {
            pos += 1;
        }
        let marker = *stream.get(pos + 1).ok_or_else(|| fail("truncated tile"))?;
        // Standalone markers before the scan are not valid here.
        if matches!(marker, 0x01 | 0xD0..=0xD9) {
            return Err(fail("unexpected marker before the scan"));
        }
        let len = be16(stream, pos + 2)?;
        if len < 2 {
            return Err(fail("marker segment length"));
        }
        let body = stream
            .get(pos + 4..pos + 2 + len)
            .ok_or_else(|| fail("truncated marker segment"))?;
        match marker {
            // SOF3: lossless, Huffman coding (T.81 B.2.2).
            0xC3 => {
                if frame.is_some() {
                    return Err(fail("second frame header"));
                }
                let p = u32::from(*body.first().ok_or_else(|| fail("short SOF3"))?);
                let y = be16(body, 1)?;
                let x = be16(body, 3)?;
                let nf = usize::from(*body.get(5).ok_or_else(|| fail("short SOF3"))?);
                if !(2..=16).contains(&p) || x == 0 || y == 0 || !(1..=4).contains(&nf) {
                    return Err(fail("unsupported frame header"));
                }
                if body.len() < 6 + 3 * nf {
                    return Err(fail("short SOF3"));
                }
                let mut comps = Vec::with_capacity(nf);
                for i in 0..nf {
                    let c = &body[6 + 3 * i..9 + 3 * i];
                    let (h, v) = (usize::from(c[1] >> 4), usize::from(c[1] & 15));
                    if !(1..=4).contains(&h) || !(1..=4).contains(&v) {
                        return Err(fail("sampling factor outside 1 to 4"));
                    }
                    if comps.iter().any(|k: &Component| k.id == c[0]) {
                        return Err(fail("duplicate component id"));
                    }
                    comps.push(Component { id: c[0], h, v });
                }
                frame = Some((p, y, x, comps));
            }
            // DHT (T.81 B.2.4.2).
            0xC4 => {
                let mut p = 0usize;
                while p < body.len() {
                    let tc_th = body[p];
                    let (tc, th) = (tc_th >> 4, usize::from(tc_th & 15));
                    if tc != 0 || th > 3 {
                        return Err(fail("not a lossless Huffman table"));
                    }
                    let bits: [u8; 16] = body
                        .get(p + 1..p + 17)
                        .and_then(|b| b.try_into().ok())
                        .ok_or_else(|| fail("short DHT"))?;
                    let total: usize = bits.iter().map(|&b| usize::from(b)).sum();
                    // R-ARW-73: more than 256 codes, or more than the bytes
                    // left in the segment, fails.
                    if total > 256 || total > body.len() - (p + 17) {
                        return Err(fail("Huffman table counts too large"));
                    }
                    tables[th] = Some(Huffman::new(&bits, &body[p + 17..p + 17 + total])?);
                    p += 17 + total;
                }
            }
            // DRI (T.81 B.2.4.4).
            0xDD => restart = be16(body, 0)?,
            // SOS (T.81 B.2.3).
            0xDA => {
                // R-ARW-72 case 2: SOF3 and DHT before the entropy data.
                let (precision, lines, samples, components) =
                    frame.ok_or_else(|| fail("scan before the frame header"))?;
                if tables.iter().all(Option::is_none) {
                    return Err(fail("scan before any Huffman table"));
                }
                let ns = usize::from(*body.first().ok_or_else(|| fail("short SOS"))?);
                if ns == 0 || ns > 4 || body.len() < 1 + 2 * ns + 3 {
                    return Err(fail("bad scan header"));
                }
                let mut scan = Vec::with_capacity(ns);
                for j in 0..ns {
                    let cs = body[1 + 2 * j];
                    let td = usize::from(body[2 + 2 * j] >> 4);
                    let fi = components
                        .iter()
                        .position(|c| c.id == cs)
                        .ok_or_else(|| fail("scan names an unknown component"))?;
                    if scan.iter().any(|&(f, _)| f == fi) {
                        return Err(fail("scan names a component twice"));
                    }
                    // R-ARW-72 case 4: an undefined Huffman table fails.
                    if td > 3 || tables[td].is_none() {
                        return Err(fail("scan refers to an undefined Huffman table"));
                    }
                    scan.push((fi, td));
                }
                let predictor = body[1 + 2 * ns];
                let se = body[2 + 2 * ns];
                let ah = body[3 + 2 * ns] >> 4;
                let pt = u32::from(body[3 + 2 * ns] & 15);
                // R-ARW-72 case 5: predictor 1 to 7 and Pt < P.
                if !(1..=7).contains(&predictor) || pt >= precision {
                    return Err(fail("predictor or point transform out of range"));
                }
                // One interleaved scan with every frame component.
                if ns != components.len() {
                    return Err(fail("scan does not contain every component"));
                }
                return Ok(Stream {
                    precision,
                    lines,
                    samples,
                    components,
                    scan,
                    predictor,
                    se,
                    ah,
                    pt,
                    restart,
                    tables,
                    data: &stream[pos + 2 + len..],
                });
            }
            // Other frame types are not lossless Huffman.
            0xC0..=0xC2 | 0xC5..=0xC7 | 0xC8..=0xCF => {
                return Err(fail("not a lossless Huffman frame"));
            }
            // DNL is not supported (Y is never 0 here).
            0xDC => return Err(fail("DNL marker")),
            // APPn, COM, DQT and others: skipped.
            _ => {}
        }
        pos += 2 + len;
    }
}

impl Stream<'_> {
    /// The largest sampling factors.
    fn max_factors(&self) -> (usize, usize) {
        let hmax = self.components.iter().map(|c| c.h).max().unwrap_or(1);
        let vmax = self.components.iter().map(|c| c.v).max().unwrap_or(1);
        (hmax, vmax)
    }

    /// MCUs per MCU row and MCU rows (T.81 A.2, data unit = one sample).
    pub(super) fn mcus(&self) -> (usize, usize) {
        let (hmax, vmax) = self.max_factors();
        (self.samples.div_ceil(hmax), self.lines.div_ceil(vmax))
    }

    /// The coded columns and lines of frame component `i`: whole MCUs.
    pub(super) fn coded_size(&self, i: usize) -> (usize, usize) {
        let (mx, my) = self.mcus();
        let c = self.components[i];
        (mx * c.h, my * c.v)
    }

    /// Decode every sample into `buffers` through the per-component
    /// `targets` (indexed like the frame components). Fails when a target
    /// does not fit its buffer, on any entropy-coding fault, and when the
    /// data end before the last sample.
    pub(in crate::raw) fn decode(
        &self,
        buffers: &mut [&mut [u16]],
        targets: &[Target],
        line_start: LineStart,
    ) -> ArwResult<()> {
        self.decode_cancellable(buffers, targets, line_start, None)
    }

    /// [`Stream::decode`] that also stops with `Error::Cancelled` when
    /// `cancel` is cancelled, checked every few MCU rows. With `None` it is
    /// [`Stream::decode`].
    pub(in crate::raw) fn decode_cancellable(
        &self,
        buffers: &mut [&mut [u16]],
        targets: &[Target],
        line_start: LineStart,
        cancel: Option<&CancelToken>,
    ) -> ArwResult<()> {
        let nc = self.components.len();
        if targets.len() != nc {
            return Err(fail("one target per component"));
        }
        for (i, t) in targets.iter().enumerate() {
            let (cw, ch) = self.coded_size(i);
            let last = (cw - 1)
                .checked_mul(t.xs)
                .and_then(|a| (ch - 1).checked_mul(t.ys).and_then(|b| a.checked_add(b)))
                .and_then(|a| a.checked_add(t.base));
            let len = buffers.get(t.buffer).map_or(0, |b| b.len());
            if last.is_none_or(|l| l >= len) || t.xs == 0 || t.ys == 0 {
                return Err(fail("decoded component does not fit its buffer"));
            }
        }
        let (mcux, mcuy) = self.mcus();
        // T.81 H.1.1: the restart interval is a whole number of MCU rows.
        let rows_per_interval = if self.restart > 0 {
            if !self.restart.is_multiple_of(mcux) {
                return Err(fail("restart interval is not a whole number of MCU rows"));
            }
            self.restart / mcux
        } else {
            0
        };
        // T.81 H.1.2.1: the first prediction is 2^(P - Pt - 1).
        let initial = 1i32 << (self.precision - self.pt - 1);
        let predictor = self.predictor;
        let mut bits = Bits::new(self.data);
        let mut intervals = 0u8;
        // The first line of the current restart interval, per component.
        let mut first_line = vec![0usize; nc];
        for my in 0..mcuy {
            if let Some(c) = cancel {
                if my % 8 == 0 {
                    c.check()?;
                }
            }
            if rows_per_interval > 0 && my > 0 && my % rows_per_interval == 0 {
                bits.restart(intervals)?;
                intervals = intervals.wrapping_add(1);
                for (f, c) in first_line.iter_mut().zip(&self.components) {
                    *f = my * c.v;
                }
            }
            for mx in 0..mcux {
                // T.81 A.2.3: per scan component, H x V samples left to
                // right, top to bottom.
                for &(fi, ti) in &self.scan {
                    let comp = self.components[fi];
                    let t = targets[fi];
                    let table = self.tables[ti].as_ref().ok_or_else(|| fail("table"))?;
                    let out = &mut *buffers[t.buffer];
                    for dv in 0..comp.v {
                        let y = my * comp.v + dv;
                        for dh in 0..comp.h {
                            let x = mx * comp.h + dh;
                            let at = t.base + x * t.xs + y * t.ys;
                            let pred = if y == first_line[fi] {
                                // First line of the scan or of a restart
                                // interval: Ra, and 2^(P - Pt - 1) at x = 0.
                                if x == 0 {
                                    initial
                                } else {
                                    i32::from(out[at - t.xs])
                                }
                            } else if x == 0 {
                                // R-ARW-39 (Sony mode): a line y with
                                // y mod V = 0, y >= V of a component with
                                // V >= 2 predicts from (0, y - V).
                                if line_start == LineStart::Sony
                                    && comp.v >= 2
                                    && y.is_multiple_of(comp.v)
                                {
                                    i32::from(out[at - comp.v * t.ys])
                                } else {
                                    // T.81: the sample above (Rb).
                                    i32::from(out[at - t.ys])
                                }
                            } else {
                                let ra = i32::from(out[at - t.xs]);
                                let rb = i32::from(out[at - t.ys]);
                                let rc = i32::from(out[at - t.xs - t.ys]);
                                // T.81 Table H.1, shifts are arithmetic.
                                match predictor {
                                    1 => ra,
                                    2 => rb,
                                    3 => rc,
                                    4 => ra + rb - rc,
                                    5 => ra + ((rb - rc) >> 1),
                                    6 => rb + ((ra - rc) >> 1),
                                    _ => (ra + rb) >> 1,
                                }
                            };
                            let d = bits.diff(table)?;
                            // T.81 H.1.2.1: reconstruction modulo 2^16.
                            out[at] = ((pred + d) & 0xFFFF) as u16;
                        }
                    }
                }
            }
        }
        // T.81 H.2.2: multiply the output by 2^Pt, clipped to the storage
        // range (R-ARW-77).
        if self.pt > 0 {
            for (i, t) in targets.iter().enumerate() {
                let (cw, ch) = self.coded_size(i);
                let out = &mut *buffers[t.buffer];
                for y in 0..ch {
                    for x in 0..cw {
                        let at = t.base + x * t.xs + y * t.ys;
                        out[at] = (u32::from(out[at]) << self.pt).min(65535) as u16;
                    }
                }
            }
        }
        Ok(())
    }
}

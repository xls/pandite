//! Strip variants V1 (16-bit), V2 (12-bit packed) and V3 (128-bit blocks
//! with a tone curve), decoded row by row into a row-major plane.

use super::meta::{Layout, Plan, Variant};
use super::plane::Plane;
use super::{fail, pool, ArwResult};
use pandite_core::CancelToken;

/// Rows decoded per work item.
const BAND: usize = 64;

/// The V3 linearization table: L(c) for every 11-bit code value c
/// (R-ARW-35). Values are clipped to 0..65535 (R-ARW-77).
pub(super) fn tone_curve(t: [u32; 4]) -> Vec<u16> {
    let knots = [
        0u64,
        u64::from(t[0]),
        u64::from(t[1]),
        u64::from(t[2]),
        u64::from(t[3]),
    ];
    (0..2048u64)
        .map(|c| {
            // S(c) = sum over i of 2^(i + 1) * max(0, min(8c, T(i + 1)) - T(i)),
            // with T(0) = 0 and T(5) infinite.
            let x = 8 * c;
            let mut s = 0u64;
            for i in 0..5 {
                let lo = knots[i];
                let hi = if i == 4 { x } else { x.min(knots[i + 1]) };
                s += (1u64 << (i + 1)) * hi.saturating_sub(lo);
            }
            // L(c) = floor(S(c) / 8).
            (s / 8).min(65535) as u16
        })
        .collect()
}

/// The fields of one V3 block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Fields {
    /// Largest code value.
    pub max: u32,
    /// Smallest code value.
    pub min: u32,
    /// Pixel index of MAX.
    pub imax: usize,
    /// Pixel index of MIN.
    pub imin: usize,
    /// Shift SH.
    pub sh: u32,
    /// D(1) to D(14).
    pub deltas: [u32; 14],
}

/// R-ARW-31 and R-ARW-32: the fields of a block.
pub(super) fn fields(bytes: &[u8; 16]) -> Fields {
    // R-ARW-31: one little-endian 128-bit integer; field (p, n) is
    // floor(B / 2^p) mod 2^n.
    let b = u128::from_le_bytes(*bytes);
    let field = |p: u32, n: u32| ((b >> p) & ((1u128 << n) - 1)) as u32;
    let max = field(0, 11);
    let min = field(11, 11);
    // R-ARW-32: SPAN = MAX - MIN (0 when MAX < MIN) and the smallest shift
    // with floor(SPAN / 2^SH) < 128.
    let span = max.saturating_sub(min);
    let mut sh = 0;
    while sh < 4 && (span >> sh) >= 128 {
        sh += 1;
    }
    let mut deltas = [0u32; 14];
    for (k, d) in deltas.iter_mut().enumerate() {
        *d = field(30 + 7 * k as u32, 7);
    }
    Fields {
        max,
        min,
        imax: field(22, 4) as usize,
        imin: field(26, 4) as usize,
        sh,
        deltas,
    }
}

/// The 16 code values of one V3 block (R-ARW-31, R-ARW-32, R-ARW-33).
pub(super) fn block(bytes: &[u8; 16]) -> [u16; 16] {
    let f = fields(bytes);
    // R-ARW-33: assign in pixel index order with a delta counter k from 1.
    let mut out = [0u16; 16];
    let mut k = 0usize;
    for (i, o) in out.iter_mut().enumerate() {
        let code = if i == f.imax {
            f.max
        } else if i == f.imin {
            f.min
        } else if k < 14 {
            let d = f.deltas[k];
            k += 1;
            f.min + (d << f.sh)
        } else {
            f.min
        };
        // Step 5: a code value above 2047 becomes 2047.
        *o = code.min(2047) as u16;
    }
    out
}

/// Decode one V3 row: chunks of 32 bytes, the first block to the even
/// columns, the second to the odd columns (R-ARW-30), then the tone curve
/// (R-ARW-35).
fn v3_row(src: &[u8], out: &mut [u16], curve: &[u16]) {
    for (chunk, o) in src.chunks_exact(32).zip(out.chunks_exact_mut(32)) {
        let mut first = [0u8; 16];
        let mut second = [0u8; 16];
        first.copy_from_slice(&chunk[..16]);
        second.copy_from_slice(&chunk[16..]);
        let a = block(&first);
        let b = block(&second);
        for i in 0..16 {
            o[2 * i] = curve[usize::from(a[i])];
            o[2 * i + 1] = curve[usize::from(b[i])];
        }
    }
}

/// Decode one V2 row: pixel 2k = b0 + 256 * (b1 mod 16), pixel 2k + 1 =
/// floor(b1 / 16) + 16 * b2, both times 4 (R-ARW-28).
fn v2_row(src: &[u8], out: &mut [u16]) {
    for (g, o) in src.chunks_exact(3).zip(out.chunks_exact_mut(2)) {
        let (b0, b1, b2) = (u16::from(g[0]), u16::from(g[1]), u16::from(g[2]));
        o[0] = (b0 + 256 * (b1 % 16)) * 4;
        o[1] = (b1 / 16 + 16 * b2) * 4;
    }
}

/// Decode one V1 row: b0 + 256 * b1 per pixel (R-ARW-27, R-ARW-5).
pub(crate) fn v1_row(src: &[u8], out: &mut [u16]) {
    for (b, o) in src.chunks_exact(2).zip(out.iter_mut()) {
        *o = u16::from_le_bytes([b[0], b[1]]);
    }
}

/// Decode the strips of a V1, V2 or V3 file into a W x H plane.
pub(super) fn decode(data: &[u8], plan: &Plan, cancel: &CancelToken) -> ArwResult<Plane> {
    let Layout::Strips {
        offsets,
        rows_per_strip,
        row_bytes,
    } = &plan.layout
    else {
        return Err(fail("not a strip layout"));
    };
    let (w, h) = (plan.width, plan.height);
    let curve = tone_curve(plan.curve);
    let mut plane = Plane::row_major(w, h)?;
    let bands: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(w * BAND).enumerate().collect();
    pool::run(bands, cancel, |(band, rows)| {
        for (i, out) in rows.chunks_exact_mut(w).enumerate() {
            let y = band * BAND + i;
            // R-ARW-24: row y is row (y mod RowsPerStrip) of strip
            // floor(y / RowsPerStrip), with no padding between rows.
            let strip = y / rows_per_strip;
            let start = offsets.get(strip).ok_or_else(|| fail("strip index"))?
                + (y % rows_per_strip) * row_bytes;
            let src = data
                .get(start..start + row_bytes)
                .ok_or_else(|| fail("row outside the file"))?;
            match plan.variant {
                Variant::V1 => v1_row(src, out),
                Variant::V2 => v2_row(src, out),
                _ => v3_row(src, out, &curve),
            }
        }
        Ok(())
    })?;
    Ok(plane)
}

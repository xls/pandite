//! The sensor data of V1 to V8, decoded into a plane of the whole raster
//! (R-NEF-24) for the shared renderer.
//!
//! The renderer takes integer black levels per CFA position, and the
//! black level of R-NEF-37 is BlackLevel / 2^(14 - bps), a real number in
//! 12-bit files. The plane therefore holds every sensor value s times
//! 2^(14 - bps), with the tag values themselves as black levels and
//! (2^bps - 1) * 2^(14 - bps) as white level. The normalization of
//! R-NEF-39 is unchanged by the common factor: (k s - k b) / (k w - k b) =
//! (s - b) / (w - b).

use super::huffman::{self, Job, Table};
use super::meta::{Layout, Plan, Variant};
use super::{fail, NefResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use pandite_core::CancelToken;

/// Rows decoded per work item of the parallel variants.
const BAND: usize = 64;

/// The plane factor 2^(14 - bps) as a shift.
pub(super) fn shift(plan: &Plan) -> u32 {
    14 - plan.bits
}

/// R-NEF-25: V1 pixel `x` of a row: b0 * 256 + b1 in big-endian files,
/// b0 + 256 * b1 in little-endian files.
#[inline]
pub(super) fn v1_pixel(row: &[u8], x: usize, le: bool) -> Option<u16> {
    let b = row.get(2 * x..2 * x + 2)?;
    Some(if le {
        u16::from(b[0]) + 256 * u16::from(b[1])
    } else {
        u16::from(b[0]) * 256 + u16::from(b[1])
    })
}

/// R-NEF-26: pixel `x` of 3-byte groups b0, b1, b2 at offset 3 * floor(x /
/// 2): 16 * b0 + floor(b1 / 16) for even x, 256 * (b1 mod 16) + b2 for odd
/// x (V2, and the groups of V3).
#[inline]
pub(super) fn v2_pixel(row: &[u8], x: usize) -> Option<u16> {
    let k = 3 * (x / 2);
    let g = row.get(k..k + 3)?;
    let (b0, b1, b2) = (u16::from(g[0]), u16::from(g[1]), u16::from(g[2]));
    Some(if x.is_multiple_of(2) {
        16 * b0 + b1 / 16
    } else {
        256 * (b1 % 16) + b2
    })
}

/// R-NEF-45: V3 pixel `x`: group g = floor(x / 10) of 16 bytes holds pixels
/// 10g to 10g + 9 in its bytes 0 to 14 as five V2 groups; byte 15 is not
/// read.
#[inline]
pub(super) fn v3_pixel(row: &[u8], x: usize) -> Option<u16> {
    let g = x / 10;
    let group = row.get(16 * g..16 * g + 15)?;
    v2_pixel(group, x % 10)
}

/// R-NEF-28: V4 and V5 pixel `x` with `n` bits per pixel: bit k of the
/// pixel is bit n * x + k of the row bit sequence, whose bit 8i + j is bit
/// j of row byte i.
#[inline]
pub(super) fn packed_pixel(row: &[u8], x: usize, n: usize) -> Option<u16> {
    let first = n * x;
    let byte = first / 8;
    let shift = first % 8;
    // The pixel spans at most 3 bytes (shift + n <= 7 + 14 = 21 bits).
    let mut v = 0u32;
    for k in 0..3 {
        let b = if 8 * k < shift + n {
            u32::from(*row.get(byte + k)?)
        } else {
            0
        };
        v |= b << (8 * k);
    }
    Some(((v >> shift) & ((1u32 << n) - 1)) as u16)
}

/// The bytes of stored row `y` that the variant reads (R-NEF-22, R-NEF-27,
/// R-NEF-45, R-NEF-63): never beyond its strip.
fn row_bytes<'a>(data: &'a [u8], plan: &Plan, y: usize) -> NefResult<&'a [u8]> {
    let (start, len) = match &plan.layout {
        Layout::Strips {
            offsets,
            rows_per_strip,
            row_bytes,
        } => {
            // R-NEF-22: row y is row (y mod RowsPerStrip) of strip
            // floor(y / RowsPerStrip).
            let rps = (*rows_per_strip).max(1);
            let strip = *offsets.get(y / rps).ok_or_else(|| fail("strip index"))?;
            let start = strip
                .checked_add((y % rps) * row_bytes)
                .ok_or_else(|| fail("row offset"))?;
            (start, *row_bytes)
        }
        Layout::Stream {
            offset, row_bytes, ..
        } => {
            let start = y
                .checked_mul(*row_bytes)
                .and_then(|r| offset.checked_add(r))
                .ok_or_else(|| fail("row offset"))?;
            // R-NEF-27: only the first ceil(n * W / 8) bytes of a V4 or V5
            // row carry pixels.
            let used = match plan.variant {
                Variant::V4 => (12 * plan.width).div_ceil(8),
                Variant::V5 => (14 * plan.width).div_ceil(8),
                _ => *row_bytes,
            };
            (start, used.min(*row_bytes))
        }
    };
    start
        .checked_add(len)
        .and_then(|end| data.get(start..end))
        .ok_or_else(|| fail("row outside the file"))
}

/// Decode the whole raster into a row-major plane of values s * 2^(14 -
/// bps). Returns the plane and, for V6 to V8, the bits the stream used and
/// the bits it holds.
pub(super) fn decode(
    data: &[u8],
    plan: &Plan,
    cancel: &CancelToken,
) -> NefResult<(Plane, Option<(u64, u64)>)> {
    if plan.variant.huffman() {
        return decode_huffman(data, plan, cancel).map(|(p, used, total)| (p, Some((used, total))));
    }
    let (w, h) = (plan.width, plan.height);
    let k = shift(plan);
    let max = plan.white as u16;
    let mut plane = Plane::row_major(w, h)?;
    let bands: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(w * BAND).enumerate().collect();
    pool::run(bands, cancel, |(band, rows)| {
        for (i, out) in rows.chunks_exact_mut(w).enumerate() {
            let y = band * BAND + i;
            let row = row_bytes(data, plan, y)?;
            for (x, o) in out.iter_mut().enumerate() {
                let s = match plan.variant {
                    // R-NEF-25: values above 2^bps - 1 are clipped.
                    Variant::V1 => v1_pixel(row, x, plan.le).map(|v| v.min(max)),
                    Variant::V2 => v2_pixel(row, x),
                    // R-NEF-45: stored pixels with x >= W are never read.
                    Variant::V3 => v3_pixel(row, x),
                    Variant::V4 => packed_pixel(row, x, 12),
                    _ => packed_pixel(row, x, 14),
                }
                .ok_or_else(|| fail("pixel outside its row"))?;
                *o = s << k;
            }
        }
        Ok(())
    })?;
    Ok((plane, None))
}

/// V6 to V8: one bit stream for the raster (R-NEF-29 to R-NEF-33), with
/// the curve lookup of R-NEF-46 for V7 and V8. Returns the plane, the bits
/// used and the bits of the stream.
fn decode_huffman(data: &[u8], plan: &Plan, cancel: &CancelToken) -> NefResult<(Plane, u64, u64)> {
    let coding = plan.coding.as_ref().ok_or_else(|| fail("no coding"))?;
    let Layout::Stream { offset, len, .. } = plan.layout else {
        return Err(fail("the Huffman variants have one stream"));
    };
    let stream = offset
        .checked_add(len)
        .and_then(|end| data.get(offset..end))
        .ok_or_else(|| fail("strip outside the file"))?;
    let table = Table::new(coding.table);
    let (w, h) = (plan.width, plan.height);
    // Every pixel consumes at least `min_bits` bits, and R-NEF-64 fails a
    // decode that reads past the strip: a strip too short for W * H pixels
    // fails here, before the plane is allocated.
    let needed = (w as u64)
        .saturating_mul(h as u64)
        .saturating_mul(u64::from(table.min_bits));
    let total = (stream.len() as u64).saturating_mul(8);
    if needed > total {
        return Err(fail("the strip is too short for the raster"));
    }
    let k = shift(plan);
    let mut plane = Plane::row_major(w, h)?;
    let job = Job {
        stream,
        width: w,
        height: h,
        table: &table,
        predictors: coding.predictors,
        limit: coding.limit(plan.bits),
    };
    let curve = coding.curve.as_deref();
    let used = huffman::decode(&job, cancel, |y, codes| {
        let out = plane
            .data
            .get_mut(y * w..(y + 1) * w)
            .ok_or_else(|| fail("row outside the plane"))?;
        for (o, &c) in out.iter_mut().zip(codes) {
            // R-NEF-33 checked 0 <= c < limit, so c indexes the curve.
            let s = match curve {
                // R-NEF-46: the sensor value is C(code value).
                Some(curve) => curve.get(c as usize).copied().unwrap_or(0),
                // V6: the code value is the sensor value.
                None => c as u16,
            };
            *o = s << k;
        }
        Ok(())
    })?;
    Ok((plane, used, total))
}

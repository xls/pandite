//! The sensor data of V1 to V8: the chunk rotation (R-RW2-21), the packed
//! values (R-RW2-23), the block code (R-RW2-25 to R-RW2-29) and the 16-bit
//! containers (R-RW2-24), decoded row by row into a plane of the active
//! area.

use super::meta::{Coding, Plan, Variant, CHUNK};
use super::{fail, Rw2Result};
use crate::pool;
use crate::raw::arw::plane::Plane;
use pandite_core::CancelToken;

/// Rows decoded per work item.
const BAND: usize = 64;
/// The chunk rotation: logical byte i of a chunk is stored at chunk offset
/// (i + 8184) mod 16384 (R-RW2-21).
pub(super) const SPLIT: u64 = 8184;

/// R-RW2-21: the file offset of logical byte `l` of a V1, V2 or V3 stream
/// that starts at `start`: S + 16384 * floor(L / 16384) + ((L mod 16384 +
/// 8184) mod 16384). 64-bit arithmetic (R-RW2-52); `None` on overflow.
pub(super) fn logical_offset(start: u64, l: u64) -> Option<u64> {
    start
        .checked_add(CHUNK.checked_mul(l / CHUNK)?)?
        .checked_add((l % CHUNK + SPLIT) % CHUNK)
}

/// The bytes `at..at + len` of the file, `None` when they do not lie
/// completely inside it (R-RW2-55).
fn slice(data: &[u8], at: u64, len: usize) -> Option<&[u8]> {
    let at = usize::try_from(at).ok()?;
    data.get(at..at.checked_add(len)?)
}

/// The 16 bytes of block `n`: logical bytes 16 * n to 16 * n + 15 of the
/// stream that starts at `start`, through the chunk rotation when
/// `rotated` (V1, V2, V3). `None` when a byte lies outside the file.
pub(super) fn block_bytes(data: &[u8], start: u64, n: u64, rotated: bool) -> Option<[u8; 16]> {
    let l = n.checked_mul(16)?;
    let mut out = [0u8; 16];
    if !rotated {
        out.copy_from_slice(slice(data, start.checked_add(l)?, 16)?);
        return Some(out);
    }
    // A block never crosses a chunk end (16384 is a multiple of 16), but
    // the block at logical chunk offset 8192 wraps from stored offset 16383
    // to 0 inside its chunk.
    let stored = (l % CHUNK + SPLIT) % CHUNK;
    if stored + 16 <= CHUNK {
        out.copy_from_slice(slice(data, logical_offset(start, l)?, 16)?);
    } else {
        for (k, o) in out.iter_mut().enumerate() {
            *o = *slice(data, logical_offset(start, l + k as u64)?, 1)?.first()?;
        }
    }
    Some(out)
}

/// R-RW2-23: value j of a packed block is floor(B / 2^(D * j)) mod 2^D,
/// with B the little-endian 128-bit integer of the block.
pub(super) fn packed(bytes: &[u8; 16], depth: u32, out: &mut [u16]) {
    let b = u128::from_le_bytes(*bytes);
    let mask = (1u128 << depth) - 1;
    for (j, o) in out.iter_mut().enumerate() {
        let shifted = b.checked_shr(depth * j as u32).unwrap_or(0);
        *o = (shifted & mask) as u16;
    }
}

/// The p pixels of one block-code block (V1, V4, V5), from the block alone
/// (R-RW2-29).
pub(super) fn block_code(bytes: &[u8; 16], variant: Variant, out: &mut [u16]) {
    // R-RW2-25: fields of the little-endian 128-bit integer B; the field
    // with start bit s and width n is floor(B / 2^s) mod 2^n.
    let b = u128::from_le_bytes(*bytes);
    let field = |s: u32, n: u32| ((b >> s) & ((1u128 << n) - 1)) as u32;
    // (D, N, zones, pixel 0 start, pixel 1 start, class of zone 0, zone
    // stride, code 0 of zone 0) from the bit layout table.
    let (d, n, zones, p0, p1, class0, stride, code0) = if variant == Variant::V5 {
        (14, 10, 3, 114, 100, 98, 32, 88)
    } else {
        (12, 8, 4, 116, 104, 102, 26, 94)
    };
    let full = (1u32 << d) - 1;
    let q = 1u32 << (n - 1);
    // R-RW2-26: pixels 0 and 1 are the initial values.
    out[0] = field(p0, d) as u16;
    out[1] = field(p1, d) as u16;
    for z in 0..zones {
        let class = field(class0 - stride * z, 2);
        for j in 0..3 {
            let i = (2 + 3 * z + j) as usize;
            if i >= out.len() {
                return;
            }
            let k = field(code0 - stride * z - n * j, n);
            // R-RW2-27: in increasing order, with P the limited value of
            // pixel i - 2 of the same block.
            let p = u32::from(out[i - 2]);
            // R-RW2-59: 32-bit arithmetic; the largest value before the
            // limit is F + (2^N - 1 - Q) * 4.
            let v = if class == 3 {
                // Class 3: k * 2^(D - N).
                k << (d - n)
            } else {
                // Classes 0 to 2: step s = 2^c, max(P - Q * s, 0) + k * s.
                let s = 1u32 << class;
                p.saturating_sub(q * s) + k * s
            };
            // R-RW2-28: limited to F before it is a pixel or a predecessor.
            out[i] = v.min(full) as u16;
        }
    }
}

/// Decode stored row `y` of the raster into `row` (W values). Every block
/// of the row is decoded by the normal rules, also all-FF blocks and the
/// blocks outside the active area (R-RW2-60).
pub(super) fn decode_row(data: &[u8], plan: &Plan, y: usize, row: &mut [u16]) -> Rw2Result<()> {
    let w = plan.width;
    let outside = || fail("sensor data outside the file");
    match plan.variant.coding() {
        Coding::Containers => {
            // R-RW2-24: pixel (x, y) is the 16-bit little-endian integer at
            // 2 * (y * W + x) from the data start, divided by 16 (R-RW2-6).
            let at = plan.start + 2 * (y as u64) * (w as u64);
            let src = slice(data, at, 2 * w).ok_or_else(outside)?;
            for (o, b) in row.iter_mut().zip(src.chunks_exact(2)) {
                *o = u16::from_le_bytes([b[0], b[1]]) / 16;
            }
        }
        coding => {
            // Block n holds raster indices n * p to n * p + p - 1, so row y
            // is the blocks y * W / p to (y + 1) * W / p - 1.
            let p = plan.variant.per_block();
            let first = (y as u64) * (w / p) as u64;
            let rotated = plan.variant.rotated();
            for (bx, out) in row.chunks_exact_mut(p).enumerate() {
                let bytes = block_bytes(data, plan.start, first + bx as u64, rotated)
                    .ok_or_else(outside)?;
                if coding == Coding::BlockCode {
                    block_code(&bytes, plan.variant, out);
                } else {
                    packed(&bytes, plan.variant.depth(), out);
                }
            }
        }
    }
    Ok(())
}

/// Decode the sensor data of the active area into a plane of
/// (right - left) x (bottom - top) values. R-RW2-56: the plane is allocated
/// only after the plan checked the size limits (R-RW2-53) and the data
/// range (R-RW2-55), with 16-bit values. Each stored row of the active area
/// is decoded whole (R-RW2-60); its active columns are kept.
pub(super) fn decode(data: &[u8], plan: &Plan, cancel: &CancelToken) -> Rw2Result<Plane> {
    let (aw, ah) = plan.active.size();
    let (top, left, right) = (plan.active.top, plan.active.left, plan.active.right);
    if aw == 0 || ah == 0 || right > plan.width || plan.active.bottom > plan.height {
        return Err(fail("active area outside the raster"));
    }
    let mut plane = Plane::row_major(aw, ah)?;
    let bands: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(aw * BAND).enumerate().collect();
    pool::run(bands, cancel, |(band, rows)| {
        let mut row = Vec::new();
        row.try_reserve_exact(plan.width)
            .map_err(|_| fail("not enough memory for a row"))?;
        row.resize(plan.width, 0u16);
        for (i, out) in rows.chunks_exact_mut(aw).enumerate() {
            let y = top + band * BAND + i;
            decode_row(data, plan, y, &mut row)?;
            out.copy_from_slice(&row[left..right]);
        }
        Ok(())
    })?;
    Ok(plane)
}

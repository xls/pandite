//! The strip of variants V1 (12-bit packed), V2 (14-bit packed) and V3
//! (16-bit containers), decoded row by row into a row-major plane (section
//! Compression).

use super::container::{Plan, Variant};
use super::{fail, RafResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use crate::raw::arw::strips;
use pandite_core::CancelToken;

/// Rows decoded per work item.
const BAND: usize = 64;

/// Decode one V1 row: R-RAF-23, pixel 2k = b0 + 256 * (b1 mod 16), pixel
/// 2k + 1 = floor(b1 / 16) + 16 * b2, on the 12-bit scale.
pub(super) fn v1_row(src: &[u8], out: &mut [u16]) {
    for (g, o) in src.chunks_exact(3).zip(out.chunks_exact_mut(2)) {
        let (b0, b1, b2) = (u16::from(g[0]), u16::from(g[1]), u16::from(g[2]));
        o[0] = b0 + 256 * (b1 % 16);
        o[1] = b1 / 16 + 16 * b2;
    }
}

/// Decode one V2 row: R-RAF-25, the LE32 words of the row written from bit
/// 31 down to bit 0 form one bit sequence, and pixel x is its bits 14x to
/// 14x + 13, the first the most significant. Seven words hold 16 pixels.
pub(super) fn v2_row(src: &[u8], out: &mut [u16]) {
    for (group, o) in src.chunks_exact(28).zip(out.chunks_exact_mut(16)) {
        // Bits not yet taken, right-aligned in `acc`: at most 13 between
        // words, so 45 after the next word.
        let (mut acc, mut n, mut k) = (0u64, 0u32, 0usize);
        for word in group.chunks_exact(4) {
            let w = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
            acc = (acc << 32) | u64::from(w);
            n += 32;
            while n >= 14 {
                n -= 14;
                o[k] = ((acc >> n) & 0x3FFF) as u16;
                k += 1;
            }
            acc &= (1u64 << n) - 1;
        }
    }
}

/// Decode the strip into a W x H plane. R-RAF-21: row y starts at
/// D + y * R; R-RAF-59: exactly R * H = StripByteCounts bytes are read.
pub(super) fn decode(data: &[u8], plan: &Plan, cancel: &CancelToken) -> RafResult<Plane> {
    let (w, h, r) = (plan.width, plan.height, plan.row_bytes);
    // R-RAF-55 was checked by the plan; a zero size never reaches here.
    if w == 0 || h == 0 {
        return Err(fail("empty raster"));
    }
    // R-RAF-58: the whole strip lies inside the file before the plane is
    // allocated (its size is bounded by the strip's).
    let end = r
        .checked_mul(h)
        .and_then(|n| n.checked_add(plan.strip))
        .ok_or_else(|| fail("strip size"))?;
    if end > data.len() {
        return Err(fail("strip outside the file"));
    }
    let mut plane = Plane::row_major(w, h)?;
    let bands: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(w * BAND).enumerate().collect();
    pool::run(bands, cancel, |(band, rows)| {
        for (i, out) in rows.chunks_exact_mut(w).enumerate() {
            let y = band * BAND + i;
            let start = plan.strip + y * r;
            let src = data
                .get(start..start + r)
                .ok_or_else(|| fail("row outside the file"))?;
            match plan.variant {
                Variant::V1 => v1_row(src, out),
                Variant::V2 => v2_row(src, out),
                // R-RAF-27: b0 + 256 * b1, the 16-bit row of the Sony
                // decoder.
                Variant::V3 => strips::v1_row(src, out),
            }
        }
        Ok(())
    })?;
    Ok(plane)
}

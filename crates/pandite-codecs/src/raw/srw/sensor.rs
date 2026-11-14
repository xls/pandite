//! The sensor data of V1 (16-bit samples) and V2 (12-bit packed), decoded
//! row by row into a plane that holds the valid area only.

use super::meta::{Plan, Variant};
use super::{fail, SrwResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use pandite_core::CancelToken;

/// Rows decoded per work item.
const BAND: usize = 64;

/// R-SRW-26: V1 pixel `x` of a row: b0 + 256 * b1 when the raw data byte
/// order is little-endian, 256 * b0 + b1 when it is big-endian.
#[inline]
pub(super) fn v1_pixel(row: &[u8], x: usize, le: bool) -> Option<u16> {
    let b = row.get(2 * x..2 * x + 2)?;
    Some(if le {
        u16::from_le_bytes([b[0], b[1]])
    } else {
        u16::from_be_bytes([b[0], b[1]])
    })
}

/// V2 pixel `x` of a row: pixels 2k and 2k + 1 come from the bytes b0, b1,
/// b2 at row offset 3k.
#[inline]
pub(super) fn v2_pixel(row: &[u8], x: usize, le: bool) -> Option<u16> {
    let k = 3 * (x / 2);
    let g = row.get(k..k + 3)?;
    let (b0, b1, b2) = (u16::from(g[0]), u16::from(g[1]), u16::from(g[2]));
    let even = x.is_multiple_of(2);
    Some(match (le, even) {
        // R-SRW-27: little-endian groups.
        (true, true) => b0 + 256 * (b1 % 16),
        (true, false) => b1 / 16 + 16 * b2,
        // R-SRW-28: big-endian groups.
        (false, true) => 16 * b0 + b1 / 16,
        (false, false) => 256 * (b1 % 16) + b2,
    })
}

/// How a stored value becomes a value of the plane that the shared
/// renderer normalizes with integer black levels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Domain {
    /// The value as read (R-SRW-29).
    AsRead,
    /// Twice the value, so that a black level of half a unit (bG of
    /// R-SRW-32) becomes an integer. A doubled value above 65535 is stored
    /// as 65535; `exact[c]` is true when that still normalizes to the same
    /// clipped value for color c, and a pixel of a color where it does not
    /// fails the decode.
    Doubled {
        /// Per color (0 red, 1 green, 2 blue).
        exact: [bool; 3],
    },
}

/// Decode the valid area (R-SRW-41) into a row-major plane whose pixel
/// (0, 0) is stored pixel (VX0, VY0). The values are used as read, without
/// a shift or a scale (R-SRW-29), in the given domain.
pub(super) fn decode(
    data: &[u8],
    plan: &Plan,
    domain: Domain,
    cancel: &CancelToken,
) -> SrwResult<Plane> {
    let v = plan.valid;
    let (le, rps, row_bytes) = (plan.data_le, plan.rows_per_strip, plan.row_bytes);
    if v.x + v.w > plan.width || v.y + v.h > plan.height || rps == 0 {
        return Err(fail("valid area outside the raster"));
    }
    let mut plane = Plane::row_major(v.w, v.h)?;
    let bands: Vec<(usize, &mut [u16])> = plane.data.chunks_mut(v.w * BAND).enumerate().collect();
    pool::run(bands, cancel, |(band, rows)| {
        for (i, out) in rows.chunks_exact_mut(v.w).enumerate() {
            let y = v.y + band * BAND + i;
            // R-SRW-24: row y is row (y mod RowsPerStrip) of strip
            // floor(y / RowsPerStrip), R bytes per row, no padding.
            let start = plan
                .strips
                .get(y / rps)
                .and_then(|&s| s.checked_add((y % rps) * row_bytes))
                .ok_or_else(|| fail("strip index"))?;
            let row = start
                .checked_add(row_bytes)
                .and_then(|end| data.get(start..end))
                .ok_or_else(|| fail("row outside the file"))?;
            for (k, o) in out.iter_mut().enumerate() {
                let x = v.x + k;
                let s = match plan.variant {
                    Variant::V1 => v1_pixel(row, x, le),
                    Variant::V2 => v2_pixel(row, x, le),
                }
                .ok_or_else(|| fail("pixel outside its row"))?;
                *o = match domain {
                    Domain::AsRead => s,
                    Domain::Doubled { exact } => {
                        let d = 2 * u32::from(s);
                        if d > 65535 {
                            // R-SRW-23: the color of stored pixel (x, y).
                            let c = usize::from(plan.cfa[2 * (y % 2) + x % 2]).min(2);
                            if !exact[c] {
                                return Err(fail("sample outside the doubled range"));
                            }
                            65535
                        } else {
                            d as u16
                        }
                    }
                };
            }
        }
        Ok(())
    })?;
    Ok(plane)
}

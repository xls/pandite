//! The sensor data of the four variants (sections Sensor data layout and
//! Compression), decoded into a plane that holds the crop rectangle only.

use super::huffman;
use super::meta::{Plan, Variant};
use super::{fail, PefResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use pandite_core::CancelToken;

/// Rows decoded per work item of P1 and P2.
const BAND: usize = 64;

/// The factor between the stored sensor value and the plane value. The
/// CFA variants hold twice the sensor value: the green black level of
/// R-PEF-38, (B1 + B2) / 2, is then a whole number for the shared
/// normalization, which takes whole black levels. P4 holds the values.
pub(super) fn scale(variant: Variant) -> u16 {
    if variant == Variant::P4 {
        1
    } else {
        2
    }
}

/// R-PEF-25: P1 pixels `x0..x0 + out.len()` of one row, from the 2 * W
/// bytes of the row, clipped to 0 to 4095 (R-PEF-66, BPS 12).
fn p1_row(src: &[u8], x0: usize, le: bool, maxv: u16, k: u16, out: &mut [u16]) {
    let src = src.get(2 * x0..).unwrap_or(&[]);
    for (o, b) in out.iter_mut().zip(src.chunks_exact(2)) {
        let v = if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        };
        *o = v.min(maxv) * k;
    }
}

/// R-PEF-26: P2 pixels `x0..x0 + out.len()` of one row, from the 3 * W / 2
/// bytes of the row. Pixel 2j is 16 * b0 + floor(b1 / 16), pixel 2j + 1 is
/// 256 * (b1 mod 16) + b2, with b0, b1, b2 at row offset 3j.
fn p2_row(src: &[u8], x0: usize, k: u16, out: &mut [u16]) {
    for (i, o) in out.iter_mut().enumerate() {
        let x = x0 + i;
        let g = 3 * (x / 2);
        let Some(&[b0, b1, b2]) = src.get(g..g + 3) else {
            return;
        };
        let (b0, b1, b2) = (u16::from(b0), u16::from(b1), u16::from(b2));
        let v = if x.is_multiple_of(2) {
            16 * b0 + b1 / 16
        } else {
            256 * (b1 % 16) + b2
        };
        *o = v * k;
    }
}

/// Decode the sensor data of a plan into a plane of the crop rectangle,
/// CW x CH values in the units of [`scale`].
pub(super) fn decode(data: &[u8], plan: &Plan, cancel: &CancelToken) -> PefResult<Plane> {
    let (w, h) = (plan.width, plan.height);
    let crop = plan.crop;
    if crop.w == 0 || crop.h == 0 || crop.x + crop.w > w || crop.y + crop.h > h {
        return Err(fail("crop outside the raster"));
    }
    let k = scale(plan.variant);
    let maxv = ((1u32 << plan.bps) - 1) as u16;
    let (off, len) = plan.strip;
    let strip = data
        .get(off..off.checked_add(len).ok_or_else(|| fail("strip size"))?)
        .ok_or_else(|| fail("the strip lies outside the file"))?;
    let mut plane = Plane::row_major(crop.w, crop.h)?;
    match plan.variant {
        Variant::P1 | Variant::P2 => {
            // R-PEF-24: row y at 2 * W * y (P1) or 3 * W * y / 2 (P2), no
            // padding between rows. R-PEF-63: never more than the 2 * W * H
            // or 3 * W * H / 2 bytes of the raster are read.
            let row_bytes = if plan.variant == Variant::P1 {
                2 * w
            } else {
                3 * w / 2
            };
            let bands: Vec<(usize, &mut [u16])> =
                plane.data.chunks_mut(crop.w * BAND).enumerate().collect();
            pool::run(bands, cancel, |(band, rows)| {
                for (i, out) in rows.chunks_exact_mut(crop.w).enumerate() {
                    let y = crop.y + band * BAND + i;
                    let start = row_bytes * y;
                    let src = strip
                        .get(start..start + row_bytes)
                        .ok_or_else(|| fail("row outside the strip"))?;
                    if plan.variant == Variant::P1 {
                        p1_row(src, crop.x, plan.le, maxv, k, out);
                    } else {
                        p2_row(src, crop.x, k, out);
                    }
                }
                Ok(())
            })?;
        }
        Variant::P3 | Variant::P4 => {
            // R-PEF-36: P4 uses the coding of P3 unchanged.
            let table = plan
                .table
                .as_ref()
                .ok_or_else(|| fail("no Huffman table"))?;
            let rows = crop.y..crop.y + crop.h;
            let cols = crop.x..crop.x + crop.w;
            // The whole raster is decoded (every pixel predicts later ones
            // and every code must fit the strip); the crop is kept.
            let data = &mut plane.data;
            huffman::decode(strip, table, (w, h), plan.bps, cancel, |x, y, v| {
                if rows.contains(&y) && cols.contains(&x) {
                    if let Some(d) = data.get_mut((y - crop.y) * crop.w + (x - crop.x)) {
                        *d = v * k;
                    }
                }
            })?;
        }
    }
    Ok(plane)
}

//! The sensor values s(x, y) of every variant (section Compression),
//! decoded into a row-major plane of the whole raster W x H.

use super::lossless;
use super::meta::{Layout, Plan, Variant};
use super::{fail, OrfResult};
use crate::pool;
use crate::raw::arw::plane::Plane;
use pandite_core::CancelToken;

/// Rows decoded per work item (V1 to V4).
const BAND: usize = 64;

/// R-ORF-21: the image row of stored row `s` of a V3 file of `h` rows:
/// 2 * s for s < ceil(H / 2), 2 * (s - ceil(H / 2)) + 1 otherwise. The
/// decoder uses its inverse, [`stored_row`].
#[cfg(test)]
pub(super) fn image_row(s: usize, h: usize) -> usize {
    let he = h.div_ceil(2);
    if s < he {
        2 * s
    } else {
        2 * (s - he) + 1
    }
}

/// The stored row that R-ORF-21 places at image row `y` (its inverse).
pub(super) fn stored_row(y: usize, h: usize) -> usize {
    if y.is_multiple_of(2) {
        y / 2
    } else {
        h.div_ceil(2) + y / 2
    }
}

/// The byte at position `i` of the V4 bit string of a row: the bytes of
/// each little-endian 32-bit word taken from its most significant byte
/// (R-ORF-25).
#[inline]
fn word_byte(row: &[u8], i: usize) -> Option<u8> {
    row.get((i & !3) | (3 - (i & 3))).copied()
}

/// The values of one stored row of V1 to V4 into `out` (W values). `le` is
/// the file byte order (V1) and `bits` the bit depth B (V1). `None` when a
/// pixel lies outside the row.
pub(super) fn unpack_row(
    variant: Variant,
    row: &[u8],
    le: bool,
    bits: u32,
    out: &mut [u16],
) -> Option<()> {
    match variant {
        Variant::V1 => {
            // R-ORF-22: b0 + 256 * b1 or 256 * b0 + b1, then the B high
            // bits: floor(sample / 2^(16 - B)).
            let shift = 16u32.checked_sub(bits)?;
            for (x, o) in out.iter_mut().enumerate() {
                let b = row.get(2 * x..2 * x + 2)?;
                let sample = if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                };
                *o = sample >> shift;
            }
        }
        Variant::V2 => {
            // R-ORF-23: group g of 16 bytes holds pixels 10g to 10g + 9,
            // pair j at group bytes 3j to 3j + 2; byte 15 is ignored.
            for (x, o) in out.iter_mut().enumerate() {
                let at = 16 * (x / 10) + 3 * ((x % 10) / 2);
                let c = row.get(at..at + 3)?;
                let (c0, c1, c2) = (u16::from(c[0]), u16::from(c[1]), u16::from(c[2]));
                *o = if x % 2 == 0 {
                    c0 + 256 * (c1 % 16)
                } else {
                    c1 / 16 + 16 * c2
                };
            }
        }
        Variant::V3 => {
            // R-ORF-24: pixels 2k and 2k + 1 from the bytes at 3k.
            for (x, o) in out.iter_mut().enumerate() {
                let at = 3 * (x / 2);
                let c = row.get(at..at + 3)?;
                let (c0, c1, c2) = (u16::from(c[0]), u16::from(c[1]), u16::from(c[2]));
                *o = if x % 2 == 0 {
                    16 * c0 + c1 / 16
                } else {
                    256 * (c1 % 16) + c2
                };
            }
        }
        Variant::V4 => {
            // R-ORF-25: consecutive 12-bit fields, MSB first, of the bit
            // string of the row's little-endian words, each word from its
            // most significant bit. Pixel x starts at bit 12x: on a byte
            // boundary for even x, in the middle of a byte for odd x.
            for (x, o) in out.iter_mut().enumerate() {
                let at = 3 * (x / 2);
                let (b0, b1, b2) = (
                    u16::from(word_byte(row, at)?),
                    u16::from(word_byte(row, at + 1)?),
                    u16::from(word_byte(row, at + 2)?),
                );
                *o = if x % 2 == 0 {
                    16 * b0 + b1 / 16
                } else {
                    256 * (b1 % 16) + b2
                };
            }
        }
        Variant::V5 | Variant::V6 => return None,
    }
    Some(())
}

/// Decode the sensor values of the whole raster into a row-major plane
/// (W x H), the values as decoded. For V5 and V6 also the number of strip
/// bits after the last code (R-ORF-26: ignored).
pub(super) fn decode(
    data: &[u8],
    plan: &Plan,
    cancel: &CancelToken,
) -> OrfResult<(Plane, Option<u64>)> {
    let (w, h) = (plan.width, plan.height);
    let mut plane = Plane::row_major(w, h)?;
    let mut unused = None;
    match &plan.layout {
        Layout::Stream { start, end } => {
            // R-ORF-26, R-ORF-60: the code bits up to the end of the strip
            // and not beyond.
            let stream = data
                .get(*start..*end)
                .ok_or_else(|| fail("strip outside the file"))?;
            let read = lossless::decode(stream, w, h, plan.bits, &mut plane.data, cancel)?;
            unused = Some(stream.len() as u64 * 8 - read);
        }
        Layout::Rows { starts, row_bytes } => {
            let rb = *row_bytes;
            let bands: Vec<(usize, &mut [u16])> =
                plane.data.chunks_mut(w * BAND).enumerate().collect();
            pool::run(bands, cancel, |(band, rows)| {
                for (i, out) in rows.chunks_exact_mut(w).enumerate() {
                    let y = band * BAND + i;
                    // R-ORF-21: V3 image row y comes from the stored row
                    // that the field placement puts there.
                    let s = match plan.variant {
                        Variant::V3 => stored_row(y, h),
                        _ => y,
                    };
                    // R-ORF-20, R-ORF-59: RB bytes of each of the H rows.
                    let row = starts
                        .get(s)
                        .and_then(|&at| data.get(at..at.checked_add(rb)?))
                        .ok_or_else(|| fail("row outside the file"))?;
                    unpack_row(plan.variant, row, plan.le, plan.bits, out)
                        .ok_or_else(|| fail("pixel outside its row"))?;
                }
                Ok(())
            })?;
        }
    }
    Ok((plane, unused))
}

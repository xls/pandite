//! From the decoded crop plane to the final image.
//!
//! P1 to P3 go through the shared renderer of the Sony decoder, which does
//! the normalization and white balance (R-PEF-41), the bilinear
//! demosaicing (R-PEF-48), the color matrix (R-PEF-45) and the orientation
//! (R-PEF-51). The plane holds the crop rectangle only, so the renderer's
//! valid area is the crop rectangle and neighbors outside it never enter
//! the demosaicing (R-PEF-49). P4 has no CFA and no matrix: each pixel is
//! normalized and delivered as gray (R-PEF-42, R-PEF-47).

use super::meta::{Plan, Variant};
use super::{fail, PefResult};
use crate::pool;
use crate::raw::arw::meta::{Layout, Plan as SharedPlan, Ycc};
use crate::raw::arw::plane::Plane;
use crate::raw::arw::render::{self, RowEncoder, Sensor};
use crate::raw::arw::{Rect, Variant as SharedVariant};
use pandite_core::CancelToken;

/// The identity matrix: P4 values reach the encoder unchanged (R-PEF-47).
const IDENTITY: [[f32; 3]; 3] = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

/// The output size for the crop and orientation of a plan (R-PEF-51).
pub(super) fn output_size(plan: &Plan) -> (usize, usize) {
    render::output_size(plan.crop, plan.orientation)
}

/// The plan of the shared renderer for the crop plane of a CFA variant.
/// The plane values are in the units of [`super::sensor::scale`] (twice the
/// sensor value), so the white and black levels are doubled as well.
fn shared_plan(p: &Plan) -> SharedPlan {
    let crop = p.crop;
    let mut cfa = [0u8; 4];
    let mut black = [0u32; 4];
    for (i, (c, b)) in cfa.iter_mut().zip(black.iter_mut()).enumerate() {
        // R-PEF-21: the CFA phase of the stored raster, origin at stored
        // pixel (0, 0), independent of the crop: plane pixel (x, y) is
        // stored pixel (x0 + x, y0 + y).
        let (dx, dy) = (i % 2, i / 2);
        *c = p.cfa[2 * ((crop.y + dy) % 2) + (crop.x + dx) % 2];
        // R-PEF-38: the black level of the pixel's color.
        *b = p.black2[usize::from(*c).min(2)];
    }
    SharedPlan {
        // The shared renderer tells only CFA data (every variant but V5)
        // from YCbCr data.
        variant: SharedVariant::V1,
        width: crop.w,
        height: crop.h,
        valid: (crop.w, crop.h),
        crop: Rect {
            x: 0,
            y: 0,
            w: crop.w,
            h: crop.h,
        },
        orientation: p.orientation,
        cfa,
        white: 2 * p.white,
        black,
        wb: p.wb,
        wb_source: p.wb_source,
        matrix: p.matrix,
        matrix_from_table: p.matrix_from_table,
        // Not read by the renderer.
        curve: [0; 4],
        ycc: Ycc {
            luma: [0.299, 0.587, 0.114],
            zero: [16384.0; 2],
            factors: [1.0; 3],
            black: 0.0,
            subsampling: None,
        },
        layout: Layout::Strips {
            offsets: Vec::new(),
            rows_per_strip: 1,
            row_bytes: 0,
        },
        model: p.model.clone(),
    }
}

/// R-PEF-51 inverted: the cropped pixel (x, y) of a W' x H' image that
/// lands on output pixel (ox, oy).
#[inline]
pub(super) fn source(o: u8, ox: usize, oy: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        2 => (w - 1 - ox, oy),
        3 => (w - 1 - ox, h - 1 - oy),
        4 => (ox, h - 1 - oy),
        5 => (oy, ox),
        6 => (oy, h - 1 - ox),
        7 => (w - 1 - oy, h - 1 - ox),
        8 => (w - 1 - oy, ox),
        _ => (ox, oy),
    }
}

/// P4: v = clip((s - b) / (w - b), 0, 1) per pixel (R-PEF-42), delivered
/// as r = g = b = v without a matrix (R-PEF-47), oriented (R-PEF-51).
fn gray(
    plan: &Plan,
    plane: &Plane,
    cancel: &CancelToken,
    encoder: &dyn RowEncoder,
) -> PefResult<Vec<u8>> {
    let (cw, ch) = (plan.crop.w, plan.crop.h);
    let o = plan.orientation;
    let (ow, oh) = output_size(plan);
    let row_bytes = ow * encoder.pixel_bytes();
    let len = row_bytes
        .checked_mul(oh)
        .filter(|&n| n > 0)
        .ok_or_else(|| fail("output size"))?;
    let mut out = crate::util::alloc(len, ow as u32, oh as u32)?;
    // R-PEF-67: w - b > 0 because each black value is below w - 255.
    let b = plan.gray_black;
    let span = f64::from(plan.white) - b;
    if span <= 0.0 {
        return Err(fail("white level not above black level"));
    }
    let (b, k) = (b as f32, (1.0 / span) as f32);
    let band = (oh / (pool::threads() * 4)).clamp(16, 256);
    let work: Vec<(usize, &mut [u8])> = out.chunks_mut(row_bytes * band).enumerate().collect();
    pool::run(work, cancel, |(i, rows)| {
        let mut v = vec![0f32; ow];
        for (r, dst) in rows.chunks_exact_mut(row_bytes).enumerate() {
            let oy = i * band + r;
            for (ox, px) in v.iter_mut().enumerate() {
                let (x, y) = source(o, ox, oy, cw, ch);
                let s = plane.data.get(y * cw + x).copied().unwrap_or(0);
                // R-PEF-66: the normalized value is clipped to 0 to 1.
                *px = ((f32::from(s) - b) * k).clamp(0.0, 1.0);
            }
            encoder.encode(&IDENTITY, [&v, &v, &v], dst);
        }
        Ok(())
    })?;
    Ok(out)
}

/// Render the decoded crop plane through `encoder`: the bytes of the
/// oriented image.
pub(super) fn render(
    plan: &Plan,
    plane: Plane,
    cancel: &CancelToken,
    encoder: &dyn RowEncoder,
) -> PefResult<Vec<u8>> {
    match plan.variant {
        Variant::P4 => gray(plan, &plane, cancel, encoder),
        Variant::P1 | Variant::P2 | Variant::P3 => {
            render::render(&shared_plan(plan), &Sensor::Cfa(plane), cancel, encoder)
        }
    }
}

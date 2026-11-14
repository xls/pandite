//! From the plan to the image through the shared rendering of the Sony
//! decoder: normalization (R-ORF-39 is R-ARW-47), bilinear demosaicing of
//! the whole raster (R-ORF-44: R-ARW-54 and R-ARW-55 with the whole raster
//! as the valid area), the crop after demosaicing (R-ORF-45 is R-ARW-56),
//! the color matrix (R-ORF-42), linear sRGB (R-ORF-43 is R-ARW-53) and the
//! orientation (R-ORF-47 is R-ARW-58), then pandite's default display
//! rendering or the colorimetric encoding.

use super::meta::Plan;
use super::sensor;
use super::{fail, OrfResult};
use crate::raw::arw::{self, render, Layout, Output, WbSource, Ycc};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{CancelToken, ImageBuf, PixelFormat};

/// The renderer's plan: the raster is the valid area, the pattern origin
/// is raster pixel (0, 0) (R-ORF-19), the black level of CFA position i is
/// b_i (R-ORF-37), the white level 2^B - 1 (R-ORF-35). The renderer
/// computes v = clip((s - b) / (w - b) * m, 0, 1) (R-ORF-39, R-ORF-62);
/// w - b >= 3w / 4 > 0 because b <= w / 4 (R-ORF-36, R-ORF-63).
pub(super) fn bridge(plan: &Plan) -> arw::Plan {
    arw::Plan {
        // The renderer treats every variant except its YCbCr one as CFA
        // data; V1 stands for "CFA plane" here.
        variant: arw::Variant::V1,
        width: plan.width,
        height: plan.height,
        // R-ORF-44: the whole raster (0, 0, W, H) in the role of the valid
        // area.
        valid: (plan.width, plan.height),
        // R-ORF-17, R-ORF-45: the crop is applied after demosaicing.
        crop: plan.crop,
        orientation: plan.orientation,
        cfa: plan.cfa,
        white: plan.white,
        black: plan.black,
        wb: plan.wb,
        wb_source: WbSource::AsShot,
        matrix: plan.matrix,
        matrix_from_table: false,
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
            rows_per_strip: 0,
            row_bytes: 0,
        },
        model: plan.model.clone(),
    }
}

/// The output size of the plan: the crop, H' x W' for orientations 5 to 8
/// (R-ORF-46, R-ARW-58).
pub(super) fn output_size(plan: &Plan) -> (usize, usize) {
    render::output_size(plan.crop, plan.orientation)
}

/// Develop the sensor data to an upright sRGB image in the requested
/// encoding (decoding procedure steps 8 to 13), with the unused bits of a
/// V5 or V6 strip. Every size is checked before the planes are allocated;
/// a failure drops everything decoded so far (R-ORF-65).
pub(super) fn develop(
    data: &[u8],
    plan: &Plan,
    cancel: &CancelToken,
    output: Output,
) -> OrfResult<(ImageBuf, Option<u64>)> {
    let rplan = bridge(plan);
    let (w, h) = output_size(plan);
    let (w, h) = (
        u32::try_from(w).map_err(|_| fail("output size"))?,
        u32::try_from(h).map_err(|_| fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    let (plane, unused) = sensor::decode(data, plan, cancel)?;
    let sensor = render::Sensor::Cfa(plane);
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => render::render(&rplan, &sensor, cancel, &arw::Colorimetric16)?,
        Output::Display16 => render::render(&rplan, &sensor, cancel, &arw::Display16(tone))?,
        Output::Display8 => render::render(&rplan, &sensor, cancel, &arw::Display8(tone))?,
    };
    Ok((ImageBuf::new(w, h, format, bytes)?, unused))
}

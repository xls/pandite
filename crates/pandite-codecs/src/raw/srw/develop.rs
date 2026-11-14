//! From the plan to the image through the shared rendering of the Sony
//! decoder: normalization (R-SRW-34 is R-ARW-47), bilinear demosaicing
//! inside the valid area (R-SRW-39 and R-SRW-40 are R-ARW-54 and
//! R-ARW-55), the color matrix (R-SRW-37), linear sRGB (R-SRW-38 is
//! R-ARW-53) and the orientation (R-SRW-43 is R-ARW-58), then pandite's
//! default display rendering or the colorimetric encoding.
//!
//! The shared renderer works on a plane whose valid area starts at its
//! pixel (0, 0) and takes integer black levels per CFA position. The SRW
//! valid area starts at (VX0, VY0) of the stored raster, so the plane holds
//! the valid area only and the CFA pattern is shifted to keep the pattern
//! origin at stored pixel (0, 0) (R-SRW-23). Neighbors outside the valid
//! area are then outside the plane and are ignored (R-SRW-40), and the
//! delivered image is exactly the valid area (R-SRW-41).

use super::meta::Plan;
use super::sensor::{self, Domain};
use super::SrwResult;
use crate::raw::arw::{self, render, Layout, Output, Rect, WbSource, Ycc};
use crate::raw::tone::Tone;
use crate::util::check_size;
use pandite_core::{CancelToken, ImageBuf, PixelFormat};

/// The colors of plane CFA positions 0 to 3 for a plane whose pixel (0, 0)
/// is stored pixel (x0, y0): stored pixel (x, y) has color
/// CFAPattern[2 * (y mod 2) + (x mod 2)] (R-SRW-23).
pub(super) fn shifted_cfa(cfa: [u8; 4], x0: usize, y0: usize) -> [u8; 4] {
    let mut out = [0u8; 4];
    for (p, o) in out.iter_mut().enumerate() {
        let (px, py) = (p % 2, p / 2);
        *o = cfa[2 * ((py + y0) % 2) + (px + x0) % 2];
    }
    out
}

/// The renderer's plan for the valid-area plane and the sample domain.
///
/// R-SRW-34: v = clip((s - b) / (w - b) * m, 0, 1) per pixel. With integer
/// black levels the plane holds the values as read. When bG is a half
/// (R-SRW-32), the plane holds doubled values with doubled levels, which
/// normalize to the same v: (2s - 2b) / (2w - 2b) = (s - b) / (w - b).
pub(super) fn bridge(plan: &Plan) -> (arw::Plan, Domain) {
    let v = plan.valid;
    let cfa = shifted_cfa(plan.cfa, v.x, v.y);
    let whole = plan.black.iter().all(|b| b.fract() == 0.0);
    let (white, black, domain) = if whole {
        (plan.white, plan.black.map(|b| b as u32), Domain::AsRead)
    } else {
        let white = 2 * plan.white;
        let black = plan.black.map(|b| (2.0 * b) as u32);
        // A doubled value clipped at 65535 normalizes to
        // (65535 - 2b) / (2w - 2b) * m; when that is at least 1, the true
        // value (larger) also clips to 1, and the plane stays exact.
        let mut exact = [false; 3];
        for c in 0..3 {
            let span = f64::from(white) - f64::from(black[c]);
            exact[c] = (65535.0 - f64::from(black[c])) * plan.wb[c] >= span * (1.0 + 1e-6);
        }
        (white, black, Domain::Doubled { exact })
    };
    let rplan = arw::Plan {
        // The renderer treats every variant except its YCbCr one as CFA
        // data; V1 stands for "CFA plane" here.
        variant: arw::Variant::V1,
        width: v.w,
        height: v.h,
        valid: (v.w, v.h),
        crop: Rect {
            x: 0,
            y: 0,
            w: v.w,
            h: v.h,
        },
        orientation: plan.orientation,
        cfa,
        white,
        // Black level of each plane CFA position, from its color.
        black: cfa.map(|c| black[usize::from(c).min(2)]),
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
    };
    (rplan, domain)
}

/// The output size of the plan: the valid area, H' x W' for orientations
/// 5 to 8 (R-SRW-41, R-SRW-43).
pub(super) fn output_size(plan: &Plan) -> (usize, usize) {
    render::output_size(plan.valid, plan.orientation)
}

/// Develop the sensor data to an upright sRGB image in the requested
/// encoding. Every size is checked before the planes are allocated.
pub(super) fn develop(
    data: &[u8],
    plan: &Plan,
    cancel: &CancelToken,
    output: Output,
) -> SrwResult<ImageBuf> {
    let (rplan, domain) = bridge(plan);
    let (w, h) = output_size(plan);
    let (w, h) = (
        u32::try_from(w).map_err(|_| super::fail("output size"))?,
        u32::try_from(h).map_err(|_| super::fail("output size"))?,
    );
    let format = match output {
        Output::Display8 => PixelFormat::Rgba8,
        Output::Colorimetric16 | Output::Display16 => PixelFormat::Rgba16,
    };
    check_size(w, h, format)?;
    let sensor = render::Sensor::Cfa(sensor::decode(data, plan, domain, cancel)?);
    let tone = Tone::unit();
    let bytes = match output {
        Output::Colorimetric16 => render::render(&rplan, &sensor, cancel, &arw::Colorimetric16)?,
        Output::Display16 => render::render(&rplan, &sensor, cancel, &arw::Display16(tone))?,
        Output::Display8 => render::render(&rplan, &sensor, cancel, &arw::Display8(tone))?,
    };
    ImageBuf::new(w, h, format, bytes)
}

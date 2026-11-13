//! The built-in thumbnail downscaler: an exact area-average (box) filter.
//!
//! Every source pixel contributes to a destination pixel with its exact
//! area weight, and color is weighted by alpha, so transparent pixels do
//! not darken edges.
//!
//! Rgba8, the common case, runs in exact integer arithmetic ([`rgba8`]):
//! the per-axis weights are integers, the sums are exact, and each output
//! sample is the exact area average rounded to the nearest step (halves
//! up). Its hot loop has vector versions ([`simd`]). Rgba16 and RgbaF32
//! run the `f64` implementation in [`reference`], which also serves as the
//! test oracle for Rgba8: the two differ by at most one step per channel,
//! and only where the exact average lies within float rounding error of a
//! half step.

mod reference;
mod rgba8;
mod simd;

#[cfg(test)]
mod tests;

use pandite_core::{ImageBuf, PixelFormat};

pub use reference::box_downscale_reference;
pub use simd::Level as DownscaleLevel;

/// The size `(width, height)` that fits `(width, height)` inside a
/// `max_side` square, keeping the aspect ratio. Never enlarges, never
/// returns a zero side.
pub fn fit_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let max_side = max_side.max(1);
    if width <= max_side && height <= max_side {
        return (width.max(1), height.max(1));
    }
    let (w, h, m) = (u64::from(width), u64::from(height), u64::from(max_side));
    if w >= h {
        let nh = ((h * m + w / 2) / w).clamp(1, m);
        (max_side, nh as u32)
    } else {
        let nw = ((w * m + h / 2) / h).clamp(1, m);
        (nw as u32, max_side)
    }
}

/// Shrink `image` so its longer side is at most `max_side`, averaging every
/// source pixel that covers a destination pixel with its exact area weight.
///
/// Color is weighted by alpha, so transparent pixels do not darken edges.
/// Rgba8 and Rgba16 average in their stored (sRGB) encoding, RgbaF32 in
/// linear light; non-finite float samples count as zero. An image that
/// already fits, or a pixel format this function does not know, is
/// returned unchanged (a cheap clone).
pub fn box_downscale(image: &ImageBuf, max_side: u32) -> ImageBuf {
    box_downscale_with(DownscaleLevel::detected(), image, max_side)
}

/// [`box_downscale`] with the Rgba8 kernels at a given instruction set
/// level. A level this CPU lacks runs the scalar kernels. Other pixel
/// formats ignore the level.
pub fn box_downscale_with(level: DownscaleLevel, image: &ImageBuf, max_side: u32) -> ImageBuf {
    let (sw, sh) = (image.width(), image.height());
    let (dw, dh) = fit_size(sw, sh, max_side);
    if (dw, dh) == (sw, sh) {
        return image.clone();
    }
    match image.format() {
        PixelFormat::Rgba8 => rgba8::downscale(level, image, dw, dh),
        _ => box_downscale_reference(image, max_side),
    }
}

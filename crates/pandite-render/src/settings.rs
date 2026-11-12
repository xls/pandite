//! Renderer settings the application controls.

use crate::text::FontChoice;
use pandite_core::Color;

/// Default zoom factor from which magnified content is sampled crisply
/// (each image pixel a sharp square) instead of bilinearly. See
/// [`RenderSettings::crisp_zoom`].
pub const CRISP_ZOOM_DEFAULT: f32 = 3.0;

/// The checkerboard drawn behind transparent content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Checkerboard {
    /// Color of the cell at the image's top-left corner.
    pub light: Color,
    /// Color of the alternating cells.
    pub dark: Color,
    /// Cell side in physical pixels. Scale it with the display scale factor.
    pub cell: f32,
}

impl Default for Checkerboard {
    fn default() -> Self {
        Self {
            light: Color::rgb(204, 204, 204),
            dark: Color::rgb(153, 153, 153),
            cell: 8.0,
        }
    }
}

/// Everything about a frame that is not content, view or interface.
#[derive(Clone, Debug, PartialEq)]
pub struct RenderSettings {
    /// Fill outside the content and under transparent content when the
    /// checkerboard is off. Opaque sRGB.
    pub background: Color,
    /// Checkerboard behind transparent content, or `None` for the plain
    /// background color.
    pub checkerboard: Option<Checkerboard>,
    /// Wait for the display refresh (FIFO presentation). Off presents as
    /// fast as possible without tearing where the platform allows it.
    pub vsync: bool,
    /// Exposure for high dynamic range content in stops (EV). 0 shows the
    /// stored values, +1 doubles them. Standard range content ignores it.
    pub exposure: f32,
    /// Zoom factor (screen pixels per image pixel) from which content is
    /// magnified crisply: every image pixel becomes a flat square with a
    /// one screen pixel antialiased seam, for pixel peeping. Below it,
    /// magnification is bilinear on the full resolution level and
    /// minification is trilinear over mip levels built in linear light.
    /// Values below 1 act as 1, so minified content is always filtered.
    pub crisp_zoom: f32,
    /// Level of detail bias for minification. Slightly negative values
    /// counter the softness of trilinear filtering without aliasing.
    pub lod_bias: f32,
    /// Interface font.
    pub font: FontChoice,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            background: Color::rgb(30, 30, 30),
            checkerboard: Some(Checkerboard::default()),
            vsync: true,
            exposure: 0.0,
            crisp_zoom: CRISP_ZOOM_DEFAULT,
            lod_bias: -0.25,
            font: FontChoice::default(),
        }
    }
}

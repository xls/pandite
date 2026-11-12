//! Display modes and the image-to-viewport transform.
//!
//! [`ViewState`] is pure math. The application feeds it input, the renderer
//! reads [`ViewState::image_rect`] to place the content quad.

use crate::geom::{Point, Rect, Size};

/// How the content is sized in the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DisplayMode {
    /// Scale to best fit the viewport by width or height. Small images are
    /// enlarged only when `ViewState::enlarge_to_fit` is set.
    Fit,
    /// One image pixel per physical screen pixel.
    OneToOne,
    /// A free zoom factor (image pixels to physical pixels).
    Zoom(f32),
}

/// The current view of one piece of content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewState {
    /// Sizing rule.
    pub mode: DisplayMode,
    /// Offset of the image center from the viewport center, in physical pixels.
    pub pan: Point,
    /// In `Fit` mode, also scale small images up to the viewport.
    pub enlarge_to_fit: bool,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            mode: DisplayMode::Fit,
            pan: Point::default(),
            enlarge_to_fit: false,
        }
    }
}

impl ViewState {
    /// Smallest allowed zoom factor.
    pub const MIN_ZOOM: f32 = 0.01;
    /// Largest allowed zoom factor.
    pub const MAX_ZOOM: f32 = 64.0;

    /// The effective scale for `image` shown in `viewport`.
    pub fn scale(&self, image: Size, viewport: Size) -> f32 {
        match self.mode {
            DisplayMode::Fit => {
                let s = image.fit_scale(viewport);
                if self.enlarge_to_fit {
                    s
                } else {
                    s.min(1.0)
                }
            }
            DisplayMode::OneToOne => 1.0,
            DisplayMode::Zoom(z) => clamp_zoom(z),
        }
    }

    /// Where the image lands in viewport coordinates.
    pub fn image_rect(&self, image: Size, viewport: Rect) -> Rect {
        let scaled = image.scaled(self.scale(image, viewport.size()));
        let pan = self.clamped_pan(image, viewport.size());
        let mut rect = viewport.centered(scaled);
        rect.x += pan.x;
        rect.y += pan.y;
        rect
    }

    /// Switch to fit mode and recenter.
    pub fn set_fit(&mut self) {
        self.mode = DisplayMode::Fit;
        self.pan = Point::default();
    }

    /// Switch to 1:1, keeping `anchor` (a viewport position) over the same
    /// image point.
    pub fn set_one_to_one(&mut self, image: Size, viewport: Rect, anchor: Point) {
        self.zoom_to(1.0, image, viewport, anchor);
        self.mode = DisplayMode::OneToOne;
    }

    /// Multiply the zoom by `factor`, keeping the image point under `anchor`
    /// (a viewport position, normally the cursor) fixed on screen.
    pub fn zoom_by(&mut self, factor: f32, image: Size, viewport: Rect, anchor: Point) {
        let current = self.scale(image, viewport.size());
        self.zoom_to(current * factor, image, viewport, anchor);
    }

    /// Set an absolute zoom, keeping the image point under `anchor` fixed.
    pub fn zoom_to(&mut self, zoom: f32, image: Size, viewport: Rect, anchor: Point) {
        if image.is_empty() || viewport.is_empty() || !zoom.is_finite() {
            return;
        }
        let before = self.image_rect(image, viewport);
        let old_scale = self.scale(image, viewport.size());
        let new_scale = clamp_zoom(zoom);
        // Image-space point under the anchor.
        let ix = (anchor.x - before.x) / old_scale;
        let iy = (anchor.y - before.y) / old_scale;
        self.mode = DisplayMode::Zoom(new_scale);
        // Solve for the pan that puts that point back under the anchor.
        let scaled = image.scaled(new_scale);
        let centered = viewport.centered(scaled);
        self.pan = Point::new(
            anchor.x - ix * new_scale - centered.x,
            anchor.y - iy * new_scale - centered.y,
        );
        self.pan = self.clamped_pan(image, viewport.size());
    }

    /// Move the image by a drag delta in physical pixels.
    pub fn pan_by(&mut self, dx: f32, dy: f32, image: Size, viewport: Size) {
        self.pan.x += dx;
        self.pan.y += dy;
        self.pan = self.clamped_pan(image, viewport);
    }

    /// The pan limited so the image cannot leave the viewport: an axis that
    /// fits is centered, an axis that overflows may scroll edge to edge.
    pub fn clamped_pan(&self, image: Size, viewport: Size) -> Point {
        let scaled = image.scaled(self.scale(image, viewport));
        let limit_x = ((scaled.w - viewport.w) * 0.5).max(0.0);
        let limit_y = ((scaled.h - viewport.h) * 0.5).max(0.0);
        Point::new(
            self.pan.x.clamp(-limit_x, limit_x),
            self.pan.y.clamp(-limit_y, limit_y),
        )
    }

    /// Convert a viewport position to image pixel coordinates.
    pub fn viewport_to_image(&self, p: Point, image: Size, viewport: Rect) -> Point {
        let rect = self.image_rect(image, viewport);
        let scale = self.scale(image, viewport.size());
        Point::new((p.x - rect.x) / scale, (p.y - rect.y) / scale)
    }
}

fn clamp_zoom(z: f32) -> f32 {
    if z.is_finite() {
        z.clamp(ViewState::MIN_ZOOM, ViewState::MAX_ZOOM)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Rect = Rect::new(0.0, 0.0, 800.0, 600.0);

    #[test]
    fn fit_mode_does_not_enlarge_by_default() {
        let mut view = ViewState::default();
        let small = Size::new(100.0, 100.0);
        assert_eq!(view.scale(small, VIEW.size()), 1.0);
        assert_eq!(
            view.image_rect(small, VIEW),
            Rect::new(350.0, 250.0, 100.0, 100.0)
        );
        view.enlarge_to_fit = true;
        assert_eq!(view.scale(small, VIEW.size()), 6.0);
        let big = Size::new(1600.0, 600.0);
        assert_eq!(view.scale(big, VIEW.size()), 0.5);
    }

    #[test]
    fn zoom_keeps_anchor_point_fixed() {
        let mut view = ViewState::default();
        let image = Size::new(4000.0, 3000.0);
        let anchor = Point::new(600.0, 150.0);
        let before = view.viewport_to_image(anchor, image, VIEW);
        view.zoom_by(3.0, image, VIEW, anchor);
        let after = view.viewport_to_image(anchor, image, VIEW);
        assert!((before.x - after.x).abs() < 0.01, "{before:?} {after:?}");
        assert!((before.y - after.y).abs() < 0.01, "{before:?} {after:?}");
        assert_eq!(view.mode, DisplayMode::Zoom(0.6));
    }

    #[test]
    fn zoom_is_clamped_and_pan_is_limited() {
        let mut view = ViewState::default();
        let image = Size::new(1000.0, 1000.0);
        view.zoom_to(1.0e9, image, VIEW, VIEW.center());
        assert_eq!(view.mode, DisplayMode::Zoom(ViewState::MAX_ZOOM));
        view.zoom_to(2.0, image, VIEW, VIEW.center());
        view.pan_by(1.0e6, -1.0e6, image, VIEW.size());
        // 2000 px image in an 800 x 600 viewport: limits are 600 and 700.
        assert_eq!(view.pan, Point::new(600.0, -700.0));
        // An image smaller than the viewport stays centered.
        view.zoom_to(0.1, image, VIEW, Point::new(3.0, 3.0));
        assert_eq!(view.pan, Point::default());
    }

    #[test]
    fn one_to_one_and_fit_switch() {
        let mut view = ViewState::default();
        let image = Size::new(4000.0, 3000.0);
        view.set_one_to_one(image, VIEW, VIEW.center());
        assert_eq!(view.mode, DisplayMode::OneToOne);
        assert_eq!(view.scale(image, VIEW.size()), 1.0);
        view.pan_by(50.0, 0.0, image, VIEW.size());
        view.set_fit();
        assert_eq!(view.pan, Point::default());
        assert_eq!(view.scale(image, VIEW.size()), 0.2);
    }
}

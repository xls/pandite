//! Sizes in physical pixels, derived from the DPI scale factor.
//!
//! Every length the interface uses is a logical size multiplied by the
//! scale factor and rounded to whole pixels, so edges stay crisp.

use std::time::Duration;

/// How long the pointer must rest before the toolbar hides.
pub(crate) const IDLE_HIDE: Duration = Duration::from_millis(2200);
/// Toolbar fade-out time.
pub(crate) const FADE_OUT: Duration = Duration::from_millis(250);
/// Toolbar fade-in time.
pub(crate) const FADE_IN: Duration = Duration::from_millis(90);
/// Hover time before a tooltip shows.
pub(crate) const TOOLTIP_DELAY: Duration = Duration::from_millis(550);
/// Maximum gap between the presses of a double click.
pub(crate) const DOUBLE_CLICK: Duration = Duration::from_millis(450);
/// How long a toast stays fully visible.
pub(crate) const TOAST_HOLD: Duration = Duration::from_millis(2200);
/// Toast fade-out time.
pub(crate) const TOAST_FADE: Duration = Duration::from_millis(350);
/// Load time before the loading overlay shows, so fast loads never flash it.
pub(crate) const LOADING_DELAY: Duration = Duration::from_millis(150);
/// Frame interval of the small background-work spinner.
pub(crate) const BUSY_FRAME: Duration = Duration::from_millis(100);

/// Scaled sizes for one scale factor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Metrics {
    pub scale: f32,
}

impl Metrics {
    /// Sizes for `scale`. Bad or extreme factors are clamped.
    pub fn new(scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale.clamp(0.5, 8.0)
        } else {
            1.0
        };
        Self { scale }
    }

    /// A logical length in whole physical pixels, at least 1.
    pub fn px(&self, logical: f32) -> f32 {
        (logical * self.scale).round().max(1.0)
    }

    /// A logical font size in physical pixels (not rounded).
    pub fn font(&self, logical: f32) -> f32 {
        logical * self.scale
    }

    pub fn text(&self) -> f32 {
        self.font(13.0)
    }
    pub fn text_small(&self) -> f32 {
        self.font(11.5)
    }
    pub fn text_title(&self) -> f32 {
        self.font(15.0)
    }

    pub fn toolbar_h(&self) -> f32 {
        self.px(40.0)
    }
    pub fn button(&self) -> f32 {
        self.px(32.0)
    }
    pub fn icon(&self) -> f32 {
        self.px(20.0)
    }
    pub fn icon_px(&self) -> u32 {
        self.icon() as u32
    }
    pub fn gap(&self) -> f32 {
        self.px(2.0)
    }
    pub fn group_gap(&self) -> f32 {
        self.px(12.0)
    }
    pub fn pad(&self) -> f32 {
        self.px(8.0)
    }
    pub fn radius(&self) -> f32 {
        self.px(6.0)
    }
    pub fn radius_small(&self) -> f32 {
        self.px(4.0)
    }
    pub fn hairline(&self) -> f32 {
        self.px(1.0)
    }

    pub fn gallery_h(&self) -> f32 {
        self.px(84.0)
    }
    pub fn cell(&self) -> f32 {
        self.px(64.0)
    }
    pub fn cell_gap(&self) -> f32 {
        self.px(6.0)
    }
    pub fn gallery_pad(&self) -> f32 {
        self.px(10.0)
    }
    /// Side of the check box in a thumbnail corner.
    pub fn gallery_check(&self) -> f32 {
        self.px(18.0)
    }
    /// Gap between a thumbnail corner and its check box.
    pub fn gallery_check_inset(&self) -> f32 {
        self.px(4.0)
    }
    /// Side of the check badge on a selected image.
    pub fn badge(&self) -> f32 {
        self.px(26.0)
    }

    pub fn menu_item_h(&self) -> f32 {
        self.px(28.0)
    }
    pub fn menu_sep_h(&self) -> f32 {
        self.px(9.0)
    }
    pub fn menu_w(&self) -> f32 {
        self.px(280.0)
    }

    pub fn dialog_w(&self) -> f32 {
        self.px(440.0)
    }
    pub fn row_h(&self) -> f32 {
        self.px(36.0)
    }
    pub fn control_h(&self) -> f32 {
        self.px(28.0)
    }
    pub fn label_w(&self) -> f32 {
        self.px(116.0)
    }

    pub fn handle(&self) -> f32 {
        self.px(9.0)
    }
    pub fn handle_hit(&self) -> f32 {
        self.px(18.0)
    }
    pub fn drag_threshold(&self) -> f32 {
        self.px(4.0)
    }
    pub fn pan_step(&self) -> f32 {
        self.px(80.0)
    }
    pub fn margin(&self) -> f32 {
        self.px(16.0)
    }
}

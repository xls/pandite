//! The auto-hiding top toolbar: layout only. Drawing and input live in
//! [`crate::ui`], which owns the state.
//!
//! The layout is a pure function of the window width and scale factor, so
//! hit testing in `handle_event` and drawing in `build` always agree, even
//! before the first frame. Buttons that do not fit move into an overflow
//! menu behind a "more" button.

use crate::command::Command;
use crate::metrics::Metrics;
use pandite_core::Rect;

/// Toolbar buttons, grouped. Groups are separated by a gap and a hairline.
pub(crate) const GROUPS: &[&[Command]] = &[
    &[Command::Open, Command::SaveAs, Command::CopyPath],
    &[Command::Previous, Command::Next],
    &[
        Command::Undo,
        Command::RotateLeft,
        Command::RotateRight,
        Command::FlipHorizontal,
        Command::FlipVertical,
        Command::Crop,
        Command::Resize,
        Command::BulkResize,
    ],
    &[
        Command::ZoomOut,
        Command::ZoomIn,
        Command::Fit,
        Command::ActualSize,
    ],
    &[Command::ToggleGallery, Command::Fullscreen],
];

/// Where everything in the toolbar sits.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ToolbarLayout {
    /// The bar itself.
    pub bar: Rect,
    /// Visible buttons.
    pub buttons: Vec<(Command, Rect)>,
    /// X positions of group separators.
    pub separators: Vec<f32>,
    /// The overflow button, when some buttons did not fit.
    pub more: Option<Rect>,
    /// Commands that did not fit, in toolbar order.
    pub overflow: Vec<Command>,
    /// Space left for the status readout.
    pub status: Rect,
}

impl ToolbarLayout {
    /// Recompute for a window `width` pixels wide, reusing allocations.
    pub fn compute(&mut self, width: f32, m: &Metrics) {
        self.buttons.clear();
        self.separators.clear();
        self.overflow.clear();
        self.more = None;
        let h = m.toolbar_h();
        let b = m.button();
        let gap = m.gap();
        let group_gap = m.group_gap();
        let pad = m.pad();
        self.bar = Rect::new(0.0, 0.0, width.max(0.0), h);
        let y = ((h - b) * 0.5).round();

        // Total width when everything fits.
        let mut total = 0.0;
        for (gi, group) in GROUPS.iter().enumerate() {
            if gi > 0 {
                total += group_gap;
            }
            total += group.len() as f32 * b + (group.len() as f32 - 1.0) * gap;
        }
        let available = (width - 2.0 * pad).max(0.0);
        let limit = if total <= available {
            available
        } else {
            (available - b - gap).max(0.0)
        };

        let mut x = pad;
        let mut overflowing = false;
        for (gi, group) in GROUPS.iter().enumerate() {
            for (ci, &cmd) in group.iter().enumerate() {
                let mut start = x;
                if ci == 0 && gi > 0 {
                    start += group_gap;
                } else if ci > 0 {
                    start += gap;
                }
                if overflowing || start + b - pad > limit {
                    overflowing = true;
                    self.overflow.push(cmd);
                    continue;
                }
                if ci == 0 && gi > 0 {
                    self.separators.push((x + group_gap * 0.5).round());
                }
                self.buttons.push((cmd, Rect::new(start, y, b, b)));
                x = start + b;
            }
        }
        let status_x = x + group_gap;
        let mut status_right = width - pad;
        if !self.overflow.is_empty() {
            let more = Rect::new((width - pad - b).max(pad), y, b, b);
            status_right = more.x - gap;
            self.more = Some(more);
        }
        self.status = Rect::new(status_x, 0.0, (status_right - status_x).max(0.0), h);
    }

    /// The command under `p`, if any.
    pub fn button_at(&self, p: pandite_core::Point) -> Option<Command> {
        self.buttons
            .iter()
            .find(|(_, r)| r.contains(p))
            .map(|(c, _)| *c)
    }

    /// The rectangle of a visible button.
    pub fn button_rect(&self, cmd: Command) -> Option<Rect> {
        self.buttons
            .iter()
            .find(|(c, _)| *c == cmd)
            .map(|(_, r)| *r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_window_shows_every_button() {
        let mut layout = ToolbarLayout::default();
        layout.compute(1600.0, &Metrics::new(1.0));
        let n: usize = GROUPS.iter().map(|g| g.len()).sum();
        assert_eq!(layout.buttons.len(), n);
        assert!(layout.overflow.is_empty());
        assert!(layout.more.is_none());
        assert_eq!(layout.separators.len(), GROUPS.len() - 1);
        assert!(layout.status.w > 100.0);
    }

    #[test]
    fn narrow_window_moves_buttons_to_overflow() {
        let mut layout = ToolbarLayout::default();
        layout.compute(300.0, &Metrics::new(1.0));
        let n: usize = GROUPS.iter().map(|g| g.len()).sum();
        assert_eq!(layout.buttons.len() + layout.overflow.len(), n);
        assert!(!layout.overflow.is_empty());
        let more = layout.more.expect("more button");
        assert!(more.right() <= 300.0);
        for (_, r) in &layout.buttons {
            assert!(r.right() <= more.x, "button overlaps the more button");
        }
        // A tiny window still has a reachable overflow menu.
        layout.compute(20.0, &Metrics::new(1.0));
        assert_eq!(layout.overflow.len(), n);
        assert!(layout.more.is_some());
    }
}

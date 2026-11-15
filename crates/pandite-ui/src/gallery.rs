//! The bottom gallery strip: layout and smooth scrolling.
//!
//! Items are square cells in one row. The strip scrolls horizontally with
//! the wheel (smoothly animated toward a target offset) or by dragging. The
//! current item is scrolled into view whenever the current index, the item
//! count or the strip width changes; manual scrolling is left alone
//! otherwise. A change of the item list that the user did not ask for
//! (files that came to or left the folder) scrolls nothing into view: the
//! items on screen keep their places ([`GalleryState::follow`]).
//!
//! A selected item carries a small round badge in the top left corner of
//! its cell. The badge is a marker only: a click anywhere on a cell shows
//! the item, and the pointer never changes the selection.
//!
//! The cells come in three sizes ([`GallerySize`]). The gap between cells
//! and the padding around them grow with the cell; the badge and the ring
//! around the current item keep their size.

use crate::metrics::Metrics;
use pandite_core::{Point, Rect};
use std::ops::Range;

/// The size of the gallery cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GallerySize {
    /// 64 logical pixels.
    Small,
    /// 96 logical pixels.
    #[default]
    Medium,
    /// 128 logical pixels.
    Large,
}

impl GallerySize {
    /// Every size, smallest first.
    pub const ALL: [GallerySize; 3] = [GallerySize::Small, GallerySize::Medium, GallerySize::Large];

    /// The cell side in logical pixels.
    pub fn cell(self) -> f32 {
        match self {
            GallerySize::Small => 64.0,
            GallerySize::Medium => 96.0,
            GallerySize::Large => 128.0,
        }
    }

    /// The gap between two cells in logical pixels.
    pub(crate) fn gap(self) -> f32 {
        self.cell() * (6.0 / 64.0)
    }

    /// The space above, below and beside the cells in logical pixels.
    pub(crate) fn pad(self) -> f32 {
        self.cell() * (10.0 / 64.0)
    }

    /// The strip height in logical pixels: the cell and its padding.
    pub(crate) fn strip(self) -> f32 {
        self.cell() + 2.0 * self.pad()
    }

    /// The next larger size, `None` for the largest.
    pub fn larger(self) -> Option<GallerySize> {
        match self {
            GallerySize::Small => Some(GallerySize::Medium),
            GallerySize::Medium => Some(GallerySize::Large),
            GallerySize::Large => None,
        }
    }

    /// The next smaller size, `None` for the smallest.
    pub fn smaller(self) -> Option<GallerySize> {
        match self {
            GallerySize::Small => None,
            GallerySize::Medium => Some(GallerySize::Small),
            GallerySize::Large => Some(GallerySize::Medium),
        }
    }

    /// The cell side in physical pixels at DPI scale `scale`, rounded as
    /// every interface length is. A thumbnail of this side fills a cell
    /// pixel for pixel.
    pub fn cell_px(self, scale: f32) -> u32 {
        Metrics::new(scale).with_gallery(self).cell() as u32
    }
}

/// Pure geometry of the strip for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GalleryLayout {
    /// The whole panel.
    pub panel: Rect,
    /// The area items scroll in.
    pub strip: Rect,
    /// Cell side.
    pub cell: f32,
    /// Distance from one cell's left edge to the next.
    pub pitch: f32,
    /// Item count.
    pub count: usize,
    /// X of item 0 at scroll 0 (centered when everything fits).
    pub origin: f32,
    /// Side of the selection badge in a cell corner.
    pub badge: f32,
    /// Gap between the cell corner and the badge.
    pub badge_inset: f32,
}

impl GalleryLayout {
    /// The layout for a panel at `panel` holding `count` items.
    pub fn new(panel: Rect, count: usize, m: &Metrics) -> Self {
        let pad = m.gallery_pad();
        let cell = m.cell();
        let pitch = cell + m.cell_gap();
        let strip = Rect::new(
            panel.x,
            panel.y + ((panel.h - cell) * 0.5).round(),
            panel.w,
            cell,
        );
        let content = Self::content_width_for(count, cell, pitch) + 2.0 * pad;
        let origin = if content <= panel.w {
            panel.x + ((panel.w - content) * 0.5).round() + pad
        } else {
            panel.x + pad
        };
        Self {
            panel,
            strip,
            cell,
            pitch,
            count,
            origin,
            badge: m.thumb_badge().min(cell),
            badge_inset: m.thumb_badge_inset(),
        }
    }

    fn content_width_for(count: usize, cell: f32, pitch: f32) -> f32 {
        if count == 0 {
            0.0
        } else {
            (count - 1) as f32 * pitch + cell
        }
    }

    /// Largest useful scroll offset.
    pub fn max_scroll(&self) -> f32 {
        let pad = self.origin - self.panel.x;
        let content = Self::content_width_for(self.count, self.cell, self.pitch) + 2.0 * pad;
        (content - self.panel.w).max(0.0)
    }

    /// Clamp a scroll offset to the valid range.
    pub fn clamp_scroll(&self, scroll: f32) -> f32 {
        if scroll.is_finite() {
            scroll.clamp(0.0, self.max_scroll())
        } else {
            0.0
        }
    }

    /// The cell of item `i` at `scroll`, snapped to whole pixels.
    pub fn item_rect(&self, i: usize, scroll: f32) -> Rect {
        let x = (self.origin + i as f32 * self.pitch - scroll).round();
        Rect::new(x, self.strip.y, self.cell, self.cell)
    }

    /// Items at least partly inside the panel at `scroll`.
    pub fn visible_range(&self, scroll: f32) -> Range<usize> {
        if self.count == 0 || self.panel.w <= 0.0 {
            return 0..0;
        }
        let left = self.panel.x + scroll - self.origin;
        let right = left + self.panel.w;
        let first = ((left - self.cell) / self.pitch).floor() + 1.0;
        let first = first.max(0.0) as usize;
        let last = (right / self.pitch).ceil().max(0.0) as usize;
        let end = last.min(self.count);
        first.min(end)..end
    }

    /// The selection badge drawn in the top left corner of the cell `cell`.
    pub fn badge_rect(&self, cell: Rect) -> Rect {
        let side = self.badge.min(cell.w).min(cell.h);
        let inset = self.badge_inset.min(((cell.w - side) * 0.5).max(0.0));
        Rect::new(cell.x + inset, cell.y + inset, side, side)
    }

    /// The item under `p` at `scroll`, if any.
    pub fn hit(&self, p: Point, scroll: f32) -> Option<usize> {
        if !self.panel.contains(p) {
            return None;
        }
        self.visible_range(scroll)
            .find(|&i| self.item_rect(i, scroll).contains(p))
    }

    /// The scroll offset that centers item `i` (clamped to the ends).
    pub fn scroll_to_center(&self, i: usize) -> f32 {
        let item_left = self.origin + i as f32 * self.pitch - self.panel.x;
        self.clamp_scroll(item_left + self.cell * 0.5 - self.panel.w * 0.5)
    }

    /// The scroll offset that shows item `i` entirely, moving as little as
    /// possible from `scroll`; an item far away is centered instead.
    pub fn scroll_to_show(&self, i: usize, scroll: f32) -> f32 {
        let item_left = self.origin + i as f32 * self.pitch - self.panel.x;
        let pad = self.origin - self.panel.x;
        let lo = item_left - pad;
        let hi = item_left + self.cell + pad - self.panel.w;
        let target = if scroll > lo || scroll < hi {
            // Not fully visible.
            let far = scroll - lo > self.panel.w || hi - scroll > self.panel.w;
            if far {
                return self.scroll_to_center(i);
            } else if scroll > lo {
                lo
            } else {
                hi
            }
        } else {
            scroll
        };
        self.clamp_scroll(target)
    }
}

/// Scroll and tracking state of the strip.
#[derive(Clone, Debug, Default)]
pub(crate) struct GalleryState {
    /// Current (animated) offset in pixels.
    pub scroll: f32,
    /// Offset the animation moves toward.
    pub target: f32,
    /// What the strip last kept in view: index, count, width, cell side.
    pub tracked: Option<(usize, usize, u32, u32)>,
    /// Hovered item.
    pub hover: Option<usize>,
}

impl GalleryState {
    /// Keep the current item in view when it, the count, the width or the
    /// cell size changed. Jumps without animation the first time and when
    /// the cell size changed, because every item moved.
    pub fn track(&mut self, layout: &GalleryLayout, index: usize, count: usize) {
        let key = (
            index,
            count,
            layout.panel.w.round() as u32,
            layout.cell.round() as u32,
        );
        if self.tracked == Some(key) {
            self.scroll = layout.clamp_scroll(self.scroll);
            self.target = layout.clamp_scroll(self.target);
            return;
        }
        let first = self.tracked.is_none_or(|t| t.3 != key.3);
        self.tracked = Some(key);
        if count == 0 {
            self.scroll = 0.0;
            self.target = 0.0;
            return;
        }
        let index = index.min(count - 1);
        if first {
            self.target = layout.scroll_to_center(index);
            self.scroll = self.target;
        } else {
            self.target = layout.scroll_to_show(index, self.target);
        }
        self.scroll = layout.clamp_scroll(self.scroll);
    }

    /// The items moved without the user: item `i` of the `old` layout is
    /// item `moved[i]` of the `new` one, `None` when it left. The first
    /// item on screen that stayed keeps its place, so the offset moves by
    /// the width of the items that came or went before it. `index` and the
    /// new count become the tracked ones: the strip does not scroll to the
    /// current item for this change. Returns the change of the offset.
    /// `None` when the strip did not show `old` or no item stayed; the
    /// strip then tracks as usual.
    pub fn follow(
        &mut self,
        old: &GalleryLayout,
        new: &GalleryLayout,
        index: usize,
        moved: &[Option<usize>],
    ) -> Option<f32> {
        let (_, count, width, cell) = self.tracked?;
        let same_strip = (width, cell) == (new.panel.w.round() as u32, new.cell.round() as u32);
        if count != moved.len() || old.count != count || !same_strip {
            return None;
        }
        let visible = old.visible_range(old.clamp_scroll(self.scroll));
        let stayed = |i: usize| Some((i, moved[i].filter(|&n| n < new.count)?));
        let (from, to) = visible
            .clone()
            .find_map(stayed)
            .or_else(|| (0..visible.start).rev().find_map(stayed))
            .or_else(|| (visible.end..count).find_map(stayed))?;
        let delta = (new.origin + to as f32 * new.pitch) - (old.origin + from as f32 * old.pitch);
        self.scroll = new.clamp_scroll(self.scroll + delta);
        self.target = new.clamp_scroll(self.target + delta);
        self.tracked = Some((index, new.count, width, cell));
        Some(delta)
    }

    /// Advance the scroll animation by `dt` seconds. Returns true while it
    /// is still moving.
    pub fn animate(&mut self, dt: f32) -> bool {
        let diff = self.target - self.scroll;
        if diff.abs() < 0.5 {
            self.scroll = self.target;
            return false;
        }
        let k = 1.0 - (-dt.max(0.0) * 16.0).exp();
        self.scroll += diff * k;
        if (self.target - self.scroll).abs() < 0.5 {
            self.scroll = self.target;
            return false;
        }
        true
    }

    /// True while the scroll animation runs.
    pub fn animating(&self) -> bool {
        (self.target - self.scroll).abs() >= 0.5
    }

    /// Forget tracking so the next frame jumps to the current item.
    pub fn reset(&mut self) {
        self.tracked = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(count: usize) -> GalleryLayout {
        let m = Metrics::new(1.0);
        GalleryLayout::new(Rect::new(0.0, 700.0, 800.0, m.gallery_h()), count, &m)
    }

    #[test]
    fn sizes_step_through_the_range() {
        use GallerySize::*;
        assert_eq!(GallerySize::default(), Medium);
        assert_eq!(GallerySize::ALL.map(GallerySize::cell), [64.0, 96.0, 128.0]);
        assert_eq!(
            GallerySize::ALL.map(GallerySize::larger),
            [Some(Medium), Some(Large), None]
        );
        assert_eq!(
            GallerySize::ALL.map(GallerySize::smaller),
            [None, Some(Small), Some(Medium)]
        );
        // The cell side in pixels follows the interface rounding.
        assert_eq!(
            [1.0, 1.25, 1.5, 2.0].map(|s| Medium.cell_px(s)),
            [96, 120, 144, 192]
        );
        assert_eq!(Small.cell_px(f32::NAN), 64, "a bad scale counts as 1");
    }

    #[test]
    fn few_items_are_centered_and_do_not_scroll() {
        let l = layout(3);
        assert_eq!(l.max_scroll(), 0.0);
        let first = l.item_rect(0, 0.0);
        let last = l.item_rect(2, 0.0);
        let left_gap = first.x - l.panel.x;
        let right_gap = l.panel.right() - last.right();
        assert!((left_gap - right_gap).abs() <= 1.0);
        assert_eq!(l.visible_range(0.0), 0..3);
    }

    #[test]
    fn visible_range_matches_item_rects() {
        let l = layout(1000);
        for scroll in [0.0, 13.0, 700.0, l.max_scroll()] {
            let range = l.visible_range(scroll);
            for i in 0..1000 {
                let r = l.item_rect(i, scroll);
                let visible = r.right() > l.panel.x && r.x < l.panel.right();
                assert_eq!(range.contains(&i), visible, "item {i} at scroll {scroll}");
            }
        }
    }

    #[test]
    fn badge_sits_in_the_cell_corner_at_every_scale() {
        let sizes = GallerySize::ALL.into_iter();
        for (size, scale) in sizes.flat_map(|g| [1.0, 1.25, 1.5, 2.0, 3.0].map(|s| (g, s))) {
            let m = Metrics::new(scale).with_gallery(size);
            let l = GalleryLayout::new(Rect::new(0.0, 700.0, 2000.0, m.gallery_h()), 40, &m);
            let cell = l.item_rect(3, 0.0);
            let b = l.badge_rect(cell);
            assert_eq!((b.w, b.h), (m.thumb_badge(), m.thumb_badge()));
            assert_eq!((b.x - cell.x, b.y - cell.y), (m.px(4.0), m.px(4.0)));
            assert!(b.right() <= cell.right() && b.bottom() <= cell.bottom());
        }
    }

    #[test]
    fn scroll_to_show_brings_item_inside() {
        let l = layout(1000);
        for i in [0usize, 5, 37, 500, 999] {
            let s = l.scroll_to_show(i, 0.0);
            let r = l.item_rect(i, s);
            assert!(r.x >= l.panel.x && r.right() <= l.panel.right(), "item {i}");
        }
    }
}

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
//!
//! A thin position track sits in the padding below the cells, in the middle
//! of the strip. Its small marker shows where the current item is in the
//! whole list: the first item puts it at the left end, the last at the
//! right end ([`marker_fraction`]). When the current item or the count
//! changes, the marker slides there with the same ease as the scrolling.
//! The track takes no extra height and does not show for a single item.

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
    /// The position track below the cells.
    pub track: Rect,
    /// Width and height of the marker on the track.
    pub marker: (f32, f32),
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
        // The track goes in the middle of the space between the ring
        // around the current cell and the bottom of the panel.
        let below = strip.bottom() + m.current_ring();
        let room = (panel.bottom() - below).max(0.0);
        let marker_h = m.position_marker_h().min(room);
        let track_h = m.position_track_h().min(marker_h);
        let marker_y = (below + (room - marker_h) * 0.5).floor();
        let track_y = marker_y + ((marker_h - track_h) * 0.5).floor();
        let track_w = (panel.w * 0.3)
            .round()
            .clamp(m.px(80.0), m.px(400.0))
            .min(panel.w - 2.0 * pad)
            .max(0.0);
        let track_x = panel.x + ((panel.w - track_w) * 0.5).round();
        let marker_w = m.position_marker_w().min((track_w * 0.25).round());
        Self {
            panel,
            strip,
            cell,
            pitch,
            count,
            origin,
            badge: m.thumb_badge().min(cell),
            badge_inset: m.thumb_badge_inset(),
            track: Rect::new(track_x, track_y, track_w, track_h),
            marker: (marker_w, marker_h),
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

    /// How far the marker moves from one end of the track to the other.
    pub fn marker_travel(&self) -> f32 {
        (self.track.w - self.marker.0).max(0.0)
    }

    /// The marker at `fraction` of the track (0 left end, 1 right end),
    /// snapped to whole pixels.
    pub fn marker_rect(&self, fraction: f32) -> Rect {
        let f = if fraction.is_finite() {
            fraction.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let (w, h) = self.marker;
        let x = (self.track.x + f * self.marker_travel()).round();
        let y = self.track.y - ((h - self.track.h) * 0.5).floor();
        Rect::new(x, y, w, h)
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

/// Where item `index` of `count` sits on the position track: 0 for the
/// first item, 1 for the last, evenly between. `None` for fewer than two
/// items: the track does not show then. An index past the end counts as
/// the last item.
pub(crate) fn marker_fraction(index: usize, count: usize) -> Option<f32> {
    if count < 2 {
        return None;
    }
    let last = count - 1;
    Some(index.min(last) as f32 / last as f32)
}

/// The position marker: where it is drawn and where it moves to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Marker {
    /// Current (animated) place, as a fraction of the track.
    pub at: f32,
    /// Place the animation moves toward.
    pub target: f32,
    /// Track travel in pixels, to stop the animation within half a pixel.
    pub travel: f32,
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
    /// The position marker, `None` while the track does not show.
    pub marker: Option<Marker>,
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

    /// Point the position marker at item `index` of `count` on the track
    /// of `layout`. A marker that was not shown starts there; a shown one
    /// moves there with [`GalleryState::animate`]. Fewer than two items
    /// hide it.
    pub fn mark(&mut self, layout: &GalleryLayout, index: usize, count: usize) {
        let Some(target) = marker_fraction(index, count) else {
            self.marker = None;
            return;
        };
        let travel = layout.marker_travel();
        self.marker = Some(match self.marker {
            Some(m) => Marker {
                target,
                travel,
                ..m
            },
            None => Marker {
                at: target,
                target,
                travel,
            },
        });
    }

    /// Advance the scroll and marker animations by `dt` seconds. Returns
    /// true while one is still moving.
    pub fn animate(&mut self, dt: f32) -> bool {
        let k = 1.0 - (-dt.max(0.0) * 16.0).exp();
        let diff = self.target - self.scroll;
        if diff.abs() < 0.5 {
            self.scroll = self.target;
        } else {
            self.scroll += diff * k;
            if (self.target - self.scroll).abs() < 0.5 {
                self.scroll = self.target;
            }
        }
        if let Some(m) = &mut self.marker {
            m.at += (m.target - m.at) * k;
            if (m.target - m.at).abs() * m.travel < 0.5 {
                m.at = m.target;
            }
        }
        self.animating()
    }

    /// True while the scroll or marker animation runs.
    pub fn animating(&self) -> bool {
        let marker = self
            .marker
            .is_some_and(|m| (m.target - m.at).abs() * m.travel >= 0.5);
        (self.target - self.scroll).abs() >= 0.5 || marker
    }

    /// Forget tracking so the next frame jumps to the current item, and
    /// hide the marker so it starts at its place when it shows again.
    pub fn reset(&mut self) {
        self.tracked = None;
        self.marker = None;
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
    fn marker_fraction_spreads_items_over_the_track() {
        assert_eq!(marker_fraction(0, 0), None);
        assert_eq!(marker_fraction(0, 1), None, "one item: no track");
        assert_eq!(marker_fraction(0, 2), Some(0.0));
        assert_eq!(marker_fraction(1, 2), Some(1.0));
        assert_eq!(marker_fraction(0, 101), Some(0.0));
        assert_eq!(marker_fraction(50, 101), Some(0.5));
        assert_eq!(marker_fraction(100, 101), Some(1.0));
        assert_eq!(marker_fraction(500, 101), Some(1.0), "past the end");
    }

    #[test]
    fn marker_stays_in_the_bottom_padding_at_every_size_and_scale() {
        let sizes = GallerySize::ALL.into_iter();
        for (size, scale) in sizes.flat_map(|g| [1.0, 1.25, 1.5, 2.0, 3.0].map(|s| (g, s))) {
            let m = Metrics::new(scale).with_gallery(size);
            let panel = Rect::new(0.0, 700.0, 1200.0, m.gallery_h());
            let l = GalleryLayout::new(panel, 40, &m);
            let why = format!("{size:?} at {scale}");
            let ring = l.strip.bottom() + m.current_ring();
            for f in [0.0, 0.5, 1.0] {
                let r = l.marker_rect(f);
                assert!(r.y >= ring && r.bottom() <= panel.bottom(), "{why}: {r:?}");
                assert!(r.x >= l.track.x && r.right() <= l.track.right(), "{why}");
                assert!(r.y <= l.track.y && r.bottom() >= l.track.bottom(), "{why}");
            }
            assert_eq!(l.marker_rect(0.0).x, l.track.x, "{why}");
            assert_eq!(l.marker_rect(1.0).right(), l.track.right(), "{why}");
            let mid = l.marker_rect(0.5).center().x - l.track.center().x;
            assert!(mid.abs() <= 1.0, "{why}: middle off by {mid}");
        }
    }

    #[test]
    fn marker_slides_to_a_new_place_and_stops() {
        let l = layout(11);
        let mut g = GalleryState::default();
        g.mark(&l, 0, 11);
        assert_eq!(g.marker.map(|m| m.at), Some(0.0), "starts at its place");
        assert!(!g.animating());
        g.mark(&l, 10, 11);
        assert!(g.animating());
        g.animate(0.016);
        let at = g.marker.unwrap().at;
        assert!(at > 0.0 && at < 1.0, "{at}");
        for _ in 0..60 {
            g.animate(0.016);
        }
        assert_eq!(g.marker.unwrap().at, 1.0);
        assert!(!g.animating());
        g.mark(&l, 0, 1);
        assert_eq!(g.marker, None, "one item hides it");
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

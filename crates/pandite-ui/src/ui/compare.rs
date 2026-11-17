//! The comparison of selected images: one image against the image on
//! screen with a slider, or two to eight images in tiles.

use super::{Target, Ui};
use crate::context::{CompareItem, CompareKind, CompareState, UiContext};
use crate::metrics::Metrics;
use crate::paint;
use crate::theme::Theme;
use pandite_core::{DrawList, Point, Rect, TextMeasure};

/// The most images a comparison shows.
pub(crate) const COMPARE_MAX: usize = 8;

/// The side of the magnifier square, in logical pixels.
pub(crate) const PEEK_SIDE: f32 = 280.0;

/// The tip in the selection box while no comparison runs.
pub(crate) const COMPARE_HINT: &str = "Compare with D";
/// The tip in the selection box while a comparison runs.
pub(crate) const COMPARE_END_HINT: &str = "Esc to end compare";
/// The tip in the selection box when too many images are selected.
pub(crate) const COMPARE_MANY_HINT: &str = "Compare: 8 images or fewer";

/// Where the application draws one image of a comparison.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ComparePane {
    /// The index of the image in [`crate::CompareView::items`], or `None`
    /// for the image on screen.
    pub item: Option<usize>,
    /// The area the view places the image in.
    pub viewport: Rect,
    /// The part of the window the image may draw to.
    pub clip: Rect,
}

/// One square of the magnifier: a picture drawn again, enlarged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PeekLens {
    /// The picture: its index in [`crate::CompareView::items`], or `None`
    /// for the image on screen.
    pub item: Option<usize>,
    /// The square. Draw the picture enlarged so that `source` lands on
    /// its center.
    pub rect: Rect,
    /// The window point of the picture, as drawn without the magnifier,
    /// that the square is about.
    pub source: Point,
    /// The square is cut to this area.
    pub bounds: Rect,
}

/// The tiles of `n` images in `area`, in reading order. The grid is the
/// one that shows a 3:2 picture largest; a last row with fewer images
/// shares the whole width.
pub(crate) fn tile_rects(area: Rect, n: usize, gap: f32) -> Vec<Rect> {
    if n == 0 || area.is_empty() {
        return Vec::new();
    }
    let mut best = (1usize, 0.0f32);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let (w, h) = (area.w / cols as f32, area.h / rows as f32);
        let score = (w / 1.5).min(h);
        if score >= best.1 {
            best = (cols, score);
        }
    }
    let cols = best.0;
    let rows = n.div_ceil(cols);
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let row = i / cols;
        let in_row = if row + 1 == rows {
            n - row * cols
        } else {
            cols
        };
        let col = i - row * cols;
        let x0 = (area.x + area.w * col as f32 / in_row as f32).round();
        let x1 = (area.x + area.w * (col + 1) as f32 / in_row as f32).round();
        let y0 = (area.y + area.h * row as f32 / rows as f32).round();
        let y1 = (area.y + area.h * (row + 1) as f32 / rows as f32).round();
        // Half a gap on every inner edge.
        let left = if col == 0 { 0.0 } else { (gap * 0.5).floor() };
        let right = if col + 1 == in_row {
            0.0
        } else {
            (gap * 0.5).ceil()
        };
        let top = if row == 0 { 0.0 } else { (gap * 0.5).floor() };
        let bottom = if row + 1 == rows {
            0.0
        } else {
            (gap * 0.5).ceil()
        };
        out.push(Rect::new(
            x0 + left,
            y0 + top,
            (x1 - x0 - left - right).max(0.0),
            (y1 - y0 - top - bottom).max(0.0),
        ));
    }
    out
}

impl Ui {
    /// Where each image of the comparison draws, in drawing order. Empty
    /// without a comparison. Draw the panes in place of the single image.
    pub fn compare_panes(&self, ctx: &UiContext<'_>) -> Vec<ComparePane> {
        let Some(compare) = ctx.compare else {
            return Vec::new();
        };
        let area = self.content_rect(ctx);
        let m = Metrics::of(ctx);
        match compare.kind {
            CompareKind::Slider => {
                let x = self.split_x(ctx).unwrap_or(area.x);
                vec![
                    ComparePane {
                        item: Some(0),
                        viewport: area,
                        clip: Rect::new(area.x, area.y, (x - area.x).max(0.0), area.h),
                    },
                    ComparePane {
                        item: None,
                        viewport: area,
                        clip: Rect::new(x, area.y, (area.right() - x).max(0.0), area.h),
                    },
                ]
            }
            CompareKind::Tiles => tile_rects(area, compare.items.len(), m.px(4.0))
                .into_iter()
                .enumerate()
                .map(|(i, r)| ComparePane {
                    item: Some(i),
                    viewport: r,
                    clip: r,
                })
                .collect(),
        }
    }

    /// The window x of the slider line, in a slider comparison.
    pub(crate) fn split_x(&self, ctx: &UiContext<'_>) -> Option<f32> {
        let compare = ctx.compare?;
        if compare.kind != CompareKind::Slider {
            return None;
        }
        let area = self.content_rect(ctx);
        Some((area.x + area.w * self.compare_split.clamp(0.0, 1.0)).round())
    }

    /// True when `p` is close enough to the slider line to take it.
    pub(crate) fn on_split(&self, p: Point, ctx: &UiContext<'_>, m: &Metrics) -> bool {
        self.split_x(ctx)
            .is_some_and(|x| (p.x - x).abs() <= m.px(10.0) && self.content_rect(ctx).contains(p))
    }

    /// Move the slider line to window x `x`.
    pub(crate) fn set_split(&mut self, x: f32, ctx: &UiContext<'_>) {
        let area = self.content_rect(ctx);
        if area.w > 0.0 {
            self.compare_split = ((x - area.x) / area.w).clamp(0.0, 1.0);
        }
    }

    /// True in a tile comparison.
    pub(crate) fn comparing_tiles(ctx: &UiContext<'_>) -> bool {
        ctx.compare.is_some_and(|c| c.kind == CompareKind::Tiles)
    }

    /// The tile under `p`, in a tile comparison.
    pub(crate) fn compare_tile_at(&self, p: Point, ctx: &UiContext<'_>) -> Option<usize> {
        if !Self::comparing_tiles(ctx) {
            return None;
        }
        self.compare_panes(ctx)
            .iter()
            .position(|pane| pane.clip.contains(p))
    }

    /// True while the magnifier is on and there is a picture to magnify.
    pub(crate) fn peek_shown(&self, ctx: &UiContext<'_>) -> bool {
        self.peek && self.crop.is_none() && (ctx.image.is_some() || ctx.compare.is_some())
    }

    /// The squares of the magnifier while the pointer is over the image
    /// area. A single image has one, centered on the pointer. A slider
    /// has two side by side at the pointer, one per picture, both showing
    /// the spot under the pointer. Tiles have one in every tile, at the
    /// spot that matches the pointer's place in its own tile.
    pub fn peek_lenses(&self, ctx: &UiContext<'_>) -> Vec<PeekLens> {
        if !self.peek_shown(ctx) || self.hover != Target::Content {
            return Vec::new();
        }
        let Some(p) = self.pointer else {
            return Vec::new();
        };
        let m = Metrics::of(ctx);
        let side = m.px(PEEK_SIDE);
        let area = self.content_rect(ctx);
        let square = |center: Point| {
            Rect::new(
                (center.x - side * 0.5).round(),
                (center.y - side * 0.5).round(),
                side,
                side,
            )
        };
        match ctx.compare.map(|c| c.kind) {
            None => vec![PeekLens {
                item: None,
                rect: square(p),
                source: p,
                bounds: area,
            }],
            Some(CompareKind::Slider) => {
                let half = (side + m.px(6.0)) * 0.5;
                [(Some(0), -half), (None, half)]
                    .into_iter()
                    .map(|(item, dx)| PeekLens {
                        item,
                        rect: square(Point::new(p.x + dx, p.y)),
                        source: p,
                        bounds: area,
                    })
                    .collect()
            }
            Some(CompareKind::Tiles) => {
                let panes = self.compare_panes(ctx);
                let Some(under) = panes.iter().find(|pane| pane.clip.contains(p)) else {
                    return Vec::new();
                };
                let from = under.viewport.center();
                panes
                    .iter()
                    .map(|pane| {
                        let to = pane.viewport.center();
                        let spot = Point::new(p.x + to.x - from.x, p.y + to.y - from.y);
                        PeekLens {
                            item: pane.item,
                            rect: square(spot),
                            source: spot,
                            bounds: pane.clip,
                        }
                    })
                    .collect()
            }
        }
    }

    /// The frames of the magnifier squares.
    pub(crate) fn draw_peek(
        &self,
        ctx: &UiContext<'_>,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        for lens in self.peek_lenses(ctx) {
            let Some(r) = lens.rect.intersect(lens.bounds) else {
                continue;
            };
            out.border(paint::outset(r, m.px(1.0)), t.check_line, m.px(3.0), 0.0);
            out.border(r, t.check_fill, m.px(1.0), 0.0);
        }
    }

    /// The tip of the selection box about comparing.
    pub(crate) fn compare_hint(ctx: &UiContext<'_>) -> &'static str {
        if ctx.compare.is_some() {
            COMPARE_END_HINT
        } else if ctx.selected.len() > COMPARE_MAX {
            COMPARE_MANY_HINT
        } else {
            COMPARE_HINT
        }
    }

    /// The label of one compared image, with its state.
    fn compare_label(item: &CompareItem) -> String {
        match item.state {
            CompareState::Ready => item.name.clone(),
            CompareState::Loading => format!("{} - loading", item.name),
            CompareState::Failed => format!("{} - cannot open", item.name),
        }
    }

    /// A file name in a small box at an edge of `area`: at the left when
    /// `left` is true, at the right when it is false, in the middle
    /// without it; at the bottom when `bottom`, else at the top.
    #[allow(clippy::too_many_arguments)]
    fn draw_compare_label(
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        area: Rect,
        label: &str,
        left: Option<bool>,
        bottom: bool,
        out: &mut DrawList,
    ) {
        let font = m.text_small();
        let (pad, margin, h) = (m.marked_pad(), m.px(8.0), m.marked_h());
        let max_w = area.w - 2.0 * margin - 2.0 * pad;
        if max_w < m.px(24.0) || area.h < h + 2.0 * margin {
            return;
        }
        let text = paint::elide_middle(measure, label, font, max_w);
        let w = (measure.measure(&text, font).w + 2.0 * pad).ceil();
        let x = match left {
            Some(true) => area.x + margin,
            Some(false) => area.right() - margin - w,
            None => area.x + (area.w - w) * 0.5,
        };
        let y = if bottom {
            area.bottom() - margin - h
        } else {
            area.y + margin
        };
        let r = Rect::new(x.round(), y.round(), w, h);
        out.rect(r, t.panel, 0.0);
        out.border(r, t.border, m.hairline(), 0.0);
        paint::text_center(out, measure, r, &text, font, t.text);
    }

    /// The comparison: the slider line with the two file names, or the
    /// outline and the file name of each tile.
    pub(crate) fn draw_compare(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        let Some(compare) = ctx.compare else { return };
        match compare.kind {
            CompareKind::Slider => {
                let Some(x) = self.split_x(ctx) else { return };
                // Below the toolbar, so the names stay readable under it.
                let area = Rect::new(
                    content.x,
                    content.y + m.toolbar_h(),
                    content.w,
                    (content.h - m.toolbar_h()).max(0.0),
                );
                if let Some(item) = compare.items.first() {
                    let label = Self::compare_label(item);
                    Self::draw_compare_label(measure, m, t, area, &label, Some(true), false, out);
                }
                if let Some(image) = ctx.image {
                    Self::draw_compare_label(
                        measure,
                        m,
                        t,
                        area,
                        image.file_name,
                        Some(false),
                        false,
                        out,
                    );
                }
                // A dark line with a light core shows on every picture.
                let (core, edge) = (m.px(2.0), m.px(1.0));
                out.rect(
                    Rect::new(
                        x - core * 0.5 - edge,
                        content.y,
                        core + 2.0 * edge,
                        content.h,
                    ),
                    t.check_line,
                    0.0,
                );
                out.rect(
                    Rect::new(x - core * 0.5, content.y, core, content.h),
                    t.check_fill,
                    0.0,
                );
                let (gw, gh) = (m.px(14.0), m.px(36.0));
                let grip = Rect::new(
                    (x - gw * 0.5).round(),
                    (content.y + (content.h - gh) * 0.5).round(),
                    gw,
                    gh,
                );
                out.rect(grip, t.check_fill, gw * 0.5);
                out.border(grip, t.check_line, m.px(1.5), gw * 0.5);
            }
            CompareKind::Tiles => {
                // The tile of the current image stands out: Space unmarks
                // it, and the arrow keys move on.
                for (pane, item) in self.compare_panes(ctx).iter().zip(compare.items) {
                    let r = pane.clip;
                    let hot = ctx.count > 0 && item.index == Some(ctx.index);
                    let line = if hot { m.px(2.0) } else { m.hairline() };
                    out.border(r, if hot { t.accent } else { t.border }, line, 0.0);
                    let label = Self::compare_label(item);
                    // In the middle, clear of the boxes in the bottom corners.
                    Self::draw_compare_label(measure, m, t, r, &label, None, true, out);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiles_fill_the_area_without_overlap() {
        let area = Rect::new(0.0, 0.0, 1200.0, 800.0);
        for n in 1..=COMPARE_MAX {
            let tiles = tile_rects(area, n, 4.0);
            assert_eq!(tiles.len(), n);
            for (i, a) in tiles.iter().enumerate() {
                assert!(a.w > 100.0 && a.h > 100.0, "{n} tiles: tile {i} is {a:?}");
                assert!(a.x >= 0.0 && a.right() <= 1200.0 && a.y >= 0.0 && a.bottom() <= 800.0);
                for b in &tiles[i + 1..] {
                    assert!(a.intersect(*b).is_none(), "{n} tiles: {a:?} meets {b:?}");
                }
            }
        }
        // Two images sit side by side in a wide window, and one above the
        // other in a tall one.
        let wide = tile_rects(area, 2, 4.0);
        assert_eq!(wide[0].y, wide[1].y);
        let tall = tile_rects(Rect::new(0.0, 0.0, 500.0, 1200.0), 2, 4.0);
        assert_eq!(tall[0].x, tall[1].x);
    }
}

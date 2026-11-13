//! Recognized text on screen: where each word lands, which word the
//! pointer is on, the highlight bands of a selection and its text.
//!
//! Word outlines arrive in pixels of the image the text was read from.
//! [`TextGeom`] carries them through the view edits made since
//! ([`crate::TextView::to_image`]) and then through the same placement the
//! image is drawn with ([`crate::ImageInfo::screen_rect`]), so highlights
//! and hit tests follow zoom, pan, quarter turns, mirrors and crops
//! exactly. A word whose center a crop cut away is not on the image any
//! more: it is never hit, highlighted or copied. All work here is in
//! window pixels.

use crate::context::UiContext;
use pandite_core::{Affine, Point, Quad, Rect, TextLayout};
use std::ops::Range;

/// A text selection: every word in reading order from `anchor` to `focus`
/// (either order), for the layout named `id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextSelection {
    pub id: u64,
    pub anchor: usize,
    pub focus: usize,
}

impl TextSelection {
    /// The first and last selected word index.
    pub fn bounds(&self) -> (usize, usize) {
        (self.anchor.min(self.focus), self.anchor.max(self.focus))
    }
}

/// The recognized words of the current frame, placed on screen.
pub(crate) struct TextGeom<'a> {
    pub layout: &'a TextLayout,
    pub id: u64,
    to_screen: Affine,
    /// Per word: its center lies on the displayed image.
    visible: Vec<bool>,
    /// Where the image is drawn, in window pixels.
    pub image_rect: Rect,
}

impl<'a> TextGeom<'a> {
    /// The geometry for `ctx`, when an image with recognized text is on
    /// screen and at least one word lies on it.
    pub fn new(ctx: &UiContext<'a>) -> Option<Self> {
        let text = ctx.text?;
        let info = ctx.image?;
        if info.width == 0 || info.height == 0 || info.screen_rect.is_empty() {
            return None;
        }
        let (w, h) = (info.width as f32, info.height as f32);
        let visible: Vec<bool> = text
            .layout
            .words()
            .map(|(_, word)| {
                let c = text.to_image.apply(word.quad.center());
                c.x >= 0.0 && c.y >= 0.0 && c.x <= w && c.y <= h
            })
            .collect();
        if !visible.contains(&true) {
            return None;
        }
        let image = Rect::new(0.0, 0.0, w, h);
        let to_screen = text
            .to_image
            .then(&Affine::rect_to_rect(image, info.screen_rect));
        Some(Self {
            layout: text.layout,
            id: text.id,
            to_screen,
            visible,
            image_rect: info.screen_rect,
        })
    }

    /// The first and last word on the image, for "select all".
    pub fn visible_bounds(&self) -> Option<(usize, usize)> {
        let first = self.visible.iter().position(|v| *v)?;
        let last = self.visible.iter().rposition(|v| *v)?;
        Some((first, last))
    }

    /// The first and last word on the image of the line holding word
    /// `index`, for a triple click.
    pub fn line_bounds(&self, index: usize) -> Option<(usize, usize)> {
        let range = self.layout.line_range(self.layout.line_of(index)?);
        let first = range.clone().find(|&i| self.visible[i])?;
        let last = range.rev().find(|&i| self.visible[i])?;
        Some((first, last))
    }

    /// Word quads on screen, in reading order, with their index and line;
    /// words off the image are skipped.
    fn screen_words(&self) -> impl Iterator<Item = (usize, usize, Quad)> + '_ {
        self.layout
            .words()
            .enumerate()
            .filter(|(i, _)| self.visible[*i])
            .map(|(i, (line, w))| (i, line, w.quad.map(|p| self.to_screen.apply(p))))
    }

    /// One band covering `quads` (words of a single line), on screen: a
    /// rectangle along the line's direction from the first word's start to
    /// the last word's end and over the full height of the words.
    fn band(quads: &[Quad]) -> Option<Quad> {
        let first = quads.first()?;
        // The reading direction: along the first word's top edge.
        let (dx, dy) = (
            first.points[1].x - first.points[0].x,
            first.points[1].y - first.points[0].y,
        );
        let len = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = if len > 1e-6 && len.is_finite() {
            (dx / len, dy / len)
        } else {
            (1.0, 0.0)
        };
        let (vx, vy) = (-uy, ux);
        let (mut u0, mut u1, mut v0, mut v1) = (
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
        );
        for q in quads {
            for p in q.points {
                let u = p.x * ux + p.y * uy;
                let v = p.x * vx + p.y * vy;
                u0 = u0.min(u);
                u1 = u1.max(u);
                v0 = v0.min(v);
                v1 = v1.max(v);
            }
        }
        if !(u0.is_finite() && u1.is_finite() && v0.is_finite() && v1.is_finite()) {
            return None;
        }
        let at = |u: f32, v: f32| Point::new(u * ux + v * vx, u * uy + v * vy);
        Some(Quad::new([at(u0, v0), at(u1, v0), at(u1, v1), at(u0, v1)]))
    }

    /// The words of `range` on screen, grouped by line: `(line, quads,
    /// word indices)` in reading order.
    fn lines_in(&self, range: Range<usize>) -> Vec<(usize, Vec<Quad>, Vec<usize>)> {
        let mut out: Vec<(usize, Vec<Quad>, Vec<usize>)> = Vec::new();
        for (i, line, q) in self.screen_words() {
            if !range.contains(&i) {
                continue;
            }
            match out.last_mut() {
                Some((l, quads, indices)) if *l == line => {
                    quads.push(q);
                    indices.push(i);
                }
                _ => out.push((line, vec![q], vec![i])),
            }
        }
        out
    }

    /// The word under `p`: a point on a word or in the gap between two
    /// words of one line, within `slop` pixels, picks the nearest word of
    /// the nearest such line.
    pub fn word_at(&self, p: Point, slop: f32) -> Option<usize> {
        let mut best: Option<(f32, Vec<usize>, Vec<Quad>)> = None;
        for (_, quads, indices) in self.lines_in(0..self.visible.len()) {
            let Some(band) = Self::band(&quads) else {
                continue;
            };
            let d = band.distance(p);
            if d <= slop && best.as_ref().is_none_or(|(bd, _, _)| d < *bd) {
                best = Some((d, indices, quads));
            }
        }
        let (_, indices, quads) = best?;
        nearest_of(p, indices.into_iter().zip(quads))
    }

    /// The word on the image nearest to `p`.
    pub fn nearest(&self, p: Point) -> Option<usize> {
        nearest_of(p, self.screen_words().map(|(i, _, q)| (i, q)))
    }

    /// The highlight bands of the words `first..=last` on the image: one
    /// per line.
    pub fn bands(&self, first: usize, last: usize) -> Vec<Quad> {
        self.lines_in(first..last.saturating_add(1))
            .into_iter()
            .filter_map(|(_, quads, _)| Self::band(&quads))
            .collect()
    }

    /// The words `first..=last` on the image as text: words of one line
    /// joined with a single space, lines with `"\n"`.
    pub fn text_of(&self, first: usize, last: usize) -> String {
        let words: Vec<&str> = self.layout.words().map(|(_, w)| w.text.as_str()).collect();
        self.lines_in(first..last.saturating_add(1))
            .into_iter()
            .map(|(_, _, indices)| {
                indices
                    .iter()
                    .map(|&i| words[i])
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The index of the quad nearest to `p` (the first of equals).
fn nearest_of(p: Point, words: impl Iterator<Item = (usize, Quad)>) -> Option<usize> {
    let mut best: Option<(f32, usize)> = None;
    for (i, q) in words {
        let d = q.distance(p);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, i));
        }
    }
    best.map(|(_, i)| i)
}

/// The axis-aligned rectangle a quad covers when its sides run along the
/// screen axes (within a hundredth of a pixel), otherwise `None`.
pub(crate) fn as_rect(q: &Quad) -> Option<Rect> {
    let straight = (0..4).all(|i| {
        let a = q.points[i];
        let b = q.points[(i + 1) % 4];
        (a.x - b.x).abs() < 0.01 || (a.y - b.y).abs() < 0.01
    });
    straight.then(|| q.bounds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ImageInfo, TextView};
    use pandite_core::{TextLine, TextWord, ViewState};

    fn word(text: &str, x: f32, y: f32) -> TextWord {
        TextWord {
            text: text.into(),
            quad: Quad::from_rect(Rect::new(x, y, 40.0, 10.0)),
        }
    }

    fn layout() -> TextLayout {
        TextLayout {
            width: 200,
            height: 100,
            lines: vec![
                TextLine {
                    words: vec![word("a", 10.0, 10.0), word("b", 60.0, 10.0)],
                },
                TextLine {
                    words: vec![word("c", 10.0, 50.0), word("d", 60.0, 52.0)],
                },
            ],
        }
    }

    fn ctx<'a>(layout: &'a TextLayout, to_image: Affine, w: u32, h: u32) -> UiContext<'a> {
        UiContext {
            image: Some(ImageInfo {
                file_name: "t.png",
                width: w,
                height: h,
                view: ViewState::default(),
                zoom_percent: 200.0,
                screen_rect: Rect::new(100.0, 100.0, w as f32 * 2.0, h as f32 * 2.0),
                edited: false,
            }),
            text: Some(TextView {
                layout,
                to_image,
                id: 1,
            }),
            ..UiContext::default()
        }
    }

    #[test]
    fn words_land_where_the_image_is_drawn() {
        let l = layout();
        let c = ctx(&l, Affine::IDENTITY, 200, 100);
        let g = TextGeom::new(&c).unwrap();
        // Word "a" covers image (10..50, 10..20): screen (120..200, 120..140).
        assert_eq!(g.word_at(Point::new(130.0, 130.0), 2.0), Some(0));
        // The gap between "a" and "b" belongs to the line; "b" is nearer.
        assert_eq!(g.word_at(Point::new(212.0, 130.0), 2.0), Some(1));
        assert_eq!(
            g.word_at(Point::new(130.0, 170.0), 2.0),
            None,
            "between lines"
        );
        assert_eq!(g.nearest(Point::new(500.0, 500.0)), Some(3));
        let bands = g.bands(1, 2);
        assert_eq!(bands.len(), 2);
        assert_eq!(
            as_rect(&bands[0]),
            Some(Rect::new(220.0, 120.0, 80.0, 20.0))
        );
        // Line two spans both words and their different heights.
        let all = g.bands(0, 3);
        assert_eq!(as_rect(&all[1]), Some(Rect::new(120.0, 200.0, 180.0, 24.0)));
        assert_eq!(g.text_of(1, 3), "b\nc d");
        assert_eq!(g.visible_bounds(), Some((0, 3)));
    }

    #[test]
    fn a_quarter_turn_moves_and_turns_the_bands() {
        let l = layout();
        // Rotated clockwise: the displayed image is 100 x 200 and
        // (x, y) -> (100 - y, x).
        let turn = Affine::new(0.0, 1.0, -1.0, 0.0, 100.0, 0.0);
        let c = ctx(&l, turn, 100, 200);
        let g = TextGeom::new(&c).unwrap();
        let band = g.bands(0, 0)[0];
        // Image (10..50, 10..20) turns to (80..90, 10..50), on screen
        // (260..280, 120..200).
        assert_eq!(as_rect(&band), Some(Rect::new(260.0, 120.0, 20.0, 80.0)));
        assert_eq!(g.word_at(Point::new(270.0, 150.0), 1.0), Some(0));
        // A slanted layout gives a band that is not axis aligned.
        let slant = Affine::new(0.9, 0.3, -0.3, 0.9, 0.0, 0.0);
        let c = ctx(&l, slant, 200, 100);
        let g = TextGeom::new(&c).unwrap();
        assert!(as_rect(&g.bands(0, 1)[0]).is_none());
    }

    #[test]
    fn words_cut_away_by_a_crop_are_gone() {
        let l = layout();
        // Cropped to (55, 0) and 145 x 100: only "b" and "d" remain.
        let c = ctx(&l, Affine::translate(-55.0, 0.0), 145, 100);
        let g = TextGeom::new(&c).unwrap();
        assert_eq!(g.visible_bounds(), Some((1, 3)));
        assert_eq!(g.line_bounds(1), Some((1, 1)));
        assert_eq!(g.text_of(0, 3), "b\nd");
        assert_eq!(g.bands(0, 3).len(), 2);
        assert_eq!(g.nearest(Point::new(0.0, 0.0)), Some(1));
        // A crop that keeps no word leaves no text at all.
        let c = ctx(&l, Affine::translate(-150.0, 0.0), 50, 100);
        assert!(TextGeom::new(&c).is_none());
    }
}

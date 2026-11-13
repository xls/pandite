//! Recognized text and the text recognition provider trait.
//!
//! A [`TextProvider`] reads the words in an [`ImageBuf`] and returns a
//! [`TextLayout`]: lines in reading order, each a list of words in reading
//! order, each word with its outline as a [`Quad`] in the pixel coordinates
//! of the image it read. That image is the upright one the viewer displays
//! (orientation already applied), at full resolution: a provider that
//! scales the image down for its engine maps the outlines back.
//!
//! Providers are compiled in and registered in [`crate::Registry`] beside
//! the decoders. A platform without a recognizer registers none.

use crate::error::Result;
use crate::geom::{Point, Rect};
use crate::image::ImageBuf;
use crate::provider::CancelToken;
use std::ops::Range;

/// Four corners of a word, in the order the text reads: top-left,
/// top-right, bottom-right, bottom-left. On slanted text the quad is a
/// rotated rectangle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    /// The corners in reading order around the outline.
    pub points: [Point; 4],
}

impl Quad {
    /// A quad from its corners: top-left, top-right, bottom-right,
    /// bottom-left as the text reads.
    pub const fn new(points: [Point; 4]) -> Self {
        Self { points }
    }

    /// The corners of an axis-aligned rectangle.
    pub fn from_rect(r: Rect) -> Self {
        Self::new([
            Point::new(r.x, r.y),
            Point::new(r.right(), r.y),
            Point::new(r.right(), r.bottom()),
            Point::new(r.x, r.bottom()),
        ])
    }

    /// Every corner moved by `f`.
    pub fn map(&self, f: impl Fn(Point) -> Point) -> Quad {
        Quad::new(self.points.map(f))
    }

    /// The smallest axis-aligned rectangle around the corners.
    pub fn bounds(&self) -> Rect {
        let xs = self.points.map(|p| p.x);
        let ys = self.points.map(|p| p.y);
        let (x0, x1) = (
            xs.iter().copied().fold(f32::INFINITY, f32::min),
            xs.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        );
        let (y0, y1) = (
            ys.iter().copied().fold(f32::INFINITY, f32::min),
            ys.iter().copied().fold(f32::NEG_INFINITY, f32::max),
        );
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// The mean of the corners.
    pub fn center(&self) -> Point {
        let (sx, sy) = self
            .points
            .iter()
            .fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
        Point::new(sx * 0.25, sy * 0.25)
    }

    /// True when `p` lies inside or on the outline. Works for convex quads
    /// of either winding.
    pub fn contains(&self, p: Point) -> bool {
        let mut sign = 0.0f32;
        for i in 0..4 {
            let a = self.points[i];
            let b = self.points[(i + 1) % 4];
            let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            if cross == 0.0 {
                continue;
            }
            if sign == 0.0 {
                sign = cross.signum();
            } else if cross.signum() != sign {
                return false;
            }
        }
        // A collapsed quad (every cross product zero) contains only points
        // on its line; treat it as containing them when inside its bounds.
        sign != 0.0 || {
            let b = self.bounds();
            p.x >= b.x && p.x <= b.right() && p.y >= b.y && p.y <= b.bottom()
        }
    }

    /// The distance from `p` to the quad: 0 inside, otherwise the distance
    /// to the nearest edge.
    pub fn distance(&self, p: Point) -> f32 {
        if self.contains(p) {
            return 0.0;
        }
        (0..4)
            .map(|i| segment_distance(p, self.points[i], self.points[(i + 1) % 4]))
            .fold(f32::INFINITY, f32::min)
    }
}

fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (cx, cy) = (a.x + t * dx, a.y + t * dy);
    ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt()
}

/// One recognized word.
#[derive(Clone, Debug, PartialEq)]
pub struct TextWord {
    /// The characters of the word, without surrounding spaces.
    pub text: String,
    /// The word's outline in pixels of the recognized image.
    pub quad: Quad,
}

/// One line of text: its words in reading order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLine {
    /// The words, in reading order.
    pub words: Vec<TextWord>,
}

impl TextLine {
    /// The words joined with single spaces.
    pub fn text(&self) -> String {
        join_words(&self.words)
    }
}

fn join_words(words: &[TextWord]) -> String {
    let mut out = String::new();
    for (i, w) in words.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&w.text);
    }
    out
}

/// The text found in one image.
///
/// Words are addressed by their position in reading order over the whole
/// layout ("word index"): the first line's words first, then the second
/// line's, and so on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextLayout {
    /// Width in pixels of the image the text was read from.
    pub width: u32,
    /// Height in pixels of the image the text was read from.
    pub height: u32,
    /// The lines, in reading order.
    pub lines: Vec<TextLine>,
}

impl TextLayout {
    /// An empty layout for an image of this size.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            lines: Vec::new(),
        }
    }

    /// Number of words over all lines.
    pub fn word_count(&self) -> usize {
        self.lines.iter().map(|l| l.words.len()).sum()
    }

    /// True when no word was found.
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(|l| l.words.is_empty())
    }

    /// Every word with its line index, in reading order. The position in
    /// this sequence is the word index.
    pub fn words(&self) -> impl Iterator<Item = (usize, &TextWord)> + '_ {
        self.lines
            .iter()
            .enumerate()
            .flat_map(|(i, l)| l.words.iter().map(move |w| (i, w)))
    }

    /// The word indices of line `line`. Empty when the line does not exist.
    pub fn line_range(&self, line: usize) -> Range<usize> {
        let start: usize = self.lines.iter().take(line).map(|l| l.words.len()).sum();
        let len = self.lines.get(line).map_or(0, |l| l.words.len());
        start..start + len
    }

    /// The line that holds word `index`.
    pub fn line_of(&self, index: usize) -> Option<usize> {
        let mut start = 0;
        for (i, l) in self.lines.iter().enumerate() {
            if index < start + l.words.len() {
                return Some(i);
            }
            start += l.words.len();
        }
        None
    }

    /// The words from index `a` to index `b` (inclusive, either order) as
    /// text: words of one line joined with a single space, lines joined
    /// with `"\n"`.
    pub fn text_of(&self, a: usize, b: usize) -> String {
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let mut out = String::new();
        let mut start = 0;
        let mut first = true;
        for line in &self.lines {
            let end = start + line.words.len();
            if end > lo && start <= hi && end > start {
                let from = lo.saturating_sub(start);
                let to = (hi + 1).min(end) - start;
                if !first {
                    out.push('\n');
                }
                first = false;
                out.push_str(&join_words(&line.words[from..to]));
            }
            start = end;
            if start > hi {
                break;
            }
        }
        out
    }

    /// All the text: lines joined with `"\n"`, words with a single space.
    pub fn text(&self) -> String {
        match self.word_count() {
            0 => String::new(),
            n => self.text_of(0, n - 1),
        }
    }

    /// Approximate memory held, for cache budgets.
    pub fn byte_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self
                .lines
                .iter()
                .map(|l| {
                    std::mem::size_of::<TextLine>()
                        + l.words
                            .iter()
                            .map(|w| std::mem::size_of::<TextWord>() + w.text.len())
                            .sum::<usize>()
                })
                .sum::<usize>()
    }
}

/// Reads text in images. Implementations are small objects compiled into
/// the binary and registered with [`crate::Registry::add_text_provider`].
pub trait TextProvider: Send + Sync {
    /// Stable lowercase id, for example `"windows-ocr"`. Unique in a
    /// registry.
    fn id(&self) -> &'static str;

    /// Human-readable name, for example `"Windows OCR"`.
    fn name(&self) -> &'static str;

    /// True when the recognizer works on this system (the platform engine
    /// and at least one of its languages are installed). May take a few
    /// milliseconds the first time; call it on a worker thread. Never
    /// panics.
    fn available(&self) -> bool;

    /// Read the text in `image`. Word outlines come back in pixels of
    /// `image`. Returns [`crate::Error::Cancelled`] soon after `cancel` is
    /// set and [`crate::Error::Unavailable`] when [`TextProvider::available`]
    /// is false. Blocks the calling thread for the whole recognition: call
    /// it on a worker thread, never the interface thread. Must be safe to
    /// call from several threads at once.
    fn recognize(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<TextLayout>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(text: &str, x: f32) -> TextWord {
        TextWord {
            text: text.into(),
            quad: Quad::from_rect(Rect::new(x, 0.0, 10.0, 5.0)),
        }
    }

    fn layout() -> TextLayout {
        TextLayout {
            width: 100,
            height: 50,
            lines: vec![
                TextLine {
                    words: vec![word("one", 0.0), word("two", 20.0)],
                },
                TextLine { words: vec![] },
                TextLine {
                    words: vec![word("three", 0.0), word("four", 20.0), word("five", 40.0)],
                },
            ],
        }
    }

    #[test]
    fn layout_text_joins_lines_and_words() {
        let l = layout();
        assert_eq!(l.word_count(), 5);
        assert!(!l.is_empty());
        assert_eq!(l.text(), "one two\nthree four five");
        assert_eq!(l.text_of(1, 3), "two\nthree four");
        assert_eq!(l.text_of(3, 1), "two\nthree four", "either order");
        assert_eq!(l.text_of(4, 4), "five");
        assert_eq!(l.text_of(4, 99), "five", "past the end");
        assert_eq!(l.line_of(2), Some(2));
        assert_eq!(l.line_of(5), None);
        assert_eq!(l.line_range(2), 2..5);
        assert_eq!(l.line_range(1), 2..2);
        assert_eq!(l.line_range(9), 5..5);
        let order: Vec<&str> = l.words().map(|(_, w)| w.text.as_str()).collect();
        assert_eq!(order, ["one", "two", "three", "four", "five"]);
        assert!(TextLayout::new(3, 3).is_empty());
        assert_eq!(TextLayout::new(3, 3).text(), "");
        assert!(l.byte_size() > 5 * std::mem::size_of::<TextWord>());
        assert_eq!(l.lines[2].text(), "three four five");
    }

    #[test]
    fn quad_contains_measures_and_maps() {
        let q = Quad::from_rect(Rect::new(10.0, 10.0, 20.0, 10.0));
        assert!(q.contains(Point::new(15.0, 15.0)));
        assert!(q.contains(Point::new(10.0, 10.0)), "edges count");
        assert!(!q.contains(Point::new(31.0, 15.0)));
        assert_eq!(q.distance(Point::new(15.0, 15.0)), 0.0);
        assert_eq!(q.distance(Point::new(35.0, 15.0)), 5.0);
        assert_eq!(q.center(), Point::new(20.0, 15.0));
        // A diamond, wound the other way.
        let d = Quad::new([
            Point::new(0.0, 5.0),
            Point::new(5.0, 10.0),
            Point::new(10.0, 5.0),
            Point::new(5.0, 0.0),
        ]);
        assert!(d.contains(Point::new(5.0, 5.0)));
        assert!(!d.contains(Point::new(1.0, 1.0)));
        assert_eq!(d.bounds(), Rect::new(0.0, 0.0, 10.0, 10.0));
        let moved = q.map(|p| Point::new(p.x * 2.0, p.y + 1.0));
        assert_eq!(moved.bounds(), Rect::new(20.0, 11.0, 40.0, 10.0));
    }
}

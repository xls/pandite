//! Shared helpers: text rendered with the font the interface embeds, the
//! ink box of every word drawn, and the platform recognizer.

#![allow(dead_code)]

use pandite_core::{ImageBuf, PixelFormat, Point, Rect, TextLayout, TextProvider};
use std::collections::HashMap;
use std::sync::Arc;

/// Fira Sans Regular subset, the interface font.
static FONT: &[u8] = include_bytes!("../../../pandite-render/fonts/FiraSans-Regular-subset.ttf");

/// One line of text to draw: its words, the top-left corner of its line
/// box and the font size in pixels.
pub struct Line<'a> {
    pub text: &'a str,
    pub x: f32,
    pub y: f32,
    pub px: f32,
}

/// A rendered page: the image and, per line, every word with its ink box.
pub struct Page {
    pub image: ImageBuf,
    pub words: Vec<Vec<(String, Rect)>>,
}

impl Page {
    /// Every drawn word with its ink box, in reading order.
    pub fn all_words(&self) -> impl Iterator<Item = &(String, Rect)> {
        self.words.iter().flatten()
    }
}

/// Draw dark text on a white `width` x `height` RGBA8 image.
pub fn render(width: u32, height: u32, lines: &[Line<'_>]) -> Page {
    let font = fontdue::Font::from_bytes(FONT, fontdue::FontSettings::default()).unwrap();
    let (w, h) = (width as usize, height as usize);
    let mut gray = vec![255u8; w * h];
    let mut glyphs: HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)> = HashMap::new();
    let mut words = Vec::new();
    for line in lines {
        let ascent = font
            .horizontal_line_metrics(line.px)
            .map_or(line.px * 0.8, |m| m.ascent);
        let baseline = line.y + ascent;
        let space = font.metrics(' ', line.px).advance_width;
        let mut pen = line.x;
        let mut boxes = Vec::new();
        for word in line.text.split(' ') {
            let mut ink: Option<(i64, i64, i64, i64)> = None;
            for ch in word.chars() {
                let (m, bitmap) = glyphs
                    .entry((ch, line.px.to_bits()))
                    .or_insert_with(|| font.rasterize(ch, line.px));
                let gx = (pen + m.xmin as f32).round() as i64;
                let gy = (baseline - m.height as f32 - m.ymin as f32).round() as i64;
                for yy in 0..m.height {
                    for xx in 0..m.width {
                        let c = bitmap[yy * m.width + xx];
                        let (x, y) = (gx + xx as i64, gy + yy as i64);
                        if c == 0 || x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                            continue;
                        }
                        let i = y as usize * w + x as usize;
                        gray[i] = gray[i].min(255 - c);
                    }
                }
                if m.width > 0 && m.height > 0 {
                    let (x1, y1) = (gx + m.width as i64, gy + m.height as i64);
                    ink = Some(match ink {
                        None => (gx, gy, x1, y1),
                        Some((a, b, c, d)) => (a.min(gx), b.min(gy), c.max(x1), d.max(y1)),
                    });
                }
                pen += m.advance_width;
            }
            pen += space;
            if let Some((x0, y0, x1, y1)) = ink {
                let r = Rect::new(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32);
                boxes.push((word.to_string(), r));
            }
        }
        words.push(boxes);
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for g in gray {
        rgba.extend_from_slice(&[g, g, g, 255]);
    }
    Page {
        image: ImageBuf::new(width, height, PixelFormat::Rgba8, rgba).unwrap(),
        words,
    }
}

/// `image` (RGBA8) turned by `degrees` clockwise on screen around its
/// center, nearest pixel, white outside. Returns the image and the map
/// from points of `image` to points of the turned one.
pub fn rotate(image: &ImageBuf, degrees: f32) -> (ImageBuf, impl Fn(Point) -> Point) {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let (s, c) = degrees.to_radians().sin_cos();
    let src = image.data();
    let mut out = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            // The inverse turn finds the source pixel.
            let (sx, sy) = (c * dx + s * dy + cx, -s * dx + c * dy + cy);
            let (ix, iy) = (sx.floor(), sy.floor());
            if ix >= 0.0 && iy >= 0.0 && (ix as usize) < w && (iy as usize) < h {
                let si = (iy as usize * w + ix as usize) * 4;
                let di = (y * w + x) * 4;
                out[di..di + 4].copy_from_slice(&src[si..si + 4]);
            }
        }
    }
    let turned = ImageBuf::new(w as u32, h as u32, PixelFormat::Rgba8, out).unwrap();
    let map = move |p: Point| {
        let (dx, dy) = (p.x - cx, p.y - cy);
        Point::new(cx + c * dx - s * dy, cy + s * dx + c * dy)
    };
    (turned, map)
}

/// True when the run must have a working recognizer.
pub fn required() -> bool {
    std::env::var("PANDITE_REQUIRE_OCR").as_deref() == Ok("1")
}

/// The platform recognizer when it works here. Without one the caller
/// skips with a printed notice; with `PANDITE_REQUIRE_OCR=1` the test
/// fails instead.
pub fn engine(test: &str) -> Option<Arc<dyn TextProvider>> {
    let found = pandite_ocr::text_providers()
        .into_iter()
        .find(|p| p.available());
    if found.is_none() {
        assert!(
            !required(),
            "{test}: no text recognizer with an installed language, and PANDITE_REQUIRE_OCR=1"
        );
        println!("skipped {test}: no text recognizer with an installed language on this system");
    }
    found
}

/// True when the recognizer gives each word the full height of its line
/// box, as the built-in PP-OCRv6 models do. Windows OCR boxes hug the ink.
pub fn line_high(ocr: &dyn TextProvider) -> bool {
    ocr.id() == "pp-ocr"
}

/// True when `found` is the box of the word whose ink is `ink`: an
/// intersection over union of at least 0.6 for boxes that hug the ink.
/// Line-high boxes are compared by their horizontal extents only, and
/// must be less than four times as high as the ink.
pub fn on_ink(ink: Rect, found: Rect, line_high: bool) -> bool {
    if !line_high {
        return iou(ink, found) >= 0.6;
    }
    let inter = (ink.right().min(found.right()) - ink.x.max(found.x)).max(0.0);
    let union = ink.right().max(found.right()) - ink.x.min(found.x);
    union > 0.0 && inter / union >= 0.8 && found.h < 4.0 * ink.h
}

/// Intersection over union of two rectangles.
pub fn iou(a: Rect, b: Rect) -> f32 {
    let inter = a.intersect(b).map_or(0.0, |r| r.w * r.h);
    let union = a.w * a.h + b.w * b.h - inter;
    if union > 0.0 {
        inter / union
    } else {
        0.0
    }
}

/// The share of `inner` that lies inside `outer`.
pub fn covered(inner: Rect, outer: Rect) -> f32 {
    let area = inner.w * inner.h;
    if area <= 0.0 {
        return 0.0;
    }
    inner.intersect(outer).map_or(0.0, |r| r.w * r.h) / area
}

/// Pair every drawn word with the first unused recognized word of the same
/// text. Returns (drawn text, ink box, recognized box) for the matches and
/// the drawn words without a match.
pub fn match_words<'a>(
    page: &'a Page,
    layout: &TextLayout,
) -> (Vec<(&'a str, Rect, Rect)>, Vec<&'a str>) {
    let mut used = vec![false; layout.word_count()];
    let found: Vec<(&str, Rect)> = layout
        .words()
        .map(|(_, w)| (w.text.as_str(), w.quad.bounds()))
        .collect();
    let mut pairs = Vec::new();
    let mut missing = Vec::new();
    for (text, ink) in page.all_words() {
        match found
            .iter()
            .enumerate()
            .find(|(i, (t, _))| !used[*i] && t == text)
        {
            Some((i, (_, rect))) => {
                used[i] = true;
                pairs.push((text.as_str(), *ink, *rect));
            }
            None => missing.push(text.as_str()),
        }
    }
    (pairs, missing)
}

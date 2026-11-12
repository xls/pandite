//! Text: fonts, the layout shared by measuring and drawing, and the glyph
//! atlas.
//!
//! Measuring and drawing run the same [`FontSet::layout`] code, so the
//! width [`TextMeasure::measure`] reports is the pen advance the drawn run
//! ends at. Glyph bitmaps are rasterized by fontdue (pure Rust, unhinted
//! area coverage) at the exact pixel size, cached in an R8 atlas and drawn
//! on whole pixels.
//!
//! The embedded font is Fira Sans Regular, SIL Open Font License 1.1
//! (`fonts/FiraSans-OFL.txt`), subset with fontTools to Latin, Greek,
//! Cyrillic, punctuation, currency, arrows and geometric shapes, hinting
//! and layout tables removed (68 KiB):
//! `pyftsubset FiraSans-Regular.ttf --unicodes=U+0020-007E,U+00A0-024F,U+0300-036F,U+0370-03FF,U+0400-04FF,U+2000-206F,U+20A0-20BF,U+2100-214F,U+2190-21FF,U+2212,U+2215,U+2219,U+221E,U+2248,U+2260,U+2264,U+2265,U+25A0-25FF,U+FFFD --no-hinting --layout-features= --drop-tables+=GPOS,GSUB,GDEF,DSIG --notdef-outline`.

use fontdue::{Font, FontSettings};
use pandite_core::{Size, TextMeasure};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// Fira Sans Regular subset (SIL Open Font License 1.1).
static EMBEDDED_FONT: &[u8] = include_bytes!("../fonts/FiraSans-Regular-subset.ttf");

/// Largest text size rasterized, in pixels.
const MAX_TEXT_PX: f32 = 512.0;

/// Which font the interface uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontChoice {
    /// The operating system interface font when it is found (Segoe UI on
    /// Windows, San Francisco or Helvetica on macOS, Noto Sans or DejaVu
    /// Sans on Linux), read from disk once per process. The embedded font
    /// covers characters it lacks and replaces it when it is missing.
    /// Parsing takes about 50 ms for Segoe UI; calling
    /// [`TextMeasurer::new`] on a worker thread at startup overlaps it with
    /// window and GPU setup.
    #[default]
    System,
    /// Only the embedded Fira Sans.
    Embedded,
}

/// A glyph placed by [`FontSet::layout`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placed {
    pub font: u16,
    pub glyph: u16,
    /// Pen position relative to the start of the run.
    pub x: f32,
}

/// Vertical metrics of a line, in pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineMetrics {
    /// Baseline below the top of the line box.
    pub ascent: f32,
    /// Line box height.
    pub height: f32,
}

/// The fonts in fallback order. The first one sets the line metrics.
pub(crate) struct FontSet {
    fonts: Vec<Font>,
}

impl FontSet {
    /// The process-wide font set for `choice`, loaded on first use.
    pub(crate) fn shared(choice: FontChoice) -> Arc<FontSet> {
        static EMBEDDED: OnceLock<Arc<FontSet>> = OnceLock::new();
        static SYSTEM: OnceLock<Arc<FontSet>> = OnceLock::new();
        let cell = match choice {
            FontChoice::Embedded => &EMBEDDED,
            FontChoice::System => &SYSTEM,
        };
        Arc::clone(cell.get_or_init(|| Arc::new(FontSet::load(choice))))
    }

    fn load(choice: FontChoice) -> FontSet {
        let mut fonts = Vec::with_capacity(2);
        if choice == FontChoice::System {
            if let Some(font) = system_font() {
                fonts.push(font);
            }
        }
        fonts.push(embedded_font());
        FontSet { fonts }
    }

    /// Name of the primary font.
    pub(crate) fn name(&self) -> &str {
        self.fonts[0].name().unwrap_or("unnamed")
    }

    pub(crate) fn font(&self, index: u16) -> &Font {
        &self.fonts[usize::from(index)]
    }

    /// Size snapped to quarter pixels, the glyph cache granularity.
    pub(crate) fn quantize(size: f32) -> f32 {
        if size.is_finite() {
            (size.clamp(0.0, MAX_TEXT_PX) * 4.0).round() / 4.0
        } else {
            0.0
        }
    }

    pub(crate) fn line(&self, px: f32) -> LineMetrics {
        match self.fonts[0].horizontal_line_metrics(px) {
            Some(m) => LineMetrics {
                ascent: m.ascent,
                height: m.ascent - m.descent + m.line_gap,
            },
            None => LineMetrics {
                ascent: px * 0.8,
                height: px * 1.2,
            },
        }
    }

    fn lookup(&self, ch: char) -> (u16, u16) {
        for (index, font) in self.fonts.iter().enumerate() {
            let glyph = font.lookup_glyph_index(ch);
            if glyph != 0 {
                return (index as u16, glyph);
            }
        }
        // The primary font's "missing glyph" box.
        (0, 0)
    }

    /// Lay out one line at `px` pixels. Calls `each` for every glyph and
    /// returns the total advance. Control characters take no space.
    pub(crate) fn layout(&self, text: &str, px: f32, mut each: impl FnMut(Placed)) -> f32 {
        let mut pen = 0.0f32;
        let mut previous: Option<(u16, u16)> = None;
        for ch in text.chars() {
            if ch.is_control() {
                continue;
            }
            let (font, glyph) = self.lookup(ch);
            let face = self.font(font);
            if let Some((prev_font, prev_glyph)) = previous {
                if prev_font == font {
                    pen += face
                        .horizontal_kern_indexed(prev_glyph, glyph, px)
                        .unwrap_or(0.0);
                }
            }
            each(Placed {
                font,
                glyph,
                x: pen,
            });
            pen += face.metrics_indexed(glyph, px).advance_width;
            previous = Some((font, glyph));
        }
        pen
    }
}

fn font_settings() -> FontSettings {
    FontSettings {
        collection_index: 0,
        scale: 20.0,
        load_substitutions: false,
    }
}

fn embedded_font() -> Font {
    match Font::from_bytes(EMBEDDED_FONT, font_settings()) {
        Ok(font) => font,
        // The bytes are compiled in and checked by a unit test.
        Err(err) => unreachable!("embedded font failed to parse: {err}"),
    }
}

fn system_font_paths() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let windir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"));
        vec![windir.join("Fonts").join("segoeui.ttf")]
    }
    #[cfg(target_vendor = "apple")]
    {
        vec![
            PathBuf::from("/System/Library/Fonts/SFNS.ttf"),
            PathBuf::from("/System/Library/Fonts/Helvetica.ttc"),
        ]
    }
    #[cfg(all(unix, not(target_vendor = "apple")))]
    {
        [
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/google-noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/google-noto-vf/NotoSans[wght].ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu-sans-fonts/DejaVuSans.ttf",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
    #[cfg(not(any(windows, unix)))]
    {
        Vec::new()
    }
}

fn system_font() -> Option<Font> {
    system_font_paths().into_iter().find_map(|path| {
        let bytes = std::fs::read(path).ok()?;
        Font::from_bytes(bytes.as_slice(), font_settings()).ok()
    })
}

/// Measures text exactly as the renderer draws it. Cheap to clone and
/// usable from any thread; [`crate::Renderer::text_measurer`] returns the
/// one matching the renderer's font.
#[derive(Clone)]
pub struct TextMeasurer {
    fonts: Arc<FontSet>,
}

impl std::fmt::Debug for TextMeasurer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TextMeasurer")
            .field("font", &self.fonts.name())
            .finish()
    }
}

impl TextMeasurer {
    /// A measurer for `choice`. Needs no GPU.
    pub fn new(choice: FontChoice) -> Self {
        Self {
            fonts: FontSet::shared(choice),
        }
    }

    pub(crate) fn from_set(fonts: Arc<FontSet>) -> Self {
        Self { fonts }
    }

    /// Name of the primary font, for diagnostics.
    pub fn font_name(&self) -> &str {
        self.fonts.name()
    }

    /// Distance from the top of the line box to the baseline at `size`.
    pub fn ascent(&self, size: f32) -> f32 {
        let px = FontSet::quantize(size);
        if px <= 0.0 {
            return 0.0;
        }
        self.fonts.line(px).ascent
    }
}

impl TextMeasure for TextMeasurer {
    fn measure(&self, text: &str, size: f32) -> Size {
        let px = FontSet::quantize(size);
        if px <= 0.0 {
            return Size::default();
        }
        let width = self.fonts.layout(text, px, |_| {});
        Size::new(width, self.fonts.line(px).height)
    }
}

/// A rasterized glyph in the atlas.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GlyphEntry {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    /// Bitmap left edge relative to the pen position.
    pub left: i32,
    /// Bitmap top edge relative to the baseline (negative is above).
    pub top: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphKey {
    font: u16,
    glyph: u16,
    quarter_px: u32,
}

#[derive(Clone, Copy, Debug)]
struct Shelf {
    y: u32,
    h: u32,
    x: u32,
}

/// The atlas has no room even at its largest size.
#[derive(Debug)]
pub(crate) struct AtlasFull;

/// CPU side of the glyph atlas: shelf packing into a square R8 image that
/// doubles in size when full, up to `max_size`.
pub(crate) struct GlyphAtlas {
    size: u32,
    max_size: u32,
    pixels: Vec<u8>,
    shelves: Vec<Shelf>,
    next_y: u32,
    entries: HashMap<GlyphKey, Option<GlyphEntry>>,
    /// Rows changed since the last upload.
    dirty: Option<(u32, u32)>,
}

const ATLAS_START: u32 = 512;
const GLYPH_PAD: u32 = 1;

impl GlyphAtlas {
    pub(crate) fn new(max_size: u32) -> Self {
        let size = ATLAS_START.min(max_size);
        Self {
            size,
            max_size,
            pixels: vec![0; (size * size) as usize],
            shelves: Vec::new(),
            next_y: 0,
            entries: HashMap::new(),
            dirty: Some((0, size)),
        }
    }

    pub(crate) fn size(&self) -> u32 {
        self.size
    }

    pub(crate) fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Take the changed row range for upload.
    pub(crate) fn take_dirty(&mut self) -> Option<(u32, u32)> {
        self.dirty.take()
    }

    /// Forget every glyph (the atlas keeps its size).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.shelves.clear();
        self.next_y = 0;
        self.pixels.fill(0);
        self.dirty = Some((0, self.size));
    }

    /// The cached glyph, rasterizing it on first use. `Ok(None)` for glyphs
    /// without ink (space).
    pub(crate) fn glyph(
        &mut self,
        fonts: &FontSet,
        placed: &Placed,
        px: f32,
    ) -> Result<Option<GlyphEntry>, AtlasFull> {
        let key = GlyphKey {
            font: placed.font,
            glyph: placed.glyph,
            quarter_px: (px * 4.0) as u32,
        };
        if let Some(entry) = self.entries.get(&key) {
            return Ok(*entry);
        }
        let (metrics, bitmap) = fonts.font(placed.font).rasterize_indexed(placed.glyph, px);
        if metrics.width == 0 || metrics.height == 0 {
            self.entries.insert(key, None);
            return Ok(None);
        }
        let (w, h) = (metrics.width as u32, metrics.height as u32);
        let (x, y) = self.allocate(w + GLYPH_PAD, h + GLYPH_PAD)?;
        let stride = self.size as usize;
        for row in 0..h as usize {
            let dst = (y as usize + row) * stride + x as usize;
            self.pixels[dst..dst + w as usize]
                .copy_from_slice(&bitmap[row * w as usize..(row + 1) * w as usize]);
        }
        self.mark_dirty(y, y + h);
        let entry = GlyphEntry {
            x,
            y,
            w,
            h,
            left: metrics.xmin,
            top: -(metrics.ymin + h as i32),
        };
        self.entries.insert(key, Some(entry));
        Ok(Some(entry))
    }

    fn mark_dirty(&mut self, y0: u32, y1: u32) {
        self.dirty = Some(match self.dirty {
            Some((a, b)) => (a.min(y0), b.max(y1)),
            None => (y0, y1),
        });
    }

    fn allocate(&mut self, w: u32, h: u32) -> Result<(u32, u32), AtlasFull> {
        loop {
            if let Some(found) = self.try_allocate(w, h) {
                return Ok(found);
            }
            if self.size >= self.max_size {
                return Err(AtlasFull);
            }
            self.grow();
        }
    }

    fn try_allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.size || h > self.size {
            return None;
        }
        // Best fitting shelf that still has room.
        let size = self.size;
        let best = self
            .shelves
            .iter_mut()
            .filter(|s| h <= s.h && s.h <= h + h / 2 + 2 && s.x + w <= size)
            .min_by_key(|s| s.h);
        if let Some(shelf) = best {
            let at = (shelf.x, shelf.y);
            shelf.x += w;
            return Some(at);
        }
        if self.next_y + h <= self.size {
            let y = self.next_y;
            self.shelves.push(Shelf { y, h, x: w });
            self.next_y += h;
            return Some((0, y));
        }
        None
    }

    /// Double the side, keeping every glyph where it is.
    fn grow(&mut self) {
        let old = self.size as usize;
        let size = (self.size * 2).min(self.max_size);
        let mut pixels = vec![0u8; (size * size) as usize];
        for row in 0..old {
            pixels[row * size as usize..row * size as usize + old]
                .copy_from_slice(&self.pixels[row * old..(row + 1) * old]);
        }
        self.pixels = pixels;
        self.size = size;
        self.dirty = Some((0, size));
    }
}

#[cfg(test)]
mod unit {
    use super::*;

    #[test]
    fn embedded_font_parses_and_covers_latin() {
        let set = FontSet::shared(FontChoice::Embedded);
        assert!(set.name().contains("Fira"), "{}", set.name());
        for ch in "Hamburgefonstiv 0123456789 AEIOU \u{e9}\u{df}\u{20ac}".chars() {
            assert_ne!(set.lookup(ch).1, 0, "missing {ch:?}");
        }
    }

    #[test]
    fn measure_is_additive_and_scales() {
        let m = TextMeasurer::new(FontChoice::Embedded);
        let a = m.measure("Hello", 20.0);
        let b = m.measure("Hello", 40.0);
        assert!(a.w > 0.0 && a.h > 0.0);
        assert!((b.w - 2.0 * a.w).abs() < 1.0, "{a:?} {b:?}");
        assert_eq!(m.measure("", 20.0).w, 0.0);
        assert_eq!(m.measure("a\nb", 20.0).w, m.measure("ab", 20.0).w);
    }

    #[test]
    fn system_font_loads_or_falls_back() {
        let start = std::time::Instant::now();
        let set = FontSet::load(FontChoice::System);
        let took = start.elapsed();
        // The embedded font is always last in the chain.
        let last = set.fonts.last().and_then(Font::name).unwrap_or("");
        assert!(last.contains("Fira"), "{last}");
        let m = TextMeasurer::from_set(Arc::new(set));
        let size = m.measure("Hamburgefonstiv \u{4e2d}", 16.0);
        assert!(size.w > 50.0 && size.h > 14.0, "{size:?}");
        println!("system font {:?} loaded in {took:?}", m.font_name());
    }

    #[test]
    fn atlas_packs_and_grows() {
        let set = FontSet::shared(FontChoice::Embedded);
        let mut atlas = GlyphAtlas::new(1024);
        let mut placed = Vec::new();
        set.layout("The quick brown fox", 64.0, |p| placed.push(p));
        for size in [64.0, 96.0, 128.0] {
            for p in &placed {
                let _ = atlas.glyph(&set, p, size).expect("fits");
            }
        }
        assert!(atlas.size() > ATLAS_START, "atlas grew to {}", atlas.size());
    }
}

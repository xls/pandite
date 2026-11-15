//! Gate G2: recognition through the provider. Text is rendered into an
//! image with the interface font and read back by the platform recognizer.
//! Tests that need the engine skip with a printed notice when this system
//! has none (no recognizer, or no OCR language), unless
//! `PANDITE_REQUIRE_OCR=1` is set; then a missing engine fails them.

mod common;

use common::{covered, engine, line_high, match_words, on_ink, render, Line};
use pandite_core::{CancelToken, Error, ImageBuf, Rect, Registry, TextProvider};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn recognizes_rendered_text() {
    let Some(ocr) = engine("recognizes_rendered_text") else {
        return;
    };
    let page = render(
        1600,
        420,
        &[
            Line {
                text: "The quick brown fox jumps over the lazy dog",
                x: 60.0,
                y: 60.0,
                px: 52.0,
            },
            Line {
                text: "Pandite reads text in photos 2026",
                x: 60.0,
                y: 230.0,
                px: 52.0,
            },
        ],
    );
    let layout = ocr.recognize(&page.image, &CancelToken::new()).unwrap();
    assert_eq!((layout.width, layout.height), (1600, 420));
    // Two lines in reading order, every word read as drawn.
    assert_eq!(
        layout.text(),
        "The quick brown fox jumps over the lazy dog\nPandite reads text in photos 2026",
        "lines: {:?}",
        layout.lines.iter().map(|l| l.text()).collect::<Vec<_>>()
    );
    // Images too small or too plain to hold text give no words (or an
    // error), never a panic.
    for (w, h) in [(1, 1), (3, 40), (64, 48)] {
        let blank = ImageBuf::solid_rgba8(w, h, [255, 255, 255, 255]).unwrap();
        if let Ok(layout) = ocr.recognize(&blank, &CancelToken::new()) {
            assert!(layout.is_empty(), "{w} x {h}: {:?}", layout.text());
            assert_eq!((layout.width, layout.height), (w, h));
        }
    }
}

#[test]
fn word_boxes_cover_the_text() {
    let Some(ocr) = engine("word_boxes_cover_the_text") else {
        return;
    };
    // Words of different sizes in different places, so a box from the
    // wrong word or a wrong scale cannot pass.
    let page = render(
        1800,
        900,
        &[
            Line {
                text: "Harbor lanterns glow tonight",
                x: 120.0,
                y: 80.0,
                px: 64.0,
            },
            Line {
                text: "seven sleepy rabbits",
                x: 900.0,
                y: 360.0,
                px: 40.0,
            },
            Line {
                text: "copper kettle whistles",
                x: 300.0,
                y: 640.0,
                px: 90.0,
            },
        ],
    );
    let layout = ocr.recognize(&page.image, &CancelToken::new()).unwrap();
    let (pairs, missing) = match_words(&page, &layout);
    assert!(
        missing.is_empty(),
        "not found: {missing:?} in {:?}",
        layout.text()
    );
    let tall = line_high(ocr.as_ref());
    for (text, ink, found) in &pairs {
        // The reported box holds the word's ink and is not much larger.
        // The built-in models place word edges to one time step of their
        // recognizer: 8 of its 48 input pixels, a sixth of the box height.
        let mx = if tall { found.h / 6.0 } else { 2.0 };
        let grown = Rect::new(
            found.x - mx,
            found.y - 2.0,
            found.w + 2.0 * mx,
            found.h + 4.0,
        );
        assert!(
            covered(*ink, grown) >= 0.97,
            "{text}: box {found:?} misses ink {ink:?}"
        );
        assert!(
            on_ink(*ink, *found, tall),
            "{text}: box {found:?} far from ink {ink:?}"
        );
    }
    // Upright text gives upright boxes. The built-in models fit a turned
    // rectangle around each line and round its corners to whole pixels,
    // as their reference does, so an edge may lean by about a degree.
    for (_, word) in layout.words() {
        let q = word.quad;
        if tall {
            let [a, b, _, d] = q.points;
            assert!(
                (b.y - a.y).atan2(b.x - a.x).to_degrees().abs() < 1.5,
                "{q:?}"
            );
            assert!(
                (d.x - a.x).atan2(d.y - a.y).to_degrees().abs() < 1.5,
                "{q:?}"
            );
        } else {
            assert!((q.points[0].y - q.points[1].y).abs() < 0.5, "{q:?}");
            assert!((q.points[0].x - q.points[3].x).abs() < 0.5, "{q:?}");
        }
    }
}

#[test]
fn large_image_is_scaled_and_boxes_map_back() {
    let Some(ocr) = engine("large_image_is_scaled_and_boxes_map_back") else {
        return;
    };
    // Wider than both the recognition cap and the engine's own limit
    // (10000 pixels for Windows OCR), so the engine can only read it
    // scaled down. One line sits far right, where unmapped boxes would
    // land at half the distance.
    let (w, h) = (12_000, 2_400);
    #[cfg(windows)]
    assert!(w > pandite_ocr::WindowsOcr::engine_limit().unwrap_or(10_000));
    assert!(w > pandite_ocr::MAX_SIDE);
    let page = render(
        w,
        h,
        &[
            Line {
                text: "Harbor lanterns glow",
                x: 8_600.0,
                y: 300.0,
                px: 150.0,
            },
            Line {
                text: "seven quiet rabbits",
                x: 500.0,
                y: 1_700.0,
                px: 150.0,
            },
        ],
    );
    let started = Instant::now();
    let layout = ocr.recognize(&page.image, &CancelToken::new()).unwrap();
    println!(
        "{w} x {h} recognized in {} ms",
        started.elapsed().as_millis()
    );
    assert_eq!((layout.width, layout.height), (w, h));
    let (pairs, missing) = match_words(&page, &layout);
    assert!(
        missing.is_empty(),
        "not found: {missing:?} in {:?}",
        layout.text()
    );
    let tall = line_high(ocr.as_ref());
    for (text, ink, found) in &pairs {
        // One recognized pixel is two image pixels here (six for the
        // built-in models, which read at most 2000 pixels and place word
        // edges to one time step, a sixth of the box height).
        let mx = if tall { found.h / 6.0 } else { 6.0 };
        let grown = Rect::new(
            found.x - mx,
            found.y - 6.0,
            found.w + 2.0 * mx,
            found.h + 12.0,
        );
        assert!(
            covered(*ink, grown) >= 0.97,
            "{text}: box {found:?} misses ink {ink:?}"
        );
        assert!(
            on_ink(*ink, *found, tall),
            "{text}: box {found:?} far from ink {ink:?}"
        );
    }
    assert_eq!(layout.lines.len(), 2);
    assert!(layout.lines[0].words[0].quad.bounds().x > 8_000.0);
}

#[test]
fn slanted_text_boxes_follow_the_slant() {
    let Some(ocr) = engine("slanted_text_boxes_follow_the_slant") else {
        return;
    };
    let page = render(
        1600,
        1000,
        &[
            Line {
                text: "Harbor lanterns glow tonight",
                x: 300.0,
                y: 300.0,
                px: 56.0,
            },
            Line {
                text: "seven quiet rabbits",
                x: 300.0,
                y: 450.0,
                px: 56.0,
            },
        ],
    );
    for degrees in [8.0f32, -11.0] {
        let (image, turn) = common::rotate(&page.image, degrees);
        let layout = ocr.recognize(&image, &CancelToken::new()).unwrap();
        let (pairs, missing) = match_words(&page, &layout);
        assert!(
            missing.len() <= 1,
            "{degrees}: not found {missing:?} in {:?}",
            layout.text()
        );
        for (_, word) in layout.words() {
            // The top edge runs along the slant.
            let [a, b, ..] = word.quad.points;
            let slope = (b.y - a.y).atan2(b.x - a.x).to_degrees();
            assert!(
                (slope - degrees).abs() < 1.5,
                "{degrees}: {} slopes {slope}",
                word.text
            );
        }
        for (text, ink, _) in &pairs {
            // Each box sits where its ink was turned to.
            let (_, word) = layout.words().find(|(_, w)| w.text == *text).unwrap();
            let want = turn(ink.center());
            let got = word.quad.center();
            let off = ((got.x - want.x).powi(2) + (got.y - want.y).powi(2)).sqrt();
            assert!(off < 8.0, "{degrees}: {text} at {got:?}, ink at {want:?}");
        }
    }
}

/// A page that keeps the engine busy for a few hundred milliseconds.
fn dense_page() -> ImageBuf {
    let text = "lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod tempor";
    let lines: Vec<Line<'_>> = (0..90)
        .map(|i| Line {
            text,
            x: 80.0,
            y: 60.0 + i as f32 * 60.0,
            px: 40.0,
        })
        .collect();
    render(3_000, 5_500, &lines).image
}

#[test]
fn cancelled_recognition_stops() {
    let Some(ocr) = engine("cancelled_recognition_stops") else {
        return;
    };
    let small = render(
        900,
        200,
        &[Line {
            text: "Harbor lanterns glow",
            x: 40.0,
            y: 40.0,
            px: 56.0,
        }],
    );
    // A token cancelled before the call stops it before any work.
    let cancel = CancelToken::new();
    cancel.cancel();
    let started = Instant::now();
    assert!(matches!(
        ocr.recognize(&small.image, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(started.elapsed() < Duration::from_millis(100));

    // Cancel while the engine reads a dense page.
    let page = dense_page();
    let cancel = CancelToken::new();
    let worker = {
        let ocr: Arc<dyn TextProvider> = Arc::clone(&ocr);
        let cancel = cancel.clone();
        std::thread::spawn(move || {
            let result = ocr.recognize(&page, &cancel);
            (result, Instant::now())
        })
    };
    std::thread::sleep(Duration::from_millis(80));
    let cancelled_at = Instant::now();
    cancel.cancel();
    let (result, returned_at) = worker.join().unwrap();
    let latency = returned_at.saturating_duration_since(cancelled_at);
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "the dense page finished before the cancel: {:?}",
        result.map(|l| l.word_count())
    );
    assert!(latency < Duration::from_millis(250), "{latency:?}");
    println!("returned {} ms after the cancel", latency.as_millis());

    // The recognizer works right after: the next request waits for the
    // engine to drop the abandoned one, then reads normally.
    let layout = ocr.recognize(&small.image, &CancelToken::new()).unwrap();
    assert_eq!(layout.text(), "Harbor lanterns glow");
}

#[test]
fn missing_engine_reports_unavailable() {
    let image = render(
        400,
        120,
        &[Line {
            text: "nothing to read",
            x: 10.0,
            y: 10.0,
            px: 40.0,
        }],
    )
    .image;
    #[cfg(windows)]
    {
        // Languages without OCR data, and a tag that is not a language.
        for tag in ["tlh", "zz-QQ", "not a language"] {
            let ocr = pandite_ocr::WindowsOcr::for_language(tag);
            assert!(!ocr.available(), "{tag}");
            match ocr.recognize(&image, &CancelToken::new()) {
                Err(Error::Unavailable(why)) => assert!(why.contains(tag), "{why}"),
                other => panic!("{tag}: expected unavailable, got {other:?}"),
            }
            // Cancellation still wins over the missing engine.
            let cancel = CancelToken::new();
            cancel.cancel();
            assert!(matches!(
                ocr.recognize(&image, &cancel),
                Err(Error::Cancelled)
            ));
        }
        // The platform registers exactly its one recognizer.
        let mut registry = Registry::new();
        pandite_ocr::register_text_providers(&mut registry);
        let ids: Vec<&str> = registry.text_providers().iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["windows-ocr"]);
    }
    #[cfg(not(windows))]
    {
        // The built-in recognizer: compiled in, nothing to install, so it
        // is always available.
        let mut registry = Registry::new();
        pandite_ocr::register_text_providers(&mut registry);
        let ids: Vec<&str> = registry.text_providers().iter().map(|p| p.id()).collect();
        assert_eq!(ids, ["pp-ocr"]);
        let ocr = &registry.text_providers()[0];
        assert!(ocr.available());
        // Cancellation wins before any work.
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            ocr.recognize(&image, &cancel),
            Err(Error::Cancelled)
        ));
    }
}

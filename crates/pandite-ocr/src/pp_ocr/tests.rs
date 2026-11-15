//! The whole recognizer on the reference fixtures, hostile images and
//! cancellation. The fixtures in `tests/fixtures` are three rendered
//! images; `fixtures.json` holds, per image, what the reference
//! implementation (RapidOCR 3.10 with these models) reads in them.

use super::*;
use pandite_core::{DecodeOptions, PixelFormat, Source};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// The largest distance, in pixels, of a line box corner from the
/// reference's corner.
const CORNER_TOLERANCE: f32 = 4.0;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn decode(name: &str) -> ImageBuf {
    let source = Source::from_path(fixture_path(name)).unwrap();
    let content = pandite_codecs::default_registry()
        .decode(&source, &DecodeOptions::default())
        .unwrap();
    content.first_image().clone()
}

fn fixtures() -> serde_json::Value {
    let text = std::fs::read_to_string(fixture_path("fixtures.json")).unwrap();
    serde_json::from_str(&text).unwrap()
}

fn point(v: &serde_json::Value) -> Point {
    Point::new(v[0].as_f64().unwrap() as f32, v[1].as_f64().unwrap() as f32)
}

fn distance(a: Point, b: Point) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// `image` placed at (`x`, `y`) on a white `width` x `height` canvas.
fn on_canvas(image: &ImageBuf, width: u32, height: u32, x: u32, y: u32) -> ImageBuf {
    assert_eq!(image.format(), PixelFormat::Rgba8);
    let mut data = vec![255u8; (width * height * 4) as usize];
    for row in 0..image.height() {
        let src = image.row(row);
        let at = (((y + row) * width + x) * 4) as usize;
        data[at..at + src.len()].copy_from_slice(src);
    }
    ImageBuf::new(width, height, PixelFormat::Rgba8, data).unwrap()
}

#[test]
fn sizes_follow_the_reference() {
    // Within bounds: read as it is.
    assert_eq!(working_size(900, 420), (900, 420));
    // 6000 x 4000: a third, rounded to multiples of 32 (62.5 rounds to
    // the even 62).
    assert_eq!(working_size(6000, 4000), (1984, 1344));
    // Too narrow: the short side grows to 30, then 32.
    assert_eq!(working_size(25, 500), (32, 608));
    assert_eq!(working_size(1, 1), (32, 32));
    assert_eq!(working_size(60_000, 3), (1984, 32));
    // Bands: none for a normal image, a quarter of the width for a wide
    // or very low one.
    assert_eq!(band(900, 420), 0);
    assert_eq!(band(2000, 100), 200);
    assert_eq!(band(320, 30), 25);
    // Detection: short side to 736, multiples of 32, long side capped.
    assert_eq!(detection_size(900, 420), (1568, 736));
    assert_eq!(detection_size(1984, 1344), (1984, 1344));
    assert_eq!(detection_size(32, 1984), (64, 4000));
}

#[test]
fn character_list_is_the_models() {
    let engine = engine().unwrap();
    // 6904 characters, the blank and the space: one per output class.
    assert_eq!(engine.charset.len(), 6906);
    assert_eq!(engine.charset[0], "blank");
    assert_eq!(engine.charset[6905], " ");
    // Every entry is one character and none is a line break that Python's
    // splitlines (the reference) would split at, so the classes line up.
    for c in &engine.charset[1..6905] {
        assert_eq!(c.chars().count(), 1, "{c:?}");
        assert!(
            !c.contains([
                '\r', '\x0b', '\x0c', '\x1c', '\x1d', '\x1e', '\u{85}', '\u{2028}', '\u{2029}'
            ]),
            "{c:?}"
        );
    }
    for c in [
        "a", "Z", "0", "\u{e5}", "\u{f8}", "\u{e6}", "\u{df}", "\u{c4}", "\u{bf}", "\u{f1}",
    ] {
        assert!(engine.charset.iter().any(|e| e == c), "{c:?} missing");
    }
}

#[test]
fn fixtures_read_as_the_reference() {
    let all = fixtures();
    let ocr = PpOcr::new();
    // A load of its own, as the first use in a process does it.
    let started = Instant::now();
    let fresh = Engine::load().unwrap();
    println!(
        "models load and optimize in {} ms",
        started.elapsed().as_millis()
    );
    drop(fresh);
    engine().unwrap();
    let mut worst_corner = 0f32;
    for name in ["page_light", "panel_dark", "sign_color"] {
        let fixture = &all[name];
        let image = decode(fixture["file"].as_str().unwrap());
        let started = Instant::now();
        let lines = ocr.read(&image, &CancelToken::new()).unwrap();
        let first = started.elapsed();
        let started = Instant::now();
        let again = ocr.read(&image, &CancelToken::new()).unwrap();
        let second = started.elapsed();
        assert_eq!(lines, again, "{name}: the same image reads the same");

        let want = fixture["rapidocr_tiny"].as_array().unwrap();
        let got: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        let expected: Vec<&str> = want.iter().map(|l| l["text"].as_str().unwrap()).collect();
        assert_eq!(got, expected, "{name}");

        let mut corner = 0f32;
        for (line, want) in lines.iter().zip(want) {
            for (i, p) in line.quad.points.iter().enumerate() {
                let d = distance(*p, point(&want["quad"][i]));
                corner = corner.max(d);
                assert!(
                    d <= CORNER_TOLERANCE,
                    "{name}: {:?} corner {i} at {p:?}, reference {:?}",
                    line.text,
                    want["quad"][i]
                );
            }
            // The words give the line, each inside the line's box.
            let words: Vec<&str> = line.words.iter().map(|w| w.text.as_str()).collect();
            assert_eq!(words.join(" "), line.text, "{name}");
            for w in &line.words {
                for p in w.quad.points {
                    assert!(
                        line.quad.distance(p) <= 0.5,
                        "{name}: {:?} at {:?} leaves its line {:?}",
                        w.text,
                        w.quad,
                        line.quad
                    );
                }
            }
            // Words run left to right without overlapping.
            for pair in line.words.windows(2) {
                assert!(
                    pair[0].quad.points[1].x <= pair[1].quad.points[0].x + 0.5,
                    "{pair:?}"
                );
            }
            assert!(line.score >= TEXT_SCORE);
        }
        worst_corner = worst_corner.max(corner);
        println!(
            "{name}: {} x {}, {} lines as the reference, largest corner deviation {corner:.2} px, \
             first read {} ms (with the detection plan when its size is new), \
             second read {} ms",
            image.width(),
            image.height(),
            lines.len(),
            first.as_millis(),
            second.as_millis()
        );
    }
    println!("largest corner deviation over all fixtures: {worst_corner:.2} px");

    // The provider gives the same lines as words.
    let image = decode("panel_dark.png");
    let layout = ocr.recognize(&image, &CancelToken::new()).unwrap();
    assert_eq!((layout.width, layout.height), (720, 300));
    let expected: Vec<&str> = all["panel_dark"]["rapidocr_tiny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["text"].as_str().unwrap())
        .collect();
    assert_eq!(layout.text(), expected.join("\n"));
}

#[test]
fn word_boxes_follow_the_characters() {
    // "Ctrl+Shift+C" is one word and "Open with Pandite" three: the box
    // of "with" lies inside the line, between the other two, and is
    // narrower than an even split of the line would make it.
    let image = decode("panel_dark.png");
    let lines = PpOcr::new().read(&image, &CancelToken::new()).unwrap();
    let open = lines
        .iter()
        .find(|l| l.text == "Open with Pandite")
        .unwrap();
    let [a, b, c] = &open.words[..] else {
        panic!("{:?}", open.words);
    };
    assert_eq!(
        (a.text.as_str(), b.text.as_str(), c.text.as_str()),
        ("Open", "with", "Pandite")
    );
    let width = |w: &TextWord| w.quad.bounds().w;
    let line = open.quad.bounds();
    assert!(a.quad.bounds().right() <= b.quad.bounds().x + 0.5);
    assert!(b.quad.bounds().right() <= c.quad.bounds().x + 0.5);
    // "Pandite" has 7 letters, "with" 4: its box is clearly wider.
    assert!(width(c) > 1.4 * width(b), "{} {}", width(c), width(b));
    // Each word box holds its letters: about the share of the line its
    // characters take (17 characters in all), not a third.
    assert!(width(b) < line.w / 3.0 * 0.9, "{} of {}", width(b), line.w);
    // The truth box of the line starts at x 30 and ends at 187.
    assert!((a.quad.bounds().x - 30.0).abs() < 6.0, "{:?}", a.quad);
    assert!(
        (c.quad.bounds().right() - 187.0).abs() < 6.0,
        "{:?}",
        c.quad
    );
}

#[test]
fn hostile_images_give_sensible_results() {
    let ocr = PpOcr::new();
    let cancel = CancelToken::new();
    // An image without pixels cannot exist.
    assert!(ImageBuf::new(0, 0, PixelFormat::Rgba8, Vec::new()).is_err());
    // Tiny, extreme and plain images: no text, no panic.
    for (w, h, rgba) in [
        (1, 1, [255, 255, 255, 255]),
        (1, 1, [0, 0, 0, 255]),
        (2, 1, [255, 255, 255, 255]),
        (1, 3, [0, 0, 0, 0]),
        (9000, 2, [255, 255, 255, 255]),
        (2, 9000, [255, 255, 255, 255]),
        (640, 480, [255, 255, 255, 255]),
        (640, 480, [0, 0, 0, 255]),
        (300, 200, [0, 0, 0, 0]),
    ] {
        let image = ImageBuf::solid_rgba8(w, h, rgba).unwrap();
        let lines = ocr.read(&image, &cancel).unwrap();
        assert!(lines.is_empty(), "{w} x {h}: {lines:?}");
        let layout = ocr.recognize(&image, &cancel).unwrap();
        assert!(layout.is_empty());
        assert_eq!((layout.width, layout.height), (w, h));
    }
    // Noise: whatever it reads stays inside the image.
    let mut seed = 7u32;
    let noise: Vec<u8> = (0..400 * 300 * 4)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect();
    let noise = ImageBuf::new(400, 300, PixelFormat::Rgba8, noise).unwrap();
    for line in ocr.read(&noise, &cancel).unwrap() {
        for p in line.quad.points {
            assert!(
                (0.0..=400.0).contains(&p.x) && (0.0..=300.0).contains(&p.y),
                "{p:?}"
            );
        }
    }

    // The sign on a very wide canvas: read at a third of its size with
    // bands, and the box lands where the sign is. The truth box of
    // "STOP" is (60, 75) to (238, 128) in the sign.
    let sign = decode("sign_color.png");
    let wide = on_canvas(&sign, 6000, 400, 4000, 20);
    let started = Instant::now();
    let lines = ocr.read(&wide, &cancel).unwrap();
    println!(
        "6000 x 400 canvas in {} ms reads {:?}",
        started.elapsed().as_millis(),
        lines.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    let found = lines.iter().find(|l| l.text == "STOP");
    let Some(found) = found else {
        panic!("{:?}", lines.iter().map(|l| &l.text).collect::<Vec<_>>());
    };
    let b = found.quad.bounds();
    let ink = pandite_core::Rect::new(4060.0, 95.0, 178.0, 53.0);
    assert!(
        b.x <= ink.x && b.y <= ink.y && b.right() >= ink.right() && b.bottom() >= ink.bottom(),
        "{b:?} misses {ink:?}"
    );
    assert!(
        b.w < ink.w + 60.0 && b.h < ink.h + 60.0,
        "{b:?} far larger than {ink:?}"
    );
    // And on a very tall one, read at a fifth of its size: no panic, and
    // every box inside the image.
    let tall = on_canvas(&sign, 700, 9000, 30, 6000);
    let started = Instant::now();
    let lines = ocr.read(&tall, &cancel).unwrap();
    println!(
        "700 x 9000 canvas in {} ms reads {:?}",
        started.elapsed().as_millis(),
        lines.iter().map(|l| &l.text).collect::<Vec<_>>()
    );
    for line in &lines {
        for p in line.quad.points {
            assert!(
                (0.0..=700.0).contains(&p.x) && (0.0..=9000.0).contains(&p.y),
                "{p:?}"
            );
        }
    }

    // Deep and float pixels read like 8-bit ones.
    let page = decode("panel_dark.png");
    let mut deep = Vec::new();
    let mut float = Vec::new();
    for p in page.data().chunks_exact(4) {
        for &c in p {
            deep.extend_from_slice(&(u16::from(c) * 257).to_ne_bytes());
            let s = f32::from(c) / 255.0;
            let linear = if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            };
            float.extend_from_slice(&linear.to_ne_bytes());
        }
    }
    let (w, h) = (page.width(), page.height());
    let text = |image: &ImageBuf| ocr.recognize(image, &cancel).unwrap().text();
    let want = text(&page);
    assert_eq!(
        text(&ImageBuf::new(w, h, PixelFormat::Rgba16, deep).unwrap()),
        want
    );
    assert_eq!(
        text(&ImageBuf::new(w, h, PixelFormat::RgbaF32, float).unwrap()),
        want
    );
}

#[test]
fn cancel_is_honored() {
    let ocr = PpOcr::new();
    let page = decode("page_light.png");
    // Cancelled before the call: nothing runs.
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(matches!(ocr.read(&page, &cancel), Err(Error::Cancelled)));
    assert!(matches!(
        ocr.recognize(&page, &cancel),
        Err(Error::Cancelled)
    ));
    // Cancelled while it reads, from another thread.
    engine().unwrap();
    for delay in [5u64, 60, 250] {
        let cancel = CancelToken::new();
        let canceller = {
            let cancel = cancel.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(delay));
                cancel.cancel();
                Instant::now()
            })
        };
        let result = ocr.read(&page, &cancel);
        let returned = Instant::now();
        let cancelled = canceller.join().unwrap();
        // The page takes far longer than the delay to read.
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "{delay} ms: {result:?}"
        );
        println!(
            "cancelled after {delay} ms: returned {} ms later",
            returned.saturating_duration_since(cancelled).as_millis()
        );
    }
    // The recognizer works right after.
    let lines = ocr.read(&page, &CancelToken::new()).unwrap();
    assert_eq!(lines.len(), 6);
}

#[test]
fn provider_reports_itself() {
    let ocr = PpOcr::new();
    assert_eq!(ocr.id(), "pp-ocr");
    assert_eq!(ocr.name(), "PP-OCRv6");
    assert!(ocr.available());
    // Several threads at once read the same.
    let image = decode("sign_color.png");
    let want = ocr.recognize(&image, &CancelToken::new()).unwrap();
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let image = image.clone();
            std::thread::spawn(move || PpOcr::new().recognize(&image, &CancelToken::new()).unwrap())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), want);
    }
}

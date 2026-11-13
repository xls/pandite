//! Gate G4: the app wires text recognition safely. Recognition starts only
//! after the full-resolution image was presented and runs off the event
//! loop; navigating away cancels it; selected text reaches the clipboard;
//! `ocr = off` disables it; without a working recognizer no text feature
//! shows. A fake recognizer stands in for the platform one.

mod support;

use pandite_app::settings::{Settings, TextRecognition};
use pandite_core::{DecoderProvider, DrawList, Point, Size};
use pandite_ui::{CursorShape, InputEvent, Key, Modifiers, PointerButton};
use std::sync::Arc;
use std::time::Duration;
use support::*;

/// Two lines of words, boxes as fractions of the image.
const LINES: &[&[(&str, [f32; 4])]] = &[
    &[
        ("hello", [0.10, 0.20, 0.25, 0.15]),
        ("world", [0.40, 0.20, 0.30, 0.15]),
    ],
    &[
        ("second", [0.10, 0.50, 0.30, 0.15]),
        ("line", [0.45, 0.50, 0.20, 0.15]),
    ],
];
const ALL_TEXT: &str = "hello world\nsecond line";

/// The center of word `i` (reading order) as a fraction of the image.
fn word_frac(i: usize) -> (f32, f32) {
    let [x, y, w, h] = LINES.iter().flat_map(|l| l.iter()).nth(i).unwrap().1;
    (x + w * 0.5, y + h * 0.5)
}

/// Where point `p` of the displayed image lands in the window now.
fn screen(h: &mut Harness, p: Point) -> Point {
    let now = h.now;
    let mut list = DrawList::new();
    let out = h.app().frame(now, &Measure, &mut list);
    let (w, hh) = h.app().display_dims().unwrap();
    let r = out
        .view
        .image_rect(Size::new(w as f32, hh as f32), out.viewport);
    Point::new(r.x + p.x * r.w / w as f32, r.y + p.y * r.h / hh as f32)
}

/// The window position of word `i` of an unedited `w` x `h` image.
fn word_at(h: &mut Harness, i: usize, w: u32, hh: u32) -> Point {
    let (fx, fy) = word_frac(i);
    screen(h, Point::new(fx * w as f32, fy * hh as f32))
}

/// A left drag through the core, as the shell feeds it.
fn drag(h: &mut Harness, a: Point, b: Point) {
    h.input(InputEvent::PointerMove { pos: a });
    h.input(InputEvent::PointerDown {
        pos: a,
        button: PointerButton::Left,
    });
    let mid = Point::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5);
    h.input(InputEvent::PointerMove { pos: mid });
    h.input(InputEvent::PointerMove { pos: b });
    h.input(InputEvent::PointerUp {
        pos: b,
        button: PointerButton::Left,
    });
}

/// Draw a frame (the full image is presented), let the recognition delay
/// pass and draw again: recognition of the image on screen starts.
fn settle(h: &mut Harness) {
    h.frame();
    h.advance(300);
    h.pump();
    h.frame();
}

fn last_cursor(h: &Harness) -> Option<CursorShape> {
    lock(&h.window).cursors.last().copied()
}

#[test]
fn ocr_runs_after_full_image_off_event_loop() {
    let dir = TempDir::new("ocr-order");
    let a = dir.join("a.stg");
    write_staged(&a, 2000, 1500);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let fake = FakeText::new(LINES);
    let mut h = Harness::new(Config {
        open: Some(a.clone()),
        registry: Some(fake.registry(&[Arc::clone(&dec) as Arc<dyn DecoderProvider>])),
        ..Config::default()
    });
    // The test decides when a frame counts as presented.
    h.auto_present = false;
    h.start();
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    // Only the reduced stage is on screen: nothing is read, however long
    // it stays there.
    for _ in 0..5 {
        h.advance(200);
        h.pump();
        h.frame();
        h.present();
    }
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());

    dec.release_full("a.stg");
    assert!(h.wait_frames(|app, ups| {
        !app.shows_reduced() && ups.last().is_some_and(|u| u.full_resolution)
    }));
    // The full image went to the shell but no frame with it was presented
    // yet: still nothing.
    for _ in 0..3 {
        h.advance(200);
        h.pump();
        h.frame();
    }
    std::thread::sleep(Duration::from_millis(30));
    h.pump();
    assert!(fake.calls().is_empty(), "{:?}", fake.calls());

    // Presented: the text is read once the image stayed a moment.
    let presented = h.now;
    h.present();
    h.frame();
    assert!(
        fake.recognitions().is_empty(),
        "waits for the image to settle"
    );
    let due = h.app().next_frame_at().expect("a frame is scheduled");
    assert!(
        due <= presented + Duration::from_millis(250),
        "the start is scheduled: {due:?}"
    );
    h.advance(300);
    h.frame();
    assert!(fake.wait_started(1));
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    let main = std::thread::current().id();
    let calls = fake.calls();
    assert!(
        calls.iter().all(|c| c.thread != main),
        "a recognizer call ran on the event-loop thread: {calls:?}"
    );
    assert_eq!(fake.recognitions().len(), 1);
    assert_eq!(
        fake.recognitions()[0].dims,
        (2000, 1500),
        "the full resolution is read"
    );
    assert_eq!(h.app().shown_text().unwrap().text(), ALL_TEXT);
    assert!(!h.app().reading_text());

    // The pointer over a word is a text cursor.
    let p = word_at(&mut h, 1, 2000, 1500);
    h.input(InputEvent::PointerMove { pos: p });
    assert_eq!(last_cursor(&h), Some(CursorShape::Text));
    // Later frames read nothing again.
    for _ in 0..3 {
        h.advance(500);
        h.present();
        h.frame();
    }
    assert_eq!(fake.recognitions().len(), 1);
    dec.release_all();
}

#[test]
fn navigation_cancels_ocr() {
    let (_dir, paths) = folder("ocr-nav", &["a.png", "b.png"], 600, 300);
    let fake = FakeText::new(LINES);
    fake.hold();
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|app| app.count() == 2));
    settle(&mut h);
    assert!(fake.wait_started(1));
    assert!(h.app().reading_text());

    // Moving on cancels the recognition of the image left behind.
    h.key(Key::Right, Modifiers::NONE);
    assert_eq!(h.app().target_path(), Some(paths[1].as_path()));
    assert!(!h.app().reading_text());
    assert!(
        fake.wait_cancelled(1),
        "the running recognition saw its cancel"
    );
    fake.release();
    assert!(h.wait_shown(&paths[1]));
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    assert_eq!(fake.finished(), 1, "only the image on screen was read");
    assert_eq!(fake.recognitions().len(), 2);
    assert_eq!(h.app().shown_text().unwrap().text(), ALL_TEXT);

    // Back to the first image: its text was never read, so it is now.
    h.key(Key::Left, Modifiers::NONE);
    assert!(h.wait_shown(&paths[0]));
    assert!(h.app().shown_text().is_none());
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    assert_eq!(fake.recognitions().len(), 3);
    // And forward again: the second image's text is cached for the
    // session and shows at once.
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[1]));
    h.frame();
    assert!(h.app().shown_text().is_some(), "cached text shows at once");
    assert_eq!(fake.recognitions().len(), 3, "nothing was read again");
}

#[test]
fn copy_selected_text_reaches_clipboard() {
    let (_dir, paths) = folder("ocr-copy", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    h.frame();

    // Drag from "hello" to "line", then Ctrl+C.
    let (a, b) = (word_at(&mut h, 0, 600, 300), word_at(&mut h, 3, 600, 300));
    drag(&mut h, a, b);
    h.key(Key::C, Modifiers::CTRL);
    assert_eq!(h.clipboard().as_deref(), Some(ALL_TEXT));
    // A partial selection copies only its words.
    let (a, b) = (word_at(&mut h, 1, 600, 300), word_at(&mut h, 2, 600, 300));
    drag(&mut h, a, b);
    h.key(Key::C, Modifiers::CTRL);
    assert_eq!(h.clipboard().as_deref(), Some("world\nsecond"));
    // Ctrl+Shift+C copies the file path, also while text is selected.
    h.key(Key::C, Modifiers::CTRL_SHIFT);
    let path = pandite_app::app::display_path(&paths[0]);
    assert_eq!(h.clipboard().as_deref(), Some(path.as_str()));
    assert!(h.images.images().is_empty(), "no image copy so far");
    // Ctrl+A, Ctrl+C.
    h.key(Key::A, Modifiers::CTRL);
    h.key(Key::C, Modifiers::CTRL);
    assert_eq!(h.clipboard().as_deref(), Some(ALL_TEXT));

    // Rotated a quarter turn, the words move with the image: (x, y) of the
    // 600 x 300 image is (300 - y, x) of the 300 x 600 one.
    h.key(Key::R, Modifiers::NONE);
    assert!(h.wait_idle());
    h.frame();
    assert_eq!(h.app().display_dims(), Some((300, 600)));
    let turned = |h: &mut Harness, i: usize| {
        let (fx, fy) = word_frac(i);
        screen(h, Point::new(300.0 - fy * 300.0, fx * 600.0))
    };
    let (a, b) = (turned(&mut h, 0), turned(&mut h, 1));
    drag(&mut h, a, b);
    h.key(Key::C, Modifiers::CTRL);
    assert_eq!(h.clipboard().as_deref(), Some("hello world"));
    // The edit read nothing again.
    assert_eq!(fake.recognitions().len(), 1);
}

#[test]
fn save_as_reads_the_saved_pixels_again() {
    let (dir, paths) = folder("ocr-save", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    // Rotate and save under a new name: the saved pixels are the new
    // image, and their text is read again from them.
    h.key(Key::R, Modifiers::NONE);
    assert!(h.wait_idle());
    let out = dir.join("turned.png");
    h.answer_dialog(vec![out.clone()]);
    h.act(pandite_ui::Action::SaveAs);
    assert!(h.wait_until(|app| app.shown_path() == Some(out.as_path()) && !app.is_busy()));
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    let reads: Vec<(u32, u32)> = fake.recognitions().iter().map(|c| c.dims).collect();
    assert_eq!(reads, vec![(600, 300), (300, 600)]);
    assert_eq!(h.app().shown_text().unwrap().width, 300);
}

#[test]
fn ocr_setting_off_disables_recognition() {
    let (_dir, paths) = folder("ocr-off", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        registry: Some(fake.registry(&[])),
        settings: Settings {
            ocr: TextRecognition::Off,
            ..Settings::default()
        },
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    for _ in 0..4 {
        settle(&mut h);
    }
    // Give a thread that should not exist time to call in.
    std::thread::sleep(Duration::from_millis(50));
    h.pump();
    assert!(
        fake.calls().is_empty(),
        "nothing runs, not even a check: {:?}",
        fake.calls()
    );
    assert!(h.app().shown_text().is_none());
    assert!(!h.app().reading_text());
    assert_eq!(h.app().next_frame_at(), h.app().ui().next_redraw_at());
    // No text features: the pointer over the words keeps its shape,
    // Ctrl+A selects nothing and Ctrl+C copies the image, not text.
    let p = word_at(&mut h, 0, 600, 300);
    h.input(InputEvent::PointerMove { pos: p });
    assert_ne!(last_cursor(&h), Some(CursorShape::Text));
    h.key(Key::A, Modifiers::CTRL);
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.wait_until(|a| !a.copying_image()));
    assert_eq!(h.images.images().len(), 1);
    assert_eq!(h.clipboard(), None, "no text was copied");
    h.key(Key::C, Modifiers::CTRL_SHIFT);
    let path = pandite_app::app::display_path(&paths[0]);
    assert_eq!(h.clipboard().as_deref(), Some(path.as_str()));
    // The setting stays off when the settings are written.
    assert_eq!(h.app().settings().ocr, TextRecognition::Off);
    assert!(h.app().settings().to_text().contains("ocr = off"));
}

/// Open the shortcut overlay and the context menu, and report whether
/// either lists a text command; closes both again.
fn shows_text_commands(h: &mut Harness) -> bool {
    h.key(Key::F1, Modifiers::NONE);
    let help = h.frame();
    h.key(Key::Escape, Modifiers::NONE);
    let p = Point::new(500.0, 400.0);
    h.input(InputEvent::PointerMove { pos: p });
    h.input(InputEvent::PointerDown {
        pos: p,
        button: PointerButton::Right,
    });
    h.input(InputEvent::PointerUp {
        pos: p,
        button: PointerButton::Right,
    });
    let menu = h.frame();
    h.key(Key::Escape, Modifiers::NONE);
    assert!(has_text(&menu, "Copy file path"), "the menu opened");
    ["Select all text", "Copy selected text"]
        .iter()
        .any(|t| has_text(&help, t) || has_text(&menu, t))
}

#[test]
fn no_provider_hides_text_features() {
    // No recognizer registered (macOS and Linux today).
    let (_dir, paths) = folder("ocr-none", &["page.png", "other.png"], 600, 300);
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        ..Config::default()
    });
    assert!(h.registry.text_providers().is_empty());
    h.start();
    assert!(h.wait_shown(&paths[0]));
    settle(&mut h);
    settle(&mut h);
    assert!(h.app().shown_text().is_none() && !h.app().reading_text());
    let p = word_at(&mut h, 0, 600, 300);
    h.input(InputEvent::PointerMove { pos: p });
    assert_ne!(last_cursor(&h), Some(CursorShape::Text));
    assert!(!shows_text_commands(&mut h));
    h.key(Key::A, Modifiers::CTRL);
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.wait_until(|a| !a.copying_image()));
    assert_eq!(h.images.images().len(), 1, "Ctrl+C copied the image");
    assert_eq!(h.clipboard(), None, "no text was copied");
    assert!(h.app().notice().is_none());

    // A recognizer that is registered but has no language: it is asked
    // once, off the event loop, never used, and the user sees nothing.
    let fake = FakeText::unavailable();
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    settle(&mut h);
    assert!(h.wait_until(|app| !app.reading_text()));
    let calls = fake.calls();
    assert!(!calls.is_empty(), "the job asked the recognizer");
    assert!(fake.recognitions().is_empty(), "{calls:?}");
    let main = std::thread::current().id();
    assert!(calls.iter().all(|c| c.thread != main), "{calls:?}");
    let list = h.frame();
    for word in ["unavailable", "Could not", "OCR", "language"] {
        assert!(!has_text(&list, word), "a message about {word} shows");
    }
    assert!(h.app().notice().is_none());
    assert!(!shows_text_commands(&mut h));
    let p = word_at(&mut h, 0, 600, 300);
    h.input(InputEvent::PointerMove { pos: p });
    assert_ne!(last_cursor(&h), Some(CursorShape::Text));
    // The next image asks nothing again.
    let asked = fake.calls().len();
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[1]));
    settle(&mut h);
    std::thread::sleep(Duration::from_millis(30));
    h.pump();
    assert_eq!(fake.calls().len(), asked);
}

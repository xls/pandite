//! Gate G4: the app wires text recognition safely. Recognition starts only
//! after the full-resolution image was presented and runs off the event
//! loop; navigating away cancels it; selected text reaches the clipboard;
//! `ocr = off` disables it; without a working recognizer no text feature
//! shows. A turn of the picture reads the text again in the new turn, a
//! crop does not, and turning back uses the stored reading. A fake
//! recognizer stands in for the platform one.

mod support;

use pandite_app::settings::{Settings, TextRecognition};
use pandite_core::{DecoderProvider, DrawList, ImageBuf, Point, Rect, Size, TextLayout};
use pandite_ui::{Action, CursorShape, InputEvent, Key, Modifiers, PixelRect, PointerButton};
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
    // Within the start delay after the turn nothing is read again; the
    // words read before it follow the picture.
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
    // No recognizer registered (the test registry has none).
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

// ----- reading again after a turn ---------------------------------------------

/// The top-left pixel [`FakeText::upright_only`] looks for.
const MARK: [u8; 4] = [255, 0, 255, 255];

/// A `w` x `h` photo stored on its side: a quarter turn to the right
/// brings its marked bottom-left pixel to the top-left corner.
fn sideways(w: u32, h: u32) -> ImageBuf {
    image(w, h, |x, y| {
        if (x, y) == (0, h - 1) {
            MARK
        } else {
            [(x * 9) as u8, (y * 9) as u8, 60, 255]
        }
    })
}

/// A harness on `path` with `fake` as the recognizer, its text read once.
fn read_once(path: &std::path::Path, fake: &Arc<FakeText>) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(path.to_path_buf()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(path));
    settle(&mut h);
    assert!(h.wait_until(|app| app.shown_text().is_some() && !app.text_pending()));
    h.frame();
    h
}

/// The text on screen, once read.
fn words(h: &mut Harness) -> Option<TextLayout> {
    h.app().shown_text().cloned()
}

/// Press `key`, wait for the edit and draw a frame: the new picture is
/// presented.
fn edit(h: &mut Harness, key: Key, mods: Modifiers) {
    h.key(key, mods);
    assert!(h.wait_idle());
    h.frame();
}

/// Let the start delay pass and wait until the reading it starts is on
/// screen; `n` readings ran by then.
fn read_turned(h: &mut Harness, fake: &FakeText, n: usize) {
    assert!(h.app().text_pending(), "a new reading waits");
    h.advance(300);
    h.pump();
    h.frame();
    assert!(h.wait_until(|app| !app.text_pending()));
    h.frame();
    assert_eq!(fake.recognitions().len(), n);
}

/// The sizes of the images the recognizer read, in order.
fn reads(fake: &FakeText) -> Vec<(u32, u32)> {
    fake.recognitions().iter().map(|c| c.dims).collect()
}

/// Clear the text clipboard, press Ctrl+C and wait for an image copy to
/// end. The text on the clipboard: `None` when Ctrl+C copied the image
/// because no text was selected.
fn ctrl_c(h: &mut Harness) -> Option<String> {
    *lock(&h.clipboard) = None;
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.wait_until(|a| !a.copying_image()));
    h.clipboard()
}

/// The window position of word `i` when the text was read from the
/// picture on screen as it stands: its box is a fraction of that picture.
fn shown_word(h: &mut Harness, i: usize) -> Point {
    let (w, hh) = h.app().display_dims().unwrap();
    word_at(h, i, w, hh)
}

/// Select from word `a` to word `b` of the picture on screen and copy.
fn copy_words(h: &mut Harness, a: usize, b: usize) -> Option<String> {
    let (pa, pb) = (shown_word(h, a), shown_word(h, b));
    drag(h, pa, pb);
    ctrl_c(h)
}

fn near(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.01
        && (a.y - b.y).abs() < 0.01
        && (a.w - b.w).abs() < 0.01
        && (a.h - b.h).abs() < 0.01
}

#[test]
fn text_read_again_after_rotation() {
    // A photo stored on its side and a recognizer that reads only upright
    // text: as stored, nothing is found.
    let dir = TempDir::new("ocr-turn");
    let path = dir.join("sideways.png");
    write_png(&path, &sideways(300, 600));
    let fake = FakeText::upright_only(LINES, MARK);
    let mut h = read_once(&path, &fake);
    assert!(
        words(&mut h).unwrap().is_empty(),
        "sideways text is not read"
    );
    assert_eq!(reads(&fake), vec![(300, 600)]);

    // Turned upright: the new picture comes first and stays for the start
    // delay, then the image is read again, turned the way it is on screen.
    h.key(Key::R, Modifiers::NONE);
    assert!(h.wait_idle());
    assert_eq!(h.app().display_dims(), Some((600, 300)));
    h.frame();
    let presented = h.now;
    assert!(h.app().text_pending(), "a new reading waits");
    let due = h.app().next_frame_at().expect("a frame is scheduled");
    assert!(
        due <= presented + Duration::from_millis(250),
        "the start is scheduled: {due:?}"
    );
    h.pump();
    std::thread::sleep(Duration::from_millis(30));
    h.pump();
    assert_eq!(fake.recognitions().len(), 1, "nothing before the delay");
    read_turned(&mut h, &fake, 2);
    assert_eq!(
        reads(&fake),
        vec![(300, 600), (600, 300)],
        "the turned image is read"
    );
    assert_eq!(words(&mut h).unwrap().text(), ALL_TEXT);
    let main = std::thread::current().id();
    let calls = fake.calls();
    assert!(
        calls.iter().all(|c| c.thread != main),
        "a recognizer call ran on the event-loop thread: {calls:?}"
    );

    // The new text is there to use: a text cursor over a word, and the
    // words copy.
    let p = shown_word(&mut h, 2);
    h.input(InputEvent::PointerMove { pos: p });
    assert_eq!(last_cursor(&h), Some(CursorShape::Text));
    h.key(Key::A, Modifiers::CTRL);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some(ALL_TEXT));
    // Later frames read nothing again.
    for _ in 0..3 {
        h.advance(500);
        h.pump();
        h.frame();
    }
    assert_eq!(fake.recognitions().len(), 2);
}

#[test]
fn text_boxes_match_turned_image() {
    // Upright only: the boxes read from the turned picture land on it.
    let dir = TempDir::new("ocr-boxes");
    let path = dir.join("sideways.png");
    write_png(&path, &sideways(300, 600));
    let fake = FakeText::upright_only(LINES, MARK);
    let mut h = read_once(&path, &fake);
    edit(&mut h, Key::R, Modifiers::NONE);
    read_turned(&mut h, &fake, 2);
    // The layout stays in pixels of the image as decoded: "hello", read at
    // (60, 60) to (210, 105) of the 600 x 300 turned picture, lies at
    // (60, 390) to (105, 540) of the 300 x 600 image.
    let layout = words(&mut h).unwrap();
    assert_eq!((layout.width, layout.height), (300, 600));
    let hello = layout.lines[0].words[0].quad.bounds();
    assert!(
        near(hello, Rect::new(60.0, 390.0, 45.0, 150.0)),
        "{hello:?}"
    );
    // On screen every word is where the recognizer saw it.
    assert_eq!(copy_words(&mut h, 0, 1).as_deref(), Some("hello world"));
    assert_eq!(copy_words(&mut h, 1, 2).as_deref(), Some("world\nsecond"));
    assert_eq!(copy_words(&mut h, 3, 2).as_deref(), Some("second line"));
    let gap = screen(&mut h, Point::new(300.0, 0.42 * 300.0));
    h.input(InputEvent::PointerMove { pos: gap });
    assert_ne!(last_cursor(&h), Some(CursorShape::Text), "between lines");

    // Every turn: each reading's boxes land on the picture it was read
    // from, mirrored or not.
    let (_dir, paths) = folder("ocr-boxes-all", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = read_once(&paths[0], &fake);
    let steps = [
        (Key::R, (300, 600)),
        (Key::H, (300, 600)),
        (Key::V, (300, 600)),
        (Key::L, (600, 300)),
    ];
    for (n, (key, dims)) in steps.into_iter().enumerate() {
        edit(&mut h, key, Modifiers::NONE);
        read_turned(&mut h, &fake, n + 2);
        assert_eq!(h.app().display_dims(), Some(dims), "{key:?}");
        assert_eq!(reads(&fake)[n + 1], dims, "{key:?}: read turned");
        let layout = words(&mut h).unwrap();
        assert_eq!((layout.width, layout.height), (600, 300), "{key:?}");
        assert_eq!(
            copy_words(&mut h, 0, 1).as_deref(),
            Some("hello world"),
            "{key:?}"
        );
        assert_eq!(
            copy_words(&mut h, 1, 2).as_deref(),
            Some("world\nsecond"),
            "{key:?}"
        );
    }
}

#[test]
fn text_turn_back_uses_stored_reading() {
    let (_dir, paths) = folder("ocr-back", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = read_once(&paths[0], &fake);
    let upright = words(&mut h).unwrap();
    assert_eq!(fake.recognitions().len(), 1);
    edit(&mut h, Key::R, Modifiers::NONE);
    read_turned(&mut h, &fake, 2);
    let right = words(&mut h).unwrap();
    assert_ne!(right, upright, "read again, turned");

    // Turning back shows the stored reading as soon as the picture is on
    // screen, without a new reading; so does turning forward again.
    edit(&mut h, Key::L, Modifiers::NONE);
    assert_eq!(words(&mut h).as_ref(), Some(&upright));
    assert!(!h.app().text_pending());
    edit(&mut h, Key::R, Modifiers::NONE);
    assert_eq!(words(&mut h).as_ref(), Some(&right));
    assert!(!h.app().text_pending());
    // Undo of a turn follows the same rule.
    edit(&mut h, Key::Z, Modifiers::CTRL);
    assert_eq!(words(&mut h).as_ref(), Some(&upright));
    edit(&mut h, Key::Z, Modifiers::CTRL);
    assert_eq!(words(&mut h).as_ref(), Some(&right));
    edit(&mut h, Key::Z, Modifiers::CTRL);
    assert_eq!(words(&mut h).as_ref(), Some(&upright));
    assert!(!h.app().is_edited());
    assert_eq!(fake.recognitions().len(), 2);

    // Four quick quarter turns, none on screen for the start delay: the
    // turns on the way are not read, and the last one is upright again.
    for _ in 0..4 {
        edit(&mut h, Key::R, Modifiers::NONE);
    }
    assert_eq!(words(&mut h).as_ref(), Some(&upright));
    assert!(!h.app().text_pending());

    // A mirror changes the turn: read again. Mirroring back and undoing
    // that use the stored readings.
    edit(&mut h, Key::H, Modifiers::NONE);
    read_turned(&mut h, &fake, 3);
    let mirrored = words(&mut h).unwrap();
    assert_ne!(mirrored, upright);
    edit(&mut h, Key::H, Modifiers::NONE);
    assert_eq!(words(&mut h).as_ref(), Some(&upright));
    edit(&mut h, Key::Z, Modifiers::CTRL);
    assert_eq!(words(&mut h).as_ref(), Some(&mirrored));
    // A flip top to bottom and a half turn undo a mirror, and mirror
    // again: both pictures were read before.
    for want in [&upright, &mirrored] {
        edit(&mut h, Key::V, Modifiers::NONE);
        edit(&mut h, Key::R, Modifiers::NONE);
        edit(&mut h, Key::R, Modifiers::NONE);
        assert_eq!(words(&mut h).as_ref(), Some(want));
        assert!(!h.app().text_pending());
    }

    for _ in 0..3 {
        h.advance(500);
        h.pump();
        h.frame();
    }
    assert_eq!(reads(&fake), vec![(600, 300), (300, 600), (600, 300)]);
}

#[test]
fn text_crop_starts_no_reading() {
    let (_dir, paths) = folder("ocr-crop", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = read_once(&paths[0], &fake);
    let read = words(&mut h).unwrap();

    // Keep x 200 to 600: "world" and "line" stay on the picture.
    h.act(Action::ApplyCrop(PixelRect {
        x: 200,
        y: 0,
        width: 400,
        height: 300,
    }));
    assert!(h.wait_idle());
    h.frame();
    assert_eq!(h.app().display_dims(), Some((400, 300)));
    assert!(!h.app().text_pending(), "a crop keeps the turn");
    for _ in 0..3 {
        h.advance(500);
        h.pump();
        h.frame();
    }
    std::thread::sleep(Duration::from_millis(30));
    h.pump();
    assert_eq!(fake.recognitions().len(), 1, "a crop reads nothing");
    assert_eq!(words(&mut h).as_ref(), Some(&read));
    // The words still land on the cropped picture: "world" is centered at
    // (130, 82.5) of it and "line" at (130, 172.5).
    let a = screen(&mut h, Point::new(130.0, 82.5));
    let b = screen(&mut h, Point::new(130.0, 172.5));
    drag(&mut h, a, b);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some("world\nline"));

    // A turn after the crop reads the whole image turned, not only the
    // part the crop keeps. Its words land on the cropped and turned
    // picture: only "second" and "line" lie in it.
    edit(&mut h, Key::R, Modifiers::NONE);
    read_turned(&mut h, &fake, 2);
    assert_eq!(reads(&fake), vec![(600, 300), (300, 600)]);
    assert_eq!(h.app().display_dims(), Some((300, 400)));
    h.key(Key::A, Modifiers::CTRL);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some("second line"));

    // Cropping the turned picture reads nothing either.
    h.act(Action::ApplyCrop(PixelRect {
        x: 0,
        y: 0,
        width: 300,
        height: 200,
    }));
    assert!(h.wait_idle());
    h.frame();
    assert_eq!(h.app().display_dims(), Some((300, 200)));
    assert!(!h.app().text_pending());
    for _ in 0..3 {
        h.advance(500);
        h.pump();
        h.frame();
    }
    assert_eq!(fake.recognitions().len(), 2);
    h.key(Key::A, Modifiers::CTRL);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some("second line"));
}

#[test]
fn text_selection_cleared_on_new_reading() {
    let (_dir, paths) = folder("ocr-sel", &["page.png"], 600, 300);
    let fake = FakeText::new(LINES);
    let mut h = read_once(&paths[0], &fake);
    h.key(Key::A, Modifiers::CTRL);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some(ALL_TEXT));

    // A turn keeps the selection on the words read before it until the
    // new reading arrives.
    edit(&mut h, Key::R, Modifiers::NONE);
    assert!(h.app().text_pending());
    assert_eq!(
        ctrl_c(&mut h).as_deref(),
        Some(ALL_TEXT),
        "the old words are still selected"
    );
    assert!(h.images.images().is_empty());
    read_turned(&mut h, &fake, 2);
    // The new words replaced the old ones, and the selection went with
    // them: Ctrl+C copies the image.
    assert_eq!(ctrl_c(&mut h), None);
    assert_eq!(h.images.images().len(), 1);
    // A selection on the new words works as before.
    h.key(Key::A, Modifiers::CTRL);
    assert_eq!(ctrl_c(&mut h).as_deref(), Some(ALL_TEXT));
    assert_eq!(copy_words(&mut h, 2, 3).as_deref(), Some("second line"));

    // A stored reading that replaces the text clears the selection too.
    edit(&mut h, Key::L, Modifiers::NONE);
    assert_eq!(fake.recognitions().len(), 2);
    assert_eq!(ctrl_c(&mut h), None);
    assert_eq!(h.images.images().len(), 2);
}

//! Ctrl+C puts the image on screen on the clipboard: full resolution, with
//! the view edits applied (what Save As writes), as 8-bit RGBA, and for an
//! animation the frame on screen. The conversion and the clipboard
//! hand-over run on a worker; a second request never starts a second job.
//! A fake clipboard records every call; nothing reaches the real one.

mod support;

use pandite_core::{
    Content, DecodeOptions, EncodeOptions, EncoderProvider, ImageBuf, PixelFormat, Point, Source,
};
use pandite_ui::{Action, InputEvent, Key, Modifiers, PixelRect};
use std::path::Path;
use std::time::Duration;
use support::*;

/// Wait until no copy runs or waits; the clipboard then holds `n` images.
fn copied(h: &mut Harness, n: usize) -> Vec<ImageBuf> {
    assert!(h.wait_until(|a| !a.copying_image()), "the copy ends");
    let images = h.images.images();
    assert_eq!(images.len(), n, "images on the clipboard");
    images
}

fn decode_file(path: &Path) -> Content {
    let reg = pandite_codecs::default_registry();
    let source = Source::from_path(path).unwrap();
    reg.decode(&source, &DecodeOptions::default()).unwrap()
}

fn open(path: &Path) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(path.to_path_buf()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(path));
    h.frame();
    h
}

#[test]
fn copy_image_reaches_clipboard_with_edits() {
    let dir = TempDir::new("copyimage");
    let a = dir.join("a.png");
    let original = numbered(6, 4);
    write_png(&a, &original);
    let mut h = open(&a);

    // Unedited: the file's pixels, and a confirmation.
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 1);
    assert_eq!(got[0].format(), PixelFormat::Rgba8);
    assert_eq!(pixels(&got[0]), pixels(&original));
    assert!(has_text(&h.frame(), "Image copied"));
    assert_eq!(h.clipboard(), None, "no text went to the clipboard");

    // Rotate right, mirror and crop, and press Ctrl+C while the edits
    // still run: the copy waits for them.
    h.act(Action::RotateRight);
    h.act(Action::FlipHorizontal);
    h.act(Action::ApplyCrop(PixelRect {
        x: 1,
        y: 1,
        width: 3,
        height: 4,
    }));
    assert!(h.app().is_busy(), "the edits run");
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 2);
    assert_eq!((got[1].width(), got[1].height()), (3, 4));
    // A quarter turn right and a mirror make a transpose; the crop then
    // starts at (1, 1).
    let expected: Vec<[u8; 4]> = (0..4)
        .flat_map(|y| (0..3).map(move |x| (x, y)))
        .map(|(x, y)| pixels(&original)[((x + 1) * 6 + (y + 1)) as usize])
        .collect();
    assert_eq!(pixels(&got[1]), expected);
    // Exactly what Save As writes.
    assert!(h.wait_idle());
    let b = dir.join("b.png");
    h.answer_dialog(vec![b.clone()]);
    h.act(Action::SaveAs);
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(b.as_path())));
    let (saved, _) = decode(&b);
    assert_eq!(
        pixels(&got[1]),
        pixels(&saved),
        "the copy equals the saved file"
    );

    // A 16-bit image arrives as 8-bit RGBA.
    let deep = dir.join("deep.png");
    let mut data = Vec::new();
    for i in 0..(5 * 3) as u16 {
        for v in [i * 4000, 65_535 - i * 3000, 30_000 + i, 65_535 - i * 1000] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
    }
    let wide = ImageBuf::new(5, 3, PixelFormat::Rgba16, data).unwrap();
    let mut bytes = Vec::new();
    pandite_codecs::PngEncoder
        .encode(&wide, &EncodeOptions::default(), &mut bytes)
        .unwrap();
    std::fs::write(&deep, bytes).unwrap();
    let mut h = open(&deep);
    assert_eq!(h.app().shown_image().unwrap().format(), PixelFormat::Rgba16);
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 1);
    assert_eq!(got[0].format(), PixelFormat::Rgba8);
    let content = decode_file(&deep);
    let want = pandite_ops::to_rgba8(content.first_image()).unwrap();
    assert_eq!(pixels(&got[0]), pixels(&want));

    // A reduced stand-in on screen: the copy waits for full resolution,
    // says so, and copies the full image, never the stand-in.
    let big = dir.join("big.stg");
    write_staged(&big, 2400, 1600);
    let dec = GatedDecoder::new();
    dec.hold_full("big.stg");
    let mut h = Harness::new(Config {
        open: Some(big.clone()),
        registry: Some(dec.registry()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    h.key(Key::C, Modifiers::CTRL);
    assert!(has_text(
        &h.frame(),
        "Copying when the full image has loaded"
    ));
    for _ in 0..5 {
        h.advance(100);
        h.pump();
        h.frame();
    }
    assert!(h.app().copying_image(), "the copy waits");
    assert_eq!(h.images.writers(), 0, "nothing copied from the stand-in");
    dec.release_full("big.stg");
    let got = copied(&mut h, 1);
    assert_eq!((got[0].width(), got[0].height()), (2400, 1600));
    assert!(
        got[0].data() == staged_pattern(2400, 1600).data(),
        "full pixels"
    );

    // An animation: the frame on screen, as the shell reports it.
    let gif = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.gif");
    let Content::Animation(anim) = decode_file(&gif) else {
        panic!("smoke.gif is an animation");
    };
    let first = anim.frames()[0].image.clone();
    let k = (1..anim.frames().len())
        .find(|&i| anim.frames()[i].image.data() != first.data())
        .expect("a frame that differs from the first");
    let mut h = open(&gif);
    let generation = h
        .updates
        .iter()
        .rev()
        .find(|u| u.full_resolution)
        .unwrap()
        .generation;
    // Before the shell reported a frame, the first one is copied.
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 1);
    assert!(got[0].data() == first.data(), "frame 0");
    h.app().frame_on_screen(generation, k);
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 2);
    let want = pandite_ops::to_rgba8(&anim.frames()[k].image).unwrap();
    assert!(got[1].data() == want.data(), "frame {k} on screen");
    // A report about older content does not count.
    h.app().frame_on_screen(generation - 1, k);
    h.key(Key::C, Modifiers::CTRL);
    let got = copied(&mut h, 3);
    assert!(got[2].data() == first.data());
}

#[test]
fn copy_image_runs_off_event_loop() {
    let (_dir, paths) = folder("copythread", &["a.png", "b.png"], 64, 48);
    let mut h = open(&paths[0]);
    assert!(h.wait_until(|a| a.count() == 2));
    let event_loop = std::thread::current().id();

    // The clipboard hand-over hangs (another program holds the
    // clipboard): Ctrl+C returns at once and the copy runs on a worker.
    h.images.hold();
    {
        let _guard = Watchdog::arm("Ctrl+C", QUICK);
        h.key(Key::C, Modifiers::CTRL);
    }
    assert!(h.images.wait_started(1));
    assert_ne!(
        h.images.threads()[0],
        event_loop,
        "the write runs on a worker"
    );
    assert!(h.app().copying_image());
    // The event loop keeps turning: input, wakes and frames.
    for i in 0..100u32 {
        let _guard = Watchdog::arm("one event loop turn", QUICK);
        h.advance(16);
        h.input(InputEvent::PointerMove {
            pos: Point::new(100.0 + i as f32, 300.0),
        });
        h.pump();
        h.frame();
    }
    // A second Ctrl+C for the same picture starts nothing and says why.
    h.key(Key::C, Modifiers::CTRL);
    assert!(has_text(&h.frame(), "Still copying the image"));
    assert_eq!(h.images.writers(), 1);
    // A new picture (rotated): its copy waits for the running one, which
    // is out of date now. More presses still make one waiting copy.
    h.key(Key::R, Modifiers::NONE);
    assert!(h.wait_idle());
    for _ in 0..3 {
        h.key(Key::C, Modifiers::CTRL);
    }
    assert_eq!(h.images.writers(), 1, "no second job while one runs");
    // The clipboard comes free: the out-of-date copy stops before it
    // touches the clipboard, then the waiting one runs.
    h.images.release();
    let images = copied(&mut h, 1);
    assert_eq!(
        (images[0].width(), images[0].height()),
        (48, 64),
        "only the rotated picture reached the clipboard"
    );
    assert_eq!(h.images.writers(), 2);
    assert!(h.images.threads().iter().all(|t| *t != event_loop));
    assert!(has_text(&h.frame(), "Image copied"));

    // The last copy wins: Ctrl+Shift+C during an image copy keeps the
    // path on the clipboard.
    h.advance(5000);
    h.images.hold();
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.images.wait_started(3));
    h.key(Key::C, Modifiers::CTRL_SHIFT);
    h.images.release();
    copied(&mut h, 1);
    let path = pandite_app::app::display_path(&paths[0]);
    assert_eq!(h.clipboard().as_deref(), Some(path.as_str()));
    assert!(
        !has_text(&h.frame(), "Image copied"),
        "the dropped copy is silent"
    );

    // Escape during a hanging copy quits without waiting for it.
    h.images.hold();
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[1]));
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.images.wait_started(4));
    {
        let _guard = Watchdog::arm("Escape during a copy", QUICK);
        h.key(Key::Escape, Modifiers::NONE);
    }
    assert!(h.app().exit_requested());
    let app = h.app.take().unwrap();
    let t = std::time::Instant::now();
    {
        let _guard = Watchdog::arm("shutdown during a copy", QUICK);
        let _ = app.shutdown(Duration::from_secs(2));
    }
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(
        h.images.images().len(),
        1,
        "the hanging copy was not joined"
    );
}

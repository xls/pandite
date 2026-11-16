//! The information panel: the I key, its place in the window, the rows it
//! shows for stills and animations, and that it follows the current image
//! without becoming a dialog.

mod support;

use pandite_app::app::{display_path, App};
use pandite_app::info::{file_size, utc_time};
use pandite_app::settings::Settings;
use pandite_core::{DrawCmd, DrawList, EncodeOptions, EncoderProvider, Rect};
use pandite_ui::{InfoRow, Key, Modifiers, UiMode};
use std::path::Path;
use support::*;

/// The value of the row labelled `label`.
fn row(rows: &[InfoRow], label: &str) -> Option<String> {
    rows.iter()
        .find(|r| r.label == label)
        .map(|r| r.value.clone())
}

/// Every label of `rows`, in order.
fn labels(rows: &[InfoRow]) -> Vec<String> {
    rows.iter().map(|r| r.label.clone()).collect()
}

/// Open `path`, wait until it is on screen at full resolution, switch the
/// panel on with I and wait for the file facts.
fn open_with_panel(path: &Path, config: Config) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(path.to_path_buf()),
        ..config
    });
    h.start();
    assert!(h.wait_shown(path), "{} shown", path.display());
    assert!(h.wait_until(|a| !a.shows_reduced()));
    h.key(Key::I, Modifiers::NONE);
    assert!(h.app().info_visible(), "I switches the panel on");
    assert!(h.wait_until(|a| !a.info_pending()), "file facts arrive");
    h
}

/// Rows of the image on screen once its decode time is known.
fn settled_rows(h: &mut Harness) -> Vec<InfoRow> {
    assert!(h.wait_until(|a| !a.info_pending() && has_row(a, "Decode time")));
    h.app().info_rows()
}

fn has_row(app: &App, label: &str) -> bool {
    app.info_rows().iter().any(|r| r.label == label)
}

/// True when the draw list fills exactly `rect`.
fn fills(list: &DrawList, rect: Rect) -> bool {
    list.cmds
        .iter()
        .any(|c| matches!(c, DrawCmd::Rect { rect: r, .. } if *r == rect))
}

fn opaque(w: u32, h: u32) -> pandite_core::ImageBuf {
    image(w, h, |x, y| [(x * 8) as u8, (y * 12) as u8, 100, 255])
}

/// One little-endian TIFF directory at `at`; longer values follow it.
fn ifd(at: usize, entries: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
    let mut head = (entries.len() as u16).to_le_bytes().to_vec();
    let mut tail = Vec::new();
    let values_at = at + 2 + 12 * entries.len() + 4;
    for (tag, typ, count, value) in entries {
        head.extend_from_slice(&tag.to_le_bytes());
        head.extend_from_slice(&typ.to_le_bytes());
        head.extend_from_slice(&count.to_le_bytes());
        if value.len() <= 4 {
            let mut v = value.clone();
            v.resize(4, 0);
            head.extend_from_slice(&v);
        } else {
            head.extend_from_slice(&((values_at + tail.len()) as u32).to_le_bytes());
            tail.extend_from_slice(value);
            if tail.len() % 2 == 1 {
                tail.push(0);
            }
        }
    }
    head.extend_from_slice(&0u32.to_le_bytes());
    head.extend(tail);
    head
}

/// An EXIF block: Make and Model, then 1/250 s, f/2.8, ISO 400, 35 mm,
/// a capture date and a lens.
fn exif_block() -> Vec<u8> {
    let text = |tag: u16, s: &str| {
        let mut b = s.as_bytes().to_vec();
        b.push(0);
        (tag, 2u16, b.len() as u32, b)
    };
    let ratio = |tag: u16, n: u32, d: u32| {
        let mut b = n.to_le_bytes().to_vec();
        b.extend_from_slice(&d.to_le_bytes());
        (tag, 5u16, 1u32, b)
    };
    let ifd0 = |at: u32| {
        vec![
            text(0x010F, "TestCam"),
            text(0x0110, "X100"),
            (0x8769, 4, 1, at.to_le_bytes().to_vec()),
        ]
    };
    let exif_at = 8 + ifd(8, &ifd0(0)).len();
    let exif = vec![
        ratio(0x829A, 1, 250),
        ratio(0x829D, 28, 10),
        (0x8827, 3, 1, 400u16.to_le_bytes().to_vec()),
        text(0x9003, "2024:05:01 12:34:56"),
        ratio(0x920A, 35, 1),
        text(0xA434, "TestLens 35mm"),
    ];
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend(ifd(8, &ifd0(exif_at as u32)));
    out.extend(ifd(exif_at, &exif));
    out
}

/// A JPEG from the crate's encoder with an EXIF segment after the start
/// marker.
fn write_jpeg_with_exif(path: &Path, img: &pandite_core::ImageBuf) {
    let mut plain = Vec::new();
    pandite_codecs::JpegEncoder
        .encode(img, &EncodeOptions::default(), &mut plain)
        .unwrap();
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(exif_block());
    let mut file = plain[..2].to_vec();
    file.extend_from_slice(&[0xFF, 0xE1]);
    file.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    file.extend(app1);
    file.extend_from_slice(&plain[2..]);
    std::fs::write(path, file).unwrap();
}

#[test]
fn info_key_toggles_panel() {
    let dir = TempDir::new("info-toggle");
    let settings_path = dir.join("settings.ini");
    let a = dir.join("a.png");
    write_png(&a, &opaque(30, 20));
    let mut h = Harness::new(Config {
        open: Some(a.clone()),
        settings_path: Some(settings_path.clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&a));
    assert!(!h.app().info_visible(), "off by default");
    let list = h.frame();
    assert_eq!(h.app().ui().info_panel_rect(), None);
    assert!(!has_text(&list, "Pixels"));

    // Only the plain key: Shift+I and Ctrl+I do nothing.
    h.key(Key::I, Modifiers::SHIFT);
    h.key(Key::I, Modifiers::CTRL);
    assert!(!h.app().info_visible());

    h.key(Key::I, Modifiers::NONE);
    assert!(h.app().info_visible());
    assert!(h.wait_until(|a| !a.info_pending()));
    let list = h.frame();
    let rect = h.app().ui().info_panel_rect().expect("the panel is drawn");
    assert!(fills(&list, rect), "the panel background is drawn");
    for text in ["Pixels", "30 x 20", "Format", "PNG", "a.png"] {
        assert!(has_text(&list, text), "{text} on screen");
    }
    assert_eq!(h.app().ui().mode(), UiMode::Normal, "not a dialog");

    // The setting is written: info_panel = on.
    assert!(h.disk.wait_returned("write_atomic", "settings.ini"));
    let saved = std::fs::read_to_string(&settings_path).unwrap();
    assert!(saved.contains("info_panel = on"), "{saved}");
    assert!(Settings::parse(&saved).unwrap().info_panel);
    assert!(h.app().settings().info_panel);

    // I again hides it.
    h.key(Key::I, Modifiers::NONE);
    assert!(!h.app().info_visible());
    let list = h.frame();
    assert_eq!(h.app().ui().info_panel_rect(), None);
    assert!(!has_text(&list, "Pixels") && !fills(&list, rect));
    assert!(!h.app().settings().info_panel);

    // A start with the setting on shows the panel at once.
    let mut on = Harness::new(Config {
        open: Some(a.clone()),
        settings: Settings {
            info_panel: true,
            ..Settings::default()
        },
        ..Config::default()
    });
    on.start();
    assert!(on.wait_shown(&a));
    assert!(on.wait_until(|a| !a.info_pending()));
    on.frame();
    assert!(on.app().ui().info_panel_rect().is_some());
}

#[test]
fn info_panel_sits_bottom_left() {
    let (_dir, paths) = folder("info-place", &["a.png", "b.png"], 30, 20);
    let mut h = open_with_panel(&paths[0], Config::default());
    assert!(h.wait_until(|a| a.count() == 2));
    for (scale, w, hgt) in [
        (1.0, 1280, 800),
        (1.0, 1000, 700),
        (1.0, 640, 480),
        (1.0, 420, 300),
        (1.0, 240, 180),
        (1.0, 160, 120),
        (1.0, 120, 90),
        (2.0, 1280, 800),
    ] {
        h.app().scale_changed(scale);
        h.app().resized(w, hgt, false);
        let now = h.now;
        let mut list = DrawList::new();
        let viewport = h.app().frame(now, &Measure, &mut list).viewport;
        let at = format!("{w} x {hgt} at scale {scale}");
        let r = h
            .app()
            .ui()
            .info_panel_rect()
            .unwrap_or_else(|| panic!("no panel at {at}"));
        // Inside the window.
        assert!(
            r.x >= 0.0 && r.y >= 0.0 && r.right() <= w as f32 && r.bottom() <= hgt as f32,
            "{r:?} leaves the window at {at}"
        );
        assert!(r.w >= 1.0 && r.h >= 1.0, "{r:?} at {at}");
        // Inside the content area: above the gallery strip.
        assert!(
            r.x >= viewport.x && r.right() <= viewport.right() && r.bottom() <= viewport.bottom(),
            "{r:?} outside {viewport:?} at {at}"
        );
        if hgt >= 300 {
            assert!(viewport.bottom() < hgt as f32, "the strip shows at {at}");
        }
        // Anchored to the bottom left corner with one margin, above the
        // place of the help hint.
        let left_gap = r.x - viewport.x;
        let bottom_gap = viewport.bottom() - r.bottom();
        assert!(
            (0.0..=16.0 * scale).contains(&left_gap)
                && bottom_gap >= left_gap - 1.0
                && bottom_gap <= 40.0 * scale,
            "gaps {left_gap} and {bottom_gap} at {at}"
        );
        // Drawn there, with its first row inside it.
        assert!(fills(&list, r), "panel fill at {at}");
        let first = list
            .cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::Text { pos, text, .. } if text == "File" => Some(*pos),
                _ => None,
            })
            .unwrap_or_else(|| panic!("first row not drawn at {at}"));
        assert!(
            r.contains(first),
            "first row at {first:?} outside {r:?} at {at}"
        );
    }
}

#[test]
fn info_rows_for_still_image() {
    let dir = TempDir::new("info-still");
    let png = dir.join("photo.png");
    write_png(&png, &opaque(30, 20));
    let mut h = open_with_panel(&png, Config::default());
    let rows = settled_rows(&mut h);
    let meta = std::fs::metadata(&png).unwrap();
    let size = meta.len();
    assert_eq!(rows[0], InfoRow::new("File", "photo.png"));
    assert_eq!(row(&rows, "Folder"), Some(display_path(dir.path())));
    assert_eq!(row(&rows, "File size"), Some(file_size(size)));
    assert_eq!(row(&rows, "Modified"), utc_time(meta.modified().unwrap()));
    assert_eq!(row(&rows, "Pixels").as_deref(), Some("30 x 20"));
    assert_eq!(
        row(&rows, "Megapixels").as_deref(),
        Some("less than 0.01 MP")
    );
    assert_eq!(row(&rows, "Aspect ratio").as_deref(), Some("3:2 (1.50:1)"));
    assert_eq!(row(&rows, "Format").as_deref(), Some("PNG"));
    assert_eq!(row(&rows, "Compression").as_deref(), Some("Deflate"));
    assert_eq!(row(&rows, "Channels").as_deref(), Some("RGB (3)"));
    assert_eq!(row(&rows, "Bits per channel").as_deref(), Some("8"));
    assert_eq!(row(&rows, "Bits per pixel").as_deref(), Some("24"));
    // 30 x 20 x 24 bits = 1800 bytes before compression.
    assert_eq!(row(&rows, "Uncompressed").as_deref(), Some("1.8 KB"));
    assert_eq!(
        row(&rows, "Compression ratio"),
        Some(format!("{:.1}:1", 1800.0 / size as f64).replace(".0:", ":"))
    );
    assert_eq!(
        row(&rows, "Bits per pixel on disk"),
        Some(format!("{:.2}", size as f64 * 8.0 / 600.0))
    );
    assert!(row(&rows, "Decode time").is_some_and(|t| t.ends_with(" ms") || t.ends_with(" s")));
    let list = h.frame();
    for text in ["photo.png", "30 x 20", "Deflate", "RGB (3)", "3:2 (1.50:1)"] {
        assert!(has_text(&list, text), "{text} on screen");
    }

    // A JPEG with EXIF: codec, chroma subsampling and the camera.
    let jpg = dir.join("shot.jpg");
    write_jpeg_with_exif(&jpg, &opaque(32, 16));
    let mut h = open_with_panel(&jpg, Config::default());
    let rows = settled_rows(&mut h);
    assert_eq!(row(&rows, "Format").as_deref(), Some("JPEG"));
    assert_eq!(
        row(&rows, "Compression").as_deref(),
        Some("Baseline DCT, Huffman")
    );
    assert_eq!(row(&rows, "Channels").as_deref(), Some("YCbCr (3)"));
    assert!(row(&rows, "Chroma subsampling").is_some_and(|s| s.starts_with("4:")));
    assert_eq!(row(&rows, "Camera").as_deref(), Some("TestCam X100"));
    assert_eq!(row(&rows, "Lens").as_deref(), Some("TestLens 35mm"));
    assert_eq!(row(&rows, "Exposure").as_deref(), Some("1/250 s"));
    assert_eq!(row(&rows, "Aperture").as_deref(), Some("f/2.8"));
    assert_eq!(row(&rows, "ISO").as_deref(), Some("400"));
    assert_eq!(row(&rows, "Focal length").as_deref(), Some("35 mm"));
    assert_eq!(row(&rows, "Taken").as_deref(), Some("2024-05-01 12:34:56"));
    let list = h.frame();
    for text in ["TestCam X100", "1/250 s", "f/2.8", "Baseline DCT, Huffman"] {
        assert!(has_text(&list, text), "{text} on screen");
    }
}

#[test]
fn info_rows_for_animation() {
    let dir = TempDir::new("info-anim");
    let gif = dir.join("spin.gif");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.gif");
    std::fs::copy(fixture, &gif).unwrap();
    let mut h = open_with_panel(&gif, Config::default());
    let rows = settled_rows(&mut h);
    assert_eq!(row(&rows, "Format").as_deref(), Some("GIF"));
    assert_eq!(row(&rows, "Compression").as_deref(), Some("LZW"));
    assert_eq!(
        row(&rows, "Channels").as_deref(),
        Some("Indexed, 4 colors (1)")
    );
    assert_eq!(row(&rows, "Bits per channel").as_deref(), Some("2"));
    assert_eq!(row(&rows, "Pixels").as_deref(), Some("48 x 32"));
    // Three frames of 100 ms that repeat forever.
    assert_eq!(row(&rows, "Frames").as_deref(), Some("3"));
    assert_eq!(row(&rows, "Duration").as_deref(), Some("0.30 s"));
    assert_eq!(row(&rows, "Frame rate").as_deref(), Some("10.0 fps"));
    assert_eq!(row(&rows, "Loop count").as_deref(), Some("Forever"));
    // 48 x 32 x 2 bits x 3 frames.
    assert_eq!(row(&rows, "Uncompressed").as_deref(), Some("1.1 KB"));
    let order = labels(&rows);
    let at = |l: &str| order.iter().position(|x| x == l).unwrap();
    assert!(at("Pixels") < at("Frames") && at("Frames") < at("Loop count"));
    let list = h.frame();
    for text in ["Frames", "0.30 s", "10.0 fps", "Forever"] {
        assert!(has_text(&list, text), "{text} on screen");
    }
}

#[test]
fn info_follows_current_image() {
    let dir = TempDir::new("info-follow");
    let a = dir.join("a.png");
    let b = dir.join("b.png");
    write_png(&a, &opaque(30, 20));
    write_png(&b, &opaque(40, 10));
    let mut h = open_with_panel(&a, Config::default());
    assert!(h.wait_until(|app| app.count() == 2));
    let rows = h.app().info_rows();
    assert_eq!(row(&rows, "File").as_deref(), Some("a.png"));
    assert_eq!(row(&rows, "Pixels").as_deref(), Some("30 x 20"));

    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&b));
    assert!(h.wait_until(|app| !app.shows_reduced() && !app.info_pending()));
    // A neighbor decoded ahead has no decode time of its own: no row.
    let rows = h.app().info_rows();
    let size_b = std::fs::metadata(&b).unwrap().len();
    assert_eq!(row(&rows, "File").as_deref(), Some("b.png"));
    assert_eq!(row(&rows, "Pixels").as_deref(), Some("40 x 10"));
    assert_eq!(row(&rows, "Aspect ratio").as_deref(), Some("4:1"));
    assert_eq!(row(&rows, "File size"), Some(file_size(size_b)));
    assert_eq!(row(&rows, "Format").as_deref(), Some("PNG"));
    let list = h.frame();
    assert!(has_text(&list, "b.png") && has_text(&list, "40 x 10"));
    assert!(!has_text(&list, "30 x 20"), "nothing of the previous image");

    // And back.
    h.key(Key::Left, Modifiers::NONE);
    assert!(h.wait_shown(&a));
    assert!(h.wait_until(|app| {
        !app.info_pending() && app.info_rows().first().is_some_and(|r| r.value == "a.png")
    }));
    let list = h.frame();
    assert!(has_text(&list, "30 x 20") && !has_text(&list, "40 x 10"));

    // A facts read stuck on a file the user left does not hold up the
    // next image.
    let c = dir.join("c.png");
    let d = dir.join("d.png");
    write_png(&c, &opaque(12, 12));
    write_png(&d, &opaque(16, 8));
    let mut h = Harness::new(Config {
        open: Some(c.clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&c));
    assert!(h.wait_until(|app| app.count() == 4));
    h.disk.stall_read("c.png");
    h.key(Key::I, Modifiers::NONE);
    assert!(h.disk.wait_call("read_head", "c.png"));
    assert!(h.app().info_pending());
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&d));
    assert!(h.wait_until(|app| !app.info_pending()));
    let rows = h.app().info_rows();
    assert_eq!(row(&rows, "File").as_deref(), Some("d.png"));
    assert_eq!(
        row(&rows, "File size"),
        Some(file_size(std::fs::metadata(&d).unwrap().len()))
    );
    h.disk.release_all();
}

#[test]
fn info_panel_keeps_its_rows_until_the_next_file_is_read() {
    let dir = TempDir::new("info-hold");
    let a = dir.join("a.png");
    let b = dir.join("b.png");
    write_png(&a, &opaque(30, 20));
    write_png(&b, &opaque(40, 10));
    let mut h = open_with_panel(&a, Config::default());
    assert!(h.wait_until(|app| app.count() == 2));
    let list = h.frame();
    assert!(has_text(&list, "30 x 20"));
    let before = h.app().ui().info_panel_rect().expect("panel");

    // The facts of b are stuck: the panel keeps the rows of a, at the
    // same size, and does not draw a short panel for b.
    h.disk.stall_read("b.png");
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&b));
    assert!(h.disk.wait_call("read_head", "b.png"));
    assert!(h.app().info_pending());
    let list = h.frame();
    assert!(has_text(&list, "30 x 20"), "the rows of a stay");
    assert_eq!(h.app().ui().info_panel_rect(), Some(before));
    assert!(
        h.app().next_frame_at().is_some(),
        "a frame is due when the wait ends"
    );

    // After a short wait the panel shows what is known of b.
    h.advance(500);
    let list = h.frame();
    assert!(!has_text(&list, "30 x 20"), "nothing of a after the wait");
    h.disk.release_all();
    assert!(h.wait_until(|app| !app.info_pending()));
    let list = h.frame();
    assert!(has_text(&list, "40 x 10"));
}

#[test]
fn info_skips_unknown_facts() {
    // A decoder that states nothing about its files but its name.
    let decoder = GatedDecoder::new();
    let dir = TempDir::new("info-bare");
    let path = dir.join("plain.stg");
    write_staged(&path, 64, 48);
    let mut h = open_with_panel(
        &path,
        Config {
            registry: Some(decoder.registry()),
            ..Config::default()
        },
    );
    let rows = settled_rows(&mut h);
    assert_eq!(
        labels(&rows),
        [
            "File",
            "Folder",
            "File size",
            "Modified",
            "Pixels",
            "Megapixels",
            "Aspect ratio",
            "Format",
            "Bits per pixel on disk",
            "Decode time",
        ]
    );
    assert_eq!(row(&rows, "Format").as_deref(), Some("Staged test"));
    for r in &rows {
        let v = r.value.trim();
        assert!(!v.is_empty(), "{} is empty", r.label);
        let guess = ["unknown", "n/a", "none"]
            .iter()
            .any(|g| v.to_lowercase() == *g);
        assert!(!guess, "{}: {v}", r.label);
    }
    let list = h.frame();
    for absent in ["Compression", "Channels", "Camera", "Frames", "ICC profile"] {
        assert!(!has_text(&list, absent), "{absent} has no row");
    }

    // A PNG without a profile, color chunk or EXIF: no color or camera rows.
    let png = dir.join("bare.png");
    write_png(&png, &opaque(8, 8));
    let mut h = open_with_panel(&png, Config::default());
    let rows = settled_rows(&mut h);
    for absent in [
        "FourCC",
        "Chroma subsampling",
        "Color space",
        "ICC profile",
        "Frames",
        "Camera",
        "Lens",
        "Exposure",
        "Aperture",
        "ISO",
        "Focal length",
        "Taken",
    ] {
        assert!(row(&rows, absent).is_none(), "{absent} has no row");
    }
}

#[test]
fn escape_quits_with_info_panel() {
    let dir = TempDir::new("info-escape");
    let a = dir.join("a.png");
    write_png(&a, &opaque(30, 20));
    let mut h = open_with_panel(&a, Config::default());
    h.frame();
    let rect = h.app().ui().info_panel_rect().expect("the panel shows");
    assert_eq!(
        h.app().ui().mode(),
        UiMode::Normal,
        "the panel is no dialog"
    );
    // A click on the panel neither pans nor changes the image.
    let view = h.app().view();
    let p = rect.center();
    h.input(pandite_ui::InputEvent::PointerMove { pos: p });
    h.input(pandite_ui::InputEvent::PointerDown {
        pos: p,
        button: pandite_ui::PointerButton::Left,
    });
    h.input(pandite_ui::InputEvent::PointerUp {
        pos: p,
        button: pandite_ui::PointerButton::Left,
    });
    assert_eq!(h.app().view(), view);
    assert!(!h.app().exit_requested());
    assert!(h.app().info_visible(), "a click leaves the panel up");
    // Escape quits at once, with the panel on screen.
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested(), "Escape quits");

    // Also while the panel waits for a file read that never returns.
    let mut h = Harness::new(Config {
        open: Some(a.clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&a));
    h.disk.stall_read("a.png");
    h.key(Key::I, Modifiers::NONE);
    assert!(
        h.disk.wait_call("read_head", "a.png"),
        "the facts read started"
    );
    assert!(h.app().info_pending());
    let watchdog = Watchdog::arm("a frame and Escape with a stuck facts read", QUICK);
    let list = h.frame();
    assert!(h.app().ui().info_panel_rect().is_some(), "the panel shows");
    assert!(has_text(&list, "30 x 20"), "what the decode told");
    assert!(!has_text(&list, "File size"), "no file facts yet");
    h.key(Key::Escape, Modifiers::NONE);
    drop(watchdog);
    assert!(h.app().exit_requested(), "Escape quits");
}

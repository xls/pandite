//! Commands end to end through the real handlers: Save As, copy path,
//! edits, display modes, full screen, navigation and bulk conversion.

mod support;

use pandite_app::cli;
use pandite_core::{DisplayMode, Point, ViewState};
use pandite_ui::{
    Action, BulkJob, BulkSource, InputEvent, Key, Modifiers, OverwritePolicy, PixelRect,
    PointerButton, ResizeSpec, WheelDelta,
};
use std::path::PathBuf;
use support::*;

fn open(path: &std::path::Path) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(path.to_path_buf()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(path), "{} did not open", path.display());
    h
}

#[test]
fn save_as_writes_decodable_file() {
    let dir = TempDir::new("saveas");
    let a = dir.join("photo.png");
    write_png(&a, &numbered(6, 4));
    let mut h = open(&a);
    let cases = [
        ("out.png", "png"),
        ("out.jpg", "jpeg"),
        ("out.webp", "webp"),
        ("out.bmp", "bmp"),
    ];
    for (name, format) in cases {
        let target = dir.join(name);
        h.answer_dialog(vec![target.clone()]);
        h.act(Action::SaveAs);
        assert!(
            h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(target.as_path())),
            "saving {name}"
        );
        let (img, id) = decode(&target);
        assert_eq!(id, format, "{name} is written as {format}");
        assert_eq!((img.width(), img.height()), (6, 4));
        if format != "jpeg" {
            assert_eq!(pixels(&img), pixels(&numbered(6, 4)), "{name} is lossless");
        }
    }
    // The dialog offered every registry encoder, the current format first.
    let shown = lock(&h.dialogs).shown.clone();
    match &shown[0] {
        pandite_app::platform::DialogKind::SaveFile {
            file_name, filters, ..
        } => {
            assert_eq!(file_name, "photo.png");
            let names: Vec<_> = filters.iter().map(|f| f.extensions[0].as_str()).collect();
            assert_eq!(names[0], "png");
            for ext in ["jpg", "webp", "bmp"] {
                assert!(names.contains(&ext), "{ext} offered");
            }
        }
        other => panic!("expected a save dialog, got {other:?}"),
    }
    // A name without a known extension gets one.
    h.answer_dialog(vec![dir.join("noext")]);
    h.act(Action::SaveAs);
    let expected = dir.join("noext.bmp");
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(expected.as_path())));
    assert_eq!(decode(&expected).1, "bmp");
}

#[test]
fn save_over_original_requires_confirmation() {
    let dir = TempDir::new("overwrite");
    let a = dir.join("a.png");
    write_png(&a, &numbered(4, 2));
    let before = std::fs::read(&a).unwrap();
    let mut h = open(&a);
    h.act(Action::RotateRight);
    assert!(h.wait_idle());

    // Choosing the original asks first; Escape answers no.
    h.answer_dialog(vec![a.clone()]);
    h.act(Action::SaveAs);
    h.pump();
    assert_eq!(h.app().ui().mode(), pandite_ui::UiMode::Modal);
    h.frame();
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.wait_idle());
    assert_eq!(
        std::fs::read(&a).unwrap(),
        before,
        "not written without a yes"
    );
    assert!(h.app().is_edited());

    // Again, and Enter confirms ("Replace" has the focus).
    h.answer_dialog(vec![a.clone()]);
    h.act(Action::SaveAs);
    h.pump();
    h.frame();
    h.key(Key::Enter, Modifiers::NONE);
    assert!(h.wait_until(|app| !app.is_busy() && !app.is_edited()));
    let (saved, _) = decode(&a);
    assert_eq!(
        (saved.width(), saved.height()),
        (2, 4),
        "the rotated image replaced it"
    );
    assert_eq!(h.app().shown_path(), Some(a.as_path()));
}

#[test]
fn copy_path_puts_absolute_path_on_clipboard() {
    let dir = TempDir::new("copypath");
    std::fs::create_dir_all(dir.join("sub dir")).unwrap();
    let a = dir.join("sub dir").join("bild \u{e4}.png");
    write_png(&a, &numbered(3, 3));
    // The command line makes a relative path absolute.
    let parsed = cli::parse(["sub dir/bild \u{e4}.png"], dir.path()).unwrap();
    let path = parsed.path.unwrap();
    assert!(path.is_absolute());
    let mut h = open(&path);
    h.key(Key::C, Modifiers::CTRL);
    let copied = h.clipboard().expect("clipboard has text");
    assert!(PathBuf::from(&copied).is_absolute(), "{copied}");
    assert_eq!(PathBuf::from(&copied), a);
    // A short confirmation shows.
    let list = h.frame();
    assert!(has_text(&list, "Path copied"));
}

#[test]
fn rotate_flip_crop_resize_change_saved_pixels() {
    let dir = TempDir::new("pixels");
    let a = dir.join("a.png");
    // 4 x 3, every pixel distinct.
    let source = numbered(4, 3);
    write_png(&a, &source);
    let mut h = open(&a);
    h.act(Action::RotateRight); // 3 x 4
    h.act(Action::FlipHorizontal);
    h.act(Action::ApplyCrop(PixelRect {
        x: 1,
        y: 1,
        width: 2,
        height: 3,
    }));
    assert!(h.wait_idle());
    let out = dir.join("edited.png");
    h.answer_dialog(vec![out.clone()]);
    h.act(Action::SaveAs);
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(out.as_path())));

    // Expected, built pixel by pixel: rotate right maps (x, y) of the
    // 4 x 3 source to (2 - y, x); a horizontal flip of the 3 x 4 result
    // maps x to 2 - x, so pixel (x, y) shows source (y, x); the crop keeps
    // x 1..3, y 1..4.
    let src = pixels(&source);
    let at = |x: u32, y: u32| src[(y * 4 + x) as usize];
    let mut expected = Vec::new();
    for y in 1..4 {
        for x in 1..3 {
            expected.push(at(y, x));
        }
    }
    let (saved, _) = decode(&out);
    assert_eq!((saved.width(), saved.height()), (2, 3));
    assert_eq!(pixels(&saved), expected);

    // Resize with Lanczos3 changes the saved size and pixels.
    h.act(Action::ApplyResize(ResizeSpec::Exact {
        width: 6,
        height: 9,
    }));
    assert!(h.wait_idle());
    let big = dir.join("big.png");
    h.answer_dialog(vec![big.clone()]);
    h.act(Action::SaveAs);
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(big.as_path())));
    let (resized, _) = decode(&big);
    assert_eq!((resized.width(), resized.height()), (6, 9));
    let reference = pandite_ops::resize(&saved, 6, 9, pandite_ops::Filter::Lanczos3).unwrap();
    assert_eq!(pixels(&resized), pixels(&reference));

    // Percent and fit rules from the resize dialog.
    h.act(Action::ApplyResize(ResizeSpec::Percent(50.0)));
    h.act(Action::ApplyResize(ResizeSpec::Fit {
        width: 2,
        height: 2,
        enlarge: false,
    }));
    assert!(h.wait_idle());
    let img = h.app().shown_image().unwrap();
    assert_eq!((img.width(), img.height()), (1, 2));
}

#[test]
fn display_mode_commands_change_view() {
    let dir = TempDir::new("view");
    let a = dir.join("wide.png");
    write_png(
        &a,
        &image(2000, 1000, |x, y| {
            [(x % 256) as u8, (y % 256) as u8, 9, 255]
        }),
    );
    let mut h = open(&a);
    h.frame();
    assert_eq!(h.app().view(), ViewState::default(), "opens in fit mode");

    h.key(Key::Digit1, Modifiers::NONE);
    assert_eq!(h.app().view().mode, DisplayMode::OneToOne);

    h.key(Key::F, Modifiers::NONE);
    assert_eq!(h.app().view().mode, DisplayMode::Fit);

    // Fit scale for 2000 x 1000 in a 1000 wide window is 0.5; one zoom step
    // multiplies it.
    h.act(Action::ZoomIn);
    match h.app().view().mode {
        DisplayMode::Zoom(z) => assert!((z - 0.5 * pandite_ui::ZOOM_STEP).abs() < 1e-4, "{z}"),
        other => panic!("expected zoom, got {other:?}"),
    }
    h.act(Action::ZoomOut);
    match h.app().view().mode {
        DisplayMode::Zoom(z) => assert!((z - 0.5).abs() < 1e-4, "{z}"),
        other => panic!("expected zoom, got {other:?}"),
    }

    // The wheel zooms at the pointer.
    let pos = Point::new(300.0, 300.0);
    h.input(InputEvent::PointerMove { pos });
    h.input(InputEvent::Wheel {
        pos,
        delta: WheelDelta::Lines { x: 0.0, y: 1.0 },
    });
    let zoomed = h.app().view();
    assert!(matches!(zoomed.mode, DisplayMode::Zoom(z) if z > 0.5));

    // Panning moves the image while it overflows.
    h.act(Action::OneToOne { anchor: None });
    let before = h.app().view().pan;
    h.act(Action::PanBy { dx: 50.0, dy: 0.0 });
    assert_ne!(h.app().view().pan, before);

    // Double click toggles between fit and 1:1.
    h.act(Action::FitToWindow);
    h.frame();
    let p = Point::new(500.0, 300.0);
    for _ in 0..2 {
        h.input(InputEvent::PointerDown {
            pos: p,
            button: PointerButton::Left,
        });
        h.input(InputEvent::PointerUp {
            pos: p,
            button: PointerButton::Left,
        });
        h.advance(50);
    }
    assert_eq!(h.app().view().mode, DisplayMode::OneToOne);
    h.act(Action::FitToWindow);
    assert_eq!(h.app().view(), ViewState::default(), "fit recenters");
}

#[test]
fn f11_toggles_fullscreen_state() {
    let (_dir, paths) = folder("fullscreen", &["a.png"], 8, 8);
    let mut h = open(&paths[0]);
    assert!(!h.app().fullscreen());
    h.key(Key::F11, Modifiers::NONE);
    assert!(h.app().fullscreen());
    h.key(Key::F11, Modifiers::NONE);
    assert!(!h.app().fullscreen());
    // Escape leaves full screen.
    h.key(Key::F11, Modifiers::NONE);
    assert!(h.app().fullscreen());
    assert!(h.app().settings().window.is_none() || h.app().settings().window.unwrap().fullscreen);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app().fullscreen());
    assert_eq!(lock(&h.window).fullscreen, vec![true, false, true, false]);
    assert!(!h.app().exit_requested());
}

#[test]
fn navigation_next_previous() {
    let (_dir, paths) = folder("nav", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[1]);
    assert!(h.wait_until(|a| a.count() == 3));
    assert_eq!(h.app().index(), Some(1));

    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[2]));
    // Wrap policy: the default wraps from the last image to the first.
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[0]));
    assert_eq!(h.app().index(), Some(0));
    h.key(Key::Left, Modifiers::NONE);
    assert!(h.wait_shown(&paths[2]));
    h.act(Action::GoTo(1));
    assert!(h.wait_shown(&paths[1]));
    assert_eq!(h.app().title(), "b.png - pandite");

    // Held keys repeat quickly: every press moves on at once and only the
    // final image ends up on screen.
    for _ in 0..20 {
        h.input(InputEvent::KeyDown {
            key: Key::Right,
            mods: Modifiers::NONE,
            repeat: true,
        });
    }
    let last = paths[(1 + 20) % 3].clone();
    assert_eq!(h.app().target_path(), Some(last.as_path()));
    assert!(h.wait_shown(&last));

    // Without wrapping, the ends stay put.
    let settings = pandite_app::settings::Settings {
        wrap_around: false,
        ..Default::default()
    };
    let mut h = Harness::new(Config {
        open: Some(paths[2].clone()),
        settings,
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[2]));
    assert!(h.wait_until(|a| a.count() == 3));
    h.key(Key::Right, Modifiers::NONE);
    h.pump();
    assert_eq!(h.app().target_path(), Some(paths[2].as_path()));
    h.key(Key::Home, Modifiers::NONE);
    assert!(h.wait_shown(&paths[0]));
    h.key(Key::Left, Modifiers::NONE);
    assert_eq!(h.app().target_path(), Some(paths[0].as_path()));
}

#[test]
fn bulk_resize_command_runs() {
    let (dir, paths) = folder("bulk", &["a.png", "b.png", "c.png"], 40, 20);
    let mut h = open(&paths[0]);
    let out = dir.join("small");
    h.act(Action::RunBulk(BulkJob {
        source: BulkSource::Folder {
            path: dir.path().to_path_buf(),
            recursive: false,
        },
        resize: Some(ResizeSpec::Percent(50.0)),
        format: Some("jpeg".into()),
        quality: 85,
        destination: out.clone(),
        overwrite: OverwritePolicy::Skip,
    }));
    assert!(h.app().bulk_running());
    // Progress shows in the interface while it runs.
    let list = h.frame();
    assert!(!list.cmds.is_empty());
    assert!(h.wait_until(|a| !a.bulk_running()));
    for name in ["a", "b", "c"] {
        let (img, id) = decode(&out.join(format!("{name}.jpg")));
        assert_eq!(id, "jpeg");
        assert_eq!((img.width(), img.height()), (20, 10));
    }
    // The result summary.
    let list = h.frame();
    assert!(has_text(&list, "3 saved"), "summary toast");

    // A second run of a file list skips what exists.
    h.act(Action::RunBulk(BulkJob {
        source: BulkSource::Files(paths.clone()),
        resize: Some(ResizeSpec::Fit {
            width: 10,
            height: 10,
            enlarge: false,
        }),
        format: Some("jpeg".into()),
        quality: 85,
        destination: out.clone(),
        overwrite: OverwritePolicy::Skip,
    }));
    assert!(h.wait_until(|a| !a.bulk_running()));
    let list = h.frame();
    assert!(has_text(&list, "3 skipped"));
}

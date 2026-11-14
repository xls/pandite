//! The app never freezes on a stalled file: the window and loading screen
//! come first, the event loop keeps turning while a read hangs forever,
//! Escape and quitting never wait for it (nor for a decode, a clipboard
//! copy or text recognition), and no file system call runs on the
//! event-loop thread.
//!
//! Stalls are latches in the test file system that only teardown opens.
//! Every "returns at once" claim is guarded by a watchdog that aborts the
//! run when the call blocks.

mod support;

use pandite_core::{DecoderProvider, Point};
use pandite_ui::{
    Action, BulkJob, BulkSource, InputEvent, Key, Modifiers, OverwritePolicy, PointerButton, UiMode,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::*;

const SPLASH_HINT: &str = "Press Esc to quit";

#[test]
fn window_appears_before_file_is_read() {
    let (_dir, paths) = folder("first", &["slow.png"], 16, 16);
    let disk = TestDisk::new();
    disk.stall_read("slow.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            size_to_image: true,
            ..Config::default()
        },
        disk,
    );
    // The first frames are built before the core may touch the file: they
    // already carry the loading screen (shown after a short delay).
    let first = h.frame();
    assert!(!first.cmds.is_empty(), "the window draws its first frame");
    h.advance(300);
    let splash = h.frame();
    assert!(has_text(&splash, SPLASH_HINT), "the loading screen is up");
    assert!(has_text(&splash, "slow.png"));
    assert!(
        h.disk.calls().is_empty(),
        "no file system call before the first frame"
    );
    assert_eq!(h.app().title(), "slow.png - Pandite");

    // Only now does the load start, on a worker.
    let now = h.now;
    {
        let _guard = Watchdog::arm("first_frame_shown", QUICK);
        h.app().first_frame_shown(now);
    }
    assert!(h.disk.wait_read_started("slow.png"));
    assert_eq!(h.disk.returned("slow.png"), 0);
    h.advance(16);
    assert!(has_text(&h.frame(), SPLASH_HINT));
}

#[test]
fn event_loop_turns_while_read_is_stalled() {
    let (_dir, paths) = folder("turns", &["stuck.png", "other.png"], 16, 16);
    let disk = TestDisk::new();
    disk.stall_read("stuck.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.disk.wait_read_started("stuck.png"));
    // Two hundred turns of the loop: input, wake handling and a frame each,
    // every one bounded, all while the read hangs.
    let started = Instant::now();
    let mut splash_frames = 0;
    for i in 0..200u32 {
        let _guard = Watchdog::arm("one event loop turn", QUICK);
        h.advance(16);
        h.input(InputEvent::PointerMove {
            pos: pandite_core::Point::new(100.0 + i as f32, 200.0),
        });
        h.pump();
        let list = h.frame();
        if has_text(&list, SPLASH_HINT) {
            splash_frames += 1;
        }
        // The loading screen animates, so the loop keeps drawing.
        assert!(h.app().needs_frame() || h.app().next_frame_at().is_some());
    }
    assert!(started.elapsed() < QUICK);
    assert!(
        splash_frames >= 190,
        "the loading screen kept drawing: {splash_frames}"
    );
    assert_eq!(h.app().loading_path(), Some(paths[0].as_path()));
    assert_eq!(h.disk.returned("stuck.png"), 0, "the read is still stuck");
    // Shortcuts still work: full screen toggles while loading.
    h.key(Key::F11, Modifiers::NONE);
    assert!(h.app().fullscreen());
}

#[test]
fn escape_while_loading_first_image_quits_at_once() {
    let (_dir, paths) = folder("escfirst", &["cloud.png"], 16, 16);
    let disk = TestDisk::new();
    disk.stall_read("cloud.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.disk.wait_read_started("cloud.png"));
    h.advance(500);
    assert!(has_text(&h.frame(), SPLASH_HINT));
    {
        let _guard = Watchdog::arm("Escape", QUICK);
        h.key(Key::Escape, Modifiers::NONE);
    }
    assert!(
        h.app().exit_requested(),
        "Escape on the first load closes the app"
    );
    let app = h.app.take().unwrap();
    let t = Instant::now();
    {
        let _guard = Watchdog::arm("shutdown", QUICK);
        let _ = app.shutdown(Duration::from_secs(2));
    }
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(
        h.disk.returned("cloud.png"),
        0,
        "nothing waited for the read"
    );
}

#[test]
fn escape_while_loading_later_image_quits() {
    let (_dir, paths) = folder("esclater", &["a.png", "b.png", "c.png"], 16, 16);
    let disk = TestDisk::new();
    disk.stall_read("b.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            settings: pandite_app::settings::Settings {
                read_ahead: 0,
                ..Default::default()
            },
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));

    h.key(Key::Right, Modifiers::NONE);
    assert!(h.disk.wait_read_started("b.png"));
    h.pump();
    assert_eq!(h.app().loading_path(), Some(paths[1].as_path()));
    assert_eq!(
        h.app().shown_path(),
        Some(paths[0].as_path()),
        "a stays on screen"
    );
    h.advance(400);
    let list = h.frame();
    assert!(
        has_text(&list, "b.png"),
        "the small loading overlay names b.png"
    );
    assert!(has_text(&list, SPLASH_HINT), "and says that Esc quits");

    // Escape quits at once: no step back to a.png first.
    {
        let _guard = Watchdog::arm("Escape", QUICK);
        h.key(Key::Escape, Modifiers::NONE);
    }
    assert!(h.app().exit_requested());
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
    let app = h.app.take().unwrap();
    let t = Instant::now();
    {
        let _guard = Watchdog::arm("shutdown", QUICK);
        let _ = app.shutdown(Duration::from_secs(2));
    }
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(h.disk.returned("b.png"), 0, "nothing waited for the read");
}

#[test]
fn escape_quits_during_stalled_load() {
    // Everything that can hang at once: text recognition held on the
    // first image, an image copy stuck in its clipboard hand-over, a full
    // decode held on the second image and a read that never returns on
    // the third.
    let dir = TempDir::new("escstall");
    let a = dir.join("a.png");
    write_png(&a, &numbered(6, 5));
    let b = dir.join("b.stg");
    write_staged(&b, 2400, 1600);
    let c = dir.join("c.png");
    write_png(&c, &numbered(4, 4));
    let fake = FakeText::new(&[&[("word", [0.1, 0.1, 0.5, 0.3])]]);
    fake.hold();
    let dec = GatedDecoder::new();
    dec.hold_full("b.stg");
    let disk = TestDisk::new();
    disk.stall_read("c.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(a.clone()),
            registry: Some(fake.registry(&[Arc::clone(&dec) as Arc<dyn DecoderProvider>])),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.wait_shown(&a));
    assert!(h.wait_until(|app| app.count() == 3));
    h.frame();
    h.advance(300);
    h.frame();
    assert!(fake.wait_started(1), "recognition runs and hangs");
    h.images.hold();
    h.key(Key::C, Modifiers::CTRL);
    assert!(h.images.wait_started(1), "the copy hangs in its hand-over");
    h.key(Key::Right, Modifiers::NONE);
    assert!(dec.wait_started("b.stg", true), "the full decode hangs");
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.disk.wait_read_started("c.png"), "the read hangs");
    h.pump();
    assert_eq!(h.app().loading_path(), Some(c.as_path()));

    {
        let _guard = Watchdog::arm("Escape with everything stuck", QUICK);
        h.key(Key::Escape, Modifiers::NONE);
    }
    assert!(h.app().exit_requested(), "Escape quits at once");
    let app = h.app.take().unwrap();
    let t = Instant::now();
    {
        let _guard = Watchdog::arm("shutdown with everything stuck", QUICK);
        let _ = app.shutdown(Duration::from_secs(2));
    }
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(h.disk.returned("c.png"), 0, "the read was not joined");
    assert_eq!(dec.full_finished("b.stg"), 0, "the decode was not joined");
    assert!(h.images.images().is_empty(), "the copy was not joined");
    dec.release_all();
    fake.release();
}

#[test]
fn escape_quits_app() {
    let (_dir, paths) = folder("escquit", &["a.png", "b.png"], 64, 48);
    let cfg = TempDir::new("escquitcfg");
    let start = |settings_path: Option<PathBuf>| {
        let mut h = Harness::new(Config {
            open: Some(paths[0].clone()),
            settings_path,
            ..Config::default()
        });
        h.start();
        assert!(h.wait_shown(&paths[0]));
        h.frame();
        h
    };

    // A plain window: Escape takes the same way out as Ctrl+Q, settings
    // included.
    let mut h = start(Some(cfg.join("settings.ini")));
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
    let app = h.app.take().unwrap();
    assert_eq!(
        app.shutdown(Duration::from_secs(5)).settings_saved,
        Some(true)
    );

    // Full screen: no step back to a window first.
    let mut h = start(None);
    h.key(Key::F11, Modifiers::NONE);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
    assert!(h.app().fullscreen());

    // Crop mode, in the middle of a drag.
    let mut h = start(None);
    h.key(Key::C, Modifiers::NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Crop);
    let p = Point::new(400.0, 300.0);
    h.input(InputEvent::PointerDown {
        pos: p,
        button: PointerButton::Left,
    });
    h.input(InputEvent::PointerMove {
        pos: Point::new(600.0, 500.0),
    });
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());

    // Unsaved edits: Escape quits like Ctrl+Q, which asks nothing either.
    let mut h = start(None);
    h.key(Key::R, Modifiers::NONE);
    assert!(h.app().is_edited());
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
    assert_eq!(h.app().ui().mode(), UiMode::Normal, "no question");

    // A dialog closes first; the next Escape quits.
    let mut h = start(None);
    h.key(Key::R, Modifiers::CTRL);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app().exit_requested());
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());

    // The question before Save As replaces the original: Escape answers
    // no and keeps the app open.
    let mut h = start(None);
    h.key(Key::R, Modifiers::NONE);
    assert!(h.wait_idle());
    h.answer_dialog(vec![paths[0].clone()]);
    h.act(Action::SaveAs);
    h.pump();
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    h.key(Key::Escape, Modifiers::NONE);
    assert!(!h.app().exit_requested());
    assert!(h.app().is_edited(), "not saved");
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());

    // Selected recognized text: Ctrl+A selects it, Escape still quits.
    let fake = FakeText::new(&[&[("word", [0.1, 0.1, 0.5, 0.3])]]);
    let mut h = Harness::new(Config {
        open: Some(paths[1].clone()),
        registry: Some(fake.registry(&[])),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[1]));
    h.frame();
    h.advance(300);
    h.frame();
    assert!(h.wait_until(|app| app.shown_text().is_some()));
    h.frame();
    h.key(Key::A, Modifiers::CTRL);
    h.key(Key::C, Modifiers::CTRL);
    assert_eq!(
        h.clipboard().as_deref(),
        Some("word"),
        "the text is selected"
    );
    h.key(Key::Escape, Modifiers::NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn quit_does_not_wait_for_stalled_worker() {
    let (_dir, paths) = folder("quit", &["hang.png"], 16, 16);
    let cfg = TempDir::new("quitcfg");
    let disk = TestDisk::new();
    disk.stall_read("hang.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            settings_path: Some(cfg.join("settings.ini")),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.disk.wait_read_started("hang.png"));
    h.key(Key::Q, Modifiers::CTRL);
    assert!(h.app().exit_requested());
    let app = h.app.take().unwrap();
    let t = Instant::now();
    let report = {
        let _guard = Watchdog::arm("shutdown with a stuck read", QUICK);
        app.shutdown(Duration::from_secs(5))
    };
    assert_eq!(report.settings_saved, Some(true), "settings still saved");
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
    assert_eq!(
        h.disk.returned("hang.png"),
        0,
        "the stuck read was not joined"
    );

    // A settings folder that hangs too: shutdown gives up after its bound.
    let disk = TestDisk::new();
    disk.stall_read("hang.png");
    disk.stall_write("settings.ini");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            settings_path: Some(cfg.join("settings.ini")),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.disk.wait_read_started("hang.png"));
    h.act(Action::Quit);
    let app = h.app.take().unwrap();
    let t = Instant::now();
    let report = {
        let _guard = Watchdog::arm("shutdown with a stuck write", QUICK);
        app.shutdown(Duration::from_millis(200))
    };
    assert_eq!(report.settings_saved, None);
    assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
}

#[test]
fn smoke_stall_exits_on_escape() {
    // The real binary: a window, a read that never finishes, Escape on the
    // loading screen, and a clean exit.
    let exe = env!("CARGO_BIN_EXE_pandite");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/smoke.png");
    let mut child = std::process::Command::new(exe)
        .arg("--smoke")
        .arg(&fixture)
        .arg("--smoke-stall")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("start pandite");
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let read = |mut r: Box<dyn std::io::Read>| {
            let mut s = String::new();
            let _ = r.read_to_string(&mut s);
            s
        };
        let out = read(Box::new(stdout));
        let err = read(Box::new(stderr));
        let status = child.wait();
        let _ = tx.send((status, out, err));
    });
    let (status, out, err) = rx
        .recv_timeout(Duration::from_secs(60))
        .expect("pandite --smoke-stall exits on its own");
    let code = status.unwrap().code();
    if code == Some(3) && std::env::var_os("PANDITE_REQUIRE_GPU").is_none() {
        eprintln!("skipped: no GPU or window system ({err})");
        return;
    }
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains("SMOKE OK"), "{out}");
}

#[test]
fn no_blocking_fs_on_event_loop_thread() {
    // Runtime: every file system call of a full session is recorded with
    // its thread; none may come from this thread, which plays the event
    // loop.
    let (dir, paths) = folder("nofs", &["a.png", "b.png", "c.png"], 24, 16);
    let sub = dir.join("folder");
    std::fs::create_dir_all(&sub).unwrap();
    write_png(&sub.join("inner.png"), &numbered(5, 5));
    let cfg = TempDir::new("nofscfg");
    let event_loop = std::thread::current().id();
    // The first image's whole-file read waits until the size probe (a head
    // read on its own thread, made because the window sizes itself to the
    // first image) has returned: the probe is then part of every run,
    // whatever the scheduling. Without the latch a fast decode could show
    // the image first, and the core rightly skips a probe it no longer
    // needs.
    let disk = TestDisk::new();
    disk.stall_full_read("a.png");
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[0].clone()),
            settings_path: Some(cfg.join("settings.ini")),
            size_to_image: true,
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(
        h.disk.wait_returned("read_head", "a.png"),
        "the size probe read the file head"
    );
    h.disk.release_full_read("a.png");
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    assert!(h.wait_thumbnails(3), "gallery thumbnails were uploaded");
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[1]));
    h.act(Action::RotateRight);
    assert!(h.wait_idle());
    let saved = dir.join("saved.png");
    h.answer_dialog(vec![saved.clone()]);
    h.act(Action::SaveAs);
    assert!(h.wait_until(|a| !a.is_busy() && a.shown_path() == Some(saved.as_path())));
    h.answer_dialog(vec![paths[2].clone()]);
    h.act(Action::Open);
    assert!(h.wait_shown(&paths[2]));
    h.act(Action::CopyPath);
    h.act(Action::ToggleGallery);
    h.act(Action::RunBulk(BulkJob {
        source: BulkSource::Folder {
            path: dir.path().to_path_buf(),
            recursive: true,
        },
        resize: None,
        format: Some("bmp".into()),
        quality: 90,
        destination: dir.join("out"),
        overwrite: OverwritePolicy::Rename,
    }));
    assert!(h.wait_until(|a| !a.bulk_running()));
    // A dropped folder opens its first image.
    let now = h.now;
    h.app().open(PathBuf::from(&sub), now);
    let inner = sub.join("inner.png");
    assert!(h.wait_shown(&inner));
    for _ in 0..5 {
        h.advance(16);
        h.frame();
        h.pump();
    }
    let app = h.app.take().unwrap();
    assert_eq!(
        app.shutdown(Duration::from_secs(10)).settings_saved,
        Some(true)
    );

    let calls = h.disk.calls();
    let ops: std::collections::HashSet<_> = calls.iter().map(|c| c.op).collect();
    for op in [
        "read",
        "read_head",
        "list_dir",
        "file_info",
        "write_atomic",
        "create_dir_all",
    ] {
        assert!(ops.contains(op), "the session exercised {op}: {ops:?}");
    }
    let on_loop: Vec<_> = calls.iter().filter(|c| c.thread == event_loop).collect();
    assert!(
        on_loop.is_empty(),
        "file system calls on the event-loop thread: {on_loop:?}"
    );

    // Static: the event-loop code never calls std::fs or path queries that
    // touch the disk. Only the disk module and code that runs on workers
    // may.
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let loop_files = [
        "app.rs",
        "app/thumbs.rs",
        "platform.rs",
        "edit.rs",
        "shell.rs",
        "shell/input.rs",
        "shell/native.rs",
    ];
    let forbidden = [
        "std::fs",
        "fs::",
        "File::",
        "OpenOptions",
        ".metadata(",
        "read_dir(",
        ".exists()",
        ".is_file()",
        ".is_dir()",
        "canonicalize(",
        "symlink_metadata(",
        "read_link(",
        "try_exists(",
    ];
    for file in loop_files {
        let text = std::fs::read_to_string(src.join(file)).unwrap();
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for pattern in forbidden {
                assert!(
                    !code.contains(pattern),
                    "{file}:{}: `{pattern}` on the event-loop side: {line}",
                    n + 1
                );
            }
        }
    }
}

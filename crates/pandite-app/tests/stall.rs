//! The app never freezes on a stalled file: the window and loading screen
//! come first, the event loop keeps turning while a read hangs forever,
//! Escape and quitting never wait for it, and no file system call runs on
//! the event-loop thread.
//!
//! Stalls are latches in the test file system that only teardown opens.
//! Every "returns at once" claim is guarded by a watchdog that aborts the
//! run when the call blocks.

mod support;

use pandite_ui::{Action, BulkJob, BulkSource, InputEvent, Key, Modifiers, OverwritePolicy};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use support::*;

const SPLASH_HINT: &str = "Press Esc to cancel";

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
    assert_eq!(h.app().title(), "slow.png - pandite");

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
fn escape_while_loading_later_image_keeps_previous() {
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
    assert!(
        has_text(&h.frame(), "b.png"),
        "the small loading overlay names b.png"
    );

    {
        let _guard = Watchdog::arm("Escape", QUICK);
        h.key(Key::Escape, Modifiers::NONE);
    }
    assert!(!h.app().exit_requested());
    assert_eq!(h.app().loading_path(), None);
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
    assert_eq!(h.app().target_path(), Some(paths[0].as_path()));
    assert_eq!(h.app().index(), Some(0));
    assert_eq!(h.app().library().current(), Some(0));
    assert_eq!(h.app().title(), "a.png - pandite");
    assert!(!has_text(&h.frame(), SPLASH_HINT));
    // The library still serves other images while that read hangs.
    h.act(Action::GoTo(2));
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.disk.returned("b.png"), 0);
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
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        settings_path: Some(cfg.join("settings.ini")),
        size_to_image: true,
        ..Config::default()
    });
    h.start();
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

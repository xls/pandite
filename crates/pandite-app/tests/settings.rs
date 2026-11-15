//! Settings persist: window placement, off-screen recovery, corrupt files,
//! the first-open screen fraction, the per-user folder and atomic writes.

mod support;

use pandite_app::app::WindowState;
use pandite_app::disk::{self, StdDisk};
use pandite_app::settings::{
    self, ensure_on_screen, first_open_rect, GallerySize, LoadNote, MonitorArea, Settings,
    ThemeChoice, WindowPlacement,
};
use pandite_core::{Rect, Size};
use pandite_ui::{Key, Modifiers};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use support::*;

fn monitor(x: f32, y: f32, w: f32, h: f32) -> MonitorArea {
    MonitorArea {
        bounds: Rect::new(x, y, w, h),
        work: Rect::new(x, y, w, h - 40.0),
    }
}

#[test]
fn settings_roundtrip_window_placement() {
    let dir = TempDir::new("placement");
    let path = dir.join("settings.ini");
    let saved = WindowPlacement {
        x: 100,
        y: 80,
        width: 1200,
        height: 700,
        maximized: false,
        fullscreen: false,
    };
    let start = Settings {
        window: Some(saved),
        gallery_visible: false,
        theme: ThemeChoice::Light,
        ..Settings::default()
    };
    // The app starts from the saved placement, the user moves and resizes
    // the window, maximizes it and leaves.
    let mut h = Harness::new(Config {
        settings: start.clone(),
        settings_path: Some(path.clone()),
        window: WindowState {
            size: Size::new(1200.0, 700.0),
            placement: Some(saved.rect()),
            ..WindowState::default()
        },
        ..Config::default()
    });
    h.app().moved(150, 90, false);
    h.app().resized(1000, 600, false);
    h.app().resized(1920, 1040, true);
    h.app().moved(-8, -8, true);
    let report = h.app.take().unwrap().shutdown(Duration::from_secs(10));
    assert_eq!(report.settings_saved, Some(true));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("window_x = 150"), "{text}");
    let (loaded, note) = settings::load(&StdDisk, &path);
    assert_eq!(note, LoadNote::Loaded);
    assert_eq!(
        loaded.window,
        Some(WindowPlacement {
            x: 150,
            y: 90,
            width: 1000,
            height: 600,
            maximized: true,
            fullscreen: false,
        }),
        "the restored rectangle and the maximized state"
    );
    assert!(!loaded.gallery_visible);
    assert_eq!(loaded.theme, ThemeChoice::Light);

    // Full screen is remembered too, with the window rectangle behind it.
    let mut h = Harness::new(Config {
        settings: loaded.clone(),
        settings_path: Some(path.clone()),
        window: WindowState {
            size: Size::new(1000.0, 600.0),
            placement: Some(Rect::new(150.0, 90.0, 1000.0, 600.0)),
            ..WindowState::default()
        },
        ..Config::default()
    });
    h.key(Key::F11, Modifiers::NONE);
    h.app().resized(2560, 1440, false);
    let report = h.app.take().unwrap().shutdown(Duration::from_secs(10));
    assert_eq!(report.settings_saved, Some(true));
    let (again, _) = settings::load(&StdDisk, &path);
    let w = again.window.unwrap();
    assert!(w.fullscreen);
    assert_eq!((w.x, w.y, w.width, w.height), (150, 90, 1000, 600));
}

#[test]
fn offscreen_window_is_moved_back() {
    let primary = monitor(0.0, 0.0, 1920.0, 1080.0);
    let left = monitor(-1280.0, 0.0, 1280.0, 1024.0);
    let place = |x, y, width, height| WindowPlacement {
        x,
        y,
        width,
        height,
        maximized: false,
        fullscreen: false,
    };

    // Saved on a monitor to the right that is gone: moved onto the primary
    // monitor's work area, centered, same size.
    let gone = place(2500, 300, 800, 600);
    let fixed = ensure_on_screen(gone, &[primary, left]);
    let work = primary.work;
    let r = fixed.rect();
    assert!(
        work.intersect(r) == Some(r),
        "inside the work area: {fixed:?}"
    );
    assert_eq!((fixed.width, fixed.height), (800, 600));
    assert_eq!(r.center().x, work.center().x);

    // On a monitor that still exists (negative coordinates): unchanged.
    let on_left = place(-1000, 100, 800, 600);
    assert_eq!(ensure_on_screen(on_left, &[primary, left]), on_left);
    // Its monitor unplugged: back on the primary one.
    let moved = ensure_on_screen(on_left, &[primary]);
    assert!(moved.x >= 0 && moved.y >= 0, "{moved:?}");

    // Hanging off the bottom with the title bar out of reach: moved.
    let below = place(100, 1070, 800, 600);
    let moved = ensure_on_screen(below, &[primary]);
    assert!(moved.y + 32 <= work.bottom() as i32, "{moved:?}");

    // Partly off screen but the title bar is reachable: kept.
    let partly = place(1500, 200, 800, 600);
    assert_eq!(ensure_on_screen(partly, &[primary]), partly);

    // Larger than the monitor it lands on: shrunk to fit.
    let huge = place(5000, 5000, 4000, 3000);
    let moved = ensure_on_screen(huge, &[primary]);
    assert!(moved.width as f32 <= work.w && moved.height as f32 <= work.h);

    // No monitor information: keep what was saved.
    assert_eq!(ensure_on_screen(gone, &[]), gone);
}

#[test]
fn corrupt_settings_fall_back_to_defaults() {
    let dir = TempDir::new("corrupt");
    let path = dir.join("settings.ini");
    let bad_path = dir.join("settings.ini.bad");
    for garbage in [
        &b"\xff\xfe\x00binary garbage\x00"[..],
        b"theme = purple\n",
        b"window_x = 10\n",
        b"this is not a settings file",
    ] {
        std::fs::write(&path, garbage).unwrap();
        let _ = std::fs::remove_file(&bad_path);
        let (loaded, note) = settings::load(&StdDisk, &path);
        assert_eq!(loaded, Settings::default());
        match note {
            LoadNote::Corrupt { kept, .. } => assert_eq!(kept.as_deref(), Some(bad_path.as_path())),
            other => panic!("expected corrupt, got {other:?}"),
        }
        // Kept aside for inspection; the next save starts fresh.
        assert_eq!(std::fs::read(&bad_path).unwrap(), garbage);
        assert!(!path.exists());
    }
    // A missing file is the first run, not an error.
    let (loaded, note) = settings::load(&StdDisk, &dir.join("missing.ini"));
    assert_eq!((loaded, note), (Settings::default(), LoadNote::Missing));

    // The app runs with the defaults and saves a valid file over it.
    std::fs::write(&path, b"\x00\x01garbage").unwrap();
    let (loaded, _) = settings::load(&StdDisk, &path);
    let h = Harness::new(Config {
        settings: loaded,
        settings_path: Some(path.clone()),
        ..Config::default()
    });
    let mut h = h;
    let report = h.app.take().unwrap().shutdown(Duration::from_secs(10));
    assert_eq!(report.settings_saved, Some(true));
    assert_eq!(settings::load(&StdDisk, &path).1, LoadNote::Loaded);
}

#[test]
fn gallery_size_roundtrips_and_survives_a_bad_value() {
    let dir = TempDir::new("gallery-size");
    let path = dir.join("settings.ini");
    let saved = Settings {
        gallery_size: GallerySize::Small,
        theme: ThemeChoice::Light,
        ..Settings::default()
    };
    settings::save(&StdDisk, &path, &saved).unwrap();
    let (loaded, note) = settings::load(&StdDisk, &path);
    assert_eq!((loaded.clone(), note), (saved.clone(), LoadNote::Loaded));
    // A size this version does not know: the default size, the other
    // settings load and the file stays where it is.
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("gallery_size = small"), "{text}");
    std::fs::write(
        &path,
        text.replace("gallery_size = small", "gallery_size = huge"),
    )
    .unwrap();
    let (loaded, note) = settings::load(&StdDisk, &path);
    assert_eq!(note, LoadNote::Loaded);
    assert_eq!(
        loaded,
        Settings {
            gallery_size: GallerySize::Medium,
            ..saved
        }
    );
    assert!(path.exists() && !dir.join("settings.ini.bad").exists());
}

#[test]
fn screen_fraction_is_configurable() {
    let parsed = Settings::parse("first_open_fraction = 0.5\n").unwrap();
    assert_eq!(parsed.first_open_fraction, 0.5);
    assert!(Settings::parse("first_open_fraction = 0.05").is_err());
    assert_eq!(Settings::default().first_open_fraction, 0.8);

    let (_dir, paths) = folder("fraction", &["big.png"], 4000, 3000);
    let work = Rect::new(0.0, 0.0, 1920.0, 1040.0);
    let mut sizes = Vec::new();
    for fraction in [0.5f32, 0.8] {
        let settings = Settings {
            first_open_fraction: fraction,
            ..Settings::default()
        };
        let mut h = Harness::new(Config {
            open: Some(paths[0].clone()),
            settings,
            size_to_image: true,
            work_area: Some(work),
            ..Config::default()
        });
        h.start();
        assert!(h.wait_shown(&paths[0]));
        let rects = lock(&h.window).client_rects.clone();
        assert_eq!(rects.len(), 1, "sized once, when the image size was known");
        let r = rects[0];
        // The image area follows initial_window_rect with this fraction;
        // the gallery strip is added below it.
        let strip = r.h - (r.w * 3000.0 / 4000.0).round();
        let expected = first_open_rect(
            work,
            Some((4000, 3000)),
            fraction,
            strip,
            Size::new(480.0, 360.0),
        );
        assert_eq!(r, expected);
        assert!(r.h - strip <= work.h * fraction + 1.0);
        assert!(
            (r.center().x - work.center().x).abs() <= 1.0,
            "centered: {r:?}"
        );
        sizes.push(r.w);
    }
    assert!(
        sizes[0] < sizes[1],
        "a larger fraction gives a larger window"
    );

    // A saved placement is kept unless the setting asks to size every time.
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        size_to_image: false,
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(lock(&h.window).client_rects.is_empty());
}

#[test]
fn settings_write_is_atomic() {
    // The per-user folder: %APPDATA%\pandite on Windows,
    // ~/Library/Application Support/pandite on macOS, the XDG folder
    // elsewhere.
    if std::env::var_os("PANDITE_CONFIG_DIR").is_none() {
        let dir = settings::config_dir().expect("a config folder");
        assert!(dir.ends_with("pandite"));
        if cfg!(windows) {
            let appdata = std::path::PathBuf::from(std::env::var_os("APPDATA").unwrap());
            assert_eq!(dir, appdata.join("pandite"));
        }
        assert_eq!(
            settings::settings_path(None).unwrap(),
            dir.join(settings::FILE_NAME)
        );
    }

    let dir = TempDir::new("atomic");
    let path = dir.join("settings.ini");
    let old = Settings::default();
    settings::save(&StdDisk, &path, &old).unwrap();
    let old_text = std::fs::read(&path).unwrap();

    // A write that fails half way leaves the old file whole and no
    // temporary file behind.
    let err = disk::write_atomic_with(&path, |f| {
        f.write_all(b"theme = li")?;
        Err(std::io::Error::other("disk full"))
    })
    .unwrap_err();
    assert_eq!(err.to_string(), "disk full");
    assert_eq!(std::fs::read(&path).unwrap(), old_text);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    // Readers never see a partial file while it is rewritten many times.
    let a = Settings {
        theme: ThemeChoice::Dark,
        jpeg_quality: 11,
        ..Settings::default()
    };
    let b = Settings {
        theme: ThemeChoice::Light,
        jpeg_quality: 99,
        ..Settings::default()
    };
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let path = path.clone();
        let stop = Arc::clone(&stop);
        let (ta, tb) = (a.to_text(), b.to_text());
        let old = String::from_utf8(old_text.clone()).unwrap();
        std::thread::spawn(move || {
            let mut seen = 0;
            while !stop.load(Ordering::Acquire) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    assert!(
                        text == ta || text == tb || text == old,
                        "a reader saw a partial file: {text:?}"
                    );
                    seen += 1;
                }
            }
            seen
        })
    };
    for i in 0..200 {
        let s = if i % 2 == 0 { &a } else { &b };
        settings::save(&StdDisk, &path, s).unwrap();
    }
    stop.store(true, Ordering::Release);
    assert!(reader.join().unwrap() > 0);
    assert_eq!(settings::load(&StdDisk, &path).0, b);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "no leftovers"
    );

    // The app saves through the atomic writer, on a worker thread.
    let test_thread = std::thread::current().id();
    let mut h = Harness::new(Config {
        settings_path: Some(path.clone()),
        ..Config::default()
    });
    h.act(pandite_ui::Action::ToggleGallery);
    let report = h.app.take().unwrap().shutdown(Duration::from_secs(10));
    assert_eq!(report.settings_saved, Some(true));
    let writes: Vec<_> = h
        .disk
        .calls()
        .into_iter()
        .filter(|c| c.op == "write_atomic")
        .collect();
    assert!(!writes.is_empty());
    assert!(writes
        .iter()
        .all(|c| c.name == "settings.ini" && c.thread != test_thread));
    assert!(!settings::load(&StdDisk, &path).0.gallery_visible);
}

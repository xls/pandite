//! Staged display: a reduced image first, full resolution after, without
//! moving the view; navigation cancels the pending full decode; edits and
//! saves wait for full resolution; the timing report.
//!
//! The staged files are decoded by a test decoder whose reduced and full
//! decodes wait on latches, so every step is ordered by the test, never by
//! timing.

mod support;

use pandite_core::{Point, Rect, Size};
use pandite_ui::{Action, Key, Modifiers};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;
use support::*;

/// A harness on `open` with the gated decoder.
fn staged_harness(open: &Path, dec: &std::sync::Arc<GatedDecoder>) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(open.to_path_buf()),
        registry: Some(dec.registry()),
        ..Config::default()
    });
    h.start();
    h
}

/// The content updates that showed something, as (full resolution, dims).
fn shown(updates: &[Update]) -> Vec<(bool, (u32, u32))> {
    updates
        .iter()
        .filter_map(|u| u.dims.map(|d| (u.full_resolution, d)))
        .collect()
}

#[test]
fn staged_display_shows_reduced_then_full() {
    let dir = TempDir::new("staged-order");
    let a = dir.join("a.stg");
    write_staged(&a, 2000, 1500);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let mut h = staged_harness(&a, &dec);
    // The reduced stage arrives while the full decode is held.
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    let first = h.updates.iter().find(|u| u.dims.is_some()).unwrap().clone();
    assert!(
        !first.full_resolution,
        "the first image is the reduced stage"
    );
    assert_eq!(first.logical, Some((2000, 1500)), "drawn at full size");
    let (rw, rh) = first.dims.unwrap();
    assert!(rw < 2000 && rh < 1500, "reduced: {rw}x{rh}");
    // Half the 1000 x 800 window: the quick stage covers 500 x 400.
    assert!(rw >= 500 || rh >= 400, "covers the quick target: {rw}x{rh}");
    assert_eq!(h.app().display_dims(), Some((2000, 1500)));
    assert!(h.app().loading_path().is_some(), "full resolution pending");
    assert!(
        dec.wait_started("a.stg", true),
        "the full decode runs beside it"
    );

    dec.release_full("a.stg");
    assert!(h.wait_frames(|app, ups| {
        !app.shows_reduced() && ups.last().is_some_and(|u| u.full_resolution)
    }));
    let last = h.updates.last().unwrap();
    assert_eq!(last.dims, Some((2000, 1500)));
    assert_eq!(last.logical, None);
    assert_eq!(h.app().shown_image().unwrap().width(), 2000);
    assert!(h.app().loading_path().is_none());
    // Exactly one reduced image, then the full one.
    assert_eq!(
        shown(&h.updates),
        vec![(false, (rw, rh)), (true, (2000, 1500))]
    );
    dec.release_all();
}

#[test]
fn full_resolution_replaces_reduced_without_view_jump() {
    let dir = TempDir::new("staged-view");
    let a = dir.join("a.stg");
    write_staged(&a, 2000, 1500);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let mut h = staged_harness(&a, &dec);
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    // The user zooms and pans the stand-in.
    h.act(Action::ZoomAt {
        factor: 3.0,
        anchor: Point::new(300.0, 200.0),
    });
    h.act(Action::PanBy {
        dx: -40.0,
        dy: 25.0,
    });
    h.frame();
    let view = h.app().view();
    let dims = h.app().display_dims().unwrap();
    assert_eq!(dims, (2000, 1500));
    // Where the image lands for any viewport: a function of the view and
    // the full image size only, both of which the swap keeps.
    let viewport = Rect::new(0.0, 0.0, 1000.0, 740.0);
    let size = Size::new(dims.0 as f32, dims.1 as f32);
    let before = view.image_rect(size, viewport);

    dec.release_full("a.stg");
    assert!(h.wait_frames(|app, _| !app.shows_reduced()));
    assert_eq!(h.app().view(), view, "the swap moved or rescaled the view");
    assert_eq!(h.app().display_dims(), Some(dims));
    let after = h.app().view().image_rect(size, viewport);
    assert_eq!(before, after);
    // The renderer receives the reduced image at the logical size of the
    // full image, so both cover the same screen rectangle.
    let staged: Vec<_> = h.updates.iter().filter(|u| u.dims.is_some()).collect();
    assert_eq!(staged[0].logical, staged.last().unwrap().dims);
    dec.release_all();
}

#[test]
fn zoom_before_full_arrives_ends_sharp() {
    let dir = TempDir::new("staged-zoom");
    let a = dir.join("a.stg");
    write_staged(&a, 2000, 1500);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let mut h = staged_harness(&a, &dec);
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    // Zoom far in before the full image exists: 4 screen pixels per image
    // pixel, which the stand-in can only show upscaled.
    h.act(Action::OneToOne { anchor: None });
    h.act(Action::ZoomAt {
        factor: 4.0,
        anchor: Point::new(500.0, 370.0),
    });
    h.frame();
    let view = h.app().view();
    assert!(
        matches!(view.mode, pandite_core::DisplayMode::Zoom(z) if (z - 4.0).abs() < 1e-3),
        "{view:?}"
    );
    assert!(h.updates.iter().all(|u| !u.full_resolution));

    dec.release_full("a.stg");
    assert!(h.wait_frames(|app, _| !app.shows_reduced()));
    // It ends sharp: the last content is the full image, one texel per
    // image pixel, at the zoom the user chose.
    let last = h.updates.last().unwrap().clone();
    assert!(last.full_resolution);
    assert_eq!(last.dims, Some((2000, 1500)));
    assert_eq!(h.app().view(), view);
    // A reduced result that arrives after the full image never replaces it.
    let b = dir.join("b.stg");
    write_staged(&b, 1800, 1200);
    dec.hold_reduced("b.stg");
    h.answer_dialog(vec![b.clone()]);
    h.act(Action::Open);
    // The dialog's answer arrives as a message: take it.
    h.pump();
    assert!(dec.wait_started("b.stg", false));
    assert!(h.wait_frames(|app, ups| {
        app.shown_path() == Some(b.as_path())
            && ups.last().is_some_and(|u| u.dims == Some((1800, 1200)))
    }));
    let count = h.updates.len();
    dec.release_reduced("b.stg");
    // The late reduced decode finishes; give its delivery time to reach
    // the core (an absence is checked, so the wait cannot make it pass).
    assert!(dec.wait_finished("b.stg", false));
    for _ in 0..5 {
        std::thread::sleep(Duration::from_millis(10));
        h.pump();
        h.frame();
    }
    assert!(
        h.updates[count..].iter().all(|u| u.full_resolution),
        "a late reduced image replaced the full one: {:?}",
        &h.updates[count..]
    );
    assert!(!h.app().shows_reduced());
    dec.release_all();
}

#[test]
fn navigation_cancels_pending_full_decode() {
    let dir = TempDir::new("staged-nav");
    let a = dir.join("a.stg");
    let b = dir.join("b.stg");
    write_staged(&a, 2000, 1500);
    write_staged(&b, 1800, 1200);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let mut h = staged_harness(&a, &dec);
    assert!(h.wait_frames(|app, _| app.shows_reduced() && app.count() == 2));
    assert!(dec.wait_started("a.stg", true));
    // Move on while a's full decode is still running.
    h.key(Key::Right, Modifiers::NONE);
    assert_eq!(h.app().target_path(), Some(b.as_path()));
    assert!(
        dec.wait_cancelled("a.stg"),
        "the pending full decode stopped"
    );
    dec.release_all();
    assert!(h.wait_frames(|app, _| {
        app.shown_path() == Some(b.as_path()) && !app.shows_reduced() && !app.is_busy()
    }));
    assert_eq!(h.app().display_dims(), Some((1800, 1200)));
    // a's full image was never decoded nor shown after the move.
    assert_eq!(dec.full_finished("a.stg"), 0);
    assert!(!shown(&h.updates).contains(&(true, (2000, 1500))));
    assert_eq!(shown(&h.updates).last(), Some(&(true, (1800, 1200))));
}

#[test]
fn edits_and_save_use_full_resolution() {
    let dir = TempDir::new("staged-edit");
    let a = dir.join("a.stg");
    write_staged(&a, 1200, 800);
    let dec = GatedDecoder::new();
    dec.hold_full("a.stg");
    let mut h = staged_harness(&a, &dec);
    assert!(h.wait_frames(|app, _| app.shows_reduced()));
    // Edit and save while only the stand-in exists: both wait.
    h.act(Action::RotateRight);
    let out = dir.join("out.png");
    h.answer_dialog(vec![out.clone()]);
    h.act(Action::SaveAs);
    h.pump();
    assert!(h.app().is_busy(), "the busy indicator shows the wait");
    assert!(has_text(&h.frame(), "Loading full resolution"));
    assert!(
        !h.disk.calls().iter().any(|c| c.op == "write_atomic"),
        "nothing is saved from the stand-in"
    );
    assert!(h.updates.iter().all(|u| !u.full_resolution));

    dec.release_full("a.stg");
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(out.as_path())));
    // The saved file is the rotated full-resolution image, pixel for pixel.
    let (saved, format) = decode(&out);
    assert_eq!(format, "png");
    assert_eq!((saved.width(), saved.height()), (800, 1200));
    let expected = pandite_ops::rotate(&staged_pattern(1200, 800), pandite_ops::Rotation::Cw90);
    assert_eq!(pixels(&saved), pixels(&expected));
    // The view shows the edited full image.
    assert_eq!(h.app().display_dims(), Some((800, 1200)));
    dec.release_all();
}

/// Parse a `TIMING` line into its named figures.
fn timing_fields(line: &str) -> Vec<(String, f64)> {
    line.split_whitespace()
        .skip(1)
        .filter_map(|f| {
            let (k, v) = f.split_once('=')?;
            Some((k.to_string(), v.parse().ok()?))
        })
        .collect()
}

#[test]
fn timing_line_is_printed() {
    // The real binary on a JPEG large enough for a reduced first stage:
    // twice the size of a 4K screen, so a scaled decode covers the quick
    // target (half the window) on any common display.
    let dir = TempDir::new("timing");
    let file: PathBuf = dir.join("large.jpg");
    let img = image(7680, 4320, |x, y| {
        [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255]
    });
    let reg = pandite_codecs::default_registry();
    let mut bytes = Vec::new();
    reg.encoder_by_id("jpeg")
        .unwrap()
        .encode(&img, &pandite_core::EncodeOptions::default(), &mut bytes)
        .unwrap();
    std::fs::write(&file, &bytes).unwrap();
    let exe = env!("CARGO_BIN_EXE_pandite");
    let mut child = std::process::Command::new(exe)
        .arg("--smoke")
        .arg(&file)
        .arg("--timing")
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
        .recv_timeout(Duration::from_secs(120))
        .expect("pandite --smoke --timing exits on its own");
    let code = status.unwrap().code();
    if code == Some(3) && std::env::var_os("PANDITE_REQUIRE_GPU").is_none() {
        eprintln!("skipped: no GPU or window system ({err})");
        return;
    }
    assert_eq!(code, Some(0), "stdout: {out}\nstderr: {err}");
    assert!(out.contains("SMOKE OK"), "{out}");
    let line = out
        .lines()
        .find(|l| l.starts_with("TIMING "))
        .unwrap_or_else(|| panic!("no TIMING line in {out}"));
    let fields = timing_fields(line);
    let names: Vec<&str> = fields.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        names[..7],
        [
            "first_pixel_ms",
            "full_res_ms",
            "read_ms",
            "decode_reduced_ms",
            "decode_full_ms",
            "upload_ms",
            "mips_ms"
        ],
        "{line}"
    );
    let get = |k: &str| fields.iter().find(|(n, _)| n == k).unwrap().1;
    assert!(get("first_pixel_ms") > 0.0, "{line}");
    assert!(get("first_pixel_ms") <= get("full_res_ms"), "{line}");
    assert!(
        get("read_ms") > 0.0 && get("decode_full_ms") > 0.0,
        "{line}"
    );
    assert!(get("upload_ms") > 0.0, "{line}");
    // The JPEG is larger than half the window: a reduced first stage ran.
    assert!(get("decode_reduced_ms") > 0.0, "{line}");
    // The full stage cannot be faster than its own decode.
    assert!(get("full_res_ms") >= get("decode_full_ms"), "{line}");
    println!("{line}");
}

//! Staged loading: with a screen size set, the current image shows a quick
//! reduced stage before its full resolution, read-ahead neighbors are
//! decoded at screen size only, a pending full decode is cancelled when
//! the user moves on, and nothing else starts while the current image
//! loads.

mod common;

use common::*;
use pandite_library::{Event, Fetch};
use std::sync::Arc;
use std::time::Duration;

const SCREEN: (u32, u32) = (400, 300);
const QUICK: (u32, u32) = (200, 150);

#[test]
fn staged_current_shows_reduced_then_full() {
    let mut h = Harness::new("staged-order", |c| {
        c.screen_size = Some(SCREEN);
        c.workers = 2;
    });
    h.dec.enable_reduced();
    let paths = h.dir.images(3, 2000, 1500);
    let held = Call::full(&img(0));
    h.dec.hold(held.clone());
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(0);
    // The quick stage arrives while the full decode is held.
    assert!(h.wait_for(|e| matches!(e, Event::Reduced { index: 0, .. })));
    let reduced = lib.reduced(0).expect("a screen image");
    assert_eq!((reduced.width, reduced.height), (2000, 1500));
    assert_eq!(
        (reduced.image.width(), reduced.image.height()),
        QUICK,
        "half the screen, covered"
    );
    assert!(matches!(lib.get(0), Fetch::Pending));
    assert!(h.dec.targets().contains(&(img(0), Some(QUICK))));
    h.dec.release(&held);
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 0, .. })));
    assert!(lib.get(0).is_ready());
    let order: Vec<&str> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Reduced { index: 0, .. } => Some("reduced"),
            Event::Ready { index: 0, .. } => Some("ready"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["reduced", "ready"]);
}

#[test]
fn staged_read_ahead_holds_screen_images() {
    let mut h = Harness::new("staged-ahead", |c| {
        c.screen_size = Some(SCREEN);
        c.workers = 2;
        c.read_ahead = 2;
    });
    h.dec.enable_reduced();
    let paths = h.dir.images(9, 2000, 1500);
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(4);
    h.idle();
    h.pump();
    // Full resolution for the current image only.
    assert_eq!(h.dec.full_starts(), imgs(&[4]));
    let targets = h.dec.targets();
    for i in [3, 5, 2, 6] {
        assert!(
            targets.contains(&(img(i), Some(SCREEN))),
            "neighbor {i} at screen size: {targets:?}"
        );
        let reduced = lib.reduced(i).expect("neighbor cached at screen size");
        assert_eq!((reduced.width, reduced.height), (2000, 1500));
        assert!(matches!(lib.get(i), Fetch::Pending));
    }
    let s = lib.stats();
    assert_eq!((s.full_entries, s.full_bytes), (1, 2000 * 1500 * 4));
    assert_eq!(s.screen_entries, 5, "four neighbors and the quick stage");
    assert!(s.screen_bytes <= s.screen_budget, "{s:?}");
    assert_eq!(s.full_budget + s.screen_budget, 512 << 20, "one budget");
    // Moving on: the neighbor's screen image is there at once, its full
    // resolution follows.
    lib.set_current(5);
    assert!(lib.reduced(5).is_some());
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 5, .. })));
    h.idle();
    assert_eq!(h.dec.full_starts(), imgs(&[4, 5]));
    // A screen image already cached needs no quick stage.
    assert!(!h.dec.targets().contains(&(img(5), Some(QUICK))));
}

#[test]
fn staged_navigation_cancels_full_decode() {
    let mut h = Harness::new("staged-cancel", |c| {
        c.screen_size = Some(SCREEN);
        c.workers = 2;
        c.read_ahead = 1;
    });
    h.dec.enable_reduced();
    let paths = h.dir.images(6, 2000, 1500);
    let held = Call::full(&img(0));
    h.dec.hold(held.clone());
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(0);
    assert!(h.dec.wait_started(&held));
    // 1 is the next image, inside the read-ahead window: still the full
    // decode of 0 stops, because only the current image gets one.
    lib.set_current(1);
    assert_eq!(h.dec.wait_end(&held), Some(End::Cancelled));
    h.dec.release(&held);
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 1, .. })));
    h.idle();
    assert!(matches!(lib.get(0), Fetch::Pending), "nothing kept of it");
    assert!(lib.reduced(0).is_some(), "its screen image stays");
}

#[test]
fn current_image_decodes_alone() {
    let mut h = Harness::new("staged-alone", |c| {
        c.screen_size = Some(SCREEN);
        c.workers = 4;
        c.read_ahead = 2;
        c.background_thumbnails = true;
        c.thumb_margin = 4;
    });
    let paths = h.dir.images(8, 64, 48);
    let held = Call::full(&img(3));
    h.dec.hold(held.clone());
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(3);
    assert!(h.dec.wait_started(&held));
    // The current image decodes now. New work arrives (the gallery shows
    // every thumbnail) beside the read-ahead already planned, and three
    // workers are free: none of it starts until the current image is done.
    // (A neighbor may have started in the instant before the worker knew
    // the file was local; it is not held and has finished by now.)
    let before = h.dec.starts();
    lib.set_visible_thumbnails(0..8);
    // An absence: the wait only gives a wrong scheduler time to show it.
    assert!(!lib.wait_idle(Duration::from_millis(200)));
    assert_eq!(
        h.dec.starts(),
        before,
        "nothing started beside the current image"
    );
    assert_eq!(lib.stats().running, 1, "only the current image runs");
    h.dec.release(&held);
    h.idle();
    h.pump();
    assert!(lib.get(3).is_ready());
    assert!(h.dec.starts().len() > before.len(), "the rest followed");
    for i in 0..8 {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }
}

/// One thread is kept for the selected image: it decodes at once even when
/// every pool worker is inside background work that cannot be interrupted,
/// and when that thread is itself still busy with an earlier selection, a
/// new one takes its place.
#[test]
fn selected_image_never_waits_for_background_work() {
    let mut h = Harness::new("reserved-thread", |c| c.workers = 2);
    h.dec.ignore_cancel();
    let paths = h.dir.images(8, 64, 48);
    // Everything blocks except the full decodes of images 0, 5 and 7.
    for i in 0..8 {
        h.dec.hold(Call::thumb(&img(i)));
        if ![0, 5, 7].contains(&i) {
            h.dec.hold(Call::full(&img(i)));
        }
    }
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(0);
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 0, .. })));
    // Both pool workers get stuck in held background decodes.
    let stuck = || h.dec.starts().len() - h.dec.count(&Call::full(&img(0)));
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while stuck() < 2 {
        assert!(std::time::Instant::now() < deadline, "pool never filled");
        std::thread::sleep(Duration::from_millis(2));
    }
    lib.set_current(5);
    assert!(
        h.wait_for(|e| matches!(e, Event::Ready { index: 5, .. })),
        "the selected image waited for background work"
    );
    // The reserved thread itself gets stuck on image 6; image 7 still loads.
    lib.set_current(6);
    assert!(h.dec.wait_started(&Call::full(&img(6))));
    lib.set_current(7);
    assert!(
        h.wait_for(|e| matches!(e, Event::Ready { index: 7, .. })),
        "no replacement for the busy reserved thread"
    );
    h.dec.release_all();
}

//! Read-ahead, priority, budgets, cancellation, errors and shutdown.
//!
//! Every ahead-of-time claim is measured by the fake decoder's call log
//! (counts and order). Workers are blocked with latches where order
//! matters; nothing waits on a sleep.

mod common;

use common::{
    img, imgs, run_bounded, Call, End, FakeDecoder, FakeFiles, Harness, TempDir, QUICK, WAIT,
};
use pandite_core::Error;
use pandite_library::{Event, Fetch, FileAccess, Library, LibraryConfig};
use std::sync::Arc;

#[test]
fn prefetch_decodes_neighbors_ahead() {
    let h = Harness::new("prefetch", |c| c.read_ahead = 2);
    let paths = h.dir.images(10, 16, 16);
    let lib = h.lib();
    lib.set_items(paths);
    lib.set_current(4);
    h.idle();
    // Only image 4 was requested; its neighbors were decoded on their own,
    // in priority order: current, next, previous, next+1, previous+1.
    assert_eq!(h.dec.full_starts(), imgs(&[4, 5, 3, 6, 2]));
    for i in [2, 3, 5, 6] {
        assert!(lib.get(i).is_ready(), "neighbor {i} decoded ahead");
    }
    assert!(matches!(lib.get(7), Fetch::Pending));
    assert!(matches!(lib.get(1), Fetch::Pending));
    // The current image's thumbnail came from its full decode.
    assert!(h.dec.thumb_starts().is_empty());
    assert!(lib.thumbnail(4).is_ready());
}

#[test]
fn wrap_around_reads_ahead_across_the_ends() {
    let h = Harness::new("wrap", |c| {
        c.read_ahead = 2;
        c.wrap_around = true;
    });
    let paths = h.dir.images(6, 16, 16);
    let lib = h.lib();
    lib.set_items(paths);
    lib.set_current(0);
    h.idle();
    assert_eq!(h.dec.full_starts(), imgs(&[0, 1, 5, 2, 4]));
}

#[test]
fn prefetched_neighbor_is_cache_hit() {
    let h = Harness::new("hit", |c| c.read_ahead = 2);
    let paths = h.dir.images(10, 16, 16);
    let lib = h.lib();
    lib.set_items(paths);
    lib.set_current(4);
    h.idle();
    let before = h.dec.full_starts().len();
    assert_eq!(h.dec.count(&Call::full(&img(5))), 1);
    // Step to the next image: it is ready the moment it becomes current.
    lib.set_current(5);
    assert!(lib.get(5).is_ready(), "next image served from the cache");
    h.idle();
    assert_eq!(h.dec.count(&Call::full(&img(5))), 1, "no second decode");
    // Only the new edge of the window was decoded.
    assert_eq!(h.dec.full_starts()[before..], imgs(&[7])[..]);
}

#[test]
fn thumbnails_generated_ahead_of_view() {
    let mut h = Harness::new("thumbs", |c| {
        c.read_ahead = 1;
        c.background_thumbnails = true;
        c.thumb_margin = 3;
    });
    let paths = h.dir.images(30, 32, 16);
    let lib = Arc::clone(h.lib());
    // Hold the current decode so the gallery range is known before any
    // thumbnail work starts; the order below is then exact.
    h.dec.hold(Call::full(&img(10)));
    lib.set_items(paths);
    lib.set_current(10);
    assert!(h.dec.wait_started(&Call::full(&img(10))));
    lib.set_visible_thumbnails(8..14);
    h.dec.release(&Call::full(&img(10)));
    h.idle();
    // No thumbnail was asked for, yet every one exists and was announced once.
    for i in 0..30 {
        let thumb = lib.thumbnail(i).ready().expect("thumbnail made ahead");
        assert_eq!((thumb.width(), thumb.height()), (8, 4));
        let announced = |e: &Event| matches!(e, Event::ThumbnailReady { index, .. } if *index == i);
        h.pump();
        assert_eq!(h.count_events(announced), 1, "thumbnail {i} announced once");
    }
    // Visible range first (from the current image outward, 9 to 11 reuse
    // their full decodes), then the margin, then the rest by distance.
    let mut expected = vec![12, 8, 13];
    for d in 1..=21 {
        if 13 + d < 30 {
            expected.push(13 + d);
        }
        if d <= 8 {
            expected.push(8 - d);
        }
    }
    assert_eq!(h.dec.thumb_starts(), imgs(&expected));

    // Without background work only the visible range and the margin get
    // thumbnails.
    let h2 = Harness::new("thumbs-margin", |c| {
        c.read_ahead = 1;
        c.thumb_margin = 3;
    });
    let paths = h2.dir.images(30, 32, 16);
    let lib = h2.lib();
    lib.set_items(paths);
    lib.set_visible_thumbnails(8..14);
    h2.idle();
    for i in 0..30 {
        let made = lib.thumbnail(i).is_ready();
        assert_eq!(made, (5..17).contains(&i), "thumbnail {i}");
    }
    assert_eq!(h2.dec.thumb_starts().len(), 12);
}

#[test]
fn current_image_has_priority() {
    let h = Harness::new("priority", |c| {
        c.read_ahead = 2;
        c.background_thumbnails = true;
        c.thumb_margin = 2;
    });
    let paths = h.dir.images(20, 16, 16);
    let lib = h.lib();
    // The only worker gets stuck in a background thumbnail.
    let stuck = Call::thumb(&img(12));
    h.dec.hold(stuck.clone());
    lib.set_items(paths);
    lib.set_current(0);
    assert!(h.dec.wait_started(&stuck));
    let first = h.dec.starts();
    assert_eq!(
        first[..3],
        [
            Call::full(&img(0)),
            Call::full(&img(1)),
            Call::full(&img(2))
        ]
    );
    assert!(
        first[3..].iter().all(|c| c.thumb),
        "thumbnails only after full decodes"
    );
    // Navigate: the busy thumbnail is preempted and the new image goes next.
    lib.set_current(15);
    assert_eq!(h.dec.wait_end(&stuck), Some(End::Cancelled));
    h.dec.release(&stuck);
    h.idle();
    let after = h.dec.starts()[first.len()..].to_vec();
    assert_eq!(
        after[..5],
        [15, 16, 14, 17, 13].map(|i| Call::full(&img(i)))
    );
    assert!(after[5..].iter().all(|c| c.thumb));
    // The preempted thumbnail was requeued, not lost.
    assert!(lib.thumbnail(12).is_ready());
}

#[test]
fn ready_events_wake_the_ui() {
    let mut h = Harness::new("wake", |c| c.read_ahead = 1);
    let paths = h.dir.images(5, 16, 16);
    let lib = Arc::clone(h.lib());
    let held = Call::full(&img(2));
    h.dec.hold(held.clone());
    lib.set_items(paths);
    lib.set_current(2);
    assert!(h.dec.wait_started(&held));
    // The only worker is blocked, so nothing can be queued yet.
    assert!(lib.poll_events().is_empty());
    assert_eq!(h.wakes(), 0);
    h.dec.release(&held);
    // The interface thread sleeps on the wake callback alone.
    assert!(h.wait_wake(), "wake callback fired");
    h.pump();
    assert!(h
        .events
        .iter()
        .any(|e| matches!(e, Event::Ready { index: 2, .. })));
    assert!(lib.get(2).is_ready());
    // Draining re-arms the wake: the neighbors arrive the same way.
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 3, .. })));
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 1, .. })));
    h.idle();
    h.pump();
    // Wakes are coalesced: never more wakes than events.
    assert!(h.wakes() >= 1 && h.wakes() <= h.events.len());
}

#[test]
fn cache_respects_byte_budget() {
    const IMAGE: usize = 16 * 16 * 4;
    let h = Harness::new("budget", |c| {
        c.read_ahead = 3;
        c.full_budget = 3 * IMAGE;
    });
    let mut paths = h.dir.images(12, 16, 16);
    paths[6] = h.dir.image(&img(6), 40, 40);
    let lib = h.lib();
    lib.set_items(paths);
    for i in 0..12 {
        lib.set_current(i);
        h.idle();
        let s = lib.stats();
        assert!(lib.get(i).is_ready(), "current image {i} is cached");
        if i == 6 {
            // The current image stays even when it alone exceeds the budget.
            assert_eq!((s.full_entries, s.full_bytes), (1, 40 * 40 * 4));
        } else {
            assert!(s.full_bytes <= s.full_budget, "step {i}: {s:?}");
            assert!(s.full_entries <= 3);
        }
    }
    // The oversized image was decoded once as read-ahead (then refused)
    // and once as the current image; its known size prevents more.
    assert_eq!(h.dec.count(&Call::full(&img(6))), 2);

    // Thumbnails obey their own budget and stop once it is full.
    const THUMB: usize = 8 * 8 * 4;
    let h2 = Harness::new("budget-thumbs", |c| {
        c.background_thumbnails = true;
        c.thumb_budget = 4 * THUMB;
    });
    let paths = h2.dir.images(20, 16, 16);
    let lib = h2.lib();
    lib.set_items(paths);
    lib.set_visible_thumbnails(9..11);
    h2.idle();
    let s = lib.stats();
    assert_eq!((s.thumb_entries, s.thumb_bytes), (4, 4 * THUMB));
    for i in 0..20 {
        assert_eq!(
            lib.thumbnail(i).is_ready(),
            (8..12).contains(&i),
            "thumbnail {i}"
        );
    }
    assert_eq!(
        h2.dec.thumb_starts().len(),
        4,
        "no decode beyond the budget"
    );
}

#[test]
fn eviction_prefers_far_items() {
    const IMAGE: usize = 16 * 16 * 4;
    let h = Harness::new("evict", |c| {
        c.read_ahead = 0;
        c.full_budget = 3 * IMAGE;
    });
    let paths = h.dir.images(10, 16, 16);
    let lib = h.lib();
    lib.set_items(paths);
    let visit = |i: usize, cached: &[usize]| {
        lib.set_current(i);
        h.idle();
        let held: Vec<usize> = (0..10).filter(|&j| lib.get(j).is_ready()).collect();
        assert_eq!(held, cached, "after visiting {i}");
        assert_eq!(lib.stats().full_bytes, held.len() * IMAGE);
    };
    visit(0, &[0]);
    visit(1, &[0, 1]);
    visit(2, &[0, 1, 2]);
    // 0 is farthest from 5.
    visit(5, &[1, 2, 5]);
    // From 3, both 1 and 5 are two away; the one behind (1) goes first.
    visit(3, &[2, 3, 5]);
    // Revisiting a cached image is a hit and evicts nothing.
    visit(2, &[2, 3, 5]);
    assert_eq!(h.dec.count(&Call::full(&img(2))), 1);
    // 2 is farthest from 9.
    visit(9, &[3, 5, 9]);
}

#[test]
fn stale_requests_are_cancelled() {
    let mut h = Harness::new("stale", |c| c.read_ahead = 1);
    let paths = h.dir.images(20, 16, 16);
    let lib = Arc::clone(h.lib());
    let held = Call::full(&img(0));
    h.dec.hold(held.clone());
    lib.set_items(paths);
    lib.set_current(0);
    assert!(h.dec.wait_started(&held));
    // Jump far away: the decode of 0 left the read-ahead window.
    lib.set_current(10);
    assert_eq!(h.dec.wait_end(&held), Some(End::Cancelled));
    h.idle();
    // Cancelled work is neither cached nor reported as an error.
    assert!(matches!(lib.get(0), Fetch::Pending));
    h.pump();
    assert_eq!(
        h.count_events(|e| matches!(
            e,
            Event::Ready { index: 0, .. } | Event::Failed { index: 0, .. }
        )),
        0
    );
    // The old neighbor never started; the new window was decoded instead.
    assert_eq!(h.dec.full_starts(), imgs(&[0, 10, 11, 9]));
    assert!(lib.get(10).is_ready());
}

#[test]
fn decode_error_is_reported_once() {
    let mut h = Harness::new("error", |c| c.read_ahead = 1);
    let paths = h.dir.images(6, 16, 16);
    h.dec.fail(&img(3));
    let lib = Arc::clone(h.lib());
    lib.set_items(paths.clone());
    for i in [3, 4, 3, 2, 3] {
        lib.set_current(i);
        h.idle();
    }
    h.pump();
    let failed = |e: &Event| matches!(e, Event::Failed { index: 3, .. });
    assert_eq!(h.count_events(failed), 1, "reported once");
    assert_eq!(h.dec.count(&Call::full(&img(3))), 1, "not retried");
    match lib.get(3) {
        Fetch::Failed(err) => assert!(matches!(*err, Error::Corrupt(_)), "{err}"),
        other => panic!("expected a cached error, got {other:?}"),
    }
    let thumb_failed = |e: &Event| matches!(e, Event::ThumbnailFailed { index: 3, .. });
    assert_eq!(h.count_events(thumb_failed), 1);
    assert_eq!(h.dec.count(&Call::thumb(&img(3))), 1);
    // An explicit invalidate is the only retry.
    lib.invalidate(&paths[3]);
    h.idle();
    assert_eq!(h.dec.count(&Call::full(&img(3))), 2);
}

#[test]
fn workers_stop_on_drop() {
    let dir = TempDir::new("drop");
    let paths = dir.images(4, 16, 16);
    let dec = FakeDecoder::new();
    let files = FakeFiles::new();
    let config = LibraryConfig {
        workers: 3,
        read_ahead: 0,
        background_thumbnails: false,
        thumb_margin: 0,
        // Long enough that a slow machine never detaches a healthy worker.
        shutdown_grace: WAIT,
        files: Arc::clone(&files) as Arc<dyn FileAccess>,
        ..LibraryConfig::default()
    };
    let lib = Library::new(dec.registry(), config, Arc::new(|| {})).unwrap();
    assert_eq!(lib.stats().threads, 3);
    let held = Call::full(&img(0));
    dec.hold(held.clone());
    lib.set_items(paths);
    lib.set_current(0);
    assert!(dec.wait_started(&held));
    // Two workers idle, one inside a decode: dropping cancels the decode
    // and joins all three.
    assert!(
        run_bounded(QUICK, move || drop(lib)).is_some(),
        "drop returned"
    );
    assert_eq!(dec.wait_end(&held), Some(End::Cancelled));
    assert_eq!(Arc::strong_count(&dec), 1, "workers released the registry");
    assert_eq!(
        Arc::strong_count(&files),
        1,
        "workers released the file access"
    );
    dec.release_all();
}

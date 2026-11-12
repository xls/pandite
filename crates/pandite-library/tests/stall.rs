//! Stall safety: a read that never returns (a cloud placeholder that never
//! downloads) must not block the caller, cancellation or shutdown, and
//! files without local data are read only when opened explicitly.
//!
//! Stalls are latches in the fake file system; every check that a call
//! does not block runs the call on a helper thread with a bounded wait.
//! The harness opens every latch in teardown.

mod common;

use common::{img, run_bounded, Call, Harness, QUICK, WAIT};
use pandite_library::{Event, Fetch};
use std::sync::Arc;
use std::time::Duration;

#[test]
fn stalled_read_does_not_block_caller() {
    let h = Harness::new("stall", |c| {
        c.workers = 2;
        c.read_ahead = 1;
    });
    let paths = h.dir.images(6, 16, 16);
    h.files.stall_read(&img(2));
    let lib = Arc::clone(h.lib());
    lib.set_items(paths.clone());
    lib.set_current(2);
    assert!(h.files.wait_read_started(&img(2)));
    assert!(matches!(lib.get(2), Fetch::Pending));
    // Every public call returns while that read is stuck forever.
    let caller = Arc::clone(&lib);
    let done = run_bounded(QUICK, move || {
        let lib = caller;
        let _ = lib.get(2);
        let _ = lib.thumbnail(2);
        let _ = lib.poll_events();
        let _ = lib.stats();
        let _ = (
            lib.len(),
            lib.is_empty(),
            lib.paths(),
            lib.current(),
            lib.path(1),
        );
        lib.set_visible_thumbnails(0..6);
        lib.set_current(3);
        lib.set_current(2);
        lib.invalidate(&paths[4]);
        lib.refresh();
        let _ = lib.set_items(paths.clone());
        lib.open(paths[2].clone());
        lib.cancel_current();
        let _ = lib.wait_idle(Duration::ZERO);
        format!("{lib:?}")
    });
    assert!(done.is_some(), "a public call blocked on the stalled read");
    assert_eq!(h.files.returned(&img(2)), 0, "the read is still stuck");
}

#[test]
fn cancel_abandons_stalled_read() {
    let mut h = Harness::new("cancel", |c| c.read_ahead = 0);
    let paths = h.dir.images(4, 16, 16);
    h.files.stall_read(&img(1));
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(1);
    assert!(h.files.wait_read_started(&img(1)));
    // Escape: returns at once even though the read cannot be interrupted.
    let caller = Arc::clone(&lib);
    assert!(run_bounded(QUICK, move || caller.cancel_current()).is_some());
    assert!(matches!(lib.get(1), Fetch::Cancelled));
    let s = lib.stats();
    assert_eq!((s.threads, s.written_off), (1, 1), "stuck worker replaced");
    // With its only original worker stuck, the library still loads images.
    lib.set_current(2);
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 2, .. })));
    // The stuck read finally returns: its bytes are thrown away.
    h.files.release_read(&img(1));
    assert!(h.files.wait_read_returned(&img(1)));
    h.idle();
    assert_eq!(lib.stats().written_off, 0, "the written-off thread left");
    assert_eq!(
        h.dec.count(&Call::full(&img(1))),
        0,
        "abandoned result dropped"
    );
    assert!(matches!(lib.get(1), Fetch::Cancelled));
    // Selecting the image again retries it.
    lib.set_current(1);
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 1, .. })));
}

#[test]
fn drop_does_not_wait_for_stalled_read() {
    // A long grace: dropping still must not wait for a worker in a read.
    let mut h = Harness::new("dropstall", |c| {
        c.read_ahead = 0;
        c.shutdown_grace = WAIT;
    });
    let paths = h.dir.images(3, 16, 16);
    h.files.stall_read(&img(0));
    let lib = h.take_lib();
    lib.set_items(paths);
    lib.set_current(0);
    assert!(h.files.wait_read_started(&img(0)));
    assert!(
        run_bounded(QUICK, move || drop(lib)).is_some(),
        "drop waited for the stuck read"
    );
    assert_eq!(
        h.files.returned(&img(0)),
        0,
        "the stuck worker was detached"
    );
    // Teardown: the detached worker finishes, drops its result and frees
    // the shared state.
    h.files.release_all();
    assert!(h.files.wait_read_returned(&img(0)));
    assert!(h.wait_gone(), "detached worker released the library state");
    assert_eq!(h.dec.count(&Call::full(&img(0))), 0);
}

#[test]
fn folder_scan_runs_off_caller_thread() {
    let mut h = Harness::new("listing", |c| {
        c.workers = 2;
        c.read_ahead = 1;
    });
    let paths = h.dir.images(8, 16, 16);
    h.files.stall_listing();
    let lib = Arc::clone(h.lib());
    let caller = Arc::clone(&lib);
    let target = paths[5].clone();
    assert!(run_bounded(QUICK, move || caller.open(target)).is_some());
    assert!(h.files.wait_listing_started());
    // The opened file is shown before its folder is known.
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 0, .. })));
    assert_eq!((lib.len(), lib.current()), (1, Some(0)));
    assert!(lib.get(0).is_ready());
    h.files.release_listing();
    assert!(h.wait_for(|e| matches!(
        e,
        Event::Listed {
            len: 8,
            current: Some(5)
        }
    )));
    assert_eq!(lib.paths(), paths);
    h.idle();
    // The early decode carried over by path; read-ahead covers the rest.
    assert!(lib.get(5).is_ready());
    assert_eq!(h.dec.count(&Call::full(&img(5))), 1);
    assert!(lib.get(4).is_ready() && lib.get(6).is_ready());
}

#[test]
fn non_local_files_are_not_prefetched() {
    let mut h = Harness::new("cloud", |c| {
        c.read_ahead = 2;
        c.background_thumbnails = true;
    });
    let paths = h.dir.images(8, 16, 16);
    h.files.set_cloud(&img(3));
    h.files.set_cloud(&img(5));
    let lib = Arc::clone(h.lib());
    lib.open(paths[4].clone());
    assert!(h.wait_for(|e| matches!(
        e,
        Event::Listed {
            current: Some(4),
            ..
        }
    )));
    h.idle();
    for i in [3, 5] {
        assert_eq!(h.files.reads(&img(i)), 0, "{i} must not be downloaded");
        assert!(matches!(lib.get(i), Fetch::NotLocal));
        assert!(matches!(lib.thumbnail(i), Fetch::NotLocal));
    }
    for i in [2, 4, 6] {
        assert!(lib.get(i).is_ready());
    }
    for i in [0, 1, 2, 4, 6, 7] {
        assert!(lib.thumbnail(i).is_ready());
    }

    // Without a listing (paths handed over directly) the workers check
    // each file themselves and report what is not local.
    let mut h2 = Harness::new("cloud-items", |c| c.read_ahead = 2);
    let paths = h2.dir.images(8, 16, 16);
    h2.files.set_cloud(&img(3));
    h2.files.set_cloud(&img(5));
    let lib = Arc::clone(h2.lib());
    lib.set_items(paths);
    lib.set_current(4);
    h2.idle();
    h2.pump();
    for i in [3, 5] {
        assert_eq!(h2.files.reads(&img(i)), 0);
        assert!(matches!(lib.get(i), Fetch::NotLocal));
        let reported = |e: &Event| matches!(e, Event::NotLocal { index, .. } if *index == i);
        assert_eq!(h2.count_events(reported), 1);
    }
}

#[test]
fn navigating_to_a_non_local_neighbor_downloads_it() {
    let mut h = Harness::new("cloud-nav", |c| {
        c.workers = 2;
        c.read_ahead = 1;
    });
    let paths = h.dir.images(8, 16, 16);
    h.files.set_cloud(&img(5));
    // Hold the read-ahead check of 5 while the user moves onto it.
    h.files.stall_info(&img(5));
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(4);
    assert!(h.files.wait_info_started(&img(5)));
    lib.set_current(5);
    // The other worker starts 6 only after its scan passed the current
    // image's entry and tied the running read-ahead job to it.
    assert!(h.dec.wait_started(&Call::full(&img(6))));
    h.files.release_info(&img(5));
    // The read-ahead job finds the file not local and skips it; as the
    // current image it is then read explicitly.
    assert!(h.wait_for(|e| matches!(e, Event::Downloading { index: 5, .. })));
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 5, .. })));
    assert_eq!(h.files.reads(&img(5)), 1);
}

#[test]
fn explicit_open_of_non_local_file_reports_downloading() {
    let mut h = Harness::new("download", |c| {
        c.workers = 2;
        c.read_ahead = 1;
    });
    let paths = h.dir.images(5, 16, 16);
    h.files.set_cloud(&img(2));
    // The download takes until the test releases it.
    h.files.stall_read(&img(2));
    let lib = Arc::clone(h.lib());
    lib.open(paths[2].clone());
    assert!(h.wait_for(|e| matches!(e, Event::Downloading { .. })));
    assert!(h.wait_for(|e| matches!(
        e,
        Event::Listed {
            current: Some(2),
            ..
        }
    )));
    assert!(matches!(lib.get(2), Fetch::Downloading));
    assert!(matches!(lib.thumbnail(2), Fetch::NotLocal));
    h.files.release_read(&img(2));
    assert!(h.wait_for(|e| matches!(e, Event::Ready { index: 2, .. })));
    h.idle();
    assert!(lib.get(2).is_ready());
    // Its thumbnail comes from the decoded image: still a single read.
    assert!(lib.thumbnail(2).is_ready());
    assert_eq!(h.files.reads(&img(2)), 1);
    // Moving on keeps the decoded image: nothing is downloaded again.
    lib.set_current(3);
    h.idle();
    assert_eq!(h.files.reads(&img(2)), 1);
}

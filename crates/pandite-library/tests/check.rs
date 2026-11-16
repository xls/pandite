//! Folder checks: a check of a folder that did not change costs one
//! listing and nothing else, and a check of a changed folder reports what
//! came, went and changed while cached work for unchanged files stays.

mod common;

use common::{img, Call, FakeFiles, Harness};
use pandite_library::{Event, FileAccess, FileInfo, ListChange};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A library over `count` fake images with every thumbnail wanted, opened
/// at image `current`, listed, and idle with every thumbnail made.
fn listed(tag: &str, count: usize, current: usize) -> (Harness, Vec<PathBuf>) {
    let mut h = Harness::new(tag, |c| c.read_ahead = 1);
    let paths = h.dir.images(count, 16, 16);
    let lib = Arc::clone(h.lib());
    lib.open(paths[current].clone());
    assert!(h.wait_for(|e| matches!(e, Event::Listed { .. })));
    lib.set_visible_thumbnails(0..count);
    h.idle();
    h.pump();
    for i in 0..count {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }
    (h, paths)
}

/// The change of the one [`Event::Changed`] logged, with the current
/// index it reported.
fn changed(h: &Harness) -> (Option<usize>, ListChange) {
    let found: Vec<_> = h
        .events
        .iter()
        .filter_map(|e| match e {
            Event::Changed {
                current, change, ..
            } => Some((*current, change.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "one change: {:?}", h.events);
    found[0].clone()
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn check_of_an_unchanged_folder_costs_one_listing_and_sends_nothing() {
    let (mut h, paths) = listed("check-same", 6, 2);
    let lib = Arc::clone(h.lib());
    let (lists, infos, decodes) = (h.files.lists(), h.files.infos(), h.dec.starts().len());
    let (wakes, events, stats) = (h.wakes(), h.events.len(), lib.stats());
    for _ in 0..3 {
        lib.check_folder();
        h.idle();
    }
    h.pump();
    // One directory listing per check and no other file system call.
    assert_eq!(h.files.lists(), lists + 3);
    assert_eq!(h.files.infos(), infos);
    // No event, no wake, no decode; every thumbnail stays.
    assert_eq!(h.events.len(), events, "{:?}", &h.events[events..]);
    assert_eq!(h.wakes(), wakes);
    assert_eq!(h.dec.starts().len(), decodes);
    assert_eq!(lib.stats().thumb_entries, stats.thumb_entries);
    assert_eq!(lib.stats().full_entries, stats.full_entries);
    assert_eq!(lib.paths(), paths);
    assert_eq!(lib.current(), Some(2));
    for i in 0..6 {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }
}

#[test]
fn check_reports_added_removed_renamed_and_changed_files() {
    let (mut h, paths) = listed("check-change", 6, 2);
    let lib = Arc::clone(h.lib());
    let thumbs = h.dec.thumb_starts().len();
    // A new file sorts between img03 and img04, img04 goes, and img01 is
    // renamed to img09.
    h.dir.image("img03a.fake", 16, 16);
    std::fs::remove_file(&paths[4]).unwrap();
    std::fs::rename(&paths[1], h.dir.path().join("img09.fake")).unwrap();
    let before = h.events.len();
    lib.check_folder();
    assert!(h.wait_for(|e| matches!(e, Event::Changed { .. })));
    assert!(!h.events[before..]
        .iter()
        .any(|e| matches!(e, Event::Listed { .. })));
    let (current, change) = changed(&h);
    assert_eq!(
        names(&lib.paths()),
        [
            "img00.fake",
            "img02.fake",
            "img03.fake",
            "img03a.fake",
            "img05.fake",
            "img09.fake"
        ]
    );
    assert_eq!(
        change,
        ListChange {
            moved: vec![Some(0), None, Some(1), Some(2), None, Some(4)],
            added: 2,
            removed: 2,
            reordered: false,
            modified: Vec::new(),
        }
    );
    assert!(!change.is_empty());
    // The current image keeps its file at its new index.
    assert_eq!(current, Some(1));
    assert_eq!(lib.current(), Some(1));
    assert_eq!(lib.path(1).as_deref(), Some(paths[2].as_path()));
    // Only the new files get thumbnail work; the cached thumbnails stay.
    lib.set_visible_thumbnails(0..6);
    h.idle();
    assert_eq!(
        h.dec.thumb_starts()[thumbs..],
        ["img03a.fake", "img09.fake"]
    );
    for i in 0..6 {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }

    // A file rewritten with another size: same path, new stamp. Its
    // thumbnail is made again; nothing else is decoded.
    h.events.clear();
    let decodes = h.dec.starts().len();
    h.dir.image("img05.fake", 160, 16);
    lib.check_folder();
    assert!(h.wait_for(|e| matches!(e, Event::Changed { .. })));
    let (current, change) = changed(&h);
    assert_eq!(current, Some(1));
    assert_eq!(change.moved, (0..6).map(Some).collect::<Vec<_>>());
    assert_eq!((change.added, change.removed), (0, 0));
    assert_eq!(change.modified, [4]);
    lib.set_visible_thumbnails(0..6);
    h.idle();
    assert_eq!(h.dec.starts()[decodes..], [Call::thumb("img05.fake")]);
    let thumb = lib.thumbnail(4).ready().expect("made again");
    assert_eq!((thumb.width(), thumb.height()), (8, 1));

    // Nothing changed since: the next check is silent again.
    h.settle();
    h.events.clear();
    let wakes = h.wakes();
    lib.check_folder();
    h.idle();
    h.pump();
    assert!(h.events.is_empty(), "{:?}", h.events);
    assert_eq!(h.wakes(), wakes);
}

#[test]
fn check_moves_to_the_neighbor_when_the_current_file_leaves() {
    let (mut h, paths) = listed("check-current", 5, 2);
    let lib = Arc::clone(h.lib());
    std::fs::remove_file(&paths[2]).unwrap();
    lib.check_folder();
    assert!(h.wait_for(|e| matches!(e, Event::Changed { .. })));
    let (current, change) = changed(&h);
    // As after a refresh: the file that takes its place is current.
    assert_eq!(current, Some(2));
    assert_eq!(lib.path(2).as_deref(), Some(paths[3].as_path()));
    assert_eq!(change.moved, [Some(0), Some(1), None, Some(2), Some(3)]);
    assert_eq!((change.added, change.removed), (0, 1));
    h.idle();
    assert!(
        lib.get(2).is_ready(),
        "the neighbor loads as the current image"
    );
}

#[test]
fn check_keeps_the_opened_file_found_by_signature() {
    // An opened file without a known extension stays in the list after
    // the user moved on, as long as the folder holds it.
    let mut h = Harness::new("check-opened", |_| {});
    h.dir.images(2, 4, 4);
    let odd = h.dir.file("scan.dat", b"FAKE 4 4");
    let lib = Arc::clone(h.lib());
    lib.open(odd.clone());
    assert!(h.wait_for(|e| matches!(e, Event::Listed { len: 3, .. })));
    lib.set_current(0);
    h.idle();
    h.pump();
    let events = h.events.len();
    lib.check_folder();
    h.idle();
    h.pump();
    assert_eq!(h.events.len(), events, "{:?}", &h.events[events..]);
    assert_eq!(lib.path(2).as_deref(), Some(odd.as_path()));
    // Once it is gone it leaves like any file.
    std::fs::remove_file(&odd).unwrap();
    lib.check_folder();
    assert!(h.wait_for(|e| matches!(e, Event::Changed { len: 2, .. })));
}

/// The fake disk, with folder listings that fail on demand.
struct Refusing {
    inner: Arc<FakeFiles>,
    refuse: AtomicBool,
}

impl FileAccess for Refusing {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        if self.refuse.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"));
        }
        self.inner.list_dir(dir)
    }

    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        self.inner.file_info(path)
    }

    fn read(
        &self,
        path: &Path,
        cancel: &pandite_core::CancelToken,
    ) -> pandite_core::Result<pandite_core::Source> {
        self.inner.read(path, cancel)
    }
}

#[test]
fn check_that_cannot_list_or_finds_a_listing_on_its_way_does_nothing() {
    let files = Arc::new(Refusing {
        inner: FakeFiles::new(),
        refuse: AtomicBool::new(false),
    });
    let shared = Arc::clone(&files);
    let mut h = Harness::new("check-refused", move |c| {
        c.files = shared as Arc<dyn FileAccess>;
    });
    let paths = h.dir.images(3, 4, 4);
    let lib = Arc::clone(h.lib());
    lib.open(paths[1].clone());
    assert!(h.wait_for(|e| matches!(e, Event::Listed { len: 3, .. })));
    h.idle();
    h.pump();
    // The folder cannot be listed now: the list, its current image and
    // the listing error stay, and nothing is sent.
    files.refuse.store(true, Ordering::SeqCst);
    std::fs::remove_file(&paths[0]).unwrap();
    let (events, wakes) = (h.events.len(), h.wakes());
    lib.check_folder();
    h.idle();
    h.pump();
    assert_eq!(h.events.len(), events);
    assert_eq!(h.wakes(), wakes);
    assert_eq!(lib.paths(), paths);
    assert_eq!(lib.current(), Some(1));
    assert_eq!(lib.listing_error(), None);
    files.refuse.store(false, Ordering::SeqCst);

    // A check while the listing of an opened file runs adds no listing:
    // that listing tells.
    let mut h = Harness::new("check-pending", |_| {});
    let paths = h.dir.images(3, 4, 4);
    let lib = Arc::clone(h.lib());
    h.files.stall_listing();
    lib.open(paths[0].clone());
    assert!(h.files.wait_listing_started());
    lib.check_folder();
    h.files.release_listing();
    assert!(h.wait_for(|e| matches!(e, Event::Listed { len: 3, .. })));
    h.idle();
    h.pump();
    assert_eq!(h.files.lists(), 1);
    assert_eq!(h.count_events(|e| matches!(e, Event::Changed { .. })), 0);
    // No folder known yet: nothing to check.
    let h = Harness::new("check-none", |_| {});
    h.lib().set_items(vec![PathBuf::from(img(0))]);
    h.lib().check_folder();
    h.idle();
    assert_eq!(h.files.lists(), 0);
}

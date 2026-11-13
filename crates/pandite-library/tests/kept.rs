//! Thumbnails kept between two libraries: the first exports the
//! thumbnails of its folder within the budget, the second uses each one
//! whose file did not change instead of decoding it again, and drops
//! thumbnails of another folder at once.

mod common;

use common::*;
use pandite_library::{Event, KeptThumbnails, LibraryConfig, Stamp};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// No read-ahead: only the current image gets a full decode, so every
/// other thumbnail needs a decode of its own unless it is kept.
fn tune(c: &mut LibraryConfig) {
    c.read_ahead = 0;
}

fn name_of(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

/// Open `path`, show every thumbnail of its folder and wait until all of
/// them are made.
fn show_folder(h: &mut Harness, path: &Path, count: usize) {
    let lib = Arc::clone(h.lib());
    lib.open(path.to_path_buf());
    assert!(h.wait_for(|e| matches!(e, Event::Listed { .. })));
    lib.set_visible_thumbnails(0..count);
    h.idle();
    h.pump();
    for i in 0..count {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }
}

/// A first session on `path`: every thumbnail made, then exported.
fn first_session(h: &mut Harness, path: &Path, count: usize) -> KeptThumbnails {
    show_folder(h, path, count);
    h.lib().export_thumbnails().expect("thumbnails to keep")
}

#[test]
fn thumbs_export_within_budget() {
    // 64 x 48 images give 8 x 6 thumbnails of 192 bytes: room for four.
    let budget = 4 * 8 * 6 * 4;
    let mut h = Harness::new("kept-budget", |c| {
        tune(c);
        c.thumb_budget = budget;
    });
    let paths = h.dir.images(12, 64, 48);
    // The whole folder is on screen: visible thumbnails stay even above
    // the budget.
    show_folder(&mut h, &paths[5], 12);
    let lib = Arc::clone(h.lib());
    let s = lib.stats();
    assert_eq!(s.thumb_entries, 12);
    assert!(
        s.thumb_bytes > budget,
        "the cache is above its budget: {s:?}"
    );

    let kept = lib.export_thumbnails().expect("thumbnails to keep");
    assert_eq!(kept.folder, h.dir.path());
    assert_eq!(kept.thumb_size, 8);
    assert!(kept.bytes() <= budget, "{kept:?}");
    // All are visible: the four nearest the current image (5) are kept,
    // the next side first.
    let names = sorted(kept.entries.keys().map(|p| name_of(p)).collect());
    assert_eq!(names, imgs(&[4, 5, 6, 7]));
    for (path, entry) in &kept.entries {
        let meta = std::fs::metadata(path).unwrap();
        assert_eq!(entry.stamp, Stamp::of(&meta), "{path:?}");
        assert!(entry.stamp.modified.is_some());
        assert_eq!((entry.image.width(), entry.image.height()), (8, 6));
    }
    // The export shares the pixels; the library keeps its thumbnails.
    assert_eq!(lib.stats().thumb_entries, 12);
}

#[test]
fn thumbs_import_skips_decode() {
    let mut first = Harness::new("kept-first", tune);
    let paths = first.dir.images(8, 64, 48);
    let kept = first_session(&mut first, &paths[0], 8);
    assert_eq!(kept.len(), 8);
    // The current image's thumbnail comes from its full decode; the
    // seven others were decoded.
    assert_eq!(
        sorted(first.dec.thumb_starts()),
        imgs(&[1, 2, 3, 4, 5, 6, 7])
    );

    let given = kept.clone();
    let mut second = Harness::new("kept-second", |c| {
        tune(c);
        c.kept_thumbnails = Some(given);
    });
    let lib = Arc::clone(second.lib());
    let s = lib.stats();
    assert_eq!((s.kept_entries, s.kept_bytes), (8, kept.bytes()));
    assert_eq!(s.thumb_entries, 0, "kept thumbnails wait for the listing");
    lib.open(paths[0].clone());
    assert!(second.wait_for(|e| matches!(e, Event::Listed { .. })));
    // The listing checked them: they are cached thumbnails now.
    let s = lib.stats();
    assert_eq!((s.kept_entries, s.kept_bytes), (0, 0));
    assert_eq!(s.thumb_entries, 8);
    assert!(s.thumb_bytes <= s.thumb_budget);
    lib.set_visible_thumbnails(0..8);
    second.idle();
    second.pump();
    for (i, path) in paths.iter().enumerate() {
        let thumb = lib.thumbnail(i).ready().expect("thumbnail ready");
        assert_eq!(thumb.data(), kept.entries[path].image.data(), "{path:?}");
        assert!(
            second
                .events
                .iter()
                .any(|e| matches!(e, Event::ThumbnailReady { index, .. } if *index == i)),
            "thumbnail {i} announced"
        );
    }
    assert!(
        second.dec.thumb_starts().is_empty(),
        "no thumbnail decoded: {:?}",
        second.dec.thumb_starts()
    );
    assert_eq!(second.dec.full_starts(), imgs(&[0]));
}

#[test]
fn thumbs_import_rejects_changed_file() {
    let mut first = Harness::new("kept-changed-a", tune);
    let paths = first.dir.images(6, 64, 48);
    let kept = first_session(&mut first, &paths[0], 6);
    assert_eq!(kept.len(), 6);
    // Image 2 changes its size on disk. Image 4 keeps its size ("FAKE 48
    // 64" is as long as "FAKE 64 48") but gets another modification time.
    first.dir.image(&img(2), 30, 120);
    first.dir.image(&img(4), 48, 64);
    let old = kept.entries[&paths[4]].stamp;
    let later = old.modified.unwrap() + Duration::from_secs(10);
    std::fs::File::options()
        .write(true)
        .open(&paths[4])
        .unwrap()
        .set_modified(later)
        .unwrap();
    let now = Stamp::of(&std::fs::metadata(&paths[4]).unwrap());
    assert_eq!(now.len, old.len, "same size");
    assert_ne!(now.modified, old.modified);

    let mut second = Harness::new("kept-changed-b", |c| {
        tune(c);
        c.kept_thumbnails = Some(kept);
    });
    show_folder(&mut second, &paths[0], 6);
    let lib = Arc::clone(second.lib());
    // Only the changed files are decoded again, and their thumbnails show
    // the new pictures.
    assert_eq!(sorted(second.dec.thumb_starts()), imgs(&[2, 4]));
    let dims = |i: usize| {
        let t = lib.thumbnail(i).ready().unwrap();
        (t.width(), t.height())
    };
    assert_eq!(dims(2), (2, 8));
    assert_eq!(dims(4), (6, 8));
    for i in [0, 1, 3, 5] {
        assert_eq!(dims(i), (8, 6), "kept thumbnail {i}");
    }
    let s = lib.stats();
    assert_eq!((s.thumb_entries, s.kept_entries), (6, 0));
}

#[test]
fn thumbs_import_dropped_for_other_folder() {
    let mut first = Harness::new("kept-other-a", tune);
    let paths = first.dir.images(5, 64, 48);
    let kept = first_session(&mut first, &paths[0], 5);
    let bytes = kept.bytes();
    assert!(bytes > 0);

    // A library with another thumbnail size drops them when it starts.
    let resized = kept.clone();
    let other_size = Harness::new("kept-other-size", |c| {
        tune(c);
        c.thumb_size = 16;
        c.kept_thumbnails = Some(resized);
    });
    let s = other_size.lib().stats();
    assert_eq!((s.kept_entries, s.kept_bytes), (0, 0));
    drop(other_size);

    let mut second = Harness::new("kept-other-b", |c| {
        tune(c);
        c.kept_thumbnails = Some(kept);
    });
    let s = second.lib().stats();
    assert_eq!((s.kept_entries, s.kept_bytes), (5, bytes));
    // Another folder with the same file names.
    let others = second.dir.images(3, 64, 48);
    let lib = Arc::clone(second.lib());
    lib.open(others[0].clone());
    // Dropped at once, before any worker looked at the folder.
    let s = lib.stats();
    assert_eq!((s.kept_entries, s.kept_bytes, s.thumb_entries), (0, 0, 0));
    assert!(second.wait_for(|e| matches!(e, Event::Listed { len: 3, .. })));
    lib.set_visible_thumbnails(0..3);
    second.idle();
    second.pump();
    for i in 0..3 {
        assert!(lib.thumbnail(i).is_ready(), "thumbnail {i}");
    }
    // Made from this folder's files, not taken from the other one.
    assert_eq!(sorted(second.dec.thumb_starts()), imgs(&[1, 2]));
    let next = lib.export_thumbnails().expect("thumbnails to keep");
    assert_eq!(next.folder, second.dir.path());
    assert_eq!(next.len(), 3);
    assert!(next
        .entries
        .keys()
        .all(|p| p.starts_with(second.dir.path())));
}

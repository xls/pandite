//! A change of the thumbnail size: every thumbnail is made again at the
//! new size, the old one shows until then, cached pixels are used instead
//! of a new decode where they suffice, and kept thumbnails of another size
//! are dropped.

mod common;

use common::*;
use pandite_library::{Event, LibraryConfig};
use std::path::Path;
use std::sync::Arc;

/// No read-ahead: only the current image has a full decode.
fn tune(c: &mut LibraryConfig) {
    c.read_ahead = 0;
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

fn dims(h: &Harness, index: usize) -> (u32, u32) {
    let t = h.lib().thumbnail(index).ready().expect("thumbnail ready");
    (t.width(), t.height())
}

/// How often the thumbnail of `index` was announced.
fn announced(h: &Harness, index: usize) -> usize {
    h.count_events(|e| matches!(e, Event::ThumbnailReady { index: i, .. } if *i == index))
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
}

#[test]
fn thumb_size_change_remakes_every_thumbnail() {
    let mut h = Harness::new("size-change", tune);
    let paths = h.dir.images(6, 64, 48);
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(0);
    lib.set_visible_thumbnails(0..6);
    h.idle();
    h.pump();
    assert_eq!(lib.thumbnail_size(), 8);
    for i in 0..6 {
        assert_eq!(dims(&h, i), (8, 6), "thumbnail {i}");
    }
    // The current image's thumbnail came from its full decode.
    assert_eq!(sorted(h.dec.thumb_starts()), imgs(&[1, 2, 3, 4, 5]));
    let starts = h.dec.starts().len();

    // Smaller: each new thumbnail is made from the old one. Nothing is
    // read or decoded again, and each is announced again.
    lib.set_thumbnail_size(4);
    assert_eq!(lib.thumbnail_size(), 4);
    h.idle();
    h.pump();
    for i in 0..6 {
        assert_eq!(dims(&h, i), (4, 3), "thumbnail {i}");
        assert_eq!(announced(&h, i), 2, "thumbnail {i}");
        assert_eq!(h.files.reads(&img(i)), 1, "{}", img(i));
    }
    assert_eq!(h.dec.starts().len(), starts, "no decode");

    // The same size again changes nothing.
    let events = h.events.len();
    lib.set_thumbnail_size(4);
    h.idle();
    h.pump();
    assert_eq!((h.events.len(), h.dec.starts().len()), (events, starts));

    // Larger: the old thumbnails are too small, so the files are decoded
    // again, except the current image, whose full decode is cached. Until
    // its new thumbnail is made, the old one shows.
    let held = Call::thumb(&img(3));
    h.dec.hold(held.clone());
    lib.set_thumbnail_size(16);
    assert!(h.dec.wait_starts(&held, 2));
    assert_eq!(dims(&h, 3), (4, 3), "the old thumbnail shows meanwhile");
    h.dec.release(&held);
    h.idle();
    h.pump();
    for i in 0..6 {
        assert_eq!(dims(&h, i), (16, 12), "thumbnail {i}");
        assert_eq!(announced(&h, i), 3, "thumbnail {i}");
    }
    assert_eq!(h.dec.full_starts(), imgs(&[0]), "one full decode");
    for i in 1..6 {
        assert_eq!(h.dec.count(&Call::thumb(&img(i))), 2, "{}", img(i));
    }
    let s = lib.stats();
    assert_eq!((s.thumb_entries, s.thumb_bytes), (6, 6 * 16 * 12 * 4));
}

#[test]
fn thumb_made_at_the_old_size_is_made_again() {
    let mut h = Harness::new("size-running", tune);
    let paths = h.dir.images(4, 64, 48);
    let lib = Arc::clone(h.lib());
    // The only worker is inside the decode of a thumbnail when the size
    // changes.
    let held = Call::thumb(&img(2));
    h.dec.hold(held.clone());
    lib.open(paths[0].clone());
    assert!(h.wait_for(|e| matches!(e, Event::Listed { .. })));
    lib.set_visible_thumbnails(0..4);
    assert!(h.dec.wait_started(&held));
    lib.set_thumbnail_size(16);
    h.dec.release(&held);
    h.idle();
    h.pump();
    // Its result had the old size: it showed until the new one came, and
    // the job ran again at the new size.
    assert_eq!(dims(&h, 2), (16, 12));
    assert_eq!(h.dec.count(&held), 2);
    assert_eq!(announced(&h, 2), 2);
    for i in [0, 1, 3] {
        assert_eq!(dims(&h, i), (16, 12), "thumbnail {i}");
    }
    // Thumbnail 3 started after the change: one decode only.
    assert_eq!(h.dec.count(&Call::thumb(&img(3))), 1);
    // Only thumbnails of the current size go to the next library.
    let kept = lib.export_thumbnails().expect("thumbnails to keep");
    assert_eq!(kept.thumb_size, 16);
    assert_eq!(kept.len(), 4);
    assert!(kept
        .entries
        .values()
        .all(|e| (e.image.width(), e.image.height()) == (16, 12)));
}

#[test]
fn stale_thumbs_are_not_exported() {
    let mut h = Harness::new("size-export", tune);
    let paths = h.dir.images(4, 64, 48);
    show_folder(&mut h, &paths[0], 4);
    let lib = Arc::clone(h.lib());
    assert_eq!(lib.export_thumbnails().expect("thumbnails").len(), 4);
    // Larger, with every decode held: only the current image's thumbnail
    // is made at the new size, from its full decode.
    for i in 1..4 {
        h.dec.hold(Call::thumb(&img(i)));
    }
    lib.set_thumbnail_size(16);
    assert!(h.dec.wait_starts(&Call::thumb(&img(1)), 2));
    assert_eq!(dims(&h, 0), (16, 12));
    let kept = lib.export_thumbnails().expect("thumbnails");
    assert_eq!(kept.thumb_size, 16);
    let names: Vec<String> = kept
        .entries
        .keys()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, imgs(&[0]), "the stale thumbnails stay behind");
    h.dec.release_all();
    h.idle();
}

#[test]
fn kept_thumbs_of_another_size_are_dropped() {
    let mut first = Harness::new("size-kept-a", tune);
    let paths = first.dir.images(5, 64, 48);
    show_folder(&mut first, &paths[0], 5);
    let kept = first.lib().export_thumbnails().expect("thumbnails to keep");
    assert_eq!((kept.thumb_size, kept.len()), (8, 5));

    // The next library starts at the same size, then the size changes
    // before the folder is listed: the kept thumbnails go at once.
    let mut second = Harness::new("size-kept-b", |c| {
        tune(c);
        c.kept_thumbnails = Some(kept);
    });
    let lib = Arc::clone(second.lib());
    assert_eq!(lib.stats().kept_entries, 5);
    lib.set_thumbnail_size(16);
    let s = lib.stats();
    assert_eq!((s.kept_entries, s.kept_bytes), (0, 0));
    show_folder(&mut second, &paths[0], 5);
    // None was used: each is made from its file at the new size.
    for i in 0..5 {
        assert_eq!(dims(&second, i), (16, 12), "thumbnail {i}");
    }
    assert_eq!(sorted(second.dec.thumb_starts()), imgs(&[1, 2, 3, 4]));
}

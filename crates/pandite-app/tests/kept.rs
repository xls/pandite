//! Thumbnails kept between window sessions: the core that ends hands the
//! thumbnails of its folder back, and the next core uses them when it
//! opens an image in the same folder, so the gallery shows without
//! decoding the thumbnails again. For another folder they are freed.

mod support;

use pandite_app::settings::Settings;
use pandite_library::KeptThumbnails;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use support::*;

const NAMES: [&str; 6] = ["a.stg", "b.stg", "c.stg", "d.stg", "e.stg", "f.stg"];

/// A folder of small staged images.
fn staged_folder(tag: &str) -> (TempDir, Vec<PathBuf>) {
    let dir = TempDir::new(tag);
    let paths = NAMES
        .iter()
        .map(|n| {
            let p = dir.join(n);
            write_staged(&p, 64, 48);
            p
        })
        .collect();
    (dir, paths)
}

/// A core on `open` with the gated decoder and no read-ahead, so every
/// thumbnail except the current image's needs a decode of its own.
fn harness(open: &Path, dec: &Arc<GatedDecoder>, thumbnails: Option<KeptThumbnails>) -> Harness {
    Harness::new(Config {
        settings: Settings {
            read_ahead: 0,
            ..Settings::default()
        },
        open: Some(open.to_path_buf()),
        registry: Some(dec.registry()),
        thumbnails,
        ..Config::default()
    })
}

/// Wait until `path` shows, every thumbnail of its folder is uploaded and
/// the library rests.
fn settle(h: &mut Harness, path: &Path) {
    assert!(h.wait_shown(path));
    assert!(h.app().library().wait_idle(WAIT));
    assert!(h.wait_thumbnails(NAMES.len()));
}

/// End the core the way the window shell does when it goes to the
/// background, and take the thumbnails it hands back.
fn end_session(h: &mut Harness) -> KeptThumbnails {
    let report = h.app.take().unwrap().shutdown(Duration::from_secs(10));
    report.thumbnails.expect("the thumbnails of the folder")
}

/// The thumbnail decodes, sorted.
fn thumbnail_decodes(dec: &GatedDecoder) -> Vec<String> {
    let mut names = dec.thumbnail_decodes();
    names.sort();
    names
}

#[test]
fn thumbs_kept_across_sessions() {
    let (dir, paths) = staged_folder("kept-sessions");
    let first_dec = GatedDecoder::new();
    let mut first = harness(&paths[0], &first_dec, None);
    first.start();
    settle(&mut first, &paths[0]);
    // The current image's thumbnail comes from its full decode; the others
    // were decoded.
    assert_eq!(thumbnail_decodes(&first_dec), &NAMES[1..]);
    let kept = end_session(&mut first);
    assert_eq!(kept.folder, dir.path());
    assert_eq!(kept.len(), NAMES.len());
    let limit = (Settings::default().thumbnail_cache_mb as usize) << 20;
    assert!(kept.bytes() <= limit, "{kept:?}");

    // The next session opens another image of the same folder.
    let second_dec = GatedDecoder::new();
    let mut second = harness(&paths[3], &second_dec, Some(kept));
    second.start();
    settle(&mut second, &paths[3]);
    assert_eq!(second.app().count(), NAMES.len());
    assert!(
        second_dec.thumbnail_decodes().is_empty(),
        "thumbnails decoded again: {:?}",
        second_dec.thumbnail_decodes()
    );
    // Only the opened image was decoded.
    for name in NAMES {
        let full = usize::from(name == "d.stg");
        assert_eq!(second_dec.full_finished(name), full, "{name}");
    }
    let s = second.app().library().stats();
    assert_eq!((s.thumb_entries, s.kept_entries), (NAMES.len(), 0));
    // The second session hands them on in turn.
    assert_eq!(end_session(&mut second).len(), NAMES.len());
}

#[test]
fn thumbs_freed_for_other_folder() {
    let (_first_dir, paths) = staged_folder("kept-folder-a");
    // Another folder with the same file names.
    let (other_dir, others) = staged_folder("kept-folder-b");
    let first_dec = GatedDecoder::new();
    let mut first = harness(&paths[0], &first_dec, None);
    first.start();
    settle(&mut first, &paths[0]);
    let kept = end_session(&mut first);
    let bytes = kept.bytes();
    assert_eq!(kept.len(), NAMES.len());

    let second_dec = GatedDecoder::new();
    let mut second = harness(&others[0], &second_dec, Some(kept));
    let s = second.app().library().stats();
    assert_eq!(
        (s.kept_entries, s.kept_bytes),
        (NAMES.len(), bytes),
        "held until the session opens its image"
    );
    second.start();
    // Freed the moment the image of the other folder opens.
    let s = second.app().library().stats();
    assert_eq!((s.kept_entries, s.kept_bytes, s.thumb_entries), (0, 0, 0));
    settle(&mut second, &others[0]);
    // This folder's thumbnails come from its own files.
    assert_eq!(thumbnail_decodes(&second_dec), &NAMES[1..]);
    let next = end_session(&mut second);
    assert_eq!(next.folder, other_dir.path());
    assert_eq!(next.len(), NAMES.len());
    assert!(next.entries.keys().all(|p| p.starts_with(other_dir.path())));
}

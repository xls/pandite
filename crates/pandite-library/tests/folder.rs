//! Folder listing: filtering, natural order, locating the opened file, and
//! tolerance of missing, empty and unreadable folders.

mod common;

use common::{Call, FakeDecoder, Harness, TempDir};
use pandite_core::Error;
use pandite_library::{Event, Fetch, FileAccess, FileInfo, FolderList};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn names(list: &FolderList) -> Vec<String> {
    list.paths()
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn folder_scan_filters_by_registry() {
    let dir = TempDir::new("filter");
    let reg = FakeDecoder::new().registry();
    let a = dir.image("a.fake", 1, 1);
    dir.image("B.FAKE", 1, 1);
    let text = dir.file("c.txt", b"FAKE 1 1");
    dir.file("d", b"FAKE 1 1");
    dir.file("e.png", b"not ours");
    std::fs::create_dir(dir.path().join("f.fake")).unwrap();
    let list = FolderList::scan(&a, &reg);
    assert_eq!(names(&list), ["a.fake", "B.FAKE"]);
    assert_eq!(list.opened_index(), Some(0));
    // The opened file stays even with an extension no decoder lists:
    // decoders also match by signature.
    let list = FolderList::scan(&text, &reg);
    assert_eq!(names(&list), ["a.fake", "B.FAKE", "c.txt"]);
    assert_eq!(list.opened_index(), Some(2));
    // A folder can be listed directly; nothing is marked as opened.
    let list = FolderList::scan(dir.path(), &reg);
    assert_eq!(names(&list), ["a.fake", "B.FAKE"]);
    assert_eq!(list.opened_index(), None);
    assert!(list.info(0).is_some_and(|i| i.is_file && i.local));
}

#[test]
fn folder_scan_natural_order() {
    let dir = TempDir::new("natural");
    let reg = FakeDecoder::new().registry();
    for name in [
        "img10.fake",
        "img2.fake",
        "Img1.fake",
        "img100.fake",
        "img9.fake",
        "Beta.fake",
        "alpha.fake",
        "photo 3.fake",
        "photo 20.fake",
    ] {
        dir.image(name, 1, 1);
    }
    let list = FolderList::scan(&dir.path().join("img9.fake"), &reg);
    assert_eq!(
        names(&list),
        [
            "alpha.fake",
            "Beta.fake",
            "Img1.fake",
            "img2.fake",
            "img9.fake",
            "img10.fake",
            "img100.fake",
            "photo 3.fake",
            "photo 20.fake",
        ]
    );
    assert_eq!(list.opened_index(), Some(4));
}

#[test]
fn folder_scan_locates_opened_file() {
    let dir = TempDir::new("locate");
    let reg = FakeDecoder::new().registry();
    let paths: Vec<PathBuf> = (1..=12)
        .map(|i| dir.image(&format!("img{i}.fake"), 1, 1))
        .collect();
    let opened = &paths[6];
    let list = FolderList::scan(opened, &reg);
    assert_eq!(list.len(), 12);
    assert_eq!(list.opened_index(), Some(6));
    assert_eq!(list.get(6), Some(opened.as_path()));
    assert_eq!(list.index_of(opened), Some(6));
    // Opened under another case on a case-insensitive file system: found,
    // and listed under the caller's spelling so later lookups match.
    let upper = dir.path().join("IMG7.FAKE");
    if upper.exists() {
        let list = FolderList::scan(&upper, &reg);
        assert_eq!(list.len(), 12);
        assert_eq!(list.opened_index(), Some(6));
        assert_eq!(list.get(6), Some(upper.as_path()));
    }
}

/// A folder that cannot be listed (access denied), while single files
/// still open.
struct Unreadable;

impl FileAccess for Unreadable {
    fn list_dir(&self, _dir: &Path) -> io::Result<Vec<FileInfo>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }
}

#[test]
fn folder_scan_handles_missing_and_empty() {
    let dir = TempDir::new("missing");
    let reg = FakeDecoder::new().registry();
    // Empty folder, opened file absent.
    let list = FolderList::scan(&dir.path().join("none.fake"), &reg);
    assert!(list.is_empty());
    assert_eq!(list.opened_index(), None);
    // Folder absent.
    let list = FolderList::scan(&dir.path().join("gone").join("x.fake"), &reg);
    assert!(list.is_empty());
    // A path with no parent, and an empty path.
    let list = FolderList::scan(Path::new("pandite-no-such-file.fake"), &reg);
    assert_eq!(list.opened_index(), None);
    assert!(FolderList::scan(Path::new(""), &reg).is_empty());
    // Unreadable folder: the opened file alone.
    let solo = dir.image("solo.fake", 1, 1);
    dir.image("other.fake", 1, 1);
    let list = FolderList::scan_with(&solo, &reg, &Unreadable);
    assert_eq!(list.paths(), std::slice::from_ref(&solo));
    assert_eq!(list.opened_index(), Some(0));
    // A file vanishes between two scans: the neighbor takes its place.
    let a = dir.image("a.fake", 1, 1);
    let b = dir.image("b.fake", 1, 1);
    let mut list = FolderList::scan(&b, &reg);
    assert_eq!(
        names(&list),
        ["a.fake", "b.fake", "other.fake", "solo.fake"]
    );
    std::fs::remove_file(&b).unwrap();
    assert_eq!(list.refresh(&reg, Some(1)), Some(1));
    assert_eq!(names(&list), ["a.fake", "other.fake", "solo.fake"]);
    for path in [&a, &solo] {
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_file(dir.path().join("other.fake")).unwrap();
    assert_eq!(list.refresh(&reg, Some(0)), None);
    assert!(list.is_empty());

    // In the library, a listed file that vanished before its decode is
    // reported once as an I/O error, and its neighbors still load.
    let mut h = Harness::new("vanished", |c| c.read_ahead = 1);
    let paths = h.dir.images(3, 4, 4);
    std::fs::remove_file(&paths[1]).unwrap();
    let lib = Arc::clone(h.lib());
    lib.set_items(paths);
    lib.set_current(1);
    h.idle();
    h.pump();
    match lib.get(1) {
        Fetch::Failed(err) => assert!(matches!(*err, Error::Io(_)), "{err}"),
        other => panic!("expected an I/O error, got {other:?}"),
    }
    assert_eq!(
        h.count_events(|e| matches!(e, Event::Failed { index: 1, .. })),
        1
    );
    assert!(lib.get(0).is_ready() && lib.get(2).is_ready());
    assert_eq!(h.dec.count(&Call::full("img01.fake")), 0);
}

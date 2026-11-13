//! Delete and Shift+Delete: the file of the current image goes to the
//! trash at once, or for good after a question. The folder list, the
//! position counter and the gallery strip lose it, and its neighbor
//! shows. These tests use the fake trash of the test disk; the last test
//! sends one file of its own to the real trash of this system.

mod support;

use pandite_app::trash;
use pandite_ui::{InputEvent, Key, Modifiers, UiMode};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use support::*;

const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;

/// A core on `path` with its folder listed and every thumbnail uploaded.
fn open(path: &Path, count: usize) -> Harness {
    let mut h = Harness::new(Config {
        open: Some(path.to_path_buf()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(path), "{} did not open", path.display());
    assert!(h.wait_until(|a| a.count() == count));
    assert!(h.wait_thumbnails(count));
    h
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect()
}

/// The fake trash beside `file`.
fn in_fake_trash(file: &Path) -> PathBuf {
    file.parent()
        .unwrap()
        .join(FAKE_TRASH)
        .join(file.file_name().unwrap())
}

/// The first pixel of the image on screen.
fn first_pixel(h: &Harness) -> [u8; 4] {
    let image = h
        .app
        .as_ref()
        .unwrap()
        .shown_image()
        .expect("an image shows");
    let rgba = pandite_ops::to_rgba8(image).unwrap();
    let p = &rgba.data()[..4];
    [p[0], p[1], p[2], p[3]]
}

#[test]
fn delete_moves_to_trash_and_shows_next() {
    let (_dir, paths) = folder("del-next", &["a.png", "b.png", "c.png"], 8, 8);
    let bytes = std::fs::read(&paths[1]).unwrap();
    let mut h = open(&paths[1], 3);
    assert_eq!(h.app().index(), Some(1));
    assert!(has_text(&h.frame(), "2 / 3"));

    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    // The file went to the trash, whole, and nothing was removed for good.
    assert!(!paths[1].exists());
    assert_eq!(std::fs::read(in_fake_trash(&paths[1])).unwrap(), bytes);
    assert_eq!(h.disk.removals(), [("trash", "b.png".to_string())]);
    // The list, the counter and the strip lost it at once, and the next
    // image takes its place.
    assert_eq!(h.app().count(), 2);
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert_eq!(h.app().target_path(), Some(paths[2].as_path()));
    assert_eq!(h.app().index(), Some(1));
    assert_eq!(h.app().thumbnails().len(), 2);
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(first_pixel(&h), decode_first_pixel(&paths[2]));
    assert_eq!(h.app().title(), "c.png - pandite");
    let list = h.frame();
    assert!(has_text(&list, "2 / 2"));
    assert!(has_text(&list, "Moved b.png to"), "a short confirmation");
    assert!(h.wait_thumbnails(2));

    // A held key never deletes a second file: one press, one file.
    h.input(InputEvent::KeyDown {
        key: Key::Delete,
        mods: NONE,
        repeat: true,
    });
    h.pump();
    assert!(!h.app().deleting());
    assert_eq!(h.disk.removal_calls(), 1);
    assert!(paths[2].exists());
}

/// The first pixel of a file as the codecs decode it.
fn decode_first_pixel(path: &Path) -> [u8; 4] {
    let (image, _) = decode(path);
    let p = &image.data()[..4];
    [p[0], p[1], p[2], p[3]]
}

#[test]
fn delete_last_shows_previous() {
    let (_dir, paths) = folder("del-last", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[2], 3);
    assert_eq!(h.app().index(), Some(2));
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(!paths[2].exists());
    assert_eq!(h.app().count(), 2);
    assert_eq!(names(&h.app().library().paths()), ["a.png", "b.png"]);
    assert!(h.wait_shown(&paths[1]));
    assert_eq!(h.app().index(), Some(1));
    assert_eq!(first_pixel(&h), decode_first_pixel(&paths[1]));
    assert!(has_text(&h.frame(), "2 / 2"));
}

#[test]
fn shift_delete_asks_then_removes() {
    let (_dir, paths) = folder("del-shift", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[1], 3);
    h.key(Key::Delete, SHIFT);
    // A question that names the file; nothing happened to it yet.
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    let list = h.frame();
    assert!(has_text(&list, "Delete permanently?"));
    assert!(has_text(&list, "b.png will be deleted for good"));
    assert!(has_text(&list, "Keep") && has_text(&list, "Delete"));
    assert!(paths[1].exists());
    assert_eq!(h.disk.removal_calls(), 0);

    // The focus starts on Keep; Right moves it to Delete, Enter confirms.
    h.key(Key::Right, NONE);
    h.key(Key::Enter, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(!paths[1].exists());
    assert!(
        !in_fake_trash(&paths[1]).exists(),
        "removed for good, not trashed"
    );
    assert_eq!(h.disk.removals(), [("remove_file", "b.png".to_string())]);
    assert_eq!(h.app().count(), 2);
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert!(h.wait_shown(&paths[2]));
    assert!(has_text(&h.frame(), "Deleted b.png"));
}

#[test]
fn shift_delete_cancel_keeps_file() {
    let (_dir, paths) = folder("del-keep", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[1], 3);

    // Enter on the default button keeps the file.
    h.key(Key::Delete, SHIFT);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    h.key(Key::Enter, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    h.pump();
    assert!(!h.app().deleting());
    assert!(has_text(&h.frame(), "Kept b.png"));

    // Escape closes the question, keeps the file and does not quit.
    h.key(Key::Delete, SHIFT);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(!h.app().exit_requested(), "Escape closed the dialog only");
    h.pump();

    assert!(paths[1].exists());
    assert_eq!(h.disk.removal_calls(), 0);
    assert_eq!(h.app().count(), 3);
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    assert_eq!(h.app().index(), Some(1));
    // With no dialog open, Escape quits as before.
    h.key(Key::Escape, NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn delete_only_image_leaves_empty_window() {
    let (dir, paths) = folder("del-only", &["only.png"], 8, 8);
    let mut h = open(&paths[0], 1);
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(!paths[0].exists());
    // The empty window: no image, no target, no counter.
    assert_eq!(h.app().shown_path(), None);
    assert_eq!(h.app().target_path(), None);
    assert_eq!(h.app().loading_path(), None);
    assert_eq!(h.app().count(), 0);
    assert!(h.app().library().is_empty());
    assert_eq!(h.app().title(), "pandite");
    let list = h.frame();
    assert!(has_text(&list, "No image open"));
    assert_eq!(
        h.updates.last().map(|u| u.dims),
        Some(None),
        "the renderer drops the image"
    );

    // Keys still work: Delete does nothing, Ctrl+O asks for a file, F11
    // toggles full screen, Escape quits.
    h.key(Key::Delete, NONE);
    h.key(Key::Delete, SHIFT);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert_eq!(h.disk.removal_calls(), 1);
    h.key(Key::O, Modifiers::CTRL);
    h.pump();
    assert_eq!(lock(&h.dialogs).shown.len(), 1, "the open dialog showed");
    h.key(Key::F11, NONE);
    assert!(h.app().fullscreen());
    // A file opened now shows as usual.
    let next = dir.join("next.png");
    write_png(&next, &numbered(4, 4));
    h.app()
        .open(next.clone(), std::time::Duration::from_millis(5));
    assert!(h.wait_shown(&next));
    h.key(Key::Escape, NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn delete_failure_keeps_image() {
    let (_dir, paths) = folder("del-fail", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[1], 3);
    h.disk
        .fail_removal("b.png", io::ErrorKind::PermissionDenied);
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    // The file, the image and the list stay as they were; the notice says
    // why.
    assert!(paths[1].exists());
    assert_eq!(h.disk.removal_calls(), 1);
    assert!(h.disk.removals().is_empty());
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    assert_eq!(h.app().count(), 3);
    assert_eq!(h.app().index(), Some(1));
    assert_eq!(h.app().library().paths(), paths);
    assert_eq!(h.app().thumbnails().len(), 3);
    let notice = h.app().notice().expect("a notice").to_string();
    assert!(
        notice.starts_with("Could not move b.png to") && notice.ends_with("the test disk refuses"),
        "{notice}"
    );
    let list = h.frame();
    assert!(has_text(&list, "Could not move b.png"));
    assert!(has_text(&list, "2 / 3"));

    // Shift+Delete fails the same way.
    h.key(Key::Delete, SHIFT);
    h.key(Key::Right, NONE);
    h.key(Key::Enter, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(paths[1].exists());
    assert!(h
        .app()
        .notice()
        .is_some_and(|n| n.starts_with("Could not delete b.png")));
    assert_eq!(h.app().count(), 3);

    // A file that is already gone leaves the list, and the next one shows.
    h.disk.allow_removal("b.png");
    std::fs::remove_file(&paths[1]).unwrap();
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert_eq!(h.app().count(), 2);
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert!(h.wait_shown(&paths[2]));
    assert!(h.app().notice().is_none());
    assert!(has_text(&h.frame(), "b.png was already gone"));
}

#[test]
fn delete_before_listing_shows_neighbor() {
    let (dir, paths) = folder("del-early", &["a.png", "b.png", "c.png"], 8, 8);
    // The folder listing hangs until released: the library knows the
    // opened file alone.
    let disk = TestDisk::new();
    let folder_name = dir.path().file_name().unwrap().to_string_lossy();
    disk.stall_read(&folder_name);
    let mut h = Harness::with_disk(
        Config {
            open: Some(paths[1].clone()),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.wait_shown(&paths[1]));
    assert!(h.disk.wait_call("list_dir", &folder_name));
    assert_eq!(h.app().count(), 0, "no listing yet");
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(!paths[1].exists());
    // The folder is listed again without the file; its neighbor shows.
    h.disk.release_all();
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().count(), 2);
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert_eq!(h.app().index(), Some(1));
}

#[test]
fn delete_ignored_in_text_field() {
    let (_dir, paths) = folder("del-field", &["a.png", "b.png"], 8, 8);
    let mut h = open(&paths[0], 2);

    // The resize dialog opens with its first text field focused: Delete
    // and Shift+Delete edit the text there.
    h.key(Key::R, Modifiers::CTRL);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    assert!(h.app().ui().wants_text_input());
    h.key(Key::Home, NONE);
    h.key(Key::Delete, NONE);
    h.key(Key::Delete, SHIFT);
    assert_eq!(h.app().ui().mode(), UiMode::Modal, "the dialog stays open");
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);

    // Crop mode keeps the file too.
    h.key(Key::C, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Crop);
    h.key(Key::Delete, NONE);
    h.key(Key::Delete, SHIFT);
    assert_eq!(h.app().ui().mode(), UiMode::Crop);
    h.key(Key::C, NONE);

    // So does the keyboard shortcut overlay.
    h.key(Key::F1, NONE);
    h.key(Key::Delete, NONE);
    h.key(Key::Escape, NONE);

    h.pump();
    assert!(!h.app().deleting());
    assert_eq!(h.disk.removal_calls(), 0);
    assert!(paths[0].exists());
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
    assert_eq!(h.app().count(), 2);
    assert!(!h.app().exit_requested());
}

/// Why the real trash cannot be used here, when it cannot.
fn trash_unusable(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied | io::ErrorKind::Interrupted
    )
}

/// Send `path` to the real trash on a helper thread with a deadline, so a
/// system that asks a question instead of trashing cannot hang the run.
/// `Err` holds why the trash of this system is not usable here.
fn real_trash(path: &Path) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    let job = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(trash::trash(&job));
    });
    match rx.recv_timeout(Duration::from_secs(120)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) if trash_unusable(&err) => Err(format!(
            "{} cannot take {}: {err}",
            trash::TRASH_NAME,
            path.display()
        )),
        Ok(Err(err)) => panic!("moving {} to the trash failed: {err}", path.display()),
        Err(_) => Err(format!("{} did not answer in 120 s", trash::TRASH_NAME)),
    }
}

#[test]
fn trash_real_file_leaves_its_folder() {
    let no_home = std::env::var_os("HOME").is_none() && std::env::var_os("XDG_DATA_HOME").is_none();
    if cfg!(unix) && no_home {
        eprintln!("trash_real_file_leaves_its_folder: skipped, no HOME for a trash");
        return;
    }
    let dir = TempDir::new("real-trash");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let unique = format!("{}-{stamp}", std::process::id());
    // A plain name, and a name with letters outside ASCII.
    let files = [
        dir.join(&format!("pandite-trash-test-{unique}.txt")),
        dir.join(&format!(
            "pandite-trash-test-\u{e4}\u{3b1}\u{1F600}-{unique}.txt"
        )),
    ];
    for path in &files {
        std::fs::write(path, b"pandite trash test").unwrap();
        if let Err(why) = real_trash(path) {
            eprintln!("trash_real_file_leaves_its_folder: skipped, {why}");
            return;
        }
        assert!(!path.exists(), "{} left its folder", path.display());
        assert!(std::fs::symlink_metadata(path).is_err());
        // The item stays in the trash: the test neither empties nor lists
        // it.
        eprintln!(
            "trash_real_file_leaves_its_folder: {} is in {}",
            path.file_name().unwrap().to_string_lossy(),
            trash::TRASH_NAME
        );
    }
    // A path longer than the 260 characters of old Windows programs.
    let mut deep = dir.path().to_path_buf();
    while deep.as_os_str().len() < 300 {
        deep.push("pandite-deep-folder-0123456789");
    }
    std::fs::create_dir_all(&deep).unwrap();
    let long = deep.join(format!("pandite-trash-long-{unique}.txt"));
    std::fs::write(&long, b"pandite trash test").unwrap();
    match real_trash(&long) {
        Ok(()) => {
            assert!(std::fs::symlink_metadata(&long).is_err());
            eprintln!(
                "trash_real_file_leaves_its_folder: a {} character path is in {}",
                long.as_os_str().len(),
                trash::TRASH_NAME
            );
        }
        Err(why) => eprintln!("trash_real_file_leaves_its_folder: long path not trashed, {why}"),
    }
}

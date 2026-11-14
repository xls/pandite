//! The selection end to end: Space and the check boxes of the gallery
//! strip select files, the selection follows the files through deletes,
//! rescans and folder changes, Delete and Shift+Delete act on it after a
//! question, and bulk convert offers it as its source. A drive without a
//! trash takes Delete as a removal for good. These tests use the fake
//! trash of the test disk only.

mod support;

use pandite_app::app::Handled;
use pandite_app::trash::{TRASH_NAME, TRASH_TITLE};
use pandite_core::{DrawCmd, DrawList, Point, Rect};
use pandite_ui::{
    thumbnail_index, Action, BulkJob, BulkSource, InputEvent, Key, Modifiers, OverwritePolicy,
    PointerButton, UiMode,
};
use std::io;
use std::path::{Path, PathBuf};
use support::*;

const SHIFT: Modifiers = Modifiers::SHIFT;
const NONE: Modifiers = Modifiers::NONE;

/// A core on `path` with its folder listed and every thumbnail uploaded.
fn open(path: &Path, count: usize) -> Harness {
    open_on(path, count, TestDisk::new())
}

fn open_on(path: &Path, count: usize, disk: std::sync::Arc<TestDisk>) -> Harness {
    let mut h = Harness::with_disk(
        Config {
            open: Some(path.to_path_buf()),
            ..Config::default()
        },
        disk,
    );
    h.start();
    assert!(h.wait_shown(path), "{} did not open", path.display());
    assert!(h.wait_until(|a| a.count() == count));
    assert!(h.wait_thumbnails(count));
    h
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

fn names(paths: &[PathBuf]) -> Vec<String> {
    paths.iter().map(|p| name(p)).collect()
}

/// The selected files by name, sorted.
fn selected(h: &mut Harness) -> Vec<String> {
    let mut out = names(h.app().selection());
    out.sort();
    out
}

/// The files at the selected indices of the gallery, by name.
fn selected_in_list(h: &mut Harness) -> Vec<String> {
    let paths = h.app().library().paths();
    let indices = h.app().selected_indices().to_vec();
    indices.iter().map(|&i| name(&paths[i])).collect()
}

fn in_fake_trash(file: &Path) -> PathBuf {
    file.parent()
        .unwrap()
        .join(FAKE_TRASH)
        .join(file.file_name().unwrap())
}

/// The box of the text run that reads exactly `exact`.
fn text_rect(list: &DrawList, exact: &str) -> Option<Rect> {
    list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text {
            pos, text, size, ..
        } if text == exact => Some(Rect::new(
            pos.x,
            pos.y,
            text.chars().count() as f32 * size * 0.5,
            size * 1.25,
        )),
        _ => None,
    })
}

fn click_at(h: &mut Harness, pos: Point) {
    h.input(InputEvent::PointerMove { pos });
    h.input(InputEvent::PointerDown {
        pos,
        button: PointerButton::Left,
    });
    h.input(InputEvent::PointerUp {
        pos,
        button: PointerButton::Left,
    });
}

fn click_label(h: &mut Harness, label: &str) {
    let list = h.frame();
    let r = text_rect(&list, label).unwrap_or_else(|| panic!("{label:?} is not drawn"));
    click_at(h, r.center());
}

/// The cell where the thumbnail of `index` is drawn.
fn cell(h: &mut Harness, index: usize) -> Rect {
    let list = h.frame();
    list.cmds
        .iter()
        .find_map(|c| match c {
            DrawCmd::Image { rect, texture, .. } if thumbnail_index(*texture) == Some(index) => {
                Some(*rect)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("thumbnail {index} is not drawn"))
}

/// Every text of the frame, joined with spaces: wrapped lines read as
/// their paragraph again.
fn all_text(list: &DrawList) -> String {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The removal calls on the file `name`: trash checks, moves and removals.
fn removal_ops(h: &Harness, name: &str) -> Vec<&'static str> {
    h.disk
        .ops_on(name)
        .into_iter()
        .filter(|op| matches!(*op, "has_trash" | "trash" | "remove_file"))
        .collect()
}

/// Press `key` with `mods` and wait for the question it asks.
fn ask(h: &mut Harness, key: Key, mods: Modifiers) -> DrawList {
    h.key(key, mods);
    assert!(
        h.wait_until(|a| a.ui().mode() == UiMode::Modal),
        "no question"
    );
    h.frame()
}

/// Answer the open question with its confirming button.
fn confirm(h: &mut Harness) {
    let list = h.frame();
    let yes = ["Move", "Delete"]
        .into_iter()
        .find_map(|l| text_rect(&list, l))
        .expect("a confirming button");
    click_at(h, yes.center());
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(h.wait_until(|a| !a.deleting()));
}

#[test]
fn selection_toggle_adds_and_removes() {
    let (_dir, paths) = folder("sel-toggle", &["a.png", "b.png", "c.png", "d.png"], 8, 8);
    let mut h = open(&paths[1], 4);
    assert!(h.app().selection().is_empty());
    assert!(!has_text(&h.frame(), "selected"));

    // Space selects the image on screen.
    h.key(Key::Space, NONE);
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    assert_eq!(h.app().selected_indices(), [1]);
    assert!(has_text(&h.frame(), "1 selected"));
    // A check box click selects another image and does not show it.
    let d = cell(&mut h, 3);
    click_at(&mut h, Point::new(d.x + 6.0, d.y + 6.0));
    assert_eq!(selected(&mut h), ["b.png", "d.png"]);
    assert_eq!(h.app().selected_indices(), [1, 3]);
    assert_eq!(h.app().index(), Some(1), "the click did not navigate");
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    assert!(has_text(&h.frame(), "2 selected"));

    // The same key and the same box unselect again.
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["d.png"]);
    assert_eq!(h.app().selected_indices(), [3]);
    let d = cell(&mut h, 3);
    click_at(&mut h, Point::new(d.x + 6.0, d.y + 6.0));
    assert!(h.app().selection().is_empty());
    assert!(h.app().selected_indices().is_empty());
    assert!(!has_text(&h.frame(), "selected"));

    // Moving on keeps the selection; Space acts on the new image.
    h.key(Key::Space, NONE);
    h.key(Key::Right, NONE);
    assert!(h.wait_shown(&paths[2]));
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);
    assert_eq!(h.app().selected_indices(), [1, 2]);
    // An index that is not in the folder selects nothing.
    assert_eq!(h.act(Action::ToggleSelect(99)), Handled::Ignored);
    assert_eq!(h.app().selected_indices(), [1, 2]);
}

#[test]
fn selection_follows_files_after_delete() {
    let (dir, paths) = folder(
        "sel-follow",
        &["a.png", "b.png", "c.png", "d.png", "e.png", "f.png"],
        8,
        8,
    );
    let mut h = open(&paths[0], 6);
    for i in [2, 3, 4, 5] {
        h.act(Action::ToggleSelect(i));
    }
    assert_eq!(h.app().selected_indices(), [2, 3, 4, 5]);

    // d and e refuse to go: c and f leave the list, d and e stay selected
    // at their new places.
    h.disk
        .fail_removal("d.png", io::ErrorKind::PermissionDenied);
    h.disk
        .fail_removal("e.png", io::ErrorKind::PermissionDenied);
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert_eq!(
        names(&h.app().library().paths()),
        ["a.png", "b.png", "d.png", "e.png"]
    );
    assert_eq!(selected(&mut h), ["d.png", "e.png"]);
    assert_eq!(h.app().selected_indices(), [2, 3]);
    assert_eq!(selected_in_list(&mut h), ["d.png", "e.png"]);
    assert!(has_text(&h.frame(), "2 selected"));

    // A rescan: b and d leave the disk and a new file comes. The new list
    // keeps e selected by its name; d left the selection with its file.
    std::fs::remove_file(&paths[1]).unwrap();
    std::fs::remove_file(&paths[3]).unwrap();
    h.act(Action::RunBulk(BulkJob {
        source: BulkSource::Files(vec![paths[0].clone()]),
        resize: None,
        format: Some("jpeg".into()),
        quality: 80,
        destination: dir.path().to_path_buf(),
        overwrite: OverwritePolicy::Skip,
    }));
    assert!(h.wait_until(|a| !a.bulk_running() && a.count() == 3));
    assert_eq!(
        names(&h.app().library().paths()),
        ["a.jpg", "a.png", "e.png"]
    );
    assert_eq!(h.app().selection(), [paths[4].clone()]);
    assert_eq!(h.app().selected_indices(), [2]);
    assert_eq!(selected_in_list(&mut h), ["e.png"]);
    // The image on screen kept its own place by name too.
    assert_eq!(h.app().target_path(), Some(paths[0].as_path()));
    assert_eq!(h.app().index(), Some(1));
}

#[test]
fn selection_cleared_on_folder_change() {
    let (_dir, paths) = folder("sel-folder", &["a.png", "b.png", "c.png"], 8, 8);
    let (_other_dir, other) = folder("sel-other", &["x.png", "y.png"], 8, 8);
    let mut h = open(&paths[0], 3);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(2));
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);

    // Another file of the same folder keeps the selection.
    let now = h.now;
    h.app().open(paths[2].clone(), now);
    assert!(h.wait_shown(&paths[2]));
    assert!(h.wait_until(|a| a.count() == 3 && a.selected_indices() == [1, 2]));
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);

    // A file of another folder starts with nothing selected.
    h.app().open(other[0].clone(), now);
    assert!(h.app().selection().is_empty(), "cleared at once");
    assert!(h.app().selected_indices().is_empty());
    assert!(h.wait_shown(&other[0]));
    assert!(h.wait_until(|a| a.count() == 2));
    assert!(h.app().selection().is_empty());
    assert!(!has_text(&h.frame(), "selected"));
    // Coming back does not bring the old selection back.
    h.app().open(paths[0].clone(), now);
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    assert!(h.app().selection().is_empty());
    // Space works in the new folder as before.
    h.key(Key::Space, NONE);
    assert_eq!(h.app().selection(), [paths[0].clone()]);
}

#[test]
fn selection_clear_command() {
    let (_dir, paths) = folder("sel-clear", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[1], 3);
    for i in 0..3 {
        h.act(Action::ToggleSelect(i));
    }
    assert!(has_text(&h.frame(), "3 selected"));

    // Backspace clears the whole selection and stays on the image.
    h.key(Key::Backspace, NONE);
    assert!(h.app().selection().is_empty());
    assert!(h.app().selected_indices().is_empty());
    assert!(!has_text(&h.frame(), "selected"));
    assert_eq!(h.app().index(), Some(1));
    // Without a selection Backspace does nothing, and never goes back.
    h.key(Key::Backspace, NONE);
    h.pump();
    assert_eq!(h.app().index(), Some(1));
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    assert_eq!(h.act(Action::ClearSelection), Handled::Ignored);

    // "Clear selection" in the context menu does the same.
    h.act(Action::ToggleSelect(0));
    h.act(Action::ToggleSelect(2));
    let p = Point::new(500.0, 200.0);
    h.input(InputEvent::PointerMove { pos: p });
    h.input(InputEvent::PointerDown {
        pos: p,
        button: PointerButton::Right,
    });
    h.input(InputEvent::PointerUp {
        pos: p,
        button: PointerButton::Right,
    });
    click_label(&mut h, "Clear selection");
    assert!(h.app().selection().is_empty());
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(!h.app().exit_requested());
}

#[test]
fn delete_selection_asks_with_count() {
    let (_dir, paths) = folder("sel-ask", &["a.png", "b.png", "c.png", "d.png"], 8, 8);
    let mut h = open(&paths[0], 4);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(3));

    // Delete asks with the count and names the files; nothing went yet.
    let list = ask(&mut h, Key::Delete, NONE);
    assert!(has_text(&list, &format!("Move 2 images to {TRASH_NAME}?")));
    assert!(has_text(&list, "b.png and d.png will move to"));
    assert!(text_rect(&list, "Move").is_some() && text_rect(&list, "Cancel").is_some());
    assert_eq!(h.disk.removal_calls(), 0);
    assert!(paths.iter().all(|p| p.exists()));
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);

    // One file: the singular.
    h.act(Action::ToggleSelect(3));
    let list = ask(&mut h, Key::Delete, NONE);
    assert!(has_text(&list, &format!("Move 1 image to {TRASH_NAME}?")));
    assert!(has_text(&list, "b.png will move to"));
    h.key(Key::Escape, NONE);

    // Shift+Delete asks for the removal for good, Keep first.
    let list = ask(&mut h, Key::Delete, SHIFT);
    assert!(has_text(&list, "Delete 1 image permanently?"));
    assert!(has_text(&list, "b.png will be deleted for good"));
    assert!(text_rect(&list, "Keep").is_some() && text_rect(&list, "Delete").is_some());
    h.key(Key::Enter, NONE);
    h.pump();
    assert!(has_text(&h.frame(), "Kept 1 image"));
    assert_eq!(h.disk.removal_calls(), 0);
    assert!(paths.iter().all(|p| p.exists()));
}

#[test]
fn delete_selection_trashes_exactly_selected() {
    let (_dir, paths) = folder(
        "sel-trash",
        &["a.png", "b.png", "c.png", "d.png", "e.png"],
        8,
        8,
    );
    let bytes: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let mut h = open(&paths[0], 5);
    // The image on screen (a) is not selected.
    for i in [1, 3, 4] {
        h.act(Action::ToggleSelect(i));
    }
    ask(&mut h, Key::Delete, NONE);
    // Move is the default answer.
    h.key(Key::Enter, NONE);
    assert!(h.wait_until(|a| !a.deleting()));

    // Exactly the three selected files went to the trash, whole, in folder
    // order; nothing was removed for good.
    assert_eq!(
        h.disk.removals(),
        [
            ("trash", "b.png".to_string()),
            ("trash", "d.png".to_string()),
            ("trash", "e.png".to_string())
        ]
    );
    assert_eq!(h.disk.removal_calls(), 3);
    for i in [1, 3, 4] {
        assert!(!paths[i].exists());
        assert_eq!(std::fs::read(in_fake_trash(&paths[i])).unwrap(), bytes[i]);
    }
    assert!(paths[0].exists() && paths[2].exists());
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert_eq!(h.app().count(), 2);
    assert!(h.app().selection().is_empty());
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
    let list = h.frame();
    assert!(has_text(&list, &format!("Moved 3 images to {TRASH_NAME}")));
    assert!(has_text(&list, "1 / 2"));
    assert!(h.app().notice().is_none());
}

#[test]
fn delete_selection_cancel_keeps_all() {
    let (_dir, paths) = folder("sel-cancel", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[0], 3);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(2));

    // Cancel, by a click.
    ask(&mut h, Key::Delete, NONE);
    click_label(&mut h, "Cancel");
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    h.pump();
    assert!(has_text(&h.frame(), "Kept 2 images"));
    // No, by the keys: Left moves to Cancel, Enter answers.
    ask(&mut h, Key::Delete, NONE);
    h.key(Key::Left, NONE);
    h.key(Key::Enter, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    // Escape closes the question and does not quit.
    ask(&mut h, Key::Delete, NONE);
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    // Shift+Delete: Enter on the default Keep, then Escape.
    ask(&mut h, Key::Delete, SHIFT);
    h.key(Key::Enter, NONE);
    ask(&mut h, Key::Delete, SHIFT);
    h.key(Key::Escape, NONE);
    h.pump();

    assert!(!h.app().exit_requested(), "Escape closed the question only");
    assert!(!h.app().deleting());
    assert_eq!(h.disk.removal_calls(), 0);
    assert!(paths.iter().all(|p| p.exists()));
    assert_eq!(h.app().count(), 3);
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
}

#[test]
fn shift_delete_selection_removes_exactly_selected() {
    let (_dir, paths) = folder("sel-shift", &["a.png", "b.png", "c.png", "d.png"], 8, 8);
    let mut h = open(&paths[0], 4);
    // The image on screen is one of them.
    h.key(Key::Space, NONE);
    h.act(Action::ToggleSelect(2));

    let list = ask(&mut h, Key::Delete, SHIFT);
    assert!(has_text(&list, "Delete 2 images permanently?"));
    assert!(has_text(&list, "a.png and c.png will be deleted for good"));
    // Keep has the focus: Right moves to Delete, Enter confirms.
    h.key(Key::Right, NONE);
    h.key(Key::Enter, NONE);
    assert!(h.wait_until(|a| !a.deleting()));

    // Removed for good, the image on screen last; nothing went to the
    // trash.
    assert_eq!(
        h.disk.removals(),
        [
            ("remove_file", "c.png".to_string()),
            ("remove_file", "a.png".to_string())
        ]
    );
    assert_eq!(h.disk.removal_calls(), 2);
    for i in [0, 2] {
        assert!(!paths[i].exists());
        assert!(!in_fake_trash(&paths[i]).exists());
    }
    assert!(paths[1].exists() && paths[3].exists());
    assert_eq!(names(&h.app().library().paths()), ["b.png", "d.png"]);
    assert!(h.app().selection().is_empty());
    assert!(h.wait_shown(&paths[1]));
    assert!(has_text(&h.frame(), "Deleted 2 images"));
}

#[test]
fn delete_selection_moves_view_to_remaining() {
    let six = ["a.png", "b.png", "c.png", "d.png", "e.png", "f.png"];

    // The image on screen stays when it is not selected.
    let (_dir, paths) = folder("sel-view-stay", &six, 8, 8);
    let mut h = open(&paths[2], 6);
    for i in [0, 4] {
        h.act(Action::ToggleSelect(i));
    }
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert_eq!(
        names(&h.app().library().paths()),
        ["b.png", "c.png", "d.png", "f.png"]
    );
    assert_eq!(h.app().shown_path(), Some(paths[2].as_path()));
    assert_eq!(h.app().index(), Some(1));
    assert!(has_text(&h.frame(), "2 / 4"));
    drop(h);

    // Selected with its next neighbor: the first remaining file after it.
    let (_dir, paths) = folder("sel-view-next", &six, 8, 8);
    let mut h = open(&paths[2], 6);
    for i in [2, 3] {
        h.act(Action::ToggleSelect(i));
    }
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert_eq!(
        names(&h.app().library().paths()),
        ["a.png", "b.png", "e.png", "f.png"]
    );
    assert!(h.wait_shown(&paths[4]));
    assert_eq!(h.app().index(), Some(2));
    drop(h);

    // Selected with every file after it: the last remaining file.
    let (_dir, paths) = folder("sel-view-last", &six, 8, 8);
    let mut h = open(&paths[3], 6);
    for i in [3, 4, 5] {
        h.act(Action::ToggleSelect(i));
    }
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().index(), Some(2));
    assert_eq!(h.app().count(), 3);
    drop(h);

    // Everything selected: the empty window.
    let (_dir, paths) = folder("sel-view-all", &six[..3], 8, 8);
    let mut h = open(&paths[1], 3);
    for i in 0..3 {
        h.act(Action::ToggleSelect(i));
    }
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert_eq!(h.app().count(), 0);
    assert_eq!(h.app().shown_path(), None);
    assert_eq!(h.app().target_path(), None);
    assert!(has_text(&h.frame(), "No image open"));
}

#[test]
fn delete_selection_partial_failure_reported() {
    let (_dir, paths) = folder(
        "sel-fail",
        &["a.png", "b.png", "c.png", "d.png", "e.png"],
        8,
        8,
    );
    let mut h = open(&paths[0], 5);
    for i in [1, 2, 3] {
        h.act(Action::ToggleSelect(i));
    }
    h.disk
        .fail_removal("c.png", io::ErrorKind::PermissionDenied);
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);

    // b and d went; c stays, still selected, and the message says why.
    assert_eq!(
        h.disk.removals(),
        [
            ("trash", "b.png".to_string()),
            ("trash", "d.png".to_string())
        ]
    );
    assert_eq!(h.disk.removal_calls(), 3, "c was tried too");
    assert!(paths[2].exists());
    assert_eq!(
        names(&h.app().library().paths()),
        ["a.png", "c.png", "e.png"]
    );
    assert_eq!(h.app().selection(), [paths[2].clone()]);
    assert_eq!(h.app().selected_indices(), [1]);
    let notice = h.app().notice().expect("a notice").to_string();
    assert_eq!(
        notice,
        format!("Could not move c.png to {TRASH_NAME}: the test disk refuses")
    );
    let list = h.frame();
    assert!(has_text(&list, "Could not move c.png"));
    assert!(has_text(&list, &format!("Moved 2 images to {TRASH_NAME}")));
    assert!(has_text(&list, "1 selected"));
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));

    // Two failures: the first with its reason, and how many more.
    h.act(Action::ToggleSelect(2));
    h.disk
        .fail_removal("e.png", io::ErrorKind::PermissionDenied);
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    let notice = h.app().notice().expect("a notice").to_string();
    assert!(
        notice.starts_with("Could not move c.png") && notice.ends_with("1 more file failed."),
        "{notice}"
    );
    assert_eq!(selected(&mut h), ["c.png", "e.png"]);
    assert_eq!(h.app().count(), 3);

    // The image on screen fails: it stays on screen and selected.
    h.disk.allow_removal("c.png");
    h.disk.allow_removal("e.png");
    h.act(Action::ClearSelection);
    h.key(Key::Space, NONE);
    h.disk
        .fail_removal("a.png", io::ErrorKind::PermissionDenied);
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert!(paths[0].exists());
    assert_eq!(h.app().selection(), [paths[0].clone()]);
    assert!(h.wait_shown(&paths[0]));
    assert_eq!(h.app().index(), Some(0));
}

#[test]
fn delete_without_selection_unchanged() {
    let (_dir, paths) = folder("sel-none", &["a.png", "b.png", "c.png", "d.png"], 8, 8);
    let mut h = open(&paths[1], 4);

    // A selection made and cleared again leaves Delete as it was: the file
    // on screen goes to the trash at once, without a question.
    h.act(Action::ToggleSelect(3));
    h.key(Key::Backspace, NONE);
    h.key(Key::Delete, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal, "no question");
    assert!(h.wait_until(|a| !a.deleting()));
    assert_eq!(h.disk.removals(), [("trash", "b.png".to_string())]);
    assert!(in_fake_trash(&paths[1]).exists());
    assert!(h.wait_shown(&paths[2]));
    assert!(has_text(
        &h.frame(),
        &format!("Moved b.png to {TRASH_NAME}")
    ));

    // Shift+Delete asks for the one file, as it did.
    let list = ask(&mut h, Key::Delete, SHIFT);
    assert!(has_text(&list, "Delete permanently?"));
    assert!(has_text(&list, "c.png will be deleted for good"));
    h.key(Key::Right, NONE);
    h.key(Key::Enter, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert_eq!(
        h.disk.removals(),
        [
            ("trash", "b.png".to_string()),
            ("remove_file", "c.png".to_string())
        ]
    );
    assert!(h.wait_shown(&paths[3]));
    assert_eq!(names(&h.app().library().paths()), ["a.png", "d.png"]);
}

#[test]
fn bulk_uses_selection_when_present() {
    let (_dir, paths) = folder("sel-bulk", &["a.png", "b.png", "c.png", "d.png"], 40, 20);
    let mut h = open(&paths[0], 4);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(3));

    // The dialog offers the selected images and has them chosen.
    h.key(Key::B, Modifiers::CTRL);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    let list = h.frame();
    assert!(text_rect(&list, "Selected images (2)").is_some());
    assert!(has_text(&list, "2 selected images"));
    assert!(has_text(&list, "Ready."), "the job is complete as it opens");
    assert!(has_text(&list, "converted"), "the destination is suggested");

    // Another source can still be picked, and the selection again.
    click_label(&mut h, "Current folder");
    let list = h.frame();
    assert!(has_text(&list, "Folder:"));
    assert!(!has_text(&list, "2 selected images"));
    click_label(&mut h, "Selected images (2)");
    assert!(has_text(&h.frame(), "2 selected images"));
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);

    // Opened again, the dialog follows the selection as it is now.
    h.act(Action::ToggleSelect(2));
    h.key(Key::B, Modifiers::CTRL);
    let list = h.frame();
    assert!(text_rect(&list, "Selected images (3)").is_some());
    assert!(has_text(&list, "3 selected images"));
    h.key(Key::Escape, NONE);
}

#[test]
fn bulk_selection_job_processes_exactly_selected() {
    let (dir, paths) = folder(
        "sel-bulk-run",
        &["a.png", "b.png", "c.png", "d.png"],
        40,
        20,
    );
    let mut h = open(&paths[0], 4);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(3));
    h.key(Key::B, Modifiers::CTRL);
    // Fit 50 x 50 keeps the size; the format stays PNG.
    h.frame();
    h.key(Key::Enter, NONE);
    assert!(h.app().bulk_running(), "Enter ran the job");
    // The dialog shows the progress while the job runs.
    assert!(has_text(&h.frame(), "Bulk convert"));
    assert!(h.wait_until(|a| !a.bulk_running()));

    let out = dir.join("converted");
    let mut written: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    written.sort();
    assert_eq!(written, ["b.png", "d.png"], "exactly the selected files");
    for n in ["b.png", "d.png"] {
        let (img, id) = decode(&out.join(n));
        assert_eq!((id, img.width(), img.height()), ("png", 40, 20));
    }
    assert!(has_text(&h.frame(), "2 saved"));
    // The sources and the selection stay as they were.
    assert!(paths.iter().all(|p| p.exists()));
    assert_eq!(selected(&mut h), ["b.png", "d.png"]);
    assert_eq!(h.app().ui().mode(), UiMode::Normal, "the dialog closed");
}

#[test]
fn bulk_without_selection_unchanged() {
    let (_dir, paths) = folder("sel-bulk-none", &["a.png", "b.png"], 8, 8);
    let mut h = open(&paths[0], 2);

    let check_plain = |h: &mut Harness| {
        h.key(Key::B, Modifiers::CTRL);
        assert_eq!(h.app().ui().mode(), UiMode::Modal);
        let list = h.frame();
        assert!(has_text(&list, "No images chosen"));
        assert!(has_text(&list, "Choose images or a folder."));
        assert!(!has_text(&list, "Selected images") && !has_text(&list, "selected image"));
        for label in ["Add files...", "Add folder...", "Current folder"] {
            assert!(text_rect(&list, label).is_some(), "{label}");
        }
        h.key(Key::Escape, NONE);
        assert_eq!(h.app().ui().mode(), UiMode::Normal);
    };
    check_plain(&mut h);

    // A selection chosen once and cleared leaves no trace in the dialog.
    h.act(Action::ToggleSelect(1));
    h.key(Key::B, Modifiers::CTRL);
    assert!(has_text(&h.frame(), "1 selected image"));
    h.key(Key::Escape, NONE);
    h.key(Key::Backspace, NONE);
    check_plain(&mut h);

    // A selection source sent without a selection converts nothing.
    let result = h.act(Action::RunBulk(BulkJob {
        source: BulkSource::Selection,
        resize: None,
        format: None,
        quality: 90,
        destination: paths[0].parent().unwrap().join("out"),
        overwrite: OverwritePolicy::Skip,
    }));
    assert_eq!(result, Handled::Ignored);
    assert!(!h.app().bulk_running());
}

#[test]
fn delete_without_trash_removes_file_silently() {
    let (_dir, paths) = folder("no-trash", &["a.png", "b.png", "c.png"], 8, 8);
    let disk = TestDisk::new();
    for n in ["a.png", "b.png", "c.png"] {
        disk.no_trash(n);
    }
    let mut h = open_on(&paths[1], 3, disk);

    h.key(Key::Delete, NONE);
    // No question, no error: the file goes for good, at once.
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(!paths[1].exists());
    assert!(!in_fake_trash(&paths[1]).exists());
    assert_eq!(h.disk.removals(), [("remove_file", "b.png".to_string())]);
    // The trash was asked first and refused before it touched the file.
    assert_eq!(removal_ops(&h, "b.png"), ["trash", "remove_file"]);
    assert!(h.app().notice().is_none());
    // The view moves on as after a move to the trash.
    assert_eq!(names(&h.app().library().paths()), ["a.png", "c.png"]);
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().index(), Some(1));
    let list = h.frame();
    assert!(has_text(&list, "Deleted b.png"));
    assert!(!has_text(&list, "Could not"));
    assert!(has_text(&list, "2 / 2"));
}

#[test]
fn delete_without_trash_real_failure_still_reported() {
    let (_dir, paths) = folder("no-trash-fail", &["a.png", "b.png", "c.png"], 8, 8);
    let disk = TestDisk::new();
    disk.no_trash("b.png");
    disk.no_trash("c.png");
    let mut h = open_on(&paths[1], 3, disk);

    // The removal for good is refused: the file, the image and the list
    // stay, and the message says why.
    h.disk
        .fail_removal("b.png", io::ErrorKind::PermissionDenied);
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert!(paths[1].exists());
    assert!(h.disk.removals().is_empty());
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    assert_eq!(h.app().count(), 3);
    assert_eq!(
        h.app().notice(),
        Some("Could not delete b.png: the test disk refuses")
    );
    assert!(has_text(&h.frame(), "Could not delete b.png"));

    // A file with a trash keeps the message it had.
    h.disk
        .fail_removal("a.png", io::ErrorKind::PermissionDenied);
    h.key(Key::Left, NONE);
    assert!(h.wait_shown(&paths[0]));
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert_eq!(
        h.app().notice(),
        Some(format!("Could not move a.png to {TRASH_NAME}: the test disk refuses").as_str())
    );
    assert!(paths[0].exists());

    // A file that is gone already leaves the list, as before.
    std::fs::remove_file(&paths[2]).unwrap();
    h.act(Action::GoTo(2));
    h.key(Key::Delete, NONE);
    assert!(h.wait_until(|a| !a.deleting()));
    assert_eq!(names(&h.app().library().paths()), ["a.png", "b.png"]);
    assert!(has_text(&h.frame(), "c.png was already gone"));
}

#[test]
fn delete_selection_without_trash_asks_permanent() {
    let (_dir, paths) = folder("no-trash-sel", &["a.png", "b.png", "c.png", "d.png"], 8, 8);
    let disk = TestDisk::new();
    for n in ["a.png", "b.png", "c.png", "d.png"] {
        disk.no_trash(n);
    }
    let mut h = open_on(&paths[0], 4, disk);
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(2));

    // Delete still asks, for several files, but in the words of a removal
    // for good: there is no trash to promise.
    let list = ask(&mut h, Key::Delete, NONE);
    let text = all_text(&list);
    assert!(has_text(&list, "Delete 2 images permanently?"));
    assert!(
        text.contains(&format!(
            "b.png and c.png will be deleted for good. Their drive has no {TRASH_TITLE}. \
             This cannot be undone."
        )),
        "{text}"
    );
    assert!(!text.contains(TRASH_NAME) && !text.contains("will move"));
    assert!(text_rect(&list, "Move").is_none());
    assert!(text_rect(&list, "Keep").is_some() && text_rect(&list, "Delete").is_some());
    // One question to the disk for the folder; nothing removed yet.
    assert_eq!(removal_ops(&h, "b.png"), ["has_trash"]);
    assert!(removal_ops(&h, "c.png").is_empty());
    assert_eq!(h.disk.removal_calls(), 0);
    // Keep is the default answer.
    h.key(Key::Enter, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    h.pump();
    assert!(paths.iter().all(|p| p.exists()));

    // Confirmed, the files go for good, never through the trash.
    ask(&mut h, Key::Delete, NONE);
    confirm(&mut h);
    assert_eq!(
        h.disk.removals(),
        [
            ("remove_file", "b.png".to_string()),
            ("remove_file", "c.png".to_string())
        ]
    );
    assert_eq!(
        removal_ops(&h, "b.png"),
        ["has_trash", "has_trash", "remove_file"]
    );
    assert_eq!(removal_ops(&h, "c.png"), ["remove_file"]);
    assert!(!paths[1].exists() && !paths[2].exists());
    assert_eq!(names(&h.app().library().paths()), ["a.png", "d.png"]);
    assert!(h.app().selection().is_empty());
    let list = h.frame();
    assert!(has_text(&list, "Deleted 2 images"));
    assert!(!has_text(&list, "Could not"));
}

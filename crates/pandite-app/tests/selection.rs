//! The selection end to end: Space, Shift+Left, Shift+Right and the menu
//! select files (a click on a thumbnail never does), the selection follows
//! the files through deletes,
//! rescans and folder changes, Delete and Shift+Delete act on it after a
//! question, bulk convert offers it as its source, and Move to moves it
//! into another folder. Shift+Left and Shift+Right select as they move.
//! A drive without a trash takes Delete as a removal for good. These tests
//! use the fake trash of the test disk only.

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
    // The menu entry selects too.
    h.act(Action::ToggleSelect(3));
    assert_eq!(selected(&mut h), ["b.png", "d.png"]);
    assert_eq!(h.app().selected_indices(), [1, 3]);
    assert!(has_text(&h.frame(), "2 selected"));

    // A click on the corner of a thumbnail, its badge or not, shows that
    // image and leaves the selection as it is.
    let c = cell(&mut h, 2);
    click_at(&mut h, Point::new(c.x + 6.0, c.y + 6.0));
    assert_eq!(h.app().index(), Some(2));
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(selected(&mut h), ["b.png", "d.png"]);
    let d = cell(&mut h, 3);
    click_at(&mut h, Point::new(d.x + 6.0, d.y + 6.0));
    assert_eq!(h.app().index(), Some(3));
    assert!(h.wait_shown(&paths[3]));
    assert_eq!(selected(&mut h), ["b.png", "d.png"]);

    // Space on a selected image unselects it.
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["b.png"]);
    assert_eq!(h.app().selected_indices(), [1]);
    h.act(Action::ToggleSelect(1));
    assert!(h.app().selection().is_empty());
    assert!(h.app().selected_indices().is_empty());
    assert!(!has_text(&h.frame(), "selected"));

    // Moving on keeps the selection; Space acts on the new image.
    h.key(Key::Space, NONE);
    h.key(Key::Left, NONE);
    assert!(h.wait_shown(&paths[2]));
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["c.png", "d.png"]);
    assert_eq!(h.app().selected_indices(), [2, 3]);
    // An index that is not in the folder selects nothing.
    assert_eq!(h.act(Action::ToggleSelect(99)), Handled::Ignored);
    assert_eq!(h.app().selected_indices(), [2, 3]);
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

#[test]
fn selection_select_does_not_toggle() {
    let (_dir, paths) = folder("sel-select", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[0], 3);
    // Select adds an image and keeps it, however often it comes.
    assert_eq!(h.act(Action::Select(1)), Handled::Done);
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    for _ in 0..3 {
        h.act(Action::Select(1));
    }
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    assert_eq!(h.app().selected_indices(), [1]);
    h.act(Action::Select(2));
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);
    // An index that is not in the folder selects nothing.
    assert_eq!(h.act(Action::Select(9)), Handled::Ignored);
    assert_eq!(h.app().selected_indices(), [1, 2]);
    // Space still toggles.
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["a.png", "b.png", "c.png"]);
    h.key(Key::Space, NONE);
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);
    // Shift+Left on a selected image keeps it selected.
    h.act(Action::GoTo(2));
    assert!(h.wait_shown(&paths[2]));
    h.key(Key::Left, SHIFT);
    assert_eq!(selected(&mut h), ["b.png", "c.png"]);
    assert_eq!(h.app().index(), Some(1));
}

#[test]
fn shift_right_selects_run_of_images() {
    let names = ["a.png", "b.png", "c.png", "d.png", "e.png"];
    let (_dir, paths) = folder("sel-run", &names, 8, 8);
    let mut h = open(&paths[0], 5);
    for _ in 0..3 {
        h.key(Key::Right, SHIFT);
    }
    // Images 0, 1 and 2 are selected; image 3 is on screen, not selected.
    assert_eq!(h.app().selected_indices(), [0, 1, 2]);
    assert_eq!(selected(&mut h), ["a.png", "b.png", "c.png"]);
    assert_eq!(h.app().index(), Some(3));
    assert!(h.wait_shown(&paths[3]));
    assert!(!h.app().selection().contains(&paths[3]));
    assert!(has_text(&h.frame(), "3 selected"));
    // A held key goes on the same way.
    h.input(InputEvent::KeyDown {
        key: Key::Right,
        mods: SHIFT,
        repeat: true,
    });
    assert_eq!(h.app().selected_indices(), [0, 1, 2, 3]);
    assert_eq!(h.app().index(), Some(4));
    // Shift+Left selects back the other way.
    h.key(Key::Left, SHIFT);
    assert_eq!(h.app().selected_indices(), [0, 1, 2, 3, 4]);
    assert_eq!(h.app().index(), Some(3));
}

#[test]
fn shift_arrow_at_folder_end_still_selects() {
    // Wrapping is on (the default): a selecting move never wraps.
    let (_dir, paths) = folder("sel-end", &["a.png", "b.png", "c.png"], 8, 8);
    let mut h = open(&paths[2], 3);
    h.key(Key::Right, SHIFT);
    assert_eq!(h.app().selected_indices(), [2]);
    assert_eq!(h.app().index(), Some(2), "no move past the last image");
    h.key(Key::Right, SHIFT);
    assert_eq!(h.app().selected_indices(), [2]);
    assert_eq!(h.app().index(), Some(2));
    h.act(Action::GoTo(0));
    assert!(h.wait_shown(&paths[0]));
    h.key(Key::Left, SHIFT);
    assert_eq!(h.app().selected_indices(), [0, 2]);
    assert_eq!(h.app().index(), Some(0), "no move before the first image");
    // A folder of one image.
    let (_dir, one) = folder("sel-end-one", &["only.png"], 8, 8);
    let mut h = open(&one[0], 1);
    h.key(Key::Right, SHIFT);
    assert_eq!(h.app().selection(), [one[0].clone()]);
    assert_eq!(h.app().index(), Some(0));
}

/// A harness on `paths[start]` of a folder and a second folder to move
/// into.
fn move_setup(
    tag: &str,
    names: &[&str],
    start: usize,
) -> (TempDir, Vec<PathBuf>, TempDir, Harness) {
    let (dir, paths) = folder(tag, names, 8, 8);
    let out = TempDir::new(&format!("{tag}-to"));
    let h = open(&paths[start], names.len());
    (dir, paths, out, h)
}

/// Press M, pick `folder` in the picker, and wait for the move to end.
fn move_to(h: &mut Harness, folder: &Path) {
    h.answer_dialog(vec![folder.to_path_buf()]);
    h.key(Key::M, NONE);
    assert!(h.wait_until(|a| !a.moving()), "the move ended");
}

fn listing(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

#[test]
fn move_selection_moves_exactly_selected() {
    let names = ["a.png", "b.png", "c.png", "d.png", "e.png"];
    let (dir, paths, out, mut h) = move_setup("mv-sel", &names, 0);
    let bytes: Vec<Vec<u8>> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    for i in [1, 3, 4] {
        h.act(Action::ToggleSelect(i));
    }
    move_to(&mut h, out.path());
    // Exactly the selected files moved, whole; no question was asked.
    assert_eq!(listing(out.path()), ["b.png", "d.png", "e.png"]);
    for i in [1, 3, 4] {
        assert!(!paths[i].exists());
        assert_eq!(std::fs::read(out.join(names[i])).unwrap(), bytes[i]);
    }
    assert_eq!(listing(dir.path()), ["a.png", "c.png"]);
    assert_eq!(h.disk.removal_calls(), 0, "nothing was deleted");
    assert_eq!(names_of(&h), ["a.png", "c.png"]);
    assert!(h.app().selection().is_empty());
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    let folder_name = name(out.path());
    assert!(has_text(
        &h.frame(),
        &format!("Moved 3 images to {folder_name}")
    ));
}

fn names_of(h: &Harness) -> Vec<String> {
    names(&h.app.as_ref().unwrap().library().paths())
}

#[test]
fn move_without_selection_moves_viewed_image() {
    let (_dir, paths, out, mut h) = move_setup("mv-one", &["a.png", "b.png", "c.png"], 1);
    move_to(&mut h, out.path());
    assert_eq!(listing(out.path()), ["b.png"]);
    assert!(!paths[1].exists());
    assert_eq!(names_of(&h), ["a.png", "c.png"]);
    // The next image takes its place, as after a delete.
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().index(), Some(1));
    // The picker named one image and started in the folder on screen.
    let shown = lock(&h.dialogs).shown.clone();
    match shown.last() {
        Some(pandite_app::platform::DialogKind::PickFolder { start_dir, title }) => {
            assert_eq!(title, "Move 1 image to");
            assert_eq!(start_dir.as_deref(), paths[0].parent());
        }
        other => panic!("expected a folder picker, got {other:?}"),
    }
}

#[test]
fn move_cancelled_picker_changes_nothing() {
    let (dir, paths, out, mut h) = move_setup("mv-cancel", &["a.png", "b.png", "c.png"], 0);
    h.act(Action::ToggleSelect(1));
    // An empty answer is a cancel.
    h.answer_dialog(Vec::new());
    h.key(Key::M, NONE);
    h.pump();
    assert!(!h.app().moving());
    assert_eq!(lock(&h.dialogs).shown.len(), 1, "the picker showed");
    assert!(listing(out.path()).is_empty());
    assert_eq!(listing(dir.path()), ["a.png", "b.png", "c.png"]);
    assert_eq!(names_of(&h), ["a.png", "b.png", "c.png"]);
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    assert_eq!(h.app().settings().last_move_dir, None);
    assert!(!h.app().exit_requested());
}

#[test]
fn move_never_overwrites_existing_name() {
    let (_dir, paths, out, mut h) = move_setup("mv-exists", &["a.png", "b.png", "c.png"], 0);
    std::fs::write(out.join("b.png"), b"already there").unwrap();
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(2));
    move_to(&mut h, out.path());
    // c moved; b stayed where it was, and the file there is untouched.
    assert_eq!(std::fs::read(out.join("b.png")).unwrap(), b"already there");
    assert!(paths[1].exists());
    assert!(out.join("c.png").exists() && !paths[2].exists());
    assert_eq!(names_of(&h), ["a.png", "b.png"]);
    assert_eq!(h.app().selection(), [paths[1].clone()], "b stays selected");
    let folder_name = name(out.path());
    let notice = h.app().notice().expect("the report").to_string();
    assert_eq!(
        notice,
        format!("Did not move b.png: {folder_name} has a file of that name")
    );
    let list = h.frame();
    assert!(has_text(&list, &format!("Moved 1 image to {folder_name}")));
    // Two names taken: the first reason and how many more. The image on
    // screen (a) is the last one tried.
    std::fs::write(out.join("a.png"), b"taken too").unwrap();
    h.act(Action::ToggleSelect(0));
    move_to(&mut h, out.path());
    let notice = h.app().notice().expect("the report").to_string();
    assert!(
        notice.starts_with("Did not move b.png") && notice.ends_with("1 more file was not moved."),
        "{notice}"
    );
    assert_eq!(selected(&mut h), ["a.png", "b.png"]);
    assert_eq!(std::fs::read(out.join("a.png")).unwrap(), b"taken too");
}

#[test]
fn move_to_same_folder_is_refused() {
    let (dir, paths, _out, mut h) = move_setup("mv-same", &["a.png", "b.png"], 0);
    h.act(Action::ToggleSelect(0));
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, dir.path());
    assert_eq!(listing(dir.path()), ["a.png", "b.png"]);
    assert_eq!(names_of(&h), ["a.png", "b.png"]);
    let ops: Vec<&str> = ["a.png", "b.png"]
        .iter()
        .flat_map(|n| h.disk.ops_on(n))
        .filter(|op| matches!(*op, "rename_new" | "copy_new" | "remove_file"))
        .collect();
    assert!(ops.is_empty(), "{ops:?}");
    assert_eq!(selected(&mut h), ["a.png", "b.png"]);
    let folder_name = name(dir.path());
    assert!(has_text(
        &h.frame(),
        &format!("The images are in {folder_name} already")
    ));
    assert!(paths.iter().all(|p| p.exists()));
}

#[test]
fn move_across_drives_copies_then_removes_source() {
    let (dir, paths, out, mut h) = move_setup("mv-drive", &["a.png", "b.png", "c.png"], 0);
    let bytes = std::fs::read(&paths[1]).unwrap();
    h.disk.other_drive("b.png");
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, out.path());
    // The copy is whole under its own name; the source is gone; no
    // temporary file is left in either folder.
    assert_eq!(std::fs::read(out.join("b.png")).unwrap(), bytes);
    assert_eq!(listing(out.path()), ["b.png"]);
    assert!(!paths[1].exists());
    assert_eq!(listing(dir.path()), ["a.png", "c.png"]);
    // The rename was refused, then a copy, then the source went.
    let ops: Vec<&str> = h
        .disk
        .ops_on("b.png")
        .into_iter()
        .filter(|op| matches!(*op, "rename_new" | "copy_new" | "remove_file"))
        .collect();
    assert_eq!(ops, ["rename_new", "copy_new", "remove_file"]);
    assert_eq!(names_of(&h), ["a.png", "c.png"]);
    assert!(h.app().notice().is_none());
}

#[test]
fn move_copy_failure_keeps_source_and_removes_partial() {
    let (dir, paths, out, mut h) = move_setup("mv-fail", &["a.png", "b.png", "c.png"], 0);
    let bytes = std::fs::read(&paths[1]).unwrap();
    h.disk.other_drive("b.png");
    h.disk.fail_copy("b.png");
    h.disk.other_drive("c.png");
    h.act(Action::ToggleSelect(1));
    h.act(Action::ToggleSelect(2));
    move_to(&mut h, out.path());
    // The half copy of b is gone, b stays whole and selected; c moved.
    assert_eq!(listing(out.path()), ["c.png"]);
    assert_eq!(std::fs::read(&paths[1]).unwrap(), bytes);
    assert_eq!(listing(dir.path()), ["a.png", "b.png"]);
    assert!(!h.disk.ops_on("b.png").contains(&"remove_file"));
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    assert_eq!(names_of(&h), ["a.png", "b.png"]);
    let folder_name = name(out.path());
    let notice = h.app().notice().expect("the report").to_string();
    assert_eq!(
        notice,
        format!("Could not move b.png to {folder_name}: the test disk stops the copy")
    );
}

#[test]
fn move_updates_view_and_selection() {
    let names = ["a.png", "b.png", "c.png", "d.png", "e.png"];
    // The image on screen and its next neighbor move: the next remaining
    // image shows.
    let (_dir, paths, out, mut h) = move_setup("mv-view", &names, 1);
    h.key(Key::Space, NONE);
    h.act(Action::ToggleSelect(2));
    move_to(&mut h, out.path());
    assert_eq!(names_of(&h), ["a.png", "d.png", "e.png"]);
    assert!(h.wait_shown(&paths[3]));
    assert_eq!(h.app().index(), Some(1));
    assert!(h.app().selection().is_empty());
    let list = h.frame();
    assert!(has_text(&list, "2 / 3") && !has_text(&list, "selected"));
    // The image on screen stays when it does not move.
    h.act(Action::ToggleSelect(0));
    move_to(&mut h, out.path());
    assert_eq!(names_of(&h), ["d.png", "e.png"]);
    assert_eq!(h.app().shown_path(), Some(paths[3].as_path()));
    assert_eq!(h.app().index(), Some(0));
    // Everything moves: the window is empty and still answers.
    h.act(Action::ToggleSelect(0));
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, out.path());
    assert_eq!(h.app().count(), 0);
    assert_eq!(h.app().shown_path(), None);
    assert!(has_text(&h.frame(), "No image open"));
    assert_eq!(listing(out.path()), names);
    h.key(Key::M, NONE);
    h.key(Key::F11, NONE);
    assert!(h.app().fullscreen());
    h.key(Key::Escape, NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn move_remembers_last_folder() {
    let dir = TempDir::new("mv-remember");
    let settings_path = dir.join("settings.ini");
    let (_src, paths) = folder("mv-remember-src", &["a.png", "b.png", "c.png"], 8, 8);
    let out = TempDir::new("mv-remember-to");
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        settings_path: Some(settings_path.clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    move_to(&mut h, out.path());
    assert_eq!(
        h.app().settings().last_move_dir.as_deref(),
        Some(out.path())
    );
    // The next picker starts there.
    h.answer_dialog(Vec::new());
    h.key(Key::M, NONE);
    h.pump();
    let shown = lock(&h.dialogs).shown.clone();
    match shown.last() {
        Some(pandite_app::platform::DialogKind::PickFolder { start_dir, .. }) => {
            assert_eq!(start_dir.as_deref(), Some(out.path()));
        }
        other => panic!("expected a folder picker, got {other:?}"),
    }
    // It is saved with the other settings and read back.
    let saved = |p: &Path| std::fs::read_to_string(p).is_ok_and(|t| t.contains("last_move_dir"));
    for _ in 0..500 {
        if saved(&settings_path) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(saved(&settings_path), "the settings were written");
    let text = std::fs::read_to_string(&settings_path).unwrap();
    let back = pandite_app::settings::Settings::parse(&text).unwrap();
    assert_eq!(back.last_move_dir.as_deref(), Some(out.path()));
}

#[test]
fn move_runs_off_event_loop() {
    let (_dir, paths, out, mut h) = move_setup("mv-thread", &["a.png", "b.png", "c.png"], 0);
    // The move of b blocks in the file system until released.
    h.disk.stall_read("b.png");
    h.act(Action::ToggleSelect(1));
    h.answer_dialog(vec![out.path().to_path_buf()]);
    let before = std::time::Instant::now();
    h.key(Key::M, NONE);
    h.pump();
    assert!(before.elapsed() < std::time::Duration::from_secs(5));
    assert!(h.disk.wait_call("rename_new", "b.png"));
    assert!(h.app().moving());
    // The window still answers: keys, frames, the progress label.
    assert!(has_text(&h.frame(), "Moving 1 of 1 image"));
    h.key(Key::I, NONE);
    assert!(h.app().info_visible());
    h.key(Key::G, NONE);
    assert!(!h.app().settings().gallery_visible);
    assert_eq!(h.app().shown_path(), Some(paths[0].as_path()));
    // The file system call ran on a worker thread.
    let main = std::thread::current().id();
    let call = h
        .disk
        .calls()
        .into_iter()
        .find(|c| c.op == "rename_new")
        .unwrap();
    assert_ne!(call.thread, main);
    h.disk.release_all();
    assert!(h.wait_until(|a| !a.moving()));
    assert_eq!(listing(out.path()), ["b.png"]);
    assert!(!paths[1].exists());
}

#[test]
fn move_without_safe_rename_writes_a_new_name() {
    let (dir, paths, out, mut h) = move_setup("mv-fat", &["a.png", "b.png", "c.png"], 0);
    let bytes = std::fs::read(&paths[1]).unwrap();
    // The target drive has no rename that cannot replace a file (FAT).
    h.disk.no_safe_rename(&name(out.path()));
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, out.path());
    // The file is written under its final name, created new; no temporary
    // file is left; the source goes after the copy.
    assert_eq!(listing(out.path()), ["b.png"]);
    assert_eq!(std::fs::read(out.join("b.png")).unwrap(), bytes);
    assert_eq!(listing(dir.path()), ["a.png", "c.png"]);
    let ops: Vec<&str> = h
        .disk
        .ops_on("b.png")
        .into_iter()
        .filter(|op| matches!(*op, "rename_new" | "copy_new" | "remove_file"))
        .collect();
    assert_eq!(ops, ["rename_new", "copy_new", "copy_new", "remove_file"]);
    assert!(h.app().notice().is_none());
}

#[test]
fn move_never_replaces_a_file_that_appears() {
    // Across drives: the copy is ready, but another program wrote the name
    // meanwhile. The final rename refuses it.
    let (dir, paths, out, mut h) = move_setup("mv-race", &["a.png", "b.png", "c.png"], 0);
    let bytes = std::fs::read(&paths[1]).unwrap();
    h.disk.other_drive("b.png");
    h.disk.appear_on_copy(&out.join("b.png"), b"theirs");
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, out.path());
    assert_eq!(std::fs::read(out.join("b.png")).unwrap(), b"theirs");
    assert_eq!(listing(out.path()), ["b.png"], "the copy went");
    assert_eq!(std::fs::read(&paths[1]).unwrap(), bytes);
    assert_eq!(listing(dir.path()), ["a.png", "b.png", "c.png"]);
    assert_eq!(h.app().selection(), [paths[1].clone()]);
    let folder_name = name(out.path());
    assert_eq!(
        h.app().notice(),
        Some(format!("Did not move b.png: {folder_name} has a file of that name").as_str())
    );

    // On a drive without a safe rename: the copy to the final name, created
    // new, refuses the name as well, and the file there is not removed.
    let (dir, paths, out, mut h) = move_setup("mv-race-fat", &["a.png", "b.png"], 0);
    h.disk.no_safe_rename(&name(out.path()));
    h.disk.appear_on_copy(&out.join("b.png"), b"theirs too");
    h.act(Action::ToggleSelect(1));
    move_to(&mut h, out.path());
    assert_eq!(std::fs::read(out.join("b.png")).unwrap(), b"theirs too");
    assert_eq!(listing(out.path()), ["b.png"]);
    assert!(paths[1].exists());
    assert_eq!(listing(dir.path()), ["a.png", "b.png"]);
    assert_eq!(h.app().selection(), [paths[1].clone()]);
}

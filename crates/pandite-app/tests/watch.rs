//! Files that come to or leave the folder on screen outside pandite. The
//! core looks at the folder again at each tick of the fake clock after
//! [`FOLDER_CHECK`]: new files join the gallery strip, removed files leave
//! it and the selection, and the image on screen, its view and the strip
//! stay where they are. A folder that did not change costs one listing and
//! no frame. No look runs while the window is hidden, while files are
//! moved, copied or deleted, or while a save runs.

mod support;

use pandite_app::app::FOLDER_CHECK;
use pandite_core::{DrawCmd, DrawList, Point};
use pandite_ui::{thumbnail_index, Action, InputEvent, Key, Modifiers, ThumbState};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::*;

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

fn name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

/// Another program writes a small PNG image at `path`.
fn add(path: &Path) {
    write_png(
        path,
        &image(8, 6, |x, y| [x as u8 * 30, y as u8 * 40, 90, 255]),
    );
}

/// A core showing `names[current]` of a fresh folder, listed, with every
/// thumbnail uploaded.
fn open(tag: &str, names: &[&str], current: usize) -> (TempDir, Vec<PathBuf>, Harness) {
    let (dir, paths) = folder(tag, names, 8, 6);
    let mut h = Harness::new(Config {
        open: Some(paths[current].clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[current]));
    assert!(h.wait_until(|a| a.count() == names.len()));
    assert!(h.wait_thumbnails(names.len().min(20)));
    (dir, paths, h)
}

/// Let the library finish its work (the listing of a look among it), then
/// take its events.
fn settle(h: &mut Harness) {
    assert!(h.app().library().wait_idle(WAIT), "the library is idle");
    h.pump();
}

/// The names of the folder list.
fn listed(h: &mut Harness) -> Vec<String> {
    h.app().library().paths().iter().map(|p| name(p)).collect()
}

/// Where the thumbnail of each file is drawn: its left edge, by name.
fn places(h: &mut Harness, list: &DrawList) -> HashMap<String, f32> {
    let paths = h.app().library().paths();
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Image { rect, texture, .. } => {
                thumbnail_index(*texture).map(|i| (name(&paths[i]), rect.x))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn added_files_show_up_within_the_check_interval() {
    // More files than the strip holds, the one on screen in the middle.
    let names: Vec<String> = (0..30).map(|i| format!("img{:02}.png", i * 2)).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let (dir, paths, mut h) = open("watch-add", &names, 15);
    let folder = name(dir.path());
    h.act(Action::Select(14));
    h.act(Action::Select(16));
    h.act(Action::ZoomIn);
    h.act(Action::PanBy { dx: 3.0, dy: 2.0 });
    settle(&mut h);
    let before = h.frame();
    let old = places(&mut h, &before);
    assert!(old.len() >= 5, "{old:?}");
    let view = h.app().view();
    let updates = h.updates.len();
    let reads = h.disk.reads("img30.png");
    let listings = h.disk.listings(&folder);

    // A camera import: one file before every thumbnail on screen, one
    // among them, one after the end.
    for new in ["img01.png", "img29.png", "img99.png"] {
        add(&dir.join(new));
    }
    // Not yet due: no look.
    h.advance(ms(FOLDER_CHECK) - 1);
    h.tick();
    settle(&mut h);
    assert_eq!(h.disk.listings(&folder), listings);
    assert_eq!(h.app().count(), 30);
    // Due: one listing, and the files join the strip in their sort places.
    h.advance(1);
    h.tick();
    assert!(h.wait_until(|a| a.count() == 33));
    assert_eq!(h.disk.listings(&folder), listings + 1);
    let all = listed(&mut h);
    assert_eq!(all[1], "img01.png");
    assert_eq!(all[16], "img29.png");
    assert_eq!(all[32], "img99.png");

    // The image on screen did not change, nor its view, its place in the
    // folder or the selection.
    let after = h.frame();
    assert_eq!(h.app().shown_path(), Some(paths[15].as_path()));
    assert_eq!(h.app().index(), Some(17));
    assert_eq!(h.app().view(), view);
    assert_eq!(h.updates.len(), updates, "no new content");
    assert_eq!(h.disk.reads("img30.png"), reads, "not read again");
    assert_eq!(h.app().selection(), [paths[14].clone(), paths[16].clone()]);
    assert_eq!(h.app().selected_indices(), [15, 18]);
    // The thumbnails before the file that came among them stay where
    // they were, at once; those after it moved on by one cell.
    let new = places(&mut h, &after);
    let pitch = 96.0 + 9.0;
    for (file, x) in &old {
        let expect = if file.as_str() < "img29.png" {
            *x
        } else {
            x + pitch
        };
        if let Some(got) = new.get(file) {
            assert_eq!(*got, expect, "{file}");
        } else {
            assert!(file.as_str() > "img29.png", "{file} is drawn");
        }
    }
    // The new file on screen gets its thumbnail.
    assert!(h.wait_frames(|a, _| matches!(a.thumbnails()[16], ThumbState::Ready { .. })));
}

#[test]
fn removed_and_renamed_files_leave_the_strip_and_the_selection() {
    let names = ["a.png", "b.png", "c.png", "d.png", "e.png"];
    let (dir, paths, mut h) = open("watch-remove", &names, 2);
    h.act(Action::Select(1));
    h.act(Action::Select(3));
    assert_eq!(h.app().selected_indices(), [1, 3]);
    std::fs::remove_file(&paths[3]).unwrap();
    h.advance(ms(FOLDER_CHECK));
    h.tick();
    assert!(h.wait_until(|a| a.count() == 4));
    assert_eq!(h.app().selection(), std::slice::from_ref(&paths[1]));
    assert_eq!(h.app().selected_indices(), [1]);
    // The toolbar count follows.
    h.input(InputEvent::PointerMove {
        pos: Point::new(400.0, 10.0),
    });
    assert!(has_text(&h.frame(), "1 selected"));
    assert_eq!(h.app().shown_path(), Some(paths[2].as_path()));
    assert_eq!(h.app().index(), Some(2));

    // A rename: the old name leaves the selection, the new one joins the
    // strip in its place.
    std::fs::rename(&paths[1], dir.join("bb.png")).unwrap();
    h.advance(ms(FOLDER_CHECK));
    h.tick();
    assert!(h.wait_until(|a| a.selection().is_empty()));
    assert_eq!(listed(&mut h), ["a.png", "bb.png", "c.png", "e.png"]);
    assert!(h.app().selected_indices().is_empty());
    assert_eq!(h.app().index(), Some(2));
    // Next and previous follow the new list.
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[4]));
    h.key(Key::Left, Modifiers::NONE);
    h.key(Key::Left, Modifiers::NONE);
    assert!(h.wait_shown(&dir.join("bb.png")));
}

#[test]
fn a_removed_file_on_screen_is_handled_as_at_a_refresh() {
    let names = ["a.png", "b.png", "c.png", "d.png"];
    let (_dir, paths, mut h) = open("watch-current", &names, 1);
    std::fs::remove_file(&paths[1]).unwrap();
    h.advance(ms(FOLDER_CHECK));
    h.tick();
    assert!(h.wait_until(|a| a.count() == 3));
    // As after a refresh: the position moves to the file that took its
    // place, and the picture stays until the user moves on.
    assert_eq!(h.app().index(), Some(1));
    assert_eq!(h.app().library().current(), Some(1));
    assert_eq!(h.app().shown_path(), Some(paths[1].as_path()));
    h.key(Key::Left, Modifiers::NONE);
    assert!(h.wait_shown(&paths[0]));
    h.key(Key::Right, Modifiers::NONE);
    assert!(h.wait_shown(&paths[2]));
}

#[test]
fn a_folder_that_did_not_change_costs_one_listing_and_no_frame() {
    let names = ["a.png", "b.png", "c.png"];
    let (dir, _paths, mut h) = open("watch-quiet", &names, 0);
    let folder = name(dir.path());
    settle(&mut h);
    h.frame();
    let (listings, wakes, redraws) = (h.disk.listings(&folder), h.wakes(), h.redraws());
    let calls = h.disk.calls().len();
    for round in 1..=3 {
        let due = h.app().next_tick_at().expect("a look is due");
        assert!(due > h.now);
        h.now = due;
        h.tick();
        // The next look comes one interval later.
        assert_eq!(h.app().next_tick_at(), Some(due + FOLDER_CHECK));
        settle(&mut h);
        assert_eq!(h.disk.listings(&folder), listings + round);
        // The listing was the only file system call.
        assert_eq!(h.disk.calls().len(), calls + round);
    }
    assert_eq!(h.wakes(), wakes, "no wake");
    assert_eq!(h.redraws(), redraws, "no redraw");
    assert!(!h.app().needs_frame());
    assert_eq!(h.app().count(), 3);
}

#[test]
fn no_look_while_hidden() {
    let names = ["a.png", "c.png"];
    let (dir, _paths, mut h) = open("watch-hidden", &names, 0);
    let folder = name(dir.path());
    let listings = h.disk.listings(&folder);
    h.app().window_hidden(true);
    add(&dir.join("b.png"));
    assert_eq!(h.app().next_tick_at(), None);
    for _ in 0..3 {
        h.advance(ms(FOLDER_CHECK) * 2);
        h.tick();
        settle(&mut h);
    }
    assert_eq!(h.disk.listings(&folder), listings);
    assert_eq!(h.app().count(), 2);
    // On screen again: the look that came due runs at the next tick.
    h.app().window_hidden(false);
    let due = h.app().next_tick_at().expect("due again");
    assert!(due <= h.now);
    h.tick();
    assert!(h.wait_until(|a| a.count() == 3));
}

#[test]
fn no_look_while_files_move_or_a_save_runs() {
    // A move: the file system call of the move blocks until released.
    let names = ["a.png", "b.png", "c.png"];
    let (dir, paths, mut h) = open("watch-batch", &names, 0);
    let folder = name(dir.path());
    let out = TempDir::new("watch-batch-out");
    h.disk.stall_read("b.png");
    h.act(Action::Select(1));
    h.answer_dialog(vec![out.path().to_path_buf()]);
    h.act(Action::MoveTo);
    h.pump();
    assert!(h.disk.wait_call("rename_new", "b.png"));
    assert!(h.app().moving());
    add(&dir.join("d.png"));
    let listings = h.disk.listings(&folder);
    assert_eq!(h.app().next_tick_at(), None);
    h.advance(ms(FOLDER_CHECK) * 3);
    h.tick();
    h.pump();
    assert_eq!(h.disk.listings(&folder), listings);
    h.disk.release_read("b.png");
    assert!(h.wait_until(|a| !a.moving()));
    assert_eq!(listed(&mut h), ["a.png", "c.png"]);
    // Once it ended, the next tick looks.
    h.tick();
    assert!(h.wait_until(|a| a.count() == 3));
    assert_eq!(listed(&mut h), ["a.png", "c.png", "d.png"]);
    assert!(!paths[1].exists());

    // A save: its write blocks until released.
    let names = ["a.png", "b.png"];
    let (dir, paths, mut h) = open("watch-save", &names, 0);
    let folder = name(dir.path());
    h.disk.stall_write("out.png");
    h.answer_dialog(vec![dir.join("out.png")]);
    h.act(Action::SaveAs);
    h.pump();
    assert!(h.disk.wait_call("write_atomic", "out.png"));
    assert!(h.app().is_busy());
    add(&dir.join("c.png"));
    let listings = h.disk.listings(&folder);
    assert_eq!(h.app().next_tick_at(), None);
    h.advance(ms(FOLDER_CHECK) * 3);
    h.tick();
    h.pump();
    assert_eq!(h.disk.listings(&folder), listings);
    h.disk.release_write("out.png");
    // Save As shows the new file; its folder is listed with it.
    let saved = dir.join("out.png");
    assert!(h.wait_shown(&saved));
    assert!(h.wait_until(|a| a.count() == 4));
    assert!(paths[0].exists());
}

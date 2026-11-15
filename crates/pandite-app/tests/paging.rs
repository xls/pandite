//! Page Down and Page Up move by one page of the gallery strip: the number
//! of thumbnails that fit whole in the strip at the window width. At scale
//! 1 the default medium strip has 15 pixels of padding on each side and 96
//! pixel cells 105 pixels apart, so a window `w` pixels wide holds
//! `(w - 30 - 96) / 105 + 1` whole thumbnails (rounded down).

mod support;

use pandite_app::settings::Settings;
use pandite_core::{DrawCmd, Rect};
use pandite_ui::{thumbnail_index, InputEvent, Key, Modifiers, ThumbState, UiMode};
use std::path::PathBuf;
use support::*;

const NONE: Modifiers = Modifiers::NONE;
/// Images in the test folder.
const COUNT: usize = 40;
/// The page of the 1000 pixel wide test window: (1000 - 126) / 105 + 1.
const PAGE_1000: usize = 9;

/// A folder of [`COUNT`] small images and a core on image `start`, with
/// the folder listed.
fn open_at(tag: &str, start: usize, settings: Settings) -> (TempDir, Vec<PathBuf>, Harness) {
    let names: Vec<String> = (0..COUNT).map(|i| format!("img{i:02}.png")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let (dir, paths) = folder(tag, &refs, 8, 8);
    let mut h = Harness::new(Config {
        open: Some(paths[start].clone()),
        settings,
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[start]));
    assert!(h.wait_until(|a| a.count() == COUNT));
    assert_eq!(h.app().index(), Some(start));
    (dir, paths, h)
}

fn no_wrap() -> Settings {
    Settings {
        wrap_around: false,
        ..Settings::default()
    }
}

/// Press `key` and check that image `index` is the one wanted and shows.
fn press_to(h: &mut Harness, paths: &[PathBuf], key: Key, index: usize) {
    h.key(key, NONE);
    assert_eq!(h.app().index(), Some(index), "{key:?}");
    assert_eq!(h.app().target_path(), Some(paths[index].as_path()));
    assert!(h.wait_shown(&paths[index]));
}

/// Draw frames with the clock running until the strip's scroll settles,
/// then the rectangle where the thumbnail of `index` is drawn.
fn thumbnail_rect(h: &mut Harness, index: usize) -> Rect {
    for _ in 0..60 {
        h.advance(50);
        h.frame();
    }
    assert!(h.wait_frames(|a, _| {
        matches!(a.thumbnails().get(index), Some(ThumbState::Ready { .. }))
    }));
    let list = h.frame();
    list.cmds
        .iter()
        .find_map(|c| match c {
            DrawCmd::Image { rect, texture, .. } if thumbnail_index(*texture) == Some(index) => {
                Some(*rect)
            }
            _ => None,
        })
        .expect("the thumbnail of the current image is drawn")
}

#[test]
fn page_down_moves_one_strip_page() {
    let (_dir, paths, mut h) = open_at("page-down", 0, Settings::default());
    press_to(&mut h, &paths, Key::PageDown, PAGE_1000);
    press_to(&mut h, &paths, Key::PageDown, 2 * PAGE_1000);
    assert!(has_text(
        &h.frame(),
        &format!("{} / {COUNT}", 2 * PAGE_1000 + 1)
    ));
    // The strip scrolled so the new current image shows whole.
    let r = thumbnail_rect(&mut h, 2 * PAGE_1000);
    assert!(r.x >= 0.0 && r.right() <= 1000.0, "{r:?}");

    // A held key moves on page by page, like the arrow keys.
    h.input(InputEvent::KeyDown {
        key: Key::PageUp,
        mods: NONE,
        repeat: true,
    });
    assert_eq!(h.app().index(), Some(PAGE_1000));

    // Nothing moves while a dialog with a text field or crop mode has the
    // keys.
    h.key(Key::R, Modifiers::CTRL);
    assert_eq!(h.app().ui().mode(), UiMode::Modal);
    h.key(Key::PageDown, NONE);
    h.key(Key::PageUp, NONE);
    assert_eq!(h.app().index(), Some(PAGE_1000));
    h.key(Key::Escape, NONE);
    h.key(Key::C, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Crop);
    h.key(Key::PageDown, NONE);
    h.key(Key::PageUp, NONE);
    assert_eq!(h.app().index(), Some(PAGE_1000));
    h.key(Key::C, NONE);
    assert_eq!(h.app().ui().mode(), UiMode::Normal);
    assert!(!h.app().exit_requested());
}

#[test]
fn page_up_moves_one_strip_page() {
    let (_dir, paths, mut h) = open_at("page-up", 30, Settings::default());
    press_to(&mut h, &paths, Key::PageUp, 30 - PAGE_1000);
    press_to(&mut h, &paths, Key::PageUp, 30 - 2 * PAGE_1000);
    assert!(has_text(
        &h.frame(),
        &format!("{} / {COUNT}", 30 - 2 * PAGE_1000 + 1)
    ));
    let r = thumbnail_rect(&mut h, 30 - 2 * PAGE_1000);
    assert!(r.x >= 0.0 && r.right() <= 1000.0, "{r:?}");
}

#[test]
fn page_keys_clamp_at_ends() {
    // Without wrapping, a page past an end stops there, and there it stays.
    let (_dir, paths, mut h) = open_at("page-clamp", 33, no_wrap());
    press_to(&mut h, &paths, Key::PageDown, COUNT - 1);
    h.key(Key::PageDown, NONE);
    assert_eq!(h.app().index(), Some(COUNT - 1));
    press_to(&mut h, &paths, Key::Home, 0);
    press_to(&mut h, &paths, Key::Right, 1);
    press_to(&mut h, &paths, Key::PageUp, 0);
    h.key(Key::PageUp, NONE);
    assert_eq!(h.app().index(), Some(0));

    // With wrapping (the default), a page first stops at the end; from the
    // end it wraps to the other end, as a single step does.
    let (_dir, paths, mut h) = open_at("page-wrap", 33, Settings::default());
    assert!(Settings::default().wrap_around);
    press_to(&mut h, &paths, Key::PageDown, COUNT - 1);
    press_to(&mut h, &paths, Key::PageDown, 0);
    press_to(&mut h, &paths, Key::PageUp, COUNT - 1);
    press_to(&mut h, &paths, Key::PageUp, COUNT - 1 - PAGE_1000);
}

#[test]
fn page_size_follows_window_width() {
    let (_dir, paths, mut h) = open_at("page-width", 0, no_wrap());
    // 500 pixels: (500 - 126) / 105 + 1 = 4.
    h.app().resized(500, 800, false);
    press_to(&mut h, &paths, Key::PageDown, 4);
    // 1400 pixels: (1400 - 126) / 105 + 1 = 13.
    h.app().resized(1400, 800, false);
    press_to(&mut h, &paths, Key::PageDown, 4 + 13);
    press_to(&mut h, &paths, Key::PageUp, 4);
    // At scale 2 every length doubles: (1400 - 252) / 210 + 1 = 6.
    h.app().scale_changed(2.0);
    press_to(&mut h, &paths, Key::PageDown, 4 + 6);
    // Small cells: (1400 - 168) / 140 + 1 = 9. Large cells:
    // (1400 - 336) / 280 + 1 = 4.
    h.act(pandite_ui::Action::SmallerThumbnails);
    press_to(&mut h, &paths, Key::PageDown, 4 + 6 + 9);
    h.act(pandite_ui::Action::LargerThumbnails);
    h.act(pandite_ui::Action::LargerThumbnails);
    press_to(&mut h, &paths, Key::PageUp, 4 + 6 + 9 - 4);
    // A window narrower than one thumbnail still moves by one image.
    h.app().scale_changed(1.0);
    h.app().resized(40, 800, false);
    press_to(&mut h, &paths, Key::PageDown, 4 + 6 + 9 - 4 + 1);
}

#[test]
fn page_keys_work_with_strip_hidden() {
    let (_dir, paths, mut h) = open_at("page-hidden", 0, no_wrap());
    h.key(Key::G, NONE);
    assert!(!h.app().settings().gallery_visible, "G hid the strip");
    // The page is the count the strip would show at this width.
    press_to(&mut h, &paths, Key::PageDown, PAGE_1000);
    press_to(&mut h, &paths, Key::PageDown, 2 * PAGE_1000);
    press_to(&mut h, &paths, Key::PageUp, PAGE_1000);
    // Shown again, or too short a window for the strip: the same page.
    h.key(Key::G, NONE);
    assert!(h.app().settings().gallery_visible);
    h.app().resized(1000, 150, false);
    press_to(&mut h, &paths, Key::PageDown, 2 * PAGE_1000);
    press_to(&mut h, &paths, Key::PageUp, PAGE_1000);
}

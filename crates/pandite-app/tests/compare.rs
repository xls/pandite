//! The comparison end to end: D sets one selected image against the image
//! on screen with a slider, or shows two to eight selected images in
//! tiles; Space on a tile unmarks it; Escape ends the comparison before
//! it quits.

mod support;

use pandite_app::app::CompareUpdate;
use pandite_core::Point;
use pandite_ui::{CompareKind, CompareState, InputEvent, Key, Modifiers, PointerButton};
use std::path::Path;
use support::*;

const NONE: Modifiers = Modifiers::NONE;

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

fn names(h: &mut Harness) -> Vec<String> {
    h.app()
        .compare_items()
        .iter()
        .map(|i| i.name.clone())
        .collect()
}

fn all_ready(h: &mut Harness) -> bool {
    h.wait_until(|a| {
        a.compare_items()
            .iter()
            .all(|i| i.state == CompareState::Ready)
    })
}

#[test]
fn one_selected_image_compares_with_the_image_on_screen() {
    let (_dir, paths) = folder("compare-slider", &["a.png", "b.png", "c.png"], 12, 8);
    let mut h = open(&paths[0], 3);

    // D on the selected image itself has nothing to compare with.
    h.key(Key::Space, NONE);
    h.key(Key::D, NONE);
    assert_eq!(h.app().compare_kind(), None);
    let list = h.frame();
    assert!(has_text(&list, "Compare with D"));
    assert!(has_text(&list, "Show another image"));

    h.key(Key::Right, NONE);
    assert!(h.wait_shown(&paths[1]));
    h.key(Key::D, NONE);
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Slider));
    assert_eq!(names(&mut h), ["a.png"]);
    assert!(all_ready(&mut h));
    let list = h.frame();
    assert!(has_text(&list, "Esc to end compare"));
    assert!(has_text(&list, "a.png"));
    assert!(matches!(
        h.compare_updates.as_slice(),
        [CompareUpdate::Set { logical: None, .. }]
    ));
    // The selected image left of the line, the image on screen right.
    let [left, right] = h.panes.clone()[..] else {
        panic!("two panes, got {:?}", h.panes);
    };
    assert!(left.slot.is_some() && right.slot.is_none());
    assert_eq!(left.viewport, right.viewport);
    assert_eq!(left.clip.right(), right.clip.x);
    let middle = left.clip.right();

    // A drag on the line moves it.
    let y = left.viewport.y + left.viewport.h * 0.5;
    h.input(InputEvent::PointerMove {
        pos: Point::new(middle, y),
    });
    h.input(InputEvent::PointerDown {
        pos: Point::new(middle, y),
        button: PointerButton::Left,
    });
    h.input(InputEvent::PointerMove {
        pos: Point::new(middle - 100.0, y),
    });
    h.input(InputEvent::PointerUp {
        pos: Point::new(middle - 100.0, y),
        button: PointerButton::Left,
    });
    h.frame();
    assert_eq!(h.panes[0].clip.right(), middle - 100.0);

    // The next image takes the right side; the comparison stays.
    h.key(Key::Right, NONE);
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Slider));

    // Escape ends the comparison; the next Escape quits.
    h.key(Key::Escape, NONE);
    assert_eq!(h.app().compare_kind(), None);
    assert!(!h.app().exit_requested());
    h.frame();
    assert!(h.panes.is_empty());
    assert!(matches!(
        h.compare_updates.last(),
        Some(CompareUpdate::Clear)
    ));
    h.key(Key::Escape, NONE);
    assert!(h.app().exit_requested());
}

#[test]
fn several_selected_images_compare_in_tiles() {
    let (_dir, paths) = folder(
        "compare-tiles",
        &["a.png", "b.png", "c.png", "d.png"],
        12,
        8,
    );
    let mut h = open(&paths[0], 4);
    // Select a, b and d.
    h.key(Key::Space, NONE);
    h.key(Key::Right, NONE);
    assert!(h.wait_shown(&paths[1]));
    h.key(Key::Space, NONE);
    h.key(Key::End, NONE);
    assert!(h.wait_shown(&paths[3]));
    h.key(Key::Space, NONE);

    h.key(Key::D, NONE);
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Tiles));
    assert_eq!(names(&mut h), ["a.png", "b.png", "d.png"]);
    assert!(all_ready(&mut h));
    h.frame();
    assert_eq!(h.panes.len(), 3);
    assert!(h.panes.iter().all(|p| p.slot.is_some()));
    let sets = h
        .compare_updates
        .iter()
        .filter(|u| matches!(u, CompareUpdate::Set { .. }))
        .count();
    assert_eq!(sets, 3);

    // The arrow keys move through the folder behind the tiles, and Space
    // marks the current image: c joins the tiles.
    h.key(Key::Left, NONE);
    assert!(h.wait_shown(&paths[2]));
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Tiles));
    h.key(Key::Space, NONE);
    assert_eq!(names(&mut h), ["a.png", "b.png", "c.png", "d.png"]);
    assert!(all_ready(&mut h));
    h.frame();
    assert_eq!(h.panes.len(), 4);

    // Space on a marked image unmarks it: its tile leaves.
    h.key(Key::Left, NONE);
    assert!(h.wait_shown(&paths[1]));
    h.key(Key::Space, NONE);
    assert_eq!(names(&mut h), ["a.png", "c.png", "d.png"]);
    assert_eq!(h.app().selected_indices(), [0, 2, 3]);
    h.frame();
    assert_eq!(h.panes.len(), 3);
    assert!(h
        .compare_updates
        .iter()
        .any(|u| matches!(u, CompareUpdate::Remove(_))));

    // A click on a tile makes its image the current one.
    let tile = h.panes[2].clip;
    for event in [
        InputEvent::PointerMove { pos: tile.center() },
        InputEvent::PointerDown {
            pos: tile.center(),
            button: PointerButton::Left,
        },
        InputEvent::PointerUp {
            pos: tile.center(),
            button: PointerButton::Left,
        },
    ] {
        h.input(event);
    }
    assert!(h.wait_shown(&paths[3]));
    h.key(Key::Space, NONE);
    assert_eq!(names(&mut h), ["a.png", "c.png"]);

    // With one image left there is nothing to compare.
    h.key(Key::Home, NONE);
    assert!(h.wait_shown(&paths[0]));
    h.key(Key::Space, NONE);
    assert_eq!(h.app().compare_kind(), None);
    assert_eq!(h.app().selected_indices(), [2]);
}

#[test]
fn d_ends_a_comparison_and_a_file_that_cannot_open_says_so() {
    let (dir, paths) = folder("compare-bad", &["a.png", "b.png"], 12, 8);
    let bad = dir.join("c.png");
    std::fs::write(&bad, b"not a picture").unwrap();
    // The bad file never gets a thumbnail: do not wait for one.
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    h.key(Key::Space, NONE);
    h.key(Key::End, NONE);
    assert!(h.wait_until(|a| a.index() == Some(2)));
    h.key(Key::Space, NONE);
    h.key(Key::D, NONE);
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Tiles));
    assert!(h.wait_until(|a| {
        a.compare_items()
            .iter()
            .all(|i| i.state != CompareState::Loading)
    }));
    let states: Vec<CompareState> = h.app().compare_items().iter().map(|i| i.state).collect();
    assert_eq!(states, [CompareState::Ready, CompareState::Failed]);
    let list = h.frame();
    assert!(has_text(&list, "c.png - cannot open"));
    h.key(Key::D, NONE);
    assert_eq!(h.app().compare_kind(), None);
}

#[test]
fn the_magnifier_draws_the_picture_again_enlarged_about_the_pointer() {
    let (_dir, paths) = folder("compare-peek", &["a.png", "b.png"], 12, 8);
    let mut h = open(&paths[0], 2);
    h.frame();
    assert!(h.panes.is_empty());
    let p = Point::new(300.0, 200.0);
    h.input(InputEvent::PointerMove { pos: p });
    h.key(Key::P, NONE);
    h.frame();
    // The image, the plain background in the square, the enlarged image.
    let [image, back, big] = h.panes.clone()[..] else {
        panic!("three panes, got {:?}", h.panes);
    };
    assert_eq!((image.slot, image.rect), (None, None));
    assert_eq!(back.slot, Some(pandite_app::app::BACKDROP_SLOT));
    let lens = back.clip;
    assert_eq!(lens.center(), p);
    assert_eq!(back.rect, Some(lens));
    assert_eq!(big.slot, None);
    assert!(lens.contains(big.clip.center()));
    // The 12 x 8 picture shows at 100 %: the magnifier doubles it.
    let rect = big.rect.expect("an enlarged rectangle");
    assert_eq!((rect.w, rect.h), (24.0, 16.0));

    // In a slider each picture has its own square, side by side, about
    // the same spot.
    h.key(Key::Space, NONE);
    h.key(Key::Right, NONE);
    assert!(h.wait_shown(&paths[1]));
    h.key(Key::D, NONE);
    assert!(all_ready(&mut h));
    h.input(InputEvent::PointerMove { pos: p });
    h.frame();
    // Two pictures, two backgrounds, two enlarged pictures.
    assert_eq!(h.panes.len(), 6, "{:?}", h.panes);
    let (left, right) = (h.panes[4], h.panes[5]);
    assert!(left.slot.is_some() && right.slot.is_none());
    assert!(left.clip.right() < p.x && right.clip.x > p.x);
    let (a, b) = (left.rect.unwrap(), right.rect.unwrap());
    assert_eq!((a.w, a.h), (b.w, b.h));
    assert_eq!(
        a.x - left.clip.center().x,
        b.x - right.clip.center().x,
        "the same spot in both squares"
    );
    // Escape closes the magnifier, then the comparison.
    h.key(Key::Escape, NONE);
    h.frame();
    assert_eq!(h.panes.len(), 2);
    assert_eq!(h.app().compare_kind(), Some(CompareKind::Slider));
}

//! Comparing selected images: D starts a slider or tiles from the
//! selection, the slider line follows a drag, Space unmarks the tile under
//! the pointer, and D or Escape ends the comparison.

mod support;

use pandite_core::{Point, Rect};
use pandite_ui::{
    Action, CompareItem, CompareKind, CompareState, CompareView, CursorShape, Key, PointerButton,
    ThumbState, Ui, UiContext,
};
use support::*;

fn item(name: &str, index: usize) -> CompareItem {
    CompareItem {
        name: name.into(),
        index: Some(index),
        state: CompareState::Ready,
    }
}

fn comparing<'a>(kind: CompareKind, items: &'a [CompareItem]) -> UiContext<'a> {
    let mut ctx = image_ctx();
    ctx.compare = Some(CompareView { kind, items });
    ctx
}

#[test]
fn d_starts_the_comparison_the_selection_allows() {
    // The image on screen is index 3.
    let mut ctx = image_ctx();
    let mut ui = Ui::new();
    assert!(press_key(&mut ui, &ctx, Key::D).is_empty(), "no selection");

    // One selected image, another on screen: a slider.
    ctx.selected = &[1];
    assert_eq!(press_key(&mut ui, &ctx, Key::D), [Action::Compare]);

    // The selected image is the one on screen: a notice, no comparison.
    ctx.selected = &[3];
    assert!(press_key(&mut ui, &ctx, Key::D).is_empty());
    assert!(has_text(&frame(&mut ui, &ctx), "Show another image"));

    // The pointer over another thumbnail: that image shows, then compares.
    let mut ui = Ui::new();
    let thumbs = [ThumbState::Ready {
        width: 120,
        height: 120,
    }; 10];
    ctx.thumbnails = &thumbs;
    let list = frame(&mut ui, &ctx);
    let (index, cell) = thumbnail_draws(&list)
        .into_iter()
        .find(|(i, _)| *i != 3)
        .expect("a second thumbnail");
    move_to(&mut ui, &ctx, center(cell));
    assert_eq!(
        press_key(&mut ui, &ctx, Key::D),
        [Action::GoTo(index), Action::Compare]
    );

    // Two to eight selected images: tiles. More: a notice.
    let mut ui = Ui::new();
    ctx.selected = &[0, 1, 2, 3, 4, 5, 6, 7];
    assert_eq!(press_key(&mut ui, &ctx, Key::D), [Action::Compare]);
    ctx.selected = &[0, 1, 2, 3, 4, 5, 6, 7, 8];
    assert!(press_key(&mut ui, &ctx, Key::D).is_empty());
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Compare shows 8 images or fewer"));
    assert!(has_text(&list, "Compare: 8 images or fewer"));
}

#[test]
fn the_selection_box_names_the_compare_key() {
    let mut ctx = image_ctx();
    ctx.selected = &[1];
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let clear = text_rect_exact(&list, "Backspace to clear selection").expect("clear tip");
    let tip = text_rect_exact(&list, "Compare with D").expect("compare tip");
    assert_eq!(tip.x, clear.x);
    assert!(tip.y >= clear.bottom(), "{tip:?} below {clear:?}");
    let count = text_rect_exact(&list, "1 selected").expect("count");

    // More images, or a running comparison, move nothing.
    ctx.selected = &[1, 2, 5];
    let list = frame(&mut ui, &ctx);
    assert_eq!(text_rect_exact(&list, "Compare with D"), Some(tip));
    assert_eq!(
        text_rect_exact(&list, "3 selected").map(|r| r.y),
        Some(count.y)
    );
    let items = [item("a.jpg", 1)];
    let mut ctx = comparing(CompareKind::Slider, &items);
    ctx.selected = &[1];
    let list = frame(&mut ui, &ctx);
    assert_eq!(
        text_rect_exact(&list, "Esc to end compare").map(|r| (r.x, r.y)),
        Some((tip.x, tip.y))
    );
}

#[test]
fn slider_line_follows_a_drag_and_escape_ends() {
    let items = [item("first.jpg", 1)];
    let mut ctx = comparing(CompareKind::Slider, &items);
    ctx.selected = &[1];
    let mut ui = Ui::new();
    let area = ui.content_rect(&ctx);
    let panes = ui.compare_panes(&ctx);
    assert_eq!(panes.len(), 2);
    assert_eq!(panes[0].item, Some(0));
    assert_eq!(panes[1].item, None);
    assert_eq!(panes[0].viewport, area);
    assert_eq!(panes[1].viewport, area);
    // The line starts in the middle; the panes meet on it.
    let x = area.x + area.w * 0.5;
    assert_eq!(panes[0].clip, Rect::new(area.x, area.y, x - area.x, area.h));
    assert_eq!(
        panes[1].clip,
        Rect::new(x, area.y, area.right() - x, area.h)
    );
    // Both file names show.
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "first.jpg"));
    assert!(has_text(&list, "photo.jpg"));

    // A drag on the line moves it and does not pan.
    let y = area.y + area.h * 0.5;
    move_to(&mut ui, &ctx, Point::new(x + 3.0, y));
    frame(&mut ui, &ctx);
    assert_eq!(ui.cursor(), CursorShape::ResizeHorizontal);
    let out = drag(
        &mut ui,
        &ctx,
        Point::new(x + 3.0, y),
        Point::new(x - 200.0, y),
        PointerButton::Left,
    );
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(ui.compare_panes(&ctx)[0].clip.right(), x - 200.0);
    // Past the edge it stops at the edge.
    drag(
        &mut ui,
        &ctx,
        Point::new(x - 200.0, y),
        Point::new(-50.0, y),
        PointerButton::Left,
    );
    assert_eq!(ui.compare_panes(&ctx)[0].clip.w, 0.0);

    // Edits wait for the end of the comparison.
    assert!(press_key(&mut ui, &ctx, Key::R).is_empty());
    assert!(press_key(&mut ui, &ctx, Key::C).is_empty());
    // D and Escape end it; Escape does not quit.
    assert_eq!(press_key(&mut ui, &ctx, Key::D), [Action::EndCompare]);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), [Action::EndCompare]);
}

#[test]
fn tiles_fill_the_image_area_and_space_unmarks_the_one_under_the_pointer() {
    let items = [item("a.jpg", 0), item("b.jpg", 2), item("c.jpg", 7)];
    let mut ctx = comparing(CompareKind::Tiles, &items);
    ctx.selected = &[0, 2, 7];
    let mut ui = Ui::new();
    let area = ui.content_rect(&ctx);
    let panes = ui.compare_panes(&ctx);
    assert_eq!(panes.len(), 3);
    for (i, pane) in panes.iter().enumerate() {
        assert_eq!(pane.item, Some(i));
        assert_eq!(pane.viewport, pane.clip);
        assert!(pane.clip.x >= area.x && pane.clip.right() <= area.right());
        assert!(pane.clip.y >= area.y && pane.clip.bottom() <= area.bottom());
    }
    let list = frame(&mut ui, &ctx);
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        assert!(has_text(&list, name), "{name}");
    }
    assert!(text_rect_exact(&list, "Marked").is_none());

    // Space marks or unmarks the current image (index 3), and the arrow
    // keys move through the folder behind the tiles.
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Space),
        [Action::ToggleSelect(3)]
    );
    assert_eq!(press_key(&mut ui, &ctx, Key::Right), [Action::Next]);
    // A click on a tile makes its image the current one.
    assert_eq!(
        click(&mut ui, &ctx, center(panes[1].clip)),
        [Action::GoTo(2)]
    );
    // A drag pans the tiles.
    let from = center(panes[0].clip);
    let out = drag(
        &mut ui,
        &ctx,
        from,
        Point::new(from.x + 40.0, from.y + 10.0),
        PointerButton::Left,
    );
    assert!(
        out.iter().any(|a| matches!(a, Action::PanBy { .. })),
        "{out:?}"
    );
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), [Action::EndCompare]);
}

#[test]
fn p_shows_a_magnifier_square_at_the_pointer() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let area = ui.content_rect(&ctx);
    let p = Point::new(area.x + 400.0, area.y + 300.0);
    move_to(&mut ui, &ctx, p);
    assert!(ui.peek_lenses(&ctx).is_empty());
    assert!(press_key(&mut ui, &ctx, Key::P).is_empty());
    let [lens] = ui.peek_lenses(&ctx)[..] else {
        panic!("one square");
    };
    assert_eq!((lens.item, lens.source, lens.bounds), (None, p, area));
    assert_eq!(lens.rect.center(), p);
    assert_eq!(lens.rect.w, lens.rect.h);
    frame(&mut ui, &ctx);
    assert_eq!(ui.cursor(), CursorShape::Crosshair);
    // It follows the pointer.
    let q = Point::new(p.x + 50.0, p.y - 20.0);
    move_to(&mut ui, &ctx, q);
    assert_eq!(ui.peek_lenses(&ctx)[0].rect.center(), q);
    // Escape closes it and does not quit; P closes it too.
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert!(ui.peek_lenses(&ctx).is_empty());
    press_key(&mut ui, &ctx, Key::P);
    assert_eq!(ui.peek_lenses(&ctx).len(), 1);
    press_key(&mut ui, &ctx, Key::P);
    assert!(ui.peek_lenses(&ctx).is_empty());
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), [Action::Quit]);
    // Without a picture there is nothing to magnify.
    let empty = base_ctx();
    let mut ui = Ui::new();
    move_to(&mut ui, &empty, p);
    press_key(&mut ui, &empty, Key::P);
    assert!(ui.peek_lenses(&empty).is_empty());
}

#[test]
fn the_magnifier_shows_every_compared_picture() {
    // A slider: two squares side by side at the pointer, one per picture,
    // about the same spot.
    let items = [item("first.jpg", 1)];
    let ctx = comparing(CompareKind::Slider, &items);
    let mut ui = Ui::new();
    let area = ui.content_rect(&ctx);
    let p = Point::new(area.x + 500.0, area.y + 300.0);
    move_to(&mut ui, &ctx, p);
    press_key(&mut ui, &ctx, Key::P);
    let [left, right] = ui.peek_lenses(&ctx)[..] else {
        panic!("two squares");
    };
    assert_eq!((left.item, right.item), (Some(0), None));
    assert_eq!((left.source, right.source), (p, p));
    assert!(left.rect.right() < p.x && right.rect.x > p.x);
    assert_eq!(left.rect.y, right.rect.y);

    // Tiles: one square in every tile, at the matching spot.
    let items = [item("a.jpg", 0), item("b.jpg", 2), item("c.jpg", 7)];
    let ctx = comparing(CompareKind::Tiles, &items);
    let mut ui = Ui::new();
    let panes = ui.compare_panes(&ctx);
    let p = Point::new(panes[1].clip.x + 90.0, panes[1].clip.y + 120.0);
    move_to(&mut ui, &ctx, p);
    press_key(&mut ui, &ctx, Key::P);
    let lenses = ui.peek_lenses(&ctx);
    assert_eq!(lenses.len(), 3);
    for (i, lens) in lenses.iter().enumerate() {
        assert_eq!(lens.item, Some(i));
        assert_eq!(lens.bounds, panes[i].clip);
        assert_eq!(lens.rect.center(), lens.source);
        let (dx, dy) = (
            lens.source.x - panes[i].viewport.center().x,
            lens.source.y - panes[i].viewport.center().y,
        );
        assert_eq!(dx, p.x - panes[1].viewport.center().x);
        assert_eq!(dy, p.y - panes[1].viewport.center().y);
    }
    assert_eq!(lenses[1].source, p);
}

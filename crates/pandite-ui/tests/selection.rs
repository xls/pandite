//! Gate G2: the selection. Space selects the image on screen and
//! Backspace clears the selection; the gallery strip draws a check box per
//! thumbnail, the image a badge, the toolbar the count, and the context
//! menu offers both commands. Space and Backspace never navigate.

mod support;

use pandite_core::{DrawCmd, DrawList, Point, Rect, Size};
use pandite_ui::{
    Action, ActionKind, Command, ConfirmRequest, Icon, InputEvent, Key, Modifiers, PointerButton,
    Theme, ThumbState, Ui, UiContext, UiMode, SHORTCUTS,
};
use support::*;

fn gallery_ctx(count: usize, index: usize, thumbs: &'static [ThumbState]) -> UiContext<'static> {
    let mut ctx = image_ctx();
    ctx.count = count;
    ctx.index = index;
    ctx.thumbnails = thumbs;
    ctx
}

fn ready(n: usize) -> &'static [ThumbState] {
    Box::leak(
        vec![
            ThumbState::Ready {
                width: 160,
                height: 120
            };
            n
        ]
        .into_boxed_slice(),
    )
}

/// The drawn cell of thumbnail `i`.
fn cell_of(list: &DrawList, i: usize) -> Rect {
    thumbnail_draws(list)
        .into_iter()
        .find(|d| d.0 == i)
        .unwrap_or_else(|| panic!("thumbnail {i} not drawn"))
        .1
}

fn inside(r: Rect, outer: Rect) -> bool {
    r.x >= outer.x && r.y >= outer.y && r.right() <= outer.right() && r.bottom() <= outer.bottom()
}

/// Outlines drawn inside `cell`: the empty check box. The ring of the
/// current image lies around its cell, not inside it.
fn boxes_in(list: &DrawList, cell: Rect) -> Vec<Rect> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Border { rect, .. } if inside(*rect, cell) => Some(*rect),
            _ => None,
        })
        .collect()
}

/// Check marks drawn with their center inside `area`.
fn checks_in(list: &DrawList, area: Rect) -> Vec<Rect> {
    icon_draws(list, Icon::Check)
        .into_iter()
        .map(|(r, _)| r)
        .filter(|r| area.contains(r.center()))
        .collect()
}

/// The top left quarter of a cell, where its check box sits.
fn corner(cell: Rect) -> Rect {
    Rect::new(cell.x, cell.y, cell.w * 0.5, cell.h * 0.5)
}

/// Filled rectangles of `color` that hold `p`.
fn fills_at(list: &DrawList, p: Point, color: pandite_core::Color) -> Vec<Rect> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect { rect, color: c, .. } if *c == color && rect.contains(p) => Some(*rect),
            _ => None,
        })
        .collect()
}

fn has_toggle(actions: &[Action]) -> bool {
    kinds(actions).contains(&ActionKind::ToggleSelect)
}

fn tall_image_ctx() -> UiContext<'static> {
    with_image(
        UiContext {
            window_size: Size::new(1280.0, 900.0),
            ..base_ctx()
        },
        4000,
        3000,
    )
}

/// True when a check mark sits in the gutter left of the menu label `label`.
fn checked(list: &DrawList, label: Rect) -> bool {
    icon_draws(list, Icon::Check).iter().any(|(r, _)| {
        (r.center().y - label.center().y).abs() < 14.0
            && r.right() <= label.x
            && label.x - r.x < 40.0
    })
}

/// Open the context menu with a right click at `p`.
fn right_click(ui: &mut Ui, ctx: &UiContext<'_>, p: Point) {
    move_to(ui, ctx, p);
    press(ui, ctx, p, PointerButton::Right);
    release(ui, ctx, p, PointerButton::Right);
    assert_eq!(ui.mode(), UiMode::Modal, "the context menu opened");
}

#[test]
fn space_toggles_selection() {
    // The image on screen has folder index 3: Space names it.
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Space),
        vec![Action::ToggleSelect(3)]
    );
    // On a selected image the same action unselects it.
    let mut selected = ctx;
    selected.selected = &[1, 3];
    assert_eq!(
        press_key(&mut ui, &selected, Key::Space),
        vec![Action::ToggleSelect(3)]
    );
    // Another image on screen, another index.
    let mut other = ctx;
    other.index = 7;
    assert_eq!(
        press_key(&mut ui, &other, Key::Space),
        vec![Action::ToggleSelect(7)]
    );
    // A file that failed to open is still in the folder list: it can be
    // selected, like it can be deleted.
    let failed = base_ctx();
    assert!(failed.image.is_none());
    assert_eq!(
        press_key(&mut ui, &failed, Key::Space),
        vec![Action::ToggleSelect(3)]
    );
    // Without an image and a folder there is nothing to select.
    let mut none = base_ctx();
    none.count = 0;
    assert!(press_key(&mut ui, &none, Key::Space).is_empty());
    // One press, one toggle: a held key does not repeat it.
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Space,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());
    // Space with Ctrl or Alt is not bound.
    assert!(key(&mut ui, &ctx, Key::Space, Modifiers::CTRL).is_empty());
    assert!(key(&mut ui, &ctx, Key::Space, Modifiers::ALT).is_empty());
    assert_eq!(Command::ToggleSelect.label(), "Select image");
    assert_eq!(
        Command::ToggleSelect.shortcut_label().as_deref(),
        Some("Space")
    );
}

#[test]
fn space_no_longer_moves_to_next() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    for _ in 0..3 {
        let out = press_key(&mut ui, &ctx, Key::Space);
        for kind in [ActionKind::Next, ActionKind::StepBy, ActionKind::GoTo] {
            assert!(!kinds(&out).contains(&kind), "Space sent {out:?}");
        }
    }
    // Right still moves on, also held.
    assert_eq!(press_key(&mut ui, &ctx, Key::Right), vec![Action::Next]);
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Right,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert_eq!(held, vec![Action::Next]);
    // The table binds Space to the selection only, and Next to Right only.
    for s in SHORTCUTS.iter().filter(|s| s.key == Key::Space) {
        assert_eq!(s.command, Command::ToggleSelect, "{s:?}");
    }
    let next: Vec<String> = SHORTCUTS
        .iter()
        .filter(|s| s.command == Command::Next)
        .map(|s| s.label())
        .collect();
    assert_eq!(next, ["Right"]);
    assert_eq!(Command::Next.tooltip(), "Next image  (Right)");
    // The shortcut overlay shows Space beside "Select image" and Right
    // alone beside "Next image".
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    assert!(!has_text(&list, "Space /") && !has_text(&list, "/ Space"));
    let same_row = |keys: &str, label: &str| {
        let k = text_rect_exact(&list, keys).unwrap_or_else(|| panic!("{keys} not shown"));
        let l = text_rect_exact(&list, label).unwrap_or_else(|| panic!("{label} not shown"));
        (k.center().y - l.center().y).abs() < 11.0 && k.x < l.x
    };
    assert!(same_row("Space", "Select image"));
    assert!(same_row("Right", "Next image"));
}

#[test]
fn gallery_draws_checkbox_for_selected() {
    let theme = Theme::for_kind(image_ctx().theme);
    for scale in [1.0, 2.0] {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        ctx.selected = &[2, 5];
        let mut ui = Ui::new();
        // The pointer rests on the image, away from the strip.
        move_to(&mut ui, &ctx, Point::new(600.0, 200.0));
        let list = frame(&mut ui, &ctx);
        let drawn: Vec<usize> = thumbnail_draws(&list).iter().map(|d| d.0).collect();
        assert!(drawn.contains(&2) && drawn.contains(&5) && drawn.contains(&7));
        for i in drawn {
            let cell = cell_of(&list, i);
            let checks = checks_in(&list, corner(cell));
            if i == 2 || i == 5 {
                // A checked box: the check mark on an accent square in the
                // top left corner, at the size of the scale.
                assert_eq!(checks.len(), 1, "thumbnail {i} at scale {scale}");
                let mark = checks[0];
                assert_eq!(mark.w, 20.0 * scale);
                let fill = fills_at(&list, mark.center(), theme.accent);
                assert!(
                    fill.iter().any(|r| r.w == 18.0 * scale
                        && inside(*r, cell)
                        && r.x - cell.x == 4.0 * scale),
                    "accent box of thumbnail {i}: {fill:?}"
                );
            } else {
                assert!(checks.is_empty(), "thumbnail {i} is not selected");
                assert!(boxes_in(&list, cell).is_empty(), "no empty box on {i}");
            }
        }
        // Nothing else in the frame carries a check mark.
        assert_eq!(icon_draws(&list, Icon::Check).len(), 2);
        // The tint of the mark is the accent text color.
        assert!(icon_draws(&list, Icon::Check)
            .iter()
            .all(|(_, tint)| *tint == theme.accent_text));
    }
}

#[test]
fn gallery_checkbox_appears_on_hover() {
    for scale in [1.0, 1.5, 2.0] {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        let mut ui = Ui::new();
        let list = frame(&mut ui, &ctx);
        let (c4, c6) = (cell_of(&list, 4), cell_of(&list, 6));
        assert!(
            boxes_in(&list, c4).is_empty(),
            "no box before the pointer comes"
        );

        // Over thumbnail 4: an empty box in its top left corner.
        move_to(&mut ui, &ctx, c4.center());
        let list = frame(&mut ui, &ctx);
        let boxes = boxes_in(&list, c4);
        assert_eq!(boxes.len(), 1, "scale {scale}");
        let b = boxes[0];
        let (side, inset) = ((18.0 * scale).round(), (4.0 * scale).round());
        assert_eq!((b.w, b.h), (side, side));
        assert_eq!((b.x - c4.x, b.y - c4.y), (inset, inset));
        assert!(checks_in(&list, c4).is_empty(), "the box is empty");
        assert!(boxes_in(&list, c6).is_empty());

        // The box follows the pointer to thumbnail 6.
        move_to(&mut ui, &ctx, c6.center());
        let list = frame(&mut ui, &ctx);
        assert!(boxes_in(&list, c4).is_empty());
        assert_eq!(boxes_in(&list, c6).len(), 1);

        // Away from the strip, or out of the window, it goes.
        move_to(&mut ui, &ctx, Point::new(600.0, 200.0));
        assert!(boxes_in(&frame(&mut ui, &ctx), c6).is_empty());
        move_to(&mut ui, &ctx, c6.center());
        assert_eq!(boxes_in(&frame(&mut ui, &ctx), c6).len(), 1);
        ui.handle_event(InputEvent::PointerLeft, &ctx);
        assert!(boxes_in(&frame(&mut ui, &ctx), c6).is_empty());

        // A selected thumbnail under the pointer keeps its checked box.
        let mut selected = ctx;
        selected.selected = &[4];
        move_to(&mut ui, &selected, c4.center());
        let list = frame(&mut ui, &selected);
        assert_eq!(checks_in(&list, corner(c4)).len(), 1);

        // Crop mode offers no box: a click there would do nothing.
        let mut ui = Ui::new();
        press_key(&mut ui, &ctx, Key::C);
        assert_eq!(ui.mode(), UiMode::Crop);
        move_to(&mut ui, &ctx, c4.center());
        assert!(boxes_in(&frame(&mut ui, &ctx), c4).is_empty());
    }
}

#[test]
fn gallery_checkbox_click_toggles_without_navigation() {
    for scale in [1.0, 1.25, 1.5, 2.0] {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        let mut ui = Ui::new();
        let list = frame(&mut ui, &ctx);
        let cell = cell_of(&list, 5);
        move_to(&mut ui, &ctx, cell.center());
        let b = boxes_in(&frame(&mut ui, &ctx), cell)[0];

        // A click on the box selects thumbnail 5 and stays on image 0.
        assert_eq!(
            click(&mut ui, &ctx, b.center()),
            vec![Action::ToggleSelect(5)],
            "scale {scale}"
        );
        // So does a click in the gap between the box and the cell corner.
        assert_eq!(
            click(&mut ui, &ctx, Point::new(cell.x + 1.0, cell.y + 1.0)),
            vec![Action::ToggleSelect(5)]
        );
        assert_eq!(
            click(&mut ui, &ctx, Point::new(b.right() + 1.0, b.bottom() + 1.0)),
            vec![Action::ToggleSelect(5)]
        );
        // A click elsewhere on the thumbnail shows the image, as before.
        assert_eq!(click(&mut ui, &ctx, cell.center()), vec![Action::GoTo(5)]);
        assert_eq!(
            click(
                &mut ui,
                &ctx,
                Point::new(cell.right() - 2.0, cell.bottom() - 2.0)
            ),
            vec![Action::GoTo(5)]
        );
        assert_eq!(
            click(&mut ui, &ctx, Point::new(cell.right() - 2.0, b.center().y)),
            vec![Action::GoTo(5)],
            "the top right corner shows the image"
        );

        // On a selected thumbnail the checked box unselects it.
        let mut selected = ctx;
        selected.selected = &[5];
        let list = frame(&mut ui, &selected);
        let mark = checks_in(&list, corner(cell))[0];
        assert_eq!(
            click(&mut ui, &selected, mark.center()),
            vec![Action::ToggleSelect(5)]
        );

        // A press on the box and a release on the picture does neither.
        move_to(&mut ui, &ctx, b.center());
        press(&mut ui, &ctx, b.center(), PointerButton::Left);
        assert!(release(&mut ui, &ctx, cell.center(), PointerButton::Left).is_empty());
        // Neither does a press on the picture and a release on the box.
        press(&mut ui, &ctx, cell.center(), PointerButton::Left);
        assert!(release(&mut ui, &ctx, b.center(), PointerButton::Left).is_empty());
        // A right click on the box opens the context menu.
        right_click(&mut ui, &ctx, b.center());
        press_key(&mut ui, &ctx, Key::Escape);
        assert_eq!(ui.mode(), UiMode::Normal);
    }

    // A drag that starts on a box scrolls the strip and selects nothing.
    let ctx = gallery_ctx(200, 0, ready(200));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let cell = cell_of(&list, 12);
    move_to(&mut ui, &ctx, cell.center());
    let b = boxes_in(&frame(&mut ui, &ctx), cell)[0];
    let out = drag(
        &mut ui,
        &ctx,
        b.center(),
        Point::new(b.center().x - 400.0, b.center().y),
        PointerButton::Left,
    );
    assert!(out.is_empty(), "{out:?}");
    assert!(ui.gallery_visible_range(&ctx).start > 0);

    // In crop mode the box selects nothing, as the strip shows nothing.
    let ctx = gallery_ctx(20, 0, ready(20));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let cell = cell_of(&list, 5);
    press_key(&mut ui, &ctx, Key::C);
    assert!(click(&mut ui, &ctx, Point::new(cell.x + 6.0, cell.y + 6.0)).is_empty());
    assert!(click(&mut ui, &ctx, cell.center()).is_empty());
}

#[test]
fn selected_image_shows_badge() {
    let theme = Theme::for_kind(image_ctx().theme);
    for scale in [1.0, 2.0] {
        let mut ctx = image_ctx();
        ctx.scale_factor = scale;
        ctx.window_size = Size::new(1280.0 * scale, 800.0 * scale);
        let ctx = with_image(ctx, 4000, 3000);
        let mut ui = Ui::new();
        let content = ui.content_rect(&ctx);
        let image = ctx.image.unwrap().screen_rect;
        assert!(checks_in(&frame(&mut ui, &ctx), content).is_empty());

        // Image 3 is on screen and selected: one badge on it.
        let mut selected = ctx;
        selected.selected = &[3];
        let list = frame(&mut ui, &selected);
        let badges = checks_in(&list, content);
        assert_eq!(badges.len(), 1, "scale {scale}");
        let mark = badges[0];
        // In the top right corner of the image, below the toolbar band.
        let visible = image.intersect(content).unwrap();
        assert!(visible.contains(mark.center()));
        assert!(mark.center().x > visible.center().x && mark.center().y < visible.center().y);
        assert!(mark.y >= 40.0 * scale, "{mark:?} under the toolbar");
        assert!(visible.right() - mark.right() < 40.0 * scale);
        // A round accent badge the size of the scale.
        let fill = fills_at(&list, mark.center(), theme.accent);
        assert!(
            fill.iter()
                .any(|r| r.w == 26.0 * scale && r.h == 26.0 * scale),
            "{fill:?}"
        );

        // Another selected image puts no badge on this one.
        let mut other = ctx;
        other.selected = &[2];
        assert!(checks_in(&frame(&mut ui, &other), content).is_empty());

        // A small image keeps the badge on its own corner.
        let mut small = with_image(ctx, 200, 100);
        small.selected = &[3];
        let list = frame(&mut ui, &small);
        let rect = small.image.unwrap().screen_rect;
        let badges = checks_in(&list, content);
        assert_eq!(badges.len(), 1);
        assert!(
            rect.contains(badges[0].center()),
            "{rect:?} {:?}",
            badges[0]
        );

        // Crop mode hides the badge, so the corner stays free to crop. The
        // crop bar at the top has a check mark of its own.
        let mut ui = Ui::new();
        press_key(&mut ui, &selected, Key::C);
        assert_eq!(ui.mode(), UiMode::Crop);
        let below_bar = Rect::new(
            content.x,
            content.y + 50.0 * scale,
            content.w,
            content.h - 50.0 * scale,
        );
        assert!(below_bar.contains(mark.center()));
        assert!(checks_in(&frame(&mut ui, &selected), below_bar).is_empty());
    }
}

#[test]
fn selection_count_is_shown() {
    let theme = Theme::for_kind(image_ctx().theme);
    let mut ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(!has_text(&list, "selected"), "nothing selected, no count");

    ctx.selected = &[1, 4, 7];
    let list = frame(&mut ui, &ctx);
    let count = text_rect_exact(&list, "3 selected").expect("the count shows");
    let position = text_rect_exact(&list, "4 / 10").expect("the position shows");
    // Beside the position, on the same toolbar row, left of it.
    assert_eq!(count.y, position.y);
    assert!(count.right() < position.x && position.x - count.right() < 30.0);
    assert!(count.bottom() <= 40.0);
    assert_eq!(text_color(&list, "3 selected"), Some(theme.accent));

    ctx.selected = &[9];
    assert!(has_text(&frame(&mut ui, &ctx), "1 selected"));

    // A file that failed to open shows the position and the count too.
    let mut failed = base_ctx();
    failed.selected = &[0, 3];
    let list = frame(&mut ui, &failed);
    let count = text_rect_exact(&list, "2 selected").expect("count without an image");
    let position = text_rect_exact(&list, "4 / 10").unwrap();
    assert!(count.right() < position.x);
}

#[test]
fn context_menu_offers_select_and_clear() {
    let ctx = tall_image_ctx();
    let theme = Theme::for_kind(ctx.theme);
    let p = Point::new(500.0, 50.0);
    let mut ui = Ui::new();

    // Nothing selected: "Select image" is unchecked, "Clear selection" is
    // there but disabled. Both name their keys.
    right_click(&mut ui, &ctx, p);
    let list = frame(&mut ui, &ctx);
    let select = text_rect_exact(&list, "Select image").expect("Select image offered");
    let clear = text_rect_exact(&list, "Clear selection").expect("Clear selection offered");
    let row = |r: Rect, keys: &str| {
        let k = text_rect_exact(&list, keys).unwrap_or_else(|| panic!("{keys} not shown"));
        (k.center().y - r.center().y).abs() < 14.0 && k.x > r.x
    };
    assert!(row(select, "Space") && row(clear, "Backspace"));
    assert_eq!(text_color(&list, "Select image"), Some(theme.text));
    assert_eq!(
        text_color(&list, "Clear selection"),
        Some(theme.text_disabled)
    );
    assert!(!checked(&list, select), "Select image is not checked");
    assert!(click_label(&mut ui, &ctx, "Clear selection").is_empty());
    assert_eq!(ui.mode(), UiMode::Modal, "a disabled entry keeps the menu");
    // "Select image" selects the image on screen.
    assert_eq!(
        click_label(&mut ui, &ctx, "Select image"),
        vec![Action::ToggleSelect(3)]
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // The image on screen selected: the entry is checked, and "Clear
    // selection" works.
    let mut selected = ctx;
    selected.selected = &[3, 8];
    right_click(&mut ui, &selected, p);
    let list = frame(&mut ui, &selected);
    let select = text_rect_exact(&list, "Select image").unwrap();
    assert!(checked(&list, select), "Select image is checked");
    assert_eq!(text_color(&list, "Clear selection"), Some(theme.text));
    assert_eq!(
        click_label(&mut ui, &selected, "Clear selection"),
        vec![Action::ClearSelection]
    );
    // Choosing "Select image" again unselects it.
    right_click(&mut ui, &selected, p);
    assert_eq!(
        click_label(&mut ui, &selected, "Select image"),
        vec![Action::ToggleSelect(3)]
    );

    // Another image selected: the entry is unchecked, clear still works.
    let mut other = ctx;
    other.selected = &[8];
    right_click(&mut ui, &other, p);
    let list = frame(&mut ui, &other);
    let select = text_rect_exact(&list, "Select image").unwrap();
    assert!(!checked(&list, select));
    assert_eq!(text_color(&list, "Clear selection"), Some(theme.text));
    press_key(&mut ui, &other, Key::Escape);

    // Crop mode offers neither.
    press_key(&mut ui, &selected, Key::C);
    right_click(&mut ui, &selected, p);
    let list = frame(&mut ui, &selected);
    assert!(text_rect_exact(&list, "Apply crop").is_some());
    assert!(!has_text(&list, "Select image") && !has_text(&list, "Clear selection"));
}

#[test]
fn space_ignored_in_text_field_dialog_and_crop() {
    let mut ctx = image_ctx();
    ctx.selected = &[3];
    let mut ui = Ui::new();

    // A text field has the focus: Space is text there, not a selection.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    let mut out = press_key(&mut ui, &ctx, Key::Space);
    out.extend(type_text(&mut ui, &ctx, " "));
    assert!(!has_toggle(&out), "{out:?}");
    assert_eq!(ui.mode(), UiMode::Modal, "the dialog stays open");
    press_key(&mut ui, &ctx, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);

    // A dialog without a focused field: Space does nothing.
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(!ui.wants_text_input());
    assert!(press_key(&mut ui, &ctx, Key::Space).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    // Its destination field focused: Space is text.
    for _ in 0..30 {
        if ui.wants_text_input() {
            break;
        }
        press_key(&mut ui, &ctx, Key::Tab);
    }
    assert!(ui.wants_text_input());
    let mut out = press_key(&mut ui, &ctx, Key::Space);
    out.extend(type_text(&mut ui, &ctx, " "));
    assert!(!has_toggle(&out), "{out:?}");
    press_key(&mut ui, &ctx, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);

    // A question: Space answers it with the focused button, never selects.
    ui.show_confirm(ConfirmRequest::new(4, "Sure?", "Are you sure?"));
    let out = press_key(&mut ui, &ctx, Key::Space);
    assert!(!has_toggle(&out), "{out:?}");
    assert_eq!(ui.mode(), UiMode::Normal);

    // The shortcut overlay: nothing.
    press_key(&mut ui, &ctx, Key::F1);
    assert!(press_key(&mut ui, &ctx, Key::Space).is_empty());
    press_key(&mut ui, &ctx, Key::Escape);

    // Crop mode: nothing, and crop mode stays.
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Crop);
    assert!(press_key(&mut ui, &ctx, Key::Space).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Normal);

    // Back to viewing, Space selects again.
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Space),
        vec![Action::ToggleSelect(3)]
    );
}

#[test]
fn backspace_clears_selection() {
    let mut ctx = image_ctx();
    ctx.selected = &[1, 3];
    let mut ui = Ui::new();
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Backspace),
        vec![Action::ClearSelection]
    );
    // A held key does not repeat it.
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Backspace,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());

    // Nothing selected: Backspace does nothing, and never goes back.
    let empty = image_ctx();
    for _ in 0..3 {
        assert!(press_key(&mut ui, &empty, Key::Backspace).is_empty());
    }
    let previous: Vec<String> = SHORTCUTS
        .iter()
        .filter(|s| s.command == Command::Previous)
        .map(|s| s.label())
        .collect();
    assert_eq!(previous, ["Left"]);
    assert_eq!(
        Command::ClearSelection.shortcut_label().as_deref(),
        Some("Backspace")
    );

    // In a text field Backspace deletes the character before the caret
    // and the selection stays.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    type_text(&mut ui, &ctx, "123");
    let out = press_key(&mut ui, &ctx, Key::Backspace);
    assert!(
        !kinds(&out).contains(&ActionKind::ClearSelection),
        "{out:?}"
    );
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "12").is_some());
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);

    // A dialog without a focused field, the overlay and crop mode: nothing.
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(press_key(&mut ui, &ctx, Key::Backspace).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::F1);
    assert!(press_key(&mut ui, &ctx, Key::Backspace).is_empty());
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::C);
    assert!(press_key(&mut ui, &ctx, Key::Backspace).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
    press_key(&mut ui, &ctx, Key::C);

    // Viewing again: Backspace clears.
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Backspace),
        vec![Action::ClearSelection]
    );
}

//! Gate G2: the selection. Space selects the image on screen, Backspace
//! clears the selection, and Shift+Left and Shift+Right select the image
//! on screen and move on. A selected thumbnail shows a small badge, the
//! image on screen a "Marked" label and the toolbar the count; the context
//! menu offers Select image and Clear selection. The pointer never changes
//! the selection, and Space and Backspace never navigate.

mod support;

use pandite_core::{DrawCmd, DrawList, Point, Rect, Size};
use pandite_ui::{
    Action, ActionKind, Command, ConfirmRequest, GallerySize, Icon, InputEvent, Key, Modifiers,
    PointerButton, Theme, ThumbState, Ui, UiContext, UiMode, SHORTCUTS,
};
use support::*;

/// A window wide enough for the first eight cells of the largest size at
/// scale 2.
fn gallery_ctx(count: usize, index: usize, thumbs: &'static [ThumbState]) -> UiContext<'static> {
    let wide = UiContext {
        window_size: Size::new(2600.0, 900.0),
        ..base_ctx()
    };
    let mut ctx = with_image(wide, 4000, 3000);
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

/// Check marks drawn with their center inside `area`.
fn checks_in(list: &DrawList, area: Rect) -> Vec<Rect> {
    icon_draws(list, Icon::Check)
        .into_iter()
        .chain(icon_draws(list, Icon::Mark))
        .map(|(r, _)| r)
        .filter(|r| area.contains(r.center()))
        .collect()
}

/// The top left quarter of a cell, where its badge sits.
fn corner(cell: Rect) -> Rect {
    Rect::new(cell.x, cell.y, cell.w * 0.5, cell.h * 0.5)
}

/// A logical length in physical pixels at `scale`, rounded as the
/// interface rounds it.
fn px(logical: f32, scale: f32) -> f32 {
    (logical * scale).round().max(1.0)
}

/// Fills of `color` that hold `p`, with their corner radius.
fn round_fills_at(list: &DrawList, p: Point, color: pandite_core::Color) -> Vec<(Rect, f32)> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect {
                rect,
                color: c,
                radius,
            } if *c == color && rect.contains(p) => Some((*rect, *radius)),
            _ => None,
        })
        .collect()
}

/// A selection marker inside `cell`: a fill in the badge color or an
/// outline. The ring of the current image lies around its cell, not
/// inside it.
fn marker_in(list: &DrawList, cell: Rect, theme: &Theme) -> Option<Rect> {
    list.cmds.iter().find_map(|c| match c {
        DrawCmd::Rect { rect, color, .. } if *color == theme.check_fill && inside(*rect, cell) => {
            Some(*rect)
        }
        DrawCmd::Border { rect, .. } if inside(*rect, cell) => Some(*rect),
        _ => None,
    })
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
fn gallery_draws_badge_for_selected() {
    let theme = Theme::for_kind(image_ctx().theme);
    // The badge keeps its size in every cell size.
    let sizes = GallerySize::ALL.into_iter();
    for (size, scale) in sizes.flat_map(|g| [1.0, 1.5, 2.0].map(|s| (g, s))) {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        ctx.gallery_size = size;
        ctx.selected = &[2, 5];
        let mut ui = Ui::new();
        // The pointer rests on the image, away from the strip.
        move_to(&mut ui, &ctx, Point::new(600.0, 200.0));
        let list = frame(&mut ui, &ctx);
        let (side, inset, line) = (px(14.0, scale), px(4.0, scale), px(1.5, scale));
        let drawn: Vec<usize> = thumbnail_draws(&list).iter().map(|d| d.0).collect();
        assert!(drawn.contains(&2) && drawn.contains(&5) && drawn.contains(&7));
        for i in drawn {
            let cell = cell_of(&list, i);
            let marks = checks_in(&list, corner(cell));
            if i == 2 || i == 5 {
                // A small round badge in the top left corner: a white
                // disc with a blue check mark, at the size of the scale.
                assert_eq!(marks.len(), 1, "thumbnail {i} at {size:?} {scale}");
                let mark = marks[0];
                assert_eq!(mark.w, side - px(2.0, scale));
                let disc = round_fills_at(&list, mark.center(), theme.check_fill);
                assert!(
                    disc.iter().any(|(r, radius)| r.w == side - line
                        && r.h == side - line
                        && *radius == (side - line) * 0.5
                        && (r.x - cell.x, r.y - cell.y)
                            == (inset + line * 0.5, inset + line * 0.5)),
                    "badge of thumbnail {i} at {size:?} {scale}: {disc:?}"
                );
            } else {
                assert!(marks.is_empty(), "thumbnail {i} is not selected");
                assert!(marker_in(&list, cell, theme).is_none(), "{i}");
            }
        }
        // Nothing else in the frame carries a check mark: the image on
        // screen (index 0) is not selected.
        assert!(icon_draws(&list, Icon::Check).is_empty());
        assert_eq!(icon_draws(&list, Icon::Mark).len(), 2);
        assert!(icon_draws(&list, Icon::Mark)
            .iter()
            .all(|(_, tint)| *tint == theme.check_mark));
        // The mark is blue and differs from the soft outline.
        assert!(theme.check_mark.b > theme.check_mark.r && theme.check_mark != theme.check_line);
    }
}

#[test]
fn gallery_no_checkbox_on_unselected() {
    let theme = Theme::for_kind(image_ctx().theme);
    let sizes = GallerySize::ALL.into_iter();
    for (size, scale) in sizes.flat_map(|g| [1.0, 1.5, 2.0].map(|s| (g, s))) {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        ctx.gallery_size = size;
        ctx.selected = &[5];
        let mut ui = Ui::new();
        let list = frame(&mut ui, &ctx);
        let cells: Vec<(usize, Rect)> = [0, 3, 4, 6]
            .into_iter()
            .map(|i| (i, cell_of(&list, i)))
            .collect();
        let bare = |ui: &mut Ui, ctx: &UiContext<'_>, why: &str| {
            let list = frame(ui, ctx);
            for (i, cell) in &cells {
                assert!(
                    marker_in(&list, *cell, theme).is_none(),
                    "thumbnail {i} at {size:?} {scale}, {why}"
                );
                assert!(checks_in(&list, *cell).is_empty(), "{i} {why}");
            }
            // The selected thumbnail keeps its badge all the time.
            assert!(
                marker_in(&list, cell_of(&list, 5), theme).is_some(),
                "{why}"
            );
        };
        bare(&mut ui, &ctx, "pointer away");
        for (i, cell) in cells.clone() {
            move_to(&mut ui, &ctx, cell.center());
            bare(&mut ui, &ctx, &format!("pointer on the middle of {i}"));
            let corner_point = Point::new(cell.x + px(6.0, scale), cell.y + px(6.0, scale));
            move_to(&mut ui, &ctx, corner_point);
            bare(&mut ui, &ctx, &format!("pointer on the corner of {i}"));
        }
        // Neither in crop mode.
        press_key(&mut ui, &ctx, Key::C);
        assert_eq!(ui.mode(), UiMode::Crop);
        move_to(&mut ui, &ctx, cells[1].1.center());
        bare(&mut ui, &ctx, "crop mode");
    }
}

#[test]
fn gallery_click_never_changes_selection() {
    let theme = Theme::for_kind(image_ctx().theme);
    let sizes = GallerySize::ALL.into_iter();
    for (size, scale) in sizes.flat_map(|g| [1.0, 1.25, 1.5, 2.0].map(|s| (g, s))) {
        let mut ctx = gallery_ctx(20, 0, ready(20));
        ctx.scale_factor = scale;
        ctx.gallery_size = size;
        ctx.selected = &[0, 5];
        let mut ui = Ui::new();
        let list = frame(&mut ui, &ctx);
        let (c5, c6) = (cell_of(&list, 5), cell_of(&list, 6));
        let badge = marker_in(&list, c5, theme).expect("thumbnail 5 has its badge");

        // A click on the badge of a selected thumbnail shows that image and
        // leaves the selection alone.
        let out = click(&mut ui, &ctx, badge.center());
        assert_eq!(out, vec![Action::GoTo(5)], "{size:?} {scale}");
        // The same corner of an unselected thumbnail: the same.
        let same = Point::new(
            c6.x + (badge.center().x - c5.x),
            c6.y + (badge.center().y - c5.y),
        );
        assert_eq!(click(&mut ui, &ctx, same), vec![Action::GoTo(6)]);
        assert_eq!(
            click(&mut ui, &ctx, Point::new(c6.x + 1.0, c6.y + 1.0)),
            vec![Action::GoTo(6)]
        );
        assert_eq!(click(&mut ui, &ctx, c5.center()), vec![Action::GoTo(5)]);
        // The badge of the image on screen: no selection either.
        let c0 = cell_of(&list, 0);
        let b0 = marker_in(&list, c0, theme).expect("thumbnail 0 has its badge");
        let out = click(&mut ui, &ctx, b0.center());
        assert_eq!(kinds(&out), [ActionKind::GoTo]);

        // The badge takes no hover highlight and the pointer is the hand of
        // any thumbnail.
        move_to(&mut ui, &ctx, badge.center());
        assert_eq!(ui.cursor(), pandite_ui::CursorShape::Hand);
        let list = frame(&mut ui, &ctx);
        assert!(!list.cmds.iter().any(|c| matches!(
            c,
            DrawCmd::Rect { rect, color, .. } if *color == theme.hover && *rect == badge
        )));
    }

    // A drag that starts on a badge scrolls the strip and selects nothing.
    let mut ctx = gallery_ctx(200, 0, ready(200));
    ctx.selected = &[12];
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let badge = marker_in(&list, cell_of(&list, 12), theme).unwrap();
    let out = drag(
        &mut ui,
        &ctx,
        badge.center(),
        Point::new(badge.center().x - 400.0, badge.center().y),
        PointerButton::Left,
    );
    assert!(out.is_empty(), "{out:?}");
    assert!(ui.gallery_visible_range(&ctx).start > 0);
}

#[test]
fn shift_arrows_select_then_move() {
    const SHIFT: Modifiers = Modifiers::SHIFT;
    let ctx = image_ctx();
    let mut ui = Ui::new();
    // Select the image on screen (index 3), then move; never a toggle.
    assert_eq!(
        key(&mut ui, &ctx, Key::Right, SHIFT),
        vec![Action::Select(3), Action::Next]
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::Left, SHIFT),
        vec![Action::Select(3), Action::Previous]
    );
    let mut selected = ctx;
    selected.selected = &[3];
    assert_eq!(
        key(&mut ui, &selected, Key::Right, SHIFT),
        vec![Action::Select(3), Action::Next],
        "a selected image stays selected while the run selects"
    );
    // Shift goes up and down again on a selected image: this run
    // unselects, and it leaves an unselected image alone.
    ui.handle_event(InputEvent::ModifiersChanged(Modifiers::NONE), &ctx);
    assert_eq!(
        key(&mut ui, &selected, Key::Right, SHIFT),
        vec![Action::ToggleSelect(3), Action::Next]
    );
    assert_eq!(key(&mut ui, &ctx, Key::Left, SHIFT), vec![Action::Previous]);
    assert_eq!(
        key(&mut ui, &selected, Key::Left, SHIFT),
        vec![Action::ToggleSelect(3), Action::Previous]
    );
    // The next run starts on an unselected image and selects again.
    ui.handle_event(InputEvent::ModifiersChanged(Modifiers::NONE), &ctx);
    // A held key repeats, so a run of images gets selected.
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Right,
            mods: SHIFT,
            repeat: true,
        },
        &ctx,
    );
    assert_eq!(held, vec![Action::Select(3), Action::Next]);
    // At an end of the folder: the selection only, no move and no wrap.
    let mut last = ctx;
    last.index = 9;
    assert_eq!(
        key(&mut ui, &last, Key::Right, SHIFT),
        vec![Action::Select(9)]
    );
    let mut first = ctx;
    first.index = 0;
    assert_eq!(
        key(&mut ui, &first, Key::Left, SHIFT),
        vec![Action::Select(0)]
    );
    let mut one = ctx;
    one.count = 1;
    one.index = 0;
    assert_eq!(
        key(&mut ui, &one, Key::Right, SHIFT),
        vec![Action::Select(0)]
    );
    // A file that failed to open is selected as well; nothing without one.
    assert_eq!(
        key(&mut ui, &base_ctx(), Key::Right, SHIFT),
        vec![Action::Select(3), Action::Next]
    );
    let mut none = base_ctx();
    none.count = 0;
    assert!(key(&mut ui, &none, Key::Right, SHIFT).is_empty());
    // The keys are in the table and the shortcut overlay, not in the menu.
    assert_eq!(
        Command::SelectNext.shortcut_label().as_deref(),
        Some("Shift+Right")
    );
    assert_eq!(
        Command::SelectPrevious.shortcut_label().as_deref(),
        Some("Shift+Left")
    );
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    assert!(text_rect_exact(&list, "Select and next image").is_some());
    assert!(text_rect_exact(&list, "Select and previous image").is_some());
    press_key(&mut ui, &ctx, Key::Escape);
    let tall = tall_image_ctx();
    right_click(&mut ui, &tall, Point::new(500.0, 50.0));
    let list = frame(&mut ui, &tall);
    assert!(!has_text(&list, "Select and"));
    press_key(&mut ui, &tall, Key::Escape);

    // In a text field Shift+arrows select text; in a dialog they do
    // nothing to the images.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    for k in [Key::Right, Key::Left] {
        let out = key(&mut ui, &ctx, k, SHIFT);
        assert!(!kinds(&out).contains(&ActionKind::Select), "{out:?}");
        assert!(!kinds(&out).contains(&ActionKind::Next));
    }
    press_key(&mut ui, &ctx, Key::Escape);
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(key(&mut ui, &ctx, Key::Right, SHIFT).is_empty());
    assert!(key(&mut ui, &ctx, Key::Left, SHIFT).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);

    // Crop mode keeps their meaning: they move the crop 10 pixels.
    let crop = |shift_right: bool| {
        let mut ui = Ui::new();
        press_key(&mut ui, &ctx, Key::C);
        drag(
            &mut ui,
            &ctx,
            Point::new(400.0, 200.0),
            Point::new(600.0, 400.0),
            PointerButton::Left,
        );
        if shift_right {
            assert!(key(&mut ui, &ctx, Key::Right, SHIFT).is_empty());
            assert_eq!(ui.mode(), UiMode::Crop);
        }
        match press_key(&mut ui, &ctx, Key::Enter).as_slice() {
            [Action::ApplyCrop(r)] => *r,
            other => panic!("{other:?}"),
        }
    };
    let (before, after) = (crop(false), crop(true));
    assert_eq!(after.y, before.y);
    assert_eq!((after.width, after.height), (before.width, before.height));
    assert_eq!(
        after.x,
        before.x + 10,
        "Shift+Right moved the crop 10 pixels"
    );
    // Without a crop rectangle, Shift+Right selects nothing in crop mode.
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    assert!(key(&mut ui, &ctx, Key::Right, SHIFT).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
}

#[test]
fn selected_image_shows_badge() {
    let theme = Theme::for_kind(image_ctx().theme);
    for scale in [1.0, 1.5, 2.0] {
        let mut ctx = image_ctx();
        ctx.scale_factor = scale;
        ctx.window_size = Size::new(1280.0 * scale, 800.0 * scale);
        let ctx = with_image(ctx, 4000, 3000);
        let mut ui = Ui::new();
        let content = ui.content_rect(&ctx);
        let image = ctx.image.unwrap().screen_rect;
        let list = frame(&mut ui, &ctx);
        assert!(text_rect_exact(&list, "Marked").is_none(), "not selected");

        // Image 3 is on screen and selected: a "Marked" pill on it.
        let mut selected = ctx;
        selected.selected = &[3];
        let list = frame(&mut ui, &selected);
        let label = text_rect_exact(&list, "Marked").expect("the pill shows");
        let pill = round_fills_at(&list, label.center(), theme.check_fill);
        let (pill, radius) = *pill
            .iter()
            .find(|(r, _)| r.h == px(20.0, scale) - px(1.5, scale))
            .unwrap_or_else(|| panic!("a pill fill at scale {scale}: {pill:?}"));
        // Rounded at its full height, and its fill (inside the outline) holds its label with
        // the padding on both sides, at every scale.
        assert_eq!(radius, pill.h * 0.5);
        assert!(inside(label, pill), "{label:?} in {pill:?}");
        assert!(
            pill.w >= label.w + 2.0 * px(8.0, scale) - 1.0 - px(1.5, scale),
            "{pill:?}"
        );
        assert_eq!(text_color(&list, "Marked"), Some(theme.check_mark));
        // White, a little translucent, with a blue label.
        assert_eq!(
            theme.check_fill,
            pandite_core::Color::rgba(255, 255, 255, 235)
        );
        // In the bottom left corner of the image.
        let visible = image.intersect(content).unwrap();
        assert!(inside(pill, visible));
        assert!(pill.center().x < visible.center().x && pill.center().y > visible.center().y);
        assert!(pill.x - visible.x < 40.0 * scale);
        assert!(visible.bottom() - pill.bottom() < 40.0 * scale);
        // A label, not a check mark.
        assert!(checks_in(&list, content).is_empty());

        // Another selected image puts no label on this one.
        let mut other = ctx;
        other.selected = &[2];
        let list = frame(&mut ui, &other);
        assert!(text_rect_exact(&list, "Marked").is_none());
        assert!(checks_in(&list, content).is_empty());

        // A small image keeps the pill on its own corner.
        let mut small = with_image(ctx, 400, 200);
        small.selected = &[3];
        let list = frame(&mut ui, &small);
        let rect = small.image.unwrap().screen_rect;
        let label = text_rect_exact(&list, "Marked").expect("the pill shows");
        assert!(inside(label, rect), "{rect:?} {label:?}");

        // Crop mode hides the pill, so the corner stays free to crop.
        let mut ui = Ui::new();
        press_key(&mut ui, &selected, Key::C);
        assert_eq!(ui.mode(), UiMode::Crop);
        assert!(text_rect_exact(&frame(&mut ui, &selected), "Marked").is_none());
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

#[test]
fn move_command_in_menu_and_shortcuts() {
    let ctx = tall_image_ctx();
    let theme = Theme::for_kind(ctx.theme);
    let mut ui = Ui::new();
    // M asks to move, with or without a selection; a held key does not
    // repeat it.
    assert_eq!(press_key(&mut ui, &ctx, Key::M), vec![Action::MoveTo]);
    let mut selected = ctx;
    selected.selected = &[1, 2];
    assert_eq!(press_key(&mut ui, &selected, Key::M), vec![Action::MoveTo]);
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::M,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());
    assert_eq!(Command::MoveTo.label(), "Move to...");
    assert_eq!(Command::MoveTo.shortcut_label().as_deref(), Some("M"));
    assert_eq!(Command::MoveTo.tooltip(), "Move to  (M)");

    // The context menu offers it next to the delete entries, with its key.
    let p = Point::new(500.0, 50.0);
    right_click(&mut ui, &ctx, p);
    let list = frame(&mut ui, &ctx);
    let entry = text_rect_exact(&list, "Move to...").expect("Move to... offered");
    let trash = text_rect_exact(&list, Command::MoveToTrash.label()).unwrap();
    let clear = text_rect_exact(&list, "Clear selection").unwrap();
    assert!(
        clear.y < entry.y && entry.y < trash.y,
        "between the selection and delete entries"
    );
    let key_m = text_rect_exact(&list, "M").expect("the key shows");
    assert!((key_m.center().y - entry.center().y).abs() < 14.0);
    assert_eq!(text_color(&list, "Move to..."), Some(theme.text));
    assert_eq!(
        click_label(&mut ui, &ctx, "Move to..."),
        vec![Action::MoveTo]
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // The shortcut overlay lists it.
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    let label = text_rect_exact(&list, "Move to...").expect("listed in the overlay");
    let key_m = text_rect_exact(&list, "M").unwrap();
    assert!((key_m.center().y - label.center().y).abs() < 11.0 && key_m.x < label.x);
    press_key(&mut ui, &ctx, Key::Escape);

    // Without an image it is grayed and does nothing.
    let mut empty = base_ctx();
    empty.count = 0;
    let mut ui = Ui::new();
    assert!(press_key(&mut ui, &empty, Key::M).is_empty());
    right_click(&mut ui, &empty, Point::new(500.0, 300.0));
    let list = frame(&mut ui, &empty);
    assert_eq!(text_color(&list, "Move to..."), Some(theme.text_disabled));
    assert!(click_label(&mut ui, &empty, "Move to...").is_empty());
    assert_eq!(ui.mode(), UiMode::Modal, "a disabled entry keeps the menu");
}

#[test]
fn move_key_ignored_in_text_field_dialog_and_crop() {
    let mut ctx = image_ctx();
    ctx.selected = &[3];
    let mut ui = Ui::new();
    let moves = |out: &[Action]| kinds(out).contains(&ActionKind::MoveTo);

    // A text field: M is a letter there.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    let mut out = press_key(&mut ui, &ctx, Key::M);
    out.extend(type_text(&mut ui, &ctx, "m"));
    assert!(!moves(&out), "{out:?}");
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);

    // A dialog without a focused field, a question, the overlay and an
    // open menu: nothing moves.
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(press_key(&mut ui, &ctx, Key::M).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);
    ui.show_confirm(ConfirmRequest::new(9, "Sure?", "Are you sure?"));
    assert!(!moves(&press_key(&mut ui, &ctx, Key::M)));
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::F1);
    assert!(press_key(&mut ui, &ctx, Key::M).is_empty());
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::ContextMenu);
    assert!(press_key(&mut ui, &ctx, Key::M).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal, "the key closed the menu only");

    // Crop mode: nothing, and crop mode stays.
    press_key(&mut ui, &ctx, Key::C);
    assert!(press_key(&mut ui, &ctx, Key::M).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
    press_key(&mut ui, &ctx, Key::C);

    // Viewing again: M moves.
    assert_eq!(press_key(&mut ui, &ctx, Key::M), vec![Action::MoveTo]);
}

#[test]
fn copy_command_in_menu_and_shortcuts() {
    let ctx = tall_image_ctx();
    let theme = Theme::for_kind(ctx.theme);
    let shift = Modifiers::SHIFT;
    let mut ui = Ui::new();
    // Shift+M asks to copy, with or without a selection; M alone still
    // moves; a held key does not repeat it.
    assert_eq!(key(&mut ui, &ctx, Key::M, shift), vec![Action::CopyTo]);
    let mut selected = ctx;
    selected.selected = &[1, 2];
    assert_eq!(key(&mut ui, &selected, Key::M, shift), vec![Action::CopyTo]);
    assert_eq!(press_key(&mut ui, &selected, Key::M), vec![Action::MoveTo]);
    for mods in [Modifiers::CTRL, Modifiers::ALT, Modifiers::CTRL_SHIFT] {
        assert!(key(&mut ui, &ctx, Key::M, mods).is_empty(), "{mods:?}");
    }
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::M,
            mods: shift,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());
    assert_eq!(Command::CopyTo.label(), "Copy to...");
    assert_eq!(Command::CopyTo.shortcut_label().as_deref(), Some("Shift+M"));
    assert_eq!(Command::CopyTo.tooltip(), "Copy to  (Shift+M)");
    assert!(!Command::CopyTo.repeats() && !Command::CopyTo.allowed_in_crop());

    // The context menu offers it directly after Move to, with its key.
    right_click(&mut ui, &ctx, Point::new(500.0, 50.0));
    let list = frame(&mut ui, &ctx);
    let entry = text_rect_exact(&list, "Copy to...").expect("Copy to... offered");
    let moves = text_rect_exact(&list, "Move to...").unwrap();
    let trash = text_rect_exact(&list, Command::MoveToTrash.label()).unwrap();
    assert!(
        moves.y < entry.y && entry.y < trash.y,
        "between Move to and the delete entries"
    );
    assert!(
        ((entry.y - moves.y) - (trash.y - entry.y)).abs() < 0.5,
        "no entry or separator between Move to and Copy to"
    );
    let keys = text_rect_exact(&list, "Shift+M").expect("the key shows");
    assert!((keys.center().y - entry.center().y).abs() < 14.0);
    assert_eq!(text_color(&list, "Copy to..."), Some(theme.text));
    assert_eq!(
        click_label(&mut ui, &ctx, "Copy to..."),
        vec![Action::CopyTo]
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // The shortcut overlay lists it directly after Move to.
    let at = |cmd| Command::ALL.iter().position(|c| *c == cmd).unwrap();
    assert_eq!(at(Command::CopyTo), at(Command::MoveTo) + 1);
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    let label = text_rect_exact(&list, "Copy to...").expect("listed in the overlay");
    let keys = text_rect_exact(&list, "Shift+M").unwrap();
    assert!((keys.center().y - label.center().y).abs() < 11.0 && keys.x < label.x);
    let moves = text_rect_exact(&list, "Move to...").unwrap();
    assert!(
        (label.x - moves.x).abs() < 0.5 && label.y > moves.y,
        "the next row of the same column"
    );
    press_key(&mut ui, &ctx, Key::Escape);

    // Without an image it is grayed and does nothing.
    let mut empty = base_ctx();
    empty.count = 0;
    let mut ui = Ui::new();
    assert!(key(&mut ui, &empty, Key::M, shift).is_empty());
    right_click(&mut ui, &empty, Point::new(500.0, 300.0));
    let list = frame(&mut ui, &empty);
    assert_eq!(text_color(&list, "Copy to..."), Some(theme.text_disabled));
    assert!(click_label(&mut ui, &empty, "Copy to...").is_empty());
    assert_eq!(ui.mode(), UiMode::Modal, "a disabled entry keeps the menu");
}

#[test]
fn copy_key_ignored_in_text_field_dialog_and_crop() {
    let mut ctx = image_ctx();
    ctx.selected = &[3];
    let shift = Modifiers::SHIFT;
    let mut ui = Ui::new();
    let copies = |out: &[Action]| kinds(out).contains(&ActionKind::CopyTo);

    // A text field: Shift+M is a capital letter there.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    let mut out = key(&mut ui, &ctx, Key::M, shift);
    out.extend(type_text(&mut ui, &ctx, "M"));
    assert!(!copies(&out), "{out:?}");
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);

    // A dialog without a focused field, a question, the overlay and an
    // open menu: nothing is copied.
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(key(&mut ui, &ctx, Key::M, shift).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);
    ui.show_confirm(ConfirmRequest::new(9, "Sure?", "Are you sure?"));
    assert!(!copies(&key(&mut ui, &ctx, Key::M, shift)));
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::F1);
    assert!(key(&mut ui, &ctx, Key::M, shift).is_empty());
    press_key(&mut ui, &ctx, Key::Escape);
    press_key(&mut ui, &ctx, Key::ContextMenu);
    assert!(key(&mut ui, &ctx, Key::M, shift).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal, "the key closed the menu only");

    // Crop mode: nothing, and crop mode stays.
    press_key(&mut ui, &ctx, Key::C);
    assert!(key(&mut ui, &ctx, Key::M, shift).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
    press_key(&mut ui, &ctx, Key::C);

    // Viewing again: Shift+M copies.
    assert_eq!(key(&mut ui, &ctx, Key::M, shift), vec![Action::CopyTo]);
}

#[test]
fn selection_keys_show_the_toolbar() {
    let ctx = image_ctx();
    let shown =
        |ui: &mut Ui, t: u64| !icon_draws(&frame(ui, &at(ctx, ms(t))), Icon::Open).is_empty();
    for (k, mods) in [
        (Key::Space, Modifiers::NONE),
        (Key::Backspace, Modifiers::NONE),
        (Key::Right, Modifiers::SHIFT),
    ] {
        let mut ui = Ui::new();
        assert!(shown(&mut ui, 0));
        // Long idle: the toolbar is gone.
        assert!(!shown(&mut ui, 30_000), "{k:?}");
        assert!(!shown(&mut ui, 30_100), "{k:?}");
        // The key brings it back, and it fades after the idle time.
        let mut selected = at(ctx, ms(30_200));
        selected.selected = &[3];
        key(&mut ui, &selected, k, mods);
        assert!(shown(&mut ui, 30_300), "{k:?}");
        assert!(shown(&mut ui, 30_900), "{k:?}");
        assert!(!shown(&mut ui, 60_000), "{k:?}");
        assert!(!shown(&mut ui, 60_100), "{k:?}");
    }
}

//! Edge cases: tiny and empty windows, odd scale factors, clocks that jump,
//! long names, one or zero images, and modal behavior.

mod support;

use pandite_core::{DrawCmd, DrawList, Point, Rect, Size};
use pandite_ui::{
    Action, Command, ConfirmRequest, Icon, InputEvent, Key, Modifiers, PointerButton, Ui,
    UiContext, UiMode, WheelDelta,
};
use support::*;

/// Every drawn rectangle and text origin, for bounds checks.
fn extents(list: &DrawList) -> Vec<Rect> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect { rect, .. } | DrawCmd::Border { rect, .. } => Some(*rect),
            DrawCmd::Image { rect, .. } => Some(*rect),
            DrawCmd::Text { pos, .. } => Some(Rect::new(pos.x, pos.y, 0.0, 0.0)),
            _ => None,
        })
        .collect()
}

fn all_finite(list: &DrawList) -> bool {
    extents(list)
        .iter()
        .all(|r| r.x.is_finite() && r.y.is_finite() && r.w.is_finite() && r.h.is_finite())
}

/// A burst of every kind of input at a few positions.
fn storm(ui: &mut Ui, ctx: &UiContext<'_>) {
    let pts = [
        Point::new(0.0, 0.0),
        Point::new(5.0, 5.0),
        Point::new(-10.0, 3000.0),
        Point::new(ctx.window_size.w * 0.5, ctx.window_size.h * 0.5),
    ];
    for p in pts {
        for button in [
            PointerButton::Left,
            PointerButton::Right,
            PointerButton::Middle,
        ] {
            drag(ui, ctx, p, Point::new(p.x + 7.0, p.y + 3.0), button);
        }
        wheel(ui, ctx, p, 2.0);
        ui.handle_event(
            InputEvent::Wheel {
                pos: p,
                delta: WheelDelta::Pixels {
                    x: f32::NAN,
                    y: 1.0e30,
                },
            },
            ctx,
        );
    }
    for k in [
        Key::Tab,
        Key::Enter,
        Key::Escape,
        Key::Down,
        Key::F1,
        Key::F1,
    ] {
        press_key(ui, ctx, k);
    }
    ui.handle_event(InputEvent::Text("12x".into()), ctx);
    ui.handle_event(InputEvent::FocusLost, ctx);
    ui.handle_event(InputEvent::PointerLeft, ctx);
    ui.handle_event(InputEvent::Tick, ctx);
}

#[test]
fn degenerate_windows_and_scales_do_not_break() {
    for (w, h, scale) in [
        (0.0, 0.0, 1.0),
        (1.0, 1.0, 1.0),
        (120.0, 90.0, 1.0),
        (300.0, 200.0, 2.5),
        (1280.0, 800.0, f32::NAN),
        (1280.0, 800.0, 0.0),
        (1280.0, 800.0, 100.0),
        (-50.0, 800.0, 1.0),
    ] {
        let mut ctx = image_ctx();
        ctx.window_size = Size::new(w, h);
        ctx.scale_factor = scale;
        let mut ui = Ui::new();
        storm(&mut ui, &ctx);
        key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
        let list = frame(&mut ui, &ctx);
        assert!(all_finite(&list), "{w} x {h} at {scale}");
        press_key(&mut ui, &ctx, Key::Escape);
        key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
        assert!(all_finite(&frame(&mut ui, &ctx)));
        press_key(&mut ui, &ctx, Key::Escape);
        press_key(&mut ui, &ctx, Key::C);
        storm(&mut ui, &ctx);
        assert!(all_finite(&frame(&mut ui, &ctx)));
        let c = ui.content_rect(&ctx);
        assert!(c.w >= 0.0 && c.h >= 0.0);
    }
}

#[test]
fn zero_and_one_image() {
    // No folder at all.
    let mut none = base_ctx();
    none.count = 0;
    none.index = 7;
    let mut ui = Ui::new();
    storm(&mut ui, &none);
    let list = frame(&mut ui, &none);
    assert!(thumbnail_draws(&list).is_empty());
    assert_eq!(ui.gallery_visible_range(&none), 0..0);
    for k in [Key::Right, Key::Left, Key::Home, Key::End, Key::Space] {
        assert!(press_key(&mut ui, &none, k).is_empty(), "{k:?}");
    }
    // One image: navigation is off, the strip shows the single item.
    let mut one = image_ctx();
    one.count = 1;
    one.index = 0;
    let mut ui = Ui::new();
    let list = frame(&mut ui, &one);
    for k in [Key::Right, Key::Left, Key::Home, Key::End] {
        assert!(press_key(&mut ui, &one, k).is_empty(), "{k:?}");
    }
    assert_eq!(ui.gallery_visible_range(&one), 0..1);
    assert_eq!(icon_draws(&list, Icon::Picture).len(), 1);
    // An index past the end does not panic and shows the last position.
    one.index = 40;
    let list = frame(&mut ui, &one);
    assert!(has_text(&list, "1 / 1"));
}

#[test]
fn time_may_jump_backwards() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    frame(&mut ui, &at(ctx, ms(50_000)));
    ui.toast("hello", ms(50_000));
    move_to(&mut ui, &at(ctx, ms(10)), Point::new(300.0, 300.0));
    let list = frame(&mut ui, &at(ctx, ms(5)));
    assert!(all_finite(&list));
    frame(&mut ui, &at(ctx, ms(60_000)));
}

#[test]
fn long_names_stay_inside_their_space() {
    let long: &'static str =
        Box::leak(format!("{}.jpeg", "very-long-file-name-".repeat(40)).into_boxed_str());
    let mut ctx = image_ctx();
    ctx.image.as_mut().unwrap().file_name = long;
    ctx.notice = Some(long);
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let shown = texts(&list)
        .into_iter()
        .find(|t| t.ends_with(".jpeg") && t.contains("..."))
        .expect("name elided in the status");
    let r = text_rect_exact(&list, &shown).unwrap();
    assert!(r.right() <= 1280.0 && r.x >= 0.0);
    // The notice wraps to at most three lines inside the window.
    let notice_lines: Vec<String> = texts(&list)
        .into_iter()
        .filter(|t| t.starts_with("very-long") || t.contains("name-very"))
        .collect();
    assert!(!notice_lines.is_empty() && notice_lines.len() <= 4);
    for line in &notice_lines {
        let r = text_rect_exact(&list, line).unwrap();
        assert!(r.right() <= 1280.0, "{line}");
    }
}

#[test]
fn dialogs_fit_small_windows_and_scroll() {
    let mut ctx = image_ctx();
    ctx.window_size = Size::new(360.0, 260.0);
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    let list = frame(&mut ui, &ctx);
    // The dialog is clipped to the window.
    let panel = list
        .cmds
        .iter()
        .find_map(|c| match c {
            DrawCmd::Clip(Some(r)) => Some(*r),
            _ => None,
        })
        .expect("dialog clip");
    assert!(panel.x >= 0.0 && panel.right() <= 360.0);
    assert!(panel.y >= 0.0 && panel.bottom() <= 260.0);
    assert!(
        !has_text(&list, "If file exists"),
        "lower rows start hidden"
    );
    // Tabbing to a lower control scrolls it into view.
    for _ in 0..11 {
        press_key(&mut ui, &ctx, Key::Tab);
    }
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "If file exists"), "{:?}", texts(&list));
    press_key(&mut ui, &ctx, Key::Tab);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Cancel") && has_text(&list, "Run"));
    // The wheel scrolls as well, and the modal ignores the image below.
    for _ in 0..20 {
        assert!(wheel(&mut ui, &ctx, panel.center(), -3.0).is_empty());
    }
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Run"));
    assert!(click(&mut ui, &ctx, Point::new(2.0, 2.0)).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
}

#[test]
fn modal_layers_block_input_below() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    // Shortcuts, gallery clicks, wheel zoom and toolbar clicks are blocked.
    assert!(press_key(&mut ui, &ctx, Key::Right).is_empty());
    assert!(press_key(&mut ui, &ctx, Key::H).is_empty());
    assert!(wheel(&mut ui, &ctx, Point::new(100.0, 300.0), 1.0).is_empty());
    assert!(click(&mut ui, &ctx, Point::new(24.0, 20.0)).is_empty());
    assert!(click(&mut ui, &ctx, Point::new(640.0, 760.0)).is_empty());
    assert!(ui
        .handle_event(
            InputEvent::PointerDown {
                pos: Point::new(640.0, 400.0),
                button: PointerButton::Back,
            },
            &ctx,
        )
        .is_empty());
    // A confirm stacks on top and answers first; queued requests follow.
    ui.show_confirm(ConfirmRequest::new(1, "First?", "One"));
    ui.show_confirm(ConfirmRequest::new(2, "Second?", "Two"));
    assert!(has_text(&frame(&mut ui, &ctx), "First?"));
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::ConfirmResult {
            id: 1,
            accepted: false
        }]
    );
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Second?"));
    assert_eq!(
        click_label(&mut ui, &ctx, "OK"),
        vec![Action::ConfirmResult {
            id: 2,
            accepted: true
        }]
    );
    // The resize dialog is still there underneath.
    assert!(has_text(&frame(&mut ui, &ctx), "Resize image"));
    press_key(&mut ui, &ctx, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);
    // Side buttons navigate outside modal layers.
    let back = ui.handle_event(
        InputEvent::PointerDown {
            pos: Point::new(640.0, 400.0),
            button: PointerButton::Back,
        },
        &ctx,
    );
    assert_eq!(back, vec![Action::Previous]);
}

#[test]
fn tooltip_hides_after_click_and_under_dialogs() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let (r, _) = icon_draws(&list, Icon::Resize)[0];
    move_to(&mut ui, &ctx, r.center());
    assert_eq!(
        click(&mut ui, &at(ctx, ms(100)), r.center()),
        vec![Action::OpenResizeDialog]
    );
    let list = frame(&mut ui, &at(ctx, ms(3000)));
    assert!(!has_text(&list, &Command::Resize.tooltip()));
    press_key(&mut ui, &at(ctx, ms(3100)), Key::Escape);
    let list = frame(&mut ui, &at(ctx, ms(4000)));
    assert!(
        !has_text(&list, &Command::Resize.tooltip()),
        "no tooltip until the pointer moves to another target"
    );
}

#[test]
fn help_overlay_lists_shortcuts() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert!(press_key(&mut ui, &ctx, Key::F1).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Keyboard shortcuts"));
    for keys in ["F11", "Ctrl+Shift+S / Ctrl+S", "Shift+R / L", "Esc"] {
        assert!(has_text(&list, keys), "{keys} listed");
    }
    assert!(
        press_key(&mut ui, &ctx, Key::Right).is_empty(),
        "overlay is modal"
    );
    press_key(&mut ui, &ctx, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);
}

#[test]
fn help_overlay_scrolls_when_it_does_not_fit() {
    let theme = pandite_ui::Theme::for_kind(image_ctx().theme);
    let tracks = |list: &DrawList| -> Vec<Rect> {
        list.cmds
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Rect { rect, color, .. }
                    if *color == theme.gallery_marker && rect.h > rect.w =>
                {
                    Some(*rect)
                }
                _ => None,
            })
            .collect()
    };

    // A tall window fits every row: no indicator.
    let ctx = image_ctx();
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Press Esc to close"));
    assert!(!has_text(&list, "Scroll for more"));
    assert!(tracks(&list).is_empty());

    // A low window: the rows scroll with the wheel, and a marker shows where.
    let mut low = image_ctx();
    low.window_size = Size::new(400.0, 300.0);
    let mut ui = Ui::new();
    press_key(&mut ui, &low, Key::F1);
    let first = frame(&mut ui, &low);
    assert!(has_text(&first, "Scroll for more"));
    let top = tracks(&first);
    assert_eq!(top.len(), 1, "one marker");
    wheel(&mut ui, &low, Point::new(200.0, 150.0), -3.0);
    let moved = frame(&mut ui, &low);
    assert_ne!(texts(&first), texts(&moved), "other rows show");
    assert!(tracks(&moved)[0].y > top[0].y, "the marker moves down");
    // To the end and back to the top; bad input changes nothing.
    wheel(&mut ui, &low, Point::new(200.0, 150.0), -1000.0);
    let end = tracks(&frame(&mut ui, &low))[0];
    ui.handle_event(
        InputEvent::Wheel {
            pos: Point::new(200.0, 150.0),
            delta: WheelDelta::Pixels {
                x: 0.0,
                y: f32::NAN,
            },
        },
        &low,
    );
    assert_eq!(tracks(&frame(&mut ui, &low))[0], end);
    wheel(&mut ui, &low, Point::new(200.0, 150.0), 1000.0);
    let back = frame(&mut ui, &low);
    assert_eq!(texts(&back), texts(&first));
    assert_eq!(tracks(&back), top);

    // ? closes the window, and it opens at the top again.
    press_key(&mut ui, &low, Key::Question);
    assert_eq!(ui.mode(), UiMode::Normal);
    wheel(&mut ui, &low, Point::new(200.0, 150.0), -3.0);
    press_key(&mut ui, &low, Key::F1);
    assert_eq!(texts(&frame(&mut ui, &low)), texts(&first));
}

#[test]
fn cursor_follows_what_is_under_the_pointer() {
    use pandite_ui::CursorShape;
    let mut ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let (open, _) = icon_draws(&list, Icon::Open)[0];
    move_to(&mut ui, &ctx, open.center());
    assert_eq!(ui.cursor(), CursorShape::Hand);
    move_to(&mut ui, &ctx, Point::new(640.0, 400.0));
    assert_eq!(
        ui.cursor(),
        CursorShape::Default,
        "fitted image is not pannable"
    );
    // Full screen hides the pointer once the toolbar has gone.
    ctx.fullscreen = true;
    move_to(&mut ui, &ctx, Point::new(640.0, 400.0));
    frame(&mut ui, &at(ctx, ms(5000)));
    assert_eq!(ui.cursor(), CursorShape::Hidden);
    move_to(&mut ui, &at(ctx, ms(5100)), Point::new(641.0, 400.0));
    assert_eq!(ui.cursor(), CursorShape::Default);
    // Text fields show a caret cursor.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    let list = frame(&mut ui, &ctx);
    let field = text_rect_exact(&list, "50").unwrap();
    move_to(&mut ui, &ctx, field.center());
    assert_eq!(ui.cursor(), CursorShape::Text);
}

#[test]
fn background_work_shows_progress_and_ticks() {
    // Within the toolbar idle time, so nothing else animates.
    let mut ctx = image_ctx();
    ctx.now = ms(1_000);
    ctx.busy = Some(pandite_ui::Progress {
        label: "Saving photo.png",
        done: 1,
        total: 4,
    });
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Saving photo.png - 25%"));
    assert_eq!(
        ui.next_redraw_at(),
        Some(ms(1_100)),
        "spinner ticks at 10 fps"
    );
    assert!(
        !ui.wants_redraw(),
        "no continuous redraw for background work"
    );
    ctx.busy = None;
    frame(&mut ui, &ctx);
    assert_ne!(ui.next_redraw_at(), Some(ms(1_100)));
}

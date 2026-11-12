//! Gate G1: the toolbar.

mod support;

use pandite_core::{DrawCmd, Point, Size};
use pandite_ui::{Action, ActionKind, Command, Icon, Key, Modifiers, Theme, Ui};
use support::*;

/// The toolbar commands the gate names, with the action a click emits.
const COMMANDS: [(Command, ActionKind); 17] = [
    (Command::Open, ActionKind::Open),
    (Command::SaveAs, ActionKind::SaveAs),
    (Command::CopyPath, ActionKind::CopyPath),
    (Command::RotateLeft, ActionKind::RotateLeft),
    (Command::RotateRight, ActionKind::RotateRight),
    (Command::FlipHorizontal, ActionKind::FlipHorizontal),
    (Command::FlipVertical, ActionKind::FlipVertical),
    (Command::Crop, ActionKind::StartCrop),
    (Command::Resize, ActionKind::OpenResizeDialog),
    (Command::BulkResize, ActionKind::OpenBulkDialog),
    (Command::Fit, ActionKind::FitToWindow),
    (Command::ActualSize, ActionKind::OneToOne),
    (Command::ZoomIn, ActionKind::ZoomIn),
    (Command::ZoomOut, ActionKind::ZoomOut),
    (Command::Fullscreen, ActionKind::ToggleFullscreen),
    (Command::Previous, ActionKind::Previous),
    (Command::Next, ActionKind::Next),
];

/// Draws of `icon` inside the 40 px toolbar band.
fn bar_draws(
    list: &pandite_core::DrawList,
    icon: Icon,
) -> Vec<(pandite_core::Rect, pandite_core::Color)> {
    icon_draws(list, icon)
        .into_iter()
        .filter(|(r, _)| r.bottom() <= 40.0)
        .collect()
}

fn icon_of(cmd: Command) -> Icon {
    cmd.icon().expect("toolbar command has an icon")
}

#[test]
fn toolbar_has_every_command() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let bar_h = 40.0;
    for (cmd, _) in COMMANDS {
        let draws = bar_draws(&list, icon_of(cmd));
        assert!(
            draws
                .iter()
                .any(|(r, _)| r.bottom() <= bar_h && r.w == 20.0),
            "{cmd:?} has no toolbar button"
        );
        let tip = cmd.tooltip();
        assert!(
            tip.starts_with(cmd.label().trim_end_matches("...")),
            "{tip}"
        );
        assert!(
            cmd.shortcut_label().is_some(),
            "{cmd:?} tooltip lacks a key"
        );
    }
    // In a narrow window the buttons that do not fit stay reachable through
    // the overflow menu.
    let mut narrow = image_ctx();
    narrow.window_size = Size::new(420.0, 600.0);
    let mut ui = Ui::new();
    let list = frame(&mut ui, &narrow);
    let more = bar_draws(&list, Icon::More);
    assert_eq!(more.len(), 1, "overflow button shown");
    let visible: Vec<Command> = COMMANDS
        .iter()
        .map(|(c, _)| *c)
        .filter(|c| !bar_draws(&list, icon_of(*c)).is_empty())
        .collect();
    click(&mut ui, &narrow, more[0].0.center());
    let menu = frame(&mut ui, &narrow);
    for (cmd, _) in COMMANDS {
        if !visible.contains(&cmd) {
            assert!(
                has_text(&menu, cmd.label()),
                "{cmd:?} neither on the bar nor in the overflow menu"
            );
        }
    }
    for (r, _) in bar_draws(&list, Icon::Open) {
        assert!(r.right() <= 420.0);
    }
}

#[test]
fn toolbar_click_emits_action() {
    for (cmd, expected) in COMMANDS {
        let mut ctx = image_ctx();
        if cmd == Command::Fullscreen {
            ctx.fullscreen = false;
        }
        let mut ui = Ui::new();
        let list = frame(&mut ui, &ctx);
        let (r, _) = bar_draws(&list, icon_of(cmd))[0];
        let actions = click(&mut ui, &ctx, r.center());
        assert_eq!(kinds(&actions), vec![expected], "clicking {cmd:?}");
        if cmd == Command::ActualSize {
            assert_eq!(actions[0], Action::OneToOne { anchor: None });
        }
    }
    // In full screen the same button shows the "leave" icon and still
    // toggles.
    let mut ctx = image_ctx();
    ctx.fullscreen = true;
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(bar_draws(&list, Icon::Fullscreen).is_empty());
    let (r, _) = bar_draws(&list, Icon::ExitFullscreen)[0];
    assert_eq!(
        click(&mut ui, &ctx, r.center()),
        vec![Action::ToggleFullscreen]
    );
}

#[test]
fn copy_path_button_emits_action() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let (r, _) = bar_draws(&list, Icon::CopyPath)[0];
    // Press alone does nothing; releasing elsewhere cancels the click.
    move_to(&mut ui, &ctx, r.center());
    assert!(press(&mut ui, &ctx, r.center(), pandite_ui::PointerButton::Left).is_empty());
    let away = Point::new(r.center().x, 400.0);
    move_to(&mut ui, &ctx, away);
    assert!(release(&mut ui, &ctx, away, pandite_ui::PointerButton::Left).is_empty());
    // A full click on the dedicated button copies the path.
    assert_eq!(click(&mut ui, &ctx, r.center()), vec![Action::CopyPath]);
    // So does Ctrl+C.
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyPath]
    );
    // The application confirms with a toast.
    ui.toast("Path copied", ctx.now);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Path copied"));
    let later = at(ctx, ms(10_000));
    let list = frame(&mut ui, &later);
    assert!(!has_text(&list, "Path copied"), "toast fades out");
}

#[test]
fn toolbar_autohides_and_returns() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let content_point = Point::new(640.0, 400.0);
    move_to(&mut ui, &at(ctx, ms(100)), content_point);
    let list = frame(&mut ui, &at(ctx, ms(200)));
    assert!(
        !bar_draws(&list, Icon::Open).is_empty(),
        "visible after movement"
    );
    let due = ui.next_redraw_at().expect("hide time scheduled");
    assert_eq!(
        due,
        ms(100) + ms(2200),
        "redraw scheduled when the idle time ends"
    );

    // Mid-fade: partly transparent and animating.
    let list = frame(&mut ui, &at(ctx, ms(2400)));
    let (_, tint) = bar_draws(&list, Icon::Open)[0];
    assert!(tint.a > 0 && tint.a < 255, "fading, alpha {}", tint.a);
    assert!(ui.wants_redraw());

    // Hidden after the fade.
    let list = frame(&mut ui, &at(ctx, ms(3000)));
    assert!(bar_draws(&list, Icon::Open).is_empty(), "hidden after idle");
    assert!(!ui.wants_redraw());
    // A click where the hidden toolbar was does not trigger its buttons.
    let open_spot = Point::new(24.0, 20.0);
    let out = press(
        &mut ui,
        &at(ctx, ms(3100)),
        open_spot,
        pandite_ui::PointerButton::Left,
    );
    let out2 = release(
        &mut ui,
        &at(ctx, ms(3150)),
        open_spot,
        pandite_ui::PointerButton::Left,
    );
    assert!(out.is_empty() && out2.is_empty());
    move_to(&mut ui, &at(ctx, ms(3200)), content_point);

    // Pointer movement brings it back.
    let ctx2 = at(ctx, ms(8000));
    let list = frame(&mut ui, &ctx2);
    assert!(bar_draws(&list, Icon::Open).is_empty());
    move_to(&mut ui, &ctx2, Point::new(700.0, 420.0));
    let list = frame(&mut ui, &at(ctx, ms(8200)));
    let (_, tint) = bar_draws(&list, Icon::Open)[0];
    assert_eq!(tint.a, 255, "fully back");

    // Resting over the toolbar keeps it.
    move_to(&mut ui, &at(ctx, ms(8300)), Point::new(600.0, 20.0));
    let list = frame(&mut ui, &at(ctx, ms(20_000)));
    assert!(!bar_draws(&list, Icon::Open).is_empty());
}

#[test]
fn toolbar_disabled_without_image() {
    let mut ctx = base_ctx();
    ctx.count = 0;
    let mut ui = Ui::new();
    let theme = Theme::for_kind(ctx.theme);
    let list = frame(&mut ui, &ctx);
    for (cmd, _) in COMMANDS {
        let draws = bar_draws(&list, icon_of(cmd));
        assert!(!draws.is_empty(), "{cmd:?} still shown");
        let tint = draws[0].1;
        let should_enable = matches!(
            cmd,
            Command::Open | Command::BulkResize | Command::Fullscreen
        );
        if should_enable {
            assert_eq!(tint, theme.icon, "{cmd:?} enabled");
        } else {
            assert_eq!(tint, theme.text_disabled, "{cmd:?} disabled");
            let out = click(&mut ui, &ctx, draws[0].0.center());
            assert!(out.is_empty(), "disabled {cmd:?} emitted {out:?}");
        }
    }
    // Keys for image commands do nothing either.
    assert!(press_key(&mut ui, &ctx, Key::R).is_empty());
    assert!(key(&mut ui, &ctx, Key::C, Modifiers::CTRL).is_empty());
    // Open still works, and the toolbar never hides without an image.
    let (r, _) = bar_draws(&list, Icon::Open)[0];
    assert_eq!(click(&mut ui, &ctx, r.center()), vec![Action::Open]);
    let list = frame(&mut ui, &at(ctx, ms(60_000)));
    assert!(!bar_draws(&list, Icon::Open).is_empty());
    // The empty screen offers an Open button too.
    assert!(has_text(&list, "Open image..."));
    let r = text_rect(&list, "Open image...").unwrap();
    assert_eq!(
        click(&mut ui, &at(ctx, ms(60_000)), r.center()),
        vec![Action::Open]
    );
}

#[test]
fn tooltip_appears_on_hover() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let (r, _) = bar_draws(&list, Icon::RotateRight)[0];
    let tip = Command::RotateRight.tooltip();
    assert_eq!(tip, "Rotate right  (R)");
    move_to(&mut ui, &at(ctx, ms(1000)), r.center());
    let list = frame(&mut ui, &at(ctx, ms(1200)));
    assert!(!has_text(&list, &tip), "no tooltip before the delay");
    let due = ui.next_redraw_at().expect("tooltip scheduled");
    assert!(due > ms(1200) && due <= ms(1600), "{due:?}");
    let list = frame(&mut ui, &at(ctx, ms(1700)));
    let tr = text_rect(&list, &tip).expect("tooltip drawn after the delay");
    assert!(tr.y >= r.bottom(), "tooltip below the button");
    assert!(tr.x >= 0.0 && tr.right() <= 1280.0);
    // A tooltip background is drawn behind it.
    let theme = Theme::for_kind(ctx.theme);
    assert!(list.cmds.iter().any(
        |c| matches!(c, DrawCmd::Rect { rect, color, .. } if *color == theme.tooltip && rect.contains(tr.center()))
    ));
    // Disabled buttons explain why.
    let mut empty = base_ctx();
    empty.count = 0;
    let mut ui2 = Ui::new();
    let l = frame(&mut ui2, &empty);
    let (r2, _) = bar_draws(&l, Icon::RotateRight)[0];
    move_to(&mut ui2, &empty, r2.center());
    let l = frame(&mut ui2, &at(empty, ms(2000)));
    assert!(has_text(&l, "open an image first"));
    // Leaving the button hides it.
    move_to(&mut ui, &at(ctx, ms(1800)), Point::new(640.0, 400.0));
    let list = frame(&mut ui, &at(ctx, ms(2600)));
    assert!(!has_text(&list, &tip));
}

//! Gate G4: keyboard coverage, themes, DPI scaling, context menu and
//! pointer gestures on the image.

mod support;

use pandite_core::{DisplayMode, DrawCmd, Point, Rect, Size};
use pandite_ui::{
    contrast, Action, ActionKind, Command, ConfirmRequest, CursorShape, Icon, InputEvent, Key,
    Modifiers, PointerButton, Progress, Theme, ThemeKind, Ui, UiContext, UiMode, WheelDelta,
    SHORTCUTS, ZOOM_STEP,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use support::*;

fn rich_ctx() -> UiContext<'static> {
    let mut ctx = image_ctx();
    ctx.can_undo = true;
    ctx.fullscreen = true;
    ctx
}

/// Tab through the dialog pressing Enter after each step until `want`
/// appears. Returns every action seen.
fn tab_enter_until(ui: &mut Ui, ctx: &UiContext<'_>, want: ActionKind) -> Vec<Action> {
    let mut seen = Vec::new();
    for _ in 0..40 {
        seen.extend(press_key(ui, ctx, Key::Tab));
        seen.extend(press_key(ui, ctx, Key::Enter));
        if seen.iter().any(|a| a.kind() == want) {
            return seen;
        }
    }
    panic!("{want:?} not reachable by keyboard; saw {seen:?}");
}

#[test]
fn shortcuts_cover_every_action() {
    let mut reached: BTreeMap<ActionKind, String> = BTreeMap::new();
    let mut note = |actions: &[Action], how: &str| {
        for a in actions {
            reached.entry(a.kind()).or_insert_with(|| how.to_string());
        }
    };
    let ctx = rich_ctx();

    // Every global binding, pressed in a fresh interface. One image is
    // selected, so the key that clears the selection has work to do.
    let mut selected = ctx;
    selected.selected = &[3];
    for s in SHORTCUTS.iter().filter(|s| !s.command.crop_only()) {
        let mut ui = Ui::new();
        move_to(&mut ui, &selected, Point::new(600.0, 300.0));
        let out = key(&mut ui, &selected, s.key, s.mods);
        note(&out, &s.label());
    }

    // Crop mode keys.
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    note(&press_key(&mut ui, &ctx, Key::Enter), "C, Ctrl+A, Enter");
    press_key(&mut ui, &ctx, Key::C);
    note(&press_key(&mut ui, &ctx, Key::C), "C, C");

    // Resize dialog keys, including clipboard keys in its field.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    note(
        &key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        "Ctrl+C in a field",
    );
    note(
        &key(&mut ui, &ctx, Key::V, Modifiers::CTRL),
        "Ctrl+V in a field",
    );
    note(&press_key(&mut ui, &ctx, Key::Enter), "Ctrl+R, Enter");

    // Bulk dialog by keyboard: Tab to each button, Enter activates.
    let mut bulk_ctx = ctx;
    bulk_ctx.folder = None;
    let mut ui = Ui::new();
    key(&mut ui, &bulk_ctx, Key::B, Modifiers::CTRL);
    let out = tab_enter_until(&mut ui, &bulk_ctx, ActionKind::PickBulkFiles);
    note(&out, "Ctrl+B, Tab, Enter");
    let out = tab_enter_until(&mut ui, &bulk_ctx, ActionKind::PickBulkFolder);
    note(&out, "Ctrl+B, Tab x2, Enter");
    let out = tab_enter_until(&mut ui, &bulk_ctx, ActionKind::PickBulkDestination);
    note(&out, "Tab to Browse, Enter");
    ui.add_bulk_files(vec![PathBuf::from("C:/a.jpg")]);
    ui.set_bulk_destination(PathBuf::from("C:/out"));
    let out = tab_enter_until(&mut ui, &bulk_ctx, ActionKind::RunBulk);
    note(&out, "Enter in the bulk dialog");
    let mut running = bulk_ctx;
    running.bulk = Some(Progress {
        label: "a.jpg",
        done: 0,
        total: 1,
    });
    let out = tab_enter_until(&mut ui, &running, ActionKind::CancelBulk);
    note(&out, "Tab to Stop, Enter");

    // Confirm dialog.
    let mut ui = Ui::new();
    ui.show_confirm(ConfirmRequest::new(7, "Replace?", "Replace the file?"));
    let out = press_key(&mut ui, &ctx, Key::Enter);
    assert_eq!(
        out,
        vec![Action::ConfirmResult {
            id: 7,
            accepted: true
        }]
    );
    note(&out, "Enter in a confirm dialog");

    // Zoom at a point from the context menu.
    let mut ui = Ui::new();
    let p = Point::new(500.0, 300.0);
    move_to(&mut ui, &ctx, p);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    note(
        &click_label(&mut ui, &ctx, "Zoom in"),
        "context menu Zoom in",
    );

    let missing: Vec<ActionKind> = ActionKind::ALL
        .iter()
        .copied()
        .filter(|k| !reached.contains_key(k))
        .collect();
    assert!(missing.is_empty(), "actions without a binding: {missing:?}");

    // Every command has a key, and the toolbar's are all keyed.
    for cmd in Command::ALL {
        assert!(cmd.shortcut_label().is_some(), "{cmd:?} has no shortcut");
    }
}

#[test]
fn f11_maps_to_fullscreen() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(
        press_key(&mut ui, &ctx, Key::F11),
        vec![Action::ToggleFullscreen]
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::Enter, Modifiers::ALT),
        vec![Action::ToggleFullscreen]
    );
    // F11 leaves full screen again. Escape does not: it quits, in full
    // screen as in a window.
    let mut full = ctx;
    full.fullscreen = true;
    assert_eq!(
        press_key(&mut ui, &full, Key::F11),
        vec![Action::ToggleFullscreen]
    );
    assert_eq!(press_key(&mut ui, &full, Key::Escape), vec![Action::Quit]);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);
    // F11 works inside a dialog and without an image; a held key does not
    // repeat it.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert_eq!(
        press_key(&mut ui, &ctx, Key::F11),
        vec![Action::ToggleFullscreen]
    );
    press_key(&mut ui, &ctx, Key::Escape);
    let mut empty = base_ctx();
    empty.count = 0;
    assert_eq!(
        press_key(&mut ui, &empty, Key::F11),
        vec![Action::ToggleFullscreen]
    );
    let held = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::F11,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());

    // The other keys the gate names. Page Down and Page Up move by the
    // thumbnails that fit whole in the 1280 pixel strip: 10 pixels of
    // padding on each side, 64 pixel cells 70 pixels apart, so
    // (1280 - 20 - 64) / 70 + 1 = 18.
    assert_eq!(Ui::gallery_page(&ctx), 18);
    let mut hidden = ctx;
    hidden.gallery_visible = false;
    assert_eq!(Ui::gallery_page(&hidden), 18, "the hidden strip counts too");
    let mut narrow = ctx;
    narrow.window_size = Size::new(60.0, 800.0);
    assert_eq!(Ui::gallery_page(&narrow), 1, "never less than one image");
    // Space selects the image and Backspace clears the selection; they
    // no longer move to the next and the previous image.
    let table: [(Key, Modifiers, Action); 11] = [
        (Key::Right, Modifiers::NONE, Action::Next),
        (Key::Space, Modifiers::NONE, Action::ToggleSelect(3)),
        (Key::PageDown, Modifiers::NONE, Action::StepBy(18)),
        (Key::Left, Modifiers::NONE, Action::Previous),
        (Key::PageUp, Modifiers::NONE, Action::StepBy(-18)),
        (Key::Plus, Modifiers::SHIFT, Action::ZoomIn),
        (Key::Equals, Modifiers::NONE, Action::ZoomIn),
        (Key::Minus, Modifiers::NONE, Action::ZoomOut),
        (Key::C, Modifiers::CTRL, Action::CopyImage),
        (Key::C, Modifiers::CTRL_SHIFT, Action::CopyPath),
        (Key::S, Modifiers::CTRL_SHIFT, Action::SaveAs),
    ];
    for (k, mods, expected) in table {
        assert_eq!(key(&mut ui, &ctx, k, mods), vec![expected.clone()], "{k:?}");
    }
    assert!(
        press_key(&mut ui, &ctx, Key::Backspace).is_empty(),
        "nothing selected, nothing to clear"
    );
    // Holding an arrow repeats navigation.
    let repeat = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Right,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert_eq!(repeat, vec![Action::Next]);
    assert_eq!(Command::Fullscreen.shortcut_label().as_deref(), Some("F11"));
}

#[test]
fn dark_and_light_themes_differ() {
    let dark = Theme::for_kind(ThemeKind::Dark);
    let light = Theme::for_kind(ThemeKind::Light);
    let (de, le) = (dark.entries(), light.entries());
    assert_eq!(de.len(), 25);
    let mut differing = 0;
    for ((dn, dc), (ln, lc)) in de.iter().zip(le.iter()) {
        assert_eq!(dn, ln);
        if dc != lc {
            differing += 1;
        }
    }
    assert!(differing >= 20, "only {differing} colors differ");
    for t in [dark, light] {
        assert!(contrast(t.text, t.panel_solid) >= 7.0);
        assert!(contrast(t.text_dim, t.panel_solid) >= 4.5);
        assert!(contrast(t.accent_text, t.accent) >= 4.5);
    }
    assert!(pandite_ui::luminance(dark.canvas) < 0.05);
    assert!(pandite_ui::luminance(light.canvas) > 0.7);

    // The same state draws differently per theme.
    let mut ctx = image_ctx();
    ctx.theme = ThemeKind::Dark;
    let mut ui = Ui::new();
    let d = frame(&mut ui, &ctx);
    ctx.theme = ThemeKind::Light;
    let mut ui = Ui::new();
    let l = frame(&mut ui, &ctx);
    assert_ne!(d, l);
    let bar_color = |list: &pandite_core::DrawList| {
        list.cmds.iter().find_map(|c| match c {
            DrawCmd::Rect { rect, color, .. } if rect.y == 0.0 && rect.h == 40.0 => Some(*color),
            _ => None,
        })
    };
    assert_eq!(bar_color(&d), Some(dark.panel));
    assert_eq!(bar_color(&l), Some(light.panel));
    let (_, dt) = icon_draws(&d, Icon::Open)[0];
    let (_, lt) = icon_draws(&l, Icon::Open)[0];
    assert_eq!((dt, lt), (dark.icon, light.icon));
    ctx.theme = ThemeKind::Dark;
    let dc = Ui::clear_color(&ctx);
    ctx.theme = ThemeKind::Light;
    assert_ne!(dc, Ui::clear_color(&ctx));
}

#[test]
fn layout_scales_with_dpi() {
    let ctx1 = image_ctx();
    let mut ctx2 = ctx1;
    ctx2.scale_factor = 2.0;
    ctx2.window_size = Size::new(2560.0, 1600.0);
    let mut ui1 = Ui::new();
    let mut ui2 = Ui::new();
    let l1 = frame(&mut ui1, &ctx1);
    let l2 = frame(&mut ui2, &ctx2);
    // Toolbar icons double in size and position.
    for icon in [Icon::Open, Icon::RotateRight, Icon::Fullscreen] {
        let (a, _) = icon_draws(&l1, icon)[0];
        let (b, _) = icon_draws(&l2, icon)[0];
        assert_eq!(a.w, 20.0);
        assert_eq!(b.w, 40.0);
        assert_eq!(
            Rect::new(a.x * 2.0, a.y * 2.0, a.w * 2.0, a.h * 2.0),
            b,
            "{icon:?}"
        );
    }
    // Text sizes double.
    let size = |list: &pandite_core::DrawList| {
        list.cmds.iter().find_map(|c| match c {
            DrawCmd::Text { text, size, .. } if text.contains("photo.jpg") => Some(*size),
            _ => None,
        })
    };
    assert_eq!(size(&l1), Some(13.0));
    assert_eq!(size(&l2), Some(26.0));
    // The gallery strip doubles in height and cell size.
    let strip1 = 800.0 - ui1.content_rect(&ctx1).h;
    let strip2 = 1600.0 - ui2.content_rect(&ctx2).h;
    assert_eq!((strip1, strip2), (84.0, 168.0));
    let cell = |list: &pandite_core::DrawList| {
        icon_draws(list, Icon::Picture)
            .first()
            .map(|(r, _)| r.w)
            .unwrap_or_default()
    };
    assert_eq!((cell(&l1), cell(&l2)), (20.0, 40.0));
    // Icons are rasterized for the scale, once.
    let mut sink = FakeSink::default();
    assert!(ui1.sync_textures(&ctx1, &mut sink));
    assert_eq!(sink.uploads.len(), Icon::ALL.len());
    assert!(sink.uploads.iter().all(|u| u.1 == 20 && u.2 == 20 && u.3));
    assert!(!ui1.sync_textures(&ctx1, &mut sink), "cached");
    assert_eq!(sink.uploads.len(), Icon::ALL.len());
    let mut ctx15 = ctx1;
    ctx15.scale_factor = 1.5;
    assert!(ui1.sync_textures(&ctx15, &mut sink));
    assert!(sink.uploads[Icon::ALL.len()..].iter().all(|u| u.1 == 30));
    assert_eq!(ui1.icon_size(), 30);
    ui1.invalidate_textures();
    assert!(ui1.sync_textures(&ctx15, &mut sink));
}

#[test]
fn context_menu_lists_commands() {
    // A window tall enough for the whole menu below the pointer.
    let ctx = with_image(
        UiContext {
            window_size: Size::new(1280.0, 900.0),
            ..base_ctx()
        },
        4000,
        3000,
    );
    let mut ui = Ui::new();
    let p = Point::new(500.0, 50.0);
    move_to(&mut ui, &ctx, p);
    press(&mut ui, &ctx, p, PointerButton::Right);
    assert!(release(&mut ui, &ctx, p, PointerButton::Right).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal);
    let list = frame(&mut ui, &ctx);
    for cmd in [
        Command::Open,
        Command::SaveAs,
        Command::CopyImage,
        Command::CopyPath,
        Command::Undo,
        Command::ToggleSelect,
        Command::ClearSelection,
        Command::MoveToTrash,
        Command::DeleteFile,
        Command::RotateLeft,
        Command::RotateRight,
        Command::FlipHorizontal,
        Command::FlipVertical,
        Command::Crop,
        Command::Resize,
        Command::BulkResize,
        Command::Fit,
        Command::ActualSize,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::Previous,
        Command::Next,
        Command::ToggleGallery,
        Command::Fullscreen,
        Command::Help,
        Command::Quit,
    ] {
        assert!(
            text_rect_exact(&list, cmd.label()).is_some(),
            "{cmd:?} missing from the context menu"
        );
    }
    for keys in [
        "Ctrl+O",
        "Ctrl+Shift+S",
        "Ctrl+C",
        "Ctrl+Shift+C",
        "F11",
        "Shift+R",
        "Del",
        "Shift+Del",
    ] {
        assert!(
            text_rect_exact(&list, keys).is_some(),
            "shortcut {keys} shown"
        );
    }
    // The menu sits at the pointer.
    let open = text_rect_exact(&list, "Open...").unwrap();
    assert!(open.x > p.x && open.y > p.y && open.y < p.y + 40.0);
    // Choosing an entry runs it and closes the menu.
    assert_eq!(
        click_label(&mut ui, &ctx, "Rotate right"),
        vec![Action::RotateRight]
    );
    assert_eq!(ui.mode(), UiMode::Normal);
    // The delete commands run from the menu as they do from the keys.
    for (cmd, action) in [
        (Command::MoveToTrash, Action::MoveToTrash),
        (Command::DeleteFile, Action::DeleteFile),
    ] {
        press(&mut ui, &ctx, p, PointerButton::Right);
        release(&mut ui, &ctx, p, PointerButton::Right);
        assert_eq!(click_label(&mut ui, &ctx, cmd.label()), vec![action]);
        assert_eq!(ui.mode(), UiMode::Normal);
    }
    // Zoom from the menu zooms at the point that was clicked.
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    assert_eq!(
        click_label(&mut ui, &ctx, "Zoom in"),
        vec![Action::ZoomAt {
            factor: ZOOM_STEP,
            anchor: p
        }]
    );
    // Keyboard: the menu key opens it on the first entry; Escape closes.
    press_key(&mut ui, &ctx, Key::ContextMenu);
    assert_eq!(press_key(&mut ui, &ctx, Key::Enter), vec![Action::Open]);
    press_key(&mut ui, &ctx, Key::ContextMenu);
    press_key(&mut ui, &ctx, Key::Down);
    assert_eq!(press_key(&mut ui, &ctx, Key::Enter), vec![Action::SaveAs]);
    press_key(&mut ui, &ctx, Key::ContextMenu);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
    // A click outside closes it without acting.
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    assert!(click(&mut ui, &ctx, Point::new(50.0, 600.0)).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);

    // Without an image, image entries are disabled and do nothing.
    let mut empty = base_ctx();
    empty.count = 0;
    let theme = Theme::for_kind(empty.theme);
    let mut ui = Ui::new();
    press(&mut ui, &empty, p, PointerButton::Right);
    release(&mut ui, &empty, p, PointerButton::Right);
    let list = frame(&mut ui, &empty);
    assert_eq!(text_color(&list, "Rotate left"), Some(theme.text_disabled));
    assert!(click_label(&mut ui, &empty, "Rotate left").is_empty());
    assert_eq!(ui.mode(), UiMode::Modal, "disabled entry keeps the menu");

    // In a short window the menu stays inside and scrolls.
    let mut short = image_ctx();
    short.window_size = Size::new(500.0, 300.0);
    let mut ui = Ui::new();
    let q = Point::new(450.0, 250.0);
    press(&mut ui, &short, q, PointerButton::Right);
    release(&mut ui, &short, q, PointerButton::Right);
    let list = frame(&mut ui, &short);
    let panel = list
        .cmds
        .iter()
        .rev()
        .find_map(|c| match c {
            DrawCmd::Clip(Some(r)) => Some(*r),
            _ => None,
        })
        .unwrap();
    assert!(panel.x >= 0.0 && panel.right() <= 500.0 && panel.bottom() <= 300.0);
    assert!(text_rect_exact(&list, "Quit").is_none_or(|r| r.y > panel.bottom()));
    for _ in 0..10 {
        wheel(&mut ui, &short, panel.center(), -3.0);
    }
    let list = frame(&mut ui, &short);
    let quit = text_rect_exact(&list, "Quit").unwrap();
    assert!(quit.bottom() <= panel.bottom() + 1.0, "scrolled to the end");
    assert_eq!(click(&mut ui, &short, quit.center()), vec![Action::Quit]);

    // In crop mode the menu offers the crop commands.
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    let list = frame(&mut ui, &ctx);
    assert!(text_rect_exact(&list, "Apply crop").is_some());
    assert_eq!(
        click_label(&mut ui, &ctx, "Cancel crop"),
        vec![Action::CancelCrop]
    );
}

#[test]
fn drag_pans_and_wheel_zooms_at_cursor() {
    // A 1:1 view of a 4000 x 3000 image overflows the window.
    let mut ctx = image_ctx();
    {
        let info = ctx.image.as_mut().unwrap();
        info.view.mode = DisplayMode::OneToOne;
        info.zoom_percent = 100.0;
        info.screen_rect = Rect::new(-1360.0, -1142.0, 4000.0, 3000.0);
    }
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    let a = Point::new(600.0, 400.0);
    move_to(&mut ui, &ctx, a);
    assert_eq!(ui.cursor(), CursorShape::Grab);
    assert!(press(&mut ui, &ctx, a, PointerButton::Left).is_empty());
    assert_eq!(
        move_to(&mut ui, &ctx, Point::new(630.0, 380.0)),
        vec![Action::PanBy {
            dx: 30.0,
            dy: -20.0
        }]
    );
    assert_eq!(ui.cursor(), CursorShape::Grabbing);
    assert_eq!(
        move_to(&mut ui, &ctx, Point::new(640.0, 390.0)),
        vec![Action::PanBy { dx: 10.0, dy: 10.0 }]
    );
    // Dragging over the toolbar keeps panning (capture), and the release
    // does not click anything there.
    assert_eq!(
        move_to(&mut ui, &ctx, Point::new(40.0, 20.0)),
        vec![Action::PanBy {
            dx: -600.0,
            dy: -370.0
        }]
    );
    assert!(release(&mut ui, &ctx, Point::new(40.0, 20.0), PointerButton::Left).is_empty());
    // The middle button pans as well.
    let out = drag(
        &mut ui,
        &ctx,
        Point::new(300.0, 300.0),
        Point::new(340.0, 300.0),
        PointerButton::Middle,
    );
    let dx: f32 = out
        .iter()
        .map(|a| match a {
            Action::PanBy { dx, .. } => *dx,
            other => panic!("unexpected {other:?}"),
        })
        .sum();
    assert_eq!(dx, 40.0);

    // The wheel zooms around the pointer.
    let c = Point::new(321.0, 222.0);
    assert_eq!(
        wheel(&mut ui, &ctx, c, 1.0),
        vec![Action::ZoomAt {
            factor: pandite_ui::WHEEL_ZOOM_STEP,
            anchor: c
        }]
    );
    let touch = ui.handle_event(
        InputEvent::Wheel {
            pos: c,
            delta: WheelDelta::Pixels { x: 0.0, y: 25.0 },
        },
        &ctx,
    );
    match touch[..] {
        [Action::ZoomAt { factor, anchor }] => {
            assert_eq!(anchor, c);
            assert!((factor - pandite_ui::WHEEL_ZOOM_STEP.sqrt()).abs() < 1e-4);
        }
        ref other => panic!("{other:?}"),
    }
    // The wheel over the toolbar does nothing.
    assert!(wheel(&mut ui, &ctx, Point::new(600.0, 20.0), 1.0).is_empty());

    // Double click toggles 1:1 and fit.
    let d = Point::new(500.0, 300.0);
    let mut out = click(&mut ui, &ctx, d);
    out.extend(click(&mut ui, &ctx, d));
    assert_eq!(out, vec![Action::FitToWindow], "1:1 goes back to fit");
    let fit = image_ctx();
    let mut out = click(&mut ui, &at(fit, ms(5000)), d);
    out.extend(click(&mut ui, &at(fit, ms(5200)), d));
    assert_eq!(out, vec![Action::OneToOne { anchor: Some(d) }]);
    // Two slow clicks are not a double click.
    let mut out = click(&mut ui, &at(fit, ms(9000)), d);
    out.extend(click(&mut ui, &at(fit, ms(9900)), d));
    assert!(out.is_empty());
    // Keyboard panning.
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Up),
        vec![Action::PanBy { dx: 0.0, dy: 80.0 }]
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::Right, Modifiers::CTRL),
        vec![Action::PanBy { dx: -80.0, dy: 0.0 }]
    );
}

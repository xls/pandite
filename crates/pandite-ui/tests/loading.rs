//! Gate G6: the loading overlay and escaping a slow load.

mod support;

use pandite_core::{Color, DrawCmd, DrawList, Point, Rect};
use pandite_ui::{
    Action, Icon, Key, Loading, Modifiers, PointerButton, Theme, ThumbState, Ui, UiContext, UiMode,
};
use std::time::Duration;
use support::*;

const HINT: &str = "Press Esc to cancel";

fn loading(
    name: &'static str,
    elapsed: Duration,
    cloud: bool,
    over_image: bool,
) -> Loading<'static> {
    Loading {
        file_name: name,
        elapsed,
        progress: None,
        cloud,
        over_image,
    }
}

/// The first screen: nothing shown yet, the first image loading.
fn splash_ctx(elapsed: Duration) -> UiContext<'static> {
    let mut ctx = base_ctx();
    ctx.count = 1;
    ctx.index = 0;
    ctx.now = Duration::from_secs(1) + elapsed;
    ctx.loading = Some(loading("first.jpg", elapsed, false, false));
    ctx
}

/// The next image loading while the previous stays on screen.
fn over_ctx(elapsed: Duration) -> UiContext<'static> {
    let mut ctx = image_ctx();
    ctx.now = Duration::from_secs(1) + elapsed;
    ctx.loading = Some(loading("next.jpg", elapsed, false, true));
    ctx
}

/// Spinner dots: small round rects in the accent color.
fn spinner(list: &DrawList, accent: Color) -> Vec<(Rect, u8)> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect {
                rect,
                color,
                radius,
            } if color.r == accent.r
                && color.g == accent.g
                && color.b == accent.b
                && *radius > 0.0
                && rect.w == rect.h
                && rect.w <= 8.0 =>
            {
                Some((*rect, color.a))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn loading_overlay_appears_after_delay() {
    for make in [splash_ctx as fn(Duration) -> UiContext<'static>, over_ctx] {
        let mut ui = Ui::new();
        let early = make(ms(40));
        let list = frame(&mut ui, &early);
        assert!(!has_text(&list, HINT), "fast loads never flash the overlay");
        assert!(!has_text(&list, "No image open"), "no empty screen either");
        assert_eq!(
            ui.next_redraw_at(),
            Some(early.now + ms(110)),
            "a frame is scheduled for when the overlay is due"
        );
        let list = frame(&mut ui, &make(ms(149)));
        assert!(!has_text(&list, HINT));
        let list = frame(&mut ui, &make(ms(150)));
        assert!(has_text(&list, HINT), "shown once the delay passed");
        assert!(ui.wants_redraw());
        // When the load finishes the overlay goes away.
        let mut done = make(ms(400));
        done.loading = None;
        let list = frame(&mut ui, &done);
        assert!(!has_text(&list, HINT));
    }
}

#[test]
fn loading_overlay_shows_name_and_escape_hint() {
    // The splash fills the content area on the first screen.
    let ctx = splash_ctx(ms(500));
    let theme = Theme::for_kind(ctx.theme);
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Opening first.jpg"), "{:?}", texts(&list));
    assert!(has_text(&list, HINT));
    let content = ui.content_rect(&ctx);
    assert!(list
        .cmds
        .iter()
        .any(|c| matches!(c, DrawCmd::Rect { rect, color, .. } if *rect == content && *color == theme.canvas)));
    let name = text_rect(&list, "first.jpg").unwrap();
    assert!(content.contains(name.center()));

    // A very long name is elided to fit, keeping its end.
    let long: &'static str = Box::leak(format!("{}-end.jpg", "x".repeat(400)).into_boxed_str());
    let mut long_ctx = ctx;
    long_ctx.loading = Some(loading(long, ms(500), false, false));
    let list = frame(&mut ui, &long_ctx);
    let shown = texts(&list)
        .into_iter()
        .find(|t| t.starts_with("Opening"))
        .unwrap();
    assert!(
        shown.contains("...") && shown.ends_with("-end.jpg"),
        "{shown}"
    );
    let r = text_rect(&list, "-end.jpg").unwrap();
    assert!(r.x >= content.x && r.right() <= content.right());

    // Over a shown image it is a small panel, and the image stays visible.
    let over = over_ctx(ms(500));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &over);
    assert!(has_text(&list, "Opening next.jpg"));
    assert!(has_text(&list, HINT));
    let content = ui.content_rect(&over);
    assert!(!list
        .cmds
        .iter()
        .any(|c| matches!(c, DrawCmd::Rect { rect, color, .. } if *rect == content && *color == theme.canvas)));
    let pill = text_rect(&list, "next.jpg").unwrap();
    assert!(
        pill.y < content.y + content.h * 0.25,
        "small overlay near the top"
    );
}

#[test]
fn loading_overlay_reports_cloud_download() {
    let mut ctx = splash_ctx(ms(3500));
    ctx.loading = Some(Loading {
        file_name: "remote.heic",
        elapsed: ms(12_400),
        progress: Some(0.42),
        cloud: true,
        over_image: false,
    });
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Downloading remote.heic"));
    assert!(has_text(&list, "Downloading from cloud drive"));
    assert!(has_text(&list, "42%"), "{:?}", texts(&list));
    assert!(has_text(&list, "12 s"), "elapsed time shown");
    assert!(!icon_draws(&list, Icon::Cloud).is_empty(), "cloud icon");
    // The small overlay says the same.
    let mut over = over_ctx(ms(800));
    over.loading = Some(Loading {
        cloud: true,
        ..loading("remote.heic", ms(800), true, true)
    });
    let list = frame(&mut Ui::new(), &over);
    assert!(has_text(&list, "Downloading from cloud drive"));
    // A local file does not claim a download.
    let local = splash_ctx(ms(500));
    let list = frame(&mut Ui::new(), &local);
    assert!(!has_text(&list, "cloud"));
    assert!(icon_draws(&list, Icon::Cloud).is_empty());
}

#[test]
fn escape_during_loading_emits_cancel_load() {
    let mut ctx = over_ctx(ms(5000));
    ctx.fullscreen = true;
    let mut ui = Ui::new();
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::CancelLoad]
    );
    // Even before the overlay shows.
    let early = over_ctx(ms(20));
    assert_eq!(
        press_key(&mut ui, &early, Key::Escape),
        vec![Action::CancelLoad]
    );
    // Before other meanings: crop mode stays, a dialog stays, a menu stays.
    let mut plain = image_ctx();
    plain.fullscreen = true;
    press_key(&mut ui, &plain, Key::C);
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::CancelLoad]
    );
    assert_eq!(ui.mode(), UiMode::Crop);
    press_key(&mut ui, &plain, Key::Escape);
    key(&mut ui, &plain, Key::R, Modifiers::CTRL);
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::CancelLoad]
    );
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &plain, Key::Escape);
    let p = Point::new(500.0, 100.0);
    press(&mut ui, &plain, p, PointerButton::Right);
    release(&mut ui, &plain, p, PointerButton::Right);
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::CancelLoad]
    );
    press_key(&mut ui, &plain, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);
    // A held Escape sends one cancel.
    let held = ui.handle_event(
        pandite_ui::InputEvent::KeyDown {
            key: Key::Escape,
            mods: Modifiers::NONE,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());
    // Without a load, Escape leaves full screen again.
    assert_eq!(
        press_key(&mut ui, &plain, Key::Escape),
        vec![Action::ToggleFullscreen]
    );
    // The splash case too, where cancel may close the application.
    let splash = splash_ctx(ms(800));
    assert_eq!(
        press_key(&mut Ui::new(), &splash, Key::Escape),
        vec![Action::CancelLoad]
    );
}

#[test]
fn loading_overlay_animates_and_input_still_works() {
    let thumbs: &'static [ThumbState] =
        Box::leak(vec![ThumbState::Pending; 200].into_boxed_slice());
    let mut ctx = over_ctx(ms(600));
    ctx.count = 200;
    ctx.index = 0;
    ctx.thumbnails = thumbs;
    let theme = Theme::for_kind(ctx.theme);
    let mut ui = Ui::new();
    let a = frame(&mut ui, &ctx);
    assert!(ui.wants_redraw(), "continuous redraw while loading");
    let mut later = ctx;
    later.now += ms(300);
    later.loading = Some(loading("next.jpg", ms(900), false, true));
    let b = frame(&mut ui, &later);
    let (sa, sb) = (spinner(&a, theme.accent), spinner(&b, theme.accent));
    assert_eq!(sa.len(), 8, "eight spinner dots");
    assert_eq!(sb.len(), 8);
    assert_ne!(
        sa.iter().map(|d| d.1).collect::<Vec<_>>(),
        sb.iter().map(|d| d.1).collect::<Vec<_>>(),
        "the spinner moves"
    );
    // Navigation, window commands, toolbar and gallery keep working.
    assert_eq!(press_key(&mut ui, &later, Key::Right), vec![Action::Next]);
    assert_eq!(
        press_key(&mut ui, &later, Key::Left),
        vec![Action::Previous]
    );
    assert_eq!(
        press_key(&mut ui, &later, Key::F11),
        vec![Action::ToggleFullscreen]
    );
    let (open, _) = icon_draws(&b, Icon::Open)[0];
    assert_eq!(click(&mut ui, &later, open.center()), vec![Action::Open]);
    let strip_y = ui.content_rect(&later).bottom() + 40.0;
    let before = ui.gallery_visible_range(&later);
    assert!(wheel(&mut ui, &later, Point::new(640.0, strip_y), -5.0).is_empty());
    let mut settled = later;
    settled.now += ms(1000);
    frame(&mut ui, &settled);
    assert!(ui.gallery_visible_range(&settled).start > before.start);
    // The splash keeps the toolbar usable as well.
    let splash = splash_ctx(ms(700));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &splash);
    assert_eq!(spinner(&list, theme.accent).len(), 8);
    let (open, _) = icon_draws(&list, Icon::Open)[0];
    assert_eq!(click(&mut ui, &splash, open.center()), vec![Action::Open]);
}

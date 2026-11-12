//! Gate G2: the gallery strip.

mod support;

use pandite_core::{DrawCmd, Point, Rect};
use pandite_ui::{Action, ActionKind, Icon, PointerButton, Theme, ThumbState, Ui, UiContext};
use support::*;

fn gallery_ctx(count: usize, index: usize, thumbs: &'static [ThumbState]) -> UiContext<'static> {
    let mut ctx = image_ctx();
    ctx.count = count;
    ctx.index = index;
    ctx.thumbnails = thumbs;
    ctx
}

fn leak(v: Vec<ThumbState>) -> &'static [ThumbState] {
    Box::leak(v.into_boxed_slice())
}

fn ready(n: usize) -> &'static [ThumbState] {
    leak(vec![
        ThumbState::Ready {
            width: 160,
            height: 120
        };
        n
    ])
}

/// The gallery panel: the window below the content area.
fn panel(ui: &Ui, ctx: &UiContext<'_>) -> Rect {
    let c = ui.content_rect(ctx);
    Rect::new(
        0.0,
        c.bottom(),
        ctx.window_size.w,
        ctx.window_size.h - c.bottom(),
    )
}

#[test]
fn gallery_wheel_scrolls_strip() {
    let ctx = gallery_ctx(200, 0, ready(200));
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    let before = ui.gallery_visible_range(&ctx);
    assert_eq!(before.start, 0);
    let strip = panel(&ui, &ctx);
    let p = strip.center();
    // Rolling toward the user (negative y) moves on to later items.
    let out = wheel(&mut ui, &ctx, p, -3.0);
    assert!(
        out.is_empty(),
        "wheel over the strip must not zoom: {out:?}"
    );
    assert!(ui.wants_redraw(), "smooth scroll animates");
    let later = at(ctx, ms(1000));
    let list = frame(&mut ui, &later);
    let after = ui.gallery_visible_range(&later);
    assert!(after.start > before.start, "{before:?} -> {after:?}");
    assert!(!ui.wants_redraw(), "animation settles");
    let drawn: Vec<usize> = thumbnail_draws(&list).iter().map(|d| d.0).collect();
    assert_eq!(drawn.first().copied(), Some(after.start));
    // Rolling back returns to the start; it never scrolls past the ends.
    wheel(&mut ui, &later, p, 100.0);
    let end = at(ctx, ms(3000));
    frame(&mut ui, &end);
    assert_eq!(ui.gallery_visible_range(&end).start, 0);
    // A drag on the strip scrolls it as well and does not navigate.
    let out = drag(
        &mut ui,
        &end,
        Point::new(900.0, p.y),
        Point::new(300.0, p.y),
        PointerButton::Left,
    );
    assert!(out.is_empty(), "{out:?}");
    assert!(ui.gallery_visible_range(&end).start > 0);
}

#[test]
fn gallery_wheel_outside_strip_zooms_image() {
    let ctx = gallery_ctx(50, 3, ready(50));
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    let p = Point::new(500.0, 300.0);
    assert!(ui.content_rect(&ctx).contains(p));
    let out = wheel(&mut ui, &ctx, p, 1.0);
    assert_eq!(out.len(), 1);
    match out[0] {
        Action::ZoomAt { factor, anchor } => {
            assert!((factor - pandite_ui::WHEEL_ZOOM_STEP).abs() < 1e-6);
            assert_eq!(anchor, p);
        }
        ref other => panic!("expected ZoomAt, got {other:?}"),
    }
    let out = wheel(&mut ui, &ctx, p, -2.0);
    assert!(matches!(out[0], Action::ZoomAt { factor, .. } if factor < 1.0));
    // The strip itself never zooms.
    let strip = panel(&ui, &ctx).center();
    assert!(wheel(&mut ui, &ctx, strip, 1.0).is_empty());
}

#[test]
fn gallery_draws_only_visible_items() {
    let ctx = gallery_ctx(10_000, 5000, ready(10_000));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let draws = thumbnail_draws(&list);
    let range = ui.gallery_visible_range(&ctx);
    assert!(!draws.is_empty());
    assert!(
        draws.len() < 30,
        "{} thumbnails drawn of 10000",
        draws.len()
    );
    assert_eq!(draws.len(), range.len());
    let strip = panel(&ui, &ctx);
    for (i, r) in &draws {
        assert!(range.contains(i));
        assert!(r.intersect(strip).is_some(), "item {i} drawn off the strip");
    }
    // The strip is clipped so partly visible cells do not spill over.
    assert!(list
        .cmds
        .iter()
        .any(|c| matches!(c, DrawCmd::Clip(Some(r)) if *r == strip)));
    // At twice the scale factor the strip and its cells are twice as big,
    // and fewer items fit.
    let mut hi = ctx;
    hi.scale_factor = 2.0;
    let mut ui2 = Ui::new();
    let list2 = frame(&mut ui2, &hi);
    let strip2 = panel(&ui2, &hi);
    assert_eq!((strip.h, strip2.h), (84.0, 168.0));
    let cell = |l: &pandite_core::DrawList| thumbnail_draws(l)[0].1.w;
    assert_eq!((cell(&list), cell(&list2)), (64.0, 128.0));
    assert_eq!((Ui::thumbnail_px(&ctx), Ui::thumbnail_px(&hi)), (64, 128));
    assert!(thumbnail_draws(&list2).len() < draws.len());
}

#[test]
fn gallery_reports_visible_range() {
    let ctx = gallery_ctx(300, 0, ready(300));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let range = ui.gallery_visible_range(&ctx);
    let mut drawn: Vec<usize> = thumbnail_draws(&list).iter().map(|d| d.0).collect();
    drawn.sort_unstable();
    assert_eq!(drawn, range.clone().collect::<Vec<_>>());
    // Moving to a far image moves the range with it.
    let far = gallery_ctx(300, 250, ready(300));
    let far_t = at(far, ms(2000));
    frame(&mut ui, &far_t);
    let moved = ui.gallery_visible_range(&far_t);
    assert!(moved.contains(&250), "{moved:?}");
    // A hidden gallery reports nothing and gives its space to the image.
    let mut hidden = far_t;
    hidden.gallery_visible = false;
    let list = frame(&mut ui, &hidden);
    assert_eq!(ui.gallery_visible_range(&hidden), 0..0);
    assert!(thumbnail_draws(&list).is_empty());
    assert_eq!(ui.content_rect(&hidden).h, 800.0);
    assert!(ui.content_rect(&far_t).h < 800.0);
}

#[test]
fn gallery_click_emits_goto() {
    let ctx = gallery_ctx(20, 0, ready(20));
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let (_, r) = thumbnail_draws(&list)
        .into_iter()
        .find(|d| d.0 == 5)
        .expect("item 5 visible");
    assert_eq!(click(&mut ui, &ctx, r.center()), vec![Action::GoTo(5)]);
    // Clicking the gap between cells does nothing.
    let gap = Point::new(r.right() + 3.0, r.center().y);
    assert!(click(&mut ui, &ctx, gap).is_empty());
    // A right click there opens the context menu instead of navigating.
    move_to(&mut ui, &ctx, r.center());
    press(&mut ui, &ctx, r.center(), PointerButton::Right);
    let out = release(&mut ui, &ctx, r.center(), PointerButton::Right);
    assert!(out.is_empty());
    assert!(has_text(&frame(&mut ui, &ctx), "Next image"));
}

#[test]
fn gallery_keeps_current_in_view() {
    let thumbs = ready(500);
    let ctx = gallery_ctx(500, 0, thumbs);
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    let theme = Theme::for_kind(ctx.theme);
    for (index, t) in [(400usize, 1000u64), (499, 2000), (3, 3000), (250, 4000)] {
        let c = at(gallery_ctx(500, index, thumbs), ms(t));
        let list = frame(&mut ui, &c);
        let strip = panel(&ui, &c);
        let (_, r) = thumbnail_draws(&list)
            .into_iter()
            .find(|d| d.0 == index)
            .unwrap_or_else(|| panic!("current item {index} not drawn"));
        assert!(
            r.x >= strip.x && r.right() <= strip.right(),
            "item {index} not fully in view: {r:?}"
        );
        // The current item carries the accent ring.
        assert!(
            list.cmds.iter().any(|cmd| matches!(
                cmd,
                DrawCmd::Border { rect, color, .. } if *color == theme.accent && rect.contains(r.center())
            )),
            "item {index} not highlighted"
        );
    }
    // Manual scrolling is not undone until the current image changes.
    let c = at(gallery_ctx(500, 250, thumbs), ms(4000));
    let strip_center = panel(&ui, &c).center();
    wheel(&mut ui, &c, strip_center, 40.0);
    let c2 = at(gallery_ctx(500, 250, thumbs), ms(6000));
    frame(&mut ui, &c2);
    assert!(!ui.gallery_visible_range(&c2).contains(&250));
    let c3 = at(gallery_ctx(500, 251, thumbs), ms(8000));
    frame(&mut ui, &c3);
    assert!(ui.gallery_visible_range(&c3).contains(&251));
}

#[test]
fn gallery_placeholder_for_missing_thumbnail() {
    let thumbs = leak(vec![
        ThumbState::Ready {
            width: 100,
            height: 50,
        },
        ThumbState::Pending,
        ThumbState::Failed,
        ThumbState::NotDownloaded,
    ]);
    // Six items; the last two have no entry at all and count as pending.
    let ctx = gallery_ctx(6, 0, thumbs);
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let draws = thumbnail_draws(&list);
    assert_eq!(
        draws.iter().map(|d| d.0).collect::<Vec<_>>(),
        vec![0],
        "only ready thumbnails use a texture"
    );
    // The ready thumbnail covers its square cell, cropping the long side.
    let uv = list.cmds.iter().find_map(|c| match c {
        DrawCmd::Image { texture, uv, .. } if *texture == pandite_ui::thumbnail_texture_id(0) => {
            Some(*uv)
        }
        _ => None,
    });
    assert_eq!(uv, Some(Rect::new(0.25, 0.0, 0.5, 1.0)));
    let cell0 = draws[0].1;
    let pitch = 64.0 + 6.0;
    let cell = |i: usize| Rect::new(cell0.x + pitch * i as f32, cell0.y, cell0.w, cell0.h);
    let inside = |icon: Icon, i: usize| {
        icon_draws(&list, icon)
            .iter()
            .any(|(r, _)| cell(i).contains(r.center()))
    };
    assert!(inside(Icon::Picture, 1), "pending placeholder");
    assert!(inside(Icon::Warning, 2), "failed placeholder");
    assert!(inside(Icon::Cloud, 3), "cloud placeholder");
    assert!(inside(Icon::Picture, 4) && inside(Icon::Picture, 5));
    assert!(!inside(Icon::Picture, 3), "cloud differs from pending");
    // Placeholders still navigate.
    assert_eq!(
        kinds(&click(&mut ui, &ctx, cell(3).center())),
        vec![ActionKind::GoTo]
    );
}

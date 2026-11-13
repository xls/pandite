//! Gate G3: selecting and copying recognized text. A drag that starts on a
//! word selects words in reading order, a drag off text pans, Ctrl+A and
//! Ctrl+C select and copy with line breaks, highlights follow every view
//! transform, and Escape quits even while text is selected.

mod support;

use pandite_core::{
    Affine, DisplayMode, DrawCmd, DrawList, Point, Quad, Rect, Size, TextLayout, TextLine,
    TextWord, ViewState,
};
use pandite_ui::{
    Action, ActionKind, CursorShape, ImageInfo, Key, Loading, Modifiers, PointerButton, TextView,
    Theme, Ui, UiContext,
};
use support::*;

/// The recognized image: 2000 x 1000 pixels, three lines of words.
const W: u32 = 2000;
const H: u32 = 1000;

fn word(text: &str, x0: f32, x1: f32, y0: f32) -> TextWord {
    TextWord {
        text: text.into(),
        quad: Quad::from_rect(Rect::new(x0, y0, x1 - x0, 60.0)),
    }
}

fn layout() -> TextLayout {
    TextLayout {
        width: W,
        height: H,
        lines: vec![
            TextLine {
                words: vec![
                    word("alpha", 100.0, 400.0, 100.0),
                    word("beta", 450.0, 700.0, 100.0),
                    word("gamma", 750.0, 1100.0, 100.0),
                ],
            },
            TextLine {
                words: vec![
                    word("delta", 100.0, 400.0, 300.0),
                    word("epsilon", 450.0, 900.0, 300.0),
                ],
            },
            TextLine {
                words: vec![word("zeta", 100.0, 300.0, 500.0)],
            },
        ],
    }
}

const ALL_TEXT: &str = "alpha beta gamma\ndelta epsilon\nzeta";

/// A context showing an image of `w` x `h` displayed pixels with `view`,
/// whose text is `layout` mapped by `to_image`.
fn ctx_with<'a>(
    layout: &'a TextLayout,
    w: u32,
    h: u32,
    view: ViewState,
    to_image: Affine,
) -> UiContext<'a> {
    let mut ctx: UiContext<'a> = base_ctx();
    let content = Ui::new().content_rect(&ctx);
    let size = Size::new(w as f32, h as f32);
    ctx.image = Some(ImageInfo {
        file_name: "scan.png",
        width: w,
        height: h,
        view,
        zoom_percent: view.scale(size, content.size()) * 100.0,
        screen_rect: view.image_rect(size, content),
        edited: false,
    });
    ctx.text = Some(TextView {
        layout,
        to_image,
        id: 7,
    });
    ctx.text_enabled = true;
    ctx
}

/// `ctx` at time `t`.
fn timed(mut ctx: UiContext<'_>, t: std::time::Duration) -> UiContext<'_> {
    ctx.now = t;
    ctx
}

/// Click at `p` at time `t`, then press again at `p` 120 ms later and keep
/// the button down: the held second press of a double click. Returns
/// every action.
fn double_press(ui: &mut Ui, ctx: &UiContext<'_>, t: std::time::Duration, p: Point) -> Vec<Action> {
    let mut out = click(ui, &timed(*ctx, t), p);
    let second = timed(*ctx, t + ms(120));
    out.extend(move_to(ui, &second, p));
    out.extend(press(ui, &second, p, PointerButton::Left));
    out
}

/// The held third press of a triple click at `p`, starting at time `t`.
fn triple_press(ui: &mut Ui, ctx: &UiContext<'_>, t: std::time::Duration, p: Point) -> Vec<Action> {
    let mut out = click(ui, &timed(*ctx, t), p);
    out.extend(click(ui, &timed(*ctx, t + ms(120)), p));
    let third = timed(*ctx, t + ms(240));
    out.extend(move_to(ui, &third, p));
    out.extend(press(ui, &third, p, PointerButton::Left));
    out
}

/// True when `out` holds a zoom or a pan.
fn zooms_or_pans(out: &[Action]) -> bool {
    kinds(out).iter().any(|k| {
        matches!(
            k,
            ActionKind::OneToOne | ActionKind::FitToWindow | ActionKind::PanBy
        )
    })
}

/// The unedited image fitted into the window.
fn fit_ctx(layout: &TextLayout) -> UiContext<'_> {
    ctx_with(layout, W, H, ViewState::default(), Affine::IDENTITY)
}

/// Where image point `p` of the displayed image lands on screen.
fn on_screen(ctx: &UiContext<'_>, p: Point) -> Point {
    let info = ctx.image.unwrap();
    let r = info.screen_rect;
    Point::new(
        r.x + p.x * r.w / info.width as f32,
        r.y + p.y * r.h / info.height as f32,
    )
}

/// The screen center of word `i` (reading order) for `ctx`.
fn word_center(ctx: &UiContext<'_>, i: usize) -> Point {
    let text = ctx.text.unwrap();
    let (_, w) = text.layout.words().nth(i).unwrap();
    on_screen(ctx, text.to_image.apply(w.quad.center()))
}

/// Selection highlights in the draw list, as corner lists.
fn highlights(list: &DrawList) -> Vec<[Point; 4]> {
    let color = Theme::DARK.selection;
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Rect { rect, color: c, .. } if *c == color => {
                Some(Quad::from_rect(*rect).points)
            }
            DrawCmd::Quad { points, color: c } if *c == color => Some(*points),
            _ => None,
        })
        .collect()
}

fn bounds(points: &[Point; 4]) -> Rect {
    Quad::new(*points).bounds()
}

fn near(a: Rect, b: Rect) -> bool {
    (a.x - b.x).abs() < 0.05
        && (a.y - b.y).abs() < 0.05
        && (a.w - b.w).abs() < 0.05
        && (a.h - b.h).abs() < 0.05
}

/// The screen rectangle of displayed-image rectangle `r`.
fn screen_rect_of(ctx: &UiContext<'_>, r: Rect) -> Rect {
    let a = on_screen(ctx, Point::new(r.x, r.y));
    let b = on_screen(ctx, Point::new(r.right(), r.bottom()));
    Rect::from_corners(a, b)
}

#[test]
fn text_drag_selects_words_in_reading_order() {
    let l = layout();
    let ctx = fit_ctx(&l);
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);

    // A text cursor over words, the normal one between lines.
    move_to(&mut ui, &ctx, word_center(&ctx, 1));
    assert_eq!(ui.cursor(), CursorShape::Text);
    move_to(&mut ui, &ctx, on_screen(&ctx, Point::new(500.0, 230.0)));
    assert_eq!(ui.cursor(), CursorShape::Default);

    // From "beta" down to "delta": beta, gamma and delta, in reading order.
    let out = drag(
        &mut ui,
        &ctx,
        word_center(&ctx, 1),
        word_center(&ctx, 3),
        PointerButton::Left,
    );
    assert!(
        !kinds(&out).contains(&ActionKind::PanBy),
        "a drag on text never pans: {out:?}"
    );
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("beta gamma\ndelta"));
    // One highlight band per line, over the image.
    let list = frame(&mut ui, &ctx);
    let bands = highlights(&list);
    assert_eq!(bands.len(), 2, "{bands:?}");
    assert!(near(
        bounds(&bands[0]),
        screen_rect_of(&ctx, Rect::new(450.0, 100.0, 650.0, 60.0))
    ));
    assert!(near(
        bounds(&bands[1]),
        screen_rect_of(&ctx, Rect::new(100.0, 300.0, 300.0, 60.0))
    ));

    // Backwards from "epsilon" to "alpha": still reading order.
    drag(
        &mut ui,
        &ctx,
        word_center(&ctx, 4),
        word_center(&ctx, 0),
        PointerButton::Left,
    );
    assert_eq!(
        ui.selected_text(&ctx).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );

    // Past the end of a line, the nearest word is the line's last one.
    drag(
        &mut ui,
        &ctx,
        word_center(&ctx, 1),
        on_screen(&ctx, Point::new(1500.0, 130.0)),
        PointerButton::Left,
    );
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("beta gamma"));

    // A plain click on a word places no selection.
    click(&mut ui, &ctx, word_center(&ctx, 2));
    assert_eq!(ui.selected_text(&ctx), None);
    assert!(highlights(&frame(&mut ui, &ctx)).is_empty());

    // A double click selects the word, a triple click its line, and
    // neither toggles fit and 1:1.
    let mut when = ms(10_000);
    let mut tap = |ui: &mut Ui, p: Point| -> Vec<Action> {
        when += ms(120);
        click(ui, &timed(fit_ctx(&l), when), p)
    };
    let p = word_center(&ctx, 4);
    let mut out = tap(&mut ui, p);
    out.extend(tap(&mut ui, p));
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("epsilon"));
    out.extend(tap(&mut ui, p));
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("delta epsilon"));
    assert!(
        !kinds(&out).contains(&ActionKind::OneToOne),
        "a double click on text is not a zoom: {out:?}"
    );
}

#[test]
fn drag_off_text_still_pans() {
    let l = layout();
    // At 1:1 the 2000 x 1000 image overflows the 1280 x 716 content area.
    let view = ViewState {
        mode: DisplayMode::OneToOne,
        ..ViewState::default()
    };
    let ctx = ctx_with(&l, W, H, view, Affine::IDENTITY);
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    let blank = on_screen(&ctx, Point::new(1300.0, 700.0));
    move_to(&mut ui, &ctx, blank);
    assert_eq!(ui.cursor(), CursorShape::Grab, "off text the image pans");

    let out = drag(
        &mut ui,
        &ctx,
        blank,
        Point::new(blank.x - 120.0, blank.y - 40.0),
        PointerButton::Left,
    );
    let panned: (f32, f32) = out.iter().fold((0.0, 0.0), |(x, y), a| match a {
        Action::PanBy { dx, dy } => (x + dx, y + dy),
        _ => (x, y),
    });
    assert_eq!(panned, (-120.0, -40.0), "{out:?}");
    assert_eq!(ui.selected_text(&ctx), None);

    // The same drag starting on a word ("epsilon", fully on screen at
    // 1:1) selects instead; the end lies past the word, nearest to it.
    let start = word_center(&ctx, 4);
    move_to(&mut ui, &ctx, start);
    assert_eq!(
        ui.cursor(),
        CursorShape::Text,
        "text wins over the grab hand"
    );
    let out = drag(
        &mut ui,
        &ctx,
        start,
        Point::new(start.x + 300.0, start.y),
        PointerButton::Left,
    );
    assert!(!kinds(&out).contains(&ActionKind::PanBy), "{out:?}");
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("epsilon"));

    // A pan keeps the selection; a click off text drops it.
    drag(
        &mut ui,
        &ctx,
        blank,
        Point::new(blank.x + 50.0, blank.y),
        PointerButton::Left,
    );
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("epsilon"));
    let late = timed(ctx, ms(20_000));
    click(&mut ui, &late, blank);
    assert_eq!(ui.selected_text(&late), None);

    // The middle button pans everywhere, also over text.
    let out = drag(
        &mut ui,
        &ctx,
        start,
        Point::new(start.x + 30.0, start.y),
        PointerButton::Middle,
    );
    assert!(kinds(&out).contains(&ActionKind::PanBy), "{out:?}");
    assert_eq!(ui.selected_text(&ctx), None);

    // Without recognized text a drag anywhere pans, as before.
    let mut plain = ctx;
    plain.text = None;
    let mut ui = Ui::new();
    let out = drag(
        &mut ui,
        &plain,
        start,
        Point::new(start.x - 30.0, start.y),
        PointerButton::Left,
    );
    assert!(kinds(&out).contains(&ActionKind::PanBy), "{out:?}");
    move_to(&mut ui, &plain, start);
    assert_eq!(ui.cursor(), CursorShape::Grab);
}

#[test]
fn select_all_and_copy_text() {
    let l = layout();
    let ctx = fit_ctx(&l);
    let mut ui = Ui::new();
    move_to(&mut ui, &ctx, Point::new(600.0, 400.0));
    // Nothing selected: Ctrl+C copies the image.
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
    // Ctrl+A selects every word, Ctrl+C copies them with line breaks.
    assert!(key(&mut ui, &ctx, Key::A, Modifiers::CTRL).is_empty());
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some(ALL_TEXT));
    assert_eq!(highlights(&frame(&mut ui, &ctx)).len(), 3);
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText(ALL_TEXT.into())]
    );
    // A partial selection copies only its words.
    drag(
        &mut ui,
        &ctx,
        word_center(&ctx, 2),
        word_center(&ctx, 4),
        PointerButton::Left,
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("gamma\ndelta epsilon".into())]
    );

    // The context menu offers both, next to the image and path copies.
    let p = word_center(&ctx, 0);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    let out = click_label(&mut ui, &ctx, "Select all text");
    assert!(out.is_empty());
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    assert_eq!(
        click_label(&mut ui, &ctx, "Copy selected text"),
        vec![Action::CopyText(ALL_TEXT.into())]
    );

    // The shortcut overlay lists them while text recognition is on.
    key(&mut ui, &ctx, Key::F1, Modifiers::NONE);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Select all text") && has_text(&list, "Copy selected text"));
    press_key(&mut ui, &ctx, Key::Escape);

    // Without text: no text commands anywhere.
    let mut plain = ctx;
    plain.text = None;
    plain.text_enabled = false;
    let mut ui = Ui::new();
    assert!(key(&mut ui, &plain, Key::A, Modifiers::CTRL).is_empty());
    assert_eq!(
        key(&mut ui, &plain, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
    press(&mut ui, &plain, p, PointerButton::Right);
    release(&mut ui, &plain, p, PointerButton::Right);
    let list = frame(&mut ui, &plain);
    assert!(!has_text(&list, "Select all text"), "{:?}", texts(&list));
    assert!(!has_text(&list, "Copy selected text"));
    press_key(&mut ui, &plain, Key::Escape);
    key(&mut ui, &plain, Key::F1, Modifiers::NONE);
    let list = frame(&mut ui, &plain);
    assert!(!has_text(&list, "Select all text"), "{:?}", texts(&list));

    // In crop mode Ctrl+A selects the crop, not text.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    press_key(&mut ui, &ctx, Key::C);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    assert_eq!(
        ui.selected_text(&ctx),
        None,
        "crop mode drops the selection"
    );
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Enter),
        vec![Action::ApplyCrop(pandite_ui::PixelRect {
            x: 0,
            y: 0,
            width: W,
            height: H
        })]
    );
}

#[test]
fn selection_follows_view_transform() {
    let l = layout();
    // "beta" covers image (450..700, 100..160).
    let beta = Rect::new(450.0, 100.0, 250.0, 60.0);
    let base = fit_ctx(&l);
    let mut ui = Ui::new();
    let at_beta = word_center(&base, 1);
    let mut t = ms(30_000);
    for _ in 0..2 {
        t += ms(100);
        click(&mut ui, &timed(base, t), at_beta);
    }
    assert_eq!(ui.selected_text(&base).as_deref(), Some("beta"));

    let check = |ui: &mut Ui, ctx: &UiContext<'_>, displayed: Rect, what: &str| {
        let bands = highlights(&frame(ui, ctx));
        assert_eq!(bands.len(), 1, "{what}: {bands:?}");
        let want = screen_rect_of(ctx, displayed);
        assert!(
            near(bounds(&bands[0]), want),
            "{what}: band {:?}, want {want:?}",
            bounds(&bands[0])
        );
        // The pointer over the moved word still finds it.
        let c = want.center();
        move_to(ui, ctx, c);
        assert_eq!(ui.cursor(), CursorShape::Text, "{what}");
    };

    check(&mut ui, &base, beta, "fit");

    // Zoomed 2x around a point and panned.
    let mut view = ViewState::default();
    let content = Ui::new().content_rect(&base);
    let size = Size::new(W as f32, H as f32);
    view.zoom_to(2.0, size, content, Point::new(300.0, 200.0));
    view.pan_by(-150.0, 200.0, size, content.size());
    let zoomed = ctx_with(&l, W, H, view, Affine::IDENTITY);
    check(&mut ui, &zoomed, beta, "zoom and pan");

    // Rotated a quarter turn clockwise: the displayed image is 1000 x 2000
    // and (x, y) goes to (1000 - y, x).
    let turn = Affine::new(0.0, 1.0, -1.0, 0.0, H as f32, 0.0);
    let rotated = ctx_with(&l, H, W, ViewState::default(), turn);
    check(
        &mut ui,
        &rotated,
        Rect::new(1000.0 - 160.0, 450.0, 60.0, 250.0),
        "rotation",
    );

    // Mirrored left to right: x goes to 2000 - x.
    let mirror = Affine::new(-1.0, 0.0, 0.0, 1.0, W as f32, 0.0);
    let flipped = ctx_with(&l, W, H, ViewState::default(), mirror);
    check(
        &mut ui,
        &flipped,
        Rect::new(2000.0 - 700.0, 100.0, 250.0, 60.0),
        "flip",
    );

    // Cropped to (300, 50) and 1200 x 600: the word moves by the corner.
    let crop = Affine::translate(-300.0, -50.0);
    let cropped = ctx_with(&l, 1200, 600, ViewState::default(), crop);
    check(
        &mut ui,
        &cropped,
        Rect::new(150.0, 50.0, 250.0, 60.0),
        "crop",
    );
    // Words the crop cut away are gone: select all and copy skip them,
    // and their old place takes no text cursor.
    let mut crop_ui = Ui::new();
    key(&mut crop_ui, &cropped, Key::A, Modifiers::CTRL);
    assert_eq!(
        key(&mut crop_ui, &cropped, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("beta gamma\nepsilon".into())]
    );
    move_to(
        &mut crop_ui,
        &cropped,
        on_screen(&cropped, crop.apply(Point::new(250.0, 130.0))),
    );
    assert_ne!(
        crop_ui.cursor(),
        CursorShape::Text,
        "alpha was cropped away"
    );

    // A slanted mapping gives a quad that follows the slant exactly.
    let (s, c) = 20f32.to_radians().sin_cos();
    let slant = Affine::new(c, s, -s, c, 400.0, 0.0);
    let slanted = ctx_with(&l, W, H, ViewState::default(), slant);
    let list = frame(&mut ui, &slanted);
    let quads: Vec<[Point; 4]> = list
        .cmds
        .iter()
        .filter_map(|cmd| match cmd {
            DrawCmd::Quad { points, .. } => Some(*points),
            _ => None,
        })
        .collect();
    assert_eq!(quads.len(), 1, "a slanted band is a quad");
    let corners = Quad::from_rect(beta).points;
    for (got, src) in quads[0].iter().zip(corners) {
        let want = on_screen(&slanted, slant.apply(src));
        assert!(
            (got.x - want.x).abs() < 0.05 && (got.y - want.y).abs() < 0.05,
            "{got:?} vs {want:?}"
        );
    }

    // A new layout (another image) drops the selection.
    let mut other = base;
    other.text = Some(TextView {
        layout: &l,
        to_image: Affine::IDENTITY,
        id: 8,
    });
    assert!(highlights(&frame(&mut ui, &other)).is_empty());
    assert_eq!(ui.selected_text(&other), None);
}

#[test]
fn escape_quits_with_text_selection() {
    let l = layout();
    let mut ctx = fit_ctx(&l);
    ctx.fullscreen = true;
    // The next image is loading behind the current one.
    ctx.loading = Some(Loading {
        file_name: "next.png",
        elapsed: ms(400),
        progress: None,
        cloud: false,
        over_image: true,
    });
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    assert!(ui.selected_text(&ctx).is_some());
    // Escape quits at once: it does not clear the selection first, cancel
    // the load or leave full screen.
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);
    // The same in a window without a load.
    ctx.loading = None;
    ctx.fullscreen = false;
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);

    // Escape during a text drag quits as well.
    let a = word_center(&ctx, 0);
    let b = word_center(&ctx, 2);
    move_to(&mut ui, &ctx, a);
    press(&mut ui, &ctx, a, PointerButton::Left);
    move_to(&mut ui, &ctx, b);
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("alpha beta gamma"));
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);
    release(&mut ui, &ctx, b, PointerButton::Left);

    // An open menu takes Escape first and the selection stays; the next
    // Escape quits.
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    press(&mut ui, &ctx, a, PointerButton::Right);
    release(&mut ui, &ctx, a, PointerButton::Right);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some(ALL_TEXT));
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);
    // Ctrl+C still copies the selected text, not the image.
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText(ALL_TEXT.into())]
    );
}

#[test]
fn ctrl_c_copies_selected_text() {
    let l = layout();
    let ctx = fit_ctx(&l);
    let mut ui = Ui::new();
    // Text is there but nothing is selected: Ctrl+C copies the image.
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
    // A selection makes Ctrl+C copy its words instead, and a toast says
    // which copy it was.
    drag(
        &mut ui,
        &ctx,
        word_center(&ctx, 0),
        word_center(&ctx, 1),
        PointerButton::Left,
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("alpha beta".into())]
    );
    assert!(has_text(&frame(&mut ui, &ctx), "Text copied"));
    // A held key copies once.
    let held = ui.handle_event(
        pandite_ui::InputEvent::KeyDown {
            key: Key::C,
            mods: Modifiers::CTRL,
            repeat: true,
        },
        &ctx,
    );
    assert!(held.is_empty());
    // Ctrl+Shift+C copies the file path, with or without a selection, and
    // keeps the selection.
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL_SHIFT),
        vec![Action::CopyPath]
    );
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some("alpha beta"));
    // A click off the text drops the selection: Ctrl+C copies the image
    // again.
    click(&mut ui, &ctx, on_screen(&ctx, Point::new(1500.0, 800.0)));
    assert_eq!(ui.selected_text(&ctx), None);
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
}

#[test]
fn double_click_drag_selects_words() {
    let l = layout();
    let t = ms(10_000);
    let held = timed(fit_ctx(&l), t + ms(200));
    let mut ui = Ui::new();
    frame(&mut ui, &held);

    // The second press of a double click on "beta" selects the word at
    // once, before the release.
    let mut out = double_press(&mut ui, &held, t, word_center(&held, 1));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("beta"));
    assert_eq!(ui.cursor(), CursorShape::Text);

    // With the button held, a drag grows the selection by whole words.
    out.extend(move_to(&mut ui, &held, word_center(&held, 2)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("beta gamma"));
    out.extend(move_to(&mut ui, &held, word_center(&held, 4)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("beta gamma\ndelta epsilon")
    );
    // The pointer near the start of "epsilon" still takes the whole word.
    let start_of_epsilon = on_screen(&held, Point::new(460.0, 330.0));
    out.extend(move_to(&mut ui, &held, start_of_epsilon));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("beta gamma\ndelta epsilon")
    );
    // One band per line, each over whole words.
    let bands = highlights(&frame(&mut ui, &held));
    assert_eq!(bands.len(), 2, "{bands:?}");
    assert!(near(
        bounds(&bands[0]),
        screen_rect_of(&held, Rect::new(450.0, 100.0, 650.0, 60.0))
    ));
    assert!(near(
        bounds(&bands[1]),
        screen_rect_of(&held, Rect::new(100.0, 300.0, 800.0, 60.0))
    ));
    // Back onto the first word: only that word stays selected.
    out.extend(move_to(&mut ui, &held, word_center(&held, 1)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("beta"));

    // The release ends the drag and keeps what it selected.
    out.extend(move_to(&mut ui, &held, word_center(&held, 3)));
    out.extend(release(
        &mut ui,
        &held,
        word_center(&held, 3),
        PointerButton::Left,
    ));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("beta gamma\ndelta")
    );
    out.extend(move_to(&mut ui, &held, word_center(&held, 5)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("beta gamma\ndelta")
    );
    assert!(!zooms_or_pans(&out), "{out:?}");
    assert_eq!(
        key(&mut ui, &held, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("beta gamma\ndelta".into())]
    );
}

#[test]
fn double_click_drag_backwards() {
    let l = layout();
    let t = ms(10_000);
    let held = timed(fit_ctx(&l), t + ms(200));
    let mut ui = Ui::new();
    frame(&mut ui, &held);

    let mut out = double_press(&mut ui, &held, t, word_center(&held, 4));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("epsilon"));
    // Back against the reading order: the double-clicked word stays
    // selected, and whole words before it join.
    out.extend(move_to(&mut ui, &held, word_center(&held, 1)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("beta gamma\ndelta epsilon")
    );
    out.extend(move_to(&mut ui, &held, word_center(&held, 0)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );
    // Left of the first word, the nearest word is still the first one.
    out.extend(move_to(
        &mut ui,
        &held,
        on_screen(&held, Point::new(20.0, 130.0)),
    ));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );
    // Forward past the double-clicked word: it stays the first one.
    out.extend(move_to(&mut ui, &held, word_center(&held, 5)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("epsilon\nzeta"));
    out.extend(move_to(&mut ui, &held, word_center(&held, 3)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("delta epsilon"));
    out.extend(release(
        &mut ui,
        &held,
        word_center(&held, 3),
        PointerButton::Left,
    ));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("delta epsilon"));
    assert!(!zooms_or_pans(&out), "{out:?}");
}

#[test]
fn triple_click_drag_selects_lines() {
    let l = layout();
    let t = ms(10_000);
    let held = timed(fit_ctx(&l), t + ms(300));
    let mut ui = Ui::new();
    frame(&mut ui, &held);

    // The third press on "epsilon" selects its line.
    let mut out = triple_press(&mut ui, &held, t, word_center(&held, 4));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("delta epsilon"));
    // With the button held, a drag grows the selection by whole lines.
    out.extend(move_to(&mut ui, &held, word_center(&held, 5)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("delta epsilon\nzeta")
    );
    // Up, against the reading order: the triple-clicked line stays.
    out.extend(move_to(&mut ui, &held, word_center(&held, 1)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );
    assert_eq!(highlights(&frame(&mut ui, &held)).len(), 2);
    // Back on the first word of the triple-clicked line: only that line.
    out.extend(move_to(&mut ui, &held, word_center(&held, 3)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("delta epsilon"));
    // The release ends the drag and keeps what it selected.
    out.extend(move_to(&mut ui, &held, word_center(&held, 0)));
    out.extend(release(
        &mut ui,
        &held,
        word_center(&held, 0),
        PointerButton::Left,
    ));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );
    out.extend(move_to(&mut ui, &held, word_center(&held, 5)));
    assert_eq!(
        ui.selected_text(&held).as_deref(),
        Some("alpha beta gamma\ndelta epsilon")
    );
    assert!(!zooms_or_pans(&out), "{out:?}");
}

#[test]
fn double_click_selects_one_word() {
    let l = layout();
    let t = ms(10_000);
    let held = timed(fit_ctx(&l), t + ms(200));
    let mut ui = Ui::new();
    frame(&mut ui, &held);

    let p = word_center(&held, 2);
    let mut out = double_press(&mut ui, &held, t, p);
    assert_eq!(ui.selected_text(&held).as_deref(), Some("gamma"));
    // A hand that shakes a little while the button is down changes
    // nothing.
    out.extend(move_to(&mut ui, &held, Point::new(p.x + 2.0, p.y - 1.0)));
    assert_eq!(ui.selected_text(&held).as_deref(), Some("gamma"));
    out.extend(release(
        &mut ui,
        &held,
        Point::new(p.x + 2.0, p.y - 1.0),
        PointerButton::Left,
    ));
    // Exactly the word, after the release too, and no zoom.
    assert_eq!(ui.selected_text(&held).as_deref(), Some("gamma"));
    assert!(
        !zooms_or_pans(&out),
        "a double click on text is not a zoom: {out:?}"
    );
    let bands = highlights(&frame(&mut ui, &held));
    assert_eq!(bands.len(), 1);
    assert!(near(
        bounds(&bands[0]),
        screen_rect_of(&held, Rect::new(750.0, 100.0, 350.0, 60.0))
    ));
    // A plain click on a word later still clears the selection.
    let later = timed(fit_ctx(&l), t + ms(2_000));
    click(&mut ui, &later, word_center(&later, 0));
    assert_eq!(ui.selected_text(&later), None);
}

#[test]
fn double_click_off_text_zooms() {
    let l = layout();
    let t = ms(10_000);
    let ctx = timed(fit_ctx(&l), t + ms(200));
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);

    // A double click on the image away from the words: the interface
    // does not take it as text. It asks for 1:1 at the pointer, selects
    // nothing and the held button pans.
    let blank = on_screen(&ctx, Point::new(1500.0, 800.0));
    move_to(&mut ui, &ctx, blank);
    assert_ne!(ui.cursor(), CursorShape::Text);
    let out = double_press(&mut ui, &ctx, t, blank);
    assert!(
        out.contains(&Action::OneToOne {
            anchor: Some(blank)
        }),
        "{out:?}"
    );
    assert_eq!(ui.selected_text(&ctx), None);
    let out = move_to(&mut ui, &ctx, Point::new(blank.x - 30.0, blank.y));
    assert_eq!(out, vec![Action::PanBy { dx: -30.0, dy: 0.0 }]);
    assert_eq!(ui.cursor(), CursorShape::Grabbing);
    release(
        &mut ui,
        &ctx,
        Point::new(blank.x - 30.0, blank.y),
        PointerButton::Left,
    );
    assert_eq!(ui.selected_text(&ctx), None);

    // At 1:1 a double click away from the words fits the image again.
    let view = ViewState {
        mode: DisplayMode::OneToOne,
        ..ViewState::default()
    };
    let t = ms(20_000);
    let one = timed(ctx_with(&l, W, H, view, Affine::IDENTITY), t + ms(200));
    let blank = on_screen(&one, Point::new(1300.0, 700.0));
    let out = double_press(&mut ui, &one, t, blank);
    assert!(kinds(&out).contains(&ActionKind::FitToWindow), "{out:?}");
    release(&mut ui, &one, blank, PointerButton::Left);
    assert_eq!(ui.selected_text(&one), None);

    // A double click off text also clears a selection, as a single click
    // off text does, and still zooms.
    let t = ms(30_000);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    assert_eq!(ui.selected_text(&ctx).as_deref(), Some(ALL_TEXT));
    let fit = timed(fit_ctx(&l), t + ms(200));
    let blank = on_screen(&fit, Point::new(1500.0, 800.0));
    let out = double_press(&mut ui, &fit, t, blank);
    assert!(kinds(&out).contains(&ActionKind::OneToOne), "{out:?}");
    release(&mut ui, &fit, blank, PointerButton::Left);
    assert_eq!(ui.selected_text(&fit), None);
}

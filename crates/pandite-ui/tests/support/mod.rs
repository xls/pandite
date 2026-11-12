//! Shared helpers for the integration tests: a fixed-advance text measure,
//! a recording texture sink, context builders and draw list queries.

#![allow(dead_code)]

use pandite_core::{
    Color, DrawCmd, DrawList, ImageBuf, Point, Rect, Size, TextMeasure, TextureId, TextureSink,
    ViewState,
};
use pandite_ui::{
    thumbnail_index, Action, ActionKind, Icon, ImageInfo, InputEvent, Key, Modifiers,
    PointerButton, Ui, UiContext, WheelDelta,
};
use std::time::Duration;

/// Every character advances half the font size; lines are 1.25 sizes tall.
pub struct FakeMeasure;

impl TextMeasure for FakeMeasure {
    fn measure(&self, text: &str, size: f32) -> Size {
        Size::new(text.chars().count() as f32 * size * 0.5, size * 1.25)
    }
}

/// Records uploads.
#[derive(Default)]
pub struct FakeSink {
    pub uploads: Vec<(TextureId, u32, u32, bool)>,
    pub removed: Vec<TextureId>,
}

impl TextureSink for FakeSink {
    fn upload(&mut self, id: TextureId, image: &ImageBuf, alpha_mask: bool) {
        self.uploads
            .push((id, image.width(), image.height(), alpha_mask));
    }
    fn remove(&mut self, id: TextureId) {
        self.removed.push(id);
    }
}

pub fn ms(v: u64) -> Duration {
    Duration::from_millis(v)
}

/// A 1280 x 800 window at scale 1 with ten images, the fourth current,
/// and no image open yet.
pub fn base_ctx() -> UiContext<'static> {
    UiContext {
        window_size: Size::new(1280.0, 800.0),
        scale_factor: 1.0,
        count: 10,
        index: 3,
        gallery_visible: true,
        ..UiContext::default()
    }
}

/// `ctx` with a `w` x `h` image fitted into the content area.
pub fn with_image(mut ctx: UiContext<'static>, w: u32, h: u32) -> UiContext<'static> {
    let content = Ui::new().content_rect(&ctx);
    let view = ViewState::default();
    let size = Size::new(w as f32, h as f32);
    let rect = view.image_rect(size, content);
    let scale = view.scale(size, content.size());
    ctx.image = Some(ImageInfo {
        file_name: "photo.jpg",
        width: w,
        height: h,
        view,
        zoom_percent: scale * 100.0,
        screen_rect: rect,
        edited: false,
    });
    ctx
}

/// The default context with a 4000 x 3000 image.
pub fn image_ctx() -> UiContext<'static> {
    with_image(base_ctx(), 4000, 3000)
}

pub fn at(mut ctx: UiContext<'static>, t: Duration) -> UiContext<'static> {
    ctx.now = t;
    ctx
}

pub fn frame(ui: &mut Ui, ctx: &UiContext<'_>) -> DrawList {
    let mut list = DrawList::new();
    ui.build(ctx, &FakeMeasure, &mut list);
    list
}

pub fn texts(list: &DrawList) -> Vec<String> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

pub fn has_text(list: &DrawList, needle: &str) -> bool {
    texts(list).iter().any(|t| t.contains(needle))
}

/// The box of the first text run containing `needle`.
pub fn text_rect(list: &DrawList, needle: &str) -> Option<Rect> {
    list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text {
            pos, text, size, ..
        } if text.contains(needle) => Some(Rect::new(
            pos.x,
            pos.y,
            text.chars().count() as f32 * size * 0.5,
            size * 1.25,
        )),
        _ => None,
    })
}

/// The text run equal to `exact`.
pub fn text_rect_exact(list: &DrawList, exact: &str) -> Option<Rect> {
    list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text {
            pos, text, size, ..
        } if text == exact => Some(Rect::new(
            pos.x,
            pos.y,
            text.chars().count() as f32 * size * 0.5,
            size * 1.25,
        )),
        _ => None,
    })
}

pub fn text_color(list: &DrawList, needle: &str) -> Option<Color> {
    list.cmds.iter().find_map(|c| match c {
        DrawCmd::Text { text, color, .. } if text.contains(needle) => Some(*color),
        _ => None,
    })
}

/// Rectangles and tints where `icon` is drawn.
pub fn icon_draws(list: &DrawList, icon: Icon) -> Vec<(Rect, Color)> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Image {
                rect,
                texture,
                tint,
                ..
            } if *texture == icon.texture_id() => Some((*rect, *tint)),
            _ => None,
        })
        .collect()
}

/// Thumbnail draws: (index, rect).
pub fn thumbnail_draws(list: &DrawList) -> Vec<(usize, Rect)> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Image { rect, texture, .. } => thumbnail_index(*texture).map(|i| (i, *rect)),
            _ => None,
        })
        .collect()
}

pub fn center(r: Rect) -> Point {
    r.center()
}

pub fn send(ui: &mut Ui, ctx: &UiContext<'_>, event: InputEvent) -> Vec<Action> {
    ui.handle_event(event, ctx)
}

pub fn move_to(ui: &mut Ui, ctx: &UiContext<'_>, p: Point) -> Vec<Action> {
    ui.handle_event(InputEvent::PointerMove { pos: p }, ctx)
}

pub fn press(ui: &mut Ui, ctx: &UiContext<'_>, p: Point, button: PointerButton) -> Vec<Action> {
    ui.handle_event(InputEvent::PointerDown { pos: p, button }, ctx)
}

pub fn release(ui: &mut Ui, ctx: &UiContext<'_>, p: Point, button: PointerButton) -> Vec<Action> {
    ui.handle_event(InputEvent::PointerUp { pos: p, button }, ctx)
}

/// Move, press and release the left button at `p`; all actions in order.
pub fn click(ui: &mut Ui, ctx: &UiContext<'_>, p: Point) -> Vec<Action> {
    let mut out = move_to(ui, ctx, p);
    out.extend(press(ui, ctx, p, PointerButton::Left));
    out.extend(release(ui, ctx, p, PointerButton::Left));
    out
}

/// Press at `a`, move to `b` in a few steps, release at `b`.
pub fn drag(
    ui: &mut Ui,
    ctx: &UiContext<'_>,
    a: Point,
    b: Point,
    button: PointerButton,
) -> Vec<Action> {
    let mut out = move_to(ui, ctx, a);
    out.extend(press(ui, ctx, a, button));
    for k in 1..=4 {
        let f = k as f32 / 4.0;
        let p = Point::new(a.x + (b.x - a.x) * f, a.y + (b.y - a.y) * f);
        out.extend(move_to(ui, ctx, p));
    }
    out.extend(release(ui, ctx, b, button));
    out
}

pub fn key(ui: &mut Ui, ctx: &UiContext<'_>, k: Key, mods: Modifiers) -> Vec<Action> {
    let mut out = ui.handle_event(
        InputEvent::KeyDown {
            key: k,
            mods,
            repeat: false,
        },
        ctx,
    );
    out.extend(ui.handle_event(InputEvent::KeyUp { key: k, mods }, ctx));
    out
}

pub fn press_key(ui: &mut Ui, ctx: &UiContext<'_>, k: Key) -> Vec<Action> {
    key(ui, ctx, k, Modifiers::NONE)
}

pub fn type_text(ui: &mut Ui, ctx: &UiContext<'_>, s: &str) -> Vec<Action> {
    ui.handle_event(InputEvent::Text(s.to_string()), ctx)
}

pub fn wheel(ui: &mut Ui, ctx: &UiContext<'_>, p: Point, lines: f32) -> Vec<Action> {
    ui.handle_event(
        InputEvent::Wheel {
            pos: p,
            delta: WheelDelta::Lines { x: 0.0, y: lines },
        },
        ctx,
    )
}

pub fn kinds(actions: &[Action]) -> Vec<ActionKind> {
    actions.iter().map(Action::kind).collect()
}

/// Click the text run that reads exactly `label`; panics when it is not
/// drawn.
pub fn click_label(ui: &mut Ui, ctx: &UiContext<'_>, label: &str) -> Vec<Action> {
    let list = frame(ui, ctx);
    let r = text_rect_exact(&list, label)
        .unwrap_or_else(|| panic!("label {label:?} not drawn; texts: {:?}", texts(&list)));
    click(ui, ctx, r.center())
}

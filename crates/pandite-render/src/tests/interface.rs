//! G5: the interface pass draws every DrawCmd.

use super::*;
use pandite_core::{Point, TextMeasure, TextureId, TextureSink};

fn ui_renderer(w: u32, h: u32) -> Option<Renderer> {
    let gpu = gpu()?;
    Some(offscreen(&gpu, w, h, plain(Color::BLACK)))
}

fn draw(r: &mut Renderer, list: &DrawList) -> Pixels {
    r.clear_content();
    frame(r, &ViewState::default(), list)
}

#[test]
fn drawlist_rect_readback() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    let mut list = DrawList::new();
    list.rect(
        Rect::new(8.0, 8.0, 16.0, 16.0),
        Color::rgb(200, 100, 50),
        0.0,
    );
    // 50 % white over black blends in linear light: 0.5 is sRGB 188.
    list.rect(
        Rect::new(32.0, 32.0, 16.0, 16.0),
        Color::rgba(255, 255, 255, 128),
        0.0,
    );
    // Later commands draw on top.
    list.rect(Rect::new(20.0, 20.0, 8.0, 8.0), Color::rgb(0, 0, 255), 0.0);
    let px = draw(&mut r, &list);
    assert_rgb(&px, 8, 8, [200, 100, 50], 1);
    assert_rgb(&px, 19, 12, [200, 100, 50], 1);
    // Integer edges are crisp: no fringe outside.
    assert_rgb(&px, 7, 8, [0, 0, 0], 0);
    assert_rgb(&px, 8, 7, [0, 0, 0], 0);
    assert_rgb(&px, 24, 12, [0, 0, 0], 0);
    assert_rgb(&px, 23, 23, [0, 0, 255], 1);
    assert_rgb(&px, 40, 40, [srgb_code(128.0 / 255.0); 3], 1);
    assert_rgb(&px, 48, 40, [0, 0, 0], 0);
    // A rectangle on half pixels covers its edge pixels by one half.
    let mut list = DrawList::new();
    list.rect(Rect::new(10.5, 10.0, 4.0, 4.0), Color::WHITE, 0.0);
    let px = draw(&mut r, &list);
    assert_rgb(&px, 10, 12, [srgb_code(0.5); 3], 2);
    assert_rgb(&px, 12, 12, [255; 3], 0);
    assert_rgb(&px, 14, 12, [srgb_code(0.5); 3], 2);

    // Batching: shapes, images and text in one list need few draw calls,
    // and steady-state frames do not grow anything.
    let mut list = DrawList::new();
    for i in 0..200 {
        list.rect(
            Rect::new((i % 60) as f32, (i / 60) as f32 * 5.0, 3.0, 3.0),
            Color::WHITE,
            1.0,
        );
        list.border(Rect::new(1.0, 1.0, 20.0, 20.0), Color::WHITE, 1.0, 2.0);
    }
    list.text(Point::new(2.0, 30.0), "batched", 14.0, Color::WHITE);
    list.rect(Rect::new(0.0, 50.0, 10.0, 10.0), Color::WHITE, 0.0);
    let mut buffer = None;
    for _ in 0..3 {
        draw(&mut r, &list);
        assert_eq!(r.ui_draw_calls(), 1, "shapes and text share one draw call");
        let current = r.ui_instance_buffer().expect("instance buffer");
        if let Some(previous) = &buffer {
            assert_eq!(
                previous, &current,
                "steady-state frames reuse the instance buffer"
            );
        }
        buffer = Some(current);
    }
}

#[test]
fn drawlist_quad_readback() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    // A diamond around (32, 32) with corners 20 pixels out.
    let mut list = DrawList::new();
    list.quad(
        [
            Point::new(32.0, 12.0),
            Point::new(52.0, 32.0),
            Point::new(32.0, 52.0),
            Point::new(12.0, 32.0),
        ],
        Color::rgb(200, 100, 50),
    );
    // A translucent quad over part of it blends like a rectangle.
    list.quad(
        [
            Point::new(40.0, 40.0),
            Point::new(60.0, 40.0),
            Point::new(60.0, 60.0),
            Point::new(40.0, 60.0),
        ],
        Color::rgba(255, 255, 255, 128),
    );
    let px = draw(&mut r, &list);
    assert_rgb(&px, 32, 32, [200, 100, 50], 1);
    assert_rgb(&px, 32, 14, [200, 100, 50], 1);
    assert_rgb(&px, 14, 32, [200, 100, 50], 1);
    // Outside the diamond, near the corners of its bounding box.
    assert_rgb(&px, 14, 14, [0, 0, 0], 0);
    assert_rgb(&px, 50, 14, [0, 0, 0], 0);
    assert_rgb(&px, 14, 50, [0, 0, 0], 0);
    assert_rgb(&px, 55, 55, [srgb_code(128.0 / 255.0); 3], 1);
    assert_rgb(&px, 39, 55, [0, 0, 0], 0);
    // A quad joins the shape batch: one draw call.
    assert_eq!(r.ui_draw_calls(), 1);
}

#[test]
fn drawlist_rounded_rect_corners() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    let mut list = DrawList::new();
    list.rect(Rect::new(10.0, 10.0, 40.0, 40.0), Color::WHITE, 12.0);
    let px = draw(&mut r, &list);
    // The corner pixel lies outside the arc.
    assert_rgb(&px, 10, 10, [0, 0, 0], 0);
    assert_rgb(&px, 11, 11, [0, 0, 0], 0);
    // Straight edges and the middle are fully covered.
    assert_rgb(&px, 30, 10, [255; 3], 0);
    assert_rgb(&px, 10, 30, [255; 3], 0);
    assert_rgb(&px, 30, 30, [255; 3], 0);
    // Along the diagonal of the top-left corner, coverage rises smoothly
    // from 0 to 1 and the arc pixel (13, 13) is partly covered.
    let diag: Vec<u8> = (10..22).map(|i| px.rgb(i, i)[0]).collect();
    assert!(diag.windows(2).all(|w| w[0] <= w[1]), "{diag:?}");
    let arc = px.rgb(13, 13)[0];
    assert!(
        arc > 40 && arc < 230,
        "antialiased arc pixel {arc}, diagonal {diag:?}"
    );
    // All four corners are rounded the same way.
    for (x, y) in [(49, 10), (10, 49), (49, 49)] {
        assert_rgb(&px, x, y, [0, 0, 0], 0);
    }
    assert_eq!(px.rgb(46, 13), px.rgb(13, 13));
    assert_eq!(px.rgb(13, 46), px.rgb(13, 13));
    // Radius 0 keeps the corner.
    let mut list = DrawList::new();
    list.rect(Rect::new(10.0, 10.0, 40.0, 40.0), Color::WHITE, 0.0);
    let px = draw(&mut r, &list);
    assert_rgb(&px, 10, 10, [255; 3], 0);
}

#[test]
fn drawlist_border_readback() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    let mut list = DrawList::new();
    list.border(
        Rect::new(10.0, 10.0, 40.0, 30.0),
        Color::rgb(255, 0, 0),
        3.0,
        0.0,
    );
    let px = draw(&mut r, &list);
    for x in 10..13 {
        assert_rgb(&px, x, 25, [255, 0, 0], 1);
    }
    for y in 37..40 {
        assert_rgb(&px, 30, y, [255, 0, 0], 1);
    }
    assert_rgb(&px, 47, 25, [255, 0, 0], 1);
    assert_rgb(&px, 49, 25, [255, 0, 0], 1);
    // Drawn inside the rectangle: nothing outside, interior empty.
    assert_rgb(&px, 9, 25, [0, 0, 0], 0);
    assert_rgb(&px, 50, 25, [0, 0, 0], 0);
    assert_rgb(&px, 13, 25, [0, 0, 0], 0);
    assert_rgb(&px, 30, 25, [0, 0, 0], 0);
    // Rounded border: the corner stays empty, the arc is drawn.
    let mut list = DrawList::new();
    list.border(Rect::new(10.0, 10.0, 40.0, 40.0), Color::WHITE, 2.0, 10.0);
    let px = draw(&mut r, &list);
    assert_rgb(&px, 10, 10, [0, 0, 0], 0);
    assert_rgb(&px, 30, 10, [255; 3], 0);
    assert_rgb(&px, 30, 30, [0, 0, 0], 0);
    let arc_max = (10..20).map(|i| px.rgb(i, i)[0]).max().unwrap_or(0);
    assert!(arc_max > 150, "the rounded corner arc is drawn: {arc_max}");
}

#[test]
fn drawlist_image_and_tint_readback() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    let quad = rgba8(2, 2, |x, y| match (x, y) {
        (0, 0) => [255, 0, 0, 255],
        (1, 0) => [0, 255, 0, 255],
        (0, 1) => [0, 0, 255, 255],
        _ => [255, 255, 255, 255],
    });
    r.upload(TextureId(1), &quad, false);
    let white = rgba8(4, 4, |_, _| [255, 255, 255, 255]);
    r.upload(TextureId(2), &white, false);
    // Color is ignored for masks: only alpha counts.
    let mask = rgba8(4, 4, |x, _| [255, 0, 255, if x < 2 { 255 } else { 0 }]);
    r.upload(TextureId(3), &mask, true);

    let mut list = DrawList::new();
    // 1:1 copy, then the same texture magnified 8x.
    list.image(Rect::new(0.0, 0.0, 2.0, 2.0), TextureId(1), Color::WHITE);
    list.image(Rect::new(16.0, 0.0, 16.0, 16.0), TextureId(1), Color::WHITE);
    // Tint multiplies: opaque red, then half-transparent red over black.
    list.image(
        Rect::new(0.0, 32.0, 8.0, 8.0),
        TextureId(2),
        Color::rgb(255, 0, 0),
    );
    list.image(
        Rect::new(16.0, 32.0, 8.0, 8.0),
        TextureId(2),
        Color::rgba(255, 0, 0, 128),
    );
    // Alpha mask tinted green: an icon.
    list.image(
        Rect::new(40.0, 32.0, 16.0, 16.0),
        TextureId(3),
        Color::rgb(0, 255, 0),
    );
    // A sub-rectangle of the texture through `uv`.
    list.cmds.push(pandite_core::DrawCmd::Image {
        rect: Rect::new(40.0, 0.0, 8.0, 8.0),
        texture: TextureId(1),
        uv: Rect::new(0.5, 0.5, 0.5, 0.5),
        tint: Color::WHITE,
    });
    let px = draw(&mut r, &list);
    assert_rgb(&px, 0, 0, [255, 0, 0], 1);
    assert_rgb(&px, 1, 0, [0, 255, 0], 1);
    assert_rgb(&px, 0, 1, [0, 0, 255], 1);
    assert_rgb(&px, 1, 1, [255, 255, 255], 1);
    assert_rgb(&px, 18, 2, [255, 0, 0], 2);
    assert_rgb(&px, 29, 13, [255, 255, 255], 2);
    assert_rgb(&px, 4, 36, [255, 0, 0], 1);
    assert_rgb(&px, 20, 36, [srgb_code(128.0 / 255.0), 0, 0], 2);
    assert_rgb(&px, 42, 40, [0, 255, 0], 1);
    assert_rgb(&px, 53, 40, [0, 0, 0], 1);
    assert_rgb(&px, 44, 4, [255, 255, 255], 2);
}

#[test]
fn drawlist_clip_limits_draws() {
    let Some(mut r) = ui_renderer(64, 64) else {
        return;
    };
    let mut list = DrawList::new();
    list.clip(Some(Rect::new(10.0, 10.0, 20.0, 20.0)));
    list.text(Point::new(0.0, 0.0), "clipped text", 40.0, Color::WHITE);
    list.rect(Rect::new(0.0, 0.0, 64.0, 64.0), Color::rgb(255, 0, 0), 0.0);
    list.clip(None);
    list.rect(
        Rect::new(40.0, 40.0, 10.0, 10.0),
        Color::rgb(0, 255, 0),
        0.0,
    );
    // A clip entirely outside the target hides what follows.
    list.clip(Some(Rect::new(100.0, 100.0, 10.0, 10.0)));
    list.rect(Rect::new(0.0, 50.0, 10.0, 10.0), Color::WHITE, 0.0);
    // Fractional clips follow pixel centers.
    list.clip(Some(Rect::new(52.6, 2.0, 5.0, 5.0)));
    list.rect(Rect::new(48.0, 0.0, 16.0, 16.0), Color::rgb(0, 0, 255), 0.0);
    list.clip(None);
    let px = draw(&mut r, &list);
    assert_rgb(&px, 5, 5, [0, 0, 0], 0);
    assert_rgb(&px, 9, 15, [0, 0, 0], 0);
    assert_rgb(&px, 10, 10, [255, 0, 0], 2);
    assert_rgb(&px, 29, 29, [255, 0, 0], 0);
    assert_rgb(&px, 30, 29, [0, 0, 0], 0);
    assert_rgb(&px, 35, 35, [0, 0, 0], 0);
    assert_rgb(&px, 45, 45, [0, 255, 0], 0);
    assert_rgb(&px, 5, 55, [0, 0, 0], 0);
    assert_rgb(&px, 52, 4, [0, 0, 0], 0);
    assert_rgb(&px, 53, 4, [0, 0, 255], 0);
    assert_rgb(&px, 57, 4, [0, 0, 255], 0);
    assert_rgb(&px, 58, 4, [0, 0, 0], 0);
    // Nothing of the clipped text escaped the clip.
    for y in 0..64 {
        for x in 0..64 {
            let inside = (10..30).contains(&x) && (10..30).contains(&y);
            if !inside && !(40..50).contains(&x) && !(53..58).contains(&x) {
                assert_rgb(&px, x, y, [0, 0, 0], 0);
            }
        }
    }
}

/// Bounding box of pixels brighter than `threshold`, as
/// (left, top, right, bottom) with right and bottom exclusive.
fn ink_box(px: &Pixels, threshold: u8) -> Option<(u32, u32, u32, u32)> {
    let mut bbox: Option<(u32, u32, u32, u32)> = None;
    for y in 0..px.height {
        for x in 0..px.width {
            if px.rgb(x, y).iter().any(|&c| c > threshold) {
                bbox = Some(match bbox {
                    None => (x, y, x + 1, y + 1),
                    Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)),
                });
            }
        }
    }
    bbox
}

#[test]
fn drawlist_text_draws_glyphs() {
    let Some(mut r) = ui_renderer(240, 48) else {
        return;
    };
    let mut list = DrawList::new();
    list.text(
        Point::new(6.0, 8.0),
        "Pandite 1:1",
        22.0,
        Color::rgb(255, 64, 32),
    );
    let px = draw(&mut r, &list);
    let mut ink = 0;
    let mut solid = 0;
    for y in 0..48 {
        for x in 0..240 {
            let [r8, g8, b8] = px.rgb(x, y);
            if r8 > 32 {
                ink += 1;
                // The text color, scaled by coverage: red dominates.
                assert!(r8 >= g8 && g8 >= b8, "({x}, {y}) {r8} {g8} {b8}");
            }
            if near([r8, g8, b8], [255, 64, 32], 2) {
                solid += 1;
            }
        }
    }
    assert!(ink > 200, "glyph pixels drawn: {ink}");
    assert!(solid > 40, "stems reach the full text color: {solid}");
    let size = r.measure("Pandite 1:1", 22.0);
    let (left, top, right, bottom) = ink_box(&px, 32).expect("ink");
    assert!(left >= 6 && top >= 8, "ink starts inside the line box");
    assert!(right as f32 <= 6.0 + size.w + 1.0 && bottom as f32 <= 8.0 + size.h + 1.0);

    // Different text draws different glyphs; empty text draws nothing.
    let mut other = DrawList::new();
    other.text(
        Point::new(6.0, 8.0),
        "xxxxxxx",
        22.0,
        Color::rgb(255, 64, 32),
    );
    let px2 = draw(&mut r, &other);
    assert_ne!(px.data, px2.data);
    let mut empty = DrawList::new();
    empty.text(Point::new(6.0, 8.0), "", 22.0, Color::WHITE);
    empty.text(Point::new(6.0, 8.0), "   ", 22.0, Color::WHITE);
    assert!(ink_box(&draw(&mut r, &empty), 0).is_none());
}

#[test]
fn text_measure_matches_drawn_extent() {
    let Some(mut r) = ui_renderer(420, 90) else {
        return;
    };
    // The embedded font and the system font (or its fallback) alike.
    for font in [crate::FontChoice::Embedded, crate::FontChoice::System] {
        let mut settings = r.settings().clone();
        settings.font = font;
        r.set_settings(settings);
        let measurer = r.text_measurer();
        let name = measurer.font_name().to_string();
        for (text, size) in [
            ("Hamburgefonstiv", 16.0),
            ("MINIMUM", 28.0),
            ("pandite 0123456789", 13.0),
            ("Save as...", 21.5),
            ("HIH", 60.0),
        ] {
            let measured = r.measure(text, size);
            assert_eq!(
                measured,
                measurer.measure(text, size),
                "measurer agrees with renderer"
            );
            let pos = Point::new(10.0, 12.0);
            let mut list = DrawList::new();
            list.text(pos, text, size, Color::WHITE);
            let px = draw(&mut r, &list);
            let (left, top, right, bottom) = ink_box(&px, 24).expect("ink");
            let (left, top, right, bottom) = (left as f32, top as f32, right as f32, bottom as f32);
            let bearing = 0.15 * size;
            // Horizontal: ink spans the measured advance up to the side
            // bearings.
            assert!(
                left >= pos.x - 1.0 && left <= pos.x + bearing,
                "{name} {text}: left {left}"
            );
            let end = pos.x + measured.w;
            assert!(
                right <= end + 1.0 && right >= end - bearing,
                "{name} {text}: ink right {right}, measured end {end}"
            );
            // Vertical: ink inside the measured line box.
            assert!(top >= pos.y - 1.0, "{name} {text}: top {top}");
            assert!(
                bottom <= pos.y + measured.h + 1.0,
                "{name} {text}: bottom {bottom}, line {}",
                measured.h
            );
            // Cap height fills a sensible part of the line.
            assert!(
                bottom - top >= 0.5 * size,
                "{name} {text}: ink height {}",
                bottom - top
            );
        }
    }
}

#[test]
fn texture_sink_upload_and_remove() {
    let Some(mut r) = ui_renderer(32, 32) else {
        return;
    };
    let id = TextureId(7);
    let mut list = DrawList::new();
    list.image(Rect::new(8.0, 8.0, 16.0, 16.0), id, Color::WHITE);
    // Drawing an id that was never uploaded is skipped, not an error.
    let px = draw(&mut r, &list);
    assert_rgb(&px, 16, 16, [0, 0, 0], 0);

    r.upload(id, &rgba8(3, 3, |_, _| [255, 0, 0, 255]), false);
    assert!(r.has_texture(id));
    assert_rgb(&draw(&mut r, &list), 16, 16, [255, 0, 0], 1);

    // Uploading the same id replaces the texture, even with a new size.
    r.upload(id, &rgba8(5, 2, |_, _| [0, 0, 255, 255]), false);
    assert_rgb(&draw(&mut r, &list), 16, 16, [0, 0, 255], 1);

    r.remove(id);
    assert!(!r.has_texture(id));
    assert_rgb(&draw(&mut r, &list), 16, 16, [0, 0, 0], 0);
    // Unknown ids are ignored.
    r.remove(TextureId(12345));
    assert_eq!(r.take_texture_error(), None);

    // A failed upload leaves the id unset and is reported, not drawn.
    let side = r.gpu().max_texture_size() + 1;
    r.upload(TextureId(9), &rgba8(side, 1, |_, _| [0, 0, 0, 255]), true);
    assert!(!r.has_texture(TextureId(9)));
    assert!(r.take_texture_error().is_some());
    assert_eq!(r.take_texture_error(), None);

    // A thumbnail drawn smaller than its size is filtered through mips:
    // a fine checker becomes flat gray instead of aliasing.
    let checker = rgba8(64, 64, |x, y| {
        if (x + y) % 2 == 0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    r.upload(TextureId(8), &checker, false);
    let mut list = DrawList::new();
    list.image(Rect::new(4.0, 4.0, 13.0, 13.0), TextureId(8), Color::WHITE);
    let px = draw(&mut r, &list);
    for y in 6..15 {
        for x in 6..15 {
            assert_rgb(&px, x, y, [188; 3], 4);
        }
    }
}

//! G1: still images render to correct pixels.

use super::*;
use crate::color::srgb_to_linear;
use crate::{Checkerboard, GpuImage};
use pandite_core::{Content, DisplayMode, ImageBuf, PixelFormat, Point, Size};
use std::sync::Arc;

fn one_to_one() -> ViewState {
    ViewState {
        mode: DisplayMode::OneToOne,
        ..ViewState::default()
    }
}

/// True when the center of pixel (x, y) lies inside `rect`.
fn covers(rect: Rect, x: u32, y: u32) -> bool {
    rect.contains(Point::new(x as f32 + 0.5, y as f32 + 0.5))
}

#[test]
fn render_still_readback() {
    let gpu = gpu_or_skip!();
    let color = |x: u32, y: u32| {
        [
            (x * 60) as u8,
            (y * 80 + 20) as u8,
            (250 - x * 30 - y * 7) as u8,
            255,
        ]
    };
    let image = rgba8(5, 3, color);
    let background = Color::rgb(0, 0, 90);
    let mut r = offscreen(&gpu, 17, 16, plain(background));
    r.set_content(Content::Still(image)).expect("set content");
    let view = one_to_one();
    let px = frame(&mut r, &view, &DrawList::new());
    let placed = r.placement(&view, r.full_viewport()).expect("placement");
    // Centered 5x3 in 17x16: (6, 6.5) with the origin rounded to (6, 7).
    assert_eq!(placed.image_rect, Rect::new(6.0, 7.0, 5.0, 3.0));
    for y in 0..16 {
        for x in 0..17 {
            if covers(placed.image_rect, x, y) {
                let [r8, g8, b8, _] = color(x - 6, y - 7);
                assert_rgb(&px, x, y, [r8, g8, b8], 1);
            } else {
                assert_rgb(&px, x, y, [0, 0, 90], 0);
            }
        }
    }
}

#[test]
fn render_respects_image_rect() {
    let gpu = gpu_or_skip!();
    let image = rgba8(20, 10, |_, _| [255, 0, 0, 255]);
    let size = Size::new(20.0, 10.0);
    let mut r = offscreen(&gpu, 120, 80, plain(Color::rgb(0, 40, 0)));
    r.set_content(Content::Still(image)).expect("set content");
    let full = r.full_viewport();

    let mut views = Vec::new();
    // Fit, enlarged: 6x, 120 x 60 centered vertically.
    views.push(ViewState {
        enlarge_to_fit: true,
        ..ViewState::default()
    });
    // Zoom 2.5 around an off-center anchor, then panned.
    let mut zoomed = ViewState::default();
    zoomed.zoom_to(2.5, size, full, Point::new(30.0, 20.0));
    zoomed.pan_by(-7.0, 3.0, size, full.size());
    views.push(zoomed);
    // Strong zoom that overflows the target (crisp sampling path).
    let mut deep = ViewState::default();
    deep.zoom_to(9.0, size, full, Point::new(100.0, 70.0));
    views.push(deep);

    for view in views {
        let px = frame(&mut r, &view, &DrawList::new());
        let expected = view.image_rect(size, full);
        let placed = r.placement(&view, full).expect("placement").image_rect;
        assert_eq!(placed.x, expected.x.round());
        assert_eq!(placed.y, expected.y.round());
        assert_eq!((placed.w, placed.h), (expected.w, expected.h));
        for y in 0..80 {
            for x in 0..120 {
                let want = if covers(placed, x, y) {
                    [255, 0, 0]
                } else {
                    [0, 40, 0]
                };
                assert_rgb(&px, x, y, want, 1);
            }
        }
    }

    // A viewport smaller than the target (a panel below): the content is
    // placed in it and clipped to it.
    let viewport = Rect::new(0.0, 0.0, 120.0, 50.0);
    let mut view = ViewState::default();
    view.zoom_to(6.0, size, viewport, viewport.center());
    let px = frame_in(&mut r, &view, viewport, &DrawList::new());
    let placed = r.placement(&view, viewport).expect("placement").image_rect;
    assert!(
        placed.bottom() > 50.0,
        "the zoomed image overflows the viewport"
    );
    for y in 0..80 {
        for x in 0..120 {
            let inside = covers(placed, x, y) && y < 50;
            let want = if inside { [255, 0, 0] } else { [0, 40, 0] };
            assert_rgb(&px, x, y, want, 1);
        }
    }
}

/// The sRGB code of `fg` with alpha `a` over opaque `bg`, blended in linear light.
fn over(fg: [u8; 3], a: u8, bg: [u8; 3]) -> [u8; 3] {
    let alpha = f32::from(a) / 255.0;
    let mut out = [0u8; 3];
    for i in 0..3 {
        let f = srgb_to_linear(f32::from(fg[i]) / 255.0);
        let b = srgb_to_linear(f32::from(bg[i]) / 255.0);
        out[i] = srgb_code(f * alpha + b * (1.0 - alpha));
    }
    out
}

#[test]
fn render_transparency_background() {
    let gpu = gpu_or_skip!();
    // Left half fully transparent (its color must not leak), right half
    // half-transparent red.
    let image = rgba8(16, 8, |x, _| {
        if x < 8 {
            [0, 255, 0, 0]
        } else {
            [255, 0, 0, 128]
        }
    });
    let light = Color::rgb(200, 200, 200);
    let dark = Color::rgb(100, 100, 100);
    let settings = RenderSettings {
        checkerboard: Some(Checkerboard {
            light,
            dark,
            cell: 2.0,
        }),
        ..plain(Color::rgb(10, 20, 30))
    };
    let mut r = offscreen(&gpu, 32, 16, settings);
    r.set_content(Content::Still(image)).expect("set content");
    let view = one_to_one();
    let px = frame(&mut r, &view, &DrawList::new());
    let origin = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let (ox, oy) = (origin.x as u32, origin.y as u32);
    for j in 0..8 {
        for i in 0..16 {
            let cell_dark = ((i / 2) + (j / 2)) % 2 == 1;
            let checker = if cell_dark {
                [100, 100, 100]
            } else {
                [200, 200, 200]
            };
            let want = if i < 8 {
                checker
            } else {
                over([255, 0, 0], 128, checker)
            };
            assert_rgb(&px, ox + i, oy + j, want, 2);
        }
    }
    // Outside the image: the background color.
    assert_rgb(&px, 0, 0, [10, 20, 30], 0);

    // Checkerboard off: transparency shows the background color.
    let mut settings = r.settings().clone();
    settings.checkerboard = None;
    r.set_settings(settings);
    let px = frame(&mut r, &view, &DrawList::new());
    assert_rgb(&px, ox + 1, oy + 1, [10, 20, 30], 1);
    assert_rgb(
        &px,
        ox + 12,
        oy + 3,
        over([255, 0, 0], 128, [10, 20, 30]),
        2,
    );
}

fn f16_to_f32(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1f);
    let frac = f32::from(bits & 0x3ff);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => sign * f32::INFINITY,
        _ => sign * (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
    }
}

fn rgba16(values: &[[u16; 4]]) -> ImageBuf {
    let data: Vec<u8> = values
        .iter()
        .flat_map(|p| p.iter().flat_map(|c| c.to_ne_bytes()))
        .collect();
    ImageBuf::new(values.len() as u32, 1, PixelFormat::Rgba16, data).expect("rgba16")
}

fn rgba_f32(values: &[[f32; 4]]) -> ImageBuf {
    let data: Vec<u8> = values
        .iter()
        .flat_map(|p| p.iter().flat_map(|c| c.to_ne_bytes()))
        .collect();
    ImageBuf::new(values.len() as u32, 1, PixelFormat::RgbaF32, data).expect("f32")
}

#[test]
fn render_rgba16_and_hdr() {
    let gpu = gpu_or_skip!();
    let view = one_to_one();

    // 16-bit content keeps more than 8 bits: two values one 8-bit step
    // apart by less than a step are stored apart in the half-float texture.
    let wide = rgba16(&[
        [0, 0x8080, 0xffff, 0xffff],
        [0x8000, 0x8100, 0x1234, 0xffff],
        [0xffff, 0xffff, 0xffff, 0x8000],
    ]);
    let stored = GpuImage::upload(&gpu, &wide).expect("upload rgba16");
    let texels = gpu
        .read_texture(stored.tile_texture(0), 0)
        .expect("read back");
    let texel = |i: usize, c: usize| {
        let o = i * 8 + c * 2;
        f16_to_f32(u16::from_ne_bytes([texels[o], texels[o + 1]]))
    };
    assert_eq!(
        stored.tile_texture(0).format(),
        wgpu::TextureFormat::Rgba16Float
    );
    for (i, value) in [0x8000u16, 0x8100, 0x1234].into_iter().enumerate() {
        let want = srgb_to_linear(f32::from(value) / 65535.0);
        let got = texel(1, i);
        assert!(
            (got - want).abs() <= want * 2e-3,
            "channel {i}: {got} vs {want}"
        );
    }
    // Premultiplied on upload: half alpha white stores 0.5.
    assert!((texel(2, 0) - 0.5).abs() < 2e-3, "{}", texel(2, 0));

    let mut r = offscreen(&gpu, 8, 8, plain(Color::BLACK));
    r.set_content(Content::Still(wide)).expect("set rgba16");
    let px = frame(&mut r, &view, &DrawList::new());
    let o = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let (ox, oy) = (o.x as u32, o.y as u32);
    assert_rgb(&px, ox, oy, [0, 128, 255], 1);
    assert_rgb(&px, ox + 1, oy, [128, 129, 18], 1);
    assert_rgb(&px, ox + 2, oy, over([255, 255, 255], 128, [0, 0, 0]), 2);

    // Floating point (linear) content: identity below the knee, a smooth
    // shoulder above, never clipped to a flat plateau before 1.0.
    let hdr = rgba_f32(&[
        [0.5, 0.25, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
        [4.0, 4.0, 4.0, 1.0],
        [f32::NAN, 0.5, f32::INFINITY, 1.0],
        [2.0, 0.5, 0.0, 1.0],
    ]);
    r.set_content(Content::Still(hdr)).expect("set hdr");
    let px = frame(&mut r, &view, &DrawList::new());
    let o = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let (ox, oy) = (o.x as u32, o.y as u32);
    assert_rgb(&px, ox, oy, [srgb_code(0.5), srgb_code(0.25), 0], 1);
    let shoulder = |v: f32| 0.75 + 0.25 * (1.0 - (-(v - 0.75) / 0.25).exp());
    let one = srgb_code(shoulder(1.0));
    let four = srgb_code(shoulder(4.0));
    assert_rgb(&px, ox + 1, oy, [one, one, one], 1);
    assert_rgb(&px, ox + 2, oy, [four, four, four], 1);
    assert!(
        one < four && one < 250,
        "tone curve keeps 1.0 below 4.0: {one} {four}"
    );
    // NaN becomes 0; infinity is clamped and tone mapped like a highlight.
    let nan_px = px.rgb(ox + 3, oy);
    assert_eq!(nan_px[0], 0, "{nan_px:?}");
    // Hue is kept: channel ratios survive the shoulder.
    let warm = px.rgb(ox + 4, oy);
    let ratio =
        srgb_to_linear(f32::from(warm[1]) / 255.0) / srgb_to_linear(f32::from(warm[0]) / 255.0);
    assert!((ratio - 0.25).abs() < 0.03, "{warm:?} ratio {ratio}");

    // Exposure -1 EV halves the values: 1.0 shows as 0.5.
    let mut settings = r.settings().clone();
    settings.exposure = -1.0;
    r.set_settings(settings);
    let px = frame(&mut r, &view, &DrawList::new());
    assert_rgb(&px, ox + 1, oy, [srgb_code(0.5); 3], 1);
    assert_rgb(&px, ox, oy, [srgb_code(0.25), srgb_code(0.125), 0], 1);
    // Standard range content ignores exposure.
    let sdr = rgba8(1, 1, |_, _| [188, 188, 188, 255]);
    r.set_content(Content::Still(sdr)).expect("set sdr");
    let px = frame(&mut r, &view, &DrawList::new());
    let o = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    assert_rgb(&px, o.x as u32, o.y as u32, [188, 188, 188], 1);
}

#[test]
fn non_srgb_target_encodes_in_shader() {
    let gpu = gpu_or_skip!();
    // Window surfaces without an sRGB format get the encoding from the
    // shaders: the stored bytes must match an sRGB target.
    let image = rgba8(8, 8, |x, y| {
        [
            (x * 30) as u8,
            (y * 30) as u8,
            77,
            if x < 4 { 255 } else { 90 },
        ]
    });
    let settings = RenderSettings {
        checkerboard: Some(Checkerboard::default()),
        ..plain(Color::rgb(40, 50, 60))
    };
    let mut list = DrawList::new();
    list.rect(Rect::new(1.0, 1.0, 5.0, 5.0), Color::rgb(10, 200, 99), 0.0);
    let view = ViewState {
        mode: DisplayMode::Zoom(2.0),
        ..ViewState::default()
    };
    let mut frames = Vec::new();
    for format in [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
    ] {
        let mut r =
            Renderer::offscreen_with_format(Arc::clone(&gpu), 32, 32, format, settings.clone())
                .expect("renderer");
        r.set_content(Content::Still(image.clone()))
            .expect("content");
        frames.push(frame(&mut r, &view, &list));
    }
    for y in 0..32 {
        for x in 0..32 {
            assert_rgb(&frames[1], x, y, frames[0].rgb(x, y), 1);
        }
    }
}

#[test]
fn renderer_follows_resize() {
    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 32, 32, plain(Color::rgb(0, 0, 80)));
    r.set_content(Content::Still(rgba8(10, 10, |_, _| [255, 255, 0, 255])))
        .expect("content");
    let view = one_to_one();
    let px = frame(&mut r, &view, &DrawList::new());
    assert_eq!((px.width, px.height), (32, 32));
    assert_rgb(&px, 16, 16, [255, 255, 0], 1);
    // A new size re-centers the content in the new target.
    r.resize(64, 20);
    assert_eq!(r.size(), (64, 20));
    let px = frame(&mut r, &view, &DrawList::new());
    assert_eq!((px.width, px.height), (64, 20));
    let rect = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    assert_eq!(rect, Rect::new(27.0, 5.0, 10.0, 10.0));
    assert_rgb(&px, 27, 5, [255, 255, 0], 1);
    assert_rgb(&px, 26, 5, [0, 0, 80], 0);
    // A zero size (minimized window) skips frames instead of failing.
    r.resize(0, 0);
    let status = r
        .render(&view, r.full_viewport(), &DrawList::new())
        .expect("render");
    assert_eq!(status, crate::FrameStatus::Skipped);
    r.resize(8, 8);
    assert_eq!(
        r.render(&view, r.full_viewport(), &DrawList::new())
            .expect("render"),
        crate::FrameStatus::Drawn
    );
}

/// Leaf 1.6: a reduced stand-in is drawn exactly where its full image
/// will be. Four flat quadrants at 64 x 48 and the same quadrants at
/// 16 x 12 (the stand-in, told the full size) cover the same screen
/// rectangle with the same colors, at fit and at a 3x zoom with a pan;
/// their natural size is the logical one, so view math sees no change.
#[test]
fn reduced_stand_in_draws_at_full_size() {
    let gpu = gpu_or_skip!();
    let quadrant = |x: u32, y: u32, w: u32, h: u32| match (x * 2 / w, y * 2 / h) {
        (0, 0) => [220, 30, 30, 255],
        (1, 0) => [30, 200, 40, 255],
        (0, _) => [30, 50, 220, 255],
        _ => [230, 230, 40, 255],
    };
    let full = rgba8(64, 48, |x, y| quadrant(x, y, 64, 48));
    let small = rgba8(16, 12, |x, y| quadrant(x, y, 16, 12));
    let background = Color::rgb(0, 0, 0);
    let size = Size::new(64.0, 48.0);
    let mut views = vec![ViewState::default()];
    let mut zoomed = ViewState::default();
    let viewport = Rect::new(0.0, 0.0, 200.0, 150.0);
    zoomed.zoom_to(3.0, size, viewport, Point::new(80.0, 60.0));
    zoomed.pan_by(-11.0, 7.0, size, viewport.size());
    views.push(zoomed);
    for view in views {
        let mut a = offscreen(&gpu, 200, 150, plain(background));
        a.set_content(Content::Still(full.clone())).expect("full");
        let mut b = offscreen(&gpu, 200, 150, plain(background));
        b.set_input(&crate::RenderInput::Reduced {
            content: Content::Still(small.clone()),
            size,
        })
        .expect("stand-in");
        assert_eq!(b.content_size(), Some(size), "logical size");
        let pa = a.placement(&view, viewport).unwrap();
        let pb = b.placement(&view, viewport).unwrap();
        assert_eq!(pa.image_rect, pb.image_rect, "same screen rectangle");
        let fa = frame_in(&mut a, &view, viewport, &DrawList::new());
        let fb = frame_in(&mut b, &view, viewport, &DrawList::new());
        let rect = pa.image_rect;
        let mut checked = 0;
        for y in 0..150 {
            for x in 0..200 {
                let inside = covers(rect, x, y);
                // Away from the quadrant borders and the image edge the
                // two agree (the stand-in is smooth, the full image crisp).
                let ix = (x as f32 + 0.5 - rect.x) / rect.w * 64.0;
                let iy = (y as f32 + 0.5 - rect.y) / rect.h * 48.0;
                let near_edge = (ix - 32.0).abs() < 6.0
                    || (iy - 24.0).abs() < 6.0
                    || !(4.0..60.0).contains(&ix)
                    || !(4.0..44.0).contains(&iy);
                if !inside {
                    assert_rgb(&fb, x, y, [0, 0, 0], 0);
                } else if !near_edge {
                    assert_rgb(&fb, x, y, fa.rgb(x, y), 2);
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "{checked} pixels compared");
    }
}

/// Leaf 1.6: large uploads go through the reusable staging buffers and
/// still land exactly; a stand-in with another aspect ratio is
/// letterboxed inside the logical rectangle instead of stretched.
#[test]
fn pooled_upload_and_letterboxed_stand_in() {
    let gpu = gpu_or_skip!();
    // 1100 x 400 RGBA8 = 1.7 MB per strip: the pooled path.
    let (w, h) = (1100u32, 400u32);
    let color = |x: u32, y: u32| [(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8, 255];
    let image = rgba8(w, h, color);
    let mut r = offscreen(&gpu, w, h, plain(Color::rgb(0, 0, 0)));
    r.set_content(Content::Still(image)).expect("upload");
    let px = frame(&mut r, &one_to_one(), &DrawList::new());
    for (x, y) in [(0, 0), (1099, 399), (517, 211), (256, 255), (1023, 3)] {
        let [cr, cg, cb, _] = color(x, y);
        assert_rgb(&px, x, y, [cr, cg, cb], 1);
    }
    // A square stand-in for a 2:1 image: drawn as a centered square.
    let square = rgba8(10, 10, |_, _| [255, 255, 255, 255]);
    let mut r = offscreen(&gpu, 200, 100, plain(Color::rgb(0, 0, 0)));
    r.set_input(&crate::RenderInput::Reduced {
        content: Content::Still(square),
        size: Size::new(200.0, 100.0),
    })
    .expect("stand-in");
    let px = frame(&mut r, &ViewState::default(), &DrawList::new());
    assert_rgb(&px, 100, 50, [255, 255, 255], 0);
    assert_rgb(&px, 10, 50, [0, 0, 0], 0);
    assert_rgb(&px, 190, 50, [0, 0, 0], 0);
    assert_rgb(&px, 60, 50, [255, 255, 255], 0);
}

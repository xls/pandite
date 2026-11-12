//! G2: zoom is properly filtered.

use super::*;
use crate::provider::{
    ContentRenderer, DrawContext, FrameContext, ProviderContext, RenderInput, RenderProvider,
};
use crate::GpuImage;
use pandite_core::{Content, ContentKind, DisplayMode, Error, ImageBuf, Point, Size};
use std::time::Duration;

fn zoom(z: f32) -> ViewState {
    ViewState {
        mode: DisplayMode::Zoom(z),
        ..ViewState::default()
    }
}

fn checkerboard(w: u32, h: u32) -> ImageBuf {
    rgba8(w, h, |x, y| {
        if (x + y) % 2 == 0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    })
}

/// Pixels whose centers lie at least `inset` pixels inside `rect`.
fn interior(rect: Rect, inset: f32, w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            if cx >= rect.x + inset
                && cy >= rect.y + inset
                && cx <= rect.right() - inset
                && cy <= rect.bottom() - inset
            {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn minified_checkerboard_is_flat_gray() {
    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 220, 220, plain(Color::rgb(255, 0, 0)));
    r.set_content(Content::Still(checkerboard(1024, 1024)))
        .expect("set content");
    // Linear-light mid gray: 0.5 linear is sRGB code 188, not 128.
    let gray = srgb_code(0.5);
    assert_eq!(gray, 188);
    for z in [0.13, 0.21, 0.37, 0.051] {
        let view = zoom(z);
        let px = frame(&mut r, &view, &DrawList::new());
        let rect = r
            .placement(&view, r.full_viewport())
            .expect("placement")
            .image_rect;
        let pixels = interior(rect, 1.0, 220, 220);
        assert!(pixels.len() > 100, "zoom {z}: {} pixels", pixels.len());
        let (mut lo, mut hi) = (255u8, 0u8);
        for &(x, y) in &pixels {
            let [r8, g8, b8] = px.rgb(x, y);
            assert!(
                r8 == g8 && g8 == b8,
                "zoom {z}: ({x}, {y}) not gray: {r8} {g8} {b8}"
            );
            lo = lo.min(r8);
            hi = hi.max(r8);
        }
        // Point sampling or missing mips would alias into a pattern of
        // black, white and moire bands. Filtered output is one flat gray.
        assert!(
            gray.abs_diff(lo) <= 3 && gray.abs_diff(hi) <= 3,
            "zoom {z}: gray range {lo}..{hi}, want {gray}"
        );
    }
}

#[test]
fn minification_is_shift_invariant() {
    let gpu = gpu_or_skip!();
    // Noise minified twice, the second time one pixel further right and
    // down. Every pixel samples the same texture coordinates in both
    // frames, but the 2x2 quads that give the derivatives pair other
    // pixels. Isotropic filtering gives the same picture, shifted.
    // Anisotropic filtering of a square footprint is implementation-
    // defined: Mesa picks a blur axis per quad from rounding noise, which
    // showed here as differences of up to 55 codes.
    const SIDE: u32 = 257;
    let image = rgba8(SIDE, SIDE, |x, y| {
        // An integer hash of the position: white noise.
        let mut h = (y * SIDE + x).wrapping_mul(0x9e37_79b9);
        h ^= h >> 15;
        h = h.wrapping_mul(0x85eb_ca6b);
        h ^= h >> 13;
        let v = (h >> 24) as u8;
        [v, v, v, 255]
    });
    let mut r = offscreen(&gpu, 122, 122, plain(Color::BLACK));
    r.set_content(Content::Still(image)).expect("set content");
    for z in [0.9, 0.6, 0.37, 0.21] {
        let view = zoom(z);
        let first = Rect::new(0.0, 0.0, 121.0, 121.0);
        let second = Rect::new(1.0, 1.0, 121.0, 121.0);
        let a = frame_in(&mut r, &view, first, &DrawList::new());
        let ra = r.placement(&view, first).expect("placement").image_rect;
        let b = frame_in(&mut r, &view, second, &DrawList::new());
        let rb = r.placement(&view, second).expect("placement").image_rect;
        assert_eq!((rb.x - ra.x, rb.y - ra.y), (1.0, 1.0), "zoom {z}");
        let mut worst = 0;
        for (x, y) in interior(ra, 1.0, 121, 121) {
            worst = worst.max(a.rgb(x, y)[0].abs_diff(b.rgb(x + 1, y + 1)[0]));
        }
        assert!(worst <= 2, "zoom {z}: shifted frames differ by {worst}");
    }
}

/// Count pixels in row `y` between `x0` and `x1` that are neither black
/// nor white.
fn soft_pixels(px: &Pixels, y: u32, x0: u32, x1: u32) -> usize {
    (x0..x1)
        .filter(|&x| {
            let v = px.rgb(x, y)[0];
            v > 8 && v < 247
        })
        .count()
}

#[test]
fn magnified_pixels_stay_sharp() {
    let gpu = gpu_or_skip!();
    let image = checkerboard(4, 4);
    let mut r = offscreen(&gpu, 128, 128, plain(Color::rgb(255, 0, 0)));
    r.set_content(Content::Still(image)).expect("set content");
    let threshold = r.settings().crisp_zoom;
    assert!(
        threshold > 1.0 && threshold <= 4.0,
        "documented default {threshold}"
    );

    // Integer zoom past the threshold: every pixel is pure black or white,
    // and texel edges fall exactly on pixel edges.
    let view = zoom(16.0);
    let px = frame(&mut r, &view, &DrawList::new());
    let rect = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    assert_eq!((rect.w, rect.h), (64.0, 64.0));
    let (ox, oy) = (rect.x as u32, rect.y as u32);
    for y in 0..64 {
        for x in 0..64 {
            let texel_white = ((x / 16) + (y / 16)) % 2 == 0;
            let want = if texel_white { [255; 3] } else { [0; 3] };
            assert_rgb(&px, ox + x, oy + y, want, 0);
        }
    }

    // Fractional zoom past the threshold: each texel edge blends across at
    // most one pixel, so a row crossing 3 inner edges has at most 3 soft
    // pixels (rows near a horizontal edge are skipped).
    let view = zoom(9.5);
    let px = frame(&mut r, &view, &DrawList::new());
    let rect = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let (x0, x1) = (rect.x.ceil() as u32 + 1, rect.right().floor() as u32 - 1);
    let mut rows = 0;
    for y in rect.y.ceil() as u32..rect.bottom().floor() as u32 {
        let local = y as f32 + 0.5 - rect.y;
        let to_edge = (1..4)
            .map(|k| (local - k as f32 * 9.5).abs())
            .fold(f32::MAX, f32::min);
        if to_edge < 1.0 {
            continue;
        }
        rows += 1;
        let soft = soft_pixels(&px, y, x0, x1);
        assert!(soft <= 3, "row {y}: {soft} soft pixels at zoom 9.5");
    }
    assert!(rows > 20);

    // Below the threshold the same image is magnified bilinearly: smooth
    // ramps, many soft pixels per row.
    let view = zoom(threshold * 0.8);
    let px = frame(&mut r, &view, &DrawList::new());
    let rect = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let y = (rect.y + rect.h * 0.3) as u32;
    let soft = soft_pixels(
        &px,
        y,
        rect.x.ceil() as u32 + 1,
        rect.right().floor() as u32 - 1,
    );
    assert!(
        soft > 3,
        "bilinear below the threshold, got {soft} soft pixels"
    );
}

#[test]
fn mipmaps_generated_in_linear_light() {
    let gpu = gpu_or_skip!();
    let gpu_image = GpuImage::upload(&gpu, &checkerboard(256, 256)).expect("upload");
    let texture = gpu_image.tile_texture(0);
    assert_eq!(texture.mip_level_count(), 9, "full chain down to 1x1");
    assert_eq!(texture.format(), wgpu::TextureFormat::Rgba8UnormSrgb);
    for level in 1..9 {
        let texels = gpu.read_texture(texture, level).expect("read mip");
        for (i, t) in texels.chunks_exact(4).enumerate() {
            // Linear-light average of black and white is 0.5, sRGB 188.
            // Averaging the encoded values would give 128.
            assert!(
                t[0].abs_diff(188) <= 1 && t[1] == t[0] && t[2] == t[0] && t[3] == 255,
                "level {level} texel {i}: {t:?}"
            );
        }
    }

    // Premultiplied averaging: a transparent green neighbor must not tint
    // the opaque red one, and alpha averages to one half.
    let fringe = rgba8(64, 64, |x, _| {
        if x % 2 == 0 {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 0]
        }
    });
    let gpu_image = GpuImage::upload(&gpu, &fringe).expect("upload");
    let texels = gpu
        .read_texture(gpu_image.tile_texture(0), 1)
        .expect("read mip");
    for t in texels.chunks_exact(4) {
        assert!(
            t[0].abs_diff(188) <= 1 && t[1] == 0 && t[2] == 0 && t[3].abs_diff(128) <= 1,
            "{t:?}"
        );
    }

    // Odd sizes keep the edge texels: a 3x1 image of white, black, black
    // averages to one third, not to the first two texels.
    let odd = rgba8(3, 1, |x, _| {
        if x == 0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    let gpu_image = GpuImage::upload(&gpu, &odd).expect("upload");
    let texels = gpu
        .read_texture(gpu_image.tile_texture(0), 1)
        .expect("read mip");
    assert!(
        texels[0].abs_diff(srgb_code(1.0 / 3.0)) <= 1,
        "{:?}",
        &texels[..4]
    );

    // The rendered result at an exact 1/4 zoom uses those levels: a pattern
    // of one white and three black columns shows 25 % linear gray (137).
    // Gamma-space mips would show 64 to 92.
    let columns = rgba8(512, 64, |x, _| {
        if x % 4 == 0 {
            [255, 255, 255, 255]
        } else {
            [0, 0, 0, 255]
        }
    });
    let mut r = offscreen(&gpu, 160, 40, plain(Color::rgb(255, 0, 0)));
    r.set_content(Content::Still(columns)).expect("set content");
    let view = zoom(0.25);
    let px = frame(&mut r, &view, &DrawList::new());
    let rect = r
        .placement(&view, r.full_viewport())
        .expect("placement")
        .image_rect;
    let quarter = srgb_code(0.25);
    for (x, y) in interior(rect, 2.0, 160, 40) {
        assert_rgb(&px, x, y, [quarter; 3], 3);
    }
}

#[test]
fn one_to_one_maps_texels_exactly() {
    let gpu = gpu_or_skip!();
    // One white texel in black, at 1:1: every pixel shows exactly its own
    // texel. These sizes and positions made Mesa's anisotropic filter blur
    // one axis by 1/12 of each neighbor (rounding noise in the per-quad
    // derivatives raised its probe count from 1 to 2).
    for (w, h) in [
        (3, 3),
        (5, 3),
        (5, 1),
        (7, 7),
        (8, 8),
        (9, 9),
        (16, 16),
        (33, 9),
    ] {
        let (cx, cy) = (w / 2, h / 2);
        let image = rgba8(w, h, |x, y| {
            if (x, y) == (cx, cy) {
                [255, 255, 255, 255]
            } else {
                [0, 0, 0, 255]
            }
        });
        let mut r = offscreen(&gpu, 48, 40, plain(Color::rgb(255, 0, 0)));
        r.set_content(Content::Still(image)).expect("set content");
        let view = ViewState {
            mode: DisplayMode::OneToOne,
            ..ViewState::default()
        };
        let px = frame(&mut r, &view, &DrawList::new());
        let rect = r
            .placement(&view, r.full_viewport())
            .expect("placement")
            .image_rect;
        let (ox, oy) = (rect.x as u32, rect.y as u32);
        for y in 0..h {
            for x in 0..w {
                let want = if (x, y) == (cx, cy) { [255; 3] } else { [0; 3] };
                let got = px.rgb(ox + x, oy + y);
                assert_eq!(got, want, "{w}x{h} at ({ox}, {oy}): texel ({x}, {y})");
            }
        }
    }
}

/// Uploads a black image and overwrites mip level 1 with green and level 2
/// with blue, so the drawn color tells which levels were sampled.
struct MipProbe;

impl RenderProvider for MipProbe {
    fn name(&self) -> &'static str {
        "mip probe"
    }

    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> pandite_core::Result<Box<dyn ContentRenderer>> {
        let RenderInput::Content(content) = input else {
            return Err(Error::Unsupported("test".into()));
        };
        let image = GpuImage::upload(ctx.gpu, content.first_image())?;
        let texture = image.tile_texture(0);
        for (level, color) in [(1, [0u8, 255, 0, 255]), (2, [0, 0, 255, 255])] {
            let (w, h) = (texture.width() >> level, texture.height() >> level);
            let data: Vec<u8> = (0..w * h).flat_map(|_| color).collect();
            // Ordered after the upload's mip passes, which were submitted.
            ctx.gpu.queue().write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: level,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w * 4),
                    rows_per_image: Some(h),
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        Ok(Box::new(ImageRenderer { image }))
    }
}

#[test]
fn level_zero_from_one_to_one_up() {
    let gpu = gpu_or_skip!();
    let mut r = offscreen(&gpu, 80, 80, plain(Color::rgb(255, 0, 0)));
    r.providers_mut()
        .register(ContentKind::Still, std::sync::Arc::new(MipProbe));
    r.set_content(Content::Still(rgba8(64, 64, |_, _| [0, 0, 0, 255])))
        .expect("set content");
    // From 1:1 up only level 0 (black) shows, whatever level of detail the
    // driver would estimate from derivatives.
    for z in [1.0, 1.5, 2.0, 2.9] {
        let view = zoom(z);
        let px = frame(&mut r, &view, &DrawList::new());
        let rect = r
            .placement(&view, r.full_viewport())
            .expect("placement")
            .image_rect;
        for (x, y) in interior(rect, 1.0, 80, 80) {
            assert_eq!(px.rgb(x, y), [0, 0, 0], "zoom {z}: ({x}, {y})");
        }
    }
    // Minified by half, the mip chain is used: level 1 (green) dominates.
    let view = zoom(0.5);
    let px = frame(&mut r, &view, &DrawList::new());
    let [red, green, _] = px.rgb(40, 40);
    assert!(red == 0 && green > 128, "zoom 0.5: {:?}", px.rgb(40, 40));
}

/// Uploads stills with an artificially small texture limit, so ordinary
/// test images take the tiled path.
struct TiledStill {
    limit: u32,
    /// Keep only the downscaled overview (the out-of-memory fallback).
    reduced: bool,
}

/// Draws one image built by a test provider.
struct ImageRenderer {
    image: GpuImage,
}

impl RenderProvider for TiledStill {
    fn name(&self) -> &'static str {
        "tiled still"
    }

    fn create(
        &self,
        ctx: &ProviderContext<'_>,
        input: &RenderInput,
    ) -> pandite_core::Result<Box<dyn ContentRenderer>> {
        let RenderInput::Content(content) = input else {
            return Err(Error::Unsupported("test".into()));
        };
        let image =
            GpuImage::upload_mode(ctx.gpu, content.first_image(), self.limit, self.reduced)?;
        assert_eq!(image.is_reduced(), self.reduced);
        assert!(
            self.reduced || image.tile_count() > 1,
            "the test image must be tiled"
        );
        Ok(Box::new(ImageRenderer { image }))
    }
}

impl ContentRenderer for ImageRenderer {
    fn size(&self) -> Size {
        self.image.size()
    }
    fn update(&mut self, _dt: Duration) -> Option<Duration> {
        None
    }
    fn prepare(&mut self, ctx: &mut FrameContext<'_>) -> pandite_core::Result<()> {
        self.image.prepare(ctx);
        Ok(())
    }
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, ctx: &DrawContext<'_>) {
        self.image.draw(pass, ctx);
    }
}

#[test]
fn oversized_image_still_renders() {
    let gpu = gpu_or_skip!();

    // 1. An image wider than this device's real texture limit.
    let max = gpu.max_texture_size();
    let width = max + 1000;
    if width <= pandite_core::image::MAX_DIMENSION {
        let wide = rgba8(width, 200, |x, _| {
            if x + 40 >= width && x + 20 < width {
                [0, 255, 0, 255]
            } else if x < width / 2 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            }
        });
        let size = Size::new(width as f32, 200.0);
        let mut r = offscreen(&gpu, 400, 40, plain(Color::BLACK));
        r.set_content(Content::Still(wide))
            .expect("oversized image uploads");
        // Fit: drawn from the downscaled overview.
        let view = ViewState {
            enlarge_to_fit: true,
            ..ViewState::default()
        };
        let px = frame(&mut r, &view, &DrawList::new());
        let rect = r
            .placement(&view, r.full_viewport())
            .expect("placement")
            .image_rect;
        let mid = (rect.y + rect.h / 2.0) as u32;
        assert_rgb(&px, 100, mid, [255, 0, 0], 2);
        assert_rgb(&px, 300, mid, [0, 0, 255], 2);
        // 1:1 at the right end: full resolution from the last tile.
        let mut view = ViewState {
            mode: DisplayMode::OneToOne,
            ..ViewState::default()
        };
        view.pan_by(-1.0e9, 0.0, size, r.full_viewport().size());
        let px = frame(&mut r, &view, &DrawList::new());
        let rect = r
            .placement(&view, r.full_viewport())
            .expect("placement")
            .image_rect;
        assert_eq!(rect.right(), 400.0);
        let y = 20;
        assert_rgb(&px, 359, y, [0, 0, 255], 0);
        assert_rgb(&px, 360, y, [0, 255, 0], 0);
        assert_rgb(&px, 379, y, [0, 255, 0], 0);
        assert_rgb(&px, 380, y, [0, 0, 255], 0);
    }

    // 2. Tiled rendering matches untiled rendering at every zoom: no seams
    // at tile edges, the overview when zoomed far out.
    let gradient = rgba8(700, 300, |x, y| {
        [
            (x * 255 / 699) as u8,
            (y * 255 / 299) as u8,
            ((x + 2 * y) * 255 / 1297) as u8,
            255,
        ]
    });
    let size = Size::new(700.0, 300.0);
    let mut plain_r = offscreen(&gpu, 240, 160, plain(Color::BLACK));
    plain_r
        .set_content(Content::Still(gradient.clone()))
        .expect("untiled");
    let mut tiled_r = offscreen(&gpu, 240, 160, plain(Color::BLACK));
    tiled_r.providers_mut().register(
        ContentKind::Still,
        std::sync::Arc::new(TiledStill {
            limit: 256,
            reduced: false,
        }),
    );
    tiled_r
        .set_content(Content::Still(gradient.clone()))
        .expect("tiled");
    // 3. The out-of-memory fallback keeps only the overview and still
    // displays the whole image at every zoom, softer when zoomed in.
    let mut reduced_r = offscreen(&gpu, 240, 160, plain(Color::BLACK));
    reduced_r.providers_mut().register(
        ContentKind::Still,
        std::sync::Arc::new(TiledStill {
            limit: 256,
            reduced: true,
        }),
    );
    reduced_r
        .set_content(Content::Still(gradient))
        .expect("reduced");
    let full = plain_r.full_viewport();
    let worst = |a: &Pixels, b: &Pixels| {
        let mut worst = 0u8;
        for y in 0..160 {
            for x in 0..240 {
                let (pa, pb) = (a.rgb(x, y), b.rgb(x, y));
                for c in 0..3 {
                    worst = worst.max(pa[c].abs_diff(pb[c]));
                }
            }
        }
        worst
    };
    for (z, anchor) in [
        (1.0, Point::new(120.0, 80.0)),
        (1.0, Point::new(10.0, 150.0)),
        (2.0, Point::new(120.0, 80.0)),
        (0.3, Point::new(120.0, 80.0)),
        (0.2, Point::new(120.0, 80.0)),
    ] {
        let mut view = ViewState::default();
        view.zoom_to(z, size, full, anchor);
        // Put a tile edge (x = 128 or 256) in view at 1:1 and 2:1.
        if z >= 1.0 {
            view.pan_by(300.0, 0.0, size, full.size());
        }
        let a = frame(&mut plain_r, &view, &DrawList::new());
        let b = frame(&mut tiled_r, &view, &DrawList::new());
        let c = frame(&mut reduced_r, &view, &DrawList::new());
        let tiled = worst(&a, &b);
        let reduced = worst(&a, &c);
        let tolerance = if z >= 1.0 { 1 } else { 4 };
        assert!(
            tiled <= tolerance,
            "zoom {z}: tiled differs from untiled by {tiled}"
        );
        assert!(
            reduced <= 4,
            "zoom {z}: reduced differs from untiled by {reduced}"
        );
    }
}

//! The pandite application icon, drawn procedurally.
//!
//! The mark is an octahedral crystal (pandite is a mineral) in four shades
//! of teal on a dark slate tile with rounded corners. Every edge is
//! antialiased from a signed distance, so the icon stays crisp at 16 pixels
//! and at 1024. The same code produces the window icon at run time, the
//! committed `assets/icon-256.png` and `assets/pandite.ico`, and the store
//! logos (through `examples/make_icon.rs`).

use pandite_core::{EncodeOptions, EncoderProvider, Error, ImageBuf, PixelFormat, Result};

/// Sizes stored in the Windows `.ico` file.
pub const ICO_SIZES: [u32; 4] = [16, 32, 48, 256];

/// Largest icon side [`render_icon`] accepts.
pub const MAX_ICON_SIZE: u32 = 4096;

type Vec2 = (f32, f32);

/// Crystal vertices in unit coordinates (y grows down): top, right, bottom,
/// left and the front vertex where the four visible facets meet.
const TOP: Vec2 = (0.50, 0.15);
const RIGHT: Vec2 = (0.81, 0.47);
const BOTTOM: Vec2 = (0.50, 0.86);
const LEFT: Vec2 = (0.19, 0.47);
const FRONT: Vec2 = (0.43, 0.56);

/// Facet colors, lit from the top left.
const FACETS: [([Vec2; 3], [u8; 3]); 4] = [
    ([TOP, LEFT, FRONT], [166, 244, 230]),
    ([TOP, FRONT, RIGHT], [94, 208, 196]),
    ([LEFT, BOTTOM, FRONT], [59, 167, 166]),
    ([FRONT, BOTTOM, RIGHT], [34, 112, 127]),
];

/// Tile gradient, top and bottom.
const TILE_TOP: [f32; 3] = [52.0, 70.0, 107.0];
const TILE_BOTTOM: [f32; 3] = [22.0, 29.0, 46.0];

/// Straight-alpha RGBA accumulator.
struct Canvas {
    size: u32,
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(size: u32) -> Self {
        Self {
            size,
            px: vec![[0.0; 4]; (size * size) as usize],
        }
    }

    /// Composite `color` with coverage `a` over pixel `(x, y)`.
    fn over(&mut self, x: u32, y: u32, color: [f32; 3], a: f32) {
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[(y * self.size + x) as usize];
        let out_a = a + p[3] * (1.0 - a);
        if out_a <= 0.0 {
            return;
        }
        for c in 0..3 {
            p[c] = (color[c] * a + p[c] * p[3] * (1.0 - a)) / out_a;
        }
        p[3] = out_a;
    }

    fn into_image(self) -> Result<ImageBuf> {
        let mut data = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            for c in &p[..3] {
                data.push(c.round().clamp(0.0, 255.0) as u8);
            }
            data.push((p[3] * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        ImageBuf::new(self.size, self.size, PixelFormat::Rgba8, data)
    }
}

/// Signed distance from `p` to a rounded box (negative inside).
fn rounded_box(p: Vec2, center: Vec2, half: Vec2, radius: f32) -> f32 {
    let qx = (p.0 - center.0).abs() - (half.0 - radius);
    let qy = (p.1 - center.1).abs() - (half.1 - radius);
    (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - radius
}

/// Signed distance from `p` to a convex polygon given in clockwise or
/// counterclockwise order (negative inside). Exact inside and along edges,
/// slightly short outside corners, which is all antialiasing needs.
fn convex(p: Vec2, poly: &[Vec2]) -> f32 {
    // Orientation decides which side of each edge is inside.
    let mut area = 0.0;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        area += a.0 * b.1 - b.0 * a.1;
    }
    let sign = if area >= 0.0 { 1.0 } else { -1.0 };
    let mut d = f32::NEG_INFINITY;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (ex, ey) = (b.0 - a.0, b.1 - a.1);
        let len = (ex * ex + ey * ey).sqrt().max(f32::EPSILON);
        // Outward normal for a positively oriented polygon is (ey, -ex).
        let dist = sign * ((p.0 - a.0) * ey - (p.1 - a.1) * ex) / len;
        d = d.max(dist);
    }
    d
}

/// Draw the icon at `size` x `size` pixels: straight-alpha RGBA8 with a
/// transparent background outside the rounded tile.
pub fn render_icon(size: u32) -> Result<ImageBuf> {
    if size == 0 || size > MAX_ICON_SIZE {
        return Err(Error::Invalid(format!(
            "icon size {size} is outside 1 to {MAX_ICON_SIZE}"
        )));
    }
    let s = size as f32;
    let mut canvas = Canvas::new(size);
    // Small icons get a slightly larger tile so the mark keeps its weight.
    let margin = if size <= 32 { 0.0 } else { 0.03 };
    let half = 0.5 - margin;
    let radius = 0.22 * (1.0 - 2.0 * margin);
    let silhouette = [TOP, RIGHT, BOTTOM, LEFT];
    for y in 0..size {
        for x in 0..size {
            // Pixel center in unit coordinates; distances convert to pixels.
            let p = ((x as f32 + 0.5) / s, (y as f32 + 0.5) / s);
            let tile = rounded_box(p, (0.5, 0.5), (half, half), radius) * s;
            let tile_a = (0.5 - tile).clamp(0.0, 1.0);
            if tile_a <= 0.0 {
                continue;
            }
            let t = p.1.clamp(0.0, 1.0);
            let shade = [
                TILE_TOP[0] + (TILE_BOTTOM[0] - TILE_TOP[0]) * t,
                TILE_TOP[1] + (TILE_BOTTOM[1] - TILE_TOP[1]) * t,
                TILE_TOP[2] + (TILE_BOTTOM[2] - TILE_TOP[2]) * t,
            ];
            canvas.over(x, y, shade, tile_a);
            // A base silhouette first, so facet seams blend with crystal
            // color instead of the tile.
            let base = (0.5 - convex(p, &silhouette) * s).clamp(0.0, 1.0);
            canvas.over(x, y, [70.0, 170.0, 168.0], base * tile_a);
            for (tri, color) in &FACETS {
                let a = (0.5 - convex(p, tri) * s).clamp(0.0, 1.0);
                let c = [
                    f32::from(color[0]),
                    f32::from(color[1]),
                    f32::from(color[2]),
                ];
                canvas.over(x, y, c, a * tile_a);
            }
        }
    }
    canvas.into_image()
}

/// Encode an icon image as PNG.
pub fn encode_png(image: &ImageBuf) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    pandite_codecs::PngEncoder.encode(image, &EncodeOptions::default(), &mut out)?;
    Ok(out)
}

/// Build a Windows `.ico` file from PNG-compressed images, one per side
/// length. Sides above 256 are rejected (the format cannot express them).
pub fn build_ico(images: &[(u32, Vec<u8>)]) -> Result<Vec<u8>> {
    if images.is_empty() || images.len() > usize::from(u16::MAX) {
        return Err(Error::Invalid(
            "an icon file needs 1 to 65535 images".into(),
        ));
    }
    let count = images.len() as u16;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * images.len();
    for (side, png) in images {
        if *side == 0 || *side > 256 {
            return Err(Error::Invalid(format!("icon side {side} is not 1 to 256")));
        }
        // 256 is written as 0 in the one-byte size fields.
        let byte = if *side == 256 { 0 } else { *side as u8 };
        out.extend_from_slice(&[byte, byte, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        let len = u32::try_from(png.len()).map_err(|_| Error::Invalid("icon too big".into()))?;
        out.extend_from_slice(&len.to_le_bytes());
        let at = u32::try_from(offset).map_err(|_| Error::Invalid("icon too big".into()))?;
        out.extend_from_slice(&at.to_le_bytes());
        offset += png.len();
    }
    for (_, png) in images {
        out.extend_from_slice(png);
    }
    Ok(out)
}

/// The window icon as raw RGBA8 rows plus its side, for the window system.
pub fn window_icon_rgba(size: u32) -> Option<(Vec<u8>, u32)> {
    let image = render_icon(size).ok()?;
    Some((image.data().to_vec(), size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::{DecodeOptions, DecoderProvider, Source};

    fn pixel(img: &ImageBuf, x: u32, y: u32) -> [u8; 4] {
        let i = (y * img.width() + x) as usize * 4;
        let d = img.data();
        [d[i], d[i + 1], d[i + 2], d[i + 3]]
    }

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_crystal() {
        for size in [16, 48, 256] {
            let img = render_icon(size).unwrap();
            assert_eq!((img.width(), img.height()), (size, size));
            assert_eq!(pixel(&img, 0, 0)[3], 0, "corner is transparent at {size}");
            let c = pixel(&img, size / 2, size / 2);
            assert_eq!(c[3], 255, "center is opaque at {size}");
            assert!(c[1] > 100, "center shows the teal crystal at {size}: {c:?}");
            // Antialiased: some edge pixel is partly covered.
            assert!(img.data().chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
        }
        assert!(render_icon(0).is_err());
    }

    #[test]
    fn ico_lists_every_size_and_png_decodes() {
        let images: Vec<(u32, Vec<u8>)> = ICO_SIZES
            .iter()
            .map(|&s| (s, encode_png(&render_icon(s).unwrap()).unwrap()))
            .collect();
        let ico = build_ico(&images).unwrap();
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 4, 0]);
        // The 256 entry stores 0 as its size byte.
        assert_eq!(ico[6 + 3 * 16], 0);
        let png = &images[3].1;
        let source = Source::from_bytes(png.clone(), None);
        let decoded = pandite_codecs::PngDecoder
            .decode(&source, &DecodeOptions::default())
            .unwrap();
        assert_eq!(decoded.dimensions(), (256, 256));
    }
}

//! The pandite application icon, drawn procedurally.
//!
//! The mark is an octahedral crystal (pandite is a mineral) in four shades
//! of teal on a dark slate tile with rounded corners. Every edge is
//! antialiased from a signed distance, so the icon stays crisp at 16 pixels
//! and at 1024. The same code produces the window icon at run time, the
//! committed `assets/icon-256.png` and `assets/pandite.ico`, and the store
//! logos (through `examples/make_icon.rs`).

use pandite_core::{EncodeOptions, EncoderProvider, Error, ImageBuf, PixelFormat, Result};

/// Sizes stored in the Windows `.ico` file. Sizes below 256 are stored as
/// 32-bit bitmaps and 256 as PNG, the layout Windows components expect.
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Bytes of the BITMAPINFOHEADER that starts an icon bitmap.
pub const ICO_BITMAP_HEADER: usize = 40;

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

/// Bytes in one row of the 1-bit AND mask of a `width` pixel wide icon
/// bitmap: rows are padded to 32 bits.
pub fn ico_mask_stride(width: u32) -> usize {
    (width as usize).div_ceil(32) * 4
}

/// Encode a square RGBA8 icon image as a classic Windows icon bitmap: a
/// BITMAPINFOHEADER with twice the height, 32-bit BGRA rows bottom-up,
/// then the 1-bit AND mask rows bottom-up (a set bit marks a fully
/// transparent pixel).
pub fn encode_ico_bitmap(image: &ImageBuf) -> Result<Vec<u8>> {
    let (w, h) = (image.width(), image.height());
    if image.format() != PixelFormat::Rgba8 || w > 256 || h > 256 {
        return Err(Error::Invalid(format!(
            "an icon bitmap needs RGBA8 pixels up to 256 x 256, not {w} x {h} {:?}",
            image.format()
        )));
    }
    let color = w as usize * h as usize * 4;
    let stride = ico_mask_stride(w);
    let mask = stride * h as usize;
    let mut out = Vec::with_capacity(ICO_BITMAP_HEADER + color + mask);
    out.extend_from_slice(&(ICO_BITMAP_HEADER as u32).to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    // The height covers the color rows and the mask rows.
    out.extend_from_slice(&(2 * h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, no compression
    out.extend_from_slice(&((color + mask) as u32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes()); // pixels per meter
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // colors used
    out.extend_from_slice(&0u32.to_le_bytes()); // colors important
    for y in (0..h).rev() {
        for p in image.row(y).chunks_exact(4) {
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    for y in (0..h).rev() {
        let mut row = vec![0u8; stride];
        for (x, p) in image.row(y).chunks_exact(4).enumerate() {
            if p[3] == 0 {
                row[x / 8] |= 0x80 >> (x % 8);
            }
        }
        out.extend_from_slice(&row);
    }
    Ok(out)
}

/// Build a Windows `.ico` file from square RGBA8 images, one per side
/// length from 1 to 256. The 256 pixel image is stored PNG-compressed and
/// every smaller one as a 32-bit bitmap ([`encode_ico_bitmap`]).
pub fn build_ico(images: &[ImageBuf]) -> Result<Vec<u8>> {
    if images.is_empty() || images.len() > usize::from(u16::MAX) {
        return Err(Error::Invalid(
            "an icon file needs 1 to 65535 images".into(),
        ));
    }
    let count = images.len() as u16;
    let mut entries = Vec::with_capacity(images.len());
    for image in images {
        let side = image.width();
        if side != image.height() || side == 0 || side > 256 {
            return Err(Error::Invalid(format!(
                "icon image {side} x {} is not a square of 1 to 256",
                image.height()
            )));
        }
        let data = if side == 256 {
            encode_png(image)?
        } else {
            encode_ico_bitmap(image)?
        };
        entries.push((side, data));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * entries.len();
    for (side, data) in &entries {
        // 256 is written as 0 in the one-byte size fields.
        let byte = if *side == 256 { 0 } else { *side as u8 };
        out.extend_from_slice(&[byte, byte, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        let len = u32::try_from(data.len()).map_err(|_| Error::Invalid("icon too big".into()))?;
        out.extend_from_slice(&len.to_le_bytes());
        let at = u32::try_from(offset).map_err(|_| Error::Invalid("icon too big".into()))?;
        out.extend_from_slice(&at.to_le_bytes());
        offset += data.len();
    }
    for (_, data) in &entries {
        out.extend_from_slice(data);
    }
    Ok(out)
}

/// The application `.ico` file: every size of [`ICO_SIZES`] drawn by
/// [`render_icon`].
pub fn program_ico() -> Result<Vec<u8>> {
    let images = ICO_SIZES
        .iter()
        .map(|&s| render_icon(s))
        .collect::<Result<Vec<_>>>()?;
    build_ico(&images)
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

    fn u16_at(b: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([b[at], b[at + 1]])
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
    }

    /// One directory entry of an `.ico` file and its image data.
    struct Entry<'a> {
        width: u32,
        height: u32,
        colors: u8,
        planes: u16,
        bits: u16,
        data: &'a [u8],
    }

    fn entries(ico: &[u8]) -> Vec<Entry<'_>> {
        assert_eq!(u16_at(ico, 0), 0, "reserved");
        assert_eq!(u16_at(ico, 2), 1, "type icon");
        let count = usize::from(u16_at(ico, 4));
        let mut end = 6 + 16 * count;
        let out: Vec<Entry<'_>> = (0..count)
            .map(|i| {
                let e = 6 + 16 * i;
                let side = |b: u8| if b == 0 { 256 } else { u32::from(b) };
                let len = u32_at(ico, e + 8) as usize;
                let offset = u32_at(ico, e + 12) as usize;
                assert_eq!(offset, end, "entry {i} follows the one before");
                end = offset + len;
                assert_eq!(ico[e + 3], 0, "reserved byte");
                Entry {
                    width: side(ico[e]),
                    height: side(ico[e + 1]),
                    colors: ico[e + 2],
                    planes: u16_at(ico, e + 4),
                    bits: u16_at(ico, e + 6),
                    data: &ico[offset..offset + len],
                }
            })
            .collect();
        assert_eq!(end, ico.len(), "no bytes after the last image");
        out
    }

    const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn decode_png(data: &[u8]) -> ImageBuf {
        let source = Source::from_vec(data.to_vec(), None);
        pandite_codecs::PngDecoder
            .decode(&source, &DecodeOptions::default())
            .unwrap()
            .first_image()
            .clone()
    }

    #[test]
    fn ico_stores_small_sizes_as_bitmaps_and_256_as_png() {
        let ico = program_ico().unwrap();
        let list = entries(&ico);
        assert_eq!(
            list.iter().map(|e| e.width).collect::<Vec<_>>(),
            ICO_SIZES.to_vec()
        );
        for (entry, &side) in list.iter().zip(&ICO_SIZES) {
            let drawn = render_icon(side).unwrap();
            if side == 256 {
                assert_eq!(decode_png(entry.data).data(), drawn.data());
                continue;
            }
            // The bitmap holds the drawn pixels, bottom row first, as BGRA,
            // and the mask marks exactly the fully transparent pixels.
            let s = side as usize;
            let colors = &entry.data[ICO_BITMAP_HEADER..ICO_BITMAP_HEADER + s * s * 4];
            let mask = &entry.data[ICO_BITMAP_HEADER + s * s * 4..];
            let stride = ico_mask_stride(side);
            for y in 0..s {
                let row = drawn.row((s - 1 - y) as u32);
                for x in 0..s {
                    let src = &row[x * 4..x * 4 + 4];
                    let got = &colors[(y * s + x) * 4..(y * s + x) * 4 + 4];
                    assert_eq!(
                        got,
                        &[src[2], src[1], src[0], src[3]],
                        "{side} px at {x},{y}"
                    );
                    let bit = mask[y * stride + x / 8] & (0x80 >> (x % 8)) != 0;
                    assert_eq!(bit, src[3] == 0, "{side} px mask at {x},{y}");
                }
            }
        }
        // A non-square or oversized image is refused.
        assert!(build_ico(&[render_icon(300).unwrap()]).is_err());
        let wide = ImageBuf::solid_rgba8(4, 2, [0; 4]).unwrap();
        assert!(build_ico(&[wide]).is_err());
    }

    /// The committed icon file holds 32-bit bitmaps for every size below
    /// 256 and PNG only for 256, with consistent headers and lengths.
    #[test]
    fn program_icon_small_sizes_are_bitmaps() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pandite.ico");
        let ico = std::fs::read(&path).unwrap();
        let list = entries(&ico);
        for side in [16, 32, 48, 256] {
            assert!(list.iter().any(|e| e.width == side), "{side} px present");
        }
        for e in &list {
            assert_eq!(e.width, e.height, "square entries");
            assert_eq!((e.colors, e.planes, e.bits), (0, 1, 32), "{} px", e.width);
            if e.width == 256 {
                assert!(e.data.starts_with(PNG_SIGNATURE), "256 px is PNG");
                let png = decode_png(e.data);
                assert_eq!((png.width(), png.height()), (256, 256));
                continue;
            }
            assert!(
                !e.data.starts_with(PNG_SIGNATURE),
                "{} px must be a bitmap",
                e.width
            );
            let d = e.data;
            assert_eq!(u32_at(d, 0) as usize, ICO_BITMAP_HEADER, "header size");
            assert_eq!(u32_at(d, 4), e.width, "bitmap width");
            assert_eq!(u32_at(d, 8), 2 * e.height, "height covers color and mask");
            assert_eq!(u16_at(d, 12), 1, "planes");
            assert_eq!(u16_at(d, 14), 32, "bits per pixel");
            assert_eq!(u32_at(d, 16), 0, "no compression");
            let color = (e.width * e.height * 4) as usize;
            let mask = ico_mask_stride(e.width) * e.height as usize;
            assert_eq!(u32_at(d, 20) as usize, color + mask, "image size field");
            assert_eq!(d.len(), ICO_BITMAP_HEADER + color + mask, "total length");
        }
        // The committed file is what the generator writes today.
        assert!(
            ico == program_ico().unwrap(),
            "assets/pandite.ico is stale: run the make_icon example"
        );
    }
}

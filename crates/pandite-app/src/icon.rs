//! The pandite application icon.
//!
//! The icon is artwork: a color aperture around the letter P on a dark
//! rounded tile. Its master is `assets/icon-1024.png` (straight-alpha RGBA,
//! transparent outside the tile). `examples/make_icon.rs` scales the master
//! to every size that the program and the packages need and writes the
//! committed files: `assets/pandite.ico`, `assets/icon-256.png` and the raw
//! window icons `assets/icon-32.rgba` and `assets/icon-64.rgba`. The
//! program embeds only the `.ico` (Windows) and the two raw window icons,
//! so it decodes and scales nothing at start.

use pandite_core::{EncodeOptions, EncoderProvider, Error, ImageBuf, PixelFormat, Result};

/// Sizes stored in the Windows `.ico` file. Sizes below 256 are stored as
/// 32-bit bitmaps and 256 as PNG, the layout Windows components expect.
pub const ICO_SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];

/// Bytes of the BITMAPINFOHEADER that starts an icon bitmap.
pub const ICO_BITMAP_HEADER: usize = 40;

/// Largest icon side [`scale_icon`] accepts.
pub const MAX_ICON_SIZE: u32 = 4096;

/// The window icons, raw straight-alpha RGBA8 rows.
const WINDOW_ICONS: [(u32, &[u8]); 2] = [
    (32, include_bytes!("../assets/icon-32.rgba")),
    (64, include_bytes!("../assets/icon-64.rgba")),
];

/// Scale the square icon `master` to `size` x `size` pixels: straight-alpha
/// RGBA8, filtered with Lanczos in premultiplied linear light, so the
/// transparent corners leave no dark fringe.
pub fn scale_icon(master: &ImageBuf, size: u32) -> Result<ImageBuf> {
    if size == 0 || size > MAX_ICON_SIZE {
        return Err(Error::Invalid(format!(
            "icon size {size} is outside 1 to {MAX_ICON_SIZE}"
        )));
    }
    if master.width() != master.height() {
        return Err(Error::Invalid(format!(
            "the icon master is {} x {}, not square",
            master.width(),
            master.height()
        )));
    }
    let master = pandite_ops::to_rgba8(master)?;
    let scaled = pandite_ops::resize(&master, size, size, pandite_ops::Filter::Lanczos3)?;
    if scaled.format() != PixelFormat::Rgba8 {
        return Err(Error::Invalid("the scaled icon is not RGBA8".into()));
    }
    Ok(scaled)
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

/// The application `.ico` file: `master` scaled to every size of
/// [`ICO_SIZES`].
pub fn program_ico(master: &ImageBuf) -> Result<Vec<u8>> {
    let images = ICO_SIZES
        .iter()
        .map(|&s| scale_icon(master, s))
        .collect::<Result<Vec<_>>>()?;
    build_ico(&images)
}

/// The window icon as raw RGBA8 rows plus its side, for the window system:
/// the smallest embedded icon that is at least `size` pixels, else the
/// largest.
pub fn window_icon_rgba(size: u32) -> Option<(Vec<u8>, u32)> {
    let (side, data) = WINDOW_ICONS
        .iter()
        .find(|(side, _)| *side >= size)
        .or(WINDOW_ICONS.last())?;
    (data.len() == (*side as usize).pow(2) * 4).then(|| (data.to_vec(), *side))
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

    fn asset(name: &str) -> Vec<u8> {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets")
            .join(name);
        std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
    }

    fn master() -> ImageBuf {
        decode_png(&asset("icon-1024.png"))
    }

    #[test]
    fn icon_has_transparent_corners_and_an_opaque_center() {
        let master = master();
        assert_eq!((master.width(), master.height()), (1024, 1024));
        for size in [16, 48, 256] {
            let img = scale_icon(&master, size).unwrap();
            assert_eq!((img.width(), img.height()), (size, size));
            assert_eq!(pixel(&img, 0, 0)[3], 0, "corner is transparent at {size}");
            assert_eq!(pixel(&img, size / 2, size / 2)[3], 255, "center at {size}");
            // The tile reaches the edges of the canvas, and its rim is soft.
            assert_eq!(pixel(&img, size / 2, 1)[3], 255, "top edge at {size}");
            assert!(img.data().chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255));
            // The aperture is colorful: some pixel is strongly saturated.
            assert!(img.data().chunks_exact(4).any(|p| p[3] == 255
                && p[..3].iter().max().unwrap() - p[..3].iter().min().unwrap() > 150));
        }
        assert!(scale_icon(&master, 0).is_err());
        let wide = ImageBuf::solid_rgba8(4, 2, [0; 4]).unwrap();
        assert!(scale_icon(&wide, 16).is_err());
    }

    /// The committed files are what the generator writes from the master
    /// today, and the program finds its window icons in them.
    #[test]
    fn committed_icon_files_match_the_master() {
        let master = master();
        for side in [32u32, 64] {
            let raw = asset(&format!("icon-{side}.rgba"));
            assert_eq!(
                raw,
                scale_icon(&master, side).unwrap().data(),
                "icon-{side}.rgba is stale"
            );
        }
        assert_eq!(
            decode_png(&asset("icon-256.png")).data(),
            scale_icon(&master, 256).unwrap().data(),
            "icon-256.png is stale"
        );
        assert_eq!(window_icon_rgba(16).unwrap().1, 32);
        assert_eq!(window_icon_rgba(32).unwrap().1, 32);
        assert_eq!(window_icon_rgba(33).unwrap().1, 64);
        assert_eq!(window_icon_rgba(64).unwrap().1, 64);
        assert_eq!(window_icon_rgba(512).unwrap().1, 64);
        let (data, side) = window_icon_rgba(64).unwrap();
        assert_eq!(data.len(), (side * side * 4) as usize);
        assert_eq!(data, asset("icon-64.rgba"));
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
        let master = master();
        let ico = program_ico(&master).unwrap();
        let list = entries(&ico);
        assert_eq!(
            list.iter().map(|e| e.width).collect::<Vec<_>>(),
            ICO_SIZES.to_vec()
        );
        for (entry, &side) in list.iter().zip(&ICO_SIZES) {
            let drawn = scale_icon(&master, side).unwrap();
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
        assert!(build_ico(&[scale_icon(&master, 300).unwrap()]).is_err());
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
            ico == program_ico(&master()).unwrap(),
            "assets/pandite.ico is stale: run the make_icon example"
        );
    }
}

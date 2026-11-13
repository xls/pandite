//! The platform-neutral part of recognition: the size an image is read
//! at, the 8-bit gray copy an engine reads, and the way back from the
//! engine's word boxes to full-image pixels.

use pandite_core::{CancelToken, Error, ImageBuf, PixelFormat, Point, Quad, Rect, Result};
use std::sync::OnceLock;

/// Source rows between two cancellation checks.
const CANCEL_ROWS: usize = 64;

/// The size to recognize a `width` x `height` image at: the image itself
/// when its long side is at most `limit`, otherwise scaled down to that
/// long side with the aspect ratio kept. Sides are at least 1. A `limit`
/// of 0 counts as 1.
pub fn recognition_size(width: u32, height: u32, limit: u32) -> (u32, u32) {
    let limit = limit.max(1);
    let (width, height) = (width.max(1), height.max(1));
    let long = width.max(height);
    if long <= limit {
        return (width, height);
    }
    let s = f64::from(limit) / f64::from(long);
    let side = |v: u32| ((f64::from(v) * s).round() as u32).clamp(1, limit);
    (side(width), side(height))
}

/// An 8-bit grayscale image: rows top first, no padding, 0 is black.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GrayImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// One byte per pixel.
    pub pixels: Vec<u8>,
}

/// `image` as 8-bit luminance at `width` x `height` (at most the image's
/// own size), scaled down by averaging the source pixels each target pixel
/// covers. Transparent pixels are laid over white, so dark text on a
/// transparent background stays readable. High dynamic range pixels are
/// clipped at display white. Checks `cancel` every few rows.
pub fn gray(image: &ImageBuf, width: u32, height: u32, cancel: &CancelToken) -> Result<GrayImage> {
    let (sw, sh) = (image.width(), image.height());
    if width == 0 || height == 0 || width > sw || height > sh {
        return Err(Error::Invalid(format!(
            "gray copy of {width} x {height} for a {sw} x {sh} image"
        )));
    }
    let pixels = match image.format() {
        PixelFormat::Rgba8 => reduce(image, width, height, cancel, luma8)?,
        PixelFormat::Rgba16 => reduce(image, width, height, cancel, luma16)?,
        PixelFormat::RgbaF32 => {
            let lut = srgb_lut();
            reduce(image, width, height, cancel, |p| luma_f32(p, lut))?
        }
        other => {
            return Err(Error::Unsupported(format!(
                "text recognition of {other:?} pixels"
            )))
        }
    };
    Ok(GrayImage {
        width,
        height,
        pixels,
    })
}

/// Area-average `image` down to `width` x `height` gray pixels. Every
/// target pixel averages a whole block of source pixels; block sides
/// differ by at most one pixel across the image.
fn reduce(
    image: &ImageBuf,
    width: u32,
    height: u32,
    cancel: &CancelToken,
    luma: impl Fn(&[u8]) -> u32,
) -> Result<Vec<u8>> {
    let (sw, sh) = (image.width() as usize, image.height() as usize);
    let (dw, dh) = (width as usize, height as usize);
    let bpp = image.format().bytes_per_pixel();
    let mut out = vec![0u8; dw * dh];
    if (dw, dh) == (sw, sh) {
        for y in 0..sh {
            if y % CANCEL_ROWS == 0 {
                cancel.check()?;
            }
            let row = image.row(y as u32);
            for (o, p) in out[y * dw..(y + 1) * dw]
                .iter_mut()
                .zip(row.chunks_exact(bpp))
            {
                *o = luma(p) as u8;
            }
        }
        return Ok(out);
    }
    // Target column of every source column, and how many source columns
    // each target column averages.
    let column: Vec<u32> = (0..sw).map(|x| (x * dw / sw) as u32).collect();
    let mut span = vec![0u64; dw];
    for &c in &column {
        span[c as usize] += 1;
    }
    let mut acc = vec![0u64; dw];
    let mut sy = 0usize;
    for dy in 0..dh {
        let end = ((dy + 1) * sh / dh).max(sy + 1).min(sh);
        for y in sy..end {
            if y % CANCEL_ROWS == 0 {
                cancel.check()?;
            }
            let row = image.row(y as u32);
            for (&c, p) in column.iter().zip(row.chunks_exact(bpp)) {
                acc[c as usize] += u64::from(luma(p));
            }
        }
        let rows = (end - sy) as u64;
        for ((o, a), s) in out[dy * dw..(dy + 1) * dw]
            .iter_mut()
            .zip(acc.iter_mut())
            .zip(&span)
        {
            let n = s * rows;
            *o = ((*a + n / 2) / n.max(1)).min(255) as u8;
            *a = 0;
        }
        sy = end;
    }
    Ok(out)
}

/// Rec. 709 luma of an RGBA8 pixel laid over white, 0 to 255.
#[inline]
fn luma8(p: &[u8]) -> u32 {
    let y = (77 * u32::from(p[0]) + 150 * u32::from(p[1]) + 29 * u32::from(p[2]) + 128) >> 8;
    let a = u32::from(p[3]);
    if a == 255 {
        y
    } else {
        (y * a + 255 * (255 - a) + 127) / 255
    }
}

/// [`luma8`] for native-endian RGBA16 pixels.
#[inline]
fn luma16(p: &[u8]) -> u32 {
    let c = |i: usize| u32::from(u16::from_ne_bytes([p[i], p[i + 1]]));
    let y = (77 * c(0) + 150 * c(2) + 29 * c(4) + 128) >> 8;
    let a = c(6);
    let y = if a == 65_535 {
        y
    } else {
        ((u64::from(y) * u64::from(a) + 65_535 * u64::from(65_535 - a) + 32_767) / 65_535) as u32
    };
    (y + 128) / 257
}

/// Luma of a linear-light float pixel laid over white, encoded with the
/// sRGB curve through `lut`.
#[inline]
fn luma_f32(p: &[u8], lut: &[u8; LUT_SIZE]) -> u32 {
    let c = |i: usize| f32::from_ne_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
    let y = (0.2126 * finite(c(0)) + 0.7152 * finite(c(4)) + 0.0722 * finite(c(8))).clamp(0.0, 1.0);
    let a = finite(c(12)).clamp(0.0, 1.0);
    let v = y * a + (1.0 - a);
    u32::from(lut[(v * (LUT_SIZE - 1) as f32).round() as usize])
}

const LUT_SIZE: usize = 4096;

/// Linear light (index / 4095) to 8-bit sRGB.
fn srgb_lut() -> &'static [u8; LUT_SIZE] {
    static LUT: OnceLock<[u8; LUT_SIZE]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut lut = [0u8; LUT_SIZE];
        for (i, v) in lut.iter_mut().enumerate() {
            let l = i as f64 / (LUT_SIZE - 1) as f64;
            let s = if l <= 0.003_130_8 {
                12.92 * l
            } else {
                1.055 * l.powf(1.0 / 2.4) - 0.055
            };
            *v = (s * 255.0).round().clamp(0.0, 255.0) as u8;
        }
        lut
    })
}

/// Where a word box lies in the full image.
///
/// `rect` is the box as an engine reports it for an image recognized at
/// `recognized` pixels; `image` is the full image's size. `angle` is the
/// text angle the engine detected, in degrees clockwise on screen: such an
/// engine reports boxes in the image turned back by that angle around its
/// center, so the box is turned forward again, then scaled to full size.
pub fn to_image(rect: Rect, recognized: (u32, u32), image: (u32, u32), angle: f32) -> Quad {
    let (rw, rh) = (recognized.0.max(1) as f32, recognized.1.max(1) as f32);
    let (sx, sy) = (image.0 as f32 / rw, image.1 as f32 / rh);
    let (cx, cy) = (rw * 0.5, rh * 0.5);
    let angle = if angle.is_finite() { angle } else { 0.0 };
    let (s, c) = angle.to_radians().sin_cos();
    Quad::from_rect(rect).map(|p| {
        let (dx, dy) = (p.x - cx, p.y - cy);
        let (x, y) = (cx + c * dx - s * dy, cy + s * dx + c * dy);
        Point::new(x * sx, y * sy)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgba8(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> ImageBuf {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&f(x, y));
            }
        }
        ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
    }

    #[test]
    fn recognition_size_caps_the_long_side() {
        assert_eq!(recognition_size(9504, 6336, 6000), (6000, 4000));
        assert_eq!(recognition_size(6336, 9504, 6000), (4000, 6000));
        assert_eq!(recognition_size(1200, 800, 6000), (1200, 800));
        assert_eq!(recognition_size(6000, 10, 6000), (6000, 10));
        assert_eq!(recognition_size(60_000, 3, 6000), (6000, 1));
        assert_eq!(recognition_size(0, 0, 0), (1, 1));
    }

    #[test]
    fn gray_averages_blocks_and_lays_alpha_over_white() {
        // Left half black, right half white, 4 x 2 to 2 x 1.
        let img = rgba8(4, 2, |x, _| {
            if x < 2 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let cancel = CancelToken::new();
        let g = gray(&img, 2, 1, &cancel).unwrap();
        assert_eq!(g.pixels, vec![0, 255]);
        // One to one keeps every pixel.
        let g = gray(&img, 4, 2, &cancel).unwrap();
        assert_eq!(g.pixels, vec![0, 0, 255, 255, 0, 0, 255, 255]);
        // A transparent black pixel reads as white, half alpha as gray.
        let img = rgba8(2, 1, |x, _| [0, 0, 0, if x == 0 { 0 } else { 128 }]);
        let g = gray(&img, 2, 1, &cancel).unwrap();
        assert_eq!(g.pixels[0], 255);
        assert!((126..=128).contains(&g.pixels[1]), "{}", g.pixels[1]);
        // Uneven blocks: 5 columns to 2 still cover every source pixel
        // (columns 0 to 2, then 3 and 4).
        let img = rgba8(5, 1, |x, _| {
            [(x * 50) as u8, (x * 50) as u8, (x * 50) as u8, 255]
        });
        let g = gray(&img, 2, 1, &cancel).unwrap();
        assert_eq!(g.pixels, vec![50, 175]);
        assert!(matches!(gray(&img, 6, 1, &cancel), Err(Error::Invalid(_))));
        cancel.cancel();
        assert!(matches!(gray(&img, 2, 1, &cancel), Err(Error::Cancelled)));
    }

    #[test]
    fn gray_reads_deep_and_float_pixels() {
        let mut data = Vec::new();
        for v in [0u16, 0, 0, 65_535, 65_535, 65_535, 65_535, 65_535] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        let deep = ImageBuf::new(2, 1, PixelFormat::Rgba16, data).unwrap();
        let g = gray(&deep, 2, 1, &CancelToken::new()).unwrap();
        assert_eq!(g.pixels, vec![0, 255]);
        let mut data = Vec::new();
        for v in [
            0.0f32, 0.0, 0.0, 1.0, 0.214, 0.214, 0.214, 1.0, 9.0, 9.0, 9.0, 1.0,
        ] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        let hdr = ImageBuf::new(3, 1, PixelFormat::RgbaF32, data).unwrap();
        let g = gray(&hdr, 3, 1, &CancelToken::new()).unwrap();
        assert_eq!(g.pixels[0], 0);
        // Linear 0.214 is sRGB 128.
        assert!((126..=129).contains(&g.pixels[1]), "{}", g.pixels[1]);
        assert_eq!(g.pixels[2], 255, "brighter than white clips");
    }

    #[test]
    fn to_image_scales_and_turns_boxes_back() {
        let r = Rect::new(100.0, 50.0, 200.0, 20.0);
        // Recognized at half size: everything doubles.
        let q = to_image(r, (1000, 500), (2000, 1000), 0.0);
        assert_eq!(q.bounds(), Rect::new(200.0, 100.0, 400.0, 40.0));
        assert_eq!(q.points[0], Point::new(200.0, 100.0));
        // A quarter turn clockwise around the center (500, 250) of a
        // 1000 x 500 image: (x, y) goes to (750 - y, x - 250).
        let q = to_image(r, (1000, 500), (1000, 500), 90.0);
        let near = |a: Point, b: Point| (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3;
        assert!(near(q.points[0], Point::new(700.0, -150.0)), "{q:?}");
        assert!(near(q.points[1], Point::new(700.0, 50.0)), "{q:?}");
        // A bad angle is ignored rather than spreading NaN.
        let q = to_image(r, (1000, 500), (1000, 500), f32::NAN);
        assert_eq!(q, Quad::from_rect(r));
    }
}

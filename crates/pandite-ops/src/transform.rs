//! Lossless geometry: quarter-turn rotation, mirroring and cropping.
//!
//! These operations move whole pixels. They copy `bytes_per_pixel` bytes at
//! a time and never look at the values, so they are exact, reversible and
//! work for every pixel format. Large images are processed in bands on
//! several threads; rotation walks the source in cache-sized tiles.

use crate::par;
use pandite_core::{Error, ImageBuf, Rect, Result};

/// A clockwise quarter-turn rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Rotation {
    /// 90 degrees clockwise. Width and height swap.
    Cw90,
    /// 180 degrees.
    Cw180,
    /// 270 degrees clockwise (90 counterclockwise). Width and height swap.
    Cw270,
}

impl Rotation {
    /// The rotation that undoes this one.
    pub const fn inverse(self) -> Rotation {
        match self {
            Rotation::Cw90 => Rotation::Cw270,
            Rotation::Cw180 => Rotation::Cw180,
            Rotation::Cw270 => Rotation::Cw90,
        }
    }
}

/// Rotate `image` clockwise by `rotation`.
pub fn rotate(image: &ImageBuf, rotation: Rotation) -> ImageBuf {
    match rotation {
        Rotation::Cw90 => remap(image, Op::Rot90),
        Rotation::Cw180 => remap(image, Op::Rot180),
        Rotation::Cw270 => remap(image, Op::Rot270),
    }
}

/// Rotate 90 degrees clockwise: the top-left pixel moves to the top-right.
pub fn rotate90(image: &ImageBuf) -> ImageBuf {
    remap(image, Op::Rot90)
}

/// Rotate 180 degrees: the top-left pixel moves to the bottom-right.
pub fn rotate180(image: &ImageBuf) -> ImageBuf {
    remap(image, Op::Rot180)
}

/// Rotate 270 degrees clockwise: the top-left pixel moves to the
/// bottom-left.
pub fn rotate270(image: &ImageBuf) -> ImageBuf {
    remap(image, Op::Rot270)
}

/// Mirror left to right.
pub fn flip_horizontal(image: &ImageBuf) -> ImageBuf {
    remap(image, Op::FlipH)
}

/// Mirror top to bottom.
pub fn flip_vertical(image: &ImageBuf) -> ImageBuf {
    remap(image, Op::FlipV)
}

/// A rectangle of whole pixels: columns `x..x + width`, rows
/// `y..y + height`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelRect {
    /// Left column.
    pub x: u32,
    /// Top row.
    pub y: u32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl PixelRect {
    /// Build a pixel rectangle.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Turn a selection in image pixel coordinates (for example two corners
    /// from `ViewState::viewport_to_image`) into whole pixels. Edges snap to
    /// the nearest pixel boundary and are clipped to the image. Returns
    /// `None` when nothing of at least one pixel remains or a coordinate is
    /// not finite.
    pub fn from_selection(selection: Rect, image_width: u32, image_height: u32) -> Option<Self> {
        let coords = [selection.x, selection.y, selection.w, selection.h];
        if coords.iter().any(|v| !v.is_finite()) {
            return None;
        }
        let snap = |v: f32, max: u32| f64::from(v).round().clamp(0.0, f64::from(max));
        let (xa, xb) = (
            snap(selection.x, image_width),
            snap(selection.right(), image_width),
        );
        let (ya, yb) = (
            snap(selection.y, image_height),
            snap(selection.bottom(), image_height),
        );
        let (x0, x1) = (xa.min(xb), xa.max(xb));
        let (y0, y1) = (ya.min(yb), ya.max(yb));
        if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
            return None;
        }
        Some(Self::new(
            x0 as u32,
            y0 as u32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        ))
    }
}

/// Cut `rect` out of `image`. Fails with [`Error::Invalid`] when the
/// rectangle is empty or reaches outside the image. Cropping to the whole
/// image returns a cheap clone.
pub fn crop(image: &ImageBuf, rect: PixelRect) -> Result<ImageBuf> {
    let (w, h) = (image.width(), image.height());
    if rect.width == 0 || rect.height == 0 {
        return Err(Error::Invalid(format!(
            "crop rectangle {} x {} is empty",
            rect.width, rect.height
        )));
    }
    let right = u64::from(rect.x) + u64::from(rect.width);
    let bottom = u64::from(rect.y) + u64::from(rect.height);
    if right > u64::from(w) || bottom > u64::from(h) {
        return Err(Error::Invalid(format!(
            "crop {} x {} at ({}, {}) reaches outside the {w} x {h} image",
            rect.width, rect.height, rect.x, rect.y
        )));
    }
    if rect == PixelRect::new(0, 0, w, h) {
        return Ok(image.clone());
    }
    let bpp = image.format().bytes_per_pixel();
    let start = rect.x as usize * bpp;
    let row_len = rect.width as usize * bpp;
    let mut out = Vec::with_capacity(row_len * rect.height as usize);
    for y in rect.y..rect.y + rect.height {
        out.extend_from_slice(&image.row(y)[start..start + row_len]);
    }
    ImageBuf::new(rect.width, rect.height, image.format(), out)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Rot90,
    Rot180,
    Rot270,
    FlipH,
    FlipV,
}

/// Rows and columns of a rotation tile. 64 source rows of 64-byte cache
/// lines stay in L1 while the tile is copied.
const TILE: usize = 64;

fn remap(image: &ImageBuf, op: Op) -> ImageBuf {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let bpp = image.format().bytes_per_pixel();
    let (dw, dh) = match op {
        Op::Rot90 | Op::Rot270 => (h, w),
        _ => (w, h),
    };
    let stride = dw * bpp;
    let mut out = vec![0u8; image.byte_size()];
    let threads = par::plan_threads(0, 2 * (w * h) as u64, dh);
    let src = image.data();
    // The band closure cannot fail, so the result is always Ok.
    let _ = par::for_each_band(&mut out, stride, threads, |first, band| {
        match bpp {
            4 => fill(4, src, w, h, op, first, band),
            8 => fill(8, src, w, h, op, first, band),
            16 => fill(16, src, w, h, op, first, band),
            n => fill(n, src, w, h, op, first, band),
        }
        Ok(())
    });
    ImageBuf::new(dw as u32, dh as u32, image.format(), out)
        .expect("a rotated or mirrored image has the same byte length as its source")
}

/// Fill output rows `first..` of `band` for `op`. Inlined into each call
/// site above so `bpp` is a constant there and the pixel copies are fixed
/// size.
#[inline(always)]
fn fill(bpp: usize, src: &[u8], w: usize, h: usize, op: Op, first: usize, band: &mut [u8]) {
    let sstride = w * bpp;
    let src_row = |y: usize| &src[y * sstride..(y + 1) * sstride];
    match op {
        Op::FlipV => {
            for (r, row) in band.chunks_exact_mut(sstride).enumerate() {
                row.copy_from_slice(src_row(h - 1 - (first + r)));
            }
        }
        Op::FlipH | Op::Rot180 => {
            for (r, row) in band.chunks_exact_mut(sstride).enumerate() {
                let y = first + r;
                let from = src_row(if op == Op::FlipH { y } else { h - 1 - y });
                for (d, s) in row.chunks_exact_mut(bpp).zip(from.chunks_exact(bpp).rev()) {
                    d.copy_from_slice(s);
                }
            }
        }
        Op::Rot90 | Op::Rot270 => {
            // Output is h wide and w tall. Output row yd is source column
            // cx; output column xd reads source row sy.
            let dstride = h * bpp;
            for x0 in (0..h).step_by(TILE) {
                let x1 = (x0 + TILE).min(h);
                for (r, row) in band.chunks_exact_mut(dstride).enumerate() {
                    let yd = first + r;
                    let cx = if op == Op::Rot90 { yd } else { w - 1 - yd };
                    let tile = &mut row[x0 * bpp..x1 * bpp];
                    for (d, xd) in tile.chunks_exact_mut(bpp).zip(x0..x1) {
                        let sy = if op == Op::Rot90 { h - 1 - xd } else { xd };
                        let s = sy * sstride + cx * bpp;
                        d.copy_from_slice(&src[s..s + bpp]);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::PixelFormat;

    #[test]
    fn selection_snaps_and_clips() {
        let sel = Rect::new(1.4, 2.6, 3.2, 10.0);
        assert_eq!(
            PixelRect::from_selection(sel, 100, 8),
            Some(PixelRect::new(1, 3, 4, 5))
        );
        let backwards = Rect::new(10.0, 10.0, -5.0, -5.0);
        assert_eq!(
            PixelRect::from_selection(backwards, 100, 100),
            Some(PixelRect::new(5, 5, 5, 5))
        );
        assert_eq!(
            PixelRect::from_selection(Rect::new(3.0, 3.0, 0.4, 9.0), 9, 9),
            None
        );
        assert_eq!(
            PixelRect::from_selection(Rect::new(50.0, 0.0, 9.0, 9.0), 9, 9),
            None
        );
        let nan = Rect::new(f32::NAN, 0.0, 1.0, 1.0);
        assert_eq!(PixelRect::from_selection(nan, 9, 9), None);
    }

    #[test]
    fn rotation_inverse_and_any_pixel_size() {
        for r in [Rotation::Cw90, Rotation::Cw180, Rotation::Cw270] {
            assert_eq!(r.inverse().inverse(), r);
        }
        // The fallback arm for pixel sizes of future formats: 3 bytes here.
        let (w, h) = (6usize, 4usize);
        let src: Vec<u8> = (0..w * h)
            .flat_map(|i| [(i % w) as u8, (i / w) as u8, 7])
            .collect();
        let mut out = vec![0u8; src.len()];
        fill(3, &src, w, h, Op::Rot90, 0, &mut out);
        for y in 0..h {
            for x in 0..w {
                let d = (x * h + (h - 1 - y)) * 3;
                assert_eq!(out[d..d + 3], [x as u8, y as u8, 7]);
            }
        }
        // A runtime pixel size gives the same bytes as the specialized copy.
        let data: Vec<u8> = (0..6 * 4 * 16).map(|i| (i % 251) as u8).collect();
        let img = ImageBuf::new(6, 4, PixelFormat::RgbaF32, data).unwrap();
        let mut out = vec![0u8; img.byte_size()];
        fill(16, img.data(), 6, 4, Op::Rot90, 0, &mut out);
        let mut generic = vec![0u8; img.byte_size()];
        let bpp = std::hint::black_box(16);
        fill(bpp, img.data(), 6, 4, Op::Rot90, 0, &mut generic);
        assert_eq!(out, generic);
        assert_eq!(rotate(&img, Rotation::Cw90).data(), &out[..]);
    }

    #[test]
    fn large_rotation_uses_tiles_and_threads() {
        // 700 x 300 crosses tile and band boundaries.
        let (w, h) = (700u32, 300u32);
        let data: Vec<u8> = (0..w * h)
            .flat_map(|i| (i.wrapping_mul(2_654_435_761)).to_le_bytes())
            .collect();
        let img = ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap();
        let r = rotate90(&img);
        assert_eq!((r.width(), r.height()), (h, w));
        for &(x, y) in &[(0u32, 0u32), (699, 0), (0, 299), (699, 299), (350, 77)] {
            let s = ((y * w + x) * 4) as usize;
            let (dx, dy) = (h - 1 - y, x);
            let d = ((dy * h + dx) * 4) as usize;
            assert_eq!(r.data()[d..d + 4], img.data()[s..s + 4]);
        }
        let back = rotate270(&r);
        assert_eq!(back.data(), img.data());
    }
}

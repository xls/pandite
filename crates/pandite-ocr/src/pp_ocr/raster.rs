//! Pixels for the models: an 8-bit copy of the image in blue, green, red
//! order, and the resampling the reference pipeline does with OpenCV
//! (bilinear resize, bicubic perspective crop with replicated borders).

use pandite_core::{CancelToken, Error, ImageBuf, PixelFormat, Result};
use std::sync::OnceLock;

/// Source rows between two cancellation checks.
const CANCEL_ROWS: usize = 64;

/// An 8-bit image with three bytes per pixel in blue, green, red order
/// (the order the models were trained on). Rows top first, no padding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Bgr {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Bgr {
    /// A `width` x `height` image with every byte set to `fill`.
    pub fn filled(width: usize, height: usize, fill: u8) -> Self {
        Self {
            width,
            height,
            data: vec![fill; width * height * 3],
        }
    }

    /// The three bytes of pixel (`x`, `y`).
    #[inline]
    fn px(&self, x: usize, y: usize) -> &[u8] {
        let i = (y * self.width + x) * 3;
        &self.data[i..i + 3]
    }

    /// The image with `rows` black rows added above and below.
    pub fn pad_rows(&self, rows: usize) -> Bgr {
        let mut out = Bgr::filled(self.width, self.height + 2 * rows, 0);
        let start = rows * self.width * 3;
        out.data[start..start + self.data.len()].copy_from_slice(&self.data);
        out
    }

    /// The image turned a quarter counter-clockwise (`numpy.rot90`): the
    /// right column becomes the top row.
    pub fn rot90(&self) -> Bgr {
        let (w, h) = (self.width, self.height);
        let mut out = Bgr::filled(h, w, 0);
        for r in 0..w {
            for c in 0..h {
                let o = (r * h + c) * 3;
                out.data[o..o + 3].copy_from_slice(self.px(w - 1 - r, c));
            }
        }
        out
    }
}

/// `image` as an 8-bit [`Bgr`] copy at `width` x `height`. Sides that
/// shrink average every source pixel a target pixel covers; sides that
/// grow are then interpolated bilinearly. Transparent pixels are laid
/// over white and high dynamic range pixels are clipped at display white,
/// as for the gray copy of [`crate::prepare::gray`]. Checks `cancel`
/// every few rows.
pub(crate) fn from_image(
    image: &ImageBuf,
    width: usize,
    height: usize,
    cancel: &CancelToken,
) -> Result<Bgr> {
    let (sw, sh) = (image.width() as usize, image.height() as usize);
    if width == 0 || height == 0 {
        return Err(Error::Invalid(format!(
            "color copy of {width} x {height} for a {sw} x {sh} image"
        )));
    }
    let (rw, rh) = (width.min(sw), height.min(sh));
    let reduced = match image.format() {
        PixelFormat::Rgba8 => reduce(image, rw, rh, cancel, rgb8)?,
        PixelFormat::Rgba16 => reduce(image, rw, rh, cancel, rgb16)?,
        PixelFormat::RgbaF32 => {
            let lut = srgb_lut();
            reduce(image, rw, rh, cancel, |p| rgb_f32(p, lut))?
        }
        other => {
            return Err(Error::Unsupported(format!(
                "text recognition of {other:?} pixels"
            )))
        }
    };
    cancel.check()?;
    Ok(resize(&reduced, width, height))
}

/// Area-average `image` down to `width` x `height` (at most its own
/// size). Every target pixel averages a whole block of source pixels.
fn reduce(
    image: &ImageBuf,
    width: usize,
    height: usize,
    cancel: &CancelToken,
    rgb: impl Fn(&[u8]) -> [u32; 3],
) -> Result<Bgr> {
    let (sw, sh) = (image.width() as usize, image.height() as usize);
    let bpp = image.format().bytes_per_pixel();
    let mut out = Bgr::filled(width, height, 0);
    // Target column of every source column, and how many source columns
    // each target column averages.
    let column: Vec<usize> = (0..sw).map(|x| x * width / sw).collect();
    let mut span = vec![0u64; width];
    for &c in &column {
        span[c] += 1;
    }
    let mut acc = vec![[0u64; 3]; width];
    let mut sy = 0usize;
    for dy in 0..height {
        let end = ((dy + 1) * sh / height).max(sy + 1).min(sh);
        for y in sy..end {
            if y % CANCEL_ROWS == 0 {
                cancel.check()?;
            }
            for (&c, p) in column.iter().zip(image.row(y as u32).chunks_exact(bpp)) {
                let [r, g, b] = rgb(p);
                let a = &mut acc[c];
                a[0] += u64::from(b);
                a[1] += u64::from(g);
                a[2] += u64::from(r);
            }
        }
        let rows = (end - sy) as u64;
        let row = &mut out.data[dy * width * 3..(dy + 1) * width * 3];
        for ((o, a), s) in row.chunks_exact_mut(3).zip(acc.iter_mut()).zip(&span) {
            let n = (s * rows).max(1);
            for (o, a) in o.iter_mut().zip(a.iter_mut()) {
                *o = ((*a + n / 2) / n).min(255) as u8;
                *a = 0;
            }
        }
        sy = end;
    }
    Ok(out)
}

/// An RGBA8 pixel laid over white, as red, green, blue from 0 to 255.
#[inline]
fn rgb8(p: &[u8]) -> [u32; 3] {
    let a = u32::from(p[3]);
    let c = |v: u8| {
        let v = u32::from(v);
        if a == 255 {
            v
        } else {
            (v * a + 255 * (255 - a) + 127) / 255
        }
    };
    [c(p[0]), c(p[1]), c(p[2])]
}

/// [`rgb8`] for native-endian RGBA16 pixels.
#[inline]
fn rgb16(p: &[u8]) -> [u32; 3] {
    let c = |i: usize| u64::from(u16::from_ne_bytes([p[i], p[i + 1]]));
    let a = c(6);
    let over = |v: u64| {
        let v = if a == 65_535 {
            v
        } else {
            (v * a + 65_535 * (65_535 - a) + 32_767) / 65_535
        };
        ((v + 128) / 257) as u32
    };
    [over(c(0)), over(c(2)), over(c(4))]
}

/// A linear-light float pixel laid over white, encoded with the sRGB
/// curve through `lut`.
#[inline]
fn rgb_f32(p: &[u8], lut: &[u8; LUT_SIZE]) -> [u32; 3] {
    let c = |i: usize| {
        let v = f32::from_ne_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
        if v.is_finite() {
            v
        } else {
            0.0
        }
    };
    let a = c(12).clamp(0.0, 1.0);
    let enc = |v: f32| {
        let v = v.clamp(0.0, 1.0) * a + (1.0 - a);
        u32::from(lut[(v * (LUT_SIZE - 1) as f32).round() as usize])
    };
    [enc(c(0)), enc(c(4)), enc(c(8))]
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

/// One output position of a bilinear resize: the two source positions
/// and the weight of the second.
#[derive(Clone, Copy)]
struct Tap {
    lo: usize,
    hi: usize,
    t: f32,
}

/// Source taps of `OpenCV`'s `INTER_LINEAR` for `dst` positions from
/// `src`: pixel centers line up, positions past an edge take the edge.
fn taps(src: usize, dst: usize) -> Vec<Tap> {
    let scale = src as f64 / dst as f64;
    (0..dst)
        .map(|d| {
            let f = (d as f64 + 0.5) * scale - 0.5;
            let lo = f.floor();
            let (lo, t) = if lo < 0.0 {
                (0, 0.0)
            } else if lo as usize >= src - 1 {
                (src - 1, 0.0)
            } else {
                (lo as usize, (f - lo) as f32)
            };
            Tap {
                lo,
                hi: (lo + 1).min(src - 1),
                t,
            }
        })
        .collect()
}

/// `src` resized to `width` x `height` with bilinear interpolation, as
/// `cv2.resize` does by default. The same size is a copy.
pub(crate) fn resize(src: &Bgr, width: usize, height: usize) -> Bgr {
    if (width, height) == (src.width, src.height) {
        return src.clone();
    }
    let xs = taps(src.width, width);
    let ys = taps(src.height, height);
    let mut out = Bgr::filled(width, height, 0);
    let mut lo_row = vec![0f32; width * 3];
    let mut hi_row = vec![0f32; width * 3];
    let horizontal = |y: usize, row: &mut [f32]| {
        for (o, tap) in row.chunks_exact_mut(3).zip(&xs) {
            let (a, b) = (src.px(tap.lo, y), src.px(tap.hi, y));
            for c in 0..3 {
                o[c] = f32::from(a[c]) + (f32::from(b[c]) - f32::from(a[c])) * tap.t;
            }
        }
    };
    for (y, tap) in ys.iter().enumerate() {
        horizontal(tap.lo, &mut lo_row);
        horizontal(tap.hi, &mut hi_row);
        let row = &mut out.data[y * width * 3..(y + 1) * width * 3];
        for ((o, a), b) in row.iter_mut().zip(&lo_row).zip(&hi_row) {
            *o = (a + (b - a) * tap.t + 0.5).clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The image as a model input: `[1, 3, height, width]` floats in blue,
/// green, red planes, each byte `v` as `v / 127.5 - 1` (mean and standard
/// deviation 0.5), padded with zeros on the right up to `padded_width`.
pub(crate) fn planes(image: &Bgr, padded_width: usize) -> Vec<f32> {
    let (w, h) = (image.width, image.height);
    let pw = padded_width.max(w);
    let mut out = vec![0f32; 3 * h * pw];
    for c in 0..3 {
        for y in 0..h {
            let row = &mut out[(c * h + y) * pw..(c * h + y) * pw + w];
            for (x, o) in row.iter_mut().enumerate() {
                *o = (f32::from(image.data[(y * w + x) * 3 + c]) / 255.0 - 0.5) / 0.5;
            }
        }
    }
    out
}

/// Which columns of a line crop hold ink: those where the brightness of
/// the middle half of the rows varies by at least a quarter of the most
/// any column varies (and at least 24 of 255). Light text on dark and
/// dark text on light count alike.
pub(crate) fn ink_columns(image: &Bgr) -> Vec<bool> {
    let (w, h) = (image.width, image.height);
    let top = h / 4;
    let bottom = (3 * h / 4).max(top + 1).min(h);
    let range: Vec<u8> = (0..w)
        .map(|x| {
            let (mut lo, mut hi) = (u8::MAX, 0u8);
            for y in top..bottom {
                let p = image.px(x, y);
                let l = (29 * u32::from(p[0]) + 150 * u32::from(p[1]) + 77 * u32::from(p[2]) + 128)
                    >> 8;
                let l = l.min(255) as u8;
                lo = lo.min(l);
                hi = hi.max(l);
            }
            hi.saturating_sub(lo)
        })
        .collect();
    let limit = (range.iter().copied().max().unwrap_or(0) / 4).max(24);
    range.iter().map(|&r| r >= limit).collect()
}

/// A perspective map from crop pixels to image pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Homography([f64; 9]);

impl Homography {
    /// The map that takes the corners of a `width` x `height` rectangle
    /// (top-left, top-right, bottom-right, bottom-left) to `quad`, as
    /// `cv2.getPerspectiveTransform` builds it. None for a degenerate quad.
    pub fn from_rect(width: f64, height: f64, quad: &[[f64; 2]; 4]) -> Option<Self> {
        let from = [[0.0, 0.0], [width, 0.0], [width, height], [0.0, height]];
        // Eight equations in the eight unknowns a..h of
        // u = (a x + b y + c) / (g x + h y + 1), v = (d x + e y + f) / (...).
        let mut m = [[0f64; 9]; 8];
        for i in 0..4 {
            let ([x, y], [u, v]) = (from[i], quad[i]);
            m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -x * u, -y * u, u];
            m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -x * v, -y * v, v];
        }
        for col in 0..8 {
            let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
            if m[pivot][col].abs() < 1e-12 {
                return None;
            }
            m.swap(col, pivot);
            let lead = m[col];
            for (row, r) in m.iter_mut().enumerate() {
                if row != col {
                    let f = r[col] / lead[col];
                    if f != 0.0 {
                        for (a, b) in r[col..].iter_mut().zip(&lead[col..]) {
                            *a -= f * b;
                        }
                    }
                }
            }
        }
        let s = |i: usize| m[i][8] / m[i][i];
        let h = [s(0), s(1), s(2), s(3), s(4), s(5), s(6), s(7), 1.0];
        h.iter().all(|v| v.is_finite()).then_some(Self(h))
    }

    /// Where crop point (`x`, `y`) lies in the image.
    pub fn apply(&self, x: f64, y: f64) -> [f64; 2] {
        let h = &self.0;
        let w = h[6] * x + h[7] * y + h[8];
        let w = if w.abs() < 1e-12 { 1e-12 } else { w };
        [
            (h[0] * x + h[1] * y + h[2]) / w,
            (h[3] * x + h[4] * y + h[5]) / w,
        ]
    }
}

/// `OpenCV`'s bicubic weights (A = -0.75) for a sample `t` past a pixel.
#[inline]
fn cubic(t: f32) -> [f32; 4] {
    const A: f32 = -0.75;
    let w0 = ((A * (t + 1.0) - 5.0 * A) * (t + 1.0) + 8.0 * A) * (t + 1.0) - 4.0 * A;
    let w1 = ((A + 2.0) * t - (A + 3.0)) * t * t + 1.0;
    let w2 = ((A + 2.0) * (1.0 - t) - (A + 3.0)) * (1.0 - t) * (1.0 - t) + 1.0;
    [w0, w1, w2, 1.0 - w0 - w1 - w2]
}

/// The `width` x `height` crop of `src` whose pixel (x, y) shows image
/// point `map(x, y)`, sampled bicubically with edge pixels repeated past
/// the border (`cv2.warpPerspective` with `INTER_CUBIC` and
/// `BORDER_REPLICATE`).
pub(crate) fn warp(src: &Bgr, map: &Homography, width: usize, height: usize) -> Bgr {
    let mut out = Bgr::filled(width, height, 0);
    let (sw, sh) = (src.width as i64, src.height as i64);
    for y in 0..height {
        for x in 0..width {
            let [u, v] = map.apply(x as f64, y as f64);
            // Far outside points clamp to the edge anyway; keep the
            // arithmetic in range.
            let u = u.clamp(-4.0, sw as f64 + 4.0);
            let v = v.clamp(-4.0, sh as f64 + 4.0);
            let (fu, fv) = (u.floor(), v.floor());
            let (wx, wy) = (cubic((u - fu) as f32), cubic((v - fv) as f32));
            let (iu, iv) = (fu as i64, fv as i64);
            let mut acc = [0f32; 3];
            for (j, wyj) in wy.iter().enumerate() {
                let sy = (iv + j as i64 - 1).clamp(0, sh - 1) as usize;
                for (i, wxi) in wx.iter().enumerate() {
                    let sx = (iu + i as i64 - 1).clamp(0, sw - 1) as usize;
                    let w = wxi * wyj;
                    for (a, &v) in acc.iter_mut().zip(src.px(sx, sy)) {
                        *a += w * f32::from(v);
                    }
                }
            }
            let o = (y * width + x) * 3;
            for (d, a) in out.data[o..o + 3].iter_mut().zip(acc) {
                *d = (a + 0.5).floor().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bgr(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Bgr {
        let mut img = Bgr::filled(w, h, 0);
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                img.data[i..i + 3].copy_from_slice(&f(x, y));
            }
        }
        img
    }

    #[test]
    fn color_copy_reorders_lays_over_white_and_averages() {
        let mut data = Vec::new();
        // Red, then transparent black, then half transparent blue.
        for p in [
            [255u8, 0, 0, 255],
            [0, 0, 0, 0],
            [0, 0, 255, 128],
            [0, 0, 255, 128],
        ] {
            data.extend_from_slice(&p);
        }
        let img = ImageBuf::new(4, 1, PixelFormat::Rgba8, data).unwrap();
        let cancel = CancelToken::new();
        let full = from_image(&img, 4, 1, &cancel).unwrap();
        assert_eq!(&full.data[0..3], &[0, 0, 255], "red is the last byte");
        assert_eq!(&full.data[3..6], &[255, 255, 255], "transparent is white");
        assert_eq!(full.data[6], 255);
        assert!((126..=128).contains(&full.data[7]), "{:?}", full.data);
        // Halved: (red + white) / 2 and the two blues.
        let half = from_image(&img, 2, 1, &cancel).unwrap();
        assert_eq!(&half.data[0..3], &[128, 128, 255]);
        // Enlarged: interpolated, edges kept.
        let big = from_image(&img, 8, 2, &cancel).unwrap();
        assert_eq!((big.width, big.height), (8, 2));
        assert_eq!(&big.data[0..3], &[0, 0, 255]);
        cancel.cancel();
        assert!(matches!(
            from_image(&img, 2, 1, &cancel),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn resize_matches_opencv_linear() {
        // cv2.resize of [0, 100] to 4 pixels: centers at -0.25, 0.25,
        // 0.75, 1.25 of the source, clamped at the edges.
        let src = bgr(2, 1, |x, _| [(x * 100) as u8; 3]);
        let out = resize(&src, 4, 1);
        let row: Vec<u8> = out.data.chunks(3).map(|p| p[0]).collect();
        assert_eq!(row, vec![0, 25, 75, 100]);
        // Same size copies.
        assert_eq!(resize(&src, 2, 1), src);
    }

    #[test]
    fn padding_turning_and_planes() {
        let img = bgr(3, 2, |x, y| [(10 * y + x) as u8, 0, 255]);
        let padded = img.pad_rows(1);
        assert_eq!((padded.width, padded.height), (3, 4));
        assert!(padded.data[..9].iter().all(|&v| v == 0));
        assert_eq!(&padded.data[9..12], &[0, 0, 255]);
        // A quarter turn counter-clockwise: the right column on top.
        let turned = img.rot90();
        assert_eq!((turned.width, turned.height), (2, 3));
        let firsts: Vec<u8> = turned.data.chunks(3).map(|p| p[0]).collect();
        assert_eq!(firsts, vec![2, 12, 1, 11, 0, 10]);
        // Planes: blue first, scaled to -1..1, zero padding on the right.
        let p = planes(&img, 4);
        assert_eq!(p.len(), 3 * 2 * 4);
        assert_eq!(p[0], -1.0);
        assert_eq!(p[3], 0.0, "padding");
        assert_eq!(p[2 * 2 * 4], 1.0, "red plane, 255");
    }

    #[test]
    fn homography_maps_corners_and_warp_copies_an_axis_aligned_crop() {
        let quad = [[10.0, 20.0], [110.0, 25.0], [105.0, 75.0], [5.0, 70.0]];
        let h = Homography::from_rect(50.0, 25.0, &quad).unwrap();
        let near =
            |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
        assert!(near(h.apply(0.0, 0.0), quad[0]));
        assert!(near(h.apply(50.0, 0.0), quad[1]));
        assert!(near(h.apply(50.0, 25.0), quad[2]));
        assert!(near(h.apply(0.0, 25.0), quad[3]));
        assert!(Homography::from_rect(10.0, 10.0, &[[0.0, 0.0]; 4]).is_none());
        // An integer-aligned crop copies pixels exactly.
        let src = bgr(8, 6, |x, y| [(x * 30) as u8, (y * 40) as u8, 7]);
        let h = Homography::from_rect(3.0, 2.0, &[[2.0, 1.0], [5.0, 1.0], [5.0, 3.0], [2.0, 3.0]])
            .unwrap();
        let crop = warp(&src, &h, 3, 2);
        for y in 0..2 {
            for x in 0..3 {
                assert_eq!(crop.px(x, y), src.px(x + 2, y + 1), "{x} {y}");
            }
        }
    }
}

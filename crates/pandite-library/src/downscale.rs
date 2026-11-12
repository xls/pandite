//! The built-in thumbnail downscaler: an exact area-average (box) filter.

use pandite_core::{ImageBuf, PixelFormat};

/// The size `(width, height)` that fits `(width, height)` inside a
/// `max_side` square, keeping the aspect ratio. Never enlarges, never
/// returns a zero side.
pub fn fit_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let max_side = max_side.max(1);
    if width <= max_side && height <= max_side {
        return (width.max(1), height.max(1));
    }
    let (w, h, m) = (u64::from(width), u64::from(height), u64::from(max_side));
    if w >= h {
        let nh = ((h * m + w / 2) / w).clamp(1, m);
        (max_side, nh as u32)
    } else {
        let nw = ((w * m + h / 2) / h).clamp(1, m);
        (nw as u32, max_side)
    }
}

/// Shrink `image` so its longer side is at most `max_side`, averaging every
/// source pixel that covers a destination pixel with its exact area weight.
///
/// Color is weighted by alpha, so transparent pixels do not darken edges.
/// Rgba8 and Rgba16 average in their stored (sRGB) encoding, RgbaF32 in
/// linear light; non-finite float samples count as zero. An image that
/// already fits, or a pixel format this function does not know, is
/// returned unchanged (a cheap clone).
pub fn box_downscale(image: &ImageBuf, max_side: u32) -> ImageBuf {
    let (sw, sh) = (image.width(), image.height());
    let (dw, dh) = fit_size(sw, sh, max_side);
    if (dw, dh) == (sw, sh) {
        return image.clone();
    }
    let format = image.format();
    let read: fn(&[u8], &mut [f64]) = match format {
        PixelFormat::Rgba8 => read_rgba8,
        PixelFormat::Rgba16 => read_rgba16,
        PixelFormat::RgbaF32 => read_rgbaf32,
        _ => return image.clone(),
    };
    let xmap = axis_map(sw, dw);
    let ymap = axis_map(sh, dh);
    let dwu = dw as usize;
    // Premultiplied red, green, blue and alpha sums per destination pixel.
    let mut acc = vec![0f64; dwu * dh as usize * 4];
    let mut src = vec![0f64; sw as usize * 4];
    let mut row = vec![0f64; dwu * 4];
    for y in 0..sh {
        read(image.row(y), &mut src);
        row.iter_mut().for_each(|v| *v = 0.0);
        for (x, span) in xmap.iter().enumerate() {
            let px = &src[x * 4..x * 4 + 4];
            let a = px[3];
            let premul = [px[0] * a, px[1] * a, px[2] * a, a];
            for (dst, weight) in span.parts() {
                let out = &mut row[dst * 4..dst * 4 + 4];
                for c in 0..4 {
                    out[c] += premul[c] * weight;
                }
            }
        }
        for (dst_y, weight) in ymap[y as usize].parts() {
            let line = &mut acc[dst_y * dwu * 4..(dst_y + 1) * dwu * 4];
            for (out, v) in line.iter_mut().zip(&row) {
                *out += v * weight;
            }
        }
    }
    let area = f64::from(sw) * f64::from(sh);
    let mut data = Vec::with_capacity(dwu * dh as usize * format.bytes_per_pixel());
    for px in acc.chunks_exact(4) {
        let alpha_sum = px[3];
        let rgb = if alpha_sum > 0.0 {
            [px[0] / alpha_sum, px[1] / alpha_sum, px[2] / alpha_sum]
        } else {
            [0.0; 3]
        };
        let out = [rgb[0], rgb[1], rgb[2], alpha_sum / area];
        write_pixel(format, out, &mut data);
    }
    ImageBuf::new(dw, dh, format, data).unwrap_or_else(|_| image.clone())
}

/// How one source pixel splits over at most two destination pixels.
#[derive(Clone, Copy)]
struct Span {
    first: usize,
    first_weight: f64,
    second_weight: f64,
}

impl Span {
    fn parts(self) -> impl Iterator<Item = (usize, f64)> {
        let second = (self.second_weight > 0.0).then_some((self.first + 1, self.second_weight));
        std::iter::once((self.first, self.first_weight)).chain(second)
    }
}

/// For a `src` to `dst` reduction (`dst <= src`), source pixel `i` covers
/// `[i * dst, (i + 1) * dst)` and destination pixel `j` covers
/// `[j * src, (j + 1) * src)` in units of `1 / src` destination pixels.
/// The overlaps are exact integers; each destination pixel collects a
/// total weight of `src`.
fn axis_map(src: u32, dst: u32) -> Vec<Span> {
    let (s, d) = (u64::from(src), u64::from(dst));
    (0..s)
        .map(|i| {
            let start = i * d;
            let end = start + d;
            let first = start / s;
            let boundary = (first + 1) * s;
            if end <= boundary {
                Span {
                    first: first as usize,
                    first_weight: d as f64,
                    second_weight: 0.0,
                }
            } else {
                Span {
                    first: first as usize,
                    first_weight: (boundary - start) as f64,
                    second_weight: (end - boundary) as f64,
                }
            }
        })
        .collect()
}

fn read_rgba8(row: &[u8], out: &mut [f64]) {
    for (o, v) in out.iter_mut().zip(row) {
        *o = f64::from(*v) / 255.0;
    }
}

fn read_rgba16(row: &[u8], out: &mut [f64]) {
    for (o, v) in out.iter_mut().zip(row.chunks_exact(2)) {
        *o = f64::from(u16::from_ne_bytes([v[0], v[1]])) / 65535.0;
    }
}

fn read_rgbaf32(row: &[u8], out: &mut [f64]) {
    for (i, (o, v)) in out.iter_mut().zip(row.chunks_exact(4)).enumerate() {
        let x = f32::from_ne_bytes([v[0], v[1], v[2], v[3]]);
        let x = if x.is_finite() { f64::from(x) } else { 0.0 };
        // Alpha is a coverage weight: keep it in 0..=1.
        *o = if i % 4 == 3 { x.clamp(0.0, 1.0) } else { x };
    }
}

fn write_pixel(format: PixelFormat, px: [f64; 4], data: &mut Vec<u8>) {
    match format {
        PixelFormat::Rgba16 => {
            for v in px {
                let q = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
                data.extend_from_slice(&q.to_ne_bytes());
            }
        }
        PixelFormat::RgbaF32 => {
            for v in px {
                data.extend_from_slice(&(v as f32).to_ne_bytes());
            }
        }
        _ => {
            for v in px {
                data.push((v.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
    }
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
    fn fit_size_keeps_aspect_and_never_enlarges() {
        assert_eq!(fit_size(4000, 3000, 256), (256, 192));
        assert_eq!(fit_size(3000, 4000, 256), (192, 256));
        assert_eq!(fit_size(100, 50, 256), (100, 50));
        assert_eq!(fit_size(10000, 1, 256), (256, 1));
        assert_eq!(fit_size(5, 5, 0), (1, 1));
    }

    #[test]
    fn box_filter_averages_exact_areas() {
        // 2x1 blocks of black and white become mid gray.
        let img = rgba8(4, 2, |x, _| {
            if x % 2 == 0 {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 255]
            }
        });
        let out = box_downscale(&img, 2);
        assert_eq!((out.width(), out.height()), (2, 1));
        for px in out.data().chunks_exact(4) {
            assert_eq!(px, &[128, 128, 128, 255]);
        }
        // Non-integer ratio 3 -> 2: weights 2/3 and 1/3 split the middle pixel.
        let img = rgba8(3, 1, |x, _| {
            [[0, 0, 0, 255], [90, 90, 90, 255], [180, 180, 180, 255]][x as usize]
        });
        let out = box_downscale(&img, 2);
        assert_eq!((out.width(), out.height()), (2, 1));
        assert_eq!(&out.data()[0..4], &[30, 30, 30, 255]);
        assert_eq!(&out.data()[4..8], &[150, 150, 150, 255]);
    }

    #[test]
    fn box_filter_weights_color_by_alpha() {
        // A transparent black pixel next to opaque red stays red, half alpha.
        let img = rgba8(2, 1, |x, _| {
            if x == 0 {
                [0, 0, 0, 0]
            } else {
                [255, 0, 0, 255]
            }
        });
        let out = box_downscale(&img, 1);
        assert_eq!(out.data(), &[255, 0, 0, 128]);
    }

    #[test]
    fn box_filter_handles_16_bit_and_float() {
        let mut data = Vec::new();
        for v in [0u16, 65535, 65535, 65535, 65535, 65535, 65535, 65535] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        let img = ImageBuf::new(2, 1, PixelFormat::Rgba16, data).unwrap();
        let out = box_downscale(&img, 1);
        assert_eq!(out.format(), PixelFormat::Rgba16);
        let r = u16::from_ne_bytes([out.data()[0], out.data()[1]]);
        assert_eq!(r, 32768);
        let mut data = Vec::new();
        for v in [4.0f32, 0.0, 0.0, 1.0, 0.0, f32::NAN, 0.0, 1.0] {
            data.extend_from_slice(&v.to_ne_bytes());
        }
        let img = ImageBuf::new(2, 1, PixelFormat::RgbaF32, data).unwrap();
        let out = box_downscale(&img, 1);
        let px: Vec<f32> = out
            .data()
            .chunks_exact(4)
            .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert_eq!(px, vec![2.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn small_images_are_returned_as_is() {
        let img = rgba8(3, 2, |_, _| [1, 2, 3, 4]);
        let out = box_downscale(&img, 8);
        assert_eq!(out.data(), img.data());
        assert_eq!((out.width(), out.height()), (3, 2));
    }
}

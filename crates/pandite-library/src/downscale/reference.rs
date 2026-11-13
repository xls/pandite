//! The floating point box filter: the path for Rgba16 and RgbaF32, and the
//! oracle the integer Rgba8 path is tested and timed against.

use super::fit_size;
use pandite_core::{ImageBuf, PixelFormat};

/// [`super::box_downscale`] in `f64` for every pixel format: the
/// implementation Rgba16 and RgbaF32 use, and the reference the Rgba8
/// integer path must stay within one step of.
pub fn box_downscale_reference(image: &ImageBuf, max_side: u32) -> ImageBuf {
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

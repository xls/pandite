//! Pixel format conversion and the sRGB transfer function.
//!
//! [`PixelFormat::Rgba8`] and [`PixelFormat::Rgba16`] hold sRGB-encoded
//! color. [`PixelFormat::RgbaF32`] holds linear light. Conversions apply the
//! exact IEC 61966-2-1 transfer curve in both directions, so 8-bit and 16-bit
//! values survive a round trip through floats unchanged.
//!
//! Float to integer conversion is a tone clamp: color above display white
//! (including positive infinity) becomes white, and negative values and NaN
//! become black. Alpha is straight (not premultiplied) in every format and is
//! clamped to `0..=1`.

use crate::par;
use pandite_core::{Error, ImageBuf, PixelFormat, Result};
use std::sync::OnceLock;

/// Decode an sRGB-encoded value (0 to 1) to linear light. Values outside
/// `0..=1` follow the same curve and are not clamped.
pub fn srgb_to_linear(v: f32) -> f32 {
    decode(f64::from(v)) as f32
}

/// Encode linear light (0 to 1) with the sRGB curve. Values outside `0..=1`
/// follow the same curve and are not clamped.
pub fn linear_to_srgb(v: f32) -> f32 {
    encode(f64::from(v)) as f32
}

/// Convert `image` to `format`. Returns a cheap clone when the format already
/// matches. Fails with [`Error::TooLarge`] when the result would exceed the
/// buffer limits (a large 8-bit image grows four times as `RgbaF32`), and
/// with [`Error::Unsupported`] for a pixel format this crate does not know.
pub fn convert(image: &ImageBuf, format: PixelFormat) -> Result<ImageBuf> {
    if image.format() == format {
        return Ok(image.clone());
    }
    let from = Layout::of(image.format())?;
    let to = Layout::of(format)?;
    let (w, h) = (image.width(), image.height());
    let mut out = vec![0u8; ImageBuf::byte_len(w, h, format)?];
    let row_len = w as usize * format.bytes_per_pixel();
    let pixels = u64::from(w) * u64::from(h);
    let threads = par::plan_threads(0, pixels, h as usize);
    par::for_each_band(&mut out, row_len, threads, |first, band| {
        for (r, dst) in band.chunks_exact_mut(row_len).enumerate() {
            convert_row(from, to, image.row((first + r) as u32), dst);
        }
        Ok(())
    })?;
    ImageBuf::new(w, h, format, out)
}

/// Convert `image` to 8-bit sRGB, the format every encoder and texture path
/// accepts. 16-bit values are rounded; float values are encoded with the
/// sRGB curve after a tone clamp to `0..=1`.
pub fn to_rgba8(image: &ImageBuf) -> Result<ImageBuf> {
    convert(image, PixelFormat::Rgba8)
}

/// The three pixel layouts this crate computes with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Layout {
    U8,
    U16,
    F32,
}

impl Layout {
    /// The layout of `format`, or [`Error::Unsupported`] for a format added
    /// to the core after this crate was written.
    pub(crate) fn of(format: PixelFormat) -> Result<Layout> {
        match format {
            PixelFormat::Rgba8 => Ok(Layout::U8),
            PixelFormat::Rgba16 => Ok(Layout::U16),
            PixelFormat::RgbaF32 => Ok(Layout::F32),
            other => Err(Error::Unsupported(format!("pixel format {other:?}"))),
        }
    }

    /// The pixel format with this layout.
    pub(crate) fn format(self) -> PixelFormat {
        match self {
            Layout::U8 => PixelFormat::Rgba8,
            Layout::U16 => PixelFormat::Rgba16,
            Layout::F32 => PixelFormat::RgbaF32,
        }
    }
}

fn decode(v: f64) -> f64 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn encode(l: f64) -> f64 {
    if l <= 0.003_130_8 {
        l * 12.92
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// Clamp to `0..=1`, mapping NaN to 0.
#[inline]
fn unit(x: f32) -> f32 {
    if x.is_nan() {
        0.0
    } else {
        x.clamp(0.0, 1.0)
    }
}

/// 8-bit sRGB code to linear light.
fn decode8_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| decode(i as f64 / 255.0) as f32))
}

/// 16-bit sRGB code to linear light.
fn decode16_table() -> &'static [f32] {
    static TABLE: OnceLock<Box<[f32]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=u16::MAX)
            .map(|i| decode(f64::from(i) / 65535.0) as f32)
            .collect()
    })
}

/// Number of steps in the linear-to-8-bit table. Fine enough that every
/// 8-bit code round-trips exactly, even in the steep dark end of the curve.
const ENCODE8_STEPS: f32 = 65535.0;

/// Linear light, quantized to `ENCODE8_STEPS`, to 8-bit sRGB code.
fn encode8_table() -> &'static [u8] {
    static TABLE: OnceLock<Box<[u8]>> = OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=65535u32)
            .map(|i| (encode(f64::from(i) / 65535.0) * 255.0 + 0.5) as u8)
            .collect()
    })
}

#[inline]
fn encode8(table: &[u8], linear: f32) -> u8 {
    table[(unit(linear) * ENCODE8_STEPS + 0.5) as usize]
}

#[inline]
fn encode16(linear: f32) -> u16 {
    (encode(f64::from(unit(linear))) * 65535.0 + 0.5) as u16
}

#[inline]
fn u16_at(p: &[u8], i: usize) -> u16 {
    u16::from_ne_bytes([p[2 * i], p[2 * i + 1]])
}

#[inline]
fn f32_at(p: &[u8], i: usize) -> f32 {
    f32::from_ne_bytes([p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]])
}

/// A float color channel read from a file: non-finite values become 0.
#[inline]
fn finite(x: f32) -> f32 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

fn put_u16(dst: &mut [u8], i: usize, v: u16) {
    dst[2 * i..2 * i + 2].copy_from_slice(&v.to_ne_bytes());
}

fn put_f32(dst: &mut [u8], i: usize, v: f32) {
    dst[4 * i..4 * i + 4].copy_from_slice(&v.to_ne_bytes());
}

/// Convert one row of straight-alpha pixels to premultiplied linear-light
/// RGBA floats, 4 per pixel. Float input keeps values above 1 (HDR) but
/// drops NaN and infinity, which would otherwise spread through the filter.
pub(crate) fn row_to_premul(layout: Layout, src: &[u8], out: &mut [f32]) {
    match layout {
        Layout::U8 => {
            let lut = decode8_table();
            for (p, o) in src.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
                let a = f32::from(p[3]) * (1.0 / 255.0);
                o[0] = lut[usize::from(p[0])] * a;
                o[1] = lut[usize::from(p[1])] * a;
                o[2] = lut[usize::from(p[2])] * a;
                o[3] = a;
            }
        }
        Layout::U16 => {
            let lut = decode16_table();
            for (p, o) in src.chunks_exact(8).zip(out.chunks_exact_mut(4)) {
                let a = f32::from(u16_at(p, 3)) * (1.0 / 65535.0);
                o[0] = lut[usize::from(u16_at(p, 0))] * a;
                o[1] = lut[usize::from(u16_at(p, 1))] * a;
                o[2] = lut[usize::from(u16_at(p, 2))] * a;
                o[3] = a;
            }
        }
        Layout::F32 => {
            for (p, o) in src.chunks_exact(16).zip(out.chunks_exact_mut(4)) {
                let a = unit(f32_at(p, 3));
                o[0] = finite(f32_at(p, 0)) * a;
                o[1] = finite(f32_at(p, 1)) * a;
                o[2] = finite(f32_at(p, 2)) * a;
                o[3] = a;
            }
        }
    }
}

/// Below this alpha a float pixel is treated as fully transparent: the
/// color would be noise divided by almost zero.
const MIN_FLOAT_ALPHA: f32 = 1e-6;

/// Convert premultiplied linear-light RGBA floats back to straight-alpha
/// pixels of `layout`. Color is divided by the exact, unclamped alpha, so
/// partly transparent edges keep their true hue and filter overshoot of
/// alpha above 1 does not brighten the color. Alpha is then clamped to
/// `0..=1` for storage, and pixels whose stored alpha is zero get zero color.
pub(crate) fn premul_to_row(layout: Layout, acc: &[f32], out: &mut [u8]) {
    match layout {
        Layout::U8 => {
            let table = encode8_table();
            for (a, o) in acc.chunks_exact(4).zip(out.chunks_exact_mut(4)) {
                let alpha = unit(a[3]);
                let stored = (alpha * 255.0 + 0.5) as u8;
                if stored == 0 {
                    o.fill(0);
                    continue;
                }
                // stored > 0 implies a[3] > 0.
                let inv = 1.0 / a[3];
                o[0] = encode8(table, a[0] * inv);
                o[1] = encode8(table, a[1] * inv);
                o[2] = encode8(table, a[2] * inv);
                o[3] = stored;
            }
        }
        Layout::U16 => {
            for (a, o) in acc.chunks_exact(4).zip(out.chunks_exact_mut(8)) {
                let alpha = unit(a[3]);
                let stored = (alpha * 65535.0 + 0.5) as u16;
                if stored == 0 {
                    o.fill(0);
                    continue;
                }
                let inv = 1.0 / a[3];
                put_u16(o, 0, encode16(a[0] * inv));
                put_u16(o, 1, encode16(a[1] * inv));
                put_u16(o, 2, encode16(a[2] * inv));
                put_u16(o, 3, stored);
            }
        }
        Layout::F32 => {
            for (a, o) in acc.chunks_exact(4).zip(out.chunks_exact_mut(16)) {
                let alpha = unit(a[3]);
                if alpha < MIN_FLOAT_ALPHA {
                    o.fill(0);
                    continue;
                }
                let inv = 1.0 / a[3];
                for (c, &premul) in a[..3].iter().enumerate() {
                    // Negative values only come from filter ringing.
                    let v = premul * inv;
                    put_f32(o, c, if v > 0.0 { finite(v) } else { 0.0 });
                }
                put_f32(o, 3, alpha);
            }
        }
    }
}

/// Convert one row of straight-alpha pixels between layouts.
fn convert_row(from: Layout, to: Layout, src: &[u8], dst: &mut [u8]) {
    match (from, to) {
        (Layout::U8, Layout::U16) => {
            for (s, d) in src.iter().zip(dst.chunks_exact_mut(2)) {
                d.copy_from_slice(&(u16::from(*s) * 257).to_ne_bytes());
            }
        }
        (Layout::U16, Layout::U8) => {
            for (s, d) in src.chunks_exact(2).zip(dst.iter_mut()) {
                let v = u32::from(u16::from_ne_bytes([s[0], s[1]]));
                *d = ((v * 255 + 32_767) / 65_535) as u8;
            }
        }
        (Layout::U8, Layout::F32) => {
            let lut = decode8_table();
            for (p, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(16)) {
                for c in 0..3 {
                    put_f32(d, c, lut[usize::from(p[c])]);
                }
                put_f32(d, 3, f32::from(p[3]) / 255.0);
            }
        }
        (Layout::U16, Layout::F32) => {
            let lut = decode16_table();
            for (p, d) in src.chunks_exact(8).zip(dst.chunks_exact_mut(16)) {
                for c in 0..3 {
                    put_f32(d, c, lut[usize::from(u16_at(p, c))]);
                }
                put_f32(d, 3, f32::from(u16_at(p, 3)) / 65535.0);
            }
        }
        (Layout::F32, Layout::U8) => {
            let table = encode8_table();
            for (p, d) in src.chunks_exact(16).zip(dst.chunks_exact_mut(4)) {
                for (c, v) in d[..3].iter_mut().enumerate() {
                    *v = encode8(table, f32_at(p, c));
                }
                d[3] = (unit(f32_at(p, 3)) * 255.0 + 0.5) as u8;
            }
        }
        (Layout::F32, Layout::U16) => {
            for (p, d) in src.chunks_exact(16).zip(dst.chunks_exact_mut(8)) {
                for c in 0..3 {
                    put_u16(d, c, encode16(f32_at(p, c)));
                }
                put_u16(d, 3, (unit(f32_at(p, 3)) * 65535.0 + 0.5) as u16);
            }
        }
        _ => dst.copy_from_slice(src),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f32_image(values: &[[f32; 4]]) -> ImageBuf {
        let data = values
            .iter()
            .flat_map(|p| p.iter().flat_map(|v| v.to_ne_bytes()))
            .collect();
        ImageBuf::new(values.len() as u32, 1, PixelFormat::RgbaF32, data).unwrap()
    }

    #[test]
    fn transfer_curve_matches_the_standard() {
        assert!((srgb_to_linear(0.5) - 0.214_041).abs() < 1e-6);
        assert!((linear_to_srgb(0.5) - 0.735_357).abs() < 1e-6);
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-6);
        for i in 0..=100 {
            let v = i as f32 / 100.0;
            assert!((linear_to_srgb(srgb_to_linear(v)) - v).abs() < 1e-5);
        }
    }

    #[test]
    fn every_8bit_code_round_trips_through_linear() {
        let table = encode8_table();
        for (code, &linear) in decode8_table().iter().enumerate() {
            assert_eq!(usize::from(encode8(table, linear)), code);
        }
    }

    #[test]
    fn every_16bit_code_round_trips_through_linear() {
        for (code, &linear) in decode16_table().iter().enumerate() {
            assert_eq!(usize::from(encode16(linear)), code);
        }
    }

    #[test]
    fn premultiplied_round_trip_keeps_pixels() {
        let src: Vec<u8> = vec![255, 0, 0, 255, 10, 200, 30, 128, 77, 77, 77, 1, 9, 9, 9, 0];
        let mut floats = vec![0.0f32; 16];
        row_to_premul(Layout::U8, &src, &mut floats);
        let mut back = vec![0u8; 16];
        premul_to_row(Layout::U8, &floats, &mut back);
        // Fully transparent pixels lose their (invisible) color.
        assert_eq!(&back[..12], &src[..12]);
        assert_eq!(&back[12..], &[0, 0, 0, 0]);
    }

    #[test]
    fn float_to_rgba8_clamps_tone() {
        let img = f32_image(&[
            [2.0, -1.0, f32::NAN, 1.0],
            [0.2, 1.0, 0.0, 0.5],
            [f32::INFINITY, 0.0, 0.0, 7.0],
        ]);
        let out = to_rgba8(&img).unwrap();
        assert_eq!(out.format(), PixelFormat::Rgba8);
        // Linear 0.2 is sRGB 0.4845, code 124.
        assert_eq!(
            out.data(),
            &[255, 0, 0, 255, 124, 255, 0, 128, 255, 0, 0, 255]
        );
    }

    #[test]
    fn integer_formats_convert_exactly() {
        let data: Vec<u8> = (0..=255).collect();
        let img = ImageBuf::new(64, 1, PixelFormat::Rgba8, data.clone()).unwrap();
        let wide = convert(&img, PixelFormat::Rgba16).unwrap();
        assert_eq!(u16::from_ne_bytes([wide.data()[2], wide.data()[3]]), 257);
        assert_eq!(
            convert(&wide, PixelFormat::Rgba8).unwrap().data(),
            &data[..]
        );
        let float = convert(&img, PixelFormat::RgbaF32).unwrap();
        assert_eq!(to_rgba8(&float).unwrap().data(), &data[..]);
        let float16 = convert(&wide, PixelFormat::RgbaF32).unwrap();
        let back16 = convert(&float16, PixelFormat::Rgba16).unwrap();
        assert_eq!(back16.data(), wide.data());
        // Same format is a shared clone.
        assert!(std::ptr::eq(
            convert(&img, PixelFormat::Rgba8).unwrap().data(),
            img.data()
        ));
    }

    #[test]
    fn rounding_16_to_8_is_nearest() {
        let px = |v: u16| -> u8 {
            let data = [v, v, v, v].iter().flat_map(|c| c.to_ne_bytes()).collect();
            let img = ImageBuf::new(1, 1, PixelFormat::Rgba16, data).unwrap();
            to_rgba8(&img).unwrap().data()[0]
        };
        assert_eq!(px(0), 0);
        assert_eq!(px(128), 0);
        assert_eq!(px(129), 1);
        assert_eq!(px(65535), 255);
        assert_eq!(px(65535 - 128), 255);
        assert_eq!(px(65535 - 129), 254);
    }
}

//! Helpers shared by every provider: panic guards, limit checks, pixel
//! conversions and buffer builders.

use pandite_core::image::MAX_BUFFER_BYTES;
use pandite_core::{Animation, Content, Error, Frame, ImageBuf, LoopCount, PixelFormat, Result};
use std::borrow::Cow;
use std::fmt::Display;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Total bytes all composed frames of one animation may use. Above this the
/// decoder returns the first frame as a still instead of failing.
pub(crate) const ANIMATION_BUDGET: usize = 1 << 30;

/// Byte budget handed to third-party decoders that accept a memory limit.
pub(crate) fn decoder_memory_limit() -> usize {
    usize::try_from(MAX_BUFFER_BYTES).unwrap_or(usize::MAX)
}

/// An `Error::Corrupt` that names the format.
pub(crate) fn corrupt(format: &str, err: impl Display) -> Error {
    Error::Corrupt(format!("{format}: {err}"))
}

/// An `Error::Unsupported` that names the format.
pub(crate) fn unsupported(format: &str, what: impl Display) -> Error {
    Error::Unsupported(format!("{format}: {what}"))
}

/// Run decoder code and turn a panic into `Error::Corrupt`.
///
/// Third-party decoders are not trusted to be panic free on hostile input.
/// This only works when the binary unwinds on panic.
pub(crate) fn guard<T>(format: &str, work: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(result) => result,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_owned())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "unknown panic".to_owned());
            Err(Error::Corrupt(format!("{format}: decoder failed: {msg}")))
        }
    }
}

/// Fail with `Error::TooLarge` when an image of this shape cannot be held,
/// or `Error::Corrupt` when a side is zero (a file never legitimately
/// declares an empty image). Returns the byte length of the buffer.
pub(crate) fn check_size(width: u32, height: u32, format: PixelFormat) -> Result<usize> {
    ImageBuf::byte_len(width, height, format).map_err(|e| match e {
        Error::Invalid(msg) => Error::Corrupt(msg),
        other => other,
    })
}

/// A zeroed byte buffer that reports allocation failure instead of aborting.
pub(crate) fn alloc(len: usize, width: u32, height: u32) -> Result<Vec<u8>> {
    // Probe that the memory is there (refusing instead of aborting), then
    // take zeroed pages from the system: no memset over fresh pages, which
    // for a 60 megapixel image would touch every page on one thread before
    // the parallel decode writes them.
    let mut probe: Vec<u8> = Vec::new();
    probe
        .try_reserve_exact(len)
        .map_err(|_| Error::TooLarge { width, height })?;
    drop(probe);
    Ok(vec![0u8; len])
}

/// A zeroed RGBA8 buffer for an image of this size.
pub(crate) fn alloc_rgba8(width: u32, height: u32) -> Result<Vec<u8>> {
    let len = check_size(width, height, PixelFormat::Rgba8)?;
    alloc(len, width, height)
}

/// Buffers at least this large are paged in by [`alloc_touched`] on the
/// shared helper threads.
const TOUCH_MIN: usize = 16 << 20;

/// A zeroed buffer like [`alloc`], with its pages made present on several
/// threads at once. For buffers a single-threaded decoder fills: page
/// faults resolve in parallel (about 5 ms instead of 25 ms for 240 MB)
/// instead of one at a time inside the decoder.
pub(crate) fn alloc_touched(len: usize, width: u32, height: u32) -> Result<Vec<u8>> {
    let mut v = alloc(len, width, height)?;
    if len >= TOUCH_MIN {
        const PAGE: usize = 4096;
        let parts = crate::pool::threads().clamp(1, 16);
        let chunk = len.div_ceil(parts).next_multiple_of(PAGE);
        let work: Vec<&mut [u8]> = v.chunks_mut(chunk).collect();
        // Writing a zero to a zeroed page changes nothing but makes it
        // present. Cancellation does not apply: this is one short step.
        crate::pool::run(work, &pandite_core::CancelToken::new(), |part| {
            for page in part.chunks_mut(PAGE) {
                page[0] = 0;
            }
            Ok(())
        })?;
    }
    Ok(v)
}

/// Read a big-endian u16 at `pos`.
pub(crate) fn be16(data: &[u8], pos: usize) -> Option<u16> {
    let b = data.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

/// Read a big-endian u32 at `pos`.
pub(crate) fn be32(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Read a little-endian u16 at `pos`.
pub(crate) fn le16(data: &[u8], pos: usize) -> Option<u16> {
    let b = data.get(pos..pos.checked_add(2)?)?;
    Some(u16::from_le_bytes([b[0], b[1]]))
}

/// Read a little-endian u32 at `pos`.
pub(crate) fn le32(data: &[u8], pos: usize) -> Option<u32> {
    let b = data.get(pos..pos.checked_add(4)?)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Major and compatible brands of a leading ISO base media `ftyp` box.
pub(crate) fn ftyp_brands(header: &[u8]) -> impl Iterator<Item = [u8; 4]> + '_ {
    let ok = header.get(4..8) == Some(b"ftyp");
    let size = if ok {
        (be32(header, 0).unwrap_or(0) as usize).min(header.len())
    } else {
        0
    };
    let major = header
        .get(8..12)
        .filter(|_| ok)
        .map(|b| [b[0], b[1], b[2], b[3]]);
    let compat = header
        .get(16..size.max(16))
        .unwrap_or(&[])
        .chunks_exact(4)
        .map(|b| [b[0], b[1], b[2], b[3]]);
    major.into_iter().chain(compat)
}

/// Expand 8-bit samples with `channels` (1 to 4: gray, gray+alpha, RGB,
/// RGBA) to RGBA8.
pub(crate) fn expand_to_rgba8(src: &[u8], channels: usize, out: &mut [u8]) {
    match channels {
        1 => {
            for (o, &g) in out.chunks_exact_mut(4).zip(src) {
                o.copy_from_slice(&[g, g, g, 255]);
            }
        }
        2 => {
            for (o, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(2)) {
                o.copy_from_slice(&[s[0], s[0], s[0], s[1]]);
            }
        }
        3 => {
            for (o, s) in out.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                o.copy_from_slice(&[s[0], s[1], s[2], 255]);
            }
        }
        _ => {
            let n = out.len().min(src.len());
            out[..n].copy_from_slice(&src[..n]);
        }
    }
}

/// Expand 16-bit samples (given as u16 values) with `channels` (1 to 4) to
/// RGBA16 native-endian bytes.
pub(crate) fn expand16_to_rgba16(
    samples: impl Iterator<Item = u16>,
    channels: usize,
    out: &mut [u8],
) {
    let mut px = [0u16; 4];
    let mut i = 0;
    let mut o = 0;
    for v in samples {
        px[i] = v;
        i += 1;
        if i == channels {
            i = 0;
            let rgba = match channels {
                1 => [px[0], px[0], px[0], u16::MAX],
                2 => [px[0], px[0], px[0], px[1]],
                3 => [px[0], px[1], px[2], u16::MAX],
                _ => px,
            };
            if o + 8 > out.len() {
                return;
            }
            for (k, c) in rgba.iter().enumerate() {
                out[o + 2 * k..o + 2 * k + 2].copy_from_slice(&c.to_ne_bytes());
            }
            o += 8;
        }
    }
}

/// Convert true CMYK (0 means no ink) to RGB with the naive formula.
pub(crate) fn cmyk_to_rgb(c: u8, m: u8, y: u8, k: u8) -> [u8; 3] {
    let kk = 255 - u32::from(k);
    let f = |v: u8| ((255 - u32::from(v)) * kk + 127) / 255;
    [f(c) as u8, f(m) as u8, f(y) as u8]
}

/// Scale a value with `bits` significant bits to 8 bits.
pub(crate) fn scale_to_u8(v: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    ((u64::from(v.min(max)) * 255 + u64::from(max) / 2) / u64::from(max)) as u8
}

/// Scale a value with maximum `max` to 16 bits.
pub(crate) fn scale_to_u16(v: u32, max: u32) -> u16 {
    if max == 0 {
        return 0;
    }
    ((u64::from(v.min(max)) * 65535 + u64::from(max) / 2) / u64::from(max)) as u16
}

/// Decode the sRGB transfer function: encoded 0..1 to linear 0..1.
pub(crate) fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Encode the sRGB transfer function: linear 0..1 to encoded 0..1. Values
/// outside 0..1 are clamped.
pub(crate) fn linear_to_srgb(v: f32) -> f32 {
    let v = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// Build an RGBA8 image from a full-size byte vector.
pub(crate) fn rgba8_image(width: u32, height: u32, data: Vec<u8>) -> Result<ImageBuf> {
    ImageBuf::new(width, height, PixelFormat::Rgba8, data)
}

/// Build an RGBA16 image from u16 samples (4 per pixel).
pub(crate) fn rgba16_image(width: u32, height: u32, samples: &[u16]) -> Result<ImageBuf> {
    let len = check_size(width, height, PixelFormat::Rgba16)?;
    let mut data = alloc(len, width, height)?;
    for (o, v) in data.chunks_exact_mut(2).zip(samples) {
        o.copy_from_slice(&v.to_ne_bytes());
    }
    ImageBuf::new(width, height, PixelFormat::Rgba16, data)
}

/// Build an RGBA f32 image from linear samples (4 per pixel).
pub(crate) fn rgbaf32_image(width: u32, height: u32, samples: &[f32]) -> Result<ImageBuf> {
    let len = check_size(width, height, PixelFormat::RgbaF32)?;
    let mut data = alloc(len, width, height)?;
    for (o, v) in data.chunks_exact_mut(4).zip(samples) {
        o.copy_from_slice(&v.to_ne_bytes());
    }
    ImageBuf::new(width, height, PixelFormat::RgbaF32, data)
}

/// Read the RGBA samples of pixel `i` of an image as 8-bit sRGB values.
fn pixel_rgba8(data: &[u8], format: PixelFormat, i: usize) -> [u8; 4] {
    match format {
        PixelFormat::Rgba16 => {
            let p = &data[i * 8..i * 8 + 8];
            let c = |k: usize| {
                let v = u16::from_ne_bytes([p[2 * k], p[2 * k + 1]]);
                ((u32::from(v) * 255 + 32767) / 65535) as u8
            };
            [c(0), c(1), c(2), c(3)]
        }
        PixelFormat::RgbaF32 => {
            let p = &data[i * 16..i * 16 + 16];
            let f =
                |k: usize| f32::from_ne_bytes([p[4 * k], p[4 * k + 1], p[4 * k + 2], p[4 * k + 3]]);
            let enc = |v: f32| (linear_to_srgb(v) * 255.0 + 0.5) as u8;
            let a = f(3);
            let a = if a.is_nan() { 0.0 } else { a.clamp(0.0, 1.0) };
            [enc(f(0)), enc(f(1)), enc(f(2)), (a * 255.0 + 0.5) as u8]
        }
        _ => {
            let p = &data[i * 4..i * 4 + 4];
            [p[0], p[1], p[2], p[3]]
        }
    }
}

/// The image as RGBA8 bytes. Borrowed when it already is RGBA8. 16-bit
/// values are rounded, linear float values are clamped to 0..1 and encoded
/// with the sRGB transfer function.
pub(crate) fn to_rgba8(image: &ImageBuf) -> Cow<'_, [u8]> {
    match image.format() {
        PixelFormat::Rgba8 => Cow::Borrowed(image.data()),
        format => {
            let n = image.width() as usize * image.height() as usize;
            let mut out = Vec::with_capacity(n * 4);
            for i in 0..n {
                out.extend_from_slice(&pixel_rgba8(image.data(), format, i));
            }
            Cow::Owned(out)
        }
    }
}

/// The image as RGBA16 samples (sRGB transfer) for 16-bit encoders.
pub(crate) fn to_rgba16(image: &ImageBuf) -> Vec<u16> {
    let n = image.width() as usize * image.height() as usize;
    let data = image.data();
    let mut out = Vec::with_capacity(n * 4);
    match image.format() {
        PixelFormat::Rgba16 => {
            for c in data.chunks_exact(2) {
                out.push(u16::from_ne_bytes([c[0], c[1]]));
            }
        }
        PixelFormat::RgbaF32 => {
            for (k, c) in data.chunks_exact(4).enumerate() {
                let v = f32::from_ne_bytes([c[0], c[1], c[2], c[3]]);
                let enc = if k % 4 == 3 {
                    if v.is_nan() {
                        0.0
                    } else {
                        v.clamp(0.0, 1.0)
                    }
                } else {
                    linear_to_srgb(v)
                };
                out.push((enc * 65535.0 + 0.5) as u16);
            }
        }
        _ => {
            for &b in data {
                out.push(u16::from(b) * 257);
            }
        }
    }
    out
}

/// The scale and output size that fit `(width, height)` inside `target`
/// while keeping the aspect ratio. Never returns a zero side.
pub(crate) fn fit_into(width: f64, height: f64, target: (u32, u32)) -> (u32, u32) {
    let (tw, th) = (f64::from(target.0.max(1)), f64::from(target.1.max(1)));
    if width <= 0.0 || height <= 0.0 {
        return (target.0.max(1), target.1.max(1));
    }
    let s = (tw / width).min(th / height);
    let w = (width * s).round().max(1.0);
    let h = (height * s).round().max(1.0);
    (w as u32, h as u32)
}

/// Collects composed animation frames within [`ANIMATION_BUDGET`].
pub(crate) struct FrameCollector {
    frames: Vec<Frame>,
    used: usize,
    over_budget: bool,
}

impl FrameCollector {
    /// An empty collector.
    pub(crate) fn new() -> Self {
        Self {
            frames: Vec::new(),
            used: 0,
            over_budget: false,
        }
    }

    /// Add one composed frame. Returns false when the budget is exhausted.
    pub(crate) fn push(&mut self, image: ImageBuf, delay: std::time::Duration) -> bool {
        let size = image.byte_size();
        if self.frames.is_empty() || self.used + size <= ANIMATION_BUDGET {
            self.used += size;
            self.frames.push(Frame { image, delay });
            true
        } else {
            self.over_budget = true;
            false
        }
    }

    /// Number of frames held.
    pub(crate) fn len(&self) -> usize {
        self.frames.len()
    }

    /// Finish: an animation when there are two or more frames and the budget
    /// held, otherwise the first frame as a still.
    pub(crate) fn finish(self, loop_count: LoopCount) -> Result<Content> {
        if self.frames.len() >= 2 && !self.over_budget {
            Ok(Content::Animation(Animation::new(self.frames, loop_count)?))
        } else {
            let first = self
                .frames
                .into_iter()
                .next()
                .ok_or_else(|| Error::Corrupt("animation has no frames".into()))?;
            Ok(Content::Still(first.image))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_turns_panic_into_corrupt() {
        let r: Result<()> = guard("test", || panic!("boom"));
        match r {
            Err(Error::Corrupt(msg)) => assert!(msg.contains("boom")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn conversions_round_trip() {
        assert_eq!(cmyk_to_rgb(0, 0, 0, 0), [255, 255, 255]);
        assert_eq!(cmyk_to_rgb(255, 0, 0, 0), [0, 255, 255]);
        assert_eq!(cmyk_to_rgb(0, 0, 0, 255), [0, 0, 0]);
        assert_eq!(scale_to_u8(31, 31), 255);
        assert_eq!(scale_to_u8(16, 31), 132);
        assert_eq!(scale_to_u16(4095, 4095), 65535);
        for v in [0u8, 1, 50, 128, 200, 255] {
            let lin = srgb_to_linear(f32::from(v) / 255.0);
            let back = (linear_to_srgb(lin) * 255.0 + 0.5) as u8;
            assert_eq!(back, v);
        }
        assert_eq!(fit_into(4000.0, 3000.0, (400, 400)), (400, 300));
        assert_eq!(fit_into(10.0, 20.0, (100, 100)), (50, 100));
    }

    #[test]
    fn to_rgba8_converts_wide_formats() {
        let img = rgba16_image(1, 1, &[65535, 32896, 0, 65535]).unwrap();
        assert_eq!(&*to_rgba8(&img), &[255, 128, 0, 255]);
        let img = rgbaf32_image(1, 1, &[1.0, 0.214_041_14, 0.0, 0.5]).unwrap();
        assert_eq!(&*to_rgba8(&img), &[255, 128, 0, 128]);
    }
}

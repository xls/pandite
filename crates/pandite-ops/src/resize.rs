//! Resizing: target size rules, the separable resampler and thumbnails.
//!
//! The resampler works in premultiplied linear light. 8-bit and 16-bit sRGB
//! pixels are decoded through lookup tables, multiplied by alpha, filtered as
//! 32-bit floats and encoded back; `RgbaF32` pixels are already linear and
//! are only premultiplied. Filtering in linear light keeps the brightness of
//! fine detail (a black and white checkerboard shrinks to sRGB 188, not 128),
//! and premultiplied alpha keeps the color of fully transparent pixels out of
//! visible edges. Float images keep values above 1 (high dynamic range);
//! negative values, which only filter ringing can produce, are clamped to 0.
//!
//! The work is split into bands of output rows, one per thread. Each band
//! streams its source rows once: the horizontal pass runs on each source row
//! as it is read and the vertical pass folds the results into output rows,
//! so memory stays a few rows per thread instead of a full intermediate
//! image. Weights are computed once per axis; nothing is allocated per pixel.

use crate::convert::{premul_to_row, row_to_premul, Layout};
use crate::filter::{Filter, Weights};
use crate::par;
use pandite_core::image::{MAX_BUFFER_BYTES, MAX_DIMENSION};
use pandite_core::{CancelToken, Error, ImageBuf, Result};

/// How to choose the size of a resized image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResizeSpec {
    /// Scale both sides by a percentage: `50.0` halves, `200.0` doubles.
    Percent(f64),
    /// Scale both sides by a factor: `0.5` halves, `2.0` doubles.
    Scale(f64),
    /// The largest size that fits inside the box and keeps the aspect ratio.
    /// One side touches the box. Images that already fit stay as they are
    /// unless `enlarge` is set.
    Fit {
        /// Box width in pixels.
        width: u32,
        /// Box height in pixels.
        height: u32,
        /// Scale images smaller than the box up to touch it.
        enlarge: bool,
    },
    /// Exactly this size. The aspect ratio may change.
    Exact {
        /// Target width in pixels.
        width: u32,
        /// Target height in pixels.
        height: u32,
    },
}

impl ResizeSpec {
    /// The target size for a `width` x `height` source.
    ///
    /// Sides are rounded to the nearest pixel and never drop below 1, so a
    /// tiny percentage of a small image gives a 1-pixel side. Fails with
    /// [`Error::Invalid`] for a zero, negative or non-finite percentage or
    /// factor, a zero box or exact side, or an empty source, and with
    /// [`Error::TooLarge`] when the result exceeds the core image limits.
    pub fn resolve(&self, width: u32, height: u32) -> Result<(u32, u32)> {
        if width == 0 || height == 0 {
            return Err(Error::Invalid(format!(
                "source image {width} x {height} is empty"
            )));
        }
        let (sw, sh) = (f64::from(width), f64::from(height));
        match *self {
            ResizeSpec::Percent(p) => scaled(sw, sh, p / 100.0, "percentage", p),
            ResizeSpec::Scale(s) => scaled(sw, sh, s, "scale factor", s),
            ResizeSpec::Fit {
                width: bw,
                height: bh,
                enlarge,
            } => {
                if bw == 0 || bh == 0 {
                    return Err(Error::Invalid(format!("fit box {bw} x {bh} is empty")));
                }
                let (bw, bh) = (f64::from(bw), f64::from(bh));
                let (sx, sy) = (bw / sw, bh / sh);
                if !enlarge && sx >= 1.0 && sy >= 1.0 {
                    return Ok((width, height));
                }
                // The limiting side matches the box exactly; the other side
                // is rounded and kept inside the box.
                if sx <= sy {
                    checked(bw, (sh * sx).round().clamp(1.0, bh))
                } else {
                    checked((sw * sy).round().clamp(1.0, bw), bh)
                }
            }
            ResizeSpec::Exact {
                width: w,
                height: h,
            } => {
                if w == 0 || h == 0 {
                    return Err(Error::Invalid(format!("target size {w} x {h} is empty")));
                }
                checked(f64::from(w), f64::from(h))
            }
        }
    }
}

fn scaled(sw: f64, sh: f64, factor: f64, what: &str, given: f64) -> Result<(u32, u32)> {
    if !(factor.is_finite() && factor > 0.0) {
        return Err(Error::Invalid(format!(
            "{what} must be a positive number, got {given}"
        )));
    }
    checked(
        (sw * factor).round().max(1.0),
        (sh * factor).round().max(1.0),
    )
}

/// Enforce the core limits on a computed size. Byte size is checked for the
/// smallest format (4 bytes per pixel); [`resize()`] re-checks with the real
/// format.
fn checked(w: f64, h: f64) -> Result<(u32, u32)> {
    let limit = f64::from(MAX_DIMENSION);
    if w > limit || h > limit || w * h * 4.0 > MAX_BUFFER_BYTES as f64 {
        // Float to int casts saturate, so huge values report as u32::MAX.
        return Err(Error::TooLarge {
            width: w as u32,
            height: h as u32,
        });
    }
    Ok((w as u32, h as u32))
}

/// Settings for [`resize_with`].
#[derive(Clone, Debug, Default)]
pub struct ResizeOptions {
    /// The resampling filter. Defaults to [`Filter::Lanczos3`].
    pub filter: Filter,
    /// Most threads to use; 0 uses every hardware thread. Small images always
    /// run on the calling thread.
    pub threads: usize,
    /// Checked once per output row; cancelling returns [`Error::Cancelled`].
    pub cancel: Option<CancelToken>,
}

/// Resize `image` to exactly `width` x `height` with `filter`, using every
/// hardware thread for large images. The result has the input's pixel
/// format. See [`resize_with`].
pub fn resize(image: &ImageBuf, width: u32, height: u32, filter: Filter) -> Result<ImageBuf> {
    resize_with(
        image,
        width,
        height,
        &ResizeOptions {
            filter,
            ..ResizeOptions::default()
        },
    )
}

/// Resize `image` to the size `spec` picks for it.
pub fn resize_to_spec(image: &ImageBuf, spec: &ResizeSpec, filter: Filter) -> Result<ImageBuf> {
    let (width, height) = spec.resolve(image.width(), image.height())?;
    resize(image, width, height, filter)
}

/// Resize `image` to exactly `width` x `height`.
///
/// Every filter except [`Filter::Nearest`] filters in premultiplied linear
/// light and widens with the reduction ratio, so downscaling does not alias.
/// [`Filter::Nearest`] copies pixels unchanged. Asking for the current size
/// returns a cheap clone. Fails with [`Error::Invalid`] for a zero side,
/// [`Error::TooLarge`] beyond the core limits and [`Error::Cancelled`] when
/// the options' token is cancelled.
pub fn resize_with(
    image: &ImageBuf,
    width: u32,
    height: u32,
    options: &ResizeOptions,
) -> Result<ImageBuf> {
    if width == 0 || height == 0 {
        return Err(Error::Invalid(format!(
            "resize target {width} x {height} has an empty side"
        )));
    }
    ImageBuf::byte_len(width, height, image.format())?;
    let cancel = options.cancel.as_ref();
    if let Some(token) = cancel {
        token.check()?;
    }
    if (width, height) == (image.width(), image.height()) {
        return Ok(image.clone());
    }
    let pixels =
        u64::from(image.width()) * u64::from(image.height()) + u64::from(width) * u64::from(height);
    let threads = par::plan_threads(options.threads, pixels, height as usize);
    if options.filter == Filter::Nearest {
        return nearest(image, width, height, threads);
    }
    let layout = Layout::of(image.format())?;
    let input = Input::Image { image, layout };
    resample(
        &input,
        width as usize,
        height as usize,
        options.filter,
        layout,
        threads,
        cancel,
    )
}

/// The size [`thumbnail`] produces: the largest size whose longer side is at
/// most `max_side` (at least 1), keeping the aspect ratio and never
/// enlarging.
pub fn thumbnail_size(width: u32, height: u32, max_side: u32) -> (u32, u32) {
    let max_side = max_side.max(1);
    let (width, height) = (width.max(1), height.max(1));
    if width <= max_side && height <= max_side {
        return (width, height);
    }
    let (w, h, m) = (f64::from(width), f64::from(height), f64::from(max_side));
    if width >= height {
        let short = ((h * m / w).round() as u32).clamp(1, max_side);
        (max_side, short)
    } else {
        let short = ((w * m / h).round() as u32).clamp(1, max_side);
        (short, max_side)
    }
}

/// A fast, good-looking reduced copy whose longer side is at most `max_side`
/// pixels (see [`thumbnail_size`]). Images that already fit are returned as
/// a cheap clone. The result keeps the input's pixel format.
///
/// Large reductions first average integer blocks of pixels in linear light
/// (touching each source pixel once), then finish with Lanczos3 from at
/// least twice the target size. This is several times faster than a single
/// wide Lanczos pass and looks the same. The block average rounds the
/// intermediate size up, which can shift the geometry by up to a quarter of
/// an output pixel; thumbnails accept that for speed.
pub fn thumbnail(image: &ImageBuf, max_side: u32) -> ImageBuf {
    let (w, h) = (image.width(), image.height());
    let (tw, th) = thumbnail_size(w, h, max_side);
    if (tw, th) == (w, h) {
        return image.clone();
    }
    let filtered = Layout::of(image.format())
        .and_then(|layout| thumbnail_filtered(image, layout, tw as usize, th as usize));
    // Only a pixel format unknown to this crate gets here; nearest copies
    // whole pixels and works for every format.
    filtered
        .or_else(|_| nearest(image, tw, th, 1))
        .unwrap_or_else(|_| image.clone())
}

fn thumbnail_filtered(image: &ImageBuf, layout: Layout, tw: usize, th: usize) -> Result<ImageBuf> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let ratio = (w as f64 / tw as f64).min(h as f64 / th as f64);
    // Leave at least a 2x reduction for the final filter.
    let factor = (ratio / 2.0).floor() as usize;
    let target_px = (tw * th) as u64;
    if factor < 2 {
        let threads = par::plan_threads(0, (w * h) as u64 + target_px, th);
        let input = Input::Image { image, layout };
        return resample(&input, tw, th, Filter::Lanczos3, layout, threads, None);
    }
    let shrink_threads = par::plan_threads(0, (w * h) as u64, h.div_ceil(factor));
    let (sw, sh, data) = box_shrink(image, layout, factor, shrink_threads)?;
    let threads = par::plan_threads(0, (sw * sh) as u64 + target_px, th);
    let input = Input::Premul {
        width: sw,
        height: sh,
        data: &data,
    };
    resample(&input, tw, th, Filter::Lanczos3, layout, threads, None)
}

/// Average `factor` x `factor` blocks into premultiplied linear floats.
/// Partial blocks at the right and bottom edges average the pixels they have.
fn box_shrink(
    image: &ImageBuf,
    layout: Layout,
    factor: usize,
    threads: usize,
) -> Result<(usize, usize, Vec<f32>)> {
    let (w, h) = (image.width() as usize, image.height() as usize);
    let (sw, sh) = (w.div_ceil(factor), h.div_ceil(factor));
    let row_len = sw * 4;
    let mut out = vec![0.0f32; sh * row_len];
    par::for_each_band(&mut out, row_len, threads, |first, band| {
        let mut scratch = vec![0.0f32; w * 4];
        for (r, acc) in band.chunks_exact_mut(row_len).enumerate() {
            let y0 = (first + r) * factor;
            let y1 = (y0 + factor).min(h);
            for y in y0..y1 {
                row_to_premul(layout, image.row(y as u32), &mut scratch);
                for (o, block) in acc.chunks_exact_mut(4).zip(scratch.chunks(4 * factor)) {
                    for p in block.chunks_exact(4) {
                        o[0] += p[0];
                        o[1] += p[1];
                        o[2] += p[2];
                        o[3] += p[3];
                    }
                }
            }
            let rows = (y1 - y0) as f32;
            for (bx, o) in acc.chunks_exact_mut(4).enumerate() {
                let cols = (((bx + 1) * factor).min(w) - bx * factor) as f32;
                let inv = 1.0 / (rows * cols);
                for v in o {
                    *v *= inv;
                }
            }
        }
        Ok(())
    })?;
    Ok((sw, sh, out))
}

/// Nearest-neighbor resize. Copies whole pixels, so it is exact and works
/// for every pixel format. Output pixel `i` takes source pixel
/// `floor((i + 0.5) * src / dst)`.
fn nearest(image: &ImageBuf, width: u32, height: u32, threads: usize) -> Result<ImageBuf> {
    let format = image.format();
    let bpp = format.bytes_per_pixel();
    let len = ImageBuf::byte_len(width, height, format)?;
    let pick = |i: u64, src: u32, dst: u32| -> usize {
        let (src, dst) = (u64::from(src), u64::from(dst));
        (((2 * i + 1) * src) / (2 * dst)).min(src - 1) as usize
    };
    let offsets: Vec<usize> = (0..u64::from(width))
        .map(|x| pick(x, image.width(), width) * bpp)
        .collect();
    let stride = width as usize * bpp;
    let mut out = vec![0u8; len];
    par::for_each_band(&mut out, stride, threads, |first, band| {
        for (r, row) in band.chunks_exact_mut(stride).enumerate() {
            let sy = pick((first + r) as u64, image.height(), height);
            copy_pixels(bpp, image.row(sy as u32), &offsets, row);
        }
        Ok(())
    })?;
    ImageBuf::new(width, height, format, out)
}

/// Copy pixel `offsets[i]` of `src` to pixel `i` of `dst`. The match lets
/// the compiler build a fixed-size copy for each common pixel size.
fn copy_pixels(bpp: usize, src: &[u8], offsets: &[usize], dst: &mut [u8]) {
    #[inline(always)]
    fn run(bpp: usize, src: &[u8], offsets: &[usize], dst: &mut [u8]) {
        for (d, &o) in dst.chunks_exact_mut(bpp).zip(offsets) {
            d.copy_from_slice(&src[o..o + bpp]);
        }
    }
    match bpp {
        4 => run(4, src, offsets, dst),
        8 => run(8, src, offsets, dst),
        16 => run(16, src, offsets, dst),
        n => run(n, src, offsets, dst),
    }
}

/// Where the resampler reads source rows from.
pub(crate) enum Input<'a> {
    /// A straight-alpha image, converted row by row.
    Image {
        /// The source.
        image: &'a ImageBuf,
        /// Its layout.
        layout: Layout,
    },
    /// Premultiplied linear-light RGBA floats, 4 per pixel.
    Premul {
        /// Width in pixels.
        width: usize,
        /// Height in pixels.
        height: usize,
        /// `width * height * 4` floats, top row first.
        data: &'a [f32],
    },
}

impl Input<'_> {
    fn width(&self) -> usize {
        match self {
            Input::Image { image, .. } => image.width() as usize,
            Input::Premul { width, .. } => *width,
        }
    }

    fn height(&self) -> usize {
        match self {
            Input::Image { image, .. } => image.height() as usize,
            Input::Premul { height, .. } => *height,
        }
    }

    /// A buffer large enough for [`Input::row`] to convert into.
    fn scratch(&self) -> Vec<f32> {
        match self {
            Input::Image { .. } => vec![0.0; self.width() * 4],
            Input::Premul { .. } => Vec::new(),
        }
    }

    /// Source row `y` as premultiplied linear floats.
    fn row<'s>(&'s self, y: usize, scratch: &'s mut [f32]) -> &'s [f32] {
        match self {
            Input::Image { image, layout } => {
                row_to_premul(*layout, image.row(y as u32), scratch);
                scratch
            }
            Input::Premul { width, data, .. } => {
                let n = width * 4;
                &data[y * n..(y + 1) * n]
            }
        }
    }
}

/// Resample `input` to `dst_w` x `dst_h` pixels of `out` layout.
pub(crate) fn resample(
    input: &Input<'_>,
    dst_w: usize,
    dst_h: usize,
    filter: Filter,
    out: Layout,
    threads: usize,
    cancel: Option<&CancelToken>,
) -> Result<ImageBuf> {
    let format = out.format();
    let (w32, h32) = (dst_w as u32, dst_h as u32);
    let mut pixels = vec![0u8; ImageBuf::byte_len(w32, h32, format)?];
    let resampler = Resampler {
        input,
        horizontal: Weights::new(input.width(), dst_w, filter),
        vertical: Weights::new(input.height(), dst_h, filter),
        out,
        dst_w,
        stride: dst_w * format.bytes_per_pixel(),
        cancel,
        // Shrinking rows: push each source row into the few output rows it
        // feeds. Growing or keeping rows: pull a short window of source rows.
        push: dst_h < input.height(),
    };
    par::for_each_band(&mut pixels, resampler.stride, threads, |first, band| {
        resampler.band(first, band)
    })?;
    ImageBuf::new(w32, h32, format, pixels)
}

/// One resize job, shared read-only by the band threads.
struct Resampler<'a> {
    input: &'a Input<'a>,
    horizontal: Weights,
    vertical: Weights,
    out: Layout,
    dst_w: usize,
    stride: usize,
    cancel: Option<&'a CancelToken>,
    push: bool,
}

impl Resampler<'_> {
    fn check(&self) -> Result<()> {
        match self.cancel {
            Some(token) => token.check(),
            None => Ok(()),
        }
    }

    fn band(&self, first: usize, band: &mut [u8]) -> Result<()> {
        if self.push {
            self.band_push(first, band)
        } else {
            self.band_pull(first, band)
        }
    }

    /// Vertical pass for growing (or unchanged) height. A ring holds the
    /// horizontally filtered source rows of the current window; each source
    /// row is filtered once while it stays in the window.
    fn band_pull(&self, first: usize, band: &mut [u8]) -> Result<()> {
        let floats = self.dst_w * 4;
        let cap = self.vertical.max_len();
        let mut ring = vec![0.0f32; cap * floats];
        let mut held = vec![usize::MAX; cap];
        let mut scratch = self.input.scratch();
        let mut acc = vec![0.0f32; floats];
        for (r, out_row) in band.chunks_exact_mut(self.stride).enumerate() {
            self.check()?;
            let (start, weights) = self.vertical.taps(first + r);
            for y in start..start + weights.len() {
                let slot = y % cap;
                if held[slot] != y {
                    let src = self.input.row(y, &mut scratch);
                    self.horizontal
                        .apply(src, &mut ring[slot * floats..(slot + 1) * floats]);
                    held[slot] = y;
                }
            }
            if weights.len() == 1 && weights[0] == 1.0 {
                let slot = start % cap;
                premul_to_row(self.out, &ring[slot * floats..(slot + 1) * floats], out_row);
                continue;
            }
            acc.fill(0.0);
            for (k, &w) in weights.iter().enumerate() {
                if w != 0.0 {
                    let slot = (start + k) % cap;
                    axpy(&mut acc, w, &ring[slot * floats..(slot + 1) * floats]);
                }
            }
            premul_to_row(self.out, &acc, out_row);
        }
        Ok(())
    }

    /// Vertical pass for shrinking height. Source rows stream through once;
    /// each is filtered horizontally and added into every open output row
    /// that reads it. An output row is written as soon as its window ends.
    /// At most `max_open` output rows are open at once, so memory stays a
    /// few rows however large the reduction.
    fn band_push(&self, first: usize, band: &mut [u8]) -> Result<()> {
        let rows = band.len() / self.stride;
        if rows == 0 {
            return Ok(());
        }
        let last = first + rows;
        let floats = self.dst_w * 4;
        let v = &self.vertical;
        let slots = v.max_open();
        let mut open = vec![0.0f32; slots * floats];
        let mut hrow = vec![0.0f32; floats];
        let mut scratch = self.input.scratch();
        // First output row of the band that is not written yet.
        let mut lo = first;
        for y in v.start(first)..v.end(last - 1) {
            self.check()?;
            let src = self.input.row(y, &mut scratch);
            self.horizontal.apply(src, &mut hrow);
            // Windows are monotone, so the open rows are lo..i.
            let mut i = lo;
            while i < last && v.start(i) <= y {
                let slot = i % slots;
                let acc = &mut open[slot * floats..(slot + 1) * floats];
                let (start, weights) = v.taps(i);
                if y == start {
                    acc.fill(0.0);
                }
                let w = weights[y - start];
                if w != 0.0 {
                    axpy(acc, w, &hrow);
                }
                i += 1;
            }
            while lo < last && v.end(lo) <= y + 1 {
                let slot = lo % slots;
                let r = lo - first;
                premul_to_row(
                    self.out,
                    &open[slot * floats..(slot + 1) * floats],
                    &mut band[r * self.stride..(r + 1) * self.stride],
                );
                lo += 1;
            }
        }
        Ok(())
    }
}

/// `acc += w * row`, element by element.
#[inline]
fn axpy(acc: &mut [f32], w: f32, row: &[f32]) {
    for (a, &v) in acc.iter_mut().zip(row) {
        *a += w * v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::PixelFormat;

    /// A deterministic pseudo-random RGBA8 image with varied alpha.
    fn noise(w: u32, h: u32) -> ImageBuf {
        let mut state = 0x2545_f491_u32;
        let data = (0..w * h * 4)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 24) as u8
            })
            .collect();
        ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
    }

    fn run(
        img: &ImageBuf,
        w: usize,
        h: usize,
        filter: Filter,
        threads: usize,
        push: Option<bool>,
    ) -> ImageBuf {
        let layout = Layout::of(img.format()).unwrap();
        let input = Input::Image { image: img, layout };
        let resampler = Resampler {
            input: &input,
            horizontal: Weights::new(img.width() as usize, w, filter),
            vertical: Weights::new(img.height() as usize, h, filter),
            out: layout,
            dst_w: w,
            stride: w * 4,
            cancel: None,
            push: push.unwrap_or(h < img.height() as usize),
        };
        let mut pixels = vec![0u8; w * h * 4];
        par::for_each_band(&mut pixels, resampler.stride, threads, |first, band| {
            resampler.band(first, band)
        })
        .unwrap();
        ImageBuf::new(w as u32, h as u32, PixelFormat::Rgba8, pixels).unwrap()
    }

    #[test]
    fn threads_do_not_change_the_result() {
        let img = noise(97, 83);
        for filter in [Filter::Lanczos3, Filter::Box, Filter::Mitchell] {
            for (w, h) in [(40, 31), (150, 170), (97, 20), (13, 83)] {
                let one = run(&img, w, h, filter, 1, None);
                for threads in [2, 3, 8] {
                    let many = run(&img, w, h, filter, threads, None);
                    assert_eq!(one.data(), many.data(), "{filter:?} {w}x{h} {threads}");
                }
            }
        }
    }

    #[test]
    fn push_and_pull_vertical_passes_agree() {
        let img = noise(50, 90);
        for filter in [
            Filter::Lanczos3,
            Filter::Triangle,
            Filter::Box,
            Filter::CatmullRom,
        ] {
            for h in [1, 7, 30, 89] {
                let pull = run(&img, 33, h, filter, 1, Some(false));
                let push = run(&img, 33, h, filter, 3, Some(true));
                assert_eq!(pull.data(), push.data(), "{filter:?} h={h}");
            }
        }
    }

    #[test]
    fn one_pixel_images() {
        let dot = ImageBuf::solid_rgba8(1, 1, [10, 20, 30, 40]).unwrap();
        for filter in Filter::ALL {
            let big = resize(&dot, 5, 3, filter).unwrap();
            assert!(big.data().chunks_exact(4).all(|p| p == [10, 20, 30, 40]));
            let back = resize(&big, 1, 1, filter).unwrap();
            assert_eq!(back.data(), &[10, 20, 30, 40]);
        }
        let column = noise(1, 9);
        let wide = resize(&column, 4, 9, Filter::Lanczos3).unwrap();
        for y in 0..9 {
            let row = wide.row(y);
            assert!(row.chunks_exact(4).all(|p| p == &row[..4]));
        }
        assert_eq!(resize(&column, 1, 1, Filter::Box).unwrap().width(), 1);
    }

    #[test]
    fn same_size_is_a_clone_and_nearest_is_exact() {
        let img = noise(9, 7);
        let same = resize(&img, 9, 7, Filter::Mitchell).unwrap();
        assert!(std::ptr::eq(same.data(), img.data()));
        let up = resize(&img, 18, 14, Filter::Nearest).unwrap();
        for y in 0..14 {
            for x in 0..18 {
                let o = ((y * 18 + x) * 4) as usize;
                let s = (((y / 2) * 9 + x / 2) * 4) as usize;
                assert_eq!(up.data()[o..o + 4], img.data()[s..s + 4]);
            }
        }
        let down = resize(&up, 9, 7, Filter::Nearest).unwrap();
        assert_eq!(down.data(), img.data());
    }

    #[test]
    fn cancel_stops_resize() {
        let img = noise(64, 64);
        let token = CancelToken::new();
        token.cancel();
        let options = ResizeOptions {
            cancel: Some(token),
            ..ResizeOptions::default()
        };
        assert!(matches!(
            resize_with(&img, 32, 32, &options),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn spec_rounding_and_limits() {
        assert_eq!(ResizeSpec::Percent(50.0).resolve(101, 3).unwrap(), (51, 2));
        assert_eq!(
            ResizeSpec::Percent(0.001).resolve(100, 100).unwrap(),
            (1, 1)
        );
        assert_eq!(ResizeSpec::Scale(1.5).resolve(3, 5).unwrap(), (5, 8));
        let fit = ResizeSpec::Fit {
            width: 100,
            height: 100,
            enlarge: false,
        };
        assert_eq!(fit.resolve(333, 100).unwrap(), (100, 30));
        assert_eq!(fit.resolve(100, 333).unwrap(), (30, 100));
        assert_eq!(fit.resolve(100_000 / 2, 1).unwrap(), (100, 1));
        assert!(matches!(
            ResizeSpec::Scale(2.0).resolve(40_000, 10),
            Err(Error::TooLarge { width: 80_000, .. })
        ));
        assert!(matches!(
            ResizeSpec::Exact {
                width: 65_535,
                height: 65_535
            }
            .resolve(1, 1),
            Err(Error::TooLarge { .. })
        ));
        assert!(ResizeSpec::Scale(f64::NAN).resolve(5, 5).is_err());
        assert!(ResizeSpec::Scale(f64::INFINITY).resolve(5, 5).is_err());
        assert!(ResizeSpec::Percent(-5.0).resolve(5, 5).is_err());
        assert!(ResizeSpec::Percent(50.0).resolve(0, 5).is_err());
    }

    #[test]
    fn thumbnail_fits_and_keeps_format() {
        assert_eq!(thumbnail_size(4000, 3000, 256), (256, 192));
        assert_eq!(thumbnail_size(3000, 4000, 256), (192, 256));
        assert_eq!(thumbnail_size(5000, 3, 256), (256, 1));
        assert_eq!(thumbnail_size(100, 50, 256), (100, 50));
        assert_eq!(thumbnail_size(100, 50, 0), (1, 1));
        let gray = ImageBuf::solid_rgba8(1000, 700, [90, 120, 150, 255]).unwrap();
        // Pre-shrink path (ratio 7.8) and direct path (ratio 2.9).
        for side in [128, 350] {
            let t = thumbnail(&gray, side);
            assert_eq!((t.width(), t.height()), thumbnail_size(1000, 700, side));
            assert!(t.data().chunks_exact(4).all(|p| p == [90, 120, 150, 255]));
        }
        let small = noise(20, 10);
        assert!(std::ptr::eq(thumbnail(&small, 64).data(), small.data()));
    }

    #[test]
    fn thumbnail_of_checkerboard_is_flat_gray() {
        let data = (0..600u32 * 400)
            .flat_map(|i| {
                let (x, y) = (i % 600, i / 600);
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            })
            .collect();
        let board = ImageBuf::new(600, 400, PixelFormat::Rgba8, data).unwrap();
        let t = thumbnail(&board, 64);
        assert_eq!((t.width(), t.height()), (64, 43));
        for p in t.data().chunks_exact(4) {
            assert!((186..=190).contains(&p[0]), "{p:?}");
        }
    }
}

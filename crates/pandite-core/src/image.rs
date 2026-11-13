//! Pixel buffers, animations and decoded content.
//!
//! Decoders hand back a [`Content`]. Today that is a still image or a frame
//! animation. The enum is `#[non_exhaustive]` and carries a [`ContentKind`]
//! so a later release can add 3D scenes without breaking providers, caches
//! or the renderer dispatch.

use crate::error::{Error, Result};
use std::sync::Arc;
use std::time::Duration;

/// Hard ceiling on a single image side, in pixels.
pub const MAX_DIMENSION: u32 = 65_535;
/// Hard ceiling on the byte size of one decoded buffer (4 GiB).
pub const MAX_BUFFER_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// In-memory pixel layouts. Rows are tightly packed, top row first, with no
/// padding. Alpha is straight (not premultiplied).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PixelFormat {
    /// 8 bits per channel, sRGB transfer, 4 bytes per pixel.
    Rgba8,
    /// 16 bits per channel, sRGB transfer, native endian, 8 bytes per pixel.
    Rgba16,
    /// 32-bit float per channel, linear light, native endian, 16 bytes per
    /// pixel. Used for high dynamic range sources (EXR, Radiance HDR).
    RgbaF32,
}

impl PixelFormat {
    /// Bytes used by one pixel.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            PixelFormat::Rgba8 => 4,
            PixelFormat::Rgba16 => 8,
            PixelFormat::RgbaF32 => 16,
        }
    }

    /// True for formats whose values can exceed display white.
    pub const fn is_hdr(self) -> bool {
        matches!(self, PixelFormat::RgbaF32)
    }
}

/// A decoded raster image. Cloning is cheap: pixel data is shared.
#[derive(Clone, Debug)]
pub struct ImageBuf {
    width: u32,
    height: u32,
    format: PixelFormat,
    /// Shared without copying the decoder's buffer: wrapping a `Vec` in an
    /// `Arc` moves only its header, while `Arc<[u8]>` would copy every
    /// byte (240 MB for a 60 megapixel image).
    data: Arc<Vec<u8>>,
}

impl ImageBuf {
    /// Wrap pixel data. Fails when a side is zero or above
    /// [`MAX_DIMENSION`], or when `data` is not exactly
    /// `width * height * bytes_per_pixel` bytes.
    pub fn new(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Result<Self> {
        let expected = Self::byte_len(width, height, format)?;
        if data.len() != expected {
            return Err(Error::Invalid(format!(
                "pixel data is {} bytes, expected {expected} for {width} x {height} {format:?}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            format,
            data: Arc::new(data),
        })
    }

    /// A buffer filled with one RGBA8 color. Handy for tests and placeholders.
    pub fn solid_rgba8(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self> {
        let len = Self::byte_len(width, height, PixelFormat::Rgba8)?;
        let mut data = Vec::with_capacity(len);
        for _ in 0..len / 4 {
            data.extend_from_slice(&rgba);
        }
        Self::new(width, height, PixelFormat::Rgba8, data)
    }

    /// The byte length a buffer of this shape needs, with limit checks.
    pub fn byte_len(width: u32, height: u32, format: PixelFormat) -> Result<usize> {
        if width == 0 || height == 0 {
            return Err(Error::Invalid(format!("empty image: {width} x {height}")));
        }
        if width > MAX_DIMENSION || height > MAX_DIMENSION {
            return Err(Error::TooLarge { width, height });
        }
        let bytes = u64::from(width) * u64::from(height) * format.bytes_per_pixel() as u64;
        if bytes > MAX_BUFFER_BYTES {
            return Err(Error::TooLarge { width, height });
        }
        usize::try_from(bytes).map_err(|_| Error::TooLarge { width, height })
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel layout.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Raw pixel bytes, top row first.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Bytes in one row.
    pub fn stride(&self) -> usize {
        self.width as usize * self.format.bytes_per_pixel()
    }

    /// The bytes of row `y`. Panics when `y` is out of range.
    pub fn row(&self, y: u32) -> &[u8] {
        let stride = self.stride();
        let start = y as usize * stride;
        &self.data[start..start + stride]
    }

    /// Memory held by the pixel data, for cache budgets.
    pub fn byte_size(&self) -> usize {
        self.data.len()
    }

    /// True when any pixel is not fully opaque. Scans the buffer.
    pub fn has_transparency(&self) -> bool {
        match self.format {
            PixelFormat::Rgba8 => self.data.chunks_exact(4).any(|p| p[3] != 0xFF),
            PixelFormat::Rgba16 => self
                .data
                .chunks_exact(8)
                .any(|p| u16::from_ne_bytes([p[6], p[7]]) != u16::MAX),
            PixelFormat::RgbaF32 => self
                .data
                .chunks_exact(16)
                .any(|p| f32::from_ne_bytes([p[12], p[13], p[14], p[15]]) < 1.0),
        }
    }
}

/// How often an animation repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopCount {
    /// Repeat forever.
    Infinite,
    /// Play this many times in total, then hold the last frame.
    Finite(u32),
}

/// One fully composed animation frame and how long it stays on screen.
#[derive(Clone, Debug)]
pub struct Frame {
    /// The complete canvas for this frame. Decoders resolve disposal and
    /// blending, so the renderer can show any frame without its neighbors.
    pub image: ImageBuf,
    /// Display time. Decoders replace zero or tiny delays with
    /// [`Animation::MIN_DELAY`] the way browsers do.
    pub delay: Duration,
}

/// A frame animation (GIF, animated WebP, APNG, animated AVIF or JPEG XL).
#[derive(Clone, Debug)]
pub struct Animation {
    frames: Vec<Frame>,
    loop_count: LoopCount,
}

impl Animation {
    /// Delays below this are treated as [`Animation::DEFAULT_DELAY`].
    pub const MIN_DELAY: Duration = Duration::from_millis(11);
    /// The delay browsers use for frames that declare none.
    pub const DEFAULT_DELAY: Duration = Duration::from_millis(100);

    /// Build an animation. Fails when there are no frames or the frames do
    /// not all share one size and pixel format. Short delays are normalized.
    pub fn new(mut frames: Vec<Frame>, loop_count: LoopCount) -> Result<Self> {
        let first = frames
            .first()
            .ok_or_else(|| Error::Invalid("animation has no frames".into()))?;
        let (w, h, fmt) = (
            first.image.width(),
            first.image.height(),
            first.image.format(),
        );
        if frames
            .iter()
            .any(|f| f.image.width() != w || f.image.height() != h || f.image.format() != fmt)
        {
            return Err(Error::Invalid("animation frames differ in shape".into()));
        }
        for frame in &mut frames {
            if frame.delay < Self::MIN_DELAY {
                frame.delay = Self::DEFAULT_DELAY;
            }
        }
        Ok(Self { frames, loop_count })
    }

    /// All frames in display order.
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    /// Repeat behavior.
    pub fn loop_count(&self) -> LoopCount {
        self.loop_count
    }

    /// Length of one pass through every frame.
    pub fn duration(&self) -> Duration {
        self.frames.iter().map(|f| f.delay).sum()
    }

    /// The frame index to show `elapsed` after playback started, and the
    /// time left until the next frame change (`None` when playback ended on
    /// the last frame of a finite animation).
    pub fn frame_at(&self, elapsed: Duration) -> (usize, Option<Duration>) {
        let total = self.duration();
        let last = self.frames.len() - 1;
        if self.frames.len() == 1 || total.is_zero() {
            return (0, None);
        }
        if let LoopCount::Finite(n) = self.loop_count {
            if elapsed >= total * n.max(1) {
                return (last, None);
            }
        }
        let t = Duration::from_nanos((elapsed.as_nanos() % total.as_nanos()) as u64);
        let mut acc = Duration::ZERO;
        for (i, frame) in self.frames.iter().enumerate() {
            acc += frame.delay;
            if t < acc {
                return (i, Some(acc - t));
            }
        }
        (last, None)
    }

    /// Memory held by all frames.
    pub fn byte_size(&self) -> usize {
        self.frames.iter().map(|f| f.image.byte_size()).sum()
    }
}

/// EXIF-style orientation. Decoders apply it before returning pixels, so
/// consumers always receive upright images. It is reported in [`ImageMeta`]
/// for display only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// Stored upright.
    #[default]
    Normal,
    /// Mirrored left to right.
    FlipH,
    /// Rotated 180 degrees.
    Rotate180,
    /// Mirrored top to bottom.
    FlipV,
    /// Mirrored along the top-left to bottom-right diagonal.
    Transpose,
    /// Rotated 90 degrees clockwise.
    Rotate90,
    /// Mirrored along the top-right to bottom-left diagonal.
    Transverse,
    /// Rotated 270 degrees clockwise.
    Rotate270,
}

impl Orientation {
    /// Map an EXIF orientation tag value (1 to 8). Unknown values are `Normal`.
    pub fn from_exif(value: u16) -> Self {
        match value {
            2 => Orientation::FlipH,
            3 => Orientation::Rotate180,
            4 => Orientation::FlipV,
            5 => Orientation::Transpose,
            6 => Orientation::Rotate90,
            7 => Orientation::Transverse,
            8 => Orientation::Rotate270,
            _ => Orientation::Normal,
        }
    }

    /// True when applying the orientation swaps width and height.
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Orientation::Transpose
                | Orientation::Rotate90
                | Orientation::Transverse
                | Orientation::Rotate270
        )
    }
}

/// The broad class of decoded content. Renderer providers dispatch on this.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ContentKind {
    /// One raster image.
    Still,
    /// A raster frame animation.
    Animation,
    /// A 3D scene with meshes, materials and optional animation. Reserved:
    /// no provider produces it yet.
    Scene3d,
}

/// What a decoder produced.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Content {
    /// One raster image.
    Still(ImageBuf),
    /// A raster frame animation.
    Animation(Animation),
}

impl Content {
    /// The class of this content.
    pub fn kind(&self) -> ContentKind {
        match self {
            Content::Still(_) => ContentKind::Still,
            Content::Animation(_) => ContentKind::Animation,
        }
    }

    /// Pixel size of the content (the canvas size for animations).
    pub fn dimensions(&self) -> (u32, u32) {
        let image = self.first_image();
        (image.width(), image.height())
    }

    /// The still image, or the first frame of an animation.
    pub fn first_image(&self) -> &ImageBuf {
        match self {
            Content::Still(image) => image,
            Content::Animation(anim) => &anim.frames()[0].image,
        }
    }

    /// Memory held by the content, for cache budgets.
    pub fn byte_size(&self) -> usize {
        match self {
            Content::Still(image) => image.byte_size(),
            Content::Animation(anim) => anim.byte_size(),
        }
    }
}

/// Facts about a file that are cheap to read without a full decode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageMeta {
    /// Width in pixels after orientation is applied.
    pub width: u32,
    /// Height in pixels after orientation is applied.
    pub height: u32,
    /// The provider id that recognized the file, for example `"jpeg"`.
    pub format: &'static str,
    /// The class of content a full decode returns.
    pub kind: ContentKind,
    /// Number of animation frames when known, 1 for stills.
    pub frame_count: u32,
    /// True when the file declares an alpha channel.
    pub has_alpha: bool,
    /// Stored orientation (already applied to `width` and `height`).
    pub orientation: Orientation,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ms: u64) -> Frame {
        Frame {
            image: ImageBuf::solid_rgba8(2, 2, [1, 2, 3, 255]).unwrap(),
            delay: Duration::from_millis(ms),
        }
    }

    #[test]
    fn imagebuf_rejects_wrong_length_and_limits() {
        assert!(ImageBuf::new(2, 2, PixelFormat::Rgba8, vec![0; 16]).is_ok());
        assert!(matches!(
            ImageBuf::new(2, 2, PixelFormat::Rgba8, vec![0; 15]),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            ImageBuf::byte_len(0, 4, PixelFormat::Rgba8),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            ImageBuf::byte_len(70_000, 4, PixelFormat::Rgba8),
            Err(Error::TooLarge { .. })
        ));
        assert!(matches!(
            ImageBuf::byte_len(65_535, 65_535, PixelFormat::RgbaF32),
            Err(Error::TooLarge { .. })
        ));
    }

    #[test]
    fn imagebuf_keeps_the_decoder_buffer() {
        let data = vec![7u8; 4 * 3 * 4];
        let ptr = data.as_ptr();
        let img = ImageBuf::new(4, 3, PixelFormat::Rgba8, data).unwrap();
        assert_eq!(img.data().as_ptr(), ptr, "no copy on construction");
        assert_eq!(img.clone().data().as_ptr(), ptr, "clones share the pixels");
    }

    #[test]
    fn imagebuf_rows_and_transparency() {
        let mut data = vec![255u8; 2 * 2 * 4];
        data[4..8].copy_from_slice(&[9, 8, 7, 255]);
        let img = ImageBuf::new(2, 2, PixelFormat::Rgba8, data.clone()).unwrap();
        assert_eq!(img.stride(), 8);
        assert_eq!(&img.row(0)[4..8], &[9, 8, 7, 255]);
        assert!(!img.has_transparency());
        data[15] = 10;
        let img = ImageBuf::new(2, 2, PixelFormat::Rgba8, data).unwrap();
        assert!(img.has_transparency());
    }

    #[test]
    fn animation_normalizes_delays_and_checks_shape() {
        let anim = Animation::new(vec![frame(0), frame(40)], LoopCount::Infinite).unwrap();
        assert_eq!(anim.frames()[0].delay, Animation::DEFAULT_DELAY);
        assert_eq!(anim.duration(), Duration::from_millis(140));
        assert!(Animation::new(vec![], LoopCount::Infinite).is_err());
        let odd = Frame {
            image: ImageBuf::solid_rgba8(3, 2, [0; 4]).unwrap(),
            delay: Duration::from_millis(50),
        };
        assert!(Animation::new(vec![frame(50), odd], LoopCount::Infinite).is_err());
    }

    #[test]
    fn animation_frame_at_loops_and_ends() {
        let anim = Animation::new(vec![frame(100), frame(50)], LoopCount::Infinite).unwrap();
        assert_eq!(
            anim.frame_at(Duration::from_millis(0)),
            (0, Some(Duration::from_millis(100)))
        );
        assert_eq!(
            anim.frame_at(Duration::from_millis(120)),
            (1, Some(Duration::from_millis(30)))
        );
        // Wraps after 150 ms.
        assert_eq!(
            anim.frame_at(Duration::from_millis(160)),
            (0, Some(Duration::from_millis(90)))
        );
        let once = Animation::new(vec![frame(100), frame(50)], LoopCount::Finite(1)).unwrap();
        assert_eq!(once.frame_at(Duration::from_millis(149)).0, 1);
        assert_eq!(once.frame_at(Duration::from_millis(150)), (1, None));
        let single = Animation::new(vec![frame(100)], LoopCount::Infinite).unwrap();
        assert_eq!(single.frame_at(Duration::from_secs(9)), (0, None));
    }

    #[test]
    fn orientation_from_exif() {
        assert_eq!(Orientation::from_exif(6), Orientation::Rotate90);
        assert!(Orientation::from_exif(8).swaps_axes());
        assert!(!Orientation::from_exif(3).swaps_axes());
        assert_eq!(Orientation::from_exif(42), Orientation::Normal);
    }

    #[test]
    fn content_reports_kind_and_size() {
        let still = Content::Still(ImageBuf::solid_rgba8(3, 5, [0; 4]).unwrap());
        assert_eq!(still.kind(), ContentKind::Still);
        assert_eq!(still.dimensions(), (3, 5));
        assert_eq!(still.byte_size(), 60);
        let anim = Content::Animation(
            Animation::new(vec![frame(20), frame(20)], LoopCount::Infinite).unwrap(),
        );
        assert_eq!(anim.kind(), ContentKind::Animation);
        assert_eq!(anim.byte_size(), 32);
    }
}

//! PNG, JPEG and lossless WebP encoders. The BMP encoder lives in `bmp.rs`.

use crate::util::{self, guard};
use pandite_core::{EncodeOptions, EncoderProvider, Error, ImageBuf, PixelFormat, Result};
use std::io::Write;

fn backend(format: &str, err: impl std::fmt::Display) -> Error {
    Error::Backend(format!("{format} encoder: {err}"))
}

/// Writes PNG files. 8-bit sources stay 8-bit, 16-bit and float sources are
/// written as 16-bit. Opaque images drop the alpha channel.
#[derive(Clone, Copy, Debug, Default)]
pub struct PngEncoder;

impl EncoderProvider for PngEncoder {
    fn id(&self) -> &'static str {
        "png"
    }

    fn name(&self) -> &'static str {
        "PNG image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["png"]
    }

    fn supports_alpha(&self) -> bool {
        true
    }

    fn has_quality(&self) -> bool {
        false
    }

    fn encode(
        &self,
        image: &ImageBuf,
        _options: &EncodeOptions,
        out: &mut dyn Write,
    ) -> Result<()> {
        let (w, h) = (image.width(), image.height());
        let alpha = image.has_transparency();
        let channels = if alpha { 4 } else { 3 };
        let (depth, data) = if image.format() == PixelFormat::Rgba8 {
            let src = image.data();
            let data: Vec<u8> = if alpha {
                src.to_vec()
            } else {
                src.chunks_exact(4)
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect()
            };
            (png::BitDepth::Eight, data)
        } else {
            let samples = util::to_rgba16(image);
            let mut data = Vec::with_capacity(samples.len() / 4 * channels * 2);
            for px in samples.chunks_exact(4) {
                for &s in &px[..channels] {
                    data.extend_from_slice(&s.to_be_bytes());
                }
            }
            (png::BitDepth::Sixteen, data)
        };
        let mut buf = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut buf, w, h);
            enc.set_color(if alpha {
                png::ColorType::Rgba
            } else {
                png::ColorType::Rgb
            });
            enc.set_depth(depth);
            enc.set_compression(png::Compression::Balanced);
            let mut writer = enc.write_header().map_err(|e| backend("png", e))?;
            writer
                .write_image_data(&data)
                .map_err(|e| backend("png", e))?;
            writer.finish().map_err(|e| backend("png", e))?;
        }
        out.write_all(&buf)?;
        Ok(())
    }
}

/// Writes baseline JPEG files. Alpha is flattened onto white. Quality 90 and
/// above keeps full chroma resolution, lower qualities use 4:2:0.
#[derive(Clone, Copy, Debug, Default)]
pub struct JpegEncoder;

/// Composite RGBA8 pixels over white and return RGB8.
pub(crate) fn flatten_on_white(rgba: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.chunks_exact(4) {
        let a = u32::from(p[3]);
        for &c in &p[..3] {
            let v = (u32::from(c) * a + 255 * (255 - a) + 127) / 255;
            rgb.push(v as u8);
        }
    }
    rgb
}

impl EncoderProvider for JpegEncoder {
    fn id(&self) -> &'static str {
        "jpeg"
    }

    fn name(&self) -> &'static str {
        "JPEG image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["jpg", "jpeg", "jpe", "jfif"]
    }

    fn supports_alpha(&self) -> bool {
        false
    }

    fn has_quality(&self) -> bool {
        true
    }

    fn encode(&self, image: &ImageBuf, options: &EncodeOptions, out: &mut dyn Write) -> Result<()> {
        let (w, h) = (image.width(), image.height());
        let (w16, h16) = match (u16::try_from(w), u16::try_from(h)) {
            (Ok(a), Ok(b)) => (a, b),
            _ => {
                return Err(Error::Invalid(
                    "JPEG sides are limited to 65535 pixels".into(),
                ))
            }
        };
        let rgb = flatten_on_white(&util::to_rgba8(image));
        let quality = options.quality.clamp(1, 100);
        let mut buf = Vec::new();
        guard("jpeg encoder", || {
            let mut enc = jpeg_encoder::Encoder::new(&mut buf, quality);
            enc.set_optimized_huffman_tables(true);
            enc.encode(&rgb, w16, h16, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| backend("jpeg", e))
        })?;
        out.write_all(&buf)?;
        Ok(())
    }
}

/// Writes lossless WebP files. WebP limits each side to 16383 pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct WebpEncoder;

impl EncoderProvider for WebpEncoder {
    fn id(&self) -> &'static str {
        "webp"
    }

    fn name(&self) -> &'static str {
        "WebP image (lossless)"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["webp"]
    }

    fn supports_alpha(&self) -> bool {
        true
    }

    fn has_quality(&self) -> bool {
        false
    }

    fn encode(
        &self,
        image: &ImageBuf,
        _options: &EncodeOptions,
        out: &mut dyn Write,
    ) -> Result<()> {
        let (w, h) = (image.width(), image.height());
        if w > 16383 || h > 16383 {
            return Err(Error::Invalid(
                "WebP sides are limited to 16383 pixels".into(),
            ));
        }
        let rgba = util::to_rgba8(image);
        let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
        let mut buf = Vec::new();
        guard("webp encoder", || {
            let enc = image_webp::WebPEncoder::new(&mut buf);
            if alpha {
                enc.encode(&rgba, w, h, image_webp::ColorType::Rgba8)
            } else {
                let rgb: Vec<u8> = rgba
                    .chunks_exact(4)
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect();
                enc.encode(&rgb, w, h, image_webp::ColorType::Rgb8)
            }
            .map_err(|e| backend("webp", e))
        })?;
        out.write_all(&buf)?;
        Ok(())
    }
}

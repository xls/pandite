//! JPEG decoder (baseline, progressive and lossless) on `jpeg-decoder`.
//!
//! Supports grayscale, YCbCr, RGB, CMYK and YCCK files, 16-bit lossless
//! files, EXIF orientation and DCT scaling to 1/2, 1/4 or 1/8 when the
//! caller asks for a smaller target size.

use crate::exif::orientation_from_exif;
use crate::util::{alloc_rgba8, check_size, cmyk_to_rgb, corrupt, guard, scale_to_u16};
use crate::{icc, orient, util};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageMeta, Orientation,
    PixelFormat, Result, Source,
};

const FORMAT: &str = "jpeg";

/// Facts read from the JPEG marker stream without decoding pixels.
#[derive(Debug, Default)]
pub(crate) struct Header<'a> {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub precision: u8,
    pub exif: Option<&'a [u8]>,
}

/// Walk the markers up to the frame header. `None` when the stream is not a
/// JPEG or the frame header is missing.
pub(crate) fn scan_header(data: &[u8]) -> Option<Header<'_>> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut header = Header::default();
    let mut pos = 2;
    loop {
        // Find the next marker, skipping fill bytes.
        while *data.get(pos)? != 0xFF {
            pos += 1;
        }
        while *data.get(pos)? == 0xFF {
            pos += 1;
        }
        let marker = *data.get(pos)?;
        pos += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => return None,
            _ => {}
        }
        let len = usize::from(util::be16(data, pos)?);
        if len < 2 {
            return None;
        }
        let payload = data.get(pos + 2..pos + len)?;
        match marker {
            0xE1 if payload.starts_with(b"Exif\0\0") && header.exif.is_none() => {
                header.exif = Some(&payload[6..]);
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                header.precision = *payload.first()?;
                header.height = u32::from(util::be16(payload, 1)?);
                header.width = u32::from(util::be16(payload, 3)?);
                header.components = *payload.get(5)?;
                return Some(header);
            }
            _ => {}
        }
        pos += len;
    }
}

/// Decodes JPEG files.
#[derive(Clone, Copy, Debug, Default)]
pub struct JpegDecoder;

impl DecoderProvider for JpegDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "JPEG image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["jpg", "jpeg", "jpe", "jfif"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(&[0xFF, 0xD8, 0xFF])
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = scan_header(source.bytes()).ok_or_else(|| corrupt(FORMAT, "no frame header"))?;
        if h.width == 0 || h.height == 0 {
            return Err(corrupt(FORMAT, "zero image size"));
        }
        let orientation = h.exif.map(orientation_from_exif).unwrap_or_default();
        let (width, height) = oriented(h.width, h.height, orientation);
        Ok(ImageMeta {
            width,
            height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: false,
            orientation,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let image = guard(FORMAT, || decode_image(source.bytes(), options))?;
        Ok(Content::Still(image))
    }
}

/// Width and height after applying an orientation.
pub(crate) fn oriented(w: u32, h: u32, o: Orientation) -> (u32, u32) {
    if o.swaps_axes() {
        (h, w)
    } else {
        (w, h)
    }
}

/// Decode a JPEG stream to an upright image. Also used for embedded JPEGs
/// (camera RAW previews, BMP and ICO payloads).
pub(crate) fn decode_image(data: &[u8], options: &DecodeOptions) -> Result<ImageBuf> {
    let header = scan_header(data).ok_or_else(|| corrupt(FORMAT, "no frame header"))?;
    check_size(header.width, header.height, PixelFormat::Rgba8)?;
    let orientation = header.exif.map(orientation_from_exif).unwrap_or_default();

    let mut dec = jpeg_decoder::Decoder::new(data);
    dec.set_max_decoding_buffer_size(util::decoder_memory_limit());
    dec.read_info().map_err(|e| corrupt(FORMAT, e))?;
    if let Some((tw, th)) = options.target_size {
        // The target is in display orientation; scale works on stored axes.
        let (rw, rh) = oriented(tw, th, orientation);
        let clamp = |v: u32| v.clamp(1, u32::from(u16::MAX)) as u16;
        dec.scale(clamp(rw), clamp(rh))
            .map_err(|e| corrupt(FORMAT, e))?;
    }
    let info = dec
        .info()
        .ok_or_else(|| corrupt(FORMAT, "missing frame information"))?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    check_size(w, h, PixelFormat::Rgba8)?;
    options.cancel.check()?;
    let pixels = dec.decode().map_err(|e| corrupt(FORMAT, e))?;
    options.cancel.check()?;
    let n = w as usize * h as usize;
    let icc_profile = dec.icc_profile();

    let image = match info.pixel_format {
        jpeg_decoder::PixelFormat::L8 => {
            need(&pixels, n)?;
            let mut out = alloc_rgba8(w, h)?;
            util::expand_to_rgba8(&pixels, 1, &mut out);
            util::rgba8_image(w, h, out)?
        }
        jpeg_decoder::PixelFormat::L16 => {
            need(&pixels, n * 2)?;
            let max = (1u32 << header.precision.clamp(2, 16)) - 1;
            let samples = pixels
                .chunks_exact(2)
                .map(|c| scale_to_u16(u32::from(u16::from_ne_bytes([c[0], c[1]])), max));
            let len = check_size(w, h, PixelFormat::Rgba16)?;
            let mut out = util::alloc(len, w, h)?;
            util::expand16_to_rgba16(samples, 1, &mut out);
            ImageBuf::new(w, h, PixelFormat::Rgba16, out)?
        }
        jpeg_decoder::PixelFormat::RGB24 => {
            need(&pixels, n * 3)?;
            let mut out = alloc_rgba8(w, h)?;
            util::expand_to_rgba8(&pixels, 3, &mut out);
            icc::to_srgb_rgba8(icc_profile.as_deref(), &mut out);
            util::rgba8_image(w, h, out)?
        }
        jpeg_decoder::PixelFormat::CMYK32 => {
            need(&pixels, n * 4)?;
            let mut out = alloc_rgba8(w, h)?;
            if !icc::cmyk_to_srgb_rgba8(icc_profile.as_deref(), &pixels, &mut out) {
                for (o, p) in out.chunks_exact_mut(4).zip(pixels.chunks_exact(4)) {
                    let [r, g, b] = cmyk_to_rgb(p[0], p[1], p[2], p[3]);
                    o.copy_from_slice(&[r, g, b, 255]);
                }
            }
            util::rgba8_image(w, h, out)?
        }
    };
    orient::apply(image, orientation)
}

fn need(pixels: &[u8], len: usize) -> Result<()> {
    if pixels.len() < len {
        Err(corrupt(FORMAT, "decoder returned too few pixels"))
    } else {
        Ok(())
    }
}

//! WebP decoder (lossy, lossless, alpha, animation) on `image-webp`.
//!
//! Stills use `image-webp` directly. Animations are composed here: every
//! `ANMF` frame is rewrapped as a standalone WebP, decoded, and drawn onto
//! the canvas with the frame's blend and dispose flags. Disposal clears to
//! transparent, as browsers do, whatever background color the file names.

use crate::compose::{self, Region};
use crate::exif::orientation_from_exif;
use crate::jpeg::oriented;
use crate::util::{self, check_size, corrupt, guard, le32, FrameCollector};
use crate::{icc, orient};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageMeta, LoopCount,
    Orientation, PixelFormat, Result, Source,
};
use std::io::Cursor;
use std::time::Duration;

const FORMAT: &str = "webp";

/// Decodes WebP files, still or animated.
#[derive(Clone, Copy, Debug, Default)]
pub struct WebpDecoder;

impl DecoderProvider for WebpDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "WebP image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["webp"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.len() >= 12 && &header[..4] == b"RIFF" && &header[8..12] == b"WEBP"
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let riff = Riff::parse(source.bytes())?;
            let mut dec = image_webp::WebPDecoder::new(Cursor::new(source.bytes()))
                .map_err(|e| corrupt(FORMAT, e))?;
            let (w, h) = dec.dimensions();
            let orientation = exif(&mut dec);
            let (width, height) = oriented(w, h, orientation);
            let frames = riff.frames.len().max(1) as u32;
            Ok(ImageMeta {
                width,
                height,
                format: FORMAT,
                kind: if frames > 1 {
                    ContentKind::Animation
                } else {
                    ContentKind::Still
                },
                frame_count: frames,
                has_alpha: dec.has_alpha(),
                orientation,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), options))
    }
}

fn exif(dec: &mut image_webp::WebPDecoder<Cursor<&[u8]>>) -> Orientation {
    dec.exif_metadata()
        .ok()
        .flatten()
        .map(|e| orientation_from_exif(&e))
        .unwrap_or_default()
}

/// The chunk layout of a WebP file, as far as animation needs it.
struct Riff<'a> {
    canvas: (u32, u32),
    loop_count: u16,
    frames: Vec<&'a [u8]>,
}

impl<'a> Riff<'a> {
    fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
            return Err(corrupt(FORMAT, "not a WebP file"));
        }
        let riff_len = le32(data, 4).unwrap_or(0) as usize;
        let end = riff_len.saturating_add(8).min(data.len());
        let mut riff = Riff {
            canvas: (0, 0),
            loop_count: 0,
            frames: Vec::new(),
        };
        let mut pos = 12;
        while pos + 8 <= end {
            let fourcc = &data[pos..pos + 4];
            let len = le32(data, pos + 4).unwrap_or(0) as usize;
            let body_end = pos + 8 + len;
            if body_end > end {
                // A truncated final chunk is reported by the frame decoder.
                riff.push_chunk(fourcc, &data[pos + 8..end]);
                break;
            }
            riff.push_chunk(fourcc, &data[pos + 8..body_end]);
            pos = body_end + (len & 1);
        }
        Ok(riff)
    }

    fn push_chunk(&mut self, fourcc: &[u8], body: &'a [u8]) {
        match fourcc {
            b"VP8X" if body.len() >= 10 => {
                let w = u32::from_le_bytes([body[4], body[5], body[6], 0]) + 1;
                let h = u32::from_le_bytes([body[7], body[8], body[9], 0]) + 1;
                self.canvas = (w, h);
            }
            b"ANIM" if body.len() >= 6 => {
                self.loop_count = u16::from_le_bytes([body[4], body[5]]);
            }
            b"ANMF" => self.frames.push(body),
            _ => {}
        }
    }
}

/// Decode a WebP stream. Also used for WebP payloads in other containers.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Content> {
    let mut dec =
        image_webp::WebPDecoder::new(Cursor::new(data)).map_err(|e| corrupt(FORMAT, e))?;
    dec.set_memory_limit(util::decoder_memory_limit());
    let (w, h) = dec.dimensions();
    check_size(w, h, PixelFormat::Rgba8)?;
    let orientation = exif(&mut dec);
    let profile = dec.icc_profile().ok().flatten();

    if !dec.is_animated() {
        let mut px = still(&mut dec, w, h)?;
        icc::to_srgb_rgba8(profile.as_deref(), &mut px);
        let image = util::rgba8_image(w, h, px)?;
        return Ok(Content::Still(orient::apply(image, orientation)?));
    }

    let riff = Riff::parse(data)?;
    if riff.canvas != (w, h) {
        return Err(corrupt(FORMAT, "canvas size mismatch"));
    }
    let loop_count = match riff.loop_count {
        0 => LoopCount::Infinite,
        n => LoopCount::Finite(u32::from(n)),
    };
    let (cwu, chu) = (w as usize, h as usize);
    let mut canvas = util::alloc_rgba8(w, h)?;
    let mut frames = FrameCollector::new();
    let mut previous: Option<Region> = None;
    for anmf in &riff.frames {
        options.cancel.check()?;
        if let Some(region) = previous.take() {
            compose::clear(&mut canvas, cwu, region);
        }
        let header = anmf
            .get(..16)
            .ok_or_else(|| corrupt(FORMAT, "truncated frame header"))?;
        let u24 = |i: usize| u32::from_le_bytes([header[i], header[i + 1], header[i + 2], 0]);
        let (fx, fy) = (u24(0) * 2, u24(3) * 2);
        let (fw, fh) = (u24(6) + 1, u24(9) + 1);
        let duration = u24(12);
        let flags = header[15];
        let blend = flags & 0b10 == 0;
        let dispose = flags & 0b01 != 0;
        if fx + fw > w || fy + fh > h {
            return Err(corrupt(FORMAT, "frame outside the canvas"));
        }
        let frame_file = wrap_frame(&anmf[16..], fw, fh)?;
        let mut fdec = image_webp::WebPDecoder::new(Cursor::new(&frame_file[..]))
            .map_err(|e| corrupt(FORMAT, e))?;
        if fdec.dimensions() != (fw, fh) {
            return Err(corrupt(FORMAT, "frame size mismatch"));
        }
        let mut px = still(&mut fdec, fw, fh)?;
        icc::to_srgb_rgba8(profile.as_deref(), &mut px);
        let region = Region::clip(fx, fy, fw, fh, cwu, chu);
        compose::draw(&mut canvas, cwu, region, &px, fw as usize, blend);

        let image = ImageBuf::new(w, h, PixelFormat::Rgba8, canvas.clone())?;
        let image = orient::apply(image, orientation)?;
        if !frames.push(image, Duration::from_millis(u64::from(duration)))
            || options.first_frame_only
        {
            break;
        }
        if dispose {
            previous = Some(region);
        }
    }
    if frames.len() == 0 {
        return Err(corrupt(FORMAT, "animation has no frames"));
    }
    frames.finish(loop_count)
}

/// Decode a still (or the first frame) to RGBA8 bytes.
fn still(dec: &mut image_webp::WebPDecoder<Cursor<&[u8]>>, w: u32, h: u32) -> Result<Vec<u8>> {
    let len = dec
        .output_buffer_size()
        .ok_or_else(|| corrupt(FORMAT, "image too large"))?;
    let mut buf = util::alloc(len, w, h)?;
    dec.read_image(&mut buf).map_err(|e| corrupt(FORMAT, e))?;
    if dec.has_alpha() {
        Ok(buf)
    } else {
        let mut out = util::alloc_rgba8(w, h)?;
        util::expand_to_rgba8(&buf, 3, &mut out);
        Ok(out)
    }
}

/// Build a standalone WebP file from the chunks inside one `ANMF` frame.
fn wrap_frame(chunks: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let mut alpha: Option<&[u8]> = None;
    let mut image: Option<(&[u8], &[u8])> = None;
    let mut pos = 0;
    while pos + 8 <= chunks.len() {
        let fourcc = &chunks[pos..pos + 4];
        let len = le32(chunks, pos + 4).unwrap_or(0) as usize;
        let end = (pos + 8).saturating_add(len).min(chunks.len());
        let chunk = &chunks[pos..end];
        match fourcc {
            b"ALPH" if alpha.is_none() => alpha = Some(chunk),
            b"VP8 " | b"VP8L" => {
                image = Some((fourcc, chunk));
                break;
            }
            _ => {}
        }
        pos = pos + 8 + len + (len & 1);
    }
    let (fourcc, image) = image.ok_or_else(|| corrupt(FORMAT, "frame without image data"))?;
    let mut body = Vec::with_capacity(chunks.len() + 32);
    if fourcc == b"VP8 " && alpha.is_some() {
        body.extend_from_slice(b"VP8X");
        body.extend_from_slice(&10u32.to_le_bytes());
        body.extend_from_slice(&[0x10, 0, 0, 0]);
        body.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        body.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        let a = alpha.unwrap_or_default();
        body.extend_from_slice(a);
        if a.len() % 2 == 1 {
            body.push(0);
        }
    }
    body.extend_from_slice(image);
    if image.len() % 2 == 1 {
        body.push(0);
    }
    let mut file = Vec::with_capacity(body.len() + 12);
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend_from_slice(&body);
    Ok(file)
}

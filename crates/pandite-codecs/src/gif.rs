//! GIF decoder on the `gif` crate.
//!
//! Frames are composed onto a transparent canvas the way browsers do it:
//! transparent indices keep the canvas, disposal "background" clears to
//! transparent, disposal "previous" restores the canvas, frames that reach
//! past the canvas are clipped, and a missing trailer after complete frames
//! is accepted. Loop counts follow browsers (NETSCAPE count plus one).

use crate::compose::{self, Region};
use crate::icc;
use crate::util::{self, check_size, corrupt, guard, FrameCollector};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageMeta, LoopCount,
    Orientation, PixelFormat, Result, Source,
};
use std::num::NonZeroU64;
use std::time::Duration;

const FORMAT: &str = "gif";

/// Decodes GIF files, animated or not.
#[derive(Clone, Copy, Debug, Default)]
pub struct GifDecoder;

impl DecoderProvider for GifDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "GIF image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["gif"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"GIF87a") || header.starts_with(b"GIF89a")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let mut opts = decode_options();
            opts.skip_frame_decoding(true);
            let mut dec = opts
                .read_info(source.bytes())
                .map_err(|e| corrupt(FORMAT, e))?;
            let (w, h) = (u32::from(dec.width()), u32::from(dec.height()));
            if w == 0 || h == 0 {
                return Err(corrupt(FORMAT, "zero canvas size"));
            }
            let mut frames = 0u32;
            let mut has_alpha = false;
            while let Ok(Some(frame)) = dec.read_next_frame() {
                frames += 1;
                has_alpha |= frame.transparent.is_some();
                if frames >= 100_000 {
                    break;
                }
            }
            Ok(ImageMeta {
                width: w,
                height: h,
                format: FORMAT,
                kind: if frames > 1 {
                    ContentKind::Animation
                } else {
                    ContentKind::Still
                },
                frame_count: frames.max(1),
                has_alpha,
                orientation: Orientation::Normal,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), options))
    }
}

fn decode_options() -> gif::DecodeOptions {
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let limit = u64::try_from(util::decoder_memory_limit()).unwrap_or(u64::MAX);
    if let Some(limit) = NonZeroU64::new(limit) {
        opts.set_memory_limit(gif::MemoryLimit::Bytes(limit));
    }
    opts.check_frame_consistency(false);
    opts.allow_unknown_blocks(true);
    opts
}

fn decode(data: &[u8], options: &DecodeOptions) -> Result<Content> {
    let mut dec = decode_options()
        .read_info(data)
        .map_err(|e| corrupt(FORMAT, e))?;
    let (cw, ch) = (u32::from(dec.width()), u32::from(dec.height()));
    check_size(cw, ch, PixelFormat::Rgba8)?;
    let (cwu, chu) = (cw as usize, ch as usize);
    let profile = dec.icc_profile().map(<[u8]>::to_vec);
    let mut canvas = util::alloc_rgba8(cw, ch)?;
    let mut frames = FrameCollector::new();
    let mut indices: Vec<u8> = Vec::new();

    loop {
        options.cancel.check()?;
        let info = match dec.next_frame_info() {
            Ok(Some(f)) => f.clone(),
            Ok(None) => break,
            // A file that ends after complete frames lacks only its trailer.
            Err(gif::DecodingError::UnexpectedEof) if frames.len() > 0 => break,
            Err(e) => return Err(corrupt(FORMAT, e)),
        };
        let fw = usize::from(info.width);
        let fh = usize::from(info.height);
        // A frame may claim a size far beyond the canvas; refuse what the
        // buffer limits refuse before allocating its index buffer.
        check_size(
            u32::from(info.width),
            u32::from(info.height),
            PixelFormat::Rgba8,
        )?;
        indices.clear();
        indices
            .try_reserve(fw * fh)
            .map_err(|_| pandite_core::Error::TooLarge {
                width: u32::from(info.width),
                height: u32::from(info.height),
            })?;
        indices.resize(fw * fh, 0);
        dec.read_into_buffer(&mut indices)
            .map_err(|e| corrupt(FORMAT, e))?;
        let palette: Vec<u8> = dec.palette().map_err(|e| corrupt(FORMAT, e))?.to_vec();

        let region = Region::clip(
            u32::from(info.left),
            u32::from(info.top),
            u32::from(info.width),
            u32::from(info.height),
            cwu,
            chu,
        );
        let saved = (info.dispose == gif::DisposalMethod::Previous)
            .then(|| compose::save(&canvas, cwu, region));
        for row in 0..region.h {
            let src = &indices[row * fw..row * fw + region.w];
            let start = ((region.y + row) * cwu + region.x) * 4;
            let dst = &mut canvas[start..start + region.w * 4];
            for (d, &i) in dst.chunks_exact_mut(4).zip(src) {
                if Some(i) == info.transparent {
                    continue;
                }
                let p = usize::from(i) * 3;
                match palette.get(p..p + 3) {
                    Some(c) => d.copy_from_slice(&[c[0], c[1], c[2], 255]),
                    None => d.copy_from_slice(&[0, 0, 0, 255]),
                }
            }
        }

        let delay = Duration::from_millis(u64::from(info.delay) * 10);
        let mut shown = canvas.clone();
        icc::to_srgb_rgba8(profile.as_deref(), &mut shown);
        let image = ImageBuf::new(cw, ch, PixelFormat::Rgba8, shown)?;
        if !frames.push(image, delay) || options.first_frame_only {
            break;
        }
        match info.dispose {
            gif::DisposalMethod::Background => compose::clear(&mut canvas, cwu, region),
            gif::DisposalMethod::Previous => {
                if let Some(saved) = saved {
                    compose::restore(&mut canvas, cwu, region, &saved);
                }
            }
            _ => {}
        }
    }

    let loop_count = match dec.repeat() {
        gif::Repeat::Infinite => LoopCount::Infinite,
        gif::Repeat::Finite(0) => LoopCount::Finite(1),
        gif::Repeat::Finite(n) => LoopCount::Finite(u32::from(n) + 1),
    };
    if frames.len() == 0 {
        return Err(corrupt(FORMAT, "no image data"));
    }
    frames.finish(loop_count)
}

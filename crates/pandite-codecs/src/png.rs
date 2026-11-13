//! PNG and APNG decoder on the `png` crate.
//!
//! Every color type and bit depth is supported, including palettes with
//! transparency and Adam7 interlacing. 16-bit stills decode to `Rgba16`.
//! APNG frames are composed onto a full canvas with dispose and blend
//! operations resolved, at 8 bits per channel. A default image that is not
//! part of the animation is skipped, but it is the still returned for
//! `first_frame_only`, since it is the file's own static stand-in. The
//! `eXIf` orientation (when it precedes the image data) and the `iCCP`
//! profile apply. Non-interlaced stills are read row by row, so a cancel
//! request stops them part way.

use crate::compose::{self, Region};
use crate::exif::orientation_from_exif;
use crate::facts::{camera_from_exif, icc_description};
use crate::jpeg::oriented;
use crate::util::{self, check_size, corrupt, guard, FrameCollector};
use crate::{icc, orient};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageFacts, ImageMeta,
    LoopCount, Orientation, PixelFormat, Result, Source,
};
use png::{BitDepth, ColorType, Transformations};
use std::io::Cursor;
use std::time::Duration;

const FORMAT: &str = "png";
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Decodes PNG and animated PNG files.
#[derive(Clone, Copy, Debug, Default)]
pub struct PngDecoder;

impl DecoderProvider for PngDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "PNG image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["png", "apng"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(&SIGNATURE)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let reader = open(source.bytes(), false)?;
            let info = reader.info();
            let orientation = exif_orientation(info);
            let (width, height) = oriented(info.width, info.height, orientation);
            let frames = info
                .animation_control
                .map(|a| a.num_frames)
                .unwrap_or(1)
                .max(1);
            let has_alpha = matches!(info.color_type, ColorType::GrayscaleAlpha | ColorType::Rgba)
                || info.trns.is_some();
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
                has_alpha,
                orientation,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), options))
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        guard(FORMAT, || header_facts(source.bytes()))
    }
}

/// The chunks before the image data: IHDR, PLTE, acTL, iCCP, sRGB, eXIf.
/// Also used for PNG payloads inside ICO files.
pub(crate) fn header_facts(data: &[u8]) -> Result<ImageFacts> {
    let limits = png::Limits {
        bytes: util::decoder_memory_limit(),
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(data), limits);
    decoder.set_ignore_text_chunk(true);
    let reader = decoder.read_info().map_err(map_err)?;
    let info = reader.info();
    let mut f = ImageFacts::named(if info.animation_control.is_some() {
        "APNG"
    } else {
        "PNG"
    });
    f.compression = Some(
        if info.interlaced {
            "Deflate, Adam7 interlaced"
        } else {
            "Deflate"
        }
        .to_string(),
    );
    let bits = info.bit_depth as u8;
    match info.color_type {
        ColorType::Grayscale => f.set_channels("Gray", 1, bits),
        ColorType::GrayscaleAlpha => f.set_channels("Gray + alpha", 2, bits),
        ColorType::Rgb => f.set_channels("RGB", 3, bits),
        ColorType::Rgba => f.set_channels("RGBA", 4, bits),
        ColorType::Indexed => {
            let colors = info.palette.as_ref().map_or(0, |p| p.len() / 3);
            let name = if colors > 0 {
                format!("Indexed, {colors} colors")
            } else {
                "Indexed".to_string()
            };
            f.set_channels(name, 1, bits);
        }
    }
    f.icc_profile = info.icc_profile.as_deref().and_then(icc_description);
    if info.srgb.is_some() {
        f.color_space = Some("sRGB".to_string());
    }
    if let Some(exif) = info.exif_metadata.as_deref() {
        f.camera = camera_from_exif(exif);
    }
    Ok(f)
}

fn exif_orientation(info: &png::Info<'_>) -> Orientation {
    info.exif_metadata
        .as_deref()
        .map(orientation_from_exif)
        .unwrap_or_default()
}

/// Open a PNG stream. `strip16` reduces 16-bit samples to 8 bits, which
/// animation compositing needs.
fn open(data: &[u8], strip16: bool) -> Result<png::Reader<Cursor<&[u8]>>> {
    let limits = png::Limits {
        bytes: util::decoder_memory_limit(),
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(data), limits);
    let mut transform = Transformations::EXPAND;
    if strip16 {
        transform |= Transformations::STRIP_16;
    }
    decoder.set_transformations(transform);
    decoder.set_ignore_text_chunk(true);
    let reader = decoder.read_info().map_err(map_err)?;
    let info = reader.info();
    check_size(info.width, info.height, PixelFormat::Rgba8)?;
    Ok(reader)
}

fn map_err(e: png::DecodingError) -> pandite_core::Error {
    match e {
        png::DecodingError::LimitsExceeded => {
            pandite_core::Error::Corrupt(format!("{FORMAT}: image exceeds the memory limit"))
        }
        other => corrupt(FORMAT, other),
    }
}

/// Decode a PNG stream into upright content. Also used for PNG payloads
/// inside ICO files.
pub(crate) fn decode(data: &[u8], options: &DecodeOptions) -> Result<Content> {
    let mut reader = open(data, false)?;
    let (cw, ch) = (reader.info().width, reader.info().height);
    let orientation = exif_orientation(reader.info());
    let profile = reader.info().icc_profile.as_ref().map(|p| p.to_vec());
    let animation = reader.info().animation_control;
    let idat_is_frame = reader.info().frame_control.is_some();
    let sixteen = reader.output_color_type().1 == BitDepth::Sixteen;
    let animated = animation.is_some_and(|a| a.num_frames > 1 || !idat_is_frame);

    if !animated || options.first_frame_only {
        let len = reader
            .output_buffer_size()
            .ok_or_else(|| corrupt(FORMAT, "image too large for this system"))?;
        let mut buf = util::alloc(len, cw, ch)?;
        let out = read_still(&mut reader, &mut buf, options)?;
        let image = if sixteen {
            to_rgba16(&buf, &out, cw, ch, profile.as_deref())?
        } else {
            let mut rgba = util::alloc_rgba8(cw, ch)?;
            expand_rows(&buf, &out, out.line_size, &mut rgba, cw as usize);
            icc::to_srgb_rgba8(profile.as_deref(), &mut rgba);
            util::rgba8_image(cw, ch, rgba)?
        };
        return Ok(Content::Still(orient::apply(image, orientation)?));
    }

    // Frames are composed in 8 bits per channel.
    if sixteen {
        reader = open(data, true)?;
    }
    let len = reader
        .output_buffer_size()
        .ok_or_else(|| corrupt(FORMAT, "image too large for this system"))?;
    let mut buf = util::alloc(len, cw, ch)?;

    let animation = animation.expect("checked above");
    let total = animation
        .num_frames
        .saturating_add(u32::from(!idat_is_frame));
    let loop_count = match animation.num_plays {
        0 => LoopCount::Infinite,
        n => LoopCount::Finite(n),
    };
    let (cwu, chu) = (cw as usize, ch as usize);
    let mut canvas = util::alloc_rgba8(cw, ch)?;
    let mut frame_rgba = util::alloc_rgba8(cw, ch)?;
    let mut frames = FrameCollector::new();
    let interlaced = reader.info().interlaced;
    let full_line = reader
        .output_line_size(cw)
        .ok_or_else(|| corrupt(FORMAT, "bad line size"))?;

    for index in 0..total {
        options.cancel.check()?;
        let out = reader.next_frame(&mut buf).map_err(map_err)?;
        if index == 0 && !idat_is_frame {
            // The default image is only for decoders without APNG support.
            continue;
        }
        let fc = reader
            .info()
            .frame_control
            .ok_or_else(|| corrupt(FORMAT, "frame without fcTL"))?;
        let stride = if interlaced { full_line } else { out.line_size };
        let fw = out.width as usize;
        expand_rows(&buf, &out, stride, &mut frame_rgba, fw);
        icc::to_srgb_rgba8(
            profile.as_deref(),
            &mut frame_rgba[..fw * out.height as usize * 4],
        );

        let region = Region::clip(fc.x_offset, fc.y_offset, out.width, out.height, cwu, chu);
        let first = frames.len() == 0;
        let mut dispose = fc.dispose_op;
        if first && dispose == png::DisposeOp::Previous {
            dispose = png::DisposeOp::Background;
        }
        let saved =
            (dispose == png::DisposeOp::Previous).then(|| compose::save(&canvas, cwu, region));
        let blend = fc.blend_op == png::BlendOp::Over;
        compose::draw(&mut canvas, cwu, region, &frame_rgba, fw, blend);

        let delay = delay_of(fc.delay_num, fc.delay_den);
        let image = ImageBuf::new(cw, ch, PixelFormat::Rgba8, canvas.clone())?;
        let image = orient::apply(image, orientation)?;
        if !frames.push(image, delay) {
            break;
        }
        match dispose {
            png::DisposeOp::None => {}
            png::DisposeOp::Background => compose::clear(&mut canvas, cwu, region),
            png::DisposeOp::Previous => {
                if let Some(saved) = saved {
                    compose::restore(&mut canvas, cwu, region, &saved);
                }
            }
        }
    }
    frames.finish(loop_count)
}

/// Read the first image. Non-interlaced images are read row by row so a
/// cancel request takes effect within a few rows.
fn read_still(
    reader: &mut png::Reader<Cursor<&[u8]>>,
    buf: &mut [u8],
    options: &DecodeOptions,
) -> Result<png::OutputInfo> {
    options.cancel.check()?;
    if reader.info().interlaced {
        return reader.next_frame(buf).map_err(map_err);
    }
    let (w, h) = (reader.info().width, reader.info().height);
    let (color_type, bit_depth) = reader.output_color_type();
    let line = reader
        .output_line_size(w)
        .ok_or_else(|| corrupt(FORMAT, "bad line size"))?;
    for y in 0..h as usize {
        if y % 64 == 0 {
            options.cancel.check()?;
        }
        let row = reader
            .next_row()
            .map_err(map_err)?
            .ok_or_else(|| corrupt(FORMAT, "missing rows"))?;
        let data = row.data();
        let n = data.len().min(line);
        buf[y * line..y * line + n].copy_from_slice(&data[..n]);
    }
    Ok(png::OutputInfo {
        width: w,
        height: h,
        color_type,
        bit_depth,
        line_size: line,
    })
}

/// APNG frame delay as a duration. A zero denominator means 1/100 s.
fn delay_of(num: u16, den: u16) -> Duration {
    let den = if den == 0 { 100 } else { u64::from(den) };
    Duration::from_micros(u64::from(num) * 1_000_000 / den)
}

/// Expand decoded 8-bit rows (gray, gray+alpha, RGB or RGBA after the
/// EXPAND transform) into tightly packed RGBA8 rows of `width` pixels.
fn expand_rows(buf: &[u8], out: &png::OutputInfo, stride: usize, rgba: &mut [u8], width: usize) {
    let channels = out.color_type.samples();
    let row_bytes = out.width as usize * channels;
    for y in 0..out.height as usize {
        let src = &buf[y * stride..y * stride + row_bytes];
        let dst = &mut rgba[y * width * 4..(y * width + out.width as usize) * 4];
        util::expand_to_rgba8(src, channels, dst);
    }
}

/// Convert a 16-bit still to RGBA16.
fn to_rgba16(
    buf: &[u8],
    out: &png::OutputInfo,
    w: u32,
    h: u32,
    profile: Option<&[u8]>,
) -> Result<ImageBuf> {
    let channels = out.color_type.samples();
    let n = w as usize * h as usize;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(n * 4)
        .map_err(|_| pandite_core::Error::TooLarge {
            width: w,
            height: h,
        })?;
    for y in 0..h as usize {
        let row = &buf[y * out.line_size..y * out.line_size + w as usize * channels * 2];
        for px in row.chunks_exact(channels * 2) {
            let s = |k: usize| u16::from_be_bytes([px[2 * k], px[2 * k + 1]]);
            let rgba = match channels {
                1 => [s(0), s(0), s(0), u16::MAX],
                2 => [s(0), s(0), s(0), s(1)],
                3 => [s(0), s(1), s(2), u16::MAX],
                _ => [s(0), s(1), s(2), s(3)],
            };
            samples.extend_from_slice(&rgba);
        }
    }
    check_size(w, h, PixelFormat::Rgba16)?;
    icc::to_srgb_rgba16(profile, &mut samples);
    util::rgba16_image(w, h, &samples)
}

//! JPEG XL decoder on `jxl` (jxl-rs, the decoder used by Chromium and
//! Firefox). Reads bare codestreams and containers, still and animated
//! (coalesced full frames), gray and color, with alpha. 8-bit images decode
//! to `Rgba8`, deeper integer images to `Rgba16`, float images and images
//! with a linear, PQ or HLG transfer to linear `RgbaF32`. The codestream
//! orientation is applied by the decoder. Embedded ICC profiles convert to
//! sRGB through the `icc` module.

use crate::icc;
use crate::util::{self, check_size, corrupt, guard, FrameCollector};
use jxl::api::{
    AlphaMode, Endianness, Event, ExtraChannel, JxlBitDepth, JxlColorEncoding, JxlColorProfile,
    JxlColorType, JxlDataFormat, JxlDecoder as Decoder, JxlDecoderOptions, JxlOutputBuffer,
    JxlParallelRunner, JxlParallelRunnerFun, JxlPixelFormat, JxlPrimaries, JxlTransferFunction,
};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageFacts, ImageMeta,
    LoopCount, Orientation, PixelFormat, Result, Source,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const FORMAT: &str = "jxl";
/// Bytes handed to the decoder per step.
const CHUNK: usize = 1024 * 1024;
const CONTAINER: [u8; 12] = [
    0, 0, 0, 0x0C, b'J', b'X', b'L', b' ', 0x0D, 0x0A, 0x87, 0x0A,
];

/// Decodes JPEG XL files.
#[derive(Clone, Copy, Debug, Default)]
pub struct JxlDecoder;

impl DecoderProvider for JxlDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "JPEG XL image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["jxl"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(&[0xFF, 0x0A]) || header.starts_with(&CONTAINER)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let mut dec = Decoder::new(options(true));
            let mut input = source.bytes();
            loop {
                match dec
                    .process(&mut input, None, None)
                    .map_err(|e| corrupt(FORMAT, e))?
                {
                    Event::BasicInfo => break,
                    Event::NeedMoreInput { .. } if input.is_empty() => {
                        return Err(corrupt(FORMAT, "truncated header"))
                    }
                    Event::Complete => return Err(corrupt(FORMAT, "no image header")),
                    _ => {}
                }
            }
            let info = dec
                .basic_info()
                .ok_or_else(|| corrupt(FORMAT, "no basic info"))?;
            let (w, h) = info.size;
            Ok(ImageMeta {
                width: w as u32,
                height: h as u32,
                format: FORMAT,
                kind: if info.animation.is_some() {
                    ContentKind::Animation
                } else {
                    ContentKind::Still
                },
                frame_count: 1,
                has_alpha: info
                    .extra_channels
                    .iter()
                    .any(|c| c.ec_type == ExtraChannel::Alpha),
                orientation: Orientation::Normal,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), options))
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        guard(FORMAT, || {
            let data = source.bytes();
            let mut dec = Decoder::new(options(true));
            let mut input = data;
            loop {
                match dec
                    .process(&mut input, None, None)
                    .map_err(|e| corrupt(FORMAT, e))?
                {
                    Event::BasicInfo => break,
                    Event::NeedMoreInput { .. } if input.is_empty() => {
                        return Err(corrupt(FORMAT, "truncated header"))
                    }
                    Event::Complete => return Err(corrupt(FORMAT, "no image header")),
                    _ => {}
                }
            }
            let info = dec
                .basic_info()
                .ok_or_else(|| corrupt(FORMAT, "no basic info"))?;
            let mut f = ImageFacts::named("JPEG XL");
            if data.starts_with(&CONTAINER) && has_box(data, b"jbrd") {
                f.compression = Some("Recompressed JPEG".to_string());
            } else if !info.uses_original_profile {
                // XYB is the color space of lossy coding only.
                f.compression = Some("Lossy (XYB)".to_string());
            }
            let alpha = info
                .extra_channels
                .iter()
                .any(|c| c.ec_type == ExtraChannel::Alpha);
            let profile = dec.embedded_color_profile();
            let gray = matches!(
                profile,
                Some(JxlColorProfile::Simple(
                    JxlColorEncoding::GrayscaleColorSpace { .. }
                ))
            );
            let (name, count) = match (gray, alpha) {
                (true, false) => ("Gray", 1),
                (true, true) => ("Gray + alpha", 2),
                (false, false) => ("RGB", 3),
                (false, true) => ("RGBA", 4),
            };
            let bits = u8::try_from(info.bit_depth.bits_per_sample()).unwrap_or(0);
            let name = match info.bit_depth {
                JxlBitDepth::Float { .. } => format!("{name}, floating point"),
                _ => name.to_string(),
            };
            f.set_channels(name, count, bits);
            match profile {
                Some(JxlColorProfile::Icc(icc)) => {
                    f.icc_profile = crate::facts::icc_description(icc);
                }
                Some(JxlColorProfile::Simple(JxlColorEncoding::RgbColorSpace {
                    primaries,
                    transfer_function,
                    ..
                })) => {
                    f.color_space = color_space_name(primaries, transfer_function);
                }
                _ => {}
            }
            Ok(f)
        })
    }
}

/// A name for the common color encodings of the codestream header.
fn color_space_name(p: &JxlPrimaries, t: &JxlTransferFunction) -> Option<String> {
    let gamut = match p {
        JxlPrimaries::SRGB => "sRGB primaries",
        JxlPrimaries::P3 => "Display P3",
        JxlPrimaries::BT2100 => "BT.2100",
        JxlPrimaries::Chromaticities { .. } => return None,
    };
    Some(match (p, t) {
        (JxlPrimaries::SRGB, JxlTransferFunction::SRGB) => "sRGB".to_string(),
        (JxlPrimaries::P3, JxlTransferFunction::SRGB) => "Display P3".to_string(),
        (_, JxlTransferFunction::PQ) => format!("{gamut}, PQ"),
        (_, JxlTransferFunction::HLG) => format!("{gamut}, HLG"),
        (_, JxlTransferFunction::Linear) => format!("{gamut}, linear"),
        _ => gamut.to_string(),
    })
}

/// True when the top-level boxes of a container include `kind`.
fn has_box(data: &[u8], kind: &[u8; 4]) -> bool {
    let mut pos = 0usize;
    for _ in 0..1024 {
        let (Some(size), Some(k)) = (util::be32(data, pos), data.get(pos + 4..pos + 8)) else {
            return false;
        };
        if k == kind {
            return true;
        }
        let size = match size {
            0 => return false,
            1 => {
                let hi = util::be32(data, pos + 8).unwrap_or(0);
                let lo = util::be32(data, pos + 12).unwrap_or(0);
                match usize::try_from((u64::from(hi) << 32) | u64::from(lo)) {
                    Ok(s) => s,
                    Err(_) => return false,
                }
            }
            s => s as usize,
        };
        if size < 8 {
            return false;
        }
        pos = pos.saturating_add(size);
    }
    false
}

fn options(scan_only: bool) -> JxlDecoderOptions {
    let mut o = JxlDecoderOptions::default();
    o.alpha_mode = AlphaMode::Unpremultiplied;
    o.scan_frames_only = scan_only;
    o
}

/// Runs the decoder's parallel work on scoped threads. A panic in a task
/// resurfaces in the calling thread when the scope ends, where the panic
/// guard catches it.
struct ScopedRunner {
    threads: usize,
}

impl ScopedRunner {
    fn new() -> Self {
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get().min(8));
        ScopedRunner { threads }
    }
}

impl JxlParallelRunner for ScopedRunner {
    fn run(&mut self, num: usize, fun: &JxlParallelRunnerFun<'_>) -> jxl::error::Result<()> {
        if num <= 1 || self.threads <= 1 {
            for i in 0..num {
                fun(i)?;
            }
            return Ok(());
        }
        let next = AtomicUsize::new(0);
        let failed: Mutex<Option<jxl::error::Error>> = Mutex::new(None);
        std::thread::scope(|s| {
            for _ in 0..self.threads.min(num) {
                s.spawn(|| loop {
                    let task = next.fetch_add(1, Ordering::Relaxed);
                    if task >= num {
                        break;
                    }
                    if let Err(e) = fun(task) {
                        // Stop handing out work after the first error.
                        next.store(num, Ordering::Relaxed);
                        if let Ok(mut slot) = failed.lock() {
                            slot.get_or_insert(e);
                        }
                        break;
                    }
                });
            }
        });
        match failed.into_inner() {
            Ok(Some(e)) => Err(e),
            _ => Ok(()),
        }
    }

    fn num_threads(&self) -> usize {
        self.threads
    }
}

/// How decoded samples are stored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Out {
    U8,
    U16,
    F32,
}

/// Output layout chosen once the basic info is known.
struct Setup {
    w: u32,
    h: u32,
    spp: usize,
    out: Out,
    icc: Option<Vec<u8>>,
}

fn decode(data: &[u8], opts: &DecodeOptions) -> Result<Content> {
    let mut dec = Decoder::new(options(false));
    // Input is handed over in slices so control returns between them and a
    // cancel request takes effect part way through a frame.
    let (mut pos, mut end) = (0usize, data.len().min(CHUNK));
    let mut runner = ScopedRunner::new();
    let mut frames = FrameCollector::new();
    let mut setup: Option<Setup> = None;
    let mut buf32: Vec<f32> = Vec::new();
    let mut buf8: Vec<u8> = Vec::new();
    let mut loop_count = LoopCount::Infinite;
    let mut delay = Duration::ZERO;
    loop {
        opts.cancel.check()?;
        let event = {
            let mut bufs: Vec<JxlOutputBuffer> = Vec::new();
            if let Some(s) = &setup {
                let row = s.w as usize * s.spp;
                let rows = s.h as usize;
                match s.out {
                    Out::F32 => bufs.push(JxlOutputBuffer::new(
                        bytemuck::cast_slice_mut(&mut buf32[..]),
                        rows,
                        row * 4,
                    )),
                    Out::U16 => bufs.push(JxlOutputBuffer::new(&mut buf8[..], rows, row * 2)),
                    Out::U8 => bufs.push(JxlOutputBuffer::new(&mut buf8[..], rows, row)),
                }
            }
            let b = if bufs.is_empty() {
                None
            } else {
                Some(&mut bufs[..])
            };
            let mut input = &data[pos..end];
            let before = input.len();
            let event = dec
                .process(&mut input, b, Some(&mut runner))
                .map_err(|e| corrupt(FORMAT, e))?;
            pos += before - input.len();
            event
        };
        match event {
            Event::BasicInfo => {
                let info = dec
                    .basic_info()
                    .ok_or_else(|| corrupt(FORMAT, "no basic info"))?
                    .clone();
                let (w, h) = (info.size.0 as u32, info.size.1 as u32);
                check_size(w, h, PixelFormat::Rgba8)?;
                if let Some(a) = &info.animation {
                    loop_count = match a.num_loops {
                        0 => LoopCount::Infinite,
                        n => LoopCount::Finite(n),
                    };
                }
                let alpha = info
                    .extra_channels
                    .iter()
                    .any(|c| c.ec_type == ExtraChannel::Alpha);
                // Decide from the image's own profile: the default output
                // profile depends on the pixel format chosen below.
                let embedded = dec.embedded_color_profile().cloned();
                let gray = matches!(
                    &embedded,
                    Some(JxlColorProfile::Simple(
                        JxlColorEncoding::GrayscaleColorSpace { .. }
                    ))
                );
                let hdr_transfer = embedded
                    .as_ref()
                    .and_then(|p| p.transfer_function())
                    .is_some_and(|t| {
                        matches!(
                            t,
                            JxlTransferFunction::PQ
                                | JxlTransferFunction::HLG
                                | JxlTransferFunction::Linear
                        )
                    });
                let out = match info.bit_depth {
                    JxlBitDepth::Float { .. } => Out::F32,
                    _ if hdr_transfer => Out::F32,
                    JxlBitDepth::Int { bits_per_sample } if bits_per_sample > 8 => Out::U16,
                    _ => Out::U8,
                };
                let color_type = match (gray, alpha) {
                    (true, false) => JxlColorType::Grayscale,
                    (true, true) => JxlColorType::GrayscaleAlpha,
                    (false, false) => JxlColorType::Rgb,
                    (false, true) => JxlColorType::Rgba,
                };
                let e = Endianness::native();
                let format = match out {
                    Out::U8 => JxlDataFormat::U8 { bit_depth: 8 },
                    Out::U16 => JxlDataFormat::U16 {
                        endianness: e,
                        bit_depth: 16,
                    },
                    Out::F32 => JxlDataFormat::F32 { endianness: e },
                };
                dec.set_pixel_format(JxlPixelFormat {
                    color_type,
                    color_data_format: Some(format),
                    extra_channel_format: vec![None; info.extra_channels.len()],
                })
                .map_err(|e| corrupt(FORMAT, e))?;
                let spp = color_type.samples_per_pixel();
                let n = w as usize * h as usize * spp;
                let fail = |_| Error::TooLarge {
                    width: w,
                    height: h,
                };
                match out {
                    Out::F32 => {
                        buf32.try_reserve_exact(n).map_err(fail)?;
                        buf32.resize(n, 0.0);
                    }
                    Out::U16 => {
                        buf8.try_reserve_exact(n * 2).map_err(fail)?;
                        buf8.resize(n * 2, 0);
                    }
                    Out::U8 => {
                        buf8.try_reserve_exact(n).map_err(fail)?;
                        buf8.resize(n, 0);
                    }
                }
                // Pixels come out in the output profile; an ICC output
                // profile still needs conversion to sRGB.
                let icc = match dec.output_color_profile() {
                    Some(JxlColorProfile::Icc(bytes)) => Some(bytes.clone()),
                    _ => None,
                };
                setup = Some(Setup {
                    w,
                    h,
                    spp,
                    out,
                    icc,
                });
            }
            Event::FrameHeader => {
                delay = dec
                    .frame_header()
                    .and_then(|f| f.duration)
                    .map(|ms| Duration::from_secs_f64((ms / 1000.0).clamp(0.0, 3600.0)))
                    .unwrap_or(Duration::ZERO);
            }
            Event::FrameComplete { has_more_frames } => {
                let s = setup
                    .as_ref()
                    .ok_or_else(|| corrupt(FORMAT, "frame before header"))?;
                let profile = dec.output_color_profile().cloned();
                let image = to_image(s, &buf8, &buf32, profile.as_ref())?;
                if !frames.push(image, delay) || opts.first_frame_only || !has_more_frames {
                    break;
                }
            }
            Event::Complete => break,
            Event::NeedMoreInput { .. } => {
                if end == data.len() {
                    return Err(corrupt(FORMAT, "truncated data"));
                }
                end = (end + CHUNK).min(data.len());
            }
        }
    }
    if frames.len() == 0 {
        return Err(corrupt(FORMAT, "no frames"));
    }
    frames.finish(loop_count)
}

/// Convert one decoded frame to an image.
fn to_image(
    setup: &Setup,
    buf8: &[u8],
    buf32: &[f32],
    profile: Option<&JxlColorProfile>,
) -> Result<ImageBuf> {
    let (w, h, spp, out) = (setup.w, setup.h, setup.spp, setup.out);
    let icc_profile = setup.icc.as_deref();
    let n = w as usize * h as usize;
    let expand = |px: &[f32]| -> [f32; 4] {
        match spp {
            1 => [px[0], px[0], px[0], 1.0],
            2 => [px[0], px[0], px[0], px[1]],
            3 => [px[0], px[1], px[2], 1.0],
            _ => [px[0], px[1], px[2], px[3]],
        }
    };
    match out {
        Out::U8 => {
            let mut rgba = util::alloc_rgba8(w, h)?;
            util::expand_to_rgba8(&buf8[..n * spp], spp, &mut rgba);
            if spp >= 3 {
                icc::to_srgb_rgba8(icc_profile, &mut rgba);
            }
            util::rgba8_image(w, h, rgba)
        }
        Out::U16 => {
            let len = check_size(w, h, PixelFormat::Rgba16)?;
            let mut data = util::alloc(len, w, h)?;
            let samples = buf8[..n * spp * 2]
                .chunks_exact(2)
                .map(|c| u16::from_ne_bytes([c[0], c[1]]));
            util::expand16_to_rgba16(samples, spp, &mut data);
            if spp >= 3 && icc_profile.is_some() {
                let mut v: Vec<u16> = data
                    .chunks_exact(2)
                    .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                    .collect();
                icc::to_srgb_rgba16(icc_profile, &mut v);
                return util::rgba16_image(w, h, &v);
            }
            ImageBuf::new(w, h, PixelFormat::Rgba16, data)
        }
        Out::F32 => {
            let transfer = profile.and_then(|p| p.transfer_function());
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
                width: w,
                height: h,
            })?;
            for px in buf32[..n * spp].chunks_exact(spp) {
                let mut p = expand(px);
                for c in &mut p[..3] {
                    *c = to_linear(*c, transfer);
                }
                rgba.extend_from_slice(&p);
            }
            util::rgbaf32_image(w, h, &rgba)
        }
    }
}

/// Map a sample in the output transfer function to linear light, with
/// SDR reference white (203 nits for PQ) at 1.0.
fn to_linear(v: f32, transfer: Option<&JxlTransferFunction>) -> f32 {
    match transfer {
        Some(JxlTransferFunction::Linear) | None => v,
        Some(JxlTransferFunction::SRGB) => {
            if v < 0.0 {
                -util::srgb_to_linear(-v)
            } else {
                util::srgb_to_linear(v)
            }
        }
        Some(JxlTransferFunction::PQ) => {
            let (m1, m2) = (0.159_301_76_f32, 78.843_75_f32);
            let (c1, c2, c3) = (0.835_937_5_f32, 18.851_562_f32, 18.687_5_f32);
            let p = v.clamp(0.0, 1.0).powf(1.0 / m2);
            let nits = 10000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1);
            nits / 203.0
        }
        Some(JxlTransferFunction::HLG) => {
            let (a, b, c) = (0.178_832_77_f32, 0.284_668_92_f32, 0.559_910_7_f32);
            let v = v.clamp(0.0, 1.0);
            let scene = if v <= 0.5 {
                v * v / 3.0
            } else {
                (((v - c) / a).exp() + b) / 12.0
            };
            // Nominal 1000 nit display with reference white at 203 nits.
            scene * 1000.0 / 203.0
        }
        Some(JxlTransferFunction::BT709) => {
            let v = v.max(0.0);
            if v < 0.081 {
                v / 4.5
            } else {
                ((v + 0.099) / 1.099).powf(1.0 / 0.45)
            }
        }
        Some(JxlTransferFunction::Gamma(g)) => v.max(0.0).powf(1.0 / g.max(0.01)),
        Some(JxlTransferFunction::DCI) => v.max(0.0).powf(2.6),
    }
}

//! TIFF decoder on the `tiff` crate, plus a small palette reader.
//!
//! Handles gray (1 to 16 bits, either polarity), gray with alpha, RGB and
//! RGBA (8, 16, 32-bit integer and 16, 32, 64-bit float), CMYK, YCbCr in
//! JPEG-compressed files, planar and chunky layouts, strips and tiles, and
//! LZW, Deflate, PackBits, JPEG and CCITT compression. Premultiplied alpha
//! is undone, the orientation tag and ICC profile apply. 16-bit integer
//! images decode to `Rgba16`, floating point images to linear `RgbaF32`.
//! Palette images, which the `tiff` crate rejects, are read here.
//!
//! Camera RAW files share the TIFF signature; they are handed to the RAW
//! decoder when that feature is compiled.

use crate::exif::{tag, Tiff};
use crate::jpeg::oriented;
use crate::util::{self, check_size, corrupt, guard, unsupported};
use crate::{icc, orient};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageFacts, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};
use std::io::{Cursor, Read};
use tiff::decoder::{Decoder, DecodingResult, Limits};
use tiff::tags::Tag;
use tiff::ColorType;

const FORMAT: &str = "tiff";

/// Decodes TIFF files (first page).
#[derive(Clone, Copy, Debug, Default)]
pub struct TiffDecoder;

/// True for the four TIFF and BigTIFF signatures.
pub(crate) fn is_tiff_signature(h: &[u8]) -> bool {
    h.starts_with(b"II*\0")
        || h.starts_with(b"MM\0*")
        || h.starts_with(b"II+\0")
        || h.starts_with(b"MM\0+")
}

impl DecoderProvider for TiffDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "TIFF image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["tif", "tiff"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        is_tiff_signature(header)
    }

    fn reduced_decode(&self, source: &Source) -> bool {
        // Camera RAW files in a TIFF structure are handed to the RAW
        // decoder, which uses their embedded previews.
        #[cfg(feature = "raw")]
        if crate::raw::is_camera_raw(source) {
            return crate::raw::RawDecoder.reduced_decode(source);
        }
        let _ = source;
        false
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        #[cfg(feature = "raw")]
        if crate::raw::is_camera_raw(source) {
            return crate::raw::RawDecoder.probe(source);
        }
        guard(FORMAT, || {
            let mut dec = open(source.bytes())?;
            let (w, h) = dec.dimensions().map_err(|e| corrupt(FORMAT, e))?;
            let orientation = orientation(&mut dec);
            let (width, height) = oriented(w, h, orientation);
            let has_alpha = matches!(
                dec.colortype(),
                Ok(ColorType::RGBA(_) | ColorType::GrayA(_) | ColorType::CMYKA(_))
            ) || dec
                .find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
                .ok()
                .flatten()
                == Some(2);
            Ok(ImageMeta {
                width,
                height,
                format: FORMAT,
                kind: ContentKind::Still,
                frame_count: 1,
                has_alpha,
                orientation,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        #[cfg(feature = "raw")]
        if crate::raw::is_camera_raw(source) {
            return crate::raw::RawDecoder.decode(source, options);
        }
        let image = guard(FORMAT, || decode(source.bytes(), options))?;
        Ok(Content::Still(image))
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        #[cfg(feature = "raw")]
        if crate::raw::is_camera_raw(source) {
            return crate::raw::RawDecoder.facts(source);
        }
        let d = source.bytes();
        if d.starts_with(b"II+\0") || d.starts_with(b"MM\0+") {
            return Ok(ImageFacts::named("BigTIFF"));
        }
        let t = Tiff::new(d).ok_or_else(|| corrupt(FORMAT, "not a TIFF file"))?;
        Ok(header_facts(&t))
    }
}

/// The facts of the first directory: compression and predictor, sample
/// layout, chroma subsampling, ICC profile, and the EXIF camera settings.
fn header_facts(t: &Tiff<'_>) -> ImageFacts {
    let mut f = ImageFacts::named("TIFF");
    f.camera = crate::facts::camera(t);
    let Some((e, _)) = t.first_ifd().and_then(|o| t.ifd(o)) else {
        return f;
    };
    let compression = t.get(&e, tag::COMPRESSION).unwrap_or(1);
    f.compression = crate::facts::tiff_compression(compression).map(|name| {
        match (t.get(&e, 317), compression) {
            (Some(2), 5 | 8 | 32946 | 34925 | 50000) => format!("{name}, horizontal predictor"),
            (Some(3), _) => format!("{name}, floating point predictor"),
            _ => name.to_string(),
        }
    });
    let spp = t.get(&e, 277).unwrap_or(1);
    let bits = t.get(&e, 258).unwrap_or(1);
    let base = match t.get(&e, tag::PHOTOMETRIC) {
        Some(0 | 1) => Some(("Gray", 1)),
        Some(2) => Some(("RGB", 3)),
        Some(3) => Some(("Indexed", 1)),
        Some(5) => Some(("CMYK", 4)),
        Some(6) => Some(("YCbCr", 3)),
        Some(8) => Some(("CIELab", 3)),
        _ => None,
    };
    if let Some((name, color)) = base {
        let name = match (name, spp.saturating_sub(color)) {
            ("RGB", 1) => "RGBA".to_string(),
            (_, 0) => name.to_string(),
            (_, 1) => format!("{name} + alpha"),
            (_, n) => format!("{name} + {n} extra"),
        };
        f.channels = Some(if t.get(&e, 339) == Some(3) {
            format!("{name}, floating point")
        } else {
            name
        });
    }
    f.channel_count = u8::try_from(spp).ok().filter(|&n| n > 0);
    f.bits_per_channel = u8::try_from(bits).ok().filter(|&b| b > 0);
    if t.get(&e, tag::PHOTOMETRIC) == Some(6) {
        if let [h, v] = t.values(&e, 530, 2)[..] {
            f.subsampling = u8::try_from(h)
                .ok()
                .zip(u8::try_from(v).ok())
                .and_then(|(h, v)| crate::facts::subsampling(h, v))
                .map(str::to_string);
        }
    }
    f.icc_profile = e
        .iter()
        .find(|x| x.tag == 34675)
        .and_then(|x| t.bytes(x))
        .and_then(crate::facts::icc_description);
    f
}

fn open(data: &[u8]) -> Result<Decoder<Cursor<&[u8]>>> {
    let mut limits = Limits::default();
    limits.decoding_buffer_size = util::decoder_memory_limit();
    limits.intermediate_buffer_size = util::decoder_memory_limit();
    limits.ifd_value_size = 64 * 1024 * 1024;
    Ok(Decoder::new(Cursor::new(data))
        .map_err(|e| corrupt(FORMAT, e))?
        .with_limits(limits))
}

fn orientation(dec: &mut Decoder<Cursor<&[u8]>>) -> Orientation {
    dec.find_tag_unsigned::<u16>(Tag::Orientation)
        .ok()
        .flatten()
        .map(Orientation::from_exif)
        .unwrap_or_default()
}

fn map_err(e: tiff::TiffError) -> Error {
    match e {
        tiff::TiffError::UnsupportedError(u) => unsupported(FORMAT, u),
        tiff::TiffError::LimitsExceeded => corrupt(FORMAT, "image exceeds the memory limit"),
        other => corrupt(FORMAT, other),
    }
}

/// Interleaved samples, `spp` per pixel.
enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

fn decode(data: &[u8], options: &DecodeOptions) -> Result<ImageBuf> {
    let mut dec = open(data)?;
    let (w, h) = dec.dimensions().map_err(map_err)?;
    check_size(w, h, PixelFormat::Rgba8)?;
    let orientation = orientation(&mut dec);
    let photometric = dec
        .find_tag_unsigned::<u16>(Tag::PhotometricInterpretation)
        .ok()
        .flatten();
    if photometric == Some(3) {
        let image = palette::decode(data, w, h)?;
        return orient::apply(image, orientation);
    }
    let profile = dec
        .find_tag(Tag::IccProfile)
        .ok()
        .flatten()
        .and_then(|v| v.into_u8_vec().ok());
    let color = dec.colortype().map_err(map_err)?;
    let spp = usize::from(
        dec.find_tag_unsigned::<u16>(Tag::SamplesPerPixel)
            .ok()
            .flatten()
            .unwrap_or(1)
            .max(1),
    );
    let extra = dec
        .find_tag_unsigned_vec::<u16>(Tag::ExtraSamples)
        .ok()
        .flatten()
        .unwrap_or_default();
    let compression = dec
        .find_tag_unsigned::<u16>(Tag::Compression)
        .ok()
        .flatten()
        .unwrap_or(1);
    let bits = color.bit_depth();

    options.cancel.check()?;
    let mut result = DecodingResult::U8(Vec::new());
    let layout = dec.read_image_to_buffer(&mut result).map_err(map_err)?;
    options.cancel.check()?;
    let planar = layout.planes > 1;
    if result.as_buffer(0).byte_len() < layout.complete_len {
        return Err(Error::TooLarge {
            width: w,
            height: h,
        });
    }
    let n = w as usize * h as usize;
    let samples = to_samples(result, bits, spp, w as usize, h as usize, planar)?;
    if samples_len(&samples) < n * spp {
        return Err(corrupt(FORMAT, "too few samples"));
    }

    // Channel layout: (gray or color kind, index of alpha, premultiplied).
    let assoc = extra.first() == Some(&1);
    let kind = match color {
        ColorType::Gray(_) | ColorType::Multiband { .. } | ColorType::GrayA(_) => Kind::Gray,
        ColorType::RGB(_) | ColorType::RGBA(_) => Kind::Rgb,
        ColorType::YCbCr(_) if compression == 7 => Kind::Rgb,
        ColorType::YCbCr(_) => Kind::YCbCr,
        ColorType::CMYK(_) | ColorType::CMYKA(_) => Kind::Cmyk,
        other => return Err(unsupported(FORMAT, format!("color type {other:?}"))),
    };
    if spp < kind.channels() {
        return Err(corrupt(FORMAT, "too few samples per pixel"));
    }
    let alpha = (spp > kind.channels()
        && (extra.first().is_some_and(|&e| e == 1 || e == 2)
            || matches!(
                color,
                ColorType::RGBA(_) | ColorType::CMYKA(_) | ColorType::GrayA(_)
            )
            || (kind == Kind::Gray && spp == 2)))
        .then_some(kind.channels());

    let image = match samples {
        Samples::U8(s) => {
            let mut out = util::alloc_rgba8(w, h)?;
            if kind == Kind::Cmyk {
                let mut cmyk = Vec::with_capacity(n * 4);
                for px in s.chunks_exact(spp) {
                    cmyk.extend_from_slice(&px[..4]);
                }
                if !icc::cmyk_to_srgb_rgba8(profile.as_deref(), &cmyk, &mut out) {
                    for (o, p) in out.chunks_exact_mut(4).zip(cmyk.chunks_exact(4)) {
                        let [r, g, b] = util::cmyk_to_rgb(p[0], p[1], p[2], p[3]);
                        o.copy_from_slice(&[r, g, b, 255]);
                    }
                }
                if let Some(ai) = alpha {
                    for (o, px) in out.chunks_exact_mut(4).zip(s.chunks_exact(spp)) {
                        o[3] = px[ai];
                    }
                }
            } else {
                for (o, px) in out.chunks_exact_mut(4).zip(s.chunks_exact(spp)) {
                    let a = alpha.map_or(255, |i| px[i]);
                    let [r, g, b] = match kind {
                        Kind::Gray => [px[0]; 3],
                        Kind::YCbCr => ycbcr(px[0], px[1], px[2]),
                        _ => [px[0], px[1], px[2]],
                    };
                    o.copy_from_slice(&[r, g, b, a]);
                    if assoc && alpha.is_some() {
                        unpremultiply8(o);
                    }
                }
                if kind != Kind::Gray {
                    icc::to_srgb_rgba8(profile.as_deref(), &mut out);
                }
            }
            util::rgba8_image(w, h, out)?
        }
        Samples::U16(s) => {
            if kind == Kind::Cmyk {
                // Reduce to 8 bits and reuse the 8-bit path.
                let mut out = util::alloc_rgba8(w, h)?;
                for (o, px) in out.chunks_exact_mut(4).zip(s.chunks_exact(spp)) {
                    let b = |v: u16| (v >> 8) as u8;
                    let [r, g, bl] = util::cmyk_to_rgb(b(px[0]), b(px[1]), b(px[2]), b(px[3]));
                    let a = alpha.map_or(255, |i| b(px[i]));
                    o.copy_from_slice(&[r, g, bl, a]);
                }
                util::rgba8_image(w, h, out)?
            } else {
                let mut rgba = Vec::new();
                rgba.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
                    width: w,
                    height: h,
                })?;
                for px in s.chunks_exact(spp) {
                    let a = alpha.map_or(u16::MAX, |i| px[i]);
                    let mut p = match kind {
                        Kind::Gray => [px[0], px[0], px[0], a],
                        _ => [px[0], px[1], px[2], a],
                    };
                    if assoc && alpha.is_some() && a != 0 && a != u16::MAX {
                        for c in &mut p[..3] {
                            *c = ((u32::from(*c) * 65535 + u32::from(a) / 2) / u32::from(a))
                                .min(65535) as u16;
                        }
                    }
                    rgba.extend_from_slice(&p);
                }
                if kind != Kind::Gray {
                    icc::to_srgb_rgba16(profile.as_deref(), &mut rgba);
                }
                util::rgba16_image(w, h, &rgba)?
            }
        }
        Samples::F32(s) => {
            if kind == Kind::Cmyk || kind == Kind::YCbCr {
                return Err(unsupported(FORMAT, "floating point CMYK or YCbCr"));
            }
            let mut rgba = Vec::new();
            rgba.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
                width: w,
                height: h,
            })?;
            for px in s.chunks_exact(spp) {
                let a = alpha.map_or(1.0, |i| px[i]);
                let mut p = match kind {
                    Kind::Gray => [px[0], px[0], px[0], a],
                    _ => [px[0], px[1], px[2], a],
                };
                if assoc && alpha.is_some() && a > 0.0 {
                    for c in &mut p[..3] {
                        *c /= a;
                    }
                }
                rgba.extend_from_slice(&p);
            }
            util::rgbaf32_image(w, h, &rgba)?
        }
    };
    orient::apply(image, orientation)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Gray,
    Rgb,
    YCbCr,
    Cmyk,
}

impl Kind {
    fn channels(self) -> usize {
        match self {
            Kind::Gray => 1,
            Kind::Rgb | Kind::YCbCr => 3,
            Kind::Cmyk => 4,
        }
    }
}

fn samples_len(s: &Samples) -> usize {
    match s {
        Samples::U8(v) => v.len(),
        Samples::U16(v) => v.len(),
        Samples::F32(v) => v.len(),
    }
}

/// BT.601 full-range YCbCr (the TIFF default coefficients) to RGB.
fn ycbcr(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    let (y, cb, cr) = (f32::from(y), f32::from(cb) - 128.0, f32::from(cr) - 128.0);
    let c = |v: f32| v.round().clamp(0.0, 255.0) as u8;
    [
        c(y + 1.402 * cr),
        c(y - 0.344_136 * cb - 0.714_136 * cr),
        c(y + 1.772 * cb),
    ]
}

fn unpremultiply8(p: &mut [u8]) {
    let a = u32::from(p[3]);
    if a == 0 || a == 255 {
        return;
    }
    for c in &mut p[..3] {
        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
    }
}

/// Turn a decoding result into interleaved samples, unpacking sub-byte
/// depths and interleaving planar data.
fn to_samples(
    result: DecodingResult,
    bits: u8,
    spp: usize,
    w: usize,
    h: usize,
    planar: bool,
) -> Result<Samples> {
    let n = w * h;
    // Source position of interleaved sample `i`.
    let order = |i: usize| -> usize {
        if planar {
            let (p, s) = (i / spp, i % spp);
            s * n + p
        } else {
            i
        }
    };
    Ok(match result {
        DecodingResult::U8(v) => {
            if bits < 8 {
                if !matches!(bits, 1 | 2 | 4) {
                    return Err(unsupported(FORMAT, format!("{bits}-bit samples")));
                }
                let max = (1u32 << bits) - 1;
                let per_row_samples = if planar { w } else { w * spp };
                let row_bytes = (per_row_samples * usize::from(bits)).div_ceil(8);
                let rows = if planar { h * spp } else { h };
                let mut unpacked = Vec::with_capacity(per_row_samples * rows);
                for r in 0..rows {
                    let row = v
                        .get(r * row_bytes..(r + 1) * row_bytes)
                        .ok_or_else(|| corrupt(FORMAT, "short packed data"))?;
                    for s in 0..per_row_samples {
                        let bit = s * usize::from(bits);
                        let byte = row[bit / 8];
                        let shift = 8 - usize::from(bits) - bit % 8;
                        let val = (u32::from(byte) >> shift) & max;
                        unpacked.push(util::scale_to_u8(val, max));
                    }
                }
                Samples::U8((0..n * spp).map(|i| unpacked[order(i)]).collect())
            } else if planar {
                Samples::U8(
                    (0..n * spp)
                        .map(|i| v.get(order(i)).copied().unwrap_or(0))
                        .collect(),
                )
            } else {
                Samples::U8(v)
            }
        }
        DecodingResult::I8(v) => Samples::U8(
            (0..n * spp)
                .map(|i| (i16::from(v.get(order(i)).copied().unwrap_or(0)) + 128) as u8)
                .collect(),
        ),
        DecodingResult::U16(v) => {
            let scale = if bits < 16 && bits > 0 {
                Some((1u32 << bits) - 1)
            } else {
                None
            };
            Samples::U16(
                (0..n * spp)
                    .map(|i| {
                        let s = v.get(order(i)).copied().unwrap_or(0);
                        scale.map_or(s, |m| util::scale_to_u16(u32::from(s), m))
                    })
                    .collect(),
            )
        }
        DecodingResult::I16(v) => Samples::U16(
            (0..n * spp)
                .map(|i| (i32::from(v.get(order(i)).copied().unwrap_or(0)) + 32768) as u16)
                .collect(),
        ),
        DecodingResult::U32(v) => Samples::U16(
            (0..n * spp)
                .map(|i| (v.get(order(i)).copied().unwrap_or(0) >> 16) as u16)
                .collect(),
        ),
        DecodingResult::I32(v) => Samples::U16(
            (0..n * spp)
                .map(|i| {
                    ((i64::from(v.get(order(i)).copied().unwrap_or(0)) + (1 << 31)) >> 16) as u16
                })
                .collect(),
        ),
        DecodingResult::U64(v) => Samples::U16(
            (0..n * spp)
                .map(|i| (v.get(order(i)).copied().unwrap_or(0) >> 48) as u16)
                .collect(),
        ),
        DecodingResult::I64(v) => Samples::U16(
            (0..n * spp)
                .map(|i| {
                    ((i128::from(v.get(order(i)).copied().unwrap_or(0)) + (1i128 << 63)) >> 48)
                        as u16
                })
                .collect(),
        ),
        DecodingResult::F16(v) => Samples::F32(
            (0..n * spp)
                .map(|i| v.get(order(i)).map_or(0.0, |x| x.to_f32()))
                .collect(),
        ),
        DecodingResult::F32(v) => Samples::F32(
            (0..n * spp)
                .map(|i| v.get(order(i)).copied().unwrap_or(0.0))
                .collect(),
        ),
        DecodingResult::F64(v) => Samples::F32(
            (0..n * spp)
                .map(|i| v.get(order(i)).copied().unwrap_or(0.0) as f32)
                .collect(),
        ),
    })
}

/// Palette (photometric 3) images in classic TIFF files: strips with no,
/// LZW, Deflate or PackBits compression and 1 to 8-bit indices.
mod palette {
    use super::*;

    pub(super) fn decode(data: &[u8], w: u32, h: u32) -> Result<ImageBuf> {
        let t = Tiff::new(data).ok_or_else(|| unsupported(FORMAT, "palette BigTIFF"))?;
        let first = t
            .first_ifd()
            .ok_or_else(|| corrupt(FORMAT, "missing directory"))?;
        let (e, _) = t
            .ifd(first)
            .ok_or_else(|| corrupt(FORMAT, "bad directory"))?;
        let bits = t.get(&e, 258).unwrap_or(1);
        if !matches!(bits, 1 | 2 | 4 | 8) {
            return Err(unsupported(FORMAT, format!("{bits}-bit palette")));
        }
        if e.iter().any(|x| x.tag == 322) {
            return Err(unsupported(FORMAT, "tiled palette image"));
        }
        let compression = t.get(&e, tag::COMPRESSION).unwrap_or(1);
        let predictor = t.get(&e, 317).unwrap_or(1);
        let map = t.values(&e, 320, 3 << 8);
        let colors = 1usize << bits;
        if map.len() < 3 * colors {
            return Err(corrupt(FORMAT, "short color map"));
        }
        let palette: Vec<[u8; 4]> = (0..colors)
            .map(|i| {
                let c = |k: usize| (map[k * colors + i] >> 8) as u8;
                [c(0), c(1), c(2), 255]
            })
            .collect();
        let offsets = t.values(&e, tag::STRIP_OFFSETS, 1 << 20);
        let counts = t.values(&e, tag::STRIP_BYTE_COUNTS, 1 << 20);
        let rows_per_strip = t.get(&e, 278).unwrap_or(h).max(1) as usize;
        let (wu, hu) = (w as usize, h as usize);
        let row_bytes = (wu * bits as usize).div_ceil(8);
        let mut indices = Vec::with_capacity(row_bytes * hu);
        for (i, &off) in offsets.iter().enumerate() {
            let rows = rows_per_strip.min(hu.saturating_sub(i * rows_per_strip));
            if rows == 0 {
                break;
            }
            let count = counts.get(i).copied().unwrap_or(0) as usize;
            let raw = data
                .get(off as usize..(off as usize).saturating_add(count))
                .ok_or_else(|| corrupt(FORMAT, "strip outside the file"))?;
            let want = rows * row_bytes;
            let mut strip = match compression {
                1 => raw.to_vec(),
                5 => lzw(raw, want)?,
                8 | 32946 => {
                    let mut out = Vec::with_capacity(want);
                    flate2::read::ZlibDecoder::new(raw)
                        .take(want as u64)
                        .read_to_end(&mut out)
                        .map_err(|e| corrupt(FORMAT, e))?;
                    out
                }
                32773 => packbits(raw, want),
                c => return Err(unsupported(FORMAT, format!("palette compression {c}"))),
            };
            if strip.len() < want {
                return Err(corrupt(FORMAT, "truncated strip"));
            }
            strip.truncate(want);
            if predictor == 2 && bits == 8 {
                for row in strip.chunks_exact_mut(row_bytes) {
                    for x in 1..row.len() {
                        row[x] = row[x].wrapping_add(row[x - 1]);
                    }
                }
            }
            indices.extend_from_slice(&strip);
        }
        if indices.len() < row_bytes * hu {
            return Err(corrupt(FORMAT, "missing strips"));
        }
        let mut out = util::alloc_rgba8(w, h)?;
        let per_byte = 8 / bits as usize;
        let mask = ((1u16 << bits) - 1) as u8;
        for y in 0..hu {
            let row = &indices[y * row_bytes..(y + 1) * row_bytes];
            for x in 0..wu {
                let shift = 8 - bits as usize * (x % per_byte + 1);
                let idx = usize::from((row[x / per_byte] >> shift) & mask);
                out[(y * wu + x) * 4..(y * wu + x) * 4 + 4].copy_from_slice(&palette[idx]);
            }
        }
        util::rgba8_image(w, h, out)
    }

    /// LZW decoding into a buffer of exactly `want` bytes, so a hostile
    /// strip cannot expand beyond the image it belongs to.
    fn lzw(src: &[u8], want: usize) -> Result<Vec<u8>> {
        let mut out = vec![0u8; want];
        let mut dec = weezl::decode::Decoder::with_tiff_size_switch(weezl::BitOrder::Msb, 8);
        let (mut filled, mut input) = (0usize, src);
        while filled < want {
            let r = dec.decode_bytes(input, &mut out[filled..]);
            filled += r.consumed_out;
            input = &input[r.consumed_in..];
            match r.status {
                // Buffered output can still drain after all input is read.
                Ok(weezl::LzwStatus::Ok) if r.consumed_in + r.consumed_out > 0 => {}
                Ok(_) => break,
                Err(e) => return Err(corrupt(FORMAT, e)),
            }
        }
        out.truncate(filled);
        Ok(out)
    }

    /// PackBits run-length decoding, stopping at `want` bytes.
    pub(super) fn packbits(src: &[u8], want: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(want);
        let mut i = 0;
        while i < src.len() && out.len() < want {
            let n = src[i] as i8;
            i += 1;
            if n >= 0 {
                let len = n as usize + 1;
                let end = (i + len).min(src.len());
                out.extend_from_slice(&src[i..end]);
                i = end;
            } else if n != -128 {
                let len = (1 - isize::from(n)) as usize;
                if let Some(&b) = src.get(i) {
                    out.extend(std::iter::repeat_n(b, len));
                }
                i += 1;
            }
        }
        out
    }
}

//! JPEG decoder: `zune-jpeg` for full-size decodes, `jpeg-decoder` for
//! DCT-scaled decodes and the rarer formats, and the camera's embedded
//! previews for fast reduced decodes.
//!
//! - Full size, 8-bit baseline or progressive, YCbCr or grayscale: decoded
//!   by `zune-jpeg`, whose Huffman decoding, IDCT, upsampling and color
//!   conversion use SSE, AVX2 or NEON as the CPU allows. Any failure there
//!   retries with `jpeg-decoder`, so the result never depends on which
//!   decoder is more lenient with a damaged file.
//! - CMYK, YCCK, RGB-transform and 16-bit lossless files, and every
//!   decode with `DecodeOptions::target_size`: `jpeg-decoder`, which
//!   scales in the DCT to 1/2, 1/4 or 1/8 (the smallest scale that still
//!   covers the target).
//! - With a target size, an embedded preview that covers it is decoded
//!   instead: the Multi-Picture Format preview cameras append (Sony stores
//!   a 1616 x 1080 copy after the main image) or the EXIF thumbnail. Only
//!   a preview with the main image's aspect ratio is used.
//!
//! EXIF orientation is applied, embedded ICC profiles are converted to
//! sRGB (CMYK through the profile when there is one).

use crate::exif::{orientation_from_exif, Tiff};
use crate::facts::{camera_from_exif, exif_color_space, icc_description, subsampling};
use crate::util::{alloc_rgba8, check_size, cmyk_to_rgb, corrupt, guard, scale_to_u16};
use crate::{icc, orient, util};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageFacts, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "jpeg";

/// Facts read from the JPEG marker stream without decoding pixels.
#[derive(Debug, Default)]
pub(crate) struct Header<'a> {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub precision: u8,
    /// Frame marker: 0xC0 to 0xCF.
    pub sof: u8,
    pub exif: Option<&'a [u8]>,
    /// Offset of the EXIF TIFF header in the stream.
    pub exif_at: Option<usize>,
    /// Offset of the Multi-Picture Format header (its byte order mark) in
    /// the stream.
    pub mpf_at: Option<usize>,
    /// An Adobe APP14 segment with color transform 0 (RGB or CMYK as
    /// stored, no YCbCr conversion).
    pub adobe_untransformed: bool,
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
                header.exif_at = Some(pos + 2 + 6);
            }
            0xE2 if payload.starts_with(b"MPF\0") && header.mpf_at.is_none() => {
                header.mpf_at = Some(pos + 2 + 4);
            }
            0xEE if payload.starts_with(b"Adobe") && payload.len() >= 12 => {
                header.adobe_untransformed = payload[11] == 0;
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                header.sof = marker;
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

    fn reduced_decode(&self, _source: &Source) -> bool {
        true
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let image = guard(FORMAT, || decode_image(source.bytes(), options))?;
        Ok(Content::Still(image))
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        header_facts(source.bytes()).ok_or_else(|| corrupt(FORMAT, "not a JPEG stream"))
    }
}

/// The name of a frame marker's coding process (ITU-T T.81 Table B.1).
fn coding_name(sof: u8) -> &'static str {
    match sof {
        0xC0 => "Baseline DCT, Huffman",
        0xC1 => "Extended sequential DCT, Huffman",
        0xC2 => "Progressive DCT, Huffman",
        0xC3 => "Lossless, Huffman",
        0xC5 => "Differential sequential DCT, Huffman",
        0xC6 => "Differential progressive DCT, Huffman",
        0xC7 => "Differential lossless, Huffman",
        0xC9 => "Extended sequential DCT, arithmetic",
        0xCA => "Progressive DCT, arithmetic",
        0xCB => "Lossless, arithmetic",
        0xCD => "Differential sequential DCT, arithmetic",
        0xCE => "Differential progressive DCT, arithmetic",
        _ => "Differential lossless, arithmetic",
    }
}

/// The facts of the marker segments before the first scan: coding
/// process, components with their sampling, precision, the ICC profile
/// (APP2, in pieces), the Adobe transform (APP14) and EXIF (APP1). `None`
/// when the data do not start a JPEG stream. A stream cut short reports
/// what came before the cut.
pub(crate) fn header_facts(data: &[u8]) -> Option<ImageFacts> {
    if data.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut icc: Vec<(u8, &[u8])> = Vec::new();
    let mut exif: Option<&[u8]> = None;
    let mut adobe: Option<u8> = None;
    let mut frame: Option<(u8, &[u8])> = None;
    let mut pos = 2;
    'walk: loop {
        loop {
            match data.get(pos) {
                Some(0xFF) => break,
                Some(_) => pos += 1,
                None => break 'walk,
            }
        }
        while data.get(pos) == Some(&0xFF) {
            pos += 1;
        }
        let Some(&marker) = data.get(pos) else {
            break;
        };
        pos += 1;
        match marker {
            0x01 | 0xD0..=0xD7 => continue,
            0xD9 | 0xDA => break,
            _ => {}
        }
        let Some(len) = util::be16(data, pos).map(usize::from).filter(|&l| l >= 2) else {
            break;
        };
        let Some(payload) = data.get(pos + 2..pos + len) else {
            break;
        };
        match marker {
            0xE1 if exif.is_none() && payload.starts_with(b"Exif\0\0") => {
                exif = Some(&payload[6..]);
            }
            0xE2 if payload.len() > 14 && payload.starts_with(b"ICC_PROFILE\0") => {
                icc.push((payload[12], &payload[14..]));
            }
            0xEE if payload.len() >= 12 && payload.starts_with(b"Adobe") => {
                adobe = Some(payload[11]);
            }
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF if frame.is_none() => {
                frame = Some((marker, payload));
            }
            _ => {}
        }
        pos += len;
    }
    let mut f = ImageFacts::named("JPEG");
    if let Some((sof, p)) = frame {
        f.compression = Some(coding_name(sof).to_string());
        let precision = p.first().copied().unwrap_or(8);
        let n = p.get(5).copied().unwrap_or(0);
        // (id, horizontal, vertical) of each component.
        let comps: Vec<(u8, u8, u8)> = p
            .get(6..)
            .unwrap_or(&[])
            .chunks_exact(3)
            .take(usize::from(n))
            .map(|c| (c[0], c[1] >> 4, c[1] & 15))
            .collect();
        let ids: Vec<u8> = comps.iter().map(|c| c.0).collect();
        let (name, chroma) = match n {
            1 => (Some("Gray"), false),
            3 if adobe == Some(0) || ids == b"RGB" => (Some("RGB"), false),
            3 => (Some("YCbCr"), true),
            4 if adobe == Some(2) => (Some("YCCK"), true),
            4 => (Some("CMYK"), false),
            _ => (None, false),
        };
        match name {
            Some(name) => f.set_channels(name, n, precision),
            None => {
                f.channel_count = (n > 0).then_some(n);
                f.bits_per_channel = Some(precision);
            }
        }
        if let (true, [y, cb, cr, ..]) = (chroma, comps.as_slice()) {
            let even = cb.1 == cr.1 && cb.2 == cr.2 && cb.1 > 0 && cb.2 > 0;
            if even && y.1 % cb.1 == 0 && y.2 % cb.2 == 0 {
                f.subsampling = subsampling(y.1 / cb.1, y.2 / cb.2).map(str::to_string);
            }
        }
    }
    if !icc.is_empty() {
        icc.sort_by_key(|c| c.0);
        let profile: Vec<u8> = icc.iter().flat_map(|c| c.1.iter().copied()).collect();
        f.icc_profile = icc_description(&profile);
    }
    if let Some(e) = exif {
        f.camera = camera_from_exif(e);
        f.color_space = exif_color_space(e);
    }
    Some(f)
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
    if let Some(target) = options.target_size {
        if let Some(image) = preview_for(data, &header, orientation, target, options)? {
            return Ok(image);
        }
    }
    decode_main(data, &header, orientation, options)
}

/// Decode the main image of a stream (no preview lookup).
fn decode_main(
    data: &[u8],
    header: &Header<'_>,
    orientation: Orientation,
    options: &DecodeOptions,
) -> Result<ImageBuf> {
    let fast = options.target_size.is_none()
        && header.precision == 8
        && matches!(header.sof, 0xC0..=0xC2)
        && (header.components == 1 || (header.components == 3 && !header.adobe_untransformed))
        && standard_tables(data);
    if fast {
        match decode_fast(data, header, options) {
            Ok(image) => return orient::apply(image, orientation),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // A file the fast decoder rejects gets the reference decoder.
            Err(_) => {}
        }
    }
    decode_reference(data, header, orientation, options)
}

/// `zune-jpeg` into a buffer of the final size: RGBA straight from the
/// YCbCr conversion, grayscale expanded.
fn decode_fast(data: &[u8], header: &Header<'_>, options: &DecodeOptions) -> Result<ImageBuf> {
    use zune_core::bytestream::ZCursor;
    use zune_core::colorspace::ColorSpace;
    use zune_core::options::DecoderOptions;
    let gray = header.components == 1;
    let out_space = if gray {
        ColorSpace::Luma
    } else {
        ColorSpace::RGBA
    };
    // Strict: a truncated or damaged stream is an error here (and then
    // gets the reference decoder), never a padded picture.
    let zopts = DecoderOptions::new_fast()
        .set_strict_mode(true)
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX))
        .jpeg_set_out_colorspace(out_space);
    let mut dec = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(data), zopts);
    dec.decode_headers().map_err(|e| corrupt(FORMAT, e))?;
    let (w, h) = dec
        .dimensions()
        .map(|(w, h)| (w as u32, h as u32))
        .ok_or_else(|| corrupt(FORMAT, "missing frame information"))?;
    if (w, h) != (header.width, header.height) {
        return Err(corrupt(FORMAT, "frame size disagrees"));
    }
    let expected = match dec.input_colorspace() {
        Some(ColorSpace::YCbCr) => !gray,
        Some(ColorSpace::Luma) => gray,
        _ => false,
    };
    if !expected {
        return Err(corrupt(FORMAT, "color space for the reference decoder"));
    }
    options.cancel.check()?;
    let image = if gray {
        let mut luma = util::alloc(w as usize * h as usize, w, h)?;
        dec.decode_into(&mut luma).map_err(|e| corrupt(FORMAT, e))?;
        options.cancel.check()?;
        let mut out = alloc_rgba8(w, h)?;
        util::expand_to_rgba8(&luma, 1, &mut out);
        util::rgba8_image(w, h, out)?
    } else {
        let len = check_size(w, h, PixelFormat::Rgba8)?;
        let mut out = util::alloc_touched(len, w, h)?;
        dec.decode_into(&mut out).map_err(|e| corrupt(FORMAT, e))?;
        options.cancel.check()?;
        icc::to_srgb_rgba8(dec.icc_profile().as_deref(), &mut out);
        util::rgba8_image(w, h, out)?
    };
    Ok(image)
}

/// `jpeg-decoder`: every JPEG flavor, and DCT scaling for target sizes.
fn decode_reference(
    data: &[u8],
    header: &Header<'_>,
    orientation: Orientation,
    options: &DecodeOptions,
) -> Result<ImageBuf> {
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

/// Code length counts (BITS) of the Huffman tables T.81 Annex K
/// suggests: DC luminance and chrominance (Tables K.3, K.4), AC luminance
/// and chrominance (Tables K.5, K.6). Cameras code with these.
const STANDARD_DC_BITS: [[u8; 16]; 2] = [
    [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0],
    [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0],
];
const STANDARD_AC_BITS: [[u8; 16]; 2] = [
    [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7D],
    [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77],
];

/// True when the stream suits the `zune-jpeg` path: every Huffman table
/// defined before the first scan has the code lengths of a T.81 Annex K
/// table, as camera JPEGs do, and an end-of-image marker follows the scan
/// (a complete file). The differential test found `zune-jpeg` 0.5 decoding
/// streams with optimized tables (one-code AC tables of flat images, some
/// 4:2:0 streams) differently from the reference decoder, and it pads a
/// stream cut short instead of failing; such streams go to `jpeg-decoder`.
pub(crate) fn standard_tables(data: &[u8]) -> bool {
    let mut pos = 2;
    let mut seen = false;
    loop {
        let (Some(&ff), Some(&marker)) = (data.get(pos), data.get(pos + 1)) else {
            return false;
        };
        if ff != 0xFF {
            return false;
        }
        if marker == 0xFF {
            pos += 1;
            continue;
        }
        if matches!(marker, 0x01 | 0xD0..=0xD7) {
            pos += 2;
            continue;
        }
        if marker == 0xDA {
            // Entropy-coded data never holds FF D9 (an FF data byte is
            // followed by 00), so a marker FF D9 after the scan header is an
            // end of image: of this image, or of a preview appended to it.
            let tail = data.get(pos..).unwrap_or(&[]);
            return seen && tail.windows(2).rev().any(|w| w == [0xFF, 0xD9]);
        }
        if marker == 0xD9 {
            return false;
        }
        let Some(len) = util::be16(data, pos + 2).map(usize::from) else {
            return false;
        };
        if len < 2 {
            return false;
        }
        if marker == 0xC4 {
            let Some(seg) = data.get(pos + 4..pos + 2 + len) else {
                return false;
            };
            let mut at = 0;
            while at < seg.len() {
                let Some(bits) = seg.get(at + 1..at + 17) else {
                    return false;
                };
                let standard = if seg[at] >> 4 == 0 {
                    &STANDARD_DC_BITS
                } else {
                    &STANDARD_AC_BITS
                };
                if !standard.iter().any(|b| b[..] == *bits) {
                    return false;
                }
                let total: usize = bits.iter().map(|&n| usize::from(n)).sum();
                at += 17 + total;
                seen = true;
            }
        }
        pos += 2 + len;
    }
}

/// One embedded preview stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Preview {
    /// Offset of the stream in the file.
    pub offset: usize,
    /// Stream length.
    pub len: usize,
    /// Stored width and height.
    pub width: u32,
    /// Stored height.
    pub height: u32,
}

/// Previews longer than this are not read (no camera stores one).
const MAX_PREVIEW_BYTES: u32 = 64 << 20;

/// The embedded previews of a JPEG file: the Multi-Picture Format entries
/// after the first (the first is the primary image) and the EXIF
/// thumbnail. Each must lie inside the file, start a JPEG stream and have
/// an 8-bit frame header with a nonzero size.
pub(crate) fn previews(data: &[u8], header: &Header<'_>) -> Vec<Preview> {
    let mut out: Vec<Preview> = Vec::new();
    let mut push = |offset: usize, len: usize| {
        let Some(stream) = data.get(offset..offset.saturating_add(len)) else {
            return;
        };
        let Some(h) = scan_header(stream) else {
            return;
        };
        if h.width == 0 || h.height == 0 || h.precision != 8 {
            return;
        }
        if !out.iter().any(|p| p.offset == offset) {
            out.push(Preview {
                offset,
                len,
                width: h.width,
                height: h.height,
            });
        }
    };
    // CIPA DC-007: the MP Index IFD's MPEntry (tag 0xB002) holds 16 bytes
    // per image: attribute, size, offset from the MP header, two
    // dependent image numbers.
    if let Some(at) = header.mpf_at {
        if let Some(t) = data.get(at..).and_then(Tiff::new) {
            if let Some((entries, _)) = t.first_ifd().and_then(|o| t.ifd(o)) {
                let mp = entries
                    .iter()
                    .find(|e| e.tag == 0xB002)
                    .filter(|e| t.bytes(e).is_some());
                if let Some(e) = mp {
                    let images = (e.count / 16).min(16) as usize;
                    for i in 1..images {
                        let base = e.value_at + i * 16;
                        let (Some(size), Some(off)) = (t.u32(base + 4), t.u32(base + 8)) else {
                            continue;
                        };
                        if off == 0 || !(4..=MAX_PREVIEW_BYTES).contains(&size) {
                            continue;
                        }
                        push(at.saturating_add(off as usize), size as usize);
                    }
                }
            }
        }
    }
    // The EXIF thumbnail: IFD1 JPEGInterchangeFormat (0x0201) and its
    // length (0x0202), offsets from the EXIF TIFF header.
    if let (Some(exif), Some(at)) = (header.exif, header.exif_at) {
        if let Some(t) = Tiff::new(exif) {
            let ifd1 = t
                .first_ifd()
                .and_then(|o| t.ifd(o))
                .map(|(_, next)| next)
                .filter(|&n| n != 0)
                .and_then(|n| t.ifd(n));
            if let Some((entries, _)) = ifd1 {
                if let (Some(off), Some(len)) = (t.get(&entries, 0x0201), t.get(&entries, 0x0202)) {
                    if off > 0 && (4..=MAX_PREVIEW_BYTES).contains(&len) {
                        push(at.saturating_add(off as usize), len as usize);
                    }
                }
            }
        }
    }
    out
}

/// The smallest embedded preview that covers `target` (display axes),
/// decoded and oriented, or `None` when no preview fits: none covers the
/// target, none is smaller than the main image, or none has its aspect
/// ratio (within 2 percent).
fn preview_for(
    data: &[u8],
    header: &Header<'_>,
    orientation: Orientation,
    target: (u32, u32),
    options: &DecodeOptions,
) -> Result<Option<ImageBuf>> {
    let (tw, th) = oriented(target.0, target.1, orientation);
    let (mw, mh) = (f64::from(header.width), f64::from(header.height));
    let main_aspect = mw / mh;
    let mut fitting: Vec<Preview> = previews(data, header)
        .into_iter()
        .filter(|p| {
            let aspect = f64::from(p.width) / f64::from(p.height);
            let smaller = u64::from(p.width) * u64::from(p.height)
                < u64::from(header.width) * u64::from(header.height);
            smaller
                && (aspect / main_aspect - 1.0).abs() <= 0.02
                && (p.width >= tw || p.height >= th)
        })
        .collect();
    fitting.sort_by_key(|p| u64::from(p.width) * u64::from(p.height));
    for p in fitting {
        options.cancel.check()?;
        let stream = &data[p.offset..p.offset + p.len];
        let Some(h) = scan_header(stream) else {
            continue;
        };
        // A preview with its own orientation tag is turned by it; one
        // without is stored like the main image and gets its orientation.
        let own = h.exif.map(orientation_from_exif).unwrap_or_default();
        let o = if own == Orientation::Normal {
            orientation
        } else {
            own
        };
        let opts = DecodeOptions {
            target_size: None,
            first_frame_only: true,
            prefer_8bit: options.prefer_8bit,
            cancel: options.cancel.clone(),
        };
        match decode_main(stream, &h, o, &opts) {
            Ok(image) => return Ok(Some(image)),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => continue,
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                // Flat blocks of saturated and white colors (where a bad
                // conversion shows) beside smooth gradients.
                let px = match (x / 16) % 4 {
                    0 => [255, 255, 255],
                    1 => [255, 0, 0],
                    2 => [0, 0, 255],
                    _ => [
                        (x * 255 / w.max(1)) as u8,
                        (y * 255 / h.max(1)) as u8,
                        ((x + y) * 3 % 256) as u8,
                    ],
                };
                rgb.extend_from_slice(&px);
            }
        }
        rgb
    }

    fn encode_with(
        w: u32,
        h: u32,
        sampling: jpeg_encoder::SamplingFactor,
        gray: bool,
        quality: u8,
        optimized: bool,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let mut enc = jpeg_encoder::Encoder::new(&mut out, quality);
        enc.set_sampling_factor(sampling);
        enc.set_optimized_huffman_tables(optimized);
        if gray {
            let luma: Vec<u8> = gradient(w, h).chunks_exact(3).map(|p| p[0]).collect();
            enc.encode(&luma, w as u16, h as u16, jpeg_encoder::ColorType::Luma)
                .unwrap();
        } else {
            enc.encode(
                &gradient(w, h),
                w as u16,
                h as u16,
                jpeg_encoder::ColorType::Rgb,
            )
            .unwrap();
        }
        out
    }

    /// The SIMD decoder agrees with the reference decoder pixel by pixel
    /// within the IDCT and upsampling tolerance of the JPEG standard's
    /// decoders, on every chroma subsampling, odd sizes and grayscale.
    #[test]
    fn jpeg_fast_path_matches_reference_decoder() {
        use jpeg_encoder::SamplingFactor as S;
        let mut fast_runs = 0;
        for (w, h) in [
            (48u32, 16u32),
            (64, 48),
            (97, 33),
            (1, 1),
            (17, 250),
            (300, 7),
        ] {
            for (sampling, gray, quality, optimized) in [
                (S::R_4_4_4, false, 92, false),
                (S::R_4_2_2, false, 92, false),
                (S::R_4_2_0, false, 92, false),
                (S::R_4_4_4, true, 92, false),
                (S::R_4_2_2, false, 100, true),
                (S::R_4_2_0, false, 75, true),
                (S::R_4_4_4, true, 50, true),
            ] {
                let file = encode_with(w, h, sampling, gray, quality, optimized);
                let header = scan_header(&file).unwrap();
                // Through the provider: whichever decoder it picks.
                let picked = decode_image(&file, &DecodeOptions::default()).unwrap();
                let reference_full = decode_reference(
                    &file,
                    &header,
                    Orientation::Normal,
                    &DecodeOptions::default(),
                )
                .unwrap();
                let worst = picked
                    .data()
                    .iter()
                    .zip(reference_full.data())
                    .map(|(a, b)| a.abs_diff(*b))
                    .max()
                    .unwrap_or(0);
                assert!(
                    worst <= 8,
                    "{w}x{h} {sampling:?} q{quality} opt {optimized}: {worst}"
                );
                if !standard_tables(&file) {
                    continue;
                }
                fast_runs += 1;
                let opts = DecodeOptions::default();
                let fast = decode_fast(&file, &header, &opts).expect("fast path");
                let reference =
                    decode_reference(&file, &header, Orientation::Normal, &opts).unwrap();
                assert_eq!((fast.width(), fast.height()), (w, h));
                let (mut max, mut sum) = (0u8, 0u64);
                for (a, b) in fast.data().iter().zip(reference.data()) {
                    let d = a.abs_diff(*b);
                    max = max.max(d);
                    sum += u64::from(d);
                }
                let mean = sum as f64 / fast.data().len() as f64;
                assert!(
                    max <= 8 && mean <= 1.0,
                    "{w}x{h} {sampling:?} gray {gray}: max {max} mean {mean:.3}"
                );
            }
        }
        assert!(fast_runs >= 20, "the fast path ran {fast_runs} times");
    }
}

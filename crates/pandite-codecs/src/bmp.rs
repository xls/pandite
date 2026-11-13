//! BMP and DIB decoder and encoder, written from the format description.
//!
//! The decoder reads OS/2 1.x core headers and Windows headers from
//! BITMAPINFOHEADER to BITMAPV5HEADER; 1, 2, 4 and 8-bit palettes; 16, 24
//! and 32-bit pixels with default masks or bit fields (any mask width);
//! RLE4 and RLE8 (skipped pixels become transparent); top-down and
//! bottom-up rows; and embedded JPEG or PNG payloads. A 32-bit alpha
//! channel that is zero everywhere is treated as opaque. The same code
//! decodes the DIB images inside ICO and CUR files.
//!
//! The encoder writes 24-bit BGR for opaque images and 32-bit BGRA with a
//! BITMAPV4HEADER and an alpha mask for images with transparency.

use crate::util::{self, check_size, corrupt, le16, le32, unsupported};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, EncodeOptions,
    EncoderProvider, Error, ImageBuf, ImageFacts, ImageMeta, Orientation, PixelFormat, Result,
    Source,
};
use std::io::Write;

const FORMAT: &str = "bmp";
const FILE_HEADER: usize = 14;

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;

/// Facts from a DIB header.
#[derive(Clone, Debug)]
pub(crate) struct DibHeader {
    pub width: u32,
    pub height: u32,
    pub top_down: bool,
    pub bpp: u16,
    pub compression: u32,
    pub colors_used: u32,
    /// Red, green, blue and alpha masks for 16 and 32-bit pixels.
    pub masks: [u32; 4],
    /// Bytes per palette entry: 3 for OS/2 core headers, 4 otherwise.
    pub palette_entry: usize,
    /// Offset just past the header and any trailing mask fields.
    pub after_header: usize,
}

/// Parse the DIB header at `at`. `icon` halves the height (XOR plus AND).
pub(crate) fn parse_header(data: &[u8], at: usize, icon: bool) -> Result<DibHeader> {
    let short = || corrupt(FORMAT, "truncated header");
    let size = le32(data, at).ok_or_else(short)? as usize;
    let (width, raw_height, bpp, compression, colors_used, palette_entry);
    match size {
        12 => {
            width = i64::from(le16(data, at + 4).ok_or_else(short)?);
            raw_height = i64::from(le16(data, at + 6).ok_or_else(short)?);
            bpp = le16(data, at + 10).ok_or_else(short)?;
            compression = BI_RGB;
            colors_used = 0;
            palette_entry = 3;
        }
        16..=124 => {
            if size < 16 || data.len() < at + size.min(40) {
                return Err(short());
            }
            width = i64::from(le32(data, at + 4).ok_or_else(short)? as i32);
            raw_height = i64::from(le32(data, at + 8).ok_or_else(short)? as i32);
            bpp = le16(data, at + 14).ok_or_else(short)?;
            compression = if size >= 20 {
                le32(data, at + 16).ok_or_else(short)?
            } else {
                BI_RGB
            };
            colors_used = if size >= 36 {
                le32(data, at + 32).ok_or_else(short)?
            } else {
                0
            };
            palette_entry = 4;
            if size == 64 && (compression == 3 || compression == 4) {
                return Err(unsupported(FORMAT, "OS/2 Huffman or RLE24 compression"));
            }
        }
        _ => return Err(corrupt(FORMAT, format!("unknown header size {size}"))),
    }
    if width <= 0 || raw_height == 0 || raw_height == i64::from(i32::MIN) {
        return Err(corrupt(FORMAT, "invalid dimensions"));
    }
    let top_down = raw_height < 0;
    let mut height = raw_height.unsigned_abs();
    if icon {
        height /= 2;
    }
    let (width, height) = (
        u32::try_from(width).map_err(|_| corrupt(FORMAT, "invalid width"))?,
        u32::try_from(height).map_err(|_| corrupt(FORMAT, "invalid height"))?,
    );
    if height == 0 {
        return Err(corrupt(FORMAT, "invalid dimensions"));
    }

    let mut after_header = at + size;
    let mut masks = match bpp {
        16 => [0x7C00, 0x03E0, 0x001F, 0],
        32 => [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000],
        _ => [0; 4],
    };
    if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        let count = if compression == BI_ALPHABITFIELDS {
            4
        } else {
            3
        };
        if size >= 52 {
            for (i, m) in masks.iter_mut().enumerate().take(count.max(3)) {
                *m = le32(data, at + 40 + 4 * i).ok_or_else(short)?;
            }
            masks[3] = if size >= 56 {
                le32(data, at + 52).ok_or_else(short)?
            } else {
                0
            };
        } else {
            for (i, m) in masks.iter_mut().enumerate().take(count) {
                *m = le32(data, after_header + 4 * i).ok_or_else(short)?;
            }
            if count == 3 {
                masks[3] = 0;
            }
            after_header += 4 * count;
        }
    }
    Ok(DibHeader {
        width,
        height,
        top_down,
        bpp,
        compression,
        colors_used,
        masks,
        palette_entry,
        after_header,
    })
}

/// Decode a DIB whose header starts at `at`. `pixels_at` is the pixel data
/// offset from a BMP file header, when there is one. `icon` reads the AND
/// mask that follows the color data in ICO and CUR files.
pub(crate) fn decode_dib(
    data: &[u8],
    at: usize,
    pixels_at: Option<usize>,
    icon: bool,
    cancel: &CancelToken,
) -> Result<ImageBuf> {
    let h = parse_header(data, at, icon)?;
    let (w, ht) = (h.width, h.height);
    check_size(w, ht, PixelFormat::Rgba8)?;

    if h.compression == BI_JPEG || h.compression == BI_PNG {
        let start = pixels_at.unwrap_or(h.after_header);
        let payload = data
            .get(start..)
            .ok_or_else(|| corrupt(FORMAT, "missing embedded image"))?;
        let opts = DecodeOptions {
            cancel: cancel.clone(),
            ..DecodeOptions::default()
        };
        return if h.compression == BI_JPEG {
            crate::jpeg::decode_image(payload, &opts)
        } else {
            match crate::png::decode(payload, &opts)? {
                Content::Still(img) => Ok(img),
                other => Ok(other.first_image().clone()),
            }
        };
    }

    // Palette.
    let mut palette: Vec<[u8; 4]> = Vec::new();
    if h.bpp <= 8 {
        if !matches!(h.bpp, 1 | 2 | 4 | 8) {
            return Err(corrupt(FORMAT, format!("bad bit depth {}", h.bpp)));
        }
        let max = 1usize << h.bpp;
        let count = match h.colors_used as usize {
            0 => max,
            n => n.min(256),
        };
        let start = h.after_header;
        for i in 0..count {
            let p = start + i * h.palette_entry;
            match data.get(p..p + 3) {
                Some(c) => palette.push([c[2], c[1], c[0], 255]),
                None => break,
            }
        }
        if palette.is_empty() {
            return Err(corrupt(FORMAT, "missing palette"));
        }
    } else if !matches!(h.bpp, 16 | 24 | 32) {
        return Err(unsupported(FORMAT, format!("bit depth {}", h.bpp)));
    }

    let palette_bytes = palette.len() * h.palette_entry;
    let start = match pixels_at {
        Some(off) if off >= h.after_header && off < data.len() => off,
        _ => h.after_header + if h.bpp <= 8 { palette_bytes } else { 0 },
    };
    let pixels = data
        .get(start..)
        .ok_or_else(|| corrupt(FORMAT, "missing pixel data"))?;

    let mut out = util::alloc_rgba8(w, ht)?;
    let wu = w as usize;
    let hu = ht as usize;
    let row_of = |y: usize| if h.top_down { y } else { hu - 1 - y };

    match h.compression {
        BI_RLE8 | BI_RLE4 => {
            if h.top_down || h.bpp != if h.compression == BI_RLE8 { 8 } else { 4 } {
                return Err(corrupt(FORMAT, "invalid RLE bitmap"));
            }
            decode_rle(pixels, &h, &palette, &mut out, cancel)?;
        }
        BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS => {
            let stride = (wu * usize::from(h.bpp)).div_ceil(32) * 4;
            let need = stride
                .checked_mul(hu)
                .ok_or_else(|| corrupt(FORMAT, "image too large"))?;
            // The last row may lack its padding in some writers.
            let unpadded = (wu * usize::from(h.bpp)).div_ceil(8);
            if pixels.len() < need - stride + unpadded {
                return Err(corrupt(FORMAT, "truncated pixel data"));
            }
            let fields = Fields::new(h.masks);
            let use_alpha = matches!(h.bpp, 16 | 32) && fields.a.mask != 0;
            for y in 0..hu {
                if y % 64 == 0 {
                    cancel.check()?;
                }
                let src = &pixels[y * stride..(y * stride + stride).min(pixels.len())];
                let dst = &mut out[row_of(y) * wu * 4..(row_of(y) + 1) * wu * 4];
                match h.bpp {
                    1 | 2 | 4 | 8 => {
                        let bits = usize::from(h.bpp);
                        let per_byte = 8 / bits;
                        let mask = ((1u16 << bits) - 1) as u8;
                        for (x, d) in dst.chunks_exact_mut(4).enumerate() {
                            let byte = src[x / per_byte];
                            let shift = 8 - bits * (x % per_byte + 1);
                            let idx = usize::from((byte >> shift) & mask);
                            d.copy_from_slice(palette.get(idx).unwrap_or(&[0, 0, 0, 255]));
                        }
                    }
                    16 => {
                        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(2)) {
                            let v = u32::from(u16::from_le_bytes([s[0], s[1]]));
                            d.copy_from_slice(&fields.rgba(v, use_alpha));
                        }
                    }
                    24 => {
                        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(3)) {
                            d.copy_from_slice(&[s[2], s[1], s[0], 255]);
                        }
                    }
                    _ => {
                        for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                            let v = u32::from_le_bytes([s[0], s[1], s[2], s[3]]);
                            d.copy_from_slice(&fields.rgba(v, use_alpha));
                        }
                    }
                }
            }
            if use_alpha && out.chunks_exact(4).all(|p| p[3] == 0) {
                // An alpha channel that is zero everywhere is unused padding,
                // unless an icon AND mask below supplies transparency.
                for p in out.chunks_exact_mut(4) {
                    p[3] = 255;
                }
                if icon {
                    apply_and_mask(pixels, &h, need, &mut out);
                }
            } else if icon && !use_alpha {
                apply_and_mask(pixels, &h, need, &mut out);
            }
        }
        other => return Err(unsupported(FORMAT, format!("compression {other}"))),
    }
    util::rgba8_image(w, ht, out)
}

/// Apply the 1-bit AND mask that follows the color data of an icon.
fn apply_and_mask(pixels: &[u8], h: &DibHeader, color_bytes: usize, out: &mut [u8]) {
    let (wu, hu) = (h.width as usize, h.height as usize);
    let stride = wu.div_ceil(32) * 4;
    let Some(mask) = pixels.get(color_bytes..) else {
        return;
    };
    if mask.len() < stride * hu {
        return;
    }
    for y in 0..hu {
        let row = &mask[y * stride..];
        let oy = if h.top_down { y } else { hu - 1 - y };
        for x in 0..wu {
            if row[x / 8] & (0x80 >> (x % 8)) != 0 {
                out[(oy * wu + x) * 4 + 3] = 0;
            }
        }
    }
}

/// Decode RLE8 or RLE4 data. Pixels the stream skips stay transparent.
fn decode_rle(
    src: &[u8],
    h: &DibHeader,
    palette: &[[u8; 4]],
    out: &mut [u8],
    cancel: &CancelToken,
) -> Result<()> {
    let (wu, hu) = (h.width as usize, h.height as usize);
    let four = h.compression == BI_RLE4;
    let color = |i: u8| *palette.get(usize::from(i)).unwrap_or(&[0, 0, 0, 255]);
    let put = |x: usize, y: usize, c: [u8; 4], out: &mut [u8]| {
        if x < wu && y < hu {
            let row = hu - 1 - y;
            out[(row * wu + x) * 4..(row * wu + x) * 4 + 4].copy_from_slice(&c);
        }
    };
    let (mut x, mut y, mut pos) = (0usize, 0usize, 0usize);
    let truncated = || corrupt(FORMAT, "truncated RLE data");
    loop {
        if y >= hu {
            return Ok(());
        }
        let (Some(&count), Some(&value)) = (src.get(pos), src.get(pos + 1)) else {
            return Err(truncated());
        };
        pos += 2;
        if count > 0 {
            for i in 0..usize::from(count) {
                let idx = if four {
                    if i % 2 == 0 {
                        value >> 4
                    } else {
                        value & 0x0F
                    }
                } else {
                    value
                };
                put(x, y, color(idx), out);
                x += 1;
            }
            continue;
        }
        match value {
            0 => {
                x = 0;
                y += 1;
                if y % 64 == 0 {
                    cancel.check()?;
                }
            }
            1 => return Ok(()),
            2 => {
                let (Some(&dx), Some(&dy)) = (src.get(pos), src.get(pos + 1)) else {
                    return Err(truncated());
                };
                pos += 2;
                x += usize::from(dx);
                y += usize::from(dy);
            }
            n => {
                let n = usize::from(n);
                let bytes = if four { n.div_ceil(2) } else { n };
                let run = src.get(pos..pos + bytes).ok_or_else(truncated)?;
                for i in 0..n {
                    let idx = if four {
                        let b = run[i / 2];
                        if i % 2 == 0 {
                            b >> 4
                        } else {
                            b & 0x0F
                        }
                    } else {
                        run[i]
                    };
                    put(x, y, color(idx), out);
                    x += 1;
                }
                pos += bytes + (bytes & 1);
            }
        }
    }
}

/// One color channel described by a bit mask.
#[derive(Clone, Copy)]
struct Field {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Field {
    fn new(mask: u32) -> Self {
        if mask == 0 {
            return Field {
                mask: 0,
                shift: 0,
                max: 0,
            };
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        Field { mask, shift, max }
    }

    fn get(&self, v: u32) -> u8 {
        util::scale_to_u8((v & self.mask) >> self.shift, self.max)
    }
}

struct Fields {
    r: Field,
    g: Field,
    b: Field,
    a: Field,
}

impl Fields {
    fn new(masks: [u32; 4]) -> Self {
        Fields {
            r: Field::new(masks[0]),
            g: Field::new(masks[1]),
            b: Field::new(masks[2]),
            a: Field::new(masks[3]),
        }
    }

    fn rgba(&self, v: u32, alpha: bool) -> [u8; 4] {
        let a = if alpha && self.a.mask != 0 {
            self.a.get(v)
        } else {
            255
        };
        [self.r.get(v), self.g.get(v), self.b.get(v), a]
    }
}

/// Decodes BMP and DIB files.
#[derive(Clone, Copy, Debug, Default)]
pub struct BmpDecoder;

fn file_header(data: &[u8]) -> Result<Option<usize>> {
    if data.len() < FILE_HEADER + 4 || &data[..2] != b"BM" {
        return Err(corrupt(FORMAT, "missing BM signature"));
    }
    let off = le32(data, 10).unwrap_or(0) as usize;
    Ok((off > FILE_HEADER).then_some(off))
}

impl DecoderProvider for BmpDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "BMP image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bmp", "dib"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.len() >= 18
            && &header[..2] == b"BM"
            && matches!(
                le32(header, 14),
                Some(12 | 16 | 40 | 52 | 56 | 64 | 108 | 124)
            )
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let data = source.bytes();
        file_header(data)?;
        let h = parse_header(data, FILE_HEADER, false)?;
        let has_alpha = h.compression == BI_RLE4
            || h.compression == BI_RLE8
            || (h.bpp == 32 && h.masks[3] != 0);
        Ok(ImageMeta {
            width: h.width,
            height: h.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha,
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let data = source.bytes();
        let pixels_at = file_header(data)?;
        util::guard(FORMAT, || {
            decode_dib(data, FILE_HEADER, pixels_at, false, &options.cancel).map(Content::Still)
        })
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        let data = source.bytes();
        file_header(data)?;
        let h = parse_header(data, FILE_HEADER, false)?;
        Ok(dib_facts(data, FILE_HEADER, &h))
    }
}

/// The facts of a DIB header at `at`: compression, pixel layout and the
/// color space of version 4 and 5 headers.
pub(crate) fn dib_facts(data: &[u8], at: usize, h: &DibHeader) -> ImageFacts {
    let mut f = ImageFacts::named("BMP");
    f.compression = match h.compression {
        BI_RGB => Some("Uncompressed"),
        BI_RLE8 => Some("RLE8"),
        BI_RLE4 => Some("RLE4"),
        BI_BITFIELDS => Some("Uncompressed, bit fields"),
        BI_JPEG => Some("JPEG"),
        BI_PNG => Some("PNG"),
        BI_ALPHABITFIELDS => Some("Uncompressed, bit fields with alpha"),
        _ => None,
    }
    .map(str::to_string);
    let bpp = h.bpp;
    let alpha = h.masks[3] != 0;
    match bpp {
        1 | 2 | 4 | 8 => {
            let colors = if h.colors_used > 0 {
                h.colors_used.min(1 << bpp)
            } else {
                1 << bpp
            };
            f.set_channels(format!("Indexed, {colors} colors"), 1, bpp as u8);
        }
        16 | 24 | 32 if !matches!(h.compression, BI_JPEG | BI_PNG) => {
            let width = |m: u32| m.count_ones() as u8;
            let (name, count) = if alpha { ("RGBA", 4) } else { ("RGB", 3) };
            f.channels = Some(name.to_string());
            f.channel_count = Some(count);
            let (r, g, b) = (width(h.masks[0]), width(h.masks[1]), width(h.masks[2]));
            if bpp == 24 {
                f.bits_per_channel = Some(8);
            } else if r == g && g == b && r > 0 {
                f.bits_per_channel = Some(r);
            }
            f.bits_per_pixel = Some(bpp);
        }
        _ => {}
    }
    // BITMAPV4HEADER and later: the color space type, and for V5 an
    // embedded profile (offset from the header start).
    let size = le32(data, at).unwrap_or(0);
    if size >= 108 {
        f.color_space = match le32(data, at + 56) {
            Some(0x7352_4742) => Some("sRGB".to_string()),
            Some(0x5769_6E20) => Some("Windows default".to_string()),
            Some(0) => Some("Calibrated RGB".to_string()),
            _ => None,
        };
    }
    if size >= 124 && le32(data, at + 56) == Some(0x4D42_4544) {
        let off = le32(data, at + 112).unwrap_or(0) as usize;
        let len = le32(data, at + 116).unwrap_or(0) as usize;
        let start = at.saturating_add(off);
        f.icc_profile = data
            .get(start..start.saturating_add(len))
            .and_then(crate::facts::icc_description);
    }
    f
}

/// Writes BMP files: 24-bit for opaque images, 32-bit with alpha otherwise.
#[derive(Clone, Copy, Debug, Default)]
pub struct BmpEncoder;

impl EncoderProvider for BmpEncoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "BMP image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["bmp", "dib"]
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
        let rgba = util::to_rgba8(image);
        let (w, h) = (image.width() as usize, image.height() as usize);
        let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
        let bpp: usize = if alpha { 32 } else { 24 };
        let stride = (w * bpp).div_ceil(32) * 4;
        let info_size: usize = if alpha { 108 } else { 40 };
        let offset = FILE_HEADER + info_size;
        let image_size = stride * h;
        let file_size = offset + image_size;
        let too_big = || Error::Invalid("image too large for BMP".into());
        let file_size32 = u32::try_from(file_size).map_err(|_| too_big())?;
        let image_size32 = u32::try_from(image_size).map_err(|_| too_big())?;

        let mut buf = Vec::with_capacity(file_size);
        buf.extend_from_slice(b"BM");
        buf.extend_from_slice(&file_size32.to_le_bytes());
        buf.extend_from_slice(&[0, 0, 0, 0]);
        buf.extend_from_slice(&(offset as u32).to_le_bytes());
        buf.extend_from_slice(&(info_size as u32).to_le_bytes());
        buf.extend_from_slice(&(w as i32).to_le_bytes());
        buf.extend_from_slice(&(h as i32).to_le_bytes());
        buf.extend_from_slice(&1u16.to_le_bytes());
        buf.extend_from_slice(&(bpp as u16).to_le_bytes());
        let compression = if alpha { BI_BITFIELDS } else { BI_RGB };
        buf.extend_from_slice(&compression.to_le_bytes());
        buf.extend_from_slice(&image_size32.to_le_bytes());
        buf.extend_from_slice(&2835i32.to_le_bytes());
        buf.extend_from_slice(&2835i32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes());
        if alpha {
            for m in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
                buf.extend_from_slice(&m.to_le_bytes());
            }
            // LCS_sRGB, then unused endpoints and gamma.
            buf.extend_from_slice(&0x7352_4742u32.to_le_bytes());
            buf.extend_from_slice(&[0u8; 36 + 12]);
        }
        let pad = stride - w * bpp / 8;
        for y in (0..h).rev() {
            let row = &rgba[y * w * 4..(y + 1) * w * 4];
            for p in row.chunks_exact(4) {
                buf.extend_from_slice(&[p[2], p[1], p[0]]);
                if alpha {
                    buf.push(p[3]);
                }
            }
            buf.extend(std::iter::repeat_n(0u8, pad));
        }
        out.write_all(&buf)?;
        Ok(())
    }
}

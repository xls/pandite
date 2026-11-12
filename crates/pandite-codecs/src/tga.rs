//! Truevision TGA decoder, written from the TGA 2.0 specification.
//!
//! Supports color-mapped, true-color and grayscale images, uncompressed and
//! RLE (runs may cross scanlines), 8 to 32-bit pixels, 15/16/24/32-bit
//! color maps, all four origins, and the TGA 2.0 extension area alpha type
//! (no alpha, straight or premultiplied). TGA has no signature, so the
//! format is matched by extension only.

use crate::util::{self, check_size, corrupt, le16, le32, unsupported};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "tga";

/// Decodes TGA files.
#[derive(Clone, Copy, Debug, Default)]
pub struct TgaDecoder;

struct Header {
    id_len: usize,
    map_type: u8,
    image_type: u8,
    map_first: usize,
    map_len: usize,
    map_bits: u8,
    width: u32,
    height: u32,
    bits: u8,
    alpha_bits: u8,
    right_to_left: bool,
    top_down: bool,
}

fn header(d: &[u8]) -> Result<Header> {
    if d.len() < 18 {
        return Err(corrupt(FORMAT, "truncated header"));
    }
    let h = Header {
        id_len: usize::from(d[0]),
        map_type: d[1],
        image_type: d[2],
        map_first: usize::from(le16(d, 3).unwrap_or(0)),
        map_len: usize::from(le16(d, 5).unwrap_or(0)),
        map_bits: d[7],
        width: u32::from(le16(d, 12).unwrap_or(0)),
        height: u32::from(le16(d, 14).unwrap_or(0)),
        bits: d[16],
        alpha_bits: d[17] & 0x0F,
        right_to_left: d[17] & 0x10 != 0,
        top_down: d[17] & 0x20 != 0,
    };
    if h.width == 0 || h.height == 0 {
        return Err(corrupt(FORMAT, "zero image size"));
    }
    if h.map_type > 1 {
        return Err(corrupt(FORMAT, "bad color map type"));
    }
    match h.image_type {
        1 | 9 => {
            if h.map_type != 1 || !matches!(h.bits, 8 | 16) {
                return Err(corrupt(FORMAT, "bad color-mapped image"));
            }
        }
        2 | 10 => {
            if !matches!(h.bits, 15 | 16 | 24 | 32) {
                return Err(corrupt(FORMAT, "bad true-color depth"));
            }
        }
        3 | 11 => {
            if !matches!(h.bits, 8 | 16) {
                return Err(corrupt(FORMAT, "bad grayscale depth"));
            }
        }
        0 => return Err(corrupt(FORMAT, "no image data")),
        t => return Err(unsupported(FORMAT, format!("image type {t}"))),
    }
    Ok(h)
}

/// Alpha type from the TGA 2.0 extension area: None when absent.
fn extension_alpha(d: &[u8]) -> Option<u8> {
    if d.len() < 26 + 18 {
        return None;
    }
    let footer = &d[d.len() - 26..];
    if &footer[8..24] != b"TRUEVISION-XFILE" {
        return None;
    }
    let ext = le32(footer, 0)? as usize;
    if ext == 0 || ext.checked_add(495)? > d.len() - 26 || le16(d, ext)? < 495 {
        return None;
    }
    d.get(ext + 494).copied()
}

impl DecoderProvider for TgaDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "Truevision TGA image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["tga"]
    }

    fn sniff(&self, _header: &[u8]) -> bool {
        false
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = header(source.bytes())?;
        Ok(ImageMeta {
            width: h.width,
            height: h.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: h.alpha_bits > 0 || h.bits == 32 || (h.map_type == 1 && h.map_bits == 32),
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        util::guard(FORMAT, || decode(source.bytes(), &options.cancel)).map(Content::Still)
    }
}

/// Convert one stored pixel (little endian, `bits` wide) to RGBA8.
fn color(p: &[u8], bits: u8, gray: bool, use_alpha: bool) -> [u8; 4] {
    if gray {
        let a = if bits == 16 && use_alpha { p[1] } else { 255 };
        return [p[0], p[0], p[0], a];
    }
    match bits {
        15 | 16 => {
            let v = u16::from_le_bytes([p[0], p[1]]);
            let c = |s: u16| util::scale_to_u8(u32::from((v >> s) & 31), 31);
            let a = if bits == 16 && use_alpha && v & 0x8000 == 0 {
                0
            } else {
                255
            };
            [c(10), c(5), c(0), a]
        }
        24 => [p[2], p[1], p[0], 255],
        _ => [p[2], p[1], p[0], if use_alpha { p[3] } else { 255 }],
    }
}

fn decode(d: &[u8], cancel: &CancelToken) -> Result<ImageBuf> {
    let h = header(d)?;
    check_size(h.width, h.height, PixelFormat::Rgba8)?;
    let ext_alpha = extension_alpha(d);
    // 0 = no alpha, 1 and 2 = undefined (ignore), 3 = alpha, 4 = premultiplied.
    let want_alpha = match ext_alpha {
        Some(0..=2) => false,
        Some(_) => true,
        None => h.alpha_bits > 0 || h.bits == 32 || (h.map_type == 1 && h.map_bits == 32),
    };
    let premultiplied = ext_alpha == Some(4);

    let mut pos = 18 + h.id_len;
    let mut palette: Vec<[u8; 4]> = Vec::new();
    if h.map_type == 1 {
        let entry = usize::from(h.map_bits).div_ceil(8);
        if !matches!(h.map_bits, 15 | 16 | 24 | 32) {
            return Err(corrupt(FORMAT, "bad color map depth"));
        }
        let bytes = entry * h.map_len;
        let map = d
            .get(pos..pos + bytes)
            .ok_or_else(|| corrupt(FORMAT, "truncated color map"))?;
        palette = map
            .chunks_exact(entry)
            .map(|p| color(p, h.map_bits, false, want_alpha))
            .collect();
        pos += bytes;
    }
    let gray = matches!(h.image_type, 3 | 11);
    let mapped = matches!(h.image_type, 1 | 9);
    let rle = h.image_type >= 9;
    let bpp = usize::from(h.bits).div_ceil(8);
    let (wu, hu) = (h.width as usize, h.height as usize);
    let n = wu * hu;

    // Read the pixel stream in file order.
    let mut out = util::alloc_rgba8(h.width, h.height)?;
    let convert = |p: &[u8]| -> [u8; 4] {
        if mapped {
            let idx = if bpp == 2 {
                usize::from(u16::from_le_bytes([p[0], p[1]]))
            } else {
                usize::from(p[0])
            };
            *idx.checked_sub(h.map_first)
                .and_then(|i| palette.get(i))
                .unwrap_or(&[0, 0, 0, 255])
        } else {
            color(p, h.bits, gray, want_alpha)
        }
    };
    let mut i = 0usize;
    let truncated = || corrupt(FORMAT, "truncated pixel data");
    while i < n {
        if i.is_multiple_of((wu * 64).max(1)) {
            cancel.check()?;
        }
        if rle {
            let head = *d.get(pos).ok_or_else(truncated)?;
            pos += 1;
            let count = usize::from(head & 0x7F) + 1;
            if head & 0x80 != 0 {
                let p = d.get(pos..pos + bpp).ok_or_else(truncated)?;
                pos += bpp;
                let c = convert(p);
                for _ in 0..count.min(n - i) {
                    out[i * 4..i * 4 + 4].copy_from_slice(&c);
                    i += 1;
                }
            } else {
                for _ in 0..count {
                    let p = d.get(pos..pos + bpp).ok_or_else(truncated)?;
                    pos += bpp;
                    if i < n {
                        let c = convert(p);
                        out[i * 4..i * 4 + 4].copy_from_slice(&c);
                        i += 1;
                    }
                }
            }
        } else {
            let row = d.get(pos..pos + wu * bpp).ok_or_else(truncated)?;
            for p in row.chunks_exact(bpp) {
                let c = convert(p);
                out[i * 4..i * 4 + 4].copy_from_slice(&c);
                i += 1;
            }
            pos += wu * bpp;
        }
    }

    if want_alpha && ext_alpha.is_none() && out.chunks_exact(4).all(|p| p[3] == 0) {
        // Alpha bits that are zero everywhere carry no information.
        for p in out.chunks_exact_mut(4) {
            p[3] = 255;
        }
    }
    if premultiplied {
        for p in out.chunks_exact_mut(4) {
            let a = u32::from(p[3]);
            if a != 0 && a != 255 {
                for c in &mut p[..3] {
                    *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
    }
    // Reorder to top-left origin.
    if !h.top_down || h.right_to_left {
        let mut fixed = util::alloc_rgba8(h.width, h.height)?;
        for y in 0..hu {
            let sy = if h.top_down { y } else { hu - 1 - y };
            for x in 0..wu {
                let sx = if h.right_to_left { wu - 1 - x } else { x };
                let s = (sy * wu + sx) * 4;
                fixed[(y * wu + x) * 4..(y * wu + x) * 4 + 4].copy_from_slice(&out[s..s + 4]);
            }
        }
        out = fixed;
    }
    util::rgba8_image(h.width, h.height, out)
}

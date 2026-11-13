//! GPU texture pixel formats shared by the DDS and KTX2 decoders: block
//! compressed formats (BC1 to BC7 through `bcdec_rs`, ETC1, ETC2, EAC and
//! ASTC through `texture2ddecoder`) and common uncompressed layouts.
//! LDR formats decode to `Rgba8`, HDR formats (BC6H, half and full float)
//! to linear `RgbaF32`.

use crate::util::{self, check_size, corrupt, unsupported};
use pandite_core::{Error, ImageBuf, PixelFormat, Result};

/// A texel layout the decoders can convert.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tex {
    Bc1,
    Bc2,
    Bc3,
    Bc4 {
        signed: bool,
    },
    Bc5 {
        signed: bool,
    },
    Bc6h {
        signed: bool,
    },
    Bc7,
    Etc1,
    Etc2Rgb,
    Etc2Rgba1,
    Etc2Rgba8,
    EacR11 {
        signed: bool,
    },
    EacRg11 {
        signed: bool,
    },
    Astc {
        bw: u8,
        bh: u8,
    },
    /// Uncompressed integer pixels: bytes per pixel and one bit mask per
    /// channel (red, green, blue, alpha; zero when absent).
    Masked {
        bytes: u8,
        masks: [u32; 4],
        luminance: bool,
    },
    Rgba16Unorm,
    Rgba16Float,
    Rgba32Float,
    Rgb32Float,
    R16Float,
    Rg16Float,
    R32Float,
    Rg32Float,
    R11G11B10Float,
    Rgb9E5,
}

impl Tex {
    /// Block width, block height and bytes per block (1 x 1 blocks for
    /// uncompressed formats).
    fn block(self) -> (usize, usize, usize) {
        match self {
            Tex::Bc1
            | Tex::Bc4 { .. }
            | Tex::Etc1
            | Tex::Etc2Rgb
            | Tex::Etc2Rgba1
            | Tex::EacR11 { .. } => (4, 4, 8),
            Tex::Bc2
            | Tex::Bc3
            | Tex::Bc5 { .. }
            | Tex::Bc6h { .. }
            | Tex::Bc7
            | Tex::Etc2Rgba8
            | Tex::EacRg11 { .. } => (4, 4, 16),
            Tex::Astc { bw, bh } => (usize::from(bw), usize::from(bh), 16),
            Tex::Masked { bytes, .. } => (1, 1, usize::from(bytes)),
            Tex::Rgba16Unorm | Tex::Rgba16Float | Tex::Rg32Float => (1, 1, 8),
            Tex::Rgba32Float => (1, 1, 16),
            Tex::Rgb32Float => (1, 1, 12),
            Tex::R16Float => (1, 1, 2),
            Tex::Rg16Float | Tex::R32Float | Tex::R11G11B10Float | Tex::Rgb9E5 => (1, 1, 4),
        }
    }

    /// True for formats that decode to linear floating point.
    pub(crate) fn is_hdr(self) -> bool {
        matches!(
            self,
            Tex::Bc6h { .. }
                | Tex::Rgba16Float
                | Tex::Rgba32Float
                | Tex::Rgb32Float
                | Tex::R16Float
                | Tex::Rg16Float
                | Tex::R32Float
                | Tex::Rg32Float
                | Tex::R11G11B10Float
                | Tex::Rgb9E5
        )
    }

    /// True when the format can carry alpha.
    pub(crate) fn has_alpha(self) -> bool {
        match self {
            Tex::Bc1 | Tex::Bc2 | Tex::Bc3 | Tex::Bc7 | Tex::Etc2Rgba1 | Tex::Etc2Rgba8 => true,
            Tex::Astc { .. } | Tex::Rgba16Unorm | Tex::Rgba16Float | Tex::Rgba32Float => true,
            Tex::Masked { masks, .. } => masks[3] != 0,
            _ => false,
        }
    }

    /// Fill the compression and channel facts of this layout: block
    /// formats name their codec and give bits per pixel, uncompressed
    /// layouts their channels and sample depth.
    pub(crate) fn describe(self, f: &mut pandite_core::ImageFacts) {
        let (codec, channels, count, bits, bpp): (&str, &str, u8, Option<u8>, Option<u16>) =
            match self {
                Tex::Bc1 => ("BC1 (DXT1)", "RGBA", 4, None, Some(4)),
                Tex::Bc2 => ("BC2 (DXT3)", "RGBA", 4, None, Some(8)),
                Tex::Bc3 => ("BC3 (DXT5)", "RGBA", 4, None, Some(8)),
                Tex::Bc4 { .. } => ("BC4", "Red", 1, None, Some(4)),
                Tex::Bc5 { .. } => ("BC5", "Red, green", 2, None, Some(8)),
                Tex::Bc6h { .. } => ("BC6H, half float", "RGB", 3, None, Some(8)),
                Tex::Bc7 => ("BC7", "RGBA", 4, None, Some(8)),
                Tex::Etc1 => ("ETC1", "RGB", 3, None, Some(4)),
                Tex::Etc2Rgb => ("ETC2", "RGB", 3, None, Some(4)),
                Tex::Etc2Rgba1 => ("ETC2, 1-bit alpha", "RGBA", 4, None, Some(4)),
                Tex::Etc2Rgba8 => ("ETC2 with EAC alpha", "RGBA", 4, None, Some(8)),
                Tex::EacR11 { .. } => ("EAC R11", "Red", 1, None, Some(4)),
                Tex::EacRg11 { .. } => ("EAC RG11", "Red, green", 2, None, Some(8)),
                // ASTC block sizes give fractional bits per pixel.
                Tex::Astc { .. } => ("ASTC", "RGBA", 4, None, None),
                Tex::Masked {
                    bytes,
                    masks,
                    luminance,
                } => {
                    let alpha = masks[3] != 0;
                    let (name, count) = match (luminance, alpha) {
                        (true, false) => ("Gray", 1),
                        (true, true) => ("Gray + alpha", 2),
                        (false, _) if masks[0] == 0 && masks[1] == 0 => ("Alpha", 1),
                        (false, false) if masks[2] == 0 => ("Red, green", 2),
                        (false, false) => ("RGB", 3),
                        (false, true) => ("RGBA", 4),
                    };
                    let widths: Vec<u32> = masks
                        .iter()
                        .filter(|m| **m != 0)
                        .map(|m| m.count_ones())
                        .collect();
                    let even = widths.windows(2).all(|w| w[0] == w[1]);
                    let bits = widths.first().filter(|_| even).map(|&b| b as u8);
                    (
                        "Uncompressed",
                        name,
                        count,
                        bits,
                        Some(u16::from(bytes) * 8),
                    )
                }
                Tex::Rgba16Unorm => ("Uncompressed", "RGBA", 4, Some(16), None),
                Tex::Rgba16Float => ("Uncompressed, half float", "RGBA", 4, Some(16), None),
                Tex::Rgba32Float => ("Uncompressed, float", "RGBA", 4, Some(32), None),
                Tex::Rgb32Float => ("Uncompressed, float", "RGB", 3, Some(32), None),
                Tex::R16Float => ("Uncompressed, half float", "Red", 1, Some(16), None),
                Tex::Rg16Float => ("Uncompressed, half float", "Red, green", 2, Some(16), None),
                Tex::R32Float => ("Uncompressed, float", "Red", 1, Some(32), None),
                Tex::Rg32Float => ("Uncompressed, float", "Red, green", 2, Some(32), None),
                Tex::R11G11B10Float => ("Packed float (R11G11B10)", "RGB", 3, None, Some(32)),
                Tex::Rgb9E5 => ("Shared exponent (RGB9E5)", "RGB", 3, None, Some(32)),
            };
        f.compression = Some(match self {
            Tex::Astc { bw, bh } => format!("{codec} {bw}x{bh}"),
            _ => codec.to_string(),
        });
        f.channels = Some(channels.to_string());
        f.channel_count = Some(count);
        f.bits_per_channel = bits;
        f.bits_per_pixel = bpp;
    }

    /// Bytes needed for one `w` x `h` image.
    pub(crate) fn size(self, w: u32, h: u32) -> Option<usize> {
        let (bw, bh, bb) = self.block();
        (w as usize)
            .div_ceil(bw)
            .checked_mul((h as usize).div_ceil(bh))?
            .checked_mul(bb)
    }
}

/// Decode one `w` x `h` image stored in `fmt`.
pub(crate) fn decode(fmt: Tex, data: &[u8], w: u32, h: u32, what: &str) -> Result<ImageBuf> {
    let need = fmt.size(w, h).ok_or(Error::TooLarge {
        width: w,
        height: h,
    })?;
    let data = data
        .get(..need)
        .ok_or_else(|| corrupt(what, "truncated texture data"))?;
    if fmt == Tex::Rgba16Unorm {
        // Keep all 16 bits.
        check_size(w, h, PixelFormat::Rgba16)?;
        let samples: Vec<u16> = data
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        return util::rgba16_image(w, h, &samples);
    }
    if fmt.is_hdr() {
        check_size(w, h, PixelFormat::RgbaF32)?;
        let px = decode_float(fmt, data, w as usize, h as usize)?;
        util::rgbaf32_image(w, h, &px)
    } else {
        let len = check_size(w, h, PixelFormat::Rgba8)?;
        let mut out = util::alloc(len, w, h)?;
        decode_ldr(fmt, data, w as usize, h as usize, &mut out, what)?;
        util::rgba8_image(w, h, out)
    }
}

/// Run `f` for each block; `f` fills a scratch tile of `bw * bh` RGBA8
/// pixels which is then clipped into the image.
fn for_blocks(
    data: &[u8],
    w: usize,
    h: usize,
    (bw, bh, bb): (usize, usize, usize),
    out: &mut [u8],
    mut f: impl FnMut(&[u8], &mut [u8]),
) {
    let bx = w.div_ceil(bw);
    let mut tile = vec![0u8; bw * bh * 4];
    for (i, block) in data.chunks_exact(bb).enumerate() {
        let (x0, y0) = ((i % bx) * bw, (i / bx) * bh);
        if y0 >= h {
            break;
        }
        f(block, &mut tile);
        for ty in 0..bh.min(h - y0) {
            let cols = bw.min(w - x0);
            let src = &tile[ty * bw * 4..ty * bw * 4 + cols * 4];
            let o = ((y0 + ty) * w + x0) * 4;
            out[o..o + cols * 4].copy_from_slice(src);
        }
    }
}

/// Convert texture2ddecoder output (u32 holding B, G, R, A bytes) to RGBA8.
fn bgra_u32_to_rgba(src: &[u32], tile: &mut [u8]) {
    for (t, v) in tile.chunks_exact_mut(4).zip(src) {
        let [b, g, r, a] = v.to_le_bytes();
        t.copy_from_slice(&[r, g, b, a]);
    }
}

fn snorm_to_u8(v: f32) -> u8 {
    ((v.clamp(-1.0, 1.0) * 0.5 + 0.5) * 255.0 + 0.5) as u8
}

fn decode_ldr(fmt: Tex, data: &[u8], w: usize, h: usize, out: &mut [u8], what: &str) -> Result<()> {
    let block = fmt.block();
    match fmt {
        Tex::Bc1 => for_blocks(data, w, h, block, out, |b, t| bcdec_rs::bc1(b, t, 16)),
        Tex::Bc2 => for_blocks(data, w, h, block, out, |b, t| bcdec_rs::bc2(b, t, 16)),
        Tex::Bc3 => for_blocks(data, w, h, block, out, |b, t| bcdec_rs::bc3(b, t, 16)),
        Tex::Bc7 => for_blocks(data, w, h, block, out, |b, t| bcdec_rs::bc7(b, t, 16)),
        Tex::Bc4 { signed } => for_blocks(data, w, h, block, out, |b, t| {
            let mut r = [0f32; 16];
            bcdec_rs::bc4_float(b, &mut r, 4, signed);
            for (px, v) in t.chunks_exact_mut(4).zip(r) {
                let g = if signed {
                    snorm_to_u8(v)
                } else {
                    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                };
                px.copy_from_slice(&[g, g, g, 255]);
            }
        }),
        Tex::Bc5 { signed } => for_blocks(data, w, h, block, out, |b, t| {
            let mut rg = [0f32; 32];
            bcdec_rs::bc5_float(b, &mut rg, 8, signed);
            for (px, v) in t.chunks_exact_mut(4).zip(rg.chunks_exact(2)) {
                let c = |x: f32| {
                    if signed {
                        snorm_to_u8(x)
                    } else {
                        (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
                    }
                };
                px.copy_from_slice(&[c(v[0]), c(v[1]), 0, 255]);
            }
        }),
        Tex::Etc1
        | Tex::Etc2Rgb
        | Tex::Etc2Rgba1
        | Tex::Etc2Rgba8
        | Tex::EacR11 { .. }
        | Tex::EacRg11 { .. }
        | Tex::Astc { .. } => {
            let mut px = [0u32; 144];
            let n = block.0 * block.1;
            for_blocks(data, w, h, block, out, |b, t| {
                let px = &mut px[..n];
                match fmt {
                    Tex::Etc1 => texture2ddecoder::decode_etc1_block(b, px),
                    Tex::Etc2Rgb => texture2ddecoder::decode_etc2_rgb_block(b, px),
                    Tex::Etc2Rgba1 => texture2ddecoder::decode_etc2_rgba1_block(b, px),
                    Tex::Etc2Rgba8 => texture2ddecoder::decode_etc2_rgba8_block(b, px),
                    Tex::EacR11 { signed: false } => texture2ddecoder::decode_eacr_block(b, px),
                    Tex::EacR11 { signed: true } => {
                        texture2ddecoder::decode_eacr_signed_block(b, px)
                    }
                    Tex::EacRg11 { signed: false } => texture2ddecoder::decode_eacrg_block(b, px),
                    Tex::EacRg11 { signed: true } => {
                        texture2ddecoder::decode_eacrg_signed_block(b, px)
                    }
                    Tex::Astc { bw, bh } => {
                        texture2ddecoder::decode_astc_block(b, usize::from(bw), usize::from(bh), px)
                    }
                    _ => {}
                }
                bgra_u32_to_rgba(px, t);
            });
        }
        Tex::Masked {
            bytes,
            masks,
            luminance,
        } => {
            let fields: Vec<(u32, u32, u32)> = masks
                .iter()
                .map(|&m| {
                    if m == 0 {
                        (0, 0, 0)
                    } else {
                        let s = m.trailing_zeros();
                        (m, s, m >> s)
                    }
                })
                .collect();
            let get = |v: u32, k: usize| {
                let (m, s, max) = fields[k];
                util::scale_to_u8((v & m) >> s, max)
            };
            for (o, p) in out
                .chunks_exact_mut(4)
                .zip(data.chunks_exact(usize::from(bytes)))
            {
                let mut b4 = [0u8; 4];
                b4[..p.len()].copy_from_slice(p);
                let v = u32::from_le_bytes(b4);
                let a = if masks[3] != 0 { get(v, 3) } else { 255 };
                if luminance {
                    let l = get(v, 0);
                    o.copy_from_slice(&[l, l, l, a]);
                } else if masks[0] == 0 && masks[1] == 0 && masks[2] == 0 {
                    o.copy_from_slice(&[255, 255, 255, a]);
                } else {
                    o.copy_from_slice(&[get(v, 0), get(v, 1), get(v, 2), a]);
                }
            }
        }
        other => return Err(unsupported(what, format!("{other:?}"))),
    }
    Ok(())
}

/// IEEE half to f32.
pub(crate) fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exp = i32::from((bits >> 10) & 0x1F);
    let frac = f32::from(bits & 0x3FF);
    match exp {
        0 => sign * frac * 2f32.powi(-24),
        31 => {
            if frac == 0.0 {
                sign * f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => sign * (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    }
}

/// Unsigned float with `mbits` mantissa bits and a 5-bit exponent (the
/// packed 11 and 10-bit formats).
fn small_float(v: u32, mbits: u32) -> f32 {
    let exp = ((v >> mbits) & 0x1F) as i32;
    let frac = (v & ((1 << mbits) - 1)) as f32;
    let scale = (1u32 << mbits) as f32;
    match exp {
        0 => frac / scale * 2f32.powi(-14),
        31 => f32::INFINITY,
        e => (1.0 + frac / scale) * 2f32.powi(e - 15),
    }
}

fn decode_float(fmt: Tex, data: &[u8], w: usize, h: usize) -> Result<Vec<f32>> {
    let n = w * h;
    let mut out = Vec::new();
    out.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
        width: w as u32,
        height: h as u32,
    })?;
    let f32_at = |p: &[u8], i: usize| {
        f32::from_le_bytes([p[4 * i], p[4 * i + 1], p[4 * i + 2], p[4 * i + 3]])
    };
    let h16_at = |p: &[u8], i: usize| half(u16::from_le_bytes([p[2 * i], p[2 * i + 1]]));
    match fmt {
        Tex::Bc6h { signed } => {
            out.resize(n * 4, 1.0);
            let bx = w.div_ceil(4);
            let mut tile = [0f32; 48];
            for (i, block) in data.chunks_exact(16).enumerate() {
                let (x0, y0) = ((i % bx) * 4, (i / bx) * 4);
                if y0 >= h {
                    break;
                }
                bcdec_rs::bc6h_float(block, &mut tile, 12, signed);
                for ty in 0..4.min(h - y0) {
                    for tx in 0..4.min(w - x0) {
                        let o = ((y0 + ty) * w + x0 + tx) * 4;
                        let s = ty * 12 + tx * 3;
                        out[o..o + 3].copy_from_slice(&tile[s..s + 3]);
                    }
                }
            }
        }
        Tex::Rgba32Float => {
            for p in data.chunks_exact(16) {
                out.extend_from_slice(&[f32_at(p, 0), f32_at(p, 1), f32_at(p, 2), f32_at(p, 3)]);
            }
        }
        Tex::Rgb32Float => {
            for p in data.chunks_exact(12) {
                out.extend_from_slice(&[f32_at(p, 0), f32_at(p, 1), f32_at(p, 2), 1.0]);
            }
        }
        Tex::Rgba16Float => {
            for p in data.chunks_exact(8) {
                out.extend_from_slice(&[h16_at(p, 0), h16_at(p, 1), h16_at(p, 2), h16_at(p, 3)]);
            }
        }
        Tex::R16Float => {
            for p in data.chunks_exact(2) {
                let v = h16_at(p, 0);
                out.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        Tex::Rg16Float => {
            for p in data.chunks_exact(4) {
                out.extend_from_slice(&[h16_at(p, 0), h16_at(p, 1), 0.0, 1.0]);
            }
        }
        Tex::R32Float => {
            for p in data.chunks_exact(4) {
                let v = f32_at(p, 0);
                out.extend_from_slice(&[v, v, v, 1.0]);
            }
        }
        Tex::Rg32Float => {
            for p in data.chunks_exact(8) {
                out.extend_from_slice(&[f32_at(p, 0), f32_at(p, 1), 0.0, 1.0]);
            }
        }
        Tex::R11G11B10Float => {
            for p in data.chunks_exact(4) {
                let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                out.extend_from_slice(&[
                    small_float(v & 0x7FF, 6),
                    small_float((v >> 11) & 0x7FF, 6),
                    small_float((v >> 22) & 0x3FF, 5),
                    1.0,
                ]);
            }
        }
        Tex::Rgb9E5 => {
            for p in data.chunks_exact(4) {
                let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                let scale = 2f32.powi((v >> 27) as i32 - 15 - 9);
                out.extend_from_slice(&[
                    (v & 0x1FF) as f32 * scale,
                    ((v >> 9) & 0x1FF) as f32 * scale,
                    ((v >> 18) & 0x1FF) as f32 * scale,
                    1.0,
                ]);
            }
        }
        _ => return Err(unsupported("texture", format!("{fmt:?}"))),
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_floats() {
        assert_eq!(half(0x3C00), 1.0);
        assert_eq!(half(0xC000), -2.0);
        assert_eq!(half(0x3800), 0.5);
        assert_eq!(half(0), 0.0);
        assert!(half(0x7C00).is_infinite());
    }

    #[test]
    fn sizes_round_up_to_blocks() {
        assert_eq!(Tex::Bc1.size(5, 5), Some(4 * 8));
        assert_eq!(Tex::Bc7.size(4, 4), Some(16));
        assert_eq!(Tex::Astc { bw: 6, bh: 6 }.size(7, 6), Some(32));
        assert_eq!(Tex::Rgba32Float.size(2, 1), Some(32));
    }
}

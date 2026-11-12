//! Photoshop PSD and PSB decoder for the merged (composite) image, written
//! from the Adobe file format specification.
//!
//! Reads bitmap, grayscale, duotone, indexed, RGB, CMYK, multichannel and
//! Lab documents at 1, 8, 16 and 32 bits, raw, PackBits RLE and ZIP data.
//! The merged alpha channel is used when the layer section says the
//! composite has transparency. The embedded ICC profile applies. 16-bit
//! documents decode to `Rgba16`, 32-bit documents to linear `RgbaF32`.
//! Layers are not composited here: files saved without "maximize
//! compatibility" show Photoshop's stored composite.

use crate::icc;
use crate::util::{self, be16, be32, check_size, corrupt, unsupported};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};
use std::io::Read;

const FORMAT: &str = "psd";

/// Decodes PSD and PSB files.
#[derive(Clone, Copy, Debug, Default)]
pub struct PsdDecoder;

struct Header {
    psb: bool,
    channels: usize,
    height: u32,
    width: u32,
    depth: u16,
    mode: u16,
}

fn header(d: &[u8]) -> Result<Header> {
    if d.len() < 26 || &d[..4] != b"8BPS" {
        return Err(corrupt(FORMAT, "missing 8BPS signature"));
    }
    let version = be16(d, 4).unwrap_or(0);
    if version != 1 && version != 2 {
        return Err(corrupt(FORMAT, "bad version"));
    }
    let h = Header {
        psb: version == 2,
        channels: usize::from(be16(d, 12).unwrap_or(0)),
        height: be32(d, 14).unwrap_or(0),
        width: be32(d, 18).unwrap_or(0),
        depth: be16(d, 22).unwrap_or(0),
        mode: be16(d, 24).unwrap_or(0),
    };
    if h.channels == 0 || h.channels > 56 || h.width == 0 || h.height == 0 {
        return Err(corrupt(FORMAT, "bad header"));
    }
    if !matches!(h.depth, 1 | 8 | 16 | 32) {
        return Err(corrupt(FORMAT, "bad depth"));
    }
    Ok(h)
}

/// Color channels of a mode, before any alpha.
fn color_channels(mode: u16) -> Result<usize> {
    Ok(match mode {
        0..=2 | 7 | 8 => 1,
        3 | 9 => 3,
        4 => 4,
        m => return Err(unsupported(FORMAT, format!("color mode {m}"))),
    })
}

impl DecoderProvider for PsdDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "Photoshop document"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["psd"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"8BPS")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = header(source.bytes())?;
        Ok(ImageMeta {
            width: h.width,
            height: h.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: h.channels > color_channels(h.mode).unwrap_or(1),
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        util::guard(FORMAT, || decode(source.bytes(), &options.cancel)).map(Content::Still)
    }
}

/// A cursor over big-endian sections.
struct Cur<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    fn u16(&mut self) -> Result<u16> {
        let v = be16(self.d, self.pos).ok_or_else(|| corrupt(FORMAT, "truncated"))?;
        self.pos += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32> {
        let v = be32(self.d, self.pos).ok_or_else(|| corrupt(FORMAT, "truncated"))?;
        self.pos += 4;
        Ok(v)
    }

    fn len(&mut self, wide: bool) -> Result<usize> {
        if wide {
            let hi = u64::from(self.u32()?);
            let lo = u64::from(self.u32()?);
            usize::try_from((hi << 32) | lo).map_err(|_| corrupt(FORMAT, "section too large"))
        } else {
            Ok(self.u32()? as usize)
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self
            .d
            .get(
                self.pos
                    ..self
                        .pos
                        .checked_add(n)
                        .ok_or_else(|| corrupt(FORMAT, "overflow"))?,
            )
            .ok_or_else(|| corrupt(FORMAT, "truncated section"))?;
        self.pos += n;
        Ok(s)
    }
}

/// Image resources: returns the ICC profile.
fn icc_profile(res: &[u8]) -> Option<Vec<u8>> {
    let mut pos = 0;
    while pos + 12 <= res.len() {
        if &res[pos..pos + 4] != b"8BIM" {
            return None;
        }
        let id = be16(res, pos + 4)?;
        let name_len = usize::from(res[pos + 6]);
        let name_total = (1 + name_len + 1) & !1;
        let at = pos + 6 + name_total;
        let size = be32(res, at)? as usize;
        let data = res.get(at + 4..at + 4 + size)?;
        if id == 1039 {
            return Some(data.to_vec());
        }
        pos = at + 4 + ((size + 1) & !1);
    }
    None
}

/// True when the layer section marks the merged image as transparent
/// (negative layer count, also inside Lr16 and Lr32 blocks).
fn merged_alpha(section: &[u8], psb: bool) -> bool {
    let mut c = Cur { d: section, pos: 0 };
    let Ok(info_len) = c.len(psb) else {
        return false;
    };
    if info_len >= 2 {
        return c.u16().map(|v| (v as i16) < 0).unwrap_or(false);
    }
    // Skip the layer info and the global mask, then scan tagged blocks.
    c.pos += info_len;
    let Ok(mask_len) = c.u32() else {
        return false;
    };
    c.pos += mask_len as usize;
    while c.pos + 12 <= section.len() {
        let sig = &section[c.pos..c.pos + 4];
        if sig != b"8BIM" && sig != b"8B64" {
            break;
        }
        let key = &section[c.pos + 4..c.pos + 8];
        c.pos += 8;
        let wide = psb
            && matches!(
                key,
                b"Lr16"
                    | b"Lr32"
                    | b"Layr"
                    | b"Mt16"
                    | b"Mt32"
                    | b"Mtrn"
                    | b"Alph"
                    | b"FMsk"
                    | b"lnk2"
                    | b"FEid"
                    | b"FXid"
                    | b"PxSD"
            );
        let Ok(len) = c.len(wide) else {
            return false;
        };
        if matches!(key, b"Lr16" | b"Lr32" | b"Layr") && len >= 2 {
            return c.u16().map(|v| (v as i16) < 0).unwrap_or(false);
        }
        c.pos += (len + 3) & !3;
    }
    false
}

fn decode(d: &[u8], cancel: &CancelToken) -> Result<ImageBuf> {
    let h = header(d)?;
    let (w, ht) = (h.width, h.height);
    // Limits for the output layout before any plane is allocated.
    let out_format = match h.depth {
        16 => PixelFormat::Rgba16,
        32 => PixelFormat::RgbaF32,
        _ => PixelFormat::Rgba8,
    };
    check_size(w, ht, out_format)?;
    let base = color_channels(h.mode)?;
    if h.channels < base && h.mode != 7 {
        return Err(corrupt(FORMAT, "too few channels"));
    }
    let mut c = Cur { d, pos: 26 };
    let mode_len = c.u32()? as usize;
    let mode_data = c.take(mode_len)?;
    let res_len = c.u32()? as usize;
    let resources = c.take(res_len)?;
    let layer_len = c.len(h.psb)?;
    let layers = c.take(layer_len)?;
    let has_alpha = h.channels > base && merged_alpha(layers, h.psb);
    let wanted = (base + usize::from(has_alpha)).min(h.channels);

    let compression = c.u16()?;
    let n = w as usize * ht as usize;
    let row_bytes = (w as usize * usize::from(h.depth)).div_ceil(8);
    let plane_bytes = row_bytes * ht as usize;
    let total = plane_bytes.checked_mul(wanted).ok_or(Error::TooLarge {
        width: w,
        height: ht,
    })?;
    let mut planes = util::alloc(total, w, ht)?;
    let rows_all = ht as usize * h.channels;
    match compression {
        0 => {
            let src = c.take(total)?;
            planes.copy_from_slice(src);
        }
        1 => {
            let count_size = if h.psb { 4 } else { 2 };
            let table = c.take(rows_all * count_size)?;
            let counts: Vec<usize> = table
                .chunks_exact(count_size)
                .map(|b| {
                    if h.psb {
                        u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
                    } else {
                        usize::from(u16::from_be_bytes([b[0], b[1]]))
                    }
                })
                .collect();
            for (row, &len) in counts.iter().enumerate().take(ht as usize * wanted) {
                if row % 256 == 0 {
                    cancel.check()?;
                }
                let src = c.take(len)?;
                let dst = &mut planes[row * row_bytes..(row + 1) * row_bytes];
                if packbits_into(src, dst) != row_bytes {
                    return Err(corrupt(FORMAT, "short RLE row"));
                }
            }
        }
        2 | 3 => {
            let mut z = flate2::read::ZlibDecoder::new(&d[c.pos..]);
            z.read_exact(&mut planes).map_err(|e| corrupt(FORMAT, e))?;
            if compression == 3 {
                unpredict(&mut planes, row_bytes, h.depth)?;
            }
        }
        m => return Err(unsupported(FORMAT, format!("compression {m}"))),
    }
    cancel.check()?;

    let plane = |ch: usize| &planes[ch * plane_bytes..(ch + 1) * plane_bytes];
    let profile = icc_profile(resources);
    match h.depth {
        1 => {
            let src = plane(0);
            let mut out = util::alloc_rgba8(w, ht)?;
            for y in 0..ht as usize {
                for x in 0..w as usize {
                    let bit = src[y * row_bytes + x / 8] & (0x80 >> (x % 8)) != 0;
                    let v = if bit { 0 } else { 255 };
                    let i = (y * w as usize + x) * 4;
                    out[i..i + 4].copy_from_slice(&[v, v, v, 255]);
                }
            }
            util::rgba8_image(w, ht, out)
        }
        8 => {
            let mut out = util::alloc_rgba8(w, ht)?;
            let get = |ch: usize, i: usize| plane(ch)[i];
            match h.mode {
                4 => {
                    // Photoshop stores CMYK inverted: 255 means no ink.
                    let mut cmyk = Vec::with_capacity(n * 4);
                    for i in 0..n {
                        cmyk.extend_from_slice(&[
                            255 - get(0, i),
                            255 - get(1, i),
                            255 - get(2, i),
                            255 - get(3, i),
                        ]);
                    }
                    if !icc::cmyk_to_srgb_rgba8(profile.as_deref(), &cmyk, &mut out) {
                        for (o, p) in out.chunks_exact_mut(4).zip(cmyk.chunks_exact(4)) {
                            let [r, g, b] = util::cmyk_to_rgb(p[0], p[1], p[2], p[3]);
                            o.copy_from_slice(&[r, g, b, 255]);
                        }
                    }
                }
                2 => {
                    if mode_data.len() < 768 {
                        return Err(corrupt(FORMAT, "missing palette"));
                    }
                    for i in 0..n {
                        let k = usize::from(get(0, i));
                        out[i * 4..i * 4 + 4].copy_from_slice(&[
                            mode_data[k],
                            mode_data[256 + k],
                            mode_data[512 + k],
                            255,
                        ]);
                    }
                }
                9 => {
                    for i in 0..n {
                        let l = f32::from(get(0, i)) * 100.0 / 255.0;
                        let a = f32::from(get(1, i)) - 128.0;
                        let b = f32::from(get(2, i)) - 128.0;
                        let [r, g, bl] = lab_to_srgb(l, a, b);
                        out[i * 4..i * 4 + 4].copy_from_slice(&[r, g, bl, 255]);
                    }
                }
                3 => {
                    for i in 0..n {
                        out[i * 4..i * 4 + 4].copy_from_slice(&[
                            get(0, i),
                            get(1, i),
                            get(2, i),
                            255,
                        ]);
                    }
                    icc::to_srgb_rgba8(profile.as_deref(), &mut out);
                }
                _ => {
                    for i in 0..n {
                        let v = get(0, i);
                        out[i * 4..i * 4 + 4].copy_from_slice(&[v, v, v, 255]);
                    }
                }
            }
            if has_alpha {
                for i in 0..n {
                    out[i * 4 + 3] = get(base, i);
                }
            }
            util::rgba8_image(w, ht, out)
        }
        16 => {
            let get = |ch: usize, i: usize| {
                let p = plane(ch);
                u16::from_be_bytes([p[i * 2], p[i * 2 + 1]])
            };
            if matches!(h.mode, 4 | 9 | 2) {
                return Err(unsupported(FORMAT, "16-bit CMYK, Lab or indexed"));
            }
            let mut rgba = Vec::with_capacity(n * 4);
            for i in 0..n {
                let a = if has_alpha { get(base, i) } else { u16::MAX };
                if h.mode == 3 {
                    rgba.extend_from_slice(&[get(0, i), get(1, i), get(2, i), a]);
                } else {
                    let v = get(0, i);
                    rgba.extend_from_slice(&[v, v, v, a]);
                }
            }
            if h.mode == 3 {
                icc::to_srgb_rgba16(profile.as_deref(), &mut rgba);
            }
            util::rgba16_image(w, ht, &rgba)
        }
        _ => {
            let get = |ch: usize, i: usize| {
                let p = plane(ch);
                f32::from_be_bytes([p[i * 4], p[i * 4 + 1], p[i * 4 + 2], p[i * 4 + 3]])
            };
            if !matches!(h.mode, 1 | 3) {
                return Err(unsupported(FORMAT, "32-bit mode other than gray or RGB"));
            }
            let mut rgba = Vec::with_capacity(n * 4);
            for i in 0..n {
                let a = if has_alpha { get(base, i) } else { 1.0 };
                if h.mode == 3 {
                    rgba.extend_from_slice(&[get(0, i), get(1, i), get(2, i), a]);
                } else {
                    let v = get(0, i);
                    rgba.extend_from_slice(&[v, v, v, a]);
                }
            }
            util::rgbaf32_image(w, ht, &rgba)
        }
    }
}

/// Decode PackBits into `dst`, returning the bytes written.
fn packbits_into(src: &[u8], dst: &mut [u8]) -> usize {
    let (mut i, mut o) = (0, 0);
    while i < src.len() && o < dst.len() {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let len = (n as usize + 1).min(dst.len() - o).min(src.len() - i);
            dst[o..o + len].copy_from_slice(&src[i..i + len]);
            i += n as usize + 1;
            o += len;
        } else if n != -128 {
            let len = ((1 - isize::from(n)) as usize).min(dst.len() - o);
            let Some(&b) = src.get(i) else {
                break;
            };
            dst[o..o + len].fill(b);
            i += 1;
            o += len;
        }
    }
    o
}

/// Undo ZIP-with-prediction (horizontal delta per row) for 8 and 16 bits.
fn unpredict(data: &mut [u8], row_bytes: usize, depth: u16) -> Result<()> {
    match depth {
        8 => {
            for row in data.chunks_exact_mut(row_bytes) {
                for x in 1..row.len() {
                    row[x] = row[x].wrapping_add(row[x - 1]);
                }
            }
        }
        16 => {
            for row in data.chunks_exact_mut(row_bytes) {
                for x in (2..row.len()).step_by(2) {
                    let prev = u16::from_be_bytes([row[x - 2], row[x - 1]]);
                    let cur = u16::from_be_bytes([row[x], row[x + 1]]);
                    row[x..x + 2].copy_from_slice(&cur.wrapping_add(prev).to_be_bytes());
                }
            }
        }
        _ => return Err(unsupported(FORMAT, "ZIP prediction for this depth")),
    }
    Ok(())
}

/// CIE L*a*b* (D50, as Photoshop uses it) to 8-bit sRGB.
fn lab_to_srgb(l: f32, a: f32, b: f32) -> [u8; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let inv = |t: f32| {
        if t > 6.0 / 29.0 {
            t * t * t
        } else {
            3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0)
        }
    };
    let (x, y, z) = (0.964_22 * inv(fx), inv(fy), 0.825_21 * inv(fz));
    // Bradford-adapted D50 XYZ to linear sRGB.
    let r = 3.133_856 * x - 1.616_867 * y - 0.490_614_6 * z;
    let g = -0.978_768_4 * x + 1.916_141_5 * y + 0.033_454 * z;
    let bl = 0.071_945_3 * x - 0.228_991_1 * y + 1.405_242_7 * z;
    let enc = |v: f32| (util::linear_to_srgb(v) * 255.0 + 0.5) as u8;
    [enc(r), enc(g), enc(bl)]
}

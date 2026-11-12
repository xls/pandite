//! Radiance HDR (RGBE) decoder, written from the Radiance file format
//! description. Reads flat, old-style RLE and adaptive RLE scanlines, RGBE
//! and XYZE data and all eight resolution string orientations. Pixels come
//! back as linear `RgbaF32` with alpha 1.

use crate::util::{self, check_size, corrupt, unsupported};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, ImageBuf, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "hdr";

/// Decodes Radiance HDR files.
#[derive(Clone, Copy, Debug, Default)]
pub struct HdrDecoder;

struct Header {
    width: u32,
    height: u32,
    xyz: bool,
    /// Transform from the stored scan order to an upright image.
    orientation: Orientation,
    /// Offset of the first scanline.
    data_start: usize,
}

fn header(d: &[u8]) -> Result<Header> {
    if !(d.starts_with(b"#?RADIANCE") || d.starts_with(b"#?RGBE") || d.starts_with(b"#?")) {
        return Err(corrupt(FORMAT, "missing #? signature"));
    }
    let mut pos = 0;
    let mut xyz = false;
    let line = |pos: &mut usize| -> Option<&[u8]> {
        let start = *pos;
        let len = d
            .get(start..)?
            .iter()
            .take(4096)
            .position(|&b| b == b'\n')?;
        *pos = start + len + 1;
        Some(&d[start..start + len])
    };
    // Header lines until the blank line.
    loop {
        let l = line(&mut pos).ok_or_else(|| corrupt(FORMAT, "unterminated header"))?;
        let l = l.strip_suffix(b"\r").unwrap_or(l);
        if l.is_empty() {
            break;
        }
        if let Some(fmt) = l.strip_prefix(b"FORMAT=") {
            match fmt {
                b"32-bit_rle_rgbe" => xyz = false,
                b"32-bit_rle_xyze" => xyz = true,
                other => {
                    return Err(unsupported(
                        FORMAT,
                        format!("format {}", String::from_utf8_lossy(other)),
                    ))
                }
            }
        }
    }
    let res = line(&mut pos).ok_or_else(|| corrupt(FORMAT, "missing resolution"))?;
    let text = std::str::from_utf8(res).map_err(|_| corrupt(FORMAT, "bad resolution"))?;
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 4 {
        return Err(corrupt(FORMAT, "bad resolution"));
    }
    let n = |s: &str| {
        s.parse::<u32>()
            .map_err(|_| corrupt(FORMAT, "bad resolution"))
    };
    let (a, an, b, bn) = (parts[0], n(parts[1])?, parts[2], n(parts[3])?);
    // The first axis is the slow (scanline) axis: `an` scanlines of `bn`
    // pixels each. The orientation maps that stored raster upright.
    let orientation = match (a, b) {
        ("-Y", "+X") => Orientation::Normal,
        ("-Y", "-X") => Orientation::FlipH,
        ("+Y", "-X") => Orientation::Rotate180,
        ("+Y", "+X") => Orientation::FlipV,
        ("+X", "-Y") => Orientation::Transpose,
        ("-X", "-Y") => Orientation::Rotate90,
        ("-X", "+Y") => Orientation::Transverse,
        ("+X", "+Y") => Orientation::Rotate270,
        _ => return Err(corrupt(FORMAT, "bad resolution axes")),
    };
    if an == 0 || bn == 0 {
        return Err(corrupt(FORMAT, "zero image size"));
    }
    Ok(Header {
        width: bn,
        height: an,
        xyz,
        orientation,
        data_start: pos,
    })
}

impl DecoderProvider for HdrDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "Radiance HDR image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["hdr"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"#?RADIANCE") || header.starts_with(b"#?RGBE")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = header(source.bytes())?;
        let (width, height) = if h.orientation.swaps_axes() {
            (h.height, h.width)
        } else {
            (h.width, h.height)
        };
        Ok(ImageMeta {
            width,
            height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: false,
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        util::guard(FORMAT, || decode(source.bytes(), &options.cancel)).map(Content::Still)
    }
}

fn decode(d: &[u8], cancel: &CancelToken) -> Result<ImageBuf> {
    let h = header(d)?;
    check_size(h.width, h.height, PixelFormat::RgbaF32)?;
    let (w, ht) = (h.width as usize, h.height as usize);
    let mut rgbe = vec![0u8; w * 4];
    let mut out: Vec<f32> = Vec::new();
    out.try_reserve_exact(w * ht * 4)
        .map_err(|_| pandite_core::Error::TooLarge {
            width: h.width,
            height: h.height,
        })?;
    let mut pos = h.data_start;
    for y in 0..ht {
        if y % 32 == 0 {
            cancel.check()?;
        }
        pos = scanline(d, pos, &mut rgbe)?;
        for p in rgbe.chunks_exact(4) {
            let [r, g, b] = to_float(p, h.xyz);
            out.extend_from_slice(&[r, g, b, 1.0]);
        }
    }
    let image = util::rgbaf32_image(h.width, h.height, &out)?;
    crate::orient::apply(image, h.orientation)
}

/// RGBE (or XYZE) to linear RGB.
fn to_float(p: &[u8], xyz: bool) -> [f32; 3] {
    if p[3] == 0 {
        return [0.0; 3];
    }
    let f = 2f32.powi(i32::from(p[3]) - 136);
    let (a, b, c) = (
        f32::from(p[0]) * f,
        f32::from(p[1]) * f,
        f32::from(p[2]) * f,
    );
    if xyz {
        // CIE XYZ to linear sRGB primaries.
        [
            3.240_454_2 * a - 1.537_138_5 * b - 0.498_531_4 * c,
            -0.969_266 * a + 1.876_010_8 * b + 0.041_556 * c,
            0.055_643_4 * a - 0.204_025_9 * b + 1.057_225_2 * c,
        ]
    } else {
        [a, b, c]
    }
}

/// Read one scanline into `out` (RGBE quadruples). Returns the new position.
fn scanline(d: &[u8], mut pos: usize, out: &mut [u8]) -> Result<usize> {
    let w = out.len() / 4;
    let truncated = || corrupt(FORMAT, "truncated pixel data");
    let head = d.get(pos..pos + 4).ok_or_else(truncated)?;
    if (8..0x8000).contains(&w) && head[0] == 2 && head[1] == 2 && head[2] & 0x80 == 0 {
        // Adaptive RLE: four channel planes, each run-length coded.
        let len = (usize::from(head[2]) << 8) | usize::from(head[3]);
        if len != w {
            return Err(corrupt(FORMAT, "scanline length mismatch"));
        }
        pos += 4;
        for c in 0..4 {
            let mut x = 0;
            while x < w {
                let count = usize::from(*d.get(pos).ok_or_else(truncated)?);
                pos += 1;
                if count > 128 {
                    let run = count - 128;
                    if x + run > w {
                        return Err(corrupt(FORMAT, "run past scanline end"));
                    }
                    let v = *d.get(pos).ok_or_else(truncated)?;
                    pos += 1;
                    for k in 0..run {
                        out[(x + k) * 4 + c] = v;
                    }
                    x += run;
                } else {
                    if count == 0 || x + count > w {
                        return Err(corrupt(FORMAT, "bad literal run"));
                    }
                    let src = d.get(pos..pos + count).ok_or_else(truncated)?;
                    for (k, &v) in src.iter().enumerate() {
                        out[(x + k) * 4 + c] = v;
                    }
                    pos += count;
                    x += count;
                }
            }
        }
        return Ok(pos);
    }
    // Flat or old-style RLE: (1, 1, 1, n) repeats the previous pixel.
    let mut x = 0;
    let mut shift = 0;
    while x < w {
        let p = d.get(pos..pos + 4).ok_or_else(truncated)?;
        pos += 4;
        if p[0] == 1 && p[1] == 1 && p[2] == 1 {
            if x == 0 {
                return Err(corrupt(FORMAT, "repeat at scanline start"));
            }
            let count = usize::from(p[3]) << shift;
            let prev = [
                out[(x - 1) * 4],
                out[(x - 1) * 4 + 1],
                out[(x - 1) * 4 + 2],
                out[(x - 1) * 4 + 3],
            ];
            for _ in 0..count.min(w - x) {
                out[x * 4..x * 4 + 4].copy_from_slice(&prev);
                x += 1;
            }
            shift += 8;
        } else {
            out[x * 4..x * 4 + 4].copy_from_slice(p);
            x += 1;
            shift = 0;
        }
    }
    Ok(pos)
}

//! DirectDraw Surface (DDS) decoder. Shows the top mip level of the first
//! surface (first array slice, first cube face, first depth slice). Reads
//! legacy pixel formats (FourCC and bit masks) and DX10 headers.

use crate::texture::{self, Tex};
use crate::util::{check_size, corrupt, le32, unsupported};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageMeta, Orientation, PixelFormat,
    Result, Source,
};

const FORMAT: &str = "dds";

/// Decodes DDS textures.
#[derive(Clone, Copy, Debug, Default)]
pub struct DdsDecoder;

struct Header {
    width: u32,
    height: u32,
    format: Tex,
    data_at: usize,
    /// DXT2 and DXT4 store premultiplied color.
    premultiplied: bool,
}

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;

fn dxgi(code: u32) -> Result<Tex> {
    let masked = |bytes: u8, masks: [u32; 4]| Tex::Masked {
        bytes,
        masks,
        luminance: false,
    };
    Ok(match code {
        2 => Tex::Rgba32Float,
        6 => Tex::Rgb32Float,
        10 => Tex::Rgba16Float,
        11 => Tex::Rgba16Unorm,
        16 => Tex::Rg32Float,
        24 => masked(4, [0x3FF, 0xFFC00, 0x3FF0_0000, 0xC000_0000]),
        26 => Tex::R11G11B10Float,
        27..=29 => masked(4, [0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000]),
        34 => Tex::Rg16Float,
        41 => Tex::R32Float,
        49 => masked(2, [0xFF, 0xFF00, 0, 0]),
        54 => Tex::R16Float,
        56 => Tex::Masked {
            bytes: 2,
            masks: [0xFFFF, 0, 0, 0],
            luminance: true,
        },
        61 => Tex::Masked {
            bytes: 1,
            masks: [0xFF, 0, 0, 0],
            luminance: true,
        },
        65 => masked(1, [0, 0, 0, 0xFF]),
        67 => Tex::Rgb9E5,
        70..=72 => Tex::Bc1,
        73..=75 => Tex::Bc2,
        76..=78 => Tex::Bc3,
        79 | 80 => Tex::Bc4 { signed: false },
        81 => Tex::Bc4 { signed: true },
        82 | 83 => Tex::Bc5 { signed: false },
        84 => Tex::Bc5 { signed: true },
        85 => masked(2, [0xF800, 0x07E0, 0x001F, 0]),
        86 => masked(2, [0x7C00, 0x03E0, 0x001F, 0x8000]),
        87 | 90 | 91 => masked(4, [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000]),
        88 | 92 | 93 => masked(4, [0xFF_0000, 0xFF00, 0xFF, 0]),
        94 | 95 => Tex::Bc6h { signed: false },
        96 => Tex::Bc6h { signed: true },
        97..=99 => Tex::Bc7,
        115 => masked(2, [0x0F00, 0x00F0, 0x000F, 0xF000]),
        other => return Err(unsupported(FORMAT, format!("DXGI format {other}"))),
    })
}

fn header(d: &[u8]) -> Result<Header> {
    if d.len() < 128 || &d[..4] != b"DDS " || le32(d, 4) != Some(124) {
        return Err(corrupt(FORMAT, "missing DDS header"));
    }
    let height = le32(d, 12).unwrap_or(0);
    let width = le32(d, 16).unwrap_or(0);
    if width == 0 || height == 0 {
        return Err(corrupt(FORMAT, "zero image size"));
    }
    let pf_flags = le32(d, 80).unwrap_or(0);
    let fourcc = &d[84..88];
    let bits = le32(d, 88).unwrap_or(0);
    let masks = [
        le32(d, 92).unwrap_or(0),
        le32(d, 96).unwrap_or(0),
        le32(d, 100).unwrap_or(0),
        le32(d, 104).unwrap_or(0),
    ];
    let mut data_at = 128;
    let format = if pf_flags & DDPF_FOURCC != 0 {
        match fourcc {
            b"DXT1" => Tex::Bc1,
            b"DXT2" | b"DXT3" => Tex::Bc2,
            b"DXT4" | b"DXT5" | b"RXGB" => Tex::Bc3,
            b"ATI1" | b"BC4U" => Tex::Bc4 { signed: false },
            b"BC4S" => Tex::Bc4 { signed: true },
            b"ATI2" | b"BC5U" => Tex::Bc5 { signed: false },
            b"BC5S" => Tex::Bc5 { signed: true },
            b"ETC " | b"ETC1" => Tex::Etc1,
            b"DX10" => {
                if d.len() < 148 {
                    return Err(corrupt(FORMAT, "truncated DX10 header"));
                }
                data_at = 148;
                dxgi(le32(d, 128).unwrap_or(0))?
            }
            _ => match u32::from_le_bytes([fourcc[0], fourcc[1], fourcc[2], fourcc[3]]) {
                36 => Tex::Rgba16Unorm,
                111 => Tex::R16Float,
                112 => Tex::Rg16Float,
                113 => Tex::Rgba16Float,
                114 => Tex::R32Float,
                115 => Tex::Rg32Float,
                116 => Tex::Rgba32Float,
                _ => {
                    return Err(unsupported(
                        FORMAT,
                        format!("FourCC {}", String::from_utf8_lossy(fourcc)),
                    ))
                }
            },
        }
    } else if pf_flags & (DDPF_RGB | DDPF_LUMINANCE | DDPF_ALPHA) != 0 {
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(unsupported(FORMAT, format!("{bits}-bit pixels")));
        }
        let mut m = masks;
        if pf_flags & DDPF_ALPHAPIXELS == 0 && pf_flags & DDPF_ALPHA == 0 {
            m[3] = 0;
        }
        if pf_flags & DDPF_ALPHA != 0 && pf_flags & (DDPF_RGB | DDPF_LUMINANCE) == 0 {
            // Alpha-only surfaces keep their mask in the alpha slot.
            m = [0, 0, 0, if masks[3] != 0 { masks[3] } else { masks[0] }];
        }
        Tex::Masked {
            bytes: (bits / 8) as u8,
            masks: m,
            luminance: pf_flags & DDPF_LUMINANCE != 0,
        }
    } else {
        return Err(unsupported(FORMAT, "unknown pixel format"));
    };
    Ok(Header {
        width,
        height,
        format,
        data_at,
        premultiplied: pf_flags & DDPF_FOURCC != 0 && matches!(fourcc, b"DXT2" | b"DXT4"),
    })
}

impl DecoderProvider for DdsDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "DirectDraw Surface"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["dds"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"DDS ")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = header(source.bytes())?;
        Ok(ImageMeta {
            width: h.width,
            height: h.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: h.format.has_alpha(),
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let d = source.bytes();
        let h = header(d)?;
        let fmt = if h.format.is_hdr() {
            PixelFormat::RgbaF32
        } else {
            PixelFormat::Rgba8
        };
        check_size(h.width, h.height, fmt)?;
        crate::util::guard(FORMAT, || {
            let image = texture::decode(h.format, &d[h.data_at..], h.width, h.height, FORMAT)?;
            if !h.premultiplied {
                return Ok(Content::Still(image));
            }
            let mut px = image.data().to_vec();
            for p in px.chunks_exact_mut(4) {
                let a = u32::from(p[3]);
                if a != 0 && a != 255 {
                    for c in &mut p[..3] {
                        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            crate::util::rgba8_image(h.width, h.height, px).map(Content::Still)
        })
    }
}

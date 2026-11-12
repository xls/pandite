//! KTX 2.0 texture decoder. Shows the top mip level of the first layer and
//! face. Supports uncompressed, Zstandard and ZLIB supercompression, BC,
//! ETC, EAC and ASTC block formats and common uncompressed Vulkan formats,
//! and honors the `KTXorientation` key. Basis Universal payloads (BasisLZ
//! supercompression or UASTC) are not supported.

use crate::texture::{self, Tex};
use crate::util::{check_size, corrupt, le32, unsupported};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageMeta, Orientation,
    PixelFormat, Result, Source,
};
use std::io::Read;

const FORMAT: &str = "ktx2";
const MAGIC: [u8; 12] = [
    0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];

/// Decodes KTX2 textures.
#[derive(Clone, Copy, Debug, Default)]
pub struct Ktx2Decoder;

struct Header {
    width: u32,
    height: u32,
    format: Tex,
    scheme: u32,
    level_offset: u64,
    level_len: u64,
    flip_y: bool,
}

fn le64(d: &[u8], at: usize) -> Option<u64> {
    let b = d.get(at..at + 8)?;
    Some(u64::from_le_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
}

fn vk_format(v: u32) -> Result<Tex> {
    let rgba8 = Tex::Masked {
        bytes: 4,
        masks: [0xFF, 0xFF00, 0xFF_0000, 0xFF00_0000],
        luminance: false,
    };
    let bgra8 = Tex::Masked {
        bytes: 4,
        masks: [0xFF_0000, 0xFF00, 0xFF, 0xFF00_0000],
        luminance: false,
    };
    Ok(match v {
        9 | 15 => Tex::Masked {
            bytes: 1,
            masks: [0xFF, 0, 0, 0],
            luminance: true,
        },
        16 | 22 => Tex::Masked {
            bytes: 2,
            masks: [0xFF, 0xFF00, 0, 0],
            luminance: false,
        },
        23 | 29 => Tex::Masked {
            bytes: 3,
            masks: [0xFF, 0xFF00, 0xFF_0000, 0],
            luminance: false,
        },
        30 | 36 => Tex::Masked {
            bytes: 3,
            masks: [0xFF_0000, 0xFF00, 0xFF, 0],
            luminance: false,
        },
        37 | 43 => rgba8,
        44 | 50 => bgra8,
        64 => Tex::Masked {
            bytes: 4,
            masks: [0x3FF, 0xFFC00, 0x3FF0_0000, 0xC000_0000],
            luminance: false,
        },
        76 => Tex::R16Float,
        83 => Tex::Rg16Float,
        91 => Tex::Rgba16Unorm,
        97 => Tex::Rgba16Float,
        100 => Tex::R32Float,
        103 => Tex::Rg32Float,
        106 => Tex::Rgb32Float,
        109 => Tex::Rgba32Float,
        122 => Tex::R11G11B10Float,
        123 => Tex::Rgb9E5,
        131..=134 => Tex::Bc1,
        135 | 136 => Tex::Bc2,
        137 | 138 => Tex::Bc3,
        139 => Tex::Bc4 { signed: false },
        140 => Tex::Bc4 { signed: true },
        141 => Tex::Bc5 { signed: false },
        142 => Tex::Bc5 { signed: true },
        143 => Tex::Bc6h { signed: false },
        144 => Tex::Bc6h { signed: true },
        145 | 146 => Tex::Bc7,
        147 | 148 => Tex::Etc2Rgb,
        149 | 150 => Tex::Etc2Rgba1,
        151 | 152 => Tex::Etc2Rgba8,
        153 => Tex::EacR11 { signed: false },
        154 => Tex::EacR11 { signed: true },
        155 => Tex::EacRg11 { signed: false },
        156 => Tex::EacRg11 { signed: true },
        157..=184 => {
            const SIZES: [(u8, u8); 14] = [
                (4, 4),
                (5, 4),
                (5, 5),
                (6, 5),
                (6, 6),
                (8, 5),
                (8, 6),
                (8, 8),
                (10, 5),
                (10, 6),
                (10, 8),
                (10, 10),
                (12, 10),
                (12, 12),
            ];
            let (bw, bh) = SIZES[((v - 157) / 2) as usize];
            Tex::Astc { bw, bh }
        }
        0 => return Err(unsupported(FORMAT, "Basis Universal payload")),
        other => return Err(unsupported(FORMAT, format!("Vulkan format {other}"))),
    })
}

/// The `KTXorientation` value, when the key/value data holds one.
fn orientation_key(kvd: &[u8]) -> Option<&[u8]> {
    let mut pos = 0;
    while pos + 4 <= kvd.len() {
        let len = le32(kvd, pos)? as usize;
        let kv = kvd.get(pos + 4..pos + 4 + len)?;
        if let Some(rest) = kv.strip_prefix(b"KTXorientation\0") {
            return Some(rest.split(|&b| b == 0).next().unwrap_or(rest));
        }
        pos += 4 + ((len + 3) & !3);
    }
    None
}

fn header(d: &[u8]) -> Result<Header> {
    if d.len() < 80 + 24 || d[..12] != MAGIC {
        return Err(corrupt(FORMAT, "missing KTX2 identifier"));
    }
    let vk = le32(d, 12).unwrap_or(0);
    let width = le32(d, 20).unwrap_or(0);
    let height = le32(d, 24).unwrap_or(0).max(1);
    let scheme = le32(d, 44).unwrap_or(0);
    if width == 0 {
        return Err(corrupt(FORMAT, "zero image size"));
    }
    let format = vk_format(vk)?;
    let kvd_off = le32(d, 56).unwrap_or(0) as usize;
    let kvd_len = le32(d, 60).unwrap_or(0) as usize;
    let flip_y = d
        .get(kvd_off..kvd_off.saturating_add(kvd_len))
        .and_then(orientation_key)
        .is_some_and(|o| o.get(1) == Some(&b'u'));
    // Level 0 is the first entry of the level index.
    let level_offset = le64(d, 80).unwrap_or(0);
    let level_len = le64(d, 88).unwrap_or(0);
    Ok(Header {
        width,
        height,
        format,
        scheme,
        level_offset,
        level_len,
        flip_y,
    })
}

impl DecoderProvider for Ktx2Decoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "KTX2 texture"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["ktx2"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(&MAGIC)
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
        let need = h.format.size(h.width, h.height).ok_or(Error::TooLarge {
            width: h.width,
            height: h.height,
        })?;
        let start = usize::try_from(h.level_offset).map_err(|_| corrupt(FORMAT, "bad offset"))?;
        let len = usize::try_from(h.level_len).map_err(|_| corrupt(FORMAT, "bad length"))?;
        let stored = d
            .get(start..start.saturating_add(len))
            .ok_or_else(|| corrupt(FORMAT, "level data outside the file"))?;
        crate::util::guard(FORMAT, || {
            let data: std::borrow::Cow<'_, [u8]> = match h.scheme {
                0 => stored.into(),
                2 => {
                    let mut out = Vec::new();
                    let mut dec = ruzstd::decoding::StreamingDecoder::new(stored)
                        .map_err(|e| corrupt(FORMAT, e))?;
                    (&mut dec)
                        .take(need as u64)
                        .read_to_end(&mut out)
                        .map_err(|e| corrupt(FORMAT, e))?;
                    out.into()
                }
                3 => {
                    let mut out = Vec::new();
                    flate2::read::ZlibDecoder::new(stored)
                        .take(need as u64)
                        .read_to_end(&mut out)
                        .map_err(|e| corrupt(FORMAT, e))?;
                    out.into()
                }
                1 => return Err(unsupported(FORMAT, "BasisLZ supercompression")),
                s => return Err(unsupported(FORMAT, format!("supercompression {s}"))),
            };
            // Level 0 holds every layer and face; the first image comes first.
            let image = texture::decode(h.format, &data, h.width, h.height, FORMAT)?;
            let image = if h.flip_y {
                crate::orient::apply(image, Orientation::FlipV)?
            } else {
                image
            };
            Ok(Content::Still(image))
        })
    }
}

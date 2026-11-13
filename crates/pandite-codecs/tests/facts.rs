//! Header facts: what each provider reports about a file without decoding
//! its pixels (container, codec, channels, color, camera settings).

mod common;

use common::*;
use pandite_core::{EncodeOptions, ImageFacts};

/// The facts the registry reports for `bytes` named `name`.
fn facts(bytes: &[u8], name: &str) -> ImageFacts {
    match reg().facts(&source(bytes, name)) {
        Ok(f) => f,
        Err(e) => panic!("facts of {name}: {e}"),
    }
}

/// A minimal ICC profile whose description (version 2 text) is `name`.
fn icc_profile(name: &str) -> Vec<u8> {
    let mut desc = b"desc\0\0\0\0".to_vec();
    desc.extend_from_slice(&(name.len() as u32 + 1).to_be_bytes());
    desc.extend_from_slice(name.as_bytes());
    desc.push(0);
    let mut p = vec![0u8; 128];
    p[16..20].copy_from_slice(b"RGB ");
    p[36..40].copy_from_slice(b"acsp");
    p.extend_from_slice(&1u32.to_be_bytes());
    p.extend_from_slice(b"desc");
    p.extend_from_slice(&144u32.to_be_bytes());
    p.extend_from_slice(&(desc.len() as u32).to_be_bytes());
    p.extend_from_slice(&desc);
    let len = p.len() as u32;
    p[..4].copy_from_slice(&len.to_be_bytes());
    p
}

/// One little-endian TIFF directory at `at`; values longer than four
/// bytes follow it.
fn ifd(at: usize, entries: &[(u16, u16, u32, Vec<u8>)]) -> Vec<u8> {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    let values_at = at + 2 + 12 * entries.len() + 4;
    head.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, typ, count, value) in entries {
        head.extend_from_slice(&tag.to_le_bytes());
        head.extend_from_slice(&typ.to_le_bytes());
        head.extend_from_slice(&count.to_le_bytes());
        if value.len() <= 4 {
            let mut v = value.clone();
            v.resize(4, 0);
            head.extend_from_slice(&v);
        } else {
            head.extend_from_slice(&((values_at + tail.len()) as u32).to_le_bytes());
            tail.extend_from_slice(value);
            if tail.len() % 2 == 1 {
                tail.push(0);
            }
        }
    }
    head.extend_from_slice(&0u32.to_le_bytes());
    head.extend(tail);
    head
}

fn ascii(tag: u16, s: &str) -> (u16, u16, u32, Vec<u8>) {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    (tag, 2, b.len() as u32, b)
}

fn rational(tag: u16, n: u32, d: u32) -> (u16, u16, u32, Vec<u8>) {
    let mut b = n.to_le_bytes().to_vec();
    b.extend_from_slice(&d.to_le_bytes());
    (tag, 5, 1, b)
}

/// An EXIF (TIFF) block: Make and Model in IFD0, and an EXIF directory
/// with 1/250 s, f/2.8, ISO 400, 35 mm, a capture date and a lens.
fn exif_block() -> Vec<u8> {
    let ifd0 = |exif_at: u32| {
        vec![
            ascii(0x010F, "TestCam"),
            ascii(0x0110, "TestCam X100"),
            (0x8769, 4, 1, exif_at.to_le_bytes().to_vec()),
        ]
    };
    let exif_at = 8 + ifd(8, &ifd0(0)).len();
    let exif = vec![
        rational(0x829A, 1, 250),
        rational(0x829D, 28, 10),
        (0x8827, 3, 1, 400u16.to_le_bytes().to_vec()),
        ascii(0x9003, "2024:05:01 12:34:56"),
        rational(0x920A, 35, 1),
        ascii(0xA434, "TestLens 35mm"),
    ];
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend(ifd(8, &ifd0(exif_at as u32)));
    out.extend(ifd(exif_at, &exif));
    out
}

fn assert_camera(f: &ImageFacts, what: &str) {
    let c = &f.camera;
    assert_eq!(c.make.as_deref(), Some("TestCam"), "{what}");
    assert_eq!(c.model.as_deref(), Some("TestCam X100"), "{what}");
    assert_eq!(c.exposure, Some((1, 250)), "{what}");
    assert_eq!(c.f_number, Some(2.8), "{what}");
    assert_eq!(c.iso, Some(400), "{what}");
    assert_eq!(c.focal_length, Some(35.0), "{what}");
    assert_eq!(c.captured.as_deref(), Some("2024-05-01 12:34:56"), "{what}");
    assert_eq!(c.lens.as_deref(), Some("TestLens 35mm"), "{what}");
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut v = (data.len() as u32).to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(data);
    let mut crc_input = kind.to_vec();
    crc_input.extend_from_slice(data);
    v.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    v
}

/// `data` as a zlib stream of stored (uncompressed) deflate blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut v = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(65_535).collect();
    for (i, b) in blocks.iter().enumerate() {
        v.push(u8::from(i + 1 == blocks.len()));
        let n = b.len() as u16;
        v.extend_from_slice(&n.to_le_bytes());
        v.extend_from_slice(&(!n).to_le_bytes());
        v.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    v.extend_from_slice(&((b << 16) | a).to_be_bytes());
    v
}

/// A PNG written by the png crate.
fn png_with(
    w: u32,
    h: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    setup: impl FnOnce(&mut png::Encoder<&mut Vec<u8>>),
) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(color);
        enc.set_depth(depth);
        setup(&mut enc);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(data).unwrap();
        writer.finish().unwrap();
    }
    out
}

#[test]
fn facts_png() {
    // RGBA from the crate's own encoder.
    let rgba = rgba8(6, 4, pattern(6, 4, true));
    let file = encode("png", &rgba, &EncodeOptions::default());
    let f = facts(&file, "a.png");
    assert_eq!(f.container.as_deref(), Some("PNG"));
    assert_eq!(f.compression.as_deref(), Some("Deflate"));
    assert_eq!(f.channels.as_deref(), Some("RGBA"));
    assert_eq!(f.channel_count, Some(4));
    assert_eq!(f.bits_per_channel, Some(8));
    assert_eq!(f.stored_bits_per_pixel(), Some(32));
    assert_eq!(f.fourcc, None);
    assert_eq!(f.subsampling, None);
    assert_eq!(f.icc_profile, None);
    assert!(f.camera.is_empty());

    // An embedded profile (iCCP) and EXIF (eXIf) before the image data.
    let mut iccp = b"ICC profile\0\0".to_vec();
    iccp.extend(zlib_stored(&icc_profile("Test Profile")));
    let at = 8 + 25;
    let mut tagged = file[..at].to_vec();
    tagged.extend(png_chunk(b"iCCP", &iccp));
    tagged.extend(png_chunk(b"eXIf", &exif_block()));
    tagged.extend_from_slice(&file[at..]);
    let f = facts(&tagged, "tagged.png");
    assert_eq!(f.icc_profile.as_deref(), Some("Test Profile"));
    assert_camera(&f, "png eXIf");

    // A 2-bit palette with an sRGB chunk.
    let palette = [255, 0, 0, 0, 255, 0, 0, 0, 255, 9, 9, 9];
    let indexed = png_with(
        4,
        2,
        png::ColorType::Indexed,
        png::BitDepth::Two,
        &[0b0001_1011, 0b1110_0100],
        |e| {
            e.set_palette(palette.to_vec());
            e.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        },
    );
    let f = facts(&indexed, "p.png");
    assert_eq!(f.channels.as_deref(), Some("Indexed, 4 colors"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(2)));
    assert_eq!(f.stored_bits_per_pixel(), Some(2));
    assert_eq!(f.color_space.as_deref(), Some("sRGB"));

    // 16-bit gray.
    let gray = png_with(
        2,
        2,
        png::ColorType::Grayscale,
        png::BitDepth::Sixteen,
        &[0; 8],
        |_| {},
    );
    let f = facts(&gray, "g.png");
    assert_eq!(f.channels.as_deref(), Some("Gray"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(16)));

    // APNG names itself.
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, 2, 2);
        enc.set_color(png::ColorType::Rgb);
        enc.set_animated(2, 0).unwrap();
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0; 12]).unwrap();
        w.write_image_data(&[255; 12]).unwrap();
        w.finish().unwrap();
    }
    assert_eq!(facts(&out, "a.apng").container.as_deref(), Some("APNG"));

    // Only the head of the file: the chunks before the image data answer.
    let head = &tagged[..tagged.len() - 12];
    assert_eq!(
        facts(head, "head.png").icc_profile.as_deref(),
        Some("Test Profile")
    );
}

/// A JPEG of a smooth pattern from the `jpeg-encoder` crate.
fn jpeg_with(
    color: jpeg_encoder::ColorType,
    channels: usize,
    setup: impl FnOnce(&mut jpeg_encoder::Encoder<&mut Vec<u8>>),
) -> Vec<u8> {
    let (w, h) = (32u16, 16u16);
    let data: Vec<u8> = (0..usize::from(w) * usize::from(h) * channels)
        .map(|i| (i * 7 % 256) as u8)
        .collect();
    let mut out = Vec::new();
    let mut enc = jpeg_encoder::Encoder::new(&mut out, 90);
    setup(&mut enc);
    enc.encode(&data, w, h, color).unwrap();
    out
}

#[test]
fn facts_jpeg() {
    use jpeg_encoder::{ColorType, SamplingFactor};
    // Baseline, 4:2:0, with an ICC profile and EXIF.
    let file = jpeg_with(ColorType::Rgb, 3, |e| {
        e.set_sampling_factor(SamplingFactor::F_2_2);
        e.add_icc_profile(&icc_profile("Test Profile")).unwrap();
        e.add_exif_metadata(&exif_block()).unwrap();
    });
    let f = facts(&file, "a.jpg");
    assert_eq!(f.container.as_deref(), Some("JPEG"));
    assert_eq!(f.compression.as_deref(), Some("Baseline DCT, Huffman"));
    assert_eq!(f.channels.as_deref(), Some("YCbCr"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(3), Some(8)));
    assert_eq!(f.stored_bits_per_pixel(), Some(24));
    assert_eq!(f.subsampling.as_deref(), Some("4:2:0"));
    assert_eq!(f.icc_profile.as_deref(), Some("Test Profile"));
    assert_camera(&f, "jpeg APP1");

    // Progressive 4:2:2 and 4:4:4.
    let prog = jpeg_with(ColorType::Rgb, 3, |e| {
        e.set_progressive(true);
        e.set_sampling_factor(SamplingFactor::F_2_1);
    });
    let f = facts(&prog, "p.jpg");
    assert_eq!(f.compression.as_deref(), Some("Progressive DCT, Huffman"));
    assert_eq!(f.subsampling.as_deref(), Some("4:2:2"));
    assert!(f.camera.is_empty() && f.icc_profile.is_none());
    let full = jpeg_with(ColorType::Rgb, 3, |e| {
        e.set_sampling_factor(SamplingFactor::F_1_1)
    });
    assert_eq!(facts(&full, "f.jpg").subsampling.as_deref(), Some("4:4:4"));

    // Gray has no chroma; CMYK carries an Adobe segment.
    let gray = facts(&jpeg_with(ColorType::Luma, 1, |_| {}), "g.jpg");
    assert_eq!(gray.channels.as_deref(), Some("Gray"));
    assert_eq!(gray.channel_count, Some(1));
    assert_eq!(gray.subsampling, None);
    let cmyk = facts(&jpeg_with(ColorType::Cmyk, 4, |_| {}), "c.jpg");
    assert_eq!(cmyk.channels.as_deref(), Some("CMYK"));
    assert_eq!(cmyk.channel_count, Some(4));

    // A stream cut after its frame header still reports it; garbage and
    // every cut of a real stream never panic.
    let sos = file.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let cut = facts(&file[..sos + 4], "cut.jpg");
    assert_eq!(cut.compression.as_deref(), Some("Baseline DCT, Huffman"));
    for n in (0..file.len()).step_by(3) {
        let _ = reg().facts(&source(&file[..n], "x.jpg"));
    }
}

/// A GIF of `frames` frames, each with the given palette (global when
/// `global`, else local) and interlace flag.
fn gif_with(palette: &[u8], global: bool, interlaced: bool, frames: usize) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let table: &[u8] = if global { palette } else { &[] };
        let mut enc = gif::Encoder::new(&mut out, 4, 2, table).unwrap();
        for i in 0..frames {
            let frame = gif::Frame {
                width: 4,
                height: 2,
                interlaced,
                delay: 10,
                palette: (!global).then(|| palette.to_vec()),
                buffer: std::borrow::Cow::Owned(vec![(i % 2) as u8; 8]),
                ..gif::Frame::default()
            };
            enc.write_frame(&frame).unwrap();
        }
    }
    out
}

#[test]
fn facts_gif() {
    let four = [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255];
    let f = facts(&gif_with(&four, true, false, 3), "a.gif");
    assert_eq!(f.container.as_deref(), Some("GIF"));
    assert_eq!(f.compression.as_deref(), Some("LZW"));
    assert_eq!(f.channels.as_deref(), Some("Indexed, 4 colors"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(2)));
    assert_eq!(f.stored_bits_per_pixel(), Some(2));
    assert!(f.camera.is_empty() && f.fourcc.is_none());

    // A local color table of 8 entries on an interlaced first frame.
    let eight: Vec<u8> = (0..24).map(|i| i * 10).collect();
    let f = facts(&gif_with(&eight, false, true, 1), "b.gif");
    assert_eq!(f.compression.as_deref(), Some("LZW, interlaced"));
    assert_eq!(f.channels.as_deref(), Some("Indexed, 8 colors"));
    assert_eq!(f.bits_per_channel, Some(3));

    // Not a GIF: an error, not a guess.
    assert!(reg().facts(&source(b"GIF00a....", "x.gif")).is_err());
}

#[test]
fn facts_webp() {
    // Simple lossy: the first chunk is "VP8 ".
    let f = facts(&fixture("lossy.webp"), "lossy.webp");
    assert_eq!(f.container.as_deref(), Some("WebP"));
    assert_eq!(f.fourcc.as_deref(), Some("VP8"));
    assert_eq!(f.compression.as_deref(), Some("VP8 lossy"));
    assert_eq!(f.channels.as_deref(), Some("YCbCr"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(3), Some(8)));
    assert_eq!(f.subsampling.as_deref(), Some("4:2:0"));

    // Extended with an alpha chunk.
    let f = facts(&fixture("lossy_alpha.webp"), "alpha.webp");
    assert_eq!(f.fourcc.as_deref(), Some("VP8X"));
    assert!(
        f.compression
            .as_deref()
            .is_some_and(|c| c.starts_with("VP8 lossy, ") && c.ends_with(" alpha")),
        "{:?}",
        f.compression
    );
    assert_eq!(f.channels.as_deref(), Some("YCbCr + alpha"));
    assert_eq!(f.channel_count, Some(4));

    // Animated: the coding of the first frame.
    let f = facts(&fixture("lossy_anim.webp"), "anim.webp");
    assert_eq!(f.fourcc.as_deref(), Some("VP8X"));
    assert!(f
        .compression
        .as_deref()
        .is_some_and(|c| c.starts_with("VP8 lossy")));

    // Lossless with transparency.
    let lossless = webp_lossless(4, 4, pattern(4, 4, true));
    let f = facts(&lossless, "l.webp");
    assert_eq!(f.fourcc.as_deref(), Some("VP8L"));
    assert_eq!(f.compression.as_deref(), Some("VP8L lossless"));
    assert_eq!(f.channels.as_deref(), Some("RGBA"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(4), Some(8)));
    assert_eq!(f.subsampling, None);

    // Extended lossless with an ICC profile and EXIF.
    let mut vp8x = vec![0x20 | 0x08 | 0x10, 0, 0, 0];
    vp8x.extend_from_slice(&3u32.to_le_bytes()[..3]);
    vp8x.extend_from_slice(&3u32.to_le_bytes()[..3]);
    let mut body = riff_chunk(b"VP8X", &vp8x);
    body.extend(riff_chunk(b"ICCP", &icc_profile("Test Profile")));
    body.extend(vp8l_chunk(&lossless));
    body.extend(riff_chunk(b"EXIF", &exif_block()));
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend(body);
    let f = facts(&file, "x.webp");
    assert_eq!(f.fourcc.as_deref(), Some("VP8X"));
    assert_eq!(f.compression.as_deref(), Some("VP8L lossless"));
    assert_eq!(f.icc_profile.as_deref(), Some("Test Profile"));
    assert_camera(&f, "webp EXIF");
    for n in 0..file.len() {
        let _ = reg().facts(&source(&file[..n], "x.webp"));
    }
}

#[cfg(all(feature = "dds", feature = "tga", feature = "hdr"))]
#[test]
fn facts_simple_headers() {
    // BMP from the crate's encoder: 24-bit opaque, 32-bit with alpha.
    let opaque = encode(
        "bmp",
        &rgba8(3, 2, solid(3, 2, [9, 8, 7, 255])),
        &EncodeOptions::default(),
    );
    let f = facts(&opaque, "a.bmp");
    assert_eq!(f.container.as_deref(), Some("BMP"));
    assert_eq!(f.compression.as_deref(), Some("Uncompressed"));
    assert_eq!(f.channels.as_deref(), Some("RGB"));
    assert_eq!(f.stored_bits_per_pixel(), Some(24));
    let clear = encode(
        "bmp",
        &rgba8(3, 2, pattern(3, 2, true)),
        &EncodeOptions::default(),
    );
    assert_eq!(facts(&clear, "b.bmp").channels.as_deref(), Some("RGBA"));

    // DDS with a DXT1 FourCC: one 4 x 4 block.
    let mut dds = vec![0u8; 128];
    dds[..4].copy_from_slice(b"DDS ");
    dds[4..8].copy_from_slice(&124u32.to_le_bytes());
    dds[12..16].copy_from_slice(&4u32.to_le_bytes());
    dds[16..20].copy_from_slice(&4u32.to_le_bytes());
    dds[76..80].copy_from_slice(&32u32.to_le_bytes());
    dds[80..84].copy_from_slice(&4u32.to_le_bytes());
    dds[84..88].copy_from_slice(b"DXT1");
    dds.extend_from_slice(&[0; 8]);
    let f = facts(&dds, "t.dds");
    assert_eq!(f.fourcc.as_deref(), Some("DXT1"));
    assert_eq!(f.compression.as_deref(), Some("BC1 (DXT1)"));
    assert_eq!(f.stored_bits_per_pixel(), Some(4));

    // TGA: no signature, matched by extension.
    let mut tga = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 1, 0, 24, 0];
    tga.extend_from_slice(&[1, 2, 3, 4, 5, 6]);
    let f = facts(&tga, "t.tga");
    assert_eq!(f.compression.as_deref(), Some("Uncompressed"));
    assert_eq!(f.channels.as_deref(), Some("RGB"));
    assert_eq!(f.stored_bits_per_pixel(), Some(24));

    // Radiance HDR, flat scanlines.
    let mut hdr = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 1 +X 2\n".to_vec();
    hdr.extend_from_slice(&[128, 64, 32, 129, 128, 64, 32, 129]);
    let f = facts(&hdr, "h.hdr");
    assert_eq!(f.channels.as_deref(), Some("RGBE"));
    assert_eq!(f.stored_bits_per_pixel(), Some(32));
}

#[cfg(all(feature = "avif", feature = "jxl", feature = "tiff"))]
#[test]
fn facts_extended_containers() {
    let f = facts(&fixture("blocks.avif"), "blocks.avif");
    assert_eq!(f.container.as_deref(), Some("AVIF"));
    assert_eq!(f.fourcc.as_deref(), Some("av01"));
    assert_eq!(f.compression.as_deref(), Some("AV1"));
    assert_eq!(f.subsampling.as_deref(), Some("4:4:4"));
    assert_eq!(f.bits_per_channel, Some(8));
    let deep = facts(&fixture("deep.avif"), "deep.avif");
    assert_eq!(deep.bits_per_channel, Some(10));
    assert_eq!(deep.subsampling.as_deref(), Some("4:2:0"));
    let alpha = facts(&fixture("alpha.avif"), "alpha.avif");
    assert_eq!(alpha.channels.as_deref(), Some("YCbCr + alpha"));
    let grid = facts(&fixture("grid.avif"), "grid.avif");
    assert_eq!(grid.fourcc.as_deref(), Some("av01"), "the tile codec");

    let lossy = facts(&fixture("lossy.jxl"), "lossy.jxl");
    assert_eq!(lossy.container.as_deref(), Some("JPEG XL"));
    assert_eq!(lossy.compression.as_deref(), Some("Lossy (XYB)"));
    let lossless = facts(&fixture("blocks.jxl"), "blocks.jxl");
    assert_eq!(lossless.compression, None, "lossless is not stated");
    assert_eq!(lossless.channels.as_deref(), Some("RGBA"));

    let tif = facts(&fixture("jpeg.tif"), "jpeg.tif");
    assert_eq!(tif.container.as_deref(), Some("TIFF"));
    assert_eq!(tif.compression.as_deref(), Some("JPEG"));
    assert_eq!(tif.channel_count, Some(3));
}

#[cfg(all(feature = "heic", any(windows, target_os = "macos")))]
#[test]
fn facts_heic() {
    let f = facts(&fixture("blocks.heic"), "blocks.heic");
    assert_eq!(f.container.as_deref(), Some("HEIC"));
    assert_eq!(f.compression.as_deref(), Some("HEVC"));
    assert_eq!(f.fourcc.as_deref(), Some("hvc1"));
    assert!(f.bits_per_channel.is_some());
}

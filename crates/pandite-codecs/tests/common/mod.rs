//! Helpers shared by the integration tests: fixture builders and pixel
//! comparison. Each test binary uses a different subset.
#![allow(dead_code)]

use pandite_codecs::default_registry;
use pandite_core::{
    Content, DecodeOptions, EncodeOptions, ImageBuf, PixelFormat, Registry, Source,
};
use std::path::PathBuf;
use std::sync::OnceLock;

/// The registry under test, built once.
pub fn reg() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(default_registry)
}

/// Wrap bytes with a file name hint.
pub fn source(bytes: &[u8], name: &str) -> Source {
    Source::from_bytes(bytes.to_vec(), Some(PathBuf::from(name)))
}

/// Decode through the registry with default options.
pub fn decode(bytes: &[u8], name: &str) -> Content {
    decode_with(bytes, name, &DecodeOptions::default())
}

/// Decode through the registry with options, panicking on error.
pub fn decode_with(bytes: &[u8], name: &str, options: &DecodeOptions) -> Content {
    match reg().decode(&source(bytes, name), options) {
        Ok(c) => c,
        Err(e) => panic!("decoding {name} failed: {e}"),
    }
}

/// Decode and require a still image.
pub fn still(bytes: &[u8], name: &str) -> ImageBuf {
    match decode(bytes, name) {
        Content::Still(img) => img,
        other => panic!("{name}: expected a still, got {:?}", other.kind()),
    }
}

/// The RGBA8 value of pixel (x, y). Panics for other formats.
pub fn px(img: &ImageBuf, x: u32, y: u32) -> [u8; 4] {
    assert_eq!(img.format(), PixelFormat::Rgba8, "expected Rgba8");
    let i = ((y * img.width() + x) * 4) as usize;
    let d = img.data();
    [d[i], d[i + 1], d[i + 2], d[i + 3]]
}

/// The RGBA16 value of pixel (x, y).
pub fn px16(img: &ImageBuf, x: u32, y: u32) -> [u16; 4] {
    assert_eq!(img.format(), PixelFormat::Rgba16, "expected Rgba16");
    let i = ((y * img.width() + x) * 8) as usize;
    let d = img.data();
    let s = |k: usize| u16::from_ne_bytes([d[i + 2 * k], d[i + 2 * k + 1]]);
    [s(0), s(1), s(2), s(3)]
}

/// The RGBA f32 value of pixel (x, y).
pub fn pxf(img: &ImageBuf, x: u32, y: u32) -> [f32; 4] {
    assert_eq!(img.format(), PixelFormat::RgbaF32, "expected RgbaF32");
    let i = ((y * img.width() + x) * 16) as usize;
    let d = img.data();
    let s = |k: usize| {
        let o = i + 4 * k;
        f32::from_ne_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
    };
    [s(0), s(1), s(2), s(3)]
}

/// Assert two RGBA8 values differ by at most `tol` per channel.
pub fn assert_near(actual: [u8; 4], expected: [u8; 4], tol: u8, what: &str) {
    for c in 0..4 {
        assert!(
            actual[c].abs_diff(expected[c]) <= tol,
            "{what}: got {actual:?}, expected {expected:?} within {tol}"
        );
    }
}

/// A deterministic RGBA8 test pattern with varied colors and alpha.
pub fn pattern(w: u32, h: u32, with_alpha: bool) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w.max(2).saturating_sub(1).max(1)) as u8;
            let g = (y * 255 / h.max(2).saturating_sub(1).max(1)) as u8;
            let b = ((x * 7 + y * 13) % 256) as u8;
            let a = if with_alpha {
                ((x * 31 + y * 17) % 256) as u8
            } else {
                255
            };
            v.extend_from_slice(&[r, g, b, a]);
        }
    }
    v
}

/// An RGBA8 image from raw bytes.
pub fn rgba8(w: u32, h: u32, data: Vec<u8>) -> ImageBuf {
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

/// Encode with a registry encoder.
pub fn encode(id: &str, img: &ImageBuf, options: &EncodeOptions) -> Vec<u8> {
    let enc = reg().encoder_by_id(id).expect("encoder registered");
    let mut out = Vec::new();
    enc.encode(img, options, &mut out).expect("encode");
    out
}

/// A minimal little-endian EXIF (TIFF) block holding one orientation tag.
pub fn exif_orientation(value: u16) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(b"II*\0");
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&274u16.to_le_bytes());
    v.extend_from_slice(&3u16.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&value.to_le_bytes());
    v.extend_from_slice(&[0, 0]);
    v.extend_from_slice(&0u32.to_le_bytes());
    v
}

/// An image of solid color blocks, `cell` pixels square, laid out as
/// `colors` (row-major, `cols` per row). JPEG-friendly when `cell` is a
/// multiple of 16.
pub fn blocks(colors: &[[u8; 3]], cols: u32, cell: u32) -> (u32, u32, Vec<u8>) {
    let rows = (colors.len() as u32).div_ceil(cols);
    let (w, h) = (cols * cell, rows * cell);
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let c = colors[((y / cell) * cols + x / cell) as usize];
            v.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    (w, h, v)
}

/// Build a RIFF WebP chunk (with padding).
pub fn riff_chunk(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(fourcc);
    v.extend_from_slice(&(body.len() as u32).to_le_bytes());
    v.extend_from_slice(body);
    if body.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// Extract the VP8L chunk (header included) from a simple lossless WebP.
pub fn vp8l_chunk(webp: &[u8]) -> Vec<u8> {
    assert_eq!(&webp[12..16], b"VP8L");
    let len = u32::from_le_bytes([webp[16], webp[17], webp[18], webp[19]]) as usize;
    let mut v = webp[12..20 + len].to_vec();
    if len % 2 == 1 {
        v.push(0);
    }
    v
}

/// A lossless WebP of an RGBA8 image, via the crate's encoder.
pub fn webp_lossless(w: u32, h: u32, data: Vec<u8>) -> Vec<u8> {
    encode("webp", &rgba8(w, h, data), &EncodeOptions::default())
}

/// One frame of a hand-built animated WebP.
pub struct WebpFrame {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub ms: u32,
    pub blend: bool,
    pub dispose: bool,
    pub rgba: Vec<u8>,
}

/// Build an animated WebP from frames (VP8L payloads).
pub fn webp_animated(cw: u32, ch: u32, loops: u16, frames: &[WebpFrame]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut vp8x = vec![0x02 | 0x10, 0, 0, 0];
    vp8x.extend_from_slice(&(cw - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(ch - 1).to_le_bytes()[..3]);
    body.extend(riff_chunk(b"VP8X", &vp8x));
    let mut anim = vec![0, 0, 0, 0];
    anim.extend_from_slice(&loops.to_le_bytes());
    body.extend(riff_chunk(b"ANIM", &anim));
    for f in frames {
        let mut anmf = Vec::new();
        anmf.extend_from_slice(&(f.x / 2).to_le_bytes()[..3]);
        anmf.extend_from_slice(&(f.y / 2).to_le_bytes()[..3]);
        anmf.extend_from_slice(&(f.w - 1).to_le_bytes()[..3]);
        anmf.extend_from_slice(&(f.h - 1).to_le_bytes()[..3]);
        anmf.extend_from_slice(&f.ms.to_le_bytes()[..3]);
        let flags = (u8::from(!f.blend) << 1) | u8::from(f.dispose);
        anmf.push(flags);
        let still = webp_lossless(f.w, f.h, f.rgba.clone());
        anmf.extend(vp8l_chunk(&still));
        body.extend(riff_chunk(b"ANMF", &anmf));
    }
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend(body);
    file
}

/// Solid RGBA8 pixels.
pub fn solid(w: u32, h: u32, c: [u8; 4]) -> Vec<u8> {
    c.iter()
        .copied()
        .cycle()
        .take((w * h * 4) as usize)
        .collect()
}

/// Path of a committed fixture file.
pub fn fixture(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

pub mod dng;
pub mod hand;

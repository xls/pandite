//! Gate G3: the encoders write files that decode back to the same image.
//! PNG, BMP and lossless WebP are exact, JPEG stays within a tolerance and
//! flattens alpha onto white.

mod common;

use common::*;
use pandite_core::{EncodeOptions, ImageBuf, PixelFormat};

fn roundtrip(id: &str, ext: &str, img: &ImageBuf, options: &EncodeOptions) -> ImageBuf {
    let file = encode(id, img, options);
    still(&file, &format!("out.{ext}"))
}

#[test]
fn encode_png_roundtrip() {
    for alpha in [true, false] {
        let data = pattern(31, 17, alpha);
        let out = roundtrip(
            "png",
            "png",
            &rgba8(31, 17, data.clone()),
            &EncodeOptions::default(),
        );
        assert_eq!(out.format(), PixelFormat::Rgba8);
        assert_eq!(out.data(), &data[..], "alpha {alpha}");
    }
    // 16-bit sources stay 16-bit.
    let mut d = Vec::new();
    for v in [1u16, 2, 3, 65535, 40000, 1234, 65535, 7] {
        d.extend_from_slice(&v.to_ne_bytes());
    }
    let img = ImageBuf::new(2, 1, PixelFormat::Rgba16, d.clone()).unwrap();
    let out = roundtrip("png", "png", &img, &EncodeOptions::default());
    assert_eq!(out.format(), PixelFormat::Rgba16);
    assert_eq!(out.data(), &d[..]);
    assert!(reg().encoder_for_extension("PNG").is_some());
}

#[test]
fn encode_bmp_roundtrip() {
    for alpha in [true, false] {
        let data = pattern(9, 11, alpha);
        let out = roundtrip(
            "bmp",
            "bmp",
            &rgba8(9, 11, data.clone()),
            &EncodeOptions::default(),
        );
        assert_eq!(out.data(), &data[..], "alpha {alpha}");
    }
    assert_eq!(reg().encoder_for_extension("bmp").unwrap().id(), "bmp");
}

#[test]
fn encode_webp_lossless_roundtrip() {
    for alpha in [true, false] {
        let data = pattern(23, 14, alpha);
        let img = rgba8(23, 14, data.clone());
        let file = encode(
            "webp",
            &img,
            &EncodeOptions {
                quality: 10,
                lossless: true,
            },
        );
        assert_eq!(&file[12..16], b"VP8L", "lossless bitstream");
        let out = still(&file, "out.webp");
        assert_eq!(out.data(), &data[..], "alpha {alpha}");
    }
    let enc = reg().encoder_by_id("webp").unwrap();
    assert!(enc.supports_alpha());
    assert!(!enc.has_quality());
}

#[test]
fn encode_jpeg_roundtrip_within_tolerance() {
    let (w, h) = (48u32, 32u32);
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&[(x * 5) as u8, (y * 7) as u8, 128 + (x + y) as u8, 255]);
        }
    }
    let img = rgba8(w, h, data.clone());
    let file = encode(
        "jpeg",
        &img,
        &EncodeOptions {
            quality: 95,
            lossless: false,
        },
    );
    let out = still(&file, "out.jpg");
    assert_eq!((out.width(), out.height()), (w, h));
    let mut total = 0u64;
    let mut worst = 0u8;
    for (a, b) in out.data().chunks_exact(4).zip(data.chunks_exact(4)) {
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            total += u64::from(d);
            worst = worst.max(d);
        }
        assert_eq!(a[3], 255);
    }
    let mean = total as f64 / f64::from(w * h * 3);
    assert!(mean < 1.5, "mean error {mean}");
    assert!(worst <= 12, "worst error {worst}");

    // Quality changes the output.
    let small = encode(
        "jpeg",
        &img,
        &EncodeOptions {
            quality: 20,
            lossless: false,
        },
    );
    assert!(small.len() < file.len());
    assert!(reg().encoder_by_id("jpeg").unwrap().has_quality());
}

#[test]
fn encode_jpeg_flattens_alpha() {
    let mut data = Vec::new();
    for x in 0..48u32 {
        let px = match x / 16 {
            0 => [255, 0, 0, 0],
            1 => [255, 0, 0, 128],
            _ => [0, 0, 255, 255],
        };
        for _ in 0..16 {
            data.extend_from_slice(&px);
        }
    }
    // Columns: build row-major from the column pattern above.
    let mut rows = vec![0u8; 48 * 16 * 4];
    for y in 0..16usize {
        for x in 0..48usize {
            let src = (x * 16 + y) * 4;
            rows[(y * 48 + x) * 4..(y * 48 + x) * 4 + 4].copy_from_slice(&data[src..src + 4]);
        }
    }
    let img = rgba8(48, 16, rows);
    let file = encode(
        "jpeg",
        &img,
        &EncodeOptions {
            quality: 100,
            lossless: false,
        },
    );
    let out = still(&file, "flat.jpg");
    assert!(!reg().encoder_by_id("jpeg").unwrap().supports_alpha());
    assert_near(
        px(&out, 8, 8),
        [255, 255, 255, 255],
        3,
        "transparent becomes white",
    );
    assert_near(
        px(&out, 24, 8),
        [255, 127, 127, 255],
        4,
        "half red over white",
    );
    assert_near(px(&out, 40, 8), [0, 0, 255, 255], 4, "opaque blue stays");
}

//! Mutation fuzzing over every compiled format. Damaged files must produce
//! an error or an image, never an escaping panic. Panics inside this
//! crate's own decoders are bugs even when the panic guard catches them, so
//! the test also fails when one is observed; panics inside third-party
//! decoders are caught by the guard and only reported.

mod common;

use common::hand::*;
use common::*;
use pandite_core::{DecodeOptions, EncodeOptions};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Mutex;

static PANICS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn noise(w: u32, h: u32, seed: u32, alpha: bool) -> Vec<u8> {
    let mut s = seed;
    (0..w * h * 4)
        .map(|i| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if i % 4 == 3 && !alpha {
                255
            } else {
                (s >> 16) as u8
            }
        })
        .collect()
}

/// One sample file per compiled format.
fn samples() -> Vec<(&'static str, Vec<u8>)> {
    let img = rgba8(24, 16, noise(24, 16, 3, true));
    let mut v = vec![
        ("png", encode("png", &img, &EncodeOptions::default())),
        ("jpg", encode("jpeg", &img, &EncodeOptions::default())),
        ("bmp", encode("bmp", &img, &EncodeOptions::default())),
        ("webp", encode("webp", &img, &EncodeOptions::default())),
        ("webp", fixture("lossy_alpha.webp")),
        ("webp", fixture("lossy_anim.webp")),
    ];
    let pal = bmp_palette(&[[0, 0, 0], [255, 0, 0], [0, 255, 0]]);
    v.push((
        "bmp",
        bmp_file(
            &bmp_info_header(4, 2, 8, 1, 3),
            &pal,
            &[3, 1, 0, 0, 2, 2, 0, 0, 0, 1],
        ),
    ));
    let mut gif = Vec::new();
    {
        let palette: Vec<u8> = (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect();
        let mut enc = gif::Encoder::new(&mut gif, 16, 16, &palette).unwrap();
        for f in 0..3u16 {
            let frame = gif::Frame {
                left: f,
                top: f,
                width: 12,
                height: 12,
                transparent: Some(7),
                dispose: gif::DisposalMethod::Previous,
                buffer: std::borrow::Cow::Owned(
                    noise(12, 12, u32::from(f), false)
                        .iter()
                        .step_by(4)
                        .copied()
                        .collect(),
                ),
                ..gif::Frame::default()
            };
            enc.write_frame(&frame).unwrap();
        }
    }
    v.push(("gif", gif));
    #[cfg(feature = "tiff")]
    {
        let mut buf = std::io::Cursor::new(Vec::new());
        tiff::encoder::TiffEncoder::new(&mut buf)
            .unwrap()
            .with_compression(tiff::encoder::Compression::Lzw)
            .write_image::<tiff::encoder::colortype::RGBA8>(24, 16, img.data())
            .unwrap();
        v.push(("tif", buf.into_inner()));
        v.push(("tif", fixture("jpeg.tif")));
    }
    #[cfg(feature = "tga")]
    {
        let mut t = vec![0u8, 0, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 4, 0, 2, 0, 32, 8];
        t.extend_from_slice(&[
            0x83, 1, 2, 3, 4, 0x03, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ]);
        v.push(("tga", t));
    }
    #[cfg(feature = "ico")]
    {
        let png = encode(
            "png",
            &rgba8(8, 8, noise(8, 8, 9, true)),
            &EncodeOptions::default(),
        );
        let mut ico = vec![0, 0, 1, 0, 1, 0, 8, 8, 0, 0, 1, 0, 32, 0];
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&22u32.to_le_bytes());
        ico.extend(png);
        v.push(("ico", ico));
    }
    #[cfg(feature = "qoi")]
    v.push(("qoi", qoi::encode_to_vec(img.data(), 24, 16).unwrap()));
    #[cfg(feature = "hdr")]
    {
        let mut f = b"#?RADIANCE\nFORMAT=32-bit_rle_rgbe\n\n-Y 2 +X 8\n".to_vec();
        for _ in 0..2 {
            f.extend_from_slice(&[
                2, 2, 0, 8, 136, 64, 8, 1, 2, 3, 4, 5, 6, 7, 8, 136, 9, 136, 129,
            ]);
        }
        v.push(("hdr", f));
    }
    #[cfg(feature = "psd")]
    {
        let mut p = b"8BPS\0\x01\0\0\0\0\0\0\0\x03\0\0\0\x02\0\0\0\x02\0\x08\0\x03".to_vec();
        p.extend_from_slice(&[0; 12]);
        p.extend_from_slice(&[0, 1]);
        for _ in 0..6 {
            p.extend_from_slice(&[0, 2]);
        }
        for c in 0..6u8 {
            p.extend_from_slice(&[0xFF, c * 40]);
        }
        v.push(("psd", p));
    }
    #[cfg(feature = "dds")]
    {
        let mut d = b"DDS ".to_vec();
        let mut hdr = [0u32; 31];
        hdr[0] = 124;
        hdr[2] = 8;
        hdr[3] = 8;
        hdr[18] = 32;
        hdr[19] = 4;
        hdr[20] = u32::from_le_bytes(*b"DXT5");
        for x in hdr {
            d.extend_from_slice(&x.to_le_bytes());
        }
        d.extend(noise(8, 2, 5, true));
        v.push(("dds", d));
    }
    #[cfg(feature = "ktx2")]
    {
        let mut k = vec![
            0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
        ];
        for x in [145u32, 1, 8, 8, 0, 0, 1, 1, 0, 0, 0, 104, 0] {
            k.extend_from_slice(&x.to_le_bytes());
        }
        for x in [0u64, 0, 112, 64, 64] {
            k.extend_from_slice(&x.to_le_bytes());
        }
        k.resize(112, 0);
        k.extend(noise(4, 4, 6, true));
        v.push(("ktx2", k));
    }
    #[cfg(feature = "exr")]
    {
        use exr::prelude::*;
        let image = Image::from_channels(
            (6, 4),
            SpecificChannels::rgba(|Vec2(x, y)| (x as f32, y as f32, 0.5f32, 1.0f32)),
        );
        let mut buf = std::io::Cursor::new(Vec::new());
        image.write().to_buffered(&mut buf).unwrap();
        v.push(("exr", buf.into_inner()));
    }
    #[cfg(feature = "svg")]
    v.push((
        "svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><circle cx="5" cy="5" r="4" fill="#f00"/><path d="M10 0L20 10" stroke="blue"/></svg>"##.to_vec(),
    ));
    #[cfg(feature = "jxl")]
    {
        v.push(("jxl", fixture("blocks.jxl")));
        v.push(("jxl", fixture("lossy.jxl")));
        v.push(("jxl", fixture("anim.jxl")));
    }
    #[cfg(feature = "avif")]
    {
        v.push(("avif", fixture("alpha.avif")));
        v.push(("avif", fixture("rotated.avif")));
    }
    v
}

#[test]
fn every_format_survives_mutation() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info
            .location()
            .map(|l| l.file().replace('\\', "/"))
            .unwrap_or_default();
        PANICS.lock().unwrap().push(format!("{at}: {info}"));
        default_hook(info);
    }));

    let mut seed = 0x9E37_79B9u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let variants = [
        DecodeOptions::default(),
        DecodeOptions {
            target_size: Some((5, 5)),
            first_frame_only: true,
            ..DecodeOptions::default()
        },
    ];
    // PANDITE_FUZZ_ROUNDS raises the mutations per sample for longer runs.
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let mut escaped = Vec::new();
    for (ext, file) in samples() {
        for round in 0..rounds {
            let mut bad = file.clone();
            match round % 4 {
                // Flip a few bytes anywhere.
                0 | 1 => {
                    for _ in 0..1 + next() % 6 {
                        let i = next() as usize % bad.len();
                        bad[i] = next() as u8;
                    }
                }
                // Damage the header region.
                2 => {
                    let limit = bad.len().min(128);
                    for _ in 0..1 + next() % 4 {
                        let i = next() as usize % limit;
                        bad[i] = next() as u8;
                    }
                }
                // Truncate.
                _ => {
                    let cut = next() as usize % bad.len();
                    bad.truncate(cut.max(1));
                }
            }
            let src = source(&bad, &format!("x.{ext}"));
            let options = &variants[round % 2];
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = reg().probe(&src);
                let _ = reg().decode(&src, options);
            }));
            if r.is_err() {
                escaped.push(format!("{ext} round {round}"));
            }
        }
    }
    let _ = std::panic::take_hook();
    let panics = PANICS.lock().unwrap().clone();
    let ours: Vec<&String> = panics
        .iter()
        .filter(|p| p.contains("pandite-codecs/src"))
        .collect();
    for p in &panics {
        eprintln!("caught panic: {p}");
    }
    assert!(escaped.is_empty(), "panics escaped: {escaped:?}");
    assert!(
        ours.is_empty(),
        "panics in this crate's decoders: {ours:#?}"
    );
}

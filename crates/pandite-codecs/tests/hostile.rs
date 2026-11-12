//! Gate G4: hostile input fails cleanly. Truncated and corrupted files
//! return errors without panicking, oversized dimensions are refused before
//! any large allocation, and cancellation stops a decode.

mod common;

use common::hand::*;
use common::*;
use pandite_core::{
    CancelToken, Content, DecodeOptions, DecoderProvider, EncodeOptions, Error, Source,
};
use std::borrow::Cow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::{Duration, Instant};

/// Noisy pixels so compressed streams are mostly image data.
fn noise(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..w * h * 4)
        .map(|i| {
            s = s.wrapping_mul(1_103_515_245).wrapping_add(12345);
            if i % 4 == 3 {
                255
            } else {
                (s >> 16) as u8
            }
        })
        .collect()
}

fn gif_bytes(frames: usize, w: u16, h: u16) -> Vec<u8> {
    let palette: Vec<u8> = (0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect();
    let mut out = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut out, w, h, &palette).unwrap();
        enc.set_repeat(gif::Repeat::Infinite).unwrap();
        for f in 0..frames {
            let px = noise(u32::from(w), u32::from(h), f as u32 + 1);
            let frame = gif::Frame {
                width: w,
                height: h,
                buffer: Cow::Owned(px.chunks_exact(4).map(|p| p[0]).collect()),
                delay: 4,
                ..gif::Frame::default()
            };
            enc.write_frame(&frame).unwrap();
        }
    }
    out
}

/// One valid sample file per base format, named by extension.
fn base_samples() -> Vec<(&'static str, Vec<u8>)> {
    let img = rgba8(48, 48, noise(48, 48, 7));
    let mut v = vec![
        ("png", encode("png", &img, &EncodeOptions::default())),
        ("jpg", encode("jpeg", &img, &EncodeOptions::default())),
        ("bmp", encode("bmp", &img, &EncodeOptions::default())),
        ("webp", encode("webp", &img, &EncodeOptions::default())),
        ("gif", gif_bytes(3, 32, 32)),
        ("webp", fixture("lossy.webp")),
        ("webp", fixture("lossy_anim.webp")),
    ];
    let frames: Vec<WebpFrame> = (0..3)
        .map(|i| WebpFrame {
            x: 0,
            y: 0,
            w: 24,
            h: 24,
            ms: 40,
            blend: i % 2 == 0,
            dispose: i == 1,
            rgba: noise(24, 24, i + 3),
        })
        .collect();
    v.push(("webp", webp_animated(24, 24, 0, &frames)));
    let mut apng = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut apng, 32, 32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_animated(3, 0).unwrap();
        let mut wr = enc.write_header().unwrap();
        for i in 0..3 {
            wr.write_image_data(&noise(32, 32, 40 + i)).unwrap();
        }
        wr.finish().unwrap();
    }
    v.push(("png", apng));
    let pal = bmp_palette(&[[0, 0, 0], [255, 0, 0], [0, 255, 0]]);
    let mut rle = Vec::new();
    for y in 0..16u8 {
        rle.extend_from_slice(&[5, 1 + y % 2, 0, 4, 2, 1, 2, 1, 0, 0]);
    }
    rle.extend_from_slice(&[0, 1]);
    v.push((
        "bmp",
        bmp_file(&bmp_info_header(9, 16, 8, 1, 3), &pal, &rle),
    ));
    v
}

fn try_decode(bytes: &[u8], ext: &str) -> pandite_core::Result<Content> {
    reg().decode(
        &source(bytes, &format!("x.{ext}")),
        &DecodeOptions::default(),
    )
}

#[test]
fn truncated_input_is_error() {
    for (ext, file) in base_samples() {
        assert!(try_decode(&file, ext).is_ok(), "{ext} sample must decode");
        for pct in [5usize, 20, 40, 60, 80, 95] {
            let cut = file.len() * pct / 100;
            let r = catch_unwind(AssertUnwindSafe(|| try_decode(&file[..cut], ext)));
            match r {
                Ok(Err(_)) => {}
                Ok(Ok(_)) => panic!("{ext} truncated to {cut} of {} decoded", file.len()),
                Err(_) => panic!("{ext} truncated to {cut} panicked"),
            }
        }
    }
}

#[test]
fn corrupt_input_never_panics() {
    let mut seed = 0x1234_5678u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    for (ext, file) in base_samples() {
        for round in 0..150 {
            let mut bad = file.clone();
            let flips = 1 + (next() % 8) as usize;
            for _ in 0..flips {
                // Bias toward headers, where damage reaches the most code.
                let limit = if round % 2 == 0 {
                    bad.len().min(96)
                } else {
                    bad.len()
                };
                let i = next() as usize % limit;
                bad[i] = next() as u8;
            }
            let src = source(&bad, &format!("x.{ext}"));
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = reg().probe(&src);
                let _ = reg().decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{ext} mutation round {round} panicked");
        }
        // Garbage after a valid signature.
        let mut junk = file[..file.len().min(16)].to_vec();
        junk.extend((0..200).map(|_| next() as u8));
        let r = catch_unwind(AssertUnwindSafe(|| try_decode(&junk, ext)));
        assert!(matches!(r, Ok(Err(_))), "{ext} junk must be an error");
    }
}

fn expect_too_large(bytes: &[u8], ext: &str) {
    match try_decode(bytes, ext) {
        Err(Error::TooLarge { .. }) => {}
        other => panic!(
            "{ext}: expected TooLarge, got {:?}",
            other.map(|c| c.dimensions())
        ),
    }
}

#[test]
fn oversized_dimensions_refused() {
    // PNG claiming 70000 x 70000.
    let file = png_file(70_000, 70_000, 8, 6, 0, &[], &[0, 0, 0, 0, 0]);
    match try_decode(&file, "png") {
        Err(Error::TooLarge { width, height }) => assert_eq!((width, height), (70_000, 70_000)),
        other => panic!("png: {:?}", other.map(|c| c.dimensions())),
    }

    // JPEG frame header claiming 65000 x 65000 (17 GB as RGBA).
    let mut jpg = encode(
        "jpeg",
        &rgba8(8, 8, noise(8, 8, 1)),
        &EncodeOptions::default(),
    );
    let sof = jpg.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    jpg[sof + 5..sof + 7].copy_from_slice(&65000u16.to_be_bytes());
    jpg[sof + 7..sof + 9].copy_from_slice(&65000u16.to_be_bytes());
    expect_too_large(&jpg, "jpg");

    // GIF logical screen of 65535 x 65535.
    let mut g = gif_bytes(1, 4, 4);
    g[6..8].copy_from_slice(&65535u16.to_le_bytes());
    g[8..10].copy_from_slice(&65535u16.to_le_bytes());
    expect_too_large(&g, "gif");

    // BMP 100000 x 100000.
    let file = bmp_file(&bmp_info_header(100_000, 100_000, 24, 0, 0), &[], &[0; 64]);
    expect_too_large(&file, "bmp");

    // WebP extended canvas of 16 M x 16 M.
    let mut vp8x = vec![0x10, 0, 0, 0];
    vp8x.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    let mut body = riff_chunk(b"VP8X", &vp8x);
    body.extend(vp8l_chunk(&webp_lossless(2, 2, noise(2, 2, 3))));
    let mut file = b"RIFF".to_vec();
    file.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    file.extend_from_slice(b"WEBP");
    file.extend(body);
    let r = try_decode(&file, "webp");
    assert!(
        matches!(r, Err(Error::TooLarge { .. }) | Err(Error::Corrupt(_))),
        "webp: {:?}",
        r.map(|c| c.dimensions())
    );
}

#[test]
fn decode_honors_cancel() {
    // Every provider refuses to start once cancelled, even when called
    // directly rather than through the registry.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    for (ext, file) in base_samples() {
        let src = source(&file, &format!("x.{ext}"));
        let provider = reg().decoder_for(&src).expect("provider");
        assert!(
            matches!(provider.decode(&src, &cancelled), Err(Error::Cancelled)),
            "{ext} ignored a cancelled token"
        );
    }

    // A long animated decode stops part way when cancelled from another
    // thread.
    let big = gif_bytes(120, 320, 240);
    let src = Source::from_bytes(big, None);
    let provider: &dyn DecoderProvider = reg().decoder_for(&src).unwrap().as_ref();
    let start = Instant::now();
    assert!(provider.decode(&src, &DecodeOptions::default()).is_ok());
    let full = start.elapsed();
    let token = CancelToken::new();
    let options = DecodeOptions {
        cancel: token.clone(),
        ..DecodeOptions::default()
    };
    let start = Instant::now();
    let result = std::thread::scope(|s| {
        let h = s.spawn(|| provider.decode(&src, &options));
        std::thread::sleep(full / 5);
        token.cancel();
        h.join().unwrap()
    });
    let took = start.elapsed();
    assert!(
        matches!(result, Err(Error::Cancelled)),
        "decode was not cancelled"
    );
    assert!(
        took < full * 4 / 5 + Duration::from_millis(50),
        "cancel took {took:?} of a {full:?} decode"
    );
}

#[cfg(all(
    feature = "tiff",
    feature = "tga",
    feature = "ico",
    feature = "qoi",
    feature = "avif",
    feature = "jxl",
    feature = "dds",
    feature = "ktx2",
    feature = "exr",
    feature = "hdr",
    feature = "svg",
    feature = "psd",
    feature = "raw",
    feature = "heic"
))]
#[test]
fn default_registry_covers_contract_extensions() {
    // The contract list from the plan: every extension the viewer opens.
    const CONTRACT: &[&str] = &[
        "jpg", "jpeg", "jpe", "jfif", "png", "apng", "gif", "webp", "bmp", "dib", "tif", "tiff",
        "tga", "ico", "cur", "qoi", "avif", "heic", "heif", "jxl", "dds", "ktx2", "exr", "hdr",
        "svg", "psd", "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2", "pef", "srw",
    ];

    let mut want: Vec<&str> = CONTRACT.to_vec();
    if !cfg!(any(windows, target_os = "macos")) {
        // HEIC uses the operating system codec, which Linux lacks.
        want.retain(|e| *e != "heic" && *e != "heif");
    }
    want.sort_unstable();
    let have = reg().decodable_extensions();
    assert_eq!(have, want);
    // Every extension routes to a provider through the registry.
    for ext in &have {
        assert!(reg().can_open_path(std::path::Path::new(&format!("a.{}", ext.to_uppercase()))));
    }
    // Encoders: PNG, JPEG, WebP and BMP.
    let mut enc: Vec<&str> = reg().encoders().iter().map(|e| e.id()).collect();
    enc.sort_unstable();
    assert_eq!(enc, ["bmp", "jpeg", "png", "webp"]);
}

/// Extended formats refuse claimed sizes above the buffer limits before
/// allocating, from their headers alone.
#[test]
fn oversized_dimensions_refused_extended() {
    #[allow(unused_mut)]
    let mut cases: Vec<(&str, Vec<u8>)> = Vec::new();
    #[cfg(feature = "tiff")]
    {
        let mut t = b"II*\0\x08\0\0\0".to_vec();
        let tags = [
            (256u16, 100_000u32),
            (257, 100_000),
            (258, 8),
            (262, 1),
            (273, 8),
            (277, 1),
            (279, 1),
        ];
        t.extend_from_slice(&(tags.len() as u16).to_le_bytes());
        for (tag, value) in tags {
            t.extend_from_slice(&tag.to_le_bytes());
            t.extend_from_slice(&4u16.to_le_bytes());
            t.extend_from_slice(&1u32.to_le_bytes());
            t.extend_from_slice(&value.to_le_bytes());
        }
        t.extend_from_slice(&0u32.to_le_bytes());
        cases.push(("tif", t));
    }
    #[cfg(feature = "qoi")]
    {
        let mut q = b"qoif".to_vec();
        q.extend_from_slice(&100_000u32.to_be_bytes());
        q.extend_from_slice(&100_000u32.to_be_bytes());
        q.extend_from_slice(&[4, 0]);
        q.extend_from_slice(&[0; 64]);
        cases.push(("qoi", q));
    }
    #[cfg(feature = "psd")]
    {
        let mut p = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
        p.extend_from_slice(&300_000u32.to_be_bytes());
        p.extend_from_slice(&300_000u32.to_be_bytes());
        p.extend_from_slice(&[0, 8, 0, 3]);
        p.extend_from_slice(&[0; 16]);
        cases.push(("psd", p));
    }
    #[cfg(feature = "dds")]
    {
        let mut d = b"DDS ".to_vec();
        let mut hdr = [0u32; 31];
        hdr[0] = 124;
        hdr[2] = 100_000;
        hdr[3] = 100_000;
        hdr[18] = 32;
        hdr[19] = 4;
        hdr[20] = u32::from_le_bytes(*b"DXT1");
        for x in hdr {
            d.extend_from_slice(&x.to_le_bytes());
        }
        cases.push(("dds", d));
    }
    #[cfg(feature = "ktx2")]
    {
        let mut k = vec![
            0xAB, b'K', b'T', b'X', b' ', b'2', b'0', 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
        ];
        for x in [37u32, 1, 100_000, 100_000, 0, 0, 1, 1, 0, 0, 0, 0, 0] {
            k.extend_from_slice(&x.to_le_bytes());
        }
        for x in [0u64, 0, 104, 16, 16] {
            k.extend_from_slice(&x.to_le_bytes());
        }
        k.extend_from_slice(&[0; 16]);
        cases.push(("ktx2", k));
    }
    #[cfg(feature = "hdr")]
    cases.push((
        "hdr",
        b"#?RADIANCE\n\n-Y 100000 +X 100000\n\x01\x01\x01\x01".to_vec(),
    ));
    #[cfg(feature = "tga")]
    {
        let mut t = vec![0u8, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        t.extend_from_slice(&65535u16.to_le_bytes());
        t.extend_from_slice(&65535u16.to_le_bytes());
        t.extend_from_slice(&[32, 0x28, 1, 2, 3, 4]);
        cases.push(("tga", t));
    }
    for (ext, file) in &cases {
        expect_too_large(file, ext);
    }

    // SVG has no pixel size of its own: a huge requested size is refused.
    #[cfg(feature = "svg")]
    {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"/>"#;
        let options = DecodeOptions {
            target_size: Some((200_000, 200_000)),
            ..DecodeOptions::default()
        };
        let r = reg().decode(&source(svg, "a.svg"), &options);
        assert!(matches!(r, Err(Error::TooLarge { .. })), "svg");
    }
}

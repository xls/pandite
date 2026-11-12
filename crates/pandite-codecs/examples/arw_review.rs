//! Develop camera RAW files and report decode times; optionally write a
//! review pair: `developed.png` (the developed image, box-downscaled to at
//! most 1600 pixels wide) and `preview.png` (the camera's embedded preview,
//! as delivered for a 1600-pixel thumbnail request).
//!
//! Usage: `arw_review [--out DIR] [--runs N] FILE...`

use pandite_core::{DecodeOptions, EncodeOptions, ImageBuf, PixelFormat, Source};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Box-downscale an image by a whole factor so it is at most `max_w`
/// pixels wide, to RGBA8.
fn downscale(img: &ImageBuf, max_w: u32) -> ImageBuf {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let f = w.div_ceil(max_w as usize).max(1);
    let (ow, oh) = (w / f, h / f);
    let d = img.data();
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let mut sum = [0u64; 4];
            for y in oy * f..oy * f + f {
                for x in ox * f..ox * f + f {
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += match img.format() {
                            PixelFormat::Rgba16 => {
                                let i = (y * w + x) * 8 + 2 * c;
                                u64::from(u16::from_ne_bytes([d[i], d[i + 1]])) / 257
                            }
                            _ => u64::from(d[(y * w + x) * 4 + c]),
                        };
                    }
                }
            }
            for c in 0..4 {
                out[(oy * ow + ox) * 4 + c] = (sum[c] / (f * f) as u64) as u8;
            }
        }
    }
    ImageBuf::new(ow as u32, oh as u32, PixelFormat::Rgba8, out).expect("image")
}

fn main() {
    let mut out_dir: Option<PathBuf> = None;
    let mut runs = 1usize;
    let mut files = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out_dir = args.next().map(PathBuf::from),
            "--runs" => runs = args.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            _ => files.push(PathBuf::from(a)),
        }
    }
    let reg = pandite_codecs::default_registry();
    for path in &files {
        let source = Source::from_path(path).expect("read file");
        let mut best = Duration::MAX;
        let mut image = None;
        for _ in 0..runs.max(1) {
            let start = Instant::now();
            let content = reg
                .decode(&source, &DecodeOptions::default())
                .expect("decode");
            best = best.min(start.elapsed());
            image = Some(content.first_image().clone());
        }
        let img = image.expect("decoded image");
        println!(
            "{}: {}x{} {:?} best of {} runs {:.0} ms",
            path.display(),
            img.width(),
            img.height(),
            img.format(),
            runs.max(1),
            best.as_secs_f64() * 1000.0
        );
        if let Some(dir) = &out_dir {
            std::fs::create_dir_all(dir).expect("create output folder");
            let png = reg.encoder_by_id("png").expect("PNG encoder");
            let mut bytes = Vec::new();
            png.encode(
                &downscale(&img, 1600),
                &EncodeOptions::default(),
                &mut bytes,
            )
            .expect("encode");
            std::fs::write(dir.join("developed.png"), bytes).expect("write");
            let thumb = DecodeOptions {
                target_size: Some((1600, 1600)),
                ..DecodeOptions::default()
            };
            let preview = reg.decode(&source, &thumb).expect("preview");
            let mut bytes = Vec::new();
            png.encode(preview.first_image(), &EncodeOptions::default(), &mut bytes)
                .expect("encode");
            std::fs::write(dir.join("preview.png"), bytes).expect("write");
            println!("wrote {}", dir.display());
        }
    }
}

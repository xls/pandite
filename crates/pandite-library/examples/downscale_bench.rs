//! Time the thumbnail downscaler on Rgba8 images: the `f64` reference (the
//! previous implementation), the scalar integer path and every vector
//! level this CPU has, on the same input, best of several runs each.
//!
//! Usage:
//!
//! ```text
//! cargo run --release -p pandite-library --example downscale_bench -- \
//!     [--sizes 6000x4000,1616x1080] [--runs 5] [--max-side 256]
//! ```
//!
//! Prints one machine-readable line per measurement:
//!
//! ```text
//! DOWNSCALE size=<w>x<h> alpha=<opaque|translucent> path=<reference|scalar|avx2|neon> best_ms=<f>
//! ```
//!
//! and one per image, where `fast` is `box_downscale` at the level this
//! process detected and `max_diff` the largest difference of one channel
//! from the reference:
//!
//! ```text
//! DOWNSCALE_SPEEDUP size=<w>x<h> alpha=<a> level=<l> reference_ms=<f> fast_ms=<f> speedup=<f> max_diff=<n>
//! ```
//!
//! Exits with status 1 when a vector level differs from the scalar path or
//! a result differs from the reference by more than one step.

use pandite_core::{ImageBuf, PixelFormat};
use pandite_library::{box_downscale, box_downscale_reference, box_downscale_with, DownscaleLevel};
use std::time::Instant;

const DEFAULT_SIZES: [(u32, u32); 4] = [(1616, 1080), (4000, 3000), (6000, 4000), (9504, 6336)];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let option = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let runs: usize = option("--runs").map_or(5, |v| v.parse().expect("--runs N"));
    let max_side: u32 = option("--max-side").map_or(256, |v| v.parse().expect("--max-side N"));
    let sizes: Vec<(u32, u32)> = match option("--sizes") {
        Some(list) => list.split(',').map(parse_size).collect(),
        None => DEFAULT_SIZES.to_vec(),
    };
    let detected = DownscaleLevel::detected();
    println!(
        "levels on this CPU: {:?}; detected {}; best of {runs} runs; longest side {max_side}",
        DownscaleLevel::available(),
        detected.name()
    );
    let mut ok = true;
    for &(w, h) in &sizes {
        for opaque in [true, false] {
            let alpha = if opaque { "opaque" } else { "translucent" };
            let image = test_image(w, h, opaque);
            let tag = format!("size={w}x{h} alpha={alpha}");
            let (reference_ms, reference) =
                best(runs, || box_downscale_reference(&image, max_side));
            println!("DOWNSCALE {tag} path=reference best_ms={reference_ms:.3}");
            let scalar = box_downscale_with(DownscaleLevel::Scalar, &image, max_side);
            for level in DownscaleLevel::available() {
                let (ms, out) = best(runs, || box_downscale_with(level, &image, max_side));
                println!("DOWNSCALE {tag} path={} best_ms={ms:.3}", level.name());
                if out.data() != scalar.data() {
                    println!("MISMATCH {tag}: {} differs from scalar", level.name());
                    ok = false;
                }
            }
            let (fast_ms, fast) = best(runs, || box_downscale(&image, max_side));
            let max_diff = fast
                .data()
                .iter()
                .zip(reference.data())
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap_or(0);
            if max_diff > 1 || fast.data().len() != reference.data().len() {
                ok = false;
            }
            println!(
                "DOWNSCALE_SPEEDUP {tag} level={} reference_ms={reference_ms:.3} fast_ms={fast_ms:.3} speedup={:.2} max_diff={max_diff}",
                detected.name(),
                reference_ms / fast_ms
            );
        }
    }
    if !ok {
        println!("DOWNSCALE CHECK FAILED");
        std::process::exit(1);
    }
}

fn parse_size(s: &str) -> (u32, u32) {
    let (w, h) = s.split_once('x').expect("size as WxH");
    (w.parse().expect("width"), h.parse().expect("height"))
}

/// Pseudo-random pixels; alpha 255 when `opaque`, else random (zero too).
fn test_image(w: u32, h: u32, opaque: bool) -> ImageBuf {
    let mut data = vec![0u8; w as usize * h as usize * 4];
    for (i, b) in data.iter_mut().enumerate() {
        *b = if opaque && i % 4 == 3 {
            255
        } else {
            (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3]
        };
    }
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).expect("image size")
}

/// The fastest of `runs` calls in milliseconds, and the last result.
fn best(runs: usize, f: impl Fn() -> ImageBuf) -> (f64, ImageBuf) {
    let mut fastest = f64::MAX;
    let mut out = None;
    for _ in 0..runs.max(1) {
        let t = Instant::now();
        let image = std::hint::black_box(f());
        fastest = fastest.min(t.elapsed().as_secs_f64() * 1e3);
        out = Some(image);
    }
    (fastest, out.expect("at least one run"))
}

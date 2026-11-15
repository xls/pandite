//! Times the stages of loading large files, the way the viewer loads them:
//! `cargo run --release -p pandite-codecs --example load_bench -- <file>...`
//!
//! For each file it prints the read time, the probe, the display decode
//! (full size, 8 bits preferred) and reduced decodes for two window sizes,
//! three runs each. The first run also prints a 64-bit FNV-1a digest of
//! the pixels, so two builds can be checked for identical output. Set
//! `PANDITE_SIMD=scalar` to compare against the scalar kernels. It writes
//! nothing.

use pandite_core::{DecodeOptions, Source};
use std::time::Instant;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// FNV-1a over the bytes.
fn digest(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn main() {
    let reg = pandite_codecs::default_registry();
    for arg in std::env::args().skip(1) {
        let path = std::path::PathBuf::from(&arg);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let t = Instant::now();
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                println!("{name}: cannot read: {err}");
                continue;
            }
        };
        let read = ms(t);
        let src = Source::from_vec(bytes, Some(path.clone()));
        let t = Instant::now();
        let meta = match reg.probe(&src) {
            Ok(meta) => meta,
            Err(err) => {
                println!("{name}: cannot probe: {err}");
                continue;
            }
        };
        println!(
            "{name}: read {read:.1} ms, probe {:.2} ms, {}x{}",
            ms(t),
            meta.width,
            meta.height
        );
        for target in [None, Some((2560u32, 1440u32)), Some((1280, 720))] {
            for run in 0..3 {
                let options = DecodeOptions {
                    target_size: target,
                    prefer_8bit: true,
                    ..DecodeOptions::default()
                };
                let t = Instant::now();
                match reg.decode(&src, &options) {
                    Ok(content) => {
                        let took = ms(t);
                        let img = content.first_image();
                        // The digest of the first run only: it takes a while.
                        let sum = if run == 0 {
                            format!(", digest {:016x}", digest(img.data()))
                        } else {
                            String::new()
                        };
                        println!(
                            "  decode target {target:?}: {took:.1} ms -> {}x{} {:?}{sum}",
                            img.width(),
                            img.height(),
                            img.format()
                        );
                    }
                    Err(err) => println!("  decode target {target:?}: {err}"),
                }
            }
        }
    }
}

//! Read the text in image files and print it.
//!
//! `cargo run --release -p pandite-ocr --example ocr_file -- <image file>...`
//!
//! Decodes each file with pandite-codecs and reads it with the platform
//! recognizer: the built-in PP-OCRv6 models on macOS and Linux, Windows OCR
//! on Windows. Prints one line `FILE <name> <width>x<height> <seconds>` per
//! file, then one line `LINE <text>` per line of text in reading order.
//! `<name>` is the file name without its folder. The seconds cover the
//! recognition only, not the decoding. The models load before the first
//! file; the first file of each new detection size also optimizes a plan
//! for that size. Errors go to standard error, and the exit code is 1 when
//! a file failed.

use pandite_core::{CancelToken, DecodeOptions, Source};
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: ocr_file <image file>...");
        return ExitCode::from(2);
    }
    let Some(ocr) = pandite_ocr::text_providers()
        .into_iter()
        .find(|p| p.available())
    else {
        eprintln!("no text recognizer is available on this system");
        return ExitCode::FAILURE;
    };
    eprintln!("recognizer: {}", ocr.name());
    let registry = pandite_codecs::default_registry();
    let options = DecodeOptions {
        first_frame_only: true,
        ..DecodeOptions::default()
    };
    let mut failed = false;
    for file in &files {
        let name = Path::new(file)
            .file_name()
            .map_or_else(|| file.clone(), |n| n.to_string_lossy().into_owned());
        let decoded = Source::from_path(file).and_then(|s| registry.decode(&s, &options));
        let image = match decoded {
            Ok(content) => content.first_image().clone(),
            Err(e) => {
                eprintln!("{file}: {e}");
                failed = true;
                continue;
            }
        };
        let started = Instant::now();
        match ocr.recognize(&image, &CancelToken::new()) {
            Ok(layout) => {
                let seconds = started.elapsed().as_secs_f64();
                println!(
                    "FILE {name} {}x{} {seconds:.3}",
                    image.width(),
                    image.height()
                );
                for line in &layout.lines {
                    println!("LINE {}", line.text());
                }
            }
            Err(e) => {
                eprintln!("{file}: {e}");
                failed = true;
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

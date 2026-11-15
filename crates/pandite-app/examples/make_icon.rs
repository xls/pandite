//! Write the pandite icon as PNG files, plus the Windows `.ico`.
//!
//! Usage: `cargo run -p pandite-app --example make_icon -- <out_dir> <size> [<size>...]`
//!
//! Scales the master `assets/icon-1024.png` of this crate. Writes
//! `icon-<size>.png` for each size (transparent outside the tile), the raw
//! window icons `icon-32.rgba` and `icon-64.rgba`, and `pandite.ico` with
//! every size of
//! `pandite_app::icon::ICO_SIZES` (32-bit bitmaps below 256 px, PNG at
//! 256 px). Copy `pandite.ico`, `icon-256.png` and the two `.rgba` files to
//! `crates/pandite-app/assets/` after the icon changes. Sizes run from 1 to 4096.

use pandite_app::icon::{encode_png, program_ico, scale_icon, MAX_ICON_SIZE};
use pandite_core::{DecodeOptions, DecoderProvider, ImageBuf, Source};
use std::path::PathBuf;
use std::process::ExitCode;

fn usage() -> ExitCode {
    eprintln!("usage: make_icon <out_dir> <size> [<size>...]   (sizes 1 to {MAX_ICON_SIZE})");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        return usage();
    };
    let mut sizes = Vec::new();
    for arg in args {
        match arg.to_str().and_then(|s| s.parse::<u32>().ok()) {
            Some(size) if (1..=MAX_ICON_SIZE).contains(&size) => sizes.push(size),
            _ => {
                eprintln!("not a size: {}", arg.to_string_lossy());
                return usage();
            }
        }
    }
    if sizes.is_empty() {
        return usage();
    }
    sizes.sort_unstable();
    sizes.dedup();
    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!("cannot create {}: {err}", dir.display());
        return ExitCode::FAILURE;
    }
    let master_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon-1024.png");
    let master: ImageBuf = match std::fs::read(&master_path)
        .map_err(|e| e.to_string())
        .and_then(|bytes| {
            pandite_codecs::PngDecoder
                .decode(&Source::from_vec(bytes, None), &DecodeOptions::default())
                .map(|content| content.first_image().clone())
                .map_err(|e| e.to_string())
        }) {
        Ok(master) => master,
        Err(err) => {
            eprintln!("cannot read {}: {err}", master_path.display());
            return ExitCode::FAILURE;
        }
    };
    for side in [32u32, 64] {
        let path = dir.join(format!("icon-{side}.rgba"));
        let raw = match scale_icon(&master, side) {
            Ok(image) => image.data().to_vec(),
            Err(err) => {
                eprintln!("cannot scale the {side} px icon: {err}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(err) = std::fs::write(&path, &raw) {
            eprintln!("cannot write {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", path.display());
    }
    for &size in &sizes {
        let png = match scale_icon(&master, size).and_then(|img| encode_png(&img)) {
            Ok(png) => png,
            Err(err) => {
                eprintln!("cannot scale the {size} px icon: {err}");
                return ExitCode::FAILURE;
            }
        };
        let path = dir.join(format!("icon-{size}.png"));
        if let Err(err) = std::fs::write(&path, &png) {
            eprintln!("cannot write {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
        println!("wrote {}", path.display());
    }
    let path = dir.join("pandite.ico");
    let ico = match program_ico(&master) {
        Ok(ico) => ico,
        Err(err) => {
            eprintln!("cannot build the icon file: {err}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(err) = std::fs::write(&path, ico) {
        eprintln!("cannot write {}: {err}", path.display());
        return ExitCode::FAILURE;
    }
    println!("wrote {}", path.display());
    ExitCode::SUCCESS
}

//! Write the pandite icon as PNG files, plus the Windows `.ico`.
//!
//! Usage: `cargo run -p pandite-app --example make_icon -- <out_dir> <size> [<size>...]`
//!
//! Writes `icon-<size>.png` for each size (antialiased, transparent
//! background) and `pandite.ico` with every size of
//! `pandite_app::icon::ICO_SIZES` (32-bit bitmaps below 256 px, PNG at
//! 256 px). Copy that file to `crates/pandite-app/assets/` after the icon
//! changes. Sizes run from 1 to 4096.

use pandite_app::icon::{encode_png, program_ico, render_icon, MAX_ICON_SIZE};
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
    for &size in &sizes {
        let png = match render_icon(size).and_then(|img| encode_png(&img)) {
            Ok(png) => png,
            Err(err) => {
                eprintln!("cannot draw the {size} px icon: {err}");
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
    let ico = match program_ico() {
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

//! Build script: embed the application icon, the version information and
//! the manifest in the Windows executable.
//!
//! It writes a compiled resource file (`.res`) directly, in pure Rust
//! (`src/winres.rs`), from `assets/pandite.ico`, the package version and
//! `assets/pandite.manifest`, and hands it to the MSVC linker, which
//! accepts `.res` inputs. No resource compiler, build dependency or
//! network access is needed. Other targets need nothing.

use std::path::PathBuf;

#[path = "src/winres.rs"]
mod winres;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/winres.rs");
    println!("cargo:rerun-if-changed=assets/pandite.ico");
    println!("cargo:rerun-if-changed=assets/pandite.manifest");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if os != "windows" || env != "msvc" {
        return;
    }
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let ico = std::fs::read(dir.join("assets/pandite.ico")).expect("read assets/pandite.ico");
    let manifest =
        std::fs::read(dir.join("assets/pandite.manifest")).expect("read assets/pandite.manifest");
    let version = env!("CARGO_PKG_VERSION");
    // Windows shows FileDescription as the program name, for example in
    // the "Open with" list.
    let version_block = winres::version_info(&winres::VersionStrings {
        version,
        fields: &[
            ("CompanyName", "Pandite"),
            ("FileDescription", "Pandite"),
            ("FileVersion", version),
            ("InternalName", "pandite"),
            ("LegalCopyright", "MIT License"),
            ("OriginalFilename", "pandite.exe"),
            ("ProductName", "Pandite"),
            ("ProductVersion", version),
        ],
    });
    let res = winres::build_res(&ico, &version_block, &manifest)
        .expect("assets/pandite.ico is a valid icon file");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("pandite.res");
    std::fs::write(&out, res).expect("write pandite.res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

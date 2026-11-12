//! Build script: embed the application icon and manifest in the Windows
//! executable.
//!
//! It writes a compiled resource file (`.res`) directly, in pure Rust, from
//! `assets/pandite.ico` and `assets/pandite.manifest`, and hands it to the
//! MSVC linker, which accepts `.res` inputs. No resource compiler, build
//! dependency or network access is needed. Other targets need nothing.

use std::path::PathBuf;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_MANIFEST: u16 = 24;
const LANG_EN_US: u16 = 0x0409;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
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
    let res = build_res(&ico, &manifest).expect("assets/pandite.ico is a valid icon file");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("out dir")).join("pandite.res");
    std::fs::write(&out, res).expect("write pandite.res");
    println!("cargo:rustc-link-arg-bins={}", out.display());
}

/// One resource entry: 32-byte header with ordinal type and name, then the
/// data padded to a 4-byte boundary.
fn push_resource(out: &mut Vec<u8>, kind: u16, id: u16, flags: u16, lang: u16, data: &[u8]) {
    let size = u32::try_from(data.len()).expect("resource fits in 4 GiB");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0xFFFFu16.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&lang.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Convert an `.ico` file into RT_ICON images plus the RT_GROUP_ICON
/// directory that names them, and add the manifest.
fn build_res(ico: &[u8], manifest: &[u8]) -> Option<Vec<u8>> {
    if u16_at(ico, 0)? != 0 || u16_at(ico, 2)? != 1 {
        return None;
    }
    let count = u16_at(ico, 4)?;
    let mut res = Vec::new();
    // The empty entry every .res file starts with.
    push_resource(&mut res, 0, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    for i in 0..usize::from(count) {
        let entry = 6 + 16 * i;
        let head = ico.get(entry..entry + 8)?;
        let len = u32_at(ico, entry + 8)? as usize;
        let offset = u32_at(ico, entry + 12)? as usize;
        let image = ico.get(offset..offset.checked_add(len)?)?;
        let id = u16::try_from(i + 1).ok()?;
        // MOVEABLE | DISCARDABLE, as resource compilers write icons.
        push_resource(&mut res, RT_ICON, id, 0x1010, LANG_EN_US, image);
        group.extend_from_slice(head);
        group.extend_from_slice(&u32::try_from(len).ok()?.to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    push_resource(&mut res, RT_GROUP_ICON, 1, 0x1030, LANG_EN_US, &group);
    push_resource(&mut res, RT_MANIFEST, 1, 0x0030, LANG_EN_US, manifest);
    Some(res)
}

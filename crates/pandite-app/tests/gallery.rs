//! Gallery thumbnails fill their cells pixel for pixel: the core makes them
//! at the cell size of the gallery size setting and the scale factor, and
//! makes them again when either changes. The old thumbnail shows until its
//! new one is uploaded.

mod support;

use pandite_app::app::{Handled, WindowState};
use pandite_app::settings::{GallerySize, Settings};
use pandite_core::{DrawCmd, DrawList, Rect, Size, TextureId};
use pandite_ui::{thumbnail_index, thumbnail_texture_id, Action};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use support::*;

/// The thumbnail draws of a frame: folder index and cell.
fn cells(list: &DrawList) -> Vec<(usize, Rect)> {
    list.cmds
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Image { rect, texture, .. } => thumbnail_index(*texture).map(|i| (i, *rect)),
            _ => None,
        })
        .collect()
}

/// PNG images of these sizes, named a.png, b.png, ... in natural order.
fn png_folder(tag: &str, sizes: &[(u32, u32)]) -> (TempDir, Vec<PathBuf>) {
    let dir = TempDir::new(tag);
    let paths = sizes
        .iter()
        .enumerate()
        .map(|(i, &(w, h))| {
            let p = dir.join(&format!("{}.png", (b'a' + i as u8) as char));
            write_png(
                &p,
                &image(w, h, |x, y| [x as u8, y as u8, i as u8 * 40, 255]),
            );
            p
        })
        .collect();
    (dir, paths)
}

/// Staged images of `w` x `h`, named a.stg, b.stg, ...
fn staged_folder(tag: &str, count: usize, w: u32, h: u32) -> (TempDir, Vec<PathBuf>) {
    let dir = TempDir::new(tag);
    let paths = (0..count)
        .map(|i| {
            let p = dir.join(&format!("{}.stg", (b'a' + i as u8) as char));
            write_staged(&p, w, h);
            p
        })
        .collect();
    (dir, paths)
}

/// The size of the texture of thumbnail `index`, when one is uploaded.
fn texture(textures: &HashMap<TextureId, (u32, u32)>, index: usize) -> Option<(u32, u32)> {
    textures.get(&thumbnail_texture_id(index)).copied()
}

/// True when thumbnails `indexes` are uploaded as `side` squares.
fn all_at(textures: &HashMap<TextureId, (u32, u32)>, indexes: &[usize], side: u32) -> bool {
    indexes
        .iter()
        .all(|&i| texture(textures, i) == Some((side, side)))
}

/// A core on the first of `paths` with the gated decoder and no
/// read-ahead, so each other thumbnail needs a decode of its own.
fn staged_harness(paths: &[PathBuf], dec: &Arc<GatedDecoder>, scale: f32) -> Harness {
    let mut h = Harness::new(Config {
        settings: Settings {
            read_ahead: 0,
            ..Settings::default()
        },
        open: Some(paths[0].clone()),
        registry: Some(dec.registry()),
        window: WindowState {
            size: Size::new(1000.0, 800.0),
            scale,
            ..WindowState::default()
        },
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_thumbnails(paths.len()));
    h
}

/// Every drawn thumbnail fills its cell pixel for pixel: the cell has the
/// side of the gallery size at the scale, and so has the texture.
fn assert_pixel_for_pixel(h: &mut Harness, side: u32, why: &str) {
    let list = h.frame();
    let drawn = cells(&list);
    assert!(!drawn.is_empty(), "{why}");
    for (i, cell) in drawn {
        assert_eq!((cell.w, cell.h), (side as f32, side as f32), "{why}: {i}");
        assert_eq!(texture(&h.textures, i), Some((side, side)), "{why}: {i}");
    }
}

#[test]
fn thumbnails_fill_their_cells_pixel_for_pixel() {
    // Landscape, portrait, square and panorama images larger than every
    // cell, and one tiny image.
    let sizes = [(600, 400), (400, 600), (500, 500), (2400, 300), (40, 30)];
    let (_dir, paths) = png_folder("cells", &sizes);
    for size in GallerySize::ALL {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let why = format!("{size:?} at {scale}");
            let side = size.cell_px(scale);
            let mut h = Harness::new(Config {
                settings: Settings {
                    gallery_size: size,
                    ..Settings::default()
                },
                open: Some(paths[0].clone()),
                window: WindowState {
                    size: Size::new(2000.0, 1200.0),
                    scale,
                    ..WindowState::default()
                },
                ..Config::default()
            });
            h.start();
            assert!(h.wait_shown(&paths[0]), "{why}");
            assert!(h.wait_thumbnails(sizes.len()), "{why}");
            assert_eq!(h.app().thumbnail_px(), side, "{why}");
            let list = h.frame();
            let drawn = cells(&list);
            assert_eq!(drawn.len(), sizes.len(), "{why}: every cell shows");
            for (i, cell) in drawn {
                assert_eq!((cell.w, cell.h), (side as f32, side as f32), "{why}: {i}");
                // The tiny image is not enlarged: its middle square at full
                // resolution covers the cell.
                let want = if i == 4 { (30, 30) } else { (side, side) };
                assert_eq!(texture(&h.textures, i), Some(want), "{why}: {i}");
            }
            // No thumbnail of another size was ever uploaded.
            for (id, dims) in &h.uploads {
                if let Some(i) = thumbnail_index(*id) {
                    let want = if i == 4 { (30, 30) } else { (side, side) };
                    assert_eq!(*dims, want, "{why}: upload of {i}");
                }
            }
        }
    }
}

#[test]
fn size_change_remakes_thumbnails() {
    let (_dir, paths) = staged_folder("resize", 4, 600, 400);
    let dec = GatedDecoder::new();
    let mut h = staged_harness(&paths, &dec, 1.0);
    let all = [0, 1, 2, 3];
    assert_eq!(h.app().settings().gallery_size, GallerySize::Medium);
    assert_eq!(h.app().thumbnail_px(), 96);
    assert_pixel_for_pixel(&mut h, 96, "medium");
    // The current image's thumbnail came from its full decode.
    let mut decoded = dec.thumbnail_decodes_at(96);
    decoded.sort();
    assert_eq!(decoded, ["b.stg", "c.stg", "d.stg"]);

    // Larger: the old thumbnails are too small, so the files are decoded
    // again; the current image's full decode is used as it is. One decode
    // is held: its cell shows the old thumbnail, scaled, until it ends.
    dec.hold_reduced("c.stg");
    assert_eq!(h.act(Action::LargerThumbnails), Handled::Done);
    assert_eq!(h.app().settings().gallery_size, GallerySize::Large);
    assert_eq!(h.app().thumbnail_px(), 128);
    assert!(dec.wait_thumbnail_decode("c.stg", 128));
    assert!(h.wait_textures(|t| all_at(t, &[0, 1, 3], 128)));
    let list = h.frame();
    let c = cells(&list)
        .into_iter()
        .find(|d| d.0 == 2)
        .expect("c drawn");
    assert_eq!((c.1.w, c.1.h), (128.0, 128.0));
    assert_eq!(texture(&h.textures, 2), Some((96, 96)), "the old one shows");
    dec.release_reduced("c.stg");
    assert!(h.wait_textures(|t| all_at(t, &all, 128)));
    assert_pixel_for_pixel(&mut h, 128, "large");
    let mut decoded = dec.thumbnail_decodes_at(128);
    decoded.sort();
    assert_eq!(decoded, ["b.stg", "c.stg", "d.stg"]);
    assert_eq!(dec.full_finished("a.stg"), 1, "one full decode");

    // The end of the range: nothing larger.
    assert_eq!(h.act(Action::LargerThumbnails), Handled::Ignored);
    assert_eq!(h.app().settings().gallery_size, GallerySize::Large);

    // Smaller: each new thumbnail is made from its larger old one or from
    // the full decode, without reading a file again.
    let reads = |h: &Harness| -> Vec<usize> {
        let names = ["a.stg", "b.stg", "c.stg", "d.stg"];
        names.iter().map(|n| h.disk.reads(n)).collect()
    };
    let before = reads(&h);
    assert_eq!(h.act(Action::SmallerThumbnails), Handled::Done);
    assert_eq!(h.act(Action::SmallerThumbnails), Handled::Done);
    assert_eq!(h.app().settings().gallery_size, GallerySize::Small);
    assert!(h.wait_textures(|t| all_at(t, &all, 64)));
    assert_pixel_for_pixel(&mut h, 64, "small");
    assert!(dec.thumbnail_decodes_at(64).is_empty());
    assert_eq!(dec.thumbnail_decodes_at(96).len(), 3, "no decode at medium");
    assert_eq!(reads(&h), before, "no file read again");
    assert_eq!(h.act(Action::SmallerThumbnails), Handled::Ignored);
    assert_eq!(h.app().settings().gallery_size, GallerySize::Small);
}

#[test]
fn scale_change_remakes_thumbnails() {
    let (_dir, paths) = staged_folder("rescale", 4, 600, 400);
    let dec = GatedDecoder::new();
    let mut h = staged_harness(&paths, &dec, 1.0);
    let all = [0, 1, 2, 3];
    assert_pixel_for_pixel(&mut h, 96, "scale 1");
    // The window moves to a screen at 150 percent: medium cells are 144
    // pixels there.
    h.app().scale_changed(1.5);
    assert_eq!(h.app().thumbnail_px(), 144);
    assert!(h.wait_textures(|t| all_at(t, &all, 144)));
    assert_pixel_for_pixel(&mut h, 144, "scale 1.5");
    let mut decoded = dec.thumbnail_decodes_at(144);
    decoded.sort();
    assert_eq!(decoded, ["b.stg", "c.stg", "d.stg"]);
    // And at 125 percent: 120 pixels, made from the larger thumbnails.
    h.app().scale_changed(1.25);
    assert_eq!(h.app().thumbnail_px(), 120);
    assert!(h.wait_textures(|t| all_at(t, &all, 120)));
    assert_pixel_for_pixel(&mut h, 120, "scale 1.25");
    assert!(dec.thumbnail_decodes_at(120).is_empty());
    // The same scale again changes nothing.
    let uploads = h.uploads.len();
    h.app().scale_changed(1.25);
    assert!(h.app().library().wait_idle(WAIT));
    h.pump();
    h.frame();
    assert_eq!(h.uploads.len(), uploads);
}

#[test]
fn gallery_size_setting_is_saved() {
    let dir = TempDir::new("size-setting");
    let path = dir.join("settings.ini");
    let mut h = Harness::new(Config {
        settings_path: Some(path.clone()),
        ..Config::default()
    });
    h.act(Action::LargerThumbnails);
    let report = h
        .app
        .take()
        .unwrap()
        .shutdown(std::time::Duration::from_secs(10));
    assert_eq!(report.settings_saved, Some(true));
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("gallery_size = large"), "{text}");
    let (loaded, _) = pandite_app::settings::load(&pandite_app::disk::StdDisk, &path);
    assert_eq!(loaded.gallery_size, GallerySize::Large);
    // The next start makes its thumbnails at the saved size.
    let h = Harness::new(Config {
        settings: loaded,
        ..Config::default()
    });
    assert_eq!(h.app.as_ref().unwrap().thumbnail_px(), 128);
}

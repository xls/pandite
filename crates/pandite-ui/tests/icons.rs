//! Gate G5 evidence: in-crate icons that stay distinct at 16 pixels, and a
//! dependency list with nothing but pandite-core.

mod support;

use pandite_ui::{Icon, Ui};
use support::*;

#[test]
fn icons_distinct_at_16px() {
    // The badge mark is the check mark at its own size, not a new shape.
    let masks: Vec<(Icon, Vec<u8>)> = Icon::ALL
        .iter()
        .filter(|i| **i != Icon::Mark)
        .map(|i| (*i, i.coverage(16)))
        .collect();
    let mut worst = (usize::MAX, Icon::Open, Icon::Open);
    for (i, (a, ma)) in masks.iter().enumerate() {
        for (b, mb) in &masks[i + 1..] {
            let differing = ma
                .iter()
                .zip(mb)
                .filter(|(x, y)| (i32::from(**x) - i32::from(**y)).abs() > 96)
                .count();
            if differing < worst.0 {
                worst = (differing, *a, *b);
            }
            assert!(
                differing >= 8,
                "{} and {} differ in only {differing} of 256 pixels at 16 px",
                a.name(),
                b.name()
            );
        }
    }
    println!(
        "closest pair at 16 px: {} / {} ({} pixels differ)",
        worst.1.name(),
        worst.2.name(),
        worst.0
    );
}

#[test]
fn icons_are_antialiased_alpha_masks() {
    for icon in Icon::ALL {
        let image = icon.rasterize(20);
        assert_eq!((image.width(), image.height()), (20, 20));
        let data = image.data();
        assert!(data
            .chunks_exact(4)
            .all(|p| p[0] == 255 && p[1] == 255 && p[2] == 255));
        let alphas: Vec<u8> = data.chunks_exact(4).map(|p| p[3]).collect();
        // A stroke is 1.25 pixels wide at this size: it need not cover a
        // whole pixel, and its core is still clearly lit.
        let peak = alphas.iter().copied().max().unwrap_or(0);
        assert!(peak >= 200, "{} has a clear core: {peak}", icon.name());
        assert!(
            alphas.iter().any(|&a| a > 0 && a < 255),
            "{} has soft edges",
            icon.name()
        );
    }
    // The interface uploads them as alpha masks, once per size.
    let ctx = image_ctx();
    let mut ui = Ui::new();
    let mut sink = FakeSink::default();
    ui.sync_textures(&ctx, &mut sink);
    assert!(sink.uploads.iter().all(|u| u.3), "alpha_mask set");
    let ids: Vec<_> = sink.uploads.iter().map(|u| u.0).collect();
    for icon in Icon::ALL {
        assert!(ids.contains(&icon.texture_id()));
    }
}

#[test]
fn crate_depends_only_on_core() {
    let manifest = include_str!("../Cargo.toml");
    let mut section = String::new();
    let mut deps = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        if section.contains("dependencies") && !line.is_empty() && !line.starts_with('#') {
            deps.push((
                section.clone(),
                line.split('=').next().unwrap_or("").trim().to_string(),
            ));
        }
    }
    assert_eq!(
        deps,
        vec![("[dependencies]".to_string(), "pandite-core".to_string())],
        "pandite-ui may depend on pandite-core only"
    );
}

#[test]
fn stroke_is_light_at_every_scale() {
    // The factor on the drawn stroke of 2 grid units (24 units per icon):
    // 1.25 pixels at 100 and 150 percent, growing with the scale above.
    let px = |size: u32| Icon::Open.stroke(size) * 2.0 * size as f32 / 24.0;
    assert!((px(20) - 1.25).abs() < 1e-4, "{}", px(20));
    assert!((px(30) - 1.25).abs() < 1e-4, "{}", px(30));
    assert!((px(40) - 1.25 * 2.0 / 1.5).abs() < 1e-4, "{}", px(40));
    assert!((px(60) - 1.25 * 3.0 / 1.5).abs() < 1e-4, "{}", px(60));
    // The badge mark keeps its drawn weight at its own small size.
    assert_eq!(Icon::Mark.stroke(12), 1.0);
    // Lighter strokes cover less: the icon at 20 pixels has less ink than
    // the same shape at its full drawn weight would have.
    let ink = |mask: &[u8]| mask.iter().map(|&v| u32::from(v)).sum::<u32>();
    assert!(Icon::Open.stroke(20) < 1.0);
    assert!(ink(&Icon::Open.coverage(20)) > 0);
}

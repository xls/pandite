//! Measure recognition time and word recall against the recognition cap.
//!
//! `cargo run --release -p pandite-ocr --example ocr_bench`
//!
//! Draws a 9504 x 6336 synthetic photo (gradients, noise and ten lines of
//! distinct words from 18 to 72 pixels) and reads it with the platform
//! recognizer at several caps of the long side. Prints, per cap, the best
//! of three full calls (gray conversion, scaling and the engine) and how
//! many words of each size came back. Nothing is written to disk.

#[cfg(windows)]
fn main() {
    use pandite_core::{CancelToken, ImageBuf, PixelFormat, TextProvider};
    use pandite_ocr::WindowsOcr;
    use std::time::{Duration, Instant};

    static FONT: &[u8] = include_bytes!("../../pandite-render/fonts/FiraSans-Regular-subset.ttf");
    const LINES: [&str; 10] = [
        "quick brown fox jumps over lazy dog",
        "pandite reads text inside photos offline",
        "seventeen orange kites drift above harbor",
        "measure every cap before choosing one",
        "silver lantern glows beside quiet river",
        "frozen meadow hides seven sleepy rabbits",
        "copper kettle whistles during stormy evening",
        "violet mountains echo distant thunder",
        "golden tractor plows northern fields slowly",
        "patient spider weaves delicate silk webs",
    ];
    const SIZES: [f32; 5] = [18.0, 24.0, 32.0, 48.0, 72.0];

    if !WindowsOcr::new().available() {
        println!("Windows OCR is not available: no OCR language is installed");
        return;
    }
    let font = fontdue::Font::from_bytes(FONT, fontdue::FontSettings::default()).unwrap();
    let (w, h) = (9504usize, 6336usize);
    let mut gray = vec![0u8; w * h];
    let mut seed = 12_345u32;
    for y in 0..h {
        for x in 0..w {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (seed >> 24) as i32 % 24 - 12;
            let wave = ((x as f32 / 300.0).sin() * 40.0 + (y as f32 / 410.0).cos() * 30.0) as i32;
            gray[y * w + x] = (150 + wave + noise).clamp(0, 255) as u8;
        }
    }
    let mut y0 = 200.0f32;
    for (i, &px) in SIZES.iter().enumerate() {
        for k in 0..2 {
            let mut pen = 300.0 + k as f32 * 4500.0;
            let base = y0 + font.horizontal_line_metrics(px).map_or(px, |m| m.ascent);
            for ch in LINES[i * 2 + k].chars() {
                let (m, bitmap) = font.rasterize(ch, px);
                let gx = (pen + m.xmin as f32).round() as usize;
                let gy = (base - m.height as f32 - m.ymin as f32).round() as usize;
                for yy in 0..m.height {
                    for xx in 0..m.width {
                        let c = u32::from(bitmap[yy * m.width + xx]);
                        let p = &mut gray[(gy + yy) * w + gx + xx];
                        *p = ((u32::from(*p) * (255 - c) + 20 * c) / 255) as u8;
                    }
                }
                pen += m.advance_width;
            }
        }
        y0 += px * 3.0 + 400.0;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for g in gray {
        rgba.extend_from_slice(&[g, g, g, 255]);
    }
    let image = ImageBuf::new(w as u32, h as u32, PixelFormat::Rgba8, rgba).unwrap();
    println!("{w} x {h} synthetic photo, text from 18 to 72 pixels");
    for cap in [4000, 5000, 6000, 8000, 10_000] {
        let ocr = WindowsOcr::new().with_max_side(cap);
        let mut best = Duration::MAX;
        let mut found = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let layout = ocr.recognize(&image, &CancelToken::new()).unwrap();
            best = best.min(started.elapsed());
            found = layout.words().map(|(_, w)| w.text.to_lowercase()).collect();
        }
        let recall: Vec<String> = SIZES
            .iter()
            .enumerate()
            .map(|(i, px)| {
                let words: Vec<&str> = LINES[i * 2..i * 2 + 2]
                    .iter()
                    .flat_map(|l| l.split(' '))
                    .collect();
                let hit = words
                    .iter()
                    .filter(|w| found.iter().any(|f| f == *w))
                    .count();
                format!("{px} px {hit}/{}", words.len())
            })
            .collect();
        println!(
            "cap {cap:>5}: {:>4} ms, words found {}",
            best.as_millis(),
            recall.join(", ")
        );
    }
}

#[cfg(not(windows))]
fn main() {
    println!("no text recognizer on this platform");
}

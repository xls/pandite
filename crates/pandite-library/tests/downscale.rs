//! The thumbnail downscaler: the Rgba8 integer path at every instruction
//! set level against its scalar version, against the `f64` reference and
//! against an independent exact oracle; alpha weighting; and output sizes
//! over a sweep of odd sizes and ratios.

use pandite_core::{ImageBuf, PixelFormat};
use pandite_library::{box_downscale, box_downscale_reference, box_downscale_with};
use pandite_library::{fit_size, DownscaleLevel};

/// A deterministic pseudo-random sequence (xorshift32).
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

fn rgba8(w: u32, h: u32, mut f: impl FnMut(u32, u32) -> [u8; 4]) -> ImageBuf {
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&f(x, y));
        }
    }
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

/// The alpha patterns every random test runs: opaque; random alpha (zero
/// included); opaque rows with a translucent row now and then, so
/// destination rows switch between the plain and the premultiplied mode;
/// and translucent rows above opaque ones.
const ALPHAS: [&str; 4] = ["opaque", "random", "some rows", "top half"];

fn random_image(w: u32, h: u32, alpha: &str, rng: &mut Rng) -> ImageBuf {
    let translucent_row = rng.next() % 7;
    rgba8(w, h, |_, y| {
        let a = match alpha {
            "opaque" => 255,
            "random" => rng.byte(),
            "some rows" if y % 7 == translucent_row => rng.byte() | 1,
            "top half" if y < h / 2 => rng.byte(),
            _ => 255,
        };
        [rng.byte(), rng.byte(), rng.byte(), a]
    })
}

/// The exact area average, rounded to the nearest step with halves up,
/// computed per destination pixel from interval overlaps in `u128`: an
/// oracle that shares no code with the library.
fn exact(image: &ImageBuf, max_side: u32) -> Vec<u8> {
    let (sw, sh) = (image.width(), image.height());
    let (dw, dh) = fit_size(sw, sh, max_side);
    let (sw64, sh64, dw64, dh64) = (sw as u64, sh as u64, dw as u64, dh as u64);
    // On an axis of src pixels shrunk to dst: source i covers
    // [i * dst, (i + 1) * dst), destination j covers [j * src, (j + 1) * src).
    let overlap = |i: u64, j: u64, src: u64, dst: u64| {
        let lo = (i * dst).max(j * src);
        let hi = ((i + 1) * dst).min((j + 1) * src);
        hi.saturating_sub(lo)
    };
    let data = image.data();
    let mut out = Vec::with_capacity(dw as usize * dh as usize * 4);
    for j in 0..dh64 {
        let ys = (j * sh64 / dh64)..((j + 1) * sh64).div_ceil(dh64);
        for i in 0..dw64 {
            let xs = (i * sw64 / dw64)..((i + 1) * sw64).div_ceil(dw64);
            let (mut color, mut alpha) = ([0u128; 3], 0u128);
            for y in ys.clone() {
                let wy = overlap(y, j, sh64, dh64);
                for x in xs.clone() {
                    let w = u128::from(wy * overlap(x, i, sw64, dw64));
                    let p = &data[((y * sw64 + x) * 4) as usize..][..4];
                    let a = u128::from(p[3]);
                    for (c, &v) in color.iter_mut().zip(p) {
                        *c += u128::from(v) * a * w;
                    }
                    alpha += a * w;
                }
            }
            let round = |n: u128, d: u128| ((2 * n + d) / (2 * d)) as u8;
            for c in color {
                out.push(if alpha == 0 { 0 } else { round(c, alpha) });
            }
            out.push(round(alpha, u128::from(sw64 * sh64)));
        }
    }
    out
}

/// The largest difference of one channel between two byte buffers, and
/// how many channels differ.
fn deviation(a: &[u8], b: &[u8]) -> (u8, usize) {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).fold((0, 0), |(max, n), (x, y)| {
        (max.max(x.abs_diff(*y)), n + usize::from(x != y))
    })
}

/// Sizes and longest sides: width or height 1, primes, powers of two and
/// their neighbors, ratios just above 1 and above 100.
fn cases() -> Vec<(u32, u32, u32)> {
    let mut cases = Vec::new();
    let sizes: [(u32, u32); 22] = [
        (1, 1),
        (2, 1),
        (1, 2),
        (1, 97),
        (97, 1),
        (3, 2),
        (5, 3),
        (7, 7),
        (8, 9),
        (13, 11),
        (16, 16),
        (17, 15),
        (31, 33),
        (40, 41),
        (61, 59),
        (64, 48),
        (97, 89),
        (127, 129),
        (131, 7),
        (7, 131),
        (257, 255),
        (263, 101),
    ];
    for (w, h) in sizes {
        let long = w.max(h);
        for max_side in [1, 2, 3, 7, 16, 100, long.saturating_sub(1).max(1)] {
            cases.push((w, h, max_side));
        }
    }
    // Ratios above 100 on each axis and on both.
    cases.extend([
        (1013, 7, 9),
        (7, 1013, 9),
        (2003, 3, 2),
        (3, 2003, 2),
        (1021, 1019, 7),
        (509, 503, 5),
    ]);
    // Larger images with ratios just above 1 and in between.
    cases.extend([(1031, 769, 1030), (1031, 769, 256), (769, 1031, 255)]);
    cases
}

#[test]
fn downscale_vector_matches_scalar() {
    let levels = DownscaleLevel::available();
    let mut rng = Rng(0xD0_5CA1E);
    let mut compared = 0usize;
    for (w, h, max_side) in cases() {
        for alpha in ALPHAS {
            let image = random_image(w, h, alpha, &mut rng);
            let scalar = box_downscale_with(DownscaleLevel::Scalar, &image, max_side);
            for &level in &levels {
                let out = box_downscale_with(level, &image, max_side);
                assert_eq!(
                    (out.width(), out.height()),
                    (scalar.width(), scalar.height())
                );
                assert!(
                    out.data() == scalar.data(),
                    "{level:?} differs from scalar: {w}x{h} to {max_side}, alpha {alpha}"
                );
                compared += out.data().len() / 4;
            }
        }
    }
    println!("levels on this CPU: {levels:?}; {compared} output pixels compared with scalar");
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        assert!(levels.contains(&DownscaleLevel::Avx2), "AVX2 path tested");
    }
    #[cfg(target_arch = "aarch64")]
    assert!(levels.contains(&DownscaleLevel::Neon), "NEON path tested");
}

/// Adversarial images: flat extremes, checkerboards, single bright pixels,
/// exact halves, and the largest sums the 32-bit column sums must hold.
fn adversarial() -> Vec<(String, ImageBuf, u32)> {
    let mut out = Vec::new();
    for &(w, h, max_side) in &[(64, 48, 16), (101, 37, 10), (257, 255, 256), (9, 9, 2)] {
        let tag = |name: &str| format!("{name} {w}x{h} to {max_side}");
        out.push((tag("all 255"), rgba8(w, h, |_, _| [255; 4]), max_side));
        out.push((tag("all 0"), rgba8(w, h, |_, _| [0; 4]), max_side));
        out.push((
            tag("checkerboard"),
            rgba8(w, h, |x, y| {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                [v, v, v, 255]
            }),
            max_side,
        ));
        out.push((
            tag("alpha checkerboard"),
            rgba8(w, h, |x, y| {
                if (x + y) % 2 == 0 {
                    [255, 255, 255, 255]
                } else {
                    [0, 0, 0, 0]
                }
            }),
            max_side,
        ));
        out.push((
            tag("2x3 checkerboard"),
            rgba8(w, h, |x, y| {
                let v = if (x / 2 + y / 3) % 2 == 0 { 255 } else { 0 };
                [v, 255 - v, v, if v == 0 { 1 } else { 255 }]
            }),
            max_side,
        ));
        for (px, py) in [(0, 0), (w - 1, h - 1), (w / 2, h / 2), (w - 1, 0)] {
            out.push((
                tag(&format!("bright pixel at {px},{py}")),
                rgba8(w, h, |x, y| {
                    if (x, y) == (px, py) {
                        [255, 255, 255, 255]
                    } else {
                        [0, 0, 0, 255]
                    }
                }),
                max_side,
            ));
        }
        // Columns 0 and 255 in pairs: averages of exactly one half.
        out.push((
            tag("halves"),
            rgba8(w, h, |x, _| {
                let v = if x % 2 == 0 { 0 } else { 255 };
                [v, v, 255 - v, 255]
            }),
            max_side,
        ));
    }
    // The tallest image: every column sum near its 2^32 bound, plain and
    // premultiplied.
    out.push(("tall all 255".into(), rgba8(3, 65_535, |_, _| [255; 4]), 1));
    out.push((
        "tall translucent".into(),
        rgba8(3, 65_535, |x, y| {
            if (x, y) == (1, 7) {
                [255, 255, 255, 254]
            } else {
                [255; 4]
            }
        }),
        1,
    ));
    out.push((
        "widest".into(),
        rgba8(65_535, 2, |x, _| [x as u8, 255, 0, 255]),
        256,
    ));
    out
}

#[test]
fn downscale_within_one_of_reference() {
    let mut rng = Rng(0x0005_EED5);
    let mut worst = 0u8;
    let mut differing = 0usize;
    let mut samples = 0usize;
    let mut images: Vec<(String, ImageBuf, u32)> = adversarial();
    for (w, h, max_side) in cases().into_iter().step_by(3) {
        for alpha in ALPHAS {
            let image = random_image(w, h, alpha, &mut rng);
            images.push((
                format!("random {w}x{h} to {max_side}, {alpha}"),
                image,
                max_side,
            ));
        }
    }
    for (name, image, max_side) in &images {
        let reference = box_downscale_reference(image, *max_side);
        for level in DownscaleLevel::available() {
            let out = box_downscale_with(level, image, *max_side);
            assert_eq!(
                (out.width(), out.height()),
                (reference.width(), reference.height())
            );
            let (max, n) = deviation(out.data(), reference.data());
            assert!(max <= 1, "{level:?} {name}: {max} steps from the reference");
            worst = worst.max(max);
            differing += n;
            samples += out.data().len();
        }
    }
    // Flat images stay flat, exactly.
    for (name, image, max_side) in &images[..2] {
        let out = box_downscale(image, *max_side);
        let first = &out.data()[..4];
        assert!(out.data().chunks_exact(4).all(|p| p == first), "{name}");
    }
    println!(
        "{} images: largest difference from the f64 reference {worst} step, {differing} of {samples} samples differ",
        images.len()
    );
}

#[test]
fn downscale_is_the_exactly_rounded_average() {
    // The integer path equals an independent exact computation, rounded
    // to nearest with halves up, on every image the reference test uses
    // (the tallest and widest excepted: the oracle is slow).
    let mut rng = Rng(0x00C0_FFEE);
    let mut images = adversarial();
    images.truncate(images.len() - 3);
    for (w, h, max_side) in cases().into_iter().step_by(2) {
        for alpha in ALPHAS {
            images.push((
                format!("random {w}x{h} to {max_side}, {alpha}"),
                random_image(w, h, alpha, &mut rng),
                max_side,
            ));
        }
    }
    for (name, image, max_side) in &images {
        let want = exact(image, *max_side);
        let got = box_downscale(image, *max_side);
        if (got.width(), got.height()) == (image.width(), image.height()) {
            continue; // already fits: returned as is
        }
        assert!(got.data() == want.as_slice(), "{name}");
    }
}

#[test]
fn downscale_alpha_weighting_holds() {
    let levels = DownscaleLevel::available();
    // Transparent pixels, whatever their color bytes, do not darken or
    // tint the opaque pixels next to them: every output pixel that has
    // any coverage keeps the opaque color exactly.
    for &(w, h, max_side) in &[(10, 4, 3), (101, 37, 7), (33, 65, 5), (2, 1, 1)] {
        let image = rgba8(w, h, |x, y| {
            if (x * 3 + y) % 5 < 2 {
                [17, 250, 3, 0]
            } else {
                [200, 100, 50, 255]
            }
        });
        for &level in &levels {
            let out = box_downscale_with(level, &image, max_side);
            for px in out.data().chunks_exact(4) {
                assert!(px[3] > 0, "{level:?} {w}x{h}: coverage lost");
                assert_eq!(
                    &px[..3],
                    &[200, 100, 50],
                    "{level:?} {w}x{h}: color changed"
                );
            }
        }
    }
    // A fully transparent area becomes transparent black.
    let image = rgba8(
        40,
        40,
        |x, _| if x < 20 { [9, 8, 7, 0] } else { [1, 2, 3, 255] },
    );
    let out = box_downscale(&image, 4);
    assert_eq!(&out.data()[..4], &[0, 0, 0, 0]);
    assert_eq!(&out.data()[12..16], &[1, 2, 3, 255]);
    // The opaque fast path equals the general path on opaque pixels: an
    // opaque image, and the same image with one translucent pixel in the
    // last column of some or all rows, which sends those destination rows
    // through the premultiplied path. The last column only reaches the
    // last destination column, so every other column must match exactly.
    let mut rng = Rng(0xA1FA);
    for &(w, h, max_side) in &[(97, 89, 16), (257, 129, 256), (64, 64, 8), (100, 1000, 30)] {
        let image = random_image(w, h, "opaque", &mut rng);
        for every in [1u32, 5, h] {
            let marked = {
                let mut data = image.data().to_vec();
                for y in (0..h).filter(|y| y % every == every - 1) {
                    data[((y * w + w - 1) * 4 + 3) as usize] = 200;
                }
                ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
            };
            for &level in &levels {
                let plain = box_downscale_with(level, &image, max_side);
                let general = box_downscale_with(level, &marked, max_side);
                let dw = plain.width() as usize;
                assert!(dw >= 2);
                for (a, b) in plain
                    .data()
                    .chunks_exact(dw * 4)
                    .zip(general.data().chunks_exact(dw * 4))
                {
                    assert_eq!(
                        &a[..(dw - 1) * 4],
                        &b[..(dw - 1) * 4],
                        "{level:?} {w}x{h}, translucent every {every} rows"
                    );
                }
                assert!(plain.data().chunks_exact(4).all(|p| p[3] == 255));
            }
        }
    }
}

#[test]
fn downscale_odd_sizes_and_ratios() {
    let sizes = [
        1u32, 2, 3, 4, 5, 7, 8, 9, 16, 17, 31, 33, 64, 65, 100, 255, 256, 257,
    ];
    let sides = [0u32, 1, 2, 3, 5, 16, 100, 256];
    let mut rng = Rng(77);
    let mut count = 0usize;
    for &w in &sizes {
        for &h in &sizes {
            let image = random_image(w, h, "random", &mut rng);
            for &max_side in &sides {
                let want = fit_size(w, h, max_side);
                for level in DownscaleLevel::available() {
                    let out = box_downscale_with(level, &image, max_side);
                    assert_eq!((out.width(), out.height()), want, "{w}x{h} to {max_side}");
                    assert_eq!(out.format(), PixelFormat::Rgba8);
                    assert_eq!(out.data().len(), want.0 as usize * want.1 as usize * 4);
                    count += 1;
                }
            }
        }
    }
    // Deep and float images keep their format and take the same sizes.
    for &(w, h) in &[(1u32, 300u32), (300, 1), (17, 33), (257, 255)] {
        for format in [PixelFormat::Rgba16, PixelFormat::RgbaF32] {
            let len = w as usize * h as usize * format.bytes_per_pixel();
            let image = ImageBuf::new(w, h, format, vec![0x3C; len]).unwrap();
            for &max_side in &sides {
                let out = box_downscale(&image, max_side);
                assert_eq!((out.width(), out.height()), fit_size(w, h, max_side));
                assert_eq!(out.format(), format);
            }
        }
    }
    println!("{count} Rgba8 downscales checked");
}

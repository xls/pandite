//! Gate G1: resize sizes, filtering quality, alpha and pixel formats.

use pandite_core::{Error, ImageBuf, PixelFormat};
use pandite_ops::{resize, resize_to_spec, srgb_to_linear, Filter, ResizeSpec};

fn rgba8(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> ImageBuf {
    let data = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| f(x, y))
        .collect();
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

fn rgba16(w: u32, h: u32, f: impl Fn(u32, u32) -> [u16; 4]) -> ImageBuf {
    let data = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| f(x, y))
        .flat_map(u16::to_ne_bytes)
        .collect();
    ImageBuf::new(w, h, PixelFormat::Rgba16, data).unwrap()
}

fn rgbaf32(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 4]) -> ImageBuf {
    let data = (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| f(x, y))
        .flat_map(f32::to_ne_bytes)
        .collect();
    ImageBuf::new(w, h, PixelFormat::RgbaF32, data).unwrap()
}

fn px8(img: &ImageBuf) -> Vec<[u8; 4]> {
    assert_eq!(img.format(), PixelFormat::Rgba8);
    img.data()
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

fn px16(img: &ImageBuf) -> Vec<[u16; 4]> {
    assert_eq!(img.format(), PixelFormat::Rgba16);
    img.data()
        .chunks_exact(8)
        .map(|p| std::array::from_fn(|c| u16::from_ne_bytes([p[2 * c], p[2 * c + 1]])))
        .collect()
}

fn pxf32(img: &ImageBuf) -> Vec<[f32; 4]> {
    assert_eq!(img.format(), PixelFormat::RgbaF32);
    img.data()
        .chunks_exact(16)
        .map(|p| {
            std::array::from_fn(|c| f32::from_ne_bytes(p[4 * c..4 * c + 4].try_into().unwrap()))
        })
        .collect()
}

fn checker(x: u32, y: u32) -> bool {
    (x + y).is_multiple_of(2)
}

/// sRGB code of linear 0.5: the true average brightness of black and white.
const MID_GRAY: u8 = 188;

const SMOOTH_FILTERS: [Filter; 5] = [
    Filter::Box,
    Filter::Triangle,
    Filter::CatmullRom,
    Filter::Mitchell,
    Filter::Lanczos3,
];

#[test]
fn resize_percent() {
    // Horizontal ramp, 40 x 30.
    let ramp = rgba8(40, 30, |x, _| [(x * 6) as u8, 100, 50, 255]);
    let half = resize_to_spec(&ramp, &ResizeSpec::Percent(50.0), Filter::Lanczos3).unwrap();
    assert_eq!((half.width(), half.height()), (20, 15));
    assert_eq!(half.format(), PixelFormat::Rgba8);
    let pixels = px8(&half);
    // Interior pixel i averages source pixels 2i and 2i + 1 of the ramp.
    for (i, p) in pixels.iter().take(20).enumerate().skip(2).take(16) {
        let expect = (12 * i + 3) as i32;
        assert!((i32::from(p[0]) - expect).abs() <= 2, "x={i}: {p:?}");
        assert_eq!(&p[1..], &[100, 50, 255]);
    }
    // Every row is the same and the ramp still rises.
    for row in pixels.chunks_exact(20) {
        assert_eq!(row, &pixels[..20]);
        assert!(row.windows(2).all(|w| w[0][0] <= w[1][0]), "{row:?}");
    }
    let bigger = resize_to_spec(&ramp, &ResizeSpec::Percent(150.0), Filter::Lanczos3).unwrap();
    assert_eq!((bigger.width(), bigger.height()), (60, 45));
    // Rounding to the nearest pixel: 40 * 0.333 = 13.3, 30 * 0.333 = 9.99.
    assert_eq!(ResizeSpec::Percent(33.3).resolve(40, 30).unwrap(), (13, 10));
    // A solid color survives every filter exactly.
    let solid = ImageBuf::solid_rgba8(40, 30, [12, 200, 77, 255]).unwrap();
    for filter in Filter::ALL {
        let out = resize_to_spec(&solid, &ResizeSpec::Percent(37.0), filter).unwrap();
        assert_eq!((out.width(), out.height()), (15, 11));
        assert!(
            px8(&out).iter().all(|p| *p == [12, 200, 77, 255]),
            "{filter:?}"
        );
    }
}

#[test]
fn resize_best_fit_keeps_aspect() {
    let fit = |w: u32, h: u32, enlarge: bool| ResizeSpec::Fit {
        width: w,
        height: h,
        enlarge,
    };
    let wide = ImageBuf::solid_rgba8(400, 100, [1, 2, 3, 255]).unwrap();
    let out = resize_to_spec(&wide, &fit(100, 100, false), Filter::Lanczos3).unwrap();
    assert_eq!((out.width(), out.height()), (100, 25));
    let tall = ImageBuf::solid_rgba8(100, 400, [1, 2, 3, 255]).unwrap();
    let out = resize_to_spec(&tall, &fit(100, 100, false), Filter::Lanczos3).unwrap();
    assert_eq!((out.width(), out.height()), (25, 100));
    assert_eq!(fit(100, 100, false).resolve(333, 100).unwrap(), (100, 30));

    // Small images stay as they are unless enlarging is allowed.
    let small = ImageBuf::solid_rgba8(50, 20, [9, 9, 9, 255]).unwrap();
    let kept = resize_to_spec(&small, &fit(100, 100, false), Filter::Lanczos3).unwrap();
    assert_eq!((kept.width(), kept.height()), (50, 20));
    assert_eq!(kept.data(), small.data());
    let grown = resize_to_spec(&small, &fit(100, 100, true), Filter::Lanczos3).unwrap();
    assert_eq!((grown.width(), grown.height()), (100, 40));

    // Over many shapes: inside the box, touching it, aspect kept to rounding.
    for (sw, sh) in [
        (640, 480),
        (480, 640),
        (1, 1000),
        (1000, 1),
        (37, 23),
        (300, 300),
    ] {
        for (bw, bh) in [(100, 100), (64, 200), (500, 90), (1, 1)] {
            for enlarge in [false, true] {
                let (w, h) = fit(bw, bh, enlarge).resolve(sw, sh).unwrap();
                assert!(w >= 1 && h >= 1);
                let fits = sw <= bw && sh <= bh;
                if fits && !enlarge {
                    assert_eq!((w, h), (sw, sh));
                    continue;
                }
                assert!(w <= bw && h <= bh, "{sw}x{sh} in {bw}x{bh}: {w}x{h}");
                assert!(w == bw || h == bh, "{sw}x{sh} in {bw}x{bh}: {w}x{h}");
                // The free side is within half a pixel of the exact aspect,
                // unless clamped to the 1-pixel minimum.
                let exact_w = f64::from(h) * f64::from(sw) / f64::from(sh);
                let exact_h = f64::from(w) * f64::from(sh) / f64::from(sw);
                let ok_w = (f64::from(w) - exact_w).abs() <= 0.5 || w == 1;
                let ok_h = (f64::from(h) - exact_h).abs() <= 0.5 || h == 1;
                assert!(ok_w || ok_h, "{sw}x{sh} in {bw}x{bh}: {w}x{h}");
            }
        }
    }
}

#[test]
fn resize_exact() {
    let solid = ImageBuf::solid_rgba8(7, 5, [250, 128, 3, 200]).unwrap();
    for filter in Filter::ALL {
        for (w, h) in [(13, 3), (1, 1), (7, 9), (2, 40)] {
            let out = resize(&solid, w, h, filter).unwrap();
            assert_eq!((out.width(), out.height()), (w, h), "{filter:?}");
            assert!(
                px8(&out).iter().all(|p| *p == [250, 128, 3, 200]),
                "{filter:?}"
            );
        }
    }
    let spec = ResizeSpec::Exact {
        width: 3,
        height: 11,
    };
    let out = resize_to_spec(&solid, &spec, Filter::Lanczos3).unwrap();
    assert_eq!((out.width(), out.height()), (3, 11));
    // Black and white average to linear 0.5, which is sRGB 188, not 128.
    let pair = rgba8(2, 1, |x, _| if x == 0 { [0, 0, 0, 255] } else { [255; 4] });
    for filter in SMOOTH_FILTERS {
        let dot = resize(&pair, 1, 1, filter).unwrap();
        assert_eq!(
            px8(&dot)[0],
            [MID_GRAY, MID_GRAY, MID_GRAY, 255],
            "{filter:?}"
        );
    }
    // Growing a 1-pixel image fills the area.
    let one = ImageBuf::solid_rgba8(1, 1, [5, 6, 7, 8]).unwrap();
    let grown = resize(&one, 9, 4, Filter::Lanczos3).unwrap();
    assert!(px8(&grown).iter().all(|p| *p == [5, 6, 7, 8]));
}

#[test]
fn resize_scale_factor() {
    // 4 x 4 blocks of distinct colors: a quarter-size box resize must give
    // exactly one pixel per block, in place.
    let color = |bx: u32, by: u32| [(bx * 40) as u8, (by * 50) as u8, 99, 255];
    let blocks = rgba8(64, 48, |x, y| color(x / 4, y / 4));
    let quarter = resize_to_spec(&blocks, &ResizeSpec::Scale(0.25), Filter::Box).unwrap();
    assert_eq!((quarter.width(), quarter.height()), (16, 12));
    for (i, p) in px8(&quarter).iter().enumerate() {
        let (bx, by) = (i as u32 % 16, i as u32 / 16);
        assert_eq!(*p, color(bx, by), "block {bx},{by}");
    }
    // Doubling with nearest neighbor gives 2 x 2 copies of each pixel.
    let small = rgba8(5, 3, |x, y| [x as u8, y as u8, 7, 255]);
    let double = resize_to_spec(&small, &ResizeSpec::Scale(2.0), Filter::Nearest).unwrap();
    assert_eq!((double.width(), double.height()), (10, 6));
    for (i, p) in px8(&double).iter().enumerate() {
        let (x, y) = (i as u32 % 10, i as u32 / 10);
        assert_eq!(*p, [(x / 2) as u8, (y / 2) as u8, 7, 255]);
    }
    assert_eq!(
        ResizeSpec::Scale(1.0 / 3.0).resolve(10, 10).unwrap(),
        (3, 3)
    );
    assert_eq!(ResizeSpec::Scale(0.5).resolve(5, 3).unwrap(), (3, 2));
    assert_eq!(ResizeSpec::Scale(1e-6).resolve(5, 3).unwrap(), (1, 1));
    let grown = resize_to_spec(&small, &ResizeSpec::Scale(3.5), Filter::CatmullRom).unwrap();
    assert_eq!((grown.width(), grown.height()), (18, 11));
}

#[test]
fn downscale_checkerboard_is_flat_gray() {
    let board = rgba8(256, 256, |x, y| {
        if checker(x, y) {
            [255; 4]
        } else {
            [0, 0, 0, 255]
        }
    });
    let spread = |out: &ImageBuf| -> u8 {
        px8(out)
            .iter()
            .map(|p| {
                assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 255, "{p:?}");
                p[0].abs_diff(MID_GRAY)
            })
            .max()
            .unwrap()
    };
    // Integer and awkward ratios, square and stretched targets, edges and
    // corners included: the default and the cubic filters give one flat
    // gray, the tent filter is within 1 code.
    let sizes = [
        (128, 128),
        (37, 37),
        (100, 51),
        (64, 200),
        (13, 7),
        (129, 129),
    ];
    for (w, h) in sizes {
        for filter in [Filter::Lanczos3, Filter::CatmullRom, Filter::Mitchell] {
            let out = resize(&board, w, h, filter).unwrap();
            assert_eq!(spread(&out), 0, "{filter:?} {w}x{h}");
        }
        let out = resize(&board, w, h, Filter::Triangle).unwrap();
        assert!(spread(&out) <= 1, "Triangle {w}x{h}");
        // A box is an exact area average; at fractional ratios its window
        // covers unequal amounts of black and white, a faint residue.
        let out = resize(&board, w, h, Filter::Box).unwrap();
        assert!(spread(&out) <= 4, "Box {w}x{h}");
    }
    // An integer box average hits the value precisely.
    for (w, h) in [(128, 128), (64, 64), (32, 128)] {
        assert_eq!(spread(&resize(&board, w, h, Filter::Box).unwrap()), 0);
    }
    // A mild 1.28x reduction sits in Lanczos3's transition band: small beat.
    assert!(spread(&resize(&board, 200, 200, Filter::Lanczos3).unwrap()) <= 3);
    // Control: nearest neighbor aliases the same input into black and white.
    let out = resize(&board, 37, 37, Filter::Nearest).unwrap();
    let values: Vec<u8> = px8(&out).iter().map(|p| p[0]).collect();
    assert!(values.contains(&0) && values.contains(&255));
}

#[test]
fn resize_alpha_edges_do_not_fringe() {
    // Left half opaque red, right half fully transparent green. Without
    // premultiplied alpha the hidden green bleeds into the edge.
    let (w, h) = (32, 8);
    let red = |x: u32| x < w / 2;
    let img8 = rgba8(w, h, |x, _| {
        if red(x) {
            [255, 0, 0, 255]
        } else {
            [0, 255, 0, 0]
        }
    });
    let img16 = rgba16(w, h, |x, _| {
        if red(x) {
            [65535, 0, 0, 65535]
        } else {
            [0, 65535, 0, 0]
        }
    });
    let imgf = rgbaf32(w, h, |x, _| {
        if red(x) {
            [1.0, 0.0, 0.0, 1.0]
        } else {
            [0.0, 1.0, 0.0, 0.0]
        }
    });
    for filter in SMOOTH_FILTERS {
        for (tw, th) in [(11, 3), (50, 12), (7, 8)] {
            let out = resize(&img8, tw, th, filter).unwrap();
            let mut partial = 0;
            for p in px8(&out) {
                if p[3] > 0 {
                    assert!(p[0] >= 254 && p[1] <= 1 && p[2] <= 1, "{filter:?}: {p:?}");
                }
                if p[3] > 0 && p[3] < 255 {
                    partial += 1;
                }
            }
            if filter != Filter::Box || tw != 50 {
                assert!(partial > 0, "{filter:?} {tw}x{th}: no edge pixels to check");
            }

            let out = resize(&img16, tw, th, filter).unwrap();
            for p in px16(&out) {
                if p[3] > 0 {
                    assert!(
                        p[0] >= 65_000 && p[1] <= 300 && p[2] <= 300,
                        "{filter:?}: {p:?}"
                    );
                }
            }

            let out = resize(&imgf, tw, th, filter).unwrap();
            for p in pxf32(&out) {
                assert!((0.0..=1.0).contains(&p[3]));
                if p[3] > 1e-3 {
                    assert!((p[0] - 1.0).abs() < 1e-3 && p[1].abs() < 1e-3 && p[2].abs() < 1e-3);
                }
            }
        }
    }
    // Half-covered output pixels are half transparent, not dark.
    let out = resize(&img8, 2, 1, Filter::Box).unwrap();
    assert_eq!(px8(&out), vec![[255, 0, 0, 255], [0, 0, 0, 0]]);
    let out = resize(&img8, 1, 1, Filter::Box).unwrap();
    assert_eq!(px8(&out), vec![[255, 0, 0, 128]]);
}

#[test]
fn resize_rgba16_and_f32() {
    // 16-bit: checkerboard averages to linear 0.5 (sRGB 0.7354 = 48193).
    let board16 = rgba16(64, 64, |x, y| {
        if checker(x, y) {
            [65535; 4]
        } else {
            [0, 0, 0, 65535]
        }
    });
    let half = resize(&board16, 32, 32, Filter::Box).unwrap();
    assert_eq!(half.format(), PixelFormat::Rgba16);
    for p in px16(&half) {
        assert!(
            p[0].abs_diff(48_193) <= 1 && p[0] == p[1] && p[1] == p[2] && p[3] == 65535,
            "{p:?}"
        );
    }
    let odd = resize(&board16, 37, 23, Filter::Lanczos3).unwrap();
    for p in px16(&odd) {
        assert!(p[0].abs_diff(48_193) <= 300, "{p:?}");
    }
    // 16-bit precision is kept: a fine ramp halves to values between the
    // 8-bit steps.
    let ramp16 = rgba16(512, 2, |x, _| [20_000 + x as u16, 0, 0, 65535]);
    let halved = resize(&ramp16, 256, 1, Filter::Box).unwrap();
    let reds: Vec<u16> = px16(&halved).iter().map(|p| p[0]).collect();
    assert!(reds.iter().any(|v| v % 257 != 0));
    assert!(reds.windows(2).all(|w| w[0] < w[1]));
    let solid16 = rgba16(10, 7, |_, _| [1000, 30_000, 65535, 40_000]);
    for filter in Filter::ALL {
        let out = resize(&solid16, 23, 4, filter).unwrap();
        for p in px16(&out) {
            for (got, want) in p.iter().zip([1000u16, 30_000, 65535, 40_000]) {
                assert!(got.abs_diff(want) <= 1, "{filter:?}: {p:?}");
            }
        }
    }

    // Float: values are linear, so the checkerboard averages to 0.5.
    let boardf = rgbaf32(64, 64, |x, y| {
        if checker(x, y) {
            [1.0; 4]
        } else {
            [0.0, 0.0, 0.0, 1.0]
        }
    });
    let half = resize(&boardf, 32, 32, Filter::Box).unwrap();
    assert_eq!(half.format(), PixelFormat::RgbaF32);
    assert!(pxf32(&half)
        .iter()
        .all(|p| (p[0] - 0.5).abs() < 1e-6 && (p[3] - 1.0).abs() < 1e-6));
    let odd = resize(&boardf, 37, 23, Filter::Lanczos3).unwrap();
    assert!(pxf32(&odd).iter().all(|p| (p[0] - 0.5).abs() < 2e-3));
    // High dynamic range values pass through unclamped.
    let hdr = rgbaf32(9, 9, |_, _| [4.0, 0.25, 12.5, 1.0]);
    for filter in Filter::ALL {
        let out = resize(&hdr, 5, 14, filter).unwrap();
        for p in pxf32(&out) {
            assert!(
                (p[0] - 4.0).abs() < 1e-4 && (p[1] - 0.25).abs() < 1e-5,
                "{p:?}"
            );
            assert!(
                (p[2] - 12.5).abs() < 1e-3 && (p[3] - 1.0).abs() < 1e-6,
                "{p:?}"
            );
        }
    }
    // An 8-bit image filtered in linear light matches the float result.
    // Premultiplied: the color is weighted by alpha, (64, a 63) and (192, a 255).
    let gray8 = rgba8(2, 1, |x, _| {
        if x == 0 {
            [64, 64, 64, 63]
        } else {
            [192, 192, 192, 255]
        }
    });
    let a0 = 63.0 / 255.0;
    let lin = (srgb_to_linear(64.0 / 255.0) * a0 + srgb_to_linear(192.0 / 255.0)) / (a0 + 1.0);
    let out = px8(&resize(&gray8, 1, 1, Filter::Box).unwrap())[0];
    let lin_out = srgb_to_linear(f32::from(out[0]) / 255.0);
    assert!((lin_out - lin).abs() < 0.004, "{out:?} vs {lin}");
    assert_eq!(out[3], 159);
}

#[test]
fn resize_rejects_zero() {
    let img = ImageBuf::solid_rgba8(10, 10, [0; 4]).unwrap();
    for (w, h) in [(0, 5), (5, 0), (0, 0)] {
        assert!(matches!(
            resize(&img, w, h, Filter::Lanczos3),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            resize(&img, w, h, Filter::Nearest),
            Err(Error::Invalid(_))
        ));
    }
    let zero_specs = [
        ResizeSpec::Percent(0.0),
        ResizeSpec::Percent(-10.0),
        ResizeSpec::Percent(f64::NAN),
        ResizeSpec::Scale(0.0),
        ResizeSpec::Scale(-1.0),
        ResizeSpec::Scale(f64::INFINITY),
        ResizeSpec::Fit {
            width: 0,
            height: 10,
            enlarge: true,
        },
        ResizeSpec::Fit {
            width: 10,
            height: 0,
            enlarge: false,
        },
        ResizeSpec::Exact {
            width: 0,
            height: 3,
        },
        ResizeSpec::Exact {
            width: 3,
            height: 0,
        },
    ];
    for spec in zero_specs {
        assert!(
            matches!(spec.resolve(10, 10), Err(Error::Invalid(_))),
            "{spec:?}"
        );
        assert!(matches!(
            resize_to_spec(&img, &spec, Filter::Box),
            Err(Error::Invalid(_))
        ));
    }
    // Too large is a different error, not a panic or an allocation.
    assert!(matches!(
        resize(&img, 70_000, 10, Filter::Lanczos3),
        Err(Error::TooLarge { .. })
    ));
    assert!(matches!(
        ResizeSpec::Percent(1e9).resolve(10, 10),
        Err(Error::TooLarge { .. })
    ));
}

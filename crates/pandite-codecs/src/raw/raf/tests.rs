//! Tests of the RAF decoder against the specification's test vectors
//! (TV-1 to TV-13), fixtures (F1 to F4) and requirements, plus hostile
//! input, cancellation and real camera files.

use super::container::{self, Class, Layout, Plan, Relative, Variant, WbSource};
use super::fixtures::*;
use super::*;
use crate::raw::arw::fixtures::{hex, jpeg};
use crate::raw::arw::render::{map, RowEncoder};
use crate::raw::arw::strips;
use crate::raw::arw::tests::{agreement, grid_means, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The specification prints 7 or 8 decimals and rounds its intermediates;
/// a match within 1e-6 is a match at the printed precision.
const EPS: f32 = 1e-6;

/// F1, F2 and F3: the expected linear sRGB of every output pixel.
const F1_RGB: [f32; 3] = [0.273_579_1, 0.330_361_5, 0.057_070_4];
const F2_RGB: [f32; 3] = [0.423_236_2, 0.432_453_4, 0.259_487_1];
const F3_RGB: [f32; 3] = [0.220_725_8, 0.336_111_4, 0.062_111_5];

fn raf(data: &[u8]) -> container::Raf<'_> {
    container::open(data).expect("open").expect("a RAF file")
}

fn class_of(data: &[u8]) -> Class {
    container::classify(&raf(data)).expect("classify")
}

fn plan_of(data: &[u8]) -> Plan {
    let r = raf(data);
    let o = preview::collect(&r).orientation;
    let a = container::analyze(&r).expect("analyze");
    container::plan(&r, &a.tags.expect("a sensor variant"), o).expect("plan")
}

/// The decoded raster as row-major W x H values.
fn values_of(data: &[u8]) -> (usize, usize, Vec<u16>) {
    let plan = plan_of(data);
    let p = sensor::decode(data, &plan, &CancelToken::new()).expect("decode sensor");
    (plan.width, plan.height, p.data)
}

/// Delivers the linear sRGB values themselves, 12 bytes per pixel.
struct LinearF32;

impl RowEncoder for LinearF32 {
    fn pixel_bytes(&self) -> usize {
        12
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        for (k, b) in out.chunks_exact_mut(12).enumerate() {
            let v = crate::raw::arw::render::to_srgb(m, [cam[0][k], cam[1][k], cam[2][k]]);
            for c in 0..3 {
                b[4 * c..4 * c + 4].copy_from_slice(&v[c].to_ne_bytes());
            }
        }
    }
}

/// Render a plan to linear sRGB values (R-RAF-38), oriented and cropped.
fn linear_with(data: &[u8], plan: &Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let p = sensor::decode(data, plan, &CancelToken::new()).expect("decode sensor");
    let bytes = render::render(plan, &p, &CancelToken::new(), &LinearF32).expect("render");
    let (w, h) = output_size(plan.crop, plan.orientation);
    let px = bytes
        .chunks_exact(12)
        .map(|c| {
            let f =
                |k: usize| f32::from_ne_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]);
            [f(0), f(1), f(2)]
        })
        .collect();
    (w, h, px)
}

fn linear(data: &[u8]) -> (usize, usize, Vec<[f32; 3]>) {
    linear_with(data, &plan_of(data))
}

/// Camera RGB after demosaicing: the plan with the identity matrix.
fn camera_rgb(data: &[u8]) -> (usize, usize, Vec<[f32; 3]>) {
    let mut plan = plan_of(data);
    plan.matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    linear_with(data, &plan)
}

fn report(data: &[u8]) -> Developed {
    decode_report(data, &DecodeOptions::default())
        .expect("decode")
        .expect("RAF path")
}

fn assert_rgb(got: [f32; 3], want: [f32; 3], eps: f32, what: &str) {
    for c in 0..3 {
        assert!(
            (got[c] - want[c]).abs() <= eps,
            "{what}: channel {c} is {} want {} ({got:?})",
            got[c],
            want[c]
        );
    }
}

/// Every pixel of a developed Rgba16 image is the sRGB encoding of `lin`
/// within 2 codes of 65535.
fn assert_rgba16_flat(img: &ImageBuf, lin: [f32; 3], what: &str) {
    assert_eq!(img.format(), PixelFormat::Rgba16, "{what}");
    for p in img.data().chunks_exact(8) {
        for c in 0..3 {
            let v = u16::from_ne_bytes([p[2 * c], p[2 * c + 1]]);
            let want = (crate::util::linear_to_srgb(lin[c]) * 65535.0 + 0.5) as u16;
            assert!(v.abs_diff(want) <= 2, "{what}: channel {c} code {v}");
        }
    }
}

/// A deterministic pseudo-random sequence.
fn noise(seed: u32) -> impl FnMut() -> u32 {
    let mut s = seed.max(1);
    move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    }
}

/// A V3 file of `w` x `h` 14-bit values with black 0, unit white balance
/// and the whole raster as the crop, so normalized values are s / 16383.
fn unit_v3(w: u32, h: u32, layout: &[u8], values: &[u16]) -> Vec<u8> {
    Builder::new(
        w,
        h,
        14,
        layout,
        (0, 0, w as u16, h as u16),
        vec![0; if layout.is_empty() { 4 } else { 36 }],
        [256, 256, 256],
        1,
        pack_v3(values),
    )
    .build()
}

// ---------------------------------------------------------------------------
// Test vectors and fixtures
// ---------------------------------------------------------------------------

/// TV-1 to TV-3 written at their offsets into a file of the X-T3 sample's
/// size; everything else is zero.
fn xt3_skeleton() -> Vec<u8> {
    let mut f = vec![0u8; 56_087_360];
    f[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
    f[16..20].copy_from_slice(b"0201");
    let tv1 = hex("00 00 00 94 00 28 B2 DD 00 28 B3 A8 00 00 56 58 00 29 0A 00 03 2E C9 40 00 00 00 00 03 2E C9 40");
    f[0x54..0x74].copy_from_slice(&tv1);
    let mut meta = hex("00 00 00 0B 01 00 00 04 10 56 18 F0 01 10 00 04 00 0D 00 06 01 11 00 04 10 40 18 60 01 30 00 04 0C 0C 0C 0C 01 31 00 24");
    meta.extend_from_slice(&XT3);
    f[2_667_432..2_667_432 + meta.len()].copy_from_slice(&meta);
    let c = 2_689_536;
    let head =
        hex("49 49 2A 00 08 00 00 00 01 00 00 F0 0D 00 01 00 00 00 1A 00 00 00 00 00 00 00 12 00");
    f[c..c + head.len()].copy_from_slice(&head);
    let entries = hex("01 F0 04 00 01 00 00 00 F0 18 00 00
         02 F0 04 00 01 00 00 00 56 10 00 00
         03 F0 04 00 01 00 00 00 0E 00 00 00
         07 F0 04 00 01 00 00 00 00 08 00 00
         08 F0 04 00 01 00 00 00 40 C1 2E 03
         0A F0 04 00 24 00 00 00 F8 00 00 00
         0E F0 04 00 03 00 00 00 4C 02 00 00");
    f[c + 28..c + 28 + entries.len()].copy_from_slice(&entries);
    for i in 0..36 {
        f[c + 248 + 4 * i..c + 252 + 4 * i].copy_from_slice(&hex("FE 03 00 00"));
    }
    f[c + 588..c + 600].copy_from_slice(&hex("2E 01 00 00 47 02 00 00 48 02 00 00"));
    f
}

#[test]
fn raf_vectors_decode() {
    // TV-1 (R-RAF-5, R-RAF-6), TV-2 (R-RAF-7) and TV-3 (R-RAF-9 to R-RAF-11)
    // on a skeleton of the X-T3 sample.
    let sk = xt3_skeleton();
    let r = raf(&sk);
    let part = |offset, len| Some(container::Part { offset, len });
    assert_eq!(r.jpeg, part(148, 2_667_229));
    assert_eq!(r.meta, part(2_667_432, 22_104));
    assert_eq!(r.cfa, part(2_689_536, 53_397_824));
    assert_eq!((r.compression, r.second_cfa), (0, 0));
    assert_eq!(class_of(&sk), Class::Sensor(Variant::V3, Layout::XTrans));
    let records = container::records(&r).unwrap();
    assert_eq!(records.get(0x0100), Some(&hex("10 56 18 F0")[..]));
    let p = plan_of(&sk);
    assert_eq!((p.width, p.height, p.bits), (6384, 4182, 14));
    // TV-2: crop x 6 to 6245, y 13 to 4172.
    assert_eq!(
        p.crop,
        Rect {
            x: 6,
            y: 13,
            w: 6240,
            h: 4160
        }
    );
    // TV-3: the strip starts at 2689536 + 2048; R = 2W.
    assert_eq!((p.strip, p.row_bytes), (2_691_584, 2 * 6384));
    assert_eq!((p.black, p.white), (1022.0, 16383));
    drop(sk);

    // TV-4: the X-T3 and X-T1 patterns by R-RAF-18.
    let names = |pat: &[u8; 36], y: usize| -> String {
        (0..6)
            .map(|x| ['R', 'G', 'B'][usize::from(pat[6 * y + x])])
            .collect()
    };
    let xt3 = pattern_of(&XT3);
    let rows: Vec<String> = (0..6).map(|y| names(&xt3, y)).collect();
    assert_eq!(
        rows,
        ["GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB", "RBGBRG"]
    );
    let xt1 = pattern_of(&XT1);
    let rows: Vec<String> = (0..6).map(|y| names(&xt1, y)).collect();
    assert_eq!(
        rows,
        ["RBGBRG", "GGRGGB", "GGBGGR", "BRGRBG", "GGBGGR", "GGRGGB"]
    );
    // The forward reading would give G R B G B R for row 0 of the X-T3.
    let forward: String = XT3[..6]
        .iter()
        .map(|&c| ['R', 'G', 'B'][usize::from(c)])
        .collect();
    assert_eq!(forward, "GRBGBR");
    assert_ne!(forward, names(&xt3, 0));
    // The same through the meta container (R-RAF-17, R-RAF-18).
    let fx = f1();
    let (layout, pattern) = container::layout(&container::records(&raf(&fx)).unwrap()).unwrap();
    assert_eq!((layout, pattern), (Layout::XTrans, xt3));

    // TV-5 (R-RAF-27), TV-6 (R-RAF-23), TV-7 (R-RAF-25).
    let mut out = [0u16; 6];
    strips::v1_row(&hex("2B 06 A7 06 E5 08 5F 06 AE 05 F0 07"), &mut out);
    assert_eq!(out, [1579, 1703, 2277, 1631, 1454, 2032]);
    sensor::v1_row(&hex("1D D6 61 AD 13 60 26 A6 43"), &mut out);
    assert_eq!(out, [1565, 1565, 941, 1537, 1574, 1082]);
    let mut out4 = [0u16; 4];
    sensor::v1_row(&hex("A7 21 19 BF D1 19"), &mut out4);
    assert_eq!(out4, [423, 402, 447, 413]);
    let mut out16 = [0u16; 16];
    sensor::v2_row(
        &hex("92 2C 6D 2B 2D B9 D2 BD B8 22 30 41 75 2A B5 51 11 9A F2 17 1D 69 29 4D 63 D1 AE D2"),
        &mut out16,
    );
    assert_eq!(
        out16,
        [
            2779, 4809, 2807, 4793, 2896, 4866, 2785, 4533, 2717, 4479, 2664, 4429, 2650, 4573,
            2747, 4451
        ]
    );
    // The packers of the fixtures invert the decoders.
    assert_eq!(
        pack_v2(&out16, 16),
        hex("92 2C 6D 2B 2D B9 D2 BD B8 22 30 41 75 2A B5 51 11 9A F2 17 1D 69 29 4D 63 D1 AE D2")
    );
    assert_eq!(
        pack_v1(&[1565, 1565, 941, 1537, 1574, 1082]),
        hex("1D D6 61 AD 13 60 26 A6 43")
    );

    // TV-9: X-Trans demosaicing (R-RAF-39, R-RAF-40) on a 6 x 6 raster
    // with the X-T3 pattern, the whole raster as the crop.
    #[rustfmt::skip]
    let tv9: [u16; 36] = [
        0, 7, 14, 21, 5, 12,
        11, 21, 8, 18, 5, 15,
        22, 12, 2, 15, 5, 18,
        10, 3, 19, 12, 5, 21,
        21, 17, 13, 9, 5, 1,
        9, 8, 7, 6, 5, 4,
    ];
    let f = unit_v3(6, 6, &XT3, &tv9);
    assert_eq!(class_of(&f), Class::Sensor(Variant::V3, Layout::XTrans));
    let (w, _, px) = camera_rgb(&f);
    let k = 16383.0f32;
    for ((x, y), want) in [
        ((1, 2), [12.0, 9.4, 49.0 / 3.0]),
        ((2, 2), [13.5, 2.0, 13.5]),
        ((0, 2), [12.0, 11.25, 22.0]),
        ((3, 3), [14.0, 12.0, 12.0]),
        ((0, 0), [13.0, 0.0, 15.0]),
    ] {
        let got = px[y * w + x].map(|v| v * k);
        assert_rgb(got, want, 2e-3, &format!("TV-9 ({x}, {y})"));
    }

    // TV-10: Bayer demosaicing on a 4 x 4 raster.
    let tv10: [u16; 16] = [0, 5, 10, 15, 9, 16, 4, 11, 18, 8, 17, 7, 8, 0, 11, 3];
    let f = unit_v3(4, 4, &[], &tv10);
    assert_eq!(class_of(&f), Class::Sensor(Variant::V3, Layout::Bayer));
    let (w, _, px) = camera_rgb(&f);
    for ((x, y), want) in [
        ((1, 1), [11.25, 6.5, 16.0]),
        ((2, 1), [13.5, 4.0, 13.5]),
        ((1, 2), [17.5, 8.0, 8.0]),
        ((0, 0), [0.0, 7.0, 16.0]),
    ] {
        let got = px[y * w + x].map(|v| v * k);
        assert_rgb(got, want, 2e-3, &format!("TV-10 ({x}, {y})"));
    }

    // TV-12: the orientation table (R-RAF-45, R-RAF-46) on a 3 x 2 image.
    let tv12 = |o: u8| -> Vec<char> {
        let (w, h) = if o >= 5 { (2, 3) } else { (3, 2) };
        let mut out = vec![' '; 6];
        for (i, name) in ['a', 'b', 'c', 'd', 'e', 'f'].into_iter().enumerate() {
            let (ox, oy) = map(o, i % 3, i / 3, 3, 2);
            out[oy * w + ox] = name;
        }
        assert_eq!(out.len(), w * h);
        out
    };
    assert_eq!(tv12(6), ['d', 'a', 'e', 'b', 'f', 'c']);
    assert_eq!(tv12(8), ['c', 'f', 'b', 'e', 'a', 'd']);
    assert_eq!(tv12(3), ['f', 'e', 'd', 'c', 'b', 'a']);

    // The builder follows the recipes byte for byte.
    assert_eq!(f1_builder().build(), f1(), "F1");
    assert_eq!(f2_builder().build(), f2(), "F2");
    assert_eq!(f3_builder().build(), f3(), "F3");
    assert_eq!((f1().len(), f2().len(), f3().len()), (672, 416, 480));

    // Specification conflict: the annotations of F1 to F3 give the output
    // size of their crop rectangles (6 x 6, 2 x 6, 14 x 2), but those
    // rectangles are narrower than 16 pixels, so R-RAF-16 (normative)
    // requires the whole raster. The decoder follows R-RAF-16: 8 x 8,
    // 4 x 8 and 16 x 4. The fields are flat, so every pixel still has the
    // fixture's value; with the crop forced, the fixture's sizes result.
    let forced = |data: &[u8], crop: Rect| {
        let mut p = plan_of(data);
        assert_eq!(
            p.crop,
            Rect {
                x: 0,
                y: 0,
                w: p.width,
                h: p.height
            },
            "R-RAF-16"
        );
        p.crop = crop;
        linear_with(data, &p)
    };

    // F1 (V3, C1): every output pixel.
    let f1 = f1();
    let (_, _, v) = values_of(&f1);
    assert_eq!(&v[..8], &[5022, 5022, 3022, 5022, 5022, 2022, 5022, 5022]);
    let (_, _, cam) = camera_rgb(&f1);
    assert_rgb(
        cam[0],
        [0.251_346_1, 0.260_399_7, 0.125_888_6],
        EPS,
        "F1 balanced",
    );
    let (w, h, px) = linear(&f1);
    assert_eq!((w, h), (8, 8));
    for p in px {
        assert_rgb(p, F1_RGB, EPS, "F1");
    }
    let (w, h, px) = forced(
        &f1,
        Rect {
            x: 1,
            y: 1,
            w: 6,
            h: 6,
        },
    );
    assert_eq!((w, h), (6, 6));
    for p in px {
        assert_rgb(p, F1_RGB, EPS, "F1 cropped");
    }
    let dev = report(&f1);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V3));
    assert_eq!(dev.class, Some(Class::Sensor(Variant::V3, Layout::XTrans)));
    assert_eq!((dev.image.width(), dev.image.height()), (8, 8));
    assert_rgba16_flat(&dev.image, F1_RGB, "F1 delivered");

    // F2 (V1, C2, orientation 6): 4 wide, 8 high.
    let f2 = f2();
    let (_, _, v) = values_of(&f2);
    assert_eq!(&v[..4], &[1255, 1755, 1255, 1755]);
    assert_eq!(&v[8..12], &[1755, 1055, 1755, 1055]);
    let (_, _, cam) = camera_rgb(&f2);
    assert_rgb(
        cam[0],
        [0.401_983_6, 0.390_704_3, 0.303_682_2],
        EPS,
        "F2 balanced",
    );
    let (w, h, px) = linear(&f2);
    assert_eq!((w, h), (4, 8));
    for p in px {
        assert_rgb(p, F2_RGB, EPS, "F2");
    }
    let (w, h, px) = forced(
        &f2,
        Rect {
            x: 1,
            y: 1,
            w: 6,
            h: 2,
        },
    );
    assert_eq!((w, h), (2, 6));
    for p in px {
        assert_rgb(p, F2_RGB, EPS, "F2 cropped");
    }
    let dev = report(&f2);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    assert_eq!((dev.image.width(), dev.image.height()), (4, 8));
    assert_eq!(dev.info.as_ref().unwrap().orientation, 6);
    assert_rgba16_flat(&dev.image, F2_RGB, "F2 delivered");

    // F3 (V2, C2): 16 x 4.
    let f3 = f3();
    let (_, _, v) = values_of(&f3);
    assert_eq!(&v[..4], &[3024, 5024, 3024, 5024]);
    assert_eq!(&v[16..20], &[5024, 2024, 5024, 2024]);
    let (_, _, cam) = camera_rgb(&f3);
    assert_rgb(
        cam[0],
        [0.214_145_6, 0.260_433_6, 0.129_708_2],
        EPS,
        "F3 balanced",
    );
    let (w, h, px) = linear(&f3);
    assert_eq!((w, h), (16, 4));
    for p in px {
        assert_rgb(p, F3_RGB, EPS, "F3");
    }
    let (w, h, px) = forced(
        &f3,
        Rect {
            x: 1,
            y: 1,
            w: 14,
            h: 2,
        },
    );
    assert_eq!((w, h), (14, 2));
    for p in px {
        assert_rgb(p, F3_RGB, EPS, "F3 cropped");
    }
    let dev = report(&f3);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V2));
    assert_rgba16_flat(&dev.image, F3_RGB, "F3 delivered");

    // F4 (X1): the strip is not read; the stub has no frame, so no
    // candidate decodes and the decoder reports an error (R-RAF-52).
    let f4 = f4();
    assert_eq!(class_of(&f4), Class::OutOfScope(Relative::X1));
    assert!(preview::collect(&raf(&f4)).candidates.is_empty());
    assert!(decode_report(&f4, &DecodeOptions::default()).is_err());

    // R-RAF-1 through the registry: by extension and by magic alone.
    let reg = crate::default_registry();
    for name in [Some(PathBuf::from("a.RAF")), None] {
        let src = pandite_core::Source::from_bytes(f2.clone(), name);
        let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
        assert_eq!(img.first_image().format(), PixelFormat::Rgba16);
        assert_eq!(
            (img.first_image().width(), img.first_image().height()),
            (4, 8)
        );
        let meta = reg.probe(&src).unwrap();
        assert_eq!((meta.width, meta.height, meta.format), (4, 8, "raw"));
    }
}

#[test]
fn raf_levels_wb_matrix_vector() {
    // R-RAF-28, R-RAF-29, R-RAF-32, R-RAF-33 on F1: X-T3 values.
    let p = plan_of(&f1());
    assert_eq!(p.white, 16383);
    assert_eq!(p.black, 1022.0);
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert!((p.wb[0] - 1.930_463_576_2).abs() < 1e-10);
    assert_eq!(p.wb[1], 1.0);
    assert!((p.wb[2] - 1.933_774_834_4).abs() < 1e-10);
    // R-RAF-29 worked example: X100 254, 255, 255, 254 gives 254.5.
    let p2 = plan_of(&f2());
    assert_eq!((p2.black, p2.white), (254.5, 4095));
    assert_eq!(container::white_level(16), 65535);
    // The report of a developed file carries the same levels.
    let info = report(&f1()).info.expect("developed");
    assert_eq!(
        (info.layout, info.bits, info.white),
        (Layout::XTrans, 14, 16383)
    );
    assert_eq!(
        (info.black, info.wb, info.wb_source),
        (p.black, p.wb, p.wb_source)
    );

    // TV-8 and TV-11 through the pipeline: a flat X-T3 field of the TV-5
    // values develops to the TV-8 normalized values (R-RAF-34) and the
    // TV-11 output (R-RAF-37).
    let flat = |r: u16, g: u16, b: u16| -> Vec<u8> {
        let pat = pattern_of(&XT3);
        let vals: Vec<u16> = (0..36).map(|i| [r, g, b][usize::from(pat[i])]).collect();
        Builder::new(
            6,
            6,
            14,
            &XT3,
            (0, 0, 6, 6),
            vec![1022; 36],
            [302, 583, 584],
            1,
            pack_v3(&vals),
        )
        .build()
    };
    let f = flat(1703, 2277, 1579);
    let (_, _, cam) = camera_rgb(&f);
    assert_rgb(
        cam[0],
        [0.085_583_34, 0.081_700_41, 0.070_119_95],
        5e-8,
        "TV-8 a",
    );
    let (_, _, px) = linear(&f);
    assert_rgb(
        px[0],
        [0.089_366_31, 0.086_767_88, 0.064_284_14],
        EPS,
        "TV-11",
    );
    let (_, _, cam) = camera_rgb(&flat(1631, 2032, 1454));
    assert_rgb(
        cam[0],
        [0.076_534_88, 0.065_750_93, 0.054_383_88],
        5e-8,
        "TV-8 b",
    );

    // R-RAF-35, R-RAF-36: the matrix of the table, each row summing to 1;
    // a neutral value stays neutral (TV-11).
    let m = container::matrix();
    assert_eq!(
        m[0][0],
        1.400_839_3 / (1.400_839_3 - 0.208_572_2 - 0.192_267_1)
    );
    for r in m {
        assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        let v: f64 = r.iter().map(|c| c * 0.25).sum();
        assert!((v - 0.25).abs() < 1e-12);
    }
    let tv11: Vec<f64> = m
        .iter()
        .map(|r| r[0] * 0.085_583_34 + r[1] * 0.081_700_41 + r[2] * 0.070_119_95)
        .collect();
    for (g, w) in tv11.iter().zip([0.089_366_31, 0.086_767_88, 0.064_284_14]) {
        assert!((g - w).abs() < 5e-8, "TV-11 {tv11:?}");
    }

    // R-RAF-33 sources 2 and 3, and R-RAF-62: a zero G skips a source.
    let base = f1_builder();
    let p = plan_of(
        &base
            .clone()
            .tag(0xF00E, 4, vec![0, 583, 584])
            .tag(0xF00D, 4, vec![300, 600, 450])
            .build(),
    );
    assert_eq!(p.wb_source, WbSource::Auto);
    assert_eq!(p.wb, [2.0, 1.0, 1.5]);
    let p = plan_of(&base.clone().untag(0xF00E).build());
    assert_eq!(p.wb_source, WbSource::Generic);
    assert_eq!(p.wb, [1.830, 1.0, 1.849]);
    // A tag with fewer than three values is not a source.
    let p = plan_of(&base.clone().tag(0xF00E, 4, vec![302, 583]).build());
    assert_eq!(p.wb_source, WbSource::Generic);
    // SHORT values are accepted (R-RAF-12).
    let p = plan_of(&base.clone().tag(0xF00E, 3, vec![302, 604, 151]).build());
    assert_eq!((p.wb_source, p.wb), (WbSource::AsShot, [2.0, 1.0, 0.5]));

    // R-RAF-31: one mean black level for every pixel, from 36 different
    // values (mean 1017.5).
    let blacks: Vec<u32> = (1000..1036).collect();
    let f = base.clone().tag(0xF00A, 4, blacks).build();
    let p = plan_of(&f);
    assert_eq!(p.black, 1017.5);
    let (_, _, cam) = camera_rgb(&f);
    let n = |s: f64, m: f64| ((s - 1017.5) / (16383.0 - 1017.5) * m) as f32;
    for c in cam {
        assert_rgb(
            c,
            [n(3022.0, p.wb[0]), n(5022.0, 1.0), n(2022.0, p.wb[2])],
            EPS,
            "R-RAF-31",
        );
    }

    // R-RAF-34: values clip to 0 and 1 (below black, above white).
    let pat = pattern_of(&XT3);
    let vals: Vec<u16> = (0..64)
        .map(|i| {
            if pat[6 * ((i / 8) % 6) + (i % 8) % 6] == 1 {
                20000
            } else {
                0
            }
        })
        .collect();
    let f = base.clone().strip_values(&vals).build();
    let (_, _, cam) = camera_rgb(&f);
    for c in cam {
        assert_eq!(c, [0.0, 1.0, 0.0]);
    }
    // R-RAF-37: the output is clipped to 0 to 1 as well.
    let (_, _, px) = linear(&f);
    for c in px {
        assert!(c.iter().all(|v| (0.0..=1.0).contains(v)), "{c:?}");
    }
}

/// An independent per-pixel implementation of R-RAF-34, R-RAF-39,
/// R-RAF-40 and R-RAF-37 in f64, on the stored raster.
fn reference<'a>(values: &'a [u16], plan: &'a Plan) -> impl Fn(usize, usize) -> [f64; 3] + 'a {
    let (w, h) = (plan.width as i64, plan.height as i64);
    let col = move |x: i64, y: i64| usize::from(plan.pattern[(6 * (y % 6) + x % 6) as usize]);
    let norm = move |x: i64, y: i64| {
        let s = f64::from(values[(y * w + x) as usize]);
        ((s - plan.black) / (f64::from(plan.white) - plan.black) * plan.wb[col(x, y)])
            .clamp(0.0, 1.0)
    };
    move |x, y| {
        let (x, y) = (x as i64, y as i64);
        let own = col(x, y);
        let mut v = [0f64; 3];
        for (c, out) in v.iter_mut().enumerate() {
            if c == own {
                *out = norm(x, y);
                continue;
            }
            for r in [1i64, 2] {
                let (mut sum, mut n) = (0f64, 0u32);
                for dy in -r..=r {
                    for dx in -r..=r {
                        let (xx, yy) = (x + dx, y + dy);
                        if (dx, dy) == (0, 0) || xx < 0 || yy < 0 || xx >= w || yy >= h {
                            continue;
                        }
                        if col(xx, yy) == c {
                            sum += norm(xx, yy);
                            n += 1;
                        }
                    }
                }
                if n > 0 {
                    *out = sum / f64::from(n);
                    break;
                }
            }
        }
        let m = &plan.matrix;
        [0, 1, 2].map(|r| (m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]).clamp(0.0, 1.0))
    }
}

/// Stored pixel of output pixel (ox, oy) by the R-RAF-45 table.
fn stored_of(o: u8, ox: usize, oy: usize, w: usize, h: usize) -> (usize, usize) {
    for y in 0..h {
        for x in 0..w {
            if map(o, x, y, w, h) == (ox, oy) {
                return (x, y);
            }
        }
    }
    unreachable!()
}

/// A pattern whose 3 x 3 windows sometimes miss blue (R-RAF-40) and that
/// still has 8 red, 20 green and 8 blue positions (R-RAF-17).
const CLUSTERED: [[u8; 6]; 6] = [
    [2, 2, 2, 2, 2, 2],
    [2, 2, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1],
    [1, 1, 1, 1, 1, 1],
    [1, 1, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 1],
];

#[test]
fn raf_orientation_and_crop() {
    // A 32 x 21 raster, crop 16 x 17 at (3, 1), every orientation, both
    // layouts and every variant (R-RAF-16, R-RAF-20, R-RAF-41, R-RAF-42,
    // R-RAF-45, R-RAF-46): the renderer equals the reference that
    // demosaics the whole raster in stored phase, then crops, then orients.
    let (w, h) = (32usize, 21usize);
    let mut next = noise(7);
    let values: Vec<u16> = (0..w * h).map(|_| 300 + (next() % 3700) as u16).collect();
    let layouts: [Vec<u8>; 4] = [XT3.to_vec(), XT1.to_vec(), xtrans_record(CLUSTERED), vec![]];
    for layout in &layouts {
        for (variant, bits) in [(Variant::V1, 12u32), (Variant::V2, 14), (Variant::V3, 16)] {
            let strip = match variant {
                Variant::V1 => pack_v1(&values),
                Variant::V2 => pack_v2(&values, w),
                Variant::V3 => pack_v3(&values),
            };
            for o in 1..=8u16 {
                let mut b = Builder::new(
                    w as u32,
                    h as u32,
                    bits,
                    layout,
                    (3, 1, 16, 17),
                    vec![256; 4],
                    [300, 500, 420],
                    o,
                    strip.clone(),
                );
                b.jpeg = jpeg_exif(48, 32, [9, 9, 9], o, None);
                let f = b.build();
                let want = Class::Sensor(
                    variant,
                    if layout.is_empty() {
                        Layout::Bayer
                    } else {
                        Layout::XTrans
                    },
                );
                assert_eq!(class_of(&f), want);
                let plan = plan_of(&f);
                assert_eq!(
                    plan.crop,
                    Rect {
                        x: 3,
                        y: 1,
                        w: 16,
                        h: 17
                    }
                );
                assert_eq!(plan.orientation, o as u8);
                let (_, _, decoded) = values_of(&f);
                assert_eq!(decoded, values, "{variant:?}");
                let (ow, oh, px) = linear(&f);
                assert_eq!((ow, oh), if o >= 5 { (17, 16) } else { (16, 17) });
                let r = reference(&values, &plan);
                for oy in 0..oh {
                    for ox in 0..ow {
                        let (x, y) = stored_of(o as u8, ox, oy, 16, 17);
                        let want = r(3 + x, 1 + y).map(|v| v as f32);
                        assert_rgb(
                            px[oy * ow + ox],
                            want,
                            2e-6,
                            &format!("{layout:?} {variant:?} o{o} ({ox}, {oy})"),
                        );
                    }
                }
                let dev = report(&f);
                assert_eq!(dev.origin, Origin::Sensor(variant));
                assert_eq!(
                    (dev.image.width() as usize, dev.image.height() as usize),
                    (ow, oh)
                );
            }
            // The whole raster, borders included (R-RAF-40 at the edges).
            let f = Builder::new(
                w as u32,
                h as u32,
                bits,
                layout,
                (0, 0, 15, 15),
                vec![256; 4],
                [300, 500, 420],
                1,
                strip.clone(),
            )
            .build();
            let plan = plan_of(&f);
            assert_eq!(plan.crop, Rect { x: 0, y: 0, w, h }, "R-RAF-16: CW < 16");
            let (ow, _, px) = linear(&f);
            let r = reference(&values, &plan);
            for y in 0..h {
                for x in 0..w {
                    assert_rgb(
                        px[y * ow + x],
                        r(x, y).map(|v| v as f32),
                        2e-6,
                        &format!("{layout:?} whole ({x}, {y})"),
                    );
                }
            }
        }
    }

    // R-RAF-40: 0 when no pixel of a missing color lies in the 5 x 5
    // neighborhood: a Bayer raster of one row has no blue.
    let f = unit_v3(16, 1, &[], &[1000; 16]);
    let (_, _, cam) = camera_rgb(&f);
    for c in cam {
        assert_eq!(c[2], 0.0);
    }

    // R-RAF-16: the crop is rejected (whole raster) when one record is
    // missing, shorter than 4 bytes, or the rectangle leaves the raster.
    let base = Builder::new(
        w as u32,
        h as u32,
        16,
        &XT3,
        (3, 1, 16, 17),
        vec![256; 4],
        [300, 500, 420],
        1,
        pack_v3(&values),
    );
    let whole = Rect { x: 0, y: 0, w, h };
    let crop_of = |b: Builder| plan_of(&b.build()).crop;
    assert_eq!(crop_of(base.clone().record(0x0110, None)), whole);
    assert_eq!(crop_of(base.clone().record(0x0111, None)), whole);
    assert_eq!(
        crop_of(base.clone().record(0x0111, Some(vec![0, 17]))),
        whole
    );
    assert_eq!(
        crop_of(base.clone().record(0x0111, Some(hex("00 11 00 1E")))),
        whole,
        "3 + 30 > 32"
    );
    assert_eq!(
        crop_of(base.clone().record(0x0110, Some(hex("00 05 00 03")))),
        whole,
        "5 + 17 > 21"
    );
    assert_eq!(
        crop_of(base.clone().record(0x0110, Some(hex("00 04 00 10")))),
        Rect {
            x: 16,
            y: 4,
            w: 16,
            h: 17
        },
        "top first, then left"
    );
    // R-RAF-43: an orientation outside 1 to 8 counts as 1.
    let mut b = base.clone();
    b.jpeg = stub(9);
    assert_eq!(plan_of(&b.build()).orientation, 1);
}

// ---------------------------------------------------------------------------
// Container rules, preview path, hostile input, cancellation
// ---------------------------------------------------------------------------

impl Builder {
    /// Replace the strip with V3 values.
    fn strip_values(mut self, values: &[u16]) -> Self {
        self.strip = pack_v3(values);
        self
    }
}

/// A good V3 X-Trans file with a 48 x 32 red main JPEG (orientation
/// `o`) holding a 16 x 8 blue IFD1 thumbnail.
fn with_previews(o: u16) -> Builder {
    let mut b = f1_builder();
    b.jpeg = jpeg_exif(48, 32, [200, 40, 40], o, Some(jpeg(16, 8, [40, 40, 200])));
    b
}

fn assert_preview(dev: &Developed, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_eq!(dev.origin, Origin::Preview, "{what}");
    assert_eq!(dev.image.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((dev.image.width(), dev.image.height()), size, "{what}");
    let p = &dev.image.data()[..4];
    for c in 0..3 {
        assert!(p[c].abs_diff(rgb[c]) <= 6, "{what}: {p:?} vs {rgb:?}");
    }
}

/// Write a BE32 into the header.
fn set_be32(f: &mut [u8], at: usize, v: u32) {
    f[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

#[test]
fn raf_out_of_scope_falls_back_to_preview() {
    let good = with_previews(1);
    let g = good.build();
    assert_eq!(report(&g).origin, Origin::Sensor(Variant::V3));
    // R-RAF-47, R-RAF-49: the main JPEG and the thumbnail, largest first.
    let pv = preview::collect(&raf(&g));
    let sizes: Vec<(u32, u32)> = pv.candidates.iter().map(|c| (c.width, c.height)).collect();
    assert_eq!(sizes, [(48, 32), (16, 8)]);
    assert_eq!(pv.candidates[0].offset, 0x94);

    // R-RAF-2, R-RAF-53: every out-of-scope or failing file delivers the
    // preview, reported as such.
    let check = |f: &[u8], class: Option<Class>, what: &str| {
        let dev = report(f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        if class.is_some() {
            assert_eq!(dev.class, class, "{what}");
        }
    };
    let oos = |r: Relative| Some(Class::OutOfScope(r));
    // R-RAF-3 rule 1: format version "0200" (X3).
    let mut b = good.clone();
    b.version = *b"0200";
    check(&b.build(), oos(Relative::X3), "X3 version");
    // Rule 2: RAFCompression 2 (X1), 3 (X2), any other non-zero value (X6).
    for (v, r) in [(2, Relative::X1), (3, Relative::X2), (1, Relative::X6)] {
        let mut b = good.clone();
        b.compression = v;
        check(&b.build(), oos(r), "RAFCompression");
    }
    // Rule 3: the CFA container absent (R-RAF-6): length 0, or past the end.
    let cfa_len_at = 104;
    let mut f = g.clone();
    set_be32(&mut f, cfa_len_at, 0);
    check(&f, oos(Relative::X6), "CFA length 0");
    let mut f = g.clone();
    let len = u32::from_be_bytes(f[cfa_len_at..cfa_len_at + 4].try_into().unwrap());
    set_be32(&mut f, cfa_len_at, len + 1);
    check(&f, oos(Relative::X6), "CFA past the end");
    // Rule 4: no little-endian TIFF header (X3; OQ-12 big-endian too).
    let cfa_at = u32::from_be_bytes(g[100..104].try_into().unwrap()) as usize;
    for magic in [[0x4D, 0x4D, 0x00, 0x2A], [0x00, 0x20, 0x00, 0x20]] {
        let mut f = g.clone();
        f[cfa_at..cfa_at + 4].copy_from_slice(&magic);
        check(&f, oos(Relative::X3), "CFA magic");
    }
    // Rule 5: a second CFA container (X4).
    let mut b = good.clone();
    b.second_cfa = 10_298_640;
    check(&b.build(), oos(Relative::X4), "X4");
    // Rule 6 (R-RAF-10, R-RAF-13, R-RAF-14, R-RAF-15): X6.
    let mut b = good.clone();
    b.ifd0 = Some(vec![(0x0100, 4, 1, 26)]);
    check(&b.build(), oos(Relative::X6), "no 0xF000");
    let mut b = good.clone();
    b.ifd0 = Some(vec![(0xF000, 13, 2, 26)]);
    check(&b.build(), oos(Relative::X6), "0xF000 count 2");
    let mut b = good.clone();
    b.ifd0 = Some(vec![(0xF000, 3, 1, 26)]);
    check(&b.build(), oos(Relative::X6), "0xF000 type 3");
    let mut b = good.clone();
    b.ifd0 = Some(vec![(0xF000, 13, 1, 100_000)]);
    check(&b.build(), oos(Relative::X6), "FujiIFD outside");
    let mut b = good.clone();
    b.ifd0 = Some(vec![(0xF000, 4, 1, 26)]);
    assert_eq!(
        report(&b.build()).origin,
        Origin::Sensor(Variant::V3),
        "type 4"
    );
    for t in [0xF001, 0xF002, 0xF003, 0xF007, 0xF008, 0xF00A] {
        check(
            &good.clone().untag(t).build(),
            oos(Relative::X6),
            "R-RAF-14",
        );
        // R-RAF-12: an entry of another type is skipped, so it is absent.
        let mut b = good.clone();
        for e in &mut b.fuji {
            if e.0 == t {
                e.1 = 9;
            }
        }
        check(&b.build(), oos(Relative::X6), "R-RAF-12 SLONG");
        check(
            &good.clone().tag(t, 4, vec![]).build(),
            oos(Relative::X6),
            "count 0",
        );
    }
    check(
        &good
            .clone()
            .record(0x0100, Some(hex("00 08 00 09")))
            .build(),
        oos(Relative::X6),
        "R-RAF-15",
    );
    // A 0x0100 record shorter than 4 bytes is not checked.
    let f = good.clone().record(0x0100, Some(hex("00 09"))).build();
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V3));
    // R-RAF-8: the first record of a tag counts; a later copy is ignored.
    let mut b = good.clone();
    b.records.push((0x0100, hex("00 08 00 09")));
    assert_eq!(report(&b.build()).origin, Origin::Sensor(Variant::V3));
    b.records.insert(0, (0x0100, hex("00 08 00 09")));
    check(&b.build(), oos(Relative::X6), "R-RAF-8 first copy");
    // R-RAF-7: reading stops at the first record whose data leave the
    // container (0x0131 here) and keeps the records before it.
    let mut f = g.clone();
    let meta_at = u32::from_be_bytes(f[92..96].try_into().unwrap()) as usize;
    let size_at = meta_at + 4 + 4 * 8 + 2;
    assert_eq!(f[size_at - 2..size_at], [0x01, 0x31]);
    f[size_at..size_at + 2].copy_from_slice(&37u16.to_be_bytes());
    let records = container::records(&raf(&f)).unwrap();
    assert_eq!(records.get(0x0130), Some(&[0x0C; 4][..]));
    assert_eq!(records.get(0x0131), None);
    check(&f, oos(Relative::X5), "R-RAF-7");
    // A record count larger than the records present stops at the end.
    let mut b = good.clone();
    b.record_count = Some(4096);
    assert_eq!(report(&b.build()).origin, Origin::Sensor(Variant::V3));
    // Rule 7 (R-RAF-17, R-RAF-19): X5.
    let mut bad = XT3;
    bad[0] = 0;
    for (tag, data) in [
        (0x0131, Some(bad.to_vec())),
        (0x0131, Some(XT3[..35].to_vec())),
        (0x0131, Some([XT3.to_vec(), vec![1]].concat())),
        (0x0131, Some([&XT3[..35], &[3u8][..]].concat())),
    ] {
        check(
            &good.clone().record(tag, data).build(),
            oos(Relative::X5),
            "C1",
        );
    }
    let bayer = f3_builder();
    let mut bp = bayer.clone();
    bp.jpeg = good.jpeg.clone();
    assert_eq!(report(&bp.build()).origin, Origin::Sensor(Variant::V2));
    for (tag, data) in [
        (0x0130, Some(hex("08 09 0B 0A"))),
        (0x0130, None),
        (0x0130, Some(hex("0A 0B 09 08 00"))),
        (0x0131, Some(XT3.to_vec())),
    ] {
        let f = bp.clone().record(tag, data.clone()).build();
        let want = if tag == 0x0131 {
            // C1 wins over 0x0130 when 0x0131 is valid.
            Class::Sensor(Variant::V2, Layout::XTrans)
        } else {
            Class::OutOfScope(Relative::X5)
        };
        assert_eq!(class_of(&f), want, "{tag:#X} {data:?}");
    }
    // No rule matches (X6): other bit depths (R-RAF-61) and byte counts
    // (R-RAF-59: only the exact counts of R-RAF-3 classify).
    check(
        &good.clone().tag(0xF003, 4, vec![12]).build(),
        oos(Relative::X6),
        "bits 12 with V3 bytes",
    );
    check(
        &good.clone().tag(0xF003, 4, vec![13]).build(),
        oos(Relative::X6),
        "bits 13",
    );
    check(
        &good.clone().tag(0xF008, 4, vec![127]).build(),
        oos(Relative::X6),
        "byte count",
    );
    let v3_16 = good.clone().tag(0xF003, 4, vec![16]).build();
    assert_eq!(class_of(&v3_16), Class::Sensor(Variant::V3, Layout::XTrans));
    // R-RAF-24: V1 with an odd W; R-RAF-26: V2 with W mod 16 not 0.
    let mut b = Builder::new(
        7,
        2,
        12,
        &[],
        (0, 0, 7, 2),
        vec![0; 4],
        [1, 1, 1],
        1,
        vec![0; 21],
    );
    b.jpeg = good.jpeg.clone();
    check(&b.build(), oos(Relative::X6), "V1 odd W");
    let mut b = Builder::new(
        8,
        4,
        14,
        &[],
        (0, 0, 8, 4),
        vec![0; 4],
        [1, 1, 1],
        1,
        vec![0; 56],
    );
    b.jpeg = good.jpeg.clone();
    check(&b.build(), oos(Relative::X6), "V2 W mod 16");
    // R-RAF-22, R-RAF-60: the strip leaves the CFA container.
    check(
        &good.clone().tag(0xF007, 4, vec![300]).build(),
        oos(Relative::X6),
        "strip",
    );
    // R-RAF-30: black not below w - 255.
    check(
        &good.clone().tag(0xF00A, 4, vec![16128]).build(),
        oos(Relative::X6),
        "black",
    );
    let f = good.clone().tag(0xF00A, 4, vec![16127]).build();
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V3));

    // Failed checks of a specified variant use the preview (R-RAF-52).
    // R-RAF-55: W 65536 with a matching strip; W = 0 with an empty one.
    let mut b = Builder::new(
        65536,
        1,
        14,
        &[],
        (0, 0, 16, 1),
        vec![0; 4],
        [1, 1, 1],
        1,
        vec![0; 131_072],
    );
    b.jpeg = good.jpeg.clone();
    let f = b.record(0x0100, None).build();
    assert_eq!(class_of(&f), Class::Sensor(Variant::V3, Layout::Bayer));
    check(&f, None, "W above 65535");
    let mut b = Builder::new(
        0,
        4,
        14,
        &[],
        (0, 0, 0, 4),
        vec![0; 4],
        [1, 1, 1],
        1,
        vec![],
    );
    b.jpeg = good.jpeg.clone();
    check(&b.build(), None, "W = 0");
    assert!(container::check_limits(65535, 2048).is_ok());
    assert!(
        container::check_limits(65535, 2049).is_err(),
        "W * H above 2^27"
    );
    assert!(container::check_limits(1, 65536).is_err());
    // R-RAF-56: more than 4096 records fail.
    let mut b = good.clone();
    b.record_count = Some(4097);
    check(&b.build(), None, "R-RAF-56");
    // R-RAF-57: a value range larger than the container fails; R-RAF-58:
    // a value outside the container fails.
    let fuji_at = cfa_at + 26;
    let black_entry = (0..7)
        .map(|i| fuji_at + 2 + 12 * i)
        .find(|&e| g[e..e + 2] == [0x0A, 0xF0])
        .unwrap();
    let mut f = g.clone();
    f[black_entry + 4..black_entry + 8].copy_from_slice(&0x10000u32.to_le_bytes());
    check(&f, None, "R-RAF-57");
    let mut f = g.clone();
    f[black_entry + 8..black_entry + 12].copy_from_slice(&5000u32.to_le_bytes());
    check(&f, None, "R-RAF-58");
    // R-RAF-13: an IFD with more than 1024 entries is invalid (X6).
    let mut f = g.clone();
    f[fuji_at..fuji_at + 2].copy_from_slice(&1025u16.to_le_bytes());
    check(&f, oos(Relative::X6), "R-RAF-13");

    // R-RAF-52: the next-largest candidate when the largest fails.
    let main = jpeg_exif(48, 32, [200, 40, 40], 1, Some(jpeg(16, 8, [40, 40, 200])));
    let sos = main.windows(2).rposition(|w| w == [0xFF, 0xDA]).unwrap();
    let mut b = good.clone();
    b.compression = 2;
    b.jpeg = main[..sos + 4].to_vec();
    assert_preview(&report(&b.build()), (16, 8), [40, 40, 200], "next-largest");
    // An error only when no candidate decodes.
    b.jpeg = stub(1);
    assert!(decode_report(&b.build(), &DecodeOptions::default()).is_err());
    // R-RAF-48: candidates outside the APP1 segment, shorter than 4 bytes
    // or without FF D8 are dropped; R-RAF-6: a JPEG of length 0 is absent.
    let mut thumbless = main.clone();
    // The TIFF header at 12, IFD1 at 26 of it, its first value field.
    let off = 12 + 26 + 2 + 8;
    thumbless[off..off + 4].copy_from_slice(&60_000u32.to_le_bytes());
    let mut b = good.clone();
    b.jpeg = thumbless;
    let f = b.build();
    assert_eq!(preview::collect(&raf(&f)).candidates.len(), 1);
    let mut f = g.clone();
    f[0x94] = 0;
    assert!(preview::collect(&raf(&f))
        .candidates
        .iter()
        .all(|c| c.offset != 0x94));
    let mut f = g.clone();
    set_be32(&mut f, 88, 0);
    let pv = preview::collect(&raf(&f));
    assert!(pv.candidates.is_empty() && pv.orientation == 1);
    let mut f = g.clone();
    set_be32(&mut f, 88, 3);
    assert!(preview::collect(&raf(&f)).candidates.is_empty());
    // R-RAF-50: bytes after EOI, and a stream that ends without EOI.
    let mut b = good.clone();
    b.compression = 2;
    let mut padded = main.clone();
    padded.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x13]);
    b.jpeg = padded;
    assert_preview(&report(&b.build()), (48, 32), [200, 40, 40], "after EOI");
    b.jpeg = main[..main.len() - 2].to_vec();
    assert_preview(&report(&b.build()), (48, 32), [200, 40, 40], "no EOI");
    // R-RAF-51: orientation 6 rotates a wide preview, not a tall one.
    let mut b = with_previews(6);
    b.compression = 3;
    assert_preview(&report(&b.build()), (32, 48), [200, 40, 40], "rotated");
    b.jpeg = jpeg_exif(32, 48, [200, 40, 40], 6, None);
    assert_preview(
        &report(&b.build()),
        (32, 48),
        [200, 40, 40],
        "tall not rotated",
    );
    b.jpeg = jpeg_exif(32, 48, [200, 40, 40], 8, None);
    assert_preview(
        &report(&b.build()),
        (32, 48),
        [200, 40, 40],
        "tall, orientation 8",
    );

    // R-RAF-43, R-RAF-44: the orientation from the first Exif APP1 before
    // SOS, also when the JPEG has no frame; a non-Exif APP1 first is
    // skipped; an Exif segment after SOS is not read.
    let o_of = |jpeg: Vec<u8>| {
        let mut b = good.clone();
        b.jpeg = jpeg;
        preview::collect(&raf(&b.build())).orientation
    };
    assert_eq!(o_of(stub(6)), 6);
    assert_eq!(o_of(stub(0)), 1);
    let mut xmp = vec![0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x08];
    xmp.extend_from_slice(b"http:/");
    xmp.extend_from_slice(&stub(3)[2..]);
    assert_eq!(o_of(xmp), 3);
    let mut late = hex("FF D8 FF DA 00 02");
    late.extend_from_slice(&stub(3)[2..]);
    assert_eq!(o_of(late), 1);
    // An IFD0 offset that leaves the segment reads nothing.
    let mut s = stub(6);
    s[16] = 0x40;
    assert_eq!(o_of(s), 1);
    // The raw image gets the orientation too (R-RAF-45).
    let f = with_previews(8).build();
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V3));
    assert_eq!((dev.image.width(), dev.image.height()), (8, 8));
    assert_eq!(dev.info.unwrap().orientation, 8);

    // R-RAF-4, R-RAF-54: not a RAF file; a RAF file shorter than 148 bytes.
    let mut f = g.clone();
    f[15] = b'_';
    assert!(decode_report(&f, &DecodeOptions::default())
        .unwrap()
        .is_none());
    assert!(!is_raf(&f, None) && is_raf(&f, Some("raf")));
    assert!(decode_report(&g[..147], &DecodeOptions::default()).is_err());
    assert!(decode_report(&g[..148], &DecodeOptions::default()).is_err());

    // Through the registry: the provider never fails where a preview
    // exists; probe reports the preview size for out-of-scope files.
    let reg = crate::default_registry();
    let mut b = with_previews(6);
    b.compression = 3;
    let src = pandite_core::Source::from_bytes(b.build(), Some("x.raf".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (32, 48));
    let src = pandite_core::Source::from_bytes(g.clone(), None);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (8, 8));
    // A thumbnail request returns 8-bit preview data (target sizes).
    let small = DecodeOptions {
        target_size: Some((16, 16)),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&g, &small).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Preview);
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut v1 = f2_builder();
    v1.jpeg = jpeg_exif(32, 16, [200, 40, 40], 6, Some(jpeg(16, 8, [40, 40, 200])));
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F1+previews", with_previews(6).build()),
        ("F2+previews", v1.build()),
    ]
}

/// R-RAF-63: damaged real-structure files never panic (also not in a
/// helper thread, whose panic would surface as an error message) and never
/// abort; failures are error values.
#[test]
fn raf_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let mut next = noise(0x9E37_79B9);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
                // The header, the meta container and the directories.
                0 | 1 => {
                    let limit = bad.len().min(if round % 2 == 0 { 160 } else { 700 });
                    for _ in 0..1 + next() % 6 {
                        let i = next() as usize % limit;
                        bad[i] = next() as u8;
                    }
                }
                // Anywhere.
                2 => {
                    for _ in 0..1 + next() % 8 {
                        let i = next() as usize % bad.len();
                        bad[i] = next() as u8;
                    }
                }
                // Huge values in header fields and IFD entries.
                _ => {
                    let i = next() as usize % bad.len().min(700);
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27, 4097][next() as usize % 6];
                    if i + 4 <= bad.len() {
                        let bytes = if i < 148 {
                            v.to_be_bytes()
                        } else {
                            v.to_le_bytes()
                        };
                        bad[i..i + 4].copy_from_slice(&bytes);
                    }
                }
            }
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&bad);
                let _ = facts(&bad);
                let _ = has_preview(&bad);
                let small = DecodeOptions {
                    target_size: Some((8, 8)),
                    ..DecodeOptions::default()
                };
                for opts in [DecodeOptions::default(), small] {
                    if let Err(e) = decode_report(&bad, &opts) {
                        let msg = e.to_string();
                        assert!(!msg.contains("panicked"), "{name} round {round}: {msg}");
                    }
                }
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.raf".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized or lying claims are refused before allocation (R-RAF-55,
    // R-RAF-57): a strip claimed far larger than the file is X6 by
    // R-RAF-22, and huge sizes never reach the allocator.
    for (w, h) in [
        (65535u32, 65535u32),
        (u32::MAX, 2),
        (2, u32::MAX),
        (1 << 16, 1 << 16),
    ] {
        let f = f1_builder()
            .tag(0xF001, 4, vec![w])
            .tag(0xF002, 4, vec![h])
            .tag(0xF008, 4, vec![w.wrapping_mul(h).wrapping_mul(2)])
            .record(0x0100, None)
            .build();
        assert!(matches!(class_of(&f), Class::OutOfScope(Relative::X6)));
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    }
}

/// R-RAF-64: a file cut anywhere gives an error or the preview, never a
/// partly decoded raw image.
#[test]
fn raf_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 15, 16, 147, 148, 149, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => {
                    assert_eq!(dev.origin, Origin::Preview, "{name} cut at {cut}")
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large V3 X-Trans file: 4096 x 2048.
fn large() -> Vec<u8> {
    let (w, h) = (4096u32, 2048u32);
    let mut next = noise(3);
    let values: Vec<u16> = (0..w * h).map(|_| 1100 + (next() % 9000) as u16).collect();
    Builder::new(
        w,
        h,
        14,
        &XT3,
        (16, 16, 4000, 2000),
        vec![1022; 36],
        [302, 583, 584],
        1,
        pack_v3(&values),
    )
    .build()
}

#[test]
fn raf_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    for f in [f1(), f4(), with_previews(1).build()] {
        assert!(matches!(
            decode_report(&f, &cancelled),
            Err(Error::Cancelled)
        ));
    }
    let src = pandite_core::Source::from_bytes(f1(), Some("a.raf".into()));
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&crate::RawDecoder, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread.
    let f = large();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V3));
    assert_eq!((dev.image.width(), dev.image.height()), (4000, 2000));
    drop(dev);
    let token = CancelToken::new();
    let options = DecodeOptions {
        cancel: token.clone(),
        ..DecodeOptions::default()
    };
    let start = Instant::now();
    let result = std::thread::scope(|s| {
        let h = s.spawn(|| decode_report(&f, &options));
        std::thread::sleep(full / 5);
        token.cancel();
        h.join().unwrap()
    });
    let took = start.elapsed();
    assert!(matches!(result, Err(Error::Cancelled)), "not cancelled");
    assert!(
        took < full * 4 / 5 + Duration::from_millis(50),
        "cancel took {took:?} of a {full:?} decode"
    );
}

/// Header facts: storage, layout, sample depth and the camera of the
/// preview's Exif segment, through the provider and the registry.
#[test]
fn facts_raf() {
    use pandite_core::{DecoderProvider, Source};
    let f = super::facts(&f1()).expect("RAF facts");
    assert_eq!(f.container.as_deref(), Some("Fujifilm RAF"));
    assert_eq!(f.compression.as_deref(), Some("Uncompressed"));
    assert_eq!(f.channels.as_deref(), Some("X-Trans"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(14)));
    let f = super::facts(&f2()).unwrap();
    assert_eq!(
        f.compression.as_deref(),
        Some("Uncompressed, 12-bit packed")
    );
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!(f.bits_per_channel, Some(12));
    let f = super::facts(&f3()).unwrap();
    assert_eq!(
        f.compression.as_deref(),
        Some("Uncompressed, 14-bit packed")
    );
    let f = super::facts(&f4()).unwrap();
    assert_eq!(
        f.compression.as_deref(),
        Some("Fujifilm lossless compressed")
    );
    assert_eq!(f.channels, None);
    let via = crate::raw::RawDecoder
        .facts(&Source::from_bytes(f2(), Some("a.raf".into())))
        .expect("provider facts");
    assert_eq!(via.channels.as_deref(), Some("Bayer RGGB"));
    let reg = crate::default_registry();
    let via = reg.facts(&Source::from_bytes(f1(), None)).unwrap();
    assert_eq!(via.container.as_deref(), Some("Fujifilm RAF"));
    assert!(super::facts(b"II*\0\x08\0\0\0").is_none());
    let f1 = f1();
    for n in 0..f1.len() {
        let _ = super::facts(&f1[..n]);
    }
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// TV-13: the class of each sample file of SOURCES.txt.
fn tv13(name: &str) -> Option<Class> {
    use Layout::{Bayer, XTrans};
    use Variant::{V1, V2, V3};
    Some(match name {
        "FinePix_X100_DSCF1676.RAF" => Class::Sensor(V1, Bayer),
        "X-Pro1_DSCF2131.RAF" => Class::Sensor(V1, XTrans),
        "X-A5_DSCF0617.RAF" => Class::Sensor(V2, Bayer),
        "X-T1_20171229_110916.RAF" => Class::Sensor(V3, XTrans),
        "X-T3_AFXT2720.RAF" => Class::Sensor(V3, XTrans),
        "GFX_50S_20170525_0036TEST.RAF" => Class::Sensor(V3, Bayer),
        "X-T4_DSCF0267.RAF" => Class::OutOfScope(Relative::X1),
        "X-T4_DSCF0268.RAF" => Class::OutOfScope(Relative::X2),
        "FinePix_S5Pro_2018_06150003.RAF" => Class::OutOfScope(Relative::X3),
        "FinePix_X10_DSCF0514.RAF" => Class::OutOfScope(Relative::X4),
        _ => return None,
    })
}

/// The worked examples of the Compression section on the sample files:
/// row, first column, file offset of the first byte, the decoded values
/// and their colors (M4 and TV-6, TV-7, TV-5).
#[allow(clippy::type_complexity)]
fn worked_example(name: &str) -> Option<(usize, usize, usize, &'static [u16], &'static str)> {
    Some(match name {
        "X-Pro1_DSCF2131.RAF" => (
            1500,
            2400,
            12_418_576,
            &[1565, 1565, 941, 1537, 1574, 1082],
            "GGRGGB",
        ),
        "FinePix_X100_DSCF1676.RAF" => (1000, 2000, 7_466_824, &[423, 402, 447, 413], "RGRG"),
        "X-A5_DSCF0617.RAF" => (
            2000,
            3008,
            21_952_656,
            &[
                2779, 4809, 2807, 4793, 2896, 4866, 2785, 4533, 2717, 4479, 2664, 4429, 2650, 4573,
                2747, 4451,
            ],
            "RGRGRGRGRGRGRGRG",
        ),
        "X-T3_AFXT2720.RAF" => (
            2000,
            3000,
            28_233_584,
            &[1579, 1703, 2277, 1631, 1454, 2032],
            "BRGRBG",
        ),
        _ => return None,
    })
}

/// Color tolerances of the comparison with the camera's own JPEG for RAF:
/// mean rg chromaticity distance 0.08 and largest cell 0.20 (the Sony
/// values are 0.05 and 0.15; rank order and cell count are the Sony ones).
/// RAF files carry no color matrix, so one generic matrix, derived from one
/// X-Trans camera at D65, serves every model (open question OQ-8). The
/// specification states 0.08 as the bound in TV-13 and measured up to
/// 0.079 for its worst correct decode (X-Pro1, M12). The bound still
/// separates a correct decode from a wrong one: the specification measured
/// red and blue swapped in the X-Trans pattern at 0.156 to 0.256 (M8), the
/// other three Bayer phases at 0.122 to 0.279 (M9), the R and B white
/// balance levels swapped at 0.104 (M11, X-T1 and X-A5) and the transposed
/// matrix at up to 0.134 (M12). Those figures use the M12 block method
/// (M8 and M9 without the color matrix); the negative controls of the test
/// below show the separation with this method on every in-scope sample
/// (wrong readings measured at 0.22 to 0.40 mean). One parameter set
/// serves every file; nothing is fitted per file.
const RAF_CHROMA_MEAN: f64 = 0.08;
const RAF_CHROMA_MAX: f64 = 0.20;

/// Wrong readings of the color pattern of `plan` that must fail the
/// tolerances: the pattern shifted by one column, and for X-Trans the 36
/// bytes of record 0x0131 read in forward order (TV-4).
fn wrong_patterns(plan: &Plan) -> Vec<(&'static str, [u8; 36])> {
    let p = plan.pattern;
    let mut out = vec![(
        "phase +1 column",
        std::array::from_fn(|i| p[6 * (i / 6) + (i % 6 + 1) % 6]),
    )];
    if plan.layout == Layout::XTrans {
        // The record holds the pattern in reverse order (R-RAF-18).
        out.push(("forward order", std::array::from_fn(|i| p[35 - i])));
    }
    out
}

/// Every `.RAF` file in the folder named by `PANDITE_RAF_SAMPLES` (an
/// absolute path, or relative to the workspace root) decodes through the
/// sensor-data path when its variant is in scope, at the size of its crop
/// rectangle, and agrees with the file's own largest embedded JPEG by the
/// method of `arw_real_samples_match_embedded_preview`: an 8 x 8 grid of
/// cell means of the default display rendering, compared in rg
/// chromaticity after one exposure gain (the RAF tolerances above) and by
/// the Spearman rank of cell luminance (at least 0.90). Files whose white
/// balance is the generic one (R-RAF-33 source 3) are compared after one
/// global gain per channel. Negative controls: the same sample developed
/// with a wrong pattern reading must fail the tolerances, which shows that
/// they discriminate. Out-of-scope files deliver the preview. The class of
/// each file of TV-13 is checked as well, and the worked examples of the
/// Compression section at their file offsets.
#[test]
fn raf_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_RAF_SAMPLES") else {
        println!("skip: PANDITE_RAF_SAMPLES is not set");
        return;
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let folder = if Path::new(&dir).is_absolute() {
        PathBuf::from(&dir)
    } else {
        root.join(&dir)
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&folder)
        .unwrap_or_else(|e| panic!("read {}: {e}", folder.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("raf"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .RAF files in {}", folder.display());
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let r = raf(&data);
        let a = container::analyze(&r).expect("classify");
        if let Some(want) = tv13(&name) {
            assert_eq!(a.class, want, "TV-13 {name}");
        }
        let start = Instant::now();
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("RAF path");
        let took = start.elapsed();
        let Class::Sensor(variant, layout) = a.class else {
            assert_eq!(dev.origin, Origin::Preview, "{name}");
            println!(
                "{name}: {:?} out of scope, preview {}x{}",
                a.class,
                dev.image.width(),
                dev.image.height()
            );
            continue;
        };
        assert_eq!(
            dev.origin,
            Origin::Sensor(variant),
            "{name} fell back to the preview"
        );
        let info = dev.info.clone().expect("raw facts");
        let tags = a.tags.expect("tags");
        // The specified size: the raster of the FujiIFD and the crop
        // rectangle of the meta container, oriented.
        assert_eq!(info.raster, (tags.w as usize, tags.h as usize), "{name}");
        assert_eq!(info.crop, container::crop(&tags), "{name}");
        assert!(
            tags.crop_top_left.is_some() && info.crop.w < info.raster.0,
            "{name}: the crop records are used"
        );
        let (cw, ch) = output_size(info.crop, info.orientation);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (cw, ch),
            "{name}"
        );
        // R-RAF-21 and the bit layouts on the camera's own bytes.
        if let Some((row, col, offset, want, colors)) = worked_example(&name) {
            let plan = plan_of(&data);
            let x_bytes = match variant {
                Variant::V1 => 3 * col / 2,
                Variant::V2 => 7 * col / 4,
                Variant::V3 => 2 * col,
            };
            assert_eq!(
                plan.strip + row * plan.row_bytes + x_bytes,
                offset,
                "{name}"
            );
            let src = &data[offset..offset + plan.row_bytes - x_bytes];
            let mut got = vec![0u16; plan.width - col];
            match variant {
                Variant::V1 => sensor::v1_row(src, &mut got),
                Variant::V2 => sensor::v2_row(src, &mut got),
                Variant::V3 => strips::v1_row(src, &mut got),
            }
            assert_eq!(&got[..want.len()], want, "{name}");
            let names: String = (col..col + want.len())
                .map(|x| ['R', 'G', 'B'][usize::from(plan.pattern[6 * (row % 6) + x % 6])])
                .collect();
            assert_eq!(names, colors, "{name}");
        }
        // The camera's largest JPEG, decoded small and oriented by R-RAF-51.
        let pv = preview::collect(&r);
        let best = pv.candidates.first().expect("an embedded preview");
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev = preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        let balance = info.wb_source == WbSource::Generic;
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let mut line = format!(
            "{name}: {variant:?} {layout:?} {}-bit raster {}x{} crop {}x{}+{}+{} out {}x{} black {} wb {:?} decode {:.2}s (debug) | vs embedded {}x{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}",
            info.bits,
            info.raster.0,
            info.raster.1,
            info.crop.w,
            info.crop.h,
            info.crop.x,
            info.crop.y,
            cw,
            ch,
            info.black,
            info.wb_source,
            took.as_secs_f64(),
            best.width,
            best.height,
        );
        let outside = |mean: f64, max: f64, rank: f64, used: usize| {
            mean > RAF_CHROMA_MEAN || max > RAF_CHROMA_MAX || rank < RANK_MIN || used < 16
        };
        if outside(mean, max, rank, used) {
            failures.push(line.clone());
        }
        // Negative controls: a wrong pattern reading must fail.
        let plan = plan_of(&data);
        for (label, pattern) in wrong_patterns(&plan) {
            let mut wrong = plan.clone();
            wrong.pattern = pattern;
            let img = develop(&data, &wrong, &CancelToken::new(), Output::Display8)
                .expect("develop the control");
            let (m, x, k, u) = agreement(&grid_means(&img, 8, 4), &theirs, balance);
            let fails = outside(m, x, k, u);
            line.push_str(&format!(
                " | control {label}: chroma mean {m:.4} max {x:.4} rank {k:.3} cells {u} {}",
                if fails { "fails" } else { "PASSES" }
            ));
            if !fails {
                failures.push(format!("{name}: control {label} passes the tolerances"));
            }
        }
        println!("{line}");
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

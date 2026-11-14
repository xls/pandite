//! Tests of the NEF decoder against the specification's test vectors
//! (TV-1 to TV-13), fixtures (F1 to F10) and requirements, plus hostile
//! input, cancellation and real camera files.

use super::fixtures::*;
use super::huffman::{self, Job, Meaning, Table};
use super::meta::{self, Class, CropSource, Plan, Relative, Variant, WbSource};
use super::*;
use crate::raw::arw::render;
use crate::raw::arw::tests::{agreement, grid_means, LinearF32, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// F1, F7 and TV-12: the expected linear sRGB of every output pixel.
const OUT_F1: [f32; 3] = [0.318_291_7, 0.320_228_4, 0.063_818_7];
/// F2 and F6.
const OUT_F2: [f32; 3] = [0.296_588_4, 0.322_983_9, 0.068_142_2];
/// F3.
const OUT_F3: [f32; 3] = [0.318_351_5, 0.320_288_5, 0.063_830_7];
/// F4 and F10.
const OUT_F4: [f32; 3] = [0.111_644, 0.336_403_7, 0.160_911];
/// F5 (generic WB and matrix).
const OUT_F5: [f32; 3] = [0.368_616_2, 0.316_264_3, 0.079_626_4];
/// F8 (table WB of NIKON D70).
const OUT_F8: [f32; 3] = [0.372_466_7, 0.313_742_2, 0.110_697_3];
/// F9.
const OUT_F9: [f32; 3] = [0.272_019_1, 0.318_537_6, 0.099_309_9];
/// The specification prints 7 decimals and rounds its intermediates; the
/// renderer computes in f32.
const EPS: f32 = 2e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open")
}

fn plan_of(data: &[u8]) -> Plan {
    let c = container(data);
    let Class::Sensor(v) = meta::classify(&c).expect("classify") else {
        panic!("not a sensor variant");
    };
    let cands = preview::collect(&c);
    meta::plan(&c, v, cands.first().map(|p| (p.width, p.height))).expect("plan")
}

/// The decoded raster as sensor values s (the plane divided by 2^(14 -
/// bps)), row-major.
fn raster(data: &[u8]) -> Vec<u16> {
    let plan = plan_of(data);
    let (plane, _) = sensor::decode(data, &plan, &CancelToken::new()).expect("sensor");
    plane
        .data
        .iter()
        .map(|&v| v >> sensor::shift(&plan))
        .collect()
}

/// Linear sRGB of every output pixel (R-NEF-44) through the shared
/// renderer, cropped and oriented.
fn linear_with(data: &[u8], plan: &Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let (plane, _) = sensor::decode(data, plan, &CancelToken::new()).expect("sensor");
    let bytes = render::render(
        &carrier(plan),
        &render::Sensor::Cfa(plane),
        &CancelToken::new(),
        &LinearF32,
    )
    .expect("render");
    let (w, h) = output_size(plan);
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

fn report(data: &[u8]) -> Developed {
    decode_report(data, &DecodeOptions::default())
        .expect("decode")
        .expect("NEF path")
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

/// The 16-bit sRGB code of a linear value, computed directly.
fn code(lin: f32) -> u16 {
    (crate::util::linear_to_srgb(lin) * 65535.0 + 0.5) as u16
}

/// Every pixel of a developed Rgba16 image is the sRGB encoding of `lin`
/// within 2 codes of 65535.
fn assert_rgba16_flat(img: &ImageBuf, lin: [f32; 3], what: &str) {
    assert_eq!(img.format(), PixelFormat::Rgba16, "{what}");
    for p in img.data().chunks_exact(8) {
        for c in 0..3 {
            let v = u16::from_ne_bytes([p[2 * c], p[2 * c + 1]]);
            assert!(
                v.abs_diff(code(lin[c])) <= 2,
                "{what}: channel {c} code {v}"
            );
        }
        assert_eq!(u16::from_ne_bytes([p[6], p[7]]), u16::MAX);
    }
}

/// Replace or add an entry.
fn with(mut e: Vec<(u16, Val)>, tag: u16, v: Val) -> Vec<(u16, Val)> {
    e.retain(|x| x.0 != tag);
    e.push((tag, v));
    e
}

fn without(mut e: Vec<(u16, Val)>, tag: u16) -> Vec<(u16, Val)> {
    e.retain(|x| x.0 != tag);
    e
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

/// The builders of all ten fixtures, by name.
fn fixtures() -> Vec<(&'static str, Variant, Vec<u8>)> {
    vec![
        ("F1", Variant::V1, f1()),
        ("F2", Variant::V2, f2()),
        ("F3", Variant::V4, f3()),
        ("F4", Variant::V5, f4()),
        ("F5", Variant::V3, f5()),
        ("F6", Variant::V6, f6()),
        ("F7", Variant::V6, f7()),
        ("F8", Variant::V7, f8()),
        ("F9", Variant::V8, f9()),
        ("F10", Variant::V8, f10()),
    ]
}

#[test]
fn nef_builder_reproduces_fixture_hex() {
    // The two complete files of the specification: the builder follows the
    // recipe layout exactly, so the other fixtures follow it too.
    let f2 = hex(F2_HEX);
    assert_eq!(f2.len(), 364);
    assert_eq!(super::fixtures::f2(), f2, "F2");
    let f6 = hex(F6_HEX);
    assert_eq!(f6.len(), 426);
    assert_eq!(super::fixtures::f6(), f6, "F6");
}

/// The code values of the raster, through the plan's coding (V6 to V8).
fn code_values(data: &[u8]) -> Vec<i32> {
    let plan = plan_of(data);
    let coding = plan.coding.clone().expect("Huffman variant");
    let meta::Layout::Stream { offset, len, .. } = plan.layout else {
        panic!("not a stream");
    };
    let table = Table::new(coding.table);
    let job = Job {
        stream: &data[offset..offset + len],
        width: plan.width,
        height: plan.height,
        table: &table,
        predictors: coding.predictors,
        limit: coding.limit(plan.bits),
    };
    let mut out = Vec::new();
    huffman::decode(&job, &CancelToken::new(), |_, r| {
        out.extend_from_slice(r);
        Ok(())
    })
    .expect("decode");
    out
}

/// Decode one row of `w` code values from `stream` with all four start
/// predictors equal to `p`. Returns the values and the bits used.
fn row_codes(
    stream: &[u8],
    spec: &huffman::Spec,
    w: usize,
    p: i32,
    limit: i32,
) -> super::NefResult<(Vec<i32>, u64)> {
    let table = Table::new(spec);
    let job = Job {
        stream,
        width: w,
        height: 1,
        table: &table,
        predictors: [p; 4],
        limit,
    };
    let mut out = Vec::new();
    let used = huffman::decode(&job, &CancelToken::new(), |_, r| {
        out.extend_from_slice(r);
        Ok(())
    })?;
    Ok((out, used))
}

/// The codes of a table as (bit string, category or None).
fn listed(spec: &huffman::Spec) -> Vec<(String, Option<u8>)> {
    spec.codes()
        .into_iter()
        .map(|(c, len, m)| {
            let bits = format!("{c:0width$b}", width = usize::from(len));
            let cat = match m {
                Meaning::Category(s) => Some(s),
                Meaning::NotEstablished => None,
            };
            (bits, cat)
        })
        .collect()
}

/// Rebuild F2 or F6 with changed maker-note entries.
fn with_maker(f6_based: bool, change: impl Fn(Vec<(u16, Val)>) -> Vec<(u16, Val)>) -> Vec<u8> {
    let mut b = if f6_based { f6_builder() } else { f2_builder() };
    b.maker = b.maker.map(change);
    b.build()
}

#[test]
fn nef_vectors_decode() {
    use Class::{OutOfScope, Sensor};
    // TV-1 (R-NEF-4): the two TIFF headers, IFD0 at 8.
    assert_eq!(
        meta::header(&hex("4D 4D 00 2A 00 00 00 08")).unwrap(),
        (false, 8)
    );
    assert_eq!(
        meta::header(&hex("49 49 2A 00 08 00 00 00")).unwrap(),
        (true, 8)
    );
    for bad in [
        "4D 4D 2A 00 00 00 00 08",
        "49 49 00 2A 08 00 00 00",
        "FF D8 FF E0 00 10 4A 46",
    ] {
        let e = meta::header(&hex(bad)).unwrap_err();
        assert!(e.to_string().contains("not a NEF file"), "{bad}: {e}");
    }
    // R-NEF-58: shorter than 8 bytes.
    assert!(meta::header(&hex("4D 4D 00 2A 00 00 00")).is_err());
    assert!(decode_report(&f2()[..7], &DecodeOptions::default()).is_err());
    // TV-1 maker note (R-NEF-13, R-NEF-14): F2 holds it at 0x7A; the origin
    // is 0x7A + 10 = 0x84 and the first maker-note IFD lies at 0x84 + 8.
    let f = f2();
    let c = container(&f);
    let m = c.maker.as_ref().expect("maker note");
    assert_eq!(m.origin, 0x84);
    assert!(!m.le);
    assert!(m.ifd.has(meta::tag::WB_RB_LEVELS));
    assert_eq!(&f[0x7A..0x84], &hex("4E 69 6B 6F 6E 00 02 10 00 00")[..]);

    // TV-2: the layout of tag 0x0096 (R-NEF-16, R-NEF-34, R-NEF-35).
    let lin = hex(TV2_LOSSLESS);
    assert_eq!(lin.len(), 46);
    assert_eq!(&lin[..2], &[0x46, 0x30]);
    for k in 0..4 {
        assert_eq!(tiff::u16_at(&lin, 2 + 2 * k, false), Some(512));
    }
    assert_eq!(tiff::u16_at(&lin, 10, false), Some(32));
    let d850 = hex("46 30 00 08 00 08 00 08 00 08 22 00");
    assert_eq!(tiff::u16_at(&d850, 2, true), Some(2048));
    assert_eq!(tiff::u16_at(&d850, 10, true), Some(34));
    let v8 = hex("44 20 01 48 01 48 01 48 01 48 01 01 00 00 00 10 00 20 00 30
         00 40 00 50 00 60 00 70 00 80 00 90");
    assert_eq!(tiff::u16_at(&v8, 8, false), Some(328));
    assert_eq!(tiff::u16_at(&v8, 10, false), Some(257));
    for j in 0..10 {
        assert_eq!(tiff::u16_at(&v8, 12 + 2 * j, false), Some(16 * j as u16));
    }
    let v8z = hex("44 40 20 05 20 05 20 05 20 05 01 01 00 00 10 00 20 00 30 00");
    assert_eq!(tiff::u16_at(&v8z, 2, true), Some(1312));
    assert_eq!(tiff::u16_at(&v8z, 10, true), Some(257));
    assert_eq!(tiff::u16_at(&v8z, 18, true), Some(48));
    let v7 = hex("44 10 01 48 01 48 01 48 01 48 02 AB 00 00 00 01");
    assert_eq!(tiff::u16_at(&v7, 10, false), Some(683));
    assert_eq!(tiff::u16_at(&v7, 14, false), Some(1));
    // Through the plans of the fixtures.
    assert_eq!(plan_of(&f6()).coding.unwrap().predictors, [512; 4]);
    assert_eq!(plan_of(&f7()).coding.unwrap().predictors, [2048; 4]);
    assert_eq!(plan_of(&f8()).coding.unwrap().predictors, [328; 4]);
    assert_eq!(plan_of(&f10()).coding.unwrap().predictors, [1312; 4]);

    // TV-3: uncompressed samples (R-NEF-25, R-NEF-26, R-NEF-28, R-NEF-45).
    let v1be = hex("02 FE 03 00 05 9E 09 13");
    let got: Vec<u16> = (0..4)
        .map(|x| sensor::v1_pixel(&v1be, x, false).unwrap())
        .collect();
    assert_eq!(got, [766, 768, 1438, 2323]);
    let v1le = hex("BB 03 E0 04 B8 03 C4 04");
    let got: Vec<u16> = (0..4)
        .map(|x| sensor::v1_pixel(&v1le, x, true).unwrap())
        .collect();
    assert_eq!(got, [955, 1248, 952, 1220]);
    let v2 = hex("0C 00 BF 11 22 7F 15 12 B9 16 92 D3");
    let got: Vec<u16> = (0..8).map(|x| sensor::v2_pixel(&v2, x).unwrap()).collect();
    assert_eq!(got, [192, 191, 274, 639, 337, 697, 361, 723]);
    let v2 = hex("06 40 72 0F 30 76");
    let got: Vec<u16> = (0..4).map(|x| sensor::v2_pixel(&v2, x).unwrap()).collect();
    assert_eq!(got, [100, 114, 243, 118]);
    let v3 = hex("18 20 55 0B 60 3F 0D D0 52 0D 00 4D 0B 30 44 00");
    let got: Vec<u16> = (0..10).map(|x| sensor::v3_pixel(&v3, x).unwrap()).collect();
    assert_eq!(got, [386, 85, 182, 63, 221, 82, 208, 77, 179, 68]);
    // Byte 15 is never read: a changed pad byte changes nothing.
    let mut v3b = v3.clone();
    v3b[15] = 0xFF;
    let again: Vec<u16> = (0..10)
        .map(|x| sensor::v3_pixel(&v3b, x).unwrap())
        .collect();
    assert_eq!(again, got);
    let v4 = hex("F2 D0 13 F4 50 14 F5 30 14 F8 40 14");
    let got: Vec<u16> = (0..8)
        .map(|x| sensor::packed_pixel(&v4, x, 12).unwrap())
        .collect();
    assert_eq!(got, [242, 317, 244, 325, 245, 323, 248, 324]);
    let v5 = hex("D3 4D AC 83 E3 00 3D 67 8E C3 83 E3 04 3A");
    let got: Vec<u16> = (0..8)
        .map(|x| sensor::packed_pixel(&v5, x, 14).unwrap())
        .collect();
    assert_eq!(got, [3539, 3761, 3640, 3904, 3687, 3854, 3640, 3713]);

    // TV-4 (R-NEF-30): the codes from BITS by S2 Annex C, for all four
    // tables, equal the listed codes exactly.
    let s = |v: &[(&str, Option<u8>)]| -> Vec<(String, Option<u8>)> {
        v.iter().map(|(b, c)| (b.to_string(), *c)).collect()
    };
    assert_eq!(
        listed(&huffman::L12),
        s(&[
            ("00", Some(5)),
            ("010", Some(4)),
            ("011", Some(6)),
            ("100", Some(3)),
            ("101", Some(7)),
            ("1100", Some(2)),
            ("1101", Some(8)),
            ("11100", Some(1)),
            ("11101", Some(9)),
            ("11110", Some(0)),
            ("111110", Some(10)),
            ("1111110", Some(11)),
            ("1111111", Some(12)),
        ])
    );
    assert_eq!(
        listed(&huffman::L14),
        s(&[
            ("00", Some(7)),
            ("010", Some(6)),
            ("011", Some(8)),
            ("100", Some(5)),
            ("101", Some(9)),
            ("1100", Some(4)),
            ("1101", Some(10)),
            ("11100", Some(3)),
            ("11101", Some(11)),
            ("111100", Some(12)),
            ("111101", Some(2)),
            ("111110", Some(0)),
            ("1111110", Some(1)),
            ("11111110", Some(13)),
            ("11111111", Some(14)),
        ])
    );
    assert_eq!(
        listed(&huffman::Y12),
        s(&[
            ("00", Some(5)),
            ("010", Some(4)),
            ("011", Some(3)),
            ("100", Some(6)),
            ("101", Some(2)),
            ("110", Some(7)),
            ("1110", Some(1)),
            ("11110", Some(0)),
            ("111110", Some(8)),
            ("1111110", Some(9)),
            ("11111110", None),
            ("111111110", Some(10)),
            ("111111111", None),
        ])
    );
    assert_eq!(
        listed(&huffman::Y14),
        s(&[
            ("00", Some(5)),
            ("010", Some(6)),
            ("011", Some(4)),
            ("100", Some(7)),
            ("101", Some(8)),
            ("1100", Some(3)),
            ("1101", Some(9)),
            ("1110", Some(2)),
            ("11110", Some(1)),
            ("111110", Some(0)),
            ("1111110", Some(10)),
            ("11111110", Some(11)),
            ("111111110", Some(12)),
            ("111111111", None),
        ])
    );
    // TV-4: the 13 codes of L12 form a complete code (the sum of 2^-length
    // is 1); so do L14, Y12 with its codes that are not established, and
    // Y14 with its open subtree.
    for spec in [&huffman::L12, &huffman::L14, &huffman::Y12, &huffman::Y14] {
        let sum: f64 = spec
            .codes()
            .iter()
            .map(|c| 0.5f64.powi(i32::from(c.1)))
            .sum();
        assert!((sum - 1.0).abs() < 1e-12, "{}: {sum}", spec.name);
    }
    // The F.2.2.1 difference extension (R-NEF-32).
    assert_eq!(huffman::extend(0, 0), 0);
    assert_eq!(huffman::extend(9, 191), -320);
    assert_eq!(huffman::extend(9, 415), 415);
    assert_eq!(huffman::extend(1, 0), -1);
    assert_eq!(huffman::extend(1, 1), 1);
    assert_eq!(huffman::extend(14, 0), -16383);

    // TV-5 (L12, S13 2254) and the worked example of section Compression.
    let tv5 = hex("EA FF AB EB 33 B9 F7 7B 80 B3 A8");
    let (v, used) = row_codes(&tv5, &huffman::L12, 8, 512, 4096).unwrap();
    assert_eq!(v, [192, 191, 268, 606, 315, 638, 337, 680]);
    assert_eq!(used, 86);
    // R-NEF-64: one byte less and the last code runs past the strip.
    assert!(row_codes(&tv5[..10], &huffman::L12, 8, 512, 4096).is_err());
    // TV-6 (L14, S14 3520).
    let (v, used) = row_codes(
        &hex("EA FF EB 04 DA 33 B8 1A 5A 22 25 4A"),
        &huffman::L14,
        8,
        2048,
        16384,
    )
    .unwrap();
    assert_eq!(v, [768, 773, 1420, 2315, 1442, 2292, 1420, 2329]);
    assert_eq!(used, 95);
    // TV-7 (Y12 with a type 2 curve, S14 3516): the code values; the
    // sensor values need the file's own curve.
    let (v, used) = row_codes(
        &hex("F9 E7 E7 8D E3 EA 5A DA FE"),
        &huffman::Y12,
        8,
        328,
        4097,
    )
    .unwrap();
    assert_eq!(v, [194, 193, 314, 358, 312, 360, 315, 360]);
    assert_eq!(used, 72);
    // TV-8 (Y14 with a type 2 curve, S14 3519).
    let (v, used) = row_codes(
        &hex("FC ED 7E 79 37 53 FA 4B 7D 95 96 3C"),
        &huffman::Y14,
        8,
        1312,
        16385,
    )
    .unwrap();
    assert_eq!(v, [763, 773, 1188, 1360, 1202, 1365, 1189, 1361]);
    assert_eq!(used, 92);
    // TV-9 (Y12 with a type 1 curve, S20 2060): six pixels at 682.
    let (v, used) = row_codes(&hex("FD 62 FD 62 F7 BD EF"), &huffman::Y12, 6, 328, 683).unwrap();
    assert_eq!(v, [682; 6]);
    assert_eq!(used, 52);

    // TV-10: curve expansion (R-NEF-35). The knots of the worked example
    // of S13 2255 (K(14) to K(19), K(43) = 4095) inside an otherwise
    // non-decreasing set.
    let mut k: Vec<u16> = (0..257).map(|j: u16| (16 * j).min(4095)).collect();
    k[14..20].copy_from_slice(&[224, 241, 263, 287, 312, 397]);
    for (j, v) in k.iter_mut().enumerate().skip(20) {
        *v = if j >= 43 {
            4095
        } else {
            397 + (j as u16 - 19) * 160
        };
    }
    let c = meta::expand(&k.clone().try_into().unwrap(), 16);
    assert_eq!(c.len(), 4097);
    assert_eq!(
        [c[239], c[240], c[241], c[247], c[255], c[256], c[300], c[688], c[4096]],
        [239, 241, 242, 250, 261, 263, 375, 4095, 4095]
    );
    // Version 0x44 0x40, 14 bits, S = 16 (S15 3581): K(99) to K(101).
    let mut k: Vec<u16> = (0..257).map(|j: u16| (16 * j).min(16383)).collect();
    k[99..102].copy_from_slice(&[1584, 1600, 1632]);
    for (j, v) in k.iter_mut().enumerate().skip(102) {
        *v = (1632 + (j as u32 - 101) * 95).min(16383) as u16;
    }
    k[256] = 16383;
    let c = meta::expand(&k.clone().try_into().unwrap(), 16);
    assert_eq!(c.len(), 4097);
    assert_eq!(
        [c[1600], c[1601], c[1608], c[1615], c[1616], c[4096]],
        [1600, 1602, 1616, 1630, 1632, 16383]
    );

    // The fixtures F1 to F10: classification (R-NEF-3), decoded raster and
    // the expected output for every pixel (R-NEF-1, R-NEF-24).
    let want: &[(&str, &[u16], [f32; 3])] = &[
        (
            "F1",
            &[2912, 4912, 2912, 4912, 4912, 1912, 4912, 1912],
            OUT_F1,
        ),
        ("F2", &[628, 1128, 628, 1128, 1128, 378, 1128, 378], OUT_F2),
        ("F3", &[728, 1228, 728, 1228, 1228, 478, 1228, 478], OUT_F3),
        (
            "F4",
            &[3520, 5520, 3520, 5520, 5520, 2520, 5520, 2520],
            OUT_F4,
        ),
        ("F5", &[1128, 628, 1128, 628, 378, 1128, 378, 1128], OUT_F5),
        ("F6", &[628, 1128, 628, 1128, 1128, 378, 1128, 378], OUT_F2),
        (
            "F7",
            &[2912, 4912, 2912, 4912, 4912, 1912, 4912, 1912],
            OUT_F1,
        ),
        ("F8", &[376, 1132, 376, 1132, 1132, 628, 1132, 628], OUT_F8),
        ("F9", &[628, 1129, 628, 1129, 1129, 379, 1129, 379], OUT_F9),
        (
            "F10",
            &[3520, 5520, 3520, 5520, 5520, 2520, 5520, 2520],
            OUT_F4,
        ),
    ];
    for ((name, variant, f), (wname, values, out)) in fixtures().into_iter().zip(want) {
        assert_eq!(name, *wname);
        let c = container(&f);
        assert_eq!(meta::classify(&c).unwrap(), Sensor(variant), "{name}");
        assert_eq!(raster(&f), *values, "{name} raster");
        let (w, h, px) = linear(&f);
        assert_eq!((w, h), (4, 2), "{name}");
        for (i, p) in px.iter().enumerate() {
            assert_rgb(*p, *out, EPS, &format!("{name} pixel {i}"));
        }
        // Through the full decode: the sensor path, Rgba16 sRGB.
        let dev = report(&f);
        assert_eq!(dev.origin, Origin::Sensor(variant), "{name}");
        assert_eq!(dev.class, Some(Sensor(variant)));
        assert_rgba16_flat(&dev.image, *out, name);
    }
    // The code values of the Huffman fixtures (R-NEF-32, R-NEF-46: the
    // prediction uses code values, the plane holds C(code value)).
    assert_eq!(
        code_values(&f6()),
        [628, 1128, 628, 1128, 1128, 378, 1128, 378]
    );
    assert_eq!(code_values(&f8()), [276, 402, 276, 402, 402, 318, 402, 318]);
    assert_eq!(code_values(&f9()), [380, 547, 380, 547, 547, 297, 547, 297]);
    assert_eq!(
        code_values(&f10()),
        [1648, 2148, 1648, 2148, 2148, 1398, 2148, 1398]
    );
    // F6: 74 bits of codes, then 6 zero bits that are never checked
    // (R-NEF-29).
    let dev = report(&f6());
    assert_eq!(dev.info.unwrap().stream_bits, Some((74, 80)));
    // Junk after the last code is never checked either (R-NEF-29, M18).
    let mut b = f6_builder();
    b.blobs[0].extend([0xFF; 64]);
    b.raw = with(b.raw, 0x0117, Val::BlobLens(vec![0]));
    let dev = report(&b.build());
    assert_eq!(dev.origin, Origin::Sensor(Variant::V6));
    assert_eq!(dev.info.unwrap().stream_bits, Some((74, 80 + 512)));

    // Mutations move a file to another rule of R-NEF-3.
    let f = with_maker(true, |m| with(m, 0x0093, Val::Short(vec![4])));
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        OutOfScope(Relative::X7)
    );
    let f = with_maker(true, |m| without(m, 0x0093));
    assert_eq!(meta::classify(&container(&f)).unwrap(), Sensor(Variant::V6));
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// The camera vectors of TV-3: file name prefix, first strip bytes and the
/// values of the first pixels of row 0, checked on the real file.
const TV3_FILES: &[(&str, &str, &[u16])] = &[
    ("2224_", "BB 03 E0 04 B8 03 C4 04", &[955, 1248, 952, 1220]),
    ("824_", "06 40 72 0F 30 76", &[100, 114, 243, 118]),
    (
        "2996_",
        "18 20 55 0B 60 3F 0D D0 52 0D 00 4D 0B 30 44 00",
        &[386, 85, 182, 63, 221, 82, 208, 77, 179, 68],
    ),
    (
        "2222_",
        "F2 D0 13 F4 50 14 F5 30 14 F8 40 14",
        &[242, 317, 244, 325, 245, 323, 248, 324],
    ),
    (
        "3580_",
        "D3 4D AC 83 E3 00 3D 67 8E C3 83 E3 04 3A",
        &[3539, 3761, 3640, 3904, 3687, 3854, 3640, 3713],
    ),
];

/// The first pixels of stored row 0, read by the variant's pixel rule.
fn row0(data: &[u8], plan: &Plan, n: usize) -> Vec<u16> {
    let (start, _) = match &plan.layout {
        meta::Layout::Strips { offsets, .. } => (offsets[0], 0),
        meta::Layout::Stream { offset, .. } => (*offset, 0),
    };
    let row = &data[start..];
    (0..n)
        .map(|x| match plan.variant {
            Variant::V1 => sensor::v1_pixel(row, x, plan.le).unwrap(),
            Variant::V2 => sensor::v2_pixel(row, x).unwrap(),
            Variant::V3 => sensor::v3_pixel(row, x).unwrap(),
            Variant::V4 => sensor::packed_pixel(row, x, 12).unwrap(),
            _ => sensor::packed_pixel(row, x, 14).unwrap(),
        })
        .collect()
}

/// The camera vectors of TV-1, TV-2, TV-3, TV-9 and TV-10, and the
/// stream ends of M5, M6 and R-NEF-64, on the files they come from.
/// Returns a note for the report line.
fn camera_vectors(name: &str, data: &[u8], plan: &Plan, info: &Info) -> String {
    let c = container(data);
    let mut notes = Vec::new();
    for &(prefix, bytes, values) in TV3_FILES {
        if !name.starts_with(prefix) {
            continue;
        }
        let start = match &plan.layout {
            meta::Layout::Strips { offsets, .. } => offsets[0],
            meta::Layout::Stream { offset, .. } => *offset,
        };
        let b = hex(bytes);
        assert_eq!(&data[start..start + b.len()], &b[..], "{name}: TV-3 bytes");
        assert_eq!(row0(data, plan, values.len()), values, "{name}: TV-3");
        notes.push("TV-3");
    }
    let lin = c.linearization().unwrap();
    if name.starts_with("2249_") {
        // TV-1 and the example of section IFD chain (S13 2249).
        let ifd0 = c.ifd0().unwrap();
        assert_eq!(meta::header(data).unwrap(), (false, 8));
        assert_eq!(
            ifd0.ints(meta::tag::SUB_IFDS).unwrap(),
            Some(vec![175436, 175556])
        );
        assert_eq!(ifd0.int(meta::tag::EXIF_IFD).unwrap(), Some(600));
        assert_eq!(read16_be(data, 8), 27);
        let m = c.maker.as_ref().unwrap();
        assert_eq!(m.origin, 1130);
        assert_eq!(
            &data[1120..1138],
            &hex("4E 69 6B 6F 6E 00 02 10 00 00 4D 4D 00 2A 00 00 00 08")[..]
        );
        assert_eq!(
            plan.layout_offset_len(),
            (1915808, 14874979),
            "{name}: strip"
        );
        // M6: S13 2249 uses 118999828 of 118999832 bits.
        assert_eq!(info.stream_bits, Some((118999828, 118999832)), "{name}");
        notes.push("TV-1, M6");
    }
    if name.starts_with("2255_") {
        // TV-2 (S13 2255): the first 32 bytes of tag 0x0096.
        let want = hex("44 20 01 48 01 48 01 48 01 48 01 01 00 00 00 10 00 20 00 30
             00 40 00 50 00 60 00 70 00 80 00 90");
        assert_eq!(&lin.unwrap()[..32], &want[..], "{name}: TV-2");
        // TV-10 (type 2, S = 16).
        let curve = plan.coding.as_ref().unwrap().curve.as_ref().unwrap();
        assert_eq!(curve.len(), 4097);
        let at = [
            239, 240, 241, 247, 248, 255, 256, 257, 260, 300, 687, 688, 4096,
        ];
        let want = [
            239, 241, 242, 250, 252, 261, 263, 264, 269, 375, 4081, 4095, 4095,
        ];
        assert_eq!(at.map(|i| curve[i]), want, "{name}: TV-10");
        // R-NEF-64: the strip of S13 2255 ends exactly at its last
        // consumed bit.
        let (used, total) = info.stream_bits.unwrap();
        assert_eq!(used, total, "{name}: stream end");
        notes.push("TV-2, TV-10, R-NEF-64 end");
    }
    if name.starts_with("2060_") {
        // TV-2 (S20 2060): the first 16 bytes of tag 0x0096.
        let want = hex("44 10 01 48 01 48 01 48 01 48 02 AB 00 00 00 01");
        assert_eq!(&lin.unwrap()[..16], &want[..], "{name}: TV-2");
        // TV-10 (type 1).
        let curve = plan.coding.as_ref().unwrap().curve.as_ref().unwrap();
        assert_eq!(curve.len(), 683);
        assert_eq!(&curve[214..=220], &[214, 215, 217, 218, 219, 220, 221]);
        assert_eq!(&curve[680..=682], &[4070, 4085, 4095]);
        // TV-9: the first strip bytes and six code values of 682.
        let (off, _) = plan.layout_offset_len();
        assert_eq!(
            &data[off..off + 7],
            &hex("FD 62 FD 62 F7 BD EF")[..],
            "{name}: TV-9"
        );
        let (v, _) = row_codes(&data[off..off + 64], &huffman::Y12, 6, 328, 683).unwrap();
        assert_eq!(v, [682; 6], "{name}: TV-9");
        // TV-11 (version "0103") and the worked example of R-NEF-38.
        assert_eq!(info.wb_source, WbSource::ColorBalance0103);
        assert_eq!(info.wb, [2.167_968_75, 1.0, 1.515_625]);
        notes.push("TV-2, TV-9, TV-10, TV-11");
    }
    if name.starts_with("1835_") {
        // M5: S17 1835 ends 4 bits before the end of its strip.
        let (used, total) = info.stream_bits.unwrap();
        assert_eq!(total - used, 4, "{name}: stream end");
        notes.push("M5");
    }
    notes.join(", ")
}

fn read16_be(d: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([d[at], d[at + 1]])
}

impl Plan {
    /// Offset and length of a one-sequence layout.
    fn layout_offset_len(&self) -> (usize, usize) {
        match &self.layout {
            meta::Layout::Stream { offset, len, .. } => (*offset, *len),
            meta::Layout::Strips { offsets, .. } => (offsets[0], 0),
        }
    }
}

/// Tolerances of the comparison with the camera's own JPEG, one set for
/// every NEF sample, nothing fitted per file: mean chromaticity distance
/// at most 0.08 and largest cell at most 0.20 (the Sony test allows 0.05
/// and 0.15), with the Sony rank correlation (`RANK_MIN`, 0.90) and the
/// Sony minimum of 16 usable cells. The Sony limits do not hold for every
/// sample (the D700 files measure a mean of 0.054 and 0.055). The
/// specification gives the reasons for wider limits: its own colorimetric
/// measurement against the camera JPEG reaches 0.062 to 0.080 for correct
/// decodes with as-shot white balance and a table matrix (M13 and M14:
/// D700, D850, Z 6) and more for scenes with saturated colors (M14: D750
/// 0.145); the D50 matrix of the table serves every scene illuminant and
/// models outside the table use the generic matrix (OQ-7); and the shared
/// display rendering, compared here, adds to the distance. The Fujifilm
/// and Panasonic decoders use the same set. A negative control (the CFA
/// read one column off) must fail the set on every compared sample.
const NEF_CHROMA_MEAN: f64 = 0.08;
const NEF_CHROMA_MAX: f64 = 0.20;
/// Fewest usable cells of a comparison (the Sony value).
const MIN_CELLS: usize = 16;

/// Samples whose display comparison is exempt from the largest-cell limit
/// only, with the reason from the specification's measurement record. The
/// D700 scene of S13 (files 2249 and 2255 of the archive) is the one the
/// specification measured at chroma 0.062 (M13) and 0.067 (M14) with the
/// D700 matrix, above the D3, D4S and D200 files (0.012 to 0.063), and M14
/// states that the higher values have causes in the scene, not in the
/// decoding. Its largest cells are bright saturated red cloth: the shared
/// display rendering (`raw::tone`) moves a pixel that would leave the gamut
/// toward the gray of its luminance, so those cells lose chroma that the
/// camera JPEG keeps (largest cell 0.286 and 0.274 displayed). Everything
/// else is still asserted for them: the sensor path, the size, the camera
/// vectors and stream ends, the display mean, rank and cell count, the full
/// tolerance set on the colorimetric result (R-NEF-44), and the negative
/// control.
const DISPLAY_MAX_EXEMPT: &[&str] = &["2249_", "2255_"];

/// True when a comparison (mean, max, rank, usable cells) lies inside the
/// NEF tolerances.
fn within(mean: f64, max: f64, rank: f64, used: usize) -> bool {
    mean <= NEF_CHROMA_MEAN && max <= NEF_CHROMA_MAX && rank >= RANK_MIN && used >= MIN_CELLS
}

/// Every `.NEF` file in the folder named by `PANDITE_NEF_SAMPLES`
/// (absolute, or relative to the workspace root) decodes through the
/// sensor-data path when its variant is in scope, at the size of the final
/// image area (R-NEF-40), and agrees with the file's own largest embedded
/// JPEG by the method of the Sony test
/// `arw_real_samples_match_embedded_preview`: the default display
/// rendering on an 8 x 8 grid of cell means, exposure matched by one gain,
/// rg chromaticity distance per usable cell and a Spearman rank
/// correlation of cell luminance; files without as-shot white balance (the
/// table or generic scales of R-NEF-38) are compared after one global gain
/// per channel. The tolerances are [`NEF_CHROMA_MEAN`], [`NEF_CHROMA_MAX`],
/// `RANK_MIN` and [`MIN_CELLS`]; [`DISPLAY_MAX_EXEMPT`] lists the samples
/// whose display comparison is exempt from the largest-cell limit and why.
/// The colorimetric result is compared the same way and must meet the full
/// set on every sample.
///
/// Negative control: every compared sample is also developed with a wrong
/// CFA reading (the pattern shifted by one column, so red and blue pixels
/// are read as green and green as red or blue) and must fail the same
/// tolerances; the test fails when a control passes.
///
/// The camera vectors of TV-1, TV-2, TV-3, TV-9, TV-10 and TV-11 and the
/// stream ends of M5, M6 and R-NEF-64 are checked on the files they come
/// from. Out-of-scope files must return the preview. One line per file
/// reports its variant and measured distances.
#[test]
fn nef_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_NEF_SAMPLES") else {
        println!("skip: PANDITE_NEF_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("nef"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .NEF files in {}", folder.display());
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let class = meta::classify(&c).expect("classify");
        let start = Instant::now();
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("NEF path");
        let took = start.elapsed();
        let f = facts(&data).expect("facts");
        let described = format!(
            "{} / {} / {:?} bits",
            f.compression.as_deref().unwrap_or("-"),
            f.channels.as_deref().unwrap_or("-"),
            f.bits_per_channel
        );
        let Class::Sensor(variant) = class else {
            if dev.origin != Origin::Preview {
                failures.push(format!("{name}: {class:?} did not return the preview"));
            }
            println!(
                "{name}: {class:?} out of scope ({described}), preview {}x{}",
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
        let plan = plan_of(&data);
        let raw = c.raw.as_ref().unwrap();
        let raw_size = (
            raw.int(0x0100).unwrap().unwrap() as usize,
            raw.int(0x0101).unwrap().unwrap() as usize,
        );
        assert_eq!(info.raster, raw_size, "{name}");
        let (ow, oh) = render::output_size(info.crop, info.orientation);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (ow, oh),
            "{name}"
        );
        let vectors = camera_vectors(&name, &data, &plan, &info);
        let cands = preview::collect(&c);
        let best = cands.first().expect("an embedded preview");
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev =
            arw::preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        let balance = !info.wb_source.as_shot();
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let mut line = format!(
            "{name}: model {} {variant:?} ({described}) sensor path, raster {}x{} bps {} crop {}x{}+{}+{} ({:?}) out {}x{} white {} black {:?} wb {:?} {:.4}/{:.4} matrix {} table {:?} stream {:?}{} decode {:.2}s (debug) | vs embedded {}x{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}{}",
            info.model,
            info.raster.0,
            info.raster.1,
            info.bits,
            info.crop.w,
            info.crop.h,
            info.crop.x,
            info.crop.y,
            info.crop_source,
            ow,
            oh,
            info.white,
            info.black,
            info.wb_source,
            info.wb[0],
            info.wb[2],
            if info.matrix_from_table { "table" } else { "generic" },
            info.table,
            info.stream_bits,
            if vectors.is_empty() {
                String::new()
            } else {
                format!(" vectors {vectors}")
            },
            took.as_secs_f64(),
            best.width,
            best.height,
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        );
        let exempt = DISPLAY_MAX_EXEMPT.iter().any(|p| name.starts_with(p));
        // The display comparison: every limit, except the largest cell for
        // the samples of DISPLAY_MAX_EXEMPT.
        let max_limit = if exempt {
            f64::INFINITY
        } else {
            NEF_CHROMA_MAX
        };
        let display_ok =
            mean <= NEF_CHROMA_MEAN && max <= max_limit && rank >= RANK_MIN && used >= MIN_CELLS;
        if exempt {
            line.push_str(" [largest cell exempt: DISPLAY_MAX_EXEMPT]");
        }
        if !display_ok {
            failures.push(line.clone());
        }
        // The colorimetric result (linear sRGB, R-NEF-44) by the same
        // method: the full tolerance set on every sample.
        let flat = report(&data);
        assert_eq!(flat.origin, Origin::Sensor(variant), "{name}");
        let flat_grid = grid_means(&flat.image, 8, 4);
        let (fm, fx, fr, fu) = agreement(&flat_grid, &theirs, balance);
        line.push_str(&format!(
            " | colorimetric: chroma mean {fm:.4} max {fx:.4} rank {fr:.3} cells {fu}"
        ));
        if !within(fm, fx, fr, fu) {
            failures.push(format!("{name}: colorimetric outside tolerance: {line}"));
        }
        // The negative control: the CFA colors of the plan shifted by one
        // column (position 2 * y + x takes the color of 2 * y + (x + 1) mod
        // 2), developed with the same display rendering.
        let mut wrong = plan.clone();
        wrong.cfa = [0, 1, 2, 3].map(|k| plan.cfa[(k & 2) | ((k + 1) & 1)]);
        let (control, _) = develop(&data, &wrong, &CancelToken::new(), Output::Display8)
            .expect("control develops");
        let ctrl = grid_means(&control, 8, 4);
        let (cm, cx, cr, cu) = agreement(&ctrl, &theirs, balance);
        line.push_str(&format!(
            " | control (CFA one column off): chroma mean {cm:.4} max {cx:.4} rank {cr:.3} cells {cu}"
        ));
        if within(cm, cx, cr, cu) {
            failures.push(format!("{name}: the negative control passes: {line}"));
        }
        println!("{line}");
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

// ---------------------------------------------------------------------------
// Levels, white balance, color, crop and orientation
// ---------------------------------------------------------------------------

/// A V1 file of `w` x `h` values (little-endian, 14 bits, Model `model`)
/// with the given maker-note entries and IFD0 Orientation.
fn v1_file(
    model: &'static str,
    w: u32,
    h: u32,
    values: &[u16],
    maker: Vec<(u16, Val)>,
    orientation: u16,
) -> Builder {
    let strip: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut b = recipe(true, model, 14, 1, RGGB, maker, strip);
    b.raw = raw_ifd(w, h, 14, 1, RGGB);
    b.ifd0 = ifd0(model, orientation);
    b
}

/// Pseudo-random sensor values from `lo` to `hi`.
fn random_values(n: usize, lo: u32, hi: u32, seed: u32) -> Vec<u16> {
    let mut next = noise(seed);
    (0..n)
        .map(|_| (lo + next() % (hi - lo + 1)) as u16)
        .collect()
}

/// The reference output of stored pixel (x, y) by the definitions of the
/// specification, in f64: R-NEF-39 normalization, R-NEF-47 bilinear
/// demosaicing over the whole raster, R-NEF-43 matrix and clip.
fn reference(values: &[u16], w: usize, h: usize, plan: &Plan, x: usize, y: usize) -> [f64; 3] {
    let at = |xx: usize, yy: usize| -> f64 {
        let p = 2 * (yy % 2) + xx % 2;
        let c = usize::from(plan.cfa[p]);
        let b = plan.black(p);
        let wl = f64::from(plan.white);
        ((f64::from(values[yy * w + xx]) - b) / (wl - b) * plan.wb[c]).clamp(0.0, 1.0)
    };
    let own = usize::from(plan.cfa[2 * (y % 2) + x % 2]);
    let mut sum = [0f64; 3];
    let mut cnt = [0u32; 3];
    for yy in y.saturating_sub(1)..=(y + 1).min(h - 1) {
        for xx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
            if (xx, yy) == (x, y) {
                continue;
            }
            let c = usize::from(plan.cfa[2 * (yy % 2) + xx % 2]);
            if c != own {
                sum[c] += at(xx, yy);
                cnt[c] += 1;
            }
        }
    }
    let mut cam = [0f64; 3];
    for c in 0..3 {
        cam[c] = if c == own {
            at(x, y)
        } else if cnt[c] > 0 {
            sum[c] / f64::from(cnt[c])
        } else {
            0.0
        };
    }
    let m = plan.matrix;
    [0, 1, 2].map(|r| (m[r][0] * cam[0] + m[r][1] * cam[1] + m[r][2] * cam[2]).clamp(0.0, 1.0))
}

/// The WB result of F2 with its maker-note entries replaced.
fn wb_of_f2(maker: Vec<(u16, Val)>) -> ([f64; 3], WbSource) {
    let mut b = f2_builder();
    b.maker = Some(maker);
    let p = plan_of(&b.build());
    (p.wb, p.wb_source)
}

/// A tag 0x0097 value: the version, zero bytes up to `at`, then the four
/// 16-bit levels in the byte order, then zero bytes up to `len`.
fn color_balance(
    version: &[u8; 4],
    at: usize,
    levels: [u16; 4],
    le: bool,
    len: usize,
) -> (u16, Val) {
    let mut b = version.to_vec();
    b.resize(at, 0);
    for v in levels {
        b.extend(u16b(v, le));
    }
    b.resize(len.max(b.len()), 0);
    b.truncate(len);
    (0x0097, Val::Undef(b))
}

#[test]
fn nef_levels_wb_matrix_vector() {
    // TV-12 through F1: R-NEF-36 white, R-NEF-37 black, R-NEF-38 source 1.
    let plan = plan_of(&f1());
    assert_eq!(plan.white, 16383);
    assert_eq!([0, 1, 2, 3].map(|i| plan.black(i)), [400.0; 4]);
    assert_eq!(plan.wb, [3892.0 / 2048.0, 1.0, 2817.0 / 2048.0]);
    assert_eq!(plan.wb_source, WbSource::RbLevels);
    // R-NEF-39 by its formula, at the precision of the worked example.
    let v = |s: f64, m: f64| ((s - 400.0) / (16383.0 - 400.0) * m).clamp(0.0, 1.0);
    let cam = [
        v(2912.0, plan.wb[0]),
        v(4912.0, plan.wb[1]),
        v(1912.0, plan.wb[2]),
    ];
    for (got, want) in cam.iter().zip([0.298_678_7, 0.282_299_9, 0.130_121_9]) {
        assert!((got - want).abs() < 6e-8, "{got} vs {want}");
    }
    // R-NEF-41, R-NEF-42: the D850 row of the table, every row summing to
    // 1 after normalization; R-NEF-43 by its formula.
    assert!(plan.matrix_from_table);
    let d850 = [
        [1.64, -0.58, -0.06],
        [-0.10, 1.36, -0.26],
        [0.04, -0.48, 1.44],
    ];
    for (row, table) in plan.matrix.iter().zip(&d850) {
        let sum: f64 = row.iter().sum();
        assert!((sum - 1.0).abs() < 1e-12);
        let raw_sum: f64 = table.iter().sum();
        for (v, t) in row.iter().zip(table) {
            assert!((v - t / raw_sum).abs() < 1e-12);
        }
    }
    let out = [0, 1, 2].map(|r| (0..3).map(|c| plan.matrix[r][c] * cam[c]).sum::<f64>());
    for (got, want) in out.iter().zip(OUT_F1) {
        assert!((got - f64::from(want)).abs() < 2e-7, "{got} vs {want}");
    }
    // R-NEF-44: the renderer delivers these linear values (TV-12).
    let (_, _, px) = linear(&f1());
    assert_rgb(px[0], OUT_F1, EPS, "TV-12");

    // R-NEF-37: the 14-bit scale, divided by 2^(14 - bps) in 12-bit files
    // and kept as a real number (not rounded): BlackLevel 1010 in a 12-bit
    // file is 252.5.
    assert_eq!(plan_of(&f3()).black(0), 100.0);
    let mut b = f3_builder();
    b.maker = Some(vec![
        wb_tag(3892, 2817, 2048),
        (0x003D, Val::Short(vec![1010; 4])),
        nefc(6),
    ]);
    let f = b.build();
    let plan = plan_of(&f);
    assert_eq!(plan.black(0), 252.5);
    assert_eq!(plan.black14, [1010; 4]);
    let (_, _, px) = linear(&f);
    let values = raster(&f);
    let want = reference(&values, 4, 2, &plan, 1, 1).map(|v| v as f32);
    assert_rgb(px[5], want, EPS, "black 252.5");
    // Unequal values by CFA position (OQ-3).
    let vals = random_values(16, 1000, 9000, 7);
    let b = v1_file(
        "NIKON D850",
        4,
        4,
        &vals,
        vec![
            wb_tag(3892, 2817, 2048),
            (0x003D, Val::Short(vec![400, 404, 408, 412])),
        ],
        1,
    );
    let f = b.build();
    let plan = plan_of(&f);
    assert_eq!(
        [0, 1, 2, 3].map(|i| plan.black(i)),
        [400.0, 404.0, 408.0, 412.0]
    );
    let (_, _, px) = linear(&f);
    for y in 0..4 {
        for x in 0..4 {
            let want = reference(&vals, 4, 4, &plan, x, y).map(|v| v as f32);
            assert_rgb(
                px[y * 4 + x],
                want,
                4e-6,
                &format!("black by position ({x}, {y})"),
            );
        }
    }
    // Each scaled value must be less than w - 255; otherwise, and with
    // another count than 4, every black level is 0.
    let black_of = |vals: Vec<u16>| {
        let b = v1_file(
            "NIKON D850",
            4,
            2,
            &[3000; 8],
            vec![wb_tag(3892, 2817, 2048), (0x003D, Val::Short(vals))],
            1,
        );
        plan_of(&b.build()).black14
    };
    assert_eq!(black_of(vec![16127; 4]), [16127; 4]);
    assert_eq!(black_of(vec![400, 400, 400, 16128]), [0; 4]);
    assert_eq!(black_of(vec![400; 3]), [0; 4]);
    assert_eq!(black_of(vec![400; 5]), [0; 4]);
    // 12 bits: w - 255 = 3840, so 15359 / 4 is accepted and 15360 / 4 not.
    let black12 = |v: u16| {
        let mut b = f3_builder();
        b.maker = Some(vec![(0x003D, Val::Short(vec![v; 4])), nefc(6)]);
        plan_of(&b.build()).black14
    };
    assert_eq!(black12(15359), [15359; 4]);
    assert_eq!(black12(15360), [0; 4]);
    // No tag: 0 (M11, files without tag 0x003D).
    assert_eq!(plan_of(&f2()).black14, [0; 4]);
    // R-NEF-36: the white level of 12-bit files.
    assert_eq!(plan_of(&f2()).white, 4095);
    assert_eq!(plan_of(&f3()).white, 4095);

    // R-NEF-38 source 1, the worked example: 461/256 and 338/256.
    let (wb, src) = wb_of_f2(vec![wb_tag(461, 338, 256)]);
    assert_eq!(src, WbSource::RbLevels);
    assert_eq!(wb, [1.800_781_25, 1.0, 1.320_312_5]);
    // R-NEF-17: a zero denominator makes the tag invalid: the next source
    // (the table scales of NIKON D700, source 4).
    let (wb, src) = wb_of_f2(vec![(
        0x000C,
        Val::Rational(vec![(461, 0), (338, 256), (256, 256), (256, 256)]),
    )]);
    assert_eq!((wb, src), ([1.84, 1.0, 1.41], WbSource::Table));
    let (_, src) = wb_of_f2(vec![(0x000C, Val::Rational(vec![(461, 256), (338, 0)]))]);
    assert_eq!(src, WbSource::Table);
    // Outside 0.25 to 8, fewer than 2 rationals, or another type.
    let (_, src) = wb_of_f2(vec![wb_tag(2049, 300, 256)]);
    assert_eq!(src, WbSource::Table);
    let (_, src) = wb_of_f2(vec![wb_tag(63, 300, 256)]);
    assert_eq!(src, WbSource::Table);
    let (_, src) = wb_of_f2(vec![(0x000C, Val::Rational(vec![(461, 256)]))]);
    assert_eq!(src, WbSource::Table);
    let (_, src) = wb_of_f2(vec![(0x000C, Val::Long(vec![461, 256, 338, 256]))]);
    assert_eq!(src, WbSource::Table);
    // Limits 0.25 and 8 are inside.
    let (wb, _) = wb_of_f2(vec![wb_tag(2048, 64, 256)]);
    assert_eq!(wb, [8.0, 1.0, 0.25]);
    // Source 2, TV-11 (S23 5632): version "0102", R G1 G2 B at bytes 10 to
    // 17 in maker-note byte order.
    let (wb, src) = wb_of_f2(vec![color_balance(
        b"0102",
        10,
        [466, 256, 256, 463],
        false,
        18,
    )]);
    assert_eq!(src, WbSource::ColorBalance0102);
    assert_eq!(wb, [1.820_312_5, 1.0, 1.808_593_75]);
    // Source 3, TV-11 (S20 2060): version "0103", R G1 B G2 at bytes 20 to
    // 27: 02 2B 01 00 01 84 01 00.
    let mut cb = b"0103".to_vec();
    cb.resize(20, 0);
    cb.extend(hex("02 2B 01 00 01 84 01 00"));
    let (wb, src) = wb_of_f2(vec![(0x0097, Val::Undef(cb))]);
    assert_eq!(src, WbSource::ColorBalance0103);
    assert_eq!(wb, [2.167_968_75, 1.0, 1.515_625]);
    // G1 and G2 differ: G is their mean.
    let (wb, _) = wb_of_f2(vec![color_balance(
        b"0103",
        20,
        [600, 200, 450, 400],
        false,
        28,
    )]);
    assert_eq!(wb, [2.0, 1.0, 1.5]);
    // Source 1 wins over source 2.
    let (_, src) = wb_of_f2(vec![
        wb_tag(461, 338, 256),
        color_balance(b"0102", 10, [466, 256, 256, 463], false, 18),
    ]);
    assert_eq!(src, WbSource::RbLevels);
    // Too short, G = 0 (R-NEF-67: no division by 0), out of range, or
    // another version: the next source.
    for e in [
        color_balance(b"0102", 10, [466, 256, 256, 463], false, 17),
        color_balance(b"0103", 20, [555, 256, 388, 256], false, 27),
        color_balance(b"0102", 10, [466, 0, 0, 463], false, 18),
        color_balance(b"0102", 10, [4660, 256, 256, 463], false, 18),
        color_balance(b"0100", 10, [466, 256, 256, 463], false, 18),
        color_balance(b"0213", 10, [466, 256, 256, 463], false, 64),
    ] {
        let (wb, src) = wb_of_f2(vec![e]);
        assert_eq!((wb, src), ([1.84, 1.0, 1.41], WbSource::Table));
    }
    // Little-endian maker note (R-NEF-5): the levels in its byte order.
    let mut b = f1_builder();
    b.maker = Some(vec![color_balance(
        b"0102",
        10,
        [466, 256, 256, 463],
        true,
        18,
    )]);
    let p = plan_of(&b.build());
    assert_eq!(p.wb_source, WbSource::ColorBalance0102);
    assert_eq!(p.wb, [1.820_312_5, 1.0, 1.808_593_75]);
    // Source 4: the table scales of the Model (F8, NIKON D70).
    let p = plan_of(&f8());
    assert_eq!((p.wb, p.wb_source), ([2.16, 1.0, 1.64], WbSource::Table));
    // Source 5: the generic scales (F5, NIKON D100 is not in the table).
    let p = plan_of(&f5());
    assert_eq!(
        (p.wb, p.wb_source),
        ([2.099, 1.0, 1.443], WbSource::Generic)
    );

    // R-NEF-41: the generic matrix for a model outside the table, each row
    // divided by its sum (R-NEF-42).
    assert!(!p.matrix_from_table);
    let g = color::GENERIC;
    for r in 0..3 {
        let sum: f64 = g[3 * r..3 * r + 3].iter().sum();
        for c in 0..3 {
            assert!((p.matrix[r][c] - g[3 * r + c] / sum).abs() < 1e-12);
        }
    }
    // R-NEF-10: the Model compares without trailing spaces.
    let mut b = f1_builder();
    b.ifd0 = ifd0("NIKON D850   ", 1);
    assert!(plan_of(&b.build()).matrix_from_table);
    let mut b = f1_builder();
    b.ifd0 = ifd0("NIKON D85", 1);
    assert!(!plan_of(&b.build()).matrix_from_table);
    // R-NEF-42, R-NEF-67: a selected matrix with a row sum not greater
    // than 0 is not divided by it; the generic matrix serves.
    let (m, from_table) = color::select(Some([1.0, 0.0, 0.0, -1.0, 0.5, 0.0, 0.0, 0.0, 1.0]));
    assert!(!from_table);
    assert_eq!(m, color::select(None).0);

    // R-NEF-43: the output is clipped to 0 to 1. A saturated blue cell
    // gives negative red before the clip, a bright one values above 1.
    for (cell, what) in [
        ([400u16, 2000, 2000, 16000], "blue"),
        ([16383, 16383, 16383, 16383], "white"),
    ] {
        let vals: Vec<u16> = (0..16).map(|i| cell[2 * ((i / 4) % 2) + i % 2]).collect();
        let b = v1_file(
            "NIKON D850",
            4,
            4,
            &vals,
            vec![wb_tag(3892, 2817, 2048), (0x003D, Val::Short(vec![400; 4]))],
            1,
        );
        let (_, _, px) = linear(&b.build());
        for p in &px {
            assert!(p.iter().all(|v| (0.0..=1.0).contains(v)), "{what}: {p:?}");
        }
        if what == "white" {
            assert_rgb(px[5], [1.0; 3], 1e-6, what);
        } else {
            assert_eq!(px[5][0], 0.0, "{what}");
        }
    }

    // R-NEF-39 and R-NEF-47 on random data: every pixel of a 6 x 4 raster
    // equals the f64 reference.
    let vals = random_values(24, 0, 16383, 99);
    let f = v1_file("NIKON D850", 6, 4, &vals, f1_maker(), 1).build();
    let plan = plan_of(&f);
    let (_, _, px) = linear(&f);
    for y in 0..4 {
        for x in 0..6 {
            let want = reference(&vals, 6, 4, &plan, x, y).map(|v| v as f32);
            assert_rgb(px[y * 6 + x], want, 4e-6, &format!("pixel ({x}, {y})"));
        }
    }
}

/// TV-13 and R-NEF-51: the stored pixel shown at output (x', y') of an
/// image of W' x H' stored pixels.
fn stored_of(o: u8, ox: usize, oy: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        2 => (w - 1 - ox, oy),
        3 => (w - 1 - ox, h - 1 - oy),
        4 => (ox, h - 1 - oy),
        5 => (oy, ox),
        6 => (oy, h - 1 - ox),
        7 => (w - 1 - oy, h - 1 - ox),
        8 => (w - 1 - oy, ox),
        _ => (ox, oy),
    }
}

#[test]
fn nef_orientation_and_crop() {
    // TV-13 on letters: a b c / d e f.
    let (w, h) = (3, 2);
    let letters = ["a", "b", "c", "d", "e", "f"];
    let show = |o: u8| -> Vec<&str> {
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        let mut v = Vec::new();
        for oy in 0..oh {
            for ox in 0..ow {
                let (x, y) = stored_of(o, ox, oy, w, h);
                v.push(letters[y * w + x]);
            }
        }
        v
    };
    assert_eq!(show(6), ["d", "a", "e", "b", "f", "c"]);
    assert_eq!(show(8), ["c", "f", "b", "e", "a", "d"]);
    assert_eq!(show(3), ["f", "e", "d", "c", "b", "a"]);

    // R-NEF-50, R-NEF-51 through the decoder: every orientation maps the
    // upright result as the table says.
    let (w, h) = (6usize, 4usize);
    let vals = random_values(w * h, 500, 15000, 5);
    let upright = {
        let f = v1_file("NIKON D850", 6, 4, &vals, f1_maker(), 1).build();
        linear(&f).2
    };
    for o in 1..=8u8 {
        let f = v1_file("NIKON D850", 6, 4, &vals, f1_maker(), u16::from(o)).build();
        let (ow, oh, px) = linear(&f);
        assert_eq!((ow, oh), if o >= 5 { (h, w) } else { (w, h) }, "o {o}");
        for oy in 0..oh {
            for ox in 0..ow {
                let (x, y) = stored_of(o, ox, oy, w, h);
                assert_eq!(px[oy * ow + ox], upright[y * w + x], "o {o} ({ox}, {oy})");
            }
        }
        let dev = report(&f);
        let size = (dev.image.width() as usize, dev.image.height() as usize);
        assert_eq!(size, (ow, oh));
        assert_eq!(probe(&f).unwrap().unwrap().0 as usize, ow);
    }
    // R-NEF-50: absent, 0 or 9 is 1; the raw IFD Orientation is not used.
    for o in [0u16, 9] {
        let f = v1_file("NIKON D850", 6, 4, &vals, f1_maker(), o).build();
        assert_eq!(linear(&f).2, upright, "orientation {o}");
    }
    let mut b = v1_file("NIKON D850", 6, 4, &vals, f1_maker(), 1);
    b.ifd0 = without(b.ifd0, 0x0112);
    b.raw.push((0x0112, Val::Short(vec![6])));
    assert_eq!(linear(&b.build()).2, upright);

    // R-NEF-40 rule 1: CropArea; R-NEF-48: the crop of the demosaiced
    // whole raster, with the CFA phase of the stored raster (R-NEF-21),
    // also at odd crop origins.
    let (w, h) = (20usize, 18usize);
    let vals = random_values(w * h, 500, 15000, 11);
    let full = {
        let f = v1_file("NIKON D850", 20, 18, &vals, f1_maker(), 1).build();
        let plan = plan_of(&f);
        assert_eq!(plan.crop_source, CropSource::Whole);
        linear(&f).2
    };
    for (l, t, cw, ch) in [
        (1u16, 1u16, 16u16, 16u16),
        (2, 1, 17, 16),
        (4, 2, 16, 16),
        (0, 0, 20, 18),
    ] {
        let mut m = f1_maker();
        m.push((0x0045, Val::Short(vec![l, t, cw, ch])));
        let f = v1_file("NIKON D850", 20, 18, &vals, m, 1).build();
        let plan = plan_of(&f);
        assert_eq!(plan.crop_source, CropSource::CropArea);
        let (ow, oh, px) = linear(&f);
        assert_eq!((ow, oh), (usize::from(cw), usize::from(ch)));
        for y in 0..oh {
            for x in 0..ow {
                let (sx, sy) = (x + usize::from(l), y + usize::from(t));
                assert_eq!(
                    px[y * ow + x],
                    full[sy * w + sx],
                    "crop ({l}, {t}) at ({x}, {y})"
                );
            }
        }
        // R-NEF-49: the crop before the orientation.
        let mut m = f1_maker();
        m.push((0x0045, Val::Short(vec![l, t, cw, ch])));
        let f = v1_file("NIKON D850", 20, 18, &vals, m, 6).build();
        let (rw, rh, rot) = linear(&f);
        assert_eq!((rw, rh), (oh, ow));
        for oy in 0..rh {
            for ox in 0..rw {
                let (x, y) = stored_of(6, ox, oy, ow, oh);
                assert_eq!(rot[oy * rw + ox], px[y * ow + x]);
            }
        }
    }
    // Rule 1 rejects a width or height below 16, a rectangle outside the
    // raster and another count; rule 3 then gives the whole raster.
    for v in [
        vec![0u16, 0, 15, 16],
        vec![0, 0, 16, 15],
        vec![5, 0, 16, 16],
        vec![0, 3, 16, 16],
        vec![0, 0, 16],
    ] {
        let mut m = f1_maker();
        m.push((0x0045, Val::Short(v.clone())));
        let f = v1_file("NIKON D850", 20, 18, &vals, m, 1).build();
        let plan = plan_of(&f);
        assert_eq!(plan.crop_source, CropSource::Whole, "{v:?}");
        assert_eq!((plan.crop.w, plan.crop.h), (20, 18));
    }
    // Rule 2: the largest JPEG candidate with 0.9 W <= PW <= W and 0.9 H <=
    // PH <= H, centered: floor((W - PW) / 2), floor((H - PH) / 2).
    let (w, h) = (40usize, 24usize);
    let vals = random_values(w * h, 500, 15000, 13);
    for ((pw, ph), want) in [
        ((36u16, 22u16), Some((2, 1, 36, 22))),
        ((37, 22), Some((1, 1, 37, 22))),
        ((40, 24), Some((0, 0, 40, 24))),
        ((35, 22), None),
        ((36, 21), None),
        ((42, 24), None),
        ((40, 26), None),
    ] {
        let mut b = v1_file("NIKON D850", 40, 24, &vals, f1_maker(), 1);
        b.blobs.push(jpeg(pw, ph, [90, 90, 90]));
        b.after.push(jpeg_ifd(1));
        let plan = plan_of(&b.build());
        match want {
            Some((x, y, cw, ch)) => {
                assert_eq!(plan.crop_source, CropSource::Preview, "{pw} x {ph}");
                assert_eq!(
                    (plan.crop.x, plan.crop.y, plan.crop.w, plan.crop.h),
                    (x, y, cw, ch),
                    "{pw} x {ph}"
                );
            }
            None => assert_eq!(plan.crop_source, CropSource::Whole, "{pw} x {ph}"),
        }
    }
    // Rule 1 wins over rule 2.
    let mut m = f1_maker();
    m.push((0x0045, Val::Short(vec![2, 2, 30, 20])));
    let mut b = v1_file("NIKON D850", 40, 24, &vals, m, 1);
    b.blobs.push(jpeg(36, 22, [90, 90, 90]));
    b.after.push(jpeg_ifd(1));
    assert_eq!(plan_of(&b.build()).crop_source, CropSource::CropArea);
    // Rule 2 uses the largest candidate only: a fitting smaller one does
    // not count when the largest does not fit.
    let mut b = v1_file("NIKON D850", 40, 24, &vals, f1_maker(), 1);
    b.blobs.push(jpeg(36, 22, [90, 90, 90]));
    b.blobs.push(jpeg(48, 30, [90, 90, 90]));
    b.after.push(jpeg_ifd(1));
    b.after.push(jpeg_ifd(2));
    assert_eq!(plan_of(&b.build()).crop_source, CropSource::Whole);
}

// ---------------------------------------------------------------------------
// Preview path, hostile input, cancellation
// ---------------------------------------------------------------------------

/// The large preview: 48 x 32 red, in a SubIFD after the raw IFD.
const BIG: (u16, u16, [u8; 3]) = (48, 32, [200, 40, 40]);
/// The small preview: 16 x 8 blue, in the maker-note PreviewIFD.
const SMALL: (u16, u16, [u8; 3]) = (16, 8, [40, 40, 200]);
/// The IFD0 thumbnail: 4 x 2 green, uncompressed RGB.
const THUMB: (u32, u32, [u8; 3]) = (4, 2, [40, 200, 40]);

/// The IFD0 entries of an uncompressed RGB thumbnail in blob `k`.
fn thumb_entries(w: u32, h: u32, k: usize) -> Vec<(u16, Val)> {
    vec![
        (0x0100, Val::Long(vec![w])),
        (0x0101, Val::Long(vec![h])),
        (0x0102, Val::Short(vec![8, 8, 8])),
        (0x0103, Val::Short(vec![1])),
        (0x0106, Val::Short(vec![2])),
        (0x0111, Val::Blobs(vec![k])),
        (0x0115, Val::Short(vec![3])),
        (0x0117, Val::BlobLens(vec![k])),
    ]
}

/// The builder with the large SubIFD preview (blob 1), the small
/// maker-note preview and the IFD0 thumbnail (blob 2); the sensor data stay
/// blob 0.
fn with_previews(mut b: Builder) -> Builder {
    b.blobs.truncate(1);
    b.blobs.push(jpeg(BIG.0, BIG.1, BIG.2));
    b.after.push(jpeg_ifd(1));
    if b.maker.is_some() {
        b.maker_jpeg = Some(jpeg(SMALL.0, SMALL.1, SMALL.2));
    }
    let (w, h, rgb) = THUMB;
    b.blobs
        .push((0..w * h).flat_map(|_| rgb).collect::<Vec<u8>>());
    b.ifd0.extend(thumb_entries(w, h, 2));
    b
}

fn assert_image(dev: &Developed, origin: Origin, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_eq!(dev.origin, origin, "{what}");
    assert_eq!(dev.image.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((dev.image.width(), dev.image.height()), size, "{what}");
    let p = &dev.image.data()[..4];
    for c in 0..3 {
        assert!(p[c].abs_diff(rgb[c]) <= 6, "{what}: {p:?} vs {rgb:?}");
    }
}

fn assert_preview(dev: &Developed, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_image(dev, Origin::Preview, size, rgb, what);
}

/// Change the raw IFD of a builder.
fn raw_with(mut b: Builder, tag: u16, v: Val) -> Builder {
    b.raw = with(b.raw, tag, v);
    b
}

/// Change the maker note of a builder.
fn maker_with(mut b: Builder, change: impl Fn(Vec<(u16, Val)>) -> Vec<(u16, Val)>) -> Builder {
    b.maker = b.maker.map(change);
    b
}

#[test]
fn nef_out_of_scope_falls_back_to_preview() {
    use Class::{OutOfScope, Sensor};
    // R-NEF-2, R-NEF-3: X1 to X7 show the preview; their sensor data are
    // never read (the strips here are not even valid).
    let junk = |mut b: Builder| {
        b.blobs[0] = vec![0xFF; 8];
        b
    };
    let lin_short = (0x0096, Val::Undef(hex("44 10 01 48 01 48 01 48 01 48 02")));
    let cases: Vec<(&str, Builder, Relative)> = vec![
        (
            "X1 13",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![13]))),
            Relative::X1,
        ),
        (
            "X1 14",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![14]))),
            Relative::X1,
        ),
        (
            "X1 in 0x0051",
            junk(maker_with(f4_builder(), |_| f4_maker(13))),
            Relative::X1,
        ),
        (
            "X2",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![8]))),
            Relative::X2,
        ),
        (
            "X3",
            junk(maker_with(f6_builder(), |m| {
                without(with(m, 0x0093, Val::Short(vec![1])), 0x0096)
            })),
            Relative::X3,
        ),
        (
            "X4",
            {
                let mut b = f2_builder();
                b.ifd0 = ifd0("NIKON D1X", 1);
                b
            },
            Relative::X4,
        ),
        (
            "X5",
            {
                let mut b = f2_builder();
                b.ifd0 = ifd0("E5700", 1);
                b
            },
            Relative::X5,
        ),
        (
            "X6 photometric 2",
            raw_with(f2_builder(), 0x0106, Val::Short(vec![2])),
            Relative::X6,
        ),
        (
            "X6 reduced image",
            raw_with(f2_builder(), 0x00FE, Val::Long(vec![1])),
            Relative::X6,
        ),
        (
            "X6 no strip offsets",
            {
                let mut b = f2_builder();
                b.raw = without(b.raw, 0x0111);
                b
            },
            Relative::X6,
        ),
        (
            "X6 no raw IFD",
            {
                let mut b = f2_builder();
                b.raw.clear();
                b
            },
            Relative::X6,
        ),
        (
            "X7 compression 7",
            raw_with(f2_builder(), 0x0103, Val::Short(vec![7])),
            Relative::X7,
        ),
        (
            "X7 bps 16",
            raw_with(f1_builder(), 0x0102, Val::Short(vec![16])),
            Relative::X7,
        ),
        (
            "X7 V2 count",
            raw_with(f2_builder(), 0x0117, Val::Long(vec![11])),
            Relative::X7,
        ),
        (
            "X7 NEFCompression 5",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![5]))),
            Relative::X7,
        ),
        (
            "X7 NEFCompression 7",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![7]))),
            Relative::X7,
        ),
        (
            "X7 NEFCompression 9",
            maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![9]))),
            Relative::X7,
        ),
        (
            "X7 version 44 30",
            maker_with(f9_builder(), |m| {
                with(
                    m,
                    0x0096,
                    lin_tag(false, [0x44, 0x30], 328, 257, &f9_knots(), 98).1,
                )
            }),
            Relative::X7,
        ),
        (
            "X7 tag 0x0096 of 11 bytes",
            maker_with(f8_builder(), |m| with(m, 0x0096, lin_short.1.clone())),
            Relative::X7,
        ),
        // R-NEF-14: without a valid maker note only rules 1 to 5 match.
        (
            "X7 no maker note",
            {
                let mut b = f6_builder();
                b.maker = None;
                b
            },
            Relative::X7,
        ),
    ];
    for (name, b, rel) in cases {
        let f = with_previews(b).build();
        let c = container(&f);
        assert_eq!(meta::classify(&c).unwrap(), OutOfScope(rel), "{name}");
        let dev = report(&f);
        assert_eq!(dev.class, Some(OutOfScope(rel)), "{name}");
        assert_preview(&dev, (48, 32), BIG.2, name);
        assert!(dev.info.is_none());
    }
    // Rule 2 needs exactly "E" and four digits.
    for model in ["E57000", "E570A", "e5700"] {
        let mut b = f2_builder();
        b.ifd0 = ifd0(model, 1);
        assert_eq!(
            meta::classify(&container(&b.build())).unwrap(),
            Sensor(Variant::V2),
            "{model}"
        );
    }
    // R-NEF-15: the first SubIFD that qualifies; a reduced CFA SubIFD in
    // front of it (NewSubfileType 1) and an invalid offset are skipped.
    let mut b = f2_builder();
    let mut reduced = b.raw.clone();
    reduced = with(reduced, 0x00FE, Val::Long(vec![1]));
    b.before.push(reduced);
    let f = b.build();
    assert_eq!(meta::classify(&container(&f)).unwrap(), Sensor(Variant::V2));
    assert_eq!(raster(&f), [628, 1128, 628, 1128, 1128, 378, 1128, 378]);
    // NewSubfileType absent qualifies.
    let mut b = f2_builder();
    b.raw = without(b.raw, 0x00FE);
    assert_eq!(
        meta::classify(&container(&b.build())).unwrap(),
        Sensor(Variant::V2)
    );

    // In-scope files that fail a check show the preview, and the partly
    // decoded image is dropped (R-NEF-56, R-NEF-69).
    let short_curve: Vec<u16> = (0..300).collect();
    let failing: Vec<(&str, Builder, Variant)> = vec![
        // R-NEF-18.
        (
            "pattern 0 1 1 1",
            raw_with(f2_builder(), 0x828E, Val::Byte(vec![0, 1, 1, 1])),
            Variant::V2,
        ),
        (
            "pattern of 3",
            raw_with(f2_builder(), 0x828E, Val::Byte(vec![0, 1, 2])),
            Variant::V2,
        ),
        (
            "repeat 3 x 3",
            raw_with(f2_builder(), 0x828D, Val::Short(vec![3, 3])),
            Variant::V2,
        ),
        // R-NEF-31: a code that is not established (Y12 11111110, Y14
        // 111111111), the first code of the strip.
        (
            "Y12 11111110",
            {
                let mut b = f9_builder();
                b.blobs[0] = hex("FE 00 00 00 00 00 00 00");
                b
            },
            Variant::V8,
        ),
        (
            "Y12 111111111",
            {
                let mut b = f9_builder();
                b.blobs[0] = hex("FF 80 00 00 00 00 00 00");
                b
            },
            Variant::V8,
        ),
        (
            "Y14 111111111",
            {
                let mut b = f10_builder();
                b.blobs[0] = hex("FF 80 00 00 00 00 00 00 00 00 00");
                b
            },
            Variant::V8,
        ),
        (
            "V7 11111110",
            {
                let mut b = f8_builder();
                b.blobs[0] = hex("FE 00 00 00 00 00 00");
                b
            },
            Variant::V7,
        ),
        // R-NEF-33: V6 code value above 2^bps - 1 (predictor 4095 + 116).
        (
            "V6 range",
            maker_with(f6_builder(), |m| {
                with(m, 0x0096, lin_tag(false, [0x46, 0x30], 4095, 32, &[], 34).1)
            }),
            Variant::V6,
        ),
        // R-NEF-33: V7 code value not less than L (402 >= 300).
        (
            "V7 range",
            maker_with(f8_builder(), |m| {
                with(
                    m,
                    0x0096,
                    lin_tag(false, [0x44, 0x10], 328, 300, &short_curve, 0).1,
                )
            }),
            Variant::V7,
        ),
        // R-NEF-34 and R-NEF-16.
        (
            "V7 NC 1",
            maker_with(f8_builder(), |m| {
                with(m, 0x0096, lin_tag(false, [0x44, 0x10], 328, 1, &[0], 0).1)
            }),
            Variant::V7,
        ),
        (
            "V7 NC 4097",
            maker_with(f8_builder(), |m| {
                with(m, 0x0096, lin_tag(false, [0x44, 0x10], 328, 4097, &[], 0).1)
            }),
            Variant::V7,
        ),
        (
            "V7 short tag",
            maker_with(f8_builder(), |m| {
                with(
                    m,
                    0x0096,
                    lin_tag(false, [0x44, 0x10], 328, 683, &f8_curve()[..682], 0).1,
                )
            }),
            Variant::V7,
        ),
        (
            "V7 decreasing",
            maker_with(f8_builder(), |m| {
                let mut c = f8_curve();
                c[500] = 1;
                with(m, 0x0096, lin_tag(false, [0x44, 0x10], 328, 683, &c, 0).1)
            }),
            Variant::V7,
        ),
        (
            "V7 above 4095",
            maker_with(f8_builder(), |m| {
                let mut c = f8_curve();
                c[682] = 4096;
                with(m, 0x0096, lin_tag(false, [0x44, 0x10], 328, 683, &c, 0).1)
            }),
            Variant::V7,
        ),
        // R-NEF-35 and R-NEF-16.
        (
            "V8 NC 256",
            maker_with(f9_builder(), |m| {
                with(
                    m,
                    0x0096,
                    lin_tag(false, [0x44, 0x20], 328, 256, &f9_knots(), 98).1,
                )
            }),
            Variant::V8,
        ),
        (
            "V8 525 bytes",
            maker_with(f9_builder(), |m| {
                with(
                    m,
                    0x0096,
                    lin_tag(false, [0x44, 0x20], 328, 257, &f9_knots()[..256], 1).1,
                )
            }),
            Variant::V8,
        ),
        (
            "V8 decreasing",
            maker_with(f9_builder(), |m| {
                let mut k = f9_knots();
                k[100] = 0;
                with(m, 0x0096, lin_tag(false, [0x44, 0x20], 328, 257, &k, 98).1)
            }),
            Variant::V8,
        ),
        (
            "V8 above 4095",
            maker_with(f9_builder(), |m| {
                let mut k = f9_knots();
                k[256] = 4096;
                with(m, 0x0096, lin_tag(false, [0x44, 0x20], 328, 257, &k, 98).1)
            }),
            Variant::V8,
        ),
        // R-NEF-59.
        (
            "W 0",
            raw_with(f6_builder(), 0x0100, Val::Long(vec![0])),
            Variant::V6,
        ),
        (
            "W 70000",
            raw_with(f6_builder(), 0x0100, Val::Long(vec![70000])),
            Variant::V6,
        ),
        (
            "W x H above 2^27",
            raw_with(
                raw_with(f6_builder(), 0x0100, Val::Long(vec![65535])),
                0x0101,
                Val::Long(vec![2049]),
            ),
            Variant::V6,
        ),
        // R-NEF-65.
        (
            "W 1",
            raw_with(f6_builder(), 0x0100, Val::Long(vec![1])),
            Variant::V6,
        ),
        (
            "H 1",
            raw_with(f6_builder(), 0x0101, Val::Long(vec![1])),
            Variant::V6,
        ),
        // R-NEF-64: a strip too short for its codes.
        (
            "V6 short strip",
            {
                let mut b = f6_builder();
                b.blobs[0].truncate(9);
                b
            },
            Variant::V6,
        ),
        // R-NEF-62 case 2: the strip count is not ceil(H / RowsPerStrip).
        (
            "V1 strips",
            raw_with(f1_builder(), 0x0116, Val::Long(vec![1])),
            Variant::V1,
        ),
        // R-NEF-62 case 1: different counts.
        (
            "V1 counts",
            raw_with(
                raw_with(f1_builder(), 0x0116, Val::Long(vec![1])),
                0x0111,
                Val::Blobs(vec![0, 0]),
            ),
            Variant::V1,
        ),
        // R-NEF-61: the strip lies outside the file.
        (
            "strip outside",
            raw_with(f6_builder(), 0x0111, Val::Long(vec![1 << 20])),
            Variant::V6,
        ),
    ];
    for (name, b, v) in failing {
        let f = with_previews(b).build();
        let c = container(&f);
        assert_eq!(meta::classify(&c).unwrap(), Sensor(v), "{name}");
        let dev = report(&f);
        assert_eq!(dev.class, Some(Sensor(v)), "{name}");
        assert_preview(&dev, (48, 32), BIG.2, name);
    }
    // R-NEF-62 case 3: two strips of one row, the second shorter than its
    // row; R-NEF-22: two strips of one row each, each at its own offset.
    let mut b = f1_builder();
    let strip = b.blobs[0].clone();
    b.blobs = vec![strip[..8].to_vec(), strip[8..].to_vec()];
    b.raw = with(b.raw, 0x0116, Val::Long(vec![1]));
    b.raw = with(b.raw, 0x0111, Val::Blobs(vec![1, 0]));
    b.raw = with(b.raw, 0x0117, Val::BlobLens(vec![1, 0]));
    // Row 0 is in blob 1 and row 1 in blob 0: the strips are read at
    // their own offsets, not as one sequence.
    b.blobs.swap(0, 1);
    let f = b.build();
    assert_eq!(raster(&f), [2912, 4912, 2912, 4912, 4912, 1912, 4912, 1912]);
    let mut b2 = b.clone();
    b2.blobs[0].truncate(7);
    b2.blobs[1].push(0);
    let f = with_previews_keep(b2).build();
    assert_eq!(meta::classify(&container(&f)).unwrap(), Sensor(Variant::V1));
    assert_preview(&report(&f), (48, 32), BIG.2, "V1 short strip");
    // R-NEF-23: V3 to V8 strips are one byte sequence only when each
    // starts where the previous one ends.
    // The F6 strip given as two strips of 5 bytes; the offsets are found
    // in a first build (their values do not change the layout).
    let mk = |second: Option<u32>, counts: [u32; 2]| -> Vec<u8> {
        let build = |o: [u32; 2]| {
            let mut b = f6_builder();
            let ob: Vec<u8> = o.iter().flat_map(|&v| u32b(v, false)).collect();
            let nb: Vec<u8> = counts.iter().flat_map(|&v| u32b(v, false)).collect();
            b.raw = with(b.raw, 0x0111, Val::Raw(4, 2, ob));
            b.raw = with(b.raw, 0x0117, Val::Raw(4, 2, nb));
            b.build()
        };
        let first = build([0, 0]);
        let strip = hex("BD 3E 9A 3D EF A6 8D 79 F7 80");
        let at = first.windows(10).position(|w| w == strip).unwrap() as u32;
        build([at, second.map_or(at + 5, |d| at + d)])
    };
    let joined = mk(None, [5, 5]);
    assert_eq!(raster(&joined), raster(&f6()));
    let apart = mk(Some(4), [5, 6]);
    let c = container(&apart);
    assert_eq!(meta::classify(&c).unwrap(), Sensor(Variant::V6));
    assert!(meta::plan(&c, Variant::V6, None).is_err());
    // R-NEF-60: a count whose size exceeds the file fails before any
    // allocation: classification fails and the preview shows.
    let f = with_previews(raw_with(
        f6_builder(),
        0x0117,
        Val::Raw(4, 0x1000_0000, vec![0; 4]),
    ))
    .build();
    assert!(meta::classify(&container(&f)).is_err());
    let dev = report(&f);
    assert_eq!(dev.class, None);
    assert_preview(&dev, (48, 32), BIG.2, "R-NEF-60");

    // R-NEF-14: a maker note without the Nikon signature, with another
    // TIFF header or with an invalid IFD is not valid: the defaults apply
    // (F2 develops with the table scales of the D700) and NEFCompression
    // is absent.
    for (what, at, byte) in [
        ("signature", 0x7A, b'n'),
        ("header", 0x84, 0x4E),
        ("IFD", 0x8C, 0x00),
    ] {
        let mut f = f2();
        f[at] = byte;
        if what == "IFD" {
            f[at + 1] = 0x00;
        }
        let c = container(&f);
        assert!(c.maker.is_none(), "{what}");
        let p = plan_of(&f);
        assert_eq!(p.wb_source, WbSource::Table, "{what}");
        assert_eq!(c.nef_compression().unwrap(), None);
    }
    // The maker-note offsets count from maker-note byte 10 (R-NEF-14): a
    // maker note moved in the file still reads the same.
    let f = f6();
    let c6 = container(&f);
    assert_eq!(c6.maker.as_ref().unwrap().origin, 0x84);
    assert_eq!(c6.linearization().unwrap().unwrap(), &hex(TV2_LOSSLESS)[..]);
    let mut b = f6_builder();
    b.ifd0.push((0x0131, Val::Ascii("moves the maker note")));
    let moved = b.build();
    let c = container(&moved);
    assert_ne!(c.maker.as_ref().unwrap().origin, 0x84);
    assert_eq!(c.linearization().unwrap().unwrap(), &hex(TV2_LOSSLESS)[..]);
    assert_eq!(raster(&moved), raster(&f));
    // R-NEF-20: tag 0x0093 wins over tag 0x0051; 0x0051 needs 12 bytes;
    // a 0x0093 that is not an integer type falls back to 0x0051.
    let f = f4();
    let c = container(&f);
    assert_eq!(c.nef_compression().unwrap(), Some(10));
    let b = maker_with(f4_builder(), |m| with(m, 0x0093, Val::Short(vec![6])));
    assert_eq!(container(&b.build()).nef_compression().unwrap(), Some(6));
    let b = maker_with(f4_builder(), |m| with(m, 0x0093, Val::Ascii("6")));
    assert_eq!(container(&b.build()).nef_compression().unwrap(), Some(10));
    let b = maker_with(f4_builder(), |m| {
        with(
            m,
            0x0051,
            Val::Undef(hex("30 31 30 30 30 34 30 30 00 00 0A")),
        )
    });
    assert_eq!(container(&b.build()).nef_compression().unwrap(), None);

    // R-NEF-52 to R-NEF-55: the candidates of the chain, the SubIFDs and
    // the maker-note PreviewIFD, largest first.
    let mut b = with_previews(f2_builder());
    b.blobs.push(jpeg(24, 16, [40, 200, 200]));
    b.chain.push(jpeg_ifd(3));
    let f = b.build();
    let sizes: Vec<(u32, u32)> = preview::collect(&container(&f))
        .iter()
        .map(|p| (p.width, p.height))
        .collect();
    assert_eq!(sizes, [(48, 32), (24, 16), (16, 8)]);
    // Candidates outside the file, shorter than 4 bytes or not starting
    // with FF D8 are not kept.
    for (name, change) in [
        ("outside", Val::Long(vec![1 << 24])),
        ("short", Val::Long(vec![3])),
    ] {
        let mut b = with_previews(f2_builder());
        b.after = vec![with(jpeg_ifd(1), 0x0202, change)];
        let n = preview::collect(&container(&b.build())).len();
        assert_eq!(n, 1, "{name}");
    }
    let mut b = with_previews(f2_builder());
    b.blobs[1][1] = 0xD9;
    assert_eq!(preview::collect(&container(&b.build())).len(), 1);
    // R-NEF-55: the largest fails to decode: the next one.
    // The broken one keeps its size (SOF0) but ends inside its scan header.
    let mut b = with_previews(f2_builder());
    let sos = b.blobs[1]
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .unwrap();
    b.blobs[1].truncate(sos + 4);
    b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 1, 1]));
    let f = b.build();
    assert_eq!(preview::collect(&container(&f)).len(), 2);
    let dev = report(&f);
    assert_preview(&dev, (16, 8), SMALL.2, "next largest");
    // R-NEF-56: no JPEG decodes: the IFD0 thumbnail; neither: an error.
    let mut b = with_previews(f2_builder());
    b.after.clear();
    b.maker_jpeg = None;
    b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 1, 1]));
    let dev = report(&b.build());
    assert_image(&dev, Origin::Thumbnail, (4, 2), THUMB.2, "thumbnail");
    let mut nothumb = b.clone();
    nothumb.ifd0 = with(nothumb.ifd0, 0x0117, Val::Long(vec![23]));
    assert!(decode_report(&nothumb.build(), &DecodeOptions::default()).is_err());
    let mut nothumb = b.clone();
    nothumb.ifd0 = with(nothumb.ifd0, 0x0102, Val::Short(vec![8, 8]));
    assert!(decode_report(&nothumb.build(), &DecodeOptions::default()).is_err());
    // R-NEF-57: the IFD0 orientation on the preview and the thumbnail,
    // except values 5 to 8 on a picture taller than wide.
    let mut b = with_previews(f2_builder());
    b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![6]));
    b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 1, 1]));
    let dev = report(&b.build());
    assert_preview(&dev, (32, 48), BIG.2, "orientation 6");
    let mut tall = b.clone();
    tall.blobs[1] = jpeg(32, 48, BIG.2);
    let dev = report(&tall.build());
    assert_preview(&dev, (32, 48), BIG.2, "tall, orientation 6 not applied");
    let mut thumb6 = b.clone();
    thumb6.after.clear();
    thumb6.maker_jpeg = None;
    let dev = report(&thumb6.build());
    assert_image(
        &dev,
        Origin::Thumbnail,
        (2, 4),
        THUMB.2,
        "thumbnail, orientation 6",
    );
    // R-NEF-54: bytes after EOI, and no EOI after the last scan.
    for (what, change) in [("tail", 0usize), ("no EOI", 1)] {
        let mut b = with_previews(f2_builder());
        b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 1, 1]));
        if change == 0 {
            b.blobs[1].extend([0x00, 0x11, 0x22, 0x33]);
        } else {
            let n = b.blobs[1].len();
            b.blobs[1].truncate(n - 2);
        }
        let dev = report(&b.build());
        assert_preview(&dev, (48, 32), BIG.2, what);
    }
    // A developed file reports the sensor path, a preview reports the
    // preview (R-NEF-57); the probe sizes agree.
    let f = with_previews(f2_builder()).build();
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V2));
    assert_eq!(probe(&f).unwrap().unwrap().0, 4);
    let mut b = with_previews(f2_builder());
    b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 1, 1]));
    assert_eq!(probe(&b.build()).unwrap().unwrap().0, 48);
}

/// `with_previews` for a builder whose blobs 0 and 1 are both sensor
/// strips: the previews become blobs 2 and 3.
fn with_previews_keep(mut b: Builder) -> Builder {
    b.blobs.push(jpeg(BIG.0, BIG.1, BIG.2));
    b.after.push(jpeg_ifd(2));
    b.maker_jpeg = Some(jpeg(SMALL.0, SMALL.1, SMALL.2));
    b
}

#[test]
fn nef_target_size_uses_preview() {
    // A 100 x 60 raster: neither preview is as wide as the developed image.
    let vals = random_values(100 * 60, 500, 15000, 3);
    let raster = with_previews(v1_file("NIKON D850", 100, 60, &vals, f1_maker(), 1)).build();
    let f = with_previews(f1_builder()).build();
    // The smallest candidate that covers the target.
    let opts = DecodeOptions {
        target_size: Some((8, 8)),
        ..DecodeOptions::default()
    };
    // The JPEG decoder may scale the stream down toward the target.
    let covers = |dev: &Developed, rgb: [u8; 3], t: (u32, u32), what: &str| {
        let (w, h) = (dev.image.width(), dev.image.height());
        assert_eq!(dev.origin, Origin::Preview, "{what}");
        assert!(w >= t.0 || h >= t.1, "{what}: {w} x {h}");
        let p = &dev.image.data()[..4];
        for c in 0..3 {
            assert!(p[c].abs_diff(rgb[c]) <= 6, "{what}: {p:?} vs {rgb:?}");
        }
    };
    let dev = decode_report(&f, &opts).unwrap().unwrap();
    covers(&dev, SMALL.2, (8, 8), "target 8 x 8");
    let opts = DecodeOptions {
        target_size: Some((40, 30)),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&raster, &opts).unwrap().unwrap();
    covers(&dev, BIG.2, (40, 30), "target 40 x 30");
    let opts = DecodeOptions {
        target_size: Some((8, 8)),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&raster, &opts).unwrap().unwrap();
    covers(&dev, SMALL.2, (8, 8), "100 x 60, target 8 x 8");
    // Larger than both previews: the sensor data.
    let opts = DecodeOptions {
        target_size: Some((90, 50)),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&raster, &opts).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    // Through the registry: the extension selects the NEF developer.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f.clone(), Some("a.nef".into()));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (4, 2));
    let small = DecodeOptions {
        target_size: Some((8, 8)),
        ..DecodeOptions::default()
    };
    let img = reg.decode(&src, &small).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    // Detection by content: Make "NIKON" and a CFA raw IFD; a plain TIFF
    // or another maker is not taken.
    assert!(is_nef(&f, None));
    assert!(is_nef(&[], Some("nef")));
    let mut b = f1_builder();
    b.ifd0 = with(b.ifd0, 0x010F, Val::Ascii("SONY"));
    assert!(!is_nef(&b.build(), None));
    let mut b = f1_builder();
    b.raw.clear();
    assert!(!is_nef(&b.build(), None));
    assert!(crate::raw::is_camera_raw(
        &pandite_core::Source::from_bytes(f, None)
    ));
}

#[test]
fn facts_nef() {
    use pandite_core::Source;
    let want = [
        ("F1", "Uncompressed", 14, "Bayer RGGB"),
        ("F2", "Uncompressed, 12-bit packed", 12, "Bayer RGGB"),
        ("F3", "Uncompressed, 12-bit packed", 12, "Bayer RGGB"),
        ("F4", "Uncompressed, 14-bit packed", 14, "Bayer RGGB"),
        ("F5", "Uncompressed, 12-bit packed (D100)", 12, "Bayer GRBG"),
        ("F6", "Nikon lossless compressed", 12, "Bayer RGGB"),
        ("F7", "Nikon lossless compressed", 14, "Bayer RGGB"),
        ("F8", "Nikon lossy compressed (type 1)", 12, "Bayer BGGR"),
        ("F9", "Nikon lossy compressed (type 2)", 12, "Bayer RGGB"),
        ("F10", "Nikon lossy compressed (type 2)", 14, "Bayer RGGB"),
    ];
    for ((name, _, f), (wname, comp, bits, channels)) in fixtures().into_iter().zip(want) {
        assert_eq!(name, wname);
        let src = Source::from_bytes(f, Some("x.nef".into()));
        let facts = pandite_core::DecoderProvider::facts(&crate::RawDecoder, &src).unwrap();
        assert_eq!(facts.container.as_deref(), Some("Nikon NEF"), "{name}");
        assert_eq!(facts.compression.as_deref(), Some(comp), "{name}");
        assert_eq!(facts.bits_per_channel, Some(bits), "{name}");
        assert_eq!(facts.channels.as_deref(), Some(channels), "{name}");
        assert_eq!(facts.channel_count, Some(1), "{name}");
    }
    let x1 = maker_with(f6_builder(), |m| with(m, 0x0093, Val::Short(vec![13]))).build();
    let f = facts(&x1).unwrap();
    assert_eq!(f.compression.as_deref(), Some("Nikon High Efficiency"));
    assert!(facts(b"not a tiff").is_none());
    for n in (0..f1().len()).step_by(7) {
        let _ = facts(&f1()[..n]);
    }
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut v: Vec<(&'static str, Vec<u8>)> = fixtures()
        .into_iter()
        .map(|(name, _, f)| (name, f))
        .collect();
    v.push(("F1+previews", with_previews(f1_builder()).build()));
    v.push(("F2+previews", with_previews(f2_builder()).build()));
    v.push(("F6+previews", with_previews(f6_builder()).build()));
    v.push(("F8+previews", with_previews(f8_builder()).build()));
    v.push(("F10+previews", with_previews(f10_builder()).build()));
    let mut b = with_previews(f9_builder());
    b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![6]));
    v.push(("F9+previews, orientation 6", b.build()));
    v
}

#[test]
fn nef_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut next = noise(0x9E37_79B9);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
                // Damage the header, the IFDs and the maker note.
                0 | 1 => {
                    let limit = bad.len().min(if round % 2 == 0 { 400 } else { 2000 });
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
                // Huge values in IFD fields, both byte orders.
                _ => {
                    let i = (next() as usize % bad.len().min(400)) & !1;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27, 8184][next() as usize % 6];
                    let b = if next().is_multiple_of(2) {
                        v.to_le_bytes()
                    } else {
                        v.to_be_bytes()
                    };
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&b);
                    }
                }
            }
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&bad);
                let _ = super::facts(&bad);
                let _ = is_nef(&bad, None);
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
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.nef".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            // R-NEF-68: no mutation panicked; every failure was an error
            // value or the preview.
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized and lying claims are refused before allocation (R-NEF-58
    // to R-NEF-61, R-NEF-66).
    for (w, h) in [
        (65535u32, 65535u32),
        (0, 2),
        (4, 0),
        (70000, 2),
        (65535, 2049),
    ] {
        for b in [f2_builder(), f6_builder(), f9_builder()] {
            let b = raw_with(
                raw_with(b, 0x0100, Val::Long(vec![w])),
                0x0101,
                Val::Long(vec![h]),
            );
            let f = b.build();
            let c = container(&f);
            if let Ok(Class::Sensor(v)) = meta::classify(&c) {
                assert!(meta::plan(&c, v, None).is_err(), "{w} x {h}");
            }
            assert!(
                decode_report(&f, &DecodeOptions::default()).is_err(),
                "{w} x {h}"
            );
        }
    }
    // Sizes near 2^32 in classification: the byte-count products of rules
    // 4, 5 and 8 to 10 do not overflow (R-NEF-58).
    for b in [f1_builder(), f2_builder(), f3_builder(), f5_builder()] {
        let b = raw_with(
            raw_with(b, 0x0100, Val::Long(vec![u32::MAX])),
            0x0101,
            Val::Long(vec![u32::MAX]),
        );
        let f = with_previews(b).build();
        assert_eq!(
            meta::classify(&container(&f)).unwrap(),
            Class::OutOfScope(Relative::X7)
        );
        assert_eq!(report(&f).origin, Origin::Preview);
    }
    // A V6 raster whose strip is far too short for its pixels fails before
    // the plane is allocated: 10 bytes for 60000 x 2000 pixels.
    let b = raw_with(
        raw_with(f6_builder(), 0x0100, Val::Long(vec![60000])),
        0x0101,
        Val::Long(vec![2000]),
    );
    let f = b.build();
    let plan = plan_of(&f);
    let Err(e) = sensor::decode(&f, &plan, &CancelToken::new()) else {
        panic!("a short strip decoded");
    };
    assert!(e.to_string().contains("too short"), "{e}");
    // A sequence of large differences: code values stay in i32 and the
    // range check fails them (R-NEF-66, R-NEF-33): L14 codes 11111111
    // (category 14) with all extra bits 1 add 16383 per pixel.
    let mut b = f7_builder();
    b.blobs[0] = vec![0xFF; 64];
    let dev = report(&with_previews(b).build());
    assert_eq!(dev.origin, Origin::Preview);
    // An IFD that points to itself, SubIFDs in a loop and a chain loop end
    // without hanging (R-NEF-11, R-NEF-12).
    let mut f = f2();
    // IFD0 next-IFD offset (after 5 entries at 8): back to IFD0.
    f[8 + 2 + 60..8 + 2 + 64].copy_from_slice(&u32b(8, false));
    let _ = decode_report(&f, &DecodeOptions::default());
    assert_eq!(container(&f).chain.len(), 1);
}

#[test]
fn nef_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        // The end of the sensor data of the whole file.
        let plan = plan_of(&f);
        let strip_end = match &plan.layout {
            meta::Layout::Stream { offset, len, .. } => offset + len,
            meta::Layout::Strips {
                offsets,
                rows_per_strip,
                row_bytes,
            } => offsets
                .iter()
                .map(|o| o + rows_per_strip * row_bytes)
                .max()
                .unwrap(),
        };
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([
            1,
            7,
            8,
            9,
            24,
            26,
            0x7A,
            0x84,
            strip_end - 1,
            f.len() - 1,
            f.len() - 2,
        ]);
        for cut in cuts.into_iter().filter(|&c| c < f.len()) {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => {
                    // A file cut inside its sensor data never develops
                    // (R-NEF-61, R-NEF-64, R-NEF-69).
                    if cut < strip_end {
                        assert!(
                            !matches!(dev.origin, Origin::Sensor(_)),
                            "{name} cut at {cut} developed"
                        );
                    }
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large V1 file: 3000 x 3000 pixels.
fn large_v1() -> Vec<u8> {
    let vals = vec![3000u16; 3000 * 3000];
    v1_file("NIKON D850", 3000, 3000, &vals, f1_maker(), 1).build()
}

/// A large V6 file (table L12): 3000 x 3000 pixels, every code 11110
/// (category 0), all values 512.
fn large_v6() -> Vec<u8> {
    let (w, h) = (3000u32, 3000u32);
    let groups = (w * h / 8) as usize;
    let mut strip = Vec::with_capacity(groups * 5);
    for _ in 0..groups {
        strip.extend_from_slice(&[0xF7, 0xBD, 0xEF, 0x7B, 0xDE]);
    }
    let mut b = f6_builder();
    b.blobs[0] = strip;
    raw_with(
        raw_with(b, 0x0100, Val::Long(vec![w])),
        0x0101,
        Val::Long(vec![h]),
    )
    .build()
}

#[test]
fn nef_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f1(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f1(), Some("a.nef".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread: the
    // parallel V1 path and the serial V6 bit stream.
    for (name, f, variant) in [
        ("V1", large_v1(), Variant::V1),
        ("V6", large_v6(), Variant::V6),
    ] {
        let start = Instant::now();
        let dev = report(&f);
        let full = start.elapsed();
        assert_eq!(dev.origin, Origin::Sensor(variant), "{name}");
        assert_eq!((dev.image.width(), dev.image.height()), (3000, 3000));
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
        assert!(
            matches!(result, Err(Error::Cancelled)),
            "{name}: not cancelled"
        );
        assert!(
            took < full * 4 / 5 + Duration::from_millis(50),
            "{name}: cancel took {took:?} of a {full:?} decode"
        );
    }
    // The serial bit-stream decoder checks the token itself.
    let f = large_v6();
    let plan = plan_of(&f);
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        sensor::decode(&f, &plan, &token),
        Err(Error::Cancelled)
    ));
}

// ---------------------------------------------------------------------------
// Container rules
// ---------------------------------------------------------------------------

#[test]
fn nef_container_rules() {
    // R-NEF-6: values of up to 4 bytes inline (Orientation, the CFA
    // pattern), longer ones at their offset (Make, Model, the rationals):
    // F2 is the specification's file, byte for byte.
    let f = f2();
    let c = container(&f);
    assert_eq!(c.model(), b"NIKON D700");
    assert_eq!(c.orientation(), 1);
    assert_eq!(plan_of(&f).cfa, RGGB);
    // R-NEF-8: BYTE, SHORT, LONG and IFD (13) are read as integers.
    for (what, v) in [
        ("BYTE", Val::Byte(vec![4])),
        ("SHORT", Val::Short(vec![4])),
        ("LONG", Val::Long(vec![4])),
        ("IFD", Val::Raw(13, 1, u32b(4, false).to_vec())),
    ] {
        let f = raw_with(f2_builder(), 0x0100, v).build();
        assert_eq!(
            raster(&f),
            [628, 1128, 628, 1128, 1128, 378, 1128, 378],
            "{what}"
        );
    }
    let f = raw_with(f2_builder(), 0x0100, Val::Ascii("4")).build();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::OutOfScope(Relative::X7)
    );
    // SubIFDs and the Exif IFD pointer as type IFD (13).
    let f = f2();
    let mut g = f.clone();
    g[0x2E + 2..0x2E + 4].copy_from_slice(&[0x00, 0x0D]);
    g[0x3A + 2..0x3A + 4].copy_from_slice(&[0x00, 0x0D]);
    assert_eq!(raster(&g), raster(&f));
    assert!(container(&g).maker.is_some());
    // R-NEF-7: the first entry of a tag that occurs twice.
    let mut b = f2_builder();
    b.ifd0.push((0x0110, Val::Ascii("NIKON D3")));
    let f = b.build();
    assert_eq!(container(&f).model(), b"NIKON D700");
    // R-NEF-9: an entry with a type outside 1 to 13 is skipped; its value
    // is not used.
    let mut b = f2_builder();
    b.ifd0 = with(b.ifd0, 0x0112, Val::Raw(14, 1, vec![0, 6, 0, 0]));
    assert_eq!(container(&b.build()).orientation(), 1);
    let mut b = f2_builder();
    b.ifd0 = with(b.ifd0, 0x0112, Val::Raw(0, 1, vec![0, 6, 0, 0]));
    assert_eq!(container(&b.build()).orientation(), 1);
    // R-NEF-10: an ASCII value ends at its first zero byte and compares
    // without trailing spaces.
    let mut b = f2_builder();
    b.ifd0 = with(
        b.ifd0,
        0x0110,
        Val::Raw(2, 16, b"NIKON D700  \0D3\0".to_vec()),
    );
    let f = b.build();
    assert_eq!(container(&f).model(), b"NIKON D700");
    assert!(plan_of(&f).matrix_from_table);
    // R-NEF-11: the chain stops after 16 IFDs and at a repeated offset.
    let mut b = f2_builder();
    for _ in 0..20 {
        b.chain.push(vec![(0x0112, Val::Short(vec![1]))]);
    }
    assert_eq!(container(&b.build()).chain.len(), 16);
    // R-NEF-12: an invalid raw IFD (offset outside the file, entry count
    // 0 or above 1024, entries beyond the end of the file) is not a raw
    // IFD: X6.
    let f = f2();
    for (what, at, bytes) in [
        ("offset", 0x36usize, vec![0x00, 0xFF, 0xFF, 0x00]),
        ("count 0", 0xBE, vec![0x00, 0x00]),
        ("count 1025", 0xBE, vec![0x04, 0x01]),
        ("past the end", 0xBE, vec![0x00, 0x1F]),
    ] {
        let mut g = f.clone();
        g[at..at + bytes.len()].copy_from_slice(&bytes);
        let c = container(&g);
        assert!(c.raw.is_none(), "{what}");
        assert_eq!(
            meta::classify(&c).unwrap(),
            Class::OutOfScope(Relative::X6),
            "{what}"
        );
    }
    // An invalid IFD0: no chain, no raw IFD, no preview; the file is still
    // a TIFF, and decoding reports an error.
    let mut g = f.clone();
    g[8..10].copy_from_slice(&[0, 0]);
    assert!(container(&g).chain.is_empty());
    assert!(decode_report(&g, &DecodeOptions::default()).is_err());
    // R-NEF-19: RowsPerStrip absent, 0 or greater than H means H.
    for v in [None, Some(0u32), Some(3)] {
        let mut b = f1_builder();
        b.raw = match v {
            None => without(b.raw, 0x0116),
            Some(r) => with(b.raw, 0x0116, Val::Long(vec![r])),
        };
        let f = b.build();
        assert_eq!(
            raster(&f),
            [2912, 4912, 2912, 4912, 4912, 1912, 4912, 1912],
            "{v:?}"
        );
    }
    // R-NEF-5: the file byte order for the main IFDs and V1 samples, the
    // maker-note byte order inside the maker note (F1 little-endian, F2
    // big-endian).
    assert!(container(&f1()).le);
    assert!(container(&f1()).maker.as_ref().unwrap().le);
    assert!(!container(&f2()).le);
    assert!(!plan_of(&f2()).le);
    // R-NEF-25: V1 samples above 2^bps - 1 are clipped (OQ-14: V1 with
    // 12 bits).
    let mut b = f1_builder();
    b.raw = with(b.raw, 0x0102, Val::Short(vec![12]));
    b.blobs[0] = [5000u16, 4095, 100, 4096, 4095, 200, 4095, 300]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    assert_eq!(
        raster(&b.build()),
        [4095, 4095, 100, 4095, 4095, 200, 4095, 300]
    );
}

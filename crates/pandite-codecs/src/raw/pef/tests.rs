//! Tests of the PEF decoder against the specification's test vectors
//! (TV-1 to TV-13), fixtures (F1 to F5) and requirements, plus hostile
//! input, cancellation, thumbnails and real camera files.

use super::fixtures::*;
use super::huffman::{self, Table};
use super::meta::{self, Class, Form, Plan, Relative, Variant};
use super::*;
use crate::raw::arw::tests::{agreement, grid_means, LinearF32, CHROMA_MAX, CHROMA_MEAN, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// TV-11 and F1 to F3: the expected linear sRGB of every output pixel.
const TV11: [f32; 3] = [0.392_285_3, 0.571_300_1, 0.207_763_9];
/// TV-11 second case and F4.
const TV11B: [f32; 3] = [0.719_690_5, 0.510_821, 0.377_289];
/// The specification prints 7 decimals and rounds its own intermediates.
const EPS: f32 = 1e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open")
}

fn class_of(data: &[u8]) -> Class {
    meta::classify(&container(data)).expect("classify")
}

fn plan_of(data: &[u8]) -> Plan {
    let c = container(data);
    let Class::Sensor(v) = meta::classify(&c).expect("classify") else {
        panic!("not a sensor variant");
    };
    meta::plan(&c, v).expect("plan")
}

/// The whole decoded raster, W x H sensor values.
fn raster(data: &[u8]) -> Vec<u16> {
    let mut p = plan_of(data);
    p.crop = Rect {
        x: 0,
        y: 0,
        w: p.width,
        h: p.height,
    };
    let k = sensor::scale(p.variant);
    let plane = sensor::decode(data, &p, &CancelToken::new()).expect("sensor data");
    plane.data.iter().map(|&v| v / k).collect()
}

/// Linear sRGB of every output pixel (R-PEF-46), oriented and cropped.
fn linear(data: &[u8]) -> (usize, usize, Vec<[f32; 3]>) {
    let p = plan_of(data);
    let plane = sensor::decode(data, &p, &CancelToken::new()).expect("sensor data");
    let bytes = render::render(&p, plane, &CancelToken::new(), &LinearF32).expect("render");
    let (w, h) = render::output_size(&p);
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

fn report(data: &[u8]) -> Developed {
    decode_report(data, &DecodeOptions::default())
        .expect("decode")
        .expect("PEF path")
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

/// Decode `w` x `h` pixels of a stream with a table, collecting the rows.
fn stream(strip: &[u8], t: &Table, w: usize, h: usize, bps: u32) -> PefResult<Vec<u16>> {
    let mut out = Vec::new();
    huffman::decode(strip, t, (w, h), bps, &CancelToken::new(), |_, _, v| {
        out.push(v)
    })?;
    Ok(out)
}

/// The codes of a table as bit strings.
fn code_strings(t: &Table) -> Vec<String> {
    t.codes
        .iter()
        .map(|&(c, l)| format!("{:0width$b}", c, width = usize::from(l)))
        .collect()
}

/// A small P1 or P2 file of `w` x `h` pixels through the F1 recipe.
fn strip_file(le: bool, compression: u16, w: u32, h: u32, strip: Vec<u8>) -> Vec<u8> {
    let mut b = f1_builder();
    b.le = le;
    b.ifd0 = common_ifd0((w, h), 12, compression, 32803, "PENTAX K10D", 1);
    b.blobs = vec![strip];
    b.build()
}

/// A P4 file of `w` x `h` 14-bit values (black 64, white 16378).
fn gray_file(w: u32, h: u32, values: &[u16], orientation: u16) -> Vec<u8> {
    let mut b = f5_builder();
    let ifd0 = common_ifd0(
        (w, h),
        14,
        65535,
        34892,
        "PENTAX K-3 Mark III Monochrome",
        orientation,
    );
    b.ifd0 = ifd0;
    b.blobs = vec![encode(
        &codes(TV6, 14, true),
        w as usize,
        h as usize,
        values,
    )];
    b.build()
}

/// A file whose maker note at `m` starts with `head` (form bytes, byte
/// order bytes, N) followed by N - 1 inline dummy entries and `entry`, with
/// `value` at file offset `value_at` (TV-2).
fn maker_file(
    le: bool,
    m: usize,
    head: &[u8],
    entry: [u8; 12],
    value_at: usize,
    value: &[u8],
) -> Vec<u8> {
    let w16 = |x: u16| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    let w32 = |x: u32| if le { x.to_le_bytes() } else { x.to_be_bytes() };
    let mut f = vec![0u8; value_at + value.len() + 16];
    f[..8].copy_from_slice(if le {
        &[0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00]
    } else {
        &[0x4D, 0x4D, 0x00, 0x2A, 0x00, 0x00, 0x00, 0x08]
    });
    // IFD0 at 8: the Exif pointer. Exif IFD at 64: the maker note.
    let mut ifd0 = w16(1).to_vec();
    ifd0.extend(w16(0x8769));
    ifd0.extend(w16(4));
    ifd0.extend(w32(1));
    ifd0.extend(w32(64));
    ifd0.extend(w32(0));
    f[8..8 + ifd0.len()].copy_from_slice(&ifd0);
    let mut exif = w16(1).to_vec();
    exif.extend(w16(0x927C));
    exif.extend(w16(7));
    exif.extend(w32(1000));
    exif.extend(w32(m as u32));
    exif.extend(w32(0));
    f[64..64 + exif.len()].copy_from_slice(&exif);
    f[m..m + head.len()].copy_from_slice(head);
    let n = usize::from(if le {
        u16::from_le_bytes([head[head.len() - 2], head[head.len() - 1]])
    } else {
        u16::from_be_bytes([head[head.len() - 2], head[head.len() - 1]])
    });
    let mut at = m + head.len();
    for i in 0..n - 1 {
        let mut e = w16(0x1000 + i as u16).to_vec();
        e.extend(w16(3));
        e.extend(w32(1));
        e.extend(w32(0));
        f[at..at + 12].copy_from_slice(&e);
        at += 12;
    }
    f[at..at + 12].copy_from_slice(&entry);
    f[value_at..value_at + value.len()].copy_from_slice(value);
    f
}

/// A P3 file of `w` x `h` with a zero strip of the smallest valid size and
/// the given maker-note entries (for plan checks; not decoded).
fn big_p3(le: bool, w: u32, h: u32, maker: Vec<(u16, Val)>) -> Vec<u8> {
    let (form, table, bps) = if le {
        (Form::Pentax, TV6, 14)
    } else {
        (Form::Aoc, TV5, 12)
    };
    let px = u64::from(w) * u64::from(h);
    let mut entries = maker;
    entries.push((0x0220, Val::Undef(hex(table))));
    Builder {
        ifd0: common_ifd0((w, h), bps, 65535, 32803, "PENTAX K-3 Mark III", 1),
        exif: Some(vec![(0xA302, Val::Undef(RGGB.to_vec()))]),
        maker: Some(Maker { form, le, entries }),
        blobs: vec![vec![0; px.div_ceil(8) as usize]],
        ..Builder::new(le)
    }
    .build()
}

// ---------------------------------------------------------------------------
// Test vectors and fixtures
// ---------------------------------------------------------------------------

#[test]
fn pef_vectors_decode() {
    // TV-1: both headers are accepted (R-PEF-4), IFD0 at 8 (R-PEF-11).
    assert_eq!(
        tiff::header(&hex("4D 4D 00 2A 00 00 00 08")).unwrap(),
        (false, 8)
    );
    assert_eq!(
        tiff::header(&hex("49 49 2A 00 08 00 00 00")).unwrap(),
        (true, 8)
    );
    // R-PEF-4: any other start is an error; R-PEF-59: so is a short file.
    assert!(tiff::header(&hex("49 49 00 2A 08 00 00 00")).is_err());
    assert!(tiff::header(&hex("4D 4D 2A 00 00 00 00 08")).is_err());
    assert!(tiff::header(&hex("4D 4D 00 2A 00 00 00")).is_err());
    assert!(meta::open(b"IIRO\x08\0\0\0").is_err());

    // TV-2: the two maker-note forms and their offset origins.
    let aoc = maker_file(
        false,
        684,
        &hex("41 4F 43 00 4D 4D 00 5B"),
        hex("02 00 00 03 00 00 00 04 00 00 06 FC")
            .try_into()
            .unwrap(),
        1788,
        &[0; 8],
    );
    let c = container(&aoc);
    let mn = c.maker.as_ref().expect("AOC maker note");
    assert_eq!((mn.form, mn.at, mn.dir.le), (Form::Aoc, 684, false));
    assert_eq!(mn.dir.entries.len(), 91);
    // R-PEF-16: origin 0; 1788 is the end of the IFD at 690.
    assert_eq!(mn.dir.entry(0x0200).unwrap().at, 690 + 2 + 12 * 91 + 4);
    assert_eq!(mn.dir.uints(0x0200).unwrap(), Some(vec![0, 0, 0, 0]));
    let pentax = maker_file(
        true,
        902,
        &hex("50 45 4E 54 41 58 20 00 49 49 63 00"),
        hex("00 02 03 00 04 00 00 00 B4 04 00 00")
            .try_into()
            .unwrap(),
        2106,
        &hex("00 01 00 01 00 01 00 01"),
    );
    let c = container(&pentax);
    let mn = c.maker.as_ref().expect("PENTAX maker note");
    assert_eq!((mn.form, mn.at, mn.dir.le), (Form::Pentax, 902, true));
    assert_eq!(mn.dir.entries.len(), 99);
    // R-PEF-16: origin M = 902; 902 + 1204 = 2106 = 912 + 2 + 12 * 99 + 4.
    assert_eq!(mn.dir.entry(0x0200).unwrap().at, 2106);
    assert_eq!(mn.dir.uints(0x0200).unwrap(), Some(vec![256; 4]));
    // R-PEF-15: other form bytes or byte-order bytes fail the maker note:
    // X3 by R-PEF-3 rule 1.
    // F4: offsets counted from the file start (origin 0) read other bytes
    // than the black levels 256 256 256 256 at M + 0x4C.
    let f = f4();
    let wrong = tiff::dir(&f, 0xD8 + 10, true, 0).unwrap();
    assert_ne!(wrong.uints(0x0200).unwrap(), Some(vec![256; 4]));
    let right = tiff::dir(&f, 0xD8 + 10, true, 0xD8).unwrap();
    assert_eq!(right.uints(0x0200).unwrap(), Some(vec![256; 4]));
    let mut bad = pentax.clone();
    bad[902 + 8] = b'X';
    assert!(container(&bad).maker.is_none());
    let mut bad = aoc.clone();
    bad[684] = b'B';
    assert!(container(&bad).maker.is_none());

    // TV-3: CFAPattern (R-PEF-19).
    assert_eq!(
        meta::bayer(&hex("00 02 00 02 00 01 01 02")),
        Some([0, 1, 1, 2])
    );
    assert_eq!(
        meta::bayer(&hex("00 02 00 02 02 01 01 00")),
        Some([2, 1, 1, 0])
    );
    assert_eq!(
        meta::bayer(&hex("02 00 02 00 01 00 02 01")),
        Some([1, 0, 2, 1])
    );
    assert_eq!(meta::bayer(&hex("00 02 00 02 00 01 01 01")), None);
    assert_eq!(meta::bayer(&hex("00 02 00 02 01 00 01 02")), None);
    assert_eq!(meta::bayer(&hex("00 03 00 02 00 01 01 02")), None);
    assert_eq!(meta::bayer(&hex("00 02 00 02 00 01 01")), None);

    // TV-4: crop tags (R-PEF-22).
    let p = plan_of(&big_p3(
        true,
        6304,
        4224,
        vec![(0x023E, Val::Undef(hex("36 00 3A 00 30 18 20 10")))],
    ));
    assert_eq!(
        (p.crop.x, p.crop.y, p.crop.w, p.crop.h),
        (54, 58, 6192, 4128)
    );
    let p = plan_of(&big_p3(
        false,
        3936,
        2624,
        vec![
            (0x0038, Val::Raw(3, 2, hex("00 08 00 08"))),
            (0x0039, Val::Short(vec![0x0F20, 0x0A20])),
        ],
    ));
    assert_eq!((p.crop.x, p.crop.y, p.crop.w, p.crop.h), (8, 8, 3872, 2592));

    // TV-5 and TV-6: the tables (R-PEF-27, R-PEF-28).
    let tv5 = Table::parse(&hex(TV5), 12, false).expect("TV-5");
    assert_eq!(
        code_strings(&tv5),
        [
            "11110",
            "110",
            "100",
            "00",
            "01",
            "101",
            "1110",
            "111110",
            "1111110",
            "11111110",
            "111111110",
            "1111111110",
            "1111111111"
        ]
    );
    let tv6 = Table::parse(&hex(TV6), 14, true).expect("TV-6");
    assert_eq!(
        code_strings(&tv6),
        [
            "11111110",
            "1111110",
            "111110",
            "11110",
            "1110",
            "101",
            "110",
            "100",
            "00",
            "01",
            "111111110",
            "1111111110",
            "11111111110",
            "111111111110",
            "111111111111"
        ]
    );
    // R-PEF-27: the count must be 14 + 3 * N and V = N - 12.
    assert!(Table::parse(&hex(TV5), 14, false).is_none());
    assert!(Table::parse(&hex(TV6), 12, true).is_none());
    assert!(Table::parse(&hex(TV5), 12, true).is_none(), "byte order");
    let mut t = hex(TV5);
    t[1] = 2;
    assert!(Table::parse(&t, 12, false).is_none(), "V");
    assert!(Table::parse(&hex(TV5)[..52], 12, false).is_none(), "count");
    // R-PEF-28: lengths 1 to 12, low bits zero, prefix-free.
    let mut t = hex(TV5);
    t[14 + 26] = 13;
    assert!(Table::parse(&t, 12, false).is_none(), "length 13");
    let mut t = hex(TV5);
    t[14 + 26] = 0;
    assert!(Table::parse(&t, 12, false).is_none(), "length 0");
    let mut t = hex(TV5);
    t[15] = 0x01; // C(0) = 0x0F01: low bits not zero
    assert!(Table::parse(&t, 12, false).is_none(), "low bits");
    let mut t = hex(TV5);
    t[14 + 2 * 12] = 0x1F; // C(12) above 4095
    assert!(Table::parse(&t, 12, false).is_none(), "C above 4095");
    let mut t = hex(TV5);
    t[14 + 2 * 12 + 1] = 0xF8; // C(12) = C(11)
    assert!(Table::parse(&t, 12, false).is_none(), "equal codes");
    let mut t = hex(TV5);
    // SSSS 0 becomes 1111 (4 bits), the beginning of 11110 and others.
    t[14 + 26] = 4;
    assert!(Table::parse(&t, 12, false).is_none(), "prefix");

    // TV-7: a 4 x 3 stream with the TV-5 table (R-PEF-29 to R-PEF-34).
    let tv7 = hex("FB 27 EC 89 AB FB 4B B2 14 A2 59 87");
    assert_eq!(
        stream(&tv7, &tv5, 4, 3, 12).unwrap(),
        [100, 200, 103, 190, 210, 50, 205, 52, 98, 205, 101, 199]
    );
    // R-PEF-35: one byte less cuts the last code word: failure.
    assert!(stream(&tv7[..11], &tv5, 4, 3, 12).is_err());
    // R-PEF-31: with L(12) = 11 the code of SSSS 12 is 11111111110 and no
    // code begins 11111111111: a stream of 1-bits fails.
    let mut t = hex(TV5);
    t[14 + 2 * 13 + 12] = 11;
    let holes = Table::parse(&t, 12, false).expect("prefix-free with a hole");
    assert!(stream(&[0xFF; 8], &holes, 4, 1, 12).is_err());

    // TV-8: the start of the K10D strip, 12-bit.
    assert_eq!(
        stream(&hex("EA 3E 93 8C 90 20 F4 89 18 4F"), &tv5, 10, 1, 12).unwrap(),
        [40, 73, 39, 82, 43, 76, 36, 76, 38, 80]
    );
    // TV-9: the start of the K-3 Mark III strip, 14-bit little-endian
    // table; the eighth code word continues past these 8 bytes.
    assert_eq!(
        stream(&hex("60 CC 03 E7 EB FB FB C3"), &tv6, 7, 1, 14).unwrap(),
        [262, 256, 260, 258, 260, 258, 253]
    );

    // TV-10: P1 and P2 samples (R-PEF-24 to R-PEF-26).
    let f = strip_file(false, 1, 4, 1, hex("00 E6 01 1C 00 DD 01 13"));
    assert_eq!(class_of(&f), Class::Sensor(Variant::P1));
    assert_eq!(raster(&f), [230, 284, 221, 275]);
    let f = strip_file(true, 1, 2, 1, hex("68 04 04 68"));
    // 0x6804 = 26628 clips to 4095 (R-PEF-66).
    assert_eq!(raster(&f), [1128, 4095]);
    let f = strip_file(false, 32773, 4, 1, hex("89 D0 92 09 40 97"));
    assert_eq!(class_of(&f), Class::Sensor(Variant::P2));
    assert_eq!(raster(&f), [2205, 146, 148, 151]);
    let f = strip_file(false, 32773, 4, 1, hex("46 88 50 85 04 04"));
    assert_eq!(raster(&f), [1128, 2128, 2128, 1028]);

    // The builder follows the recipes byte for byte, and the encoder
    // reproduces the coded strips of F3 to F5.
    assert_eq!(f1_builder().build(), f1(), "F1");
    assert_eq!(f3_builder().build(), f3(), "F3");
    assert_eq!(f4_builder().build(), f4(), "F4");
    assert_eq!(f5_builder().build(), f5(), "F5");
    assert_eq!((f1().len(), f2().len()), (288, 284));
    assert_eq!((f3().len(), f4().len(), f5().len()), (406, 429, 375));

    // F1 (P1) and F2 (P2): 4 x 2, every pixel TV-11.
    for (name, f, v) in [("F1", f1(), Variant::P1), ("F2", f2(), Variant::P2)] {
        assert_eq!(class_of(&f), Class::Sensor(v), "{name}");
        assert_eq!(
            raster(&f),
            [1128, 2128, 1128, 2128, 2128, 1028, 2128, 1028],
            "{name}"
        );
        let (w, h, px) = linear(&f);
        assert_eq!((w, h), (4, 2), "{name}");
        for p in px {
            assert_rgb(p, TV11, EPS, name);
        }
        let dev = report(&f);
        assert_eq!(dev.origin, Origin::Sensor(v), "{name}");
        assert_rgba16_flat(&dev.image, TV11, name);
    }

    // F3 (P3, big-endian, AOC, crop (1, 0, 4, 4) at an odd column).
    let f = f3();
    assert_eq!(class_of(&f), Class::Sensor(Variant::P3));
    assert_eq!(raster(&f), f3_raster());
    let p = plan_of(&f);
    assert_eq!((p.crop.x, p.crop.y, p.crop.w, p.crop.h), (1, 0, 4, 4));
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (4, 4));
    for p in px {
        assert_rgb(p, TV11, EPS, "F3");
    }

    // F4 (P3, little-endian, PENTAX, 14-bit, crop (2, 1, 4, 2)).
    let f = f4();
    assert_eq!(class_of(&f), Class::Sensor(Variant::P3));
    assert_eq!(raster(&f), f4_raster());
    let p = plan_of(&f);
    assert_eq!((p.crop.x, p.crop.y, p.crop.w, p.crop.h), (2, 1, 4, 2));
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (4, 2));
    for p in px {
        assert_rgb(p, TV11B, EPS, "F4");
    }

    // F5 (P4): gray output, r = g = b = v (R-PEF-42, R-PEF-47).
    let f = f5();
    assert_eq!(class_of(&f), Class::Sensor(Variant::P4));
    assert_eq!(raster(&f), f5_raster());
    let want = [
        0.0,
        0.061_297_0,
        0.122_594_1,
        0.183_891_1,
        0.245_188_2,
        0.306_485_2,
        0.367_782_3,
        1.0,
        1.0,
        0.5,
        0.0,
        0.002_206_7,
        0.008_336_4,
        0.014_466_1,
        0.020_595_8,
        0.026_725_5,
    ];
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (8, 2));
    for (p, &v) in px.iter().zip(&want) {
        assert_rgb(*p, [v; 3], EPS, "F5");
    }
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::P4));

    // TV-13: orientation (R-PEF-50, R-PEF-51) on a 3 x 2 gray image a b c /
    // d e f.
    let vals = [1064u16, 2064, 3064, 4064, 5064, 6064];
    let v = |s: u16| (f32::from(s) - 64.0) / 16314.0;
    let [a, b, c, d, e, g] = vals.map(v);
    for (o, size, order) in [
        (1u16, (3, 2), vec![a, b, c, d, e, g]),
        (2, (3, 2), vec![c, b, a, g, e, d]),
        (3, (3, 2), vec![g, e, d, c, b, a]),
        (4, (3, 2), vec![d, e, g, a, b, c]),
        (5, (2, 3), vec![a, d, b, e, c, g]),
        (6, (2, 3), vec![d, a, e, b, g, c]),
        (7, (2, 3), vec![g, c, e, b, d, a]),
        (8, (2, 3), vec![c, g, b, e, a, d]),
    ] {
        let f = gray_file(3, 2, &vals, o);
        let (w, h, px) = linear(&f);
        assert_eq!((w, h), size, "orientation {o}");
        for (p, &want) in px.iter().zip(&order) {
            assert_rgb(*p, [want; 3], EPS, &format!("orientation {o}"));
        }
        // An absent or invalid tag is 1.
        if o == 1 {
            let mut bad = f.clone();
            let at = bad
                .windows(4)
                .position(|x| x == [0x12, 0x01, 0x03, 0x00])
                .unwrap();
            bad[at + 8] = 9;
            assert_eq!(plan_of(&bad).orientation, 1);
        }
    }
    // The CFA path orients through the shared renderer: F1 at 6 is 2 x 4.
    let mut b = f1_builder();
    b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![6]));
    let (w, h, px) = linear(&b.build());
    assert_eq!((w, h), (2, 4));
    for p in px {
        assert_rgb(p, TV11, EPS, "F1 at 6");
    }
}

#[test]
fn pef_levels_wb_matrix_vector() {
    // F3 through the steps: TV-11.
    let p = plan_of(&f3());
    // R-PEF-37: no tag 0x007E, BPS 12: w = 4095.
    assert_eq!(p.white, 4095);
    // R-PEF-38: 128 128 128 128.
    assert_eq!(p.black_tag, Some([128; 4]));
    assert_eq!(p.black2, [256; 3]);
    // R-PEF-40 source 1: G = 8192, mR = 1.6796875, mB = 1.15234375.
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert_eq!(p.wb, [1.679_687_5, 1.0, 1.152_343_75]);
    // R-PEF-41 worked example.
    let n = |s: f64, m: f64| ((s - 128.0) / (4095.0 - 128.0) * m).clamp(0.0, 1.0);
    let v = [n(1128.0, p.wb[0]), n(2128.0, 1.0), n(1028.0, p.wb[2])];
    for (g, w) in v.iter().zip([0.423_415_0, 0.504_159_3, 0.261_434_2]) {
        assert!((g - w).abs() < 5e-8, "normalized {v:?}");
    }
    // R-PEF-43, R-PEF-44: the K10D rows normalized.
    assert!(p.matrix_from_table);
    let want = [
        [1.683_168_3, -0.584_158_4, -0.099_009_9],
        [-0.08, 1.33, -0.25],
        [0.009_901_0, -0.227_722_8, 1.217_821_8],
    ];
    for (r, wr) in p.matrix.iter().zip(want) {
        for (g, w) in r.iter().zip(wr) {
            assert!((g - w).abs() < 5e-8, "matrix {:?}", p.matrix);
        }
    }
    // R-PEF-45 worked example.
    let out: Vec<f64> = p
        .matrix
        .iter()
        .map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        .collect();
    for (g, w) in out.iter().zip(TV11) {
        assert!((g - f64::from(w)).abs() < 1e-6, "TV-11 {out:?}");
    }

    // TV-11 second case (F4): tag 0x007E 16376, black 256, the generic
    // matrix of the K-3 Mark III.
    let p = plan_of(&f4());
    assert_eq!(p.white, 16376);
    assert_eq!(p.black2, [512; 3]);
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert_eq!(p.wb, [2.484_375, 1.0, 1.820_312_5]);
    assert!(!p.matrix_from_table, "R-PEF-43: generic matrix");
    let want = [
        [1.778_221_8, -0.670_329_7, -0.107_892_1],
        [-0.139_139_1, 1.487_487_5, -0.348_348_3],
        [0.055_944_1, -0.456_543_5, 1.400_599_4],
    ];
    for (r, wr) in p.matrix.iter().zip(want) {
        for (g, w) in r.iter().zip(wr) {
            assert!((g - w).abs() < 5e-8, "generic {:?}", p.matrix);
        }
    }
    let v: [f64; 3] = [
        4000.0 / 16120.0 * p.wb[0],
        8000.0 / 16120.0,
        3600.0 / 16120.0 * p.wb[2],
    ];
    for (g, w) in v.iter().zip([0.616_470_2, 0.496_277_9, 0.406_521_4]) {
        assert!((g - w).abs() < 5e-8, "normalized {v:?}");
    }
    // Neutral stays neutral for every normalized matrix.
    for model in ["PENTAX K10D", "PENTAX 645Z", "PENTAX *ist D", "unknown"] {
        let (m, _) = color::matrix(model.as_bytes());
        for r in m {
            assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12, "{model}");
        }
    }

    // TV-12: P4 normalization (R-PEF-42): b = 64, w = 16378, no WB.
    let p = plan_of(&f5());
    assert_eq!((p.white, p.gray_black), (16378, 64.0));
    assert_eq!(p.wb_source, WbSource::NotApplied);
    let (_, _, px) = linear(&gray_file(4, 1, &[64, 1064, 8221, 16378], 1));
    for (p, want) in px.iter().zip([0.0, 0.061_297_0, 0.5, 1.0]) {
        assert_rgb(*p, [want; 3], EPS, "TV-12");
    }

    // Variants of the levels on a 4 x 2 P1 file (values of F1). The maker
    // note keeps one other entry: an IFD with N = 0 is invalid (R-PEF-12).
    let mk = |maker: Vec<(u16, Val)>, model: &str| {
        let mut b = f1_builder();
        b.ifd0 = with(b.ifd0, 0x0110, Val::Ascii(model.to_string()));
        b.maker.as_mut().unwrap().entries = with(maker, 0x0008, Val::Short(vec![4]));
        plan_of(&b.build())
    };
    // R-PEF-39 and R-PEF-40 sources 2 and 3: no tags.
    let p = mk(vec![], "PENTAX K10D");
    assert_eq!((p.black_tag, p.black2), (None, [0; 3]));
    assert_eq!(p.wb_source, WbSource::Table);
    assert_eq!(p.wb, [2.08, 1.0, 1.03]);
    let p = mk(vec![], "PENTAX *ist D");
    assert_eq!(p.wb_source, WbSource::Generic);
    assert_eq!(p.wb, [2.162, 1.0, 1.483]);
    assert!(!p.matrix_from_table);
    // R-PEF-10: trailing spaces and zero bytes are removed.
    let p = mk(vec![], "PENTAX K-5 II s   ");
    assert!(p.matrix_from_table);
    assert_eq!(p.wb, [2.10, 1.0, 1.72]);
    // R-PEF-40: a WB level of 0 skips the tag; so does a count other than 4.
    let p = mk(
        vec![(0x0201, Val::Short(vec![13760, 0, 8192, 9440]))],
        "PENTAX K10D",
    );
    assert_eq!(p.wb_source, WbSource::Table);
    let p = mk(vec![(0x0201, Val::Long(vec![2, 1, 1, 3]))], "PENTAX K10D");
    assert_eq!((p.wb_source, p.wb), (WbSource::AsShot, [2.0, 1.0, 3.0]));
    // R-PEF-38: the color order with the mean of the greens.
    let p = mk(vec![(0x0200, Val::Short(vec![65, 65, 66, 66]))], "x");
    assert_eq!(p.black2, [130, 131, 132]);
    assert_eq!(p.gray_black, 65.5);
    // R-PEF-38: a value not below w - 255 makes the tag invalid (R-PEF-39).
    let p = mk(vec![(0x0200, Val::Short(vec![64, 64, 3840, 64]))], "x");
    assert_eq!(p.black2, [0; 3]);
    let p = mk(vec![(0x0200, Val::Short(vec![64, 64, 3839, 64]))], "x");
    assert_eq!(p.black2, [128, 3903, 128]);
    let p = mk(vec![(0x0200, Val::Short(vec![64, 64, 64]))], "x");
    assert_eq!(p.black_tag, None);
    // R-PEF-37: tag 0x007E from 256 to 2^BPS - 1.
    for (tag, want) in [
        (255u32, 4095),
        (256, 256),
        (4095, 4095),
        (4096, 4095),
        (3839, 3839),
    ] {
        let p = mk(vec![(0x007E, Val::Long(vec![tag]))], "x");
        assert_eq!(p.white, want, "0x007E = {tag}");
    }
    // R-PEF-70: the first value of a count-1 tag with more values.
    let p = mk(vec![(0x007E, Val::Short(vec![3000, 9]))], "x");
    assert_eq!(p.white, 3000);

    // R-PEF-66: values clip to 0 and 1 (highlights and below black).
    let f = strip_file(
        false,
        1,
        4,
        2,
        hex("FF FF FF FF FF FF FF FF 00 00 00 00 00 00 00 00"),
    );
    assert_eq!(raster(&f), [4095, 4095, 4095, 4095, 0, 0, 0, 0]);
    let (_, _, px) = linear(&f);
    for p in px {
        for c in p {
            assert!((0.0..=1.0).contains(&c));
        }
    }
}

// ---------------------------------------------------------------------------
// Preview path, hostile input, cancellation, thumbnails
// ---------------------------------------------------------------------------

/// A builder's file with previews in IFD1 (`a`, blob 1) and IFD2 (`b`,
/// blob 2) and the given IFD0 orientation.
fn with_previews(mut b: Builder, a: Vec<u8>, c: Vec<u8>, orientation: u16) -> Vec<u8> {
    b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![orientation]));
    b.blobs.truncate(1);
    b.blobs.extend([a, c]);
    b.chain = vec![
        vec![(0x0201, Val::Blob(1)), (0x0202, Val::BlobLen(1))],
        vec![(0x0201, Val::Blob(2)), (0x0202, Val::BlobLen(2))],
    ];
    b.build()
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

/// Change the maker-note entries of a builder.
fn maker_with(mut b: Builder, tag: u16, v: Option<Val>) -> Builder {
    let m = b.maker.as_mut().unwrap();
    m.entries = match v {
        Some(v) => with(m.entries.clone(), tag, v),
        None => without(m.entries.clone(), tag),
    };
    b
}

#[test]
fn pef_out_of_scope_falls_back_to_preview() {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let pv = |b: Builder| with_previews(b, small.clone(), big.clone(), 1);
    // The fixtures with previews develop; every variant is in scope.
    for (b, v) in [
        (f1_builder(), Variant::P1),
        (f3_builder(), Variant::P3),
        (f4_builder(), Variant::P3),
        (f5_builder(), Variant::P4),
    ] {
        assert_eq!(report(&pv(b)).origin, Origin::Sensor(v));
    }
    let check = |f: &[u8], class: Option<Class>, what: &str| {
        let dev = report(f);
        // R-PEF-54: the largest candidate (IFD2 here) is chosen.
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        if class.is_some() {
            assert_eq!(dev.class, class, "{what}");
        }
    };
    let x2 = Some(Class::OutOfScope(Relative::X2));
    let x3 = Some(Class::OutOfScope(Relative::X3));
    let set0 = |mut b: Builder, tag: u16, v: Val| {
        b.ifd0 = with(b.ifd0, tag, v);
        b
    };
    // X2 (R-PEF-3 rule 4): Compression 65535 with BPS 16, a table that is
    // not valid, a missing table, PhotometricInterpretation 2, or
    // SamplesPerPixel 3.
    check(
        &pv(set0(f3_builder(), 0x0102, Val::Short(vec![16]))),
        x2,
        "BPS 16",
    );
    check(
        &pv(set0(f3_builder(), 0x0102, Val::Short(vec![14]))),
        x2,
        "BPS 14 with a 12-bit table",
    );
    let mut t = hex(TV5);
    t[1] = 3;
    check(
        &pv(maker_with(f3_builder(), 0x0220, Some(Val::Undef(t)))),
        x2,
        "table V",
    );
    check(&pv(maker_with(f3_builder(), 0x0220, None)), x2, "no table");
    check(
        &pv(set0(f3_builder(), 0x0106, Val::Short(vec![2]))),
        x2,
        "RGB",
    );
    check(
        &pv(set0(f3_builder(), 0x0115, Val::Short(vec![3]))),
        x2,
        "SPP 3",
    );
    // X3: another compression, P1 with BPS 14, a packed strip of another
    // length, W odd for P2.
    check(
        &pv(set0(f1_builder(), 0x0103, Val::Short(vec![7]))),
        x3,
        "JPEG",
    );
    check(
        &pv(set0(f1_builder(), 0x0102, Val::Short(vec![14]))),
        x3,
        "P1 BPS 14",
    );
    let mut b = f1_builder();
    b.ifd0 = with(b.ifd0, 0x0103, Val::Short(vec![32773]));
    b.blobs = vec![vec![0; 13]];
    check(&pv(b), x3, "packed length");
    let mut b = f1_builder();
    b.ifd0 = common_ifd0((3, 2), 12, 32773, 32803, "PENTAX *ist DS", 1);
    b.blobs = vec![vec![0; 9]];
    check(&pv(b), x3, "packed W odd");
    let mut b = f1_builder();
    b.ifd0 = without(b.ifd0, 0x0117);
    check(&pv(b), x3, "P1 without a byte count");
    // X3 by R-PEF-3 rule 1: the maker note cannot be parsed (R-PEF-71,
    // R-PEF-14, R-PEF-15, R-PEF-17).
    let mut b = f3_builder();
    b.exif = None;
    b.maker = None;
    check(&pv(b), x3, "no Exif IFD");
    let mut b = f3_builder();
    b.maker = None;
    check(&pv(b), x3, "no maker note");
    let mut b = f3_builder();
    b.exif = Some(vec![
        (0xA302, Val::Undef(RGGB.to_vec())),
        (
            0x927C,
            Val::Undef(hex("41 4F 43 00 4D 4D 00 01 00 00 00 00 00")),
        ),
    ]);
    b.maker = None;
    check(&pv(b), x3, "maker note count 13");
    let f = pv(f3_builder());
    let m = f.windows(4).position(|w| w == b"AOC\0").unwrap();
    let mut bad = f.clone();
    bad[m + 1] = b'X';
    check(&bad, x3, "unknown form");
    let mut bad = f.clone();
    bad[m + 4] = b'I';
    check(&bad, x3, "byte order bytes");
    let mut bad = f.clone();
    bad[m + 6..m + 8].copy_from_slice(&[0, 0]);
    check(&bad, x3, "maker-note IFD with N = 0");
    // R-PEF-71: a value the decoder reads lies outside the file.
    let at = f
        .windows(4)
        .position(|w| w == [0x02, 0x20, 0x00, 0x07])
        .unwrap();
    let mut bad = f.clone();
    bad[at + 8..at + 12].copy_from_slice(&0x00FF_0000u32.to_be_bytes());
    check(&bad, x3, "table outside the file");

    // Failed checks of a specified variant use the preview (R-PEF-57):
    // R-PEF-18 a required tag missing.
    let p1 = Some(Class::Sensor(Variant::P1));
    let p3 = Some(Class::Sensor(Variant::P3));
    let mut b = f1_builder();
    b.ifd0 = without(b.ifd0, 0x0111);
    check(&pv(b), p1, "StripOffsets missing");
    let mut b = f3_builder();
    b.ifd0 = without(b.ifd0, 0x0100);
    check(&pv(b), p3, "ImageWidth missing");
    let mut b = f3_builder();
    b.exif = Some(Vec::new());
    check(&pv(b), p3, "CFAPattern missing");
    // A count of 0 is absent.
    let mut b = f3_builder();
    b.exif = Some(vec![(0xA302, Val::Raw(7, 0, Vec::new()))]);
    check(&pv(b), p3, "CFAPattern count 0");
    // R-PEF-19: three greens (TV-3).
    let mut b = f3_builder();
    b.exif = Some(vec![(0xA302, Val::Undef(hex("00 02 00 02 00 01 01 01")))]);
    check(&pv(b), p3, "CFA three greens");
    // R-PEF-8: the pattern as BYTE is accepted.
    let mut b = f3_builder();
    b.exif = Some(vec![(0xA302, Val::Byte(RGGB.to_vec()))]);
    assert_eq!(report(&pv(b)).origin, Origin::Sensor(Variant::P3));
    // R-PEF-20: two strips; a strip outside the file.
    let mut b = f1_builder();
    b.ifd0 = with(b.ifd0, 0x0111, Val::Long(vec![0x110, 0x110]));
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![16, 16]));
    check(&pv(b), p1, "two strips");
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![1 << 30]));
    check(&pv(b), p3, "strip outside the file");
    // R-PEF-64: fewer than ceil(W * H / 8) bytes.
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![3]));
    check(&pv(b), p3, "strip shorter than one bit per pixel");
    // R-PEF-35: a strip that ends inside a code word.
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![50]));
    check(&pv(b), p3, "strip ends inside a pixel");
    // R-PEF-31: a stream that no code matches (a table with a hole).
    let mut t = hex(TV5);
    t[14 + 2 * 13 + 12] = 11;
    let mut b = maker_with(f3_builder(), 0x0220, Some(Val::Undef(t)));
    b.blobs = vec![vec![0xFF; 54]];
    check(&pv(b), p3, "no code matches");
    // R-PEF-60: W * H above 2^27, W above 65535.
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0100, Val::Long(vec![65535]));
    b.ifd0 = with(b.ifd0, 0x0101, Val::Long(vec![4096]));
    check(&pv(b), p3, "W * H above 2^27");
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0100, Val::Long(vec![65536]));
    check(&pv(b), p3, "W above 65535");
    // R-PEF-23: a crop that does not fit uses the whole raster; R-PEF-22:
    // 0x023E of another type is not used.
    let mut b = maker_with(f3_builder(), 0x0039, Some(Val::Short(vec![8, 4])));
    let p = plan_of(&b.build());
    assert_eq!((p.crop.x, p.crop.w), (0, 8), "crop beyond the raster");
    b = maker_with(f3_builder(), 0x0039, Some(Val::Short(vec![0, 4])));
    assert_eq!(plan_of(&b.build()).crop.w, 8, "empty crop");
    b = maker_with(f3_builder(), 0x0039, Some(Val::Short(vec![4, 4, 4])));
    assert_eq!(plan_of(&b.build()).crop.w, 8, "three values");
    b = maker_with(f3_builder(), 0x023E, Some(Val::Short(vec![2, 0, 4, 4])));
    assert_eq!(plan_of(&b.build()).crop.x, 1, "0x023E as SHORT");
    b = maker_with(
        f3_builder(),
        0x023E,
        Some(Val::Undef(hex("00 02 00 00 00 04 00 04"))),
    );
    assert_eq!(plan_of(&b.build()).crop.x, 2, "0x023E before 0x0038");
    // R-PEF-49: a crop of one pixel, of one row and of one column: missing
    // colors without a neighbor inside the crop are 0.
    for (rect, size) in [
        ([3u16, 1, 1, 1], (1, 1)),
        ([1, 2, 4, 1], (4, 1)),
        ([2, 0, 1, 4], (1, 4)),
    ] {
        let b = maker_with(
            f3_builder(),
            0x023E,
            Some(Val::Undef(
                rect.iter().flat_map(|v| v.to_be_bytes()).collect(),
            )),
        );
        let f = b.build();
        let (w, h, px) = linear(&f);
        assert_eq!((w, h), size, "crop {rect:?}");
        assert!(px.iter().flatten().all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(report(&f).origin, Origin::Sensor(Variant::P3));
    }
    // Crop (3, 1, 1, 1) is the blue pixel 1028 alone: red and green are 0.
    let b = maker_with(
        f3_builder(),
        0x023E,
        Some(Val::Undef(hex("00 03 00 01 00 01 00 01"))),
    );
    let (_, _, px) = linear(&b.build());
    let p = plan_of(&b.build());
    let blue = (1028.0 - 128.0) / 3967.0 * p.wb[2];
    let m = p.matrix;
    let want = [0, 1, 2].map(|r| ((m[r][2] * blue) as f32).clamp(0.0, 1.0));
    assert_rgb(px[0], want, EPS, "single blue pixel");

    // X1 (R-PEF-52): SubIFDs are never read; IFD0 decodes alone.
    let mut b = f3_builder();
    b.ifd0.push((0x014A, Val::Long(vec![0xFFFF_FF00, 8, 0])));
    let dev = report(&pv(b));
    assert_eq!(dev.origin, Origin::Sensor(Variant::P3));
    assert!(dev.info.unwrap().multi_frame);

    // R-PEF-57: the next-largest candidate when the largest fails.
    let mut broken = big.clone();
    let sos = broken.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    broken.truncate(sos + 4);
    let x2b = set0(f3_builder(), 0x0102, Val::Short(vec![16]));
    let f = with_previews(x2b.clone(), small.clone(), broken, 1);
    assert_preview(&report(&f), (16, 8), [40, 40, 200], "next-largest");
    // An error only when no candidate decodes.
    let f = with_previews(x2b.clone(), vec![0xFF, 0xD8, 0, 0], vec![1, 2, 3, 4], 1);
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    // R-PEF-53: a candidate outside the file or without FF D8 is dropped.
    let mut b = x2b.clone();
    b.chain = vec![
        vec![
            (0x0201, Val::Long(vec![1_000_000])),
            (0x0202, Val::Long(vec![10])),
        ],
        vec![(0x0201, Val::Long(vec![8])), (0x0202, Val::Long(vec![10]))],
        vec![(0x0201, Val::Long(vec![8])), (0x0202, Val::Long(vec![3]))],
    ];
    assert!(preview::collect(&container(&b.build())).is_empty());
    // R-PEF-55: bytes after EOI, and a stream without EOI.
    let mut padded = big.clone();
    padded.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let f = with_previews(x2b.clone(), small.clone(), padded, 1);
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "bytes after EOI");
    let no_eoi = big[..big.len() - 2].to_vec();
    let f = with_previews(x2b.clone(), small.clone(), no_eoi, 1);
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "no EOI");
    // R-PEF-56: orientation 6 rotates a wide preview, not a tall one.
    let f = with_previews(x2b.clone(), small.clone(), big.clone(), 6);
    assert_preview(&report(&f), (32, 48), [200, 40, 40], "rotated preview");
    let tall = jpeg(32, 48, [200, 40, 40]);
    let f = with_previews(x2b.clone(), small.clone(), tall, 6);
    assert_preview(&report(&f), (32, 48), [200, 40, 40], "tall preview");
    // R-PEF-4, R-PEF-11: other data and an invalid IFD0 are errors.
    assert!(decode_report(b"PK\x03\x04 not a TIFF file", &DecodeOptions::default()).is_err());
    let mut bad = pv(f3_builder());
    bad[8..10].copy_from_slice(&[0, 0]);
    assert!(decode_report(&bad, &DecodeOptions::default()).is_err());

    // Through the registry: PEF by extension and by structure.
    let reg = crate::default_registry();
    let x2f = pv(x2b);
    let src = pandite_core::Source::from_bytes(x2f.clone(), Some("IMGP0001.PEF".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (48, 32));
    for (f, size) in [(pv(f3_builder()), (4, 4)), (f5(), (8, 2)), (f2(), (4, 2))] {
        assert!(is_pef(&f, None));
        let src = pandite_core::Source::from_bytes(f, None);
        let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
        assert_eq!(img.first_image().format(), PixelFormat::Rgba16);
        let meta = reg.probe(&src).unwrap();
        assert_eq!(
            (meta.width, meta.height, meta.format),
            (size.0, size.1, "raw")
        );
    }
    // A TIFF without a Pentax maker note is not taken as PEF.
    let mut b = f3_builder();
    b.maker = None;
    assert!(!is_pef(&b.build(), None));
    assert!(is_pef(&b.build(), Some("pef")));
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F5", f5()),
        (
            "F3+previews",
            with_previews(f3_builder(), small.clone(), big.clone(), 6),
        ),
        ("F4+previews", with_previews(f4_builder(), small, big, 1)),
    ]
}

#[test]
fn pef_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let mut next = noise(0x9E37_79B9);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        let le = f[0] == 0x49;
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
                // Damage the directories, the maker note and the table.
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
                // Huge values in IFD fields, in the file's byte order.
                _ => {
                    let i = (next() as usize % bad.len().min(400)) & !1;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27, 8][next() as usize % 6];
                    let b = if le { v.to_le_bytes() } else { v.to_be_bytes() };
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&b);
                    }
                }
            }
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&bad);
                let _ = facts(&bad);
                let _ = has_preview(&bad);
                let _ = is_pef(&bad, None);
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
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.pef".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized claims are refused before allocation (R-PEF-60, R-PEF-61,
    // R-PEF-64, R-PEF-65).
    for (w, h) in [
        (65535u32, 65535u32),
        (0, 2),
        (4, 0),
        (70000, 2),
        (16384, 8193),
    ] {
        let mut b = f3_builder();
        b.ifd0 = with(b.ifd0, 0x0100, Val::Long(vec![w]));
        b.ifd0 = with(b.ifd0, 0x0101, Val::Long(vec![h]));
        let f = b.build();
        let c = container(&f);
        assert!(meta::plan(&c, Variant::P3).is_err(), "{w} x {h}");
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    }
    // A Model claiming more bytes than the file fails (R-PEF-61).
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0110, Val::Raw(2, u32::MAX, vec![0; 4]));
    let f = b.build();
    assert!(meta::plan(&container(&f), Variant::P3).is_err());
    // Strip offset plus count beyond 2^32 is checked in 64 bits (R-PEF-59).
    let mut b = f3_builder();
    b.ifd0 = with(b.ifd0, 0x0111, Val::Long(vec![u32::MAX]));
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![u32::MAX]));
    assert!(meta::plan(&container(&b.build()), Variant::P3).is_err());
}

#[test]
fn pef_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        // The sensor data may still develop when the cut leaves the whole
        // strip (the previews follow it in these files).
        let (off, len) = plan_of(&f).strip;
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 7, 8, 9, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) if dev.origin != Origin::Preview => {
                    assert!(cut >= off + len, "{name} cut at {cut}: {:?}", dev.origin)
                }
                Ok(Ok(Some(_))) => {}
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large P3 file: 4096 x 4096 pixels, all 0 (the SSSS 0 code of TV-5 for
/// every pixel).
fn large_p3() -> Vec<u8> {
    let (w, h) = (4096usize, 4096usize);
    let codes = codes(TV5, 12, false);
    let mut b = f3_builder();
    b.ifd0 = common_ifd0((w as u32, h as u32), 12, 65535, 32803, "PENTAX K10D", 1);
    b = maker_with(b, 0x0039, None);
    b = maker_with(b, 0x0038, None);
    // 5 bits per pixel: 11110 repeated, eight pixels in five bytes.
    let (c, l) = codes[0];
    assert_eq!((c, l), (0b11110, 5));
    let unit = hex("F7 BD EF 7B DE");
    let strip: Vec<u8> = unit.iter().copied().cycle().take(w * h * 5 / 8).collect();
    b.blobs = vec![strip];
    b.build()
}

#[test]
fn pef_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f4(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f4(), Some("a.pef".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread.
    let f = large_p3();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor(Variant::P3));
    assert_eq!((dev.image.width(), dev.image.height()), (4096, 4096));
    drop(dev);
    // The Huffman stream alone is sequential: cancel inside it.
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

#[test]
fn pef_target_size_uses_preview() {
    // A 64 x 32 P1 sensor with previews of 16 x 8 and 48 x 32 (wide).
    let (w, h) = (64u32, 32u32);
    let vals: Vec<u8> = (0..w * h)
        .flat_map(|i| (1000 + (i % 2000) as u16).to_be_bytes())
        .collect();
    let mut b = f1_builder();
    b.ifd0 = common_ifd0((w, h), 12, 1, 32803, "PENTAX *ist D", 1);
    b.blobs = vec![vals];
    let f = with_previews(
        b,
        jpeg(16, 8, [10, 200, 10]),
        jpeg(32, 16, [200, 10, 10]),
        1,
    );
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    // The smallest preview that covers the target.
    let dev = decode_report(&f, &opts((10, 5))).unwrap().unwrap();
    assert_preview(&dev, (16, 8), [10, 200, 10], "small target");
    let dev = decode_report(&f, &opts((30, 30))).unwrap().unwrap();
    assert_preview(&dev, (32, 16), [200, 10, 10], "medium target");
    // A target larger than every preview develops the sensor data.
    let dev = decode_report(&f, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::P1));
    assert_eq!((dev.image.width(), dev.image.height()), (64, 32));
    // Through the registry: a thumbnail request returns 8-bit preview data.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f, Some("t.pef".into()));
    let img = reg.decode(&src, &opts((12, 12))).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (64, 32));
    assert!(has_preview(src.bytes()));
}

/// Header facts of the fixtures through the provider: storage, CFA layout,
/// sample depth and the camera of IFD0.
#[test]
fn facts_pef() {
    let f = facts(&f3()).expect("PEF facts");
    assert_eq!(f.container.as_deref(), Some("Pentax PEF"));
    assert_eq!(f.compression.as_deref(), Some("Pentax Huffman"));
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(12)));
    assert_eq!(f.camera.model.as_deref(), Some("PENTAX K10D"));
    let compression = |d: &[u8]| facts(d).and_then(|f| f.compression);
    assert_eq!(compression(&f1()).as_deref(), Some("Uncompressed"));
    assert_eq!(compression(&f2()).as_deref(), Some("Pentax 12-bit packed"));
    let mono = facts(&f5()).expect("P4 facts");
    assert_eq!(mono.channels.as_deref(), Some("Gray"));
    assert_eq!(mono.bits_per_channel, Some(14));
    // Through the registry by file name.
    let reg = crate::default_registry();
    let via = reg
        .facts(&pandite_core::Source::from_bytes(
            f4(),
            Some("a.pef".into()),
        ))
        .expect("registry facts");
    assert_eq!(via.container.as_deref(), Some("Pentax PEF"));
    assert_eq!(via.compression.as_deref(), Some("Pentax Huffman"));
    for n in 0..f4().len() {
        let _ = facts(&f4()[..n]);
    }
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// The crop rectangles of the specification's table of measured crops,
/// by raster size and variant (the *ist D and *ist DS share a raster).
fn specified_crop(w: usize, h: usize, v: Variant) -> Option<(usize, usize, usize, usize)> {
    Some(match (w, h, v) {
        (3040, 2024, Variant::P1) => (19, 11, 3008, 2008),
        (3040, 2024, Variant::P2) => (16, 8, 3008, 2008),
        (3936, 2624, _) => (8, 8, 3872, 2592),
        (4864, 3136, _) => (8, 8, 4672, 3104),
        (4736, 3136, _) => (10, 10, 4672, 3104),
        (4352, 2868, _) => (10, 10, 4288, 2848),
        (4992, 3284, _) => (22, 10, 4928, 3264),
        (7424, 5552, _) => (80, 59, 7264, 5440),
        (5504, 3664, _) => (10, 4, 5472, 3648),
        (6080, 4032, _) => (16, 16, 6016, 4000),
        (6080, 4064, _) => (66, 32, 6000, 4000),
        (6112, 4060, _) => (60, 44, 6016, 4000),
        (7392, 4950, _) => (14, 28, 7360, 4912),
        (4832, 3238, _) => (14, 28, 4800, 3200),
        (6304, 4224, _) => (54, 58, 6192, 4128),
        (8384, 6208, _) => (58, 8, 8256, 6192),
        _ => return None,
    })
}

/// Models whose developed colors the specification itself does not bring
/// within the chromaticity gate of the Sony test, with the reason. Both use
/// the generic matrix (no model calibration exists, OQ-7). For them the
/// chromaticity distances are printed and not gated; the rank correlation
/// and the cell count stay gated. Every other file meets the Sony gate.
const CHROMA_EXCEPTIONS: &[(&str, &str)] = &[
    (
        "PENTAX K-3 Mark III",
        "generic matrix (OQ-7); M17 records 0.184, the largest chromaticity error of the kept samples, from the higher saturation of the camera JPEG in green foliage",
    ),
    (
        "PENTAX *ist DS",
        "generic matrix (OQ-7); one saturated red cell; the 3008 x 2000 preview is not an exact crop of the raster (M9)",
    ),
];

/// Every `.PEF` file in the folder named by `PANDITE_PEF_SAMPLES` (absolute,
/// or relative to the workspace root) decodes through the sensor-data path
/// when its variant is in scope, at the crop size of the specification, and
/// agrees with the file's own largest embedded JPEG by the method and
/// tolerances of `arw_real_samples_match_embedded_preview`: 8 x 8 grid cell
/// means of the default display rendering, exposure matched by one gain on
/// luminance, rg chromaticity distance (mean 0.05, largest 0.15) over the
/// usable cells and a Spearman rank correlation of cell luminance of at
/// least 0.90; a global per-channel balance first when the white balance
/// is not as shot. The models of [`CHROMA_EXCEPTIONS`] have their
/// chromaticity distances printed, not gated. Out-of-scope files must show
/// their preview. The Huffman streams must end in the last byte of their
/// strip (M5), and the strips of TV-8 and TV-9 must start with the values
/// of the vectors.
#[test]
fn pef_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_PEF_SAMPLES") else {
        println!("skip: PANDITE_PEF_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("pef"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .PEF files in {}", folder.display());
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        assert!(is_pef(&data, None), "{name}: not detected by structure");
        let c = container(&data);
        let class = meta::classify(&c).expect("classify");
        let start = Instant::now();
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("PEF path");
        let took = start.elapsed();
        let Class::Sensor(variant) = class else {
            assert_eq!(dev.origin, Origin::Preview, "{name}");
            println!(
                "{name}: {class:?} is out of scope, preview {}x{}",
                dev.image.width(),
                dev.image.height()
            );
            continue;
        };
        // The sensor-data path, not the preview fallback.
        assert_eq!(
            dev.origin,
            Origin::Sensor(variant),
            "{name} fell back to the preview"
        );
        let info = dev.info.clone().expect("raw facts");
        let plan = plan_of(&data);
        // The specified size: the crop rectangle of the measured table.
        let crop = (info.crop.x, info.crop.y, info.crop.w, info.crop.h);
        let want = specified_crop(info.raster.0, info.raster.1, variant);
        if want.is_some() {
            assert_eq!(Some(crop), want, "{name}: crop rectangle");
        } else {
            println!("{name}: raster not in the table of measured crops");
        }
        let (cw, ch) = render::output_size(&plan);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (cw, ch),
            "{name}"
        );
        // M5: the Huffman stream ends in the last byte of the strip.
        let mut stream_note = String::new();
        if let Some(t) = &plan.table {
            let (off, len) = plan.strip;
            let strip = &data[off..off + len];
            let (mut lo, mut hi) = (u16::MAX, 0u16);
            let coded = huffman::decode(
                strip,
                t,
                (plan.width, plan.height),
                plan.bps,
                &CancelToken::new(),
                |_, _, v| {
                    lo = lo.min(v);
                    hi = hi.max(v);
                },
            )
            .expect("stream");
            let unused = 8 * len as u64 - coded;
            assert!(unused <= 7, "{name}: {unused} unused bits");
            stream_note = format!(" | stream: {unused} unused bits, values {lo} to {hi}");
            // TV-8 and TV-9: the first values of row 0.
            let row0 = |n: usize| {
                let mut v = Vec::new();
                huffman::decode(
                    strip,
                    t,
                    (n, 1),
                    plan.bps,
                    &CancelToken::new(),
                    |_, _, x| v.push(x),
                )
                .unwrap();
                v
            };
            if off == 85472 && plan.width == 3936 {
                assert_eq!(row0(10), [40, 73, 39, 82, 43, 76, 36, 76, 38, 80], "TV-8");
                assert_eq!(unused, 0, "TV-8: the K10D stream uses every bit");
                stream_note.push_str(" | TV-8 ok");
            }
            if off == 204800 && plan.width == 6304 {
                assert_eq!(row0(8), [262, 256, 260, 258, 260, 258, 253, 258], "TV-9");
                stream_note.push_str(" | TV-9 ok");
            }
        }
        // The camera's largest JPEG, decoded small and oriented (R-PEF-56).
        let cands = preview::collect(&c);
        let best = cands.first().expect("an embedded preview");
        // M9: the IFD2 preview has the crop size, except in the *ist DS
        // family (P2), whose preview is 3008 x 2000.
        if variant != Variant::P2 {
            assert_eq!(
                best.displayed(info.orientation),
                (cw as u32, ch as u32),
                "{name}: full-size JPEG"
            );
        }
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev = crate::raw::arw::preview::decode(&data, best, info.orientation, &opts)
            .expect("preview decodes");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        let balance = matches!(info.wb, WbSource::Table | WbSource::Generic);
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let line = format!(
            "{name}: model {} {variant:?} raster {}x{} crop {}x{}+{}+{} out {}x{} wb {:?} matrix {}{} decode {:.2}s (debug) | vs embedded {}x{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}{}{stream_note}",
            info.model,
            info.raster.0,
            info.raster.1,
            info.crop.w,
            info.crop.h,
            info.crop.x,
            info.crop.y,
            cw,
            ch,
            info.wb,
            match (variant, info.matrix_from_table) {
                (Variant::P4, _) => "none",
                (_, true) => "table",
                (_, false) => "generic",
            },
            if info.multi_frame { " multi-frame (IFD0 only)" } else { "" },
            took.as_secs_f64(),
            best.width,
            best.height,
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        );
        let chroma_ok = mean <= CHROMA_MEAN && max <= CHROMA_MAX;
        let exception = CHROMA_EXCEPTIONS
            .iter()
            .find(|e| e.0 == info.model && !info.matrix_from_table);
        if rank < RANK_MIN || used < 16 || (!chroma_ok && exception.is_none()) {
            failures.push(line.clone());
        }
        println!("{line}");
        if let (false, Some((_, why))) = (chroma_ok, exception) {
            println!(
                "{name}: TOLERANCE EXCEPTION: chroma outside the gate of the Sony test (mean {CHROMA_MEAN}, max {CHROMA_MAX}): {why}"
            );
        }
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

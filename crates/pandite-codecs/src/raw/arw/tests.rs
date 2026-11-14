//! Tests of the ARW decoder against the specification's test vectors
//! (TV-1 to TV-13), fixtures (F1 to F5) and requirements, plus hostile
//! input, cancellation, thumbnails and real camera files.

use super::fixtures::*;
use super::lj92::{self, LineStart, Target};
use super::meta::{self, Class, Plan, Relative, Variant, WbSource};
use super::render::{self, Sensor};
use super::tiles;
use super::*;
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// TV-12 and F1 to F4: the expected linear sRGB of every output pixel.
const TV12: [f32; 3] = [0.362_867_6, 0.305_728_5, 0.050_974_7];
/// The specification prints 7 decimals and rounds its own intermediates;
/// a match within 1e-6 is a match at the printed precision.
const EPS: f32 = 1e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open").expect("little-endian TIFF")
}

fn plan_of(data: &[u8]) -> Plan {
    let c = container(data);
    let Class::Sensor(v) = meta::classify(&c).expect("classify") else {
        panic!("not a sensor variant");
    };
    meta::plan(&c, v).expect("plan")
}

fn sensor_of(data: &[u8], plan: &Plan) -> Sensor {
    super::decode_sensor(data, plan, &CancelToken::new()).expect("decode sensor")
}

/// The CFA plane as row-major W x H values.
fn cfa_values(data: &[u8]) -> (usize, usize, Vec<u16>) {
    let plan = plan_of(data);
    let Sensor::Cfa(p) = sensor_of(data, &plan) else {
        panic!("not CFA");
    };
    let mut v = Vec::new();
    for y in 0..plan.height {
        for tc in 0..p.across {
            v.extend_from_slice(p.row(y, tc));
        }
    }
    (plan.width, plan.height, v)
}

/// Render a plan to linear sRGB values (R-ARW-53), oriented and cropped.
fn linear_with(data: &[u8], plan: &Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let sensor = sensor_of(data, plan);
    let bytes = render::render(plan, &sensor, &CancelToken::new(), &LinearF32).expect("render");
    let (w, h) = render::output_size(plan.crop, plan.orientation);
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

/// Delivers the linear sRGB values themselves, 12 bytes per pixel.
pub(crate) struct LinearF32;

impl render::RowEncoder for LinearF32 {
    fn pixel_bytes(&self) -> usize {
        12
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        for (k, b) in out.chunks_exact_mut(12).enumerate() {
            let v = render::to_srgb(m, [cam[0][k], cam[1][k], cam[2][k]]);
            for k in 0..3 {
                b[4 * k..4 * k + 4].copy_from_slice(&v[k].to_ne_bytes());
            }
        }
    }
}

fn linear(data: &[u8]) -> (usize, usize, Vec<[f32; 3]>) {
    linear_with(data, &plan_of(data))
}

fn report(data: &[u8]) -> Developed {
    decode_report(data, &DecodeOptions::default())
        .expect("decode")
        .expect("ARW path")
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
/// within 2 codes of 65535 (the encoding table quantizes linear values to
/// 1/65535).
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

/// A file with the given raw entries and blobs and the common IFD0.
fn file(
    model: &'static str,
    orientation: u16,
    raw: Vec<(u16, Val)>,
    blobs: Vec<Vec<u8>>,
) -> Vec<u8> {
    Builder {
        ifd0: common_ifd0(model, orientation),
        raw,
        blobs,
        sub_ifds: true,
        ..Builder::default()
    }
    .build()
}

/// Replace or add a raw entry.
fn with(mut raw: Vec<(u16, Val)>, tag: u16, v: Val) -> Vec<(u16, Val)> {
    raw.retain(|e| e.0 != tag);
    raw.push((tag, v));
    raw
}

fn without(mut raw: Vec<(u16, Val)>, tag: u16) -> Vec<(u16, Val)> {
    raw.retain(|e| e.0 != tag);
    raw
}

/// A deterministic pseudo-random sequence.
pub(crate) fn noise(seed: u32) -> impl FnMut() -> u32 {
    let mut s = seed.max(1);
    move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    }
}

/// A V1 file of `w` x `h` 14-bit values with the common raw tags.
fn v1_file(w: u32, h: u32, values: &[u16], extra: Vec<(u16, Val)>, orientation: u16) -> Vec<u8> {
    let data: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut raw = strip_raw(w, h, 14, 1, h);
    for (t, v) in extra {
        raw = with(raw, t, v);
    }
    file("ILCE-7M4", orientation, raw, vec![data])
}

// ---------------------------------------------------------------------------
// Container (TV-1, TV-2)
// ---------------------------------------------------------------------------

#[test]
fn arw_container_vectors() {
    // TV-1, R-ARW-4, R-ARW-11: little-endian header, IFD0 at 8.
    assert_eq!(
        ifd::header(&hex("49 49 2A 00 08 00 00 00")).unwrap(),
        Some(8)
    );
    // Big-endian is X6: not taken by the ARW path (preview path).
    assert_eq!(ifd::header(&hex("4D 4D 00 2A 00 00 00 08")).unwrap(), None);
    // R-ARW-65: shorter than 8 bytes fails.
    assert!(ifd::header(&hex("49 49 2A 00 08 00")).is_err());

    // TV-2: four entries in one IFD, with the values of the vectors at
    // 0x114 and 0x124 (R-ARW-6, R-ARW-7: unsorted on purpose).
    let mut d = vec![0u8; 0x130];
    d[..8].copy_from_slice(&hex("49 49 2A 00 08 00 00 00"));
    let entries = [
        "13 73 08 00 04 00 00 00 24 01 00 00",
        "03 01 03 00 01 00 00 00 FF 7F 00 00",
        "10 70 03 00 04 00 00 00 14 01 00 00",
        "8E 82 01 00 04 00 00 00 00 01 01 02",
        // R-ARW-9: type 14 is outside 1 to 13 and is skipped.
        "99 99 0E 00 01 00 00 00 05 00 00 00",
    ];
    d[8..10].copy_from_slice(&(entries.len() as u16).to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        d[10 + 12 * i..22 + 12 * i].copy_from_slice(&hex(e));
    }
    d[0x114..0x11C].copy_from_slice(&hex("40 1F A0 28 64 32 14 37"));
    d[0x124..0x12C].copy_from_slice(&hex("7D 09 00 04 00 04 F7 06"));
    let dir = ifd::dir(&d, 8).expect("valid IFD");
    assert_eq!(dir.entries.len(), 4, "type 14 skipped");
    assert!(!dir.has(0x9999));
    assert_eq!(dir.uint(0x0103).unwrap(), Some(32767));
    assert_eq!(
        dir.uints(0x7010).unwrap(),
        Some(vec![8000, 10400, 12900, 14100])
    );
    assert_eq!(dir.uints(0x828E).unwrap(), Some(vec![0, 1, 1, 2]));
    assert_eq!(
        dir.sshorts(0x7313).unwrap(),
        Some(vec![2429, 1024, 1024, 1783])
    );

    // R-ARW-67, R-ARW-68: a value range outside the file fails.
    let mut bad = d.clone();
    bad[10 + 12 * 2 + 8..10 + 12 * 2 + 12].copy_from_slice(&0x12Eu32.to_le_bytes());
    let dir = ifd::dir(&bad, 8).unwrap();
    assert!(dir.uints(0x7010).is_err());
    // A count far beyond the file fails before any allocation.
    let mut huge = d.clone();
    huge[10 + 12 * 2 + 4..10 + 12 * 2 + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(ifd::dir(&huge, 8).unwrap().uints(0x7010).is_err());

    // R-ARW-14: invalid IFDs.
    assert!(ifd::dir(&d, 0x1000).is_none(), "offset outside the file");
    let mut zero = d.clone();
    zero[8..10].copy_from_slice(&0u16.to_le_bytes());
    assert!(ifd::dir(&zero, 8).is_none(), "N = 0");
    let mut many = d.clone();
    many[8..10].copy_from_slice(&1025u16.to_le_bytes());
    assert!(ifd::dir(&many, 8).is_none(), "N > 1024");
    assert!(ifd::dir(&d[..0x40], 8).is_none(), "entries beyond the end");

    // R-ARW-12: the chain stops at a repeated offset and after 16 IFDs.
    let mut looped = d.clone();
    let next_at = 8 + 2 + 12 * entries.len();
    looped[next_at..next_at + 4].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(ifd::chain(&looped, 8).len(), 1);
    let mut long = vec![0u8; 8 + 20 * 18];
    long[..8].copy_from_slice(&hex("49 49 2A 00 08 00 00 00"));
    for i in 0..20usize {
        let at = 8 + 18 * i;
        long[at..at + 2].copy_from_slice(&1u16.to_le_bytes());
        long[at + 2..at + 14].copy_from_slice(&hex("12 01 03 00 01 00 00 00 01 00 00 00"));
        let next = if i < 19 { at + 18 } else { 0 };
        long[at + 14..at + 18].copy_from_slice(&(next as u32).to_le_bytes());
    }
    assert_eq!(ifd::chain(&long, 8).len(), 16);

    // R-ARW-10: ASCII compared after trailing zeros and spaces go.
    let f = f1();
    let c = container(&f);
    assert_eq!(c.model(), b"ILCE-7M4");
    // R-ARW-8: SubIFDs given with the IFD type (13) still works.
    let mut f13 = f.clone();
    let sub = 8 + 2 + 3 * 12;
    assert_eq!(&f13[sub..sub + 2], &[0x4A, 0x01]);
    f13[sub + 2..sub + 4].copy_from_slice(&13u16.to_le_bytes());
    assert_eq!(
        meta::classify(&container(&f13)).unwrap(),
        Class::Sensor(Variant::V1)
    );
}

/// The builder follows the recipe layout: it reproduces F1 and F3 byte for
/// byte.
#[test]
fn arw_builder_reproduces_fixture_hex() {
    let mut raw1 = strip_raw(4, 2, 14, 1, 2);
    raw1.retain(|e| e.0 != 0x0111 && e.0 != 0x0117);
    raw1.push((0x0111, Val::Blobs(vec![0])));
    raw1.push((0x0117, Val::BlobLens(vec![0])));
    let data1 = hex("D0 09 A0 11 D0 09 A0 11 A0 11 E8 05 A0 11 E8 05");
    assert_eq!(file("ILCE-7M4", 1, raw1, vec![data1]), f1());
    let mut raw3 = strip_raw(32, 2, 14, 32767, 2);
    raw3.push((0x7010, Val::Short(vec![8000, 10400, 12900, 14100])));
    let mut data3 = Vec::new();
    for c in [1128u16, 1464, 1464, 756] {
        data3.extend_from_slice(&flat_block(c));
    }
    assert_eq!(file("ILCE-7M4", 1, raw3, vec![data3]), f3());
    assert_eq!(f1().len(), 304);
    assert_eq!(f2().len(), 300);
    assert_eq!(f3().len(), 368);
}

// ---------------------------------------------------------------------------
// V1, V2, V3
// ---------------------------------------------------------------------------

#[test]
fn arw_v1_uncompressed() {
    // TV-8 (R-ARW-27): b0 + 256 * b1.
    let f = f1();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::Sensor(Variant::V1)
    );
    let (w, h, v) = cfa_values(&f);
    assert_eq!((w, h), (4, 2));
    assert_eq!(v, vec![2512, 4512, 2512, 4512, 4512, 1512, 4512, 1512]);
    // F1 expected output: every pixel TV-12, size W x H.
    let (ow, oh, px) = linear(&f);
    assert_eq!((ow, oh), (4, 2));
    for p in px {
        assert_rgb(p, TV12, EPS, "F1");
    }
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    assert_eq!((dev.image.width(), dev.image.height()), (4, 2));
    assert_rgba16_flat(&dev.image, TV12, "F1 delivered");

    // R-ARW-24: two strips of one row each, stored in reverse order.
    let rows: Vec<u8> = hex("D0 09 A0 11 D0 09 A0 11 A0 11 E8 05 A0 11 E8 05");
    let mut raw = strip_raw(4, 2, 14, 1, 1);
    raw = with(raw, 0x0111, Val::Blobs(vec![1, 0]));
    raw = with(raw, 0x0117, Val::BlobLens(vec![1, 0]));
    let two = file(
        "ILCE-7M4",
        1,
        raw,
        vec![rows[8..].to_vec(), rows[..8].to_vec()],
    );
    assert_eq!(cfa_values(&two).2, v);
    // R-ARW-69 case 3: the strip count must be ceil(H / RowsPerStrip).
    let mut raw = strip_raw(4, 2, 14, 1, 2);
    raw = with(raw, 0x0111, Val::Blobs(vec![1, 0]));
    raw = with(raw, 0x0117, Val::BlobLens(vec![1, 0]));
    let bad = file(
        "ILCE-7M4",
        1,
        raw,
        vec![rows[8..].to_vec(), rows[..8].to_vec()],
    );
    let c = container(&bad);
    assert!(meta::plan(&c, Variant::V1).is_err());
    // R-ARW-19: RowsPerStrip 0 or above H counts as H.
    let zero = v1_file(4, 2, &v, vec![(0x0116, Val::Short(vec![0]))], 1);
    assert_eq!(cfa_values(&zero).2, v);
    let big = v1_file(4, 2, &v, vec![(0x0116, Val::Short(vec![9]))], 1);
    assert_eq!(cfa_values(&big).2, v);
}

#[test]
fn arw_v2_packed12() {
    // TV-8 (R-ARW-28): the S21 bytes and the fixture bytes.
    let one = file(
        "DSLR-A900",
        1,
        strip_raw(4, 1, 12, 32767, 1),
        vec![hex("88 40 09 87 D0 09")],
    );
    assert_eq!(
        meta::classify(&container(&one)).unwrap(),
        Class::Sensor(Variant::V2)
    );
    assert_eq!(cfa_values(&one).2, vec![544, 592, 540, 628]);
    let f = f2();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::Sensor(Variant::V2)
    );
    assert_eq!(
        cfa_values(&f).2,
        vec![2512, 4512, 2512, 4512, 4512, 1512, 4512, 1512]
    );
    let (_, _, px) = linear(&f);
    for p in px {
        assert_rgb(p, TV12, EPS, "F2");
    }
    assert_rgba16_flat(&report(&f).image, TV12, "F2 delivered");
}

#[test]
fn arw_v3_block_vectors() {
    let curve = strips::tone_curve([8000, 10400, 12900, 14100]);
    let check = |bytes: &str,
                 max,
                 min,
                 imax,
                 imin,
                 sh,
                 deltas: &[u32],
                 codes: &[u16],
                 lin: Option<&[u16]>| {
        let b: [u8; 16] = hex(bytes).try_into().unwrap();
        // R-ARW-31, R-ARW-32.
        let f = strips::fields(&b);
        assert_eq!(
            (f.max, f.min, f.imax, f.imin, f.sh),
            (max, min, imax, imin, sh),
            "{bytes}"
        );
        if !deltas.is_empty() {
            assert_eq!(&f.deltas[..], deltas, "{bytes}");
        }
        // R-ARW-33.
        let c = strips::block(&b);
        assert_eq!(&c[..], codes, "{bytes}");
        // R-ARW-35.
        if let Some(lin) = lin {
            let l: Vec<u16> = c.iter().map(|&k| curve[usize::from(k)]).collect();
            assert_eq!(&l[..], lin, "{bytes}");
        }
    };
    // TV-3: shift 0.
    check(
        "E8 23 5C B1 82 E2 41 C9 78 46 A8 76 0C 08 08 06",
        1000,
        900,
        5,
        12,
        0,
        &[10, 20, 30, 40, 50, 60, 70, 80, 90, 99, 0, 1, 2, 3],
        &[
            910, 920, 930, 940, 950, 1000, 960, 970, 980, 990, 999, 900, 900, 901, 902, 903,
        ],
        Some(&[
            1820, 1840, 1860, 1880, 1900, 2000, 1920, 1940, 1960, 1980, 1998, 1800, 1800, 1802,
            1804, 1806,
        ]),
    );
    // TV-4: shift 3.
    check(
        "A4 66 22 3C A0 A0 78 50 32 9E 11 AA 25 BB F1 94",
        1700,
        1100,
        0,
        15,
        3,
        &[0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55, 60, 74],
        &[
            1700, 1100, 1140, 1180, 1220, 1260, 1300, 1340, 1380, 1420, 1460, 1500, 1540, 1580,
            1692, 1100,
        ],
        Some(&[
            7100, 2400, 2560, 2720, 2880, 3040, 3200, 3520, 3840, 4160, 4480, 4800, 5120, 5440,
            6972, 2400,
        ]),
    );
    // TV-5: shift 4 and the 2047 limit.
    check(
        "FF 47 DF C3 1F 08 08 08 06 84 82 E1 80 48 28 16",
        2047,
        1000,
        15,
        0,
        4,
        &[127, 64, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        &[
            1000, 2047, 2024, 1000, 1016, 1032, 1048, 1064, 1080, 1096, 1112, 1128, 1144, 1160,
            1176, 2047,
        ],
        None,
    );
    // TV-6: two blocks from a camera file.
    check(
        "E0 A9 4A C3 C1 B4 FB 30 16 02 4C 51 C7 EB 0D 7F",
        480,
        341,
        13,
        0,
        1,
        &[],
        &[
            341, 355, 417, 459, 403, 365, 363, 345, 389, 479, 457, 461, 463, 480, 475, 467,
        ],
        Some(&[
            682, 710, 834, 918, 806, 730, 726, 690, 778, 958, 914, 922, 926, 960, 950, 934,
        ]),
    );
    check(
        "79 FB 50 03 E2 06 5C 18 02 88 DA 12 BA 7C 36 AD",
        889,
        543,
        13,
        0,
        2,
        &[],
        &[
            543, 575, 763, 799, 587, 567, 547, 575, 755, 843, 863, 843, 859, 889, 851, 887,
        ],
        Some(&[
            1086, 1150, 1526, 1598, 1174, 1134, 1094, 1150, 1510, 1686, 1726, 1686, 1718, 1778,
            1702, 1774,
        ]),
    );
    // R-ARW-32: MAX < MIN gives SPAN 0 and shift 0.
    let mut inv = [0u8; 16];
    inv.copy_from_slice(&(10u128 | (20u128 << 11) | (1u128 << 26)).to_le_bytes());
    assert_eq!(strips::fields(&inv).sh, 0);
    // R-ARW-33 step 4: IMAX = IMIN leaves the last pixel at MIN.
    let mut same = [0u8; 16];
    let b = 100u128 | (50u128 << 11) | (3u128 << 22) | (3u128 << 26) | (127u128 << 121);
    same.copy_from_slice(&b.to_le_bytes());
    let c = strips::block(&same);
    assert_eq!(c[3], 100);
    assert_eq!(c[15], 50, "beyond D(14) the code is MIN");
    assert_eq!(c[14], 50 + 127, "D(14) is the 14th remaining pixel");

    // F3: complete file, R-ARW-29, R-ARW-30.
    let f = f3();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::Sensor(Variant::V3)
    );
    let (w, h, v) = cfa_values(&f);
    assert_eq!((w, h), (32, 2));
    for x in 0..32 {
        let want0 = if x % 2 == 0 { 2512 } else { 4512 };
        let want1 = if x % 2 == 0 { 4512 } else { 1512 };
        assert_eq!((v[x], v[32 + x]), (want0, want1), "column {x}");
    }
    let (_, _, px) = linear(&f);
    for p in px {
        assert_rgb(p, TV12, EPS, "F3");
    }
    assert_rgba16_flat(&report(&f).image, TV12, "F3 delivered");

    // R-ARW-30 for chunk j > 0: chunk j covers columns 32j to 32j + 31.
    let mut data = Vec::new();
    for c in [300u16, 400, 500, 600] {
        data.extend_from_slice(&flat_block(c));
    }
    let mut raw = strip_raw(64, 1, 14, 32767, 1);
    raw.push((0x7010, Val::Short(vec![8000, 10400, 12900, 14100])));
    let two = file("ILCE-7M4", 1, raw, vec![data]);
    let (_, _, v) = cfa_values(&two);
    for x in 0..64 {
        let code = [300, 400, 500, 600][(x / 32) * 2 + x % 2];
        assert_eq!(v[x], curve[code], "column {x}");
    }
    // R-ARW-29: W not a multiple of 32 is not V3.
    let raw = strip_raw(48, 1, 14, 32767, 1);
    let odd = file("ILCE-7M4", 1, raw, vec![vec![0; 48]]);
    assert_eq!(
        meta::classify(&container(&odd)).unwrap(),
        Class::OutOfScope(Relative::X4)
    );
}

#[test]
fn arw_v3_tone_curve_vector() {
    // TV-7, R-ARW-35: the standard curve.
    let std_curve = strips::tone_curve([8000, 10400, 12900, 14100]);
    for (c, l) in [
        (0, 0),
        (255, 510),
        (256, 512),
        (257, 514),
        (1000, 2000),
        (1001, 2004),
        (1300, 3200),
        (1301, 3208),
        (1612, 5696),
        (1613, 5708),
        (1762, 8092),
        (1763, 8116),
        (2021, 16372),
        (2022, 16404),
        (2047, 17204),
    ] {
        assert_eq!(std_curve[c], l, "standard curve at {c}");
    }
    // TV-7: the NEX-5 curve.
    let nex = strips::tone_curve([0, 9824, 11512, 16362]);
    for (c, l) in [
        (128, 512),
        (1228, 4912),
        (1439, 6600),
        (2046, 16324),
        (2047, 16356),
    ] {
        assert_eq!(nex[c], l, "NEX-5 curve at {c}");
    }
    // R-ARW-77: extreme knots are clipped to 65535, never overflow.
    let steep = strips::tone_curve([1, 1, 1, 1]);
    assert_eq!(steep[2047], 65500);

    // R-ARW-34: tag values used only when ascending with T4 > 0.
    let f = f3();
    assert_eq!(plan_of(&f).curve, [8000, 10400, 12900, 14100]);
    let mut raw = strip_raw(32, 2, 14, 32767, 2);
    raw.push((0x7010, Val::Short(vec![0, 9824, 11512, 16362])));
    let blocks: Vec<u8> = [1128u16, 1464, 1464, 756]
        .iter()
        .flat_map(|&c| flat_block(c))
        .collect();
    let nex_file = file("NEX-5", 1, raw.clone(), vec![blocks.clone()]);
    assert_eq!(plan_of(&nex_file).curve, [0, 9824, 11512, 16362]);
    for bad in [
        vec![9000, 8000, 12900, 14100],
        vec![0, 0, 0, 0],
        vec![1, 2, 3],
    ] {
        let r = with(raw.clone(), 0x7010, Val::Short(bad.clone()));
        let f = file("NEX-5", 1, r, vec![blocks.clone()]);
        assert_eq!(plan_of(&f).curve, [8000, 10400, 12900, 14100], "{bad:?}");
    }
    let f = file("NEX-5", 1, without(raw, 0x7010), vec![blocks]);
    assert_eq!(plan_of(&f).curve, [8000, 10400, 12900, 14100], "absent");
}

// ---------------------------------------------------------------------------
// V4 and V5
// ---------------------------------------------------------------------------

/// TV-9: the LJ92 CFA stream.
const TV9: &str = "FF D8
FF C3 00 14 0E 00 02 00 02 04 01 11 00 02 11 00 03 11 00 04 11 00
FF C4 00 17 00 00 03 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 02 03
FF DA 00 0E 04 01 00 02 00 03 00 04 00 01 00 00
1A BA 33 06 36 49
FF D9";

/// TV-10: the LJ92 YCbCr stream with the line-start rule.
const TV10: &str = "FF D8
FF C3 00 11 10 00 04 00 02 03 01 22 00 02 11 00 03 11 00
FF C4 00 17 00 00 03 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 02 03
FF DA 00 0C 03 01 00 02 00 03 00 01 00 00
A6 AC 5A 26 E6 BF
FF D9";

/// Decode a 4-component stream into a 2X x 2Y CFA tile (R-ARW-38).
fn cfa_tile(stream: &[u8], x: usize, y: usize) -> pandite_core::Result<Vec<u16>> {
    let s = lj92::parse(stream)?;
    let mut buf = vec![0u16; 4 * x * y];
    let targets: Vec<Target> = (0..4)
        .map(|c| Target {
            buffer: 0,
            base: (c & 1) + (c >> 1) * 2 * x,
            xs: 2,
            ys: 4 * x,
        })
        .collect();
    s.decode(&mut [&mut buf], &targets, LineStart::Sony)?;
    Ok(buf)
}

fn tv10_planes(mode: LineStart) -> (Vec<u16>, Vec<u16>, Vec<u16>) {
    let t = hex(TV10);
    let s = lj92::parse(&t).unwrap();
    let (mut y, mut cb, mut cr) = (vec![0u16; 8], vec![0u16; 2], vec![0u16; 2]);
    let targets = [
        Target {
            buffer: 0,
            base: 0,
            xs: 1,
            ys: 2,
        },
        Target {
            buffer: 1,
            base: 0,
            xs: 1,
            ys: 1,
        },
        Target {
            buffer: 2,
            base: 0,
            xs: 1,
            ys: 1,
        },
    ];
    s.decode(&mut [&mut y, &mut cb, &mut cr], &targets, mode)
        .unwrap();
    (y, cb, cr)
}

/// A V4 file of `across` x `down` tiles with valid area `valid`, values
/// from `f(x, y)`, tiles stored in reverse file order.
fn v4_file(
    across: usize,
    down: usize,
    valid: (u32, u32),
    f: impl Fn(usize, usize) -> u16,
    restart: usize,
) -> Vec<u8> {
    let mut blobs = Vec::new();
    for t in 0..across * down {
        let (tx, ty) = ((t % across) * 512, (t / across) * 512);
        let comp: Vec<Vec<u16>> = (0..4)
            .map(|c| {
                let mut v = Vec::with_capacity(256 * 256);
                for fy in 0..256 {
                    for fx in 0..256 {
                        v.push(f(tx + 2 * fx + (c & 1), ty + 2 * fy + (c >> 1)));
                    }
                }
                v
            })
            .collect();
        let comps: Vec<Comp<'_>> = comp
            .iter()
            .map(|s| Comp {
                h: 1,
                v: 1,
                samples: s,
                width: 256,
            })
            .collect();
        blobs.push(lj92(14, 256, 256, &comps, restart, true));
    }
    blobs.reverse();
    let n = blobs.len();
    let order: Vec<usize> = (0..n).rev().collect();
    let mut raw = vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Long(vec![(across * 512) as u32])),
        (0x0101, Val::Long(vec![(down * 512) as u32])),
        (0x0102, Val::Short(vec![14])),
        (0x0103, Val::Short(vec![7])),
        (0x0106, Val::Short(vec![32803])),
        (0x0115, Val::Short(vec![1])),
        (0x0142, Val::Short(vec![512])),
        (0x0143, Val::Short(vec![512])),
        (0x0144, Val::Blobs(order.clone())),
        (0x0145, Val::BlobLens(order)),
        (0x7038, Val::Long(vec![valid.0, valid.1])),
    ];
    raw.extend(common_raw());
    file("ILCE-7M4", 1, raw, blobs)
}

/// The value pattern of the synthetic V4 file: smooth image inside the
/// valid area, large values in the padding.
fn v4_pattern(valid: (usize, usize)) -> impl Fn(usize, usize) -> u16 {
    move |x, y| {
        if x >= valid.0 || y >= valid.1 {
            16000
        } else {
            (600 + (x * 7 + y * 3) % 5000 + 300 * ((x / 2 + y / 2) % 3)) as u16
        }
    }
}

#[test]
fn arw_v4_lossless_cfa_tiles() {
    // TV-9: component values and the 4 x 4 CFA corner (R-ARW-36, R-ARW-38).
    let tile = cfa_tile(&hex(TV9), 2, 2).expect("TV-9 decodes");
    assert_eq!(
        tile,
        vec![
            8192, 8193, 8194, 8193, //
            8191, 8195, 8187, 8195, //
            8192, 8194, 8193, 8194, //
            8191, 8196, 8189, 8196,
        ]
    );
    // R-ARW-75: TV-9 ends with one padding bit; the decoder must not need
    // more data than the 47 coded bits, even without EOI.
    let t = hex(TV9);
    assert!(cfa_tile(&t[..t.len() - 2], 2, 2).is_ok(), "no EOI");
    // A stream cut inside the coded bits fails.
    assert!(cfa_tile(&t[..t.len() - 3], 2, 2).is_err());

    // F4 (R-ARW-37): every cell R 2512, G 4512, G 4512, B 1512.
    let f = f4();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::Sensor(Variant::V4)
    );
    let (w, h, v) = cfa_values(&f);
    assert_eq!((w, h), (512, 512));
    for y in 0..512 {
        for x in 0..512 {
            let want = [2512, 4512, 4512, 1512][2 * (y % 2) + x % 2];
            assert_eq!(v[y * 512 + x], want, "({x}, {y})");
        }
    }
    let (ow, oh, px) = linear(&f);
    assert_eq!((ow, oh), (512, 512));
    for p in px {
        assert_rgb(p, TV12, EPS, "F4");
    }
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V4));
    assert_rgba16_flat(&dev.image, TV12, "F4 delivered");

    // R-ARW-25, R-ARW-26: 2 x 2 tiles stored in reverse order, valid area
    // smaller than the raster (R-ARW-20), with restart intervals of one MCU
    // row (R-ARW-76).
    let valid = (1000usize, 1010usize);
    let pat = v4_pattern(valid);
    for restart in [0, 256] {
        let f = v4_file(2, 2, (valid.0 as u32, valid.1 as u32), &pat, restart);
        let (w, h, v) = cfa_values(&f);
        assert_eq!((w, h), (1024, 1024));
        for y in 0..h {
            for x in 0..w {
                assert_eq!(v[y * w + x], pat(x, y), "({x}, {y}) restart {restart}");
            }
        }
        let p = plan_of(&f);
        assert_eq!(p.valid, valid);
        assert_eq!(
            p.crop,
            meta::Rect {
                x: 0,
                y: 0,
                w: 1000,
                h: 1010
            }
        );
    }

    // R-ARW-37: a frame that is not P 14, 256 x 256, 4 components fails.
    let comp = vec![0u16; 256 * 256];
    let comps: Vec<Comp<'_>> = (0..4)
        .map(|_| Comp {
            h: 1,
            v: 1,
            samples: &comp,
            width: 256,
        })
        .collect();
    let p12 = lj92(12, 256, 256, &comps, 0, true);
    let s = lj92::parse(&p12).unwrap();
    assert!(tiles::check_v4(&s).is_err());
    assert!(tiles::check_v4(&lj92::parse(&f4_tile()).unwrap()).is_ok());
    let mut raw = without(meta_raw_of_f4(), 0x0145);
    raw.push((0x0145, Val::BlobLens(vec![0])));
    let bad = file("ILCE-7M4", 1, raw, vec![p12]);
    let dev = decode_report(&bad, &DecodeOptions::default());
    assert!(dev.is_err(), "no preview: an error");
}

/// The raw entries of F4 with the tile in blob 0.
fn meta_raw_of_f4() -> Vec<(u16, Val)> {
    let mut raw = vec![
        (0x00FE, Val::Long(vec![0])),
        (0x0100, Val::Short(vec![512])),
        (0x0101, Val::Short(vec![512])),
        (0x0102, Val::Short(vec![14])),
        (0x0103, Val::Short(vec![7])),
        (0x0106, Val::Short(vec![32803])),
        (0x0115, Val::Short(vec![1])),
        (0x0142, Val::Short(vec![512])),
        (0x0143, Val::Short(vec![512])),
        (0x0144, Val::Blobs(vec![0])),
        (0x0145, Val::BlobLens(vec![0])),
        (0x7038, Val::Long(vec![512, 512])),
    ];
    raw.extend(common_raw());
    raw
}

/// A one-tile V5 file with luma sampling V, planes from functions.
fn v5_file(
    v: usize,
    luma: impl Fn(usize, usize) -> u16,
    cb: impl Fn(usize, usize) -> u16,
    cr: impl Fn(usize, usize) -> u16,
    extra: Vec<(u16, Val)>,
    restart: usize,
) -> Vec<u8> {
    let ys: Vec<u16> = (0..512 * 512).map(|i| luma(i % 512, i / 512)).collect();
    let ch = 512 / v;
    let cbs: Vec<u16> = (0..256 * ch).map(|i| cb(i % 256, i / 256)).collect();
    let crs: Vec<u16> = (0..256 * ch).map(|i| cr(i % 256, i / 256)).collect();
    let comps = [
        Comp {
            h: 2,
            v,
            samples: &ys,
            width: 512,
        },
        Comp {
            h: 1,
            v: 1,
            samples: &cbs,
            width: 256,
        },
        Comp {
            h: 1,
            v: 1,
            samples: &crs,
            width: 256,
        },
    ];
    let tile = lj92(16, 512, 512, &comps, restart, true);
    let mut raw = f5_raw();
    raw = with(raw, 0x0145, Val::BlobLens(vec![0]));
    raw = with(raw, 0x0212, Val::Short(vec![2, v as u16]));
    for (t, val) in extra {
        raw = with(raw, t, val);
    }
    file("ILCE-7M4", 1, raw, vec![tile])
}

#[test]
fn arw_v5_ycbcr_tiles() {
    // F5: Y 5000, Cb 16384, Cr 16384 everywhere; output 0.2505198.
    let f = f5();
    assert_eq!(
        meta::classify(&container(&f)).unwrap(),
        Class::Sensor(Variant::V5)
    );
    let plan = plan_of(&f);
    let Sensor::Ycc(p) = sensor_of(&f, &plan) else {
        panic!("not YCbCr");
    };
    assert_eq!(p.v, 2);
    assert!(p.y.data.iter().all(|&v| v == 5000));
    assert!(p.cb.data.iter().all(|&v| v == 16384));
    assert!(p.cr.data.iter().all(|&v| v == 16384));
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (512, 512));
    let want = 0.250_519_8;
    for q in px {
        assert_rgb(q, [want; 3], EPS, "F5");
    }
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V5));
    assert_rgba16_flat(&dev.image, [want; 3], "F5 delivered");
    // R-ARW-49: WB_RGGBLevels are present in F5 but not applied.
    assert_eq!(plan.wb_source, WbSource::NotApplied);
    // R-ARW-18 applies to V1 to V4 only: the F5 CFAPattern 255 is ignored.

    // TV-11: R-ARW-42 and R-ARW-48.
    let ycc = plan.ycc;
    let rgb = render::ycc_rgb(5000.0, 16484.0, 16184.0, &ycc);
    for (g, w) in rgb.iter().zip([4719.5984, 5108.4119, 5177.2003]) {
        assert!((g - w).abs() < 1e-4, "TV-11 RGB {rgb:?}");
    }
    let v = render::v5_levels(rgb, &ycc);
    assert_rgb(
        v,
        [0.232_852_3, 0.257_350_6, 0.261_684_9],
        EPS,
        "TV-11 levels",
    );

    // TV-11 through the whole pipeline with an identity matrix.
    let tv11 = v5_file(2, |_, _| 5000, |_, _| 16484, |_, _| 16184, vec![], 0);
    let mut plan = plan_of(&tv11);
    plan.matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (_, _, px) = linear_with(&tv11, &plan);
    assert_rgb(
        px[1000],
        [0.232_852_3, 0.257_350_6, 0.261_684_9],
        EPS,
        "TV-11 pipeline",
    );

    // R-ARW-48: tag 0x7039 ratios apply; a ratio outside 0.5 to 2 counts
    // as 1; a zero denominator gives the tag default (R-ARW-17).
    let factors = Val::Rational(vec![(2425, 2429), (1024, 1024), (1816, 1802)]);
    let f = v5_file(
        2,
        |_, _| 5000,
        |_, _| 16484,
        |_, _| 16184,
        vec![(0x7039, factors)],
        0,
    );
    let mut plan = plan_of(&f);
    plan.matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (_, _, px) = linear_with(&f, &plan);
    let want = [
        0.232_852_3 * 2425.0 / 2429.0,
        0.257_350_6,
        0.261_684_9 * 1816.0 / 1802.0,
    ];
    assert_rgb(px[0], want, EPS, "0x7039 applied");
    let odd = Val::Rational(vec![(3, 1), (1, 4), (1, 1)]);
    let f = v5_file(
        2,
        |_, _| 5000,
        |_, _| 16384,
        |_, _| 16384,
        vec![(0x7039, odd)],
        0,
    );
    assert_eq!(plan_of(&f).ycc.factors, [1.0, 1.0, 1.0]);
    let zero = Val::Rational(vec![(3, 0), (1, 1), (1, 1)]);
    let f = v5_file(
        2,
        |_, _| 5000,
        |_, _| 16384,
        |_, _| 16384,
        vec![(0x7039, zero)],
        0,
    );
    assert_eq!(plan_of(&f).ycc.factors, [1.0, 1.0, 1.0]);

    // Size S (V = 1) and size M (V = 2) with varying planes and restart
    // intervals: decoded planes equal the encoded ones; R-ARW-41 chroma
    // placement and R-ARW-42 per pixel.
    for (v, restart) in [(1usize, 0usize), (2, 0), (2, 256), (1, 256)] {
        let luma = |x: usize, y: usize| (2000 + (x * 13 + y * 7) % 9000) as u16;
        let cb = |x: usize, y: usize| (16384 + (x % 50) * 10 - (y % 30) * 7) as u16;
        let cr = |x: usize, y: usize| (16384 - (x % 40) * 9 + (y % 20) * 11) as u16;
        let f = v5_file(
            v,
            luma,
            cb,
            cr,
            vec![(0x7038, Val::Long(vec![500, 400]))],
            restart,
        );
        let mut plan = plan_of(&f);
        let Sensor::Ycc(p) = sensor_of(&f, &plan) else {
            panic!("not YCbCr");
        };
        assert_eq!(p.v, v);
        for y in 0..512 {
            for x in 0..512 {
                assert_eq!(p.y.data[y * 512 + x], luma(x, y), "Y ({x}, {y}) V {v}");
            }
        }
        for y in 0..512 / v {
            for x in 0..256 {
                assert_eq!(p.cb.data[y * 256 + x], cb(x, y), "Cb ({x}, {y}) V {v}");
                assert_eq!(p.cr.data[y * 256 + x], cr(x, y), "Cr ({x}, {y}) V {v}");
            }
        }
        plan.matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let (w, h, px) = linear_with(&f, &plan);
        assert_eq!((w, h), (500, 400));
        for (x, y) in [(0, 0), (1, 0), (3, 1), (499, 399), (250, 3), (77, 201)] {
            let rgb = render::ycc_rgb(
                f64::from(luma(x, y)),
                f64::from(cb(x / 2, y / v)),
                f64::from(cr(x / 2, y / v)),
                &plan.ycc,
            );
            assert_rgb(
                px[y * w + x],
                render::v5_levels(rgb, &plan.ycc),
                EPS,
                "V5 pixel",
            );
        }
    }

    // R-ARW-40: YCbCrSubSampling must agree with the frame; H must be 2.
    let f = v5_file(
        2,
        |_, _| 5000,
        |_, _| 16384,
        |_, _| 16384,
        vec![(0x0212, Val::Short(vec![2, 1]))],
        0,
    );
    let c = container(&f);
    let plan = meta::plan(&c, Variant::V5).unwrap();
    assert!(super::decode_sensor(&f, &plan, &CancelToken::new()).is_err());
    let f = v5_file(
        2,
        |_, _| 5000,
        |_, _| 16384,
        |_, _| 16384,
        vec![(0x0212, Val::Short(vec![1, 1]))],
        0,
    );
    let c = container(&f);
    let plan = meta::plan(&c, Variant::V5).unwrap();
    assert!(super::decode_sensor(&f, &plan, &CancelToken::new()).is_err());
}

#[test]
fn arw_v5_line_start_rule() {
    // TV-10 with the rule of R-ARW-39.
    let (y, cb, cr) = tv10_planes(LineStart::Sony);
    assert_eq!(
        y,
        vec![32770, 32771, 32769, 32772, 32774, 32773, 32775, 32776]
    );
    assert_eq!(cb, vec![32768, 32766]);
    assert_eq!(cr, vec![32767, 32769]);
    // The plain T.81 rule gives the values the specification predicts for
    // a decoder without the rule.
    let (y, _, _) = tv10_planes(LineStart::T81);
    assert_eq!(&y[4..], &[32773, 32772, 32774, 32775]);

    // F5 is decoded with the rule (its luma lines 2, 4, ... would drift by
    // a whole row with the plain rule only when the data are not flat).
    // A non-flat 512 x 512 tile: lines with y mod 2 = 0 start from two
    // lines above. Encoded with the rule, decoded exactly.
    let luma = |x: usize, y: usize| (1000 + x * 3 + (y * y) % 977) as u16;
    let f = v5_file(2, luma, |_, _| 16384, |_, _| 16384, vec![], 0);
    let plan = plan_of(&f);
    let Sensor::Ycc(p) = sensor_of(&f, &plan) else {
        panic!("not YCbCr");
    };
    assert!((0..512 * 512).all(|i| p.y.data[i] == luma(i % 512, i / 512)));
    // The same tile read with the plain T.81 rule differs from line 2 on.
    let s = lj92::parse(&f[plan_tile_range(&plan)]).unwrap();
    let mut yb = vec![0u16; 512 * 512];
    let mut cbb = vec![0u16; 256 * 256];
    let mut crb = vec![0u16; 256 * 256];
    let targets = [
        Target {
            buffer: 0,
            base: 0,
            xs: 1,
            ys: 512,
        },
        Target {
            buffer: 1,
            base: 0,
            xs: 1,
            ys: 256,
        },
        Target {
            buffer: 2,
            base: 0,
            xs: 1,
            ys: 256,
        },
    ];
    s.decode(&mut [&mut yb, &mut cbb, &mut crb], &targets, LineStart::T81)
        .unwrap();
    assert_eq!(&yb[..1024], &p.y.data[..1024], "lines 0 and 1 agree");
    assert_ne!(yb[1024], luma(0, 2), "line 2 differs with the T.81 rule");
}

fn plan_tile_range(plan: &Plan) -> std::ops::Range<usize> {
    match &plan.layout {
        meta::Layout::Tiles { tiles, .. } => tiles[0].0..tiles[0].0 + tiles[0].1,
        meta::Layout::Strips { .. } => panic!("not tiled"),
    }
}

/// R-ARW-72 to R-ARW-75: faulty tile streams fail, never panic.
#[test]
fn arw_lossless_stream_faults() {
    let good = hex(TV9);
    assert!(cfa_tile(&good, 2, 2).is_ok());
    let fails = |s: &[u8], what: &str| {
        assert!(cfa_tile(s, 2, 2).is_err(), "{what} must fail");
    };
    // Case 1: no SOI.
    fails(&good[2..], "missing SOI");
    // Case 2: no DHT before the scan.
    let mut no_dht = good[..24].to_vec();
    no_dht.extend_from_slice(&good[49..]);
    fails(&no_dht, "missing DHT");
    // Missing SOF3.
    let mut no_sof = good[..2].to_vec();
    no_sof.extend_from_slice(&good[24..]);
    fails(&no_sof, "missing SOF3");
    // Case 4: a scan that names table 1.
    let mut t1 = good.clone();
    t1[49 + 6] = 0x10;
    fails(&t1, "undefined table");
    // Case 5: predictor 0 and 8, Pt >= P.
    for (at, v, what) in [
        (49 + 13, 0u8, "predictor 0"),
        (49 + 13, 8, "predictor 8"),
        (49 + 15, 14, "Pt = P"),
    ] {
        let mut b = good.clone();
        b[at] = v;
        fails(&b, what);
    }
    // R-ARW-73: counts summing beyond the segment, and code overflow.
    let mut over = good.clone();
    over[24 + 6] = 200;
    fails(&over, "counts beyond the segment");
    // Three codes of length 1 and one of length 3: four symbols fit the
    // segment, but length 1 has room for two codes only.
    let mut overflow = good.clone();
    overflow[29] = 3;
    overflow[30] = 0;
    fails(&overflow, "code space overflow");
    // R-ARW-74: a symbol above 16 (HUFFVAL 3 replaced by 17), and a code
    // that is not in the table (all-ones prefix 111).
    // Layout of TV-9: SOI 0..2, SOF3 2..24, DHT 24..49 (counts 29..45,
    // symbols 45..49), SOS 49..65, entropy data 65..71, EOI 71..73.
    let mut big = good.clone();
    big[48] = 17;
    fails(&big, "SSSS 17");
    let mut ones = good.clone();
    ones[65] = 0xFF;
    ones.insert(66, 0x00);
    fails(&ones, "code not in the table");
    // R-ARW-74: a marker other than RST inside the entropy data.
    let mut marker = good.clone();
    marker.splice(67..67, [0xFF, 0xD9]);
    fails(&marker, "EOI inside the data");
    // R-ARW-76: a restart interval that is not whole MCU rows fails.
    let comp = vec![1000u16; 256 * 256];
    let comps: Vec<Comp<'_>> = (0..4)
        .map(|_| Comp {
            h: 1,
            v: 1,
            samples: &comp,
            width: 256,
        })
        .collect();
    let mut s = lj92(14, 256, 256, &comps, 256, true);
    let dri = s.windows(2).position(|w| w == [0xFF, 0xDD]).unwrap();
    s[dri + 4..dri + 6].copy_from_slice(&100u16.to_be_bytes());
    assert!(cfa_tile(&s, 256, 256).is_err());
    // A wrong RST number fails.
    let mut s = lj92(14, 256, 256, &comps, 256, true);
    let rst = s.windows(2).position(|w| w == [0xFF, 0xD0]).unwrap();
    s[rst + 1] = 0xD3;
    assert!(cfa_tile(&s, 256, 256).is_err());
    // Every truncation and every single-byte change: no panic.
    for cut in 0..good.len() {
        let _ = cfa_tile(&good[..cut], 2, 2);
    }
    for i in 0..good.len() {
        for v in [0x00, 0xFF, 0x7F, 0x80] {
            let mut b = good.clone();
            b[i] = v;
            let _ = cfa_tile(&b, 2, 2);
        }
    }
}

// ---------------------------------------------------------------------------
// Levels, white balance, color
// ---------------------------------------------------------------------------

#[test]
fn arw_levels_wb_matrix_vector() {
    // TV-12 through F1 is in arw_v1_uncompressed; here the steps.
    let p = plan_of(&f1());
    // R-ARW-43, R-ARW-44.
    assert_eq!(p.white, 16383);
    assert_eq!(p.black, [512; 4]);
    // R-ARW-46 source 1: G = 1024, mR = 2429 / 1024, mB = 1783 / 1024.
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert!((p.wb[0] - 2.372_070_312_5).abs() < 1e-12);
    assert!((p.wb[2] - 1.741_210_937_5).abs() < 1e-12);
    // R-ARW-47 worked example.
    let n = |s: f64, m: f64| ((s - 512.0) / (16383.0 - 512.0) * m).clamp(0.0, 1.0);
    assert!((n(2512.0, p.wb[0]) - 0.298_918_8).abs() < 5e-8);
    assert!((n(4512.0, 1.0) - 0.252_032_0).abs() < 5e-8);
    assert!((n(1512.0, p.wb[2]) - 0.109_710_2).abs() < 5e-8);
    // R-ARW-50, R-ARW-51: ILCE-7M4 row 3 normalized.
    assert!(p.matrix_from_table);
    let r3 = p.matrix[2];
    for (g, w) in r3.iter().zip([0.039_604_0, -0.465_346_5, 1.425_742_6]) {
        assert!((g - w).abs() < 5e-8, "row 3 {r3:?}");
    }
    // R-ARW-52 worked example.
    let v = [0.298_918_8, 0.252_032_0, 0.109_710_2];
    let out: Vec<f64> = p
        .matrix
        .iter()
        .map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        .collect();
    for (g, w) in out.iter().zip(TV12) {
        assert!((g - f64::from(w)).abs() < 1e-6, "TV-12 {out:?}");
    }
    // Neutral stays neutral for every normalized matrix.
    for model in ["ILCE-7M4", "DSC-RX100", "NEX-5T", "unknown"] {
        let (m, _) = color::matrix(model.as_bytes());
        for r in m {
            assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12, "{model}");
        }
    }

    // R-ARW-46 source 2 (table) and 3 (generic); R-ARW-45 defaults.
    let vals = [2512u16, 4512, 2512, 4512, 4512, 1512, 4512, 1512];
    let mk = |model: &'static str, extra: Vec<(u16, Val)>, drop: &[u16]| {
        let data: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
        let mut raw = strip_raw(4, 2, 14, 1, 2);
        for &t in drop {
            raw = without(raw, t);
        }
        for (t, v) in extra {
            raw = with(raw, t, v);
        }
        plan_of(&file(model, 1, raw, vec![data]))
    };
    let p = mk("ILCE-7M4", vec![], &[0x7313, 0x7310]);
    assert_eq!(p.wb_source, WbSource::Table);
    assert_eq!(p.wb, [2.47, 1.0, 1.61]);
    assert_eq!(p.black, [512; 4]);
    let p = mk("DSLR-A900", vec![], &[0x7313, 0x7310]);
    assert_eq!(p.wb_source, WbSource::Generic);
    assert_eq!(p.wb, [2.512, 1.0, 1.678]);
    assert!(!p.matrix_from_table, "R-ARW-50: generic matrix");
    for rx in ["DSC-RX100", "DSC-RX100M3", "DSC-RX0", "DSC-RX10M4"] {
        assert_eq!(mk(rx, vec![], &[0x7310]).black, [800; 4], "{rx}");
    }
    assert_eq!(mk("DSC-RX1", vec![], &[0x7310]).black, [512; 4]);
    // A WB level of 0 or below skips the tag.
    let p = mk(
        "ILCE-7M4",
        vec![(0x7313, Val::SShort(vec![2429, 0, 1024, 1783]))],
        &[],
    );
    assert_eq!(p.wb_source, WbSource::Table);
    // R-ARW-44: a black level not below w - 255 makes the tag invalid.
    let p = mk(
        "ILCE-7M4",
        vec![(0x7310, Val::Short(vec![512, 512, 16200, 512]))],
        &[],
    );
    assert_eq!(p.black, [512; 4]);
    let p = mk(
        "ILCE-7M4",
        vec![(0x7310, Val::Short(vec![500, 510, 520, 530]))],
        &[],
    );
    assert_eq!(p.black, [500, 510, 520, 530]);
    // R-ARW-43: white level outside 4096 to 65535 uses 16383.
    assert_eq!(
        mk("ILCE-7M4", vec![(0xC61D, Val::Short(vec![4095]))], &[]).white,
        16383
    );
    assert_eq!(
        mk("ILCE-7M4", vec![(0xC61D, Val::Long(vec![70000]))], &[]).white,
        16383
    );
    assert_eq!(
        mk("ILCE-7M4", vec![(0xC61D, Val::Long(vec![16380]))], &[]).white,
        16380
    );
    // R-ARW-77: values clip to 0 and 1 (highlights and below black).
    let data: Vec<u8> = [16383u16, 16383, 16383, 16383, 0, 0, 0, 0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let f = file("ILCE-7M4", 1, strip_raw(4, 2, 14, 1, 2), vec![data]);
    let (_, _, px) = linear(&f);
    for p in px {
        for c in p {
            assert!((0.0..=1.0).contains(&c));
        }
    }
}

// ---------------------------------------------------------------------------
// Orientation, crop, demosaicing
// ---------------------------------------------------------------------------

/// An independent per-pixel implementation of R-ARW-47, R-ARW-54,
/// R-ARW-55 and R-ARW-52 in f64, on the stored raster.
fn reference<'a>(
    values: &'a [u16],
    w: usize,
    plan: &'a Plan,
) -> impl Fn(usize, usize) -> [f64; 3] + 'a {
    let (vw, vh) = plan.valid;
    let col = move |x: usize, y: usize| usize::from(plan.cfa[2 * (y % 2) + x % 2]);
    let norm = move |x: usize, y: usize| {
        let p = 2 * (y % 2) + x % 2;
        let b = f64::from(plan.black[p]);
        ((f64::from(values[y * w + x]) - b) / (f64::from(plan.white) - b) * plan.wb[col(x, y)])
            .clamp(0.0, 1.0)
    };
    move |x, y| {
        let own = col(x, y);
        let mut sum = [0f64; 3];
        let mut cnt = [0u32; 3];
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                if (dx == 0 && dy == 0) || xx < 0 || yy < 0 || xx >= vw as i64 || yy >= vh as i64 {
                    continue;
                }
                let c = col(xx as usize, yy as usize);
                if c != own {
                    sum[c] += norm(xx as usize, yy as usize);
                    cnt[c] += 1;
                }
            }
        }
        let mut v = [0f64; 3];
        for c in 0..3 {
            v[c] = if c == own {
                norm(x, y)
            } else if cnt[c] > 0 {
                sum[c] / f64::from(cnt[c])
            } else {
                0.0
            };
        }
        let m = &plan.matrix;
        [0, 1, 2].map(|r| (m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]).clamp(0.0, 1.0))
    }
}

/// Stored pixel of output pixel (ox, oy) by the R-ARW-58 table.
fn stored_of(o: u8, ox: usize, oy: usize, w: usize, h: usize) -> (usize, usize) {
    for y in 0..h {
        for x in 0..w {
            let out = match o {
                1 => (x, y),
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                _ => (y, w - 1 - x),
            };
            if out == (ox, oy) {
                return (x, y);
            }
        }
    }
    unreachable!()
}

#[test]
fn arw_orientation_and_crop() {
    // TV-13: a b c / d e f with orientations 6, 8 and 3 (R-ARW-58).
    let tv13 = |o: u8| -> Vec<char> {
        let (w, h) = if o >= 5 { (2, 3) } else { (3, 2) };
        let names = ['a', 'b', 'c', 'd', 'e', 'f'];
        let mut out = Vec::new();
        for oy in 0..h {
            for ox in 0..w {
                let (x, y) = stored_of(o, ox, oy, 3, 2);
                out.push(names[y * 3 + x]);
            }
        }
        out
    };
    assert_eq!(tv13(6), ['d', 'a', 'e', 'b', 'f', 'c']);
    assert_eq!(tv13(8), ['c', 'f', 'b', 'e', 'a', 'd']);
    assert_eq!(tv13(3), ['f', 'e', 'd', 'c', 'b', 'a']);

    // A 21 x 19 V1 raster, crop at the odd origin (3, 1), 16 x 17, every
    // orientation and several CFA patterns (R-ARW-21, R-ARW-23, R-ARW-56,
    // R-ARW-57). The renderer must equal the reference: demosaic the whole
    // valid area in stored CFA phase, then crop, then orient.
    let (w, h) = (21usize, 19usize);
    let mut next = noise(7);
    let values: Vec<u16> = (0..w * h).map(|_| 400 + (next() % 15000) as u16).collect();
    for pattern in [[0u8, 1, 1, 2], [2, 1, 1, 0], [1, 0, 2, 1], [1, 1, 0, 2]] {
        for o in 1..=8u8 {
            let extra = vec![
                (0xC61F, Val::Short(vec![3, 1])),
                (0xC620, Val::Short(vec![16, 17])),
                (0x828E, Val::Byte(pattern.to_vec())),
            ];
            let f = v1_file(w as u32, h as u32, &values, extra, u16::from(o));
            let plan = plan_of(&f);
            assert_eq!(
                plan.crop,
                meta::Rect {
                    x: 3,
                    y: 1,
                    w: 16,
                    h: 17
                }
            );
            assert_eq!(plan.orientation, o);
            let (ow, oh, px) = linear(&f);
            assert_eq!((ow, oh), if o >= 5 { (17, 16) } else { (16, 17) });
            let r = reference(&values, w, &plan);
            for oy in 0..oh {
                for ox in 0..ow {
                    let (x, y) = stored_of(o, ox, oy, 16, 17);
                    let want = r(3 + x, 1 + y).map(|v| v as f32);
                    assert_rgb(
                        px[oy * ow + ox],
                        want,
                        2e-6,
                        &format!("{pattern:?} o{o} ({ox}, {oy})"),
                    );
                }
            }
            // The delivered image has the same size and orientation.
            let dev = report(&f);
            assert_eq!(
                (dev.image.width() as usize, dev.image.height() as usize),
                (ow, oh)
            );
        }
    }

    // The worked example of R-ARW-54: pixel (1, 0) is green; red is the
    // mean of (0, 0) and (2, 0); blue is (1, 1) alone. Whole valid area,
    // no crop: border pixels use the general rule (R-ARW-55).
    let f = v1_file(w as u32, h as u32, &values, vec![], 1);
    let plan = plan_of(&f);
    let (ow, _, px) = linear(&f);
    let r = reference(&values, w, &plan);
    for (x, y) in [
        (0, 0),
        (1, 0),
        (20, 0),
        (0, 18),
        (20, 18),
        (10, 0),
        (0, 9),
        (20, 9),
        (10, 18),
    ] {
        assert_rgb(
            px[y * ow + x],
            r(x, y).map(|v| v as f32),
            2e-6,
            &format!("border ({x}, {y})"),
        );
    }
    let mut id = plan.clone();
    id.matrix = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let (_, _, raw_rgb) = linear_with(&f, &id);
    let n = |x: usize, y: usize, m: f64| {
        ((f64::from(values[y * w + x]) - 512.0) / 15871.0 * m).clamp(0.0, 1.0)
    };
    let red = (n(0, 0, plan.wb[0]) + n(2, 0, plan.wb[0])) / 2.0;
    let blue = n(1, 1, plan.wb[2]);
    let p = raw_rgb[1];
    assert!((f64::from(p[0]) - red.min(1.0)).abs() < 2e-6);
    assert!((f64::from(p[1]) - n(1, 0, 1.0)).abs() < 2e-6);
    assert!((f64::from(p[2]) - blue).abs() < 2e-6);

    // R-ARW-21: crop rejected (whole valid area) when too small, outside
    // the valid area, or with one tag missing; RATIONAL accepted.
    let crop_of =
        |extra: Vec<(u16, Val)>| plan_of(&v1_file(w as u32, h as u32, &values, extra, 1)).crop;
    let whole = meta::Rect { x: 0, y: 0, w, h };
    assert_eq!(
        crop_of(vec![
            (0xC61F, Val::Short(vec![0, 0])),
            (0xC620, Val::Short(vec![15, 16]))
        ]),
        whole
    );
    assert_eq!(
        crop_of(vec![
            (0xC61F, Val::Short(vec![6, 0])),
            (0xC620, Val::Short(vec![16, 16]))
        ]),
        whole
    );
    assert_eq!(crop_of(vec![(0xC620, Val::Short(vec![16, 16]))]), whole);
    assert_eq!(
        crop_of(vec![
            (0xC61F, Val::Rational(vec![(4, 2), (1, 1)])),
            (0xC620, Val::Rational(vec![(16, 1), (16, 1)])),
        ]),
        meta::Rect {
            x: 2,
            y: 1,
            w: 16,
            h: 16
        }
    );
    // R-ARW-17: a zero denominator uses the tag default (origin 0, 0).
    assert_eq!(
        crop_of(vec![
            (0xC61F, Val::Rational(vec![(4, 0), (1, 1)])),
            (0xC620, Val::Long(vec![16, 16])),
        ]),
        meta::Rect {
            x: 0,
            y: 0,
            w: 16,
            h: 16
        }
    );
    // R-ARW-57: orientation outside 1 to 8 counts as 1.
    let f = v1_file(w as u32, h as u32, &values, vec![], 9);
    assert_eq!(plan_of(&f).orientation, 1);
    // R-ARW-22: SonyCropTopLeft and SonyCropSize are ignored.
    let extra = vec![
        (0x74C7, Val::Long(vec![2, 2])),
        (0x74C8, Val::Long(vec![16, 16])),
    ];
    assert_eq!(crop_of(extra), whole);
    // R-ARW-20: SonyRawImageSize larger than the raster is ignored.
    let f = v1_file(
        w as u32,
        h as u32,
        &values,
        vec![(0x7038, Val::Long(vec![22, 19]))],
        1,
    );
    assert_eq!(plan_of(&f).valid, (w, h));
    let f = v1_file(
        w as u32,
        h as u32,
        &values,
        vec![(0x7038, Val::Long(vec![18, 17]))],
        1,
    );
    assert_eq!(plan_of(&f).valid, (18, 17));

    // R-ARW-55 at the valid-area edge of a tiled file: the padding (16000)
    // must not leak into the edge pixels.
    let valid = (1000usize, 1010usize);
    let pat = v4_pattern(valid);
    let f = v4_file(2, 2, (1000, 1010), &pat, 0);
    let plan = plan_of(&f);
    let (_, _, v) = cfa_values(&f);
    let (ow, oh, px) = linear(&f);
    assert_eq!((ow, oh), valid);
    let r = reference(&v, 1024, &plan);
    for (x, y) in [
        (999, 500),
        (998, 1009),
        (999, 1009),
        (0, 1009),
        (500, 1009),
        (999, 0),
        (1, 1),
    ] {
        assert_rgb(
            px[y * ow + x],
            r(x, y).map(|v| v as f32),
            2e-6,
            &format!("edge ({x}, {y})"),
        );
    }
}

// ---------------------------------------------------------------------------
// Preview path, hostile input, cancellation, thumbnails
// ---------------------------------------------------------------------------

/// A preview-bearing ARW: previews in IFD0 (blob 0) and IFD1 (blob 1),
/// sensor data last.
fn with_previews(
    raw: Vec<(u16, Val)>,
    sensor: Vec<u8>,
    previews: [Vec<u8>; 2],
    orientation: u16,
    model: &'static str,
) -> Vec<u8> {
    let mut ifd0 = common_ifd0(model, orientation);
    ifd0.push((0x0201, Val::Blobs(vec![0])));
    ifd0.push((0x0202, Val::BlobLens(vec![0])));
    let ifd1 = vec![
        (0x0201, Val::Blobs(vec![1])),
        (0x0202, Val::BlobLens(vec![1])),
    ];
    // The sensor data are blob 2: rewrite the raw blob references.
    let raw = raw
        .into_iter()
        .map(|(t, v)| match v {
            Val::Blobs(i) => (t, Val::Blobs(i.iter().map(|k| k + 2).collect())),
            Val::BlobLens(i) => (t, Val::BlobLens(i.iter().map(|k| k + 2).collect())),
            other => (t, other),
        })
        .collect();
    let [a, b] = previews;
    Builder {
        ifd0,
        raw,
        chain: vec![ifd1],
        blobs: vec![a, b, sensor],
        sub_ifds: true,
    }
    .build()
}

fn f1_raw_and_data() -> (Vec<(u16, Val)>, Vec<u8>) {
    (
        strip_raw(4, 2, 14, 1, 2),
        hex("D0 09 A0 11 D0 09 A0 11 A0 11 E8 05 A0 11 E8 05"),
    )
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

#[test]
fn arw_out_of_scope_falls_back_to_preview() {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let (raw, data) = f1_raw_and_data();
    // A good file develops; its classification is V1.
    let good = with_previews(
        raw.clone(),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    assert_eq!(report(&good).origin, Origin::Sensor(Variant::V1));
    // R-ARW-60: the largest candidate (IFD1 here) is chosen.
    let check = |f: &[u8], class: Option<Class>, what: &str| {
        let dev = report(f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        if class.is_some() {
            assert_eq!(dev.class, class, "{what}");
        }
    };
    // X1: IFD0 without SubIFDs.
    let mut b = Builder {
        ifd0: common_ifd0("DSC-F828", 1),
        chain: vec![vec![
            (0x0201, Val::Blobs(vec![0])),
            (0x0202, Val::BlobLens(vec![0])),
        ]],
        blobs: vec![big.clone()],
        ..Builder::default()
    };
    check(&b.build(), Some(Class::OutOfScope(Relative::X1)), "X1");
    // X2: Model DSC-R1 (the sensor data would be V1-like).
    let x2 = with_previews(
        raw.clone(),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "DSC-R1",
    );
    check(&x2, Some(Class::OutOfScope(Relative::X2)), "X2");
    // X3: the SubIFD offset does not point to a valid IFD.
    b.ifd0 = common_ifd0("DSLR-A100", 1);
    b.ifd0.push((0x014A, Val::Long(vec![0xFFFF_FF00])));
    check(&b.build(), Some(Class::OutOfScope(Relative::X3)), "X3");
    // X3 also when the only SubIFD has NewSubfileType 1 (R-ARW-13).
    let x3 = with_previews(
        with(raw.clone(), 0x00FE, Val::Long(vec![1])),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    check(&x3, Some(Class::OutOfScope(Relative::X3)), "X3 subfile");
    // X4: Compression 32767 with a byte count of neither V2 nor V3.
    let x4raw = with(
        with(raw.clone(), 0x0103, Val::Short(vec![32767])),
        0x0117,
        Val::Long(vec![10]),
    );
    check(
        &with_previews(
            x4raw,
            data.clone(),
            [small.clone(), big.clone()],
            1,
            "DSLR-A200",
        ),
        Some(Class::OutOfScope(Relative::X4)),
        "X4",
    );
    // X5: an unknown compression.
    let x5raw = with(raw.clone(), 0x0103, Val::Short(vec![34713]));
    check(
        &with_previews(
            x5raw,
            data.clone(),
            [small.clone(), big.clone()],
            1,
            "ILCE-7M4",
        ),
        Some(Class::OutOfScope(Relative::X5)),
        "X5",
    );
    // X6: big-endian: the ARW path declines (generic preview search).
    let mut be = good.clone();
    be[..4].copy_from_slice(&[0x4D, 0x4D, 0x00, 0x2A]);
    assert!(decode_report(&be, &DecodeOptions::default())
        .unwrap()
        .is_none());

    // Failed checks of a specified variant use the preview (R-ARW-63):
    // R-ARW-16 a required tag missing.
    let f = with_previews(
        without(raw.clone(), 0x0111),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    check(&f, Some(Class::Sensor(Variant::V1)), "StripOffsets missing");
    // R-ARW-18 a CFA pattern that is not Bayer.
    let f = with_previews(
        with(raw.clone(), 0x828E, Val::Byte(vec![0, 0, 1, 2])),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    check(&f, None, "CFA pattern");
    let f = with_previews(
        with(raw.clone(), 0x828D, Val::Short(vec![2, 4])),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    check(&f, None, "CFA dim");
    // R-ARW-66: W * H above 2^27 (with a matching byte count claim).
    let f = with_previews(
        with(
            with(raw.clone(), 0x0100, Val::Long(vec![65535])),
            0x0101,
            Val::Long(vec![4096]),
        ),
        data.clone(),
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Preview, "R-ARW-66");
    // R-ARW-25: a tile size other than 512.
    let mut t = meta_raw_of_f4();
    t = with(t, 0x0142, Val::Short(vec![256]));
    let f = with_previews(t, f4_tile(), [small.clone(), big.clone()], 1, "ILCE-7M4");
    check(&f, Some(Class::Sensor(Variant::V4)), "tile size");
    // A corrupt tile stream (R-ARW-72) falls back as well.
    let mut tile = f4_tile();
    tile[0] = 0;
    let f = with_previews(
        meta_raw_of_f4(),
        tile,
        [small.clone(), big.clone()],
        1,
        "ILCE-7M4",
    );
    check(&f, Some(Class::Sensor(Variant::V4)), "bad tile");

    // R-ARW-63: the next-largest candidate when the largest fails.
    let mut broken = big.clone();
    let sos = broken.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    broken.truncate(sos + 4);
    let x5raw = with(raw.clone(), 0x0103, Val::Short(vec![34713]));
    let f = with_previews(
        x5raw.clone(),
        data.clone(),
        [small.clone(), broken],
        1,
        "ILCE-7M4",
    );
    assert_preview(&report(&f), (16, 8), [40, 40, 200], "next-largest");
    // An error only when no candidate decodes.
    let f = with_previews(
        x5raw.clone(),
        data.clone(),
        [vec![0xFF, 0xD8, 0, 0], vec![1, 2, 3]],
        1,
        "ILCE-7M4",
    );
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    // R-ARW-59: a candidate outside the file or without FF D8 is dropped.
    let mut far = Builder {
        ifd0: common_ifd0("DSC-F828", 1),
        chain: vec![vec![
            (0x0201, Val::Long(vec![1_000_000])),
            (0x0202, Val::Long(vec![10])),
        ]],
        ..Builder::default()
    }
    .build();
    far.extend_from_slice(&[0; 32]);
    assert!(preview::collect(&container(&far)).is_empty());
    // R-ARW-61: bytes after EOI, and a stream without EOI.
    let mut padded = big.clone();
    padded.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let f = with_previews(
        x5raw.clone(),
        data.clone(),
        [small.clone(), padded],
        1,
        "ILCE-7M4",
    );
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "bytes after EOI");
    let no_eoi = big[..big.len() - 2].to_vec();
    let f = with_previews(
        x5raw.clone(),
        data.clone(),
        [small.clone(), no_eoi],
        1,
        "ILCE-7M4",
    );
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "no EOI");
    // R-ARW-62: orientation 6 rotates a wide preview, not a tall one.
    let f = with_previews(
        x5raw.clone(),
        data.clone(),
        [small.clone(), big.clone()],
        6,
        "ILCE-7M4",
    );
    assert_preview(&report(&f), (32, 48), [200, 40, 40], "rotated preview");
    let tall = jpeg(32, 48, [200, 40, 40]);
    let f = with_previews(x5raw, data, [small, tall], 6, "ILCE-7M4");
    assert_preview(
        &report(&f),
        (32, 48),
        [200, 40, 40],
        "tall preview not rotated",
    );

    // Through the registry: ARW by extension and by Make "SONY" without a
    // name; the provider never fails where a preview exists.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(x2.clone(), Some("x.ARW".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    // Without a name: Make "SONY" with SubIFDs and SR2Private is ARW (V5
    // has no CFA photometric value for the generic RAW test to find).
    let mut ifd0 = common_ifd0("ILCE-7M4", 1);
    ifd0.push((0xC634, Val::Byte(vec![0, 0, 0, 0])));
    let named = Builder {
        ifd0,
        raw: f5_raw(),
        blobs: vec![f5_tile()],
        sub_ifds: true,
        ..Builder::default()
    }
    .build();
    assert!(is_arw(&named, None));
    let src = pandite_core::Source::from_bytes(named, None);
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(
        img.first_image().format(),
        PixelFormat::Rgba16,
        "Sony raw TIFF without a name"
    );
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (512, 512, "raw"));
    // A TIFF with Make "SONY" but no SR2Private is not taken as ARW.
    assert!(!is_arw(&good, None));
    assert!(is_arw(&good, Some("arw")));
    let src = pandite_core::Source::from_bytes(good.clone(), Some("y.arw".into()));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (4, 2, "raw"));
    let meta = reg
        .probe(&pandite_core::Source::from_bytes(x2, Some("x.arw".into())))
        .unwrap();
    assert_eq!((meta.width, meta.height), (48, 32));
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let (raw, data) = f1_raw_and_data();
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F5", f5()),
        (
            "F1+previews",
            with_previews(raw, data, [small.clone(), big.clone()], 6, "ILCE-7M4"),
        ),
        (
            "F4+previews",
            with_previews(meta_raw_of_f4(), f4_tile(), [small, big], 1, "ILCE-7M4"),
        ),
    ]
}

#[test]
fn arw_hostile_input_never_panics() {
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
                // Damage the directories and tile headers.
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
                // Huge values in IFD fields.
                _ => {
                    let i = (next() as usize % bad.len().min(400)) & !3;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27][next() as usize % 5];
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&bad);
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
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.arw".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized claims are refused before allocation (R-ARW-66, R-ARW-67).
    let (raw, data) = f1_raw_and_data();
    for (w, h) in [(65535u32, 65535u32), (0, 2), (4, 0), (70000, 2)] {
        let r = with(
            with(raw.clone(), 0x0100, Val::Long(vec![w])),
            0x0101,
            Val::Long(vec![h]),
        );
        let f = file("ILCE-7M4", 1, r, vec![data.clone()]);
        let c = container(&f);
        assert!(meta::plan(&c, Variant::V1).is_err(), "{w} x {h}");
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    }
}

#[test]
fn arw_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 7, 8, 9, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => assert_eq!(dev.origin, Origin::Preview, "{name} cut at {cut}"),
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large V4 file: 8 x 8 tiles of 512 x 512, all tile offsets pointing at
/// one flat tile (the specification does not require distinct tiles).
fn large_v4() -> Vec<u8> {
    let mut raw = meta_raw_of_f4();
    raw = with(raw, 0x0100, Val::Long(vec![4096]));
    raw = with(raw, 0x0101, Val::Long(vec![4096]));
    raw = with(raw, 0x7038, Val::Long(vec![4096, 4096]));
    raw = with(raw, 0x0144, Val::Blobs(vec![0; 64]));
    raw = with(raw, 0x0145, Val::BlobLens(vec![0; 64]));
    file("ILCE-7M4", 1, raw, vec![f4_tile()])
}

#[test]
fn arw_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f4(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f4(), Some("a.arw".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread.
    let f = large_v4();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V4));
    assert_eq!((dev.image.width(), dev.image.height()), (4096, 4096));
    drop(dev);
    let token = CancelToken::new();
    let options = DecodeOptions {
        cancel: token.clone(),
        ..DecodeOptions::default()
    };
    let start = Instant::now();
    let result = std::thread::scope(|s| {
        let h = s.spawn(|| decode_report(&f, &options));
        // Early and independent of the measured time: a loaded machine
        // makes a fraction of it unreliable.
        std::thread::sleep((full / 5).min(Duration::from_millis(2)));
        token.cancel();
        h.join().unwrap()
    });
    let took = start.elapsed();
    assert!(matches!(result, Err(Error::Cancelled)), "not cancelled");
    // No time bound: a loaded machine makes one unreliable. The Cancelled
    // result above proves that the decode stopped before its end.
    println!("cancel took {took:?} of a {full:?} decode");
}

#[test]
fn arw_target_size_uses_preview() {
    // A 128 x 64 V1 sensor with previews of 32 x 16 and 64 x 32.
    let (w, h) = (128u32, 64u32);
    let vals: Vec<u16> = (0..w * h).map(|i| 1000 + (i % 3000) as u16).collect();
    let data: Vec<u8> = vals.iter().flat_map(|v| v.to_le_bytes()).collect();
    let raw = strip_raw(w, h, 14, 1, h);
    let f = with_previews(
        raw,
        data,
        [jpeg(32, 16, [10, 200, 10]), jpeg(64, 32, [200, 10, 10])],
        1,
        "ILCE-7M4",
    );
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    // The smallest preview that covers the target.
    let dev = decode_report(&f, &opts((20, 10))).unwrap().unwrap();
    assert_preview(&dev, (32, 16), [10, 200, 10], "small target");
    let dev = decode_report(&f, &opts((40, 40))).unwrap().unwrap();
    assert_preview(&dev, (64, 32), [200, 10, 10], "medium target");
    // A target larger than every preview develops the sensor data.
    let dev = decode_report(&f, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    assert_eq!((dev.image.width(), dev.image.height()), (128, 64));
    // Through the registry: a thumbnail request returns 8-bit preview data.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f.clone(), Some("t.arw".into()));
    let img = reg.decode(&src, &opts((16, 16))).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (128, 64));
}

// ---------------------------------------------------------------------------
// Real camera files (gate G4)
// ---------------------------------------------------------------------------

/// Mean linear RGB of each cell of a `n` x `n` grid, sampling every
/// `step`-th pixel.
pub(crate) fn grid_means(img: &ImageBuf, n: usize, step: usize) -> Vec<[f64; 3]> {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let lin8: Vec<f64> = (0..256)
        .map(|v| f64::from(crate::util::srgb_to_linear(v as f32 / 255.0)))
        .collect();
    let mut sum = vec![[0f64; 3]; n * n];
    let mut cnt = vec![0u64; n * n];
    let d = img.data();
    for y in (0..h).step_by(step) {
        for x in (0..w).step_by(step) {
            let cell = (y * n / h) * n + x * n / w;
            let rgb: [f64; 3] = match img.format() {
                PixelFormat::Rgba16 => {
                    let i = (y * w + x) * 8;
                    [0, 1, 2].map(|k| {
                        let v = u16::from_ne_bytes([d[i + 2 * k], d[i + 2 * k + 1]]);
                        f64::from(crate::util::srgb_to_linear(f32::from(v) / 65535.0))
                    })
                }
                _ => {
                    let i = (y * w + x) * 4;
                    [0, 1, 2].map(|k| lin8[usize::from(d[i + k])])
                }
            };
            for k in 0..3 {
                sum[cell][k] += rgb[k];
            }
            cnt[cell] += 1;
        }
    }
    sum.iter()
        .zip(&cnt)
        .map(|(s, &c)| s.map(|v| v / c.max(1) as f64))
        .collect()
}

/// Agreement of two grids: (mean, max) rg-chromaticity distance over the
/// usable cells and the Spearman rank correlation of cell luminance, after
/// matching overall exposure. With `balance`, one gain per channel over
/// the usable cells first removes a global color cast.
pub(crate) fn agreement(
    ours: &[[f64; 3]],
    theirs: &[[f64; 3]],
    balance: bool,
) -> (f64, f64, f64, usize) {
    let lum = |c: &[f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (lo, lt): (Vec<f64>, Vec<f64>) = ours
        .iter()
        .zip(theirs)
        .map(|(a, b)| (lum(a), lum(b)))
        .unzip();
    let k = lt.iter().sum::<f64>() / lo.iter().sum::<f64>().max(1e-12);
    let max_t = lt.iter().copied().fold(0.0, f64::max);
    // Usable: neither dark (noise) nor clipped in either image.
    let usable = |a: &[f64; 3], b: &[f64; 3]| {
        lum(a) > 0.03 * max_t
            && lum(b) > 0.03 * max_t
            && a.iter().chain(b.iter()).all(|&v| v < 0.95)
    };
    let mut gain = [k; 3];
    if balance {
        let mut sa = [0f64; 3];
        let mut sb = [0f64; 3];
        for (a, b) in ours.iter().zip(theirs) {
            let a = a.map(|v| v * k);
            if usable(&a, b) {
                for c in 0..3 {
                    sa[c] += a[c];
                    sb[c] += b[c];
                }
            }
        }
        for c in 0..3 {
            gain[c] = k * sb[c] / sa[c].max(1e-12);
        }
    }
    let mut dists = Vec::new();
    for (a, b) in ours.iter().zip(theirs) {
        let a = [a[0] * gain[0], a[1] * gain[1], a[2] * gain[2]];
        if usable(&a, b) {
            let rg = |c: [f64; 3]| {
                let s = c[0] + c[1] + c[2];
                [c[0] / s, c[1] / s]
            };
            let (p, q) = (rg(a), rg(*b));
            dists.push(((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt());
        }
    }
    let rank = |v: &[f64]| {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
        let mut r = vec![0f64; v.len()];
        for (i, &k) in idx.iter().enumerate() {
            r[k] = i as f64;
        }
        r
    };
    let (ro, rt) = (rank(&lo), rank(&lt));
    let n = ro.len() as f64;
    let d2: f64 = ro.iter().zip(&rt).map(|(a, b)| (a - b).powi(2)).sum();
    let spearman = 1.0 - 6.0 * d2 / (n * (n * n - 1.0));
    let mean = dists.iter().sum::<f64>() / dists.len().max(1) as f64;
    let max = dists.iter().copied().fold(0.0, f64::max);
    (mean, max, spearman, dists.len())
}

/// Tolerances of the comparison with the camera's own JPEG, explained in
/// the test documentation.
pub(crate) const CHROMA_MEAN: f64 = 0.05;
pub(crate) const CHROMA_MAX: f64 = 0.15;
pub(crate) const RANK_MIN: f64 = 0.90;

/// Gate G4: every `.ARW` file in the folder named by `PANDITE_ARW_SAMPLES`
/// (relative to the workspace root) decodes through the sensor-data path
/// when its variant is in scope, at the full crop size, and agrees with the
/// file's own largest embedded JPEG on an 8 x 8 grid of cell means. Since
/// leaf 1.6 the image compared is the default display rendering (tone
/// curve on luminance with every channel scaled alike), so the test also
/// shows that the rendering keeps the developed colors.
///
/// The camera JPEG has its own tone curve and picture style, so values are
/// not compared directly. Overall exposure is matched by one gain on
/// luminance; then each usable cell (not dark, not clipped) is compared in
/// rg chromaticity, and the cell luminances must keep their rank order.
/// Tolerances: mean chromaticity distance 0.05 (the specification measured
/// 0.026 to 0.030 against the full-size camera JPEG with the as-shot white
/// balance, M10 and M12, and the D50 matrix is used for every illuminant,
/// OQ-19), largest cell 0.15 (saturated colors where the camera's picture
/// style departs most from a colorimetric rendering), and a Spearman rank
/// correlation of at least 0.90 (a monotonic tone curve keeps the order;
/// local contrast processing in the camera moves a few cells). Files
/// without as-shot white balance (WB from the model table or the generic
/// scales, OQ-2) are compared after one global gain per channel, because
/// the specification forbids the as-shot balance the camera JPEG used.
#[test]
fn arw_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_ARW_SAMPLES") else {
        println!("skip: PANDITE_ARW_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("arw"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .ARW files in {}", folder.display());
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let class = meta::classify(&c).expect("classify");
        let raw_size = c
            .raw
            .as_ref()
            .and_then(|r| Some((r.uint(0x0100).ok()??, r.uint(0x0101).ok()??)));
        let start = Instant::now();
        // The default display rendering, as the viewer shows it: the tone
        // curve must keep the colors of the colorimetric development.
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("ARW path");
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
        // Full sensor size: the raster of the raw IFD, the crop inside the
        // valid area, the output equal to the oriented crop.
        assert_eq!(
            Some((info.raster.0 as u32, info.raster.1 as u32)),
            raw_size,
            "{name}"
        );
        let (cw, ch) = render::output_size(info.crop, info.orientation);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (cw, ch),
            "{name}"
        );
        assert!(
            info.crop.w * 10 >= info.valid.0 * 9 && info.crop.h * 10 >= info.valid.1 * 9,
            "{name}: crop {:?}",
            info.crop
        );
        // R-ARW-39 on real data: for V = 2 tiles, the Sony line-start rule
        // gives a smoother luma plane at the lines it affects than the
        // plain T.81 rule (which offsets whole lines).
        let mut line_start = String::new();
        if variant == Variant::V5 {
            let plan = plan_of(&data);
            if let meta::Layout::Tiles { tiles, .. } = &plan.layout {
                let (mut sony, mut t81) = (0f64, 0f64);
                for &(off, len) in tiles.iter().take(6) {
                    let s = lj92::parse(&data[off..off + len]).unwrap();
                    if s.components[0].v != 2 {
                        break;
                    }
                    for (mode, acc) in [(LineStart::Sony, &mut sony), (LineStart::T81, &mut t81)] {
                        let mut y = vec![0u16; 512 * 512];
                        let mut cb = vec![0u16; 256 * 256];
                        let mut cr = vec![0u16; 256 * 256];
                        let targets = [
                            Target {
                                buffer: 0,
                                base: 0,
                                xs: 1,
                                ys: 512,
                            },
                            Target {
                                buffer: 1,
                                base: 0,
                                xs: 1,
                                ys: 256,
                            },
                            Target {
                                buffer: 2,
                                base: 0,
                                xs: 1,
                                ys: 256,
                            },
                        ];
                        s.decode(&mut [&mut y, &mut cb, &mut cr], &targets, mode)
                            .unwrap();
                        for line in (2..512).step_by(2) {
                            for x in 0..512 {
                                let (a, b) = (y[line * 512 + x], y[(line - 1) * 512 + x]);
                                *acc += f64::from(a.abs_diff(b));
                            }
                        }
                    }
                }
                if sony > 0.0 {
                    assert!(sony < t81, "{name}: R-ARW-39 rule {sony} vs T.81 {t81}");
                    line_start = format!(
                        " | V=2 line-start roughness: R-ARW-39 {:.1}, T.81 {:.1}",
                        sony / (6.0 * 255.0 * 512.0),
                        t81 / (6.0 * 255.0 * 512.0)
                    );
                }
            }
        }
        // The camera's largest JPEG, decoded small and oriented by R-ARW-62.
        let cands = preview::collect(&c);
        let best = cands.first().expect("an embedded preview");
        if best.width > 2000 {
            // M13: the full-size JPEG equals DefaultCropSize.
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
        let prev = preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        // Without as-shot white balance the specification uses the table or
        // generic scales (R-ARW-46, OQ-2), while the camera JPEG uses its
        // as-shot balance: compare after removing the global color cast.
        let balance = matches!(info.wb, WbSource::Table | WbSource::Generic);
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let mut line = format!(
            "{name}: model {} {variant:?} raster {}x{} valid {}x{} crop {}x{}+{}+{} out {}x{} wb {:?} matrix {} decode {:.2}s (debug) | vs embedded {}x{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}{}",
            info.model,
            info.raster.0,
            info.raster.1,
            info.valid.0,
            info.valid.1,
            info.crop.w,
            info.crop.h,
            info.crop.x,
            info.crop.y,
            cw,
            ch,
            info.wb,
            if info.matrix_from_table { "table" } else { "generic" },
            took.as_secs_f64(),
            best.width,
            best.height,
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        );
        if mean > CHROMA_MEAN || max > CHROMA_MAX || rank < RANK_MIN || used < 16 {
            failures.push(line.clone());
        }
        // A camera JPEG beside the raw file (same stem) is compared too.
        for ext in ["JPG", "jpg"] {
            let side = path.with_extension(ext);
            if let Ok(j) = std::fs::read(&side) {
                let img = crate::jpeg::decode_image(&j, &opts).expect("side JPEG");
                let (m2, x2, r2, u2) = agreement(&ours, &grid_means(&img, 8, 1), balance);
                line.push_str(&format!(
                    " | vs {}: chroma mean {m2:.4} max {x2:.4} rank {r2:.3} cells {u2}",
                    side.file_name().unwrap().to_string_lossy()
                ));
                if m2 > CHROMA_MEAN || x2 > CHROMA_MAX || r2 < RANK_MIN {
                    failures.push(format!("{name} vs side JPEG"));
                }
                break;
            }
        }
        println!("{line}{line_start}");
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

// ---------------------------------------------------------------------------
// Default display rendering (leaf 1.6, gate G3)
// ---------------------------------------------------------------------------

/// Rec. 709 luminance of linear sRGB.
fn luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// CIE lightness L* of a relative luminance.
fn lightness(y: f64) -> f64 {
    if y > 0.008_856 {
        116.0 * y.cbrt() - 16.0
    } else {
        903.3 * y
    }
}

/// Linear luminance of every `step`-th pixel of an sRGB image (8 or 16
/// bits), sorted.
fn luminances(img: &ImageBuf, step: usize) -> Vec<f64> {
    let lin8: Vec<f64> = (0..256)
        .map(|v| f64::from(crate::util::srgb_to_linear(v as f32 / 255.0)))
        .collect();
    let d = img.data();
    let n = img.width() as usize * img.height() as usize;
    let mut out: Vec<f64> = (0..n)
        .step_by(step)
        .map(|i| match img.format() {
            PixelFormat::Rgba16 => luminance([0, 1, 2].map(|k| {
                let v = u16::from_ne_bytes([d[i * 8 + 2 * k], d[i * 8 + 2 * k + 1]]);
                f64::from(crate::util::srgb_to_linear(f32::from(v) / 65535.0))
            })),
            _ => luminance([0, 1, 2].map(|k| lin8[usize::from(d[i * 4 + k])])),
        })
        .collect();
    out.sort_by(f64::total_cmp);
    out
}

/// Brightness and mid-tone contrast of a picture: mean luminance, mean
/// lightness, and the interquartile range of lightness.
#[derive(Clone, Copy, Debug)]
struct Tone3 {
    mean_y: f64,
    mean_l: f64,
    iqr_l: f64,
}

fn tone_stats(sorted: &[f64]) -> Tone3 {
    let n = sorted.len().max(1);
    let pct = |p: f64| sorted[((sorted.len() - 1) as f64 * p) as usize];
    Tone3 {
        mean_y: sorted.iter().sum::<f64>() / n as f64,
        mean_l: sorted.iter().map(|&y| lightness(y)).sum::<f64>() / n as f64,
        iqr_l: lightness(pct(0.75)) - lightness(pct(0.25)),
    }
}

/// Tolerances of the default rendering against the camera's own JPEG:
/// mean luminance within a third of a stop, mean lightness within 5 L*
/// units, and mid-tone contrast (the L* interquartile range) within -20
/// to +25 percent. The camera adds scene-dependent processing (its
/// dynamic range optimizer, picture style), which one fixed curve cannot
/// follow exactly; these bounds still separate the default rendering from
/// the flat colorimetric image, which misses by 0.6 EV with three quarters
/// of the contrast on the owner's samples and by 1.1 to 1.4 EV with half
/// the contrast on the public samples.
const BRIGHTNESS_EV: f64 = 0.35;
const LIGHTNESS: f64 = 5.0;
const CONTRAST: (f64, f64) = (0.8, 1.25);

/// Gate G3: the default display rendering of developed RAW data lands
/// within the stated brightness tolerance of the camera's embedded
/// preview.
///
/// Synthetic: a uniform gray scene whose sensor values sit at the curve's
/// exposure anchor (3.7 EV below clipping, where a camera meters mid-gray)
/// develops to display mid-gray, the value a camera JPEG gives a metered
/// gray card (sRGB 118); its preview holds that gray. A ramp from black to
/// clipping develops to a rising curve from 0 to 255.
///
/// Real: every `.ARW` in `PANDITE_ARW_SAMPLES` with sensor data in scope
/// is developed with the default rendering and compared with its largest
/// embedded JPEG (decoded at about 1600 pixels) on mean luminance, mean
/// lightness and lightness interquartile range. One parameter set serves
/// every file; nothing is fitted per file.
#[test]
fn raw_default_rendering_matches_preview_brightness() {
    use crate::raw::tone;
    // --- synthetic gray card ---
    let (w, h) = (32u32, 16u32);
    let probe = v1_file(w, h, &vec![1000u16; (w * h) as usize], vec![], 1);
    let plan = plan_of(&probe);
    let anchor = 0.075f64;
    let mut vals = Vec::with_capacity((w * h) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = 2 * (y & 1) + (x & 1);
            let color = usize::from(plan.cfa[p]).min(2);
            let b = f64::from(plan.black[p]);
            let span = f64::from(plan.white) - b;
            // R-ARW-47 inverted: the raw value that normalizes to `anchor`.
            vals.push((b + anchor * span / plan.wb[color]).round() as u16);
        }
    }
    let gray = 118u8;
    let file = with_previews(
        strip_raw(w, h, 14, 1, h),
        vals.iter().flat_map(|v| v.to_le_bytes()).collect(),
        [jpeg(16, 8, [gray; 3]), jpeg(32, 16, [gray; 3])],
        1,
        "ILCE-7M4",
    );
    let options = DecodeOptions {
        prefer_8bit: true,
        ..DecodeOptions::default()
    };
    let dev = decode_with(&file, &options, Rendering::Display)
        .unwrap()
        .unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    assert_eq!(dev.image.format(), PixelFormat::Rgba8);
    let ours = tone_stats(&luminances(&dev.image, 1));
    let preview =
        preview::decode(&file, &preview::collect(&container(&file))[0], 1, &options).unwrap();
    let theirs = tone_stats(&luminances(&preview, 1));
    let ev = (ours.mean_y / theirs.mean_y).log2();
    assert!(ev.abs() <= 0.1, "gray card: {ev:+.3} EV");
    assert!(
        (ours.mean_l - theirs.mean_l).abs() <= 1.5,
        "{ours:?} {theirs:?}"
    );
    // The colorimetric rendering of the same data is the dark one.
    let flat = report(&file);
    let flat = tone_stats(&luminances(&flat.image, 1));
    assert!((flat.mean_y / theirs.mean_y).log2() < -1.0, "{flat:?}");
    // --- synthetic ramp: black to clipping, rising, ends at white ---
    let (w, h) = (64u32, 2u32);
    let ramp: Vec<u16> = (0..h)
        .flat_map(|_| (0..w).map(move |x| (512 + (16_383 - 512) * x / (w - 1)) as u16))
        .collect();
    let file = v1_file(
        w,
        h,
        &ramp,
        vec![(0x7313, Val::SShort(vec![1024, 1024, 1024, 1024]))],
        1,
    );
    let dev = decode_with(&file, &options, Rendering::Display)
        .unwrap()
        .unwrap();
    let row: Vec<u8> = dev
        .image
        .data()
        .chunks_exact(4)
        .take(w as usize)
        .map(|p| p[1])
        .collect();
    // Black (its demosaiced neighbors are barely above black) stays dark.
    assert!(row[0] <= 12, "{row:?}");
    assert!(row.windows(2).all(|p| p[1] >= p[0]), "{row:?}");
    assert!(
        row[w as usize - 2] >= 250,
        "highlights roll off into white: {row:?}"
    );
    // The anchor of the curve itself.
    assert!((tone::curve(anchor, 1.0) - 0.18).abs() < 0.005);

    // --- real camera files ---
    let Ok(dir) = std::env::var("PANDITE_ARW_SAMPLES") else {
        println!("skip real samples: PANDITE_ARW_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("arw"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .ARW files in {}", folder.display());
    let mut failures = Vec::new();
    let mut compared = 0;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        if !matches!(meta::classify(&c), Ok(Class::Sensor(_))) {
            println!("{name}: out of scope, previews only");
            continue;
        }
        let dev = decode_with(&data, &options, Rendering::Display)
            .unwrap()
            .unwrap();
        let info = dev.info.clone().expect("developed");
        let best = preview::collect(&c).into_iter().next().expect("a preview");
        let small = DecodeOptions {
            target_size: Some((1600, 1600)),
            ..DecodeOptions::default()
        };
        let prev = preview::decode(&data, &best, info.orientation, &small).unwrap();
        let ours = tone_stats(&luminances(&dev.image, 7));
        let theirs = tone_stats(&luminances(&prev, 1));
        let flat = report(&data);
        let flat = tone_stats(&luminances(&flat.image, 7));
        let ev = (ours.mean_y / theirs.mean_y).log2();
        let dl = ours.mean_l - theirs.mean_l;
        let ratio = ours.iqr_l / theirs.iqr_l;
        let line = format!(
            "{name}: mean luminance {:+.2} EV, mean L* {:.1} vs {:.1} ({dl:+.1}), L* IQR {:.1} vs {:.1} (x{ratio:.2}) | colorimetric: {:+.2} EV, L* {:.1}, IQR x{:.2}",
            ev,
            ours.mean_l,
            theirs.mean_l,
            ours.iqr_l,
            theirs.iqr_l,
            (flat.mean_y / theirs.mean_y).log2(),
            flat.mean_l,
            flat.iqr_l / theirs.iqr_l,
        );
        println!("{line}");
        if ev.abs() > BRIGHTNESS_EV
            || dl.abs() > LIGHTNESS
            || ratio < CONTRAST.0
            || ratio > CONTRAST.1
        {
            failures.push(line);
        }
        compared += 1;
    }
    assert!(compared > 0, "no file in scope");
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

/// Header facts of every fixture variant: storage, CFA layout, sample
/// depth and the camera of IFD0, through the provider and through the
/// registry (the TIFF provider hands ARW files over). Tags only: a head
/// cut before the sensor data answers the same, and no cut panics.
#[test]
fn facts_arw() {
    use pandite_core::{DecoderProvider, Source};
    let f = super::facts(&f1()).expect("ARW facts");
    assert_eq!(f.container.as_deref(), Some("Sony ARW"));
    assert_eq!(f.compression.as_deref(), Some("Uncompressed"));
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(14)));
    assert_eq!(f.stored_bits_per_pixel(), Some(14));
    assert_eq!(f.subsampling, None);
    assert_eq!(f.camera.make.as_deref(), Some("SONY"));
    assert_eq!(f.camera.model.as_deref(), Some("ILCE-7M4"));
    assert_eq!(f.fourcc, None);

    let compression = |data: &[u8]| super::facts(data).and_then(|f| f.compression);
    assert_eq!(compression(&f2()).as_deref(), Some("Sony 12-bit packed"));
    assert_eq!(
        compression(&f3()).as_deref(),
        Some("Sony lossy, 8 bits per pixel")
    );
    assert_eq!(compression(&f4()).as_deref(), Some("Lossless JPEG (ARW)"));
    let v5 = super::facts(&f5()).expect("V5 facts");
    assert_eq!(v5.compression.as_deref(), Some("Lossless JPEG YCbCr (ARW)"));
    assert_eq!(v5.channels.as_deref(), Some("YCbCr"));
    assert_eq!((v5.channel_count, v5.bits_per_channel), (Some(3), Some(15)));
    assert_eq!(v5.subsampling.as_deref(), Some("4:2:0"));

    // Through the registry, by file name: TIFF signature, ARW extension.
    let reg = crate::default_registry();
    let via = reg
        .facts(&Source::from_bytes(f4(), Some("DSC00001.ARW".into())))
        .expect("registry facts");
    assert_eq!(via.compression.as_deref(), Some("Lossless JPEG (ARW)"));
    assert_eq!(
        crate::raw::RawDecoder
            .facts(&Source::from_bytes(f1(), Some("a.arw".into())))
            .expect("provider facts")
            .channels
            .as_deref(),
        Some("Bayer RGGB")
    );

    // The head of the file: the directories come before the tile.
    let f4 = f4();
    let head = super::facts(&f4[..512]).expect("head facts");
    assert_eq!(head.compression.as_deref(), Some("Lossless JPEG (ARW)"));
    for n in (0..f4.len().min(4096)).step_by(5) {
        let _ = super::facts(&f4[..n]);
    }
    assert!(super::facts(b"MM\0*\0\0\0\x08").is_none(), "big-endian");
}

//! Tests of the ORF decoder against the specification's test vectors
//! (TV-1 to TV-14), fixtures (F1 to F7) and requirements, plus hostile
//! input, cancellation and real camera files.

use super::fixtures::*;
use super::meta::{self, Class, Layout, Plan, Relative, Source, Style, Variant};
use super::*;
use crate::raw::arw::preview::Candidate;
use crate::raw::arw::render;
use crate::raw::arw::tests::{agreement, grid_means, LinearF32, CHROMA_MAX, CHROMA_MEAN, RANK_MIN};
use crate::raw::srw::ifd;
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// TV-11 and F1, F3 to F6: the expected linear sRGB of every pixel.
const TV11: [f32; 3] = [0.573_065_9, 0.419_217_2, 0.270_741_1];
/// TV-11 for V6 and F7.
const TV11_V6: [f32; 3] = [0.572_954, 0.419_135_4, 0.270_688_2];
/// F2.
const F2_OUT: [f32; 3] = [0.573_514_1, 0.419_545_1, 0.270_952_8];
/// The specification prints 7 decimals; the renderer computes in f32.
const EPS: f32 = 2e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open")
}

fn plan_of(data: &[u8]) -> Plan {
    let c = container(data);
    let Class::Sensor(v) = meta::classify(&c).expect("classify") else {
        panic!("not a sensor variant");
    };
    meta::plan(&c, v).expect("plan")
}

fn class_of(data: &[u8]) -> Class {
    meta::classify(&container(data)).expect("classify")
}

/// The raster values as decoded (row-major, image rows).
fn sensor_values(data: &[u8]) -> Vec<u16> {
    let plan = plan_of(data);
    sensor::decode(data, &plan, &CancelToken::new())
        .expect("decode sensor")
        .0
        .data
}

/// Linear sRGB of every output pixel (R-ORF-43), through the shared
/// renderer, cropped and oriented.
fn linear_with(data: &[u8], plan: &Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let rplan = develop::bridge(plan);
    let (plane, _) = sensor::decode(data, plan, &CancelToken::new()).expect("sensor");
    let bytes = render::render(
        &rplan,
        &render::Sensor::Cfa(plane),
        &CancelToken::new(),
        &LinearF32,
    )
    .expect("render");
    let (w, h) = develop::output_size(plan);
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
        .expect("ORF path")
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

/// The entries of the sub-IFD `tag` of a builder's maker note.
fn sub_mut(b: &mut Builder, tag: u16) -> &mut Vec<(u16, Val)> {
    let note = b.note.as_mut().expect("maker note");
    if !note.subs.iter().any(|s| s.0 == tag) {
        note.subs.push((tag, Vec::new()));
    }
    &mut note.subs.iter_mut().find(|s| s.0 == tag).unwrap().1
}

/// Change the entries of the sub-IFD `tag` of a builder's maker note.
fn edit_sub(b: &mut Builder, tag: u16, f: impl FnOnce(Vec<(u16, Val)>) -> Vec<(u16, Val)>) {
    let e = sub_mut(b, tag);
    *e = f(std::mem::take(e));
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

/// `n` pseudo-random values below `max`.
fn random_values(n: usize, max: u32, seed: u32) -> Vec<u16> {
    let mut next = noise(seed);
    (0..n).map(|_| (next() % max) as u16).collect()
}

/// A flat CFA cell: R, G and B at the positions of pattern `cfa`.
fn cell_values(w: usize, h: usize, rgb: [u16; 3], cfa: [u8; 4]) -> Vec<u16> {
    (0..w * h)
        .map(|i| rgb[usize::from(cfa[2 * ((i / w) % 2) + (i % w) % 2])])
        .collect()
}

/// The strip of a W x H raster in a variant (V3 rows in stored order).
fn strip_of(variant: Variant, w: usize, h: usize, values: &[u16]) -> Vec<u8> {
    match variant {
        Variant::V1 => pack_v1(values, true, 12),
        Variant::V2 => pack_v2(values),
        Variant::V3 => {
            let stored: Vec<u16> = (0..h)
                .flat_map(|s| {
                    let y = sensor::image_row(s, h);
                    values[y * w..(y + 1) * w].to_vec()
                })
                .collect();
            pack_v3(&stored)
        }
        Variant::V4 => pack_v4(values),
        Variant::V5 => strip_v5v6(values, w, h, 12),
        Variant::V6 => strip_v5v6(values, w, h, 14),
    }
}

/// A synthetic file of a variant in the recipe layout (OLYMPUS style,
/// little-endian), with BlackLevel2 256 and the fixture's WB and matrix.
fn synth(variant: Variant, w: u32, h: u32, values: &[u16]) -> Builder {
    let (bps, bits) = match variant {
        Variant::V3 | Variant::V4 => (12, 12),
        Variant::V6 => (16, 14),
        _ => (16, 12),
    };
    let mut b = recipe(
        w,
        h,
        bps,
        bits,
        strip_of(variant, w as usize, h as usize, values),
    );
    b.rs = variant == Variant::V3;
    b
}

/// R-ARW-58 (R-ORF-47): the output pixel of stored pixel (x, y) of a W' x
/// H' image, written from the specification's table.
fn oriented(o: u8, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        1 => (x, y),
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        _ => (y, w - 1 - x),
    }
}

/// An independent per-pixel implementation of R-ORF-19, R-ORF-37,
/// R-ORF-39, R-ORF-44 and R-ORF-42 in f64 on the whole raster: linear
/// sRGB of raster pixel (x, y).
fn reference<'a>(values: &'a [u16], plan: &'a Plan) -> impl Fn(usize, usize) -> [f64; 3] + 'a {
    move |x, y| {
        let (w, h) = (plan.width, plan.height);
        let pos = |x: usize, y: usize| 2 * (y % 2) + x % 2;
        let color = |x: usize, y: usize| usize::from(plan.cfa[pos(x, y)]);
        let norm = |x: usize, y: usize| {
            let b = f64::from(plan.black[pos(x, y)]);
            let s = f64::from(values[y * w + x]);
            ((s - b) / (f64::from(plan.white) - b) * plan.wb[color(x, y)]).clamp(0.0, 1.0)
        };
        let own = color(x, y);
        let mut sum = [0f64; 3];
        let mut cnt = [0u32; 3];
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                if (dx == 0 && dy == 0) || xx < 0 || yy < 0 || xx >= w as i64 || yy >= h as i64 {
                    continue;
                }
                let c = color(xx as usize, yy as usize);
                if c != own {
                    sum[c] += norm(xx as usize, yy as usize);
                    cnt[c] += 1;
                }
            }
        }
        let mut cam = [0f64; 3];
        for c in 0..3 {
            cam[c] = if c == own {
                norm(x, y)
            } else if cnt[c] > 0 {
                sum[c] / f64::from(cnt[c])
            } else {
                0.0
            };
        }
        let m = plan.matrix;
        [0, 1, 2].map(|r| (m[r][0] * cam[0] + m[r][1] * cam[1] + m[r][2] * cam[2]).clamp(0.0, 1.0))
    }
}

/// The rendered linear image equals the reference at every pixel of the
/// crop, at its oriented position.
fn assert_reference(data: &[u8], values: &[u16], what: &str) {
    let plan = plan_of(data);
    let (ow, oh, px) = linear_with(data, &plan);
    let c = plan.crop;
    assert_eq!(
        (ow, oh),
        render::output_size(c, plan.orientation),
        "{what}: size"
    );
    let r = reference(values, &plan);
    for y in c.y..c.y + c.h {
        for x in c.x..c.x + c.w {
            let (ox, oy) = oriented(plan.orientation, x - c.x, y - c.y, c.w, c.h);
            let want = r(x, y).map(|v| v as f32);
            assert_rgb(px[oy * ow + ox], want, 2e-6, &format!("{what} ({x}, {y})"));
        }
    }
}

// ---------------------------------------------------------------------------
// Vectors and fixtures
// ---------------------------------------------------------------------------

/// The F2 recipe: V1 big-endian, B = 10, OLYMP note with the levels in the
/// main maker-note IFD.
fn f2_builder() -> Builder {
    let values = cell_values(4, 2, [304, 464, 264], [0, 1, 1, 2]);
    Builder {
        le: false,
        rs: false,
        ifd0: ifd0(4, 2, 16, 1),
        exif: vec![cfa([0, 1, 1, 2], false)],
        note: Some(NoteSpec {
            style: Style::Olymp,
            le: false,
            main: olymp_main(64, 10),
            subs: Vec::new(),
        }),
        ifd1: Vec::new(),
        blobs: vec![pack_v1(&values, false, 10)],
    }
}

/// The F4 recipe: V3, two strips of 2 stored rows, OLYMP note.
fn f4_builder() -> Builder {
    let values = cell_values(4, 4, [1216, 1856, 1056], [0, 1, 1, 2]);
    let strip = strip_of(Variant::V3, 4, 4, &values);
    let mut e = ifd0(4, 4, 12, 1);
    e = with(e, 0x0106, Val::Short(vec![2]));
    e = with(e, 0x0111, Val::Blobs(vec![0, 1]));
    e = with(e, 0x0116, Val::Long(vec![2]));
    e = with(e, 0x0117, Val::BlobLens(vec![0, 1]));
    Builder {
        le: true,
        rs: true,
        ifd0: e,
        exif: vec![cfa([0, 1, 1, 2], true)],
        note: Some(NoteSpec {
            style: Style::Olymp,
            le: true,
            main: olymp_main(256, 12),
            subs: Vec::new(),
        }),
        ifd1: Vec::new(),
        blobs: vec![strip[..12].to_vec(), strip[12..].to_vec()],
    }
}

#[test]
fn orf_builder_reproduces_fixture_hex() {
    // The builder, the packers and the encoder follow the recipes byte for
    // byte, so the other synthetic files share the layout of F1 to F7.
    let sizes = [452, 448, 352, 448, 380, 440, 454];
    let files = [f6(), f1(), f2(), f3(), f4(), f5(), f7()];
    for (f, n) in files.iter().zip(sizes) {
        assert_eq!(f.len(), n);
    }
    let cell = |w, h| cell_values(w, h, [1216, 1856, 1056], [0, 1, 1, 2]);
    assert_eq!(synth(Variant::V5, 4, 4, &cell(4, 4)).build(), f6(), "F6");
    assert_eq!(synth(Variant::V1, 4, 4, &cell(4, 4)).build(), f1(), "F1");
    assert_eq!(synth(Variant::V2, 10, 2, &cell(10, 2)).build(), f3(), "F3");
    assert_eq!(synth(Variant::V4, 8, 2, &cell(8, 2)).build(), f5(), "F5");
    let v6 = cell_values(4, 4, [4864, 7424, 4224], [0, 1, 1, 2]);
    assert_eq!(synth(Variant::V6, 4, 4, &v6).build(), f7(), "F7");
    assert_eq!(f2_builder().build(), f2(), "F2");
    assert_eq!(f4_builder().build(), f4(), "F4");
}

/// A file with the given IFD0 size, BitsPerSample, header and strips, for
/// the rules of R-ORF-3.
fn classify_with(le: bool, rs: bool, w: u32, h: u32, bps: u16, strips: Vec<Vec<u8>>) -> Class {
    let n = strips.len();
    let mut e = ifd0(w, h, bps, 1);
    e = with(e, 0x0111, Val::Blobs((0..n).collect()));
    e = with(e, 0x0117, Val::BlobLens((0..n).collect()));
    let f = Builder {
        le,
        rs,
        ifd0: e,
        exif: vec![cfa([0, 1, 1, 2], le)],
        blobs: strips,
        ..Builder::default()
    }
    .build();
    class_of(&f)
}

/// An Exif IFD at offset 0 holding only `0xA302` with the given value.
fn cfa_dir(value: &[u8], le: bool) -> Vec<u8> {
    let w16 = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let w32 = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() };
    let mut d = w16(1).to_vec();
    d.extend(w16(0xA302));
    d.extend(w16(7));
    d.extend(w32(value.len() as u32));
    if value.len() <= 4 {
        let mut v = value.to_vec();
        v.resize(4, 0);
        d.extend(v);
    } else {
        d.extend(w32(18));
    }
    d.extend(w32(0));
    if value.len() > 4 {
        d.extend(value);
    }
    d
}

/// TV-8: the code bits of the 10 x 4 raster.
const TV8: &str = "00 00 0F B0 03 7A BF A4 D2 29 19 28 68 78 00 74 00 2D A8 D4 14 B4 BC 4C
    8C D1 48 C5 C1 43 47 43 80 00 1E FA E8 28 04 F1 52 11 10 72 94 2C C0 09 B7 8D 00";

/// TV-8: the expected values.
#[rustfmt::skip]
const TV8_VALUES: [u16; 40] = [
    4000, 300, 3990, 310, 3980, 320, 3985, 322, 3983, 321,
    500, 600, 510, 590, 520, 580, 515, 585, 512, 583,
    4010, 305, 4003, 312, 3985, 318, 2000, 330, 3990, 325,
    495, 610, 502, 600, 518, 570, 540, 900, 511, 588,
];

fn decode_stream(stream: &[u8], w: usize, h: usize, bits: u32) -> OrfResult<(Vec<u16>, u64)> {
    let mut out = vec![0u16; w * h];
    let n = lossless::decode(stream, w, h, bits, &mut out, &CancelToken::new())?;
    Ok((out, n))
}

#[test]
fn orf_vectors_decode() {
    // TV-1 (R-ORF-4, R-ORF-5): the three ORF starts and their byte order.
    let (d6, d4, d2) = (f6(), f4(), f2());
    let c = container(&d6);
    assert!(c.le && !c.rs, "49 49 52 4F");
    let c = container(&d4);
    assert!(c.le && c.rs, "49 49 52 53");
    let c = container(&d2);
    assert!(!c.le, "4D 4D 4F 52");
    // R-ORF-8: IFD0 at the header offset 8, with 13 entries (F2: 00 00 00
    // 08, big-endian).
    assert_eq!(c.ifd0().entries.len(), 13);
    // TV-1: a plain TIFF header is an error without a preview (R-ORF-4),
    // even with a JPEG inside; so is another start.
    for start in [
        [0x49, 0x49, 0x2A, 0x00],
        [0x4D, 0x4D, 0x00, 0x2A],
        [0x49, 0x49, 0x52, 0x50],
    ] {
        let mut f = with_previews(
            f6_builder(),
            jpeg(32, 16, [200, 40, 40]),
            jpeg(16, 8, [9; 3]),
        )
        .build();
        f[..4].copy_from_slice(&start);
        assert!(meta::open(&f).is_err(), "{start:?}");
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
        assert!(probe(&f).is_err());
        assert!(!has_preview(&f));
    }
    // R-ORF-55: shorter than 8 bytes.
    assert!(meta::open(&f6()[..7]).is_err());
    // R-ORF-4: an invalid IFD0 (R-ARW-14) is an error without a preview.
    let previews = with_previews(
        f6_builder(),
        jpeg(32, 16, [200, 40, 40]),
        jpeg(16, 8, [9; 3]),
    );
    let good = previews.build();
    assert_eq!(report(&good).origin, Origin::Sensor(Variant::V5));
    let mut f = good.clone();
    f[4..8].copy_from_slice(&(good.len() as u32).to_le_bytes());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "outside"
    );
    let mut f = good.clone();
    f[8..10].copy_from_slice(&0u16.to_le_bytes());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "N = 0"
    );
    let mut f = good.clone();
    f[8..10].copy_from_slice(&1025u16.to_le_bytes());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "N > 1024"
    );
    let mut f = good.clone();
    f[8..10].copy_from_slice(&((good.len() / 12) as u16).to_le_bytes());
    assert!(meta::open(&f).is_err(), "entries past the end");

    // TV-2: maker-note headers. OLYMP (F2): main IFD 8 bytes after the
    // start, 5 entries, offsets from the file start.
    let f = f2();
    let c = container(&f);
    let n = c.note.as_ref().expect("maker note");
    assert_eq!(
        (n.style, n.start, n.origin, n.le),
        (Style::Olymp, 0xD8, 0, false)
    );
    let main = n.main.as_ref().expect("main IFD");
    assert_eq!(main.entries.len(), 5);
    assert_eq!(main.entry(0x1011).unwrap().at, 0x122, "file offset");
    // OLYMPUS (F6): main IFD at 0xD8 + 12 with 1 entry, 0x2040 type 13
    // count 1 value 0x1E: ImageProcessing at 0xD8 + 0x1E = 0xF6 with 9
    // entries whose value offsets count from 0xD8.
    let f = f6();
    let c = container(&f);
    let n = c.note.as_ref().expect("maker note");
    assert_eq!(
        (n.style, n.start, n.origin, n.le),
        (Style::Olympus, 0xD8, 0xD8, true)
    );
    let main = n.main.as_ref().unwrap();
    let e = main.entry(0x2040).unwrap();
    assert_eq!((main.entries.len(), e.typ, e.count), (1, 13, 1));
    assert_eq!(main.uint(0x2040).unwrap(), Some(0x1E));
    let ip = n.image_processing.as_ref().expect("ImageProcessing");
    assert_eq!(ip.entries.len(), 9);
    assert_eq!(ip.entry(0x0610).unwrap().at, 0xF6 + 2 + 3 * 12 + 8);
    assert_eq!(ip.entry(0x0100).unwrap().at, 0xD8 + 0x90);
    assert!(n.camera_settings.is_none() && n.raw_info.is_none());
    // OM SYSTEM, both byte orders: main IFD 16 bytes after the start.
    for le in [true, false] {
        let mut b = f6_builder();
        let note = b.note.as_mut().unwrap();
        note.style = Style::OmSystem;
        note.le = le;
        let f = b.build();
        let c = container(&f);
        let n = c.note.as_ref().unwrap();
        assert_eq!((n.style, n.le), (Style::OmSystem, le));
        assert_eq!(
            &f[n.start as usize..n.start as usize + 12],
            b"OM SYSTEM\0\0\0"
        );
        for v in linear(&f).2 {
            assert_rgb(v, TV11, EPS, "OM SYSTEM note");
        }
        // R-ORF-12: a big-endian maker note in a little-endian file.
        let mut b = f6_builder();
        let note = b.note.as_mut().unwrap();
        note.le = le;
        for v in linear(&b.build()).2 {
            assert_rgb(v, TV11, EPS, "OLYMPUS note");
        }
    }
    // R-ORF-13: sub-IFD pointers of type 4 work; type 13 with count 2,
    // type 3 and an invalid sub-IFD make it absent (preview path, no black
    // level).
    let base = f6();
    let at = 0xE6; // the 0x2040 entry in F6
    let mut f = base.clone();
    f[at + 2] = 4;
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V5), "type 4");
    for (typ, count, value) in [(13u16, 2u32, 0x1Eu32), (3, 1, 0x1E), (13, 1, 0x1000)] {
        let mut f = base.clone();
        f[at + 2..at + 4].copy_from_slice(&typ.to_le_bytes());
        f[at + 4..at + 8].copy_from_slice(&count.to_le_bytes());
        f[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
        let c = container(&f);
        assert!(
            c.note.as_ref().unwrap().image_processing.is_none(),
            "{typ} {count}"
        );
        assert!(meta::plan(&c, Variant::V5).is_err());
    }
    // OLYMP style sub-IFDs: type 7 with the sub-IFD as the value, offsets
    // from the file start (R-ORF-13); a type 7 entry with 4 bytes is not a
    // sub-IFD.
    let mut b = f2_builder();
    b.note.as_mut().unwrap().main = Vec::new();
    edit_sub(&mut b, 0x2040, |_| {
        vec![
            (0x0100, Val::Short(vec![512, 64])),
            (0x0200, Val::Short(MATRIX.to_vec())),
            (0x0611, Val::Short(vec![10])),
        ]
    });
    edit_sub(&mut b, 0x3000, |_| vec![(0x0600, Val::Short(vec![64; 4]))]);
    let f = b.build();
    let c = container(&f);
    let n = c.note.as_ref().unwrap();
    let e = n.main.as_ref().unwrap().entry(0x2040).unwrap();
    assert_eq!(e.typ, 7);
    assert!(n.image_processing.is_some() && n.raw_info.is_some());
    let p = plan_of(&f);
    assert_eq!(
        (p.bits, p.black, p.black_source),
        (10, [64; 4], Source::RawInfo)
    );
    assert_eq!(p.matrix_source, Source::ImageProcessing);
    let mut short = f.clone();
    let main_at = n.start as usize + 8;
    let raw_info_entry = main_at + 2 + 12; // sorted: 0x2040, 0x3000
    short[raw_info_entry + 4..raw_info_entry + 8].copy_from_slice(&4u32.to_be_bytes());
    assert!(
        container(&short).note.unwrap().raw_info.is_none(),
        "count 4"
    );

    // TV-3: CFAPattern values (R-ORF-15, R-ORF-19).
    for (bytes, le, want) in [
        ("02 00 02 00 00 01 01 02", true, [0, 1, 1, 2]),
        ("00 02 00 02 00 01 01 02", false, [0, 1, 1, 2]),
        ("02 00 02 00 01 00 02 01", true, [1, 0, 2, 1]),
        ("02 00 02 00 02 01 01 00", true, [2, 1, 1, 0]),
    ] {
        let d = cfa_dir(&hex(bytes), le);
        let dir = ifd::dir(&d, le, 0, 0).unwrap();
        assert_eq!(meta::cfa_pattern(&dir, le).unwrap(), want, "{bytes}");
    }
    for bad in [
        "02 00 02 00 00 01 01",
        "02 00 02 00 00 01 01 02 00",
        "02 00 03 00 00 01 01 02",
        "00 02 00 02 00 01 01 02",
        "02 00 02 00 00 00 01 02",
        "02 00 02 00 00 01 01 03",
    ] {
        let d = cfa_dir(&hex(bad), true);
        let dir = ifd::dir(&d, true, 0, 0).unwrap();
        assert!(meta::cfa_pattern(&dir, true).is_err(), "{bad}");
    }

    // TV-4: V1 samples (R-ORF-22).
    let row = |variant, bytes: &str, le, bits, n| {
        let mut out = vec![0u16; n];
        sensor::unpack_row(variant, &hex(bytes), le, bits, &mut out).unwrap();
        out
    };
    assert_eq!(
        row(Variant::V1, "0C 00 0C C0 0B 80 0D 80", false, 10, 4),
        [48, 51, 46, 54]
    );
    assert_eq!(
        row(Variant::V1, "60 43 30 22 B0 43 A0 23", true, 12, 4),
        [1078, 547, 1083, 570]
    );
    // TV-5: a V2 group (R-ORF-23), the pad byte ignored.
    let tv5 = "6B 22 61 BA A2 5E 97 02 5E D5 F2 5D BF 72 5E 00";
    let want = [619, 1554, 698, 1514, 663, 1504, 725, 1503, 703, 1511];
    assert_eq!(row(Variant::V2, tv5, true, 12, 10), want);
    assert_eq!(
        row(Variant::V2, &tv5.replace(" 00", " FF"), true, 12, 10),
        want
    );
    // TV-6: V3 pairs (R-ORF-24) and the field order (R-ORF-21).
    assert_eq!(
        row(Variant::V3, "2B F5 93 31 15 8F", true, 12, 4),
        [703, 1427, 785, 1423]
    );
    let rows: Vec<usize> = (0..4).map(|s| sensor::image_row(s, 4)).collect();
    assert_eq!(rows, [0, 2, 1, 3]);
    let rows: Vec<usize> = (0..5).map(|s| sensor::image_row(s, 5)).collect();
    assert_eq!(rows, [0, 2, 4, 1, 3]);
    assert_eq!(sensor::image_row(974, 1949), 1948);
    assert_eq!(sensor::image_row(975, 1949), 1);
    for h in [1, 2, 5, 1949] {
        for s in 0..h {
            assert_eq!(sensor::stored_row(sensor::image_row(s, h), h), s);
        }
    }
    // TV-7: V4 words (R-ORF-25).
    assert_eq!(
        row(
            Variant::V4,
            "79 A4 CE 7A 0E 79 B4 CE D8 4E 7C E4",
            true,
            12,
            8
        ),
        [1964, 3748, 1948, 3764, 1936, 3812, 1988, 3800]
    );

    // TV-8: the synthetic V5 stream (R-ORF-26 to R-ORF-31): every row of
    // the prediction table, escapes with k = 4 and k = 2, negative E, C
    // and D, both branches of the code length rule; 6 bits unused.
    let tv8 = hex(TV8);
    assert_eq!(tv8.len(), 51);
    let (values, read) = decode_stream(&tv8, 10, 4, 12).unwrap();
    assert_eq!(values, TV8_VALUES);
    assert_eq!(read, 51 * 8 - 6);
    // The rules in reverse reproduce the stream.
    assert_eq!(encode(&TV8_VALUES, 10, 4), tv8);
    // R-ORF-26: bytes after the last pixel are ignored.
    let mut longer = tv8.clone();
    longer.extend([0xFF; 9]);
    assert_eq!(decode_stream(&longer, 10, 4, 12).unwrap().0, TV8_VALUES);
    // R-ORF-60: one byte less needs bits beyond the end.
    assert!(decode_stream(&tv8[..50], 10, 4, 12).is_err());
    // R-ORF-27: the state is reset per row: one row of TV-8 alone decodes
    // to the same first row.
    assert_eq!(decode_stream(&tv8, 10, 1, 12).unwrap().0, TV8_VALUES[..10]);
    // R-ORF-32, R-ORF-61: a value below 0 (S = 1, L = 0, Q = 0, R = 0 at
    // pixel (0, 0): s = -4) or above 2^B - 1 fails.
    assert!(decode_stream(&[0x90, 0, 0, 0], 2, 1, 12).is_err());
    let high = encode(&[4096, 0], 2, 1);
    assert!(decode_stream(&high, 2, 1, 12).is_err());
    assert_eq!(decode_stream(&high, 2, 1, 14).unwrap().0, [4096, 0]);
    assert!(decode_stream(&encode(&[16383, 0], 2, 1), 2, 1, 14).is_ok());

    // TV-9: the start of the E-3 strip.
    let tv9 = hex("00 00 00 00 01 00 00 4B 32 86 2A 8A 22 A2 67 A9");
    assert_eq!(tv9[..7], meta::V5_HEADER);
    let (values, _) = decode_stream(&tv9[7..], 8, 1, 12).unwrap();
    assert_eq!(values, [90, 147, 103, 177, 109, 197, 121, 250]);
    // M12: re-encoding the first 8 values gives the first code bytes.
    let again = encode(&values, 8, 1);
    assert_eq!(again[..again.len() - 1], tv9[7..7 + again.len() - 1]);
    // TV-10: the start of the V6 strip of the OM-1 Mark II file.
    let tv10 = hex("00 00 00 00 00 01 00 00 00 00 04 78 00 00 0A C2 CC 7A 03 21 4B");
    assert_eq!(tv10[..8], meta::V6_HEADER);
    let (values, _) = decode_stream(&tv10[8..], 6, 1, 14).unwrap();
    assert_eq!(values, [1136, 1344, 1141, 1339, 1150, 1355]);

    // TV-12: the orientation table (R-ORF-47) on a 3 x 2 image.
    let place = |o: u8| {
        let (w, h) = (3, 2);
        let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
        let mut out = vec![' '; ow * oh];
        for (i, ch) in "abcdef".chars().enumerate() {
            let (x, y) = oriented(o, i % w, i / w, w, h);
            out[y * ow + x] = ch;
        }
        out.into_iter().collect::<String>()
    };
    assert_eq!(place(6), "daebfc");
    assert_eq!(place(8), "cfbead");
    assert_eq!(place(3), "fedcba");

    // F1 to F7: every output pixel is TV-11 (F7 its V6 values, F2 its own).
    let cell = cell_values(4, 4, [1216, 1856, 1056], [0, 1, 1, 2]);
    for (name, f, variant, want, values, size) in [
        ("F1", f1(), Variant::V1, TV11, cell.clone(), (4, 4)),
        (
            "F2",
            f2(),
            Variant::V1,
            F2_OUT,
            cell_values(4, 2, [304, 464, 264], [0, 1, 1, 2]),
            (4, 2),
        ),
        (
            "F3",
            f3(),
            Variant::V2,
            TV11,
            cell_values(10, 2, [1216, 1856, 1056], [0, 1, 1, 2]),
            (10, 2),
        ),
        ("F4", f4(), Variant::V3, TV11, cell.clone(), (4, 4)),
        (
            "F5",
            f5(),
            Variant::V4,
            TV11,
            cell_values(8, 2, [1216, 1856, 1056], [0, 1, 1, 2]),
            (8, 2),
        ),
        ("F6", f6(), Variant::V5, TV11, cell.clone(), (4, 4)),
        (
            "F7",
            f7(),
            Variant::V6,
            TV11_V6,
            cell_values(4, 4, [4864, 7424, 4224], [0, 1, 1, 2]),
            (4, 4),
        ),
    ] {
        assert_eq!(class_of(&f), Class::Sensor(variant), "{name}");
        assert_eq!(sensor_values(&f), values, "{name} sensor values");
        let (w, h, px) = linear(&f);
        assert_eq!((w, h), size, "{name}");
        for (i, &v) in px.iter().enumerate() {
            assert_rgb(v, want, EPS, &format!("{name} pixel {i}"));
        }
        let dev = report(&f);
        assert_eq!(dev.origin, Origin::Sensor(variant), "{name}");
        assert_eq!(dev.class, Some(Class::Sensor(variant)));
        assert_rgba16_flat(&dev.image, want, name);
    }
    // F6, F7: the code bits fill whole bytes (232 and 240 bits): no unused
    // bit. F6 fills exactly 29 bytes.
    assert_eq!(report(&f6()).info.unwrap().unused_bits, Some(0));
    assert_eq!(report(&f7()).info.unwrap().unused_bits, Some(0));
    assert_eq!(encode(&cell, 4, 4).len(), 29);

    // TV-14: CFA phase GRBG, demosaicing of the whole raster, then the crop
    // (1, 1, 2, 2): normalized values v = (s - 95) / 4000, unit white
    // balance and the identity matrix, so the output is camera RGB.
    #[rustfmt::skip]
    let v: [f64; 16] = [
        0.10, 0.20, 0.30, 0.40,
        0.50, 0.60, 0.70, 0.80,
        0.15, 0.25, 0.35, 0.45,
        0.55, 0.65, 0.75, 0.85,
    ];
    let s: Vec<u16> = v
        .iter()
        .map(|&v| (95.0 + v * 4000.0).round() as u16)
        .collect();
    let tv14 = |crop: Option<[u32; 4]>| {
        let mut b = synth(Variant::V1, 4, 4, &s);
        b.exif = vec![cfa([1, 0, 2, 1], true)];
        edit_sub(&mut b, 0x2040, |e| {
            let mut e = with(e, 0x0100, Val::Short(vec![256, 256]));
            e = with(e, 0x0200, Val::Short(vec![256, 0, 0, 0, 256, 0, 0, 0, 256]));
            e = with(e, 0x0600, Val::Short(vec![95; 4]));
            if let Some([x, y, w, h]) = crop {
                e = with(e, 0x0612, Val::Short(vec![x as u16, 0]));
                e = with(e, 0x0613, Val::Short(vec![y as u16, 0]));
                e = with(e, 0x0614, Val::Long(vec![w]));
                e = with(e, 0x0615, Val::Long(vec![h]));
            }
            e
        });
        b.build()
    };
    let f = tv14(Some([1, 1, 2, 2]));
    let p = plan_of(&f);
    assert_eq!(p.cfa, [1, 0, 2, 1]);
    assert_eq!((p.crop.x, p.crop.y, p.crop.w, p.crop.h), (1, 1, 2, 2));
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (2, 2));
    let want = [
        [0.225, 0.60, 0.60],
        [0.325, 0.5125, 0.70],
        [0.25, 0.4375, 0.625],
        [0.35, 0.35, 0.725],
    ];
    for (i, (&got, want)) in px.iter().zip(want).enumerate() {
        assert_rgb(got, want, 1e-6, &format!("TV-14 output {i}"));
    }
    let (_, _, px) = linear(&tv14(None));
    assert_rgb(px[0], [0.20, 0.10, 0.50], 1e-6, "TV-14 corner");

    // R-ORF-3: the classification rules, in order.
    use Class::{OutOfScope, Sensor};
    let bytes = |n: usize| vec![0x55u8; n];
    let stream = |head: &[u8], n: usize| {
        let mut s = head.to_vec();
        s.resize(n, 0x55);
        s
    };
    // Rule 1, in both byte orders, and before the stream rules.
    assert_eq!(
        classify_with(true, false, 4, 2, 16, vec![bytes(16)]),
        Sensor(Variant::V1)
    );
    assert_eq!(
        classify_with(false, false, 4, 2, 16, vec![bytes(16)]),
        Sensor(Variant::V1)
    );
    assert_eq!(
        classify_with(true, false, 4, 2, 16, vec![stream(&meta::V5_HEADER, 16)]),
        Sensor(Variant::V1)
    );
    assert_eq!(
        classify_with(true, false, 4, 2, 16, vec![bytes(6), bytes(10)]),
        Sensor(Variant::V1)
    );
    // Rule 2: big-endian and not V1.
    assert_eq!(
        classify_with(false, false, 8, 2, 12, vec![bytes(24)]),
        OutOfScope(Relative::X2)
    );
    assert_eq!(
        classify_with(false, false, 4, 2, 16, vec![stream(&meta::V5_HEADER, 9)]),
        OutOfScope(Relative::X2)
    );
    // Rule 3: W mod 10 = 0 and T = 16 n / 10.
    assert_eq!(
        classify_with(true, false, 10, 2, 16, vec![bytes(32)]),
        Sensor(Variant::V2)
    );
    assert_eq!(
        classify_with(true, false, 5, 2, 16, vec![bytes(16)]),
        OutOfScope(Relative::X1)
    );
    // Rules 4 and 5: BPS 12, the header bytes 2 and 3, W and T = 3 n / 2.
    assert_eq!(
        classify_with(true, true, 6, 2, 12, vec![bytes(18)]),
        Sensor(Variant::V3)
    );
    assert_eq!(
        classify_with(true, true, 8, 2, 12, vec![bytes(24)]),
        Sensor(Variant::V3)
    );
    assert_eq!(
        classify_with(true, false, 8, 2, 12, vec![bytes(24)]),
        Sensor(Variant::V4)
    );
    assert_eq!(
        classify_with(true, false, 6, 2, 12, vec![bytes(18)]),
        OutOfScope(Relative::X1)
    );
    assert_eq!(
        classify_with(true, true, 8, 2, 16, vec![bytes(24)]),
        OutOfScope(Relative::X1)
    );
    // Rules 6 and 7: one strip, W even, the stream header; a strip shorter
    // than 8 bytes does not match.
    let v5 = stream(&meta::V5_HEADER, 9);
    let v6 = stream(&meta::V6_HEADER, 9);
    assert_eq!(
        classify_with(true, false, 4, 4, 16, vec![v5.clone()]),
        Sensor(Variant::V5)
    );
    assert_eq!(
        classify_with(true, true, 4, 4, 16, vec![v6.clone()]),
        Sensor(Variant::V6)
    );
    assert_eq!(
        classify_with(true, false, 3, 4, 16, vec![v5.clone()]),
        OutOfScope(Relative::X1)
    );
    assert_eq!(
        classify_with(true, false, 4, 4, 16, vec![v5.clone(), bytes(4)]),
        OutOfScope(Relative::X1)
    );
    assert_eq!(
        classify_with(true, false, 4, 4, 16, vec![meta::V5_HEADER.to_vec()]),
        OutOfScope(Relative::X1)
    );
    let mut other = v5.clone();
    other[4] = 2;
    assert_eq!(
        classify_with(true, false, 4, 4, 16, vec![other]),
        OutOfScope(Relative::X1)
    );
    // Rule 3 comes before the stream rules.
    assert_eq!(
        classify_with(true, false, 10, 1, 16, vec![stream(&meta::V5_HEADER, 16)]),
        Sensor(Variant::V2)
    );
    // R-ORF-14: the required IFD0 tags are checked first.
    for tag in [0x0100u16, 0x0101, 0x0102, 0x0111, 0x0117] {
        let mut b = f6_builder();
        b.ifd0 = without(b.ifd0, tag);
        let f = b.build();
        let c = container(&f);
        assert!(meta::classify(&c).is_err(), "{tag:04X}");
        assert!(meta::plan(&c, Variant::V5).is_err(), "{tag:04X}");
    }
    // The ExifIFD entry of F6 (at 0x9A) under another tag number, and an
    // Exif offset outside the file.
    let mut f = f6();
    f[0x9A] = 0x68;
    assert!(meta::classify(&container(&f)).is_err(), "ExifIFD");
    let mut f = f6();
    f[0xA2..0xA6].copy_from_slice(&0xFFFFu32.to_le_bytes());
    assert!(
        meta::plan(&container(&f), Variant::V5).is_err(),
        "Exif outside"
    );
    // A count of 0 counts as absent.
    let mut f = f6();
    f[0x0E..0x12].copy_from_slice(&0u32.to_le_bytes());
    assert!(
        meta::classify(&container(&f)).is_err(),
        "ImageWidth count 0"
    );

    // R-ORF-20: strips in any order and of any size, a whole number of
    // rows each, StripByteCounts / RB rows per strip; RowsPerStrip is not
    // used.
    let values = random_values(6 * 5, 4096, 3);
    let packed = pack_v1(&values, true, 12);
    let mut b = synth(Variant::V1, 6, 5, &values);
    b.blobs = vec![
        packed[36..48].to_vec(),
        packed[..36].to_vec(),
        packed[48..].to_vec(),
    ];
    b.ifd0 = with(b.ifd0, 0x0111, Val::Blobs(vec![1, 0, 2]));
    b.ifd0 = with(b.ifd0, 0x0117, Val::BlobLens(vec![1, 0, 2]));
    b.ifd0 = with(b.ifd0, 0x0116, Val::Long(vec![2]));
    let f = b.build();
    assert_eq!(sensor_values(&f), values);
    // R-ORF-58 case 2: a strip that is not a whole number of rows.
    let mut b2 = b.clone();
    b2.blobs = vec![
        packed[36..47].to_vec(),
        packed[..37].to_vec(),
        packed[48..].to_vec(),
    ];
    assert!(meta::plan(&container(&b2.build()), Variant::V1).is_err());
    // R-ORF-58 case 1: different counts.
    let mut b3 = b.clone();
    b3.ifd0 = with(b3.ifd0, 0x0111, Val::Blobs(vec![1, 0]));
    assert!(meta::plan(&container(&b3.build()), Variant::V1).is_err());
    // R-ORF-21 with an odd H and fields in separate strips (M7): the even
    // field fills strips of 2 stored rows, the odd field starts a new one.
    let (w, h) = (6usize, 5usize);
    let values = random_values(w * h, 4096, 4);
    let strip = strip_of(Variant::V3, w, h, &values);
    let rb = 3 * w / 2;
    let mut b = synth(Variant::V3, w as u32, h as u32, &values);
    b.blobs = vec![
        strip[..2 * rb].to_vec(),
        strip[2 * rb..3 * rb].to_vec(),
        strip[3 * rb..].to_vec(),
    ];
    b.ifd0 = with(b.ifd0, 0x0111, Val::Blobs(vec![0, 1, 2]));
    b.ifd0 = with(b.ifd0, 0x0117, Val::BlobLens(vec![0, 1, 2]));
    let f = b.build();
    assert_eq!(class_of(&f), Sensor(Variant::V3));
    assert_eq!(sensor_values(&f), values);
    assert_reference(&f, &values, "V3 fields");

    // Every variant against the reference implementation, with random
    // values, a crop and every orientation (R-ORF-17, R-ORF-19, R-ORF-37,
    // R-ORF-39, R-ORF-42, R-ORF-44 to R-ORF-47).
    for (variant, w, h, max) in [
        (Variant::V1, 10, 6, 4096),
        (Variant::V2, 20, 6, 4096),
        (Variant::V3, 10, 7, 4096),
        (Variant::V4, 16, 6, 4096),
        (Variant::V5, 12, 9, 4096),
        (Variant::V6, 12, 9, 16384),
    ] {
        let values = random_values(w * h, max, w as u32 * 31 + h as u32);
        for o in 1..=8u16 {
            let mut b = synth(variant, w as u32, h as u32, &values);
            b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![o]));
            b.exif = vec![cfa([2, 1, 1, 0], true)];
            edit_sub(&mut b, 0x2040, |e| {
                let mut e = with(e, 0x0600, Val::Short(vec![250, 251, 252, 253]));
                e = with(e, 0x0612, Val::Short(vec![1, 0]));
                e = with(e, 0x0613, Val::Short(vec![2, 0]));
                e = with(e, 0x0614, Val::Long(vec![w as u32 - 3]));
                e = with(e, 0x0615, Val::Long(vec![h as u32 - 3]));
                e
            });
            let f = b.build();
            assert_eq!(class_of(&f), Sensor(variant));
            assert_eq!(sensor_values(&f), values, "{variant:?}");
            let p = plan_of(&f);
            let scale = if variant == Variant::V6 { 4 } else { 1 };
            assert_eq!(p.black, [250, 251, 252, 253].map(|b| b * scale));
            assert_reference(&f, &values, &format!("{variant:?} orientation {o}"));
            let dev = report(&f);
            let size = render::output_size(p.crop, o as u8);
            assert_eq!(
                (dev.image.width() as usize, dev.image.height() as usize),
                size
            );
            assert_eq!(probe(&f).unwrap().unwrap().0 as usize, size.0);
        }
    }
    // R-ORF-56: limits on W, H and W * H.
    for (w, h) in [(0u32, 4u32), (4, 0), (65536, 2), (2, 65536), (16384, 8194)] {
        let mut b = f6_builder();
        b.ifd0 = with(b.ifd0, 0x0100, Val::Long(vec![w]));
        b.ifd0 = with(b.ifd0, 0x0101, Val::Long(vec![h]));
        let f = b.build();
        let c = container(&f);
        assert!(meta::plan(&c, Variant::V5).is_err(), "{w} x {h}");
        assert!(meta::plan(&c, Variant::V1).is_err(), "{w} x {h}");
    }
    // R-ORF-58 case 3 and R-ORF-60: a compressed strip of only its header
    // plus 1 byte fails, a strip too short for the raster too.
    let mut b = synth(Variant::V6, 4, 4, &[0; 16]);
    b.blobs = vec![meta::V6_HEADER.to_vec()];
    let f = b.build();
    assert_eq!(class_of(&f), Sensor(Variant::V6));
    assert!(meta::plan(&container(&f), Variant::V6).is_err());
    let mut b = f6_builder();
    b.blobs[0].truncate(18);
    let f = b.build();
    assert!(meta::plan(&container(&f), Variant::V5).is_err());
    // A stream long enough by the bound, but cut inside the codes: the
    // decode fails (R-ORF-60) and nothing partial is delivered (R-ORF-65).
    let values = random_values(16 * 16, 4096, 9);
    let mut b = synth(Variant::V5, 16, 16, &values);
    let n = b.blobs[0].len();
    b.blobs[0].truncate(n - 4);
    let f = b.build();
    assert!(meta::plan(&container(&f), Variant::V5).is_ok());
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    let layout = plan_of(&synth(Variant::V5, 16, 16, &values).build()).layout;
    assert!(matches!(layout, Layout::Stream { .. }));
}

/// A V1 file of 4 x 4 zeros with a main maker-note BlackLevel of 16 (for
/// bit depths below 12, which skip BlackLevel2 of ImageProcessing).
fn v1_builder() -> Builder {
    let mut b = synth(Variant::V1, 4, 4, &[0; 16]);
    b.note.as_mut().unwrap().main = vec![(0x1012, Val::Short(vec![16; 4]))];
    b
}

/// F6 through the builder.
fn f6_builder() -> Builder {
    synth(
        Variant::V5,
        4,
        4,
        &cell_values(4, 4, [1216, 1856, 1056], [0, 1, 1, 2]),
    )
}

#[test]
fn orf_levels_wb_matrix_vector() {
    // TV-11 and the worked example of R-ORF-39: normalized and balanced.
    let norm = |s: f64, b: f64, w: f64, m: f64| ((s - b) / (w - b) * m).clamp(0.0, 1.0);
    let v = [
        norm(1216.0, 256.0, 4095.0, 2.0),
        norm(1856.0, 256.0, 4095.0, 1.0),
        norm(1056.0, 256.0, 4095.0, 1.5),
    ];
    for (got, want) in v.iter().zip([0.500_130_2, 0.416_775_2, 0.312_581_4]) {
        assert!((got - want).abs() < 5e-8, "{got}");
    }
    // The V6 cell: black 256 * 4, white 16383.
    let v6 = [
        norm(4864.0, 1024.0, 16383.0, 2.0),
        norm(7424.0, 1024.0, 16383.0, 1.0),
        norm(4224.0, 1024.0, 16383.0, 1.5),
    ];
    for (got, want) in v6.iter().zip([0.500_032_6, 0.416_693_8, 0.312_520_3]) {
        assert!((got - want).abs() < 5e-8, "{got}");
    }
    // F2 (black 64, white 1023).
    let v2 = [
        norm(304.0, 64.0, 1023.0, 2.0),
        norm(464.0, 64.0, 1023.0, 1.0),
        norm(264.0, 64.0, 1023.0, 1.5),
    ];
    for (got, want) in v2.iter().zip([0.500_521_4, 0.417_101_1, 0.312_825_9]) {
        assert!((got - want).abs() < 5e-8, "{got}");
    }

    // F6: every level from the ImageProcessing IFD.
    let p = plan_of(&f6());
    assert_eq!((p.bits, p.white), (12, 4095), "R-ORF-33, R-ORF-35");
    assert_eq!(
        (p.black, p.black_source),
        ([256; 4], Source::ImageProcessing)
    );
    assert_eq!(
        (p.wb, p.wb_source),
        ([2.0, 1.0, 1.5], Source::ImageProcessing)
    );
    assert_eq!(p.matrix_source, Source::ImageProcessing);
    // R-ORF-16, R-ORF-40, R-ORF-41: signed values over 256, rows summing to
    // 256 stay as they are; the matrix product of the worked example.
    let m = [
        [440.0, -152.0, -32.0],
        [-70.0, 388.0, -62.0],
        [14.0, -128.0, 370.0],
    ];
    for (got, want) in p.matrix.iter().flatten().zip(m.iter().flatten()) {
        assert!((got - want / 256.0).abs() < 1e-12);
    }
    let rgb = [0, 1, 2].map(|r| (0..3).map(|k| p.matrix[r][k] * v[k]).sum::<f64>());
    for (got, want) in rgb.iter().zip([0.573_065_9, 0.419_217_2, 0.270_741_1]) {
        assert!((got - want).abs() < 5e-8, "{got}");
    }
    // A neutral value stays neutral for every normalized matrix.
    for row in p.matrix {
        assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }
    // F7: B = 14 (R-ORF-33), white 16383 (R-ORF-35), BlackLevel2 on the
    // 12-bit scale times 4 (R-ORF-36 source 1).
    let p = plan_of(&f7());
    assert_eq!((p.bits, p.white, p.black), (14, 16383, [1024; 4]));
    // F2: OLYMP levels from the main maker-note IFD (sources 3), B = 10.
    let p = plan_of(&f2());
    assert_eq!((p.bits, p.white), (10, 1023), "R-ORF-34, R-ORF-35");
    assert_eq!((p.black, p.black_source), ([64; 4], Source::Main));
    assert_eq!((p.wb, p.wb_source), ([2.0, 1.0, 1.5], Source::Main));
    assert_eq!(p.matrix_source, Source::Main);
    // F4: V3 is B = 12 whatever ValidBits says.
    let mut b = f4_builder();
    b.note.as_mut().unwrap().main = with(olymp_main(256, 12), 0x102C, Val::Short(vec![10, 0]));
    assert_eq!(plan_of(&b.build()).bits, 12, "R-ORF-33");

    // R-ORF-34: the V1 bit depth from the first present tag.
    let v1 = |ip: Option<u16>, ri: Option<u16>, main: Option<u16>| {
        let mut b = v1_builder();
        edit_sub(&mut b, 0x2040, |e| match ip {
            Some(v) => with(e, 0x0611, Val::Short(vec![v, 0])),
            None => without(e, 0x0611),
        });
        if let Some(v) = ri {
            edit_sub(&mut b, 0x3000, |e| with(e, 0x0611, Val::Short(vec![v, 0])));
        }
        if let Some(v) = main {
            let note = b.note.as_mut().unwrap();
            note.main = with(
                std::mem::take(&mut note.main),
                0x102C,
                Val::Short(vec![v, 0]),
            );
        }
        plan_of(&b.build()).bits
    };
    assert_eq!(v1(Some(14), Some(10), Some(10)), 14);
    assert_eq!(v1(None, Some(10), Some(11)), 10);
    assert_eq!(v1(None, None, Some(11)), 11);
    assert_eq!(v1(None, None, None), 12);
    assert_eq!(v1(Some(16), None, None), 16);
    // A value outside 10 to 16 gives 12 without trying the next tag.
    assert_eq!(v1(Some(9), Some(10), None), 12);
    assert_eq!(v1(Some(17), Some(10), None), 12);
    assert_eq!(v1(None, Some(8), Some(10)), 12);
    // R-ORF-22: B = 10 takes the 10 high bits.
    let mut b = v1_builder();
    b.blobs = vec![pack_v1(&[1023; 16], true, 10)];
    edit_sub(&mut b, 0x2040, |e| with(e, 0x0611, Val::Short(vec![10, 0])));
    let f = b.build();
    assert_eq!(sensor_values(&f), [1023; 16]);

    // R-ORF-36: the black level sources in order.
    let black = |ip: Option<Vec<u16>>, ri: Option<Vec<u16>>, main: Option<Vec<u16>>, bits: u16| {
        let mut b = synth(Variant::V1, 4, 4, &[0; 16]);
        edit_sub(&mut b, 0x2040, |e| {
            let e = with(e, 0x0611, Val::Short(vec![bits, 0]));
            match &ip {
                Some(v) => with(e, 0x0600, Val::Short(v.clone())),
                None => without(e, 0x0600),
            }
        });
        if let Some(v) = &ri {
            edit_sub(&mut b, 0x3000, |e| with(e, 0x0600, Val::Short(v.clone())));
        }
        if let Some(v) = &main {
            let note = b.note.as_mut().unwrap();
            note.main = with(
                std::mem::take(&mut note.main),
                0x1012,
                Val::Short(v.clone()),
            );
        }
        let f = b.build();
        let c = container(&f);
        meta::plan(&c, Variant::V1).map(|p| (p.black, p.black_source))
    };
    let four = |v: u16| Some(vec![v; 4]);
    assert_eq!(
        black(four(200), four(10), four(20), 12).unwrap(),
        ([200; 4], Source::ImageProcessing)
    );
    // Source 1 is on the 12-bit scale: x 4 for B = 14, x 16 for B = 16.
    assert_eq!(black(four(255), None, None, 14).unwrap().0, [1020; 4]);
    assert_eq!(black(four(255), None, None, 16).unwrap().0, [4080; 4]);
    // Source 1 applies only when B >= 12.
    assert_eq!(
        black(four(200), four(10), four(20), 10).unwrap(),
        ([10; 4], Source::RawInfo)
    );
    assert_eq!(
        black(four(200), None, four(20), 11).unwrap(),
        ([20; 4], Source::Main)
    );
    // Exactly 4 values, each at most w / 4 (1023 for B = 12).
    assert_eq!(
        black(Some(vec![200; 3]), four(10), None, 12).unwrap().1,
        Source::RawInfo
    );
    assert_eq!(black(four(1023), None, None, 12).unwrap().0, [1023; 4]);
    assert_eq!(
        black(four(1024), four(30), None, 12).unwrap().1,
        Source::RawInfo
    );
    assert_eq!(
        black(None, Some(vec![1, 2, 3, 4]), None, 12).unwrap().0,
        [1, 2, 3, 4]
    );
    // No source qualifies: the preview path (OQ-17).
    assert!(black(None, None, None, 12).is_err());
    assert!(black(four(1024), Some(vec![1; 5]), four(2000), 12).is_err());

    // R-ORF-38: the white balance sources in order.
    let wb = |ip: Option<Vec<u16>>,
              gain: Option<u16>,
              ri: Option<Vec<u16>>,
              main: Option<(u16, u16)>| {
        let mut b = synth(Variant::V1, 4, 4, &[0; 16]);
        edit_sub(&mut b, 0x2040, |e| {
            let e = match &ip {
                Some(v) => with(e, 0x0100, Val::Short(v.clone())),
                None => without(e, 0x0100),
            };
            match gain {
                Some(g) => with(e, 0x0610, Val::Short(vec![g])),
                None => without(e, 0x0610),
            }
        });
        if let Some(v) = &ri {
            edit_sub(&mut b, 0x3000, |e| with(e, 0x0100, Val::Short(v.clone())));
        }
        if let Some((r, bl)) = main {
            let note = b.note.as_mut().unwrap();
            let m = with(
                std::mem::take(&mut note.main),
                0x1017,
                Val::Short(vec![r, 64]),
            );
            note.main = with(m, 0x1018, Val::Short(vec![bl, 64]));
        }
        let p = plan_of(&b.build());
        (p.wb, p.wb_source)
    };
    let ip = |r: u16, b: u16| Some(vec![r, b, 256, 256]);
    assert_eq!(
        wb(ip(512, 384), Some(256), None, None),
        ([2.0, 1.0, 1.5], Source::ImageProcessing)
    );
    // The E-M10 Mark II example.
    assert_eq!(
        wb(ip(512, 450), Some(256), None, None).0,
        [2.0, 1.0, 1.757_812_5]
    );
    // Only values 1 and 2 count; 2 values are enough.
    assert_eq!(
        wb(Some(vec![512, 384]), None, None, None).0,
        [2.0, 1.0, 1.5]
    );
    // GainBase from 64 to 4096, else 256 (R-ORF-63).
    assert_eq!(wb(ip(512, 384), Some(512), None, None).0, [1.0, 1.0, 0.75]);
    assert_eq!(wb(ip(512, 384), Some(64), None, None).0, [8.0, 1.0, 6.0]);
    assert_eq!(wb(ip(512, 384), Some(63), None, None).0, [2.0, 1.0, 1.5]);
    assert_eq!(wb(ip(512, 384), Some(4097), None, None).0, [2.0, 1.0, 1.5]);
    assert_eq!(wb(ip(512, 384), Some(0), None, None).0, [2.0, 1.0, 1.5]);
    // Multipliers from 0.0625 to 16, else the next source.
    assert_eq!(wb(ip(16, 4096), None, None, None).0, [0.0625, 1.0, 16.0]);
    assert_eq!(
        wb(ip(15, 384), None, Some(vec![768, 256]), None),
        ([3.0, 1.0, 1.0], Source::RawInfo)
    );
    assert_eq!(
        wb(ip(512, 4097), None, None, Some((640, 320))),
        ([2.5, 1.0, 1.25], Source::Main)
    );
    assert_eq!(
        wb(Some(vec![512]), None, None, Some((640, 320))).1,
        Source::Main
    );
    assert_eq!(
        wb(None, None, None, Some((640, 0))),
        ([1.944, 1.0, 1.801], Source::Generic)
    );
    assert_eq!(
        wb(None, None, None, None),
        ([1.944, 1.0, 1.801], Source::Generic)
    );
    // Source 3 needs both tags.
    let mut b = synth(Variant::V1, 4, 4, &[0; 16]);
    edit_sub(&mut b, 0x2040, |e| without(e, 0x0100));
    b.note.as_mut().unwrap().main = vec![(0x1017, Val::Short(vec![512, 64]))];
    assert_eq!(plan_of(&b.build()).wb_source, Source::Generic);

    // R-ORF-40, R-ORF-41: the matrix sources in order.
    let matrix = |ip: Option<Val>, ri: Option<Val>, main: Option<Val>| {
        let mut b = synth(Variant::V1, 4, 4, &[0; 16]);
        edit_sub(&mut b, 0x2040, |e| match &ip {
            Some(v) => with(e, 0x0200, v.clone()),
            None => without(e, 0x0200),
        });
        if let Some(v) = &ri {
            edit_sub(&mut b, 0x3000, |e| with(e, 0x0200, v.clone()));
        }
        if let Some(v) = &main {
            b.note.as_mut().unwrap().main = vec![(0x1011, v.clone())];
        }
        let p = plan_of(&b.build());
        (p.matrix, p.matrix_source)
    };
    let short = |v: [i16; 9]| Val::Short(v.iter().map(|&x| x as u16).collect());
    let identity = short([256, 0, 0, 0, 256, 0, 0, 0, 256]);
    let unit = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    assert_eq!(
        matrix(Some(identity.clone()), None, None),
        (unit, Source::ImageProcessing)
    );
    // Not 9 values: the next tag.
    let eight = Val::Short(vec![256; 8]);
    assert_eq!(
        matrix(Some(eight.clone()), Some(identity.clone()), None).1,
        Source::RawInfo
    );
    assert_eq!(
        matrix(Some(eight.clone()), None, Some(identity.clone())).1,
        Source::Main
    );
    let (gm, gs) = matrix(Some(eight), None, None);
    assert_eq!(gs, Source::Generic);
    for (row, want) in gm.iter().zip(meta::GENERIC_MATRIX) {
        for (a, b) in row.iter().zip(want) {
            assert!((a - b).abs() < 1e-9);
        }
        assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    }
    // R-ORF-7: SSHORT (type 8) is read as signed too.
    let mut ss = Vec::new();
    for v in [440i16, -152, -32, -70, 388, -62, 14, -128, 370] {
        ss.extend(v.to_le_bytes());
    }
    let (m8, s8) = matrix(Some(Val::Raw(8, 9, ss)), None, None);
    assert_eq!(s8, Source::ImageProcessing);
    assert!((m8[0][1] + 152.0 / 256.0).abs() < 1e-12);
    // Each row divided by its sum (E-1: 250, 246, 248).
    let (m, _) = matrix(
        Some(short([300, -30, -20, -10, 280, -24, 0, -40, 288])),
        None,
        None,
    );
    assert!((m[0][0] - 300.0 / 250.0).abs() < 1e-12);
    assert!((m[1][1] - 280.0 / 246.0).abs() < 1e-12);
    assert!((m[2][2] - 288.0 / 248.0).abs() < 1e-12);
    // A row sum of 0 or less: the generic matrix (R-ORF-63).
    let zero_row = short([256, 0, 0, 100, -100, 0, 0, 0, 256]);
    assert_eq!(
        matrix(Some(zero_row), Some(identity.clone()), None).1,
        Source::Generic
    );
    let negative = short([256, 0, 0, 0, 256, 0, 0, -300, 256]);
    assert_eq!(matrix(Some(negative), None, None).1, Source::Generic);
    // A LONG matrix is not a signed SHORT matrix: the next tag.
    assert_eq!(
        matrix(Some(Val::Long(vec![256; 9])), Some(identity), None).1,
        Source::RawInfo
    );

    // R-ORF-17: the crop sources and their conditions.
    let crop = |ip: Option<[u32; 4]>, ri: Option<[u32; 4]>| {
        let mut b = synth(Variant::V1, 10, 8, &[0; 80]);
        edit_sub(&mut b, 0x2040, |e| {
            let mut e = e;
            for t in [0x0612, 0x0613, 0x0614, 0x0615] {
                e = without(e, t);
            }
            if let Some([x, y, w, h]) = ip {
                e = with(e, 0x0612, Val::Short(vec![x as u16, 0]));
                e = with(e, 0x0613, Val::Short(vec![y as u16, 0]));
                e = with(e, 0x0614, Val::Long(vec![w]));
                e = with(e, 0x0615, Val::Long(vec![h]));
            }
            e
        });
        if let Some([x, y, w, h]) = ri {
            edit_sub(&mut b, 0x3000, |e| {
                let mut e = with(e, 0x0612, Val::Short(vec![x as u16]));
                e = with(e, 0x0613, Val::Short(vec![y as u16]));
                e = with(e, 0x0614, Val::Long(vec![w]));
                with(e, 0x0615, Val::Long(vec![h]))
            });
        }
        let c = plan_of(&b.build()).crop;
        [c.x, c.y, c.w, c.h]
    };
    assert_eq!(crop(Some([1, 2, 8, 6]), Some([0, 0, 4, 4])), [1, 2, 8, 6]);
    assert_eq!(crop(Some([2, 2, 8, 6]), None), [2, 2, 8, 6], "CX + CW = W");
    assert_eq!(
        crop(Some([3, 2, 8, 6]), Some([0, 1, 4, 4])),
        [0, 1, 4, 4],
        "CX + CW > W"
    );
    assert_eq!(
        crop(Some([0, 3, 8, 6]), Some([0, 1, 4, 4])),
        [0, 1, 4, 4],
        "CY + CH > H"
    );
    assert_eq!(crop(Some([0, 0, 1, 6]), None), [0, 0, 10, 8], "CW < 2");
    assert_eq!(crop(Some([0, 0, 6, 1]), None), [0, 0, 10, 8], "CH < 2");
    assert_eq!(crop(None, Some([6, 0, 4, 8])), [6, 0, 4, 8]);
    assert_eq!(crop(None, Some([6, 0, 5, 8])), [0, 0, 10, 8]);
    assert_eq!(crop(None, None), [0, 0, 10, 8]);
    // Three of the four tags are not a crop.
    let mut b = synth(Variant::V1, 10, 8, &[0; 80]);
    edit_sub(&mut b, 0x2040, |e| {
        let e = with(e, 0x0612, Val::Short(vec![2, 0]));
        let e = with(e, 0x0614, Val::Long(vec![6]));
        without(e, 0x0615)
    });
    assert_eq!(plan_of(&b.build()).crop.w, 10);
    // R-ORF-18: AspectFrame does not change the crop.
    let mut b = synth(Variant::V1, 10, 8, &[0; 80]);
    edit_sub(&mut b, 0x2040, |e| {
        let e = with(e, 0x0612, Val::Short(vec![1, 0]));
        let e = with(e, 0x0613, Val::Short(vec![1, 0]));
        let e = with(e, 0x0614, Val::Long(vec![8]));
        let e = with(e, 0x0615, Val::Long(vec![6]));
        let e = with(e, 0x1112, Val::Byte(vec![2, 0]));
        with(e, 0x1113, Val::Short(vec![0, 1, 7, 4]))
    });
    let c = plan_of(&b.build()).crop;
    assert_eq!([c.x, c.y, c.w, c.h], [1, 1, 8, 6]);

    // R-ORF-39, R-ORF-62: values below the black level clip to 0 and
    // white-balanced values above 1 clip to 1, before the matrix.
    let cell = cell_values(4, 4, [4095, 100, 4095], [0, 1, 1, 2]);
    let f = synth(Variant::V1, 4, 4, &cell).build();
    let (_, _, px) = linear(&f);
    let p = plan_of(&f);
    let cam = [1.0, 0.0, 1.0];
    let want = [0, 1, 2].map(|r| {
        (0..3)
            .map(|k| p.matrix[r][k] * cam[k])
            .sum::<f64>()
            .clamp(0.0, 1.0) as f32
    });
    for v in px {
        assert_rgb(v, want, EPS, "clipped");
        assert!(v.iter().all(|c| (0.0..=1.0).contains(c)), "R-ORF-42 clip");
    }
}

/// Add a CameraSettings preview (`full`, R-ORF-48 first), a maker-note
/// thumbnail (`thumb`, second) to a builder's maker note.
fn with_previews(mut b: Builder, full: Vec<u8>, thumb: Vec<u8>) -> Builder {
    let k = b.blobs.len();
    b.blobs.push(full);
    edit_sub(&mut b, 0x2020, |_| {
        vec![
            (0x0100, Val::Long(vec![1])),
            (0x0101, Val::Blobs(vec![k])),
            (0x0102, Val::BlobLens(vec![k])),
        ]
    });
    let note = b.note.as_mut().unwrap();
    note.main = with(std::mem::take(&mut note.main), 0x0100, Val::Undef(thumb));
    b
}

/// An IFD1 JPEG (`jpeg`, the IFD chain of R-ORF-48).
fn with_ifd1(mut b: Builder, jpeg: Vec<u8>) -> Builder {
    let k = b.blobs.len();
    b.blobs.push(jpeg);
    b.ifd1 = vec![
        (0x0201, Val::Blobs(vec![k])),
        (0x0202, Val::BlobLens(vec![k])),
    ];
    b
}

fn assert_preview(dev: &Developed, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_eq!(dev.origin, Origin::Preview, "{what}");
    assert_eq!(dev.image.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((dev.image.width(), dev.image.height()), size, "{what}");
    let p = &dev.image.data()[..3];
    for c in 0..3 {
        assert!(
            p[c].abs_diff(rgb[c]) <= 12,
            "{what}: pixel {p:?} want {rgb:?}"
        );
    }
}

/// A JPEG whose top half is `top` and bottom half `bottom`.
fn jpeg_split(w: u16, h: u16, top: [u8; 3], bottom: [u8; 3]) -> Vec<u8> {
    let px: Vec<u8> = (0..usize::from(w) * usize::from(h))
        .flat_map(|i| {
            let c = if i / usize::from(w) < usize::from(h) / 2 {
                top
            } else {
                bottom
            };
            [c[0], c[1], c[2], 255]
        })
        .collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 95)
        .encode(&px, w, h, jpeg_encoder::ColorType::Rgba)
        .expect("encode preview");
    out
}

fn pixel(img: &ImageBuf, x: usize, y: usize) -> [u8; 3] {
    let i = (y * img.width() as usize + x) * 4;
    let d = img.data();
    [d[i], d[i + 1], d[i + 2]]
}

fn close(a: [u8; 3], b: [u8; 3]) -> bool {
    (0..3).all(|c| a[c].abs_diff(b[c]) <= 24)
}

#[test]
fn orf_out_of_scope_falls_back_to_preview() {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let opts = DecodeOptions::default();
    // X1: a V5 file with another stream header.
    let mut b = with_previews(f6_builder(), big.clone(), small.clone());
    b.blobs[0][4] = 2;
    let f = b.build();
    assert_eq!(class_of(&f), Class::OutOfScope(Relative::X1));
    let dev = decode_report(&f, &opts).unwrap().unwrap();
    assert_preview(&dev, (48, 32), [200, 40, 40], "X1");
    assert_eq!(dev.class, Some(Class::OutOfScope(Relative::X1)));
    // R-ORF-2: the X1 sensor data are never interpreted, even if a plan
    // could be made for them.
    assert!(dev.info.is_none());
    // X2: a big-endian file of another layout (F2 with 15 strip bytes).
    let mut b = f2_builder();
    b.blobs[0].pop();
    let f = with_ifd1(
        with_previews(b, big.clone(), small.clone()),
        jpeg(8, 8, [9; 3]),
    )
    .build();
    assert_eq!(class_of(&f), Class::OutOfScope(Relative::X2));
    assert_preview(
        &decode_report(&f, &opts).unwrap().unwrap(),
        (48, 32),
        [200, 40, 40],
        "X2",
    );
    assert_eq!(probe(&f).unwrap().unwrap().0, 48);
    // OQ-14, X3: an ORI-like file is classified by its content alone.
    let f = with_previews(f6_builder(), big.clone(), small.clone()).build();
    assert!(is_orf(&f, Some("ori")) && is_orf(&f, None));
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V5));

    // R-ORF-48 to R-ORF-50: the candidates of the three locations, largest
    // first; equal sizes keep the order CameraSettings, thumbnail, IFD1.
    let f = with_ifd1(
        with_previews(f6_builder(), big.clone(), small.clone()),
        jpeg(32, 16, [9; 3]),
    )
    .build();
    let c = container(&f);
    let cands = preview::collect(&c);
    let sizes: Vec<(u32, u32)> = cands.iter().map(|p| (p.width, p.height)).collect();
    assert_eq!(sizes, [(48, 32), (32, 16), (16, 8)]);
    let cs = jpeg(16, 8, [200, 40, 40]);
    let th = jpeg(16, 8, [40, 200, 40]);
    let i1 = jpeg(16, 8, [40, 40, 200]);
    let first = |a: &[u8], b2: &[u8], c2: &[u8]| {
        let f = with_ifd1(
            with_previews(f6_builder(), a.to_vec(), b2.to_vec()),
            c2.to_vec(),
        )
        .build();
        let order = preview::collect(&container(&f));
        f[order[0].offset..order[0].offset + order[0].len].to_vec()
    };
    assert_eq!(first(&cs, &th, &i1), cs, "CameraSettings first");
    assert_eq!(first(&jpeg(8, 8, [1; 3]), &th, &i1), th, "thumbnail next");
    assert_eq!(
        first(&jpeg(8, 8, [1; 3]), &jpeg(8, 4, [1; 3]), &i1),
        i1,
        "IFD1 last"
    );
    // The OLYMP style: CameraSettings offsets count from the file start.
    let mut b = with_previews(f2_builder(), big.clone(), small.clone());
    b.blobs[0].pop();
    let f = b.build();
    let c = container(&f);
    assert_eq!(c.note.as_ref().unwrap().style, Style::Olymp);
    assert_eq!(preview::collect(&c).len(), 2);
    // R-ORF-49: a candidate outside the file, shorter than 4 bytes or not
    // starting with FF D8 is not kept; R-ORF-50: one without SOF0 to SOF2
    // is dropped. PreviewImageValid is not read.
    let no_sof: Vec<u8> = vec![0xFF, 0xD8, 0xFF, 0xD9, 0, 0];
    for (full, want) in [
        (vec![0xFF, 0xD8, 0xFF], 1usize),
        (vec![0x00; 64], 1),
        (no_sof, 1),
        (big.clone(), 2),
    ] {
        let mut b = with_previews(f6_builder(), full, small.clone());
        edit_sub(&mut b, 0x2020, |e| with(e, 0x0100, Val::Long(vec![0])));
        assert_eq!(preview::collect(&container(&b.build())).len(), want);
    }
    let mut b = with_previews(f6_builder(), big.clone(), small.clone());
    edit_sub(&mut b, 0x2020, |e| {
        with(e, 0x0102, Val::Long(vec![1 << 20]))
    });
    assert_eq!(preview::collect(&container(&b.build())).len(), 1, "outside");

    // R-ORF-52: the IFD0 orientation is applied to every preview, also
    // values 5 to 8 to a preview taller than wide (unlike R-ARW-62).
    let tall = jpeg_split(16, 32, [220, 30, 30], [30, 30, 220]);
    for o in [1u16, 3, 6, 8] {
        let mut b = with_previews(f6_builder(), tall.clone(), small.clone());
        b.ifd0 = with(b.ifd0, 0x0112, Val::Short(vec![o]));
        b.blobs[0][4] = 2;
        let f = b.build();
        let dev = decode_report(&f, &opts).unwrap().unwrap();
        let (w, h) = (dev.image.width() as usize, dev.image.height() as usize);
        let want = if o >= 5 { (32, 16) } else { (16, 32) };
        assert_eq!((w, h), want, "orientation {o}");
        assert_eq!(probe(&f).unwrap().unwrap().0 as usize, want.0);
        let red = [220, 30, 30];
        let (at_red, at_blue) = match o {
            1 => ((8, 2), (8, 29)),
            3 => ((8, 29), (8, 2)),
            6 => ((29, 8), (2, 8)),
            _ => ((2, 8), (29, 8)),
        };
        assert!(
            close(pixel(&dev.image, at_red.0, at_red.1), red),
            "orientation {o}"
        );
        assert!(
            !close(pixel(&dev.image, at_blue.0, at_blue.1), red),
            "orientation {o}"
        );
    }

    // R-ORF-53: a candidate that fails to decode gives way to the next; an
    // error only when none decodes.
    let header_only = {
        let sos = big.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        big[..sos].to_vec()
    };
    let mut b = with_previews(f6_builder(), header_only.clone(), small.clone());
    b.blobs[0][4] = 2;
    let dev = decode_report(&b.build(), &opts).unwrap().unwrap();
    assert_preview(&dev, (16, 8), [40, 40, 200], "next candidate");
    let mut b = with_previews(f6_builder(), header_only.clone(), header_only.clone());
    b.blobs[0][4] = 2;
    assert!(
        decode_report(&b.build(), &opts).is_err(),
        "no candidate decodes"
    );
    // R-ORF-51: bytes after EOI, or no EOI after the last scan.
    for full in [
        [big.clone(), vec![0x11; 40]].concat(),
        big[..big.len() - 2].to_vec(),
    ] {
        let mut b = with_previews(f6_builder(), full, small.clone());
        b.blobs[0][4] = 2;
        assert_preview(
            &decode_report(&b.build(), &opts).unwrap().unwrap(),
            (48, 32),
            [200, 40, 40],
            "R-ORF-51",
        );
    }

    // Every failure of the raw path falls back to the preview (R-ORF-53).
    let fallback = |f: Vec<u8>, what: &str| {
        let dev = decode_report(&f, &opts).unwrap().unwrap();
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
    };
    let base = || with_previews(f6_builder(), big.clone(), small.clone());
    // R-ORF-11: no maker note, another header, a header cut short: the
    // previews of the IFD chain stay (here IFD1).
    let mut b = with_ifd1(f6_builder(), big.clone());
    b.note = None;
    let f = b.build();
    assert!(container(&f).note.is_none());
    fallback(f, "no maker note");
    let note_at = container(&with_ifd1(f6_builder(), big.clone()).build())
        .note
        .unwrap()
        .start as usize;
    for (at, v) in [(0usize, b'X'), (7, b'X'), (8, b'X'), (9, b'M')] {
        let mut f = with_ifd1(f6_builder(), big.clone()).build();
        f[note_at + at] = v;
        assert!(container(&f).note.is_none(), "header byte {at}");
        fallback(f, "maker note header");
    }
    let mut f = with_ifd1(f6_builder(), big.clone()).build();
    // The MakerNote count (Exif entry 0x927C) becomes 11: shorter than the
    // OLYMPUS header.
    let exif_at = container(&f).ifd0().uint(0x8769).unwrap().unwrap() as usize;
    f[exif_at + 2 + 4..exif_at + 2 + 8].copy_from_slice(&11u32.to_le_bytes());
    assert!(container(&f).note.is_none());
    fallback(f, "short maker note");
    // R-ORF-14: a required tag missing, a required offset outside the file.
    for tag in [0x0100u16, 0x0101, 0x0102, 0x0111, 0x0117] {
        let mut b = base();
        b.ifd0 = without(b.ifd0, tag);
        fallback(b.build(), &format!("IFD0 tag {tag:04X}"));
    }
    let mut b = base();
    b.exif = Vec::new();
    fallback(b.build(), "no CFAPattern");
    let mut b = base();
    b.ifd0 = with(b.ifd0, 0x0111, Val::Long(vec![1 << 30]));
    fallback(b.build(), "strip outside the file");
    let mut b = base();
    b.ifd0 = with(b.ifd0, 0x0117, Val::Long(vec![1 << 30]));
    fallback(b.build(), "strip count outside the file");
    // R-ORF-15: not a Bayer pattern.
    let mut b = base();
    b.exif = vec![cfa([0, 0, 1, 2], true)];
    fallback(b.build(), "CFA 0 0 1 2");
    // R-ORF-36: no black level.
    let mut b = base();
    edit_sub(&mut b, 0x2040, |e| without(e, 0x0600));
    fallback(b.build(), "no black level");
    // R-ORF-32: a corrupt stream (a value outside the range) is dropped
    // (R-ORF-65) and the preview delivered.
    let mut b = base();
    b.blobs[0][7] = 0x90;
    fallback(b.build(), "corrupt stream");
    // R-ORF-57: a tag count far beyond the file size fails before any
    // allocation.
    let mut b = base();
    edit_sub(&mut b, 0x2040, |e| {
        with(e, 0x0600, Val::Raw(3, 0x4000_0000, vec![0; 8]))
    });
    let f = b.build();
    assert!(container(&f)
        .note
        .unwrap()
        .image_processing
        .unwrap()
        .uints(0x0600)
        .is_err());
    fallback(f, "huge count");

    // A target size uses the smallest preview that covers it; a larger
    // target develops the sensor data.
    let values = random_values(64 * 48, 4096, 11);
    let f = with_previews(
        synth(Variant::V5, 64, 48, &values),
        jpeg(32, 24, [200, 40, 40]),
        small.clone(),
    )
    .build();
    let t = |w, h| DecodeOptions {
        target_size: Some((w, h)),
        ..DecodeOptions::default()
    };
    assert_preview(
        &decode_report(&f, &t(10, 5)).unwrap().unwrap(),
        (16, 8),
        [40, 40, 200],
        "small target",
    );
    assert_preview(
        &decode_report(&f, &t(20, 20)).unwrap().unwrap(),
        (32, 24),
        [200, 40, 40],
        "medium target",
    );
    let dev = decode_report(&f, &t(500, 500)).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V5));

    // Through the registry: by extension and by signature, the developed
    // image at full size, a thumbnail from the preview.
    let reg = crate::default_registry();
    for name in ["P1010001.ORF", "noext"] {
        let src = pandite_core::Source::from_bytes(f.clone(), Some(name.into()));
        let meta = reg.probe(&src).unwrap();
        assert_eq!((meta.width, meta.height), (64, 48), "{name}");
        let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
        assert_eq!(
            (img.first_image().width(), img.first_image().height()),
            (64, 48)
        );
        let img = reg.decode(&src, &t(16, 16)).unwrap();
        assert_eq!(img.first_image().width(), 16, "{name}");
    }
    assert!(has_preview(&f));
    assert!(!has_preview(&f6()));
    // F6 has no preview: a failure of its raw path is an error (R-ORF-53).
    let mut f = f6();
    f[0x1A7] = 0x90;
    assert!(decode_report(&f, &opts).is_err());
    // Detection: the ORF starts or the extension, not a plain TIFF.
    assert!(is_orf(&f6(), None) && is_orf(&f2(), None) && is_orf(&f4(), None));
    assert!(is_orf(b"II*\0", Some("orf")));
    assert!(!is_orf(b"II*\0\x08\0\0\0", None));
    assert!(!is_orf(b"II*\0\x08\0\0\0", Some("tif")));
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let values = random_values(24 * 12, 4096, 21);
    let mut x2 = with_previews(f2_builder(), big.clone(), small.clone());
    x2.blobs[0].pop();
    let mut v5 = with_previews(
        synth(Variant::V5, 24, 12, &values),
        big.clone(),
        small.clone(),
    );
    v5.ifd0 = with(v5.ifd0, 0x0112, Val::Short(vec![6]));
    let mut v3 = with_previews(
        synth(Variant::V3, 24, 12, &values),
        big.clone(),
        small.clone(),
    );
    v3.note.as_mut().unwrap().style = Style::Olymp;
    let mut om = with_previews(
        synth(Variant::V6, 24, 12, &values),
        big.clone(),
        small.clone(),
    );
    om.note.as_mut().unwrap().style = Style::OmSystem;
    om.note.as_mut().unwrap().le = false;
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F5", f5()),
        ("F6", f6()),
        ("F7", f7()),
        ("V5+previews", v5.build()),
        ("V3 OLYMP+previews", v3.build()),
        ("V6 OM SYSTEM BE note+previews", om.build()),
        ("X2+previews", x2.build()),
        (
            "V4+previews",
            with_previews(synth(Variant::V4, 24, 12, &values), big, small).build(),
        ),
    ]
}

#[test]
fn orf_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut next = noise(0x9E37_79B9);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        let le = f[0] == 0x49;
        let strip = plan_of_any(&f);
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 5 {
                // Damage the directories and the maker note.
                0 | 1 => {
                    let limit = bad.len().min(if round % 2 == 0 { 600 } else { 2000 });
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
                // The sensor data: stream bits, packed samples.
                3 => {
                    if let Some((a, b)) = strip {
                        for _ in 0..1 + next() % 4 {
                            let i = a + next() as usize % (b - a).max(1);
                            if i < bad.len() {
                                bad[i] ^= 1 << (next() % 8);
                            }
                        }
                    }
                }
                // Huge values in IFD fields, in the file's byte order.
                _ => {
                    let i = (next() as usize % bad.len().min(600)) & !1;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27][next() as usize % 5];
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
                let small = DecodeOptions {
                    target_size: Some((8, 8)),
                    ..DecodeOptions::default()
                };
                for opts in [DecodeOptions::default(), small] {
                    match decode_report(&bad, &opts) {
                        Ok(Some(dev)) => {
                            // R-ORF-65: a sensor image is complete.
                            let (w, h) = (dev.image.width(), dev.image.height());
                            assert!(w > 0 && h > 0);
                            if let Some(info) = &dev.info {
                                let want = render::output_size(info.crop, info.orientation);
                                assert_eq!((w as usize, h as usize), want);
                            }
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let msg = e.to_string();
                            assert!(!msg.contains("panicked"), "{name} round {round}: {msg}");
                        }
                    }
                }
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.orf".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
                let _ = reg.facts(&src);
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // R-ORF-56, R-ORF-57: oversized claims are refused before allocation.
    for (w, h) in [
        (65535u32, 65535u32),
        (0, 2),
        (4, 0),
        (70000, 2),
        (16384, 16384),
    ] {
        let mut f = f6();
        f[0x12..0x16].copy_from_slice(&w.to_le_bytes());
        f[0x1E..0x22].copy_from_slice(&h.to_le_bytes());
        let c = container(&f);
        for v in [Variant::V1, Variant::V5] {
            assert!(meta::plan(&c, v).is_err(), "{w} x {h}");
        }
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    }
    // A V5 raster that the strip cannot hold (6 bits per pixel at least) is
    // refused before its plane is allocated.
    let mut f = f6();
    f[0x12..0x16].copy_from_slice(&8000u32.to_le_bytes());
    f[0x1E..0x22].copy_from_slice(&8000u32.to_le_bytes());
    assert!(meta::plan(&container(&f), Variant::V5).is_err());
    // A count far beyond the file size fails before allocation.
    let mut f = f6();
    // The count of StripOffsets (IFD0 entry 7 at 0x5E).
    f[0x62..0x66].copy_from_slice(&0x4000_0000u32.to_le_bytes());
    assert!(container(&f).ifd0().uints(0x0111).is_err());
    assert!(meta::classify(&container(&f)).is_err());
    // R-ARW-12: a next-IFD loop stops.
    let mut f = f6();
    f[0xA6..0xAA].copy_from_slice(&8u32.to_le_bytes());
    assert_eq!(container(&f).chain.len(), 1);
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V5));
    // A sub-IFD pointer back to the main IFD and an Exif pointer to IFD0:
    // read once each, no recursion.
    let mut f = f6();
    f[0xEE..0xF2].copy_from_slice(&12u32.to_le_bytes());
    let _ = decode_report(&f, &DecodeOptions::default());
    let mut f = f6();
    f[0xA2..0xA6].copy_from_slice(&8u32.to_le_bytes());
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
}

/// The byte range of the sensor data of an in-scope file.
fn plan_of_any(f: &[u8]) -> Option<(usize, usize)> {
    let c = meta::open(f).ok()?;
    let Ok(Class::Sensor(v)) = meta::classify(&c) else {
        return None;
    };
    let p = meta::plan(&c, v).ok()?;
    Some(match &p.layout {
        Layout::Stream { start, end } => (*start, *end),
        Layout::Rows { starts, row_bytes } => {
            (*starts.iter().min()?, starts.iter().max()? + row_bytes)
        }
    })
}

#[test]
fn orf_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        // The end of the sensor data of an in-scope file: a cut after it
        // (inside the previews that follow) still holds every sensor byte.
        let strip_end = plan_of_any(&f).map_or(usize::MAX, |(_, b)| b);
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 7, 8, 9, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) if dev.origin == Origin::Preview => {}
                Ok(Ok(Some(dev))) => {
                    // R-ORF-14, R-ORF-65: sensor data only when every strip
                    // lies inside the cut file.
                    assert!(cut >= strip_end, "{name} cut at {cut}: {:?}", dev.origin);
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
    // The strip is last: a file cut inside it fails its checks before any
    // sensor value is read.
    for f in [f1(), f6(), f7()] {
        let c = container(&f[..f.len() - 1]);
        assert!(meta::classify(&c).is_err() || meta::plan(&c, Variant::V5).is_err());
    }
}

/// A large V5 file: 2048 x 1536 smooth values.
fn large_v5() -> Vec<u8> {
    let (w, h) = (2048usize, 1536usize);
    let values: Vec<u16> = (0..w * h)
        .map(|i| (300 + (i % w) + (i / w) / 2 + 37 * (i % 3)) as u16)
        .collect();
    synth(Variant::V5, w as u32, h as u32, &values).build()
}

#[test]
fn orf_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    for f in [f6(), f1(), f4()] {
        assert!(matches!(
            decode_report(&f, &cancelled),
            Err(Error::Cancelled)
        ));
    }
    let src = pandite_core::Source::from_bytes(f6(), Some("a.orf".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread.
    let f = large_v5();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V5));
    assert_eq!((dev.image.width(), dev.image.height()), (2048, 1536));
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

/// Header facts of the fixtures through the module and the registry.
#[test]
fn facts_orf() {
    use pandite_core::Source;
    let f = super::facts(&f6()).expect("ORF facts");
    assert_eq!(f.container.as_deref(), Some("Olympus ORF"));
    assert_eq!(f.compression.as_deref(), Some("Olympus lossless"));
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(12)));
    assert_eq!(f.camera.make.as_deref(), Some("OLYMPUS"));
    assert_eq!(f.camera.model.as_deref(), Some("TESTCAM"));
    let facts_of = |d: &[u8]| {
        let f = super::facts(d).unwrap();
        (f.compression, f.bits_per_channel)
    };
    assert_eq!(facts_of(&f1()), (Some("Uncompressed".into()), Some(12)));
    assert_eq!(facts_of(&f2()), (Some("Uncompressed".into()), Some(10)));
    assert_eq!(facts_of(&f3()).0.as_deref(), Some("Olympus 12-bit packed"));
    assert_eq!(
        facts_of(&f4()).0.as_deref(),
        Some("Olympus 12-bit packed, two fields")
    );
    assert_eq!(
        facts_of(&f5()).0.as_deref(),
        Some("Olympus 12-bit packed in 32-bit words")
    );
    assert_eq!(facts_of(&f7()), (Some("Olympus lossless".into()), Some(14)));
    assert_eq!(
        super::facts(&f2()).unwrap().camera.make.as_deref(),
        Some("OLYMPUS")
    );
    let reg = crate::default_registry();
    let via = reg
        .facts(&Source::from_bytes(f6(), Some("P1010001.ORF".into())))
        .expect("registry facts");
    assert_eq!(via.container.as_deref(), Some("Olympus ORF"));
    assert_eq!(via.channels.as_deref(), Some("Bayer RGGB"));
    // The head of the file: the directories come before the strip.
    let head = super::facts(&f1()[..0x1A0]).expect("head facts");
    assert_eq!(head.compression.as_deref(), Some("Uncompressed"));
    for n in 0..f6().len() {
        let _ = super::facts(&f6()[..n]);
    }
    assert!(super::facts(b"II*\0\x08\0\0\0").is_none());
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// Expected facts of the specification's measured files (section Sensor
/// data layout, M12, M13, M19): file name start, variant, raster, crop x,
/// y, w, h, unused bits of a compressed strip, preview size.
type Expected = (
    &'static str,
    Variant,
    (usize, usize),
    [usize; 4],
    Option<u64>,
    (u32, u32),
);

/// The part of the developed image that the largest embedded JPEG shows,
/// as a centered fraction of each side, for files where it is not the
/// whole image. E-10: the specification gives no crop (R-ORF-17: the whole
/// raster) and reports a log luminance correlation of 0.48 with the
/// 160x120 thumbnail (M18). The thumbnail shows the central 85 percent of
/// the raster: its luminance correlates 0.976 with that area and 0.421
/// with the whole raster (measured on this sample by a search over
/// centered and shifted areas). The decoder still delivers the whole
/// raster; only the comparison uses that area.
const PREVIEW_AREA: &[(&str, f64)] = &[("E-10_", 0.85)];

/// The centered part of `img` with `frac` of its width and height.
fn central(img: &ImageBuf, frac: f64) -> ImageBuf {
    let (iw, ih) = (img.width(), img.height());
    let (cw, ch) = (
        (f64::from(iw) * frac).round() as u32,
        (f64::from(ih) * frac).round() as u32,
    );
    let px = img.format().bytes_per_pixel();
    let (x0, y0) = ((iw - cw) / 2, (ih - ch) / 2);
    let mut data = Vec::with_capacity(cw as usize * ch as usize * px);
    for y in y0..y0 + ch {
        let at = (y as usize * iw as usize + x0 as usize) * px;
        data.extend_from_slice(&img.data()[at..at + cw as usize * px]);
    }
    ImageBuf::new(cw, ch, img.format(), data).unwrap()
}

const MEASURED: &[Expected] = &[
    (
        "E-10_",
        Variant::V1,
        (2256, 1684),
        [0, 0, 2256, 1684],
        None,
        (160, 120),
    ),
    (
        "E-1_",
        Variant::V1,
        (2624, 1966),
        [36, 34, 2560, 1920],
        None,
        (1280, 960),
    ),
    (
        "E-500_",
        Variant::V2,
        (3360, 2504),
        [23, 45, 3264, 2448],
        None,
        (1600, 1200),
    ),
    (
        "C5060WZ_",
        Variant::V3,
        (2608, 1949),
        [0, 0, 2608, 1949],
        None,
        (160, 120),
    ),
    (
        "XZ-2_",
        Variant::V4,
        (3984, 2986),
        [8, 10, 3968, 2976],
        None,
        (3200, 2400),
    ),
    (
        "E-3_",
        Variant::V5,
        (3720, 2800),
        [54, 22, 3648, 2736],
        Some(4),
        (1600, 1200),
    ),
    (
        "E-M10MarkII_",
        Variant::V5,
        (4640, 3472),
        [8, 8, 4608, 3456],
        Some(6),
        (3200, 2400),
    ),
    (
        "E-M1MarkII_",
        Variant::V5,
        (5240, 3912),
        [12, 12, 5184, 3888],
        Some(0),
        (3200, 2400),
    ),
    (
        "OM-1_",
        Variant::V5,
        (5220, 3912),
        [12, 12, 5184, 3888],
        Some(7),
        (3200, 2400),
    ),
    (
        "OM-1MarkII_",
        Variant::V6,
        (8172, 6132),
        [6, 6, 8160, 6120],
        Some(4),
        (3200, 2400),
    ),
];

/// The vectors of the specification that are bytes of the sample files
/// (TV-2, TV-9, TV-10, TV-13, the IFD0 example).
fn sample_vectors(name: &str, data: &[u8], c: &meta::Container<'_>, cands: &[Candidate]) {
    let ifd0 = c.ifd0();
    let strip = ifd0.uint(0x0111).unwrap().unwrap() as usize;
    let note = c.note.as_ref().expect("maker note");
    if name.starts_with("E-3_") {
        // TV-2: OLYMPUS style at 3344, main IFD at 3356 with 8 entries,
        // ImageProcessing at 3344 + 1164, CameraSettings at 3344 + 396.
        assert_eq!((note.style, note.start), (Style::Olympus, 3344));
        let main = note.main.as_ref().unwrap();
        assert_eq!(main.entries.len(), 8);
        assert_eq!(main.uint(0x2040).unwrap(), Some(1164));
        assert_eq!(main.uint(0x2020).unwrap(), Some(396));
        let ip = ifd::dir(data, true, 4508, 3344).unwrap();
        assert_eq!(
            ip.entries.len(),
            note.image_processing.as_ref().unwrap().entries.len()
        );
        // TV-9: the strip at 655360, its first values.
        assert_eq!(strip, 655_360);
        assert_eq!(
            data[strip..strip + 16],
            hex("00 00 00 00 01 00 00 4B 32 86 2A 8A 22 A2 67 A9")[..]
        );
        let len = ifd0.uint(0x0117).unwrap().unwrap() as usize;
        assert_eq!(len, 9_009_482);
        // TV-13: the 1600 x 1200 preview at 24576 and the thumbnail at
        // 13824, chosen in that order.
        assert_eq!((cands[0].offset, cands[0].len), (24_576, 419_789));
        assert_eq!((cands[0].width, cands[0].height), (1600, 1200));
        assert!(cands
            .iter()
            .any(|p| p.offset == 13_824 && p.len == 5135 && p.width == 160));
    }
    if name.starts_with("OM-1MarkII_") {
        // TV-10: the strip at 1817600.
        assert_eq!(strip, 1_817_600);
        assert_eq!(
            data[strip..strip + 21],
            hex("00 00 00 00 00 01 00 00 00 00 04 78 00 00 0A C2 CC 7A 03 21 4B")[..]
        );
        assert_eq!(note.style, Style::OmSystem);
    }
    if name.starts_with("OM-1_") {
        // TV-2: OM SYSTEM style at 5512, main IFD at 5528.
        assert_eq!((note.style, note.start), (Style::OmSystem, 5512));
    }
    if name.starts_with("E-M10MarkII_") {
        // The IFD0 example: 24 entries, Exif IFD at 302, maker note at 3572,
        // preview at 52224 (1029414 bytes), strip at 1485312 (13786930).
        assert_eq!(ifd0.entries.len(), 24);
        assert_eq!(ifd0.uint(0x8769).unwrap(), Some(302));
        assert_eq!(note.start, 3572);
        assert_eq!((cands[0].offset, cands[0].len), (52_224, 1_029_414));
        assert_eq!(strip, 1_485_312);
        assert_eq!(ifd0.uint(0x0117).unwrap(), Some(13_786_930));
    }
    if name.starts_with("E-1_") {
        // TV-2 OLYMP header and TV-3 E-1 pattern; the preview at 34156 is a
        // file offset.
        assert_eq!(note.style, Style::Olymp);
        assert_eq!(
            data[note.start as usize..note.start as usize + 8],
            hex("4F 4C 59 4D 50 00 02 00")[..]
        );
        assert_eq!(
            meta::cfa_pattern(c.exif.as_ref().unwrap(), true).unwrap(),
            [1, 0, 2, 1]
        );
        assert_eq!(cands[0].offset, 34_156);
    }
}

/// Every `.ORF` file in the folder named by `PANDITE_ORF_SAMPLES` (absolute,
/// or relative to the workspace root) decodes through the sensor-data path
/// when its variant is in scope, at the size of its crop rectangle, and
/// agrees with the file's own largest embedded JPEG on an 8 x 8 grid of
/// cell means, by the method and tolerances of
/// `arw_real_samples_match_embedded_preview`: exposure matched by one gain
/// on luminance, rg chromaticity distance of the usable cells (mean 0.05,
/// largest 0.15), Spearman rank correlation of cell luminance at least
/// 0.90, at least 16 usable cells. Files with the generic multipliers of
/// R-ORF-38 are compared after one global gain per channel. When the JPEG
/// shows only a central part of the image ([`PREVIEW_AREA`]), that part is
/// compared, and the figures of the whole image are printed too.
/// Out-of-scope files must return the preview.
#[test]
fn orf_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_ORF_SAMPLES") else {
        println!("skip: PANDITE_ORF_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("orf"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .ORF files in {}", folder.display());
    let mut failures = Vec::new();
    let mut developed = 0;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let class = meta::classify(&c).expect("classify");
        let cands = preview::collect(&c);
        let start = Instant::now();
        // The default display rendering, as the viewer shows it.
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("ORF path");
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
        // The sensor-data path, not the preview fallback.
        assert_eq!(
            dev.origin,
            Origin::Sensor(variant),
            "{name} fell back to the preview"
        );
        developed += 1;
        let info = dev.info.clone().expect("raw facts");
        let (ow, oh) = render::output_size(info.crop, info.orientation);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (ow, oh),
            "{name}"
        );
        // The measurements of the specification for this file.
        if let Some(m) = MEASURED.iter().find(|m| name.starts_with(m.0)) {
            let crop = [info.crop.x, info.crop.y, info.crop.w, info.crop.h];
            assert_eq!((variant, info.raster, crop), (m.1, m.2, m.3), "{name}");
            assert_eq!(info.unused_bits, m.4, "{name}: M12, M13");
            assert_eq!((cands[0].width, cands[0].height), m.5, "{name}: M19");
        }
        // R-ORF-26: at most 7 bits of the last byte unused.
        if let Some(u) = info.unused_bits {
            assert!(u < 8, "{name}: {u} unused bits");
        }
        sample_vectors(&name, &data, &c, &cands);
        // The camera's largest JPEG, decoded small and oriented (R-ORF-52).
        let best = cands.first().expect("an embedded preview");
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev = preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let theirs = grid_means(&prev, 8, 1);
        let balance = info.wb_source == Source::Generic;
        let area = PREVIEW_AREA
            .iter()
            .find(|a| name.starts_with(a.0))
            .map(|a| a.1);
        let mut whole = String::new();
        if area.is_some() {
            let (m, x, r, u) = agreement(&grid_means(&dev.image, 8, 4), &theirs, balance);
            whole = format!(" (whole image: chroma mean {m:.4} max {x:.4} rank {r:.3} cells {u})");
        }
        let compared = area.map_or_else(|| dev.image.clone(), |f| central(&dev.image, f));
        let ours = grid_means(&compared, 8, 4);
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let lum = |g: &[[f64; 3]]| {
            g.iter()
                .map(|c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])
                .sum::<f64>()
        };
        let ev = (lum(&ours) / lum(&theirs)).log2();
        let line = format!(
            "{name}: model {} {variant:?} ({described}) sensor path, note {:?}, raster {}x{} crop {}x{}+{}+{} out {}x{} B {} black {:?} ({:?}) wb {:.4} {:.4} ({:?}) matrix {:?} unused bits {:?} decode {:.2}s (debug) | vs embedded {}x{} (compared {}x{}): chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}, mean luminance {ev:+.2} EV{whole}{}",
            info.model,
            info.style,
            info.raster.0,
            info.raster.1,
            info.crop.w,
            info.crop.h,
            info.crop.x,
            info.crop.y,
            ow,
            oh,
            info.bits,
            info.black,
            info.black_source,
            info.wb[0],
            info.wb[2],
            info.wb_source,
            info.matrix_source,
            info.unused_bits,
            took.as_secs_f64(),
            best.width,
            best.height,
            compared.width(),
            compared.height(),
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        );
        if mean > CHROMA_MEAN || max > CHROMA_MAX || rank < RANK_MIN || used < 16 {
            failures.push(line.clone());
        }
        println!("{line}");
    }
    println!(
        "{developed} of {} files developed from sensor data",
        files.len()
    );
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

//! Tests of the RW2 decoder against the specification's test vectors
//! (TV-1 to TV-10), fixtures (F1 to F8) and requirements, plus hostile
//! input, cancellation, thumbnails and real camera files.

use super::fixtures::*;
use super::meta::{self, Class, Relative, Variant, WbSource};
use super::sensor;
use super::*;
use crate::raw::arw::render;
use crate::raw::arw::tests::{agreement, grid_means, noise, LinearF32, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// TV-9: the expected linear sRGB of every output pixel of F1, F3, F5.
const OUT_12: [f32; 3] = [0.51125, 0.547_029_7, 0.31];
/// TV-9: F2, F4, F6.
const OUT_14: [f32; 3] = [0.511_282_2, 0.547_064_2, 0.310_019_5];
/// TV-9: F7, F8.
const OUT_RAW: [f32; 3] = [0.596_587_3, 0.548_672, 0.363_777_8];
/// The specification prints 7 decimals; a match within 1e-6 is a match at
/// the printed precision.
const EPS: f32 = 1e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open").expect("RW2 header")
}

fn plan_of(data: &[u8]) -> meta::Plan {
    let c = container(data);
    let Class::Sensor(v) = meta::classify(&c) else {
        panic!("not a sensor variant: {:?}", meta::classify(&c));
    };
    meta::plan(&c, v).expect("plan")
}

/// The decoded plane of the active area, row-major.
fn plane_of(data: &[u8]) -> (usize, usize, Vec<u16>) {
    let plan = plan_of(data);
    let p = sensor::decode(data, &plan, &CancelToken::new()).expect("decode sensor");
    let (w, h) = plan.active.size();
    let mut v = Vec::new();
    for y in 0..h {
        v.extend_from_slice(p.row(y, 0));
    }
    (w, h, v)
}

/// Render a plan to linear sRGB values (R-RW2-39), oriented.
fn linear_with(data: &[u8], plan: &meta::Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let sensor = render::Sensor::Cfa(sensor::decode(data, plan, &CancelToken::new()).unwrap());
    let shared = carrier(plan);
    let bytes = render::render(&shared, &sensor, &CancelToken::new(), &LinearF32).expect("render");
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
        .expect("RW2 path")
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
/// within 2 codes.
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

/// A V8 file of `w` x `h` 12-bit values (row-major) with the F8 tags and
/// the given extra entries.
fn v8_file(w: u16, h: u16, values: &[u16], extra: Vec<(u16, Val)>) -> Vec<u8> {
    let data: Vec<u8> = values.iter().flat_map(|v| (v * 16).to_le_bytes()).collect();
    let mut e = f8_entries();
    e = with(e, 0x0002, Val::Short(w));
    e = with(e, 0x0003, Val::Short(h));
    e = with(e, 0x0006, Val::Short(h));
    e = with(e, 0x0007, Val::Short(w));
    e = with(e, 0x0117, Val::Long(2 * u32::from(w) * u32::from(h)));
    for (t, v) in extra {
        e = with(e, t, v);
    }
    raw(e, data)
}

/// The block-code pixels of one V1/V4 or V5 block.
fn code_block(bytes: &[u8], variant: Variant) -> Vec<u16> {
    let mut b = [0u8; 16];
    b.copy_from_slice(bytes);
    let mut out = vec![0u16; variant.per_block()];
    sensor::block_code(&b, variant, &mut out);
    out
}

/// The packed pixels of one block.
fn packed_block(bytes: &[u8], depth: u32) -> Vec<u16> {
    let mut b = [0u8; 16];
    b.copy_from_slice(bytes);
    let mut out = vec![0u16; (128 / depth) as usize];
    sensor::packed(&b, depth, &mut out);
    out
}

// ---------------------------------------------------------------------------
// Test vectors and fixtures
// ---------------------------------------------------------------------------

#[test]
fn rw2_vectors_decode() {
    // TV-1 (R-RW2-5, R-RW2-7): RW2 with IFD0 at 24, RAW with IFD0 at 8.
    let file1 = f1();
    let c = container(&file1);
    assert_eq!(c.ifd0.entries.len(), 22);
    assert_eq!(c.ifd0_end, 24 + 2 + 12 * 22 + 4);
    assert_eq!(c.ifd0_end, 0x126);
    let file7 = f7();
    let c = container(&file7);
    assert_eq!(c.ifd0.entries.len(), 23);
    assert_eq!(c.ifd0_end, 0x11E + 4);
    // R-RW2-7: bytes 8 to 23 are not read: any content there decodes alike.
    let mut f = f1();
    f[8..24].copy_from_slice(&hex("88 E7 74 D8 F8 25 1D 4D 94 7A 6E 77 82 2B 5D 6A"));
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V1));
    // X4: a plain TIFF and a big-endian header are not RW2 (R-RW2-5).
    for head in ["49 49 2A 00", "4D 4D 00 55"] {
        let mut x = f1();
        x[..4].copy_from_slice(&hex(head));
        assert!(meta::open(&x).unwrap().is_none(), "{head}");
        assert!(decode_report(&x, &DecodeOptions::default())
            .unwrap()
            .is_none());
        assert!(!is_rw2(&x, None));
    }
    assert!(is_rw2(&f1(), None) && is_rw2(&f7(), None));
    assert!(is_rw2(b"MM", Some("rw2")));

    // TV-2: IFD entries (R-RW2-8, R-RW2-9, R-RW2-10).
    let c = container(&file1);
    assert_eq!(c.uint(meta::tag::RAW_FORMAT), Some(4));
    assert_eq!(c.uint(meta::tag::COMPRESSION), Some(34316));
    assert_eq!(c.uint(meta::tag::RAW_DATA_OFFSET), Some(512));
    let model = c.ifd0.entry(meta::tag::MODEL).unwrap();
    assert_eq!((model.typ, model.count, model.at), (2, 7, 0x126));
    assert_eq!(c.model(), b"DC-GH5");
    // The S10 entries, in a small IFD: JpgFromRaw (UNDEFINED, count 778752
    // at 1536) and RawDataOffset 780288.
    let mut f = hex("49 49 55 00 08 00 00 00 02 00");
    f.extend(hex("2E 00 07 00 00 E2 0B 00 00 06 00 00"));
    f.extend(hex("18 01 04 00 01 00 00 00 00 E8 0B 00"));
    f.extend([0u8; 4]);
    let c = container(&f);
    let jpg = c.ifd0.entry(0x002E).unwrap();
    assert_eq!(
        (jpg.typ, jpg.count, jpg.at, jpg.len),
        (7, 778752, 1536, 778752)
    );
    assert_eq!(c.uint(0x0118), Some(780288));
    // The stream lies outside this file: no candidate (R-RW2-45).
    assert!(preview::collect(&c).is_empty());
    // R-RW2-9: entry order does not matter; R-RW2-10: SHORT or LONG, first
    // value; a type outside 1 to 12 is skipped.
    let mut e = f1_entries();
    e = with(e, 0x002D, Val::Long(4));
    let order = rw2(e, rotate(&f1_blocks()));
    assert_eq!(report(&order).origin, Origin::Sensor(Variant::V1));
    let mut swapped = f1();
    // Swap entries 0 (0x0002) and 21 (0x0118).
    let (a, b) = (24 + 2, 24 + 2 + 12 * 21);
    let ea: Vec<u8> = swapped[a..a + 12].to_vec();
    let eb: Vec<u8> = swapped[b..b + 12].to_vec();
    swapped[a..a + 12].copy_from_slice(&eb);
    swapped[b..b + 12].copy_from_slice(&ea);
    assert_eq!(plan_of(&swapped).width, 28);
    assert_eq!(report(&swapped).origin, Origin::Sensor(Variant::V1));
    let typ13 = rw2(
        with(f1_entries(), 0x002D, Val::Raw(13, 1, 4)),
        rotate(&f1_blocks()),
    );
    assert_eq!(
        meta::classify(&container(&typ13)),
        Class::OutOfScope(Relative::X3)
    );
    // Two values: the first is used.
    let mut two = f1();
    // RawFormat entry at 0xF2: SHORT count 2, values 4 and 9.
    two[0xF2 + 4..0xF2 + 8].copy_from_slice(&2u32.to_le_bytes());
    two[0xF2 + 10..0xF2 + 12].copy_from_slice(&9u16.to_le_bytes());
    assert_eq!(meta::classify(&container(&two)), Class::Sensor(Variant::V1));

    // TV-3: V1 block, all classes and the offset reset (R-RW2-25 to
    // R-RW2-27).
    let tv3 = hex("03 FF 64 03 FE 2B 08 28 83 1C 20 9E 20 D0 87 3E");
    assert_eq!(
        code_block(&tv3, Variant::V1),
        [1000, 2000, 1002, 1992, 1002, 2136, 846, 2136, 374, 2644, 512, 1600, 4080, 48]
    );
    // V4 has the layout of V1.
    assert_eq!(code_block(&tv3, Variant::V4), code_block(&tv3, Variant::V1));
    // TV-4: limit at F (R-RW2-28).
    let tv4 = hex("80 80 80 00 02 02 02 08 08 08 20 C0 3F C8 F0 FF");
    assert_eq!(
        code_block(&tv4, Variant::V1),
        [4095, 200, 4095, 72, 4095, 128, 4095, 128, 4095, 128, 4095, 128, 4095, 128]
    );
    // R-RW2-29: the same block gives the same pixels wherever it lies.
    assert_eq!(code_block(&tv4, Variant::V1), code_block(&tv4, Variant::V1));

    // TV-5: V1 blocks from S14 with the chunk rotation (R-RW2-21).
    let s = 718336u64;
    assert_eq!(sensor::logical_offset(s, 0), Some(726520));
    assert_eq!(sensor::logical_offset(s, 11746208), Some(12456344));
    // The rotation example: logical 0, 8199, 8200 and 16383 of chunk 0.
    for (l, at) in [(0u64, 8184u64), (8199, 16383), (8200, 0), (16383, 8183)] {
        assert_eq!(sensor::logical_offset(0, l), Some(at), "logical byte {l}");
    }
    let row0 = hex("81 87 9D 08 96 15 72 88 17 07 E0 A0 1E 00 61 0C");
    assert_eq!(
        code_block(&row0, Variant::V1),
        [198, 256, 192, 259, 192, 244, 184, 251, 189, 224, 191, 253, 198, 254]
    );
    let row1952 = hex("89 7E 82 F4 DD 1D 02 38 88 88 5F 60 21 B0 90 09");
    assert_eq!(
        code_block(&row1952, Variant::V1),
        [153, 176, 158, 177, 156, 185, 159, 185, 166, 176, 163, 178, 161, 187]
    );
    // Block 512 of a chunk wraps inside the chunk: its first 8 bytes at
    // chunk offsets 16376 to 16383, the last 8 at 0 to 7.
    let mut chunk = vec![0u8; 16384];
    for (k, b) in chunk.iter_mut().enumerate() {
        *b = (k % 251) as u8;
    }
    let block = sensor::block_bytes(&chunk, 0, 512, true).unwrap();
    let want: Vec<u8> = (16376..16384)
        .chain(0..8)
        .map(|k| (k % 251) as u8)
        .collect();
    assert_eq!(block.to_vec(), want);
    let block = sensor::block_bytes(&chunk, 0, 0, true).unwrap();
    assert_eq!(block.to_vec(), chunk[8184..8200].to_vec());
    assert!(sensor::block_bytes(&chunk, 0, 1024, true).is_none());

    // TV-6: packed blocks (R-RW2-23).
    let p14 = hex("00 02 FA F0 FF 03 00 FF 9F 34 11 0E 61 09 BC 02");
    assert_eq!(
        packed_block(&p14, 14),
        [512, 1000, 16383, 0, 8191, 1234, 4321, 600, 700]
    );
    let p12 = hex("80 F0 FF 00 00 80 2C 21 4D 8A 7C 3E 01 D0 04 00");
    assert_eq!(
        packed_block(&p12, 12),
        [128, 4095, 0, 2048, 300, 1234, 3210, 999, 1, 77]
    );
    assert_eq!(sensor::logical_offset(922112, 4994496), Some(5908408));
    let s15 = hex("0A 03 65 61 2F 38 15 F9 82 3D 11 2E BC 14 FB 02");
    assert_eq!(
        packed_block(&s15, 14),
        [778, 1428, 758, 1358, 761, 1270, 737, 1327, 763]
    );
    let s16 = hex("A4 50 0E A2 20 0E A7 A0 0E A3 70 0E A5 10 0F 00");
    assert_eq!(
        packed_block(&s16, 12),
        [164, 229, 162, 226, 167, 234, 163, 231, 165, 241]
    );
    let s19 = hex("9C 06 DC A1 6A D4 1D C1 06 DB 31 69 74 1D A5 06");
    assert_eq!(
        packed_block(&s19, 14),
        [1692, 1904, 1706, 1909, 1729, 1900, 1683, 1885, 1701]
    );
    let s21 = hex("BF 63 81 D5 53 7A C8 73 76 BA A3 78 CD 83 77 00");
    assert_eq!(
        packed_block(&s21, 12),
        [959, 2070, 981, 1957, 968, 1895, 954, 1930, 973, 1912]
    );

    // TV-7: V5 blocks.
    let a = hex("C0 EB FF 20 8C E5 FF 00 04 20 7D 08 82 32 22 4E");
    assert_eq!(
        code_block(&a, Variant::V5),
        [5000, 9000, 5008, 8988, 5008, 7964, 6030, 8140, 512, 16368, 11200]
    );
    let b = hex("00 E0 FF 00 00 20 80 00 42 38 96 64 88 BB 70 17");
    assert_eq!(
        code_block(&b, Variant::V5),
        [1500, 3000, 400, 3352, 3600, 3352, 3600, 3352, 3088, 3863, 3088]
    );
    let s17 = hex("F0 5E 79 EB 41 20 98 F6 A1 96 7F 53 82 3F F0 18");
    assert_eq!(
        code_block(&s17, Variant::V5),
        [1596, 1016, 1679, 1014, 1529, 1004, 1625, 1008, 1604, 981, 1587]
    );

    // TV-8: V8 samples (R-RW2-24, R-RW2-6).
    let (_, _, v) = plane_of(&f8());
    assert_eq!(v, [1131, 2119, 1131, 2119, 2119, 1131, 2119, 1131]);
    let top = v8_file(4, 2, &[4095; 8], vec![]);
    assert_eq!(plane_of(&top).2, [4095; 8]);

    // The fixtures: the builder reproduces the printed bytes of F1.
    let f = f1();
    assert_eq!(f.len(), 16896);
    assert_eq!(&f[..0x130], &hex(F1_HEAD_HEX)[..]);
    assert_eq!(&f[0x21F8..0x2238], &f1_blocks()[..]);
    for (name, f, len) in [
        ("F2", f2(), 16896),
        ("F3", f3(), 16896),
        ("F4", f4(), 576),
        ("F5", f5(), 576),
        ("F6", f6(), 576),
        ("F7", f7(), 576),
        ("F8", f8(), 528),
    ] {
        assert_eq!(f.len(), len, "{name} length");
    }
    // F7: next-IFD offset at 0x11E, Model at 0x122; F8: 0x112 and 0x116.
    assert_eq!(
        &f7()[0x11E..0x129],
        &hex("00 00 00 00 44 43 2D 47 48 35 00")[..]
    );
    assert_eq!(
        &f8()[0x112..0x11D],
        &hex("00 00 00 00 44 43 2D 47 48 35 00")[..]
    );

    // Every fixture decodes to its flat CFA image and its TV-9 output
    // (R-RW2-1).
    let fixtures = [
        ("F1", f1(), Variant::V1, OUT_12, [1131u16, 2119, 1131]),
        ("F2", f2(), Variant::V3, OUT_14, [4480, 8448, 4480]),
        ("F3", f3(), Variant::V2, OUT_12, [1131, 2119, 1131]),
        ("F4", f4(), Variant::V5, OUT_14, [4480, 8448, 4480]),
        ("F5", f5(), Variant::V4, OUT_12, [1131, 2119, 1131]),
        ("F6", f6(), Variant::V6, OUT_14, [4480, 8448, 4480]),
        ("F7", f7(), Variant::V7, OUT_RAW, [1131, 2119, 1131]),
        ("F8", f8(), Variant::V8, OUT_RAW, [1131, 2119, 1131]),
    ];
    for (name, f, variant, out, rgb) in fixtures {
        assert_eq!(
            meta::classify(&container(&f)),
            Class::Sensor(variant),
            "{name}"
        );
        let plan = plan_of(&f);
        let (w, h, v) = plane_of(&f);
        assert_eq!((w, h), (plan.width, plan.height), "{name}");
        for y in 0..h {
            for x in 0..w {
                let color = plan.colors[2 * (y % 2) + x % 2];
                assert_eq!(v[y * w + x], rgb[usize::from(color)], "{name} ({x}, {y})");
            }
        }
        let (lw, lh, px) = linear(&f);
        assert_eq!((lw, lh), (w, h), "{name}");
        for p in px {
            assert_rgb(p, out, EPS, name);
        }
        // Through the provider path: sensor data, linear sRGB with the sRGB
        // transfer (R-RW2-50), the size of the active area.
        let dev = report(&f);
        assert_eq!(dev.origin, Origin::Sensor(variant), "{name}");
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (w, h)
        );
        assert_rgba16_flat(&dev.image, out, name);
    }

    // R-RW2-60, M12: all-FF blocks right of the active area decode by the
    // normal rules without an error. V4, W 42 (3 blocks a row), H 16, the
    // active area columns 0 to 27; the third block of every row is FF.
    let ff = [0xFFu8; 16];
    let blocks = f1_blocks();
    let mut data = Vec::new();
    for y in 0..16 {
        let b = &blocks[32 * (y % 2)..32 * (y % 2) + 32];
        data.extend_from_slice(b);
        data.extend_from_slice(&ff);
    }
    let e = common(42, 16, 12, 128, 6);
    let e = with(e, 0x0007, Val::Short(28));
    let f = rw2(e, data);
    let plan = plan_of(&f);
    assert_eq!(plan.active.size(), (28, 16));
    let mut row = vec![0u16; 42];
    sensor::decode_row(&f, &plan, 0, &mut row).unwrap();
    assert_eq!(&row[28..32], &[4095, 4095, 4080, 4080], "all-FF block");
    let dev = report(&f);
    assert_eq!(dev.origin, Origin::Sensor(Variant::V4));
    assert_eq!((dev.image.width(), dev.image.height()), (28, 16));
    assert_rgba16_flat(&dev.image, OUT_12, "all-FF blocks outside");

    // TV-10: orientation (R-RW2-43, R-RW2-44): a 3 x 2 image a b c / d e f.
    let vals = [1000u16, 1500, 2000, 2500, 3000, 3500];
    let pixels = |o: u16| {
        let f = v8_file(3, 2, &vals, vec![(0x0112, Val::Short(o))]);
        let dev = report(&f);
        let img = dev.image;
        let px: Vec<[u8; 8]> = img
            .data()
            .chunks_exact(8)
            .map(|c| c.try_into().unwrap())
            .collect();
        (img.width(), img.height(), px)
    };
    let (_, _, upright) = pixels(1);
    let name =
        |p: &[u8; 8]| ['a', 'b', 'c', 'd', 'e', 'f'][upright.iter().position(|q| q == p).unwrap()];
    for (o, size, want) in [
        (6u16, (2, 3), "daebfc"),
        (8, (2, 3), "cfbead"),
        (3, (3, 2), "fedcba"),
        (1, (3, 2), "abcdef"),
    ] {
        let (w, h, px) = pixels(o);
        assert_eq!((w, h), size, "orientation {o}");
        let got: String = px.iter().map(name).collect();
        assert_eq!(got, want, "orientation {o}");
    }
    // R-RW2-43: a value outside 1 to 8 counts as 1.
    let (w, h, px) = pixels(9);
    assert_eq!((w, h), (3, 2));
    assert_eq!(px, upright);
    // The other orientations by the table of R-RW2-44.
    for o in [2u16, 4, 5, 7] {
        let (w, h, px) = pixels(o);
        let (w, h) = (w as usize, h as usize);
        for y in 0..2usize {
            for x in 0..3usize {
                let (ox, oy) = match o {
                    2 => (2 - x, y),
                    4 => (x, 1 - y),
                    5 => (y, x),
                    _ => (1 - y, 2 - x),
                };
                assert!(ox < w && oy < h);
                assert_eq!(px[oy * w + ox], upright[y * 3 + x], "orientation {o}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Levels, white balance, matrix, demosaicing
// ---------------------------------------------------------------------------

/// An independent per-pixel implementation of R-RW2-35, R-RW2-40,
/// R-RW2-41, R-RW2-18 and R-RW2-38 in f64, on the stored raster, for
/// stored pixel (x, y) of the active area.
fn reference(values: &[u16], w: usize, plan: &meta::Plan, x: usize, y: usize) -> [f64; 3] {
    let a = plan.active;
    let col = |x: usize, y: usize| usize::from(plan.colors[2 * (y % 2) + x % 2]);
    let norm = |x: usize, y: usize| {
        let c = col(x, y);
        let b = f64::from(plan.black[c]);
        ((f64::from(values[y * w + x]) - b) / (f64::from(plan.white[c]) - b) * plan.wb[c])
            .clamp(0.0, 1.0)
    };
    let own = col(x, y);
    let mut sum = [0f64; 3];
    let mut cnt = [0u32; 3];
    for dy in -1i64..=1 {
        for dx in -1i64..=1 {
            let (xx, yy) = (x as i64 + dx, y as i64 + dy);
            let inside = xx >= a.left as i64
                && xx < a.right as i64
                && yy >= a.top as i64
                && yy < a.bottom as i64;
            if (dx == 0 && dy == 0) || !inside {
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

#[test]
fn rw2_levels_wb_matrix_vector() {
    // TV-9 V1 through F1 (R-RW2-30 to R-RW2-34).
    let p = plan_of(&f1());
    assert_eq!(p.white, [4095; 3]);
    assert_eq!(p.black, [143; 3], "R-RW2-32: tag 128 + 15");
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert_eq!(p.wb, [2.0, 1.0, 1.5]);
    // R-RW2-35 worked example.
    let n = |v: f64, b: f64, w: f64, m: f64| ((v - b) / (w - b) * m).clamp(0.0, 1.0);
    assert!((n(1131.0, 143.0, 4095.0, 2.0) - 0.5).abs() < 1e-12);
    assert!((n(2119.0, 143.0, 4095.0, 1.0) - 0.5).abs() < 1e-12);
    assert!((n(1131.0, 143.0, 4095.0, 1.5) - 0.375).abs() < 1e-12);
    // R-RW2-36, R-RW2-37: the DC-GH5 row 2 normalized.
    assert!(p.matrix_from_table);
    for (g, w) in p.matrix[1]
        .iter()
        .zip([-0.217_821_8, 1.594_059_4, -0.376_237_6])
    {
        assert!((g - w).abs() < 5e-8, "row 2 {:?}", p.matrix[1]);
    }
    // R-RW2-38 worked example.
    let v = [0.5, 0.5, 0.375];
    let out: Vec<f64> = p
        .matrix
        .iter()
        .map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        .collect();
    for (g, w) in out.iter().zip(OUT_12) {
        assert!((g - f64::from(w)).abs() < 1e-6, "TV-9 {out:?}");
    }
    // TV-9 V3 (F2) and V7, V8 (F7, F8): R-RW2-30 F, R-RW2-32 no offset.
    let p = plan_of(&f2());
    assert_eq!((p.white, p.black), ([16383; 3], [512; 3]));
    for f in [f7(), f8()] {
        let p = plan_of(&f);
        assert_eq!((p.white, p.black), ([4095; 3], [0; 3]));
    }
    // A neutral value stays neutral for every normalized matrix.
    for model in ["DC-GH5", "DMC-LX3", "DC-S1R", "unknown"] {
        let (m, _) = color::matrix(model.as_bytes());
        for r in m {
            assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12, "{model}");
            let g: f64 = r.iter().map(|k| k * 0.25).sum();
            assert!((g - 0.25).abs() < 1e-12);
        }
    }

    let mk = |e: Vec<(u16, Val)>| plan_of(&rw2(e, rotate(&f1_blocks())));
    // R-RW2-31: LinearityLimit within 0.9 F to F is the white level.
    for (limit, want) in [
        (3967u16, 3967u32),
        (3686, 3686),
        (3685, 4095),
        (2111, 4095),
        (4096, 4095),
        (16383, 4095),
        (4095, 4095),
    ] {
        let e = with(f1_entries(), 0x000F, Val::Short(limit));
        let p = mk(e);
        assert_eq!(p.white, [4095, want, 4095], "LinearityLimit {limit}");
    }
    assert_eq!(mk(without(f1_entries(), 0x000E)).white[0], 4095);
    // R-RW2-33: a missing black tag is 0 before the offset; b_c >= w_c - 255
    // uses the preview path.
    assert_eq!(mk(without(f1_entries(), 0x001E)).black, [143, 143, 15]);
    let e = with(f1_entries(), 0x001D, Val::Short(3824));
    assert_eq!(mk(e).black[1], 3839);
    let c_f = rw2(
        with(f1_entries(), 0x001D, Val::Short(3825)),
        rotate(&f1_blocks()),
    );
    assert!(
        meta::plan(&container(&c_f), Variant::V1).is_err(),
        "b = w - 255"
    );
    // R-RW2-34: sources 2 (table) and 3 (generic).
    let p = mk(without(f1_entries(), 0x0026));
    assert_eq!((p.wb_source, p.wb), (WbSource::Table, [2.14, 1.0, 2.01]));
    let p = mk(with(f1_entries(), 0x0025, Val::Short(0)));
    assert_eq!(p.wb_source, WbSource::Table, "a level of 0 skips the tags");
    let p = mk(with(
        without(f1_entries(), 0x0024),
        0x0110,
        Val::Ascii("DC-G9"),
    ));
    assert_eq!(
        (p.wb_source, p.wb),
        (WbSource::Generic, [2.337, 1.0, 1.735])
    );
    assert!(!p.matrix_from_table, "R-RW2-36: generic matrix");
    // R-RW2-12: trailing spaces and zero bytes do not count.
    let p = mk(with(f1_entries(), 0x0110, Val::Ascii("DMC-LX3  ")));
    assert!(p.matrix_from_table);
    assert_eq!(p.model, "DMC-LX3");
    // R-RW2-34 worked example: 600, 300, 450 give 2 and 1.5 (above).

    // R-RW2-61: values above white and below black clip to 0 and 1, the
    // normalized values and the output values.
    let f = v8_file(4, 2, &[4095, 4095, 4095, 4095, 0, 0, 0, 0], vec![]);
    for p in linear(&f).2 {
        assert!(p.iter().all(|c| (0.0..=1.0).contains(c)), "{p:?}");
    }
    // R-RW2-59, R-RW2-28: the largest value before the limit (F + 508)
    // stays in range: TV-4 class 2 with code 255 on P = F.
    let mut b = 0u128;
    b |= (4095u128 << 116) | (4095u128 << 104) | (2u128 << 102) | (255u128 << 94);
    let out = code_block(&b.to_le_bytes(), Variant::V1);
    assert_eq!(out[2], 4095);

    // R-RW2-15, R-RW2-17, R-RW2-18, R-RW2-35, R-RW2-38, R-RW2-40 to
    // R-RW2-42: an odd active area (left 15, top 7, as in the worked
    // example) of random data, every CFAPattern, against the independent
    // reference.
    let (w, h) = (40u16, 30u16);
    let mut next = noise(0x1234_5678);
    let vals: Vec<u16> = (0..usize::from(w) * usize::from(h))
        .map(|_| (next() % 4096) as u16)
        .collect();
    for pattern in 1u16..=4 {
        let extra = vec![
            (0x0004, Val::Short(7)),
            (0x0005, Val::Short(15)),
            (0x0006, Val::Short(25)),
            (0x0007, Val::Short(33)),
            (0x0009, Val::Short(pattern)),
            (0x000F, Val::Short(4000)),
            (0x001C, Val::Short(60)),
            (0x001D, Val::Short(40)),
            (0x001E, Val::Short(20)),
        ];
        let f = v8_file(w, h, &vals, extra);
        let plan = plan_of(&f);
        assert_eq!(
            plan.active,
            Active {
                top: 7,
                left: 15,
                bottom: 25,
                right: 33
            }
        );
        assert_eq!(plan.white, [4095, 4000, 4095]);
        assert_eq!(plan.black, [60, 40, 20]);
        let (ow, oh, px) = linear(&f);
        assert_eq!((ow, oh), (18, 18), "R-RW2-17: the active area");
        for y in 0..18 {
            for x in 0..18 {
                let want = reference(&vals, usize::from(w), &plan, x + 15, y + 7);
                let got = px[y * 18 + x];
                for c in 0..3 {
                    assert!(
                        (f64::from(got[c]) - want[c]).abs() < 2e-5,
                        "pattern {pattern} ({x}, {y}) {got:?} {want:?}"
                    );
                }
            }
        }
        // R-RW2-42 worked example: CFAPattern 1, pixel (15, 7) has CFA
        // position 2 * 1 + 1 = 3 and is blue.
        if pattern == 1 {
            let (x, y) = (15usize, 7usize);
            assert_eq!(plan.colors[2 * (y % 2) + x % 2], 2);
        }
    }
    // R-RW2-15: invalid borders use the whole raster.
    for (t, l, b, r) in [
        (7u16, 15u16, 22u16, 33u16),
        (7, 15, 25, 41),
        (25, 15, 7, 33),
        (0, 30, 30, 31),
    ] {
        let extra = vec![
            (0x0004, Val::Short(t)),
            (0x0005, Val::Short(l)),
            (0x0006, Val::Short(b)),
            (0x0007, Val::Short(r)),
        ];
        let plan = plan_of(&v8_file(w, h, &vals, extra));
        assert_eq!(plan.active.size(), (40, 30), "borders {t} {l} {b} {r}");
    }
    // Absent border tags take their defaults 0, 0, H, W.
    let mut e = f8_entries();
    for t in 0x0004..=0x0007 {
        e = without(e, t);
    }
    assert_eq!(plan_of(&raw(e, f8_data())).active.size(), (4, 2));
}

// ---------------------------------------------------------------------------
// Preview path
// ---------------------------------------------------------------------------

fn assert_preview(dev: &Developed, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_eq!(dev.origin, Origin::Preview, "{what}");
    assert_eq!(dev.image.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((dev.image.width(), dev.image.height()), size, "{what}");
    let p = &dev.image.data()[..4];
    for c in 0..3 {
        assert!(p[c].abs_diff(rgb[c]) <= 6, "{what}: {p:?} vs {rgb:?}");
    }
}

/// F1 entries with a small (16 x 8) JpgFromRaw and a large (48 x 32)
/// JpgFromRaw2, built with the F1 blocks.
fn f1_with_previews(e: Vec<(u16, Val)>) -> Vec<u8> {
    let e = with_previews(e, jpeg(16, 8, [40, 40, 200]), jpeg(48, 32, [200, 40, 40]));
    rw2(e, rotate(&f1_blocks()))
}

#[test]
fn rw2_out_of_scope_falls_back_to_preview() {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    // A good file develops: V1.
    let good = f1_with_previews(f1_entries());
    assert_eq!(report(&good).origin, Origin::Sensor(Variant::V1));
    let check = |f: &[u8], class: Option<Class>, what: &str| {
        let dev = report(f);
        // R-RW2-46: the largest candidate (JpgFromRaw2 here).
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        if let Some(class) = class {
            assert_eq!(dev.class, class, "{what}");
        }
    };
    // X1: RawFormat 8 (R-RW2-2: its sensor data are not interpreted).
    for bits in [12u16, 14, 16] {
        let e = with(
            with(f1_entries(), 0x002D, Val::Short(8)),
            0x000A,
            Val::Short(bits),
        );
        check(
            &f1_with_previews(e),
            Some(Class::OutOfScope(Relative::X1)),
            "X1",
        );
    }
    // X2: another RawFormat, or a pair outside the table (R-RW2-3 rules 3
    // to 9; R-RW2-58: never a layout from the file size, so these files
    // with V1-sized data are not developed).
    for (rf, bits) in [
        (9u16, 12u16),
        (1, 12),
        (3, 12),
        (7, 12),
        (4, 14),
        (2, 14),
        (6, 16),
    ] {
        let e = with(
            with(f1_entries(), 0x002D, Val::Short(rf)),
            0x000A,
            Val::Short(bits),
        );
        check(
            &f1_with_previews(e),
            Some(Class::OutOfScope(Relative::X2)),
            &format!("X2 {rf}/{bits}"),
        );
    }
    // X3: no RawFormat and the V8 rule does not match.
    let x3 = |e: Vec<(u16, Val)>, what: &str| {
        let e = with_previews(e, small.clone(), big.clone());
        check(
            &raw(e, f8_data()),
            Some(Class::OutOfScope(Relative::X3)),
            what,
        );
    };
    x3(
        with(f8_entries(), 0x000B, Val::Short(34826)),
        "X3 compression",
    );
    x3(with(f8_entries(), 0x0117, Val::Long(15)), "X3 byte count");
    x3(with(f8_entries(), 0x000A, Val::Short(14)), "X3 bits");
    x3(without(f8_entries(), 0x0117), "X3 no byte count");
    // X5: W not a multiple of p (R-RW2-4).
    let e = with(
        with(f1_entries(), 0x0002, Val::Short(30)),
        0x0007,
        Val::Short(30),
    );
    check(
        &f1_with_previews(e),
        Some(Class::OutOfScope(Relative::X5)),
        "X5",
    );
    let e = with(
        with(f7_entries(), 0x0002, Val::Short(25)),
        0x0007,
        Val::Short(25),
    );
    let e = with_previews(e, small.clone(), big.clone());
    check(
        &raw(e, f3_blocks()),
        Some(Class::OutOfScope(Relative::X5)),
        "X5 V7",
    );
    // X4: not 49 49 55 00: the RW2 path declines, the provider's generic
    // preview search shows the JPEG.
    let mut be = good.clone();
    be[..4].copy_from_slice(&hex("4D 4D 00 55"));
    assert!(decode_report(&be, &DecodeOptions::default())
        .unwrap()
        .is_none());
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(be, Some("x.rw2".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8, "X4 preview");

    // Failed checks of a specified variant use the preview (R-RW2-49).
    let v1 = Some(Class::Sensor(Variant::V1));
    // R-RW2-14: a required tag absent, count 0, W or H of 0.
    for t in [0x0002u16, 0x0003, 0x0009, 0x0118] {
        check(
            &f1_with_previews(without(f1_entries(), t)),
            v1,
            &format!("tag {t:#06x} missing"),
        );
    }
    check(
        &f1_with_previews(with(f1_entries(), 0x0118, Val::Raw(4, 0, 512))),
        v1,
        "count 0",
    );
    check(
        &f1_with_previews(with(f1_entries(), 0x0003, Val::Short(0))),
        v1,
        "H = 0",
    );
    // R-RW2-16: CFAPattern outside 1 to 4.
    for p in [0u16, 5] {
        check(
            &f1_with_previews(with(f1_entries(), 0x0009, Val::Short(p))),
            v1,
            "CFAPattern",
        );
    }
    // R-RW2-57: data start 0xFFFFFFFF, inside IFD0 (100, and 0x13A, the
    // next-IFD offset field of these 24 entries), outside the file.
    for at in [0xFFFF_FFFFu32, 100, 0x13A, 0x0FFF_FFFF] {
        check(
            &f1_with_previews(with(f1_entries(), 0x0118, Val::Long(at))),
            v1,
            "data start",
        );
    }
    // R-RW2-22: the data range is not whole chunks inside the file.
    let mut short = good.clone();
    short.truncate(short.len() - 1);
    check(&short, v1, "short chunk");
    // R-RW2-20: V4 data shorter than 16 * NB bytes.
    let e = with_previews(common(28, 2, 12, 128, 6), small.clone(), big.clone());
    let f = rw2(e, f1_blocks()[..63].to_vec());
    check(&f, Some(Class::Sensor(Variant::V4)), "V4 short");
    // R-RW2-53: W * H above 2^27, W above 65535.
    let e = with(
        with(f1_entries(), 0x0002, Val::Long(65534)),
        0x0003,
        Val::Long(4096),
    );
    check(&f1_with_previews(e), v1, "W * H");
    let e = with(f1_entries(), 0x0002, Val::Long(70000));
    check(&f1_with_previews(e), None, "W > 65535");
    // R-RW2-33: black level too close to white.
    check(
        &f1_with_previews(with(f1_entries(), 0x001C, Val::Short(4000))),
        v1,
        "black",
    );

    // R-RW2-49: the next-largest candidate when the largest fails.
    let mut broken = big.clone();
    let sos = broken.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    broken.truncate(sos + 4);
    let x1 = with(f1_entries(), 0x002D, Val::Short(8));
    let f = rw2(
        with_previews(x1.clone(), small.clone(), broken),
        rotate(&f1_blocks()),
    );
    assert_preview(&report(&f), (16, 8), [40, 40, 200], "next-largest");
    // An error only when no candidate decodes.
    let f = rw2(
        with_previews(
            x1.clone(),
            hex("FF D8 00 00 00 00"),
            hex("01 02 03 04 05 06"),
        ),
        rotate(&f1_blocks()),
    );
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    // R-RW2-45: a candidate with count below 4, without FF D8, or outside
    // the file is dropped.
    let e = with(x1.clone(), 0x002E, Val::Raw(7, 3, 0x00D8_FFFF));
    let e = with(e, 0x0127, Val::Raw(7, 1000, 0x0FFF_FFFF));
    let f = rw2(e, rotate(&f1_blocks()));
    assert!(preview::collect(&container(&f)).is_empty());
    let mut not_jpeg = big.clone();
    not_jpeg[1] = 0xD9;
    let f = rw2(
        with_previews(x1.clone(), not_jpeg, small.clone()),
        vec![0; 16],
    );
    let cands = preview::collect(&container(&f));
    assert_eq!(cands.len(), 1);
    assert_eq!((cands[0].width, cands[0].height), (16, 8));
    // R-RW2-47: bytes after EOI, and a stream without EOI.
    let mut padded = big.clone();
    padded.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x12]);
    let f = rw2(
        with_previews(x1.clone(), small.clone(), padded),
        rotate(&f1_blocks()),
    );
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "bytes after EOI");
    let no_eoi = big[..big.len() - 2].to_vec();
    let f = rw2(
        with_previews(x1.clone(), small.clone(), no_eoi),
        rotate(&f1_blocks()),
    );
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "no EOI");
    // R-RW2-48: orientation 6 turns a wide preview, not a tall one.
    let x1o = with(x1.clone(), 0x0112, Val::Short(6));
    let f = rw2(
        with_previews(x1o.clone(), small.clone(), big.clone()),
        rotate(&f1_blocks()),
    );
    assert_preview(&report(&f), (32, 48), [200, 40, 40], "rotated preview");
    let tall = jpeg(32, 48, [200, 40, 40]);
    let f = rw2(
        with_previews(x1o, small.clone(), tall),
        rotate(&f1_blocks()),
    );
    assert_preview(
        &report(&f),
        (32, 48),
        [200, 40, 40],
        "tall preview not rotated",
    );
    // R-RW2-51: the Exif data inside the preview stream are not used: an
    // APP1 Exif segment with Orientation 6 (and a thumbnail-sized target)
    // changes nothing; IFD0 orientation 1 leaves the preview as stored.
    let exif = hex("FF E1 00 22 45 78 69 66 00 00 49 49 2A 00 08 00 00 00 01 00
         12 01 03 00 01 00 00 00 06 00 00 00 00 00 00 00");
    let mut with_exif = big[..2].to_vec();
    with_exif.extend_from_slice(&exif);
    with_exif.extend_from_slice(&big[2..]);
    assert_eq!(
        crate::jpeg::decode_image(&with_exif, &DecodeOptions::default())
            .unwrap()
            .width(),
        32,
        "the plain JPEG decoder turns this stream by its own Exif"
    );
    let stripped = preview::without_exif(&with_exif).unwrap();
    assert_eq!(stripped, big, "APP1 removed, the rest unchanged");
    let f = rw2(
        with_previews(x1.clone(), small.clone(), with_exif),
        rotate(&f1_blocks()),
    );
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "preview Exif ignored");
    let thumb = DecodeOptions {
        target_size: Some((40, 30)),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&f, &thumb).unwrap().unwrap();
    assert_preview(
        &dev,
        (48, 32),
        [200, 40, 40],
        "preview Exif ignored, target",
    );
    // Malformed segments are copied for the JPEG decoder to judge.
    for s in [
        &[0xFFu8, 0xD8][..],
        &[0xFF, 0xD8, 0xFF, 0xE1, 0x00],
        &[0xFF, 0xD8, 0xFF, 0xE1, 0x00, 0x01, 9, 9],
    ] {
        assert_eq!(preview::without_exif(s).unwrap(), s);
    }
    // R-RW2-66: a RAW file without a preview that fails is an error.
    let f = raw(with(f7_entries(), 0x0009, Val::Short(7)), f3_blocks());
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    let f = raw(with(f8_entries(), 0x0117, Val::Long(1)), f8_data());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "X3 without preview"
    );
    // R-RW2-11: an invalid IFD0 is an error (no preview without IFD0).
    let mut f = good.clone();
    f[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    let mut f = good.clone();
    f[24..26].copy_from_slice(&0u16.to_le_bytes());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "N = 0"
    );
    f[24..26].copy_from_slice(&1025u16.to_le_bytes());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "N = 1025"
    );
    assert!(
        decode_report(&f1()[..290], &DecodeOptions::default()).is_err(),
        "cut IFD"
    );
    // R-RW2-52: shorter than 8 bytes.
    assert!(decode_report(&f1()[..7], &DecodeOptions::default()).is_err());
    // R-RW2-13, R-RW2-63: a CameraIFD (0x0120) or Exif IFD pointer that is
    // not an IFD changes nothing.
    let e = with(
        f1_entries(),
        0x0120,
        Val::Undefined(hex("05 6F 71 40 01 02 03 04 05 06 07 08 09 0A 0B 0C")),
    );
    let e = with(e, 0x8769, Val::Long(0xFFFF_FF00));
    let dev = report(&rw2(e, rotate(&f1_blocks())));
    assert_eq!(dev.origin, Origin::Sensor(Variant::V1));
    assert_rgba16_flat(&dev.image, OUT_12, "CameraIFD");

    // Through the registry: by the .rw2 extension, by content without a
    // name, and an older .RAW file by content.
    let src = pandite_core::Source::from_bytes(good.clone(), Some("P1000001.RW2".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba16, "developed");
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (28, 2, "raw"));
    let src = pandite_core::Source::from_bytes(f7(), None);
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(
        img.first_image().format(),
        PixelFormat::Rgba16,
        "RAW by content"
    );
    let src = pandite_core::Source::from_bytes(f8(), Some("P1000002.RAW".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(
        (img.first_image().width(), img.first_image().height()),
        (4, 2)
    );
    let x1f = rw2(with_previews(x1, small, big), rotate(&f1_blocks()));
    let src = pandite_core::Source::from_bytes(x1f, Some("x.rw2".into()));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (48, 32));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
}

#[test]
fn rw2_target_size_uses_preview() {
    use pandite_core::DecoderProvider;
    // A 140 x 64 V4 sensor with previews of 32 x 16 and 64 x 32.
    let (w, h) = (140u16, 64u16);
    let blocks = usize::from(w) * usize::from(h) / 14;
    let data: Vec<u8> = (0..blocks)
        .flat_map(|_| f1_blocks()[..16].to_vec())
        .collect();
    let e = with_previews(
        common(w, h, 12, 128, 6),
        jpeg(32, 16, [10, 200, 10]),
        jpeg(64, 32, [200, 10, 10]),
    );
    let f = rw2(e, data);
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&f, &opts((20, 10))).unwrap().unwrap();
    assert_preview(&dev, (32, 16), [10, 200, 10], "small target");
    let dev = decode_report(&f, &opts((40, 40))).unwrap().unwrap();
    assert_preview(&dev, (64, 32), [200, 10, 10], "medium target");
    let dev = decode_report(&f, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V4));
    assert_eq!((dev.image.width(), dev.image.height()), (140, 64));
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f, Some("t.rw2".into()));
    let img = reg.decode(&src, &opts((16, 16))).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    assert!(crate::raw::RawDecoder.reduced_decode(&src));
}

/// Header facts of every fixture variant, through the provider.
#[test]
fn facts_rw2() {
    use pandite_core::{DecoderProvider, Source};
    let f = super::facts(&f1()).expect("RW2 facts");
    assert_eq!(f.container.as_deref(), Some("Panasonic RW2"));
    assert_eq!(
        f.compression.as_deref(),
        Some("Panasonic 12-bit block code (RawFormat 4)")
    );
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(12)));
    assert_eq!(f.camera.model.as_deref(), Some("DC-GH5"));
    let compression = |data: &[u8]| super::facts(data).and_then(|f| f.compression);
    assert_eq!(
        compression(&f2()).as_deref(),
        Some("Panasonic 14-bit packed (RawFormat 5)")
    );
    assert_eq!(
        compression(&f4()).as_deref(),
        Some("Panasonic 14-bit block code (RawFormat 6)")
    );
    assert_eq!(
        compression(&f8()).as_deref(),
        Some("Uncompressed, 16-bit containers")
    );
    let x1 = rw2(with(f1_entries(), 0x002D, Val::Short(8)), vec![]);
    assert_eq!(
        compression(&x1).as_deref(),
        Some("Panasonic variable-length (RawFormat 8)")
    );
    let x3 = raw(with(f8_entries(), 0x000B, Val::Short(34826)), vec![]);
    assert_eq!(compression(&x3).as_deref(), Some("Panasonic RAW 2"));
    let bggr = rw2(with(f1_entries(), 0x0009, Val::Short(4)), vec![]);
    assert_eq!(
        super::facts(&bggr).unwrap().channels.as_deref(),
        Some("Bayer BGGR")
    );
    let via = crate::raw::RawDecoder
        .facts(&Source::from_bytes(f7(), Some("a.RAW".into())))
        .expect("provider facts");
    assert_eq!(via.container.as_deref(), Some("Panasonic RW2"));
    assert_eq!(
        via.compression.as_deref(),
        Some("Panasonic 12-bit packed (RawFormat 2)")
    );
    let f = f1();
    for n in (0..600).step_by(7) {
        let _ = super::facts(&f[..n]);
    }
}

// ---------------------------------------------------------------------------
// Robustness
// ---------------------------------------------------------------------------

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let small = jpeg(16, 8, [40, 40, 200]);
    let big = jpeg(48, 32, [200, 40, 40]);
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F5", f5()),
        ("F6", f6()),
        ("F7", f7()),
        ("F8", f8()),
        (
            "F1+previews",
            f1_with_previews(with(f1_entries(), 0x0112, Val::Short(6))),
        ),
        (
            "F5+previews",
            rw2(
                with_previews(common(28, 2, 12, 128, 6), small.clone(), big.clone()),
                f1_blocks(),
            ),
        ),
        (
            "F4+previews",
            rw2(
                with_previews(common(22, 2, 14, 497, 6), small.clone(), big.clone()),
                f4_blocks(),
            ),
        ),
        (
            "F8+previews",
            raw(with_previews(f8_entries(), small, big), f8_data()),
        ),
    ]
}

#[test]
fn rw2_hostile_input_never_panics() {
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
                // Damage the header, IFD0 and its values.
                0 | 1 => {
                    let limit = bad.len().min(if round % 2 == 0 { 320 } else { 2000 });
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
                    let i = (next() as usize % bad.len().min(320)) & !1;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27, 8184][next() as usize % 6];
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&v.to_le_bytes());
                    }
                }
            }
            let r = catch_unwind(AssertUnwindSafe(|| {
                let _ = probe(&bad);
                let _ = super::facts(&bad);
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
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.rw2".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized and lying claims are refused before allocation (R-RW2-53,
    // R-RW2-54, R-RW2-55, R-RW2-56).
    for (w, h) in [
        (65535u32, 65535u32),
        (0, 2),
        (28, 0),
        (70000, 2),
        (65534, 2048 + 1),
    ] {
        let e = with(
            with(f1_entries(), 0x0002, Val::Long(w)),
            0x0003,
            Val::Long(h),
        );
        let f = rw2(e, rotate(&f1_blocks()));
        let c = container(&f);
        assert!(meta::plan(&c, Variant::V1).is_err(), "{w} x {h}");
        assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    }
    // R-RW2-64: no mutation panicked; failures were error values.
    // A tag whose count * size exceeds the file (R-RW2-54): an unreadable
    // required tag fails, an unreadable optional tag takes its default.
    let e = with(f1_entries(), 0x0118, Val::Raw(4, 0x4000_0000, 0x200));
    assert!(meta::plan(&container(&rw2(e, rotate(&f1_blocks()))), Variant::V1).is_err());
    let e = with(f1_entries(), 0x0112, Val::Raw(3, 0x4000_0000, 0x200));
    assert_eq!(container(&rw2(e, rotate(&f1_blocks()))).orientation(), 1);
    // A lying V8 byte count: the data range check still refuses it.
    let f = raw(
        with(f8_entries(), 0x0117, Val::Long(1 << 30)),
        f8_data()[..8].to_vec(),
    );
    assert_eq!(meta::classify(&container(&f)), Class::Sensor(Variant::V8));
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
}

#[test]
fn rw2_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 7, 8, 9, 24, 26, 294, f.len() - 1, f.len() - 2]);
        for cut in cuts.into_iter().filter(|&c| c < f.len()) {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => {
                    // The sensor data of every fixture end the file, so a
                    // cut file never develops (R-RW2-20, R-RW2-22).
                    assert_eq!(dev.origin, Origin::Preview, "{name} cut at {cut}")
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large V4 file: 3010 x 3000 pixels of repeated blocks.
fn large_v4() -> Vec<u8> {
    let (w, h) = (3010u16, 3000u16);
    let blocks = usize::from(w) * usize::from(h) / 14;
    let tv3 = hex("03 FF 64 03 FE 2B 08 28 83 1C 20 9E 20 D0 87 3E");
    let mut data = Vec::with_capacity(blocks * 16);
    for _ in 0..blocks {
        data.extend_from_slice(&tv3);
    }
    rw2(common(w, h, 12, 128, 6), data)
}

#[test]
fn rw2_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f1(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f1(), Some("a.rw2".into()));
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
    assert_eq!((dev.image.width(), dev.image.height()), (3010, 3000));
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

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// The camera vectors of TV-5, TV-6 and TV-7: file name, stored row, first
/// column and the expected values, checked on the real file when present.
const CAMERA_VECTORS: &[(&str, usize, usize, &[u16])] = &[
    (
        "DC-GH5_1x1.RW2",
        0,
        0,
        &[
            198, 256, 192, 259, 192, 244, 184, 251, 189, 224, 191, 253, 198, 254,
        ],
    ),
    (
        "DC-GH5_1x1.RW2",
        1952,
        2604,
        &[
            153, 176, 158, 177, 156, 185, 159, 185, 166, 176, 163, 178, 161, 187,
        ],
    ),
    (
        "DC-GH5S_14bit_1x1.RW2",
        1000,
        1404,
        &[778, 1428, 758, 1358, 761, 1270, 737, 1327, 763],
    ),
    (
        "DC-G9_HighRes_secondary_4x3.RW2",
        1000,
        2640,
        &[164, 229, 162, 226, 167, 234, 163, 231, 165, 241],
    ),
    (
        "DC-S5_LiveComposite_3x2.RW2",
        2000,
        3024,
        &[1692, 1904, 1706, 1909, 1729, 1900, 1683, 1885, 1701],
    ),
    (
        "DMC-FZ8_4x3.RAW",
        1000,
        1560,
        &[959, 2070, 981, 1957, 968, 1895, 954, 1930, 973, 1912],
    ),
    (
        "DC-S1_3x2.RW2",
        2000,
        3003,
        &[
            1596, 1016, 1679, 1014, 1529, 1004, 1625, 1008, 1604, 981, 1587,
        ],
    ),
];

/// Tolerances of the comparison with the camera's own JPEG, one set for
/// every RW2 sample, nothing fitted per file: mean chromaticity distance
/// at most 0.08 and largest cell at most 0.20 (the Sony test allows 0.05
/// and 0.15), with the Sony rank correlation (0.90) and minimum of 16
/// usable cells. The limits are wider than the Sony ones because several
/// models have no row in the color table and use the generic matrix
/// (OQ-9: DC-G9, DC-GH5S), and because the shared display rendering adds
/// to the distance (DC-S1: 0.0295 and 0.0994 colorimetric, 0.0593 and
/// 0.1498 displayed). The Fujifilm decoder uses the same set for the same
/// reason. A negative control (the CFA read one column off) must fail them.
const RW2_CHROMA_MEAN: f64 = 0.08;
const RW2_CHROMA_MAX: f64 = 0.20;
/// Fewest usable cells of a comparison (the Sony value).
const MIN_CELLS: usize = 16;

/// True when a comparison (mean, max, rank, usable cells) lies inside the
/// RW2 tolerances.
fn within(mean: f64, max: f64, rank: f64, used: usize) -> bool {
    mean <= RW2_CHROMA_MEAN && max <= RW2_CHROMA_MAX && rank >= RANK_MIN && used >= MIN_CELLS
}

/// Every `.RW2` and `.RAW` file in the folder named by
/// `PANDITE_RW2_SAMPLES` (absolute, or relative to the workspace root)
/// decodes through the sensor-data path when its variant is in scope, at
/// the size of the active area, and agrees with the file's own largest
/// embedded JPEG by the method of the Sony test
/// `arw_real_samples_match_embedded_preview`: the default display
/// rendering on an 8 x 8 grid of cell means, exposure matched by one gain,
/// rg chromaticity distance per usable cell and a Spearman rank
/// correlation of cell luminance; files without as-shot white balance are
/// compared after one global gain per channel. The tolerances are
/// [`RW2_CHROMA_MEAN`], [`RW2_CHROMA_MAX`], `RANK_MIN` and [`MIN_CELLS`].
///
/// Negative control: every in-scope sample with an embedded JPEG is also
/// developed with a wrong CFA reading (the pattern shifted by one column,
/// so red and blue pixels are read as green and green as red or blue) and
/// must fail the same tolerances; the test fails when a control passes.
///
/// RAW files (V7, V8) carry no embedded JPEG (M13, OQ-16): they must
/// develop, and are reported without a comparison. Out-of-scope files must
/// return the preview. The camera vectors of TV-5 to TV-7 are checked on
/// the files they come from.
#[test]
fn rw2_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_RW2_SAMPLES") else {
        println!("skip: PANDITE_RW2_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("rw2") || e.eq_ignore_ascii_case("raw"))
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "no .RW2 or .RAW files in {}",
        folder.display()
    );
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let class = meta::classify(&c);
        let start = Instant::now();
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("RW2 path");
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
        assert_eq!(
            dev.origin,
            Origin::Sensor(variant),
            "{name} fell back to the preview"
        );
        let info = dev.info.clone().expect("raw facts");
        let plan = meta::plan(&c, variant).expect("plan");
        // The specified size: the active area, oriented.
        let (aw, ah) = info.active.size();
        let (ow, oh) = if info.orientation >= 5 {
            (ah, aw)
        } else {
            (aw, ah)
        };
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (ow, oh),
            "{name}"
        );
        // The camera vectors on the real file (TV-5 to TV-7).
        for &(file, y, x0, want) in CAMERA_VECTORS {
            if file != name {
                continue;
            }
            let mut row = vec![0u16; plan.width];
            sensor::decode_row(&data, &plan, y, &mut row).unwrap();
            assert_eq!(
                &row[x0..x0 + want.len()],
                want,
                "{name} row {y} column {x0}"
            );
        }
        let a = info.active;
        let mut line = format!(
            "{name}: model {} {variant:?} raster {}x{} active {}x{}+{}+{} out {}x{} black {:?} white {:?} wb {:?} {:.3}/{:.3} matrix {} decode {:.2}s (debug)",
            info.model,
            info.raster.0,
            info.raster.1,
            aw,
            ah,
            a.left,
            a.top,
            ow,
            oh,
            plan.black,
            plan.white,
            info.wb,
            plan.wb[0],
            plan.wb[2],
            if info.matrix_from_table { "table" } else { "generic" },
            took.as_secs_f64(),
        );
        let cands = preview::collect(&c);
        let Some(best) = cands.first() else {
            // M13, OQ-16: the RAW files have no embedded JPEG; R-RW2-66.
            assert!(variant.legacy(), "{name}: no embedded preview");
            line.push_str(" | no embedded JPEG: no comparison");
            println!("{line}");
            continue;
        };
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev = preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        let balance = matches!(info.wb, WbSource::Table | WbSource::Generic);
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        line.push_str(&format!(
            " | vs embedded {}x{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}{}",
            best.width,
            best.height,
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        ));
        if !within(mean, max, rank, used) {
            failures.push(line.clone());
        }
        // The negative control: the CFA colors of the plan shifted by one
        // column (position 2 * y + x takes the color of 2 * y + (x + 1) mod
        // 2), developed with the same display rendering.
        let mut wrong = plan.clone();
        wrong.colors = [0, 1, 2, 3].map(|k| plan.colors[(k & 2) | ((k + 1) & 1)]);
        let control = develop(&data, &wrong, &CancelToken::new(), Output::Display8)
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

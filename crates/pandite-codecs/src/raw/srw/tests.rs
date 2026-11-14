//! Tests of the SRW decoder against the specification's test vectors
//! (TV-1 to TV-8), fixtures (F1 to F5) and requirements, plus hostile
//! input, cancellation and real camera files.

use super::fixtures::*;
use super::meta::{self, Class, MatrixSource, Plan, Relative, Variant, WbSource};
use super::*;
use crate::raw::arw::render;
use crate::raw::arw::tests::{agreement, grid_means, LinearF32, CHROMA_MAX, CHROMA_MEAN, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// TV-7 and F1, F2: the expected linear sRGB of every output pixel.
const TV7: [f32; 3] = [0.737_82, 0.360_820_9, 0.208_661];
/// F3: the expected linear sRGB of every output pixel.
const F3_OUT: [f32; 3] = [0.738_915, 0.360_592_3, 0.209];
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
    meta::plan(&c, v).expect("plan")
}

/// The valid-area plane as row-major values, as read.
fn sensor_values(data: &[u8], plan: &Plan) -> Vec<u16> {
    sensor::decode(data, plan, sensor::Domain::AsRead, &CancelToken::new())
        .expect("decode sensor")
        .data
}

/// Linear sRGB of every output pixel (R-SRW-38), through the shared
/// renderer, oriented and cropped.
fn linear_with(data: &[u8], plan: &Plan) -> (usize, usize, Vec<[f32; 3]>) {
    let (rplan, domain) = develop::bridge(plan);
    let plane = sensor::decode(data, plan, domain, &CancelToken::new()).expect("sensor");
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
        .expect("SRW path")
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

/// A synthetic file of `w` x `h` values with the recipe's maker note.
struct Synth {
    w: u16,
    h: u16,
    values: Vec<u16>,
    variant: Variant,
    le_tiff: bool,
    /// Maker-note tag 0x0040 (None: the unused tag 0x0041).
    order: Option<u32>,
    areas: [u32; 8],
    wb: [i64; 4],
    black: [i64; 4],
    white: i64,
    model: &'static str,
    orientation: u16,
    cfa: [u8; 4],
    /// Rows per strip (all strips in one blob, in order).
    rows: u16,
}

impl Synth {
    fn new(w: u16, h: u16, values: Vec<u16>, variant: Variant) -> Self {
        Synth {
            w,
            h,
            values,
            variant,
            le_tiff: false,
            order: None,
            areas: [
                0,
                0,
                u32::from(w),
                u32::from(h),
                0,
                0,
                u32::from(w),
                u32::from(h),
            ],
            wb: [9776, 4096, 4096, 7232],
            black: [3; 4],
            white: if variant == Variant::V1 { 16383 } else { 4095 },
            model: "NX210",
            orientation: 1,
            cfa: [1, 0, 2, 1],
            rows: h,
        }
    }

    fn data_le(&self) -> bool {
        match self.order {
            Some(0) => true,
            Some(_) => false,
            None => self.le_tiff,
        }
    }

    fn builder(&self) -> Builder {
        let strip = match self.variant {
            Variant::V1 => pack16(&self.values, self.data_le()),
            Variant::V2 => pack12(&self.values, self.data_le()),
        };
        let (bits, compression, row) = match self.variant {
            Variant::V1 => (14, 32769, 2 * usize::from(self.w)),
            Variant::V2 => (12, 32770, 3 * usize::from(self.w) / 2),
        };
        let mut raw = raw_ifd(self.w, self.h, bits, compression, 0);
        raw = with(raw, 0x828E, Val::Byte(self.cfa.to_vec()));
        if self.rows != self.h {
            // Several strips: offsets and counts into the one blob.
            let n = usize::from(self.h).div_ceil(usize::from(self.rows));
            let mut blobs = Vec::new();
            for s in 0..n {
                let r =
                    usize::from(self.rows).min(usize::from(self.h) - s * usize::from(self.rows));
                blobs.push(strip[s * usize::from(self.rows) * row..][..r * row].to_vec());
            }
            raw = with(raw, 0x0116, Val::Short(vec![self.rows]));
            raw = with(raw, 0x0111, Val::Blobs((0..n).collect()));
            raw = with(raw, 0x0117, Val::BlobLens((0..n).collect()));
            return Builder {
                le: self.le_tiff,
                ifd0: ifd0(self.model, self.orientation),
                maker: maker(self.order, self.areas, self.wb, self.black, self.white),
                raw,
                blobs,
                ..Builder::default()
            };
        }
        Builder {
            le: self.le_tiff,
            ifd0: ifd0(self.model, self.orientation),
            maker: maker(self.order, self.areas, self.wb, self.black, self.white),
            raw,
            blobs: vec![strip],
            ..Builder::default()
        }
    }

    fn build(&self) -> Vec<u8> {
        self.builder().build()
    }
}

/// `n` pseudo-random values below `max`.
fn random_values(n: usize, max: u32, seed: u32) -> Vec<u16> {
    let mut next = noise(seed);
    (0..n).map(|_| (next() % max) as u16).collect()
}

/// R-SRW-43: the output pixel of stored pixel (x, y) of a W' x H' image,
/// written from the specification's table.
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

/// An independent per-pixel implementation of R-SRW-23, R-SRW-34,
/// R-SRW-39, R-SRW-40 and R-SRW-37 in f64 on the stored raster: camera
/// RGB of stored pixel (x, y) of the valid area, through the matrix.
fn reference<'a>(
    values: &'a [u16],
    w: usize,
    plan: &'a Plan,
) -> impl Fn(usize, usize) -> [f64; 3] + 'a {
    move |x, y| {
        let v = plan.valid;
        let color = |x: usize, y: usize| usize::from(plan.cfa[2 * (y % 2) + x % 2]);
        let norm = |x: usize, y: usize| {
            let c = color(x, y);
            let s = f64::from(values[y * w + x]);
            let b = plan.black[c];
            ((s - b) / (f64::from(plan.white) - b) * plan.wb[c]).clamp(0.0, 1.0)
        };
        let own = color(x, y);
        let mut sum = [0f64; 3];
        let mut cnt = [0u32; 3];
        for dy in -1i64..=1 {
            for dx in -1i64..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let (xx, yy) = (x as i64 + dx, y as i64 + dy);
                let inside = xx >= v.x as i64
                    && xx < (v.x + v.w) as i64
                    && yy >= v.y as i64
                    && yy < (v.y + v.h) as i64;
                if !inside {
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

/// The rendered linear image equals the reference at every pixel.
fn assert_reference(data: &[u8], values: &[u16], w: usize, what: &str) {
    let plan = plan_of(data);
    let (ow, oh, px) = linear_with(data, &plan);
    let v = plan.valid;
    assert_eq!(
        (ow, oh),
        render::output_size(v, plan.orientation),
        "{what}: R-SRW-41 size"
    );
    let r = reference(values, w, &plan);
    for y in v.y..v.y + v.h {
        for x in v.x..v.x + v.w {
            let (ox, oy) = oriented(plan.orientation, x - v.x, y - v.y, v.w, v.h);
            let want = r(x, y).map(|c| c as f32);
            assert_rgb(px[oy * ow + ox], want, 2e-6, &format!("{what} ({x}, {y})"));
        }
    }
}

// ---------------------------------------------------------------------------
// Vectors and fixtures
// ---------------------------------------------------------------------------

#[test]
fn srw_builder_reproduces_fixture_hex() {
    // The builder follows the recipe layout byte for byte, so the other
    // synthetic files share the layout of F1 to F3.
    assert_eq!(f1().len(), 600);
    assert_eq!(f3().len(), 624);
    assert_eq!(recipe(false, false, None).build(), f1(), "F1");
    assert_eq!(recipe(false, false, Some(0)).build(), f2(), "F2");
    assert_eq!(recipe(false, true, Some(0)).build(), f3(), "F3");
}

#[test]
fn srw_vectors_decode() {
    // TV-1 (R-SRW-4, R-SRW-5): the two headers, and another start.
    assert_eq!(
        ifd::header(&hex("4D 4D 00 2A 00 00 00 08")).unwrap(),
        (false, 8)
    );
    assert_eq!(
        ifd::header(&hex("49 49 2A 00 08 00 00 00")).unwrap(),
        (true, 8)
    );
    assert!(ifd::header(&hex("4D 4D 2A 00 00 00 00 08")).is_err());
    // R-SRW-4: an error without a preview, even with a JPEG inside.
    let mut bad = hex("4D 4D 2A 00 00 00 00 08");
    bad.extend(jpeg(16, 8, [200, 40, 40]));
    assert!(decode_report(&bad, &DecodeOptions::default()).is_err());
    assert!(probe(&bad).is_err());
    // R-SRW-50: shorter than 8 bytes.
    assert!(ifd::header(&hex("4D 4D 00 2A 00 00 00")).is_err());

    // TV-2: big-endian entries of the NX10 raw IFD.
    let mut d = hex("4D 4D 00 2A 00 00 00 08 00 04");
    d.extend(hex("01 03 00 03 00 00 00 01 80 02 00 00
         01 11 00 04 00 00 00 01 00 63 82 F0
         01 17 00 04 00 00 00 01 01 50 59 40
         82 8E 00 01 00 00 00 04 02 01 01 00
         00 00 00 00"));
    let dir = ifd::dir(&d, false, 8, 0).expect("IFD");
    assert_eq!(dir.uint(0x0103).unwrap(), Some(32770));
    assert_eq!(dir.uint(0x0111).unwrap(), Some(6_521_584));
    assert_eq!(dir.uint(0x0117).unwrap(), Some(22_042_944));
    assert_eq!(22_042_944, 3 * 4704 * 3124 / 2);
    assert_eq!(dir.byte_values(0x828E).unwrap(), Some(&[2u8, 1, 1, 0][..]));
    // TV-2: little-endian NX3000 IFD0 entries, unsorted, with the all-zero
    // entry (R-SRW-7).
    let mut d = hex("49 49 2A 00 08 00 00 00 03 00");
    d.extend(hex("69 87 04 00 01 00 00 00 A6 00 00 00
         4A 01 04 00 02 00 00 00 9E 00 00 00
         00 00 00 00 00 00 00 00 00 00 00 00
         00 00 00 00"));
    d.resize(200, 0);
    d[158..166].copy_from_slice(&hex("40 00 00 00 80 00 00 00"));
    let dir = ifd::dir(&d, true, 8, 0).expect("IFD");
    assert_eq!(dir.entries.len(), 2, "type 0 skipped");
    assert_eq!(dir.uint(0x8769).unwrap(), Some(166));
    let e = dir.entry(0x014A).expect("SubIFDs");
    assert_eq!((e.count, e.at), (2, 158));
    assert_eq!(dir.uints(0x014A).unwrap(), Some(vec![64, 128]));
    // R-SRW-6: inline when count * size <= 4, else at the offset.
    assert_eq!(dir.entry(0x8769).unwrap().at, 8 + 2 + 8);
    // R-SRW-11: invalid IFDs.
    assert!(ifd::dir(&d, true, 200, 0).is_none(), "outside the file");
    assert!(ifd::dir(&d, true, 190, 0).is_none(), "entries past the end");
    let mut zero = d.clone();
    zero[8] = 0;
    assert!(ifd::dir(&zero, true, 8, 0).is_none(), "N = 0");
    zero[8..10].copy_from_slice(&1025u16.to_le_bytes());
    zero.resize(20_000, 0);
    assert!(ifd::dir(&zero, true, 8, 0).is_none(), "N > 1024");
    // R-SRW-7: the first entry of a tag number is used.
    let mut d = hex("4D 4D 00 2A 00 00 00 08 00 02");
    d.extend(hex("01 12 00 03 00 00 00 01 00 06 00 00
         01 12 00 03 00 00 00 01 00 03 00 00
         00 00 00 00"));
    assert_eq!(
        ifd::dir(&d, false, 8, 0).unwrap().uint(0x0112).unwrap(),
        Some(6)
    );

    // TV-3: maker note and decryption (S10 values), in a big-endian file.
    let tv3 = Builder {
        ifd0: ifd0("NX10", 1),
        maker: vec![
            (
                0xA010,
                Val::Raw(
                    4,
                    8,
                    hex("00 00 00 00 00 00 00 00 00 00 12 50 00 00 0C 34 00 00 00 15 00 00 00 0E 00 00 12 05 00 00 0B FE"),
                ),
            ),
            (
                0xA020,
                Val::Raw(
                    4,
                    11,
                    hex("00 00 01 31 00 00 00 48 00 00 02 E1 00 00 01 C8 00 00 01 1A 00 00 01 33 00 00 02 07 00 00 02 D4 00 00 00 0D 00 00 01 F9 00 00 00 C1"),
                ),
            ),
            (
                0xA021,
                Val::Raw(4, 4, hex("00 00 1C 4C 00 00 10 48 00 00 12 E1 00 00 12 37")),
            ),
            (
                0xA028,
                Val::Raw(9, 4, hex("00 00 01 34 00 00 00 4B 00 00 02 E4 00 00 01 CB")),
            ),
            (
                0xA031,
                Val::Raw(
                    9,
                    9,
                    hex("00 00 00 67 FF FF FF 46 FF FF FC F9 FF FF FE 16 00 00 00 42 FF FF FE 93 FF FF FD F1 FF FF FC C4 00 00 01 63"),
                ),
            ),
            (
                0xA048,
                Val::Raw(
                    4,
                    12,
                    hex("FF FF FE CF FF FF FF B8 FF FF FD 1F FF FF FE 38 00 00 0E E5 00 00 0E CC 00 00 0D F8 00 00 0D 2B 00 00 0F F2 00 00 0E 06 00 00 0F 3E 00 00 0E CE"),
                ),
            ),
        ],
        ..Builder::default()
    }
    .build();
    let c = container(&tv3);
    let mk = c.maker.as_ref().expect("maker note");
    assert_eq!(mk.entries.len(), 6);
    assert_eq!(
        mk.longs(0xA010).unwrap(),
        Some(vec![0, 0, 4688, 3124, 21, 14, 4613, 3070])
    );
    let key = meta::key(mk).unwrap().expect("key");
    assert_eq!(key, KEY.map(i64::from));
    let dec = |t: u16, s: i64| meta::decrypt(mk, Some(&key), t, s, 0).unwrap().unwrap();
    assert_eq!(dec(0xA021, -1), vec![6939, 4096, 4096, 4207]);
    assert_eq!(dec(0xA028, -1), vec![3, 3, 3, 3]);
    assert_eq!(
        mk.signed(0xA031).unwrap().unwrap(),
        vec![103, -186, -775, -490, 66, -365, -527, -828, 355]
    );
    assert_eq!(
        dec(0xA031, 1),
        vec![408, -114, -38, -34, 348, -58, -8, -104, 368]
    );
    let mut white = vec![0; 4];
    white.extend([4095; 8]);
    assert_eq!(dec(0xA048, 1), white);
    // R-SRW-20: a LONG element is read as a signed 32-bit value.
    assert_eq!(mk.signed(0xA048).unwrap().unwrap()[0], -305);
    assert_eq!(mk.longs(0xA048).unwrap().unwrap()[0], 0xFFFF_FECF);
    // TV-3 Exif entry: P = 432 for 98179 bytes, a value outside this small
    // file (R-SRW-53).
    let mut d = hex("4D 4D 00 2A 00 00 00 08 00 01");
    d.extend(hex("92 7C 00 07 00 01 7F 83 00 00 01 B0 00 00 00 00"));
    let dir = ifd::dir(&d, false, 8, 0).unwrap();
    let e = dir.entry(0x927C).unwrap();
    assert_eq!((e.typ, e.count, e.at), (7, 98179, 432));
    assert!(dir.bytes(e).is_err());

    // TV-4: V2 groups (R-SRW-27, R-SRW-28).
    let px = |b: &str, le: bool| -> Vec<u16> {
        let row = hex(b);
        (0..row.len() / 3 * 2)
            .map(|x| sensor::v2_pixel(&row, x, le).unwrap())
            .collect()
    };
    assert_eq!(px("21 82 37 18 B1 6D", false), [536, 567, 395, 365]);
    assert_eq!(px("FF F5 DC", false), [4095, 1500]);
    assert_eq!(px("25 85 DC", false), [600, 1500]);
    assert_eq!(px("1A B5 18 00 C5 17", true), [1306, 395, 1280, 380]);
    assert_eq!(px("FF CF 5D", true), [4095, 1500]);
    assert_eq!(px("58 C2 5D", true), [600, 1500]);
    // TV-5: V1 samples (R-SRW-26).
    let row = hex("21 0B 70 14 7D 0B D2 15");
    let v: Vec<u16> = (0..4)
        .map(|x| sensor::v1_pixel(&row, x, true).unwrap())
        .collect();
    assert_eq!(v, [2849, 5232, 2941, 5586]);
    assert_eq!(sensor::v1_pixel(&hex("70 17"), 0, true), Some(6000));
    assert_eq!(sensor::v1_pixel(&hex("A0 0F"), 0, true), Some(4000));
    assert_eq!(sensor::v1_pixel(&hex("60 09"), 0, true), Some(2400));
    assert_eq!(sensor::v1_pixel(&hex("17 70"), 0, false), Some(6000));

    // TV-6: classification (R-SRW-3) from the tags alone.
    let class = |compression: u16, w: u32, h: u32, bytes: u32| {
        let raw = vec![
            (0x0100, Val::Long(vec![w])),
            (0x0101, Val::Long(vec![h])),
            (0x0103, Val::Short(vec![compression])),
            (0x0111, Val::Long(vec![8])),
            (0x0117, Val::Long(vec![bytes])),
        ];
        let f = Builder {
            ifd0: ifd0("NX", 1),
            raw,
            ..Builder::default()
        }
        .build();
        meta::classify(&container(&f)).unwrap()
    };
    use Class::{OutOfScope, Sensor};
    assert_eq!(class(32770, 4704, 3124, 22_042_944), Sensor(Variant::V2));
    assert_eq!(
        class(32770, 5546, 3714, 18_537_424),
        OutOfScope(Relative::X1)
    );
    assert_eq!(class(32769, 3688, 2780, 20_505_280), Sensor(Variant::V1));
    assert_eq!(class(32772, 5600, 3714, 1), OutOfScope(Relative::X2));
    assert_eq!(class(32773, 6496, 4336, 1), OutOfScope(Relative::X3));
    // Rule 2 needs at least 2 * W * H bytes; rule 3 an even W.
    assert_eq!(class(32769, 8, 6, 95), OutOfScope(Relative::X4));
    assert_eq!(class(32769, 8, 6, 200), Sensor(Variant::V1));
    assert_eq!(class(32770, 7, 6, 63), OutOfScope(Relative::X1));
    assert_eq!(class(1, 8, 6, 96), OutOfScope(Relative::X4));
    assert_eq!(class(7, 8, 6, 96), OutOfScope(Relative::X4));
    // BitsPerSample: absent, 12 or 14 for V1; absent or 12 for V2.
    let bits = |compression: u16, bits: u16, bytes: u32| {
        let raw = vec![
            (0x0100, Val::Long(vec![8])),
            (0x0101, Val::Long(vec![6])),
            (0x0102, Val::Short(vec![bits])),
            (0x0103, Val::Short(vec![compression])),
            (0x0111, Val::Long(vec![8])),
            (0x0117, Val::Long(vec![bytes])),
        ];
        let f = Builder {
            ifd0: ifd0("NX", 1),
            raw,
            ..Builder::default()
        }
        .build();
        meta::classify(&container(&f)).unwrap()
    };
    assert_eq!(bits(32769, 12, 96), Sensor(Variant::V1));
    assert_eq!(bits(32769, 16, 96), OutOfScope(Relative::X4));
    assert_eq!(bits(32770, 14, 72), OutOfScope(Relative::X1));
    // Rule 1 (X5): no SubIFD qualifies as the raw IFD (R-SRW-12).
    let no_raw = Builder {
        ifd0: ifd0("NX", 1),
        raw: vec![(0x0100, Val::Long(vec![8])), (0x0101, Val::Long(vec![6]))],
        ..Builder::default()
    }
    .build();
    assert_eq!(
        meta::classify(&container(&no_raw)).unwrap(),
        OutOfScope(Relative::X5)
    );

    // F1 (V2, big-endian groups from the big-endian header): 6 x 4 pixels,
    // every one TV-7. Every pixel outside the valid area is 4095, so a
    // decoder that does not crop or that reads outside gets other edges.
    let f = f1();
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V2));
    let p = plan_of(&f);
    assert!(!p.data_le, "R-SRW-25: no tag 0x0040, big-endian header");
    assert_eq!(p.cfa, [1, 0, 2, 1]);
    assert_eq!((p.valid.x, p.valid.y, p.valid.w, p.valid.h), (1, 1, 6, 4));
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (6, 4));
    for (i, &v) in px.iter().enumerate() {
        assert_rgb(v, TV7, EPS, &format!("F1 pixel {i}"));
    }
    let dev = report(&f);
    assert_eq!((dev.image.width(), dev.image.height()), (6, 4));
    assert_rgba16_flat(&dev.image, TV7, "F1");
    assert_eq!(dev.class, Some(Sensor(Variant::V2)));
    // The stored values of the valid area (R-SRW-29: as read): row 1 is
    // green and blue, row 2 red and green.
    let vals = sensor_values(&f, &p);
    assert_eq!(&vals[..6], &[1500, 600, 1500, 600, 1500, 600]);
    assert_eq!(&vals[6..12], &[1000, 1500, 1000, 1500, 1000, 1500]);

    // F2 (V2, little-endian groups, tag 0x0040 = 0): identical output.
    let f = f2();
    let p = plan_of(&f);
    assert!(p.data_le);
    let (_, _, px) = linear(&f);
    for v in px {
        assert_rgb(v, TV7, EPS, "F2");
    }
    // Read with big-endian groups, F2 gives other values.
    let mut wrong = p.clone();
    wrong.data_le = false;
    let (_, _, px) = linear_with(&f, &wrong);
    assert!(
        px.iter().any(|v| (v[0] - TV7[0]).abs() > 0.01),
        "F2 as big-endian"
    );

    // F3 (V1, little-endian 16-bit samples): white 16383, black 12.
    let f = f3();
    let p = plan_of(&f);
    assert_eq!(p.variant, Variant::V1);
    assert_eq!(p.white, 16383);
    assert_eq!(p.black, [12.0; 3]);
    assert!((p.wb[0] - 2.390_793_339_9).abs() < 1e-9);
    assert!((p.wb[2] - 1.767_874_632_7).abs() < 1e-9);
    let (w, h, px) = linear(&f);
    assert_eq!((w, h), (6, 4));
    for v in px {
        assert_rgb(v, F3_OUT, EPS, "F3");
    }
    assert_rgba16_flat(&report(&f).image, F3_OUT, "F3");
    assert_eq!(&sensor_values(&f, &p)[..2], &[6000, 2400]);

    // F4 (X2) and F5 (X1): preview path without a candidate, so an error
    // (R-SRW-48).
    for (name, f, class) in [("F4", f4(), Relative::X2), ("F5", f5(), Relative::X1)] {
        assert_eq!(
            meta::classify(&container(&f)).unwrap(),
            OutOfScope(class),
            "{name}"
        );
        assert!(
            decode_report(&f, &DecodeOptions::default()).is_err(),
            "{name}"
        );
    }

    // The whole recipe in a little-endian file: the maker note in the header
    // byte order (R-SRW-5) and, without tag 0x0040, little-endian groups
    // (R-SRW-25).
    for (order, le) in [(None, true), (Some(0), true), (Some(1), false)] {
        let f = recipe(true, false, order).build();
        assert_eq!(&f[..4], &[0x49, 0x49, 0x2A, 0x00]);
        let p = plan_of(&f);
        assert_eq!(p.data_le, le, "{order:?}");
        for v in linear(&f).2 {
            assert_rgb(v, TV7, EPS, "little-endian F1");
        }
    }
    // R-SRW-25: a value other than 0 or 1 uses the preview path.
    let f = recipe(false, false, Some(2)).build();
    assert!(meta::plan(&container(&f), Variant::V2).is_err());
    // V1 with big-endian samples (tag 0x0040 = 1).
    for v in linear(&recipe(false, true, Some(1)).build()).2 {
        assert_rgb(v, F3_OUT, EPS, "F3 big-endian samples");
    }

    // R-SRW-24: several strips, the last one shorter.
    let values = random_values(16 * 10, 4096, 7);
    let mut s = Synth::new(16, 10, values.clone(), Variant::V2);
    s.rows = 4;
    s.areas = [0, 0, 16, 10, 3, 1, 15, 9];
    let f = s.build();
    let p = plan_of(&f);
    assert_eq!((p.strips.len(), p.rows_per_strip), (3, 4));
    assert_reference(&f, &values, 16, "three strips");
    // R-SRW-18: RowsPerStrip 0 or greater than H means H.
    for rps in [0u16, 11] {
        let mut b = Synth::new(16, 10, values.clone(), Variant::V2).builder();
        b.raw = with(b.raw, 0x0116, Val::Short(vec![rps]));
        assert_eq!(plan_of(&b.build()).rows_per_strip, 10, "{rps}");
    }
}

#[test]
fn srw_levels_wb_matrix_vector() {
    // TV-7 and the worked example of R-SRW-34.
    let (m, src) = meta::multipliers(Some(&[9776, 4096, 4096, 7232]), [3.0; 3]);
    assert_eq!(src, WbSource::AsShot);
    assert!((m[0] - 2.387_735_157_6).abs() < 1e-9, "{m:?}");
    assert_eq!(m[1], 1.0);
    assert!((m[2] - 1.766_186_171_5).abs() < 1e-9, "{m:?}");
    let n = |s: f64, k: f64| ((s - 3.0) / (4095.0 - 3.0) * k).clamp(0.0, 1.0);
    let v = [n(1000.0, m[0]), n(1500.0, 1.0), n(600.0, m[2])];
    for (g, w) in v.iter().zip([0.581_762_5, 0.365_835_8, 0.257_676_7]) {
        assert!((g - w).abs() < 5e-8, "{v:?}");
    }
    // R-SRW-35, R-SRW-36, R-SRW-37: the matrix of TV-7, normalized by its
    // row sums (256).
    let mx = [430, -152, -22, -36, 352, -60, 4, -128, 380];
    let (mat, src) = meta::matrix(Some(&mx), None, b"");
    assert_eq!(src, MatrixSource::Srgb);
    assert_eq!(mat[0], [1.679_687_5, -0.593_75, -0.085_937_5]);
    let out: Vec<f64> = mat
        .iter()
        .map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        .collect();
    for (g, w) in out.iter().zip(TV7) {
        assert!((g - f64::from(w)).abs() < 1e-6, "TV-7 {out:?}");
    }
    // The S10 worked example of R-SRW-33.
    let (m, _) = meta::multipliers(Some(&[6939, 4096, 4096, 4207]), [3.0; 3]);
    assert!(
        (m[0] - 1.6946).abs() < 5e-5 && (m[2] - 1.0271).abs() < 5e-5,
        "{m:?}"
    );

    // R-SRW-33 fallbacks.
    let fallback = ([2.1137, 1.0, 1.5737], WbSource::Fallback);
    assert_eq!(meta::multipliers(None, [0.0; 3]), fallback);
    assert_eq!(
        meta::multipliers(Some(&[100, 3, 3, 100]), [3.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[3, 100, 100, 50]), [3.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[50, 100, 100, 3]), [3.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[1100, 100, 100, 50]), [0.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[50, 1000, 1000, 99]), [0.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[100, 100, 100]), [0.0; 3]),
        fallback
    );
    assert_eq!(
        meta::multipliers(Some(&[1000, 100, 100, 10]), [0.0; 3]).1,
        WbSource::AsShot,
        "10 and 0.1 are inside"
    );

    // R-SRW-31: the white level.
    use meta::white_level;
    let tag = |v: i64| {
        let mut l = vec![0i64; 4];
        l.extend([v; 8]);
        l
    };
    assert_eq!(
        white_level(b"EX1", Some(&tag(16383)), Variant::V1, Some(14)),
        15881
    );
    assert_eq!(
        white_level(b"NX10", Some(&tag(4095)), Variant::V2, Some(12)),
        4095
    );
    assert_eq!(white_level(b"X", Some(&tag(9000)), Variant::V2, None), 9000);
    assert_eq!(white_level(b"X", Some(&tag(256)), Variant::V2, None), 256);
    assert_eq!(white_level(b"X", Some(&tag(255)), Variant::V2, None), 4095);
    assert_eq!(
        white_level(b"X", Some(&tag(65536)), Variant::V1, None),
        16383
    );
    assert_eq!(
        white_level(b"X", Some(&tag(9000)[..11]), Variant::V1, None),
        16383
    );
    assert_eq!(white_level(b"X", None, Variant::V1, Some(12)), 4095);
    assert_eq!(white_level(b"X", None, Variant::V1, Some(14)), 16383);
    assert_eq!(white_level(b"X", None, Variant::V2, Some(12)), 4095);

    // R-SRW-32: the black levels.
    use meta::black_level;
    assert_eq!(black_level(Some(&[3, 2, 2, 3]), 4095), [3.0, 2.0, 3.0]);
    assert_eq!(black_level(Some(&[3, 2, 3, 3]), 4095), [3.0, 2.5, 3.0]);
    assert_eq!(
        black_level(Some(&[0, 0, 0, 3839]), 4095),
        [0.0, 0.0, 3839.0]
    );
    assert_eq!(black_level(Some(&[0, 0, 0, 3840]), 4095), [0.0; 3]);
    assert_eq!(black_level(Some(&[-1, 0, 0, 0]), 4095), [0.0; 3]);
    assert_eq!(black_level(Some(&[1, 1, 1]), 4095), [0.0; 3]);
    assert_eq!(black_level(None, 4095), [0.0; 3]);

    // R-SRW-35, R-SRW-36: matrix sources in order.
    let bad = [430, -152, -22, -36, 352, -60, 4, -128, 1000];
    let cam = [300, -30, -14, -20, 300, -24, 0, -50, 306];
    assert_eq!(
        meta::matrix(Some(&bad), Some(&cam), b"NX10").1,
        MatrixSource::Camera
    );
    let (m, s) = meta::matrix(Some(&bad), Some(&mx[..8]), b"NX10");
    assert_eq!(s, MatrixSource::Table);
    assert!((m[0][0] - 408.0 / 256.0).abs() < 1e-12);
    let (m, s) = meta::matrix(None, None, b"NX300");
    assert_eq!(s, MatrixSource::Generic);
    assert!((m[1][0] + 46.8 / 256.0).abs() < 1e-12);
    for model in [
        "NX5", "NX10", "NX11", "NX20", "NX200", "NX210", "NX1000", "NX1100", "EX1", "WB2000",
    ] {
        let (m, s) = meta::matrix(None, None, model.as_bytes());
        assert_eq!(s, MatrixSource::Table, "{model}");
        for r in m {
            assert!((r.iter().sum::<f64>() - 1.0).abs() < 1e-12, "{model}");
        }
    }
    // Row sums 128 and 512 are valid, 127 and 513 are not.
    let rows = |a: i64, b: i64| [a, 0, 0, 0, b, 0, 0, 0, 256];
    assert_eq!(
        meta::matrix(Some(&rows(128, 512)), None, b"").1,
        MatrixSource::Srgb
    );
    assert_eq!(
        meta::matrix(Some(&rows(127, 256)), None, b"").1,
        MatrixSource::Generic
    );
    assert_eq!(
        meta::matrix(Some(&rows(256, 513)), None, b"").1,
        MatrixSource::Generic
    );

    // Through a file: R-SRW-19, a key without 11 values makes every
    // encrypted tag absent.
    let mut b = recipe(false, false, None);
    b.maker = with(b.maker, 0xA020, Val::Long(KEY[..10].to_vec()));
    let p = plan_of(&b.build());
    assert_eq!(p.wb_source, WbSource::Fallback);
    assert_eq!(p.black, [0.0; 3]);
    assert_eq!(p.white, 4095);
    assert_eq!(p.matrix_source, MatrixSource::Table, "NX210 row");
    // The key as SLONG is accepted (R-SRW-8).
    let mut b = recipe(false, false, None);
    b.maker = with(
        b.maker,
        0xA020,
        Val::SLong(KEY.iter().map(|&k| k as i32).collect()),
    );
    let p = plan_of(&b.build());
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert_eq!(p.matrix_source, MatrixSource::Srgb);
    // The model table of white levels through a file.
    let mut b = recipe(false, true, Some(0));
    b.ifd0 = ifd0("EX1", 1);
    assert_eq!(plan_of(&b.build()).white, 15881);
    // R-SRW-9: trailing spaces and bytes after the first zero are ignored.
    let mut b = recipe(false, true, Some(0));
    b.ifd0 = with(b.ifd0, 0x0110, Val::Raw(2, 9, b"EX1  \0ab\0".to_vec()));
    assert_eq!(plan_of(&b.build()).white, 15881);

    // R-SRW-32 with bG a half: the plane is doubled and the result equals
    // the reference.
    let values = random_values(16 * 8, 16383, 3);
    let mut s = Synth::new(16, 8, values.clone(), Variant::V1);
    s.order = Some(0);
    s.black = [12, 12, 13, 12];
    s.areas = [0, 0, 16, 8, 1, 1, 15, 7];
    let f = s.build();
    let p = plan_of(&f);
    assert_eq!(p.black, [12.0, 12.5, 12.0]);
    assert!(matches!(
        develop::bridge(&p).1,
        sensor::Domain::Doubled { .. }
    ));
    assert_reference(&f, &values, 16, "half black");
    // Integer levels keep the values as read.
    s.black = [12, 12, 12, 12];
    assert_eq!(
        develop::bridge(&plan_of(&s.build())).1,
        sensor::Domain::AsRead
    );
    // White 60000: a doubled value above 65535 stays exact for blue
    // (multiplier 2.2: (65535 - 24) * 2.2 >= 120000 - 24) and not for
    // green (multiplier 1).
    let mut hot = vec![2000u16; 4 * 4];
    hot[4] = 40000; // pixel (0, 1): blue
    let mut s = Synth::new(4, 4, hot.clone(), Variant::V1);
    s.order = Some(0);
    s.black = [12, 12, 13, 12];
    s.white = 60000;
    s.wb = [1000, 4096, 4096, 9000];
    let f = s.build();
    let p = plan_of(&f);
    assert_eq!(p.cfa[2], 2, "pixel (0, 1) is blue");
    let sensor::Domain::Doubled { exact } = develop::bridge(&p).1 else {
        panic!("doubled");
    };
    assert_eq!(exact, [false, false, true]);
    assert_reference(&f, &hot, 4, "clipped blue");
    // A pixel of a color where the clipped value would differ fails the
    // decode (and the preview path takes over).
    let mut hot = vec![2000u16; 4 * 4];
    hot[5] = 40000; // pixel (1, 1): green
    s.values = hot;
    let f = s.build();
    let p = plan_of(&f);
    assert!(develop::develop(&f, &p, &CancelToken::new(), Output::Colorimetric16).is_err());
    assert!(
        decode_report(&f, &DecodeOptions::default()).is_err(),
        "no preview"
    );

    // R-SRW-34 and R-SRW-37: values clip to 0 and 1 (highlights, below
    // black).
    let mut v = vec![16383u16; 8 * 4];
    v[16..].fill(0);
    let f = Synth::new(8, 4, v, Variant::V1).build();
    for p in linear(&f).2 {
        for c in p {
            assert!((0.0..=1.0).contains(&c));
        }
    }
}

#[test]
fn srw_orientation_and_demosaic() {
    // R-SRW-23, R-SRW-39, R-SRW-40, R-SRW-41, R-SRW-42, R-SRW-43 against
    // the per-pixel reference: random values, odd and even valid origins,
    // every orientation and the four Bayer patterns.
    let (w, h) = (14u16, 10u16);
    for (k, cfa) in [[2, 1, 1, 0], [1, 0, 2, 1], [0, 1, 1, 2], [1, 2, 0, 1]]
        .into_iter()
        .enumerate()
    {
        for o in 1..=8u16 {
            for (variant, max) in [(Variant::V2, 4096), (Variant::V1, 16384)] {
                let values = random_values(usize::from(w * h), max, 11 + k as u32 + u32::from(o));
                let mut s = Synth::new(w, h, values.clone(), variant);
                s.cfa = cfa;
                s.orientation = o;
                s.order = Some(u32::from(o % 2));
                s.areas = [
                    0,
                    0,
                    14,
                    10,
                    1 + u32::from(o % 2),
                    2 - u32::from(o % 2),
                    13,
                    9,
                ];
                let f = s.build();
                let what = format!("cfa {cfa:?} orientation {o} {variant:?}");
                assert_reference(&f, &values, usize::from(w), &what);
                let dev = report(&f);
                let (vw, vh) = (12 - usize::from(o % 2), 7 + usize::from(o % 2));
                let size = if o >= 5 { (vh, vw) } else { (vw, vh) };
                assert_eq!(
                    (dev.image.width() as usize, dev.image.height() as usize),
                    size,
                    "{what}"
                );
            }
        }
    }
    // TV-8: a 3 x 2 image a b c / d e f.
    let stored = [['a', 'b', 'c'], ['d', 'e', 'f']];
    let out = |o: u8| {
        let (ow, oh) = if o >= 5 { (2, 3) } else { (3, 2) };
        let mut g = vec![vec![' '; ow]; oh];
        for (y, row) in stored.iter().enumerate() {
            for (x, &c) in row.iter().enumerate() {
                let (ox, oy) = oriented(o, x, y, 3, 2);
                g[oy][ox] = c;
            }
        }
        g.into_iter()
            .map(|r| r.into_iter().collect::<String>())
            .collect::<Vec<_>>()
    };
    assert_eq!(out(6), ["da", "eb", "fc"]);
    assert_eq!(out(8), ["cf", "be", "ad"]);
    assert_eq!(out(3), ["fed", "cba"]);
    // The worked example of R-SRW-39: stored (22, 14) of an NX10 file
    // (pattern 2 1 1 0, valid area from (21, 14)) is blue; its green is the
    // mean of three green neighbors, its red of two red ones.
    let cfa = [2u8, 1, 1, 0];
    let color = |x: usize, y: usize| cfa[2 * (y % 2) + x % 2];
    assert_eq!(color(22, 14), 2);
    let inside: Vec<(usize, usize)> = [(21, 14), (23, 14), (21, 15), (23, 15), (22, 15)].to_vec();
    assert_eq!(inside.iter().filter(|&&(x, y)| color(x, y) == 1).count(), 3);
    assert_eq!(inside.iter().filter(|&&(x, y)| color(x, y) == 0).count(), 2);
    // R-SRW-42: orientation 0 or 9 counts as 1.
    for o in [0u16, 9] {
        let mut s = Synth::new(8, 6, vec![1000; 48], Variant::V2);
        s.orientation = o;
        assert_eq!(plan_of(&s.build()).orientation, 1);
    }
    // The origin of the pattern is stored (0, 0), not the valid-area origin:
    // shifting the valid area by one column changes the plane pattern.
    assert_eq!(develop::shifted_cfa([1, 0, 2, 1], 1, 1), [1, 2, 0, 1]);
    assert_eq!(develop::shifted_cfa([1, 0, 2, 1], 1, 0), [0, 1, 1, 2]);
    assert_eq!(develop::shifted_cfa([1, 0, 2, 1], 2, 2), [1, 0, 2, 1]);
}

// ---------------------------------------------------------------------------
// Preview path, hostile input, cancellation
// ---------------------------------------------------------------------------

/// A builder with a full-size preview (blob 1, first SubIFD of IFD0) and a
/// thumbnail (blob 2, SubIFD of IFD1); the sensor data stay blob 0.
fn with_previews(mut b: Builder, full: Vec<u8>, thumb: Vec<u8>) -> Builder {
    b.blobs.truncate(1);
    if b.blobs.is_empty() {
        b.blobs.push(Vec::new());
    }
    b.blobs.push(full);
    b.blobs.push(thumb);
    b.preview = Some(1);
    b.thumb = Some(2);
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

#[test]
fn srw_out_of_scope_falls_back_to_preview() {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let prev = |b: Builder| with_previews(b, big.clone(), small.clone());
    // A good file develops (R-SRW-1); its candidates are the full-size JPEG
    // of IFD0's first SubIFD and the thumbnail of IFD1's SubIFD (R-SRW-44).
    let good = prev(recipe(false, false, None)).build();
    let c = container(&good);
    let cands = preview::collect(&c);
    assert_eq!(cands.len(), 2);
    assert_eq!((cands[0].width, cands[0].height), (48, 32), "R-SRW-45");
    assert_eq!(report(&good).origin, Origin::Sensor(Variant::V2));
    // The same in a little-endian file.
    let good_le = prev(recipe(true, false, None)).build();
    assert_eq!(preview::collect(&container(&good_le)).len(), 2);
    assert_eq!(report(&good_le).origin, Origin::Sensor(Variant::V2));

    let check = |f: &[u8], class: Option<Class>, what: &str| {
        let dev = report(f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        if class.is_some() {
            assert_eq!(dev.class, class, "{what}");
        }
    };
    use Class::OutOfScope;
    // X1: Compression 32770 with another strip size. R-SRW-30: the strip
    // is never read, here it lies outside the file.
    let mut b = prev(recipe(false, false, None));
    b.raw = with(b.raw, 0x0117, Val::Long(vec![71]));
    b.raw = with(b.raw, 0x0111, Val::Long(vec![0x7FFF_FFF0]));
    check(&b.build(), Some(OutOfScope(Relative::X1)), "X1");
    // X2 and X3.
    let mut b = prev(recipe(false, false, None));
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32772]));
    check(&b.build(), Some(OutOfScope(Relative::X2)), "X2");
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32773]));
    check(&b.build(), Some(OutOfScope(Relative::X3)), "X3");
    // X4: Compression 32769 with fewer than 2 * W * H bytes, Compression 1,
    // an unexpected BitsPerSample.
    let mut b = prev(recipe(false, true, Some(0)));
    b.raw = with(b.raw, 0x0117, Val::Long(vec![95]));
    check(&b.build(), Some(OutOfScope(Relative::X4)), "X4 short");
    let mut b = prev(recipe(false, true, Some(0)));
    b.raw = with(b.raw, 0x0103, Val::Short(vec![1]));
    check(
        &b.build(),
        Some(OutOfScope(Relative::X4)),
        "X4 compression 1",
    );
    let mut b = prev(recipe(false, true, Some(0)));
    b.raw = with(b.raw, 0x0102, Val::Short(vec![16]));
    check(&b.build(), Some(OutOfScope(Relative::X4)), "X4 16 bits");
    // X5: no raw IFD.
    let mut b = prev(recipe(false, false, None));
    b.raw = without(b.raw, 0x0117);
    check(&b.build(), Some(OutOfScope(Relative::X5)), "X5");

    // In-scope files that fail a check use the preview path too.
    let fails: Vec<(&str, Builder)> = vec![
        ("no maker note (R-SRW-13, R-SRW-16)", {
            let mut b = prev(recipe(false, false, None));
            b.maker.clear();
            b
        }),
        ("no tag 0xA010 (R-SRW-16)", {
            let mut b = prev(recipe(false, false, None));
            b.maker = without(b.maker, 0xA010);
            b
        }),
        ("0xA010 with 7 values", {
            let mut b = prev(recipe(false, false, None));
            b.maker = with(b.maker, 0xA010, Val::Long(vec![0, 0, 8, 6, 1, 1, 7]));
            b
        }),
        ("valid area wider than W (R-SRW-22)", {
            let mut b = prev(recipe(false, false, None));
            b.maker = with(b.maker, 0xA010, Val::Long(vec![0, 0, 8, 6, 1, 1, 9, 5]));
            b
        }),
        ("valid area of 1 column (R-SRW-22)", {
            let mut b = prev(recipe(false, false, None));
            b.maker = with(b.maker, 0xA010, Val::Long(vec![0, 0, 8, 6, 1, 1, 2, 5]));
            b
        }),
        ("negative valid area origin (R-SRW-22)", {
            let mut b = prev(recipe(false, false, None));
            b.maker = with(b.maker, 0xA010, Val::SLong(vec![0, 0, 8, 6, -1, 1, 7, 5]));
            b
        }),
        ("CFA pattern with two reds (R-SRW-17)", {
            let mut b = prev(recipe(false, false, None));
            b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 0, 1, 2]));
            b
        }),
        ("CFA repeat 2 x 4 (R-SRW-17)", {
            let mut b = prev(recipe(false, false, None));
            b.raw = with(b.raw, 0x828D, Val::Short(vec![2, 4]));
            b
        }),
        ("CFA pattern of 3 values (R-SRW-17)", {
            let mut b = prev(recipe(false, false, None));
            b.raw = with(b.raw, 0x828E, Val::Byte(vec![0, 1, 2]));
            b
        }),
        ("raw data byte order 5 (R-SRW-25)", {
            let mut b = prev(recipe(false, false, None));
            b.maker = with(b.maker, 0x0040, Val::Long(vec![5]));
            b
        }),
        ("two strip offsets, one count (R-SRW-54)", {
            let mut b = prev(recipe(false, false, None));
            b.raw = with(b.raw, 0x0111, Val::Long(vec![0x210, 0x210]));
            b
        }),
        ("strip count against RowsPerStrip (R-SRW-54)", {
            let mut b = prev(recipe(false, false, None));
            b.raw = with(b.raw, 0x0116, Val::Short(vec![3]));
            b
        }),
        ("strip outside the file (R-SRW-53)", {
            let mut b = prev(recipe(false, true, Some(0)));
            b.raw = with(b.raw, 0x0117, Val::Long(vec![1 << 20]));
            b
        }),
        ("Exif IFD pointer outside the file (R-SRW-13)", {
            let mut b = prev(recipe(false, false, None));
            b.ifd0 = with(b.ifd0, 0x8769, Val::Long(vec![0x00FF_FFFF]));
            b
        }),
    ];
    for (what, b) in fails {
        let f = b.build();
        let dev = report(&f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        assert!(matches!(dev.class, Some(Class::Sensor(_))), "{what}");
    }
    // R-SRW-51: oversized and empty rasters.
    for (w, h) in [(0u32, 6u32), (8, 0), (70000, 6), (65535, 65535)] {
        let mut b = prev(recipe(false, true, Some(0)));
        b.raw = with(b.raw, 0x0100, Val::Long(vec![w]));
        b.raw = with(b.raw, 0x0101, Val::Long(vec![h]));
        b.raw = with(b.raw, 0x0117, Val::Long(vec![u32::MAX]));
        let f = b.build();
        assert!(
            meta::plan(&container(&f), Variant::V1).is_err(),
            "{w} x {h}"
        );
        assert_eq!(report(&f).origin, Origin::Preview, "{w} x {h}");
    }
    // R-SRW-48: a candidate that fails to decode gives way to the next. The
    // broken one keeps its size (SOF0) but declares 0 components.
    let mut broken = big.clone();
    let sof = broken.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    broken[sof + 9] = 0;
    let mut b = with_previews(recipe(false, false, None), broken, small.clone());
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32772]));
    let dev = report(&b.build());
    assert_preview(&dev, (16, 8), [40, 40, 200], "next candidate");
    // R-SRW-46: bytes after the EOI marker, and no EOI marker at all.
    for (what, tail) in [("trailing bytes", true), ("no EOI", false)] {
        let mut j = big.clone();
        if tail {
            j.extend([0u8; 45]);
        } else {
            assert_eq!(&j[j.len() - 2..], &[0xFF, 0xD9]);
            j.truncate(j.len() - 2);
        }
        let mut b = with_previews(recipe(false, false, None), j, small.clone());
        b.raw = with(b.raw, 0x0103, Val::Short(vec![32773]));
        assert_preview(&report(&b.build()), (48, 32), [200, 40, 40], what);
    }
    // R-SRW-44: a candidate that does not start with FF D8 or whose length
    // passes the end of the file is not kept.
    let mut b = with_previews(recipe(false, false, None), vec![0u8; 64], small.clone());
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32772]));
    let f = b.build();
    assert_eq!(preview::collect(&container(&f)).len(), 1);
    // R-SRW-47: orientation 6 turns a wide preview; a tall preview keeps
    // its stored orientation.
    let mut b = prev(recipe(false, false, None));
    b.ifd0 = ifd0("NX210", 6);
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32772]));
    assert_preview(
        &report(&b.build()),
        (32, 48),
        [200, 40, 40],
        "orientation 6",
    );
    let mut b = with_previews(
        recipe(false, false, None),
        jpeg(32, 48, [200, 40, 40]),
        small.clone(),
    );
    b.ifd0 = ifd0("NX210", 6);
    b.raw = with(b.raw, 0x0103, Val::Short(vec![32772]));
    assert_preview(&report(&b.build()), (32, 48), [200, 40, 40], "tall preview");
    // The size probe of an out-of-scope file is the displayed preview.
    let probe_size = probe(&b.build()).unwrap().unwrap();
    assert_eq!((probe_size.0, probe_size.1), (32, 48));

    // A thumbnail request takes the smallest preview that covers it; a
    // larger one develops the sensor data (a 128 x 64 sensor with previews
    // of 32 x 16 and 64 x 32).
    let s = Synth::new(128, 64, random_values(128 * 64, 4096, 9), Variant::V2);
    let thumbs = with_previews(
        s.builder(),
        jpeg(64, 32, [200, 10, 10]),
        jpeg(32, 16, [10, 200, 10]),
    )
    .build();
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    let dev = decode_report(&thumbs, &opts((20, 10))).unwrap().unwrap();
    assert_preview(&dev, (32, 16), [10, 200, 10], "small target");
    let dev = decode_report(&thumbs, &opts((40, 40))).unwrap().unwrap();
    assert_preview(&dev, (64, 32), [200, 10, 10], "medium target");
    let dev = decode_report(&thumbs, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V2));
    assert_eq!((dev.image.width(), dev.image.height()), (128, 64));
    let src = pandite_core::Source::from_bytes(thumbs.clone(), Some("t.srw".into()));
    let img = crate::default_registry()
        .decode(&src, &opts((16, 16)))
        .unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);

    // Through the registry: by extension, and by structure (the TIFF
    // provider hands a Samsung TIFF with an SRW compression over).
    let reg = crate::default_registry();
    for name in [Some("a.srw"), Some("A.SRW"), None] {
        let src = pandite_core::Source::from_bytes(good.clone(), name.map(Into::into));
        let img = reg.decode(&src, &DecodeOptions::default()).expect("decode");
        let img = img.first_image();
        assert_eq!((img.width(), img.height()), (6, 4), "{name:?}");
        let meta = reg.probe(&src).expect("probe");
        assert_eq!((meta.width, meta.height), (6, 4), "{name:?}");
        assert!(crate::raw::is_camera_raw(&src), "{name:?}");
    }
    let x3 = {
        let mut b = prev(recipe(false, false, None));
        b.raw = with(b.raw, 0x0103, Val::Short(vec![32773]));
        b.build()
    };
    let src = pandite_core::Source::from_bytes(x3, Some("b.srw".into()));
    let img = reg
        .decode(&src, &DecodeOptions::default())
        .expect("X3 preview");
    assert_eq!(img.first_image().width(), 48);
    assert!(is_srw(&good, None) && is_srw(&f1(), None));
    let mut sony = good.clone();
    let at = sony.windows(7).position(|w| w == b"SAMSUNG").unwrap();
    sony[at..at + 7].copy_from_slice(b"SONY\0\0\0");
    assert!(!is_srw(&sony, None), "another maker");
    assert!(is_srw(&sony, Some("srw")), "by extension");
}

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let big = jpeg(48, 32, [200, 40, 40]);
    let small = jpeg(16, 8, [40, 40, 200]);
    let mut x2 = with_previews(recipe(true, false, None), big.clone(), small.clone());
    x2.raw = with(x2.raw, 0x0103, Val::Short(vec![32772]));
    let mut rotated = with_previews(recipe(false, true, Some(0)), big.clone(), small.clone());
    rotated.ifd0 = ifd0("NX210", 6);
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("F5", f5()),
        (
            "F1+previews",
            with_previews(recipe(false, false, None), big.clone(), small.clone()).build(),
        ),
        (
            "F1 LE+previews",
            with_previews(recipe(true, false, Some(0)), big, small).build(),
        ),
        ("F3 rotated+previews", rotated.build()),
        ("X2 LE+previews", x2.build()),
    ]
}

#[test]
fn srw_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut next = noise(0x9E37_79B9);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        let le = f[0] == 0x49;
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
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
                            // R-SRW-58: a sensor image is complete.
                            let (w, h) = (dev.image.width(), dev.image.height());
                            assert!(w > 0 && h > 0);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let msg = e.to_string();
                            assert!(!msg.contains("panicked"), "{name} round {round}: {msg}");
                        }
                    }
                }
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.srw".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // R-SRW-52: a count far beyond the file size fails before allocation.
    let mut b = recipe(false, false, None);
    b.raw = with(b.raw, 0x0111, Val::Raw(4, 0x4000_0000, vec![0; 8]));
    let f = b.build();
    let c = container(&f);
    assert!(c.raw.as_ref().unwrap().uints(0x0111).is_err());
    assert!(meta::plan(&c, Variant::V2).is_err());
    let mut b = recipe(false, false, None);
    b.maker = with(b.maker, 0xA021, Val::Raw(4, 0x4000_0000, vec![0; 8]));
    let f = b.build();
    assert!(
        meta::plan(&container(&f), Variant::V2).is_err(),
        "maker note"
    );
    // R-SRW-59: a maker-note value outside the file makes its tag absent;
    // a tag of the TIFF structure fails.
    let mut f = f1();
    // 0xA021: offset 0xA6 from P becomes 0x7FFF_0000.
    f[0x98..0x9C].copy_from_slice(&0x7FFF_0000u32.to_be_bytes());
    let p = plan_of(&f);
    assert_eq!(p.wb_source, WbSource::Fallback);
    let mut f = f1();
    // IFD0 Model: offset 0x52 becomes 0x7FFF_0000.
    f[0x1E..0x22].copy_from_slice(&0x7FFF_0000u32.to_be_bytes());
    assert!(meta::plan(&container(&f), Variant::V2).is_err());
    // R-SRW-10: a next-IFD loop stops.
    let mut f = f1();
    f[0x46..0x4A].copy_from_slice(&8u32.to_be_bytes());
    assert_eq!(container(&f).chain.len(), 1);
    assert_eq!(report(&f).origin, Origin::Sensor(Variant::V2));
}

#[test]
fn srw_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        // The end of the strip of an in-scope file: a cut after it (inside
        // the previews that follow) still holds every sensor byte.
        let c = container(&f);
        let (strip_end, size) = match meta::classify(&c) {
            Ok(Class::Sensor(v)) => meta::plan(&c, v).map_or((usize::MAX, (0, 0)), |p| {
                (
                    p.strips[0] + p.row_bytes * p.height,
                    develop::output_size(&p),
                )
            }),
            _ => (usize::MAX, (0, 0)),
        };
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
                    // R-SRW-53, R-SRW-58: sensor data only when the whole
                    // strip lies inside the cut file.
                    assert!(cut >= strip_end, "{name} cut at {cut}: {:?}", dev.origin);
                    let got = (dev.image.width() as usize, dev.image.height() as usize);
                    assert_eq!(got, size, "{name} cut at {cut}");
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
    // The strip is last: a file cut inside it fails its checks before any
    // sensor value is read.
    let f = f1();
    assert!(meta::plan(&container(&f[..f.len() - 1]), Variant::V2).is_err());
}

/// A large V2 file: 4096 x 4096 in 256 strips of 16 rows, all pointing at
/// one block (the specification does not require distinct strips).
fn large_v2() -> Vec<u8> {
    let (w, h, rows) = (4096u16, 4096u16, 16u16);
    let block = pack12(
        &random_values(usize::from(w) * usize::from(rows), 4096, 5),
        false,
    );
    let mut raw = raw_ifd(w, h, 12, 32770, 0);
    raw = with(raw, 0x0116, Val::Short(vec![rows]));
    raw = with(raw, 0x0111, Val::Blobs(vec![0; 256]));
    raw = with(raw, 0x0117, Val::BlobLens(vec![0; 256]));
    Builder {
        ifd0: ifd0("NX210", 1),
        maker: maker(
            None,
            [0, 0, 4096, 4096, 0, 0, 4096, 4096],
            [9776, 4096, 4096, 7232],
            [3; 4],
            4095,
        ),
        raw,
        blobs: vec![block],
        ..Builder::default()
    }
    .build()
}

#[test]
fn srw_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f1(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f1(), Some("a.srw".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread.
    let f = large_v2();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor(Variant::V2));
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
fn facts_srw() {
    use pandite_core::Source;
    let f = super::facts(&f1()).expect("SRW facts");
    assert_eq!(f.container.as_deref(), Some("Samsung SRW"));
    assert_eq!(f.compression.as_deref(), Some("Samsung 12-bit packed"));
    assert_eq!(f.channels.as_deref(), Some("Bayer GRBG"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(12)));
    assert_eq!(f.camera.make.as_deref(), Some("SAMSUNG"));
    assert_eq!(f.camera.model.as_deref(), Some("NX210"));
    let compression = |data: &[u8]| super::facts(data).and_then(|f| f.compression);
    assert_eq!(compression(&f3()).as_deref(), Some("Uncompressed"));
    assert_eq!(
        compression(&f4()).as_deref(),
        Some("Samsung SRW Compressed 2")
    );
    assert_eq!(
        compression(&f5()).as_deref(),
        Some("Samsung SRW Compressed")
    );
    assert_eq!(super::facts(&f3()).unwrap().bits_per_channel, Some(14));
    let reg = crate::default_registry();
    let via = reg
        .facts(&Source::from_bytes(f1(), Some("SAM_0001.SRW".into())))
        .expect("registry facts");
    assert_eq!(via.container.as_deref(), Some("Samsung SRW"));
    assert_eq!(via.channels.as_deref(), Some("Bayer GRBG"));
    // The head of the file: the directories come before the strip.
    let f = f1();
    let head = super::facts(&f[..0x210]).expect("head facts");
    assert_eq!(head.compression.as_deref(), Some("Samsung 12-bit packed"));
    for n in 0..f.len() {
        let _ = super::facts(&f[..n]);
    }
    assert!(super::facts(b"MM\0\x2A").is_none());
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// The centered part of `img` with the aspect ratio `w` : `h` (M11: the
/// full-size JPEG is the valid area or a centered aspect-ratio crop of it).
fn centered(img: &ImageBuf, w: u32, h: u32) -> ImageBuf {
    let (iw, ih) = (img.width(), img.height());
    let (cw, ch) = if u64::from(iw) * u64::from(h) > u64::from(ih) * u64::from(w) {
        ((u64::from(ih) * u64::from(w) / u64::from(h)) as u32, ih)
    } else {
        (iw, (u64::from(iw) * u64::from(h) / u64::from(w)) as u32)
    };
    if (cw, ch) == (iw, ih) {
        return img.clone();
    }
    let px = img.format().bytes_per_pixel();
    let (x0, y0) = ((iw - cw) / 2, (ih - ch) / 2);
    let mut data = Vec::with_capacity(cw as usize * ch as usize * px);
    for y in y0..y0 + ch {
        let at = (y as usize * iw as usize + x0 as usize) * px;
        data.extend_from_slice(&img.data()[at..at + cw as usize * px]);
    }
    ImageBuf::new(cw, ch, img.format(), data).unwrap()
}

/// Every `.SRW` file in the folder named by `PANDITE_SRW_SAMPLES` (absolute,
/// or relative to the workspace root) decodes through the sensor-data path
/// when its variant is in scope, at the valid-area size, and agrees with
/// the file's own largest embedded JPEG on an 8 x 8 grid of cell means, by
/// the method and tolerances of `arw_real_samples_match_embedded_preview`:
/// exposure matched by one gain on luminance, rg chromaticity distance of
/// the usable cells (mean 0.05, largest 0.15), Spearman rank correlation of
/// cell luminance at least 0.90, at least 16 usable cells. Files with the
/// fallback multipliers of R-SRW-33 are compared after one global gain per
/// channel. When the JPEG is an aspect-ratio crop of the valid area (M11),
/// the centered part of the developed image is compared. Out-of-scope files
/// must return the preview.
#[test]
fn srw_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_SRW_SAMPLES") else {
        println!("skip: PANDITE_SRW_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("srw"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .SRW files in {}", folder.display());
    let mut failures = Vec::new();
    let mut developed = 0;
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
        // The default display rendering, as the viewer shows it.
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("SRW path");
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
        assert_eq!(
            Some((info.raster.0 as u32, info.raster.1 as u32)),
            raw_size,
            "{name}"
        );
        // R-SRW-41: exactly the valid area, oriented.
        let (ow, oh) = render::output_size(info.valid, info.orientation);
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            (ow, oh),
            "{name}"
        );
        // The camera's largest JPEG, decoded small and oriented (R-SRW-47).
        let cands = preview::collect(&c);
        let best = cands.first().expect("an embedded preview");
        let (jw, jh) = best.displayed(info.orientation);
        if best.width > 2000 {
            // M11, M13: the full-size JPEG has the size of the valid area,
            // or is a crop of it to another aspect ratio (WB2000).
            let (jw, jh, ow, oh) = (jw as usize, jh as usize, ow, oh);
            assert!(
                (jw, jh) == (ow, oh) || (jw == ow && jh < oh) || (jh == oh && jw < ow),
                "{name}: full-size JPEG {jw}x{jh}, valid area {ow}x{oh}"
            );
        }
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev =
            arw::preview::decode(&data, best, info.orientation, &opts).expect("preview decodes");
        let ours_img = centered(&dev.image, jw, jh);
        let ours = grid_means(&ours_img, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        let balance = info.wb == WbSource::Fallback;
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let lum = |g: &[[f64; 3]]| {
            g.iter()
                .map(|c| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2])
                .sum::<f64>()
        };
        let ev = (lum(&ours) / lum(&theirs)).log2();
        // The colorimetric result (linear sRGB, R-SRW-38) for comparison.
        let flat = report(&data);
        let flat = grid_means(&centered(&flat.image, jw, jh), 8, 4);
        let flat_ev = (lum(&flat) / lum(&theirs)).log2();
        let line = format!(
            "{name}: model {} {variant:?} ({described}) sensor path, raster {}x{} valid {}x{}+{}+{} out {}x{} white {} black {:?} wb {:?} matrix {:?} data {} decode {:.2}s (debug) | vs embedded {}x{} (compared {}x{}): chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}, mean luminance {ev:+.2} EV (colorimetric {flat_ev:+.2} EV){}",
            info.model,
            info.raster.0,
            info.raster.1,
            info.valid.w,
            info.valid.h,
            info.valid.x,
            info.valid.y,
            ow,
            oh,
            info.white,
            info.black,
            info.wb,
            info.matrix,
            if info.data_le { "LE" } else { "BE" },
            took.as_secs_f64(),
            best.width,
            best.height,
            ours_img.width(),
            ours_img.height(),
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

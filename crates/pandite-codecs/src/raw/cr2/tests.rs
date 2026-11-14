//! Tests of the CR2 decoder against the specification's test vectors
//! (TV-1 to TV-12), fixtures (F1 to F4) and requirements, plus hostile
//! input, cancellation, thumbnails and real camera files.

use super::fixtures::*;
use super::meta::{self, Class, Plan};
use super::sensor::{self, Raster};
use super::*;
use crate::raw::arw::ifd;
use crate::raw::arw::lj92;
use crate::raw::arw::tests::{agreement, grid_means, LinearF32, CHROMA_MAX, CHROMA_MEAN, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The specification prints 7 decimals and rounds its own intermediates;
/// a match within 1e-6 is a match at the printed precision.
const EPS: f32 = 1e-6;

fn container(data: &[u8]) -> meta::Container<'_> {
    meta::open(data).expect("open").expect("CR2 header")
}

fn plan_of(data: &[u8]) -> Plan {
    let c = container(data);
    assert_eq!(meta::classify(&c), Class::V1, "classification");
    meta::plan(&c).expect("plan")
}

fn raster_of(data: &[u8]) -> (Plan, Raster) {
    let plan = plan_of(data);
    let r = sensor::decode(data, &plan, &CancelToken::new()).expect("decode raster");
    (plan, r)
}

/// Linear sRGB of every output pixel (R-CR2-33), oriented, and the levels.
fn linear(data: &[u8]) -> (usize, usize, Vec<[f32; 3]>, Levels) {
    let (plan, raster) = raster_of(data);
    let (bytes, levels) =
        develop_raster(raster, &plan, &CancelToken::new(), &LinearF32).expect("develop");
    let (w, h) = render::output_size(crop_of(&plan), plan.orientation);
    let px = bytes
        .chunks_exact(12)
        .map(|c| {
            let f =
                |k: usize| f32::from_ne_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]);
            [f(0), f(1), f(2)]
        })
        .collect();
    (w, h, px, levels)
}

fn report(data: &[u8]) -> Developed {
    decode_report(data, &DecodeOptions::default())
        .expect("decode")
        .expect("CR2 path")
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
/// within 2 codes of 65535, after the linear value is quantized to steps
/// of 1/65535 as the 16-bit encoder does (near 0 one step is 13 codes).
fn assert_rgba16_flat(img: &ImageBuf, lin: [f32; 3], what: &str) {
    assert_eq!(img.format(), PixelFormat::Rgba16, "{what}");
    let code = |v: f32| {
        let q = ((v * 65535.0 + 0.5) as u32) as f32 / 65535.0;
        (crate::util::linear_to_srgb(q) * 65535.0 + 0.5) as u16
    };
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

/// The stream of a header-only LJ92 frame of the TV-3 tables (no entropy
/// data): enough for the parser.
fn header_only(p: u8, x: u16, y: u16, nf: u8) -> Vec<u8> {
    let mut s = hex(TV3_DHT);
    s.splice(0..0, [0xFF, 0xD8]);
    let mut sof = vec![0xFF, 0xC3];
    sof.extend_from_slice(&(8 + 3 * u16::from(nf)).to_be_bytes());
    sof.push(p);
    sof.extend_from_slice(&y.to_be_bytes());
    sof.extend_from_slice(&x.to_be_bytes());
    sof.push(nf);
    for k in 1..=nf {
        sof.extend_from_slice(&[k, 0x11, 0]);
    }
    s.extend(sof);
    let mut sos = vec![0xFF, 0xDA];
    sos.extend_from_slice(&(6 + 2 * u16::from(nf)).to_be_bytes());
    sos.push(nf);
    for k in 1..=nf {
        sos.extend_from_slice(&[k, if k % 2 == 1 { 0x00 } else { 0x10 }]);
    }
    sos.extend_from_slice(&[1, 0, 0]);
    s.extend(sos);
    s.extend_from_slice(&[0xFF, 0xD9]);
    s
}

/// The DHT segment of TV-3 (tables 0 and 1, the 14-bit Canon table).
const TV3_DHT: &str = "FF C4 00 42 00 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 04 \
08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 \
04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E";

/// TV-3: the complete 2-component stream.
const TV3: &str = "FF D8
FF C4 00 42 00 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E 01 00 01 04 02 03 01 01 01 01 01 00 00 00 00 00 00 06 04 08 05 07 03 09 00 0A 02 01 0C 0B 0D 0E
FF C3 00 0E 0E 00 02 00 02 02 01 11 00 02 11 00
FF DA 00 0A 02 01 00 02 10 01 00 00
FF 00 20 07 FC 80 0F 5E 3C 7A 5B E7
FF D9";

/// TV-4: the complete 4-component stream with the 12-bit Canon table.
const TV4: &str = "FF D8
FF C4 00 3E 00 00 02 02 02 03 01 01 01 01 00 00 00 00 00 00 00 04 05 03 06 07 02 08 00 01 09 0A 0C 0B 01 00 02 02 02 03 01 01 01 01 00 00 00 00 00 00 00 04 05 03 06 07 02 08 00 01 09 0A 0C 0B
FF C3 00 14 0C 00 02 00 01 04 01 11 00 02 11 00 03 11 00 04 11 00
FF DA 00 0E 04 01 00 02 10 03 00 04 10 01 00 00
FF 00 12 BF F1 01 FF 00 10 5F F0 FE 28 72 3D
FF D9";

fn seq_of(stream: &[u8]) -> Vec<u16> {
    let s = lj92::parse(stream).expect("parse");
    assert!(meta::is_v1_stream(&s), "R-CR2-21");
    sensor::sequence(&s, &CancelToken::new()).expect("decode")
}

// ---------------------------------------------------------------------------
// Test vectors and fixtures
// ---------------------------------------------------------------------------

/// A fixture: name, file, layout, output size, white level, every output
/// pixel.
type Fixture = (
    &'static str,
    Vec<u8>,
    Option<Layout>,
    (usize, usize),
    u32,
    [f32; 3],
);

#[test]
fn cr2_vectors_decode() {
    // TV-1 (R-CR2-4, R-CR2-5, R-CR2-8): the S15 header.
    let tv1 = hex("49 49 2A 00 10 00 00 00 43 52 02 00 64 19 01 00");
    assert!(is_cr2(&tv1, None));
    assert_eq!(ifd::le32(&tv1, 4), Some(16));
    assert_eq!(ifd::le32(&tv1, 12), Some(72036));
    let c = container(&tv1);
    assert!(c.chain.is_empty() && c.raw.is_none() && c.maker.is_none());
    // Any value of byte 11 (R-CR2-4).
    let mut minor = tv1.clone();
    minor[11] = 9;
    assert!(meta::open(&minor).unwrap().is_some());
    // X3: no CR2 signature, or big-endian (R-CR2-3 rule 1, R-CR2-2).
    let x3 = hex("49 49 2A 00 08 00 00 00 00 00 00 00 00 00 00 00");
    assert!(!is_cr2(&x3, None));
    assert!(meta::open(&x3).unwrap().is_none());
    assert!(decode_report(&x3, &DecodeOptions::default())
        .unwrap()
        .is_none());
    let be = hex("4D 4D 00 2A 00 00 00 10 43 52 02 00 00 00 00 00");
    assert!(meta::open(&be).unwrap().is_none());
    assert!(is_cr2(&x3, Some("cr2")), "by extension");
    // R-CR2-43: a CR2 header in fewer than 16 bytes fails.
    assert!(meta::open(&tv1[..15]).is_err());

    // TV-2 (R-CR2-6, R-CR2-13, R-CR2-16, R-CR2-19): five IFD entries with
    // their out-of-line values at the offsets the entries name.
    let mut buf = vec![0u8; 50_000];
    let entries = [
        "03 01 03 00 01 00 00 00 06 00 00 00",
        "E0 C5 04 00 01 00 00 00 03 00 00 00",
        "40 C6 03 00 03 00 00 00 A4 AD 00 00",
        "10 00 04 00 01 00 00 00 85 02 00 80",
        "01 40 07 00 00 14 00 00 92 0C 00 00",
    ];
    buf[16..18].copy_from_slice(&5u16.to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        buf[18 + 12 * i..30 + 12 * i].copy_from_slice(&hex(e));
    }
    buf[44452..44458].copy_from_slice(&hex("02 00 90 07 80 07"));
    buf[3218..3220].copy_from_slice(&[0xFD, 0xFF]);
    let d = ifd::dir(&buf, 16).expect("valid IFD");
    assert_eq!(d.uint(0x0103).unwrap(), Some(6));
    assert_eq!(d.uint(0xC5E0).unwrap(), Some(3));
    assert_eq!(meta::cfa_colors(3).unwrap(), [1, 2, 0, 1], "G B / R G");
    let slices = d.uints(0xC640).unwrap().unwrap();
    assert_eq!(slices, vec![2, 1936, 1920]);
    assert_eq!(
        meta::slices(Some(&slices), 5792).unwrap(),
        Some((2, 1936, 1920))
    );
    assert_eq!(d.uint(0x0010).unwrap(), Some(0x8000_0285));
    let cd = meta::color_data(&d).unwrap().unwrap();
    assert_eq!(cd.len(), 2560);
    assert_eq!(cd[0], -3);
    // R-CR2-7: IFDs with N = 0, N > 1024 or bytes past the end are invalid.
    assert!(ifd::dir(&buf, 100).is_none(), "N = 0");
    let mut big = buf.clone();
    big[16..18].copy_from_slice(&1025u16.to_le_bytes());
    assert!(ifd::dir(&big, 16).is_none(), "N > 1024");
    assert!(ifd::dir(&buf[..60], 16).is_none(), "entries past the end");
    // R-CR2-45: a value range beyond the file fails before allocation.
    let mut far = buf.clone();
    far[18 + 48 + 4..18 + 48 + 8].copy_from_slice(&0x7FFF_FFFFu32.to_le_bytes());
    let d = ifd::dir(&far, 16).unwrap();
    assert!(meta::color_data(&d).is_err());

    // TV-3 (R-CR2-20, R-CR2-22): the 2-component stream.
    assert_eq!(
        seq_of(&hex(TV3)),
        vec![2050, 2049, 2052, 2046, 2047, 2051, 2060, 2049]
    );
    // TV-4: the 4-component stream with the 12-bit table; with W = 2 and
    // one slice the raster rows are 300 258 / 262 255 / 310 250 / 270 256.
    let tv4 = seq_of(&hex(TV4));
    assert_eq!(tv4, vec![300, 258, 262, 255, 310, 250, 270, 256]);
    assert_eq!(meta::slices(None, 2).unwrap(), None);

    // TV-10: the first entropy-coded bytes of S15 in a 3 x 1 frame of the
    // TV-3 headers: six samples, raster (0, 0) to (5, 0) of one slice.
    let mut tv10 = hex(TV3);
    let sof = tv10.windows(2).position(|w| w == [0xFF, 0xC3]).unwrap();
    tv10[sof + 5..sof + 9].copy_from_slice(&[0, 1, 0, 3]);
    let sos = tv10.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    tv10.truncate(sos + 12);
    tv10.extend(hex("FF 00 1F EF FC 77 3A 2A 4A 61 76 C6 FF D9"));
    assert_eq!(seq_of(&tv10), vec![2044, 1908, 1927, 1926, 1951, 1940]);

    // TV-5 (R-CR2-17): slices n = 1, a = 4, b = 2, W = 6, H = 4.
    let cancel = CancelToken::new();
    let seq: Vec<u16> = (0..24).collect();
    let out = sensor::reassemble(&seq, 6, 4, (1, 4, 2), &cancel).unwrap();
    for (s, (x, y)) in [
        (0, (0, 0)),
        (3, (3, 0)),
        (4, (0, 1)),
        (5, (1, 1)),
        (15, (3, 3)),
        (16, (4, 0)),
        (17, (5, 0)),
        (18, (4, 1)),
        (19, (5, 1)),
        (23, (5, 3)),
    ] {
        assert_eq!(out[y * 6 + x], s, "TV-5 sample {s}");
    }
    // The loop reassembly equals the formula of the specification, also
    // when the last slice is wider than the others.
    for (w, h, sl) in [(6, 4, (1, 4, 2)), (7, 3, (2, 2, 3)), (10, 5, (3, 2, 4))] {
        let seq: Vec<u16> = (0..(w * h) as u16).collect();
        let out = sensor::reassemble(&seq, w, h, sl, &cancel).unwrap();
        for s in 0..w * h {
            let (x, y) = slice_position(s, Some(sl), w, h);
            assert_eq!(out[y * w + x], s as u16, "{sl:?} sample {s}");
        }
    }
    // S14: n = 2, a = 1936, b = 1920, W = 5792, H = 3804.
    let (w, h) = (5792usize, 3804usize);
    let tag = |s: usize| ((s as u32).wrapping_mul(2_654_435_761) >> 16) as u16;
    let seq: Vec<u16> = (0..w * h).map(tag).collect();
    let out = sensor::reassemble(&seq, w, h, (2, 1936, 1920), &cancel).unwrap();
    for (s, (x, y)) in [
        (1935, (1935, 0)),
        (1936, (0, 1)),
        (7_364_543, (1935, 3803)),
        (7_364_544, (1936, 0)),
        (14_729_088, (3872, 0)),
        (14_731_008, (3872, 1)),
        (22_032_767, (5791, 3803)),
    ] {
        assert_eq!(out[y * w + x], tag(s), "TV-5 S14 sample {s}");
    }
    drop((seq, out));

    // TV-6 (R-CR2-22): layout C of S17, X = 3192, Nf = 4, one slice.
    let s17 = header_only(14, 3192, 2112, 4);
    let st = lj92::parse(&s17).unwrap();
    assert!(meta::is_v1_stream(&st));
    assert_eq!(meta::layout(3192, 4, 6384), Some(Layout::C));
    let t = sensor::targets(&st).unwrap();
    for ((v, u, k), s, (x, y)) in [
        ((0, 0, 1), 0, (0, 0)),
        ((0, 1595, 4), 6383, (6383, 0)),
        ((0, 1596, 1), 6384, (0, 1)),
        ((0, 3191, 4), 12767, (6383, 1)),
        ((1, 0, 1), 12768, (0, 2)),
    ] {
        let tk = t[k - 1];
        assert_eq!(tk.base + u * tk.xs + v * tk.ys, s, "TV-6 ({v}, {u}, {k})");
        assert_eq!(slice_position(s, None, 6384, 4224), (x, y));
    }
    assert_eq!(meta::slices(Some(&[0, 6384, 6384]), 6384).unwrap(), None);
    // S19: 0, 0, 4176 with W = 4192 is one slice of 4192 (R-CR2-16).
    assert_eq!(meta::slices(Some(&[0, 0, 4176]), 4192).unwrap(), None);
    assert!(meta::slices(Some(&[1, 4176, 0]), 4176).is_err(), "b = 0");
    assert!(meta::slices(Some(&[2, 10, 5]), 26).is_err(), "sum");
    assert!(meta::slices(Some(&[1, 10]), 20).is_err(), "two values");
    // A scan order other than the frame order still numbers the samples
    // by the scan position (R-CR2-22).
    let mut swapped = hex(TV3);
    let sos = swapped.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    swapped[sos + 5] = 2;
    swapped[sos + 7] = 1;
    let st = lj92::parse(&swapped).unwrap();
    let t = sensor::targets(&st).unwrap();
    assert_eq!((t[0].base, t[1].base), (1, 0));

    // TV-7 (R-CR2-14, R-CR2-18, R-CR2-24): SensorInfo of S15 and S18.
    let words = |s: &str| -> Vec<u32> {
        hex(s)
            .chunks_exact(2)
            .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
            .collect()
    };
    let mut s15 =
        words("22 00 20 17 6E 0F 01 00 01 00 8C 00 60 00 0B 17 5F 0F 00 00 00 00 00 00 00 00");
    s15.extend([0; 4]);
    assert_eq!(&s15[..3], &[34, 5920, 3950]);
    let (v, b) = meta::areas(&s15, 5920, 3950).unwrap();
    assert_eq!((v.left, v.top, v.right, v.bottom), (140, 96, 5899, 3935));
    assert_eq!((v.width(), v.height()), (5760, 3840));
    assert_eq!((b.left, b.top, b.right, b.bottom), (0, 96, 69, 3935));
    let s18 =
        words("22 00 A0 0E E0 0A 01 00 01 00 48 00 18 00 87 0E C7 0A 28 00 18 00 2B 00 C7 0A");
    let (v, b) = meta::areas(&s18, 3744, 2784).unwrap();
    assert_eq!((v.left, v.top, v.right, v.bottom), (72, 24, 3719, 2759));
    assert_eq!((b.left, b.top, b.right, b.bottom), (40, 24, 43, 2759));
    // R-CR2-14: with fewer than 13 values, indices 9 to 12 count as 0.
    let (_, b) = meta::areas(&s18[..12], 3744, 2784).unwrap();
    assert_eq!((b.left, b.right), (0, 35), "rule 2");
    // R-CR2-24: an overlapping rectangle falls back to rule 2.
    let mut over = s18.clone();
    over[11] = 80;
    let (_, b) = meta::areas(&over, 3744, 2784).unwrap();
    assert_eq!((b.left, b.right), (0, 35));

    // Fixtures F1 to F4: complete files (R-CR2-1 in layouts A, B and C).
    let fixtures: [Fixture; 4] = [
        (
            "F1",
            f1(),
            Some(Layout::A),
            (12, 6),
            16383,
            [0.373_239_1, 0.325_955_2, 0.002_599_0],
        ),
        (
            "F2",
            f2(),
            Some(Layout::C),
            (8, 6),
            15000,
            [0.484_304_9, 1.0, 0.677_314],
        ),
        (
            "F3",
            f3(),
            Some(Layout::B),
            (12, 4),
            4095,
            [0.676_789, 0.426_520_1, 0.121_890_6],
        ),
        (
            "F4",
            f4(),
            Some(Layout::A),
            (4, 8),
            4095,
            [0.466_265_1, 0.403_225_4, 0.214_639_2],
        ),
    ];
    let sizes = [652, 697, 638, 571];
    for ((name, data, layout, size, white, rgb), len) in fixtures.into_iter().zip(sizes) {
        assert_eq!(data.len(), len, "{name} length");
        let plan = plan_of(&data);
        assert_eq!(plan.layout, layout, "{name}");
        let (w, h, px, levels) = linear(&data);
        assert_eq!((w, h), size, "{name} output size");
        assert_eq!(levels.white, white, "{name} white level");
        for (i, p) in px.iter().enumerate() {
            assert_rgb(*p, rgb, EPS, &format!("{name} pixel {i}"));
        }
        // Through the decoder: the sensor path, Rgba16 sRGB codes
        // (R-CR2-42).
        let dev = report(&data);
        assert_eq!(dev.origin, Origin::Sensor, "{name}");
        assert_eq!(dev.class, Class::V1, "{name}");
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            size
        );
        assert_rgba16_flat(&dev.image, rgb, name);
        let info = dev.info.expect("info");
        assert_eq!(info.layout, layout);
        assert_eq!(info.levels, levels);
    }
    // F1 details: two slices of 10, CFA value 1, black rectangle columns 0
    // to 3, rows 2 to 7.
    let p = plan_of(&f1());
    assert_eq!(p.slices, Some((1, 10, 10)));
    assert_eq!(p.frame, (10, 8, 2));
    assert_eq!(
        (
            p.black_area.left,
            p.black_area.top,
            p.black_area.right,
            p.black_area.bottom
        ),
        (0, 2, 3, 7)
    );
    // F3: three slices 7, 7, 6 and CFA value 3; F4: no slice tag, CFA value
    // 4, black rectangle from SensorInfo, orientation 6 (R-CR2-36).
    let p = plan_of(&f3());
    assert_eq!((p.slices, p.cfa_value), (Some((2, 7, 6)), 3));
    let p = plan_of(&f4());
    assert_eq!((p.slices, p.cfa_value, p.orientation), (None, 4, 6));
    assert_eq!(
        (
            p.black_area.left,
            p.black_area.top,
            p.black_area.right,
            p.black_area.bottom
        ),
        (10, 2, 11, 5)
    );
    let (_, r) = raster_of(&f4());
    assert_eq!(r.at(10, 2), 128);
    assert_eq!(r.at(2, 2), 1628, "F4 raster (2, 2) is green under value 4");
    assert_eq!(r.at(3, 2), 1128, "F4 raster (3, 2) is red");
    assert_eq!(r.at(2, 3), 628, "F4 raster (2, 3) is blue");
}

/// Decode a stream to its sample sequence, or the first failure.
fn try_seq(stream: &[u8]) -> Cr2Result<Vec<u16>> {
    let s = lj92::parse(stream)?;
    if !meta::is_v1_stream(&s) {
        return Err(fail("not V1"));
    }
    sensor::sequence(&s, &CancelToken::new())
}

/// The stream faults of R-CR2-47 to R-CR2-50 on variants of TV-3, and a
/// faulty stream inside a file takes the preview path.
#[test]
fn cr2_stream_faults() {
    let tv3 = hex(TV3);
    let at = |m: [u8; 2]| tv3.windows(2).position(|w| w == m).unwrap();
    let (dht, sof, sos) = (at([0xFF, 0xC4]), at([0xFF, 0xC3]), at([0xFF, 0xDA]));
    assert!(try_seq(&tv3).is_ok());
    // R-CR2-47 case 1: no SOI.
    assert!(try_seq(&tv3[2..]).is_err(), "no SOI");
    // Case 2: SOF3, DHT or SOS missing; a frame header shorter than its
    // length field.
    let cut = |a: usize, b: usize| [&tv3[..a], &tv3[b..]].concat();
    assert!(try_seq(&cut(dht, sof)).is_err(), "no DHT");
    assert!(try_seq(&cut(sof, sos)).is_err(), "no SOF3");
    assert!(try_seq(&tv3[..sos]).is_err(), "no SOS");
    let mut short = tv3.clone();
    short[sof + 3] = 0x0B;
    assert!(try_seq(&short).is_err(), "short frame header");
    let mut short = tv3.clone();
    short[sos + 3] = 0x07;
    assert!(try_seq(&short).is_err(), "short scan header");
    // Case 3: a frame outside R-CR2-21 (P = 16).
    let mut p16 = tv3.clone();
    p16[sof + 4] = 16;
    assert!(try_seq(&p16).is_err(), "P = 16");
    // Case 4: the scan refers to the undefined table 2.
    let mut undefined = tv3.clone();
    undefined[sos + 8] = 0x20;
    assert!(try_seq(&undefined).is_err(), "undefined table");
    // R-CR2-48: counts above 256, above the bytes left, overflowing codes.
    let mut many = tv3.clone();
    many[dht + 5..dht + 21].copy_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 255, 2, 0, 0, 0, 0, 0, 0]);
    assert!(try_seq(&many).is_err(), "257 codes");
    let mut long = tv3.clone();
    long[dht + 5 + 15] = 40;
    assert!(try_seq(&long).is_err(), "counts beyond the segment");
    let mut overflow = tv3.clone();
    overflow[dht + 5] = 3;
    overflow[dht + 6] = 0;
    assert!(try_seq(&overflow).is_err(), "3 codes of length 1");
    // R-CR2-49: a code not in the table (all one bits), SSSS 17, a marker
    // before the last MCU.
    let data = |bytes: &str| [&tv3[..sos + 12], &hex(bytes)[..]].concat();
    assert!(
        try_seq(&data("FF 00 FF 00 FF 00 FF 00 FF 00 FF 00 FF D9")).is_err(),
        "code not in the table"
    );
    let mut ssss = tv3.clone();
    // HUFFVAL 6 (code 00) of both tables becomes 17.
    ssss[dht + 21] = 17;
    ssss[dht + 21 + 32] = 17;
    let mut bad = ssss[..sos + 12].to_vec();
    bad.extend(hex("00 00 00 00 FF D9"));
    assert!(try_seq(&bad).is_err(), "SSSS 17");
    let mut marker = tv3.clone();
    marker.splice(sos + 16..sos + 16, [0xFF, 0xC4, 0x00, 0x02]);
    assert!(try_seq(&marker).is_err(), "marker inside the data");
    let mut rst = tv3.clone();
    rst.splice(sos + 16..sos + 16, [0xFF, 0xD0]);
    assert!(try_seq(&rst).is_err(), "RST without DRI");
    // R-CR2-50: bits beyond the data fail; the end of the strip counts as
    // the end of the data; look-ahead bits that are not consumed do not
    // (TV-3 fills its last byte exactly, TV-10 leaves 11 bits).
    assert!(try_seq(&tv3[..tv3.len() - 4]).is_err(), "cut data");
    assert!(try_seq(&tv3[..tv3.len() - 2]).is_ok(), "no EOI");
    let mut padded = tv3[..tv3.len() - 2].to_vec();
    padded.extend(hex("FF FF FF D9"));
    assert!(try_seq(&padded).is_ok(), "fill bytes after the data");
    // Inside a file, a stream fault takes the preview path (R-CR2-41).
    let mut r = base();
    let mut s = r.encode();
    let dht = s.windows(2).position(|w| w == [0xFF, 0xC4]).unwrap();
    s[dht + 5] = 3;
    r.stream = Some(s);
    let dev = report(&r.build());
    assert_eq!((dev.origin, dev.class), (Origin::Preview, Class::X2));
}

// ---------------------------------------------------------------------------
// Synthetic round trips against an independent reference
// ---------------------------------------------------------------------------

/// An independent per-pixel implementation of R-CR2-29, R-CR2-34,
/// R-CR2-19 and R-CR2-32 in f64 on the raster: linear sRGB of every pixel
/// of the visible area, row-major, before orientation.
fn reference(r: &Recipe, plan: &Plan, levels: Levels) -> Vec<[f64; 3]> {
    let v = plan.visible;
    let color = |x: usize, y: usize| usize::from(plan.cfa[2 * (y % 2) + (x % 2)]);
    let norm = |x: usize, y: usize| {
        let p = 2 * (y % 2) + (x % 2);
        let b = f64::from(levels.black[p]);
        let s = f64::from(r.raster[y * r.w + x]);
        ((s - b) / (f64::from(levels.white) - b) * plan.wb[color(x, y)]).clamp(0.0, 1.0)
    };
    let mut out = Vec::new();
    for y in v.top..=v.bottom {
        for x in v.left..=v.right {
            let own = color(x, y);
            let mut sum = [0f64; 3];
            let mut cnt = [0u32; 3];
            for yy in y.saturating_sub(1)..=y + 1 {
                for xx in x.saturating_sub(1)..=x + 1 {
                    let inside =
                        (v.left..=v.right).contains(&xx) && (v.top..=v.bottom).contains(&yy);
                    if (xx, yy) == (x, y) || !inside {
                        continue;
                    }
                    let c = color(xx, yy);
                    sum[c] += norm(xx, yy);
                    cnt[c] += 1;
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
            out.push(
                [0, 1, 2].map(|k| {
                    (m[k][0] * cam[0] + m[k][1] * cam[1] + m[k][2] * cam[2]).clamp(0.0, 1.0)
                }),
            );
        }
    }
    out
}

/// A gradient with texture inside the visible area.
fn gradient(precision: u8) -> impl Fn(usize, usize) -> u16 {
    let top = (1u32 << precision) - 1;
    move |x, y| {
        let v = 900 + 23 * x as u32 + 41 * y as u32 + ((x * 7 + y * 13) % 29) as u32 * 11;
        v.min(top) as u16
    }
}

/// R-CR2-34 and R-CR2-35 on textured data: every CFA value, odd and even
/// visible area origins, the three layouts, slices, restart intervals and
/// both precisions agree with the independent reference, and the raster
/// is reassembled exactly (R-CR2-17, R-CR2-23).
#[test]
fn cr2_demosaic_matches_reference() {
    let cases = [
        // (W, H, Nf, P, layout C, slices, restart, L, T, cfa)
        (24, 10, 2, 14, false, None, 0, 8, 2, 1),
        (24, 10, 2, 14, false, Some((2, 8, 8)), 0, 9, 1, 2),
        (24, 10, 4, 12, false, Some((1, 16, 8)), 0, 10, 3, 3),
        (24, 10, 4, 14, true, None, 0, 11, 2, 4),
        (24, 12, 4, 14, true, Some((2, 6, 12)), 0, 8, 1, 1),
        (24, 10, 2, 12, false, None, 24, 9, 2, 3),
        (24, 10, 4, 14, false, Some((1, 12, 12)), 12, 8, 3, 4),
    ];
    for (i, &(w, h, nf, p, c, slices, restart, l, t, cfa)) in cases.iter().enumerate() {
        let what = format!("case {i}");
        let v = [l, t, w - 2, h - 1];
        let mut r = Recipe::new((w, h), nf, p, v, 1024, gradient(p));
        // A few unusual masked columns (M5): the lower median ignores them.
        for y in 0..h {
            r.raster[y * w] = 2149;
        }
        r.layout_c = c;
        r.slices = slices;
        r.slice_tag = slices.map(|(n, a, b)| vec![n as u16, a as u16, b as u16]);
        r.restart = restart;
        r.raw_extra.push((0xC5E0, V::Long(vec![cfa])));
        let f = r.build();
        let (plan, raster) = raster_of(&f);
        assert_eq!(raster.data, r.raster, "{what}: raster (R-CR2-17)");
        assert_eq!(plan.cfa_value, cfa);
        let (ow, oh, px, levels) = linear(&f);
        assert_eq!(levels.black, [1024; 4], "{what}: lower median");
        assert_eq!(levels.white, (1 << p) - 1, "{what}: white level");
        assert_eq!((ow, oh), (w - 2 - l + 1, h - t), "{what}");
        let want = reference(&r, &plan, levels);
        for (k, (g, e)) in px.iter().zip(&want).enumerate() {
            let e = e.map(|x| x as f32);
            assert_rgb(*g, e, 2e-5, &format!("{what} pixel {k}"));
        }
    }
}

// ---------------------------------------------------------------------------
// Levels, white balance and matrix
// ---------------------------------------------------------------------------

/// The white level of a 64 x 64 raster (all visible) of precision `p`
/// with `c` samples at `m` and the others at m / 2, black 0.
fn white_of(p: u32, m: u16, c: usize) -> Cr2Result<u32> {
    let mut plan = plan_of(&f1());
    plan.precision = p;
    plan.visible = Area {
        left: 0,
        top: 0,
        right: 63,
        bottom: 63,
    };
    let mut data = vec![m / 2; 64 * 64];
    for d in data.iter_mut().take(c) {
        *d = m;
    }
    let r = Raster {
        data,
        width: 64,
        height: 64,
    };
    sensor::white_level(&r, &plan, [0; 4])
}

#[test]
fn cr2_levels_wb_matrix_vector() {
    // F1: R-CR2-25 black 2048, R-CR2-26 white 16383 (M = 6048 < 8192),
    // R-CR2-28 source 1: G = 1024, mR = 2429 / 1024, mB = 1783 / 1024.
    let p = plan_of(&f1());
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert!((p.wb[0] - 2.372_070_3).abs() < 5e-8);
    assert!((p.wb[2] - 1.741_210_9).abs() < 5e-8);
    let (_, _, _, levels) = linear(&f1());
    assert_eq!(levels.black, [2048; 4]);
    assert_eq!(levels.white, 16383);
    // R-CR2-29 worked example.
    let n = |s: f64, m: f64| ((s - 2048.0) / (16383.0 - 2048.0) * m).clamp(0.0, 1.0);
    assert!((n(4048.0, p.wb[0]) - 0.330_948_1).abs() < 5e-8);
    assert!((n(6048.0, 1.0) - 0.279_037_3).abs() < 5e-8);
    assert!((n(3048.0, p.wb[2]) - 0.121_465_7).abs() < 5e-8);

    // R-CR2-30, R-CR2-31: the EOS 5D Mark III row, normalized (TV-11).
    assert!(p.matrix_from_table);
    let want = [
        [2.27, -1.42, 0.15],
        [-0.28, 1.67, -0.39],
        [0.040_404_0, -0.808_080_8, 1.767_676_8],
    ];
    for (r, w) in p.matrix.iter().zip(want) {
        for (g, e) in r.iter().zip(w) {
            assert!((g - e).abs() < 5e-8, "{:?}", p.matrix);
        }
    }
    // R-CR2-32 worked example (TV-11).
    let v = [0.330_948_1, 0.279_037_3, 0.121_465_7];
    let out: Vec<f64> = p
        .matrix
        .iter()
        .map(|r| r[0] * v[0] + r[1] * v[1] + r[2] * v[2])
        .collect();
    for (g, e) in out.iter().zip([0.373_239_1, 0.325_955_2, 0.002_599_0]) {
        assert!((g - e).abs() < 1e-6, "TV-11 {out:?}");
    }
    // A neutral input stays neutral for every normalized matrix.
    let ids = color::TABLE
        .iter()
        .map(|r| Some(r.0))
        .chain([None, Some(7)]);
    for id in ids {
        let (m, _) = color::matrix(id);
        for r in m {
            let g = r[0] * 0.25 + r[1] * 0.25 + r[2] * 0.25;
            assert!((g - 0.25).abs() < 1e-12, "{id:?}");
        }
    }
    // The table: 70 rows, distinct ids, the generic matrix and scales are
    // its means (transcription check).
    assert_eq!(color::TABLE.len(), 70);
    for (i, r) in color::TABLE.iter().enumerate() {
        assert!(color::TABLE[..i].iter().all(|o| o.0 != r.0), "{:#x}", r.0);
    }
    for (k, g) in color::GENERIC.iter().enumerate() {
        let mean = color::TABLE.iter().map(|r| r.3[k]).sum::<f64>() / 70.0;
        assert!((mean - g).abs() <= 0.0005 + 1e-9, "element {k}: {mean}");
    }
    let wr = color::TABLE.iter().map(|r| r.1).sum::<f64>() / 70.0;
    let wb = color::TABLE.iter().map(|r| r.2).sum::<f64>() / 70.0;
    assert!((wr - color::GENERIC_WB.0).abs() <= 0.0005, "{wr}");
    assert!((wb - color::GENERIC_WB.1).abs() <= 0.0005, "{wb}");
    let (m, from_table) = color::matrix(Some(0x8000_9999));
    assert!(!from_table);
    assert!((m[0][0] - 2.039 / (2.039 - 1.142 + 0.104)).abs() < 1e-12);

    // TV-8 (R-CR2-27, R-CR2-28).
    let close = |a: f64, b: f64| (a - b).abs() < 5e-8;
    let mut s15 = vec![0i32; 1312];
    s15[0] = 10;
    s15[63..67].copy_from_slice(&[1689, 1024, 1024, 2217]);
    let (m, src) = color::white_balance(Some(&s15), Some(0x8000_0285));
    assert_eq!(src, WbSource::AsShot);
    assert!(close(m[0], 1.649_414_1) && close(m[2], 2.165_039_1) && m[1] == 1.0);
    let mut s12 = vec![0i32; 582];
    s12[0] = 1164;
    s12[25..29].copy_from_slice(&[2170, 1018, 1020, 1755]);
    assert_eq!(color::wb_index(&s12), Some(25));
    let (m, _) = color::white_balance(Some(&s12), None);
    assert!(
        close(m[0], 2.129_538_8) && close(m[2], 1.722_276_7),
        "{m:?}"
    );
    let mut s18 = vec![0i32; 2560];
    s18[0] = -3;
    s18[71..75].copy_from_slice(&[1483, 889, 889, 1922]);
    let (m, _) = color::white_balance(Some(&s18), None);
    assert!(
        close(m[0], 1.668_166_5) && close(m[2], 2.161_979_8),
        "{m:?}"
    );
    s18[0] = -4;
    assert_eq!(color::wb_index(&s18), Some(71));
    // Version 99, no ColorData, a zero WB value: the table scales of the
    // CanonModelID; an unknown model: the generic scales.
    let mut odd = s15.clone();
    odd[0] = 99;
    for cd in [Some(&odd[..]), None, Some(&s15[..60])] {
        let (m, src) = color::white_balance(cd, Some(0x8000_0285));
        assert_eq!((src, m), (WbSource::Table, [2.02, 1.0, 1.73]));
    }
    let mut zero = s15.clone();
    zero[64] = 0;
    assert_eq!(
        color::white_balance(Some(&zero), Some(0x8000_0285)).1,
        WbSource::Table
    );
    let (m, src) = color::white_balance(None, Some(0x8000_9999));
    assert_eq!((src, m), (WbSource::Generic, [2.073, 1.0, 1.672]));
    // Index rules: 66 values with a version are too short; ColorData5 needs
    // 75 values.
    assert_eq!(color::wb_index(&s15[..66]), None);
    assert_eq!(color::wb_index(&s18[..74]), None);
    assert_eq!(color::wb_index(&s15[..67]), Some(63));
    // F3: version 99 and an unknown model: generic scales and matrix.
    let p = plan_of(&f3());
    assert_eq!(
        (p.wb_source, p.wb),
        (WbSource::Generic, [2.073, 1.0, 1.672])
    );
    assert!(!p.matrix_from_table);
    // F4: ColorData as UNDEFINED bytes (R-CR2-13), version -3, index 71.
    let p = plan_of(&f4());
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert!(close(p.wb[0], 1.668_166_5) && close(p.wb[2], 2.161_979_8));
    // R-CR2-13: SHORT values are read as signed (0xFFFD is -3); a LONG
    // ColorData counts as absent.
    let base = Recipe::new(
        (24, 6),
        2,
        12,
        [8, 2, 23, 5],
        128,
        flat_cfa(1, [1128, 1628, 628]),
    );
    let mut r = base.clone();
    let mut cd = vec![0u16; 80];
    cd[0] = 0xFFFD;
    cd[71..75].copy_from_slice(&[1483, 889, 889, 1922]);
    r.maker_extra.push((0x4001, V::Short(cd)));
    let p = plan_of(&r.build());
    assert_eq!(p.wb_source, WbSource::AsShot);
    assert!(close(p.wb[0], 1.668_166_5));
    let mut r = base.clone();
    r.maker_extra.push((0x4001, V::Long(vec![10; 70])));
    assert_eq!(plan_of(&r.build()).wb_source, WbSource::Table);
    let mut r = base.clone();
    r.maker_drop.push(0x0010);
    r.maker_drop.push(0x4001);
    let p = plan_of(&r.build());
    assert_eq!(
        (p.wb_source, p.matrix_from_table),
        (WbSource::Generic, false)
    );

    // TV-9 black: one CFA position holds 1024 1030 1022 2149 1025 1023:
    // sorted, rank floor(5 / 2) = 2 is 1024 (R-CR2-25).
    let mut data = vec![500u16; 12 * 2];
    for (i, v) in [1024, 1030, 1022, 2149, 1025, 1023].iter().enumerate() {
        data[2 * i] = *v;
    }
    let r = Raster {
        data,
        width: 12,
        height: 2,
    };
    let area = Area {
        left: 0,
        top: 0,
        right: 11,
        bottom: 1,
    };
    let b = sensor::black_levels(&r, area, &CancelToken::new()).unwrap();
    assert_eq!(b, [1024, 500, 500, 500]);
    // TV-9 white (R-CR2-26).
    assert_eq!(white_of(14, 15464, 1899).unwrap(), 15464);
    assert_eq!(white_of(14, 12223, 1).unwrap(), 16383);
    assert_eq!(white_of(14, 6048, 36).unwrap(), 16383);
    assert_eq!(white_of(14, 16383, 4096).unwrap(), 16383);
    assert_eq!(white_of(12, 3398, 3903).unwrap(), 3398);
    assert_eq!(white_of(12, 1756, 24).unwrap(), 4095);
    assert_eq!(white_of(14, 15464, 16).unwrap(), 15464, "C = 16");
    assert_eq!(white_of(14, 15464, 15).unwrap(), 16383, "C = 15");
    assert_eq!(white_of(14, 8192, 20).unwrap(), 8192, "M = 2^(P - 1)");
    // F2: M = 15000 with C = 24.
    assert_eq!(linear(&f2()).3.white, 15000);
    // The preview path when w <= max(b) + 255.
    let mut plan = plan_of(&f1());
    plan.precision = 12;
    plan.visible = Area {
        left: 0,
        top: 0,
        right: 63,
        bottom: 63,
    };
    let r = Raster {
        data: vec![100; 64 * 64],
        width: 64,
        height: 64,
    };
    assert!(sensor::white_level(&r, &plan, [3840, 0, 0, 0]).is_err());
    assert!(sensor::white_level(&r, &plan, [3839, 0, 0, 0]).is_ok());
    // R-CR2-51: values clip to 0 and 1 (below black, above white).
    let mut r = Recipe::new((24, 6), 2, 14, [8, 2, 23, 5], 2048, |x, _| {
        if x % 3 == 0 {
            100
        } else {
            16383
        }
    });
    r.sensor_info = sensor_info(24, 6, [8, 2, 23, 5], [0; 4]);
    for p in linear(&r.build()).2 {
        for c in p {
            assert!((0.0..=1.0).contains(&c) && c.is_finite());
        }
    }
}

// ---------------------------------------------------------------------------
// Out of scope, failed checks, previews
// ---------------------------------------------------------------------------

/// A 32 x 12 layout A file in two slices with a 48 x 32 IFD0 preview and a
/// 16 x 8 IFD1 thumbnail.
fn base() -> Recipe {
    let mut r = Recipe::new(
        (32, 12),
        2,
        14,
        [8, 2, 31, 11],
        2048,
        flat_cfa(1, [4048, 6048, 3048]),
    );
    r.slices = Some((1, 16, 16));
    r.slice_tag = Some(vec![1, 16, 16]);
    r.preview = Some(jpeg(48, 32, [200, 40, 40]));
    r.thumbnail = Some(jpeg(16, 8, [40, 40, 200]));
    r
}

fn assert_preview(dev: &Developed, size: (u32, u32), rgb: [u8; 3], what: &str) {
    assert_eq!(dev.origin, Origin::Preview, "{what}");
    assert!(dev.info.is_none(), "{what}");
    assert_eq!(dev.image.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((dev.image.width(), dev.image.height()), size, "{what}");
    let p = &dev.image.data()[..4];
    for c in 0..3 {
        assert!(p[c].abs_diff(rgb[c]) <= 6, "{what}: {p:?} vs {rgb:?}");
    }
}

/// The stream of `r` with the scan header byte at `k` after the scan
/// component list (0 Ss, 1 Se, 2 Ah Al) set to `v`.
fn with_scan_byte(r: &Recipe, k: usize, v: u8) -> Vec<u8> {
    let mut s = r.encode();
    let sos = s.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let ns = usize::from(s[sos + 4]);
    s[sos + 5 + 2 * ns + k] = v;
    s
}

/// A frame from the ARW test encoder with the given components.
fn frame(p: u8, x: usize, y: usize, hv: &[(usize, usize)]) -> Vec<u8> {
    use crate::raw::arw::fixtures::{lj92 as encode, Comp};
    let (hmax, vmax) = (
        hv.iter().map(|c| c.0).max().unwrap(),
        hv.iter().map(|c| c.1).max().unwrap(),
    );
    let planes: Vec<Vec<u16>> = hv
        .iter()
        .map(|&(h, v)| vec![1000; x.div_ceil(hmax) * h * y.div_ceil(vmax) * v])
        .collect();
    let comps: Vec<Comp<'_>> = hv
        .iter()
        .zip(&planes)
        .map(|(&(h, v), s)| Comp {
            h,
            v,
            samples: s,
            width: x.div_ceil(hmax) * h,
        })
        .collect();
    encode(p, x, y, &comps, 0, false)
}

#[test]
fn cr2_out_of_scope_falls_back_to_preview() {
    // A good file develops; its classification is V1 (R-CR2-3 rule 5).
    let good = base().build();
    let dev = report(&good);
    assert_eq!((dev.origin, dev.class), (Origin::Sensor, Class::V1));
    assert_eq!((dev.image.width(), dev.image.height()), (24, 10));
    // R-CR2-38: the largest candidate (the IFD0 strip here) is chosen.
    let check = |f: &[u8], class: Class, what: &str| {
        let dev = report(f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        assert_eq!(dev.class, class, "{what}");
    };
    // X1 (R-CR2-2, R-CR2-3 rule 3): SRawType 4.
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    check(&r.build(), Class::X1, "X1 SRawType");
    // X1 (rule 4): an sRAW-like frame, Nf = 3 with H = 2, P = 15, without
    // the SRawType tag.
    let mut r = base();
    r.raw_drop.push(0xC6C5);
    r.stream = Some(frame(15, 16, 12, &[(2, 1), (1, 1), (1, 1)]));
    check(&r.build(), Class::X1, "X1 frame");
    let mut r = base();
    r.stream = Some(frame(14, 16, 12, &[(1, 2), (1, 1)]));
    check(&r.build(), Class::X1, "X1 V = 2");
    let mut r = base();
    r.stream = Some(frame(14, 16, 12, &[(1, 1), (1, 1), (1, 1)]));
    check(&r.build(), Class::X1, "X1 Nf = 3");
    // X2 (rule 2): Compression 7, no StripOffsets, an invalid raw IFD.
    let mut r = base();
    r.raw_extra.push((0x0103, V::Short(vec![7])));
    check(&r.build(), Class::X2, "X2 compression");
    let mut r = base();
    r.raw_drop.push(0x0103);
    check(&r.build(), Class::X2, "X2 no compression");
    let mut r = base();
    r.raw_drop.push(0x0111);
    check(&r.build(), Class::X2, "X2 no StripOffsets");
    let mut f = good.clone();
    f[12..16].copy_from_slice(&0xFFFF_FF00u32.to_le_bytes());
    check(&f, Class::X2, "X2 raw IFD outside");
    // X2 (rule 6, R-CR2-21): P, Nf and scan parameters outside V1.
    for (p, nf) in [(16, 2), (8, 2), (13, 2), (14, 1)] {
        let mut r = base();
        r.stream = Some(frame(p, 32 / nf, 12, &vec![(1, 1); nf]));
        check(&r.build(), Class::X2, &format!("X2 P {p} Nf {nf}"));
    }
    let r = base();
    for (k, v, what) in [
        (0, 2, "predictor 2"),
        (1, 1, "Se 1"),
        (2, 0x10, "Ah 1"),
        (2, 1, "Pt 1"),
    ] {
        let mut x = base();
        x.stream = Some(with_scan_byte(&r, k, v));
        check(&x.build(), Class::X2, what);
    }
    // R-CR2-12: two strips.
    let mut r = base();
    r.raw_extra.push((0x0111, V::Long(vec![16, 32])));
    r.raw_extra.push((0x0117, V::Long(vec![4, 4])));
    check(&r.build(), Class::X2, "two strips");
    // X3: not a CR2 file. The decoder declines (R-CR2-2) and the provider
    // reports it through its generic path.
    let mut x3 = good.clone();
    x3[8..11].copy_from_slice(b"XY\x02");
    assert!(decode_report(&x3, &DecodeOptions::default())
        .unwrap()
        .is_none());
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(jpeg(20, 10, [9, 9, 9]), Some("x.cr2".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().width(), 20);

    // Failed checks of V1 use the preview (R-CR2-41).
    let fails: Vec<(&str, Recipe)> = vec![
        ("no SensorInfo", {
            let mut r = base();
            r.maker_drop.push(0x00E0);
            r
        }),
        ("SensorInfo of 8 values", {
            let mut r = base();
            r.sensor_info.truncate(8);
            r
        }),
        ("W * H differs from the frame", {
            let mut r = base();
            r.sensor_info[1] = 34;
            r
        }),
        ("no MakerNote", {
            let mut r = base();
            r.no_maker_note = true;
            r
        }),
        ("slices do not add up", {
            let mut r = base();
            r.slice_tag = Some(vec![1, 16, 15]);
            r
        }),
        ("slice tag of 2 values", {
            let mut r = base();
            r.slice_tag = Some(vec![1, 16]);
            r
        }),
        ("visible area outside", {
            let mut r = base();
            r.sensor_info[7] = 32;
            r
        }),
        ("visible area 1 wide", {
            let mut r = base();
            r.sensor_info[7] = 8;
            r
        }),
        ("CFA value 5", {
            let mut r = base();
            r.raw_extra.push((0xC5E0, V::Long(vec![5])));
            r
        }),
        ("CFA value 0", {
            let mut r = base();
            r.raw_extra.push((0xC5E0, V::Long(vec![0])));
            r
        }),
        ("L < 8 without a rectangle", {
            let mut r = base();
            r.sensor_info[5] = 6;
            r
        }),
        ("W above 65535", {
            let mut r = base();
            let mut si: Vec<u32> = r.sensor_info.iter().map(|&v| u32::from(v)).collect();
            si[1] = 70000;
            r.maker_extra.push((0x00E0, V::Long(si)));
            r
        }),
        ("white level not above black + 255", {
            let mut r = base();
            r.raster = Recipe::new((32, 12), 2, 14, [8, 2, 31, 11], 16200, |_, _| 16300).raster;
            r
        }),
        ("truncated entropy data", {
            let mut r = base();
            let s = r.encode();
            let mut cut = s[..s.len() / 2].to_vec();
            cut.extend_from_slice(&[0xFF, 0xD9]);
            r.stream = Some(cut);
            r
        }),
        ("a marker inside the entropy data", {
            let mut r = base();
            let mut s = r.encode();
            let at = s.len() - 20;
            s.splice(at..at, [0xFF, 0xC4, 0x00, 0x02]);
            r.stream = Some(s);
            r
        }),
        ("restart marker missing", {
            let mut r = base();
            r.restart = 16;
            let mut s = r.encode();
            let rst = s.windows(2).position(|w| w == [0xFF, 0xD0]).unwrap();
            s[rst + 1] = 0xD3;
            r.stream = Some(s);
            r
        }),
    ];
    for (what, r) in fails {
        let f = r.build();
        let dev = report(&f);
        assert_preview(&dev, (48, 32), [200, 40, 40], what);
        let c = container(&f);
        assert!(
            meta::classify(&c) != Class::V1
                || meta::plan(&c).is_err()
                || what.contains("entropy")
                || what.contains("white")
                || what.contains("restart"),
            "{what}"
        );
    }
    // R-CR2-23: a stream with restart intervals decodes.
    let mut r = base();
    r.restart = 16;
    assert_eq!(report(&r.build()).origin, Origin::Sensor);
    // Allocation bound: a frame of 64 million samples in a 300-byte strip
    // is refused by the plan, before the raster is allocated.
    let mut r = base();
    let mut big = header_only(14, 8000, 4000, 2);
    big.truncate(big.len() - 2);
    big.extend(vec![0u8; 200]);
    big.extend_from_slice(&[0xFF, 0xD9]);
    r.stream = Some(big);
    r.sensor_info[1] = 16000;
    r.sensor_info[2] = 4000;
    let f = r.build();
    assert!(meta::plan(&container(&f)).is_err());
    let start = Instant::now();
    assert_preview(&report(&f), (48, 32), [200, 40, 40], "short strip");
    assert!(start.elapsed() < Duration::from_secs(2));

    // R-CR2-41: the next-largest candidate when the largest fails.
    let big = jpeg(48, 32, [200, 40, 40]);
    let mut broken = big.clone();
    let sos = broken.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    broken.truncate(sos + 4);
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.preview = Some(broken);
    assert_preview(&report(&r.build()), (16, 8), [40, 40, 200], "next-largest");
    // An error only when no candidate decodes.
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.preview = Some(vec![0xFF, 0xD8, 0xFF, 0xC0, 0, 0]);
    r.thumbnail = Some(vec![1, 2, 3, 4]);
    assert!(decode_report(&r.build(), &DecodeOptions::default()).is_err());
    // R-CR2-37: the IFD0 strip needs Compression 6; a candidate shorter
    // than 4 bytes or without FF D8 is dropped; a thumbnail larger than the
    // strip wins (R-CR2-38).
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.thumbnail = Some(jpeg(64, 40, [10, 200, 10]));
    let f = r.build();
    assert_eq!(preview::collect(&container(&f)).len(), 2);
    assert_preview(&report(&f), (64, 40), [10, 200, 10], "larger thumbnail");
    let mut f = r.build();
    let c = container(&f);
    let at = c.ifd0().unwrap().entry(0x0103).unwrap().at as usize;
    f[at] = 1;
    assert_eq!(preview::collect(&container(&f)).len(), 1, "Compression 1");
    let mut r2 = r.clone();
    r2.thumbnail = Some(vec![0xFF, 0xD8, 0xFF]);
    assert_eq!(
        preview::collect(&container(&r2.build())).len(),
        1,
        "3 bytes"
    );
    r2.thumbnail = Some(jpeg(64, 40, [10, 200, 10])[2..].to_vec());
    assert_eq!(preview::collect(&container(&r2.build())).len(), 1, "no SOI");
    // R-CR2-39: bytes after EOI, and a stream without EOI.
    let mut padded = big.clone();
    padded.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00]);
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.preview = Some(padded);
    assert_preview(&report(&r.build()), (48, 32), [200, 40, 40], "after EOI");
    r.preview = Some(big[..big.len() - 2].to_vec());
    assert_preview(&report(&r.build()), (48, 32), [200, 40, 40], "no EOI");
    // R-CR2-40: the IFD0 orientation turns the preview, wide or tall.
    r.preview = Some(big.clone());
    r.orientation = 6;
    assert_preview(
        &report(&r.build()),
        (32, 48),
        [200, 40, 40],
        "orientation 6",
    );
    r.preview = Some(jpeg(32, 48, [200, 40, 40]));
    assert_preview(&report(&r.build()), (48, 32), [200, 40, 40], "tall, 6");
    // R-CR2-36: an orientation outside 1 to 8 counts as 1.
    let mut r = base();
    r.orientation = 9;
    assert_eq!(plan_of(&r.build()).orientation, 1);

    // Dual pixel (M11): IFD4 after the raw IFD is not decoded; the raw IFD
    // of the header is (R-CR2-8). The raw IFD need not be in the chain.
    let mut r = base();
    r.after_raw.push(vec![
        (0x0103, V::Short(vec![6])),
        (0xC6DD, V::Short(vec![0; 256])),
    ]);
    assert_eq!(report(&r.build()).origin, Origin::Sensor, "dual pixel");
    // R-CR2-11: the next-IFD field of the raw IFD is not followed: an IFD
    // reached only through it gives no preview candidate.
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.preview = None;
    r.thumbnail = None;
    let mut f = r.build();
    let raw_at = ifd::le32(&f, 12).unwrap() as usize;
    let n = usize::from(u16::from_le_bytes([f[raw_at], f[raw_at + 1]]));
    let next_field = raw_at + 2 + 12 * n;
    assert_eq!(ifd::le32(&f, next_field as u64), Some(0));
    f.resize(f.len().div_ceil(2) * 2, 0);
    let ifd4 = f.len();
    let thumb = jpeg(48, 32, [200, 40, 40]);
    let jpeg_at = ifd4 + 2 + 24 + 4;
    f.extend_from_slice(&2u16.to_le_bytes());
    f.extend(hex("01 02 04 00 01 00 00 00"));
    f.extend_from_slice(&(jpeg_at as u32).to_le_bytes());
    f.extend(hex("02 02 04 00 01 00 00 00"));
    f.extend_from_slice(&(thumb.len() as u32).to_le_bytes());
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(&thumb);
    assert!(ifd::dir(&f, ifd4 as u64).is_some());
    f[next_field..next_field + 4].copy_from_slice(&(ifd4 as u32).to_le_bytes());
    assert!(preview::collect(&container(&f)).is_empty(), "R-CR2-11");
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    let f = Builder {
        ifds: vec![vec![(0x010F, V::Ascii("Canon"))]],
        chain: vec![0],
        raw: None,
        ..Builder::default()
    }
    .build();
    assert_eq!(meta::classify(&container(&f)), Class::X2);
}

// ---------------------------------------------------------------------------
// Hostile input, truncation, cancellation, thumbnails
// ---------------------------------------------------------------------------

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    let mut c = Recipe::new((32, 12), 4, 14, [9, 3, 30, 11], 512, gradient(14));
    c.layout_c = true;
    c.slice_tag = Some(vec![0, 32, 32]);
    c.preview = Some(jpeg(24, 16, [40, 200, 40]));
    let mut b = Recipe::new((24, 10), 4, 12, [8, 2, 21, 9], 128, gradient(12));
    b.sensor_info = sensor_info(24, 10, [6, 2, 21, 9], [22, 2, 23, 9]);
    b.slices = Some((2, 8, 8));
    b.slice_tag = Some(vec![2, 8, 8]);
    b.restart = 12;
    b.orientation = 6;
    b.thumbnail = Some(jpeg(16, 8, [40, 40, 200]));
    let mut x1 = base();
    x1.raw_extra.push((0xC6C5, V::Long(vec![4])));
    x1.stream = Some(frame(15, 16, 12, &[(2, 1), (1, 1), (1, 1)]));
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("F4", f4()),
        ("A+previews", base().build()),
        ("C+preview", c.build()),
        ("B+restart+rectangle", b.build()),
        ("X1", x1.build()),
    ]
}

#[test]
fn cr2_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut next = noise(0x2545_F491);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        // The unmutated sample decodes.
        assert!(
            decode_report(&f, &DecodeOptions::default()).is_ok(),
            "{name}"
        );
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
                // Damage the directories and stream headers.
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
                // Huge values in IFD fields.
                _ => {
                    let i = (next() as usize % bad.len().min(600)) & !1;
                    let v = [u32::MAX, 0x7FFF_FFFF, 65535, 0, 1 << 27][next() as usize % 5];
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&v.to_le_bytes());
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
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.cr2".into()));
                let _ = reg.probe(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }
    // Oversized claims are refused before allocation (R-CR2-44, R-CR2-45).
    for (w, h) in [
        (65535u32, 65535u32),
        (0, 2),
        (4, 0),
        (70000, 2),
        (40000, 40000),
    ] {
        let mut r = base();
        let mut si: Vec<u32> = r.sensor_info.iter().map(|&v| u32::from(v)).collect();
        si[1] = w;
        si[2] = h;
        r.maker_extra.push((0x00E0, V::Long(si)));
        let f = r.build();
        assert!(meta::plan(&container(&f)).is_err(), "{w} x {h}");
        assert_eq!(report(&f).origin, Origin::Preview);
    }
    // A ColorData count far beyond the file (R-CR2-45): the entry is
    // patched to SHORT with 2^30 values.
    let mut r = base();
    r.maker_extra.push((0x4001, V::Long(vec![0x7FFF_FFFF; 2])));
    let mut f = r.build();
    let entry = f
        .windows(8)
        .position(|w| w == hex("01 40 04 00 02 00 00 00"))
        .expect("ColorData entry");
    f[entry + 2..entry + 4].copy_from_slice(&3u16.to_le_bytes());
    f[entry + 4..entry + 8].copy_from_slice(&0x4000_0000u32.to_le_bytes());
    let c = container(&f);
    assert!(meta::color_data(c.maker.as_ref().unwrap()).is_err());
    assert!(meta::plan(&c).is_err());
    assert_eq!(report(&f).origin, Origin::Preview);
}

#[test]
fn cr2_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        let step = (f.len() / 97).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 7, 8, 11, 12, 15, 16, 17, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => {
                    // R-CR2-53: a raw image is never delivered from a cut
                    // stream; only the preview remains.
                    assert_eq!(dev.origin, Origin::Preview, "{name} cut at {cut}")
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
}

/// A large layout A file: 2048 x 2048 in two slices.
fn large() -> Vec<u8> {
    let mut r = Recipe::new(
        (2048, 2048),
        2,
        14,
        [64, 16, 2047, 2047],
        1024,
        gradient(14),
    );
    r.slices = Some((1, 1024, 1024));
    r.slice_tag = Some(vec![1, 1024, 1024]);
    r.build()
}

#[test]
fn cr2_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    for f in [f1(), base().build()] {
        assert!(matches!(
            decode_report(&f, &cancelled),
            Err(Error::Cancelled)
        ));
    }
    let src = pandite_core::Source::from_bytes(f1(), Some("a.cr2".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    let plan = plan_of(&f1());
    assert!(matches!(
        sensor::decode(&f1(), &plan, &cancelled.cancel),
        Err(Error::Cancelled)
    ));
    // A long decode stops part way when cancelled from another thread: the
    // stream decoding checks the token every few MCU rows, the rendering
    // between bands.
    let f = large();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.origin, Origin::Sensor);
    assert_eq!((dev.image.width(), dev.image.height()), (1984, 2032));
    drop(dev);
    for frac in [10, 3] {
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
            std::thread::sleep((full / frac).min(Duration::from_millis(2)));
            token.cancel();
            h.join().unwrap()
        });
        let took = start.elapsed();
        println!("full decode {full:?}, cancelled at 1/{frac}: returned after {took:?}");
        assert!(matches!(result, Err(Error::Cancelled)), "not cancelled");
        // No time bound: a loaded machine makes one unreliable. The Cancelled
        // result above proves that the decode stopped before its end.
        println!("cancel took {took:?} of a {full:?} decode");
    }
}

#[test]
fn cr2_target_size_uses_preview() {
    // A 24 x 10 developed image with previews of 16 x 8 and 8 x 4.
    let mut r = base();
    r.preview = Some(jpeg(16, 8, [200, 10, 10]));
    r.thumbnail = Some(jpeg(8, 4, [10, 200, 10]));
    let f = r.build();
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    // The smallest preview that covers the target.
    let dev = decode_report(&f, &opts((6, 3))).unwrap().unwrap();
    assert_preview(&dev, (8, 4), [10, 200, 10], "small target");
    let dev = decode_report(&f, &opts((12, 6))).unwrap().unwrap();
    assert_preview(&dev, (16, 8), [200, 10, 10], "medium target");
    // A target larger than every preview develops the sensor data.
    let dev = decode_report(&f, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.origin, Origin::Sensor);
    assert_eq!((dev.image.width(), dev.image.height()), (24, 10));
    // Through the registry: a thumbnail request returns 8-bit preview data,
    // a full decode the developed image with the display rendering.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f.clone(), Some("t.cr2".into()));
    let img = reg.decode(&src, &opts((16, 16))).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba16);
    assert_eq!(img.first_image().width(), 24);
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (24, 10));
    // Without a name: the CR2 signature selects the decoder.
    let src = pandite_core::Source::from_bytes(f.clone(), None);
    assert!(crate::raw::is_camera_raw(&src));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height, meta.format), (24, 10, "raw"));
    // Probe of F4 (orientation 6) and of an X1 file (its preview).
    assert_eq!(
        probe(&f4()).unwrap(),
        Some((4, 8, pandite_core::Orientation::from_exif(6)))
    );
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.orientation = 8;
    assert_eq!(
        probe(&r.build()).unwrap().map(|p| (p.0, p.1)),
        Some((32, 48))
    );
    assert!(has_preview(&r.build()));
    assert!(!has_preview(&f1()));
}

/// Header facts: storage, CFA layout, precision and the camera of IFD0,
/// through the provider and the registry. Tags and stream headers only.
#[test]
fn facts_cr2() {
    use pandite_core::{DecoderProvider, Source};
    let f = facts(&f1()).expect("CR2 facts");
    assert_eq!(f.container.as_deref(), Some("Canon CR2"));
    assert_eq!(f.compression.as_deref(), Some("Lossless JPEG"));
    assert_eq!(f.channels.as_deref(), Some("Bayer RGGB"));
    assert_eq!((f.channel_count, f.bits_per_channel), (Some(1), Some(14)));
    assert_eq!(f.camera.make.as_deref(), Some("Canon"));
    assert_eq!(f.camera.model.as_deref(), Some("Canon EOS 5D Mark III"));
    let channels = |d: &[u8]| facts(d).and_then(|f| f.channels);
    assert_eq!(channels(&f3()).as_deref(), Some("Bayer GBRG"));
    assert_eq!(channels(&f4()).as_deref(), Some("Bayer GRBG"));
    assert_eq!(facts(&f3()).unwrap().bits_per_channel, Some(12));
    let mut r = base();
    r.raw_extra.push((0xC5E0, V::Long(vec![2])));
    assert_eq!(channels(&r.build()).as_deref(), Some("Bayer BGGR"));
    // X1: YCbCr with the subsampling of the luma component.
    let mut r = base();
    r.raw_extra.push((0xC6C5, V::Long(vec![4])));
    r.stream = Some(frame(15, 16, 12, &[(2, 1), (1, 1), (1, 1)]));
    let x1 = facts(&r.build()).unwrap();
    assert_eq!(x1.channels.as_deref(), Some("YCbCr"));
    assert_eq!((x1.channel_count, x1.bits_per_channel), (Some(3), Some(15)));
    assert_eq!(x1.subsampling.as_deref(), Some("4:2:2"));
    // Through the provider and the registry.
    let via = crate::raw::RawDecoder
        .facts(&Source::from_bytes(f1(), Some("a.cr2".into())))
        .unwrap();
    assert_eq!(via.channels.as_deref(), Some("Bayer RGGB"));
    let reg = crate::default_registry();
    let via = reg
        .facts(&Source::from_bytes(f2(), Some("IMG_0001.CR2".into())))
        .unwrap();
    assert_eq!(via.container.as_deref(), Some("Canon CR2"));
    // The head of the file: directories and stream headers come first.
    let f = base().build();
    let (off, _) = container(&f).strip().unwrap();
    let head = facts(&f[..off + 200]).expect("head facts");
    assert_eq!(head.channels.as_deref(), Some("Bayer RGGB"));
    for n in (0..f.len()).step_by(7) {
        let _ = facts(&f[..n]);
    }
    assert!(
        facts(b"MM\0*\0\0\0\x08CR\x02\0\0\0\0\0").is_none(),
        "big-endian"
    );
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// MD5 (RFC 1321) of `data`, as lower-case hex.
fn md5_hex(data: &[u8]) -> String {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k: Vec<u32> = (0..64)
        .map(|i| (f64::from(i + 1).sin().abs() * 4_294_967_296.0) as u32)
        .collect();
    let mut st = [0x6745_2301u32, 0xEFCD_AB89, 0x98BA_DCFE, 0x1032_5476];
    let mut block = |b: &[u8]| {
        let m: Vec<u32> = b
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        let [mut a, mut bb, mut c, mut d] = st;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((bb & c) | (!bb & d), i),
                1 => ((d & bb) | (!d & c), (5 * i + 1) % 16),
                2 => (bb ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (bb | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = bb;
            bb = bb.wrapping_add(f.rotate_left(S[i]));
        }
        st = [
            st[0].wrapping_add(a),
            st[1].wrapping_add(bb),
            st[2].wrapping_add(c),
            st[3].wrapping_add(d),
        ];
    };
    let full = data.len() / 64 * 64;
    for b in data[..full].chunks_exact(64) {
        block(b);
    }
    let mut tail = data[full..].to_vec();
    tail.push(0x80);
    while tail.len() % 64 != 56 {
        tail.push(0);
    }
    tail.extend_from_slice(&((data.len() as u64).wrapping_mul(8)).to_le_bytes());
    for b in tail.chunks_exact(64) {
        block(b);
    }
    st.iter()
        .flat_map(|v| v.to_le_bytes())
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn md5_helper_matches_rfc1321() {
    assert_eq!(md5_hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(md5_hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(
        md5_hex(
            b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
        ),
        "57edf4a22be3c955ac49da2e2107b67a"
    );
}

/// Sums linear sRGB (R-CR2-33) over the image: the TV-12 means.
struct MeanSum(Mutex<([f64; 3], u64)>);

impl render::RowEncoder for MeanSum {
    fn pixel_bytes(&self) -> usize {
        1
    }

    fn encode(&self, m: &[[f32; 3]; 3], cam: [&[f32]; 3], out: &mut [u8]) {
        let mut s = [0f64; 3];
        let [r, g, b] = cam;
        for ((&r, &g), &b) in r.iter().zip(g).zip(b).take(out.len()) {
            let v = render::to_srgb(m, [r, g, b]);
            for (acc, c) in s.iter_mut().zip(v) {
                *acc += f64::from(c);
            }
        }
        let mut total = self.0.lock().unwrap();
        for (acc, c) in total.0.iter_mut().zip(s) {
            *acc += c;
        }
        total.1 += out.len() as u64;
    }
}

/// TV-12: file, raster W x H, raster MD5, black b(0..3), w, mR and mB,
/// mean output r, g, b.
type Tv12 = (
    &'static str,
    (usize, usize),
    &'static str,
    [u32; 4],
    u32,
    (f64, f64),
    [f64; 3],
);

const TV12: [Tv12; 8] = [
    (
        "EOS_350D_RAW.CR2",
        (3516, 2328),
        "8fe9f0449e5fd5946b297dbaac580e43",
        [257; 4],
        4095,
        (2.129_538_8, 1.722_276_7),
        [0.08807, 0.08978, 0.08126],
    ),
    (
        "EOS_20D_RAW.CR2",
        (3596, 2360),
        "c45df124a2211bc8576b8aec394341b0",
        [128; 4],
        4095,
        (1.550_699_3, 2.173_951_0),
        [0.12976, 0.07774, 0.04426],
    ),
    (
        "EOS_5D_Mark_II_RAW.CR2",
        (5792, 3804),
        "6d66209bcefed3d77eb723e3371826e6",
        [1024; 4],
        16383,
        (2.426_757_8, 1.551_757_8),
        [0.07773, 0.06996, 0.06497],
    ),
    (
        "EOS_5D_Mark_III_RAW.CR2",
        (5920, 3950),
        "a18235d6abced4df9948b6fbe91aba7c",
        [2048, 2048, 2048, 2047],
        15464,
        (1.649_414_1, 2.165_039_1),
        [0.15664, 0.08921, 0.04530],
    ),
    (
        "EOS_80D_RAW.CR2",
        (6288, 4056),
        "c939b7ef677c1e137d0f6ff9e9cfda35",
        [512, 512, 513, 513],
        16383,
        (1.729_492_2, 1.784_179_7),
        [0.11926, 0.11899, 0.11609],
    ),
    (
        "EOS_6D_Mark_II_RAW.CR2",
        (6384, 4224),
        "d5c08b5bfdc1a2d88034fefc0a467b16",
        [513; 4],
        16383,
        (2.036_132_8, 1.774_414_1),
        [0.06500, 0.06222, 0.04481],
    ),
    (
        "PowerShot_G12_RAW.CR2",
        (3744, 2784),
        "bcead3d6a125a5ea5c4f3596fced0023",
        [128; 4],
        4095,
        (1.668_166_5, 2.161_979_8),
        [0.06161, 0.09369, 0.14029],
    ),
    (
        "PowerShot_S120_RAW.CR2",
        (4192, 3062),
        "649bf40b6cd80ea03b4d2f08ef8f06b4",
        [130; 4],
        4095,
        (2.161_764_7, 1.632_352_9),
        [0.22366, 0.12664, 0.07817],
    ),
];

/// TV-12 raster spot values: file, (x, y), value.
const SPOTS: [(&str, (usize, usize), u16); 13] = [
    ("EOS_350D_RAW.CR2", (0, 0), 137),
    ("EOS_350D_RAW.CR2", (1758, 0), 136),
    ("EOS_350D_RAW.CR2", (1758, 1164), 1288),
    ("EOS_5D_Mark_II_RAW.CR2", (2896, 1902), 2418),
    ("EOS_5D_Mark_III_RAW.CR2", (0, 0), 2044),
    ("EOS_5D_Mark_III_RAW.CR2", (1, 0), 1908),
    ("EOS_5D_Mark_III_RAW.CR2", (2, 0), 1927),
    ("EOS_5D_Mark_III_RAW.CR2", (3, 0), 1926),
    ("EOS_5D_Mark_III_RAW.CR2", (4, 0), 1951),
    ("EOS_5D_Mark_III_RAW.CR2", (5, 0), 1940),
    ("EOS_5D_Mark_III_RAW.CR2", (6, 0), 2231),
    ("EOS_6D_Mark_II_RAW.CR2", (3192, 2112), 1041),
    ("EOS_6D_Mark_II_RAW.CR2", (6383, 4223), 562),
];

/// Files whose embedded preview the camera corrected for lens distortion
/// (M14: the S19 file, PowerShot S120): the preview and the raw data differ
/// in geometry (M9: 0.154 over the whole frame), and the specification
/// forbids a lens correction (OQ-7). For these files the exact TV-12 check
/// values replace the preview tolerances; the comparison, whole frame and
/// central 30 percent, is printed. Without a matching TV-12 row the
/// preview tolerances apply as for every other file.
const LENS_CORRECTED: [&str; 1] = ["PowerShot_S120_RAW.CR2"];

/// The central `frac` of the width and height of an RGBA image.
fn center(img: &ImageBuf, frac: f64) -> ImageBuf {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let (cw, ch) = (
        ((w as f64 * frac) as usize).max(1),
        ((h as f64 * frac) as usize).max(1),
    );
    let (x0, y0) = ((w - cw) / 2, (h - ch) / 2);
    let px = img.format().bytes_per_pixel();
    let mut out = Vec::with_capacity(cw * ch * px);
    for y in y0..y0 + ch {
        out.extend_from_slice(&img.data()[(y * w + x0) * px..(y * w + x0 + cw) * px]);
    }
    ImageBuf::new(cw as u32, ch as u32, img.format(), out).unwrap()
}

/// Every `.CR2` file in the folder named by `PANDITE_CR2_SAMPLES` (an
/// absolute path, or relative to the workspace root): in-scope files (V1)
/// decode through the sensor path at the visible-area size and agree with
/// the file's own largest embedded JPEG by the method and tolerances of
/// `arw_real_samples_match_embedded_preview` (8 x 8 grid of cell means of
/// the default display rendering, one exposure gain, rg chromaticity
/// distance mean 0.05 and max 0.15, Spearman rank of cell luminance at
/// least 0.90; with one gain per channel first when the white balance did
/// not come from the file). Files listed in TV-12 must also reproduce the
/// raster MD5, the levels, the multipliers and the mean output within
/// 0.0005; for a file of [`LENS_CORRECTED`] these exact values replace
/// the preview tolerances. Out-of-scope files must return the preview.
#[test]
fn cr2_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_CR2_SAMPLES") else {
        println!("skip: PANDITE_CR2_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("cr2"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .CR2 files in {}", folder.display());
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let class = meta::classify(&c);
        let cands = preview::collect(&c);
        let start = Instant::now();
        let display = DecodeOptions {
            prefer_8bit: true,
            ..DecodeOptions::default()
        };
        let dev = decode_with(&data, &display, Rendering::Display)
            .expect("decode")
            .expect("CR2 path");
        let took = start.elapsed();
        if class != Class::V1 {
            assert_eq!(dev.origin, Origin::Preview, "{name}");
            let best = cands.first().expect("a preview");
            assert_eq!(
                (dev.image.width(), dev.image.height()),
                preview::displayed(best, c.orientation()),
                "{name}"
            );
            // TV-12: the two sRAW files deliver the 5760 x 3840 preview.
            if name.contains("sRAW") {
                assert_eq!(class, Class::X1, "{name}");
                assert_eq!((dev.image.width(), dev.image.height()), (5760, 3840));
            }
            println!(
                "{name}: {class:?} out of scope, preview {}x{}",
                dev.image.width(),
                dev.image.height()
            );
            continue;
        }
        // The sensor-data path, not the preview fallback.
        assert_eq!(
            dev.origin,
            Origin::Sensor,
            "{name} fell back to the preview"
        );
        let info = dev.info.clone().expect("raw facts");
        let v = info.visible;
        let size = render::output_size(
            arw::Rect {
                x: 0,
                y: 0,
                w: v.width(),
                h: v.height(),
            },
            info.orientation,
        );
        assert_eq!(
            (dev.image.width() as usize, dev.image.height() as usize),
            size,
            "{name}: the visible area"
        );
        // TV-12 check values.
        let mut tv = String::new();
        let mut tv_ok = false;
        if let Some(row) = TV12.iter().find(|r| r.0 == name) {
            let (plan, raster) = raster_of(&data);
            assert_eq!(info.raster, row.1, "{name}");
            let bytes: Vec<u8> = raster.data.iter().flat_map(|v| v.to_le_bytes()).collect();
            let digest = md5_hex(&bytes);
            drop(bytes);
            for &(_, (x, y), want) in SPOTS.iter().filter(|s| s.0 == name) {
                assert_eq!(raster.at(x, y), want, "{name} raster ({x}, {y})");
            }
            let sum = MeanSum(Mutex::new(([0.0; 3], 0)));
            let (_, levels) =
                develop_raster(raster, &plan, &CancelToken::new(), &sum).expect("develop");
            let (s, n) = *sum.0.lock().unwrap();
            let mean = s.map(|v| v / n as f64);
            let ok = digest == row.2
                && levels.black == row.3
                && levels.white == row.4
                && (info.wb[0] - row.5 .0).abs() < 5e-7
                && (info.wb[2] - row.5 .1).abs() < 5e-7
                && mean.iter().zip(row.6).all(|(a, b)| (a - b).abs() <= 0.0005);
            tv = format!(
                " | TV-12 {}: md5 {digest} black {:?} white {} mR {:.7} mB {:.7} mean {:.5} {:.5} {:.5}",
                if ok { "ok" } else { "MISMATCH" },
                levels.black,
                levels.white,
                info.wb[0],
                info.wb[2],
                mean[0],
                mean[1],
                mean[2]
            );
            if !ok {
                failures.push(format!("{name}: TV-12 {tv}"));
            }
            tv_ok = ok;
        }
        // The camera's largest JPEG, decoded small and oriented (R-CR2-40).
        let best = cands.first().expect("an embedded preview");
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let prev = preview::decode(&data, best, info.orientation, &opts).expect("preview");
        let ours = grid_means(&dev.image, 8, 4);
        let theirs = grid_means(&prev, 8, 1);
        // Without white balance from ColorData the specification uses the
        // table or generic scales (R-CR2-28) while the camera JPEG uses its
        // as-shot balance: compare after removing the global color cast.
        let balance = matches!(info.wb_source, WbSource::Table | WbSource::Generic);
        let (raw_mean, raw_max, _, _) = agreement(&ours, &theirs, false);
        let (mean, max, rank, used) = agreement(&ours, &theirs, balance);
        let lens = LENS_CORRECTED.contains(&name.as_str()) && tv_ok;
        let region = if lens {
            let (m, x, r, _) = agreement(
                &grid_means(&center(&dev.image, 0.3), 8, 2),
                &grid_means(&center(&prev, 0.3), 8, 1),
                balance,
            );
            format!(
                " (lens-corrected preview, M14, OQ-7: not enforced, TV-12 enforced; central 30 percent: chroma mean {m:.4} max {x:.4} rank {r:.3})"
            )
        } else {
            String::new()
        };
        let full = preview::displayed(best, info.orientation) == (size.0 as u32, size.1 as u32);
        let line = format!(
            "{name}: model {} V1 layout {:?} frame {:?} P {} raster {}x{} visible {}x{}+{}+{} out {}x{} slices {:?} cfa {} wb {:?} matrix {} decode {:.2}s (debug) | vs embedded {}x{}{}: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}{}{region}{tv}",
            info.model,
            info.layout,
            info.frame,
            info.precision,
            info.raster.0,
            info.raster.1,
            v.width(),
            v.height(),
            v.left,
            v.top,
            size.0,
            size.1,
            info.slices,
            info.cfa_value,
            info.wb_source,
            if info.matrix_from_table { "table" } else { "generic" },
            took.as_secs_f64(),
            best.width,
            best.height,
            if full { " (full size)" } else { "" },
            if balance {
                format!(" (after global balance; before: mean {raw_mean:.4} max {raw_max:.4})")
            } else {
                String::new()
            },
        );
        if !lens && (mean > CHROMA_MEAN || max > CHROMA_MAX || rank < RANK_MIN || used < 16) {
            failures.push(line.clone());
        }
        println!("{line}");
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

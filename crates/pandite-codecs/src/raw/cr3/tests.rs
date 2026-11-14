//! Tests of the CR3 decoder against the specification's test vectors
//! (TV-1 to TV-15), fixtures (F1 to F3) and requirements, plus hostile
//! input, cancellation, thumbnails and real camera files.

use super::boxes::{self, Bx, Container, Role};
use super::fixtures::*;
use super::meta;
use super::preview::{self, PreviewSource as P};
use super::*;
use crate::raw::arw::tests::{agreement, grid_means, noise, CHROMA_MAX, CHROMA_MEAN, RANK_MIN};
use pandite_core::{CancelToken, DecodeOptions, Error, PixelFormat};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn container(d: &[u8]) -> Container<'_> {
    boxes::open(d).expect("open").expect("a CR3 file")
}

fn report(d: &[u8]) -> Developed {
    decode_report(d, &DecodeOptions::default())
        .expect("decode")
        .expect("CR3 path")
}

/// The error text of a failed decode.
fn error_of(d: &[u8]) -> String {
    match decode_report(d, &DecodeOptions::default()) {
        Err(e) => e.to_string(),
        Ok(Some(dev)) => panic!("decoded {:?}", dev.source),
        Ok(None) => panic!("declined"),
    }
}

/// Every pixel of an Rgba8 image within `tol` of `rgb`.
fn assert_flat(img: &ImageBuf, size: (u32, u32), rgb: [u8; 3], tol: u8, what: &str) {
    assert_eq!(img.format(), PixelFormat::Rgba8, "{what}");
    assert_eq!((img.width(), img.height()), size, "{what}");
    for p in img.data().chunks_exact(4) {
        for c in 0..3 {
            assert!(p[c].abs_diff(rgb[c]) <= tol, "{what}: {p:?} vs {rgb:?}");
        }
    }
}

/// The RGB of pixel (x, y) of an Rgba8 image.
fn px(img: &ImageBuf, x: u32, y: u32) -> [u8; 3] {
    let i = ((y * img.width() + x) * 4) as usize;
    let d = img.data();
    [d[i], d[i + 1], d[i + 2]]
}

fn types(boxes: &[Bx]) -> Vec<String> {
    boxes
        .iter()
        .map(|b| String::from_utf8_lossy(&b.typ).into_owned())
        .collect()
}

/// (source, width, height, offset, length) of every candidate.
fn candidates(d: &[u8]) -> Vec<(P, u32, u32, usize, usize)> {
    preview::collect(&container(d))
        .iter()
        .map(|p| {
            (
                p.source,
                p.stream.width,
                p.stream.height,
                p.stream.offset,
                p.stream.len,
            )
        })
        .collect()
}

/// A file with the Canon metadata box of F1 (orientation `o`), one JPEG
/// track and no preview holder.
fn one_track(o: u16, w: u16, h: u16, stream: Vec<u8>) -> Vec<u8> {
    Cr3 {
        canon: vec![cncv(), cmt1(o)],
        tracks: vec![TrackSpec::jpeg(w, h, stream)],
        holder: None,
    }
    .build()
}

/// A THMB or PRVW box whose payload starts with `version`.
fn versioned(typ: &[u8; 4], version: u8, rest: &[u8]) -> Vec<u8> {
    bx(typ, &cat(&[&[version, 0, 0, 0], rest]))
}

/// The CRX sample bytes of the CRX fixtures: the start measured in M13
/// followed by a fake JPEG of 9000 x 6000, larger than every preview. The
/// decoder never reads it (R-CR3-28); a byte scan would take it.
fn crx_sample() -> Vec<u8> {
    let mut s = hex("FF 01 00 08 00 00 00 00");
    s.extend(hex(
        "FF D8 FF C0 00 11 08 17 70 23 28 03 01 21 00 02 11 01 03 11 01 FF DA 00 08 01 01 00 00 3F 00",
    ));
    s.extend([0u8; 64]);
    s
}

/// A file of variant X5 (M15): HEVC track 1, THMB and PRVW with version
/// 1, the CRX and metadata tracks of the other fixtures.
fn x5_file() -> Vec<u8> {
    let mut s = full_spec(3, false, crx_sample());
    s.canon[0] = bx(b"CNCV", b"CanonCR3_003/00.10.00/00.00.00");
    let thmb = versioned(
        b"THMB",
        1,
        &hex("01 40 00 D6 00 00 00 10 00 02 00 00 01 02 03 04"),
    );
    s.canon.retain(|b| &b[4..8] != b"THMB");
    s.canon.push(thmb);
    s.tracks[0] = TrackSpec {
        handler: *b"vide",
        entry: hevc_entry(48, 32),
        sample: Sample::Data(vec![0, 0, 0, 1, 0x40, 0x01, 0x0C]),
        fixed: false,
        count: 1,
    };
    s.holder = Some(vec![versioned(
        b"PRVW",
        1,
        &hex("00 02 06 54 04 38 FF FF 00 00 00 04 00 00 00 01"),
    )]);
    s.build()
}

// ---------------------------------------------------------------------------
// Fixtures and test vectors
// ---------------------------------------------------------------------------

#[test]
fn cr3_builder_reproduces_fixture_hex() {
    // F1 from its recipe equals the hex of the specification.
    let f1 = f1();
    assert_eq!(f1.len(), 928);
    assert_eq!(f1_spec(6).build(), f1, "F1 recipe");
    // TV-14 is the THMB stream of F1 and the form of every fixture JPEG.
    let tv14 = hex(TV14_HEX);
    assert_eq!(tv14.len(), 141);
    assert_eq!(gray_jpeg(8, 8, 128), tv14);
    assert_eq!(&f1[0xC4..0xC4 + 141], &tv14[..]);
    // F2 differs from F1 in the byte at 0x30F only.
    let f2 = f2();
    let diff: Vec<usize> = (0..928).filter(|&i| f1[i] != f2[i]).collect();
    assert_eq!(diff, vec![0x30F]);
    // F3 is 2741 bytes: F1 with orientation 1 (byte 0x090) and a second
    // track of 141 bytes inside "moov", the metadata sample in "mdat".
    let f3 = f3();
    assert_eq!(f3.len(), 2741);
    let o1 = f1_spec(1).build();
    let diff: Vec<usize> = (0..928).filter(|&i| f1[i] != o1[i]).collect();
    assert_eq!(diff, vec![0x090]);
    assert_eq!(&f3[0x18..0x1C], &(544u32 + 141).to_be_bytes());
}

/// TV-5: the "stsd" of track 1 of S14.
const TV5: &str = "
00 00 00 80 73 74 73 64 00 00 00 00 00 00 00 01
00 00 00 70 43 52 41 57 00 00 00 00 00 00 00 01
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
0D 50 08 E0 00 48 00 00 00 48 00 00 00 00 00 00
00 01 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 18 FF FF 00 03 00 00 00 00 00 0C 4A 50
45 47 00 00 00 00 00 00 00 0A 66 72 65 65 00 00
";
/// TV-6: "stsz" and "co64" of track 1 of S14.
const TV6_STSZ: &str = "00 00 00 18 73 74 73 7A 00 00 00 00 00 00 00 00 00 00 00 01 00 0B 32 44";
const TV6_CO64: &str = "00 00 00 18 63 6F 36 34 00 00 00 00 00 00 00 01 00 00 00 00 00 06 74 00";
/// TV-9: the SOF0 segment of the track-1 JPEG of S14.
const TV9: &str = "FF C0 00 11 08 08 E0 0D 50 03 01 21 00 02 11 01 03 11 01";

/// A "trak" with the given handler and "stbl" children.
fn trak(handler: &[u8; 4], stbl: &[u8]) -> Vec<u8> {
    let hdlr = bx(b"hdlr", &cat(&[&[0u8; 8], handler, &[0u8; 13]]));
    bx(
        b"trak",
        &bx(b"mdia", &cat(&[&hdlr, &bx(b"minf", &bx(b"stbl", stbl))])),
    )
}

/// TV-10 and TV-11: a metadata sample with the record of type 1 and the
/// record of type 8 of S14 (record 8 is 106518 bytes, its block 106506,
/// the TIFF block with 4 entries and ColorData at TIFF offset 58).
fn tv10_sample() -> Vec<u8> {
    let mut t = vec![0u8; 106_498];
    t[..10].copy_from_slice(&hex("49 49 2A 00 08 00 00 00 04 00"));
    t[10..22].copy_from_slice(&hex("01 40 03 00 48 0E 00 00 3A 00 00 00"));
    for (k, tag) in [0x4002u16, 0x4003, 0x4004].into_iter().enumerate() {
        let at = 22 + 12 * k;
        t[at..at + 2].copy_from_slice(&tag.to_le_bytes());
        t[at + 2..at + 12].copy_from_slice(&hex("03 00 01 00 00 00 05 00 00 00"));
    }
    let mut c = vec![0u16; 3656];
    c[0] = 33;
    // M10: c[0] and c[1] fill the next-IFD field with 64225313.
    c[1] = 980;
    c[85..90].copy_from_slice(&[1856, 1024, 1024, 1670, 4965]);
    c[343..347].copy_from_slice(&[512; 4]);
    c[810..813].copy_from_slice(&[12735, 14008, 12735]);
    for (i, v) in c.iter().enumerate() {
        t[58 + 2 * i..60 + 2 * i].copy_from_slice(&v.to_le_bytes());
    }
    let mut rec8 = hex("16 A0 01 00 08 00 01 01 01 01 FF FF 0A A0 01 00 7C 92 00 00");
    rec8.extend(t);
    assert_eq!(rec8.len(), 106_518);
    let mut s = hex("18 00 00 00 01 00 00 00 00 01 00 00");
    s.extend([0u8; 12]);
    s.extend(rec8);
    s
}

#[test]
fn cr3_vectors_decode() {
    // TV-1 and R-CR3-4.
    let tv1 = hex("00 00 00 18 66 74 79 70 63 72 78 20 00 00 00 01 63 72 78 20 69 73 6F 6D");
    assert_eq!(tv1, FTYP);
    assert!(boxes::is_cr3(&tv1));
    assert_eq!(types(&container(&tv1).top), vec!["ftyp"]);
    let mut isom = tv1.clone();
    isom[8..12].copy_from_slice(b"isom");
    assert!(!boxes::is_cr3(&isom));
    assert!(boxes::open(&isom).unwrap().is_none(), "not a CR3 file");
    assert!(decode_report(&isom, &DecodeOptions::default())
        .unwrap()
        .is_none());
    let mut short_ftyp = tv1.clone();
    short_ftyp[3] = 15;
    assert!(!boxes::is_cr3(&short_ftyp), "first box under 16 bytes");
    let mut to_end = tv1.clone();
    to_end[3] = 0;
    assert!(boxes::is_cr3(&to_end), "size 0: to the end of the file");
    // R-CR3-49: shorter than 24 bytes fails.
    assert!(boxes::open(&tv1[..23]).is_err());
    assert!(decode_report(&tv1[..23], &DecodeOptions::default()).is_err());

    // TV-2: box headers (R-CR3-5, R-CR3-6).
    let mut big = vec![0u8; 9_815_638];
    let len = big.len() as u64;
    big[24..32].copy_from_slice(&hex("00 00 8E 10 6D 6F 6F 76"));
    let moov = boxes::header(&big, 24, len, true).unwrap();
    assert_eq!((moov.typ, moov.body, moov.end), (*b"moov", 32, 36_392));
    big[422_896..422_912].copy_from_slice(&hex("00 00 00 01 6D 64 61 74 00 00 00 00 00 8F 52 66"));
    let mdat = boxes::header(&big, 422_896, len, true).unwrap();
    assert_eq!((mdat.body, mdat.end), (422_912, 9_815_638));
    drop(big);
    let mut s = vec![0u8; 1000];
    s[900..908].copy_from_slice(&hex("00 00 00 00 6D 64 61 74"));
    let b = boxes::header(&s, 900, 1000, true).unwrap();
    assert_eq!(b.size(), 100, "size 0 at the top level");
    assert!(
        boxes::header(&s, 900, 1000, false).is_none(),
        "size 0 inside"
    );
    s[..8].copy_from_slice(&hex("00 00 00 04 66 72 65 65"));
    assert!(boxes::header(&s, 0, 1000, true).is_none(), "size 4");
    // A uuid box needs 8 + 16 bytes; an extended size below 16 fails.
    assert!(boxes::header(&bx(b"uuid", &[0; 15]), 0, 23, true).is_none());
    assert!(boxes::header(&bx(b"uuid", &[0; 16]), 0, 24, true).is_some());
    let ext = hex("00 00 00 01 66 72 65 65 00 00 00 00 00 00 00 0F 00");
    assert!(boxes::header(&ext, 0, 17, true).is_none());
    // A box past the end of its parent is invalid; the walk keeps the
    // boxes before the first invalid one.
    let w = cat(&[
        &bx(b"free", &[0; 4]),
        &hex("00 00 00 04 66 72 65 65"),
        &bx(b"skip", &[]),
    ]);
    assert_eq!(types(&boxes::walk(&w, 0, w.len() as u64, 1)), vec!["free"]);
    let cut = &bx(b"free", &[0; 4])[..11];
    assert!(boxes::walk(cut, 0, 11, 1).is_empty());
    let inner = cat(&[&bx(b"abcd", &[0; 4]), &bx(b"free", &[])]);
    assert!(
        boxes::header(&inner, 0, 11, false).is_none(),
        "past the parent"
    );
    // R-CR3-7: an unknown type is skipped by its size.
    let w = cat(&[&bx(b"abcd", &[0xFF; 20]), &bx(b"free", &[])]);
    assert_eq!(
        types(&boxes::walk(&w, 0, w.len() as u64, 1)),
        vec!["abcd", "free"]
    );

    // TV-3: the Canon metadata box at 32 and its first child at 56.
    let filler = bx(b"free", &vec![0u8; 34_096 - 8 - 16 - 38 - 8]);
    let canon = uuid(&boxes::CANON_UUID, &cat(&[&cncv(), &filler]));
    assert_eq!(canon.len(), 34_096);
    let f = cat(&[&FTYP, &bx(b"moov", &canon)]);
    assert_eq!(
        &f[32..56],
        &hex("00 00 85 30 75 75 69 64 85 C0 B6 87 82 0F 11 E0 81 11 F4 CE 46 2B 6A 48")[..]
    );
    let c = container(&f);
    let cb = c.canon.expect("Canon metadata box");
    assert_eq!((cb.start, cb.size()), (32, 34_096));
    let first = c.canon_children[0];
    assert_eq!((first.typ, first.start, first.size()), (*b"CNCV", 56, 38));
    assert_eq!(first.payload(&f), b"CanonCR3_001/00.10.00/00.00.00");
    // R-CR3-8: all 16 bytes of the extended type count.
    let mut other = f.clone();
    other[32 + 8 + 15] ^= 1;
    assert!(container(&other).canon.is_none());

    // TV-4 and TV-5: handler and sample entry of track 1 (R-CR3-12 to
    // R-CR3-15); TV-6: the sample location (R-CR3-17, R-CR3-18).
    let hdlr = hex(
        "00 00 00 21 68 64 6C 72 00 00 00 00 00 00 00 00 76 69 64 65 00 00 00 00 00 00 00 00 00 00 00 00 00",
    );
    let stsd = hex(TV5);
    assert_eq!(stsd.len(), 128);
    let tk = trak(b"vide", &cat(&[&stsd, &hex(TV6_STSZ), &hex(TV6_CO64)]));
    assert_eq!(&tk[16..49], &hdlr[..], "TV-4");
    let mut f = cat(&[&FTYP, &bx(b"moov", &tk)]);
    f.resize(1_156_676, 0);
    let c = container(&f);
    let t = &c.tracks[0];
    assert_eq!(t.handler, Some(*b"vide"));
    assert_eq!(t.format, Some(*b"CRAW"));
    assert_eq!(t.role, Role::Jpeg);
    let (at, size) = t.entry.unwrap();
    assert_eq!(size, 112);
    assert_eq!(boxes::be16(&f, at + 32), Some(3408));
    assert_eq!(boxes::be16(&f, at + 34), Some(2272));
    assert_eq!(&f[at as usize + 86..at as usize + 90], &[0, 3, 0, 0]);
    assert_eq!(types(&t.entry_children), vec!["JPEG", "free"]);
    assert_eq!(t.entry_children[0].start, at + 90);
    assert_eq!(
        t.entry_children.iter().map(Bx::size).collect::<Vec<_>>(),
        vec![12, 10]
    );
    assert_eq!(t.sample, Some((422_912, 733_764)));
    assert_eq!(t.samples, 1);
    f.truncate(1_156_675);
    assert_eq!(container(&f).tracks[0].sample, None, "R-CR3-18");

    // R-CR3-17: "stco" when there is no "co64"; "co64" first; the fixed
    // sample size; R-CR3-18: missing boxes and zero counts.
    let stco = hex("00 00 00 14 73 74 63 6F 00 00 00 00 00 00 00 01 00 00 00 30");
    let fixed = hex("00 00 00 14 73 74 73 7A 00 00 00 00 00 00 00 10 00 00 00 01");
    let sample_of = |stbl: &[u8]| {
        let mut f = cat(&[&FTYP, &bx(b"moov", &trak(b"vide", stbl))]);
        f.resize(f.len().max(0x80), 0);
        container(&f).tracks[0].sample
    };
    assert_eq!(sample_of(&cat(&[&stsd, &fixed, &stco])), Some((0x30, 16)));
    let co64 = hex("00 00 00 18 63 6F 36 34 00 00 00 00 00 00 00 01 00 00 00 00 00 00 00 40");
    assert_eq!(
        sample_of(&cat(&[&stsd, &fixed, &stco, &co64])),
        Some((0x40, 16))
    );
    assert_eq!(sample_of(&cat(&[&stsd, &stco])), None, "no stsz");
    assert_eq!(sample_of(&cat(&[&stsd, &fixed])), None, "no chunk offsets");
    let zero = hex("00 00 00 14 73 74 73 7A 00 00 00 00 00 00 00 10 00 00 00 00");
    assert_eq!(sample_of(&cat(&[&stsd, &zero, &stco])), None, "0 entries");
    let no_chunks = hex("00 00 00 10 73 74 63 6F 00 00 00 00 00 00 00 00");
    assert_eq!(sample_of(&cat(&[&stsd, &fixed, &no_chunks])), None);
    // R-CR3-13 and R-CR3-14: an entry smaller than 8 or past "stsd", and a
    // CRAW entry shorter than 90 bytes, give no JPEG track.
    for (what, entry) in [
        ("size 7", hex("00 00 00 07 43 52 41 57")),
        ("past stsd", hex("00 00 00 70 43 52 41 57")),
        (
            "under 90",
            cat(&[
                &hex("00 00 00 18 43 52 41 57"),
                &bx(b"JPEG", &[0; 4]),
                &[0; 4],
            ]),
        ),
    ] {
        let sd = bx(b"stsd", &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &entry]));
        let f = cat(&[&FTYP, &bx(b"moov", &trak(b"vide", &sd))]);
        assert_eq!(container(&f).tracks[0].role, Role::Other, "{what}");
    }
    // R-CR3-15: the handler and the data format count, not the position.
    let sd = bx(
        b"stsd",
        &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &jpeg_entry(8, 8)]),
    );
    for (h, role) in [
        (b"vide", Role::Jpeg),
        (b"soun", Role::Other),
        (b"meta", Role::Other),
    ] {
        let f = cat(&[&FTYP, &bx(b"moov", &trak(h, &sd))]);
        assert_eq!(container(&f).tracks[0].role, role);
    }
    let sd = bx(b"stsd", &cat(&[&[0, 0, 0, 0, 0, 0, 0, 1], &ctmd_entry()]));
    let f = cat(&[&FTYP, &bx(b"moov", &trak(b"meta", &sd))]);
    assert_eq!(container(&f).tracks[0].role, Role::Metadata);

    // TV-7: the THMB box (R-CR3-19, R-CR3-20).
    let mut th =
        hex("00 00 3D F8 54 48 4D 42 00 00 00 00 00 A0 00 78 00 00 3D DF 00 01 00 00 FF D8 FF DB");
    th.resize(15_864, 0);
    let b = boxes::header(&th, 0, th.len() as u64, true).unwrap();
    assert_eq!((b.typ, b.size()), (*b"THMB", 15_864));
    assert_eq!(boxes::be16(&th, b.body + 4), Some(160));
    assert_eq!(boxes::be16(&th, b.body + 6), Some(120));
    let (off, l) = preview::boxed_stream(&th, &b, 8).unwrap();
    // At file offset 18264: the JPEG occupies 18288 to 34126, 1 fill byte.
    assert_eq!((18_264 + off, l), (18_288, 15_839));
    assert_eq!(b.end - (off + l), 1);
    let mut long = th.clone();
    long[19] = 0xE0;
    assert_eq!(
        preview::boxed_stream(&long, &b, 8),
        Some((24, 15_840)),
        "exact fit"
    );
    long[19] = 0xE1;
    assert!(
        preview::boxed_stream(&long, &b, 8).is_none(),
        "16 + L past the payload"
    );
    let mut v1 = th.clone();
    v1[8] = 1;
    assert!(preview::boxed_stream(&v1, &b, 8).is_none(), "version 1");

    // TV-8: the preview holder box and PRVW (R-CR3-11, R-CR3-19).
    let mut f = vec![0u8; 422_808];
    f[..24].copy_from_slice(&FTYP);
    f[24..32].copy_from_slice(&hex("00 01 8E 28 66 72 65 65"));
    f[101_952..101_952 + 60].copy_from_slice(&hex(
        "00 04 E5 58 75 75 69 64 EA F4 2B 5E 1C 98 4B 88 B9 FB B7 DC 40 6E 4D 16 00 00 00 00 00 00 00 01 00 04 E5 38 50 52 56 57 00 00 00 00 00 01 06 54 04 38 00 01 00 04 E5 1F FF D8 FF DB",
    ));
    let c = container(&f);
    let h = c.holder.expect("preview holder");
    assert_eq!((h.start, h.size()), (101_952, 320_856));
    let p = c.prvw[0];
    assert_eq!((p.start, p.size()), (101_984, 320_824));
    assert_eq!(boxes::be16(&f, p.body + 6), Some(1620));
    assert_eq!(boxes::be16(&f, p.body + 8), Some(1080));
    let (off, l) = preview::boxed_stream(&f, &p, 12).unwrap();
    assert_eq!((off, off + l - 1), (102_008, 422_806));

    // TV-9: the SOF0 segment gives 3408 x 2272 (R-CR3-37).
    let stream = cat(&[&[0xFF, 0xD8], &hex(TV9), &[0xFF, 0xD9]]);
    assert_eq!(
        crate::raw::arw::preview::frame_size(&stream),
        Some((3408, 2272))
    );

    // TV-10: the records and blocks of the metadata sample (R-CR3-25,
    // R-CR3-26); TV-11: the ColorData values (R-CR3-27).
    let s = tv10_sample();
    let recs = meta::records(&s);
    assert_eq!(
        recs.iter().map(|(t, p)| (*t, p.len())).collect::<Vec<_>>(),
        vec![(1, 12), (8, 106_506)]
    );
    let blocks = meta::blocks(recs[1].1);
    assert_eq!(blocks.len(), 1);
    assert_eq!((blocks[0].0, blocks[0].1.len()), (0x927C, 106_498));
    let t = blocks[0].1;
    // The value bytes at the file offsets of TV-11, relative to the TIFF
    // block at 9706274.
    assert_eq!(&t[58..60], &[0x21, 0x00]);
    assert_eq!(&t[228..238], &hex("40 07 00 04 00 04 86 06 65 13")[..]);
    assert_eq!(&t[744..752], &hex("00 02 00 02 00 02 00 02")[..]);
    assert_eq!(&t[1678..1684], &hex("BF 31 B8 36 BF 31")[..]);
    let cd = meta::color_data(&meta::block(t).unwrap()).unwrap();
    assert_eq!((cd.version, cd.count), (33, 3656));
    assert_eq!(cd.wb_levels, Some([1856, 1024, 1024, 1670]));
    assert_eq!(cd.color_temp, Some(4965));
    assert_eq!(cd.black, Some([512; 4]));
    assert_eq!(
        (cd.normal_white, cd.specular_white),
        (Some(12735), Some(14008))
    );
    assert_eq!(cd.linearity_upper_margin, Some(12735));
    assert_eq!(cd.multipliers, Some([1.8125, 1.0, 1.630_859_375]));

    // TV-12: SensorInfo (R-CR3-34).
    let mut cmt3 = vec![0u8; 12_066 + 34];
    cmt3[..10].copy_from_slice(&hex("49 49 2A 00 08 00 00 00 01 00"));
    cmt3[10..22].copy_from_slice(&hex("E0 00 03 00 11 00 00 00 22 2F 00 00"));
    for (i, v) in [34u16, 3584, 2386, 1, 1, 156, 108, 3563, 2379]
        .iter()
        .enumerate()
    {
        cmt3[12_066 + 2 * i..12_068 + 2 * i].copy_from_slice(&v.to_le_bytes());
    }
    assert_eq!(
        &cmt3[12_066..12_084],
        &hex("22 00 00 0E 52 09 01 00 01 00 9C 00 6C 00 EB 0D 4B 09")[..]
    );
    let with_cmt3 = |cmt3: &[u8]| {
        Cr3 {
            canon: vec![cncv(), cmt1(1), bx(b"CMT3", cmt3)],
            tracks: vec![],
            holder: Some(vec![prvw(16, 8, &gray_jpeg(16, 8, 96))]),
        }
        .build()
    };
    let f = with_cmt3(&cmt3);
    let si = meta::sensor_info(&container(&f)).expect("SensorInfo");
    assert_eq!(si.size, (3584, 2386));
    assert_eq!(si.crop, (156, 108, 3563, 2379));
    assert_eq!(si.crop_size(), (3408, 2272));
    let mut eight = cmt3.clone();
    eight[14] = 8;
    assert!(
        meta::sensor_info(&container(&with_cmt3(&eight))).is_none(),
        "8 values"
    );
    let mut inverted = cmt3.clone();
    inverted[12_080..12_082].copy_from_slice(&100u16.to_le_bytes());
    assert!(
        meta::sensor_info(&container(&with_cmt3(&inverted))).is_none(),
        "right < left"
    );

    // TV-13: CMP1 and the variant (R-CR3-2, R-CR3-16).
    let raw =
        hex("FF 00 00 30 01 00 00 00 00 00 0E 00 00 00 09 52 00 00 0E 00 00 00 09 52 0E 40 00 00");
    let craw =
        hex("FF 10 00 30 02 00 00 00 00 00 0E 00 00 00 09 52 00 00 0E 00 00 00 09 52 0E 40 03 00");
    for (payload, variant) in [(raw, Variant::X1), (craw, Variant::X2)] {
        let mut p = payload.clone();
        p.resize(52, 0);
        let cmp1 = bx(b"CMP1", &p);
        assert_eq!(&cmp1[..8], &hex("00 00 00 3C 43 4D 50 31")[..]);
        let mut spec = f1_spec(1);
        spec.tracks.push(TrackSpec {
            handler: *b"vide",
            entry: craw_entry(3408, 2272, CRX_FIELD, &cmp1),
            sample: Sample::Data(crx_sample()),
            fixed: false,
            count: 1,
        });
        let f = spec.build();
        let c = container(&f);
        let main = c.main_crx().expect("main CRX track");
        assert_eq!(main.cmp1_size(&f), Some((3584, 2386)));
        assert_eq!(c.variant(), variant);
        assert_eq!(report(&f).variant, variant);
    }

    // TV-14: the minimal JPEG decodes to 64 gray pixels of 128; SOF1 is
    // accepted too (R-CR3-29).
    let tv14 = hex(TV14_HEX);
    let img = crate::jpeg::decode_image(&tv14, &DecodeOptions::default()).unwrap();
    assert_flat(&img, (8, 8), [128; 3], 0, "TV-14");
    let mut sof1 = tv14.clone();
    let at = sof1.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    sof1[at + 1] = 0xC1;
    let f = one_track(1, 8, 8, sof1);
    assert_flat(&report(&f).image, (8, 8), [128; 3], 0, "SOF1");

    // TV-15: the orientation mapping (R-CR3-38, R-CR3-39) on a 3 x 2
    // preview of flat 8 x 8 blocks a b c / d e f.
    let vals = [40u8, 80, 120, 160, 200, 240];
    let stream = blocks_jpeg(3, 2, &vals);
    let at_block = |img: &ImageBuf, x: u32, y: u32| px(img, x * 8 + 4, y * 8 + 4)[0];
    for o in 1..=8u16 {
        let dev = report(&one_track(o, 24, 16, stream.clone()));
        let (w2, h2) = if o >= 5 { (2, 3) } else { (3, 2) };
        assert_eq!(
            (dev.image.width(), dev.image.height()),
            (w2 * 8, h2 * 8),
            "orientation {o}"
        );
        for (k, &v) in vals.iter().enumerate() {
            let (x, y) = (k as u32 % 3, k as u32 / 3);
            let (ox, oy) = match o {
                1 => (x, y),
                2 => (2 - x, y),
                3 => (2 - x, 1 - y),
                4 => (x, 1 - y),
                5 => (y, x),
                6 => (1 - y, x),
                7 => (1 - y, 2 - x),
                _ => (y, 2 - x),
            };
            let got = at_block(&dev.image, ox, oy);
            assert!(got.abs_diff(v) <= 1, "orientation {o} block {k}: {got}");
        }
    }
    // The rows TV-15 states: 6 gives d a / e b / f c, 8 gives c f / b e /
    // a d, 3 gives f e d / c b a.
    let rows = |o: u16| {
        let img = report(&one_track(o, 24, 16, stream.clone())).image;
        let (bw, bh) = (img.width() / 8, img.height() / 8);
        (0..bh)
            .map(|y| {
                (0..bw)
                    .map(|x| at_block(&img, x, y) / 40)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    // a..f are 1..6 after the division.
    assert_eq!(rows(6), vec![vec![4, 1], vec![5, 2], vec![6, 3]]);
    assert_eq!(rows(8), vec![vec![3, 6], vec![2, 5], vec![1, 4]]);
    assert_eq!(rows(3), vec![vec![6, 5, 4], vec![3, 2, 1]]);
    // A 2 x 3 (taller than wide) preview with orientation 6 stays 2 x 3.
    let tall = blocks_jpeg(2, 3, &vals);
    let dev = report(&one_track(6, 16, 24, tall));
    assert_eq!((dev.image.width(), dev.image.height()), (16, 24));
    assert_eq!(at_block(&dev.image, 1, 0), 80, "not rotated");

    // F1: candidates P1 32 x 16, P2 16 x 8, P3 8 x 8; P1 delivered,
    // rotated by orientation 6 to 16 x 32, gray 200; variant unknown;
    // ColorSpace 1 (no CMT2).
    let f = f1();
    let c = container(&f);
    assert_eq!(
        c.top
            .iter()
            .map(|b| (b.typ, b.start, b.size()))
            .collect::<Vec<_>>(),
        vec![
            (*b"ftyp", 0, 24),
            (*b"moov", 0x18, 544),
            (*b"uuid", 0x238, 199),
            (*b"mdat", 0x2FF, 161)
        ]
    );
    assert_eq!(c.canon.map(|b| (b.start, b.size())), Some((0x20, 305)));
    assert_eq!(
        c.canon_children
            .iter()
            .map(|b| (b.typ, b.start, b.size()))
            .collect::<Vec<_>>(),
        vec![
            (*b"CNCV", 0x38, 38),
            (*b"CMT1", 0x5E, 78),
            (*b"THMB", 0xAC, 165)
        ]
    );
    let t = &c.tracks[0];
    assert_eq!(t.entry, Some((0x1A2, 102)));
    // The JPEG child box starts at entry offset 90 (0x1FC; the Canon field
    // 00 03 00 00 is at 0x1F8).
    assert_eq!(
        t.entry_children
            .iter()
            .map(|b| (b.typ, b.start, b.size()))
            .collect::<Vec<_>>(),
        vec![(*b"JPEG", 0x1FC, 12)]
    );
    assert_eq!(t.sample, Some((0x30F, 145)));
    assert_eq!(
        c.prvw
            .iter()
            .map(|b| (b.start, b.size()))
            .collect::<Vec<_>>(),
        vec![(0x258, 167)]
    );
    assert_eq!(
        candidates(&f),
        vec![
            (P::P1, 32, 16, 0x30F, 145),
            (P::P2, 16, 8, 0x270, 143),
            (P::P3, 8, 8, 0xC4, 141)
        ]
    );
    let m = meta::read(&c);
    assert_eq!(m.make.as_deref(), Some("Canon"));
    assert_eq!(m.model.as_deref(), Some("Canon EOS R6"));
    assert_eq!((m.orientation, m.color_space), (6, 1));
    let dev = report(&f);
    assert_eq!((dev.source, dev.stored), (P::P1, (32, 16)));
    assert_flat(&dev.image, (16, 32), [200; 3], 0, "F1");
    assert_eq!(dev.variant, Variant::Unknown);
    assert!(!dev.dual_pixel);
    assert_eq!(dev.color_space, 1);

    // F2: P1 no longer starts with FF D8 and is dropped (R-CR3-36): P2
    // rotated to 8 x 16, gray 96.
    let f = f2();
    assert_eq!(
        candidates(&f).iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![P::P2, P::P3]
    );
    let dev = report(&f);
    assert_eq!(dev.source, P::P2);
    assert_flat(&dev.image, (8, 16), [96; 3], 0, "F2");

    // F3: P1 32 x 16 not rotated; the metadata track (R-CR3-15, R-CR3-17,
    // R-CR3-25 to R-CR3-27) and its values.
    let f = f3();
    let c = container(&f);
    assert_eq!(c.moov.map(|b| b.size()), Some(685));
    assert_eq!(c.tracks[0].sample, Some((924, 145)));
    assert_eq!(c.tracks[1].role, Role::Metadata);
    assert_eq!(c.tracks[1].sample, Some((1069, 1672)));
    assert_eq!(
        &f[1069..1089],
        &hex("88 06 00 00 08 00 01 01 01 01 FF FF 7C 06 00 00 7C 92 00 00")[..]
    );
    let dev = report(&f);
    assert_eq!(dev.source, P::P1);
    assert_flat(&dev.image, (32, 16), [200; 3], 0, "F3");
    let cd = dev.meta.color.expect("ColorData");
    assert_eq!(cd.multipliers, Some([1.8125, 1.0, 1.630_859_375]));
    assert_eq!(cd.black, Some([512; 4]));
    assert_eq!(
        (cd.normal_white, cd.specular_white),
        (Some(12735), Some(14008))
    );

    // R-CR3-9: boxes found by type in any order (the preview holder before
    // "moov", the Canon metadata box after the tracks).
    let f = f1();
    let (ftyp, moov, holder, mdat) = (&f[..0x18], &f[0x18..0x238], &f[0x238..0x2FF], &f[0x2FF..]);
    let canon_box = &moov[8..8 + 305];
    let trak_box = &moov[8 + 305..];
    let moov2 = bx(b"moov", &cat(&[trak_box, canon_box]));
    let g = cat(&[ftyp, holder, &moov2, mdat]);
    // The sample did not move: the same bytes come before "mdat".
    assert_eq!(g.len(), f.len());
    let dev = report(&g);
    assert_eq!((dev.source, dev.meta.orientation), (P::P1, 6));
    // R-CR3-37: the earlier candidate on equal size; R-CR3-36: every JPEG
    // track in "moov" order.
    let s = Cr3 {
        canon: vec![cncv(), cmt1(1)],
        tracks: vec![
            TrackSpec::jpeg(16, 8, gray_jpeg(16, 8, 40)),
            TrackSpec::jpeg(16, 8, gray_jpeg(16, 8, 80)),
        ],
        holder: Some(vec![prvw(16, 8, &gray_jpeg(16, 8, 120))]),
    }
    .build();
    assert_eq!(
        candidates(&s).iter().map(|c| c.0).collect::<Vec<_>>(),
        vec![P::P1, P::P1, P::P2]
    );
    assert_flat(&report(&s).image, (16, 8), [40; 3], 0, "earlier on a tie");
}

// ---------------------------------------------------------------------------
// Levels, white balance and color
// ---------------------------------------------------------------------------

/// A ColorData block of `count` values of `typ` with `set` applied.
fn color_block(version: i16, count: usize, typ: u16, set: &[(usize, i16)]) -> Vec<u8> {
    let mut c = vec![0i16; count];
    if count > 0 {
        c[0] = version;
    }
    for &(i, v) in set {
        if i < count {
            c[i] = v;
        }
    }
    let bytes: Vec<u8> = c.iter().flat_map(|v| v.to_le_bytes()).collect();
    tiff(true, &[(0x4001, Val::Raw(typ, count as u32, bytes))])
}

/// A file with F1's previews and a metadata track with `sample`.
fn with_ctmd(sample: Vec<u8>) -> Vec<u8> {
    let mut s = f1_spec(1);
    s.tracks.push(TrackSpec::ctmd(sample));
    s.build()
}

/// The ColorData values of the metadata track of a file.
fn color_of(f: &[u8]) -> Option<meta::ColorData> {
    meta::find_color_data(&container(f))
}

/// Levels and white balance (section Levels and white balance) and the
/// color of the previews (section Color conversion). The specification
/// gives no color matrix (OQ-10): the previews are camera-rendered sRGB,
/// delivered with the S6 conversion only, and the levels and multipliers
/// are reported, never applied (R-CR3-33).
#[test]
fn cr3_levels_wb_matrix_vector() {
    // TV-11 through the whole path: the metadata track of a file.
    let dev = report(&with_ctmd(tv10_sample()));
    let cd = dev.meta.color.expect("ColorData");
    assert_eq!(cd.version, 33);
    assert_eq!(cd.wb_levels, Some([1856, 1024, 1024, 1670]));
    assert_eq!(cd.multipliers, Some([1.8125, 1.0, 1.630_859_375]));
    assert_eq!(cd.color_temp, Some(4965));
    assert_eq!(cd.black, Some([512; 4]));
    assert_eq!(
        (cd.normal_white, cd.specular_white),
        (Some(12735), Some(14008))
    );
    // R-CR3-33: the levels do not modify the delivered image: F3's gray
    // 200 stays neutral although the multipliers are 1.81 and 1.63.
    let dev = report(&f3());
    assert!(dev.meta.color.is_some());
    assert_flat(&dev.image, (32, 16), [200; 3], 0, "levels not applied");

    // Every layout of the table, read at its own indices.
    let layouts: [(&[i16], [usize; 6]); 4] = [
        (&[16, 17, 18, 19], [71, 75, 329, 796, 797, 798]),
        (&[32, 33], [85, 89, 343, 810, 811, 812]),
        (&[34, 48], [105, 109, 363, 640, 641, 642]),
        (&[64, 65, 66, 67], [105, 109, 383, 660, 661, 662]),
    ];
    for (versions, [wb, temp, black, normal, specular, margin]) in layouts {
        for &v in versions {
            let set = [
                (wb, 2000),
                (wb + 1, 1000),
                (wb + 2, 1048),
                (wb + 3, 1536),
                (temp, 5200),
                (black, 2047),
                (black + 1, 2048),
                (black + 2, 2049),
                (black + 3, 2050),
                (normal, 13035),
                (specular, -14000),
                (margin, 10000),
            ];
            let note = color_block(v, 1000, 3, &set);
            let cd = color_of(&with_ctmd(ctmd_record(8, &[(0x927C, note)]))).unwrap();
            assert_eq!(cd.version, v);
            assert_eq!(cd.wb_levels, Some([2000, 1000, 1048, 1536]), "version {v}");
            // R-CR3-32: G = (1000 + 1048) / 2 = 1024.
            assert_eq!(cd.multipliers, Some([2000.0 / 1024.0, 1.0, 1.5]));
            assert_eq!(cd.color_temp, Some(5200));
            assert_eq!(cd.black, Some([2047, 2048, 2049, 2050]));
            // R-CR3-33: unsigned (the value -14000 is 51536).
            assert_eq!(cd.normal_white, Some(13035));
            assert_eq!(cd.specular_white, Some(51536));
            assert_eq!(cd.linearity_upper_margin, Some(10000));
        }
    }
    // OQ-12, R-CR3-31: a version outside the table reports the version and
    // nothing else.
    let cd = color_of(&with_ctmd(ctmd_record(
        8,
        &[(0x927C, color_block(20, 1000, 3, &[]))],
    )))
    .unwrap();
    assert_eq!(cd.version, 20);
    assert_eq!((cd.wb_levels, cd.multipliers, cd.black), (None, None, None));
    assert_eq!((cd.normal_white, cd.specular_white), (None, None));
    // R-CR3-31 and R-CR3-33: each value only when its index lies inside
    // the count (version 33 with 344 values: WB and temperature known,
    // black levels from 343 to 346 not, white levels not).
    let set = [(85, 1856), (86, 1024), (87, 1024), (88, 1670), (343, 512)];
    let cd = color_of(&with_ctmd(ctmd_record(
        8,
        &[(0x927C, color_block(33, 344, 3, &set))],
    )))
    .unwrap();
    assert_eq!(cd.wb_levels, Some([1856, 1024, 1024, 1670]));
    assert_eq!((cd.black, cd.normal_white), (None, None));
    let cd = color_of(&with_ctmd(ctmd_record(
        8,
        &[(0x927C, color_block(33, 88, 3, &set))],
    )))
    .unwrap();
    assert_eq!(
        (cd.wb_levels, cd.multipliers),
        (None, None),
        "c[88] outside"
    );
    // R-CR3-32 and R-CR3-55: a level of 0 or below makes the multipliers
    // unknown (no division by 0, no NaN).
    for bad in [0i16, -5] {
        for k in 0..4 {
            let mut set = vec![(85, 1856), (86, 1024), (87, 1024), (88, 1670)];
            set[k].1 = bad;
            let cd = color_of(&with_ctmd(ctmd_record(
                8,
                &[(0x927C, color_block(33, 1000, 8, &set))],
            )))
            .unwrap();
            assert!(cd.wb_levels.is_some());
            assert_eq!(cd.multipliers, None, "level {k} = {bad}");
        }
    }
    assert_eq!(meta::multipliers([1, 1, 1, 1]), Some([1.0, 1.0, 1.0]));
    // R-CR3-27: SHORT or SSHORT only; the first record of type 8 and its
    // first block with the tag 0x927C only.
    let good = color_block(33, 1000, 3, &set_tv11());
    let long = tiff(true, &[(0x4001, Val::Long(vec![33; 900]))]);
    assert!(color_of(&with_ctmd(ctmd_record(8, &[(0x927C, long.clone())]))).is_none());
    let sshort = color_block(33, 1000, 8, &set_tv11());
    assert!(color_of(&with_ctmd(ctmd_record(8, &[(0x927C, sshort)]))).is_some());
    let first_then = |a: Vec<u8>, b: Vec<u8>| {
        let mut s = ctmd_record(1, &[]);
        s.extend(ctmd_record(7, &[(0x927C, good.clone())]));
        s.extend(a);
        s.extend(b);
        color_of(&with_ctmd(s))
    };
    // Record 7 before record 8 is not used; a later record 8 is not used.
    assert!(first_then(
        ctmd_record(8, &[(0x8769, good.clone())]),
        ctmd_record(8, &[(0x927C, good.clone())])
    )
    .is_none());
    // Within record 8: the first 0x927C block, even when a later one has
    // ColorData.
    let other = tiff(true, &[(0x0001, Val::Short(vec![1]))]);
    assert!(first_then(
        ctmd_record(
            8,
            &[
                (0x8769, other.clone()),
                (0x927C, other),
                (0x927C, good.clone())
            ]
        ),
        vec![]
    )
    .is_none());
    assert!(first_then(
        ctmd_record(8, &[(0x8769, good.clone()), (0x927C, good.clone())]),
        vec![]
    )
    .is_some());
    // R-CR3-22: a big-endian TIFF block (OQ-15) is read in its own byte
    // order; a header without 42 is not a TIFF block.
    let be: Vec<u8> = {
        let mut c = vec![0i16; 1000];
        c[0] = 48;
        c[105..109].copy_from_slice(&[2144, 1024, 1024, 1362]);
        let bytes = c.iter().flat_map(|v| v.to_be_bytes()).collect();
        tiff(false, &[(0x4001, Val::Raw(3, 1000, bytes))])
    };
    let cd = color_of(&with_ctmd(ctmd_record(8, &[(0x927C, be.clone())]))).unwrap();
    assert_eq!(cd.wb_levels, Some([2144, 1024, 1024, 1362]));
    let mut not42 = be;
    not42[3] = 43;
    assert!(color_of(&with_ctmd(ctmd_record(8, &[(0x927C, not42)]))).is_none());
    // R-CR3-23: a field type outside 1 to 12 is ignored.
    let mut t13 = good.clone();
    t13[12..14].copy_from_slice(&13u16.to_le_bytes());
    assert!(color_of(&with_ctmd(ctmd_record(8, &[(0x927C, t13)]))).is_none());

    // R-CR3-30 and R-CR3-35: the S6 conversion of Y 120, Cb 100, Cr 160
    // is R 164.864, G 106.78344, B 70.384; no matrix, no linearization.
    // With the sampling factors of M7 (2 x 1, 1 x 1, 1 x 1) and with 1 x 1.
    for samp in [
        [(2, 1), (1, 1), (1, 1)],
        [(1, 1); 3],
        [(2, 2), (1, 1), (1, 1)],
    ] {
        let (w, h) = (u16::from(samp[0].0) * 8, u16::from(samp[0].1) * 8);
        let f = one_track(1, w, h, ycc_jpeg(w, h, [120, 100, 160], samp));
        let dev = report(&f);
        assert_flat(&dev.image, (w.into(), h.into()), [165, 107, 70], 1, "S6");
    }
    // R-CR3-35 and R-CR3-41: the ColorSpace value is reported with the
    // image, 1 without CMT2, the stored value otherwise (OQ-11).
    for (cs, want) in [(None, 1u16), (Some(1u16), 1), (Some(0xFFFF), 0xFFFF)] {
        let mut s = f1_spec(1);
        if let Some(v) = cs {
            s.canon
                .push(bx(b"CMT2", &tiff(true, &[(0xA001, Val::Short(vec![v]))])));
        }
        let dev = report(&s.build());
        assert_eq!(dev.color_space, want);
        assert_eq!(dev.source, P::P1);
    }
}

/// The TV-11 values for version 33.
fn set_tv11() -> Vec<(usize, i16)> {
    vec![(85, 1856), (86, 1024), (87, 1024), (88, 1670), (89, 4965)]
}

// ---------------------------------------------------------------------------
// Out-of-scope variants: the preview path
// ---------------------------------------------------------------------------

#[test]
fn cr3_out_of_scope_falls_back_to_preview() {
    let check = |f: &[u8], variant: Variant, dual: bool, what: &str| {
        let dev = report(f);
        assert_eq!(dev.variant, variant, "{what}");
        assert_eq!(dev.dual_pixel, dual, "{what}");
        assert_eq!((dev.source, dev.stored), (P::P1, (48, 32)), "{what}");
        assert_flat(&dev.image, (48, 32), [200; 3], 0, what);
    };
    // X1 and X2 by CMP1 byte 26 of the main CRX track (R-CR3-2, M13).
    let x1 = full_spec(0, false, crx_sample()).build();
    check(&x1, Variant::X1, false, "X1");
    let x2 = full_spec(3, false, crx_sample()).build();
    check(&x2, Variant::X2, false, "X2");
    // X3: a second CRX track of the main size (track 5).
    check(
        &full_spec(0, true, crx_sample()).build(),
        Variant::X1,
        true,
        "X3 RAW",
    );
    check(
        &full_spec(3, true, crx_sample()).build(),
        Variant::X2,
        true,
        "X3 C-RAW",
    );
    // X6: the smaller CRX image of track 2 is not the main one; the main
    // track is the larger one wherever it stands, the first on a tie
    // (R-CR3-16).
    let c = container(&x1);
    let main = c.main_crx().unwrap();
    assert_eq!(main.cmp1_size(&x1), Some((64, 40)));
    let mut s = full_spec(0, false, crx_sample());
    s.tracks.swap(1, 2);
    s.tracks[2].entry = crx_entry(64, 40, 3);
    let f = s.build();
    let c = container(&f);
    assert_eq!(
        c.main_crx().unwrap().cmp1_mode(&f),
        Some(0),
        "first on a tie"
    );
    assert!(c.dual_pixel(), "same size as the main track");
    // X4: an image track with more than one sample.
    let mut s = full_spec(0, false, crx_sample());
    s.tracks[0].count = 2;
    check(&s.build(), Variant::X4, false, "X4");
    let mut s = full_spec(0, false, crx_sample());
    s.tracks[2].count = 3;
    check(&s.build(), Variant::X4, false, "X4 CRX");
    // Unknown: no CRX track (F1), or a CMP1 without byte 26.
    assert_eq!(report(&f1()).variant, Variant::Unknown);
    let mut s = full_spec(0, false, crx_sample());
    for k in [1, 2] {
        s.tracks[k].entry = craw_entry(0, 0, CRX_FIELD, &bx(b"CMP1", &[0xFF, 0, 0, 0]));
    }
    check(&s.build(), Variant::Unknown, false, "short CMP1");

    // R-CR3-1 and R-CR3-28: the CRX samples are never read. They hold a
    // fake 9000 x 6000 JPEG that a byte scan (the generic path) takes as
    // the largest preview; the CR3 path delivers P1.
    let generic = crate::raw::previews(&x1);
    assert!(generic.iter().any(|p| (p.width, p.height) == (9000, 6000)));
    assert!(candidates(&x1).iter().all(|c| c.1 <= 48));
    // CRX samples outside the file change nothing.
    let mut s = full_spec(0, true, vec![]);
    for k in [1, 2, 4] {
        s.tracks[k].sample = Sample::At(u64::MAX - 10, 1 << 30);
    }
    check(&s.build(), Variant::X1, true, "CRX outside the file");
    // Tracks are classified by content: the JPEG track last.
    let mut s = full_spec(0, false, crx_sample());
    let jpeg = s.tracks.remove(0);
    s.tracks.push(jpeg);
    check(&s.build(), Variant::X1, false, "JPEG track last");

    // X5 (M15): HEVC track, THMB and PRVW of version 1: no JPEG candidate,
    // the error "no supported preview" naming the variant (R-CR3-37).
    let x5 = x5_file();
    let c = container(&x5);
    assert_eq!(c.variant(), Variant::X5);
    assert!(preview::collect(&c).is_empty());
    let e = error_of(&x5);
    assert!(e.contains(NO_PREVIEW) && e.contains("X5"), "{e}");
    assert!(probe(&x5).is_err());
    assert!(!has_preview(&x5));
    assert!(facts(&x5).is_some());
    // Rule 1 also for a PRVW of version 1 alone (F1 with its PRVW version
    // byte set): X5, and P1 is still delivered.
    let mut f = f1();
    f[0x260] = 1;
    let dev = report(&f);
    assert_eq!((dev.variant, dev.source), (Variant::X5, P::P1));

    // R-CR3-40: the next-largest candidate when the largest fails; OQ-16:
    // the thumbnail only when no larger candidate decodes.
    let mut broken = gray_jpeg(48, 32, 200);
    let sos = broken.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    broken.truncate(sos + 4);
    let mut s = full_spec(0, false, crx_sample());
    s.tracks[0] = TrackSpec::jpeg(48, 32, broken.clone());
    let dev = report(&s.build());
    assert_eq!(dev.source, P::P2);
    assert_flat(&dev.image, (16, 8), [96; 3], 0, "next-largest");
    let mut s = f1_spec(1);
    s.tracks[0] = TrackSpec::jpeg(32, 16, broken.clone());
    s.holder = Some(vec![prvw(16, 8, &broken)]);
    let dev = report(&s.build());
    assert_eq!(dev.source, P::P3);
    assert_flat(&dev.image, (8, 8), [128; 3], 0, "thumbnail last");
    // An error only when no candidate decodes (R-CR3-40, R-CR3-54).
    s.canon[2] = thmb(8, 8, &broken);
    let f = s.build();
    assert_eq!(candidates(&f).len(), 3);
    assert!(decode_report(&f, &DecodeOptions::default()).is_err());
    // R-CR3-51: a stream without SOF0, SOF1 or SOF2 before SOS (here its
    // only frame header is SOF3, lossless) is no candidate.
    let mut sof3 = gray_jpeg(48, 32, 200);
    let at = sof3.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    sof3[at + 1] = 0xC3;
    let mut s = full_spec(0, false, crx_sample());
    s.tracks[0] = TrackSpec::jpeg(48, 32, sof3);
    assert_eq!(report(&s.build()).source, P::P2);
    // R-CR3-42: bytes after EOI, and a stream that ends without EOI.
    for (what, stream) in [
        (
            "bytes after EOI",
            cat(&[&gray_jpeg(48, 32, 200), &[0xFF; 6]]),
        ),
        ("no EOI", {
            let j = gray_jpeg(48, 32, 200);
            j[..j.len() - 2].to_vec()
        }),
    ] {
        let mut s = full_spec(0, false, crx_sample());
        s.tracks[0] = TrackSpec::jpeg(48, 32, stream);
        check(&s.build(), Variant::X1, false, what);
    }
    // R-CR3-38: values 5 to 8 do not rotate a preview taller than wide;
    // other values apply.
    let tall = one_track(6, 16, 32, gray_jpeg(16, 32, 200));
    assert_flat(&report(&tall).image, (16, 32), [200; 3], 0, "tall, 6");
    let tall = one_track(3, 16, 32, gray_jpeg(16, 32, 200));
    assert_flat(&report(&tall).image, (16, 32), [200; 3], 0, "tall, 3");
    // R-CR3-38 and R-CR3-52: Orientation absent, 0 or 9 counts as 1.
    for o in [0u16, 9] {
        let dev = report(&one_track(o, 32, 16, gray_jpeg(32, 16, 200)));
        assert_eq!((dev.meta.orientation, dev.image.width()), (1, 32));
    }
    let no_cmt1 = Cr3 {
        canon: vec![cncv()],
        tracks: vec![TrackSpec::jpeg(32, 16, gray_jpeg(32, 16, 200))],
        holder: None,
    }
    .build();
    assert_eq!(report(&no_cmt1).meta.orientation, 1);

    // R-CR3-50: no "moov" and no PRVW fails in the specification's own
    // procedure; the provider then searches the file like before this
    // module (a test of tests/extended.rs relies on that).
    let mut bare = FTYP.to_vec();
    bare.extend([0u8; 40]);
    bare.extend(gray_jpeg(16, 16, 200));
    bare.extend([0u8; 16]);
    assert!(decode_report(&bare, &DecodeOptions::default()).is_err());
    assert!(decode(&bare, &DecodeOptions::default()).unwrap().is_none());
    // Without "moov" but with a PRVW: P2.
    let holder_only = cat(&[
        &FTYP,
        &uuid(
            &boxes::PREVIEW_UUID,
            &cat(&[
                &[0, 0, 0, 0, 0, 0, 0, 1],
                &prvw(16, 8, &gray_jpeg(16, 8, 96)),
            ]),
        ),
    ]);
    let dev = report(&holder_only);
    assert_eq!(dev.source, P::P2);
    assert_eq!(dev.variant, Variant::Unknown);

    // Through the registry: by extension and by signature; a file the CR3
    // path declines keeps the generic preview path.
    let reg = crate::default_registry();
    for name in [Some(PathBuf::from("x.CR3")), None] {
        let src = pandite_core::Source::from_bytes(x2.clone(), name.clone());
        let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
        assert_flat(img.first_image(), (48, 32), [200; 3], 0, "registry");
        let meta = reg.probe(&src).unwrap();
        assert_eq!((meta.width, meta.height), (48, 32), "{name:?}");
    }
    let src = pandite_core::Source::from_bytes(bare, Some("e.cr3".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_eq!(img.first_image().width(), 16, "generic path");
    let not_cr3 = gray_jpeg(16, 8, 96);
    assert!(decode_report(&not_cr3, &DecodeOptions::default())
        .unwrap()
        .is_none());
    let src = pandite_core::Source::from_bytes(not_cr3, Some("renamed.cr3".into()));
    let img = reg.decode(&src, &DecodeOptions::default()).unwrap();
    assert_flat(img.first_image(), (16, 8), [96; 3], 0, "renamed JPEG");
    let src = pandite_core::Source::from_bytes(x5, Some("hevc.cr3".into()));
    assert!(reg.decode(&src, &DecodeOptions::default()).is_err());
}

// ---------------------------------------------------------------------------
// Hostile input, truncation, cancellation, thumbnails
// ---------------------------------------------------------------------------

/// Fixtures and synthetic files for the robustness tests.
fn robustness_samples() -> Vec<(&'static str, Vec<u8>)> {
    vec![
        ("F1", f1()),
        ("F2", f2()),
        ("F3", f3()),
        ("X1", full_spec(0, false, crx_sample()).build()),
        ("X3", full_spec(3, true, crx_sample()).build()),
        ("X5", x5_file()),
        ("TV-10", with_ctmd(tv10_sample())),
    ]
}

#[test]
fn cr3_hostile_input_never_panics() {
    let rounds: usize = std::env::var("PANDITE_FUZZ_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut next = noise(0x2545_F491);
    let reg = crate::default_registry();
    for (name, f) in robustness_samples() {
        // The TV-10 sample is large: fewer rounds over its tail.
        let rounds = if f.len() > 50_000 { rounds / 4 } else { rounds };
        for round in 0..rounds {
            let mut bad = f.clone();
            match round % 4 {
                // Damage box headers, sample tables and the Canon boxes.
                0 | 1 => {
                    let limit = bad.len().min(if round % 2 == 0 { 400 } else { 3000 });
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
                // Lying sizes, counts and offsets (big-endian fields).
                _ => {
                    let i = next() as usize % bad.len().saturating_sub(4).max(1);
                    let v =
                        [u32::MAX, 0x7FFF_FFFF, 0, 1, 8, 16, 65535, 1 << 28][next() as usize % 8];
                    if i + 4 <= bad.len() {
                        bad[i..i + 4].copy_from_slice(&v.to_be_bytes());
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
                            assert!(dev.image.width() <= 65535 && dev.image.height() <= 65535)
                        }
                        Ok(None) => {}
                        Err(e) => {
                            let msg = e.to_string();
                            assert!(!msg.contains("panicked"), "{name} round {round}: {msg}");
                        }
                    }
                }
                let src = pandite_core::Source::from_bytes(bad.clone(), Some("x.cr3".into()));
                let _ = reg.probe(&src);
                let _ = reg.facts(&src);
                let _ = reg.decode(&src, &DecodeOptions::default());
            }));
            assert!(r.is_ok(), "{name} mutation round {round} panicked");
        }
    }

    // R-CR3-44: at most 4096 boxes per container and 16 levels.
    let mut many: Vec<u8> = std::iter::repeat_n(bx(b"free", &[]), 4096)
        .flatten()
        .collect();
    many.extend(bx(b"trak", &[]));
    let w = boxes::walk(&many, 0, many.len() as u64, 2);
    assert_eq!(w.len(), boxes::MAX_BOXES);
    assert!(w.iter().all(|b| b.is(b"free")));
    assert_eq!(boxes::walk(&many, 0, many.len() as u64, 16).len(), 4096);
    assert!(boxes::walk(&many, 0, many.len() as u64, 17).is_empty());
    // R-CR3-44: at most 64 "trak" boxes; a JPEG track as the 65th is not
    // read.
    let jpeg = TrackSpec::jpeg(32, 16, gray_jpeg(32, 16, 200));
    let build = |n: usize| {
        let filler: Vec<u8> = std::iter::repeat_n(bx(b"trak", &[]), n).flatten().collect();
        let canon = uuid(&boxes::CANON_UUID, &cat(&[&cncv(), &cmt1(1)]));
        let moov_len = 8 + canon.len() + filler.len() + jpeg.bytes(0).len();
        let offset = (FTYP.len() + moov_len + 16) as u64;
        let moov = bx(b"moov", &cat(&[&canon, &filler, &jpeg.bytes(offset)]));
        let Sample::Data(s) = &jpeg.sample else {
            unreachable!()
        };
        cat(&[&FTYP, &moov, &bx64(b"mdat", s)])
    };
    let f = build(63);
    assert_eq!(container(&f).tracks.len(), 64);
    assert_eq!(report(&f).source, P::P1);
    let f = build(64);
    assert_eq!(container(&f).tracks.len(), boxes::MAX_TRAKS);
    assert!(candidates(&f).is_empty());
    assert!(error_of(&f).contains(NO_PREVIEW));
    // R-CR3-46: a frame of more than 2^28 pixels is refused before any
    // allocation; 2^28 exactly is a candidate.
    let mut huge = gray_jpeg(32, 16, 200);
    let at = huge.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    huge[at + 5..at + 9].copy_from_slice(&[0x40, 0x01, 0x40, 0x00]);
    let f = one_track(1, 32, 16, huge.clone());
    assert!(candidates(&f).is_empty(), "16385 x 16384");
    huge[at + 5..at + 9].copy_from_slice(&[0x40, 0x00, 0x40, 0x00]);
    assert_eq!(candidates(&one_track(1, 32, 16, huge.clone())).len(), 1);
    huge[at + 5..at + 9].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    assert!(candidates(&one_track(1, 32, 16, huge.clone())).is_empty());
    huge[at + 5..at + 9].copy_from_slice(&[0x00, 0x00, 0x40, 0x00]);
    assert!(candidates(&one_track(1, 32, 16, huge)).is_empty(), "Y = 0");
    // R-CR3-46: a stream longer than 256 MiB is no candidate; 256 MiB is.
    let mut s = f1_spec(1);
    s.tracks[0].sample = Sample::At(0, 0);
    let head = s.build();
    let at = head.len() as u64;
    for (len, kept) in [
        (preview::MAX_STREAM + 1, false),
        (preview::MAX_STREAM, true),
    ] {
        s.tracks[0].sample = Sample::At(at, len as u32);
        let mut f = s.build();
        f.extend(gray_jpeg(32, 16, 200));
        f.resize((at + len) as usize, 0);
        let got = candidates(&f).iter().any(|c| c.0 == P::P1);
        assert_eq!(got, kept, "length {len}");
    }
    // R-CR3-47: a metadata sample over 16 MiB is ignored; 16 MiB is read.
    let mut s = f1_spec(1);
    s.tracks.push(TrackSpec {
        sample: Sample::At(0, 0),
        ..TrackSpec::ctmd(vec![])
    });
    let at = s.build().len() as u64;
    for (len, found) in [(meta::MAX_CTMD + 1, false), (meta::MAX_CTMD, true)] {
        s.tracks[1].sample = Sample::At(at, len as u32);
        let mut f = s.build();
        f.extend(f3_sample());
        f.resize((at + len) as usize, 0);
        assert_eq!(color_of(&f).is_some(), found, "sample of {len} bytes");
    }
    // R-CR3-47: a TIFF block with more than 4096 entries in IFD0.
    for (n, ok) in [(4096usize, true), (4097, false)] {
        let entries: Vec<(u16, Val)> = (0..n)
            .map(|i| (if i == 0 { 0x4001 } else { 0x5000 }, Val::Short(vec![1])))
            .collect();
        assert_eq!(
            meta::block(&tiff(true, &entries)).is_some(),
            ok,
            "{n} entries"
        );
    }
    // R-CR3-48: at most 65536 records and blocks.
    let rec8 = ctmd_record(8, &[(0x927C, color_block(33, 1000, 3, &set_tv11()))]);
    for (n, found) in [(65_535usize, true), (65_536, false)] {
        let mut s: Vec<u8> = std::iter::repeat_n(ctmd_record(1, &[]), n)
            .flatten()
            .collect();
        s.extend(&rec8);
        let recs = meta::records(&s);
        assert_eq!(recs.iter().any(|r| r.0 == 8), found, "{n} records");
        assert!(recs.len() <= meta::MAX_WALK);
        let blocks: Vec<(u32, Vec<u8>)> = (0..n).map(|_| (0x8769, vec![])).collect();
        let r = ctmd_record(8, &blocks);
        let mut payload = r[12..].to_vec();
        payload.extend(&rec8[12..]);
        let bl = meta::blocks(&payload);
        assert_eq!(bl.iter().any(|b| b.0 == 0x927C), found, "{n} blocks");
    }
    // R-CR3-25 and R-CR3-26: a record or block size below its header or
    // past the end stops the walk.
    let mut s = ctmd_record(1, &[]);
    s.extend(hex("0B 00 00 00 08 00 00 00 00 00 00 00"));
    s.extend(&rec8);
    assert_eq!(meta::records(&s).len(), 1);
    let mut s = ctmd_record(1, &[]);
    s.extend(&rec8[..rec8.len() - 1]);
    assert_eq!(meta::records(&s).len(), 1);
    let short = hex("07 00 00 00 7C 92 00 00 00 00 00 00");
    assert!(meta::blocks(&short).is_empty());
}

#[test]
fn cr3_truncated_is_error_or_preview() {
    for (name, f) in robustness_samples() {
        let step = (f.len() / 197).max(1);
        let mut cuts: Vec<usize> = (0..f.len()).step_by(step).collect();
        cuts.extend([1, 23, 24, 25, 31, 32, f.len() - 1, f.len() - 2]);
        for cut in cuts {
            let r = catch_unwind(AssertUnwindSafe(|| {
                decode_report(&f[..cut], &DecodeOptions::default())
            }));
            match r {
                Err(_) => panic!("{name} cut at {cut} panicked"),
                Ok(Ok(Some(dev))) => {
                    // A delivered preview lies inside the cut file.
                    let c = container(&f[..cut]);
                    let ok = preview::collect(&c)
                        .iter()
                        .any(|p| p.source == dev.source && p.stream.offset + p.stream.len <= cut);
                    assert!(ok, "{name} cut at {cut}");
                }
                Ok(Ok(None)) | Ok(Err(_)) => {}
            }
        }
    }
    // F1 cut inside P1 (the end of the file): P2 is delivered; cut inside
    // "moov": the file fails (R-CR3-6, R-CR3-50).
    let f = f1();
    for cut in [0x30F + 10, 0x3A0 - 1] {
        assert_eq!(report(&f[..cut]).source, P::P2, "cut at {cut}");
    }
    for cut in [0x30, 0x150, 0x237] {
        assert!(decode_report(&f[..cut], &DecodeOptions::default()).is_err());
    }
}

/// A 1024 x 1024 noise JPEG that both shared JPEG decoders reject late:
/// 64 one-bits (stuffed FF 00 pairs) are inserted at 90 % of the
/// entropy-coded data, so the decoders fail on a code not in the table
/// after decoding most of the image.
fn late_failing_jpeg() -> Vec<u8> {
    let mut next = noise(7);
    let px: Vec<u8> = (0..1024 * 1024 * 3).map(|_| next() as u8).collect();
    let mut out = Vec::new();
    jpeg_encoder::Encoder::new(&mut out, 90)
        .encode(&px, 1024, 1024, jpeg_encoder::ColorType::Rgb)
        .expect("encode");
    let sos = out.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    let start = sos + 2 + usize::from(u16::from_be_bytes([out[sos + 2], out[sos + 3]]));
    let mut at = start + (out.len() - 2 - start) * 9 / 10;
    while out[at - 1] == 0xFF {
        at += 1;
    }
    let ones: Vec<u8> = std::iter::repeat_n([0xFF, 0x00], 8).flatten().collect();
    out.splice(at..at, ones);
    out
}

#[test]
fn cr3_honors_cancel() {
    // A cancelled token stops before any work, through the provider too.
    let cancelled = DecodeOptions::default();
    cancelled.cancel.cancel();
    assert!(matches!(
        decode_report(&f1(), &cancelled),
        Err(Error::Cancelled)
    ));
    let src = pandite_core::Source::from_bytes(f1(), Some("a.cr3".into()));
    let provider = crate::RawDecoder;
    assert!(matches!(
        pandite_core::DecoderProvider::decode(&provider, &src, &cancelled),
        Err(Error::Cancelled)
    ));
    // Six full-size JPEG tracks that each fail late, then a PRVW: the full
    // decode tries every candidate (R-CR3-40) and delivers P2.
    let bad = late_failing_jpeg();
    let f = Cr3 {
        canon: vec![cncv(), cmt1(1)],
        tracks: (0..6)
            .map(|_| TrackSpec::jpeg(1024, 1024, bad.clone()))
            .collect(),
        holder: Some(vec![prvw(16, 8, &gray_jpeg(16, 8, 96))]),
    }
    .build();
    let start = Instant::now();
    let dev = report(&f);
    let full = start.elapsed();
    assert_eq!(dev.source, P::P2);
    // Cancelled from another thread, the decode stops part way.
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
fn cr3_target_size_uses_preview() {
    let f = full_spec(0, false, crx_sample()).build();
    let opts = |t: (u32, u32)| DecodeOptions {
        target_size: Some(t),
        ..DecodeOptions::default()
    };
    // The smallest candidate that covers the target, never the thumbnail
    // (OQ-16).
    let dev = decode_report(&f, &opts((12, 6))).unwrap().unwrap();
    assert_eq!(dev.source, P::P2);
    let dev = decode_report(&f, &opts((4, 4))).unwrap().unwrap();
    assert_eq!(dev.source, P::P2);
    let dev = decode_report(&f, &opts((40, 30))).unwrap().unwrap();
    assert_eq!(dev.source, P::P1);
    let dev = decode_report(&f, &opts((500, 500))).unwrap().unwrap();
    assert_eq!(dev.source, P::P1);
    // With orientation 6 the target is in display orientation.
    let mut s = full_spec(0, false, crx_sample());
    s.canon[1] = cmt1(6);
    let f6 = s.build();
    let dev = decode_report(&f6, &opts((8, 16))).unwrap().unwrap();
    assert_eq!(dev.source, P::P2);
    assert_eq!((dev.image.width(), dev.image.height()), (8, 16));
    // Through the registry: a thumbnail request returns 8-bit preview data.
    let reg = crate::default_registry();
    let src = pandite_core::Source::from_bytes(f, Some("t.cr3".into()));
    let img = reg.decode(&src, &opts((16, 16))).unwrap();
    assert_eq!(img.first_image().format(), PixelFormat::Rgba8);
    assert!(pandite_core::DecoderProvider::reduced_decode(
        &crate::RawDecoder,
        &src
    ));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (48, 32));
    let src = pandite_core::Source::from_bytes(f6, Some("t.cr3".into()));
    let meta = reg.probe(&src).unwrap();
    assert_eq!((meta.width, meta.height), (32, 48));
}

/// Header facts of the fixtures, through the module, the provider and the
/// registry. Tags only: every cut of the head answers without a panic.
#[test]
fn facts_cr3() {
    use pandite_core::DecoderProvider;
    let x1 = full_spec(0, false, crx_sample()).build();
    let f = facts(&x1).expect("CR3 facts");
    assert_eq!(f.container.as_deref(), Some("Canon CR3"));
    assert_eq!(f.compression.as_deref(), Some("Canon CRX lossless (RAW)"));
    assert_eq!(f.color_space.as_deref(), Some("sRGB"));
    assert_eq!(f.camera.make.as_deref(), Some("Canon"));
    assert_eq!(f.camera.model.as_deref(), Some("Canon EOS R6"));
    assert_eq!(
        (f.channels, f.channel_count, f.bits_per_channel),
        (None, None, None)
    );
    let compression = |d: &[u8]| facts(d).and_then(|f| f.compression);
    assert_eq!(
        compression(&full_spec(3, false, crx_sample()).build()).as_deref(),
        Some("Canon CRX lossy (C-RAW)")
    );
    assert_eq!(
        compression(&full_spec(0, true, crx_sample()).build()).as_deref(),
        Some("Canon CRX lossless (RAW), Dual Pixel RAW")
    );
    assert_eq!(
        compression(&x5_file()).as_deref(),
        Some("Canon CRX lossy (C-RAW)")
    );
    assert_eq!(compression(&f1()), None);
    let mut s = f1_spec(1);
    s.canon.push(bx(
        b"CMT2",
        &tiff(true, &[(0xA001, Val::Short(vec![0xFFFF]))]),
    ));
    assert_eq!(
        facts(&s.build()).unwrap().color_space.as_deref(),
        Some("Uncalibrated")
    );

    let reg = crate::default_registry();
    let via = reg
        .facts(&pandite_core::Source::from_bytes(
            x1.clone(),
            Some("IMG_0001.CR3".into()),
        ))
        .expect("registry facts");
    assert_eq!(via.compression.as_deref(), Some("Canon CRX lossless (RAW)"));
    let p = crate::raw::RawDecoder
        .facts(&pandite_core::Source::from_bytes(x1.clone(), None))
        .expect("provider facts");
    assert_eq!(p.camera.model.as_deref(), Some("Canon EOS R6"));
    for n in (0..x1.len().min(2000)).step_by(7) {
        let _ = facts(&x1[..n]);
    }
    assert!(facts(b"not a CR3 file at all, longer than 24").is_none());
}

// ---------------------------------------------------------------------------
// Real camera files
// ---------------------------------------------------------------------------

/// SOF0 to SOF2 size of a JPEG stream, written for this test apart from
/// the decoder from the syntax of T.81 (B.2): marker segments from SOI to
/// a frame header with P = 8, Nf of 1 or 3, Lf = 8 + 3 Nf, sampling
/// factors 1 to 4 and Tq 0 to 3, then segments up to an SOS with
/// Ls = 6 + 2 Ns and 1 <= Ns <= Nf. Bytes FF D8 FF inside other data
/// (the CRX samples hold some) fail this syntax.
fn scan_frame(s: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2usize;
    let mut frame: Option<(u32, u32, u8)> = None;
    for _ in 0..100 {
        if *s.get(pos)? != 0xFF {
            return None;
        }
        let m = *s.get(pos + 1)?;
        if m == 0xFF {
            pos += 1;
            continue;
        }
        if matches!(m, 0x00 | 0x01 | 0xD0..=0xD9) {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([*s.get(pos + 2)?, *s.get(pos + 3)?]));
        if len < 2 {
            return None;
        }
        let seg = s.get(pos + 4..pos + 2 + len)?;
        match m {
            0xC0..=0xC2 if frame.is_none() => {
                let (p, nf) = (*seg.first()?, *seg.get(5)?);
                if p != 8 || !matches!(nf, 1 | 3) || len != 8 + 3 * usize::from(nf) {
                    return None;
                }
                let y = u16::from_be_bytes([seg[1], seg[2]]);
                let x = u16::from_be_bytes([seg[3], seg[4]]);
                let comps_ok = seg[6..].chunks_exact(3).all(|c| {
                    (1..=4).contains(&(c[1] >> 4)) && (1..=4).contains(&(c[1] & 15)) && c[2] <= 3
                });
                if x == 0 || y == 0 || !comps_ok {
                    return None;
                }
                frame = Some((u32::from(x), u32::from(y), nf));
            }
            0xDA => {
                let (x, y, nf) = frame?;
                let ns = *seg.first()?;
                let ok = (1..=nf).contains(&ns) && len == 6 + 2 * usize::from(ns);
                return ok.then_some((x, y));
            }
            0xC3..=0xCF if m != 0xC4 && m != 0xC8 && m != 0xCC => return None,
            _ => {}
        }
        pos += 2 + len;
    }
    None
}

/// Every FF D8 FF in the file with a frame header: (offset, X, Y).
fn scan_jpegs(d: &[u8]) -> Vec<(usize, u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 <= d.len() {
        if d[i] == 0xFF && d[i + 1] == 0xD8 && d[i + 2] == 0xFF {
            if let Some((x, y)) = scan_frame(&d[i..]) {
                out.push((i, x, y));
            }
        }
        i += 1;
    }
    out
}

/// The variant and Dual Pixel flag SOURCES.txt states for each file.
fn source_variants(folder: &Path) -> Vec<(String, Variant, bool)> {
    let Ok(text) = std::fs::read_to_string(folder.join("SOURCES.txt")) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| {
            let parts: Vec<&str> = l.split('|').map(str::trim).collect();
            let v = *parts.get(3)?;
            let variant = if v.starts_with("X5") {
                Variant::X5
            } else if v.starts_with("X1") {
                Variant::X1
            } else if v.starts_with("X2") {
                Variant::X2
            } else {
                return None;
            };
            Some((parts[0].to_string(), variant, v.contains("X3")))
        })
        .collect()
}

/// TV-1 to TV-13 at their offsets in the camera file S14.
fn check_s14(data: &[u8], c: &Container<'_>) {
    assert_eq!(data.len(), 9_815_638);
    assert_eq!(&data[..24], &FTYP[..], "TV-1");
    assert_eq!(
        c.moov.map(|b| (b.start, b.size())),
        Some((24, 36_368)),
        "TV-2"
    );
    let mdat = c.top.last().unwrap();
    assert_eq!(
        (mdat.typ, mdat.start, mdat.body, mdat.end),
        (*b"mdat", 422_896, 422_912, 9_815_638)
    );
    assert_eq!(
        c.canon.map(|b| (b.start, b.size())),
        Some((32, 34_096)),
        "TV-3"
    );
    let cncv = c.canon_children[0];
    assert_eq!(
        (cncv.start, cncv.payload(data)),
        (56, &b"CanonCR3_001/00.10.00/00.00.00"[..])
    );
    assert_eq!(&data[34_376 + 16..34_376 + 20], b"vide", "TV-4");
    let t = &c.tracks[0];
    assert_eq!((t.role, t.entry), (Role::Jpeg, Some((34_497, 112))), "TV-5");
    assert_eq!(t.entry_children[0].start, 34_587);
    assert_eq!(t.sample, Some((422_912, 733_764)), "TV-6");
    let thmb = c.canon_box(b"THMB").unwrap();
    assert_eq!((thmb.start, thmb.size()), (18_264, 15_864), "TV-7");
    assert_eq!(
        preview::boxed_stream(data, &thmb, 8),
        Some((18_288, 15_839))
    );
    let h = c.holder.unwrap();
    assert_eq!((h.start, h.size()), (101_952, 320_856), "TV-8");
    assert_eq!((c.prvw[0].start, c.prvw[0].size()), (101_984, 320_824));
    assert_eq!(
        preview::boxed_stream(data, &c.prvw[0], 12),
        Some((102_008, 320_799))
    );
    assert_eq!(&data[422_912 + 136..422_912 + 155], &hex(TV9)[..], "TV-9");
    assert_eq!(
        candidates(data)
            .iter()
            .map(|c| (c.0, c.1, c.2))
            .collect::<Vec<_>>(),
        vec![(P::P1, 3408, 2272), (P::P2, 1620, 1080), (P::P3, 160, 120)]
    );
    let ctmd = c.tracks.iter().find(|t| t.role == Role::Metadata).unwrap();
    assert_eq!(ctmd.sample, Some((9_696_768, 118_870)), "TV-10");
    let s = meta::ctmd_sample(c).unwrap();
    let recs = meta::records(s);
    assert_eq!((recs[0].0, recs[0].1.len()), (1, 12));
    let r8 = recs.iter().find(|r| r.0 == 8).unwrap();
    let r8_at = r8.1.as_ptr() as usize - data.as_ptr() as usize - 12;
    assert_eq!((r8_at, r8.1.len() + 12), (9_706_254, 106_518));
    let cd = meta::find_color_data(c).unwrap();
    assert_eq!((cd.version, cd.count), (33, 3656), "TV-11");
    assert_eq!(cd.wb_levels, Some([1856, 1024, 1024, 1670]));
    assert_eq!(cd.color_temp, Some(4965));
    assert_eq!(cd.black, Some([512; 4]));
    assert_eq!(
        (cd.normal_white, cd.specular_white),
        (Some(12_735), Some(14_008))
    );
    assert_eq!(cd.multipliers, Some([1.8125, 1.0, 1.630_859_375]));
    assert_eq!(c.canon_box(b"CMT3").map(|b| b.body), Some(2384), "TV-12");
    let si = meta::sensor_info(c).unwrap();
    assert_eq!((si.size, si.crop), ((3584, 2386), (156, 108, 3563, 2379)));
    let main = c.main_crx().unwrap();
    assert_eq!(main.cmp1.map(|b| b.start), Some(35_687), "TV-13");
    assert_eq!(main.cmp1_size(data), Some((3584, 2386)));
    assert_eq!(c.variant(), Variant::X1);
}

/// Every `.CR3` file in the folder named by `PANDITE_CR3_SAMPLES`
/// (absolute, or relative to the workspace root): the image returned has
/// the size of the largest JPEG stream an independent scan of the whole
/// file finds, oriented by R-CR3-38 and R-CR3-39, and its pixels agree
/// with that stream decoded on its own (the method and tolerances of
/// `arw_real_samples_match_embedded_preview`: 8 x 8 grid of cell means,
/// rg chromaticity distance and rank correlation). Files with HEVC
/// previews only (X5) return the error of R-CR3-37. Each line also shows
/// what the generic preview path returned before this module.
#[test]
fn cr3_real_samples_match_embedded_preview() {
    let Ok(dir) = std::env::var("PANDITE_CR3_SAMPLES") else {
        println!("skip: PANDITE_CR3_SAMPLES is not set");
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
                .is_some_and(|e| e.eq_ignore_ascii_case("cr3"))
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "no .CR3 files in {}", folder.display());
    let listed = source_variants(&folder);
    let mut failures = Vec::new();
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = std::fs::read(path).unwrap();
        let c = container(&data);
        let m = meta::read(&c);
        let variant = c.variant();
        let dual = c.dual_pixel();
        if name == "Canon_EOS_R6_RAW_ISO_100_crop_nodual.CR3" {
            check_s14(&data, &c);
        }
        if let Some((_, v, d)) = listed.iter().find(|l| l.0 == name) {
            assert_eq!((variant, dual), (*v, *d), "{name}: SOURCES.txt");
        }
        let cd = m.color.clone();
        let levels = match &cd {
            Some(cd) => format!(
                "ColorData v{} WB {:?} mult {:?} K {:?} black {:?} white {:?}/{:?}",
                cd.version,
                cd.wb_levels,
                cd.multipliers.map(|m| m.map(|v| (v * 1e4).round() / 1e4)),
                cd.color_temp,
                cd.black,
                cd.normal_white,
                cd.specular_white
            ),
            None => "ColorData none".to_string(),
        };
        // M11: every measured file has ColorData of a known version.
        if cd.as_ref().is_none_or(|cd| cd.wb_levels.is_none()) {
            failures.push(format!("{name}: no ColorData"));
        }
        // The independent scan and the generic path of before.
        let scan = scan_jpegs(&data);
        let largest = scan
            .iter()
            .max_by_key(|s| u64::from(s.1) * u64::from(s.2))
            .copied();
        let today = crate::raw::decode(&data, &DecodeOptions::default());
        let today_size = today.as_ref().ok().map(|i| (i.width(), i.height()));
        let today_o = crate::raw::container_orientation(&data);
        drop(today);
        // The size the generic probe reported: its largest byte-scan hit.
        let before_probe = crate::raw::previews(&data)
            .into_iter()
            .max_by_key(|p| p.area())
            .map_or("none".to_string(), |p| format!("{}x{}", p.width, p.height));
        let start = Instant::now();
        let result = decode_report(&data, &DecodeOptions::default());
        let took = start.elapsed();
        let before = today_size.map_or("error".to_string(), |s| format!("{}x{}", s.0, s.1));
        let today_o = (1..=8u16)
            .find(|&v| pandite_core::Orientation::from_exif(v) == today_o)
            .unwrap_or(1);
        let dev = match result {
            Ok(Some(dev)) => dev,
            Ok(None) => panic!("{name}: not a CR3 file"),
            Err(e) => {
                // X5: no JPEG anywhere in the file, the error of R-CR3-37.
                let e = e.to_string();
                assert_eq!(variant, Variant::X5, "{name}: {e}");
                assert!(e.contains(NO_PREVIEW), "{name}: {e}");
                assert_eq!(largest, None, "{name}: a JPEG stream exists");
                let changed = if today_size.is_none() {
                    "unchanged: error before too"
                } else {
                    "CHANGED"
                };
                println!(
                    "{name}: model {:?} {variant:?} dual {dual} orientation {} | error \"{e}\" | JPEG streams in the file: {} | before: decode {before}, probe {before_probe} ({changed}) | {levels}",
                    m.model,
                    m.orientation,
                    scan.len(),
                );
                continue;
            }
        };
        let (sx, sy, soff) = largest.map(|l| (l.1, l.2, l.0)).expect("a JPEG stream");
        // The chosen candidate is the largest stream of the file.
        assert_eq!(dev.stored, (sx, sy), "{name}: largest JPEG");
        let chosen = preview::collect(&c)[0];
        assert_eq!(chosen.stream.offset, soff, "{name}: stream offset");
        assert_eq!(dev.source, P::P1, "{name}: M8");
        // R-CR3-38 and R-CR3-39: the delivered size.
        let o = m.orientation;
        let want = if o >= 5 && sy <= sx {
            (sy, sx)
        } else {
            (sx, sy)
        };
        let got = (dev.image.width(), dev.image.height());
        assert_eq!(got, want, "{name}: orientation {o}");
        // M8 and M12: the P1 size equals the SensorInfo crop.
        if let Some(si) = m.sensor {
            assert_eq!(si.crop_size(), (sx, sy), "{name}: SensorInfo crop");
        }
        // The delivered pixels against the stream decoded on its own, and
        // against the PRVW preview (M7: the same camera rendering).
        let opts = DecodeOptions {
            target_size: Some((600, 600)),
            ..DecodeOptions::default()
        };
        let own = crate::jpeg::decode_image(&data[soff..], &opts).expect("stream decodes");
        let own = if o >= 5 && sy > sx {
            own
        } else {
            crate::orient::apply(own, pandite_core::Orientation::from_exif(u16::from(o))).unwrap()
        };
        let ours = grid_means(&dev.image, 8, 4);
        let (mean, max, rank, used) = agreement(&ours, &grid_means(&own, 8, 1), false);
        let mut line = format!(
            "{name}: model {:?} {variant:?} dual {dual} orientation {o} | {:?} {sx}x{sy} at {soff} of {} streams -> {}x{} in {:.2}s (debug) | vs stream: chroma mean {mean:.4} max {max:.4} rank {rank:.3} cells {used}",
            m.model,
            dev.source,
            scan.len(),
            got.0,
            got.1,
            took.as_secs_f64(),
        );
        if mean > CHROMA_MEAN || max > CHROMA_MAX || rank < RANK_MIN || used < 16 {
            failures.push(line.clone());
        }
        if let Some(p2) = preview::collect(&c).iter().find(|p| p.source == P::P2) {
            let prev = preview::decode(&data, p2, o, &opts).expect("PRVW decodes");
            let (m2, x2, r2, u2) = agreement(&ours, &grid_means(&prev, 8, 1), false);
            line.push_str(&format!(
                " | vs PRVW {}x{}: chroma mean {m2:.4} max {x2:.4} rank {r2:.3} cells {u2}",
                p2.stream.width, p2.stream.height
            ));
            if m2 > CHROMA_MEAN || x2 > CHROMA_MAX || r2 < RANK_MIN {
                failures.push(format!("{name} vs PRVW"));
            }
        }
        let changed = match (today_size == Some(got), today_o == u16::from(o)) {
            (true, true) => "size and orientation unchanged",
            (false, true) => "SIZE CHANGED",
            (true, false) => "ORIENTATION CHANGED",
            (false, false) => "SIZE AND ORIENTATION CHANGED",
        };
        line.push_str(&format!(
            " | before: decode {before} orientation {today_o}, probe {before_probe} ({changed}) | {levels}"
        ));
        if let Some(si) = m.sensor {
            line.push_str(&format!(
                " | SensorInfo {}x{} crop {:?}",
                si.size.0, si.size.1, si.crop
            ));
        }
        // A 128 x 128 thumbnail request: the CR3 path leaves the THMB out
        // (OQ-16); the generic path took the smallest preview that covers
        // the target.
        let thumb = DecodeOptions {
            target_size: Some((128, 128)),
            ..DecodeOptions::default()
        };
        let t = decode_report(&data, &thumb).unwrap().unwrap();
        assert_ne!(t.source, P::P3, "{name}: thumbnail");
        let before_t = crate::raw::decode(&data, &thumb).map_or("error".to_string(), |i| {
            format!("{}x{}", i.width(), i.height())
        });
        line.push_str(&format!(
            " | thumbnail 128: {:?} {}x{}, before {before_t}",
            t.source,
            t.image.width(),
            t.image.height()
        ));
        println!("{line}");
    }
    assert!(failures.is_empty(), "outside tolerance: {failures:#?}");
}

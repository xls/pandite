//! Synthetic ORF files for the tests: the specification's fixtures F1 to
//! F7 (section Test vectors, Fixture recipes), a builder that follows the
//! fixture layout in either byte order and any maker-note style, with
//! optional previews, the inverse packing of V1 to V4, and a V5/V6 encoder
//! written from the rules of section Compression.

pub(super) use crate::raw::arw::fixtures::{hex, jpeg};
pub(super) use crate::raw::srw::fixtures::Val;
use crate::raw::srw::fixtures::{ifd_size, write_ifd};

use super::meta::Style;
use std::collections::HashMap;

/// Fixture F6 (V5, 4 x 4), the complete 452-byte file of the
/// specification.
pub(super) const F6_HEX: &str = "
0000: 49 49 52 4F 08 00 00 00 0D 00 00 01 04 00 01 00
0010: 00 00 04 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 10 00 00 00 03 01
0030: 03 00 01 00 00 00 01 00 00 00 06 01 03 00 01 00
0040: 00 00 01 00 00 00 0F 01 02 00 08 00 00 00 AA 00
0050: 00 00 10 01 02 00 08 00 00 00 B2 00 00 00 11 01
0060: 04 00 01 00 00 00 A0 01 00 00 12 01 03 00 01 00
0070: 00 00 01 00 00 00 15 01 03 00 01 00 00 00 01 00
0080: 00 00 16 01 04 00 01 00 00 00 04 00 00 00 17 01
0090: 04 00 01 00 00 00 24 00 00 00 69 87 04 00 01 00
00A0: 00 00 BA 00 00 00 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 02 00 7C 92 07 00
00C0: B2 00 00 00 D8 00 00 00 02 A3 07 00 08 00 00 00
00D0: 8A 01 00 00 00 00 00 00 4F 4C 59 4D 50 55 53 00
00E0: 49 49 03 00 01 00 40 20 0D 00 01 00 00 00 1E 00
00F0: 00 00 00 00 00 00 09 00 00 01 03 00 04 00 00 00
0100: 90 00 00 00 00 02 03 00 09 00 00 00 98 00 00 00
0110: 00 06 03 00 04 00 00 00 AA 00 00 00 10 06 03 00
0120: 01 00 00 00 00 01 00 00 11 06 03 00 02 00 00 00
0130: 0C 00 00 00 12 06 03 00 02 00 00 00 00 00 00 00
0140: 13 06 03 00 02 00 00 00 00 00 00 00 14 06 04 00
0150: 01 00 00 00 04 00 00 00 15 06 04 00 01 00 00 00
0160: 04 00 00 00 00 00 00 00 00 02 80 01 00 01 00 01
0170: B8 01 68 FF E0 FF BA FF 84 01 C2 FF 0E 00 80 FF
0180: 72 01 00 01 00 01 00 01 00 01 02 00 02 00 00 01
0190: 01 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00
01A0: 00 00 00 00 01 00 00 00 00 04 E0 00 00 0E C2 4D
01B0: CA A0 00 00 76 00 00 00 86 25 54 97 10 10 10 10
01C0: 10 10 10 10
";

/// Fixture F2 (V1, big-endian, B = 10, OLYMP style), the complete
/// 352-byte file of the specification.
pub(super) const F2_HEX: &str = "
0000: 4D 4D 4F 52 00 00 00 08 00 0D 01 00 00 04 00 00
0010: 00 01 00 00 00 04 01 01 00 04 00 00 00 01 00 00
0020: 00 02 01 02 00 03 00 00 00 01 00 10 00 00 01 03
0030: 00 03 00 00 00 01 00 01 00 00 01 06 00 03 00 00
0040: 00 01 00 01 00 00 01 0F 00 02 00 00 00 08 00 00
0050: 00 AA 01 10 00 02 00 00 00 08 00 00 00 B2 01 11
0060: 00 04 00 00 00 01 00 00 01 50 01 12 00 03 00 00
0070: 00 01 00 01 00 00 01 15 00 03 00 00 00 01 00 01
0080: 00 00 01 16 00 04 00 00 00 01 00 00 00 02 01 17
0090: 00 04 00 00 00 01 00 00 00 10 87 69 00 04 00 00
00A0: 00 01 00 00 00 BA 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 00 02 92 7C 00 07
00C0: 00 00 00 64 00 00 00 D8 A3 02 00 07 00 00 00 08
00D0: 00 00 01 3C 00 00 00 00 4F 4C 59 4D 50 00 00 02
00E0: 00 05 10 11 00 03 00 00 00 09 00 00 01 22 10 12
00F0: 00 03 00 00 00 04 00 00 01 34 10 17 00 03 00 00
0100: 00 02 02 00 00 40 10 18 00 03 00 00 00 02 01 80
0110: 00 40 10 2C 00 03 00 00 00 02 00 0A 00 00 00 00
0120: 00 00 01 B8 FF 68 FF E0 FF BA 01 84 FF C2 00 0E
0130: FF 80 01 72 00 40 00 40 00 40 00 40 00 02 00 02
0140: 00 01 01 02 00 00 00 00 00 00 00 00 00 00 00 00
0150: 4C 00 74 00 4C 00 74 00 74 00 42 00 74 00 42 00
";

/// Fixture F4 (V3, two strips, OLYMP style, little-endian), the complete
/// 380-byte file of the specification.
pub(super) const F4_HEX: &str = "
0000: 49 49 52 53 08 00 00 00 0D 00 00 01 04 00 01 00
0010: 00 00 04 00 00 00 01 01 04 00 01 00 00 00 04 00
0020: 00 00 02 01 03 00 01 00 00 00 0C 00 00 00 03 01
0030: 03 00 01 00 00 00 01 00 00 00 06 01 03 00 01 00
0040: 00 00 02 00 00 00 0F 01 02 00 08 00 00 00 AA 00
0050: 00 00 10 01 02 00 08 00 00 00 B2 00 00 00 11 01
0060: 04 00 02 00 00 00 BA 00 00 00 12 01 03 00 01 00
0070: 00 00 01 00 00 00 15 01 03 00 01 00 00 00 01 00
0080: 00 00 16 01 04 00 01 00 00 00 02 00 00 00 17 01
0090: 04 00 02 00 00 00 C2 00 00 00 69 87 04 00 01 00
00A0: 00 00 CA 00 00 00 00 00 00 00 4F 4C 59 4D 50 55
00B0: 53 00 54 45 53 54 43 41 4D 00 60 01 00 00 70 01
00C0: 00 00 0C 00 00 00 0C 00 00 00 02 00 7C 92 07 00
00D0: 64 00 00 00 E8 00 00 00 02 A3 07 00 08 00 00 00
00E0: 4C 01 00 00 00 00 00 00 4F 4C 59 4D 50 00 02 00
00F0: 05 00 11 10 03 00 09 00 00 00 32 01 00 00 12 10
0100: 03 00 04 00 00 00 44 01 00 00 17 10 03 00 02 00
0110: 00 00 00 02 40 00 18 10 03 00 02 00 00 00 80 01
0120: 40 00 2C 10 03 00 02 00 00 00 0C 00 00 00 00 00
0130: 00 00 B8 01 68 FF E0 FF BA FF 84 01 C2 FF 0E 00
0140: 80 FF 72 01 00 01 00 01 00 01 00 01 02 00 02 00
0150: 00 01 01 02 00 00 00 00 00 00 00 00 00 00 00 00
0160: 4C 07 40 4C 07 40 4C 07 40 4C 07 40 00 00 00 00
0170: 74 04 20 74 04 20 74 04 20 74 04 20
";

/// F6 from its hex dump.
pub(super) fn f6() -> Vec<u8> {
    hex(F6_HEX)
}

/// F6 with the listed bytes before 0x1A0 changed and another strip at
/// 0x1A0 (the table of the fixture recipes).
fn from_f6(changes: &[(usize, u8)], strip: &str) -> Vec<u8> {
    let mut f = f6();
    f.truncate(0x1A0);
    for &(at, v) in changes {
        f[at] = v;
    }
    f.extend(hex(strip));
    f
}

/// F1 (V1, 4 x 4, B = 12, little-endian).
pub(super) fn f1() -> Vec<u8> {
    from_f6(
        &[(0x96, 0x20)],
        "00 4C 00 74 00 4C 00 74 00 74 00 42 00 74 00 42
         00 4C 00 74 00 4C 00 74 00 74 00 42 00 74 00 42",
    )
}

/// F2 from its hex dump.
pub(super) fn f2() -> Vec<u8> {
    hex(F2_HEX)
}

/// F3 (V2, 10 x 2).
pub(super) fn f3() -> Vec<u8> {
    from_f6(
        &[
            (0x12, 0x0A),
            (0x1E, 0x02),
            (0x8A, 0x02),
            (0x96, 0x20),
            (0x154, 0x0A),
            (0x160, 0x02),
        ],
        "C0 04 74 C0 04 74 C0 04 74 C0 04 74 C0 04 74 00
         40 07 42 40 07 42 40 07 42 40 07 42 40 07 42 00",
    )
}

/// F4 from its hex dump.
pub(super) fn f4() -> Vec<u8> {
    hex(F4_HEX)
}

/// F5 (V4, 8 x 2).
pub(super) fn f5() -> Vec<u8> {
    from_f6(
        &[
            (0x12, 0x08),
            (0x1E, 0x02),
            (0x2A, 0x0C),
            (0x8A, 0x02),
            (0x96, 0x18),
            (0x154, 0x08),
            (0x160, 0x02),
        ],
        "4C 40 07 4C 07 4C 40 07 40 07 4C 40 74 20 04 74 04 74 20 04 20 04 74 20",
    )
}

/// F7 (V6, 4 x 4, B = 14).
pub(super) fn f7() -> Vec<u8> {
    from_f6(
        &[(0x96, 0x26), (0x130, 0x0E)],
        "00 00 00 00 00 01 00 00 00 00 13 20 00 00 3A 42 4E 32 AD 00
         00 1D 20 00 00 21 42 55 B2 62 10 10 10 10 10 10 10 10",
    )
}

/// The maker note of a synthetic file.
#[derive(Clone, Debug)]
pub(super) struct NoteSpec {
    /// The header form.
    pub style: Style,
    /// Byte order of an OLYMPUS or OM SYSTEM note (OLYMP uses the file's).
    pub le: bool,
    /// Main maker-note IFD entries without the sub-IFD pointers.
    pub main: Vec<(u16, Val)>,
    /// The sub-IFDs by their main-IFD tag: type 13 pointers for OLYMPUS
    /// and OM SYSTEM, type 7 entries that hold the sub-IFD for OLYMP.
    pub subs: Vec<(u16, Vec<(u16, Val)>)>,
}

/// A synthetic ORF file in the layout of the fixtures: header, IFD0 with
/// its values, the Exif IFD with the maker note as its value, IFD1, then
/// the blobs at multiples of 16. `Val::Blobs` inside the maker note count
/// from the offset origin of its style.
#[derive(Clone, Debug, Default)]
pub(super) struct Builder {
    /// Little-endian file.
    pub le: bool,
    /// Header bytes 2 and 3 are 52 53 (little-endian only).
    pub rs: bool,
    /// IFD0 entries without the Exif pointer.
    pub ifd0: Vec<(u16, Val)>,
    /// Exif entries without the maker note; no Exif IFD when this is empty
    /// and there is no maker note.
    pub exif: Vec<(u16, Val)>,
    /// The maker note.
    pub note: Option<NoteSpec>,
    /// IFD1 entries; no IFD1 when empty.
    pub ifd1: Vec<(u16, Val)>,
    /// Data blobs: strips, previews.
    pub blobs: Vec<Vec<u8>>,
}

/// File positions of the out-of-line values of an IFD at `at`, by tag, as
/// `write_ifd` places them (sorted by tag, each padded to an even size).
fn value_positions(entries: &[(u16, Val)], at: usize, blobs: &[Vec<u8>]) -> HashMap<u16, usize> {
    let mut sorted: Vec<&(u16, Val)> = entries.iter().collect();
    sorted.sort_by_key(|e| e.0);
    let mut pos = at + 2 + 12 * sorted.len() + 4;
    let mut out = HashMap::new();
    for e in sorted {
        // The out-of-line size of one entry: its IFD size less 2 + 12 + 4.
        let size = ifd_size(std::slice::from_ref(e), blobs) - 18;
        if size > 0 {
            out.entry(e.0).or_insert(pos);
            pos += size;
        }
    }
    out
}

fn u16b(v: u16, le: bool) -> [u8; 2] {
    if le {
        v.to_le_bytes()
    } else {
        v.to_be_bytes()
    }
}

fn u32b(v: u32, le: bool) -> [u8; 4] {
    if le {
        v.to_le_bytes()
    } else {
        v.to_be_bytes()
    }
}

/// The bytes of a maker note that starts at file offset `at`.
fn note_bytes(
    spec: &NoteSpec,
    file_le: bool,
    at: usize,
    blob_at: &[usize],
    blobs: &[Vec<u8>],
) -> Vec<u8> {
    let olymp = spec.style == Style::Olymp;
    let le = if olymp { file_le } else { spec.le };
    let origin = if olymp { 0 } else { at };
    let rel: Vec<usize> = blob_at.iter().map(|&b| b.wrapping_sub(origin)).collect();
    let order = if le { *b"II" } else { *b"MM" };
    // Absolute positions: the note is written after `at` zero bytes.
    let mut out = vec![0u8; at];
    match spec.style {
        Style::Olymp => {
            out.extend_from_slice(b"OLYMP\0");
            out.extend_from_slice(&u16b(2, le));
        }
        Style::Olympus => {
            out.extend_from_slice(b"OLYMPUS\0");
            out.extend_from_slice(&order);
            out.extend_from_slice(&u16b(3, le));
        }
        Style::OmSystem => {
            out.extend_from_slice(b"OM SYSTEM\0\0\0");
            out.extend_from_slice(&order);
            out.extend_from_slice(&u16b(4, le));
        }
    }
    let main_at = out.len();
    let mut main = spec.main.clone();
    if olymp {
        // Each sub-IFD is the value of its entry: find where the value
        // lands, then write the sub-IFD for that position.
        let mut probe = main.clone();
        for (t, sub) in &spec.subs {
            probe.push((*t, Val::Undef(vec![0; ifd_size(sub, blobs)])));
        }
        let pos = value_positions(&probe, main_at, blobs);
        for (t, sub) in &spec.subs {
            let p = pos[t];
            let mut buf = vec![0u8; p];
            write_ifd(&mut buf, sub, 0, origin, le, false, &rel, blobs);
            main.push((*t, Val::Undef(buf.split_off(p))));
        }
        write_ifd(&mut out, &main, 0, origin, le, false, &rel, blobs);
    } else {
        let mut probe = main.clone();
        for (t, _) in &spec.subs {
            probe.push((*t, Val::Raw(13, 1, vec![0; 4])));
        }
        let mut sub_at = main_at + ifd_size(&probe, blobs);
        for (t, sub) in &spec.subs {
            main.push((
                *t,
                Val::Raw(13, 1, u32b((sub_at - origin) as u32, le).to_vec()),
            ));
            sub_at += ifd_size(sub, blobs);
        }
        write_ifd(&mut out, &main, 0, origin, le, false, &rel, blobs);
        for (_, sub) in &spec.subs {
            write_ifd(&mut out, sub, 0, origin, le, false, &rel, blobs);
        }
    }
    out.split_off(at)
}

impl Builder {
    /// Serialize the file.
    pub(super) fn build(&self) -> Vec<u8> {
        let (le, blobs) = (self.le, &self.blobs);
        let has_exif = self.note.is_some() || !self.exif.is_empty();
        let exif_with = |note: Option<Vec<u8>>| {
            let mut e = self.exif.clone();
            if let Some(n) = note {
                e.push((0x927C, Val::Undef(n)));
            }
            e
        };
        let ifd0_with = |exif_at: usize| {
            let mut e = self.ifd0.clone();
            if has_exif {
                e.push((0x8769, Val::Long(vec![exif_at as u32])));
            }
            e
        };
        // First pass: positions (sizes do not depend on offset values).
        let zeros = vec![0; blobs.len()];
        let note_len = self
            .note
            .as_ref()
            .map(|n| note_bytes(n, le, 0, &zeros, blobs).len());
        let exif_at = 8 + ifd_size(&ifd0_with(0), blobs);
        let probe = exif_with(note_len.map(|n| vec![0; n]));
        let note_at = value_positions(&probe, exif_at, blobs)
            .get(&0x927C)
            .copied()
            .unwrap_or(0);
        let mut at = exif_at;
        if has_exif {
            at += ifd_size(&probe, blobs);
        }
        let ifd1_at = at;
        if !self.ifd1.is_empty() {
            at += ifd_size(&self.ifd1, blobs);
        }
        let mut blob_at = Vec::new();
        for b in blobs {
            at = at.div_ceil(16) * 16;
            blob_at.push(at);
            at += b.len();
        }
        // Second pass: write.
        let mut out = if le {
            vec![
                0x49,
                0x49,
                0x52,
                if self.rs { 0x53 } else { 0x4F },
                8,
                0,
                0,
                0,
            ]
        } else {
            vec![0x4D, 0x4D, 0x4F, 0x52, 0, 0, 0, 8]
        };
        let next = if self.ifd1.is_empty() { 0 } else { ifd1_at };
        write_ifd(
            &mut out,
            &ifd0_with(exif_at),
            next,
            0,
            le,
            false,
            &blob_at,
            blobs,
        );
        if has_exif {
            assert_eq!(out.len(), exif_at);
            let note = self
                .note
                .as_ref()
                .map(|n| note_bytes(n, le, note_at, &blob_at, blobs));
            write_ifd(&mut out, &exif_with(note), 0, 0, le, false, &blob_at, blobs);
        }
        if !self.ifd1.is_empty() {
            assert_eq!(out.len(), ifd1_at);
            write_ifd(&mut out, &self.ifd1, 0, 0, le, false, &blob_at, blobs);
        }
        for (b, &a) in blobs.iter().zip(&blob_at) {
            out.resize(a, 0);
            out.extend_from_slice(b);
        }
        out
    }
}

/// The IFD0 entries of the fixtures: W x H, BitsPerSample, one strip in
/// blob 0 with H rows per strip, Orientation, Make "OLYMPUS" and Model
/// "TESTCAM".
pub(super) fn ifd0(w: u32, h: u32, bps: u16, orientation: u16) -> Vec<(u16, Val)> {
    vec![
        (0x0100, Val::Long(vec![w])),
        (0x0101, Val::Long(vec![h])),
        (0x0102, Val::Short(vec![bps])),
        (0x0103, Val::Short(vec![1])),
        (0x0106, Val::Short(vec![1])),
        (0x010F, Val::Ascii("OLYMPUS")),
        (0x0110, Val::Ascii("TESTCAM")),
        (0x0111, Val::Blobs(vec![0])),
        (0x0112, Val::Short(vec![orientation])),
        (0x0115, Val::Short(vec![1])),
        (0x0116, Val::Long(vec![h])),
        (0x0117, Val::BlobLens(vec![0])),
    ]
}

/// The Exif CFAPattern entry with pattern `p` in the given byte order.
pub(super) fn cfa(p: [u8; 4], le: bool) -> (u16, Val) {
    let mut v = u16b(2, le).to_vec();
    v.extend(u16b(2, le));
    v.extend(p);
    (0xA302, Val::Undef(v))
}

/// The matrix of the fixtures (E-M10 Mark II), as stored.
pub(super) const MATRIX: [u16; 9] = [440, 65384, 65504, 65466, 388, 65474, 14, 65408, 370];

/// The ImageProcessing entries of the fixtures: WB_RBLevels 512 384 256
/// 256, the matrix, BlackLevel2 `black`, GainBase 256, ValidBits `bits`,
/// and the crop of the whole W x H raster.
pub(super) fn image_processing(w: u32, h: u32, black: u16, bits: u16) -> Vec<(u16, Val)> {
    vec![
        (0x0100, Val::Short(vec![512, 384, 256, 256])),
        (0x0200, Val::Short(MATRIX.to_vec())),
        (0x0600, Val::Short(vec![black; 4])),
        (0x0610, Val::Short(vec![256])),
        (0x0611, Val::Short(vec![bits, 0])),
        (0x0612, Val::Short(vec![0, 0])),
        (0x0613, Val::Short(vec![0, 0])),
        (0x0614, Val::Long(vec![w])),
        (0x0615, Val::Long(vec![h])),
    ]
}

/// The common layout of F1, F3, F5, F6 and F7 (OLYMPUS style,
/// little-endian) for a W x H raster in `strip`.
pub(super) fn recipe(w: u32, h: u32, bps: u16, bits: u16, strip: Vec<u8>) -> Builder {
    Builder {
        le: true,
        rs: false,
        ifd0: ifd0(w, h, bps, 1),
        exif: vec![cfa([0, 1, 1, 2], true)],
        note: Some(NoteSpec {
            style: Style::Olympus,
            le: true,
            main: Vec::new(),
            subs: vec![(0x2040, image_processing(w, h, 256, bits))],
        }),
        ifd1: Vec::new(),
        blobs: vec![strip],
    }
}

/// The OLYMP-style main maker-note entries of F2 and F4: ColorMatrix,
/// BlackLevel, RedBalance 512 64, BlueBalance 384 64, ValidBits.
pub(super) fn olymp_main(black: u16, bits: u16) -> Vec<(u16, Val)> {
    vec![
        (0x1011, Val::Short(MATRIX.to_vec())),
        (0x1012, Val::Short(vec![black; 4])),
        (0x1017, Val::Short(vec![512, 64])),
        (0x1018, Val::Short(vec![384, 64])),
        (0x102C, Val::Short(vec![bits, 0])),
    ]
}

/// V1 samples: each value in the B high bits of a 16-bit sample in the
/// file byte order (the inverse of R-ORF-22).
pub(super) fn pack_v1(values: &[u16], le: bool, bits: u32) -> Vec<u8> {
    values
        .iter()
        .flat_map(|&v| u16b(v << (16 - bits), le))
        .collect()
}

/// V2 rows of W values (W a multiple of 10): 5 pairs and a zero pad byte
/// per group of 16 bytes (the inverse of R-ORF-23).
pub(super) fn pack_v2(values: &[u16]) -> Vec<u8> {
    values
        .chunks_exact(10)
        .flat_map(|g| {
            let mut out: Vec<u8> = g
                .chunks_exact(2)
                .flat_map(|p| {
                    let (a, b) = (p[0], p[1]);
                    [
                        (a & 0xFF) as u8,
                        ((a >> 8) | ((b & 15) << 4)) as u8,
                        (b >> 4) as u8,
                    ]
                })
                .collect();
            out.push(0);
            out
        })
        .collect()
}

/// 12-bit pairs, MSB-first, in the given row order (the inverse of
/// R-ORF-24).
pub(super) fn pack_v3(values: &[u16]) -> Vec<u8> {
    values
        .chunks_exact(2)
        .flat_map(|p| {
            let (a, b) = (p[0], p[1]);
            [
                (a >> 4) as u8,
                (((a & 15) << 4) | (b >> 8)) as u8,
                (b & 0xFF) as u8,
            ]
        })
        .collect()
}

/// V4 rows of W values (W a multiple of 8): the MSB-first 12-bit string
/// stored as little-endian 32-bit words (the inverse of R-ORF-25).
pub(super) fn pack_v4(values: &[u16]) -> Vec<u8> {
    let msb = pack_v3(values);
    msb.chunks_exact(4)
        .flat_map(|w| [w[3], w[2], w[1], w[0]])
        .collect()
}

/// MSB-first bit writer.
#[derive(Default)]
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn push(&mut self, v: u32, k: u32) {
        assert!(k <= 32 && (k == 32 || u64::from(v) < 1 << k));
        self.acc = (self.acc << k) | u64::from(v);
        self.n += k;
        while self.n >= 8 {
            self.out.push((self.acc >> (self.n - 8)) as u8);
            self.n -= 8;
        }
        self.acc &= (1 << self.n) - 1;
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

/// The prediction of the table of section Compression, written apart from
/// the decoder's.
fn prediction(v: &[u16], w: usize, x: usize, y: usize) -> i32 {
    let s = |x: usize, y: usize| i32::from(v[y * w + x]);
    match (x >= 2, y >= 2) {
        (false, false) => 0,
        (true, false) => s(x - 2, y),
        (false, true) => s(x, y - 2),
        (true, true) => {
            let (ra, rb, rc) = (s(x - 2, y), s(x, y - 2), s(x - 2, y - 2));
            let between = (ra < rc && rc < rb) || (rb < rc && rc < ra);
            match (between, (ra - rc).abs(), (rb - rc).abs()) {
                (true, a, b) if a > 32 || b > 32 => ra + rb - rc,
                (true, _, _) => (ra + rb).div_euclid(2),
                (false, a, b) if a > b => ra,
                (false, _, _) => rb,
            }
        }
    }
}

/// The V5/V6 code bits of a W x H raster (without the stream header), the
/// rules of section Compression applied in reverse, padded with zero bits
/// to a whole byte. The escape is used for every quotient of 12 or more.
pub(super) fn encode(values: &[u16], w: usize, h: usize) -> Vec<u8> {
    let mut bits = BitWriter::default();
    for y in 0..h {
        let mut a = [0u32; 2];
        let mut runs = [0u32; 2];
        let mut carry = [0i32; 2];
        for x in 0..w {
            let p = x & 1;
            let e = i32::from(values[y * w + x]) - prediction(values, w, x, y);
            // e = 4 D + L with L from 0 to 3; E = D - C.
            let (d, low) = (e.div_euclid(4), e.rem_euclid(4) as u32);
            let ev = d - carry[p];
            let (sign, m) = if ev >= 0 {
                (0, ev as u32)
            } else {
                (1, (-ev - 1) as u32)
            };
            let len = 32 - a[p].leading_zeros();
            let k = if runs[p] >= 3 {
                len.max(2)
            } else {
                len.saturating_sub(2).max(4)
            };
            let (q, r) = (m >> k, m & ((1 << k) - 1));
            bits.push(sign, 1);
            bits.push(low, 2);
            if q < 12 {
                // q zero bits closed by a 1 bit.
                bits.push(1, q + 1);
            } else {
                bits.push(0, 12);
                bits.push(q, 15 - k);
                bits.push(1, 1);
            }
            bits.push(r, k);
            carry[p] = (3 * d + carry[p]).div_euclid(32);
            a[p] = m;
            runs[p] = if m <= 16 { runs[p] + 1 } else { 0 };
        }
    }
    bits.finish()
}

/// A complete V5 (`bits` 12) or V6 (14) strip: stream header and code
/// bits.
pub(super) fn strip_v5v6(values: &[u16], w: usize, h: usize, bits: u32) -> Vec<u8> {
    let mut s = if bits == 14 {
        super::meta::V6_HEADER.to_vec()
    } else {
        super::meta::V5_HEADER.to_vec()
    };
    s.extend(encode(values, w, h));
    s
}

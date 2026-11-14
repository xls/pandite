//! The box structure of a CR3 file (section Container layout): box
//! headers, the top-level walk, the Canon boxes and the tracks.
//!
//! Every multi-byte field of a box, of the sample tables and of the Canon
//! boxes CMP1, THMB and PRVW is big-endian (R-CR3-3). Every offset and
//! size is a u64, and a sum that overflows counts as outside the file
//! (R-CR3-43). A byte range is checked against its parent box and the file
//! before it is read (R-CR3-45).

use super::{fail, Cr3Result};

/// Extended type of the Canon metadata box (glossary, M2).
pub(crate) const CANON_UUID: [u8; 16] = [
    0x85, 0xC0, 0xB6, 0x87, 0x82, 0x0F, 0x11, 0xE0, 0x81, 0x11, 0xF4, 0xCE, 0x46, 0x2B, 0x6A, 0x48,
];
/// Extended type of the preview holder box (glossary, M6).
pub(crate) const PREVIEW_UUID: [u8; 16] = [
    0xEA, 0xF4, 0x2B, 0x5E, 0x1C, 0x98, 0x4B, 0x88, 0xB9, 0xFB, 0xB7, 0xDC, 0x40, 0x6E, 0x4D, 0x16,
];

/// R-CR3-44: deepest box level read (the top level is level 1).
pub(crate) const MAX_LEVEL: usize = 16;
/// R-CR3-44: most boxes read from one container.
pub(crate) const MAX_BOXES: usize = 4096;
/// R-CR3-44: most "trak" boxes read.
pub(crate) const MAX_TRAKS: usize = 64;

/// The bytes `start..end` of the file, or `None` when the range does not
/// lie completely inside it (R-CR3-45).
pub(crate) fn range(d: &[u8], start: u64, end: u64) -> Option<&[u8]> {
    if start > end || end > d.len() as u64 {
        return None;
    }
    d.get(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
}

/// `n` bytes at `at`, inside the file (R-CR3-43: an overflowing end is
/// outside).
fn bytes(d: &[u8], at: u64, n: u64) -> Option<&[u8]> {
    range(d, at, at.checked_add(n)?)
}

/// A big-endian u16 (R-CR3-3).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn be16(d: &[u8], at: u64) -> Option<u16> {
    let b = bytes(d, at, 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

/// A big-endian u32 (R-CR3-3).
pub(crate) fn be32(d: &[u8], at: u64) -> Option<u32> {
    let b = bytes(d, at, 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// A big-endian u64 (R-CR3-3).
pub(crate) fn be64(d: &[u8], at: u64) -> Option<u64> {
    let b = bytes(d, at, 8)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(b);
    Some(u64::from_be_bytes(a))
}

/// One valid box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bx {
    /// The four-character type.
    pub typ: [u8; 4],
    /// File offset of the header.
    pub start: u64,
    /// File offset of the payload: after the 8-byte header, or the
    /// 16-byte header with an extended size. For a uuid box the payload
    /// starts with the 16-byte extended type.
    pub body: u64,
    /// File offset of the first byte after the box.
    pub end: u64,
}

impl Bx {
    /// True for a box of type `t`.
    pub(crate) fn is(&self, t: &[u8; 4]) -> bool {
        &self.typ == t
    }

    /// True for a uuid box with the extended type `ext`, compared in all
    /// 16 bytes (R-CR3-8).
    pub(crate) fn is_uuid(&self, d: &[u8], ext: &[u8; 16]) -> bool {
        self.is(b"uuid") && bytes(d, self.body, 16) == Some(&ext[..])
    }

    /// Size of the payload.
    pub(crate) fn payload_len(&self) -> u64 {
        self.end - self.body
    }

    /// Payload byte `i`, when the payload holds it.
    pub(crate) fn byte(&self, d: &[u8], i: u64) -> Option<u8> {
        if i >= self.payload_len() {
            return None;
        }
        bytes(d, self.body + i, 1).map(|x| x[0])
    }

    /// The payload bytes. A valid box lies inside the file (R-CR3-6), so
    /// the slice is never cut; an empty slice is returned otherwise.
    pub(crate) fn payload<'a>(&self, d: &'a [u8]) -> &'a [u8] {
        range(d, self.body, self.end).unwrap_or(&[])
    }

    /// Size of the whole box.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn size(&self) -> u64 {
        self.end - self.start
    }
}

/// The box whose header starts at `at` inside a parent that ends at
/// `parent_end`, or `None` when it is invalid (R-CR3-5, R-CR3-6).
pub(crate) fn header(d: &[u8], at: u64, parent_end: u64, top: bool) -> Option<Bx> {
    let head_end = at.checked_add(8)?;
    if head_end > parent_end {
        return None;
    }
    let size32 = be32(d, at)?;
    let t = bytes(d, at + 4, 4)?;
    let typ = [t[0], t[1], t[2], t[3]];
    // R-CR3-5: the u32 size, the u64 extended size when it is 1, and for a
    // top-level box with size 0 the distance to the end of the file.
    let (size, head) = match size32 {
        1 => {
            if at.checked_add(16)? > parent_end {
                return None;
            }
            (be64(d, at + 8)?, 16u64)
        }
        // R-CR3-6: size 0 is invalid below the top level.
        0 if !top => return None,
        0 => (parent_end - at, 8),
        s => (u64::from(s), 8),
    };
    // R-CR3-6: smaller than the header (plus the extended type of a uuid
    // box), or past the end of the parent.
    let min = head + if &typ == b"uuid" { 16 } else { 0 };
    if size < min {
        return None;
    }
    let end = at.checked_add(size)?;
    if end > parent_end {
        return None;
    }
    Some(Bx {
        typ,
        start: at,
        body: at + head,
        end,
    })
}

/// The boxes of the byte range `from..to`, a container at box level
/// `level` (1 for the top level). Reading stops at the first invalid box
/// and keeps the boxes before it (R-CR3-6). At most [`MAX_BOXES`] boxes are
/// read, and nothing below level [`MAX_LEVEL`] (R-CR3-44). Unknown types
/// are kept by type and size only; their payload is never read (R-CR3-7).
pub(crate) fn walk(d: &[u8], from: u64, to: u64, level: usize) -> Vec<Bx> {
    let mut out = Vec::new();
    if level == 0 || level > MAX_LEVEL || to > d.len() as u64 {
        return out;
    }
    let mut at = from;
    while at < to && out.len() < MAX_BOXES {
        let Some(b) = header(d, at, to, level == 1) else {
            break;
        };
        out.push(b);
        at = b.end;
    }
    out
}

/// The children of a container box at `level`.
pub(crate) fn children(d: &[u8], b: &Bx, level: usize) -> Vec<Bx> {
    walk(d, b.body, b.end, level + 1)
}

/// The first box of type `t`, in any order (R-CR3-9).
pub(crate) fn find(boxes: &[Bx], t: &[u8; 4]) -> Option<Bx> {
    boxes.iter().copied().find(|b| b.is(t))
}

/// The role of a track (R-CR3-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Role {
    /// Handler "vide", format "CRAW", child "JPEG".
    Jpeg,
    /// Handler "vide", format "CRAW", child "HEVC".
    Hevc,
    /// Handler "vide", format "CRAW", child "CMP1".
    Crx,
    /// Handler "meta", format "CTMD".
    Metadata,
    /// Any other track; ignored.
    Other,
}

/// One "trak" box, read as far as this document needs it.
#[derive(Clone, Debug)]
pub(crate) struct Track {
    /// Classification by content (R-CR3-15).
    pub role: Role,
    /// Handler type from "hdlr" (R-CR3-12).
    pub handler: Option<[u8; 4]>,
    /// Data format of the first sample entry (R-CR3-13).
    pub format: Option<[u8; 4]>,
    /// File offset and size of the first sample entry.
    pub entry: Option<(u64, u64)>,
    /// The child boxes of a CRAW entry (R-CR3-14).
    pub entry_children: Vec<Bx>,
    /// The CMP1 child of a CRX track.
    pub cmp1: Option<Bx>,
    /// File offset and size of the first sample, when usable (R-CR3-17,
    /// R-CR3-18).
    pub sample: Option<(u64, u64)>,
    /// The "stsz" number of entries, 0 without "stsz" (R-CR3-2 rule 2).
    pub samples: u32,
}

impl Track {
    /// True for an image track: handler "vide" with a CRAW entry of a
    /// known kind.
    pub(crate) fn is_image(&self) -> bool {
        matches!(self.role, Role::Jpeg | Role::Hevc | Role::Crx)
    }

    /// The CRX image size from CMP1 payload bytes 8 to 11 and 12 to 15
    /// (R-CR3-16).
    pub(crate) fn cmp1_size(&self, d: &[u8]) -> Option<(u32, u32)> {
        let b = self.cmp1?;
        if b.payload_len() < 16 {
            return None;
        }
        Some((be32(d, b.body + 8)?, be32(d, b.body + 12)?))
    }

    /// CMP1 payload byte 26 (R-CR3-2 rules 3 and 4).
    pub(crate) fn cmp1_mode(&self, d: &[u8]) -> Option<u8> {
        self.cmp1?.byte(d, 26)
    }
}

/// The handler type at bytes 8 to 11 of the "hdlr" payload (R-CR3-12).
fn handler(d: &[u8], hdlr: &Bx) -> Option<[u8; 4]> {
    if hdlr.payload_len() < 12 {
        return None;
    }
    let b = bytes(d, hdlr.body + 8, 4)?;
    Some([b[0], b[1], b[2], b[3]])
}

/// The first sample entry of "stsd": at payload offset 8, size at least 8
/// and inside the "stsd" payload (R-CR3-13).
fn first_entry(d: &[u8], stsd: &Bx) -> Option<(u64, u64)> {
    let at = stsd.body.checked_add(8)?;
    if at.checked_add(8)? > stsd.end {
        return None;
    }
    let size = u64::from(be32(d, at)?);
    if size < 8 || at.checked_add(size)? > stsd.end {
        return None;
    }
    Some((at, size))
}

/// Offset of the CRAW child boxes inside the entry (R-CR3-14, M4).
const CRAW_CHILDREN: u64 = 90;

/// R-CR3-17 and R-CR3-18: the first sample of a track from "stsz" and
/// "co64" (or "stco" when the track has no "co64").
fn first_sample(d: &[u8], stbl: &[Bx]) -> (Option<(u64, u64)>, u32) {
    let Some(stsz) = find(stbl, b"stsz") else {
        return (None, 0);
    };
    // stsz: version and flags, sample size, number of entries, table.
    let count = if stsz.payload_len() >= 12 {
        be32(d, stsz.body + 8).unwrap_or(0)
    } else {
        0
    };
    let sample = (|| {
        if count == 0 {
            return None;
        }
        let fixed = be32(d, stsz.body + 4)?;
        let size = if fixed != 0 {
            u64::from(fixed)
        } else {
            // R-CR3-45: the first table entry lies inside "stsz".
            if stsz.payload_len() < 16 {
                return None;
            }
            u64::from(be32(d, stsz.body + 12)?)
        };
        // co64 replaces stco (S2); 8-byte entries in co64, 4 in stco.
        let (chunks, wide) = match find(stbl, b"co64") {
            Some(b) => (b, true),
            None => (find(stbl, b"stco")?, false),
        };
        let need = if wide { 16 } else { 12 };
        if chunks.payload_len() < need || be32(d, chunks.body + 4)? == 0 {
            return None;
        }
        let offset = if wide {
            be64(d, chunks.body + 8)?
        } else {
            u64::from(be32(d, chunks.body + 8)?)
        };
        // R-CR3-18: the sample lies completely inside the file (64-bit
        // sum, R-CR3-43).
        let end = offset.checked_add(size)?;
        (end <= d.len() as u64).then_some((offset, size))
    })();
    (sample, count)
}

/// Read one "trak" box at box level `level` (R-CR3-12 to R-CR3-18).
fn track(d: &[u8], trak: &Bx, level: usize) -> Track {
    let mut t = Track {
        role: Role::Other,
        handler: None,
        format: None,
        entry: None,
        entry_children: Vec::new(),
        cmp1: None,
        sample: None,
        samples: 0,
    };
    let kids = children(d, trak, level);
    let Some(mdia) = find(&kids, b"mdia") else {
        return t;
    };
    let mdia_kids = children(d, &mdia, level + 1);
    t.handler = find(&mdia_kids, b"hdlr").and_then(|h| handler(d, &h));
    let Some(minf) = find(&mdia_kids, b"minf") else {
        return t;
    };
    let minf_kids = children(d, &minf, level + 2);
    let Some(stbl) = find(&minf_kids, b"stbl") else {
        return t;
    };
    let stbl_kids = children(d, &stbl, level + 3);
    if let Some(stsd) = find(&stbl_kids, b"stsd") {
        if let Some((at, size)) = first_entry(d, &stsd) {
            t.entry = Some((at, size));
            t.format = bytes(d, at + 4, 4).map(|b| [b[0], b[1], b[2], b[3]]);
            // R-CR3-14: children from entry offset 90; none when the entry
            // is shorter than 90 bytes. The entry is at level + 5, its
            // children one below.
            if t.format == Some(*b"CRAW") && size >= CRAW_CHILDREN {
                t.entry_children = walk(d, at + CRAW_CHILDREN, at + size, level + 6);
            }
        }
    }
    let (sample, samples) = first_sample(d, &stbl_kids);
    t.sample = sample;
    t.samples = samples;
    // R-CR3-15: by content, not by track number or position.
    let has = |c: &[u8; 4]| t.entry_children.iter().any(|b| b.is(c));
    t.role = match (t.handler.as_ref(), t.format.as_ref()) {
        (Some(b"vide"), Some(b"CRAW")) if has(b"JPEG") => Role::Jpeg,
        (Some(b"vide"), Some(b"CRAW")) if has(b"HEVC") => Role::Hevc,
        (Some(b"vide"), Some(b"CRAW")) if has(b"CMP1") => Role::Crx,
        (Some(b"meta"), Some(b"CTMD")) => Role::Metadata,
        _ => Role::Other,
    };
    if t.role == Role::Crx {
        t.cmp1 = find(&t.entry_children, b"CMP1");
    }
    t
}

/// The parsed box structure of a CR3 file. The tests read every field.
#[derive(Clone, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Container<'a> {
    /// The whole file.
    pub data: &'a [u8],
    /// The top-level boxes up to the first invalid one.
    pub top: Vec<Bx>,
    /// The first "moov" box (R-CR3-9).
    pub moov: Option<Bx>,
    /// The Canon metadata box among the children of "moov" (R-CR3-9).
    pub canon: Option<Bx>,
    /// The children of the Canon metadata box (R-CR3-10).
    pub canon_children: Vec<Bx>,
    /// The preview holder box among the top-level boxes (R-CR3-9).
    pub holder: Option<Bx>,
    /// The "PRVW" boxes of the preview holder box (R-CR3-11).
    pub prvw: Vec<Bx>,
    /// The tracks, in "moov" order (at most 64, R-CR3-44).
    pub tracks: Vec<Track>,
}

/// Minimum file size (R-CR3-49).
const MIN_FILE: usize = 24;

/// R-CR3-4: bytes 4 to 7 are "ftyp", the first box size is at least 16
/// and the major brand is "crx ". The size is the one of R-CR3-5: the
/// extended size when the u32 size is 1, the file size when it is 0.
pub(crate) fn is_cr3(d: &[u8]) -> bool {
    let size = match be32(d, 0) {
        Some(0) => Some(d.len() as u64),
        Some(1) => be64(d, 8),
        s => s.map(u64::from),
    };
    d.get(4..8) == Some(b"ftyp") && size.is_some_and(|s| s >= 16) && d.get(8..12) == Some(b"crx ")
}

/// Read the box structure. Fails for a file shorter than 24 bytes
/// (R-CR3-49); `Ok(None)` means "not a CR3 file" (R-CR3-4).
pub(crate) fn open(d: &[u8]) -> Cr3Result<Option<Container<'_>>> {
    if d.len() < MIN_FILE {
        return Err(fail("file shorter than 24 bytes"));
    }
    if !is_cr3(d) {
        return Ok(None);
    }
    // Steps 2 and 3: the top-level boxes (level 1).
    let top = walk(d, 0, d.len() as u64, 1);
    let moov = find(&top, b"moov");
    let moov_kids = moov.map(|m| children(d, &m, 1)).unwrap_or_default();
    let canon = moov_kids
        .iter()
        .copied()
        .find(|b| b.is_uuid(d, &CANON_UUID));
    // R-CR3-10: the Canon metadata box payload after its extended type.
    let canon_children = canon
        .and_then(|b| Some(walk(d, b.body.checked_add(16)?, b.end, 3)))
        .unwrap_or_default();
    let holder = top.iter().copied().find(|b| b.is_uuid(d, &PREVIEW_UUID));
    // R-CR3-11: the preview holder payload from byte 24.
    let prvw = holder
        .and_then(|b| {
            let from = b.body.checked_add(24)?;
            (from <= b.end).then(|| walk(d, from, b.end, 2))
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|b| b.is(b"PRVW"))
        .collect();
    // Step 4: every "trak" of "moov", at most 64 (R-CR3-44).
    let tracks = moov_kids
        .iter()
        .filter(|b| b.is(b"trak"))
        .take(MAX_TRAKS)
        .map(|b| track(d, b, 2))
        .collect();
    Ok(Some(Container {
        data: d,
        top,
        moov,
        canon,
        canon_children,
        holder,
        prvw,
        tracks,
    }))
}

impl Container<'_> {
    /// The first child of the Canon metadata box of type `t`.
    pub(crate) fn canon_box(&self, t: &[u8; 4]) -> Option<Bx> {
        find(&self.canon_children, t)
    }

    /// The main CRX track: the largest product of the CMP1 width and
    /// height, the first in "moov" order on a tie (R-CR3-16). A CMP1 too
    /// short to hold its size counts as 0 x 0.
    pub(crate) fn main_crx(&self) -> Option<&Track> {
        let mut best: Option<(&Track, u64)> = None;
        for t in self.tracks.iter().filter(|t| t.role == Role::Crx) {
            let area = t
                .cmp1_size(self.data)
                .map_or(0, |(w, h)| u64::from(w) * u64::from(h));
            if best.is_none_or(|(_, a)| area > a) {
                best = Some((t, area));
            }
        }
        best.map(|(t, _)| t)
    }

    /// True when the file is Dual Pixel RAW: a second CRX track with the
    /// CMP1 width and height of the main CRX track (R-CR3-2, X3).
    pub(crate) fn dual_pixel(&self) -> bool {
        let Some(main) = self.main_crx() else {
            return false;
        };
        let Some(size) = main.cmp1_size(self.data) else {
            return false;
        };
        self.tracks.iter().any(|t| {
            t.role == Role::Crx && !std::ptr::eq(t, main) && t.cmp1_size(self.data) == Some(size)
        })
    }

    /// The version byte (payload offset 0) of a THMB or PRVW box.
    pub(crate) fn version(&self, b: &Bx) -> Option<u8> {
        b.byte(self.data, 0)
    }

    /// R-CR3-2: the variant of the file.
    pub(crate) fn variant(&self) -> Variant {
        // Rule 1: an HEVC track, or THMB or PRVW with version 1.
        let thmb_v1 = self.canon_box(b"THMB").and_then(|b| self.version(&b)) == Some(1);
        let prvw_v1 = self.prvw.iter().any(|b| self.version(b) == Some(1));
        if self.tracks.iter().any(|t| t.role == Role::Hevc) || thmb_v1 || prvw_v1 {
            return Variant::X5;
        }
        // Rule 2: an image track with more than one sample.
        if self.tracks.iter().any(|t| t.is_image() && t.samples > 1) {
            return Variant::X4;
        }
        // Rules 3 to 5: CMP1 byte 26 of the main CRX track.
        match self.main_crx().and_then(|t| t.cmp1_mode(self.data)) {
            Some(0) => Variant::X1,
            Some(_) => Variant::X2,
            None => Variant::Unknown,
        }
    }

    /// The sensor data coding of the main CRX track by CMP1 byte 26
    /// (variant table: X1 when 0, X2 otherwise), independent of the
    /// precedence of R-CR3-2. Used for the facts.
    pub(crate) fn crx_mode(&self) -> Option<Variant> {
        match self.main_crx()?.cmp1_mode(self.data)? {
            0 => Some(Variant::X1),
            _ => Some(Variant::X2),
        }
    }
}

/// The variants of R-CR3-2. Dual Pixel RAW (X3) is reported apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Variant {
    /// CRX lossless (camera setting RAW).
    X1,
    /// CRX lossy (camera setting C-RAW).
    X2,
    /// Image tracks with more than one sample.
    X4,
    /// HEVC previews.
    X5,
    /// Any other file.
    Unknown,
}

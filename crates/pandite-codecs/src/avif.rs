//! AVIF decoder: a HEIF container reader written here plus the pure Rust
//! AV1 decoder `rav1d` (the `sqzer-rav1d` package: rav1d with its safe Rust
//! API, so a decoder panic unwinds into the panic guard instead of aborting
//! at a C boundary). It decodes on the calling thread; worker threads would
//! not survive a panic.
//!
//! Supports single images and grids, 8, 10 and 12-bit, 4:0:0, 4:2:0, 4:2:2
//! and 4:4:4, limited and full range, BT.601, BT.709, BT.2020 and identity
//! matrices, alpha auxiliary images (straight or premultiplied), the clean
//! aperture, rotation and mirror properties, ICC profiles and CICP color
//! descriptions. SDR 8-bit images decode to `Rgba8`, deeper SDR images to
//! `Rgba16`, PQ and HLG images to linear `RgbaF32` with BT.2020 converted
//! to sRGB primaries. Image sequences (animated AVIF) show their primary
//! still item when the file has one.

use crate::util::{self, be16, be32, check_size, corrupt, guard, unsupported};
use crate::{icc, orient};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageFacts,
    ImageMeta, Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "avif";

/// Decodes AVIF files.
#[derive(Clone, Copy, Debug, Default)]
pub struct AvifDecoder;

/// True when the `ftyp` box names an AVIF brand.
pub(crate) fn is_avif(header: &[u8]) -> bool {
    util::ftyp_brands(header).any(|b| b == *b"avif" || b == *b"avis")
}

impl DecoderProvider for AvifDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "AVIF image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["avif"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        is_avif(header)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let heif = Heif::parse(source.bytes())?;
        let primary = heif.primary;
        let (w, h) = heif.output_size(primary)?;
        let orientation = heif.orientation(primary);
        let (width, height) = crate::jpeg::oriented(w, h, orientation);
        Ok(ImageMeta {
            width,
            height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: heif.alpha_for(primary).is_some(),
            orientation,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), &options.cancel)).map(Content::Still)
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        guard(FORMAT, || Ok(heif_facts(source.bytes())))
    }
}

// ---------------------------------------------------------------------------
// HEIF container
// ---------------------------------------------------------------------------

/// Item location.
#[derive(Clone, Debug, Default)]
struct Location {
    method: u8,
    extents: Vec<(u64, u64)>,
}

#[derive(Clone, Debug)]
enum Prop<'a> {
    Ispe(u32, u32),
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full: bool,
    },
    Icc(&'a [u8]),
    Irot(u8),
    Imir(u8),
    Clap([i64; 8]),
    AuxC(&'a [u8]),
    /// HEVC configuration: chroma format (0 to 3) and luma bit depth.
    Hvcc {
        chroma: u8,
        bits: u8,
    },
    /// AV1 configuration: bit depth, monochrome, chroma subsampling x, y.
    Av1c {
        bits: u8,
        mono: bool,
        sub_x: bool,
        sub_y: bool,
    },
    /// Bits per channel of each channel.
    Pixi(&'a [u8]),
    Other,
}

/// The parts of a HEIF `meta` box an image decoder needs.
pub(crate) struct Heif<'a> {
    data: &'a [u8],
    primary: u32,
    items: Vec<(u32, [u8; 4])>,
    locations: Vec<(u32, Location)>,
    props: Vec<Prop<'a>>,
    assoc: Vec<(u32, Vec<u16>)>,
    refs: Vec<([u8; 4], u32, Vec<u32>)>,
    idat: &'a [u8],
}

/// Iterate the child boxes in `data`: (type, body).
fn boxes(data: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        if pos + 8 > data.len() {
            return None;
        }
        let size32 = be32(data, pos)? as u64;
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let (header, size) = match size32 {
            0 => (8u64, (data.len() - pos) as u64),
            1 => {
                let hi = u64::from(be32(data, pos + 8)?);
                let lo = u64::from(be32(data, pos + 12)?);
                (16, (hi << 32) | lo)
            }
            s => (8, s),
        };
        let end = pos.checked_add(usize::try_from(size).ok()?)?;
        if size < header || end > data.len() {
            return None;
        }
        let body = &data[pos + header as usize..end];
        pos = end;
        Some((kind, body))
    })
}

/// A big-endian reader over one box body.
struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    fn new(d: &'a [u8]) -> Self {
        Rd { d, p: 0 }
    }
    fn u8(&mut self) -> Result<u8> {
        let v = *self
            .d
            .get(self.p)
            .ok_or_else(|| corrupt(FORMAT, "short box"))?;
        self.p += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16> {
        let v = be16(self.d, self.p).ok_or_else(|| corrupt(FORMAT, "short box"))?;
        self.p += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32> {
        let v = be32(self.d, self.p).ok_or_else(|| corrupt(FORMAT, "short box"))?;
        self.p += 4;
        Ok(v)
    }
    /// An unsigned integer of 0, 4 or 8 bytes.
    fn uint(&mut self, size: u8) -> Result<u64> {
        Ok(match size {
            0 => 0,
            4 => u64::from(self.u32()?),
            8 => (u64::from(self.u32()?) << 32) | u64::from(self.u32()?),
            2 => u64::from(self.u16()?),
            _ => return Err(corrupt(FORMAT, "bad field size")),
        })
    }
    fn full(&mut self) -> Result<(u8, u32)> {
        let v = self.u32()?;
        Ok(((v >> 24) as u8, v & 0xFF_FFFF))
    }
    fn fourcc(&mut self) -> Result<[u8; 4]> {
        let b = self
            .d
            .get(self.p..self.p + 4)
            .ok_or_else(|| corrupt(FORMAT, "short box"))?;
        self.p += 4;
        Ok([b[0], b[1], b[2], b[3]])
    }
    fn rest(&self) -> &'a [u8] {
        self.d.get(self.p..).unwrap_or(&[])
    }
}

impl<'a> Heif<'a> {
    pub(crate) fn parse(data: &'a [u8]) -> Result<Self> {
        let meta = boxes(data)
            .find(|(k, _)| k == b"meta")
            .ok_or_else(|| corrupt(FORMAT, "missing meta box"))?
            .1;
        let mut h = Heif {
            data,
            primary: 0,
            items: Vec::new(),
            locations: Vec::new(),
            props: Vec::new(),
            assoc: Vec::new(),
            refs: Vec::new(),
            idat: &[],
        };
        let mut have_primary = false;
        for (kind, body) in boxes(meta.get(4..).unwrap_or(&[])) {
            let mut r = Rd::new(body);
            match &kind {
                b"pitm" => {
                    let (v, _) = r.full()?;
                    h.primary = if v == 0 {
                        u32::from(r.u16()?)
                    } else {
                        r.u32()?
                    };
                    have_primary = true;
                }
                b"iinf" => {
                    let (v, _) = r.full()?;
                    let _count = if v == 0 {
                        u32::from(r.u16()?)
                    } else {
                        r.u32()?
                    };
                    for (k, infe) in boxes(r.rest()) {
                        if &k != b"infe" {
                            continue;
                        }
                        let mut e = Rd::new(infe);
                        let (v, _) = e.full()?;
                        if v < 2 {
                            continue;
                        }
                        let id = if v == 2 {
                            u32::from(e.u16()?)
                        } else {
                            e.u32()?
                        };
                        let _protection = e.u16()?;
                        h.items.push((id, e.fourcc()?));
                    }
                }
                b"iloc" => h.locations = parse_iloc(&mut r)?,
                b"iref" => {
                    let (v, _) = r.full()?;
                    for (k, refbox) in boxes(r.rest()) {
                        let mut e = Rd::new(refbox);
                        let id = |e: &mut Rd| -> Result<u32> {
                            if v == 0 {
                                Ok(u32::from(e.u16()?))
                            } else {
                                e.u32()
                            }
                        };
                        let from = id(&mut e)?;
                        let n = e.u16()?;
                        let mut to = Vec::with_capacity(usize::from(n).min(4096));
                        for _ in 0..n {
                            to.push(id(&mut e)?);
                        }
                        h.refs.push((k, from, to));
                    }
                }
                b"iprp" => {
                    for (k, sub) in boxes(body) {
                        match &k {
                            b"ipco" => {
                                for (pk, pb) in boxes(sub) {
                                    h.props.push(parse_prop(&pk, pb).unwrap_or(Prop::Other));
                                }
                            }
                            b"ipma" => parse_ipma(sub, &mut h.assoc)?,
                            _ => {}
                        }
                    }
                }
                b"idat" => h.idat = body,
                _ => {}
            }
        }
        if !have_primary {
            return Err(corrupt(FORMAT, "missing primary item"));
        }
        Ok(h)
    }

    fn item_type(&self, id: u32) -> Option<[u8; 4]> {
        self.items.iter().find(|(i, _)| *i == id).map(|(_, t)| *t)
    }

    fn props_of(&self, id: u32) -> impl Iterator<Item = &Prop<'a>> {
        self.assoc
            .iter()
            .filter(move |(i, _)| *i == id)
            .flat_map(|(_, list)| list.iter())
            .filter_map(|&idx| self.props.get(usize::from(idx).checked_sub(1)?))
    }

    /// The bytes of an item, extents joined.
    fn item_data(&self, id: u32) -> Result<Vec<u8>> {
        let loc = self
            .locations
            .iter()
            .find(|(i, _)| *i == id)
            .map(|(_, l)| l)
            .ok_or_else(|| corrupt(FORMAT, "item without location"))?;
        let src = match loc.method {
            0 => self.data,
            1 => self.idat,
            _ => return Err(unsupported(FORMAT, "item-offset construction")),
        };
        let mut out = Vec::new();
        for &(off, len) in &loc.extents {
            let off = usize::try_from(off).map_err(|_| corrupt(FORMAT, "bad extent"))?;
            let end = if len == 0 {
                src.len()
            } else {
                off.checked_add(usize::try_from(len).map_err(|_| corrupt(FORMAT, "bad extent"))?)
                    .ok_or_else(|| corrupt(FORMAT, "bad extent"))?
            };
            let part = src
                .get(off..end)
                .ok_or_else(|| corrupt(FORMAT, "item data outside the file"))?;
            out.extend_from_slice(part);
        }
        Ok(out)
    }

    fn refs_from(&self, kind: &[u8; 4], from: u32) -> Vec<u32> {
        self.refs
            .iter()
            .filter(|(k, f, _)| k == kind && *f == from)
            .flat_map(|(_, _, to)| to.iter().copied())
            .collect()
    }

    /// The alpha auxiliary item for `id`, when present.
    pub(crate) fn alpha_for(&self, id: u32) -> Option<u32> {
        self.refs
            .iter()
            .filter(|(k, _, to)| k == b"auxl" && to.contains(&id))
            .map(|(_, from, _)| *from)
            .find(|&aux| {
                self.props_of(aux).any(|p| {
                    matches!(p, Prop::AuxC(urn) if urn.starts_with(b"urn:mpeg:mpegB:cicp:systems:auxiliary:alpha")
                        || urn.starts_with(b"urn:mpeg:hevc:2015:auxid:1"))
                })
            })
    }

    fn ispe(&self, id: u32) -> Option<(u32, u32)> {
        self.props_of(id).find_map(|p| match p {
            Prop::Ispe(w, h) => Some((*w, *h)),
            _ => None,
        })
    }

    /// Size after the clean aperture, before rotation.
    pub(crate) fn output_size(&self, id: u32) -> Result<(u32, u32)> {
        let (w, h) = self
            .ispe(id)
            .ok_or_else(|| corrupt(FORMAT, "missing image size"))?;
        Ok(match self.clap_rect(id, w, h) {
            Some((_, _, cw, ch)) => (cw, ch),
            None => (w, h),
        })
    }

    /// The clean aperture as (x, y, w, h) in pixels.
    fn clap_rect(&self, id: u32, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
        let c = self.props_of(id).find_map(|p| match p {
            Prop::Clap(c) => Some(*c),
            _ => None,
        })?;
        if c.iter().skip(1).step_by(2).any(|&d| d == 0) {
            return None;
        }
        let cw = c[0] / c[1];
        let ch = c[2] / c[3];
        // Center offset relative to the image center.
        let cx = f64::from(w - 1) / 2.0 + c[4] as f64 / c[5] as f64;
        let cy = f64::from(h - 1) / 2.0 + c[6] as f64 / c[7] as f64;
        let x = (cx - (cw - 1) as f64 / 2.0).round();
        let y = (cy - (ch - 1) as f64 / 2.0).round();
        if cw <= 0 || ch <= 0 || x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y, cw, ch) = (x as u32, y as u32, cw as u32, ch as u32);
        (x + cw <= w && y + ch <= h).then_some((x, y, cw, ch))
    }

    /// Rotation then mirror, as one orientation.
    pub(crate) fn orientation(&self, id: u32) -> Orientation {
        let rot = self
            .props_of(id)
            .find_map(|p| match p {
                Prop::Irot(a) => Some(*a),
                _ => None,
            })
            .unwrap_or(0);
        let mir = self.props_of(id).find_map(|p| match p {
            Prop::Imir(a) => Some(*a),
            _ => None,
        });
        // irot turns anti-clockwise; imir axis 0 mirrors left-right.
        match (rot & 3, mir) {
            (0, None) => Orientation::Normal,
            (1, None) => Orientation::Rotate270,
            (2, None) => Orientation::Rotate180,
            (3, None) => Orientation::Rotate90,
            (0, Some(0)) => Orientation::FlipH,
            (0, Some(_)) => Orientation::FlipV,
            (1, Some(0)) => Orientation::Transverse,
            (1, Some(_)) => Orientation::Transpose,
            (2, Some(0)) => Orientation::FlipV,
            (2, Some(_)) => Orientation::FlipH,
            (3, Some(0)) => Orientation::Transpose,
            (_, Some(_)) => Orientation::Transverse,
            _ => Orientation::Normal,
        }
    }

    fn nclx(&self, id: u32) -> Option<(u16, u16, u16, bool)> {
        self.props_of(id).find_map(|p| match p {
            Prop::Nclx {
                primaries,
                transfer,
                matrix,
                full,
            } => Some((*primaries, *transfer, *matrix, *full)),
            _ => None,
        })
    }

    fn icc(&self, id: u32) -> Option<&'a [u8]> {
        self.props_of(id).find_map(|p| match p {
            Prop::Icc(d) => Some(*d),
            _ => None,
        })
    }
}

fn parse_iloc(r: &mut Rd) -> Result<Vec<(u32, Location)>> {
    let (v, _) = r.full()?;
    let a = r.u8()?;
    let b = r.u8()?;
    let (offset_size, length_size, base_size) = (a >> 4, a & 15, b >> 4);
    let index_size = if v >= 1 { b & 15 } else { 0 };
    let count = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
    let mut out = Vec::new();
    for _ in 0..count {
        let id = if v < 2 { u32::from(r.u16()?) } else { r.u32()? };
        let method = if v >= 1 { (r.u16()? & 15) as u8 } else { 0 };
        let _dref = r.u16()?;
        let base = r.uint(base_size)?;
        let n = r.u16()?;
        let mut extents = Vec::with_capacity(usize::from(n).min(1024));
        for _ in 0..n {
            if index_size > 0 {
                r.uint(index_size)?;
            }
            let off = r.uint(offset_size)?;
            let len = r.uint(length_size)?;
            extents.push((base.saturating_add(off), len));
        }
        out.push((id, Location { method, extents }));
    }
    Ok(out)
}

fn parse_ipma(body: &[u8], out: &mut Vec<(u32, Vec<u16>)>) -> Result<()> {
    let mut r = Rd::new(body);
    let (v, flags) = r.full()?;
    let n = r.u32()?;
    for _ in 0..n {
        let id = if v < 1 { u32::from(r.u16()?) } else { r.u32()? };
        let count = r.u8()?;
        let mut list = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let idx = if flags & 1 != 0 {
                r.u16()? & 0x7FFF
            } else {
                u16::from(r.u8()? & 0x7F)
            };
            list.push(idx);
        }
        out.push((id, list));
    }
    Ok(())
}

fn parse_prop<'a>(kind: &[u8; 4], body: &'a [u8]) -> Result<Prop<'a>> {
    let mut r = Rd::new(body);
    Ok(match kind {
        b"ispe" => {
            r.full()?;
            Prop::Ispe(r.u32()?, r.u32()?)
        }
        b"colr" => match &r.fourcc()? {
            b"nclx" => Prop::Nclx {
                primaries: r.u16()?,
                transfer: r.u16()?,
                matrix: r.u16()?,
                full: r.u8()? & 0x80 != 0,
            },
            b"rICC" | b"prof" => Prop::Icc(r.rest()),
            _ => Prop::Other,
        },
        b"irot" => Prop::Irot(r.u8()? & 3),
        b"imir" => Prop::Imir(r.u8()? & 1),
        b"clap" => {
            let mut c = [0i64; 8];
            for (i, v) in c.iter_mut().enumerate() {
                let raw = r.u32()?;
                // Widths and heights are unsigned, offsets signed.
                *v = if i == 4 || i == 6 {
                    i64::from(raw as i32)
                } else {
                    i64::from(raw)
                };
            }
            Prop::Clap(c)
        }
        b"auxC" => {
            r.full()?;
            Prop::AuxC(r.rest())
        }
        // ISO/IEC 14496-15: chroma_format_idc and bit_depth_luma_minus8
        // follow 16 bytes of profile, level and segmentation fields.
        b"hvcC" => {
            let b = body
                .get(16..18)
                .ok_or_else(|| corrupt(FORMAT, "short box"))?;
            Prop::Hvcc {
                chroma: b[0] & 3,
                bits: (b[1] & 7) + 8,
            }
        }
        // AV1 codec configuration: the third byte holds the flags.
        b"av1C" => {
            let b = *body.get(2).ok_or_else(|| corrupt(FORMAT, "short box"))?;
            let high = b & 0x40 != 0;
            let twelve = b & 0x20 != 0;
            Prop::Av1c {
                bits: match (high, twelve) {
                    (true, true) => 12,
                    (true, false) => 10,
                    _ => 8,
                },
                mono: b & 0x10 != 0,
                sub_x: b & 0x08 != 0,
                sub_y: b & 0x04 != 0,
            }
        }
        b"pixi" => {
            r.full()?;
            let n = usize::from(r.u8()?);
            Prop::Pixi(
                r.rest()
                    .get(..n)
                    .ok_or_else(|| corrupt(FORMAT, "short box"))?,
            )
        }
        _ => Prop::Other,
    })
}

/// The header facts of a HEIF container (AVIF, HEIC, HEIF): the brand,
/// the codec of the primary item (of its first tile for grids), chroma
/// format and bit depth from the codec configuration, the color
/// description, alpha and the EXIF item. Reads boxes only.
pub(crate) fn heif_facts(data: &[u8]) -> ImageFacts {
    let brand = util::ftyp_brands(data).next();
    let container = match brand.as_ref().map(|b| &b[..]) {
        Some(b"avif" | b"avis") => "AVIF",
        Some(b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx") => "HEIC",
        _ => "HEIF",
    };
    let mut f = ImageFacts::named(container);
    let Ok(heif) = Heif::parse(data) else {
        return f;
    };
    let primary = heif.primary;
    let mut coded = primary;
    let mut kind = heif.item_type(primary);
    if kind == Some(*b"grid") {
        if let Some(&tile) = heif.refs_from(b"dimg", primary).first() {
            coded = tile;
            kind = heif.item_type(tile);
        }
    }
    if let Some(k) = kind {
        f.fourcc = Some(String::from_utf8_lossy(&k).to_string());
        f.compression = match &k {
            b"av01" => Some("AV1"),
            b"hvc1" | b"hev1" => Some("HEVC"),
            b"vvc1" => Some("VVC"),
            b"jpeg" => Some("JPEG"),
            b"unci" => Some("Uncompressed"),
            _ => None,
        }
        .map(str::to_string);
    }
    let alpha = heif.alpha_for(primary).is_some();
    let config = heif
        .props_of(coded)
        .chain(heif.props_of(primary))
        .find_map(|p| match *p {
            Prop::Hvcc { chroma, bits } => Some((chroma == 0, chroma, bits)),
            Prop::Av1c {
                bits,
                mono,
                sub_x,
                sub_y,
            } => {
                let chroma = match (mono, sub_x, sub_y) {
                    (true, _, _) => 0,
                    (false, true, true) => 1,
                    (false, true, false) => 2,
                    (false, false, _) => 3,
                };
                Some((mono, chroma, bits))
            }
            _ => None,
        });
    if let Some((mono, chroma, bits)) = config {
        let (name, count) = match (mono, alpha) {
            (true, false) => ("Gray", 1),
            (true, true) => ("Gray + alpha", 2),
            (false, false) => ("YCbCr", 3),
            (false, true) => ("YCbCr + alpha", 4),
        };
        f.set_channels(name, count, bits);
        f.subsampling = match chroma {
            1 => Some("4:2:0"),
            2 => Some("4:2:2"),
            3 => Some("4:4:4"),
            _ => None,
        }
        .map(str::to_string);
    }
    let pixi = heif.props_of(primary).find_map(|p| match p {
        Prop::Pixi(b) => Some(*b),
        _ => None,
    });
    if let Some(&bits) = pixi.and_then(|b| b.first()) {
        f.bits_per_channel = Some(bits);
    }
    if let Some((primaries, transfer, _, _)) = heif.nclx(primary) {
        f.color_space = crate::facts::cicp_name(primaries, transfer);
    }
    f.icc_profile = heif.icc(primary).and_then(crate::facts::icc_description);
    // The EXIF item: a 4-byte offset to the TIFF header, then the data.
    let exif = heif
        .items
        .iter()
        .find(|(_, t)| t == b"Exif")
        .and_then(|&(id, _)| heif.item_data(id).ok());
    if let Some(e) = exif {
        let skip = be32(&e, 0).map_or(0, |s| s as usize);
        if let Some(tiff) = e.get(4usize.saturating_add(skip)..) {
            f.camera = crate::facts::camera_from_exif(tiff);
        }
    }
    f
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// One decoded plane set in a common sample representation.
struct Planes {
    w: usize,
    h: usize,
    bits: u32,
    /// Chroma subsampling shifts (x, y); None for monochrome.
    sub: Option<(u32, u32)>,
    y: Vec<u16>,
    u: Vec<u16>,
    v: Vec<u16>,
    cicp: (u16, u16, u16, bool),
}

fn av1_decode(data: Vec<u8>, cancel: &CancelToken) -> Result<rav1d::Picture> {
    if data.is_empty() {
        return Err(corrupt(FORMAT, "empty AV1 item"));
    }
    let mut s = rav1d::Settings::new();
    s.set_n_threads(1);
    s.set_max_frame_delay(1);
    s.set_frame_size_limit(256 * 1024 * 1024);
    let mut dec = rav1d::Decoder::with_settings(&s).map_err(|e| corrupt(FORMAT, e))?;
    let mut sent = dec.send_data(data.into_boxed_slice(), None, None, None);
    for _ in 0..1_000_000 {
        cancel.check()?;
        match sent {
            Ok(()) | Err(rav1d::Rav1dError::TryAgain) => {}
            Err(e) => return Err(corrupt(FORMAT, e)),
        }
        match dec.get_picture() {
            Ok(p) => return Ok(p),
            Err(rav1d::Rav1dError::TryAgain) => {
                if matches!(sent, Err(rav1d::Rav1dError::TryAgain)) {
                    sent = dec.send_pending_data();
                } else {
                    return Err(corrupt(FORMAT, "AV1 stream produced no picture"));
                }
            }
            Err(e) => return Err(corrupt(FORMAT, e)),
        }
    }
    Err(corrupt(FORMAT, "AV1 decoder did not finish"))
}

fn planes(pic: &rav1d::Picture, nclx: Option<(u16, u16, u16, bool)>) -> Result<Planes> {
    use rav1d::{PixelLayout, PlanarImageComponent as C};
    let (w, h) = (pic.width() as usize, pic.height() as usize);
    let bits = pic.bits_per_component().map_or(8, |b| u32::from(b.0));
    let wide = pic.bit_depth() > 8;
    let sub = match pic.pixel_layout() {
        PixelLayout::I400 => None,
        PixelLayout::I420 => Some((1, 1)),
        PixelLayout::I422 => Some((1, 0)),
        PixelLayout::I444 => Some((0, 0)),
    };
    let read = |c: C, pw: usize, ph: usize| -> Result<Vec<u16>> {
        let stride = pic.stride(c) as usize;
        let plane = pic.plane(c);
        let bytes = if wide { 2 } else { 1 };
        let mut out = Vec::with_capacity(pw * ph);
        for row in 0..ph {
            let line = plane
                .get(row * stride..row * stride + pw * bytes)
                .ok_or_else(|| corrupt(FORMAT, "short picture plane"))?;
            if wide {
                out.extend(
                    line.chunks_exact(2)
                        .map(|b| u16::from_ne_bytes([b[0], b[1]])),
                );
            } else {
                out.extend(line.iter().map(|&b| u16::from(b)));
            }
        }
        Ok(out)
    };
    let y = read(C::Y, w, h)?;
    let (u, v) = match sub {
        Some((sx, sy)) => {
            let (cw, ch) = ((w + sx as usize) >> sx, (h + sy as usize) >> sy);
            (read(C::U, cw, ch)?, read(C::V, cw, ch)?)
        }
        None => (Vec::new(), Vec::new()),
    };
    // The container's color description wins; the bitstream's is the
    // fallback (read only then: reserved codes make its accessors fail).
    let cicp = match nclx {
        Some(c) => c,
        None => (
            pic.color_primaries() as u16,
            pic.transfer_characteristic() as u16,
            pic.matrix_coefficients() as u16,
            matches!(pic.color_range(), rav1d::pixel::YUVRange::Full),
        ),
    };
    Ok(Planes {
        w,
        h,
        bits,
        sub,
        y,
        u,
        v,
        cicp,
    })
}

/// Decode an `av01` or `grid` item to planes.
fn decode_item(heif: &Heif, id: u32, cancel: &CancelToken, depth: u32) -> Result<Planes> {
    let kind = heif
        .item_type(id)
        .ok_or_else(|| corrupt(FORMAT, "unknown item"))?;
    let nclx = heif.nclx(id);
    match &kind {
        b"av01" => {
            let pic = av1_decode(heif.item_data(id)?, cancel)?;
            planes(&pic, nclx)
        }
        b"grid" if depth == 0 => {
            let desc = heif.item_data(id)?;
            let mut r = Rd::new(&desc);
            let _version = r.u8()?;
            let flags = r.u8()?;
            let rows = usize::from(r.u8()?) + 1;
            let cols = usize::from(r.u8()?) + 1;
            let size = if flags & 1 != 0 { 4 } else { 2 };
            let ow = r.uint(size)? as usize;
            let oh = r.uint(size)? as usize;
            check_size(ow as u32, oh as u32, PixelFormat::Rgba8)?;
            let tiles = heif.refs_from(b"dimg", id);
            if tiles.len() != rows * cols {
                return Err(corrupt(FORMAT, "grid tile count mismatch"));
            }
            let decoded = decode_tiles(heif, &tiles, cancel, depth + 1);
            let mut out: Option<Planes> = None;
            for (i, t) in decoded.into_iter().enumerate() {
                cancel.check()?;
                let t = t?;
                let (tx, ty) = ((i % cols) * t.w, (i / cols) * t.h);
                let o = out.get_or_insert_with(|| {
                    let (sx, sy) = t.sub.unwrap_or((0, 0));
                    let (cw, ch) = ((ow + sx as usize) >> sx, (oh + sy as usize) >> sy);
                    let chroma = if t.sub.is_some() { cw * ch } else { 0 };
                    Planes {
                        w: ow,
                        h: oh,
                        bits: t.bits,
                        sub: t.sub,
                        y: vec![0; ow * oh],
                        u: vec![0; chroma],
                        v: vec![0; chroma],
                        cicp: nclx.unwrap_or(t.cicp),
                    }
                });
                if o.sub != t.sub || o.bits != t.bits {
                    return Err(corrupt(FORMAT, "grid tiles differ in format"));
                }
                blit(&t.y, t.w, t.h, &mut o.y, ow, oh, tx, ty);
                if let Some((sx, sy)) = t.sub {
                    let (tcw, tch) = ((t.w + sx as usize) >> sx, (t.h + sy as usize) >> sy);
                    let (ocw, och) = ((ow + sx as usize) >> sx, (oh + sy as usize) >> sy);
                    blit(&t.u, tcw, tch, &mut o.u, ocw, och, tx >> sx, ty >> sy);
                    blit(&t.v, tcw, tch, &mut o.v, ocw, och, tx >> sx, ty >> sy);
                }
            }
            out.ok_or_else(|| corrupt(FORMAT, "empty grid"))
        }
        other => Err(unsupported(
            FORMAT,
            format!("item type {}", String::from_utf8_lossy(other)),
        )),
    }
}

/// Decode grid tiles, several at a time on scoped threads. Each tile is an
/// independent AV1 stream, so this parallelizes large grid images (phones
/// and cameras write them) without sharing decoder state. A panic in a tile
/// resurfaces here when the scope ends.
fn decode_tiles(
    heif: &Heif,
    tiles: &[u32],
    cancel: &CancelToken,
    depth: u32,
) -> Vec<Result<Planes>> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get().min(8))
        .min(tiles.len());
    if threads <= 1 {
        return tiles
            .iter()
            .map(|&t| decode_item(heif, t, cancel, depth))
            .collect();
    }
    let slots: Vec<Mutex<Option<Result<Planes>>>> =
        tiles.iter().map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= tiles.len() || cancel.is_cancelled() {
                    break;
                }
                let r = decode_item(heif, tiles[i], cancel, depth);
                if let Ok(mut slot) = slots[i].lock() {
                    *slot = Some(r);
                }
            });
        }
    });
    slots
        .into_iter()
        .map(|m| {
            m.into_inner()
                .ok()
                .flatten()
                .unwrap_or(Err(pandite_core::Error::Cancelled))
        })
        .collect()
}

/// Copy a tile into a larger plane, clipped.
#[allow(clippy::too_many_arguments)]
fn blit(
    src: &[u16],
    sw: usize,
    sh: usize,
    dst: &mut [u16],
    dw: usize,
    dh: usize,
    x: usize,
    y: usize,
) {
    if x >= dw || y >= dh {
        return;
    }
    let cols = sw.min(dw - x);
    for row in 0..sh.min(dh - y) {
        dst[(y + row) * dw + x..(y + row) * dw + x + cols]
            .copy_from_slice(&src[row * sw..row * sw + cols]);
    }
}

/// Kr and Kb for a matrix coefficients code.
fn kr_kb(matrix: u16) -> (f32, f32) {
    match matrix {
        1 => (0.2126, 0.0722),
        4 => (0.30, 0.11),
        7 => (0.212, 0.087),
        9 | 10 => (0.2627, 0.0593),
        _ => (0.299, 0.114),
    }
}

/// Convert planes to normalized RGB (0..1, nonlinear) and hand each pixel
/// (index, RGB) to `emit`, row by row.
fn for_each_rgb(p: &Planes, mut emit: impl FnMut(usize, [f32; 3])) {
    let (_, _, matrix, full) = p.cicp;
    let max = ((1u32 << p.bits) - 1) as f32;
    let scale = (1u32 << (p.bits - 8)) as f32;
    let (y_off, y_rng, c_rng) = if full {
        (0.0, max, max)
    } else {
        (16.0 * scale, 219.0 * scale, 224.0 * scale)
    };
    let mid = (1u32 << (p.bits - 1)) as f32;
    let (kr, kb) = kr_kb(matrix);
    let kg = 1.0 - kr - kb;
    let (cr_r, cb_g, cr_g, cb_b) = (
        2.0 * (1.0 - kr),
        2.0 * kb * (1.0 - kb) / kg,
        2.0 * kr * (1.0 - kr) / kg,
        2.0 * (1.0 - kb),
    );
    let Some((sx, sy)) = p.sub else {
        for (i, &v) in p.y.iter().enumerate() {
            let y = (f32::from(v) - y_off) / y_rng;
            emit(i, [y, y, y]);
        }
        return;
    };
    let (cw, ch) = ((p.w + sx as usize) >> sx, (p.h + sy as usize) >> sy);
    // Per-column chroma taps, shared by every row.
    let xtaps: Vec<(usize, usize, f32)> = (0..p.w).map(|x| taps(x, sx, cw)).collect();
    let mut urow = vec![0f32; cw];
    let mut vrow = vec![0f32; cw];
    for yy in 0..p.h {
        // Interpolate the two nearest chroma rows once per luma row.
        let (r0, r1, wy) = taps(yy, sy, ch);
        for c in 0..cw {
            let mix = |plane: &[u16]| {
                let a = f32::from(plane[r0 * cw + c]);
                let b = f32::from(plane[r1 * cw + c]);
                a + (b - a) * wy
            };
            urow[c] = mix(&p.u);
            vrow[c] = mix(&p.v);
        }
        for (xx, &(c0, c1, wx)) in xtaps.iter().enumerate() {
            let y = (f32::from(p.y[yy * p.w + xx]) - y_off) / y_rng;
            let u = urow[c0] + (urow[c1] - urow[c0]) * wx;
            let v = vrow[c0] + (vrow[c1] - vrow[c0]) * wx;
            let rgb = if matrix == 0 {
                // Identity: the planes hold G, B, R.
                let n = |s: f32| (s - y_off) / y_rng;
                [n(v), y, n(u)]
            } else {
                let cb = (u - mid) / c_rng;
                let cr = (v - mid) / c_rng;
                [y + cr_r * cr, y - cb_g * cb - cr_g * cr, y + cb_b * cb]
            };
            emit(yy * p.w + xx, rgb);
        }
    }
}

/// Chroma taps for luma position `i`: (first, second, weight of second).
/// Subsampled chroma is center-sited, so luma 2k mixes chroma k-1 and k
/// at 1:3 and luma 2k+1 mixes k and k+1 at 3:1.
fn taps(i: usize, sub: u32, n: usize) -> (usize, usize, f32) {
    if sub == 0 || n < 2 {
        let k = (i >> sub).min(n.saturating_sub(1));
        return (k, k, 0.0);
    }
    let k = (i >> 1).min(n - 1);
    if i & 1 == 0 {
        (k.saturating_sub(1), k, if k == 0 { 0.0 } else { 0.75 })
    } else {
        (k, (k + 1).min(n - 1), 0.25)
    }
}

/// PQ (SMPTE ST 2084) to linear, 203 nits = 1.0.
fn pq_to_linear(v: f32) -> f32 {
    let (m1, m2) = (0.159_301_76_f32, 78.843_75_f32);
    let (c1, c2, c3) = (0.835_937_5_f32, 18.851_562_f32, 18.687_5_f32);
    let p = v.clamp(0.0, 1.0).powf(1.0 / m2);
    10000.0 * ((p - c1).max(0.0) / (c2 - c3 * p)).powf(1.0 / m1) / 203.0
}

/// HLG (ARIB STD-B67) to linear on a 1000 nit display, 203 nits = 1.0.
fn hlg_to_linear(v: f32) -> f32 {
    let (a, b, c) = (0.178_832_77_f32, 0.284_668_92_f32, 0.559_910_7_f32);
    let v = v.clamp(0.0, 1.0);
    let scene = if v <= 0.5 {
        v * v / 3.0
    } else {
        (((v - c) / a).exp() + b) / 12.0
    };
    scene * 1000.0 / 203.0
}

/// Linear BT.2020 to linear BT.709 / sRGB primaries.
fn bt2020_to_srgb(c: [f32; 3]) -> [f32; 3] {
    [
        1.660_491 * c[0] - 0.587_641_1 * c[1] - 0.072_849_9 * c[2],
        -0.124_550_5 * c[0] + 1.132_9 * c[1] - 0.008_349_4 * c[2],
        -0.018_150_8 * c[0] - 0.100_578_9 * c[1] + 1.118_729_7 * c[2],
    ]
}

/// Linear Display P3 to linear sRGB primaries.
fn p3_to_srgb(c: [f32; 3]) -> [f32; 3] {
    [
        1.224_940_2 * c[0] - 0.224_940_2 * c[1],
        -0.042_056_95 * c[0] + 1.042_057 * c[1],
        -0.019_637_555 * c[0] - 0.078_636_04 * c[1] + 1.098_273_6 * c[2],
    ]
}

fn decode(data: &[u8], cancel: &CancelToken) -> Result<ImageBuf> {
    let heif = Heif::parse(data)?;
    let id = heif.primary;
    if let Some((w, h)) = heif.ispe(id) {
        check_size(w, h, PixelFormat::Rgba8)?;
    }
    let color = decode_item(&heif, id, cancel, 0)?;
    let alpha = match heif.alpha_for(id) {
        Some(a) => Some(decode_item(&heif, a, cancel, 0)?),
        None => None,
    };
    let premultiplied = heif
        .refs
        .iter()
        .any(|(k, from, _)| k == b"prem" && *from == id);
    cancel.check()?;

    let (w, h) = (color.w as u32, color.h as u32);
    check_size(w, h, PixelFormat::Rgba8)?;
    let n = color.w * color.h;
    let icc_profile = heif.icc(id);
    let (primaries, transfer, _, _) = color.cicp;
    // An ICC profile describes the colors; CICP primaries are then only
    // informative.
    let primaries = if icc_profile.is_some() { 1 } else { primaries };
    let alpha_at = |i: usize| -> f32 {
        match &alpha {
            Some(a) if a.w == color.w && a.h == color.h => {
                let max = ((1u32 << a.bits) - 1) as f32;
                let (_, _, _, full) = a.cicp;
                let v = f32::from(a.y[i]);
                let v = if full {
                    v / max
                } else {
                    let s = (1u32 << (a.bits - 8)) as f32;
                    (v - 16.0 * s) / (219.0 * s)
                };
                v.clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    };
    let unpremul = |c: f32, a: f32| if premultiplied && a > 0.0 { c / a } else { c };

    let image = if transfer == 16 || transfer == 18 {
        // HDR: linear float with sRGB primaries.
        let mut out = Vec::new();
        out.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
            width: w,
            height: h,
        })?;
        out.resize(n * 4, 0.0);
        for_each_rgb(&color, |i, c| {
            let a = alpha_at(i);
            let f = |v: f32| {
                let v = unpremul(v, a);
                if transfer == 16 {
                    pq_to_linear(v)
                } else {
                    hlg_to_linear(v)
                }
            };
            let lin = [f(c[0]), f(c[1]), f(c[2])];
            let lin = match primaries {
                9 => bt2020_to_srgb(lin),
                12 => p3_to_srgb(lin),
                _ => lin,
            };
            out[i * 4..i * 4 + 4].copy_from_slice(&[lin[0], lin[1], lin[2], a]);
        });
        util::rgbaf32_image(w, h, &out)?
    } else if color.bits > 8 {
        let mut out = Vec::new();
        out.try_reserve_exact(n * 4).map_err(|_| Error::TooLarge {
            width: w,
            height: h,
        })?;
        out.resize(n * 4, 0u16);
        let q = |v: f32| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
        for_each_rgb(&color, |i, c| {
            let a = alpha_at(i);
            let c = gamut(c, primaries);
            out[i * 4..i * 4 + 4].copy_from_slice(&[
                q(unpremul(c[0], a)),
                q(unpremul(c[1], a)),
                q(unpremul(c[2], a)),
                q(a),
            ]);
        });
        icc::to_srgb_rgba16(icc_profile, &mut out);
        util::rgba16_image(w, h, &out)?
    } else {
        let mut out = util::alloc_rgba8(w, h)?;
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        for_each_rgb(&color, |i, c| {
            let a = alpha_at(i);
            let c = gamut(c, primaries);
            out[i * 4..i * 4 + 4].copy_from_slice(&[
                q(unpremul(c[0], a)),
                q(unpremul(c[1], a)),
                q(unpremul(c[2], a)),
                q(a),
            ]);
        });
        icc::to_srgb_rgba8(icc_profile, &mut out);
        util::rgba8_image(w, h, out)?
    };

    // Clean aperture, then rotation and mirror.
    let image = match heif.clap_rect(id, w, h) {
        Some((x, y, cw, ch)) if (cw, ch) != (w, h) => crop(&image, x, y, cw, ch)?,
        _ => image,
    };
    orient::apply(image, heif.orientation(id))
}

/// Convert SDR (sRGB-like transfer) RGB with wide primaries to sRGB
/// primaries. BT.709 and unspecified primaries pass through.
fn gamut(c: [f32; 3], primaries: u16) -> [f32; 3] {
    let convert: fn([f32; 3]) -> [f32; 3] = match primaries {
        9 => bt2020_to_srgb,
        12 => p3_to_srgb,
        _ => return c,
    };
    let lin = c.map(|v| util::srgb_to_linear(v.clamp(0.0, 1.0)));
    convert(lin).map(util::linear_to_srgb)
}

/// Copy a sub-rectangle of an image.
fn crop(img: &ImageBuf, x: u32, y: u32, w: u32, h: u32) -> Result<ImageBuf> {
    let bpp = img.format().bytes_per_pixel();
    let mut out = Vec::with_capacity(w as usize * h as usize * bpp);
    for row in y..y + h {
        let r = img.row(row);
        out.extend_from_slice(&r[x as usize * bpp..(x + w) as usize * bpp]);
    }
    ImageBuf::new(w, h, img.format(), out)
}

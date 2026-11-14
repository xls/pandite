//! Camera RAW decoder.
//!
//! DNG files are developed from sensor data: uncompressed or lossless JPEG
//! tiles and strips, CFA (any repeat pattern) or linear RGB, linearization
//! table, black and white levels, active area and default crop, as-shot
//! white balance, the color matrix to sRGB, baseline exposure and the
//! orientation tag. The developed image gets pandite's default display
//! rendering ([`tone`]: exposure normalization and a hue-preserving filmic
//! curve) and is delivered as 16-bit sRGB, or as 8-bit sRGB when the
//! caller prefers 8 bits (display).
//!
//! Sony ARW files are developed from sensor data by the [`arw`] module,
//! written from the clean-room specification `specs/raw/arw.md`, with the
//! same rendering and output. Its out-of-scope variants show their preview.
//!
//! The other proprietary formats (CR2, CR3, NEF, RAF, ORF, RW2, PEF, SRW)
//! show the camera's embedded JPEG preview, the largest one in the file
//! (full resolution on most cameras). DNG files whose sensor data uses an
//! unsupported compression fall back to their preview as well.
//!
//! With `DecodeOptions::target_size`, the smallest preview that still
//! covers the target is used, which makes thumbnails fast.

mod arw;
mod pef;
pub(crate) mod tone;

use crate::exif::{tag, Entry, Tiff};
use crate::util::{self, check_size, corrupt, guard, unsupported};
use crate::{jpeg, ljpeg, orient};
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageFacts,
    ImageMeta, Orientation, PixelFormat, Result, Source,
};

const FORMAT: &str = "raw";
const EXTENSIONS: &[&str] = &[
    "dng", "cr2", "cr3", "nef", "arw", "raf", "orf", "rw2", "pef", "srw",
];

/// Width, height and orientation of an image.
type Probed = (u32, u32, Orientation);

/// A RAW format that its own module develops from sensor data. The
/// functions return `None` (or `Ok(None)`) when the data are not a
/// container the module reads; the provider then uses its generic preview
/// search.
pub(crate) struct Developer {
    /// The format's display name, for example "Sony ARW".
    #[cfg_attr(not(test), allow(dead_code))]
    name: &'static str,
    /// True when the module reads the data: by their structure, or by the
    /// lowercase file extension when one is given.
    detect: fn(&[u8], Option<&str>) -> bool,
    /// True when the file carries an embedded preview.
    has_preview: fn(&[u8]) -> bool,
    /// Width, height and orientation of the image `decode` delivers at
    /// full size.
    probe: fn(&[u8]) -> Result<Option<Probed>>,
    /// The upright image.
    decode: fn(&[u8], &DecodeOptions) -> Result<Option<ImageBuf>>,
    /// Header facts.
    facts: fn(&[u8]) -> Option<ImageFacts>,
}

/// The formats developed from sensor data, tried in order. To add a
/// format, declare its module (`mod <fmt>;`) and add `&<fmt>::DEVELOPER`
/// to this table; nothing else in this file changes.
const DEVELOPERS: &[&Developer] = &[&arw::DEVELOPER, &pef::DEVELOPER];

/// Decodes camera RAW files (DNG from sensor data, other formats through
/// their embedded preview).
#[derive(Clone, Copy, Debug, Default)]
pub struct RawDecoder;

/// True for signatures that only RAW files carry.
fn distinctive_signature(h: &[u8]) -> bool {
    h.starts_with(b"FUJIFILMCCD-RAW")
        || (h.get(4..12) == Some(b"ftypcrx "))
        || h.starts_with(b"IIRO")
        || h.starts_with(b"IIRS")
        || h.starts_with(b"MMOR")
        || h.starts_with(b"IIU\0")
        || (h.starts_with(b"II*\0") && h.get(8..12) == Some(b"CR\x02\0"))
}

/// True when the source is a camera RAW file rather than a plain TIFF:
/// a RAW extension, a RAW-only signature, a DNG version tag, or sensor
/// (CFA or linear raw) data in any directory.
pub(crate) fn is_camera_raw(source: &Source) -> bool {
    if source
        .extension()
        .is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
    {
        return true;
    }
    let d = source.bytes();
    if distinctive_signature(d) || DEVELOPERS.iter().any(|dev| (dev.detect)(d, None)) {
        return true;
    }
    let Some(t) = Tiff::new(d) else {
        return false;
    };
    let mut raw = false;
    t.walk(|e| {
        raw |= e.iter().any(|x| x.tag == tag::DNG_VERSION)
            || matches!(t.get(e, tag::PHOTOMETRIC), Some(32803 | 34892));
    });
    raw
}

impl DecoderProvider for RawDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "Camera RAW image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        EXTENSIONS
    }

    fn sniff(&self, header: &[u8]) -> bool {
        // TIFF-structured RAW files without a distinctive signature reach
        // this provider through the TIFF provider, which hands them over.
        #[cfg(not(feature = "tiff"))]
        if crate::exif::Tiff::new(header).is_some() {
            return true;
        }
        distinctive_signature(header)
    }

    fn reduced_decode(&self, source: &Source) -> bool {
        // Reduced images come from embedded previews; a file without one
        // would be developed in full.
        let d = source.bytes();
        if let Some(dev) = developer(source) {
            return (dev.has_preview)(d);
        }
        !previews(d).is_empty()
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let d = source.bytes();
            if let Some(dev) = developer(source) {
                if let Some((w, h, orientation)) = (dev.probe)(d)? {
                    return Ok(ImageMeta {
                        width: w,
                        height: h,
                        format: FORMAT,
                        kind: ContentKind::Still,
                        frame_count: 1,
                        has_alpha: false,
                        orientation,
                    });
                }
            }
            let orientation = container_orientation(d);
            let (w, h) = match dng_info(d) {
                Some(info) => info.output_size(),
                None => {
                    let p = previews(d)
                        .into_iter()
                        .max_by_key(|p| p.area())
                        .ok_or_else(|| corrupt(FORMAT, "no decodable image"))?;
                    (p.width, p.height)
                }
            };
            let (width, height) = jpeg::oriented(w, h, orientation);
            Ok(ImageMeta {
                width,
                height,
                format: FORMAT,
                kind: ContentKind::Still,
                frame_count: 1,
                has_alpha: false,
                orientation,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || {
            if let Some(dev) = developer(source) {
                if let Some(img) = (dev.decode)(source.bytes(), options)? {
                    return Ok(img);
                }
            }
            decode(source.bytes(), options)
        })
        .map(Content::Still)
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        guard(FORMAT, || {
            if let Some(dev) = developer(source) {
                if let Some(f) = (dev.facts)(source.bytes()) {
                    return Ok(f);
                }
            }
            Ok(other_facts(source))
        })
    }
}

/// The maker's name of a RAW container, from its signature or extension.
fn container_name(source: &Source) -> &'static str {
    let d = source.bytes();
    if d.starts_with(b"FUJIFILMCCD-RAW") {
        return "Fujifilm RAF";
    }
    if d.get(4..12) == Some(b"ftypcrx ") {
        return "Canon CR3";
    }
    if d.starts_with(b"IIRO") || d.starts_with(b"IIRS") || d.starts_with(b"MMOR") {
        return "Olympus ORF";
    }
    if d.starts_with(b"IIU\0") {
        return "Panasonic RW2";
    }
    if d.starts_with(b"II*\0") && d.get(8..12) == Some(b"CR\x02\0") {
        return "Canon CR2";
    }
    match source.extension().as_deref() {
        Some("dng") => "DNG",
        Some("nef") => "Nikon NEF",
        Some("pef") => "Pentax PEF",
        Some("srw") => "Samsung SRW",
        Some("arw") => "Sony ARW",
        Some("cr2") => "Canon CR2",
        Some("orf") => "Olympus ORF",
        Some("rw2") => "Panasonic RW2",
        Some("raf") => "Fujifilm RAF",
        Some("cr3") => "Canon CR3",
        _ => "Camera RAW",
    }
}

/// Facts of DNG and the RAW formats shown through their previews: the
/// container name, the DNG sensor data layout, and the camera settings
/// (from the TIFF structure, the CR3 CMT1 box or the RAF preview's EXIF).
fn other_facts(source: &Source) -> ImageFacts {
    let d = source.bytes();
    let mut f = ImageFacts::named(container_name(source));
    if let Some(t) = Tiff::new(d) {
        f.camera = crate::facts::camera(&t);
    } else if d.get(4..12) == Some(b"ftypcrx ") {
        let cmt1 = d.windows(4).take(1 << 20).position(|w| w == b"CMT1");
        if let Some(t) = cmt1.and_then(|p| d.get(p + 4..)).and_then(Tiff::new) {
            f.camera = crate::facts::camera(&t);
        }
    } else if d.starts_with(b"FUJIFILMCCD-RAW") {
        // The header names the embedded JPEG: offset and length at 84.
        let preview = util::be32(d, 84).and_then(|o| d.get(o as usize..));
        if let Some(jpeg) = preview.and_then(jpeg::header_facts) {
            f.camera = jpeg.camera;
        }
    }
    let Some(info) = dng_info(d) else {
        return f;
    };
    f.container = Some("DNG".to_string());
    f.compression = match info.compression {
        1 => Some("Uncompressed"),
        7 => Some("Lossless JPEG"),
        34892 => Some("Lossy JPEG"),
        52546 => Some("JPEG XL"),
        _ => None,
    }
    .map(str::to_string);
    f.bits_per_channel = u8::try_from(info.bits).ok().filter(|&b| b > 0);
    if info.cfa {
        let names = ["R", "G", "B", "C", "M", "Y"];
        let pattern: Option<String> = (info.pattern_dim == (2, 2) && info.pattern.len() == 4)
            .then(|| {
                info.pattern
                    .iter()
                    .map(|&c| names.get(usize::from(c)).copied())
                    .collect()
            })
            .flatten();
        f.channels = Some(match pattern {
            Some(p) => format!("Bayer {p}"),
            None => "Color filter array".to_string(),
        });
        f.channel_count = Some(1);
    } else {
        f.channels = Some("Linear RGB".to_string());
        f.channel_count = u8::try_from(info.spp).ok();
    }
    f
}

/// The developer that handles the source: the first entry of
/// [`DEVELOPERS`] whose detect function accepts the data and extension.
/// None for a DNG (a DNG version tag in IFD0), which the DNG path develops.
fn developer(source: &Source) -> Option<&'static Developer> {
    let d = source.bytes();
    let extension = source.extension();
    let found = DEVELOPERS
        .iter()
        .copied()
        .find(|dev| (dev.detect)(d, extension.as_deref()))?;
    let dng = Tiff::new(d)
        .and_then(|t| t.ifd(t.first_ifd()?))
        .is_some_and(|(ifd0, _)| ifd0.iter().any(|e| e.tag == tag::DNG_VERSION));
    (!dng).then_some(found)
}

/// One embedded JPEG preview.
#[derive(Clone, Copy, Debug)]
struct Preview {
    offset: usize,
    width: u32,
    height: u32,
}

impl Preview {
    fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Find embedded baseline or progressive JPEG streams by their start
/// markers. Lossless JPEG (the sensor data of some formats) is skipped.
fn previews(d: &[u8]) -> Vec<Preview> {
    let mut out: Vec<Preview> = Vec::new();
    let mut i = 0;
    let mut examined = 0;
    while i + 4 <= d.len() && examined < 256 {
        let Some(off) = d[i..].windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) else {
            break;
        };
        let at = i + off;
        examined += 1;
        if let Some(info) = sof(&d[at..]) {
            if !out.iter().any(|p| p.offset == at) {
                out.push(Preview {
                    offset: at,
                    width: info.0,
                    height: info.1,
                });
            }
        }
        i = at + 3;
    }
    out
}

/// Size of a JPEG stream when its frame is 8-bit baseline, extended or
/// progressive with 1 or 3 components.
fn sof(d: &[u8]) -> Option<(u32, u32)> {
    let mut pos = 2;
    for _ in 0..64 {
        while *d.get(pos)? == 0xFF && *d.get(pos + 1)? == 0xFF {
            pos += 1;
        }
        if *d.get(pos)? != 0xFF {
            return None;
        }
        let m = *d.get(pos + 1)?;
        let len = usize::from(util::be16(d, pos + 2)?);
        if len < 2 {
            return None;
        }
        match m {
            0xC0..=0xC2 => {
                let p = d.get(pos + 4..pos + 2 + len)?;
                let (prec, h, w, n) = (p[0], util::be16(p, 1)?, util::be16(p, 3)?, p[5]);
                return (prec == 8 && w > 0 && h > 0 && (n == 1 || n == 3))
                    .then_some((u32::from(w), u32::from(h)));
            }
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDA | 0xD9 => return None,
            _ => pos += 2 + len,
        }
    }
    None
}

/// The orientation stored by the RAW container.
fn container_orientation(d: &[u8]) -> Orientation {
    if let Some(t) = Tiff::new(d) {
        return t.orientation();
    }
    // CR3 keeps a TIFF IFD0 in its CMT1 box.
    if d.get(4..12) == Some(b"ftypcrx ") {
        if let Some(p) = d.windows(4).take(1 << 20).position(|w| w == b"CMT1") {
            if let Some(t) = d.get(p + 4..).and_then(Tiff::new) {
                return t.orientation();
            }
        }
    }
    Orientation::Normal
}

fn decode(d: &[u8], options: &DecodeOptions) -> Result<ImageBuf> {
    let orientation = container_orientation(d);
    let found = previews(d);
    let dng = dng_info(d);

    // A preview is enough when the caller wants a smaller image.
    if let Some(target) = options.target_size {
        // Previews are stored in sensor orientation, like the target here.
        let (tw, th) = jpeg::oriented(target.0, target.1, orientation);
        let (fw, _) = full_size(&dng, &found);
        let fits = |p: &Preview| {
            p.width >= tw
                || p.height >= th
                || (fw > 0 && u64::from(p.width) * 20 >= u64::from(fw) * 19)
        };
        if let Some(p) = found.iter().filter(|p| fits(p)).min_by_key(|p| p.area()) {
            if let Ok(img) = preview_image(d, p, orientation, options) {
                return Ok(img);
            }
        }
    }
    if let Some(info) = dng {
        match develop(d, &info, &options.cancel, options.prefer_8bit) {
            Ok(img) => return orient::apply(img, orientation),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e @ Error::TooLarge { .. }) => return Err(e),
            Err(e) if found.is_empty() => return Err(e),
            Err(_) => {}
        }
    }
    let mut by_size = found;
    by_size.sort_by_key(|p| std::cmp::Reverse(p.area()));
    let mut last = corrupt(FORMAT, "no decodable image");
    for p in &by_size {
        match preview_image(d, p, orientation, options) {
            Ok(img) => return Ok(img),
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn full_size(dng: &Option<Dng>, found: &[Preview]) -> (u32, u32) {
    match dng {
        Some(i) => i.output_size(),
        None => found
            .iter()
            .max_by_key(|p| p.area())
            .map_or((0, 0), |p| (p.width, p.height)),
    }
}

/// Decode a preview and orient it like the RAW (unless the preview carries
/// its own EXIF orientation).
fn preview_image(
    d: &[u8],
    p: &Preview,
    container: Orientation,
    options: &DecodeOptions,
) -> Result<ImageBuf> {
    let stream = &d[p.offset..];
    let own = jpeg::scan_header(stream)
        .and_then(|h| h.exif)
        .map(crate::exif::orientation_from_exif)
        .unwrap_or_default();
    // The JPEG decoder maps the target through its own orientation only.
    let target = options.target_size.map(|(w, h)| {
        if own == Orientation::Normal {
            jpeg::oriented(w, h, container)
        } else {
            (w, h)
        }
    });
    let opts = DecodeOptions {
        target_size: target,
        first_frame_only: true,
        prefer_8bit: true,
        cancel: options.cancel.clone(),
    };
    let img = jpeg::decode_image(stream, &opts)?;
    if own == Orientation::Normal {
        orient::apply(img, container)
    } else {
        Ok(img)
    }
}

// ---------------------------------------------------------------------------
// DNG
// ---------------------------------------------------------------------------

/// Everything needed to develop the DNG sensor image.
#[derive(Clone, Debug)]
struct Dng {
    le: bool,
    width: u32,
    height: u32,
    bits: u32,
    spp: usize,
    compression: u32,
    cfa: bool,
    pattern_dim: (usize, usize),
    pattern: Vec<u8>,
    /// (offset, length) of each strip or tile.
    chunks: Vec<(usize, usize)>,
    /// Tile size, or (width, rows per strip) for strips.
    tile: (usize, usize),
    tiled: bool,
    linearization: Vec<u16>,
    black_dim: (usize, usize),
    black: Vec<f32>,
    white: f32,
    active: (u32, u32, u32, u32),
    crop: Option<(f64, f64, f64, f64)>,
    color_matrix: Option<[f64; 9]>,
    neutral: Option<[f64; 3]>,
    baseline_exposure: f64,
    float: bool,
}

impl Dng {
    /// Size after the active area and default crop.
    fn output_size(&self) -> (u32, u32) {
        let (t, l, b, r) = self.active;
        let (aw, ah) = (r - l, b - t);
        match self.crop {
            Some((_, _, cw, ch)) => (
                (cw.round() as u32).clamp(1, aw.max(1)),
                (ch.round() as u32).clamp(1, ah.max(1)),
            ),
            None => (aw.max(1), ah.max(1)),
        }
    }
}

fn rational(t: &Tiff, e: &Entry, i: u32) -> Option<f64> {
    match e.typ {
        5 | 10 => {
            let at = e.value_at + 8 * i as usize;
            let (n, dd) = (t.u32(at)?, t.u32(at + 4)?);
            if dd == 0 {
                return None;
            }
            Some(if e.typ == 10 {
                f64::from(n as i32) / f64::from(dd as i32)
            } else {
                f64::from(n) / f64::from(dd)
            })
        }
        11 => {
            let b = t
                .data()
                .get(e.value_at + 4 * i as usize..e.value_at + 4 * i as usize + 4)?;
            let v = if t.u16(0) == Some(0x4949) {
                f32::from_le_bytes([b[0], b[1], b[2], b[3]])
            } else {
                f32::from_be_bytes([b[0], b[1], b[2], b[3]])
            };
            Some(f64::from(v))
        }
        _ => t.value(e, i).map(f64::from),
    }
}

fn reals(t: &Tiff, entries: &[Entry], tag: u16, limit: u32) -> Vec<f64> {
    entries
        .iter()
        .find(|e| e.tag == tag)
        .map(|e| {
            (0..e.count.min(limit))
                .filter_map(|i| rational(t, e, i))
                .collect()
        })
        .unwrap_or_default()
}

/// Read the DNG raw image description, or None for non-DNG files.
fn dng_info(d: &[u8]) -> Option<Dng> {
    let t = Tiff::new(d)?;
    let first = t.first_ifd()?;
    let (ifd0, _) = t.ifd(first)?;
    if !ifd0.iter().any(|e| e.tag == tag::DNG_VERSION) {
        return None;
    }
    // The main sensor image: NewSubFileType 0 with CFA or linear raw data.
    let mut best: Option<Vec<Entry>> = None;
    t.walk(|e| {
        let photometric = t.get(e, tag::PHOTOMETRIC);
        let subfile = t.get(e, tag::NEW_SUBFILE_TYPE).unwrap_or(0);
        if subfile == 0 && matches!(photometric, Some(32803 | 34892)) {
            let area = |x: &[Entry]| {
                u64::from(t.get(x, tag::IMAGE_WIDTH).unwrap_or(0))
                    * u64::from(t.get(x, tag::IMAGE_LENGTH).unwrap_or(0))
            };
            if best.as_ref().is_none_or(|b| area(e) > area(b)) {
                best = Some(e.to_vec());
            }
        }
    });
    let e = best?;
    let width = t.get(&e, tag::IMAGE_WIDTH)?;
    let height = t.get(&e, tag::IMAGE_LENGTH)?;
    let spp = t.get(&e, 277).unwrap_or(1) as usize;
    let cfa = t.get(&e, tag::PHOTOMETRIC) == Some(32803);
    let bits = t.get(&e, 258).unwrap_or(16);
    let tiled = e.iter().any(|x| x.tag == 324);
    let (offsets, counts, tile) = if tiled {
        (
            t.values(&e, 324, 1 << 20),
            t.values(&e, 325, 1 << 20),
            (
                t.get(&e, 322).unwrap_or(width) as usize,
                t.get(&e, 323).unwrap_or(height) as usize,
            ),
        )
    } else {
        (
            t.values(&e, tag::STRIP_OFFSETS, 1 << 20),
            t.values(&e, tag::STRIP_BYTE_COUNTS, 1 << 20),
            (width as usize, t.get(&e, 278).unwrap_or(height) as usize),
        )
    };
    let chunks = offsets
        .iter()
        .zip(&counts)
        .map(|(&o, &c)| (o as usize, c as usize))
        .collect();
    let dim = t.values(&e, 33421, 2);
    let pattern_dim = if dim.len() == 2 {
        (dim[0] as usize, dim[1] as usize)
    } else {
        (2, 2)
    };
    let pattern = e
        .iter()
        .find(|x| x.tag == 33422)
        .and_then(|x| t.bytes(x))
        .map(<[u8]>::to_vec)
        .unwrap_or_else(|| vec![0, 1, 1, 2]);
    let bdim = t.values(&e, 50713, 2);
    let black_dim = if bdim.len() == 2 {
        (bdim[0].max(1) as usize, bdim[1].max(1) as usize)
    } else {
        (1, 1)
    };
    let black: Vec<f32> = reals(&t, &e, 50714, 64).iter().map(|&v| v as f32).collect();
    let white = t
        .get(&e, 50717)
        .map_or(((1u64 << bits.min(16)) - 1) as f32, |v| v as f32);
    let active = {
        let a = t.values(&e, 50829, 4);
        if a.len() == 4 && a[2] > a[0] && a[3] > a[1] && a[2] <= height && a[3] <= width {
            (a[0], a[1], a[2], a[3])
        } else {
            (0, 0, height, width)
        }
    };
    let origin = reals(&t, &e, 50719, 2);
    let size = reals(&t, &e, 50720, 2);
    let crop =
        (origin.len() == 2 && size.len() == 2).then(|| (origin[0], origin[1], size[0], size[1]));
    // Color data lives in IFD0.
    let m1 = reals(&t, &ifd0, 50721, 9);
    let m2 = reals(&t, &ifd0, 50722, 9);
    let ill2 = t.get(&ifd0, 50779);
    let pick = if m2.len() == 9 && (ill2 == Some(21) || m1.len() != 9) {
        m2
    } else {
        m1
    };
    let color_matrix = (pick.len() == 9).then(|| {
        let mut m = [0.0; 9];
        m.copy_from_slice(&pick);
        m
    });
    let n = reals(&t, &ifd0, 50728, 3);
    let neutral = (n.len() == 3 && n.iter().all(|&v| v > 0.0)).then(|| [n[0], n[1], n[2]]);
    let baseline_exposure = reals(&t, &ifd0, 50730, 1).first().copied().unwrap_or(0.0);
    let linearization = t
        .values(&e, 50712, 1 << 16)
        .into_iter()
        .map(|v| v.min(65535) as u16)
        .collect();
    let float = t.get(&e, 339) == Some(3);
    Some(Dng {
        le: t.u16(0) == Some(0x4949),
        width,
        height,
        bits,
        spp,
        compression: t.get(&e, tag::COMPRESSION).unwrap_or(1),
        cfa,
        pattern_dim,
        pattern,
        chunks,
        tile,
        tiled,
        linearization,
        black_dim,
        black,
        white,
        active,
        crop,
        color_matrix,
        neutral,
        baseline_exposure,
        float,
    })
}

/// Read every strip or tile into one plane of `spp` samples per pixel.
fn read_sensor(d: &[u8], i: &Dng, cancel: &CancelToken) -> Result<Vec<u16>> {
    let (w, h, spp) = (i.width as usize, i.height as usize, i.spp);
    if i.float {
        return Err(unsupported(FORMAT, "floating point DNG"));
    }
    if spp == 0 || spp > 4 || (i.cfa && spp != 1) {
        return Err(unsupported(FORMAT, "sample layout"));
    }
    let len = w
        .checked_mul(h)
        .and_then(|v| v.checked_mul(spp))
        .ok_or(Error::TooLarge {
            width: i.width,
            height: i.height,
        })?;
    let mut out: Vec<u16> = Vec::new();
    out.try_reserve_exact(len).map_err(|_| Error::TooLarge {
        width: i.width,
        height: i.height,
    })?;
    out.resize(len, 0);
    let (tw, th) = (i.tile.0.max(1), i.tile.1.max(1));
    let across = if i.tiled { w.div_ceil(tw) } else { 1 };
    for (k, &(off, count)) in i.chunks.iter().enumerate() {
        cancel.check()?;
        let src = d
            .get(off..off.saturating_add(count))
            .ok_or_else(|| corrupt(FORMAT, "sensor data outside the file"))?;
        let (x0, y0) = if i.tiled {
            ((k % across) * tw, (k / across) * th)
        } else {
            (0, k * th)
        };
        if y0 >= h {
            break;
        }
        let cw = if i.tiled { tw } else { w };
        let samples: Vec<u16> = match i.compression {
            1 => unpack(src, cw * th * spp, i.bits, i.le)?,
            7 => {
                let lj = ljpeg::decode(src)?;
                // DNG packs tile rows into the lossless JPEG frame in
                // several shapes; only the sample count must agree.
                let rows = if i.tiled { th } else { th.min(h - y0) };
                if lj.width * lj.height * lj.components < cw * rows * spp {
                    return Err(corrupt(FORMAT, "lossless JPEG tile too small"));
                }
                lj.samples
            }
            c => return Err(unsupported(FORMAT, format!("DNG compression {c}"))),
        };
        // Place the chunk; its rows are `cw * spp` samples long.
        let row = cw * spp;
        for r in 0..th.min(h - y0) {
            let s = r * row;
            if s >= samples.len() {
                break;
            }
            let cols = (w - x0).min(cw) * spp;
            let take = cols.min(samples.len() - s);
            let o = ((y0 + r) * w + x0) * spp;
            out[o..o + take].copy_from_slice(&samples[s..s + take]);
        }
    }
    Ok(out)
}

/// Unpack uncompressed samples: 8 and 16 bits in file byte order, other
/// depths as a big-endian bit stream.
fn unpack(src: &[u8], count: usize, bits: u32, le: bool) -> Result<Vec<u16>> {
    match bits {
        8 => Ok(src.iter().take(count).map(|&b| u16::from(b)).collect()),
        16 => Ok(src
            .chunks_exact(2)
            .take(count)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect()),
        b if (1..16).contains(&b) => {
            let mut out = Vec::with_capacity(count);
            let (mut acc, mut n, mut i) = (0u32, 0u32, 0usize);
            while out.len() < count && i < src.len() {
                acc = (acc << 8) | u32::from(src[i]);
                n += 8;
                i += 1;
                while n >= b && out.len() < count {
                    n -= b;
                    out.push(((acc >> n) & ((1 << b) - 1)) as u16);
                }
            }
            Ok(out)
        }
        b => Err(unsupported(FORMAT, format!("{b}-bit samples"))),
    }
}

/// Invert a 3 x 3 matrix.
fn invert(m: [f64; 9]) -> Option<[f64; 9]> {
    let det = m[0] * (m[4] * m[8] - m[5] * m[7]) - m[1] * (m[3] * m[8] - m[5] * m[6])
        + m[2] * (m[3] * m[7] - m[4] * m[6]);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    Some(inv.map(|v| v / det))
}

fn mul(a: [f64; 9], b: [f64; 9]) -> [f64; 9] {
    let mut o = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            o[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    o
}

/// Camera RGB (white balanced) to linear sRGB, from the XYZ to camera
/// matrix. Rows are normalized so neutral stays neutral.
fn camera_to_srgb(xyz_to_cam: [f64; 9]) -> Option<[f64; 9]> {
    // Linear sRGB (D65) to XYZ.
    const SRGB_TO_XYZ: [f64; 9] = [
        0.412_456_4,
        0.357_576_1,
        0.180_437_5,
        0.212_672_9,
        0.715_152_2,
        0.072_175,
        0.019_333_9,
        0.119_192,
        0.950_304_1,
    ];
    let mut cam_rgb = mul(xyz_to_cam, SRGB_TO_XYZ);
    for r in 0..3 {
        let sum: f64 = cam_rgb[r * 3..r * 3 + 3].iter().sum();
        if sum.abs() < 1e-9 {
            return None;
        }
        for c in 0..3 {
            cam_rgb[r * 3 + c] /= sum;
        }
    }
    invert(cam_rgb)
}

/// Develop the DNG sensor image in one pass over the output area:
/// normalize, demosaic, white balance, color matrix, then the default
/// display rendering ([`tone`], with the file's baseline exposure as its
/// exposure gain) and sRGB encoding per pixel, so only the sensor data and
/// the result are held in memory. `eight_bit` delivers `Rgba8`, else
/// `Rgba16`.
fn develop(d: &[u8], i: &Dng, cancel: &CancelToken, eight_bit: bool) -> Result<ImageBuf> {
    let format = if eight_bit {
        PixelFormat::Rgba8
    } else {
        PixelFormat::Rgba16
    };
    check_size(i.width, i.height, PixelFormat::Rgba16)?;
    let (ow, oh) = i.output_size();
    check_size(ow, oh, format)?;
    let raw = read_sensor(d, i, cancel)?;
    let (w, spp) = (i.width as usize, i.spp);
    let (top, left, bottom, right) = (
        i.active.0 as usize,
        i.active.1 as usize,
        i.active.2 as usize,
        i.active.3 as usize,
    );
    let (aw, ah) = (right - left, bottom - top);

    // Normalized (0..1) sample `s` at active-area position (y, x).
    let lut: Option<&[u16]> = (!i.linearization.is_empty()).then_some(&i.linearization[..]);
    let (br, bc) = i.black_dim;
    let norm = |y: usize, x: usize, s: usize| -> f32 {
        let v = raw[((y + top) * w + x + left) * spp + s];
        let v = match lut {
            Some(t) => f32::from(t[usize::from(v).min(t.len() - 1)]),
            None => f32::from(v),
        };
        let b = if i.black.is_empty() {
            0.0
        } else {
            let idx = ((y % br) * bc + (x % bc)) * spp + s;
            i.black.get(idx).or(i.black.first()).copied().unwrap_or(0.0)
        };
        ((v - b) / (i.white - b).max(1.0)).clamp(0.0, 1.0)
    };
    let (pr, pc) = (i.pattern_dim.0.max(1), i.pattern_dim.1.max(1));
    let color = |r: usize, c: usize| -> usize {
        usize::from(*i.pattern.get((r % pr) * pc + (c % pc)).unwrap_or(&1)).min(2)
    };
    // Camera RGB at a pixel: demosaiced for CFA data.
    let camera = |y: usize, x: usize| -> [f32; 3] {
        if !i.cfa {
            return [0, 1, 2].map(|c| norm(y, x, c.min(spp - 1)));
        }
        let mut sum = [0f32; 3];
        let mut cnt = [0u32; 3];
        let own = color(y, x);
        sum[own] = norm(y, x, 0);
        cnt[own] = 1;
        // Average the nearest samples of each missing color: the 3 x 3
        // neighborhood covers Bayer, 5 x 5 covers X-Trans.
        for radius in [1usize, 2] {
            let missing = cnt.map(|c| c == 0);
            if !missing.contains(&true) {
                break;
            }
            for yy in y.saturating_sub(radius)..=(y + radius).min(ah - 1) {
                for xx in x.saturating_sub(radius)..=(x + radius).min(aw - 1) {
                    let c = color(yy, xx);
                    if missing[c] {
                        sum[c] += norm(yy, xx, 0);
                        cnt[c] += 1;
                    }
                }
            }
        }
        [0, 1, 2].map(|c| {
            if cnt[c] > 0 {
                sum[c] / cnt[c] as f32
            } else {
                0.0
            }
        })
    };

    let neutral = i.neutral.unwrap_or([1.0, 1.0, 1.0]);
    let wb = neutral.map(|n| (1.0 / n) as f32);
    let matrix = i
        .color_matrix
        .and_then(camera_to_srgb)
        .map(|m| m.map(|v| v as f32));
    // BaselineExposure moves the exposure: the tone curve maps clipping
    // times this gain to white.
    let gain = 2f64.powf(i.baseline_exposure.clamp(-10.0, 10.0));
    let rendering = tone::Tone::new(gain);
    // Linear 0..1 (16-bit steps) to 16-bit sRGB codes.
    let encode: Vec<u16> = if eight_bit {
        Vec::new()
    } else {
        (0..=u16::MAX)
            .map(|v| (util::linear_to_srgb(f32::from(v) / 65535.0) * 65535.0 + 0.5) as u16)
            .collect()
    };
    let srgb8 = tone::srgb8_table();
    let bpp = format.bytes_per_pixel();
    let (cx, cy, cw, ch) = match i.crop {
        Some((x, y, _, _)) => (
            (x.round().max(0.0) as usize).min(aw - 1),
            (y.round().max(0.0) as usize).min(ah - 1),
            ow as usize,
            oh as usize,
        ),
        None => (0, 0, aw, ah),
    };
    let (cw, ch) = (cw.min(aw - cx), ch.min(ah - cy));
    let len = check_size(cw as u32, ch as u32, format)?;
    let mut out = util::alloc(len, cw as u32, ch as u32)?;
    for (row, y) in (cy..cy + ch).enumerate() {
        if row % 128 == 0 {
            cancel.check()?;
        }
        let line = &mut out[row * cw * bpp..(row + 1) * cw * bpp];
        for (px, x) in line.chunks_exact_mut(bpp).zip(cx..cx + cw) {
            let p = camera(y, x);
            // Clip after white balance so highlights stay neutral.
            let c = [0, 1, 2].map(|k| (p[k] * wb[k]).min(1.0));
            let rgb = match matrix {
                Some(m) => {
                    [0, 1, 2].map(|r| m[r * 3] * c[0] + m[r * 3 + 1] * c[1] + m[r * 3 + 2] * c[2])
                }
                None => c,
            };
            let lin = rgb.map(|v| v.clamp(0.0, 1.0));
            let shown = rendering.apply(lin);
            if eight_bit {
                for k in 0..3 {
                    px[k] = srgb8[tone::srgb8_index(shown[k])];
                }
                px[3] = 255;
            } else {
                for (k, v) in shown.into_iter().enumerate() {
                    let code = encode[(v.clamp(0.0, 1.0) * 65535.0 + 0.5) as usize];
                    px[2 * k..2 * k + 2].copy_from_slice(&code.to_ne_bytes());
                }
                px[6..8].copy_from_slice(&u16::MAX.to_ne_bytes());
            }
        }
    }
    ImageBuf::new(cw as u32, ch as u32, format, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn developer_names_are_distinct() {
        for (i, dev) in DEVELOPERS.iter().enumerate() {
            assert!(!dev.name.is_empty());
            assert!(
                DEVELOPERS[..i].iter().all(|other| other.name != dev.name),
                "{} is in the table twice",
                dev.name
            );
        }
    }

    #[test]
    fn unpack_bit_streams() {
        // Two 12-bit samples 0xABC, 0x123 packed big-endian.
        assert_eq!(
            unpack(&[0xAB, 0xC1, 0x23], 2, 12, false).unwrap(),
            vec![0xABC, 0x123]
        );
        assert_eq!(unpack(&[0x34, 0x12], 1, 16, true).unwrap(), vec![0x1234]);
    }

    #[test]
    fn identity_like_matrix_keeps_neutral() {
        // XYZ to camera equal to XYZ to sRGB makes camera = sRGB.
        let xyz_to_srgb = [
            3.240_454_2,
            -1.537_138_5,
            -0.498_531_4,
            -0.969_266,
            1.876_010_8,
            0.041_556,
            0.055_643_4,
            -0.204_025_9,
            1.057_225_2,
        ];
        let m = camera_to_srgb(xyz_to_srgb).unwrap();
        for r in 0..3 {
            let sum: f64 = m[r * 3..r * 3 + 3].iter().sum();
            assert!((sum - 1.0).abs() < 1e-3, "row {r} sums to {sum}");
            assert!((m[r * 3 + r] - 1.0).abs() < 1e-2, "diagonal {m:?}");
        }
    }
}

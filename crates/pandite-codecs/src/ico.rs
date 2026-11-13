//! ICO and CUR decoder. Picks the largest image in the directory (ties go
//! to the deeper color depth) and decodes its PNG or DIB payload; DIB
//! payloads use the AND mask for transparency unless they carry alpha.

use crate::util::{self, corrupt, le16, le32};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageFacts, ImageMeta, Orientation,
    Result, Source,
};

const FORMAT: &str = "ico";

/// Decodes ICO and CUR files.
#[derive(Clone, Copy, Debug, Default)]
pub struct IcoDecoder;

#[derive(Clone, Copy, Debug)]
struct Entry {
    width: u32,
    height: u32,
    bits: u16,
    size: usize,
    offset: usize,
}

fn entries(d: &[u8]) -> Result<Vec<Entry>> {
    if le16(d, 0) != Some(0) || !matches!(le16(d, 2), Some(1 | 2)) {
        return Err(corrupt(FORMAT, "not an icon directory"));
    }
    let count = usize::from(le16(d, 4).unwrap_or(0));
    if count == 0 {
        return Err(corrupt(FORMAT, "empty icon directory"));
    }
    let mut v = Vec::with_capacity(count);
    for i in 0..count {
        let at = 6 + i * 16;
        let Some(e) = d.get(at..at + 16) else {
            break;
        };
        let dim = |b: u8| if b == 0 { 256 } else { u32::from(b) };
        let entry = Entry {
            width: dim(e[0]),
            height: dim(e[1]),
            bits: le16(e, 6).unwrap_or(0),
            size: le32(e, 8).unwrap_or(0) as usize,
            offset: le32(e, 12).unwrap_or(0) as usize,
        };
        if entry.offset >= 6 && entry.offset < d.len() {
            v.push(entry);
        }
    }
    if v.is_empty() {
        return Err(corrupt(FORMAT, "no usable icon images"));
    }
    Ok(v)
}

/// The bytes of one directory entry's image.
fn payload_of<'a>(d: &'a [u8], e: &Entry) -> &'a [u8] {
    let end = if e.size == 0 {
        d.len()
    } else {
        e.offset.saturating_add(e.size).min(d.len())
    };
    &d[e.offset..end]
}

fn best(d: &[u8]) -> Result<Entry> {
    let list = entries(d)?;
    Ok(*list
        .iter()
        .max_by_key(|e| (u64::from(e.width) * u64::from(e.height), e.bits))
        .expect("non-empty"))
}

impl DecoderProvider for IcoDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "Windows icon or cursor"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["ico", "cur"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        // Reserved 0, type 1 or 2, a sane count, and a first entry whose
        // reserved byte is 0 and whose offset lies past the directory.
        let ok = header.len() >= 22
            && le16(header, 0) == Some(0)
            && matches!(le16(header, 2), Some(1 | 2))
            && matches!(le16(header, 4), Some(1..=255))
            && header[9] == 0;
        ok && le32(header, 18).is_some_and(|o| o as usize >= 6 + 16)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let e = best(source.bytes())?;
        Ok(ImageMeta {
            width: e.width,
            height: e.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: true,
            orientation: Orientation::Normal,
        })
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        let d = source.bytes();
        let count = entries(d)?.len();
        let e = best(d)?;
        let kind = if le16(d, 2) == Some(2) { "CUR" } else { "ICO" };
        let images = if count == 1 { "image" } else { "images" };
        // The facts of the image on screen: the largest one.
        let payload = payload_of(d, &e);
        let mut f = if payload.starts_with(&[0x89, b'P', b'N', b'G']) {
            let mut f =
                util::guard(FORMAT, || crate::png::header_facts(payload)).unwrap_or_default();
            f.compression = Some("PNG".to_string());
            f
        } else {
            crate::bmp::parse_header(payload, 0, true)
                .map(|h| crate::bmp::dib_facts(payload, 0, &h))
                .unwrap_or_default()
        };
        f.container = Some(format!("{kind}, {count} {images}"));
        Ok(f)
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let d = source.bytes();
        let e = best(d)?;
        let payload = payload_of(d, &e);
        let image = util::guard(FORMAT, || {
            if payload.starts_with(&[0x89, b'P', b'N', b'G']) {
                let opts = DecodeOptions {
                    cancel: options.cancel.clone(),
                    first_frame_only: true,
                    ..DecodeOptions::default()
                };
                Ok(crate::png::decode(payload, &opts)?.first_image().clone())
            } else {
                crate::bmp::decode_dib(payload, 0, None, true, &options.cancel)
            }
        })?;
        Ok(Content::Still(image))
    }
}

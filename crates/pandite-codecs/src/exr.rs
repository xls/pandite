//! OpenEXR decoder on the `exr` crate. Reads the largest resolution level
//! of the first valid layer: RGB(A) or luminance (Y) channels, half, float
//! or integer samples, any compression the crate supports. Pixels come back
//! as linear `RgbaF32` with straight alpha (EXR stores premultiplied color).

use crate::util::{self, check_size, corrupt, guard};
use exr::prelude::*;
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageMeta, Orientation, PixelFormat,
    Result, Source,
};
use std::io::Cursor;

const FORMAT: &str = "exr";
const MAGIC: [u8; 4] = [0x76, 0x2F, 0x31, 0x01];

/// Decodes OpenEXR files.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExrDecoder;

impl DecoderProvider for ExrDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "OpenEXR image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["exr"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(&MAGIC)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        guard(FORMAT, || {
            let meta = exr::meta::MetaData::read_from_buffered(Cursor::new(source.bytes()), false)
                .map_err(|e| corrupt(FORMAT, e))?;
            let header = meta
                .headers
                .first()
                .ok_or_else(|| corrupt(FORMAT, "no layers"))?;
            let size = header.layer_size;
            let has_alpha = header
                .channels
                .list
                .iter()
                .any(|c| c.name.eq("A") || c.name.to_string().ends_with(".A"));
            Ok(ImageMeta {
                width: size.width() as u32,
                height: size.height() as u32,
                format: FORMAT,
                kind: ContentKind::Still,
                frame_count: 1,
                has_alpha,
                orientation: Orientation::Normal,
            })
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || decode(source.bytes(), options)).map(Content::Still)
    }
}

fn sample(s: &FlatSamples, i: usize) -> f32 {
    match s {
        FlatSamples::F16(v) => v.get(i).map_or(0.0, |x| x.to_f32()),
        FlatSamples::F32(v) => v.get(i).copied().unwrap_or(0.0),
        FlatSamples::U32(v) => v.get(i).map_or(0.0, |&x| x as f32),
    }
}

fn decode(data: &[u8], options: &DecodeOptions) -> Result<pandite_core::ImageBuf> {
    // Check the declared size before any sample buffer is allocated.
    let meta = exr::meta::MetaData::read_from_buffered(Cursor::new(data), false)
        .map_err(|e| corrupt(FORMAT, e))?;
    for h in &meta.headers {
        let s = h.layer_size;
        let (w, ht) = (
            u32::try_from(s.width()).unwrap_or(u32::MAX),
            u32::try_from(s.height()).unwrap_or(u32::MAX),
        );
        check_size(w, ht, PixelFormat::RgbaF32)?;
    }
    options.cancel.check()?;
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels()
        .first_valid_layer()
        .all_attributes()
        .non_parallel()
        .from_buffered(Cursor::new(data))
        .map_err(|e| corrupt(FORMAT, e))?;
    options.cancel.check()?;
    let layer = image.layer_data;
    let (w, h) = (layer.size.width(), layer.size.height());
    let (w32, h32) = (w as u32, h as u32);
    check_size(w32, h32, PixelFormat::RgbaF32)?;
    let channels = &layer.channel_data.list;
    let find = |name: &str| {
        channels
            .iter()
            .find(|c| c.name.eq(name))
            .or_else(|| {
                channels
                    .iter()
                    .find(|c| c.name.to_string().ends_with(&format!(".{name}")))
            })
            .filter(|c| c.sampling == Vec2(1, 1))
            .map(|c| &c.sample_data)
    };
    let (r, g, b, y, a) = (find("R"), find("G"), find("B"), find("Y"), find("A"));
    let n = w * h;
    let mut out = Vec::new();
    out.try_reserve_exact(n * 4)
        .map_err(|_| pandite_core::Error::TooLarge {
            width: w32,
            height: h32,
        })?;
    let any = channels
        .iter()
        .find(|c| c.sampling == Vec2(1, 1))
        .map(|c| &c.sample_data)
        .ok_or_else(|| corrupt(FORMAT, "no full-resolution channel"))?;
    for i in 0..n {
        let alpha = a.map_or(1.0, |s| sample(s, i));
        let rgb = match (r, g, b, y) {
            (Some(r), Some(g), Some(b), _) => [sample(r, i), sample(g, i), sample(b, i)],
            (_, _, _, Some(y)) => [sample(y, i); 3],
            // No color channels: show the first channel as gray.
            _ => [sample(any, i); 3],
        };
        // OpenEXR color is premultiplied by alpha; the contract is straight.
        let rgb = if alpha > 0.0 && alpha != 1.0 {
            rgb.map(|c| c / alpha)
        } else {
            rgb
        };
        out.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
    }
    util::rgbaf32_image(w32, h32, &out)
}

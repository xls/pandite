//! QOI decoder on the `qoi` crate.

use crate::util::{self, check_size, corrupt, guard};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, ImageFacts, ImageMeta, Orientation,
    PixelFormat, Result, Source,
};

const FORMAT: &str = "qoi";

/// Decodes QOI files.
#[derive(Clone, Copy, Debug, Default)]
pub struct QoiDecoder;

impl DecoderProvider for QoiDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "QOI image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["qoi"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"qoif")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let h = qoi::decode_header(source.bytes()).map_err(|e| corrupt(FORMAT, e))?;
        Ok(ImageMeta {
            width: h.width,
            height: h.height,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: h.channels == qoi::Channels::Rgba,
            orientation: Orientation::Normal,
        })
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        let h = qoi::decode_header(source.bytes()).map_err(|e| corrupt(FORMAT, e))?;
        let mut f = ImageFacts::named("QOI");
        f.compression = Some("QOI".to_string());
        if h.channels == qoi::Channels::Rgba {
            f.set_channels("RGBA", 4, 8);
        } else {
            f.set_channels("RGB", 3, 8);
        }
        f.color_space = Some(
            match h.colorspace {
                qoi::ColorSpace::Srgb => "sRGB, linear alpha",
                qoi::ColorSpace::Linear => "Linear",
            }
            .to_string(),
        );
        Ok(f)
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        let data = source.bytes();
        // Size limits first, from the raw header: the qoi crate reports its
        // own pixel limit as a format error.
        if let (Some(w), Some(h)) = (util::be32(data, 4), util::be32(data, 8)) {
            check_size(w, h, PixelFormat::Rgba8)?;
        }
        let h = qoi::decode_header(data).map_err(|e| corrupt(FORMAT, e))?;
        check_size(h.width, h.height, PixelFormat::Rgba8)?;
        guard(FORMAT, || {
            let mut dec = qoi::Decoder::new(data)
                .map_err(|e| corrupt(FORMAT, e))?
                .with_channels(qoi::Channels::Rgba);
            let pixels = dec.decode_to_vec().map_err(|e| corrupt(FORMAT, e))?;
            util::rgba8_image(h.width, h.height, pixels).map(Content::Still)
        })
    }
}

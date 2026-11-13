//! SVG renderer on `resvg`. Renders at the document's intrinsic size, or
//! at the size that fits `DecodeOptions::target_size` (larger or smaller
//! than intrinsic: vector art stays sharp). With the `svg-text` feature,
//! text uses the system fonts, loaded once on the first document that
//! contains text; without it, text is not drawn. Embedded and relative
//! raster images (PNG, JPEG, GIF, WebP) are drawn; relative references
//! resolve against the file's folder.

use crate::util::{self, check_size, corrupt, fit_into, guard};
use pandite_core::{
    Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageFacts, ImageMeta,
    Orientation, PixelFormat, Result, Source,
};
use resvg::{tiny_skia, usvg};
#[cfg(feature = "svg-text")]
use std::sync::{Arc, OnceLock};

const FORMAT: &str = "svg";
/// Longest side used when the caller gives no target size.
const MAX_INTRINSIC: f32 = 8192.0;

/// Renders SVG documents.
#[derive(Clone, Copy, Debug, Default)]
pub struct SvgDecoder;

#[cfg(feature = "svg-text")]
fn fonts() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

#[cfg(feature = "svg-text")]
fn has_text(data: &[u8]) -> bool {
    data.windows(5).any(|w| w == b"<text" || w == b":text")
}

fn parse(source: &Source) -> Result<usvg::Tree> {
    let data = source.bytes();
    #[allow(unused_mut)]
    let mut opt = usvg::Options {
        resources_dir: source
            .path()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf()),
        ..usvg::Options::default()
    };
    #[cfg(feature = "svg-text")]
    if has_text(data) {
        opt.fontdb = fonts();
    }
    usvg::Tree::from_data(data, &opt).map_err(|e| corrupt(FORMAT, e))
}

/// True when the leading bytes look like an SVG document.
fn looks_like_svg(header: &[u8]) -> bool {
    let h = header.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(header);
    let start = h
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(h.len());
    let h = &h[start..];
    h.starts_with(b"<svg") || (h.starts_with(b"<?xml") && h.windows(4).any(|w| w == b"<svg"))
}

impl DecoderProvider for SvgDecoder {
    fn id(&self) -> &'static str {
        FORMAT
    }

    fn name(&self) -> &'static str {
        "SVG drawing"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["svg"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        looks_like_svg(header)
    }

    fn reduced_decode(&self, _source: &Source) -> bool {
        true
    }

    fn facts(&self, source: &Source) -> Result<ImageFacts> {
        // A vector drawing has no stored pixels: no channels or depth.
        let mut f = ImageFacts::named("SVG");
        f.compression = Some(
            if source.bytes().starts_with(&[0x1F, 0x8B]) {
                "gzip"
            } else {
                "None (XML text)"
            }
            .to_string(),
        );
        Ok(f)
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let tree = guard(FORMAT, || parse(source))?;
        let (w, h) = intrinsic(&tree);
        Ok(ImageMeta {
            width: w,
            height: h,
            format: FORMAT,
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: true,
            orientation: Orientation::Normal,
        })
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        options.cancel.check()?;
        guard(FORMAT, || {
            let tree = parse(source)?;
            options.cancel.check()?;
            let size = tree.size();
            let (w, h) = match options.target_size {
                Some(t) => fit_into(f64::from(size.width()), f64::from(size.height()), t),
                None => intrinsic(&tree),
            };
            check_size(w, h, PixelFormat::Rgba8)?;
            let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or(Error::TooLarge {
                width: w,
                height: h,
            })?;
            let transform =
                tiny_skia::Transform::from_scale(w as f32 / size.width(), h as f32 / size.height());
            resvg::render(&tree, transform, &mut pixmap.as_mut());
            let mut data = pixmap.take();
            // tiny-skia stores premultiplied alpha.
            for p in data.chunks_exact_mut(4) {
                let a = u32::from(p[3]);
                if a != 0 && a != 255 {
                    for c in &mut p[..3] {
                        *c = ((u32::from(*c) * 255 + a / 2) / a).min(255) as u8;
                    }
                }
            }
            util::rgba8_image(w, h, data).map(Content::Still)
        })
    }
}

/// The intrinsic pixel size, scaled down to fit [`MAX_INTRINSIC`].
fn intrinsic(tree: &usvg::Tree) -> (u32, u32) {
    let s = tree.size();
    let (w, h) = (s.width(), s.height());
    let scale = (MAX_INTRINSIC / w.max(h)).min(1.0);
    (
        (w * scale).ceil().max(1.0) as u32,
        (h * scale).ceil().max(1.0) as u32,
    )
}

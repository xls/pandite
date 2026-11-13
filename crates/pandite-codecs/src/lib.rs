//! pandite-codecs: the image decoders and encoders pandite ships.
//!
//! Every format is a small provider type implementing
//! [`pandite_core::DecoderProvider`] or [`pandite_core::EncoderProvider`].
//! [`default_registry`] registers all providers compiled into this build.
//!
//! Base formats are always compiled: JPEG, PNG and APNG, GIF, WebP and BMP.
//! Extended formats each sit behind a cargo feature that is on by default:
//! `tiff`, `tga`, `ico`, `qoi`, `avif`, `jxl`, `dds`, `ktx2`, `exr`, `hdr`,
//! `svg`, `psd`, `raw` (camera RAW) and `heic` (operating system codec on
//! Windows and macOS only). `svg-text` adds text rendering with system
//! fonts to SVG. The `icc` feature converts embedded ICC profiles to sRGB.
//!
//! All decoders return upright pixels (EXIF orientation applied), compose
//! animation frames onto full canvases, honor [`pandite_core::DecodeOptions`]
//! and refuse images above the [`pandite_core::ImageBuf`] limits. Hostile
//! input produces an error, never a panic: third-party decoders run inside
//! a panic guard (effective when the binary unwinds on panic).
//!
//! Parallel decode work (RAW development) runs on one bounded set of
//! helper threads shared by every decode in the process (`pool`), so
//! concurrent decodes never start a thread per core each. The hot pixel
//! loops of RAW development have vector versions (AVX2 on x86_64, NEON on
//! aarch64) chosen at run time, each checked bit for bit against its
//! scalar reference (`simd`); that module and the operating system codec
//! module are the only ones with `unsafe` code.

#![deny(unsafe_code)]
// Helpers shared with optional formats are unused in reduced builds.
#![cfg_attr(
    not(all(
        feature = "tiff",
        feature = "tga",
        feature = "ico",
        feature = "qoi",
        feature = "avif",
        feature = "jxl",
        feature = "dds",
        feature = "ktx2",
        feature = "exr",
        feature = "hdr",
        feature = "svg",
        feature = "psd",
        feature = "raw",
        feature = "icc"
    )),
    allow(dead_code, unused_imports)
)]

mod bmp;
mod compose;
mod encode;
mod exif;
mod gif;
mod icc;
mod jpeg;
mod orient;
mod png;
mod pool;
#[cfg(feature = "raw")]
mod simd;
mod util;
mod webp;

#[cfg(feature = "avif")]
mod avif;
#[cfg(feature = "dds")]
mod dds;
#[cfg(feature = "exr")]
mod exr;
#[cfg(feature = "hdr")]
mod hdr;
#[cfg(all(feature = "heic", any(windows, target_os = "macos")))]
mod heic;
#[cfg(feature = "ico")]
mod ico;
#[cfg(feature = "jxl")]
mod jxl;
#[cfg(feature = "ktx2")]
mod ktx2;
#[cfg(feature = "raw")]
mod ljpeg;
#[cfg(feature = "psd")]
mod psd;
#[cfg(feature = "qoi")]
mod qoi;
#[cfg(feature = "raw")]
mod raw;
#[cfg(feature = "svg")]
mod svg;
#[cfg(any(feature = "dds", feature = "ktx2"))]
mod texture;
#[cfg(feature = "tga")]
mod tga;
#[cfg(feature = "tiff")]
mod tiff;

pub use bmp::{BmpDecoder, BmpEncoder};
pub use encode::{JpegEncoder, PngEncoder, WebpEncoder};
pub use gif::GifDecoder;
pub use jpeg::JpegDecoder;
pub use png::PngDecoder;
pub use webp::WebpDecoder;

#[cfg(feature = "avif")]
pub use avif::AvifDecoder;
#[cfg(feature = "dds")]
pub use dds::DdsDecoder;
#[cfg(feature = "exr")]
pub use exr::ExrDecoder;
#[cfg(feature = "hdr")]
pub use hdr::HdrDecoder;
#[cfg(all(feature = "heic", any(windows, target_os = "macos")))]
pub use heic::HeicDecoder;
#[cfg(feature = "ico")]
pub use ico::IcoDecoder;
#[cfg(feature = "jxl")]
pub use jxl::JxlDecoder;
#[cfg(feature = "ktx2")]
pub use ktx2::Ktx2Decoder;
#[cfg(feature = "psd")]
pub use psd::PsdDecoder;
#[cfg(feature = "qoi")]
pub use qoi::QoiDecoder;
#[cfg(feature = "raw")]
pub use raw::RawDecoder;
#[cfg(feature = "svg")]
pub use svg::SvgDecoder;
#[cfg(feature = "tga")]
pub use tga::TgaDecoder;
#[cfg(feature = "tiff")]
pub use tiff::TiffDecoder;

use pandite_core::Registry;
use std::sync::Arc;

/// Add every decoder compiled into this build to `registry`.
///
/// Registration order matters for signature sniffing: formats with strong
/// signatures come first, formats matched by extension only come last.
pub fn register_decoders(registry: &mut Registry) {
    registry.add_decoder(Arc::new(JpegDecoder));
    registry.add_decoder(Arc::new(PngDecoder));
    registry.add_decoder(Arc::new(GifDecoder));
    registry.add_decoder(Arc::new(WebpDecoder));
    registry.add_decoder(Arc::new(BmpDecoder));
    // RAW before TIFF: RAW claims only signatures plain TIFF never uses;
    // TIFF-structured RAW files are handed over by the TIFF provider.
    #[cfg(feature = "raw")]
    registry.add_decoder(Arc::new(RawDecoder));
    #[cfg(feature = "tiff")]
    registry.add_decoder(Arc::new(TiffDecoder));
    #[cfg(feature = "qoi")]
    registry.add_decoder(Arc::new(QoiDecoder));
    #[cfg(feature = "avif")]
    registry.add_decoder(Arc::new(AvifDecoder));
    // After AVIF: both use HEIF containers; AVIF brands win.
    #[cfg(all(feature = "heic", any(windows, target_os = "macos")))]
    registry.add_decoder(Arc::new(HeicDecoder));
    #[cfg(feature = "jxl")]
    registry.add_decoder(Arc::new(JxlDecoder));
    #[cfg(feature = "dds")]
    registry.add_decoder(Arc::new(DdsDecoder));
    #[cfg(feature = "ktx2")]
    registry.add_decoder(Arc::new(Ktx2Decoder));
    #[cfg(feature = "exr")]
    registry.add_decoder(Arc::new(ExrDecoder));
    #[cfg(feature = "hdr")]
    registry.add_decoder(Arc::new(HdrDecoder));
    #[cfg(feature = "psd")]
    registry.add_decoder(Arc::new(PsdDecoder));
    #[cfg(feature = "svg")]
    registry.add_decoder(Arc::new(SvgDecoder));
    // Weak or missing signatures last.
    #[cfg(feature = "ico")]
    registry.add_decoder(Arc::new(IcoDecoder));
    #[cfg(feature = "tga")]
    registry.add_decoder(Arc::new(TgaDecoder));
}

/// Add the PNG, JPEG, lossless WebP and BMP encoders to `registry`.
pub fn register_encoders(registry: &mut Registry) {
    registry.add_encoder(Arc::new(PngEncoder));
    registry.add_encoder(Arc::new(JpegEncoder));
    registry.add_encoder(Arc::new(WebpEncoder));
    registry.add_encoder(Arc::new(BmpEncoder));
}

/// A registry holding every decoder and encoder compiled into this build.
pub fn default_registry() -> Registry {
    let mut registry = Registry::new();
    register_decoders(&mut registry);
    register_encoders(&mut registry);
    registry
}

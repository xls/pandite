//! pandite-ocr: text recognition for the pandite viewer.
//!
//! Every recognizer is a [`pandite_core::TextProvider`] compiled into the
//! binary. [`register_text_providers`] adds the ones this platform has to a
//! [`Registry`]:
//!
//! - Windows: [`WindowsOcr`], the recognizer built into Windows 10 and 11
//!   (`Windows.Media.Ocr`). It runs fully offline and needs an OCR
//!   language, which Windows installs with most display languages.
//! - macOS and Linux: none yet. The registry stays empty and the viewer
//!   shows no text features.
//!
//! Large images are scaled down before recognition: the long side is at
//! most [`MAX_SIDE`] pixels (and never above the engine's own limit), and
//! word outlines are mapped back to full-image pixels. [`prepare`] holds
//! that platform-neutral part.
//!
//! The Windows module is the only one with `unsafe` code: it lowers the
//! worker thread's scheduling priority for the duration of a recognition.

#![deny(unsafe_code)]

pub mod prepare;
#[cfg(windows)]
mod windows_ocr;

#[cfg(windows)]
pub use windows_ocr::WindowsOcr;

use pandite_core::{Registry, TextProvider};
use std::sync::Arc;

/// The longest side, in pixels, an image is recognized at. Larger images
/// are scaled down by area averaging first.
///
/// Chosen by measurement (`examples/ocr_bench.rs`, Windows OCR, release
/// build): a 9504 x 6336 synthetic photo with ten lines of text from 18 to
/// 72 pixels took 138 ms per call at a 4000 pixel cap, 173 ms at 5000,
/// 216 ms at 6000, 268 ms at 8000 and 352 ms unscaled. At 4000 the 18
/// pixel lines were lost (1 of 13 words) and at 5000 partly lost (11 of
/// 13); from 6000 up every word was found. 6000 keeps the small print of
/// 60 megapixel photos at 60 percent of the unscaled time. The engine
/// alone needs about 90 ms of that at 6000; a dense page of 1456 words
/// takes about 800 ms at 6000 x 8400.
pub const MAX_SIDE: u32 = 6000;

/// Every text recognizer this platform has, best first. Empty on
/// platforms without one.
pub fn text_providers() -> Vec<Arc<dyn TextProvider>> {
    #[cfg(windows)]
    {
        vec![Arc::new(WindowsOcr::new()) as Arc<dyn TextProvider>]
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Add [`text_providers`] to `registry`.
pub fn register_text_providers(registry: &mut Registry) {
    for provider in text_providers() {
        registry.add_text_provider(provider);
    }
}

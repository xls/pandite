//! pandite-core: the contract every other pandite crate builds on.
//!
//! This crate holds plain data types and provider traits only. It has no
//! dependencies, touches no GPU and does no file or network I/O, so decoder,
//! operation, cache, renderer and interface crates can be built and tested
//! in isolation and then joined by the application crate.
//!
//! Layout:
//! - [`error`]: the shared error type.
//! - [`geom`]: points, sizes, rectangles and fit math.
//! - [`image`]: pixel buffers, animations and decoded content.
//! - [`provider`]: decoder, encoder and text recognizer registration.
//! - [`text`]: recognized text (lines, words, outlines) and the text
//!   recognition provider trait.
//! - [`view`]: display modes and the image-to-screen transform.
//! - [`draw`]: the retained draw list the interface emits and the renderer consumes.

#![forbid(unsafe_code)]

pub mod draw;
pub mod error;
pub mod geom;
pub mod image;
pub mod provider;
pub mod text;
pub mod view;

pub use draw::{Color, DrawCmd, DrawList, TextMeasure, TextureId, TextureSink};
pub use error::{Error, Result};
pub use geom::{Affine, Point, Rect, Size};
pub use image::{
    Animation, Content, ContentKind, Frame, ImageBuf, ImageMeta, LoopCount, Orientation,
    PixelFormat,
};
pub use provider::{
    CancelToken, DecodeOptions, DecoderProvider, EncodeOptions, EncoderProvider, Registry, Source,
};
pub use text::{Quad, TextLayout, TextLine, TextProvider, TextWord};
pub use view::{DisplayMode, ViewState};

//! pandite-ops: image operations for the pandite viewer.
//!
//! Every operation takes a [`pandite_core::ImageBuf`] and returns a new one;
//! inputs never change. Failures are [`pandite_core::Error`] values. The
//! crate uses only the standard library and contains no unsafe code.
//!
//! - [`resize`]: size rules ([`ResizeSpec`]), the high-quality separable
//!   resampler ([`resize()`], [`resize_with`]) and fast thumbnails
//!   ([`thumbnail`]).
//! - [`filter`]: the resampling filters ([`Filter`]).
//! - [`transform`]: quarter-turn rotation, mirroring and cropping.
//! - [`convert`]: pixel format conversion and the sRGB transfer curve.
//! - [`bulk`]: the batch resize and reformat engine ([`run_bulk`]), which
//!   reaches decoders and encoders only through a
//!   [`pandite_core::Registry`].

#![forbid(unsafe_code)]

pub mod bulk;
pub mod convert;
pub mod filter;
mod par;
pub mod resize;
pub mod transform;

pub use bulk::{
    run_bulk, BulkJob, BulkReport, Destination, FileOutcome, FileReport, OverwritePolicy, Progress,
};
pub use convert::{convert, linear_to_srgb, srgb_to_linear, to_rgba8};
pub use filter::Filter;
pub use resize::{
    resize, resize_to_spec, resize_with, thumbnail, thumbnail_size, ResizeOptions, ResizeSpec,
};
pub use transform::{
    crop, flip_horizontal, flip_vertical, rotate, rotate180, rotate270, rotate90, PixelRect,
    Rotation,
};

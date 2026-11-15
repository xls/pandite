//! pandite-library: the images beside the opened file, decoded ahead of
//! time.
//!
//! - [`FolderList`] lists the files of a folder that a [`Registry`] can
//!   open, in natural order (`img2` before `img10`, case-insensitive).
//! - [`Library`] owns worker threads and memory-budgeted caches. After
//!   [`Library::set_current`] it decodes the current image first, then its
//!   neighbors (next, previous, next+1, previous+1, ...), then thumbnails
//!   for the visible gallery range, a margin beyond it, and the rest of the
//!   folder. While the current image loads, nothing else starts. Results
//!   arrive through [`Library::poll_events`] after a wake callback, so the
//!   interface never waits for a decode.
//! - With [`LibraryConfig::screen_size`] set (staged loading), the current
//!   image delivers a quick reduced stage ([`Event::Reduced`],
//!   [`Library::reduced`]) before its full resolution, neighbors are kept
//!   at screen resolution only, and full resolution is decoded for the
//!   current image alone.
//! - Every file system call goes through [`FileAccess`] on a worker
//!   thread. A stalled read never blocks the caller or shutdown, and files
//!   that are not stored locally (cloud placeholders) are read only when
//!   the user opens them.
//! - [`box_downscale`] is the built-in thumbnail filter; the application
//!   may inject a better one through [`LibraryConfig::downscale`], and its
//!   own thumbnail shape through [`LibraryConfig::make_thumbnail`].
//!   [`Library::set_thumbnail_size`] changes the thumbnail size while the
//!   library runs; the old thumbnails stay until the new ones are made.
//! - [`Library::export_thumbnails`] hands the thumbnails of the folder to
//!   the next library ([`LibraryConfig::kept_thumbnails`]), which uses each
//!   one whose file did not change instead of decoding it again.
//!
//! The crate uses only the standard library.
//!
//! [`Registry`]: pandite_core::Registry

// Denied, not forbidden: the downscaler's vector module allows it.
#![deny(unsafe_code)]

mod cache;
mod downscale;
mod folder;
mod fs;
mod kept;
mod library;
mod state;
mod worker;

pub use downscale::{box_downscale, fit_size};
#[doc(hidden)]
pub use downscale::{box_downscale_reference, box_downscale_with, DownscaleLevel};
pub use folder::{natural_cmp, FolderList};
pub use fs::{
    is_local, read_chunked, std_list_dir, std_read_header, FileAccess, FileInfo, Stamp, StdFiles,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    READ_CHUNK,
};
pub use kept::{KeptThumbnail, KeptThumbnails};
pub use library::{
    DownscaleFn, Event, Fetch, Library, LibraryConfig, LoadTimings, Reduced, Stats, ThumbnailFn,
    WakeFn,
};

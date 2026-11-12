//! pandite-library: the images beside the opened file, decoded ahead of
//! time.
//!
//! - [`FolderList`] lists the files of a folder that a [`Registry`] can
//!   open, in natural order (`img2` before `img10`, case-insensitive).
//! - [`Library`] owns worker threads and two memory-budgeted caches. After
//!   [`Library::set_current`] it decodes the current image first, then its
//!   neighbors (next, previous, next+1, previous+1, ...), then thumbnails
//!   for the visible gallery range, a margin beyond it, and the rest of the
//!   folder. Results arrive through [`Library::poll_events`] after a wake
//!   callback, so the interface never waits for a decode.
//! - Every file system call goes through [`FileAccess`] on a worker
//!   thread. A stalled read never blocks the caller or shutdown, and files
//!   that are not stored locally (cloud placeholders) are read only when
//!   the user opens them.
//! - [`box_downscale`] is the built-in thumbnail filter; the application
//!   may inject a better one through [`LibraryConfig::downscale`].
//!
//! The crate uses only the standard library.
//!
//! [`Registry`]: pandite_core::Registry

#![forbid(unsafe_code)]

mod cache;
mod downscale;
mod folder;
mod fs;
mod library;
mod state;
mod worker;

pub use downscale::{box_downscale, fit_size};
pub use folder::{natural_cmp, FolderList};
pub use fs::{
    is_local, read_chunked, std_list_dir, FileAccess, FileInfo, Stamp, StdFiles,
    FILE_ATTRIBUTE_OFFLINE, FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS, FILE_ATTRIBUTE_RECALL_ON_OPEN,
    READ_CHUNK,
};
pub use library::{DownscaleFn, Event, Fetch, Library, LibraryConfig, Stats, WakeFn};

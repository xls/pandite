//! Thumbnails handed from a library that ends to the next one, so a new
//! session in the same folder shows its gallery without decoding again.

use crate::fs::Stamp;
use pandite_core::ImageBuf;
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// One kept thumbnail and the stamp of its file when it was made.
#[derive(Clone)]
pub struct KeptThumbnail {
    /// Size and modification time of the file when the thumbnail was made.
    /// The next library uses the thumbnail only when the file still has
    /// this stamp.
    pub stamp: Stamp,
    /// The thumbnail pixels.
    pub image: ImageBuf,
}

impl fmt::Debug for KeptThumbnail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeptThumbnail")
            .field("stamp", &self.stamp)
            .field("width", &self.image.width())
            .field("height", &self.image.height())
            .finish()
    }
}

/// The thumbnails of one folder, taken from a library with
/// [`crate::Library::export_thumbnails`] and given to the next one through
/// [`crate::LibraryConfig::kept_thumbnails`]. A plain owned value: dropping
/// it frees the pixels.
#[derive(Clone, Default)]
pub struct KeptThumbnails {
    /// The folder the thumbnails belong to.
    pub folder: PathBuf,
    /// The longest thumbnail side they were made for. A library with
    /// another thumbnail size drops them.
    pub thumb_size: u32,
    /// The thumbnails by file path.
    pub entries: HashMap<PathBuf, KeptThumbnail>,
}

impl KeptThumbnails {
    /// Number of thumbnails.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no thumbnail is kept.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Pixel bytes of all thumbnails together.
    pub fn bytes(&self) -> usize {
        self.entries.values().map(|e| e.image.byte_size()).sum()
    }

    /// True when opening `path` opens this folder: `path` is the folder
    /// itself or a file in it.
    pub fn serves(&self, path: &Path) -> bool {
        path == self.folder || path.parent() == Some(self.folder.as_path())
    }
}

impl fmt::Debug for KeptThumbnails {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeptThumbnails")
            .field("folder", &self.folder)
            .field("thumb_size", &self.thumb_size)
            .field("entries", &self.len())
            .field("bytes", &self.bytes())
            .finish()
    }
}

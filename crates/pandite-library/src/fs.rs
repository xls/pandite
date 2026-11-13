//! File system access. Every disk call the library makes goes through
//! [`FileAccess`], and the library only calls it on worker threads, so a
//! stalled read (a cloud placeholder that never downloads, a dead network
//! share) can never block the caller.

use pandite_core::{CancelToken, Result, Source};
use std::ffi::OsString;
use std::fs::{File, Metadata};
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

/// Bytes read between two cancellation checks in [`read_chunked`].
pub const READ_CHUNK: u64 = 1 << 20;

/// Largest buffer [`read_chunked`] reserves up front from the file length.
const MAX_PREALLOC: u64 = 256 << 20;

/// Files at least this large are read into a buffer whose pages were made
/// present on several threads first.
const TOUCH_MIN: usize = 16 << 20;

/// A zeroed buffer of `len` bytes with every page present: the page
/// faults of a fresh allocation resolve on up to 8 threads at once instead
/// of one by one inside the file read (about 2 ms instead of 12 ms for
/// 130 MB).
fn touched(len: usize) -> Vec<u8> {
    const PAGE: usize = 4096;
    let mut bytes = vec![0u8; len];
    let parts = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, 8);
    let chunk = len.div_ceil(parts).next_multiple_of(PAGE).max(PAGE);
    std::thread::scope(|s| {
        for part in bytes.chunks_mut(chunk) {
            // A thread that fails to start leaves its pages to the read.
            let _ = std::thread::Builder::new().spawn_scoped(s, move || {
                for page in part.chunks_mut(PAGE) {
                    page[0] = 0;
                }
            });
        }
    });
    bytes
}

/// Windows: the data is not stored locally and reading it starts a recall.
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x0040_0000;
/// Windows: opening the file starts a recall.
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x0004_0000;
/// Windows: the data is held on offline storage.
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;

/// Size and modification time of a file. Two equal stamps mean the
/// library treats the file as unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Stamp {
    /// Length in bytes.
    pub len: u64,
    /// Last modification time, when the platform reports one.
    pub modified: Option<SystemTime>,
}

impl Stamp {
    /// The stamp of a metadata record.
    pub fn of(meta: &Metadata) -> Self {
        Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

/// Facts about one file or directory, gathered without reading its data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// The final path component.
    pub name: OsString,
    /// True for a regular file (after following symbolic links).
    pub is_file: bool,
    /// True for a directory (after following symbolic links).
    pub is_dir: bool,
    /// Size and modification time.
    pub stamp: Stamp,
    /// False when the data is not stored on this machine (cloud
    /// placeholder, offline storage). Such files are read only when the
    /// user opens them explicitly.
    pub local: bool,
}

impl FileInfo {
    /// Build the record for `name` from its metadata, using [`is_local`].
    pub fn from_metadata(name: OsString, meta: &Metadata) -> Self {
        Self {
            name,
            is_file: meta.is_file(),
            is_dir: meta.is_dir(),
            stamp: Stamp::of(meta),
            local: is_local(meta),
        }
    }
}

/// The library's window on the file system. Every method may block for
/// any length of time; the library calls them on worker threads only.
/// The default methods use `std::fs`; implement the trait to inject
/// stalls, locality or failures in tests.
pub trait FileAccess: Send + Sync {
    /// List the entries of `dir`. Entries that vanish during the listing
    /// are skipped.
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        std_list_dir(dir)
    }

    /// Facts about one path, following symbolic links. Must not read the
    /// file data.
    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        let meta = std::fs::metadata(path)?;
        let name = path.file_name().map(OsString::from).unwrap_or_default();
        Ok(FileInfo::from_metadata(name, &meta))
    }

    /// Read the whole file. Check `cancel` at least every
    /// [`READ_CHUNK`] bytes and return [`pandite_core::Error::Cancelled`] once
    /// it is set.
    fn read(&self, path: &Path, cancel: &CancelToken) -> Result<Source> {
        read_chunked(path, cancel)
    }
}

/// [`FileAccess`] through `std::fs`. The default.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdFiles;

impl FileAccess for StdFiles {}

/// True when the file data is stored on this machine.
///
/// On Windows a file with `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`,
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN` or `FILE_ATTRIBUTE_OFFLINE` is not local
/// (OneDrive, Dropbox and Google Drive placeholders set these). Other
/// platforms report every file as local.
#[cfg(windows)]
pub fn is_local(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    let remote = FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS
        | FILE_ATTRIBUTE_RECALL_ON_OPEN
        | FILE_ATTRIBUTE_OFFLINE;
    meta.file_attributes() & remote == 0
}

/// True when the file data is stored on this machine.
///
/// On Windows a file with `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`,
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN` or `FILE_ATTRIBUTE_OFFLINE` is not local
/// (OneDrive, Dropbox and Google Drive placeholders set these). Other
/// platforms report every file as local.
#[cfg(not(windows))]
pub fn is_local(_meta: &Metadata) -> bool {
    true
}

/// List a directory through `std::fs`. Symbolic links are followed;
/// entries whose metadata cannot be read are skipped.
pub fn std_list_dir(dir: &Path) -> io::Result<Vec<FileInfo>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let Ok(entry) = entry else { continue };
        // On Windows the entry metadata comes from the directory listing
        // itself, so cloud placeholders are not opened.
        let meta = match entry.file_type() {
            Ok(kind) if kind.is_symlink() => std::fs::metadata(entry.path()),
            Ok(_) => entry.metadata(),
            Err(err) => Err(err),
        };
        if let Ok(meta) = meta {
            out.push(FileInfo::from_metadata(entry.file_name(), &meta));
        }
    }
    Ok(out)
}

/// Read a whole file in [`READ_CHUNK`] pieces, checking `cancel` before
/// each piece. A read that the operating system itself blocks cannot be
/// interrupted; the library abandons such a worker instead.
pub fn read_chunked(path: &Path, cancel: &CancelToken) -> Result<Source> {
    cancel.check()?;
    let mut file = File::open(path)?;
    let hint = file.metadata().map(|m| m.len()).unwrap_or(0);
    let hint = usize::try_from(hint.min(MAX_PREALLOC)).unwrap_or(0);
    let mut bytes = if hint >= TOUCH_MIN {
        // A large file: the whole buffer at once, its pages made present
        // on several threads, then filled chunk by chunk.
        let mut bytes = touched(hint);
        let mut filled = 0;
        while filled < bytes.len() {
            cancel.check()?;
            let end = (filled + READ_CHUNK as usize).min(bytes.len());
            match file.read(&mut bytes[filled..end]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        // The file shrank since its size was read.
        bytes.truncate(filled);
        bytes
    } else {
        Vec::with_capacity(hint)
    };
    // The rest (all of a small file, anything a large file grew by).
    loop {
        cancel.check()?;
        let got = (&mut file).take(READ_CHUNK).read_to_end(&mut bytes)?;
        if got == 0 {
            break;
        }
    }
    // The buffer becomes the source as it is: no copy of the file.
    Ok(Source::from_vec(bytes, Some(path.to_path_buf())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pandite_core::Error;

    #[test]
    fn read_chunked_reads_everything_and_honors_cancel() {
        let dir =
            std::env::temp_dir().join(format!("pandite-library-unit-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.bin");
        let data: Vec<u8> = (0..(READ_CHUNK as usize * 2 + 17))
            .map(|i| (i % 251) as u8)
            .collect();
        std::fs::write(&path, &data).unwrap();
        let cancel = CancelToken::new();
        let source = read_chunked(&path, &cancel).unwrap();
        assert_eq!(source.bytes(), &data[..]);
        assert_eq!(source.path(), Some(path.as_path()));
        cancel.cancel();
        assert!(matches!(
            read_chunked(&path, &cancel),
            Err(Error::Cancelled)
        ));
        // A large file takes the pre-paged buffer and reads the same bytes.
        let big_path = dir.join("large.bin");
        let big: Vec<u8> = (0..TOUCH_MIN + 4097).map(|i| (i % 253) as u8).collect();
        std::fs::write(&big_path, &big).unwrap();
        let source = read_chunked(&big_path, &CancelToken::new()).unwrap();
        assert_eq!(source.bytes(), &big[..]);
        assert!(matches!(
            read_chunked(&big_path, &cancel),
            Err(Error::Cancelled)
        ));
        let info = StdFiles.file_info(&path).unwrap();
        assert!(info.is_file && !info.is_dir);
        assert_eq!(info.stamp.len, data.len() as u64);
        let listed = StdFiles.list_dir(&dir).unwrap();
        assert!(listed.iter().any(|f| f.name == "big.bin" && f.is_file));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

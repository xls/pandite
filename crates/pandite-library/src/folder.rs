//! The list of images beside the opened file.
//!
//! [`FolderList::scan`] is synchronous and touches the disk. It is meant for
//! tests and command line tools. The viewer uses [`crate::Library::open`],
//! which runs the same scan on a worker thread and reports the result
//! through [`crate::Event::Listed`].

use crate::fs::{FileAccess, FileInfo, StdFiles};
use pandite_core::Registry;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::io;
use std::path::{Path, PathBuf};

/// Compare two file names the way people sort them: case-insensitive, and
/// runs of ASCII digits compare by numeric value, so `img2` sorts before
/// `img10`. Names that differ only in case or leading zeros still get a
/// stable order (fewer leading zeros first, then by code point).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    natural_cmp_folded(a, b).then_with(|| a.cmp(b))
}

fn natural_cmp_folded(a: &str, b: &str) -> Ordering {
    let (ab, bb) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0usize, 0usize);
    let mut zero_tie = Ordering::Equal;
    loop {
        let (Some(c), Some(d)) = (a[i..].chars().next(), b[j..].chars().next()) else {
            let ends = (i < a.len()).cmp(&(j < b.len()));
            return ends.then(zero_tie);
        };
        if c.is_ascii_digit() && d.is_ascii_digit() {
            let ei = i + ab[i..].iter().take_while(|x| x.is_ascii_digit()).count();
            let ej = j + bb[j..].iter().take_while(|x| x.is_ascii_digit()).count();
            let (ra, rb) = (&a[i..ei], &b[j..ej]);
            let (ta, tb) = (ra.trim_start_matches('0'), rb.trim_start_matches('0'));
            let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
            if ord != Ordering::Equal {
                return ord;
            }
            if zero_tie == Ordering::Equal {
                zero_tie = ra.len().cmp(&rb.len());
            }
            i = ei;
            j = ej;
            continue;
        }
        let ord = if c.is_ascii() && d.is_ascii() {
            c.to_ascii_lowercase().cmp(&d.to_ascii_lowercase())
        } else {
            c.to_lowercase().cmp(d.to_lowercase())
        };
        if ord != Ordering::Equal {
            return ord;
        }
        i += c.len_utf8();
        j += d.len_utf8();
    }
}

fn names_match_ignoring_case(a: &OsStr, b: &OsStr) -> bool {
    a.eq_ignore_ascii_case(b)
        || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}

/// The openable files of one folder in natural order.
#[derive(Clone, Debug, Default)]
pub struct FolderList {
    dir: PathBuf,
    paths: Vec<PathBuf>,
    info: Vec<FileInfo>,
    opened: Option<usize>,
    listing_error: Option<io::ErrorKind>,
}

impl FolderList {
    /// List the folder of `opened` with `std::fs`. See [`FolderList::scan_with`].
    pub fn scan(opened: &Path, registry: &Registry) -> Self {
        Self::scan_with(opened, registry, &StdFiles)
    }

    /// List the folder of `opened` through `files`.
    ///
    /// The list holds every regular file whose extension the registry can
    /// open ([`Registry::can_open_path`]), sorted with [`natural_cmp`]. The
    /// opened file is always included when it exists, even with an
    /// unknown extension, because decoders also match by signature. When
    /// `opened` is a directory, that directory is listed and no entry is
    /// marked as opened. Failures never panic: an unreadable folder yields
    /// just the opened file (and [`FolderList::listing_error`] tells why),
    /// and a missing file yields its siblings with no opened index.
    pub fn scan_with(opened: &Path, registry: &Registry, files: &dyn FileAccess) -> Self {
        if opened.as_os_str().is_empty() {
            return Self::default();
        }
        let opened_info = files.file_info(opened).ok();
        if opened_info.as_ref().is_some_and(|i| i.is_dir) {
            return Self::scan_dir(opened.to_path_buf(), None, registry, files);
        }
        let dir = opened.parent().map(Path::to_path_buf).unwrap_or_default();
        let target = opened.file_name().and_then(|name| {
            let info = opened_info.filter(|i| i.is_file)?;
            Some(FileInfo {
                name: name.to_os_string(),
                ..info
            })
        });
        Self::scan_dir(dir, target, registry, files)
    }

    fn scan_dir(
        dir: PathBuf,
        target: Option<FileInfo>,
        registry: &Registry,
        files: &dyn FileAccess,
    ) -> Self {
        let listing_dir = if dir.as_os_str().is_empty() {
            Path::new(".")
        } else {
            dir.as_path()
        };
        let (listed, listing_error) = match files.list_dir(listing_dir) {
            Ok(listed) => (listed, None),
            Err(err) => (Vec::new(), Some(err.kind())),
        };
        let mut entries: Vec<FileInfo> = listed
            .into_iter()
            .filter(|info| info.is_file && registry.can_open_path(&dir.join(&info.name)))
            .collect();
        if let Some(target) = &target {
            let exact = entries.iter().position(|e| e.name == target.name);
            let found = exact.or_else(|| {
                entries
                    .iter()
                    .position(|e| names_match_ignoring_case(&e.name, &target.name))
            });
            match found {
                // Keep the caller's spelling so paths compare equal.
                Some(at) => entries[at].name = target.name.clone(),
                None => entries.push(target.clone()),
            }
        }
        let mut keyed: Vec<(String, FileInfo)> = entries
            .into_iter()
            .map(|info| (info.name.to_string_lossy().into_owned(), info))
            .collect();
        keyed.sort_by(|a, b| natural_cmp(&a.0, &b.0).then_with(|| a.1.name.cmp(&b.1.name)));
        keyed.dedup_by(|a, b| a.1.name == b.1.name);
        let info: Vec<FileInfo> = keyed.into_iter().map(|(_, info)| info).collect();
        let paths: Vec<PathBuf> = info.iter().map(|i| dir.join(&i.name)).collect();
        let opened = target.and_then(|t| info.iter().position(|i| i.name == t.name));
        Self {
            dir,
            paths,
            info,
            opened,
            listing_error,
        }
    }

    /// List the folder again with `std::fs`. See [`FolderList::refresh_with`].
    pub fn refresh(&mut self, registry: &Registry, current: Option<usize>) -> Option<usize> {
        self.refresh_with(registry, &StdFiles, current)
    }

    /// List the folder again and return the new index of the entry that
    /// was at `current`. When that file vanished, the entry that now takes
    /// its sort position is returned (or the last entry), so the viewer
    /// moves to a neighbor. Returns `None` when the folder is now empty.
    pub fn refresh_with(
        &mut self,
        registry: &Registry,
        files: &dyn FileAccess,
        current: Option<usize>,
    ) -> Option<usize> {
        let keep = current.and_then(|i| self.paths.get(i).cloned());
        let fresh = match &keep {
            Some(path) => Self::scan_with(path, registry, files),
            None => Self::scan_dir(self.dir.clone(), None, registry, files),
        };
        *self = fresh;
        keep.and_then(|path| self.nearest_index(&path))
    }

    /// The listed folder. Empty for a relative path without a parent.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// All entries in display order.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// True when the folder holds no openable file.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The entry at `index`.
    pub fn get(&self, index: usize) -> Option<&Path> {
        self.paths.get(index).map(PathBuf::as_path)
    }

    /// Facts gathered for the entry at `index` during the scan.
    pub fn info(&self, index: usize) -> Option<&FileInfo> {
        self.info.get(index)
    }

    /// The index of the file passed to the scan, when it exists.
    pub fn opened_index(&self) -> Option<usize> {
        self.opened
    }

    /// Why the folder could not be listed, when it could not. The list
    /// then holds only the opened file. A folder the system does not let
    /// the program read (the macOS App Sandbox before the user allows the
    /// folder) gives [`io::ErrorKind::PermissionDenied`].
    pub fn listing_error(&self) -> Option<io::ErrorKind> {
        self.listing_error
    }

    /// The index of `path`, compared component by component.
    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.paths.iter().position(|p| p == path)
    }

    /// The index of `path`, or when it is not listed, the index of the
    /// entry that sorts right after its name (clamped to the last entry).
    pub fn nearest_index(&self, path: &Path) -> Option<usize> {
        if self.paths.is_empty() {
            return None;
        }
        if let Some(found) = self.index_of(path) {
            return Some(found);
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let after = self
            .info
            .iter()
            .position(|i| natural_cmp(&i.name.to_string_lossy(), &name) == Ordering::Greater)
            .unwrap_or(self.paths.len() - 1);
        Some(after)
    }

    /// A map from path to index, for bulk lookups.
    pub(crate) fn index_map(&self) -> HashMap<&Path, usize> {
        self.paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_path(), i))
            .collect()
    }

    /// Split into paths and per-entry facts.
    pub(crate) fn into_parts(self) -> (PathBuf, Vec<PathBuf>, Vec<FileInfo>) {
        (self.dir, self.paths, self.info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_cmp_orders_numbers_and_case() {
        let mut names = vec![
            "img10.png",
            "IMG2.png",
            "img1.png",
            "img002.png",
            "img2.png",
            "b.png",
            "A.png",
            "img100.png",
            "img2a.png",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            vec![
                "A.png",
                "b.png",
                "img1.png",
                "IMG2.png",
                "img2.png",
                "img002.png",
                "img2a.png",
                "img10.png",
                "img100.png",
            ]
        );
        assert_eq!(natural_cmp("x", "x"), Ordering::Equal);
        assert_eq!(natural_cmp("x", "x1"), Ordering::Less);
        assert_eq!(
            natural_cmp("a99999999999999999999999", "a100000000000000000000000"),
            Ordering::Less
        );
        assert_eq!(natural_cmp("\u{c4}b", "\u{e4}a"), Ordering::Greater);
    }
}

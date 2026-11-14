//! File system access for the application's own work.
//!
//! [`Disk`] extends the library's [`FileAccess`] with the few calls the
//! application needs besides reading images: a bounded read of a file's
//! head (to probe its size), small reads and atomic writes (settings, Save
//! As), directory creation, the removal of a file (to the trash or for
//! good), whether a file has a trash, and a rename and a copy that never
//! replace a file (to move files into another folder). The application
//! calls these only from worker threads; tests inject implementations that
//! stall or record the calling thread.

use pandite_core::Registry;
use pandite_library::{natural_cmp, FileAccess, StdFiles};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// File system access used by the application. Every method may block for
/// any length of time; call them on worker threads only.
pub trait Disk: FileAccess {
    /// Read at most `max` bytes from the start of `path`.
    fn read_head(&self, path: &Path, max: usize) -> io::Result<Vec<u8>> {
        read_head(path, max)
    }

    /// Read a whole small file (settings).
    fn read_small(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    /// Replace `path` with `data` atomically: readers see either the old or
    /// the new content, never a partial file. See [`write_atomic`].
    fn write_atomic(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        write_atomic(path, data)
    }

    /// Rename `from` to `to`, replacing `to`.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    /// Create `dir` and its parents.
    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dir)
    }

    /// Move the file `path` to the trash of the system. See
    /// [`crate::trash::trash`]. When its drive or volume has no trash, the
    /// file stays and the error is [`crate::trash::no_trash`].
    fn trash(&self, path: &Path) -> io::Result<()> {
        crate::trash::trash(path)
    }

    /// False when the drive or volume of the file `path` is known to have
    /// no trash. Moves nothing. See [`crate::trash::has_trash`].
    fn has_trash(&self, path: &Path) -> bool {
        crate::trash::has_trash(path)
    }

    /// Remove the file `path` for good.
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    /// Give the file `from` the new name `to` in one step that never
    /// replaces a file: fails with [`io::ErrorKind::AlreadyExists`] when
    /// `to` exists, with [`io::ErrorKind::CrossesDevices`] when the two
    /// names are on different drives, and with
    /// [`io::ErrorKind::Unsupported`] when the drive has no such step (the
    /// file is then where it was). See [`rename_new`].
    fn rename_new(&self, from: &Path, to: &Path) -> io::Result<()> {
        rename_new(from, to)
    }

    /// Copy the file `from` to the new file `to` and flush the copy to the
    /// disk. Fails with [`io::ErrorKind::AlreadyExists`] when `to` exists.
    /// Returns the number of bytes written. On failure a part of the copy
    /// can be left at `to`. See [`copy_new`].
    fn copy_new(&self, from: &Path, to: &Path) -> io::Result<u64> {
        copy_new(from, to)
    }
}

/// [`Disk`] through `std::fs`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdDisk;

impl FileAccess for StdDisk {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<pandite_library::FileInfo>> {
        StdFiles.list_dir(dir)
    }

    fn file_info(&self, path: &Path) -> io::Result<pandite_library::FileInfo> {
        StdFiles.file_info(path)
    }

    fn read(
        &self,
        path: &Path,
        cancel: &pandite_core::CancelToken,
    ) -> pandite_core::Result<pandite_core::Source> {
        StdFiles.read(path, cancel)
    }
}

impl Disk for StdDisk {}

/// Read at most `max` bytes from the start of `path`.
pub fn read_head(path: &Path, max: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::with_capacity(max.min(1 << 20));
    file.take(max as u64).read_to_end(&mut bytes)?;
    Ok(bytes)
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A hidden temporary name next to `path`, unique within this process.
pub fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "file".into());
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = format!(".{name}.{}-{n}.pandite-tmp", std::process::id());
    path.with_file_name(temp)
}

/// Replace `path` with `data` atomically. See [`write_atomic_with`].
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |file| file.write_all(data))
}

/// Replace `path` atomically with whatever `fill` writes.
///
/// The content goes to a new hidden file in the same folder, is flushed to
/// disk, and then takes the final name in one rename. If `fill`, the flush
/// or the rename fails, the temporary file is removed and `path` keeps its
/// old content. A rename refused because another program briefly holds the
/// target (virus scanners, indexers on Windows) is retried a few times.
pub fn write_atomic_with(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let temp = temp_path_for(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let written = fill(&mut file).and_then(|()| file.sync_all());
    drop(file);
    if let Err(err) = written {
        let _ = std::fs::remove_file(&temp);
        return Err(err);
    }
    let mut attempt = 0;
    loop {
        match std::fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied && attempt < 5 => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(30 * attempt));
            }
            Err(err) => {
                let _ = std::fs::remove_file(&temp);
                return Err(err);
            }
        }
    }
}

/// Rename `from` to `to` in one step that never replaces a file. See
/// [`Disk::rename_new`].
///
/// - Windows: `MoveFileExW` without the flag that replaces.
/// - Linux: `renameat2` with `RENAME_NOREPLACE`.
/// - macOS: `renamex_np` with `RENAME_EXCL`.
///
/// When Linux or macOS cannot do that on the drive (an old kernel, a file
/// system without the flag), a second link is made and the first removed;
/// a link never replaces a file either. A drive without links (FAT, some
/// network shares) answers [`io::ErrorKind::Unsupported`]: there is no
/// check followed by a plain rename, which could replace a file that
/// appears in between.
pub fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
    platform::rename_new(from, to)
}

/// Copy `from` to the new file `to` and flush it. See [`Disk::copy_new`].
pub fn copy_new(from: &Path, to: &Path) -> io::Result<u64> {
    let mut source = File::open(from)?;
    let mut target = OpenOptions::new().write(true).create_new(true).open(to)?;
    let n = io::copy(&mut source, &mut target)?;
    target.sync_all()?;
    Ok(n)
}

/// Move the file `from` into the folder `dir` under its own name and
/// return the new path. A file of that name in `dir` is never replaced:
/// the move then fails with [`io::ErrorKind::AlreadyExists`] and nothing
/// changes.
///
/// On the same drive this is a rename that never replaces a file (see
/// [`Disk::rename_new`]). Across drives, or on a drive without such a
/// rename, the file is copied to a temporary name in `dir`, flushed,
/// checked for its full size and given its final name by that rename. On
/// a drive without it, the copy is written under the final name itself,
/// created new, so a file that appears there in between stays and the move
/// fails. Only after a complete copy is the source removed. When a step
/// fails, the copy goes and the source stays, and a file that is not ours
/// is never removed.
pub fn move_into(disk: &dyn Disk, from: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = from
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the path has no file name"))?;
    let to = dir.join(name);
    // Only a quick answer for the common case; the steps below never
    // replace a file on their own.
    if disk.file_info(&to).is_ok() {
        return Err(already_there());
    }
    match disk.rename_new(from, &to) {
        Ok(()) => return Ok(to),
        Err(err)
            if matches!(
                err.kind(),
                io::ErrorKind::CrossesDevices | io::ErrorKind::Unsupported
            ) => {}
        Err(err) => return Err(err),
    }
    copy_then_remove(disk, from, &to)?;
    Ok(to)
}

/// Copy `from` to the free name `to` without ever replacing a file there,
/// then remove `from`. See [`move_into`].
fn copy_then_remove(disk: &dyn Disk, from: &Path, to: &Path) -> io::Result<()> {
    let size = disk.file_info(from)?.stamp.len;
    let complete = |path: &Path, written: u64| -> io::Result<()> {
        let on_disk = disk.file_info(path)?.stamp.len;
        if written == size && on_disk == size {
            Ok(())
        } else {
            Err(io::Error::other("the copy is not complete"))
        }
    };
    // A copy that failed to create its file (the name was taken) made
    // nothing to remove.
    let drop_copy = |path: &Path, err: &io::Error| {
        if err.kind() != io::ErrorKind::AlreadyExists {
            let _ = disk.remove_file(path);
        }
    };
    let temp = temp_path_for(to);
    if let Err(err) = disk
        .copy_new(from, &temp)
        .and_then(|written| complete(&temp, written))
    {
        drop_copy(&temp, &err);
        return Err(err);
    }
    match disk.rename_new(&temp, to) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::Unsupported => {
            // No rename here can promise not to replace: write the final
            // name itself, created new.
            let _ = disk.remove_file(&temp);
            if let Err(err) = disk
                .copy_new(from, to)
                .and_then(|written| complete(to, written))
            {
                drop_copy(to, &err);
                return Err(err);
            }
        }
        Err(err) => {
            let _ = disk.remove_file(&temp);
            return Err(err);
        }
    }
    if let Err(err) = disk.remove_file(from) {
        // The source stays, so the copy goes: the file is in one place.
        let _ = disk.remove_file(to);
        return Err(err);
    }
    Ok(())
}

/// The error of a move onto a name that is taken.
fn already_there() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "a file of that name is there")
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }

    /// The absolute path with the long path prefix, NUL-terminated.
    fn wide(path: &Path) -> io::Result<Vec<u16>> {
        let abs = std::path::absolute(path)?;
        let units: Vec<u16> = abs.as_os_str().encode_wide().collect();
        let mut out = crate::trash::with_verbatim(&units);
        out.push(0);
        Ok(out)
    }

    pub(super) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (wide(from)?, wide(to)?);
        // SAFETY: both paths are NUL-terminated and live for the call.
        // Flags 0: never replace, never copy across drives.
        let ok = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } != 0;
        if ok {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
mod platform {
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;

    fn c_path(path: &Path) -> io::Result<CString> {
        CString::new(path.as_os_str().as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "the path holds a NUL byte"))
    }

    /// The rename of this system that never replaces a file.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (c_path(from)?, c_path(to)?);
        // The system call itself, so no particular C library is needed.
        // SAFETY: both paths are NUL-terminated and live for the call; the
        // other arguments are plain integers.
        let r = unsafe {
            libc::syscall(
                libc::SYS_renameat2,
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if r == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// The rename of this system that never replaces a file.
    #[cfg(target_vendor = "apple")]
    fn rename_exclusive(from: &Path, to: &Path) -> io::Result<()> {
        let (from, to) = (c_path(from)?, c_path(to)?);
        // SAFETY: both paths are NUL-terminated and live for the call.
        let r = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        if r == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Other Unix systems have no such rename here; links still work.
    #[cfg(not(any(target_os = "linux", target_os = "android", target_vendor = "apple")))]
    fn rename_exclusive(_from: &Path, _to: &Path) -> io::Result<()> {
        Err(io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// True when `err` says the kernel or the file system cannot do the
    /// step at all (rather than that this file cannot go).
    fn not_supported(err: &io::Error) -> bool {
        // ENOTSUP and EOPNOTSUPP are one code on Linux and two on macOS.
        let codes = [libc::ENOSYS, libc::EINVAL, libc::ENOTSUP, libc::EOPNOTSUPP];
        err.raw_os_error().is_some_and(|code| codes.contains(&code))
    }

    pub(super) fn rename_new(from: &Path, to: &Path) -> io::Result<()> {
        match rename_exclusive(from, to) {
            Err(err) if not_supported(&err) => {}
            done => return done,
        }
        // A second link never replaces a file; then the first goes.
        match std::fs::hard_link(from, to) {
            Ok(()) => match std::fs::remove_file(from) {
                Ok(()) => Ok(()),
                Err(err) => {
                    let _ = std::fs::remove_file(to);
                    Err(err)
                }
            },
            // FAT answers EPERM: it has no links.
            Err(err) if not_supported(&err) || err.raw_os_error() == Some(libc::EPERM) => {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "this drive cannot rename a file without the risk of replacing another",
                ))
            }
            Err(err) => Err(err),
        }
    }
}

/// Every file under `dir` that `registry` can open, in natural order. With
/// `recursive`, subfolders are included (depth-first, folders in natural
/// order). Uses `files` for every listing.
pub fn list_images(
    files: &dyn FileAccess,
    registry: &Registry,
    dir: &Path,
    recursive: bool,
) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    let mut first = true;
    while let Some(folder) = stack.pop() {
        let entries = match files.list_dir(&folder) {
            Ok(entries) => entries,
            // The top folder must be readable; a broken subfolder is skipped.
            Err(err) if first => return Err(err),
            Err(_) => continue,
        };
        first = false;
        let mut names: Vec<_> = entries
            .into_iter()
            .map(|e| (e.name.to_string_lossy().into_owned(), e))
            .collect();
        names.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        let mut subdirs = Vec::new();
        for (_, entry) in names {
            let path = folder.join(&entry.name);
            if entry.is_file && registry.can_open_path(&path) {
                out.push(path);
            } else if entry.is_dir && recursive {
                subdirs.push(path);
            }
        }
        // Reverse so the first folder in natural order is processed first.
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pandite-app-disk-{tag}-{}-{}",
            std::process::id(),
            TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn read_head_stops_at_the_limit() {
        let dir = temp_dir("head");
        let path = dir.join("big.bin");
        std::fs::write(&path, vec![7u8; 5000]).unwrap();
        assert_eq!(read_head(&path, 100).unwrap().len(), 100);
        assert_eq!(read_head(&path, 1 << 20).unwrap().len(), 5000);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn moves_never_replace_a_file() {
        let dir = temp_dir("move");
        let (src, dst) = (dir.join("src"), dir.join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join("a.png"), b"new a").unwrap();
        std::fs::write(src.join("b.png"), b"new b").unwrap();
        std::fs::write(dst.join("b.png"), b"old b").unwrap();
        // A free name: the file moves.
        let moved = move_into(&StdDisk, &src.join("a.png"), &dst).unwrap();
        assert_eq!(moved, dst.join("a.png"));
        assert_eq!(std::fs::read(&moved).unwrap(), b"new a");
        assert!(!src.join("a.png").exists());
        // A taken name: nothing changes on either side.
        let err = move_into(&StdDisk, &src.join("b.png"), &dst).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(src.join("b.png")).unwrap(), b"new b");
        assert_eq!(std::fs::read(dst.join("b.png")).unwrap(), b"old b");
        // The rename itself refuses a taken name too.
        let err = rename_new(&src.join("b.png"), &dst.join("b.png")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(dst.join("b.png")).unwrap(), b"old b");
        // A copy never writes over a file either.
        let err = copy_new(&src.join("b.png"), &dst.join("b.png")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(dst.join("b.png")).unwrap(), b"old b");
        assert_eq!(copy_new(&src.join("b.png"), &dst.join("c.png")).unwrap(), 5);
        // In one folder too: the rename of this system refuses the name of
        // another file and leaves both as they were.
        std::fs::write(src.join("x.png"), b"x").unwrap();
        std::fs::write(src.join("y.png"), b"y").unwrap();
        let err = rename_new(&src.join("x.png"), &src.join("y.png")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(src.join("x.png")).unwrap(), b"x");
        assert_eq!(std::fs::read(src.join("y.png")).unwrap(), b"y");
        rename_new(&src.join("x.png"), &src.join("z.png")).unwrap();
        assert!(!src.join("x.png").exists());
        assert_eq!(std::fs::read(src.join("z.png")).unwrap(), b"x");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn list_images_filters_sorts_and_recurses() {
        let dir = temp_dir("list");
        for name in ["img10.png", "img2.png", "notes.txt", "IMG1.JPG"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub").join("deep.gif"), b"x").unwrap();
        let reg = pandite_codecs::default_registry();
        let flat = list_images(&StdFiles, &reg, &dir, false).unwrap();
        let names: Vec<_> = flat
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["IMG1.JPG", "img2.png", "img10.png"]);
        let deep = list_images(&StdFiles, &reg, &dir, true).unwrap();
        assert_eq!(deep.len(), 4);
        assert!(deep[3].ends_with(Path::new("sub").join("deep.gif")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

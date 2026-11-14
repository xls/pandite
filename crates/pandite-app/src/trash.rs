//! Move a file to the trash of the system.
//!
//! [`trash`] is the one entry point. It blocks for as long as the system
//! takes: call it on a worker thread. When the drive or volume of the file
//! has no trash, [`trash`] refuses before it touches the file, with an
//! error that [`is_no_trash`] recognizes; the caller decides whether to
//! delete the file for good instead. [`has_trash`] asks the same question
//! without moving anything.
//!
//! - Windows: the Recycle Bin through `SHFileOperationW`. Only fixed
//!   drives have a Recycle Bin; on a network, removable or optical drive
//!   [`trash`] refuses instead of letting Windows delete the file for good,
//!   and the shell is never called for such a file.
//!   When Windows cannot recycle a file on a fixed drive (the file is
//!   larger than the Recycle Bin, or the Recycle Bin is turned off), it
//!   asks the user before it deletes the file for good.
//! - macOS: the Trash through `NSFileManager trashItemAtURL`. A volume
//!   without a Trash makes that call fail, which [`trash`] reports as no
//!   trash. [`has_trash`] cannot tell in advance and answers true.
//! - Other Unix systems: the FreeDesktop Trash specification 1.0. A file
//!   on the device of the home trash (`$XDG_DATA_HOME/Trash`) goes there;
//!   a file on another mount goes to `$topdir/.Trash/$uid` or
//!   `$topdir/.Trash-$uid` at the top of its volume. A volume where
//!   neither can exist has no trash.
//!
//! The text and name rules of these formats are plain functions with unit
//! tests on every system.

use std::io;
use std::path::Path;

/// The trash as messages name it on this system.
pub const TRASH_NAME: &str = if cfg!(windows) {
    "the Recycle Bin"
} else {
    "the Trash"
};

/// The name of the trash on this system without the article.
pub const TRASH_TITLE: &str = if cfg!(windows) {
    "Recycle Bin"
} else {
    "Trash"
};

/// The cause inside the error of [`no_trash`].
#[derive(Debug)]
struct NoTrash(&'static str);

impl std::fmt::Display for NoTrash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for NoTrash {}

/// The error for a file whose drive or volume has no trash, with `why` as
/// its text. Nothing happened to the file. Its kind is
/// [`io::ErrorKind::Unsupported`]; [`is_no_trash`] tells it apart from
/// other errors of that kind.
pub fn no_trash(why: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, NoTrash(why))
}

/// True when `err` came from [`no_trash`]: the drive or volume of the file
/// has no trash and the file is where it was.
pub fn is_no_trash(err: &io::Error) -> bool {
    err.get_ref().is_some_and(|e| e.is::<NoTrash>())
}

/// True unless the drive or volume of the file `path` is known to have no
/// trash, so that [`trash`] would refuse it with [`no_trash`]. Creates and
/// moves nothing. It blocks for as long as the system takes (a network
/// drive): call it on a worker thread.
pub fn has_trash(path: &Path) -> bool {
    match std::path::absolute(path) {
        Ok(path) => platform::has_trash(&path),
        Err(_) => true,
    }
}

/// Move the file `path` to the trash of the system. A symbolic link moves
/// itself, not its target. A folder is refused with
/// [`io::ErrorKind::InvalidInput`], a missing file reports
/// [`io::ErrorKind::NotFound`]. On success the path no longer exists.
pub fn trash(path: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "this is a folder, not a file",
        ));
    }
    let path = std::path::absolute(path)?;
    platform::trash(&path)?;
    match std::fs::symlink_metadata(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        _ => Err(io::Error::other("the file is still in its folder")),
    }
}

fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, text.to_string())
}

// ----- text and name rules ---------------------------------------------------

/// Percent-encode a path for the `Path` key of a trash info file: every
/// byte except ASCII letters, digits, `-`, `.`, `_`, `~` and `/` becomes
/// `%XX` with upper-case hex digits.
#[cfg_attr(any(windows, target_vendor = "apple"), allow(dead_code))]
pub(crate) fn percent_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 15)]));
        }
    }
    out
}

/// The text of a trash info file for the file at `path` (absolute for the
/// home trash, relative to the top of the volume otherwise), deleted at
/// `date` (see [`deletion_date`]).
#[cfg_attr(any(windows, target_vendor = "apple"), allow(dead_code))]
pub(crate) fn trash_info(path: &[u8], date: &str) -> String {
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={date}\n",
        percent_encode(path)
    )
}

/// `YYYY-MM-DDThh:mm:ss` for `secs` seconds after 1970-01-01 00:00:00 in
/// the time zone that is `offset` seconds ahead of UTC.
#[cfg_attr(any(windows, target_vendor = "apple"), allow(dead_code))]
pub(crate) fn deletion_date(secs: i64, offset: i64) -> String {
    let local = secs.saturating_add(offset);
    let days = local.div_euclid(86_400);
    let rest = local.rem_euclid(86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        rest / 3600,
        rest / 60 % 60,
        rest % 60
    )
}

/// The name of copy `n` of the file name `name` in the trash: the name
/// itself for copy 1, then `stem.N.ext` (`photo.2.jpg`, `photo.3.jpg`,
/// ...). A leading dot does not start an extension.
#[cfg_attr(any(windows, target_vendor = "apple"), allow(dead_code))]
pub(crate) fn copy_name(name: &[u8], n: u32) -> Vec<u8> {
    if n <= 1 {
        return name.to_vec();
    }
    let dot = name
        .iter()
        .rposition(|&b| b == b'.')
        .filter(|&at| at > 0 && at + 1 < name.len());
    let (stem, ext) = match dot {
        Some(at) => name.split_at(at),
        None => (name, &[][..]),
    };
    let mut out = stem.to_vec();
    out.extend_from_slice(format!(".{n}").as_bytes());
    out.extend_from_slice(ext);
    out
}

/// `wide` (a path in UTF-16) without the `\\?\` prefix Windows uses for
/// long paths: `\\?\C:\a` becomes `C:\a`, `\\?\UNC\srv\a` becomes
/// `\\srv\a`. The shell file functions do not take the prefix.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn without_verbatim(wide: &[u16]) -> Vec<u16> {
    let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let unc = units(r"\\?\UNC\");
    let plain = units(r"\\?\");
    if wide.len() >= unc.len() && same_ascii_folded(&wide[..unc.len()], &unc) {
        let mut out = units(r"\\");
        out.extend_from_slice(&wide[unc.len()..]);
        out
    } else if wide.starts_with(&plain) {
        wide[plain.len()..].to_vec()
    } else {
        wide.to_vec()
    }
}

/// `wide` (an absolute path in UTF-16) with the `\\?\` prefix that lets
/// the file functions of Windows take a path longer than 260 characters.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn with_verbatim(wide: &[u16]) -> Vec<u16> {
    let units = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let plain = units(r"\\?\");
    if wide.starts_with(&plain) {
        return wide.to_vec();
    }
    let unc_start = units(r"\\");
    let mut out;
    if wide.starts_with(&unc_start) {
        out = units(r"\\?\UNC\");
        out.extend_from_slice(&wide[2..]);
    } else {
        out = plain;
        out.extend_from_slice(wide);
    }
    out
}

/// The list of one path that `SHFileOperationW` takes: the path without
/// the `\\?\` prefix, ended by two NUL units. `None` when the path is
/// empty or holds a NUL.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn shell_path_list(wide: &[u16]) -> Option<Vec<u16>> {
    let mut list = without_verbatim(wide);
    if list.is_empty() || list.contains(&0) {
        return None;
    }
    list.extend_from_slice(&[0, 0]);
    Some(list)
}

/// True when two runs of UTF-16 units are equal, ignoring ASCII case.
#[cfg_attr(not(windows), allow(dead_code))]
fn same_ascii_folded(a: &[u16], b: &[u16]) -> bool {
    let fold = |u: u16| match u8::try_from(u) {
        Ok(byte) => u16::from(byte.to_ascii_lowercase()),
        Err(_) => u,
    };
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| fold(x) == fold(y))
}

// ----- Windows -----------------------------------------------------------------

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::{invalid, no_trash, shell_path_list, with_verbatim, without_verbatim};
    use std::ffi::c_void;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    const FO_DELETE: u32 = 3;
    const FOF_SILENT: u16 = 0x0004;
    const FOF_NOCONFIRMATION: u16 = 0x0010;
    const FOF_ALLOWUNDO: u16 = 0x0040;
    const FOF_NOERRORUI: u16 = 0x0400;
    /// Ask before a file that cannot be recycled is deleted for good. It
    /// overrides `FOF_NOCONFIRMATION` for that case only.
    const FOF_WANTNUKEWARNING: u16 = 0x4000;
    const FLAGS: u16 =
        FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING;
    /// Longest path the shell file functions take, without the final NUL.
    const MAX_PATH: usize = 259;
    const DRIVE_FIXED: u32 = 3;
    const COINIT_APARTMENTTHREADED: u32 = 0x2;
    const COINIT_DISABLE_OLE1DDE: u32 = 0x4;

    /// SHFILEOPSTRUCTW. The header packs it to one byte on 32-bit Windows.
    #[cfg_attr(target_pointer_width = "64", repr(C))]
    #[cfg_attr(not(target_pointer_width = "64"), repr(C, packed(1)))]
    struct ShFileOp {
        hwnd: isize,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        any_aborted: i32,
        name_mappings: *mut c_void,
        progress_title: *const u16,
    }

    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOp) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetVolumePathNameW(file: *const u16, out: *mut u16, len: u32) -> i32;
        fn GetDriveTypeW(root: *const u16) -> u32;
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }

    #[link(name = "ole32")]
    extern "system" {
        fn CoInitializeEx(reserved: *const c_void, coinit: u32) -> i32;
        fn CoUninitialize();
    }

    /// COM on this thread for the shell, released on drop.
    struct Com(bool);

    impl Com {
        fn init() -> Com {
            // SAFETY: a null reserved pointer is required; the call takes
            // plain flags. S_OK and S_FALSE both need a CoUninitialize.
            let hr = unsafe {
                CoInitializeEx(
                    std::ptr::null(),
                    COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE,
                )
            };
            Com(hr >= 0)
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: balances the successful CoInitializeEx of this
                // thread.
                unsafe { CoUninitialize() };
            }
        }
    }

    fn nul_terminated(wide: &[u16]) -> Vec<u16> {
        let mut out = wide.to_vec();
        out.push(0);
        out
    }

    /// Refuse a drive without a Recycle Bin, where Windows would delete the
    /// file for good. A drive whose type cannot be found is let through:
    /// Windows asks before it deletes for good.
    fn check_drive(wide: &[u16]) -> io::Result<()> {
        if drive_has_bin(wide) {
            Ok(())
        } else {
            Err(no_trash("this drive has no Recycle Bin"))
        }
    }

    pub(super) fn has_trash(path: &Path) -> bool {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        drive_has_bin(&wide)
    }

    /// False for a drive that is not fixed (network, removable, optical,
    /// RAM disk); true for a fixed drive and for a drive of unknown type.
    fn drive_has_bin(wide: &[u16]) -> bool {
        let input = nul_terminated(wide);
        let mut volume = vec![0u16; (wide.len() + 2).max(MAX_PATH + 2)];
        // SAFETY: `input` is NUL-terminated and `volume` is valid for
        // writes of the length passed.
        let ok =
            unsafe { GetVolumePathNameW(input.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) }
                != 0;
        if !ok {
            return true;
        }
        let end = volume.iter().position(|&u| u == 0).unwrap_or(volume.len());
        let root = nul_terminated(&without_verbatim(&volume[..end]));
        // SAFETY: `root` is a NUL-terminated root path.
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        kind == DRIVE_FIXED
    }

    /// The 8.3 form of a path longer than the shell takes, as a path list.
    fn short_list(wide: &[u16]) -> io::Result<Vec<u16>> {
        let too_long = || invalid("the path is too long for the Recycle Bin");
        let long = nul_terminated(&with_verbatim(wide));
        let mut short = vec![0u16; long.len()];
        // SAFETY: `long` is NUL-terminated and `short` is valid for writes
        // of the length passed.
        let n = unsafe { GetShortPathNameW(long.as_ptr(), short.as_mut_ptr(), short.len() as u32) }
            as usize;
        if n == 0 || n >= short.len() {
            return Err(too_long());
        }
        let list = shell_path_list(&short[..n]).ok_or_else(too_long)?;
        if list.len() - 2 > MAX_PATH {
            return Err(too_long());
        }
        Ok(list)
    }

    fn error(code: i32, aborted: bool) -> io::Error {
        use io::ErrorKind as K;
        let (kind, text) = match code as u32 {
            0 if aborted => (K::Interrupted, "the delete was cancelled"),
            2 | 3 => (K::NotFound, "the file no longer exists"),
            5 | 0x78 => (K::PermissionDenied, "access is denied"),
            19 => (K::ReadOnlyFilesystem, "the disk is write-protected"),
            32 | 33 => (K::ResourceBusy, "another program is using the file"),
            0x75 | 1223 => (K::Interrupted, "the delete was cancelled"),
            0x79 | 0x81 | 206 => (K::InvalidInput, "the path is too long"),
            other => {
                return io::Error::other(format!("Windows reported error 0x{other:X}"));
            }
        };
        io::Error::new(kind, text)
    }

    pub(super) fn trash(path: &Path) -> io::Result<()> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        check_drive(&wide)?;
        let mut list = shell_path_list(&wide).ok_or_else(|| invalid("this is not a valid path"))?;
        if list.len() - 2 > MAX_PATH {
            list = short_list(&wide)?;
        }
        let _com = Com::init();
        let mut op = ShFileOp {
            hwnd: 0,
            func: FO_DELETE,
            from: list.as_ptr(),
            to: std::ptr::null(),
            flags: FLAGS,
            any_aborted: 0,
            name_mappings: std::ptr::null_mut(),
            progress_title: std::ptr::null(),
        };
        // SAFETY: `op` is a valid SHFILEOPSTRUCTW; `from` is a
        // double-NUL terminated list that outlives the call; the other
        // pointers are null, which the function allows for a delete.
        let code = unsafe { SHFileOperationW(&mut op) };
        // A copy of the field: the struct is packed on 32-bit Windows.
        let aborted = { op.any_aborted } != 0;
        if code == 0 && !aborted {
            Ok(())
        } else {
            Err(error(code, aborted))
        }
    }
}

// ----- macOS -------------------------------------------------------------------

#[cfg(target_vendor = "apple")]
#[allow(unsafe_code)]
mod platform {
    use super::{invalid, no_trash};
    use objc2_foundation::{NSFileManager, NSURL};
    use std::ffi::CString;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::path::Path;
    use std::ptr::NonNull;

    /// NSFileNoSuchFileError.
    const NO_SUCH_FILE: isize = 4;
    /// NSFileReadNoPermissionError.
    const READ_NO_PERMISSION: isize = 257;
    /// NSFileWriteNoPermissionError.
    const WRITE_NO_PERMISSION: isize = 513;
    /// NSFileWriteVolumeReadOnlyError.
    const VOLUME_READ_ONLY: isize = 642;
    /// NSFeatureUnsupportedError: the volume has no trash.
    const UNSUPPORTED: isize = 3328;

    /// The Trash of a volume shows only when the move fails.
    pub(super) fn has_trash(_path: &Path) -> bool {
        true
    }

    pub(super) fn trash(path: &Path) -> io::Result<()> {
        let bytes = CString::new(path.as_os_str().as_bytes())
            .map_err(|_| invalid("the path holds a NUL byte"))?;
        objc2::rc::autoreleasepool(|_| {
            let ptr = NonNull::new(bytes.as_ptr().cast_mut())
                .ok_or_else(|| invalid("this is not a valid path"))?;
            // SAFETY: `ptr` points to a NUL-terminated file system path
            // that lives until the call returns; NSURL copies it.
            let url = unsafe {
                NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                    ptr, false, None,
                )
            };
            NSFileManager::defaultManager()
                .trashItemAtURL_resultingItemURL_error(&url, None)
                .map_err(|err| {
                    if err.code() == UNSUPPORTED {
                        return no_trash("this volume has no Trash");
                    }
                    let kind = match err.code() {
                        NO_SUCH_FILE => io::ErrorKind::NotFound,
                        READ_NO_PERMISSION | WRITE_NO_PERMISSION => io::ErrorKind::PermissionDenied,
                        VOLUME_READ_ONLY => io::ErrorKind::ReadOnlyFilesystem,
                        _ => io::ErrorKind::Other,
                    };
                    io::Error::new(kind, err.localizedDescription().to_string())
                })
        })
    }
}

// ----- FreeDesktop -------------------------------------------------------------

#[cfg(all(unix, not(target_vendor = "apple")))]
#[allow(unsafe_code)]
mod platform {
    use super::{copy_name, deletion_date, invalid, no_trash, trash_info};
    use std::ffi::OsStr;
    use std::fs::{self, DirBuilder, OpenOptions};
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
    use std::path::{Path, PathBuf};

    /// Names tried in one trash before giving up.
    const MAX_COPIES: u32 = 10_000;

    pub(super) fn trash(path: &Path) -> io::Result<()> {
        // SAFETY: getuid takes nothing, touches no memory and cannot fail.
        let uid = unsafe { libc::getuid() };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        let date = deletion_date(now, local_offset(now));
        trash_with(path, &home_trash()?, uid, &date)
    }

    pub(super) fn has_trash(path: &Path) -> bool {
        // SAFETY: getuid takes nothing, touches no memory and cannot fail.
        let uid = unsafe { libc::getuid() };
        match home_trash() {
            Ok(home) => has_trash_with(path, &home, uid),
            Err(_) => false,
        }
    }

    /// True when [`trash_with`] can find or make a trash for `path`: the
    /// home trash `home` on the same device, or a trash of the volume that
    /// exists or that this user can make. Makes nothing. A file that
    /// cannot be looked at counts as having a trash: the move then says
    /// what is wrong.
    pub(crate) fn has_trash_with(path: &Path, home: &Path, uid: u32) -> bool {
        let Some(name) = path.file_name() else {
            return true;
        };
        let Ok(parent) = fs::canonicalize(path.parent().unwrap_or(Path::new("/"))) else {
            return true;
        };
        let Ok(meta) = fs::symlink_metadata(parent.join(name)) else {
            return true;
        };
        let dev = meta.dev();
        if device(home).is_ok_and(|d| d == dev) {
            return true;
        }
        let top = volume_top(&parent, dev);
        let shared = top.join(".Trash");
        let sticky =
            fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir() && m.mode() & 0o1000 != 0);
        let mine = shared.join(uid.to_string());
        if sticky && (own_dir(&mine, uid) || (!exists(&mine) && writable(&shared))) {
            return true;
        }
        let mine = top.join(format!(".Trash-{uid}"));
        own_dir(&mine, uid) || (!exists(&mine) && writable(&top))
    }

    fn exists(path: &Path) -> bool {
        fs::symlink_metadata(path).is_ok()
    }

    /// True when this process may make entries in the folder `dir`.
    fn writable(dir: &Path) -> bool {
        let Ok(text) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
            return false;
        };
        // SAFETY: `text` is a NUL-terminated path that lives for the call.
        unsafe { libc::access(text.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
    }

    /// Seconds the local time zone is ahead of UTC at `secs`; 0 when the
    /// system cannot tell.
    // `tm_gmtoff` is a `c_long`: 32 bits on some targets, 64 on others.
    #[allow(clippy::useless_conversion)]
    fn local_offset(secs: i64) -> i64 {
        // A 32-bit time_t wraps after 2038; the offset is still right.
        let t = secs as libc::time_t;
        // SAFETY: `tm` is plain data, for which all zero bytes are a value.
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: both pointers are valid for the call; the thread-safe
        // localtime_r writes `tm` only.
        if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return 0;
        }
        i64::from(tm.tm_gmtoff)
    }

    /// `$XDG_DATA_HOME/Trash`, or `$HOME/.local/share/Trash`.
    fn home_trash() -> io::Result<PathBuf> {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute());
        if let Some(data) = data {
            return Ok(data.join("Trash"));
        }
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| no_trash("HOME is not set, so there is no trash"))?;
        Ok(home.join(".local/share/Trash"))
    }

    /// The device of `path`, or of its nearest ancestor that exists.
    fn device(path: &Path) -> io::Result<u64> {
        let mut last = None;
        for p in path.ancestors() {
            match fs::metadata(p) {
                Ok(meta) => return Ok(meta.dev()),
                Err(err) => last = Some(err),
            }
        }
        Err(last.unwrap_or_else(|| invalid("no folder of the trash exists")))
    }

    /// Create `dir` and the folders above it, readable by the owner only.
    fn make_dirs(dir: &Path) -> io::Result<()> {
        DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }

    /// True when `dir` is a real folder (not a link) owned by `uid`.
    fn own_dir(dir: &Path, uid: u32) -> bool {
        fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && m.uid() == uid)
    }

    /// The highest folder above `dir` (which is on `dev`) that is still on
    /// `dev`: the top of the volume.
    fn volume_top(dir: &Path, dev: u64) -> PathBuf {
        let mut top = dir.to_path_buf();
        while let Some(up) = top.parent() {
            if fs::metadata(up).map_or(true, |m| m.dev() != dev) {
                break;
            }
            top = up.to_path_buf();
        }
        top
    }

    /// The trash of the volume whose top is `top`: `$top/.Trash/$uid` when
    /// the administrator made `$top/.Trash` (a sticky folder, not a link),
    /// otherwise `$top/.Trash-$uid`.
    fn volume_trash(top: &Path, uid: u32) -> io::Result<PathBuf> {
        let shared = top.join(".Trash");
        let sticky =
            fs::symlink_metadata(&shared).is_ok_and(|m| m.is_dir() && m.mode() & 0o1000 != 0);
        if sticky {
            let mine = shared.join(uid.to_string());
            let _ = DirBuilder::new().mode(0o700).create(&mine);
            if own_dir(&mine, uid) {
                return Ok(mine);
            }
        }
        let mine = top.join(format!(".Trash-{uid}"));
        let _ = DirBuilder::new().mode(0o700).create(&mine);
        if own_dir(&mine, uid) {
            Ok(mine)
        } else {
            Err(no_trash("this volume has no trash"))
        }
    }

    /// Move `path` into the home trash `home` when it is on the same
    /// device, otherwise into the trash of its volume. `date` is the
    /// deletion date text. The info file is made first, with `O_EXCL`, under
    /// the first free name; then the file moves.
    pub(crate) fn trash_with(path: &Path, home: &Path, uid: u32, date: &str) -> io::Result<()> {
        let name = path
            .file_name()
            .ok_or_else(|| invalid("the path has no file name"))?;
        let parent = fs::canonicalize(path.parent().unwrap_or(Path::new("/")))?;
        let file = parent.join(name);
        let dev = fs::symlink_metadata(&file)?.dev();
        let (dir, recorded) = if device(home)? == dev {
            make_dirs(home)?;
            (home.to_path_buf(), file.as_os_str().as_bytes().to_vec())
        } else {
            let top = volume_top(&parent, dev);
            let dir = volume_trash(&top, uid)?;
            let relative = file.strip_prefix(&top).unwrap_or(&file);
            (dir, relative.as_os_str().as_bytes().to_vec())
        };
        let (info_dir, files_dir) = (dir.join("info"), dir.join("files"));
        make_dirs(&info_dir)?;
        make_dirs(&files_dir)?;
        let text = trash_info(&recorded, date);
        for n in 1..=MAX_COPIES {
            let trashed = copy_name(name.as_bytes(), n);
            let mut info_name = trashed.clone();
            info_name.extend_from_slice(b".trashinfo");
            let info = info_dir.join(OsStr::from_bytes(&info_name));
            let mut out = match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&info)
            {
                Ok(out) => out,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            };
            let target = files_dir.join(OsStr::from_bytes(&trashed));
            if fs::symlink_metadata(&target).is_ok() {
                // A file without its info file holds this name.
                drop(out);
                let _ = fs::remove_file(&info);
                continue;
            }
            let written = out.write_all(text.as_bytes()).and_then(|()| out.sync_all());
            drop(out);
            let moved = written.and_then(|()| fs::rename(&file, &target));
            if moved.is_err() {
                let _ = fs::remove_file(&info);
            }
            return moved;
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "the trash holds too many files of this name",
        ))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn temp(tag: &str) -> PathBuf {
            let dir =
                std::env::temp_dir().join(format!("pandite-trash-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            dir
        }

        #[test]
        fn freedesktop_home_trash_names_and_info() {
            let root = temp("home");
            let home = root.join("data").join("Trash");
            let folder = root.join("photos dir");
            fs::create_dir_all(&folder).unwrap();
            // SAFETY: getuid cannot fail.
            let uid = unsafe { libc::getuid() };
            for round in 1..=3u32 {
                let file = folder.join("a b.png");
                fs::write(&file, format!("round {round}")).unwrap();
                trash_with(&file, &home, uid, "2026-11-14T10:20:30").unwrap();
                assert!(!file.exists());
                let trashed = match round {
                    1 => "a b.png".to_string(),
                    n => format!("a b.{n}.png"),
                };
                let moved = home.join("files").join(&trashed);
                assert_eq!(
                    fs::read_to_string(&moved).unwrap(),
                    format!("round {round}")
                );
                let info = home.join("info").join(format!("{trashed}.trashinfo"));
                let text = fs::read_to_string(info).unwrap();
                let canonical = fs::canonicalize(&folder).unwrap().join("a b.png");
                let expected = trash_info(canonical.as_os_str().as_bytes(), "2026-11-14T10:20:30");
                assert_eq!(text, expected);
                assert!(text.contains("%20b.png"), "{text}");
            }
            // A folder is refused before anything moves.
            assert!(super::super::trash(&folder).is_err());
            // Beside the home trash, every file has a trash.
            let file = folder.join("c.png");
            fs::write(&file, "c").unwrap();
            assert!(has_trash_with(&file, &home, uid));
            assert!(file.exists(), "the question moves nothing");
            fs::remove_dir_all(&root).unwrap();
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use std::io;
    use std::path::Path;

    pub(super) fn has_trash(_path: &Path) -> bool {
        false
    }

    pub(super) fn trash(_path: &Path) -> io::Result<()> {
        Err(super::no_trash("this system has no trash pandite knows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn no_trash_is_its_own_outcome() {
        let err = no_trash("this drive has no Recycle Bin");
        assert!(is_no_trash(&err));
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(err.to_string(), "this drive has no Recycle Bin");
        // Other errors, also of the same kind or with the same text, are not.
        for other in [
            io::Error::new(io::ErrorKind::Unsupported, "this drive has no Recycle Bin"),
            io::Error::from(io::ErrorKind::Unsupported),
            io::Error::new(io::ErrorKind::PermissionDenied, "access is denied"),
        ] {
            assert!(!is_no_trash(&other), "{other}");
        }
    }

    #[test]
    fn percent_encoding_keeps_only_safe_bytes() {
        assert_eq!(percent_encode(b"/home/u/a-b_c.d~e"), "/home/u/a-b_c.d~e");
        assert_eq!(percent_encode(b"/x/a b%.png"), "/x/a%20b%25.png");
        assert_eq!(
            percent_encode("/f/\u{e4}\u{1F600}#?&=+.jpg".as_bytes()),
            "/f/%C3%A4%F0%9F%98%80%23%3F%26%3D%2B.jpg"
        );
        // Bytes that are not UTF-8 encode as they are.
        assert_eq!(percent_encode(&[b'/', 0xFF, 0x00, b'\n']), "/%FF%00%0A");
    }

    #[test]
    fn trash_info_text() {
        assert_eq!(
            trash_info(b"/home/u/My Photos/a.png", "2026-11-14T08:05:09"),
            "[Trash Info]\nPath=/home/u/My%20Photos/a.png\nDeletionDate=2026-11-14T08:05:09\n"
        );
        // Relative to the top of a volume.
        assert!(trash_info(b"pics/b.jpg", "2000-01-01T00:00:00").contains("\nPath=pics/b.jpg\n"));
    }

    #[test]
    fn deletion_dates() {
        assert_eq!(deletion_date(0, 0), "1970-01-01T00:00:00");
        assert_eq!(deletion_date(951_782_400, 0), "2000-02-29T00:00:00");
        assert_eq!(deletion_date(1_794_646_809, 0), "2026-11-14T09:00:09");
        // An hour ahead of UTC moves past midnight.
        assert_eq!(deletion_date(1_794_700_799, 3600), "2026-11-15T00:59:59");
        assert_eq!(deletion_date(-1, 0), "1969-12-31T23:59:59");
    }

    #[test]
    fn copy_names_keep_the_extension() {
        let name = |s: &str, n| String::from_utf8(copy_name(s.as_bytes(), n)).unwrap();
        assert_eq!(name("photo.jpg", 1), "photo.jpg");
        assert_eq!(name("photo.jpg", 2), "photo.2.jpg");
        assert_eq!(name("photo.tar.gz", 3), "photo.tar.3.gz");
        assert_eq!(name("README", 2), "README.2");
        assert_eq!(name(".hidden", 2), ".hidden.2");
        assert_eq!(name("dot.", 2), "dot..2");
        // Every copy has its own name.
        let names: std::collections::HashSet<_> = (1..50).map(|n| copy_name(b"a.png", n)).collect();
        assert_eq!(names.len(), 49);
    }

    #[test]
    fn shell_path_lists_end_with_two_nuls() {
        let list = shell_path_list(&wide(r"C:\pics\a.png")).unwrap();
        assert_eq!(list, [wide(r"C:\pics\a.png"), vec![0, 0]].concat());
        // Non-ASCII names keep their UTF-16 units, surrogate pairs too.
        let name = "C:\\Bilder\\\u{e4}\u{1F600}.png";
        let list = shell_path_list(&wide(name)).unwrap();
        assert_eq!(&list[..list.len() - 2], &wide(name)[..]);
        assert_eq!(list.iter().filter(|&&u| u == 0).count(), 2);
        // The long path prefix goes, local and network.
        assert_eq!(
            shell_path_list(&wide(r"\\?\D:\x\y.jpg")).unwrap(),
            [wide(r"D:\x\y.jpg"), vec![0, 0]].concat()
        );
        assert_eq!(
            shell_path_list(&wide(r"\\?\UNC\server\share\y.jpg")).unwrap(),
            [wide(r"\\server\share\y.jpg"), vec![0, 0]].concat()
        );
        assert_eq!(
            without_verbatim(&wide(r"\\?\unc\server\s\y.jpg")),
            wide(r"\\server\s\y.jpg")
        );
        // A NUL inside, or nothing at all, is no list.
        assert!(shell_path_list(&[u16::from(b'a'), 0, u16::from(b'b')]).is_none());
        assert!(shell_path_list(&[]).is_none());
    }

    #[test]
    fn long_path_prefix_round_trip() {
        assert_eq!(with_verbatim(&wide(r"C:\a")), wide(r"\\?\C:\a"));
        assert_eq!(with_verbatim(&wide(r"\\srv\s\a")), wide(r"\\?\UNC\srv\s\a"));
        assert_eq!(with_verbatim(&wide(r"\\?\C:\a")), wide(r"\\?\C:\a"));
        let long = format!(r"C:\{}\a.png", "d".repeat(300));
        assert_eq!(without_verbatim(&with_verbatim(&wide(&long))), wide(&long));
    }
}

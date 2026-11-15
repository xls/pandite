//! The macOS App Sandbox: folders the user allows, kept across sessions.
//!
//! Inside the App Sandbox (the Mac App Store build) the program reads and
//! writes only its own container, the files and folders the user chose in
//! a system panel or opened from Finder, and folders for which it holds a
//! security-scoped bookmark from an earlier grant. An image opened from
//! Finder comes without its folder: the folder listing fails with
//! "permission denied", and the viewer shows that one image.
//!
//! [`active`] tells whether the process runs in the sandbox. Only then does
//! the shell make a [`Store`]: it reads the grants file beside the settings
//! file on a worker, resolves every bookmark and starts access to its
//! folder. The application core asks the user to allow a refused folder
//! (see [`Session`]) and keeps a bookmark for each folder the user allows
//! and for each Move to and Copy to folder ([`Grants::add`]). Outside the
//! sandbox none of this runs: no store, no grants file, no question.
//!
//! The calls to macOS sit behind [`Bookmarks`]; everything else here is
//! plain Rust and runs in the tests with a fake.

use crate::disk::{self, Disk};
use pandite_library::{FileAccess, FileInfo, StdFiles};
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// The environment variable that macOS sets in every process of the App
/// Sandbox.
pub const CONTAINER_VAR: &str = "APP_SANDBOX_CONTAINER_ID";

/// Name of the grants file, in the folder of the settings file.
pub const GRANTS_FILE: &str = "grants";

/// First line of the grants file. Each further line is one bookmark in
/// hexadecimal digits.
const HEADER: &str = "pandite grants 1";

/// Most grants kept. A new grant beyond it drops the oldest.
pub const MAX_GRANTS: usize = 64;

/// The prompt that asks for a refused folder.
pub const ALLOW_MESSAGE: &str = "Allow Pandite to show the other images in this folder";

/// The button of the prompt.
pub const ALLOW_BUTTON: &str = "Allow...";

/// The title of the folder picker that the button opens.
pub const ALLOW_TITLE: &str = "Allow access to this folder";

/// True when this process runs in the macOS App Sandbox. One environment
/// lookup on macOS; always false elsewhere.
pub fn active() -> bool {
    cfg!(target_os = "macos") && sandboxed(std::env::var_os(CONTAINER_VAR).as_deref())
}

/// [`active`] for this value of [`CONTAINER_VAR`]: true on macOS when it is
/// set and not empty.
pub fn sandboxed(container: Option<&OsStr>) -> bool {
    cfg!(target_os = "macos") && container.is_some_and(|c| !c.is_empty())
}

/// One path component as the folder comparisons see it: without case on
/// macOS and Windows, whose volumes usually ignore it.
fn fold(c: Component<'_>) -> String {
    let text = c.as_os_str().to_string_lossy();
    if cfg!(any(windows, target_os = "macos")) {
        text.to_lowercase()
    } else {
        text.into_owned()
    }
}

/// True when `path` is the folder `dir` or lies inside it. Compares the
/// paths component by component without touching the disk. An empty `dir`
/// contains nothing.
pub fn contains(dir: &Path, path: &Path) -> bool {
    if dir.components().next().is_none() {
        return false;
    }
    let mut inner = path.components().map(fold);
    dir.components()
        .map(fold)
        .all(|c| inner.next().is_some_and(|i| i == c))
}

/// True when `a` and `b` name the same folder (see [`contains`]).
fn same_dir(a: &Path, b: &Path) -> bool {
    contains(a, b) && contains(b, a)
}

// ----- bookmarks ------------------------------------------------------------------

/// A bookmark resolved by [`Bookmarks::resolve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// The folder the bookmark names now (it follows a moved folder).
    pub dir: PathBuf,
    /// The bookmark still works, but the system asks for a new one.
    pub stale: bool,
    /// Access to the folder started. Without it the grant gives nothing.
    pub access: bool,
}

/// Security-scoped bookmarks: the macOS calls behind [`Grants`]. The calls
/// may touch the disk; they run on worker threads only.
pub trait Bookmarks: Send {
    /// A security-scoped bookmark of the folder `dir`, which this process
    /// may use now (the user chose it in a system panel, or a grant covers
    /// it).
    fn create(&self, dir: &Path) -> Result<Vec<u8>, String>;

    /// Resolve `bookmark` and start access to the folder it names, for the
    /// rest of the session.
    fn resolve(&self, bookmark: &[u8]) -> Result<Resolved, String>;
}

/// The bookmarks of this system: security-scoped bookmarks through
/// Foundation on macOS, an error for every call elsewhere.
pub fn system_bookmarks() -> Box<dyn Bookmarks> {
    #[cfg(target_os = "macos")]
    {
        Box::new(mac::SystemBookmarks)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(NoBookmarks)
    }
}

/// Stands in where no bookmark can be made.
struct NoBookmarks;

impl Bookmarks for NoBookmarks {
    fn create(&self, _dir: &Path) -> Result<Vec<u8>, String> {
        Err("this system has no security-scoped bookmarks".into())
    }

    fn resolve(&self, _bookmark: &[u8]) -> Result<Resolved, String> {
        Err("this system has no security-scoped bookmarks".into())
    }
}

// ----- grants -----------------------------------------------------------------------

/// One folder the user allowed, with its bookmark.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Grant {
    dir: PathBuf,
    bookmark: Vec<u8>,
}

/// The folders the user allowed, their bookmarks and the file that keeps
/// them. A grant on a folder covers its subfolders.
pub struct Grants {
    bookmarks: Box<dyn Bookmarks>,
    /// The grants file; `None` keeps the grants for this session only.
    file: Option<PathBuf>,
    /// Oldest first.
    list: Vec<Grant>,
}

impl Grants {
    /// No grants yet. `file` is where [`Grants::load`] reads them and
    /// where every change is written.
    pub fn new(bookmarks: Box<dyn Bookmarks>, file: Option<PathBuf>) -> Self {
        Self {
            bookmarks,
            file,
            list: Vec::new(),
        }
    }

    /// Read the grants file, resolve each bookmark and start access to its
    /// folder. A bookmark that does not resolve or gives no access is
    /// dropped; one the system calls stale is made again. The file is
    /// written again when anything changed. A missing file gives no
    /// grants. A file that cannot be read or has another format is left
    /// as it is, and nothing is written to it in this session.
    pub fn load(&mut self, disk: &dyn Disk) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let bytes = match disk.read_small(&file) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return,
            Err(_) => {
                self.file = None;
                return;
            }
        };
        let Some((saved, mut changed)) = std::str::from_utf8(&bytes).ok().and_then(parse) else {
            self.file = None;
            return;
        };
        for mut bookmark in saved {
            let found = match self.bookmarks.resolve(&bookmark) {
                Ok(found) if found.access => found,
                _ => {
                    changed = true;
                    continue;
                }
            };
            if found.stale {
                if let Ok(fresh) = self.bookmarks.create(&found.dir) {
                    bookmark = fresh;
                    changed = true;
                }
            }
            // Two bookmarks of one folder: the later one stays.
            let before = self.list.len();
            self.list.retain(|g| !same_dir(&g.dir, &found.dir));
            changed |= self.list.len() != before;
            self.list.push(Grant {
                dir: found.dir,
                bookmark,
            });
        }
        if changed {
            let _ = self.save(disk);
        }
    }

    /// True when a grant covers the folder `dir`.
    pub fn covers(&self, dir: &Path) -> bool {
        self.list.iter().any(|g| contains(&g.dir, dir))
    }

    /// The folders of the grants, oldest first.
    pub fn folders(&self) -> Vec<PathBuf> {
        self.list.iter().map(|g| g.dir.clone()).collect()
    }

    /// Keep a grant for the folder `dir`, which this process may use now:
    /// make its bookmark, drop the grants of `dir` and of the folders in it
    /// (the new one covers them), keep at most [`MAX_GRANTS`] (the oldest
    /// go first) and write the grants file. Access to `dir` is not started
    /// here: the system panel that chose it gave access for this session.
    pub fn add(&mut self, dir: &Path, disk: &dyn Disk) -> Result<(), String> {
        let bookmark = self.bookmarks.create(dir)?;
        self.list.retain(|g| !contains(dir, &g.dir));
        self.list.push(Grant {
            dir: dir.to_path_buf(),
            bookmark,
        });
        let extra = self.list.len().saturating_sub(MAX_GRANTS);
        self.list.drain(..extra);
        self.save(disk).map_err(|e| e.to_string())
    }

    /// Write the grants file, making its folder.
    fn save(&self, disk: &dyn Disk) -> io::Result<()> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            disk.create_dir_all(dir)?;
        }
        disk.write_atomic(file, format(&self.list).as_bytes())
    }
}

/// The text of the grants file.
fn format(list: &[Grant]) -> String {
    let mut text = String::from(HEADER);
    text.push('\n');
    for grant in list {
        text.push_str(&to_hex(&grant.bookmark));
        text.push('\n');
    }
    text
}

/// The bookmarks of a grants file, and true when a line was dropped (the
/// file is then written again). `None` for another format.
fn parse(text: &str) -> Option<(Vec<Vec<u8>>, bool)> {
    let mut lines = text.lines();
    if lines.next()?.trim_end() != HEADER {
        return None;
    }
    let mut out = Vec::new();
    let mut dropped = false;
    for line in lines.map(str::trim).filter(|l| !l.is_empty()) {
        match from_hex(line) {
            Some(bookmark) => out.push(bookmark),
            None => dropped = true,
        }
    }
    Some((out, dropped))
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        text.push(char::from(DIGITS[usize::from(b >> 4)]));
        text.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    text
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    let digits = text.as_bytes();
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return None;
    }
    let digit = |c: u8| char::from(c).to_digit(16).map(|d| d as u8);
    digits
        .chunks(2)
        .map(|pair| Some(digit(pair[0])? << 4 | digit(pair[1])?))
        .collect()
}

// ----- store ------------------------------------------------------------------------

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The grants of the session, shared by the jobs that use them. They load
/// on a worker; every use waits until the load ended, so no job sees the
/// grants half loaded and no write replaces a file that was not read.
pub struct Store {
    grants: Mutex<Option<Grants>>,
    loaded: Condvar,
}

impl Store {
    /// Load the grants of `file` on a worker thread (see [`Grants::load`])
    /// and return at once. A load that panics keeps what it resolved and
    /// writes nothing more to the file.
    pub fn open(
        bookmarks: Box<dyn Bookmarks>,
        file: Option<PathBuf>,
        disk: Arc<dyn Disk>,
    ) -> Arc<Store> {
        let store = Arc::new(Store {
            grants: Mutex::new(None),
            loaded: Condvar::new(),
        });
        let shared = Arc::clone(&store);
        let spawned = std::thread::Builder::new()
            .name("pandite-grants".into())
            .spawn(move || {
                let mut grants = Grants::new(bookmarks, file);
                if catch_unwind(AssertUnwindSafe(|| grants.load(&*disk))).is_err() {
                    grants.file = None;
                }
                shared.finish(grants);
            });
        if spawned.is_err() {
            store.finish(Grants::new(Box::new(NoBookmarks), None));
        }
        store
    }

    fn finish(&self, grants: Grants) {
        *lock(&self.grants) = Some(grants);
        self.loaded.notify_all();
    }

    /// Run `f` on the grants once the load ended. Blocks until then, and
    /// while another job uses them: call it on worker threads only.
    pub fn with<R>(&self, f: impl FnOnce(&mut Grants) -> R) -> R {
        let mut guard = lock(&self.grants);
        loop {
            if let Some(grants) = guard.as_mut() {
                return f(grants);
            }
            guard = self
                .loaded
                .wait(guard)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }
}

// ----- the questions of a session -------------------------------------------------

/// What to do after the listing of a folder was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A grant covers the folder; its access may have started after the
    /// listing began. List the folder again (once).
    ListAgain,
    /// Ask the user to allow the folder ([`Session::asking`]).
    Ask,
    /// The user dismissed the question for this folder in this session:
    /// show the image alone.
    Quiet,
}

/// The folder questions of one session: the one on screen, the folders the
/// user dismissed, and the folders listed again after a grant covered them.
#[derive(Debug, Default)]
pub struct Session {
    asking: Option<(u64, PathBuf)>,
    last_id: u64,
    dismissed: Vec<PathBuf>,
    listed_again: Vec<PathBuf>,
}

impl Session {
    /// The listing of the folder `dir` was refused; `covered` tells whether
    /// a grant covers it. A covered folder is listed again once; a second
    /// refusal asks, so a grant that gives no access never loops.
    pub fn refused(&mut self, dir: &Path, covered: bool) -> Refused {
        if covered && !self.listed_again.iter().any(|d| same_dir(d, dir)) {
            self.listed_again.push(dir.to_path_buf());
            return Refused::ListAgain;
        }
        if self.dismissed.iter().any(|d| same_dir(d, dir)) {
            self.asking = None;
            return Refused::Quiet;
        }
        if !self.asking.as_ref().is_some_and(|(_, d)| same_dir(d, dir)) {
            self.last_id += 1;
            self.asking = Some((self.last_id, dir.to_path_buf()));
        }
        Refused::Ask
    }

    /// The folder `dir` was listed: no question stays, and a later refusal
    /// of `dir` may list it again.
    pub fn listed(&mut self, dir: &Path) {
        self.listed_again.retain(|d| !same_dir(d, dir));
        self.asking = None;
    }

    /// The folder `dir` is on screen now: a question about another folder
    /// goes.
    pub fn entered(&mut self, dir: &Path) {
        if !self.asking.as_ref().is_some_and(|(_, d)| same_dir(d, dir)) {
            self.asking = None;
        }
    }

    /// The question on screen: its id and its folder.
    pub fn asking(&self) -> Option<(u64, &Path)> {
        self.asking.as_ref().map(|(id, dir)| (*id, dir.as_path()))
    }

    /// The folder of question `id`, while it is on screen.
    pub fn folder_of(&self, id: u64) -> Option<&Path> {
        self.asking
            .as_ref()
            .filter(|(at, _)| *at == id)
            .map(|(_, dir)| dir.as_path())
    }

    /// The user dismissed question `id`: its folder is not asked about
    /// again in this session. Returns the folder.
    pub fn dismiss(&mut self, id: u64) -> Option<PathBuf> {
        let dir = self.asking.take_if(|(at, _)| *at == id)?.1;
        self.dismissed.push(dir.clone());
        Some(dir)
    }
}

// ----- writes ---------------------------------------------------------------------

/// [`Disk`] inside the App Sandbox: [`crate::disk::StdDisk`] with an atomic
/// write that also works where only the target file may be written (see
/// [`write_atomic`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct SandboxDisk;

impl FileAccess for SandboxDisk {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        StdFiles.list_dir(dir)
    }

    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        StdFiles.file_info(path)
    }

    fn read_header(&self, path: &Path, len: usize) -> Option<io::Result<Vec<u8>>> {
        StdFiles.read_header(path, len)
    }

    fn read(
        &self,
        path: &Path,
        cancel: &pandite_core::CancelToken,
    ) -> pandite_core::Result<pandite_core::Source> {
        StdFiles.read(path, cancel)
    }
}

impl Disk for SandboxDisk {
    fn write_atomic(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        write_atomic(path, data, &std::env::temp_dir())
    }
}

/// Replace `path` with `data` atomically, also where the sandbox lets the
/// program write `path` itself but no other file in its folder (Save As
/// into a folder that no grant covers).
///
/// First [`crate::disk::write_atomic`], with its temporary file beside
/// `path`. When that is refused with [`io::ErrorKind::PermissionDenied`],
/// the temporary file goes to `temp_dir` (the temporary folder of the
/// container) and takes the final name in one rename. When `path` is on
/// another volume than `temp_dir`, a new file is written in place, created
/// new and removed again on failure; an existing file then keeps its
/// content and the first error is returned.
pub fn write_atomic(path: &Path, data: &[u8], temp_dir: &Path) -> io::Result<()> {
    write_atomic_with(path, data, temp_dir, disk::write_atomic)
}

fn write_atomic_with(
    path: &Path,
    data: &[u8],
    temp_dir: &Path,
    beside: impl FnOnce(&Path, &[u8]) -> io::Result<()>,
) -> io::Result<()> {
    let refused = match beside(path, data) {
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => err,
        done => return done,
    };
    let name = disk::temp_path_for(path);
    let temp = temp_dir.join(name.file_name().unwrap_or(OsStr::new(".pandite-tmp")));
    write_new(&temp, data)?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = std::fs::remove_file(&temp);
            if err.kind() != io::ErrorKind::CrossesDevices {
                return Err(err);
            }
            write_new(path, data).map_err(|e| {
                if e.kind() == io::ErrorKind::AlreadyExists {
                    refused
                } else {
                    e
                }
            })
        }
    }
}

/// Write the new file `path` and flush it. A file of that name stays as it
/// is ([`io::ErrorKind::AlreadyExists`]); a failed write leaves no file.
fn write_new(path: &Path, data: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let written = file.write_all(data).and_then(|()| file.sync_all());
    drop(file);
    if written.is_err() {
        let _ = std::fs::remove_file(path);
    }
    written
}

// ----- macOS ------------------------------------------------------------------------

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
mod mac {
    use super::{Bookmarks, Resolved};
    use objc2::rc::autoreleasepool;
    use objc2::runtime::Bool;
    use objc2_foundation::{
        NSData, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions, NSURL,
    };
    use std::path::Path;

    /// Security-scoped bookmarks through `NSURL`.
    pub(super) struct SystemBookmarks;

    impl Bookmarks for SystemBookmarks {
        fn create(&self, dir: &Path) -> Result<Vec<u8>, String> {
            autoreleasepool(|_| {
                let url = NSURL::from_directory_path(dir)
                    .ok_or_else(|| "this is not a valid folder path".to_string())?;
                url.bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(
                    NSURLBookmarkCreationOptions::WithSecurityScope,
                    None,
                    None,
                )
                .map(|data| data.to_vec())
                .map_err(|err| err.localizedDescription().to_string())
            })
        }

        fn resolve(&self, bookmark: &[u8]) -> Result<Resolved, String> {
            autoreleasepool(|_| {
                let data = NSData::with_bytes(bookmark);
                // No dialog from a worker thread at start-up.
                let options = NSURLBookmarkResolutionOptions::WithSecurityScope
                    | NSURLBookmarkResolutionOptions::WithoutUI;
                let mut stale = Bool::NO;
                // SAFETY: `stale` is a valid, writable `BOOL` that lives
                // until the call returns.
                let url = unsafe {
                    NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
                        &data,
                        options,
                        None,
                        &mut stale,
                    )
                }
                .map_err(|err| err.localizedDescription().to_string())?;
                let dir = url
                    .to_file_path()
                    .ok_or_else(|| "the bookmark names no file path".to_string())?;
                // SAFETY: `url` is the security-scoped URL resolved above.
                // The access lasts for the session: nothing stops it, and
                // the system ends it with the process.
                let access = unsafe { url.startAccessingSecurityScopedResource() };
                Ok(Resolved {
                    dir,
                    stale: stale.as_bool(),
                    access,
                })
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::StdDisk;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pandite-app-sandbox-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Bookmarks as `b"<folder>"`; resolution follows a table of moves,
    /// stale marks and failures.
    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<FakeState>>);

    #[derive(Default)]
    struct FakeState {
        moved: HashMap<String, String>,
        stale: Vec<String>,
        gone: Vec<String>,
        no_access: Vec<String>,
        refuse_create: bool,
        created: Vec<String>,
        resolved: Vec<String>,
    }

    impl Bookmarks for Fake {
        fn create(&self, dir: &Path) -> Result<Vec<u8>, String> {
            let mut s = lock(&self.0);
            if s.refuse_create {
                return Err("no bookmark".into());
            }
            let name = dir.to_string_lossy().into_owned();
            s.created.push(name.clone());
            Ok(format!("v2:{name}").into_bytes())
        }

        fn resolve(&self, bookmark: &[u8]) -> Result<Resolved, String> {
            let mut s = lock(&self.0);
            let text = String::from_utf8(bookmark.to_vec()).map_err(|e| e.to_string())?;
            let name = text.trim_start_matches("v2:").to_string();
            s.resolved.push(name.clone());
            if s.gone.contains(&name) {
                return Err("gone".into());
            }
            let dir = s.moved.get(&name).cloned().unwrap_or(name.clone());
            Ok(Resolved {
                dir: PathBuf::from(dir),
                stale: s.stale.contains(&name) && !text.starts_with("v2:"),
                access: !s.no_access.contains(&name),
            })
        }
    }

    fn grants_text(folders: &[&str]) -> String {
        let mut text = format!("{HEADER}\n");
        for f in folders {
            text.push_str(&to_hex(f.as_bytes()));
            text.push('\n');
        }
        text
    }

    #[test]
    fn active_only_on_macos_with_the_variable() {
        assert!(!sandboxed(None));
        assert!(!sandboxed(Some(OsStr::new(""))));
        assert_eq!(
            sandboxed(Some(OsStr::new("net.darklabs.pandite"))),
            cfg!(target_os = "macos")
        );
        if !cfg!(target_os = "macos") {
            assert!(!active(), "never outside macOS");
        }
    }

    #[test]
    fn folders_contain_their_subfolders_only() {
        let root = Path::new("/Users/me/Pictures");
        assert!(contains(root, root));
        assert!(contains(root, Path::new("/Users/me/Pictures/2026/trip")));
        assert!(contains(root, Path::new("/Users/me/Pictures/")));
        assert!(!contains(root, Path::new("/Users/me/Pictures2")));
        assert!(!contains(root, Path::new("/Users/me")));
        assert!(!contains(Path::new(""), root));
        assert_eq!(
            contains(root, Path::new("/users/ME/pictures/x")),
            cfg!(any(windows, target_os = "macos"))
        );
    }

    #[test]
    fn hex_and_file_text_round_trip() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(from_hex(&to_hex(&bytes)), Some(bytes.clone()));
        assert_eq!(from_hex("0g"), None);
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex(""), None);
        let list = vec![
            Grant {
                dir: PathBuf::from("/a"),
                bookmark: bytes.clone(),
            },
            Grant {
                dir: PathBuf::from("/b"),
                bookmark: vec![1, 2],
            },
        ];
        let text = format(&list);
        assert!(text.starts_with("pandite grants 1\n"));
        assert!(text.is_ascii());
        assert_eq!(parse(&text), Some((vec![bytes, vec![1, 2]], false)));
        // Windows line ends and a broken line: the rest stays.
        let mixed = "pandite grants 1\r\n0102\r\nzz\r\n\r\n0304\r\n";
        assert_eq!(parse(mixed), Some((vec![vec![1, 2], vec![3, 4]], true)));
        assert_eq!(parse("pandite grants 2\n0102\n"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn load_resolves_refreshes_and_drops() {
        let dir = temp_dir("load");
        let file = dir.join(GRANTS_FILE);
        std::fs::write(
            &file,
            grants_text(&["/keep", "/old", "/gone", "/denied", "/stale", "/keep"]),
        )
        .unwrap();
        let fake = Fake::default();
        {
            let mut s = lock(&fake.0);
            s.moved.insert("/old".into(), "/new".into());
            s.gone.push("/gone".into());
            s.no_access.push("/denied".into());
            s.stale.push("/stale".into());
        }
        let mut grants = Grants::new(Box::new(fake.clone()), Some(file.clone()));
        grants.load(&StdDisk);
        // Every bookmark was resolved; the stale one was made again.
        assert_eq!(lock(&fake.0).resolved.len(), 6);
        assert_eq!(lock(&fake.0).created, ["/stale"]);
        let folders = grants.folders();
        let want: Vec<PathBuf> = ["/new", "/stale", "/keep"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(folders, want);
        assert!(grants.covers(Path::new("/new/sub")));
        assert!(!grants.covers(Path::new("/old")));
        assert!(!grants.covers(Path::new("/gone")));
        // The file now holds what was kept, the fresh bookmark included.
        let text = std::fs::read_to_string(&file).unwrap();
        let (saved, _) = parse(&text).unwrap();
        assert_eq!(saved.len(), 3);
        assert!(saved.contains(&b"v2:/stale".to_vec()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn load_leaves_a_clean_file_and_foreign_files_alone() {
        let dir = temp_dir("clean");
        let file = dir.join(GRANTS_FILE);
        let text = grants_text(&["/a", "/b"]);
        std::fs::write(&file, &text).unwrap();
        let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
        let mut grants = Grants::new(Box::new(Fake::default()), Some(file.clone()));
        grants.load(&StdDisk);
        assert_eq!(grants.folders().len(), 2);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), text);
        assert_eq!(
            std::fs::metadata(&file).unwrap().modified().unwrap(),
            modified
        );

        // Another format: nothing loads, and nothing is ever written.
        std::fs::write(&file, "pandite grants 9\nxyz\n").unwrap();
        let mut grants = Grants::new(Box::new(Fake::default()), Some(file.clone()));
        grants.load(&StdDisk);
        assert!(grants.folders().is_empty());
        grants.add(Path::new("/c"), &StdDisk).unwrap();
        assert!(grants.covers(Path::new("/c")), "kept for the session");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "pandite grants 9\nxyz\n"
        );

        // No file: no grants, and the first grant makes it and its folder.
        let file = dir.join("sub").join(GRANTS_FILE);
        let mut grants = Grants::new(Box::new(Fake::default()), Some(file.clone()));
        grants.load(&StdDisk);
        assert!(!file.exists());
        grants.add(Path::new("/d"), &StdDisk).unwrap();
        assert_eq!(
            parse(&std::fs::read_to_string(&file).unwrap()),
            Some((vec![b"v2:/d".to_vec()], false))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn add_covers_subfolders_and_keeps_the_newest() {
        let dir = temp_dir("add");
        let file = dir.join(GRANTS_FILE);
        let fake = Fake::default();
        let mut grants = Grants::new(Box::new(fake.clone()), Some(file.clone()));
        grants.add(Path::new("/p/a"), &StdDisk).unwrap();
        grants.add(Path::new("/p/b"), &StdDisk).unwrap();
        grants.add(Path::new("/q"), &StdDisk).unwrap();
        // A parent replaces the grants inside it.
        grants.add(Path::new("/p"), &StdDisk).unwrap();
        assert_eq!(grants.folders(), [PathBuf::from("/q"), PathBuf::from("/p")]);
        assert!(grants.covers(Path::new("/p/a/deep")));
        // The oldest go beyond the limit.
        for i in 0..MAX_GRANTS {
            grants
                .add(Path::new(&format!("/many/{i}")), &StdDisk)
                .unwrap();
        }
        let folders = grants.folders();
        assert_eq!(folders.len(), MAX_GRANTS);
        assert_eq!(folders[0], PathBuf::from("/many/0"));
        assert!(!grants.covers(Path::new("/q")));
        // A bookmark that cannot be made changes nothing.
        lock(&fake.0).refuse_create = true;
        let before = std::fs::read(&file).unwrap();
        assert!(grants.add(Path::new("/r"), &StdDisk).is_err());
        assert!(!grants.covers(Path::new("/r")));
        assert_eq!(std::fs::read(&file).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn store_waits_for_its_load() {
        let dir = temp_dir("store");
        let file = dir.join(GRANTS_FILE);
        std::fs::write(&file, grants_text(&["/a"])).unwrap();
        let store = Store::open(
            Box::new(Fake::default()),
            Some(file),
            Arc::new(StdDisk) as Arc<dyn Disk>,
        );
        // The first use sees the loaded grants, however early it comes.
        assert!(store.with(|g| g.covers(Path::new("/a/b"))));
        // Without a file, grants live for the session only.
        let store = Store::open(Box::new(Fake::default()), None, Arc::new(StdDisk));
        store.with(|g| g.add(Path::new("/x"), &StdDisk)).unwrap();
        assert!(store.with(|g| g.covers(Path::new("/x"))));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn session_asks_once_per_folder_and_never_loops() {
        let (a, b) = (Path::new("/a"), Path::new("/b"));
        let mut s = Session::default();
        assert_eq!(s.asking(), None);
        // Covered: list again once, then ask.
        assert_eq!(s.refused(a, true), Refused::ListAgain);
        assert_eq!(s.asking(), None);
        assert_eq!(s.refused(a, true), Refused::Ask);
        let (id, dir) = s.asking().unwrap();
        assert_eq!(dir, a);
        assert_eq!(s.folder_of(id), Some(a));
        assert_eq!(s.folder_of(id + 1), None);
        // The same folder keeps its question and id.
        assert_eq!(s.refused(a, false), Refused::Ask);
        assert_eq!(s.asking().unwrap().0, id);
        // A dismissal with a stale id does nothing; the right id ends it.
        assert_eq!(s.dismiss(id + 1), None);
        assert_eq!(s.dismiss(id), Some(a.to_path_buf()));
        assert_eq!(s.asking(), None);
        assert_eq!(s.refused(a, false), Refused::Quiet);
        assert_eq!(s.asking(), None);
        // Another folder is asked about with a new id.
        assert_eq!(s.refused(b, false), Refused::Ask);
        let (id_b, _) = s.asking().unwrap();
        assert_ne!(id_b, id);
        // Entering another folder ends it; entering the same keeps it.
        s.entered(b);
        assert!(s.asking().is_some());
        s.entered(a);
        assert_eq!(s.asking(), None);
        // A listing ends the question and allows another list-again.
        assert_eq!(s.refused(b, true), Refused::ListAgain);
        assert_eq!(s.refused(b, true), Refused::Ask);
        s.listed(b);
        assert_eq!(s.asking(), None);
        assert_eq!(s.refused(b, true), Refused::ListAgain);
    }

    #[test]
    fn sandbox_writes_go_through_the_container_when_refused() {
        let dir = temp_dir("write");
        let (target_dir, temp) = (dir.join("chosen"), dir.join("tmp"));
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::create_dir_all(&temp).unwrap();
        let path = target_dir.join("out.png");
        let refuse = |_: &Path, _: &[u8]| -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "sandbox"))
        };
        // Refused beside the target: written through the temporary folder.
        write_atomic_with(&path, b"new", &temp, refuse).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        // It replaces an existing file the same way.
        write_atomic_with(&path, b"newer", &temp, refuse).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"newer");
        assert_eq!(std::fs::read_dir(&temp).unwrap().count(), 0, "no leftovers");
        assert_eq!(std::fs::read_dir(&target_dir).unwrap().count(), 1);
        // Allowed beside the target: the temporary folder is never used.
        write_atomic(&path, b"plain", &dir.join("missing")).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"plain");
        // Another error is returned as it is.
        let other = |_: &Path, _: &[u8]| -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::StorageFull, "full"))
        };
        let err = write_atomic_with(&path, b"x", &temp, other).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
        assert_eq!(std::fs::read(&path).unwrap(), b"plain");
        // A temporary folder that cannot take the file: the target keeps
        // its content.
        let err = write_atomic_with(&path, b"x", &dir.join("missing"), refuse).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert_eq!(std::fs::read(&path).unwrap(), b"plain");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

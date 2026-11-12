//! The [`Library`]: worker threads, caches and the event queue.

use crate::downscale::box_downscale;
use crate::fs::{FileAccess, StdFiles};
use crate::state::{Settings, State};
use crate::worker::replenish;
use pandite_core::{Content, Error, ImageBuf, Registry, Result};
use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// Called from a worker thread when the event queue goes from empty to
/// non-empty. Use it to wake the interface event loop, then drain the
/// queue with [`Library::poll_events`]. Must return quickly.
pub type WakeFn = Arc<dyn Fn() + Send + Sync>;

/// Shrinks an image so its longer side is at most the given size, keeping
/// the aspect ratio. Runs on worker threads.
pub type DownscaleFn = Arc<dyn Fn(&ImageBuf, u32) -> ImageBuf + Send + Sync>;

/// Tuning for a [`Library`].
#[derive(Clone)]
pub struct LibraryConfig {
    /// Worker threads in the pool (at least 1). Default: one less than the
    /// processor count, between 2 and 4.
    pub workers: usize,
    /// Most threads written off at once because they are stuck in a
    /// cancelled file read (default 8). A written-off thread is replaced,
    /// so at most `workers + max_written_off` threads exist. Beyond the
    /// cap the pool shrinks until a stuck read returns.
    pub max_written_off: usize,
    /// Neighbors decoded ahead on each side of the current image (default 2).
    pub read_ahead: usize,
    /// Read-ahead wraps from the last image to the first (default false).
    pub wrap_around: bool,
    /// Byte budget for full decodes (default 512 MiB). The current image is
    /// kept even when it alone is larger.
    pub full_budget: usize,
    /// Byte budget for thumbnails (default 64 MiB). Thumbnails in the
    /// visible gallery range are kept even above the budget.
    pub thumb_budget: usize,
    /// Longest thumbnail side in pixels (default 256).
    pub thumb_size: u32,
    /// Thumbnails made beyond each end of the visible range before the
    /// rest of the folder (default 32).
    pub thumb_margin: usize,
    /// Make thumbnails for the whole folder, nearest first, until the
    /// budget is full (default true). When false, only the visible range
    /// and the margin get thumbnails.
    pub background_thumbnails: bool,
    /// How long dropping the library waits for workers that are not
    /// inside a file system call (default 250 ms). Workers still running
    /// after that, and workers stuck in a read, are detached.
    pub shutdown_grace: Duration,
    /// The thumbnail downscaler. Default: [`box_downscale`].
    pub downscale: DownscaleFn,
    /// File system access. Default: [`StdFiles`].
    pub files: Arc<dyn FileAccess>,
}

impl Default for LibraryConfig {
    fn default() -> Self {
        let cores = thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            workers: cores.saturating_sub(1).clamp(2, 4),
            max_written_off: 8,
            read_ahead: 2,
            wrap_around: false,
            full_budget: 512 << 20,
            thumb_budget: 64 << 20,
            thumb_size: 256,
            thumb_margin: 32,
            background_thumbnails: true,
            shutdown_grace: Duration::from_millis(250),
            downscale: Arc::new(box_downscale),
            files: Arc::new(StdFiles),
        }
    }
}

impl fmt::Debug for LibraryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LibraryConfig")
            .field("workers", &self.workers)
            .field("max_written_off", &self.max_written_off)
            .field("read_ahead", &self.read_ahead)
            .field("wrap_around", &self.wrap_around)
            .field("full_budget", &self.full_budget)
            .field("thumb_budget", &self.thumb_budget)
            .field("thumb_size", &self.thumb_size)
            .field("thumb_margin", &self.thumb_margin)
            .field("background_thumbnails", &self.background_thumbnails)
            .field("shutdown_grace", &self.shutdown_grace)
            .finish_non_exhaustive()
    }
}

/// The state of a requested image or thumbnail.
#[derive(Clone, Debug)]
pub enum Fetch<T> {
    /// Decoded and cached.
    Ready(T),
    /// Not decoded yet: queued, decoding, or outside the read-ahead window.
    Pending,
    /// The current image is not stored locally and is being downloaded by
    /// the operating system. Show a "downloading" note; the user may cancel
    /// with [`Library::cancel_current`]. Never returned for thumbnails.
    Downloading,
    /// The file is not stored locally (cloud placeholder). It is read only
    /// when opened as the current image. Show a "not downloaded" slot.
    NotLocal,
    /// The user cancelled this load. Selecting the image again retries.
    Cancelled,
    /// Decoding failed. The error is kept until the file changes or
    /// [`Library::invalidate`] is called; it is not retried on its own.
    Failed(Arc<Error>),
}

impl<T> Fetch<T> {
    /// The value when ready.
    pub fn ready(self) -> Option<T> {
        match self {
            Fetch::Ready(value) => Some(value),
            _ => None,
        }
    }

    /// True when the value is ready.
    pub fn is_ready(&self) -> bool {
        matches!(self, Fetch::Ready(_))
    }
}

/// Something the interface may want to react to. Indexes refer to the
/// folder list at the time the event was queued; after
/// [`Event::Listed`] earlier indexes are stale, so match by path.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Event {
    /// The folder listing arrived and replaced the list. Earlier indexes
    /// are stale and the visible gallery range was cleared: report it
    /// again with [`Library::set_visible_thumbnails`].
    Listed {
        /// Number of images in the new list.
        len: usize,
        /// Index of the current image in the new list.
        current: Option<usize>,
    },
    /// A full decode is cached; [`Library::get`] returns it.
    Ready {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
    },
    /// A full decode failed. Reported once per file.
    Failed {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
        /// What went wrong.
        error: Arc<Error>,
    },
    /// The current image is not stored locally; its download started.
    Downloading {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
    },
    /// A worker found that this file is not stored locally, so it gets no
    /// read-ahead and no thumbnail.
    NotLocal {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
    },
    /// A thumbnail is cached; [`Library::thumbnail`] returns it.
    ThumbnailReady {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
    },
    /// Making a thumbnail failed. Reported once per file.
    ThumbnailFailed {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
        /// What went wrong.
        error: Arc<Error>,
    },
}

/// Cache and thread figures, for tests and a debug overlay.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Bytes held by full decodes.
    pub full_bytes: usize,
    /// Number of cached full decodes.
    pub full_entries: usize,
    /// Byte budget for full decodes.
    pub full_budget: usize,
    /// Bytes held by thumbnails.
    pub thumb_bytes: usize,
    /// Number of cached thumbnails.
    pub thumb_entries: usize,
    /// Byte budget for thumbnails.
    pub thumb_budget: usize,
    /// Jobs in flight (decodes, thumbnails, folder listing).
    pub running: usize,
    /// Live worker threads in the pool.
    pub threads: usize,
    /// Live threads written off while stuck in a cancelled read.
    pub written_off: usize,
}

pub(crate) struct Shared {
    state: Mutex<State>,
    work_cv: Condvar,
    pub idle_cv: Condvar,
    pub exit_cv: Condvar,
    pub registry: Arc<Registry>,
    pub files: Arc<dyn FileAccess>,
    pub downscale: DownscaleFn,
    pub wake: WakeFn,
    pub thumb_size: u32,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn wait_work<'a>(&self, guard: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.work_cv
            .wait(guard)
            .unwrap_or_else(PoisonError::into_inner)
    }

    pub fn notify_work(&self) {
        self.work_cv.notify_all();
    }

    pub fn notify_one_work(&self) {
        self.work_cv.notify_one();
    }
}

/// Folder navigation with read-ahead decoding and thumbnails.
///
/// The library owns worker threads and two memory-budgeted caches: full
/// decodes keyed by path (evicting the image farthest from the current one
/// first, never the current image) and thumbnails (evicting the one
/// farthest from the visible gallery range first). Every method except
/// [`Library::wait_idle`] returns at once: file reads, folder listings and
/// decodes run only on workers, and no lock is held across them. Results arrive as [`Event`]s; the wake
/// callback tells the interface to call [`Library::poll_events`].
///
/// A read that the operating system blocks (a cloud file that never
/// downloads) cannot be interrupted. When such a job is cancelled, its
/// thread is written off and replaced, and its result is dropped when the
/// read finally returns. Dropping the library does not wait for it.
pub struct Library {
    shared: Arc<Shared>,
    grace: Duration,
}

impl fmt::Debug for Library {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Library")
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl Library {
    /// Start the worker threads. Fails only when no thread can be spawned.
    pub fn new(registry: Arc<Registry>, config: LibraryConfig, wake: WakeFn) -> Result<Self> {
        let settings = Settings {
            workers: config.workers.max(1),
            max_written_off: config.max_written_off,
            read_ahead: config.read_ahead,
            wrap: config.wrap_around,
            margin: config.thumb_margin,
            background: config.background_thumbnails,
            thumb_size: config.thumb_size.max(1),
            full_budget: config.full_budget,
            thumb_budget: config.thumb_budget,
        };
        let shared = Arc::new(Shared {
            state: Mutex::new(State::new(settings)),
            work_cv: Condvar::new(),
            idle_cv: Condvar::new(),
            exit_cv: Condvar::new(),
            registry,
            files: config.files,
            downscale: config.downscale,
            wake,
            thumb_size: config.thumb_size.max(1),
        });
        let library = Self {
            shared,
            grace: config.shutdown_grace,
        };
        {
            let mut st = library.shared.lock();
            let spawned = replenish(&library.shared, &mut st);
            if st.thread_counts().0 == 0 {
                let err = spawned.err().map_or_else(
                    || Error::Backend("no worker thread started".into()),
                    Error::Io,
                );
                return Err(err);
            }
        }
        Ok(library)
    }

    /// Run `op` on the state, refill the pool if `op` wrote off a thread,
    /// and wake the workers.
    fn update<R>(&self, op: impl FnOnce(&mut State) -> R) -> R {
        let mut st = self.shared.lock();
        let result = op(&mut st);
        st.replanned = false;
        // A spawn failure leaves the pool smaller; the remaining workers
        // carry on.
        let _ = replenish(&self.shared, &mut st);
        drop(st);
        self.shared.notify_work();
        result
    }

    /// Open `path`: it becomes a one-image list with itself current and
    /// starts decoding at once. Its folder is listed on a worker; when the
    /// listing arrives the list is replaced and [`Event::Listed`] is sent.
    /// The file is read even when it is not stored locally.
    pub fn open(&self, path: impl Into<PathBuf>) {
        let path = path.into();
        self.update(|st| st.open(path));
    }

    /// List the folder again on a worker. Cached images whose file changed
    /// are dropped, vanished files leave the list, and the current image
    /// stays selected by path (or moves to its neighbor when it vanished).
    pub fn refresh(&self) {
        self.update(State::refresh);
    }

    /// Replace the list with `paths` (no disk access). Cached data follows
    /// its path. Returns the new index of the current image, if it stayed.
    pub fn set_items(&self, paths: Vec<PathBuf>) -> Option<usize> {
        self.update(|st| st.set_items(paths))
    }

    /// Number of images in the list.
    pub fn len(&self) -> usize {
        self.shared.lock().len()
    }

    /// True when the list is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The path at `index`.
    pub fn path(&self, index: usize) -> Option<PathBuf> {
        self.shared.lock().path(index)
    }

    /// A copy of the whole list.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.shared.lock().paths()
    }

    /// The current image index.
    pub fn current(&self) -> Option<usize> {
        self.shared.lock().current()
    }

    /// Show image `index`. Its decode is scheduled before anything else;
    /// read-ahead follows (next, previous, next+1, previous+1, ...), then
    /// thumbnails. Running decodes that left the read-ahead window are
    /// cancelled, and running thumbnails are cancelled when no worker is
    /// free for the new image. Calling it again with the current index
    /// retries a load cancelled by [`Library::cancel_current`] and does
    /// nothing otherwise. Out-of-range indexes are ignored.
    pub fn set_current(&self, index: usize) {
        self.update(|st| st.set_current(index));
    }

    /// Report the gallery range on screen. Thumbnails are made for it
    /// first (from the current image outward), then for the margin beyond
    /// both ends, then for the rest of the folder.
    pub fn set_visible_thumbnails(&self, range: Range<usize>) {
        self.update(|st| st.set_visible(range));
    }

    /// Stop loading the current image (the Escape key). A read stuck in
    /// the operating system is abandoned and its worker replaced.
    /// [`Library::get`] then returns [`Fetch::Cancelled`].
    pub fn cancel_current(&self) {
        self.update(State::cancel_current);
    }

    /// Forget everything cached for `path` (the file was rewritten) and
    /// load it again if it is still wanted.
    pub fn invalidate(&self, path: &Path) {
        self.update(|st| st.invalidate(path));
    }

    /// The full decode of image `index`.
    pub fn get(&self, index: usize) -> Fetch<Arc<Content>> {
        self.shared.lock().get(index)
    }

    /// The thumbnail of image `index`.
    pub fn thumbnail(&self, index: usize) -> Fetch<ImageBuf> {
        self.shared.lock().thumbnail(index)
    }

    /// Take every queued event, oldest first. Call it after each wake;
    /// events accumulate until polled.
    pub fn poll_events(&self) -> Vec<Event> {
        self.shared.lock().drain_events()
    }

    /// Cache and thread figures.
    pub fn stats(&self) -> Stats {
        self.shared.lock().stats()
    }

    /// Block until no job runs or can start and no written-off thread is
    /// stuck, or until `timeout` passes. Returns true when idle. Meant for
    /// tests and batch tools; never call it on the interface thread.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now().checked_add(timeout);
        let mut st = self.shared.lock();
        loop {
            if st.is_idle() {
                return true;
            }
            let wait = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return false;
                    }
                    deadline - now
                }
                None => Duration::from_secs(3600),
            };
            st = self
                .shared
                .idle_cv
                .wait_timeout(st, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl Drop for Library {
    /// Cancel all work and stop the workers. Waits up to the configured
    /// grace for workers that are idle or decoding, joins those that
    /// stopped, and detaches the rest (a worker stuck in a read exits on
    /// its own when the read returns).
    fn drop(&mut self) {
        let me = thread::current().id();
        let mut st = self.shared.lock();
        st.begin_shutdown();
        self.shared.notify_work();
        let deadline = Instant::now().checked_add(self.grace);
        while st.shutdown_waiting(me) {
            let wait = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    deadline - now
                }
                None => Duration::from_secs(3600),
            };
            st = self
                .shared
                .exit_cv
                .wait_timeout(st, wait)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        let threads = st.take_threads();
        drop(st);
        for (exited, handle) in threads {
            if exited && handle.thread().id() != me {
                let _ = handle.join();
            }
        }
    }
}

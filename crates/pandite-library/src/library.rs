//! The [`Library`]: worker threads, caches and the event queue.

use crate::downscale::box_downscale;
use crate::fs::{FileAccess, StdFiles};
use crate::kept::KeptThumbnails;
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

/// Makes the thumbnail of a decoded image for the thumbnail size (for
/// example a square that fills a gallery cell). Runs on worker threads.
pub type ThumbnailFn = Arc<dyn Fn(&ImageBuf, u32) -> ImageBuf + Send + Sync>;

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
    /// Byte budget for decoded images (default 512 MiB): the pixel bytes of
    /// every full-resolution and screen-resolution image the library
    /// holds. With a screen size set, a quarter of it is kept for screen
    /// images and the rest for full images. The current image's full and
    /// screen images are kept even when they alone are larger.
    pub full_budget: usize,
    /// Byte budget for thumbnails (default 64 MiB). Thumbnails in the
    /// visible gallery range are kept even above the budget.
    pub thumb_budget: usize,
    /// The thumbnail size in pixels that [`LibraryConfig::make_thumbnail`]
    /// gets (default 256). With the default maker it is the longest
    /// thumbnail side. [`Library::set_thumbnail_size`] changes it later.
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
    /// The screen size for staged loading (default `None`). When set, the
    /// current image first shows a quick reduced decode (an embedded
    /// preview or a scaled decode covering half this size) and then its
    /// full resolution; read-ahead neighbors are decoded to cover this
    /// size only, and full resolution is decoded for the current image
    /// only. When `None`, read-ahead decodes neighbors at full resolution.
    pub screen_size: Option<(u32, u32)>,
    /// The downscaler for screen-resolution images, and for thumbnails
    /// when [`LibraryConfig::make_thumbnail`] is `None`. Default:
    /// [`box_downscale`].
    pub downscale: DownscaleFn,
    /// The thumbnail maker (default `None`: [`LibraryConfig::downscale`],
    /// which fits the longer side into the thumbnail size).
    pub make_thumbnail: Option<ThumbnailFn>,
    /// File system access. Default: [`StdFiles`].
    pub files: Arc<dyn FileAccess>,
    /// Thumbnails kept from an earlier library (default `None`). They wait
    /// for the folder listing; then each one whose file still has the same
    /// size and modification time becomes a cached thumbnail and is not
    /// decoded again. The others are dropped and made again. Thumbnails of
    /// another folder than the one opened, or of another thumbnail size,
    /// are dropped at once.
    pub kept_thumbnails: Option<KeptThumbnails>,
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
            screen_size: None,
            shutdown_grace: Duration::from_millis(250),
            downscale: Arc::new(box_downscale),
            make_thumbnail: None,
            files: Arc::new(StdFiles),
            kept_thumbnails: None,
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
            .field("screen_size", &self.screen_size)
            .field("shutdown_grace", &self.shutdown_grace)
            .field("kept_thumbnails", &self.kept_thumbnails)
            .finish_non_exhaustive()
    }
}

/// A screen-resolution decode standing in for a larger image: an embedded
/// preview or a scaled decode, upright like every decode.
#[derive(Clone, Debug)]
pub struct Reduced {
    /// The reduced image.
    pub image: ImageBuf,
    /// Width of the full image (after orientation).
    pub width: u32,
    /// Height of the full image (after orientation).
    pub height: u32,
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

/// How a folder check ([`Library::check_folder`]) found the list changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListChange {
    /// For each index of the old list, the index of the same path in the
    /// new list, or `None` when the path left the list (removed, or
    /// renamed to a new path).
    pub moved: Vec<Option<usize>>,
    /// Paths that joined the list (new files, and new names of renamed
    /// files).
    pub added: usize,
    /// Paths that left the list.
    pub removed: usize,
    /// True when paths that stayed come in another order.
    pub reordered: bool,
    /// New indexes of the paths that stayed whose file changed (size,
    /// modification time, or whether it is stored locally), ascending.
    /// Their cached images, thumbnails and errors are dropped when the
    /// stamp changed, and made again when wanted.
    pub modified: Vec<usize>,
}

impl ListChange {
    /// True when no path joined, left or moved, and no file changed.
    pub fn is_empty(&self) -> bool {
        self.added == 0 && self.removed == 0 && !self.reordered && self.modified.is_empty()
    }
}

/// Something the interface may want to react to. Indexes refer to the
/// folder list at the time the event was queued; after
/// [`Event::Listed`] and [`Event::Changed`] earlier indexes are stale, so
/// match by path.
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
    /// A folder check ([`Library::check_folder`]) found the folder changed
    /// and replaced the list. The current image stayed by path, or moved
    /// to its neighbor when its file left, as after [`Library::refresh`].
    /// Earlier indexes are stale: `change.moved` maps them to the new
    /// ones. The visible gallery range was cleared: report it again with
    /// [`Library::set_visible_thumbnails`].
    Changed {
        /// Number of images in the new list.
        len: usize,
        /// Index of the current image in the new list.
        current: Option<usize>,
        /// What changed.
        change: ListChange,
    },
    /// A screen-resolution image is cached; [`Library::reduced`] returns
    /// it. For the current image it comes before [`Event::Ready`], and
    /// never after it.
    Reduced {
        /// Folder index.
        index: usize,
        /// File path.
        path: PathBuf,
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

/// How long the stages of one image load took, for diagnostics and the
/// application's timing report. A stage that did not run is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoadTimings {
    /// Reading the file (all reads of the load together).
    pub read: Option<Duration>,
    /// Decoding the reduced (screen size) image.
    pub decode_reduced: Option<Duration>,
    /// Decoding the full resolution image.
    pub decode_full: Option<Duration>,
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
    /// Bytes held by screen-resolution images.
    pub screen_bytes: usize,
    /// Number of cached screen-resolution images.
    pub screen_entries: usize,
    /// Byte budget for screen-resolution images.
    pub screen_budget: usize,
    /// Bytes held by thumbnails.
    pub thumb_bytes: usize,
    /// Number of cached thumbnails.
    pub thumb_entries: usize,
    /// Byte budget for thumbnails.
    pub thumb_budget: usize,
    /// Bytes of thumbnails kept from an earlier library that wait for the
    /// folder listing (not counted in `thumb_bytes`).
    pub kept_bytes: usize,
    /// Number of thumbnails kept from an earlier library that wait for the
    /// folder listing.
    pub kept_entries: usize,
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
    pub make_thumbnail: ThumbnailFn,
    pub wake: WakeFn,
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

    /// Wait for work, at most `limit`.
    pub fn wait_work_for<'a>(
        &self,
        guard: MutexGuard<'a, State>,
        limit: Duration,
    ) -> MutexGuard<'a, State> {
        self.work_cv
            .wait_timeout(guard, limit)
            .unwrap_or_else(PoisonError::into_inner)
            .0
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
            screen: config.screen_size.map(|(w, h)| (w.max(1), h.max(1))),
        };
        let mut state = State::new(settings);
        if let Some(kept) = config.kept_thumbnails {
            state.keep_thumbnails(kept);
        }
        let make_thumbnail = config
            .make_thumbnail
            .unwrap_or_else(|| Arc::clone(&config.downscale));
        let shared = Arc::new(Shared {
            state: Mutex::new(state),
            work_cv: Condvar::new(),
            idle_cv: Condvar::new(),
            exit_cv: Condvar::new(),
            registry,
            files: config.files,
            downscale: config.downscale,
            make_thumbnail,
            wake,
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
    /// The file is read even when it is not stored locally. Kept
    /// thumbnails ([`LibraryConfig::kept_thumbnails`]) of another folder
    /// are dropped here.
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

    /// Look on a worker whether the folder changed, at the cost of one
    /// directory listing. When the folder holds the same files with the
    /// same sizes, modification times and locality as the list, nothing
    /// changes: no event is queued and the wake callback is not called.
    /// Otherwise the list is replaced as [`Library::refresh`] replaces it,
    /// cached data of unchanged files stays, and [`Event::Changed`] tells
    /// what changed. The current file and the opened file stay in the
    /// list while the folder holds them, also when the registry does not
    /// know their extension. A folder that cannot be listed changes
    /// nothing. Does nothing while another listing is queued or runs, and
    /// before the first listing.
    pub fn check_folder(&self) {
        self.update(State::check);
    }

    /// Replace the list with `paths` (no disk access). Cached data follows
    /// its path. Kept thumbnails not used yet are dropped: without a
    /// listing their files cannot be checked. Returns the new index of the
    /// current image, if it stayed.
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

    /// Why the folder listing that made the list could not read the
    /// folder, when it could not (see [`crate::FolderList::listing_error`]).
    /// The list then holds only the opened file. `None` while the listing
    /// of a newly opened file runs, and after [`Library::set_items`].
    pub fn listing_error(&self) -> Option<std::io::ErrorKind> {
        self.shared.lock().listing_error()
    }

    /// True when the list holds files whose data is not stored on this
    /// machine (cloud placeholders, offline storage), as the folder
    /// listing or a worker found.
    pub fn any_not_local(&self) -> bool {
        self.shared.lock().any_not_local()
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

    /// Stop loading the current image. A read stuck in
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

    /// The screen-resolution image of `index`, when one is cached. Show it
    /// while [`Library::get`] is still pending.
    pub fn reduced(&self, index: usize) -> Option<Reduced> {
        self.shared.lock().reduced(index)
    }

    /// Change the screen size of staged loading (the window was resized).
    /// No effect when the library was made without a screen size.
    pub fn set_screen_size(&self, width: u32, height: u32) {
        self.shared.lock().set_screen((width, height));
    }

    /// The thumbnail of image `index`. After [`Library::set_thumbnail_size`]
    /// this is the thumbnail of the old size until the new one is made.
    pub fn thumbnail(&self, index: usize) -> Fetch<ImageBuf> {
        self.shared.lock().thumbnail(index)
    }

    /// Make thumbnails for `side` from now on (the gallery cells changed
    /// size). Each cached thumbnail of another size stays available until
    /// its new one is made, which is announced with
    /// [`Event::ThumbnailReady`] like a first thumbnail. The new one is
    /// made without a decode from the cached full image of the file, from
    /// its screen image when the short side of that reaches the new size,
    /// or from the old thumbnail when the new size is not larger or the old
    /// thumbnail holds its whole source unreduced. Otherwise the file is
    /// read and decoded again. Kept thumbnails
    /// ([`LibraryConfig::kept_thumbnails`]) of another size are dropped.
    /// The same size again does nothing.
    pub fn set_thumbnail_size(&self, side: u32) {
        self.update(|st| st.set_thumb_size(side));
    }

    /// The thumbnail size in pixels: [`LibraryConfig::thumb_size`] or the
    /// last [`Library::set_thumbnail_size`].
    pub fn thumbnail_size(&self) -> u32 {
        self.shared.lock().thumb_size()
    }

    /// Take every queued event, oldest first. Call it after each wake;
    /// events accumulate until polled.
    pub fn poll_events(&self) -> Vec<Event> {
        self.shared.lock().drain_events()
    }

    /// Stage durations of the most recent load of `path`, once its full
    /// decode finished.
    pub fn timings(&self, path: &Path) -> Option<LoadTimings> {
        self.shared.lock().timings(path)
    }

    /// Cache and thread figures.
    pub fn stats(&self) -> Stats {
        self.shared.lock().stats()
    }

    /// The thumbnails of the folder, for the next library
    /// ([`LibraryConfig::kept_thumbnails`]). Thumbnails in the visible
    /// gallery range come first, then those nearest the current image; the
    /// total stays within the thumbnail budget. Only thumbnails whose file
    /// size and modification time are known are taken. Kept thumbnails
    /// this library did not use yet are passed on. The pixels are shared,
    /// not copied. `None` when no folder is known or nothing is cached.
    pub fn export_thumbnails(&self) -> Option<KeptThumbnails> {
        self.shared.lock().export_thumbnails()
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

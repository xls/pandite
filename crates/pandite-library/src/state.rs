//! Scheduler state behind the library mutex.
//!
//! Nothing in this module touches the disk, runs a decoder or calls a user
//! callback, so every method returns quickly and the lock is never held
//! across slow work. Workers take a [`Task`] out, run it unlocked, and
//! hand the [`Outcome`] back to [`State::finish`].

mod sched;

use crate::cache::{Cache, Entry, Rank, NO_INDEX};
use crate::folder::FolderList;
use crate::fs::{FileInfo, Stamp};
use crate::kept::{KeptThumbnail, KeptThumbnails};
use crate::{Event, Fetch, ListChange, LoadTimings, Reduced, Stats};
use pandite_core::{CancelToken, Content, Error, ImageBuf, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::{JoinHandle, ThreadId};

/// Tuning copied from [`crate::LibraryConfig`].
pub(crate) struct Settings {
    pub workers: usize,
    pub max_written_off: usize,
    pub read_ahead: usize,
    pub wrap: bool,
    pub margin: usize,
    pub background: bool,
    pub thumb_size: u32,
    pub full_budget: usize,
    pub thumb_budget: usize,
    /// Screen size for reduced decodes; `None` decodes read-ahead at full
    /// resolution and shows no reduced stage.
    pub screen: Option<(u32, u32)>,
}

impl Settings {
    /// The part of the image budget kept for screen-resolution images.
    pub fn screen_budget(&self) -> usize {
        if self.screen.is_some() {
            self.full_budget / 4
        } else {
            0
        }
    }

    /// Target of the quick first stage of the current image: half the
    /// screen, so an embedded preview of about that size qualifies.
    pub fn quick_target(&self) -> Option<(u32, u32)> {
        self.screen.map(|(w, h)| ((w / 2).max(1), (h / 2).max(1)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Full,
    /// A screen-resolution decode of a read-ahead neighbor.
    Screen,
    Thumb,
}

/// One unit of work handed to a worker.
pub(crate) struct Task {
    pub id: u64,
    pub cancel: CancelToken,
    pub work: Work,
}

pub(crate) enum Work {
    /// Decode the whole file. `explicit` is set for the current image: it
    /// is read even when its data is not stored locally. `quick` is the
    /// target of a reduced first stage decoded beside the full decode.
    Full {
        path: PathBuf,
        explicit: bool,
        quick: Option<(u32, u32)>,
    },
    /// Decode a read-ahead neighbor to cover `target` (screen size).
    Screen { path: PathBuf, target: (u32, u32) },
    /// Make a thumbnail for `side`, from cached pixels of the file when
    /// `from` is set.
    Thumb {
        path: PathBuf,
        from: Option<(Arc<Content>, Option<Stamp>)>,
        side: u32,
    },
    /// List a folder.
    List { gen: u64, listing: Listing },
}

/// What a folder listing job lists.
pub(crate) enum Listing {
    /// The folder of a file (or a folder itself), as
    /// [`FolderList::scan_with`] lists it.
    Scan(PathBuf),
    /// A check of `dir` with one directory listing
    /// ([`FolderList::relist_with`]), keeping the files of `keep`.
    Check { dir: PathBuf, keep: Vec<PathBuf> },
}

/// What the listing that made the list found about one file: its stamp
/// and whether its data is stored locally. `None` for a path no listing
/// reported.
type Facts = Option<(Stamp, bool)>;

/// What a worker reports back.
pub(crate) enum Outcome {
    Full(Result<Content>, Option<Stamp>),
    /// A screen-resolution decode, or the full decode of an image no
    /// larger than the screen (then cached as full).
    Screen(Result<Staged>, Option<Stamp>),
    Thumb(Result<ImageBuf>, Option<Stamp>),
    /// The file is not stored locally and the job was not explicit.
    NotLocal,
    /// The path is a directory (opened before its listing arrived).
    Directory,
    /// The job learned it was abandoned and stopped.
    Abandoned,
    Listed(Option<FolderList>),
}

/// What a screen task produced.
pub(crate) enum Staged {
    /// A reduced image standing in for a larger one.
    Reduced(Reduced),
    /// The image itself: it is no larger than the screen.
    Full(Content),
}

/// A cached thumbnail and the thumbnail size it was made for.
#[derive(Clone)]
pub(crate) struct Thumb {
    pub image: ImageBuf,
    pub side: u32,
}

/// A job in flight.
struct Running {
    id: u64,
    thread: u64,
    cancel: CancelToken,
    plan_gen: u64,
    plan_pos: Option<usize>,
    /// The thumbnail size a thumbnail job makes (0 for other jobs).
    side: u32,
    downloading: bool,
    /// The file is local and being read (or was read): a full job of the
    /// current image in this state holds other jobs back.
    local_read: bool,
    /// When the job started.
    started: std::time::Instant,
    /// The file changed: drop the result and run the job again.
    discard: bool,
}

struct ListRunning {
    id: u64,
    thread: u64,
    cancel: CancelToken,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// Counts toward the configured worker count.
    Pool,
    /// Stuck in a cancelled read; replaced and no longer counted.
    WrittenOff,
    /// Kept free for the current image: it takes no other work, so a newly
    /// selected file never waits for background work to finish. Not counted
    /// toward the configured worker count.
    Reserved,
}

struct ThreadSlot {
    handle: Option<JoinHandle<()>>,
    os_id: Option<ThreadId>,
    role: Role,
    /// Inside a file system call that may block indefinitely.
    io: bool,
    exited: bool,
}

struct ErrorEntry {
    error: Arc<Error>,
    stamp: Option<Stamp>,
}

#[derive(Default)]
struct Plan {
    gen: u64,
    jobs: Vec<PlanJob>,
    taken: Vec<bool>,
    cursor: usize,
    end: usize,
}

#[derive(Clone, Copy, Debug)]
struct PlanJob {
    kind: Kind,
    index: usize,
}

/// Rank of a full decode: distance from the current image, next side first.
#[derive(Clone, Copy)]
struct FullRanker {
    current: Option<usize>,
    len: usize,
    wrap: bool,
}

impl FullRanker {
    fn rank(self, index: usize) -> Rank {
        let Some(c) = self.current else {
            return (usize::MAX, false);
        };
        if index == NO_INDEX || index >= self.len {
            return (usize::MAX, true);
        }
        if self.wrap {
            let n = self.len;
            let forward = (index + n - c) % n;
            let backward = (c + n - index) % n;
            if forward <= backward {
                (forward, false)
            } else {
                (backward, true)
            }
        } else if index >= c {
            (index - c, false)
        } else {
            (c - index, true)
        }
    }
}

/// Rank of a thumbnail: distance from the visible gallery range.
#[derive(Clone, Copy)]
struct ThumbRanker {
    range: Option<(usize, usize)>,
}

impl ThumbRanker {
    fn rank(self, index: usize) -> Rank {
        let Some((start, end)) = self.range else {
            return (usize::MAX, false);
        };
        if index == NO_INDEX {
            (usize::MAX, true)
        } else if index < start {
            (start - index, true)
        } else if index >= end {
            (index - end + 1, false)
        } else {
            (0, false)
        }
    }
}

pub(crate) struct State {
    cfg: Settings,
    items: Vec<PathBuf>,
    /// What the listings found about each item, by index.
    facts: Vec<Facts>,
    index_of: HashMap<PathBuf, usize>,
    current: Option<usize>,
    visible: Option<(usize, usize)>,
    full: Cache<Arc<Content>>,
    /// Screen-resolution images (read-ahead and quick first stages).
    screen: Cache<Reduced>,
    /// Thumbnails. One made for another size than the current one is
    /// stale: it shows until its new thumbnail replaces it.
    thumbs: Cache<Thumb>,
    /// Thumbnails kept from an earlier library, waiting for the folder
    /// listing to check their files.
    kept: Option<KeptThumbnails>,
    full_errors: HashMap<PathBuf, ErrorEntry>,
    thumb_errors: HashMap<PathBuf, ErrorEntry>,
    /// Decoded byte size seen for a path, to skip read-ahead that cannot fit.
    full_sizes: HashMap<PathBuf, (usize, Option<Stamp>)>,
    non_local: HashSet<PathBuf>,
    user_cancelled: HashSet<PathBuf>,
    plan: Plan,
    running_full: HashMap<PathBuf, Running>,
    running_screen: HashMap<PathBuf, Running>,
    running_thumb: HashMap<PathBuf, Running>,
    /// Screen decodes that failed; the full decode runs when the image
    /// becomes current and reports any error.
    screen_failed: HashSet<PathBuf>,
    list_pending: Option<(u64, Listing)>,
    list_running: Option<ListRunning>,
    list_gen: u64,
    last_dir: Option<PathBuf>,
    /// The file passed to the last [`State::open`]: a folder check keeps
    /// it in the list, also with an extension the registry does not know.
    opened: Option<PathBuf>,
    /// Why the listing that made the list could not read its folder.
    listing_error: Option<io::ErrorKind>,
    events: VecDeque<Event>,
    next_id: u64,
    threads: HashMap<u64, ThreadSlot>,
    /// Stage durations of recent loads, by path.
    timings: HashMap<PathBuf, LoadTimings>,
    /// Set by `replan`; workers clear it and wake their peers.
    pub replanned: bool,
    /// Workers asleep on the work condition variable.
    pub waiting: usize,
    pub shutdown: bool,
}

impl State {
    pub fn new(cfg: Settings) -> Self {
        let screen_budget = cfg.screen_budget();
        let full_budget = cfg.full_budget - screen_budget;
        let thumb_budget = cfg.thumb_budget;
        Self {
            cfg,
            items: Vec::new(),
            facts: Vec::new(),
            index_of: HashMap::new(),
            current: None,
            visible: None,
            full: Cache::new(full_budget),
            screen: Cache::new(screen_budget),
            thumbs: Cache::new(thumb_budget),
            kept: None,
            full_errors: HashMap::new(),
            thumb_errors: HashMap::new(),
            full_sizes: HashMap::new(),
            non_local: HashSet::new(),
            user_cancelled: HashSet::new(),
            plan: Plan::default(),
            running_full: HashMap::new(),
            running_screen: HashMap::new(),
            running_thumb: HashMap::new(),
            screen_failed: HashSet::new(),
            list_pending: None,
            list_running: None,
            list_gen: 0,
            last_dir: None,
            opened: None,
            listing_error: None,
            events: VecDeque::new(),
            next_id: 1,
            threads: HashMap::new(),
            timings: HashMap::new(),
            replanned: false,
            waiting: 0,
            shutdown: false,
        }
    }

    fn full_ranker(&self) -> FullRanker {
        FullRanker {
            current: self.current,
            len: self.items.len(),
            wrap: self.cfg.wrap,
        }
    }

    fn thumb_range(&self) -> Option<(usize, usize)> {
        self.visible.or_else(|| self.current.map(|c| (c, c + 1)))
    }

    fn thumb_ranker(&self) -> ThumbRanker {
        ThumbRanker {
            range: self.thumb_range(),
        }
    }

    fn in_window(&self, index: usize) -> bool {
        self.current.is_some() && self.full_ranker().rank(index).0 <= self.cfg.read_ahead
    }

    /// True when full resolution is decoded for the current image only
    /// and read-ahead holds screen-resolution images.
    fn staged(&self) -> bool {
        self.cfg.screen.is_some()
    }

    /// Change the screen size of reduced decodes. Cached screen images
    /// stay; they are stand-ins until the full image arrives.
    pub fn set_screen(&mut self, size: (u32, u32)) {
        if self.cfg.screen.is_some() {
            self.cfg.screen = Some((size.0.max(1), size.1.max(1)));
        }
    }

    pub fn thumb_size(&self) -> u32 {
        self.cfg.thumb_size
    }

    /// Make thumbnails for `side` from now on. Cached thumbnails of another
    /// size become stale and are made again in plan order; kept thumbnails
    /// of another size are dropped.
    pub fn set_thumb_size(&mut self, side: u32) {
        let side = side.max(1);
        if side == self.cfg.thumb_size {
            return;
        }
        self.cfg.thumb_size = side;
        if self.kept.as_ref().is_some_and(|k| k.thumb_size != side) {
            self.kept = None;
        }
        self.replan();
    }

    /// True when the cached thumbnail of `path` has the current size.
    fn thumb_fresh(&self, path: &Path) -> bool {
        self.thumbs
            .get(path)
            .is_some_and(|e| e.value.side == self.cfg.thumb_size)
    }

    /// The screen image of `path` when a thumbnail can be made from it: its
    /// short side reaches the thumbnail size. A screen image of a panorama
    /// can be too low for that.
    fn screen_source(&self, path: &Path) -> Option<&Entry<Reduced>> {
        let e = self.screen.get(path)?;
        let image = &e.value.image;
        (image.width().min(image.height()) >= self.cfg.thumb_size).then_some(e)
    }

    /// The stale thumbnail of `path` when the new one can be made from it:
    /// the new size is not larger, or the old thumbnail holds its whole
    /// source (it is smaller than the size it was made for).
    fn stale_source(&self, path: &Path) -> Option<&ImageBuf> {
        let e = self.thumbs.get(path)?;
        let old = &e.value;
        let whole = old.image.width().max(old.image.height()) < old.side;
        (old.side != self.cfg.thumb_size && (self.cfg.thumb_size <= old.side || whole))
            .then_some(&old.image)
    }

    fn thumb_wanted(&self, index: usize) -> bool {
        let (distance, _) = self.thumb_ranker().rank(index);
        distance != usize::MAX
            && (distance == 0 || self.cfg.background || distance <= self.cfg.margin)
    }

    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Queue an event for the interface.
    fn push(&mut self, event: Event) {
        self.events.push_back(event);
    }

    // ----- caller side -----

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn path(&self, index: usize) -> Option<PathBuf> {
        self.items.get(index).cloned()
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.items.clone()
    }

    pub fn current(&self) -> Option<usize> {
        self.current
    }

    pub fn listing_error(&self) -> Option<io::ErrorKind> {
        self.listing_error
    }

    pub fn any_not_local(&self) -> bool {
        !self.non_local.is_empty()
    }

    /// Show `path` at once as a one-item list and list its folder on a worker.
    pub fn open(&mut self, path: PathBuf) {
        if self.kept.as_ref().is_some_and(|k| !k.serves(&path)) {
            // Another folder: free the kept thumbnails now.
            self.kept = None;
        }
        self.cancel_listing();
        self.list_gen += 1;
        self.list_pending = Some((self.list_gen, Listing::Scan(path.clone())));
        self.last_dir = path.parent().map(Path::to_path_buf);
        self.opened = Some(path.clone());
        self.listing_error = None;
        self.user_cancelled.remove(&path);
        self.replace_items(vec![path], vec![None], Some(0), false);
        self.preempt();
    }

    /// List the folder again on a worker.
    pub fn refresh(&mut self) {
        let target = self
            .current
            .and_then(|c| self.items.get(c).cloned())
            .or_else(|| self.last_dir.clone());
        let Some(target) = target else { return };
        self.cancel_listing();
        self.list_gen += 1;
        self.list_pending = Some((self.list_gen, Listing::Scan(target)));
    }

    /// Look on a worker whether the folder of the list changed. Nothing
    /// happens while a listing is queued or runs, or with no folder.
    pub fn check(&mut self) {
        if self.shutdown || self.list_pending.is_some() || self.list_running.is_some() {
            return;
        }
        let Some(dir) = self.last_dir.clone() else {
            return;
        };
        let current = self.current.and_then(|c| self.items.get(c));
        let opened = self
            .opened
            .as_ref()
            .filter(|p| self.index_of.contains_key(p.as_path()));
        let mut keep: Vec<PathBuf> = current.into_iter().chain(opened).cloned().collect();
        keep.dedup();
        self.list_gen += 1;
        self.list_pending = Some((self.list_gen, Listing::Check { dir, keep }));
    }

    pub fn set_items(&mut self, paths: Vec<PathBuf>) -> Option<usize> {
        self.kept = None;
        self.cancel_listing();
        self.list_pending = None;
        self.list_gen += 1;
        self.listing_error = None;
        let current = self
            .current
            .and_then(|c| self.items.get(c))
            .and_then(|p| paths.iter().position(|x| x == p));
        // Facts follow their path.
        let facts = paths
            .iter()
            .map(|p| {
                let old = *self.index_of.get(p)?;
                self.facts.get(old).copied().flatten()
            })
            .collect();
        self.replace_items(paths, facts, current, true);
        self.current
    }

    pub fn set_current(&mut self, index: usize) {
        let Some(path) = self.items.get(index) else {
            return;
        };
        let retry = self.user_cancelled.remove(path);
        if Some(index) == self.current && !retry {
            return;
        }
        self.current = Some(index);
        self.replan();
        self.cancel_stale();
        self.preempt();
    }

    pub fn set_visible(&mut self, range: Range<usize>) {
        let n = self.items.len();
        let (start, end) = (range.start.min(n), range.end.min(n));
        let visible = (start < end).then_some((start, end));
        if visible == self.visible {
            return;
        }
        self.visible = visible;
        self.replan();
        self.cancel_stale();
    }

    pub fn cancel_current(&mut self) {
        let Some(path) = self.current.and_then(|c| self.items.get(c)).cloned() else {
            return;
        };
        if self.full.contains(&path) || self.full_errors.contains_key(&path) {
            return;
        }
        self.cancel_job(Kind::Full, &path);
        self.cancel_job(Kind::Thumb, &path);
        self.user_cancelled.insert(path);
    }

    pub fn invalidate(&mut self, path: &Path) {
        self.full.remove(path);
        self.screen.remove(path);
        self.screen_failed.remove(path);
        self.thumbs.remove(path);
        if let Some(kept) = &mut self.kept {
            kept.entries.remove(path);
        }
        self.full_errors.remove(path);
        self.thumb_errors.remove(path);
        self.full_sizes.remove(path);
        self.non_local.remove(path);
        self.user_cancelled.remove(path);
        for kind in [Kind::Full, Kind::Screen, Kind::Thumb] {
            let map = self.running_mut(kind);
            if let Some(running) = map.get_mut(path) {
                running.discard = true;
            }
            self.cancel_job(kind, path);
        }
        self.replan();
        self.cancel_stale();
    }

    pub fn get(&self, index: usize) -> Fetch<Arc<Content>> {
        let Some(path) = self.items.get(index) else {
            return out_of_range(index);
        };
        if let Some(entry) = self.full.get(path) {
            return Fetch::Ready(Arc::clone(&entry.value));
        }
        if let Some(err) = self.full_errors.get(path) {
            return Fetch::Failed(Arc::clone(&err.error));
        }
        if self.user_cancelled.contains(path) {
            return Fetch::Cancelled;
        }
        let downloading = self.running_full.get(path).is_some_and(|r| r.downloading);
        let non_local = self.non_local.contains(path);
        if downloading || (non_local && Some(index) == self.current) {
            return Fetch::Downloading;
        }
        if non_local {
            return Fetch::NotLocal;
        }
        Fetch::Pending
    }

    /// The screen-resolution image of `index`, when cached.
    pub fn reduced(&self, index: usize) -> Option<Reduced> {
        let path = self.items.get(index)?;
        self.screen.get(path).map(|e| e.value.clone())
    }

    pub fn thumbnail(&self, index: usize) -> Fetch<ImageBuf> {
        let Some(path) = self.items.get(index) else {
            return out_of_range(index);
        };
        if let Some(entry) = self.thumbs.get(path) {
            return Fetch::Ready(entry.value.image.clone());
        }
        if let Some(err) = self.thumb_errors.get(path) {
            return Fetch::Failed(Arc::clone(&err.error));
        }
        if self.user_cancelled.contains(path) {
            return Fetch::Cancelled;
        }
        if self.non_local.contains(path) && !self.full.contains(path) && !self.screen.contains(path)
        {
            return Fetch::NotLocal;
        }
        Fetch::Pending
    }

    pub fn timings(&self, path: &Path) -> Option<LoadTimings> {
        self.timings.get(path).copied()
    }

    /// Remember the stage durations of a load. Only the paths in the list
    /// are kept, so the map stays as small as the folder. A reduced stage
    /// recorded earlier for the same load stays.
    pub fn note_timings(&mut self, path: &Path, timings: LoadTimings) {
        if self.index_of.contains_key(path) {
            let entry = self.timings.entry(path.to_path_buf()).or_default();
            entry.read = timings.read;
            entry.decode_full = timings.decode_full;
            if timings.decode_reduced.is_some() {
                entry.decode_reduced = timings.decode_reduced;
            }
        }
    }

    /// Remember how long the quick stage of a load took.
    pub fn note_reduced_time(&mut self, path: &Path, took: std::time::Duration) {
        if self.index_of.contains_key(path) {
            let entry = self.timings.entry(path.to_path_buf()).or_default();
            entry.decode_reduced = Some(took);
        }
    }

    pub fn drain_events(&mut self) -> Vec<Event> {
        self.events.drain(..).collect()
    }

    pub fn stats(&self) -> Stats {
        let (threads, written_off) = self.thread_counts();
        Stats {
            full_bytes: self.full.used(),
            full_entries: self.full.len(),
            full_budget: self.full.budget(),
            screen_bytes: self.screen.used(),
            screen_entries: self.screen.len(),
            screen_budget: self.screen.budget(),
            thumb_bytes: self.thumbs.used(),
            thumb_entries: self.thumbs.len(),
            thumb_budget: self.thumbs.budget(),
            kept_bytes: self.kept.as_ref().map_or(0, KeptThumbnails::bytes),
            kept_entries: self.kept.as_ref().map_or(0, KeptThumbnails::len),
            running: self.running_full.len()
                + self.running_screen.len()
                + self.running_thumb.len()
                + usize::from(self.list_running.is_some()),
            threads,
            written_off,
        }
    }

    /// True when no job runs, none can start, and no written-off thread is
    /// still stuck.
    pub fn is_idle(&mut self) -> bool {
        self.running_full.is_empty()
            && self.running_screen.is_empty()
            && self.running_thumb.is_empty()
            && self.list_running.is_none()
            && self.list_pending.is_none()
            && self.thread_counts().1 == 0
            && self.scan(usize::MAX).is_none()
    }

    /// Replace the folder list; `facts` holds what the listings found about
    /// each path. Cached data follows its path to the new index. Paths
    /// that left the list are dropped when `drop_missing` is set,
    /// otherwise kept at the back of the eviction line.
    fn replace_items(
        &mut self,
        paths: Vec<PathBuf>,
        facts: Vec<Facts>,
        current: Option<usize>,
        drop_missing: bool,
    ) {
        self.index_of = paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), i))
            .collect();
        self.items = paths;
        self.facts = facts;
        let index_of = &self.index_of;
        self.full.remap(|p| index_of.get(p).copied(), drop_missing);
        self.screen
            .remap(|p| index_of.get(p).copied(), drop_missing);
        self.thumbs
            .remap(|p| index_of.get(p).copied(), drop_missing);
        if drop_missing {
            self.full_errors.retain(|p, _| index_of.contains_key(p));
            self.thumb_errors.retain(|p, _| index_of.contains_key(p));
            self.full_sizes.retain(|p, _| index_of.contains_key(p));
            self.screen_failed.retain(|p| index_of.contains_key(p));
            self.non_local.retain(|p| index_of.contains_key(p));
            self.user_cancelled.retain(|p| index_of.contains_key(p));
        }
        self.current = current.filter(|&c| c < self.items.len());
        self.visible = None;
        self.replan();
        self.cancel_stale();
    }

    /// Fold a finished folder listing in: drop cached data whose file
    /// changed, record which files are not local, keep the current image
    /// selected by path, and cache the kept thumbnails whose file did not
    /// change (each announced with [`Event::ThumbnailReady`] after
    /// [`Event::Listed`]). The listing of a folder `check` that finds the
    /// list as it is changes nothing and queues no event; one that finds
    /// a change is announced with [`Event::Changed`].
    fn merge_listing(&mut self, list: FolderList, check: bool) {
        let change = if check {
            let Some(change) = self.list_change(&list) else {
                return;
            };
            Some(change)
        } else {
            None
        };
        let current_path = self.current.and_then(|c| self.items.get(c).cloned());
        let new_current = current_path.as_ref().and_then(|p| {
            list.index_of(p).or_else(|| {
                if p.as_path() == list.dir() {
                    (!list.is_empty()).then_some(0)
                } else {
                    list.nearest_index(p)
                }
            })
        });
        {
            let map = list.index_map();
            let unchanged = |path: &Path, stamp: Option<Stamp>| match map.get(path) {
                Some(&i) => list.info(i).is_some_and(|f| Some(f.stamp) == stamp),
                None => true,
            };
            self.full.retain(|p, e| unchanged(p, e.stamp));
            self.screen.retain(|p, e| unchanged(p, e.stamp));
            self.thumbs.retain(|p, e| unchanged(p, e.stamp));
            self.full_errors.retain(|p, e| unchanged(p, e.stamp));
            self.thumb_errors.retain(|p, e| unchanged(p, e.stamp));
            self.full_sizes.retain(|p, e| unchanged(p, e.1));
            self.non_local = list
                .paths()
                .iter()
                .enumerate()
                .filter(|(i, _)| list.info(*i).is_some_and(|f| !f.local))
                .map(|(_, p)| p.clone())
                .collect();
        }
        self.listing_error = list.listing_error();
        let (dir, paths, info) = list.into_parts();
        // Kept thumbnails of another folder or size are dropped here.
        let side = self.cfg.thumb_size;
        let kept = self
            .kept
            .take()
            .filter(|k| k.folder == dir && k.thumb_size == side);
        self.last_dir = Some(dir);
        let facts = info.iter().map(|f| Some((f.stamp, f.local))).collect();
        self.replace_items(paths, facts, new_current, true);
        let seeded = kept.map(|k| self.seed_thumbnails(k, &info));
        let (len, current) = (self.items.len(), self.current);
        self.push(match change {
            Some(change) => Event::Changed {
                len,
                current,
                change,
            },
            None => Event::Listed { len, current },
        });
        for (index, path) in seeded.into_iter().flatten() {
            self.push(Event::ThumbnailReady { index, path });
        }
    }

    /// What `list` would change in the list, or `None` when it holds the
    /// same paths in the same order, each with the same stamp and
    /// locality, and the same listing error.
    fn list_change(&self, list: &FolderList) -> Option<ListChange> {
        let facts_of = |i: usize| list.info(i).map(|f| (f.stamp, f.local));
        let fact = |i: usize| self.facts.get(i).copied().flatten();
        let same = list.listing_error() == self.listing_error
            && list.paths() == self.items.as_slice()
            && (0..list.len()).all(|i| fact(i) == facts_of(i));
        if same {
            return None;
        }
        let map = list.index_map();
        let moved: Vec<Option<usize>> = self
            .items
            .iter()
            .map(|p| map.get(p.as_path()).copied())
            .collect();
        let removed = moved.iter().filter(|m| m.is_none()).count();
        let added = list.len().saturating_sub(moved.len() - removed);
        let stayed = || moved.iter().flatten();
        let reordered = stayed().zip(stayed().skip(1)).any(|(a, b)| a >= b);
        let mut modified: Vec<usize> = moved
            .iter()
            .enumerate()
            .filter_map(|(old, new)| new.filter(|&n| fact(old) != facts_of(n)))
            .collect();
        modified.sort_unstable();
        Some(ListChange {
            moved,
            added,
            removed,
            reordered,
            modified,
        })
    }

    /// Hold thumbnails kept from an earlier library until the folder
    /// listing checks their files. Thumbnails of another size are dropped.
    pub fn keep_thumbnails(&mut self, kept: KeptThumbnails) {
        if kept.thumb_size == self.cfg.thumb_size && !kept.is_empty() {
            self.kept = Some(kept);
        }
    }

    /// Cache the kept thumbnails whose file still has the stamp it had
    /// when the thumbnail was made, best ranked first, within the budget.
    /// `info` holds the listing's facts by index. Returns the index and
    /// path of each thumbnail cached.
    fn seed_thumbnails(
        &mut self,
        kept: KeptThumbnails,
        info: &[FileInfo],
    ) -> Vec<(usize, PathBuf)> {
        let ranker = self.thumb_ranker();
        let side = kept.thumb_size;
        let mut fresh: Vec<(Rank, usize, PathBuf, KeptThumbnail)> = kept
            .entries
            .into_iter()
            .filter_map(|(path, entry)| {
                let index = *self.index_of.get(&path)?;
                let unchanged = entry.stamp.modified.is_some()
                    && info.get(index).is_some_and(|f| f.stamp == entry.stamp);
                // A thumbnail made in this session is newer.
                let known = self.thumbs.contains(&path) || self.thumb_errors.contains_key(&path);
                (unchanged && !known).then(|| (ranker.rank(index), index, path, entry))
            })
            .collect();
        fresh.sort_by_key(|a| (a.0, a.1));
        let mut seeded = Vec::new();
        for (rank, index, path, entry) in fresh {
            let cached = Entry {
                bytes: entry.image.byte_size(),
                value: Thumb {
                    image: entry.image,
                    side,
                },
                index,
                stamp: Some(entry.stamp),
            };
            if self
                .thumbs
                .insert(path.clone(), cached, rank, rank.0 == 0, |i| ranker.rank(i))
            {
                seeded.push((index, path));
            }
        }
        seeded
    }

    /// The thumbnails of the folder for the next library, best ranked
    /// first (visible range, then nearest the current image) within the
    /// thumbnail budget. Stale thumbnails are left out. Kept thumbnails not
    /// used yet come last.
    pub fn export_thumbnails(&self) -> Option<KeptThumbnails> {
        let folder = self
            .last_dir
            .clone()
            .or_else(|| self.kept.as_ref().map(|k| k.folder.clone()))?;
        let thumbs = self.thumb_ranker();
        let full = self.full_ranker();
        let worst = ((usize::MAX, true), (usize::MAX, true));
        let side = self.cfg.thumb_size;
        let mut candidates: Vec<((Rank, Rank), &Path, KeptThumbnail)> = self
            .thumbs
            .iter()
            .filter(|(path, e)| path.parent() == Some(folder.as_path()) && e.value.side == side)
            .filter_map(|(path, e)| {
                let stamp = e.stamp.filter(|s| s.modified.is_some())?;
                let key = (thumbs.rank(e.index), full.rank(e.index));
                let image = e.value.image.clone();
                Some((key, path, KeptThumbnail { stamp, image }))
            })
            .collect();
        if let Some(kept) = self
            .kept
            .as_ref()
            .filter(|k| k.folder == folder && k.thumb_size == side)
        {
            candidates.extend(
                kept.entries
                    .iter()
                    .filter(|(path, _)| !self.thumbs.contains(path))
                    .map(|(path, entry)| (worst, path.as_path(), entry.clone())),
            );
        }
        candidates.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
        let budget = self.thumbs.budget();
        let mut used = 0usize;
        let mut entries = HashMap::new();
        for (_, path, entry) in candidates {
            let total = used.saturating_add(entry.image.byte_size());
            if total <= budget {
                used = total;
                entries.insert(path.to_path_buf(), entry);
            }
        }
        (!entries.is_empty()).then_some(KeptThumbnails {
            folder,
            thumb_size: self.cfg.thumb_size,
            entries,
        })
    }

    // ----- worker thread bookkeeping -----

    /// Live pool threads and live written-off threads.
    pub fn thread_counts(&self) -> (usize, usize) {
        let live = || self.threads.values().filter(|t| !t.exited);
        let pool = live().filter(|t| t.role == Role::Pool).count();
        let off = live().filter(|t| t.role == Role::WrittenOff).count();
        (pool, off)
    }

    /// True when the pool is short and the written-off cap allows a new thread.
    pub fn wants_thread(&self) -> bool {
        let (pool, off) = self.thread_counts();
        !self.shutdown
            && pool + off < self.cfg.workers + self.cfg.max_written_off
            && (pool < self.cfg.workers || self.reserved_thread().is_none())
    }

    /// The live thread kept for the current image, when there is one.
    pub fn reserved_thread(&self) -> Option<u64> {
        self.threads
            .iter()
            .find(|(_, t)| !t.exited && t.role == Role::Reserved)
            .map(|(&tid, _)| tid)
    }

    /// True when `tid` is the thread kept for the current image.
    pub fn is_reserved(&self, tid: u64) -> bool {
        self.threads
            .get(&tid)
            .is_some_and(|t| t.role == Role::Reserved)
    }

    /// Register a thread before it is spawned, so it finds its slot.
    pub fn reserve_thread(&mut self) -> u64 {
        let tid = self.new_id();
        // The thread for the current image comes first: it must exist
        // before background work can occupy the pool.
        let role = if self.reserved_thread().is_none() {
            Role::Reserved
        } else {
            Role::Pool
        };
        self.threads.insert(
            tid,
            ThreadSlot {
                handle: None,
                os_id: None,
                role,
                io: false,
                exited: false,
            },
        );
        tid
    }

    pub fn attach_thread(&mut self, tid: u64, handle: JoinHandle<()>) {
        if let Some(slot) = self.threads.get_mut(&tid) {
            slot.os_id = Some(handle.thread().id());
            slot.handle = Some(handle);
        }
    }

    pub fn release_thread(&mut self, tid: u64) {
        self.threads.remove(&tid);
    }

    /// Remove slots of threads that exited and return their handles.
    pub fn reap(&mut self) -> Vec<JoinHandle<()>> {
        let gone: Vec<u64> = self
            .threads
            .iter()
            .filter(|(_, t)| t.exited && t.handle.is_some())
            .map(|(&tid, _)| tid)
            .collect();
        gone.into_iter()
            .filter_map(|tid| self.threads.remove(&tid)?.handle)
            .collect()
    }

    fn set_io(&mut self, tid: u64, io: bool) {
        if let Some(slot) = self.threads.get_mut(&tid) {
            slot.io = io;
        }
    }

    fn write_off(&mut self, tid: u64) {
        if let Some(slot) = self.threads.get_mut(&tid) {
            slot.role = Role::WrittenOff;
        }
    }

    /// After a task: false when the thread must exit. A written-off thread
    /// rejoins the pool only when the pool is short.
    pub fn after_task(&mut self, tid: u64) -> bool {
        if self.shutdown {
            return false;
        }
        let (pool, _) = self.thread_counts();
        let target = self.cfg.workers;
        match self.threads.get_mut(&tid) {
            None => false,
            Some(slot) if slot.role == Role::WrittenOff => {
                if pool < target {
                    slot.role = Role::Pool;
                    true
                } else {
                    false
                }
            }
            Some(_) => true,
        }
    }

    pub fn exit_thread(&mut self, tid: u64) {
        if let Some(slot) = self.threads.get_mut(&tid) {
            slot.exited = true;
            slot.io = false;
        }
    }

    /// Stop everything: no new tasks, every running job cancelled.
    pub fn begin_shutdown(&mut self) {
        self.shutdown = true;
        self.list_pending = None;
        // A worker stuck in a read keeps this state alive: do not keep
        // the pixels with it.
        self.kept = None;
        for running in self
            .running_full
            .values()
            .chain(self.running_screen.values())
            .chain(self.running_thumb.values())
        {
            running.cancel.cancel();
        }
        if let Some(list) = &self.list_running {
            list.cancel.cancel();
        }
    }

    /// True while some thread other than `me` is neither exited nor inside
    /// a file system call: such a thread is expected to stop soon.
    pub fn shutdown_waiting(&self, me: ThreadId) -> bool {
        self.threads
            .values()
            .any(|t| !t.exited && !t.io && t.os_id != Some(me))
    }

    /// Hand out every thread handle with its exited flag.
    pub fn take_threads(&mut self) -> Vec<(bool, JoinHandle<()>)> {
        self.threads
            .drain()
            .filter_map(|(_, t)| Some((t.exited, t.handle?)))
            .collect()
    }
}

fn out_of_range<T>(index: usize) -> Fetch<T> {
    Fetch::Failed(Arc::new(Error::Invalid(format!(
        "index {index} is outside the folder list"
    ))))
}

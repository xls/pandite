//! Scheduler state behind the library mutex.
//!
//! Nothing in this module touches the disk, runs a decoder or calls a user
//! callback, so every method returns quickly and the lock is never held
//! across slow work. Workers take a [`Task`] out, run it unlocked, and
//! hand the [`Outcome`] back to [`State::finish`].

mod sched;

use crate::cache::{Cache, Rank, NO_INDEX};
use crate::folder::FolderList;
use crate::fs::Stamp;
use crate::{Event, Fetch, Stats};
use pandite_core::{CancelToken, Content, Error, ImageBuf, Result};
use std::collections::{HashMap, HashSet, VecDeque};
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Full,
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
    /// is read even when its data is not stored locally.
    Full { path: PathBuf, explicit: bool },
    /// Make a thumbnail, from a cached full decode when `from` is set.
    Thumb {
        path: PathBuf,
        from: Option<(Arc<Content>, Option<Stamp>)>,
    },
    /// List the folder of `target`.
    List { gen: u64, target: PathBuf },
}

/// What a worker reports back.
pub(crate) enum Outcome {
    Full(Result<Content>, Option<Stamp>),
    Thumb(Result<ImageBuf>, Option<Stamp>),
    /// The file is not stored locally and the job was not explicit.
    NotLocal,
    /// The path is a directory (opened before its listing arrived).
    Directory,
    /// The job learned it was abandoned and stopped.
    Abandoned,
    Listed(Option<FolderList>),
}

/// A job in flight.
struct Running {
    id: u64,
    thread: u64,
    cancel: CancelToken,
    plan_gen: u64,
    plan_pos: Option<usize>,
    downloading: bool,
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
    index_of: HashMap<PathBuf, usize>,
    current: Option<usize>,
    visible: Option<(usize, usize)>,
    full: Cache<Arc<Content>>,
    thumbs: Cache<ImageBuf>,
    full_errors: HashMap<PathBuf, ErrorEntry>,
    thumb_errors: HashMap<PathBuf, ErrorEntry>,
    /// Decoded byte size seen for a path, to skip read-ahead that cannot fit.
    full_sizes: HashMap<PathBuf, (usize, Option<Stamp>)>,
    non_local: HashSet<PathBuf>,
    user_cancelled: HashSet<PathBuf>,
    plan: Plan,
    running_full: HashMap<PathBuf, Running>,
    running_thumb: HashMap<PathBuf, Running>,
    list_pending: Option<(u64, PathBuf)>,
    list_running: Option<ListRunning>,
    list_gen: u64,
    last_dir: Option<PathBuf>,
    events: VecDeque<Event>,
    next_id: u64,
    threads: HashMap<u64, ThreadSlot>,
    /// Set by `replan`; workers clear it and wake their peers.
    pub replanned: bool,
    /// Workers asleep on the work condition variable.
    pub waiting: usize,
    pub shutdown: bool,
}

impl State {
    pub fn new(cfg: Settings) -> Self {
        let (full_budget, thumb_budget) = (cfg.full_budget, cfg.thumb_budget);
        Self {
            cfg,
            items: Vec::new(),
            index_of: HashMap::new(),
            current: None,
            visible: None,
            full: Cache::new(full_budget),
            thumbs: Cache::new(thumb_budget),
            full_errors: HashMap::new(),
            thumb_errors: HashMap::new(),
            full_sizes: HashMap::new(),
            non_local: HashSet::new(),
            user_cancelled: HashSet::new(),
            plan: Plan::default(),
            running_full: HashMap::new(),
            running_thumb: HashMap::new(),
            list_pending: None,
            list_running: None,
            list_gen: 0,
            last_dir: None,
            events: VecDeque::new(),
            next_id: 1,
            threads: HashMap::new(),
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

    /// Show `path` at once as a one-item list and list its folder on a worker.
    pub fn open(&mut self, path: PathBuf) {
        self.cancel_listing();
        self.list_gen += 1;
        self.list_pending = Some((self.list_gen, path.clone()));
        self.last_dir = path.parent().map(Path::to_path_buf);
        self.user_cancelled.remove(&path);
        self.replace_items(vec![path], Some(0), false);
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
        self.list_pending = Some((self.list_gen, target));
    }

    pub fn set_items(&mut self, paths: Vec<PathBuf>) -> Option<usize> {
        self.cancel_listing();
        self.list_pending = None;
        self.list_gen += 1;
        let current = self
            .current
            .and_then(|c| self.items.get(c))
            .and_then(|p| paths.iter().position(|x| x == p));
        self.replace_items(paths, current, true);
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
        self.thumbs.remove(path);
        self.full_errors.remove(path);
        self.thumb_errors.remove(path);
        self.full_sizes.remove(path);
        self.non_local.remove(path);
        self.user_cancelled.remove(path);
        for kind in [Kind::Full, Kind::Thumb] {
            let map = match kind {
                Kind::Full => &mut self.running_full,
                Kind::Thumb => &mut self.running_thumb,
            };
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

    pub fn thumbnail(&self, index: usize) -> Fetch<ImageBuf> {
        let Some(path) = self.items.get(index) else {
            return out_of_range(index);
        };
        if let Some(entry) = self.thumbs.get(path) {
            return Fetch::Ready(entry.value.clone());
        }
        if let Some(err) = self.thumb_errors.get(path) {
            return Fetch::Failed(Arc::clone(&err.error));
        }
        if self.user_cancelled.contains(path) {
            return Fetch::Cancelled;
        }
        if self.non_local.contains(path) && !self.full.contains(path) {
            return Fetch::NotLocal;
        }
        Fetch::Pending
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
            thumb_bytes: self.thumbs.used(),
            thumb_entries: self.thumbs.len(),
            thumb_budget: self.thumbs.budget(),
            running: self.running_full.len()
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
            && self.running_thumb.is_empty()
            && self.list_running.is_none()
            && self.list_pending.is_none()
            && self.thread_counts().1 == 0
            && self.scan(usize::MAX).is_none()
    }

    /// Replace the folder list. Cached data follows its path to the new
    /// index. Paths that left the list are dropped when `drop_missing` is
    /// set, otherwise kept at the back of the eviction line.
    fn replace_items(&mut self, paths: Vec<PathBuf>, current: Option<usize>, drop_missing: bool) {
        self.index_of = paths
            .iter()
            .enumerate()
            .map(|(i, p)| (p.clone(), i))
            .collect();
        self.items = paths;
        let index_of = &self.index_of;
        self.full.remap(|p| index_of.get(p).copied(), drop_missing);
        self.thumbs
            .remap(|p| index_of.get(p).copied(), drop_missing);
        if drop_missing {
            self.full_errors.retain(|p, _| index_of.contains_key(p));
            self.thumb_errors.retain(|p, _| index_of.contains_key(p));
            self.full_sizes.retain(|p, _| index_of.contains_key(p));
            self.non_local.retain(|p| index_of.contains_key(p));
            self.user_cancelled.retain(|p| index_of.contains_key(p));
        }
        self.current = current.filter(|&c| c < self.items.len());
        self.visible = None;
        self.replan();
        self.cancel_stale();
    }

    /// Fold a finished folder listing in: drop cached data whose file
    /// changed, record which files are not local, and keep the current
    /// image selected by path.
    fn merge_listing(&mut self, list: FolderList) {
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
        let (dir, paths, _) = list.into_parts();
        self.last_dir = Some(dir);
        self.replace_items(paths, new_current, true);
        self.push(Event::Listed {
            len: self.items.len(),
            current: self.current,
        });
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
            && pool < self.cfg.workers
            && pool + off < self.cfg.workers + self.cfg.max_written_off
    }

    /// Register a thread before it is spawned, so it finds its slot.
    pub fn reserve_thread(&mut self) -> u64 {
        let tid = self.new_id();
        self.threads.insert(
            tid,
            ThreadSlot {
                handle: None,
                os_id: None,
                role: Role::Pool,
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
        for running in self
            .running_full
            .values()
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

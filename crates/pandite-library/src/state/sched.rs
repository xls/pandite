//! The priority plan, job selection, cancellation and job completion.
//!
//! The plan is an ordered list of wanted jobs, rebuilt whenever the
//! current image, the visible gallery range or the folder list changes:
//! the current image, then read-ahead neighbors (next, previous, next+1,
//! previous+1, ...), then thumbnails for the visible range from the
//! current image outward, then the margin, then the rest. Workers always
//! take the first job that can start, so a newly current image is next in
//! line the moment the plan is rebuilt.
//!
//! With a screen size set (staged mode), the current image is the only
//! one decoded at full resolution; its job also delivers a quick reduced
//! first stage. Read-ahead neighbors are decoded at screen resolution. A
//! full decode of an image that is no longer current is cancelled.
//!
//! While the current image loads (unless its file is being downloaded), no
//! other job starts except the folder listing: decoders use every core,
//! and read-ahead or thumbnails would slow the image the user waits for.
//! Jobs already running finish.

use super::{ErrorEntry, Role, Staged, Thumb};
use super::{Kind, ListRunning, Outcome, PlanJob, Running, State, Task, Work};
use crate::cache::Entry;
use crate::fs::Stamp;
use crate::{Event, Reduced};
use pandite_core::{CancelToken, Content, ImageBuf};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long the current image's job holds other jobs back before its file
/// is known to be local: a lookup slower than this (a stalled network
/// share) lets the rest of the folder go on.
const LOOKUP_GRACE: Duration = Duration::from_millis(250);
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Bytes per pixel assumed for a thumbnail before it exists (Rgba8).
const THUMB_BYTES_PER_PIXEL: usize = 4;

/// What a plan entry needs right now.
enum Class {
    /// Cached, failed, cancelled by the user, not local or cannot fit.
    Done,
    /// A worker runs it.
    Running,
    /// Waits for the full decode of the same file, which is running.
    Wait,
    /// The thumbnail budget is full of better entries: stop here.
    Saturated,
    /// Can start now.
    Go,
}

impl State {
    /// The running jobs of one kind.
    pub(super) fn running_mut(&mut self, kind: Kind) -> &mut HashMap<PathBuf, Running> {
        match kind {
            Kind::Full => &mut self.running_full,
            Kind::Screen => &mut self.running_screen,
            Kind::Thumb => &mut self.running_thumb,
        }
    }

    /// The running jobs of one kind.
    fn running(&self, kind: Kind) -> &HashMap<PathBuf, Running> {
        match kind {
            Kind::Full => &self.running_full,
            Kind::Screen => &self.running_screen,
            Kind::Thumb => &self.running_thumb,
        }
    }

    /// True while the current image's full job holds other jobs back: from
    /// its start, while its file is read (once known to be local) and
    /// while it decodes. A file being downloaded holds nothing up, and a
    /// lookup of the file's facts that has not answered within
    /// [`LOOKUP_GRACE`] stops holding, so a stalled lookup or download
    /// never stops the rest of the folder.
    fn current_decoding(&self) -> bool {
        let Some(path) = self.current.and_then(|c| self.items.get(c)) else {
            return false;
        };
        self.running_full.get(path).is_some_and(|r| {
            let in_io = self.threads.get(&r.thread).is_some_and(|t| t.io);
            !r.cancel.is_cancelled()
                && !r.downloading
                && (r.local_read || !in_io || r.started.elapsed() < LOOKUP_GRACE)
        })
    }

    /// Time left until the current image's lookup grace ends, while other
    /// jobs wait on it; a waiting worker looks again then.
    pub fn gate_ends(&self) -> Option<Duration> {
        let path = self.current.and_then(|c| self.items.get(c))?;
        let r = self.running_full.get(path)?;
        let in_io = self.threads.get(&r.thread).is_some_and(|t| t.io);
        if r.cancel.is_cancelled() || r.downloading || r.local_read || !in_io {
            return None;
        }
        LOOKUP_GRACE
            .checked_sub(r.started.elapsed())
            .map(|left| left + Duration::from_millis(1))
    }

    /// A worker found the file of `task` stored locally and starts reading
    /// it.
    pub fn mark_local_read(&mut self, task: &Task) {
        if let Work::Full { path, .. } = &task.work {
            if let Some(running) = self.running_full.get_mut(path).filter(|r| r.id == task.id) {
                running.local_read = true;
            }
        }
    }

    /// Rebuild the plan from the current image and visible range.
    pub(super) fn replan(&mut self) {
        let n = self.items.len();
        let mut jobs: Vec<PlanJob> = Vec::new();
        if let Some(c) = self.current.filter(|&c| c < n) {
            jobs.push(PlanJob {
                kind: Kind::Full,
                index: c,
            });
            // Staged: neighbors at screen resolution only.
            let ahead = if self.staged() {
                Kind::Screen
            } else {
                Kind::Full
            };
            for d in 1..=self.cfg.read_ahead.min(n) {
                let (next, prev) = if self.cfg.wrap {
                    (Some((c + d) % n), Some((c + n - d) % n))
                } else {
                    ((c + d < n).then_some(c + d), c.checked_sub(d))
                };
                for index in [next, prev].into_iter().flatten() {
                    if !jobs.iter().any(|j| j.index == index) {
                        jobs.push(PlanJob { kind: ahead, index });
                    }
                }
            }
        }
        if let Some((start, end)) = self.thumb_range() {
            let thumb = |index| PlanJob {
                kind: Kind::Thumb,
                index,
            };
            let anchor = self.current.map_or(start, |c| c.clamp(start, end - 1));
            jobs.push(thumb(anchor));
            for d in 1.. {
                let after = (anchor + d < end).then_some(anchor + d);
                let before = (anchor >= start + d).then(|| anchor - d);
                if after.is_none() && before.is_none() {
                    break;
                }
                jobs.extend([after, before].into_iter().flatten().map(thumb));
            }
            let limit = if self.cfg.background {
                n
            } else {
                self.cfg.margin
            };
            for d in 1..=limit {
                let after = (end - 1 + d < n).then_some(end - 1 + d);
                let before = start.checked_sub(d);
                if after.is_none() && before.is_none() {
                    break;
                }
                jobs.extend([after, before].into_iter().flatten().map(thumb));
            }
        }
        self.plan.gen += 1;
        self.plan.taken = vec![false; jobs.len()];
        self.plan.end = jobs.len();
        self.plan.cursor = 0;
        self.plan.jobs = jobs;
        self.replanned = true;
    }

    /// Cancel running jobs the new plan no longer wants.
    pub(super) fn cancel_stale(&mut self) {
        let staged = self.staged();
        let stale_full: Vec<PathBuf> = self
            .running_full
            .iter()
            .filter(|(_, r)| !r.cancel.is_cancelled())
            .filter(|(p, r)| match self.index_of.get(p.as_path()) {
                None => true,
                // Staged: full resolution is for the current image only.
                Some(&i) if staged => Some(i) != self.current,
                Some(&i) => !self.in_window(i) || (r.downloading && Some(i) != self.current),
            })
            .map(|(p, _)| p.clone())
            .collect();
        let stale_screen: Vec<PathBuf> = self
            .running_screen
            .iter()
            .filter(|(_, r)| !r.cancel.is_cancelled())
            .filter(|(p, _)| match self.index_of.get(p.as_path()) {
                None => true,
                Some(&i) => !self.in_window(i),
            })
            .map(|(p, _)| p.clone())
            .collect();
        let stale_thumb: Vec<PathBuf> = self
            .running_thumb
            .iter()
            .filter(|(_, r)| !r.cancel.is_cancelled())
            .filter(|(p, _)| match self.index_of.get(p.as_path()) {
                None => true,
                Some(&i) => !self.thumb_wanted(i),
            })
            .map(|(p, _)| p.clone())
            .collect();
        for path in stale_full {
            self.cancel_job(Kind::Full, &path);
        }
        for path in stale_screen {
            self.cancel_job(Kind::Screen, &path);
        }
        for path in stale_thumb {
            self.cancel_job(Kind::Thumb, &path);
        }
    }

    /// Cancel one running job. A job inside a file system call cannot be
    /// interrupted, so its thread is written off (a replacement is spawned
    /// by the caller) and the job is forgotten: its result will be dropped.
    pub(super) fn cancel_job(&mut self, kind: Kind, path: &Path) {
        let Some(running) = self.running(kind).get(path) else {
            return;
        };
        running.cancel.cancel();
        let tid = running.thread;
        let in_io = self
            .threads
            .get(&tid)
            .is_some_and(|t| t.io && t.role != Role::WrittenOff);
        if in_io {
            if let Some(running) = self.running_mut(kind).remove(path) {
                self.write_off(tid);
                self.requeue(&running);
            }
        }
    }

    /// Cancel the running folder listing, writing off its thread.
    pub(super) fn cancel_listing(&mut self) {
        if let Some(list) = self.list_running.take() {
            list.cancel.cancel();
            if self.threads.get(&list.thread).is_some_and(|t| t.io) {
                self.write_off(list.thread);
            }
        }
    }

    /// Make a job's plan entry available again.
    fn requeue(&mut self, running: &Running) {
        if running.plan_gen != self.plan.gen {
            return;
        }
        if let Some(pos) = running.plan_pos.filter(|&p| p < self.plan.taken.len()) {
            self.plan.taken[pos] = false;
            self.plan.cursor = self.plan.cursor.min(pos);
        }
    }

    /// Free workers for the current image and its read-ahead: cancel the
    /// least useful running thumbnails, and when the current image still
    /// has no worker, the least useful running read-ahead decode.
    pub(super) fn preempt(&mut self) {
        let Some(current) = self.current else { return };
        // Jobs on the reserved thread hold no pool worker.
        let reserved = self.reserved_thread();
        let active = |r: &Running| !r.cancel.is_cancelled() && Some(r.thread) != reserved;
        let busy = self.running_full.values().filter(|r| active(r)).count()
            + self.running_screen.values().filter(|r| active(r)).count()
            + self.running_thumb.values().filter(|r| active(r)).count()
            + usize::from(self.list_running.is_some());
        let (pool, _) = self.thread_counts();
        let mut free = pool.saturating_sub(busy);
        let full_positions: Vec<usize> = (0..self.plan.jobs.len())
            .filter(|&p| self.plan.jobs[p].kind != Kind::Thumb)
            .collect();
        let pending = full_positions
            .iter()
            .filter(|&&p| matches!(self.classify(p), Class::Go))
            .count();
        if pending > free {
            let ranker = self.thumb_ranker();
            let mut thumbs: Vec<_> = self
                .running_thumb
                .iter()
                .filter(|(_, r)| active(r))
                .filter_map(|(p, _)| {
                    let index = *self.index_of.get(p.as_path())?;
                    Some((ranker.rank(index), p.clone()))
                })
                .collect();
            thumbs.sort_by(|a, b| b.cmp(a));
            for (_, path) in thumbs.into_iter().take(pending - free) {
                self.cancel_job(Kind::Thumb, &path);
                free += 1;
            }
        }
        let current_waits = full_positions
            .first()
            .is_some_and(|&p| matches!(self.classify(p), Class::Go));
        if !current_waits {
            return;
        }
        // The reserved thread takes the current image when it is idle. When
        // it still works on an earlier selection, it leaves its role and
        // the caller starts a fresh reserved thread at once. This does not
        // look at `free`: a cancelled job that has not stopped yet still
        // holds its pool worker.
        if let Some(tid) = reserved {
            let occupied = [
                &self.running_full,
                &self.running_screen,
                &self.running_thumb,
            ]
            .iter()
            .any(|map| map.values().any(|r| r.thread == tid));
            if !occupied {
                return;
            }
            let (_, off) = self.thread_counts();
            if off < self.cfg.max_written_off {
                self.write_off(tid);
                return;
            }
        }
        if free == 0 {
            let ranker = self.full_ranker();
            let victim = self
                .running_full
                .iter()
                .map(|(p, r)| (Kind::Full, p, r))
                .chain(
                    self.running_screen
                        .iter()
                        .map(|(p, r)| (Kind::Screen, p, r)),
                )
                .filter(|(_, _, r)| active(r))
                .filter_map(|(kind, p, _)| {
                    let index = *self.index_of.get(p.as_path())?;
                    (index != current).then(|| (ranker.rank(index), kind == Kind::Full, p.clone()))
                })
                .max_by(|a, b| a.0.cmp(&b.0));
            if let Some((_, full, path)) = victim {
                let kind = if full { Kind::Full } else { Kind::Screen };
                self.cancel_job(kind, &path);
            }
        }
    }

    fn classify(&self, pos: usize) -> Class {
        let job = self.plan.jobs[pos];
        let Some(path) = self.items.get(job.index) else {
            return Class::Done;
        };
        if self.user_cancelled.contains(path) {
            return Class::Done;
        }
        match job.kind {
            Kind::Full => {
                if self.full.contains(path) || self.full_errors.contains_key(path) {
                    return Class::Done;
                }
                if self.running_full.contains_key(path) {
                    return Class::Running;
                }
                if Some(job.index) != self.current {
                    if self.non_local.contains(path) {
                        return Class::Done;
                    }
                    if let Some(&(bytes, _)) = self.full_sizes.get(path) {
                        let ranker = self.full_ranker();
                        let rank = ranker.rank(job.index);
                        if !self.full.would_fit(bytes, rank, false, |i| ranker.rank(i)) {
                            return Class::Done;
                        }
                    }
                }
                Class::Go
            }
            Kind::Screen => {
                if self.screen.contains(path)
                    || self.full.contains(path)
                    || self.full_errors.contains_key(path)
                    || self.screen_failed.contains(path)
                    || self.non_local.contains(path)
                {
                    return Class::Done;
                }
                if self.running_screen.contains_key(path) {
                    return Class::Running;
                }
                if self
                    .running_full
                    .get(path)
                    .is_some_and(|r| !r.cancel.is_cancelled())
                {
                    return Class::Wait;
                }
                if let Some((w, h)) = self.cfg.screen {
                    let ranker = self.full_ranker();
                    let rank = ranker.rank(job.index);
                    let estimate = (w as usize).saturating_mul(h as usize).saturating_mul(4);
                    if !self
                        .screen
                        .would_fit(estimate, rank, false, |i| ranker.rank(i))
                    {
                        return Class::Done;
                    }
                }
                Class::Go
            }
            Kind::Thumb => {
                // A stale thumbnail is made again; it shows meanwhile.
                if self.thumb_fresh(path) || self.thumb_errors.contains_key(path) {
                    return Class::Done;
                }
                if self.running_thumb.contains_key(path) {
                    return Class::Running;
                }
                let cached = self.full.contains(path)
                    || self.screen_source(path).is_some()
                    || self.stale_source(path).is_some();
                if !cached {
                    let decoding = |map: &HashMap<PathBuf, Running>| {
                        map.get(path).is_some_and(|r| !r.cancel.is_cancelled())
                    };
                    if decoding(&self.running_full) || decoding(&self.running_screen) {
                        return Class::Wait;
                    }
                    if self.non_local.contains(path) {
                        return Class::Done;
                    }
                }
                let ranker = self.thumb_ranker();
                let rank = ranker.rank(job.index);
                let side = self.cfg.thumb_size as usize;
                // The new thumbnail replaces the stale one of the file.
                let stale = self.thumbs.get(path).map_or(0, |e| e.bytes);
                let estimate = side
                    .saturating_mul(side)
                    .saturating_mul(THUMB_BYTES_PER_PIXEL)
                    .saturating_sub(stale);
                if !self
                    .thumbs
                    .would_fit(estimate, rank, rank.0 == 0, |i| ranker.rank(i))
                {
                    return Class::Saturated;
                }
                Class::Go
            }
        }
    }

    /// The first plan position below `limit` that can start now. Marks
    /// finished and running entries on the way so later scans skip them.
    pub(super) fn scan(&mut self, limit: usize) -> Option<usize> {
        let stop = self.plan.end.min(limit);
        let mut pos = self.plan.cursor;
        while pos < stop {
            if !self.plan.taken[pos] {
                match self.classify(pos) {
                    Class::Go => return Some(pos),
                    Class::Wait => {}
                    Class::Done => self.plan.taken[pos] = true,
                    Class::Running => {
                        self.plan.taken[pos] = true;
                        self.adopt(pos);
                    }
                    Class::Saturated => {
                        self.plan.end = pos;
                        break;
                    }
                }
            }
            if pos == self.plan.cursor && self.plan.taken[pos] {
                self.plan.cursor += 1;
            }
            pos += 1;
        }
        None
    }

    /// Tie a running job to its entry in the current plan, so a cancelled
    /// job can be requeued at the right place.
    fn adopt(&mut self, pos: usize) {
        let job = self.plan.jobs[pos];
        let Some(path) = self.items.get(job.index) else {
            return;
        };
        let path = path.clone();
        let gen = self.plan.gen;
        if let Some(running) = self.running_mut(job.kind).get_mut(&path) {
            running.plan_gen = gen;
            running.plan_pos = Some(pos);
        }
    }

    /// The next task for thread `tid`: the current image first, then a
    /// pending folder listing, then the rest of the plan.
    pub fn take_task(&mut self, tid: u64) -> Option<Task> {
        if self.shutdown {
            return None;
        }
        let first = usize::from(self.current.is_some());
        if let Some(pos) = self.scan(first) {
            return Some(self.start(pos, tid));
        }
        // The reserved thread takes the current image and nothing else.
        if self.is_reserved(tid) {
            return None;
        }
        if let Some((gen, target)) = self.list_pending.take() {
            let id = self.new_id();
            let cancel = CancelToken::new();
            self.list_running = Some(ListRunning {
                id,
                thread: tid,
                cancel: cancel.clone(),
            });
            self.set_io(tid, true);
            return Some(Task {
                id,
                cancel,
                work: Work::List { gen, target },
            });
        }
        // The current image has the machine to itself while it decodes.
        if self.current_decoding() {
            return None;
        }
        let pos = self.scan(usize::MAX)?;
        Some(self.start(pos, tid))
    }

    /// True when another task could start now. Used to wake one sleeping
    /// peer after a worker took its task.
    pub fn has_work(&mut self) -> bool {
        if self.shutdown {
            return false;
        }
        if self.list_pending.is_some() {
            return true;
        }
        let first = usize::from(self.current.is_some());
        if self.scan(first).is_some() {
            return true;
        }
        !self.current_decoding() && self.scan(usize::MAX).is_some()
    }

    fn start(&mut self, pos: usize, tid: u64) -> Task {
        let job = self.plan.jobs[pos];
        self.plan.taken[pos] = true;
        while self.plan.cursor < self.plan.taken.len() && self.plan.taken[self.plan.cursor] {
            self.plan.cursor += 1;
        }
        let path = self.items[job.index].clone();
        let id = self.new_id();
        let cancel = CancelToken::new();
        let side = if job.kind == Kind::Thumb {
            self.cfg.thumb_size
        } else {
            0
        };
        let running = Running {
            id,
            thread: tid,
            cancel: cancel.clone(),
            plan_gen: self.plan.gen,
            plan_pos: Some(pos),
            side,
            downloading: false,
            local_read: false,
            started: Instant::now(),
            discard: false,
        };
        let work = match job.kind {
            Kind::Full => {
                let explicit = Some(job.index) == self.current;
                // The quick first stage, when this is the current image and
                // no screen image of it is cached yet.
                let quick = if self.staged() && explicit && !self.screen.contains(&path) {
                    self.cfg.quick_target()
                } else {
                    None
                };
                self.running_full.insert(path.clone(), running);
                self.set_io(tid, true);
                Work::Full {
                    path,
                    explicit,
                    quick,
                }
            }
            Kind::Screen => {
                let target = self.cfg.screen.unwrap_or((1, 1));
                self.running_screen.insert(path.clone(), running);
                self.set_io(tid, true);
                Work::Screen { path, target }
            }
            Kind::Thumb => {
                // Cached pixels first: the full decode, then a screen image
                // or a stale thumbnail that is large enough.
                let still = |image: &ImageBuf| Arc::new(Content::Still(image.clone()));
                let from = self
                    .full
                    .get(&path)
                    .map(|e| (Arc::clone(&e.value), e.stamp))
                    .or_else(|| {
                        self.screen_source(&path)
                            .map(|e| (still(&e.value.image), e.stamp))
                    })
                    .or_else(|| {
                        let stamp = self.thumbs.get(&path)?.stamp;
                        self.stale_source(&path).map(|image| (still(image), stamp))
                    });
                self.set_io(tid, from.is_none());
                self.running_thumb.insert(path.clone(), running);
                Work::Thumb { path, from, side }
            }
        };
        Task { id, cancel, work }
    }

    fn is_mine(&self, task: &Task) -> bool {
        match &task.work {
            Work::Full { path, .. } => self.running_full.get(path).is_some_and(|r| r.id == task.id),
            Work::Screen { path, .. } => self
                .running_screen
                .get(path)
                .is_some_and(|r| r.id == task.id),
            Work::Thumb { path, .. } => self
                .running_thumb
                .get(path)
                .is_some_and(|r| r.id == task.id),
            Work::List { .. } => self.list_running.as_ref().is_some_and(|l| l.id == task.id),
        }
    }

    /// Called after the file system part of a task. Returns false when the
    /// task was abandoned meanwhile (its result would be dropped anyway).
    pub fn checkpoint(&mut self, tid: u64, task: &Task) -> bool {
        self.set_io(tid, false);
        !self.shutdown && self.is_mine(task)
    }

    /// The current image is not local and its read starts now. Returns
    /// true when the event queue went from empty to non-empty.
    pub fn mark_downloading(&mut self, task: &Task) -> bool {
        let Work::Full { path, .. } = &task.work else {
            return false;
        };
        if self.shutdown {
            return false;
        }
        let was_empty = self.events.is_empty();
        let Some(running) = self.running_full.get_mut(path).filter(|r| r.id == task.id) else {
            return false;
        };
        running.downloading = true;
        self.non_local.insert(path.clone());
        if let Some(&index) = self.index_of.get(path) {
            self.push(Event::Downloading {
                index,
                path: path.clone(),
            });
        }
        was_empty && !self.events.is_empty()
    }

    /// Store a finished task's result. Returns true when the event queue
    /// went from empty to non-empty, so the caller wakes the interface.
    pub fn finish(&mut self, tid: u64, task: Task, outcome: Outcome) -> bool {
        self.set_io(tid, false);
        let was_empty = self.events.is_empty();
        let mine = self.is_mine(&task);
        match task.work {
            Work::List { gen, .. } => {
                if mine {
                    self.list_running = None;
                    if gen == self.list_gen && !self.shutdown {
                        if let Outcome::Listed(Some(list)) = outcome {
                            self.merge_listing(list);
                        }
                    }
                }
            }
            Work::Full { path, .. } if mine => self.settle(Kind::Full, path, outcome),
            Work::Screen { path, .. } if mine => self.settle(Kind::Screen, path, outcome),
            Work::Thumb { path, .. } if mine => self.settle(Kind::Thumb, path, outcome),
            // Abandoned: the result is dropped.
            Work::Full { .. } | Work::Screen { .. } | Work::Thumb { .. } => {}
        }
        was_empty && !self.events.is_empty()
    }

    /// The quick first stage of the current image's full job arrived.
    /// Kept as its screen image unless the job was abandoned or the full
    /// image is already cached. Returns true when the event queue went
    /// from empty to non-empty.
    pub fn deliver_quick(
        &mut self,
        path: &Path,
        id: u64,
        reduced: Reduced,
        stamp: Option<Stamp>,
        took: std::time::Duration,
    ) -> bool {
        let mine = self.running_full.get(path).is_some_and(|r| r.id == id);
        if self.shutdown || !mine || self.full.contains(path) {
            return false;
        }
        self.note_reduced_time(path, took);
        let Some(index) = self.index_of.get(path).copied() else {
            return false;
        };
        let was_empty = self.events.is_empty();
        if self.insert_screen(path.to_path_buf(), index, reduced, stamp) {
            self.push(Event::Reduced {
                index,
                path: path.to_path_buf(),
            });
        }
        was_empty && !self.events.is_empty()
    }

    /// Cache a screen image at `index`; the current image's is protected.
    fn insert_screen(
        &mut self,
        path: PathBuf,
        index: usize,
        reduced: Reduced,
        stamp: Option<Stamp>,
    ) -> bool {
        let ranker = self.full_ranker();
        let entry = Entry {
            bytes: reduced.image.byte_size(),
            value: reduced,
            index,
            stamp,
        };
        let protected = Some(index) == self.current;
        let rank = ranker.rank(index);
        self.screen
            .insert(path, entry, rank, protected, |i| ranker.rank(i))
    }

    fn settle(&mut self, kind: Kind, path: PathBuf, outcome: Outcome) {
        let Some(running) = self.running_mut(kind).remove(&path) else {
            return;
        };
        if self.shutdown {
            return;
        }
        if running.discard {
            self.requeue(&running);
            return;
        }
        let cancelled = running.cancel.is_cancelled();
        let Some(index) = self.index_of.get(&path).copied() else {
            return;
        };
        match outcome {
            Outcome::Full(Ok(content), stamp) => {
                let bytes = content.byte_size();
                self.full_sizes.insert(path.clone(), (bytes, stamp));
                let ranker = self.full_ranker();
                let entry = Entry {
                    value: Arc::new(content),
                    bytes,
                    index,
                    stamp,
                };
                let protected = Some(index) == self.current;
                let rank = ranker.rank(index);
                if self
                    .full
                    .insert(path.clone(), entry, rank, protected, |i| ranker.rank(i))
                {
                    self.push(Event::Ready { index, path });
                }
            }
            Outcome::Screen(Ok(Staged::Reduced(reduced)), stamp) => {
                if !self.full.contains(&path)
                    && self.insert_screen(path.clone(), index, reduced, stamp)
                {
                    self.push(Event::Reduced { index, path });
                }
            }
            Outcome::Screen(Ok(Staged::Full(content)), stamp) => {
                // No larger than the screen: this is the full image.
                let bytes = content.byte_size();
                self.full_sizes.insert(path.clone(), (bytes, stamp));
                let ranker = self.full_ranker();
                let entry = Entry {
                    value: Arc::new(content),
                    bytes,
                    index,
                    stamp,
                };
                let protected = Some(index) == self.current;
                let rank = ranker.rank(index);
                if self
                    .full
                    .insert(path.clone(), entry, rank, protected, |i| ranker.rank(i))
                {
                    self.push(Event::Ready { index, path });
                }
            }
            Outcome::Screen(Err(_), _) if cancelled => self.requeue(&running),
            Outcome::Screen(Err(_), _) => {
                // The full decode runs when the image becomes current and
                // reports the error then.
                self.screen_failed.insert(path);
            }
            Outcome::Thumb(Ok(image), stamp) => {
                let ranker = self.thumb_ranker();
                let rank = ranker.rank(index);
                let side = running.side;
                let stale = side != self.cfg.thumb_size;
                if stale {
                    // Made for the size before the last change: it shows
                    // only where nothing else does, and the job runs again.
                    self.requeue(&running);
                    if self.thumbs.contains(&path) {
                        return;
                    }
                }
                let entry = Entry {
                    bytes: image.byte_size(),
                    value: Thumb { image, side },
                    index,
                    stamp,
                };
                if self
                    .thumbs
                    .insert(path.clone(), entry, rank, rank.0 == 0, |i| ranker.rank(i))
                {
                    self.push(Event::ThumbnailReady { index, path });
                } else if !stale && running.plan_gen == self.plan.gen {
                    // Everything after this entry ranks no better: stop.
                    if let Some(pos) = running.plan_pos {
                        self.plan.end = self.plan.end.min(pos);
                    }
                }
            }
            Outcome::Full(Err(_), _) | Outcome::Thumb(Err(_), _) if cancelled => {
                self.requeue(&running);
            }
            Outcome::Full(Err(error), stamp) => {
                let error = Arc::new(error);
                self.full_errors.insert(
                    path.clone(),
                    ErrorEntry {
                        error: Arc::clone(&error),
                        stamp,
                    },
                );
                self.push(Event::Failed { index, path, error });
            }
            Outcome::Thumb(Err(error), stamp) => {
                let error = Arc::new(error);
                self.thumb_errors.insert(
                    path.clone(),
                    ErrorEntry {
                        error: Arc::clone(&error),
                        stamp,
                    },
                );
                self.push(Event::ThumbnailFailed { index, path, error });
            }
            Outcome::NotLocal => {
                if self.non_local.insert(path.clone()) {
                    self.push(Event::NotLocal { index, path });
                }
                // When the file became the current image meanwhile, its
                // entry must run again as an explicit read.
                self.requeue(&running);
            }
            Outcome::Directory => {}
            Outcome::Abandoned | Outcome::Listed(_) => self.requeue(&running),
        }
    }
}

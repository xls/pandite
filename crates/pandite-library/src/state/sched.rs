//! The priority plan, job selection, cancellation and job completion.
//!
//! The plan is an ordered list of wanted jobs, rebuilt whenever the
//! current image, the visible gallery range or the folder list changes:
//! the current image, then read-ahead neighbors (next, previous, next+1,
//! previous+1, ...), then thumbnails for the visible range from the
//! current image outward, then the margin, then the rest. Workers always
//! take the first job that can start, so a newly current image is next in
//! line the moment the plan is rebuilt.

use super::{ErrorEntry, Role};
use super::{Kind, ListRunning, Outcome, PlanJob, Running, State, Task, Work};
use crate::cache::Entry;
use crate::Event;
use pandite_core::CancelToken;
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
    /// Rebuild the plan from the current image and visible range.
    pub(super) fn replan(&mut self) {
        let n = self.items.len();
        let mut jobs: Vec<PlanJob> = Vec::new();
        if let Some(c) = self.current.filter(|&c| c < n) {
            jobs.push(PlanJob {
                kind: Kind::Full,
                index: c,
            });
            for d in 1..=self.cfg.read_ahead.min(n) {
                let (next, prev) = if self.cfg.wrap {
                    (Some((c + d) % n), Some((c + n - d) % n))
                } else {
                    ((c + d < n).then_some(c + d), c.checked_sub(d))
                };
                for index in [next, prev].into_iter().flatten() {
                    if !jobs.iter().any(|j| j.index == index) {
                        jobs.push(PlanJob {
                            kind: Kind::Full,
                            index,
                        });
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
        let stale_full: Vec<PathBuf> = self
            .running_full
            .iter()
            .filter(|(_, r)| !r.cancel.is_cancelled())
            .filter(|(p, r)| match self.index_of.get(p.as_path()) {
                None => true,
                Some(&i) => !self.in_window(i) || (r.downloading && Some(i) != self.current),
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
        for path in stale_thumb {
            self.cancel_job(Kind::Thumb, &path);
        }
    }

    /// Cancel one running job. A job inside a file system call cannot be
    /// interrupted, so its thread is written off (a replacement is spawned
    /// by the caller) and the job is forgotten: its result will be dropped.
    pub(super) fn cancel_job(&mut self, kind: Kind, path: &Path) {
        let map = match kind {
            Kind::Full => &mut self.running_full,
            Kind::Thumb => &mut self.running_thumb,
        };
        let Some(running) = map.get(path) else { return };
        running.cancel.cancel();
        let tid = running.thread;
        let in_io = self
            .threads
            .get(&tid)
            .is_some_and(|t| t.io && t.role == Role::Pool);
        if in_io {
            if let Some(running) = map.remove(path) {
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
        let active = |r: &Running| !r.cancel.is_cancelled();
        let busy = self.running_full.values().filter(|r| active(r)).count()
            + self.running_thumb.values().filter(|r| active(r)).count()
            + usize::from(self.list_running.is_some());
        let (pool, _) = self.thread_counts();
        let mut free = pool.saturating_sub(busy);
        let full_positions: Vec<usize> = (0..self.plan.jobs.len())
            .filter(|&p| self.plan.jobs[p].kind == Kind::Full)
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
        if current_waits && free == 0 {
            let ranker = self.full_ranker();
            let victim = self
                .running_full
                .iter()
                .filter(|(_, r)| active(r))
                .filter_map(|(p, _)| {
                    let index = *self.index_of.get(p.as_path())?;
                    (index != current).then(|| (ranker.rank(index), p.clone()))
                })
                .max();
            if let Some((_, path)) = victim {
                self.cancel_job(Kind::Full, &path);
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
            Kind::Thumb => {
                if self.thumbs.contains(path) || self.thumb_errors.contains_key(path) {
                    return Class::Done;
                }
                if self.running_thumb.contains_key(path) {
                    return Class::Running;
                }
                if !self.full.contains(path) {
                    if self
                        .running_full
                        .get(path)
                        .is_some_and(|r| !r.cancel.is_cancelled())
                    {
                        return Class::Wait;
                    }
                    if self.non_local.contains(path) {
                        return Class::Done;
                    }
                }
                let ranker = self.thumb_ranker();
                let rank = ranker.rank(job.index);
                let side = self.cfg.thumb_size as usize;
                let estimate = side
                    .saturating_mul(side)
                    .saturating_mul(THUMB_BYTES_PER_PIXEL);
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
        let map = match job.kind {
            Kind::Full => &mut self.running_full,
            Kind::Thumb => &mut self.running_thumb,
        };
        if let Some(running) = map.get_mut(path) {
            running.plan_gen = self.plan.gen;
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
        let pos = self.scan(usize::MAX)?;
        Some(self.start(pos, tid))
    }

    /// True when another task could start now. Used to wake one sleeping
    /// peer after a worker took its task.
    pub fn has_work(&mut self) -> bool {
        !self.shutdown && (self.list_pending.is_some() || self.scan(usize::MAX).is_some())
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
        let running = Running {
            id,
            thread: tid,
            cancel: cancel.clone(),
            plan_gen: self.plan.gen,
            plan_pos: Some(pos),
            downloading: false,
            discard: false,
        };
        let work = match job.kind {
            Kind::Full => {
                self.running_full.insert(path.clone(), running);
                self.set_io(tid, true);
                Work::Full {
                    path,
                    explicit: Some(job.index) == self.current,
                }
            }
            Kind::Thumb => {
                let from = self
                    .full
                    .get(&path)
                    .map(|e| (Arc::clone(&e.value), e.stamp));
                self.set_io(tid, from.is_none());
                self.running_thumb.insert(path.clone(), running);
                Work::Thumb { path, from }
            }
        };
        Task { id, cancel, work }
    }

    fn is_mine(&self, task: &Task) -> bool {
        match &task.work {
            Work::Full { path, .. } => self.running_full.get(path).is_some_and(|r| r.id == task.id),
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
            Work::Thumb { path, .. } if mine => self.settle(Kind::Thumb, path, outcome),
            // Abandoned: the result is dropped.
            Work::Full { .. } | Work::Thumb { .. } => {}
        }
        was_empty && !self.events.is_empty()
    }

    fn settle(&mut self, kind: Kind, path: PathBuf, outcome: Outcome) {
        let map = match kind {
            Kind::Full => &mut self.running_full,
            Kind::Thumb => &mut self.running_thumb,
        };
        let Some(running) = map.remove(&path) else {
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
            Outcome::Thumb(Ok(image), stamp) => {
                let ranker = self.thumb_ranker();
                let rank = ranker.rank(index);
                let entry = Entry {
                    bytes: image.byte_size(),
                    value: image,
                    index,
                    stamp,
                };
                if self
                    .thumbs
                    .insert(path.clone(), entry, rank, rank.0 == 0, |i| ranker.rank(i))
                {
                    self.push(Event::ThumbnailReady { index, path });
                } else if running.plan_gen == self.plan.gen {
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

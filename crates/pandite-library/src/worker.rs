//! Worker threads: take a task under the lock, run it unlocked, report back.

use crate::folder::FolderList;
use crate::fs::Stamp;
use crate::library::Shared;
use crate::state::{Outcome, State, Task, Work};
use pandite_core::{Content, DecodeOptions, Error, ImageBuf, Result, Source};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::thread;

/// Run `f`, turning a panic into an error so one bad file cannot kill a
/// worker (with `panic = "abort"` the process stops instead).
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(f))
        .unwrap_or_else(|_| Err(Error::Backend("a provider panicked".into())))
}

/// Start worker threads until the pool is full or the written-off cap is
/// reached. Called with the lock held; a new thread blocks on the lock
/// until the caller releases it.
pub(crate) fn replenish(shared: &Arc<Shared>, st: &mut State) -> io::Result<()> {
    // Threads that marked themselves exited are detached, not joined: a
    // join under the lock could wait on thread teardown.
    drop(st.reap());
    while st.wants_thread() {
        let tid = st.reserve_thread();
        let worker_shared = Arc::clone(shared);
        let spawned = thread::Builder::new()
            .name(format!("pandite-library-{tid}"))
            .spawn(move || worker_main(worker_shared, tid));
        match spawned {
            Ok(handle) => st.attach_thread(tid, handle),
            Err(err) => {
                st.release_thread(tid);
                return Err(err);
            }
        }
    }
    Ok(())
}

fn worker_main(shared: Arc<Shared>, tid: u64) {
    let mut st = shared.lock();
    'run: loop {
        let task = loop {
            if st.shutdown {
                break 'run;
            }
            if let Some(task) = st.take_task(tid) {
                // Chain wake-up: one finished job can unblock several, so
                // a worker that found work wakes one sleeping peer when
                // more is waiting. No idle worker sleeps past open work.
                if st.waiting > 0 && st.has_work() {
                    shared.notify_one_work();
                }
                break task;
            }
            shared.idle_cv.notify_all();
            st.waiting += 1;
            st = shared.wait_work(st);
            st.waiting -= 1;
        };
        drop(st);
        let outcome = run_task(&shared, tid, &task);
        st = shared.lock();
        let wake = st.finish(tid, task, outcome);
        let keep = st.after_task(tid);
        if std::mem::take(&mut st.replanned) {
            // A finished listing rebuilt the plan and may have cancelled
            // jobs: refill the pool and wake idle peers.
            let _ = replenish(&shared, &mut st);
            shared.notify_work();
        }
        shared.idle_cv.notify_all();
        if wake {
            drop(st);
            (shared.wake)();
            st = shared.lock();
        }
        if !keep {
            break;
        }
    }
    st.exit_thread(tid);
    drop(st);
    shared.exit_cv.notify_all();
    shared.idle_cv.notify_all();
}

/// Why a read stopped before producing bytes.
enum Early {
    Failed(Error, Option<Stamp>),
    NotLocal,
    Directory,
    Abandoned,
}

fn run_task(shared: &Shared, tid: u64, task: &Task) -> Outcome {
    match &task.work {
        Work::List { target, .. } => {
            let listed = guard(|| {
                Ok(FolderList::scan_with(
                    target,
                    &shared.registry,
                    &*shared.files,
                ))
            });
            Outcome::Listed(listed.ok())
        }
        Work::Full { path, explicit } => match read(shared, tid, task, path, *explicit) {
            Ok((source, stamp)) => {
                let options = DecodeOptions {
                    cancel: task.cancel.clone(),
                    ..DecodeOptions::default()
                };
                let content = guard(|| shared.registry.decode(&source, &options));
                Outcome::Full(content, stamp)
            }
            Err(early) => early_outcome(early, Outcome::Full),
        },
        Work::Thumb {
            from: Some((content, stamp)),
            ..
        } => Outcome::Thumb(thumbnail_of(shared, task, content), *stamp),
        Work::Thumb { path, from: None } => match read(shared, tid, task, path, false) {
            Ok((source, stamp)) => {
                let side = shared.thumb_size;
                let options = DecodeOptions {
                    target_size: Some((side, side)),
                    first_frame_only: true,
                    cancel: task.cancel.clone(),
                };
                let image = guard(|| shared.registry.decode(&source, &options))
                    .and_then(|content| thumbnail_of(shared, task, &content));
                Outcome::Thumb(image, stamp)
            }
            Err(early) => early_outcome(early, Outcome::Thumb),
        },
    }
}

fn early_outcome<T>(early: Early, wrap: fn(Result<T>, Option<Stamp>) -> Outcome) -> Outcome {
    match early {
        Early::Failed(err, stamp) => wrap(Err(err), stamp),
        Early::NotLocal => Outcome::NotLocal,
        Early::Directory => Outcome::Directory,
        Early::Abandoned => Outcome::Abandoned,
    }
}

fn thumbnail_of(shared: &Shared, task: &Task, content: &Content) -> Result<ImageBuf> {
    task.cancel.check()?;
    let side = shared.thumb_size;
    guard(|| Ok((shared.downscale)(content.first_image(), side)))
}

/// The file system part of a task: look at the file, skip non-local files
/// unless `explicit`, then read it in cancellable chunks. The thread is
/// marked as inside a file system call until the checkpoint.
fn read(
    shared: &Shared,
    tid: u64,
    task: &Task,
    path: &Path,
    explicit: bool,
) -> std::result::Result<(Source, Option<Stamp>), Early> {
    // Leave the file system part; an abandoned task reports nothing else.
    let leave = |early: Early| {
        if shared.lock().checkpoint(tid, task) {
            early
        } else {
            Early::Abandoned
        }
    };
    let info = match guard(|| shared.files.file_info(path).map_err(Error::Io)) {
        Ok(info) => info,
        Err(err) => return Err(leave(Early::Failed(err, None))),
    };
    let stamp = Some(info.stamp);
    if info.is_dir {
        return Err(leave(Early::Directory));
    }
    if !info.local {
        if !explicit {
            return Err(leave(Early::NotLocal));
        }
        let wake = shared.lock().mark_downloading(task);
        if wake {
            (shared.wake)();
        }
    }
    match guard(|| shared.files.read(path, &task.cancel)) {
        Ok(source) => {
            if shared.lock().checkpoint(tid, task) {
                Ok((source, stamp))
            } else {
                Err(Early::Abandoned)
            }
        }
        Err(err) => Err(leave(Early::Failed(err, stamp))),
    }
}

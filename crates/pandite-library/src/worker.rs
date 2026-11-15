//! Worker threads: take a task under the lock, run it unlocked, report back.

use crate::folder::FolderList;
use crate::fs::Stamp;
use crate::library::{LoadTimings, Reduced, Shared};
use crate::state::{Listing, Outcome, Staged, State, Task, Work};
use pandite_core::{
    CancelToken, Content, DecodeOptions, Error, ImageBuf, ImageMeta, Result, Source,
};
use std::io;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Instant;

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
            // The reserved thread refuses background work: when a wake-up
            // meant for that work reached it, pass the wake-up on.
            if st.is_reserved(tid) && st.waiting > 0 && st.has_work() {
                shared.notify_one_work();
            }
            shared.idle_cv.notify_all();
            st.waiting += 1;
            // Held back by a current image whose file lookup has not
            // answered yet: look again when its grace ends.
            st = match st.gate_ends() {
                Some(left) => shared.wait_work_for(st, left),
                None => shared.wait_work(st),
            };
            st.waiting -= 1;
        };
        drop(st);
        let mut timings = LoadTimings::default();
        let outcome = run_task(&shared, tid, &task, &mut timings);
        st = shared.lock();
        if let Work::Full { path, .. } = &task.work {
            if timings.decode_full.is_some() {
                st.note_timings(path, timings);
            }
        }
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

fn run_task(shared: &Arc<Shared>, tid: u64, task: &Task, timings: &mut LoadTimings) -> Outcome {
    match &task.work {
        Work::List {
            listing: Listing::Scan(target),
            ..
        } => {
            let listed = guard(|| {
                Ok(FolderList::scan_with(
                    target,
                    &shared.registry,
                    &*shared.files,
                ))
            });
            Outcome::Listed(listed.ok())
        }
        Work::List {
            listing: Listing::Check { dir, keep },
            ..
        } => {
            let listed = guard(|| {
                Ok(FolderList::relist_with(
                    dir,
                    keep,
                    &shared.registry,
                    &*shared.files,
                ))
            });
            Outcome::Listed(listed.ok().flatten())
        }
        Work::Full {
            path,
            explicit,
            quick,
        } => match read(shared, tid, task, path, *explicit, timings) {
            Ok((source, stamp)) => {
                if let Some(target) = *quick {
                    start_quick(shared, task, &source, target, stamp);
                }
                let options = DecodeOptions {
                    prefer_8bit: true,
                    cancel: task.cancel.clone(),
                    ..DecodeOptions::default()
                };
                let start = Instant::now();
                let content = guard(|| shared.registry.decode(&source, &options));
                timings.decode_full = Some(start.elapsed());
                Outcome::Full(content, stamp)
            }
            Err(early) => early_outcome(early, Outcome::Full),
        },
        Work::Screen { path, target } => match read(shared, tid, task, path, false, timings) {
            Ok((source, stamp)) => {
                Outcome::Screen(screen_image(shared, task, &source, *target), stamp)
            }
            Err(early) => early_outcome(early, Outcome::Screen),
        },
        Work::Thumb {
            from: Some((content, stamp)),
            side,
            ..
        } => Outcome::Thumb(thumbnail_of(shared, task, content, *side), *stamp),
        Work::Thumb {
            path,
            from: None,
            side,
        } => match read(shared, tid, task, path, false, timings) {
            Ok((source, stamp)) => {
                let side = *side;
                let options = DecodeOptions {
                    target_size: Some((side, side)),
                    first_frame_only: true,
                    prefer_8bit: true,
                    cancel: task.cancel.clone(),
                };
                let image = guard(|| shared.registry.decode(&source, &options))
                    .and_then(|content| thumbnail_of(shared, task, &content, side));
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

/// The image's size when a quick reduced decode is worth it: the image is
/// larger than `target`. Runs on the quick stage's own thread (a probe
/// through an operating system codec takes a while).
fn quick_meta(shared: &Shared, source: &Source, target: (u32, u32)) -> Option<ImageMeta> {
    let meta = guard(|| shared.registry.probe(source)).ok()?;
    (meta.width > target.0 || meta.height > target.1).then_some(meta)
}

/// Start the quick first stage of the current image on its own thread,
/// beside the full decode: a reduced decode covering `target`, delivered
/// as the image's screen image as soon as it is ready. It shares the
/// task's cancel token; a result that arrives after the full image (or
/// after the user moved on) is dropped by the state.
fn start_quick(
    shared: &Arc<Shared>,
    task: &Task,
    source: &Source,
    target: (u32, u32),
    stamp: Option<Stamp>,
) {
    let Work::Full { path, .. } = &task.work else {
        return;
    };
    let (shared, source, path, id) = (Arc::clone(shared), source.clone(), path.clone(), task.id);
    let cancel: CancelToken = task.cancel.clone();
    let spawned = thread::Builder::new()
        .name("pandite-library-quick".into())
        .spawn(move || {
            // Only for decoders that make reduced images of this file
            // cheaply (asking may look into the file, so it happens here,
            // beside the full decode).
            let cheap = shared
                .registry
                .decoder_for(&source)
                .is_some_and(|d| d.reduced_decode(&source));
            if !cheap {
                return;
            }
            let Some(meta) = quick_meta(&shared, &source, target) else {
                return;
            };
            let options = DecodeOptions {
                target_size: Some(target),
                first_frame_only: true,
                prefer_8bit: true,
                cancel,
            };
            let start = Instant::now();
            let Ok(content) = guard(|| shared.registry.decode(&source, &options)) else {
                return;
            };
            let elapsed = start.elapsed();
            let image = content.first_image().clone();
            if image.width() >= meta.width && image.height() >= meta.height {
                // Not reduced after all: the full stage delivers it.
                return;
            }
            let reduced = Reduced {
                image,
                width: meta.width,
                height: meta.height,
            };
            let wake = shared
                .lock()
                .deliver_quick(&path, id, reduced, stamp, elapsed);
            if wake {
                (shared.wake)();
            }
        });
    // No thread: the full stage alone shows the image.
    drop(spawned);
}

/// A read-ahead neighbor at screen resolution: a reduced decode covering
/// `target`, the image itself when it is no larger, or the full decode
/// shrunk to the screen when the format has no reduced decode.
fn screen_image(
    shared: &Shared,
    task: &Task,
    source: &Source,
    target: (u32, u32),
) -> Result<Staged> {
    let meta = guard(|| shared.registry.probe(source)).ok();
    let options = DecodeOptions {
        target_size: Some(target),
        prefer_8bit: true,
        cancel: task.cancel.clone(),
        ..DecodeOptions::default()
    };
    let content = guard(|| shared.registry.decode(source, &options))?;
    task.cancel.check()?;
    let image = content.first_image();
    let (w, h) = (image.width(), image.height());
    if let Some(meta) = meta.filter(|m| w < m.width || h < m.height) {
        return Ok(Staged::Reduced(Reduced {
            image: image.clone(),
            width: meta.width,
            height: meta.height,
        }));
    }
    if (w <= target.0 && h <= target.1) || content.kind() != pandite_core::ContentKind::Still {
        return Ok(Staged::Full(content));
    }
    let side = target.0.max(target.1);
    let small = guard(|| Ok((shared.downscale)(image, side)))?;
    Ok(Staged::Reduced(Reduced {
        image: small,
        width: w,
        height: h,
    }))
}

fn thumbnail_of(shared: &Shared, task: &Task, content: &Content, side: u32) -> Result<ImageBuf> {
    task.cancel.check()?;
    guard(|| Ok((shared.make_thumbnail)(content.first_image(), side)))
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
    timings: &mut LoadTimings,
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
    } else {
        shared.lock().mark_local_read(task);
    }
    // A file that no decoder knows by its first bytes or its extension is
    // refused here, before the whole file is read: a large document, or
    // one on a slow drive, would keep the user waiting for nothing.
    if let Some(Ok(header)) = shared
        .files
        .read_header(path, pandite_core::provider::SNIFF_LEN)
    {
        let probe = Source::from_vec(header, Some(path.to_path_buf()));
        if shared.registry.decoder_for(&probe).is_none() {
            let err = Error::Unsupported(path.display().to_string());
            return Err(leave(Early::Failed(err, stamp)));
        }
    }
    let start = Instant::now();
    let read = guard(|| shared.files.read(path, &task.cancel));
    timings.read = Some(start.elapsed());
    match read {
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

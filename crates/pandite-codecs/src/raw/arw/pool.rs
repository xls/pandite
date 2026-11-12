//! A minimal work queue on scoped standard threads.

use super::{fail, ArwResult};
use pandite_core::{CancelToken, Error};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Worker threads to use: the available parallelism, at least 1.
pub(super) fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Run `work` on every item, on up to [`threads`] threads, checking the
/// cancel token before each item. The first error stops the remaining
/// work; cancellation wins over other errors.
pub(super) fn run<T, F>(items: Vec<T>, cancel: &CancelToken, work: F) -> ArwResult<()>
where
    T: Send,
    F: Fn(T) -> ArwResult<()> + Sync,
{
    let n = threads().min(items.len());
    if n <= 1 {
        for item in items {
            cancel.check()?;
            work(item)?;
        }
        return Ok(());
    }
    let queue = Mutex::new(items.into_iter());
    let stop = AtomicBool::new(false);
    let worker = || -> ArwResult<()> {
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if let Err(e) = cancel.check() {
                stop.store(true, Ordering::Relaxed);
                return Err(e);
            }
            let next = match queue.lock() {
                Ok(mut q) => q.next(),
                Err(_) => return Err(fail("work queue poisoned")),
            };
            let Some(item) = next else {
                return Ok(());
            };
            if let Err(e) = work(item) {
                stop.store(true, Ordering::Relaxed);
                return Err(e);
            }
        }
    };
    let mut results: Vec<ArwResult<()>> = Vec::with_capacity(n);
    std::thread::scope(|s| {
        // Helpers that fail to start are simply missing; the calling thread
        // always works too.
        let helpers: Vec<_> = (1..n)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(s, worker).ok())
            .collect();
        results.push(worker());
        for h in helpers {
            results.push(h.join().unwrap_or_else(|_| Err(fail("worker panicked"))));
        }
    });
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    results.into_iter().collect()
}

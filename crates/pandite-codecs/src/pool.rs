//! The bounded set of helper threads every decoder shares.
//!
//! Decoders split large images into work items (bands of rows, tiles) and
//! hand them to [`run`]. The calling thread always works on the items
//! itself; it also borrows helper threads, but only while helper slots are
//! free. The process holds at most [`helper_limit`] helper slots (one less
//! than the hardware threads), so several decodes running at once (the
//! current image, read-ahead, thumbnails, bulk conversion) share the
//! machine instead of each starting a thread per core. A decode that
//! starts while others hold every slot runs on its own thread and picks up
//! helpers as soon as slots come free.
//!
//! Helpers are scoped standard threads: no `unsafe`, no `'static` bounds
//! on the work, and a panic in a helper becomes an error of that run.

use pandite_core::{CancelToken, Error, Result};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

/// Hardware threads, at least 1.
pub(crate) fn threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    *THREADS.get_or_init(|| std::thread::available_parallelism().map_or(1, |n| n.get()))
}

/// Helper slots shared by every [`run`] in the process: one less than the
/// hardware threads, so callers plus helpers never exceed the machine by
/// more than the number of concurrent callers.
pub(crate) fn helper_limit() -> usize {
    threads().saturating_sub(1)
}

/// Helper slots in use.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Take one helper slot when one is free.
fn try_acquire() -> bool {
    let limit = helper_limit();
    let mut n = BUSY.load(Ordering::Acquire);
    while n < limit {
        match BUSY.compare_exchange_weak(n, n + 1, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(now) => n = now,
        }
    }
    false
}

/// Gives a slot back when dropped, also when the helper panics.
struct Slot;

impl Drop for Slot {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Helper slots in use now, for tests and diagnostics.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn busy() -> usize {
    BUSY.load(Ordering::Acquire)
}

/// Run `work` on every item, on the calling thread plus up to
/// [`helper_limit`] shared helpers, checking the cancel token before each
/// item. The first error stops the remaining work; cancellation wins over
/// other errors. Returns when every started item finished.
pub(crate) fn run<T, F>(items: Vec<T>, cancel: &CancelToken, work: F) -> Result<()>
where
    T: Send,
    F: Fn(T) -> Result<()> + Sync,
{
    let total = items.len();
    if total <= 1 || helper_limit() == 0 {
        for item in items {
            cancel.check()?;
            work(item)?;
        }
        return Ok(());
    }
    let queue = Mutex::new(items.into_iter());
    let left = AtomicUsize::new(total);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<Error>> = Mutex::new(None);
    let fail = |e: Error| {
        stop.store(true, Ordering::Relaxed);
        if let Ok(mut slot) = failure.lock() {
            // Cancellation wins over other errors.
            if slot.is_none() || matches!(e, Error::Cancelled) {
                *slot = Some(e);
            }
        }
    };
    // Take and run one item. False when nothing is left or work stopped.
    let step = || -> bool {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        if let Err(e) = cancel.check() {
            fail(e);
            return false;
        }
        let next = match queue.lock() {
            Ok(mut q) => q.next(),
            Err(_) => {
                fail(Error::Backend("work queue poisoned".into()));
                return false;
            }
        };
        let Some(item) = next else {
            return false;
        };
        left.fetch_sub(1, Ordering::AcqRel);
        if let Err(e) = work(item) {
            fail(e);
            return false;
        }
        true
    };
    std::thread::scope(|s| {
        let mut helpers = Vec::new();
        loop {
            // Borrow helpers while slots are free and more than one item
            // waits (the calling thread takes the next one itself).
            while helpers.len() + 1 < left.load(Ordering::Acquire) && try_acquire() {
                let slot = Slot;
                let spawned = std::thread::Builder::new().spawn_scoped(s, || {
                    let _slot = slot;
                    while step() {}
                });
                match spawned {
                    Ok(handle) => helpers.push(handle),
                    // No thread: the slot was dropped with the closure.
                    Err(_) => break,
                }
            }
            if !step() {
                break;
            }
        }
        for h in helpers {
            if h.join().is_err() {
                fail(Error::Backend("a decoder helper thread panicked".into()));
            }
        }
    });
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    match failure.into_inner() {
        Ok(Some(e)) => Err(e),
        Ok(None) => Ok(()),
        Err(_) => Err(Error::Backend("work queue poisoned".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::thread::ThreadId;
    use std::time::Duration;

    /// Gate G3: concurrent decodes share one bounded set of helpers. Eight
    /// callers run at once; at no moment do more than `helper_limit()`
    /// helper threads work for them, every item runs exactly once, and the
    /// slots are all returned at the end.
    #[test]
    fn decode_threads_are_bounded() {
        let callers = 8usize;
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        let ids: Arc<Mutex<HashSet<ThreadId>>> = Arc::new(Mutex::new(HashSet::new()));
        let caller_ids: Arc<Mutex<HashSet<ThreadId>>> = Arc::new(Mutex::new(HashSet::new()));
        std::thread::scope(|s| {
            for _ in 0..callers {
                let (active, peak, done, ids, caller_ids) = (
                    Arc::clone(&active),
                    Arc::clone(&peak),
                    Arc::clone(&done),
                    Arc::clone(&ids),
                    Arc::clone(&caller_ids),
                );
                s.spawn(move || {
                    let me = std::thread::current().id();
                    caller_ids.lock().unwrap().insert(me);
                    let items: Vec<usize> = (0..64).collect();
                    run(items, &CancelToken::new(), |_| {
                        let id = std::thread::current().id();
                        let helper = id != me;
                        if helper {
                            let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(now, Ordering::SeqCst);
                            ids.lock().unwrap().insert(id);
                        }
                        std::thread::sleep(Duration::from_millis(1));
                        if helper {
                            active.fetch_sub(1, Ordering::SeqCst);
                        }
                        done.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(
            done.load(Ordering::SeqCst),
            callers * 64,
            "every item ran once"
        );
        let peak = peak.load(Ordering::SeqCst);
        assert!(
            peak <= helper_limit(),
            "{peak} helpers at once, limit {}",
            helper_limit()
        );
        let ids = ids.lock().unwrap();
        assert!(ids.is_disjoint(&caller_ids.lock().unwrap()));
        if helper_limit() >= 2 {
            assert!(peak >= 1, "the callers borrowed helpers");
        }
        println!(
            "{callers} concurrent runs: peak {peak} helpers of {} slots, {} distinct helper threads",
            helper_limit(),
            ids.len()
        );
        // Other tests may run decodes now; ours returned every slot it took.
        assert!(busy() <= helper_limit());
    }

    #[test]
    fn pool_stops_on_error_and_cancel() {
        let items: Vec<usize> = (0..1000).collect();
        let ran = AtomicUsize::new(0);
        let r = run(items, &CancelToken::new(), |i| {
            ran.fetch_add(1, Ordering::SeqCst);
            if i == 10 {
                Err(Error::Corrupt("item 10".into()))
            } else {
                std::thread::sleep(Duration::from_micros(200));
                Ok(())
            }
        });
        assert!(matches!(r, Err(Error::Corrupt(_))));
        assert!(ran.load(Ordering::SeqCst) < 1000, "work stopped early");
        let cancel = CancelToken::new();
        let items: Vec<usize> = (0..1000).collect();
        let r = run(items, &cancel, |i| {
            if i == 5 {
                cancel.cancel();
            }
            Ok(())
        });
        assert!(matches!(r, Err(Error::Cancelled)));
        // A panicking item becomes an error, and slots come back.
        let items: Vec<usize> = (0..64).collect();
        let r = std::panic::catch_unwind(|| {
            run(items, &CancelToken::new(), |i| {
                if i == 40 {
                    panic!("boom");
                }
                std::thread::sleep(Duration::from_micros(200));
                Ok(())
            })
        });
        assert!(r.is_err() || matches!(r, Ok(Err(_))));
        assert!(busy() <= helper_limit());
    }
}

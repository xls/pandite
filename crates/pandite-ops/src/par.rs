//! Row-parallel helpers on std scoped threads.

use pandite_core::{Error, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// Each thread gets at least this many pixels of work (source plus
/// destination): below that, starting a thread costs more than it saves.
const MIN_PIXELS_PER_THREAD: u64 = 1 << 16;

/// Each thread gets at least this many output rows.
const MIN_ROWS_PER_THREAD: usize = 8;

/// Number of hardware threads, at least 1.
pub(crate) fn available_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// The thread count for `pixels` of work that produces `rows` output rows.
/// `max` caps the count; 0 means every hardware thread.
pub(crate) fn plan_threads(max: usize, pixels: u64, rows: usize) -> usize {
    let max = if max == 0 { available_threads() } else { max };
    let by_work = usize::try_from(pixels / MIN_PIXELS_PER_THREAD).unwrap_or(usize::MAX);
    max.min(rows / MIN_ROWS_PER_THREAD).min(by_work).max(1)
}

/// Split `out`, a buffer of rows that are `row_len` elements long, into
/// `threads` contiguous bands and call `work(first_row, band)` once per band.
///
/// Bands run on scoped threads plus the calling thread. A thread that
/// fails to start is not an error: the other threads take its band. The
/// first error stops the remaining bands and is returned.
pub(crate) fn for_each_band<T, F>(
    out: &mut [T],
    row_len: usize,
    threads: usize,
    work: F,
) -> Result<()>
where
    T: Send,
    F: Fn(usize, &mut [T]) -> Result<()> + Sync,
{
    let rows = out.len().checked_div(row_len).unwrap_or(0);
    let threads = threads.clamp(1, rows.max(1));
    if threads == 1 {
        return work(0, out);
    }
    let band_rows = rows.div_ceil(threads);
    let mut bands: Vec<(usize, &mut [T])> = out
        .chunks_mut(band_rows * row_len)
        .enumerate()
        .map(|(i, band)| (i * band_rows, band))
        .collect();
    // Pop from the back hands out the top band first.
    bands.reverse();
    let queue = Mutex::new(bands);
    let stop = AtomicBool::new(false);
    let failure: Mutex<Option<Error>> = Mutex::new(None);
    let run = || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let next = queue.lock().unwrap_or_else(PoisonError::into_inner).pop();
        let Some((first_row, band)) = next else {
            break;
        };
        if let Err(err) = work(first_row, band) {
            stop.store(true, Ordering::Relaxed);
            let mut slot = failure.lock().unwrap_or_else(PoisonError::into_inner);
            if slot.is_none() {
                *slot = Some(err);
            }
            break;
        }
    };
    std::thread::scope(|scope| {
        for _ in 1..threads {
            // Ignoring a failed start is deliberate: the queue still drains.
            let _ = std::thread::Builder::new()
                .name("pandite-ops".into())
                .spawn_scoped(scope, run);
        }
        run();
    });
    match failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bands_cover_every_row_once() {
        let mut out = vec![0u32; 7 * 3];
        for_each_band(&mut out, 3, 4, |first, band| {
            for (r, row) in band.chunks_exact_mut(3).enumerate() {
                for v in row {
                    *v += (first + r) as u32 + 1;
                }
            }
            Ok(())
        })
        .unwrap();
        let rows: Vec<u32> = out.chunks_exact(3).map(|r| r[0]).collect();
        assert_eq!(rows, vec![1, 2, 3, 4, 5, 6, 7]);
        assert!(out.chunks_exact(3).all(|r| r[0] == r[1] && r[1] == r[2]));
    }

    #[test]
    fn first_error_is_returned() {
        let mut out = vec![0u8; 64];
        let res = for_each_band(&mut out, 1, 8, |first, _| {
            if first >= 32 {
                Err(Error::Invalid("band failed".into()))
            } else {
                Ok(())
            }
        });
        assert!(matches!(res, Err(Error::Invalid(_))));
    }

    #[test]
    fn small_work_stays_on_one_thread() {
        assert_eq!(plan_threads(0, 100, 1000), 1);
        assert_eq!(plan_threads(64, (1 << 17) - 1, 1000), 1);
        assert_eq!(plan_threads(64, 5 << 16, 1000), 5);
        assert_eq!(plan_threads(4, 1 << 30, 1000), 4);
        assert_eq!(plan_threads(64, 1 << 30, 16), 2);
        assert!(plan_threads(0, 1 << 30, 1 << 20) >= 1);
    }
}

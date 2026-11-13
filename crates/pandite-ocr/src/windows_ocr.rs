//! `Windows.Media.Ocr`: the text recognizer built into Windows 10 and 11.
//!
//! The engine is reached through WinRT. It reads 8-bit gray bitmaps of at
//! most `OcrEngine::MaxImageDimension` (10000) pixels per side; larger
//! images are scaled to [`crate::MAX_SIDE`] first and the word boxes are
//! mapped back to full-image pixels.
//!
//! The engine works on a system thread pool thread, at the pool's
//! priority, and ignores cancellation of a running request: `Cancel` is
//! sent, but the request runs to its end. A cancelled
//! [`WindowsOcr::recognize`] therefore stops waiting and returns at once,
//! and the next call waits until the engine has finished the abandoned
//! request, so abandoned requests never pile up. The calling thread blocks
//! while it waits; it converts and scales the image and waits at the
//! lowest normal scheduling priority.
//!
//! This module is the crate's only `unsafe` code: the calls that read and
//! set the calling thread's priority.

#![allow(unsafe_code)]

use crate::prepare::{self, GrayImage};
use pandite_core::{
    CancelToken, Error, ImageBuf, Rect, Result, TextLayout, TextLine, TextProvider, TextWord,
};
use std::marker::PhantomData;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Storage::Streams::DataWriter;
use windows::Win32::System::Threading::{
    GetCurrentThread, GetThreadPriority, SetThreadPriority, THREAD_PRIORITY, THREAD_PRIORITY_LOWEST,
};

/// How often a waiting call looks at its cancel token.
const POLL: Duration = Duration::from_millis(5);
/// A request the engine has not finished after this long is given up.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// The documented engine limit, used when the engine cannot be asked.
const FALLBACK_LIMIT: u32 = 10_000;
/// What `GetThreadPriority` returns on failure.
const PRIORITY_ERROR: i32 = i32::MAX;

/// Text recognition with the Windows OCR engine.
///
/// The default instance reads the languages of the user's profile (the
/// display and input languages that have OCR data). Recognition runs fully
/// offline.
#[derive(Debug)]
pub struct WindowsOcr {
    /// A BCP-47 language tag to read instead of the profile languages.
    language: Option<String>,
    /// Longest side recognized, before the engine's own limit applies.
    max_side: u32,
    available: OnceLock<bool>,
}

impl Default for WindowsOcr {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsOcr {
    /// The recognizer for the user's profile languages, reading images at
    /// most [`crate::MAX_SIDE`] pixels on the long side.
    pub fn new() -> Self {
        Self {
            language: None,
            max_side: crate::MAX_SIDE,
            available: OnceLock::new(),
        }
    }

    /// The recognizer for one language (a BCP-47 tag such as `"en-US"`).
    /// A tag without installed OCR data gives a recognizer that reports
    /// itself unavailable.
    pub fn for_language(tag: &str) -> Self {
        Self {
            language: Some(tag.to_string()),
            ..Self::new()
        }
    }

    /// The same recognizer reading images at most `pixels` on the long
    /// side (still never above the engine's limit). For measurements.
    pub fn with_max_side(mut self, pixels: u32) -> Self {
        self.max_side = pixels.max(1);
        self
    }

    /// The largest side the engine accepts, when it can be asked.
    pub fn engine_limit() -> Option<u32> {
        OcrEngine::MaxImageDimension().ok().filter(|&v| v > 0)
    }

    fn engine(&self) -> Result<OcrEngine> {
        let engine = match &self.language {
            None => OcrEngine::TryCreateFromUserProfileLanguages(),
            Some(tag) => Language::CreateLanguage(&HSTRING::from(tag.as_str()))
                .and_then(|language| OcrEngine::TryCreateFromLanguage(&language)),
        };
        engine.map_err(|_| {
            Error::Unavailable(match &self.language {
                None => "no Windows OCR language is installed for this user".into(),
                Some(tag) => format!("Windows OCR has no data for the language {tag}"),
            })
        })
    }

    fn run(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<TextLayout> {
        let _priority = LowPriority::enter();
        let engine = self.engine()?;
        let limit = Self::engine_limit()
            .unwrap_or(FALLBACK_LIMIT)
            .min(self.max_side);
        let size = prepare::recognition_size(image.width(), image.height(), limit);
        let gray = prepare::gray(image, size.0, size.1, cancel)?;
        cancel.check()?;
        let bitmap = software_bitmap(&gray)?;
        drop(gray);
        let result = recognize_bitmap(&engine, &bitmap, cancel)?;
        cancel.check()?;
        layout(&result, size, (image.width(), image.height()))
    }
}

impl TextProvider for WindowsOcr {
    fn id(&self) -> &'static str {
        "windows-ocr"
    }

    fn name(&self) -> &'static str {
        "Windows OCR"
    }

    fn available(&self) -> bool {
        *self.available.get_or_init(|| {
            catch_unwind(AssertUnwindSafe(|| self.engine().is_ok())).unwrap_or(false)
        })
    }

    fn recognize(&self, image: &ImageBuf, cancel: &CancelToken) -> Result<TextLayout> {
        cancel.check()?;
        if !self.available() {
            // The engine call reports the precise reason.
            return Err(self
                .engine()
                .err()
                .unwrap_or_else(|| Error::Unavailable("Windows OCR is not available".into())));
        }
        match catch_unwind(AssertUnwindSafe(|| self.run(image, cancel))) {
            Ok(result) => result,
            Err(_) => Err(Error::Backend("Windows OCR stopped unexpectedly".into())),
        }
    }
}

fn backend(what: &str, err: windows::core::Error) -> Error {
    Error::Backend(format!("Windows OCR: {what}: {err}"))
}

/// The gray pixels as a WinRT bitmap. The data is copied twice (into the
/// writer's buffer, then into the bitmap); both copies are small next to
/// the recognition itself.
fn software_bitmap(gray: &GrayImage) -> Result<SoftwareBitmap> {
    let too_large = || Error::TooLarge {
        width: gray.width,
        height: gray.height,
    };
    let w = i32::try_from(gray.width).map_err(|_| too_large())?;
    let h = i32::try_from(gray.height).map_err(|_| too_large())?;
    u32::try_from(gray.pixels.len()).map_err(|_| too_large())?;
    let writer = DataWriter::new().map_err(|e| backend("data writer", e))?;
    writer
        .WriteBytes(&gray.pixels)
        .map_err(|e| backend("data writer", e))?;
    let buffer = writer
        .DetachBuffer()
        .map_err(|e| backend("data writer", e))?;
    SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, w, h)
        .map_err(|e| backend("bitmap", e))
}

/// Run one request: wait until the engine is free, start the request and
/// wait for its answer, checking `cancel` throughout.
fn recognize_bitmap(
    engine: &OcrEngine,
    bitmap: &SoftwareBitmap,
    cancel: &CancelToken,
) -> Result<OcrResult> {
    let ticket = GATE.acquire(cancel)?;
    let op = match engine.RecognizeAsync(bitmap) {
        Ok(op) => op,
        Err(e) => {
            GATE.release(ticket);
            return Err(backend("recognition did not start", e));
        }
    };
    let (tx, rx) = mpsc::channel();
    // The completion handler runs on a system thread when the engine is
    // done, also for a request this call abandoned: only then is the
    // engine free for the next request.
    let registered = op.when(move |result| {
        GATE.release(ticket);
        let _ = tx.send(result);
    });
    if let Err(e) = registered {
        let _ = op.Cancel();
        GATE.release(ticket);
        return Err(backend("recognition did not start", e));
    }
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        match rx.recv_timeout(POLL) {
            Ok(Ok(result)) => return Ok(result),
            Ok(Err(e)) => return Err(backend("recognition failed", e)),
            Err(RecvTimeoutError::Timeout) => {
                if cancel.is_cancelled() {
                    let _ = op.Cancel();
                    return Err(Error::Cancelled);
                }
                if Instant::now() >= deadline {
                    let _ = op.Cancel();
                    return Err(Error::Backend("Windows OCR did not answer in time".into()));
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(Error::Backend("Windows OCR dropped the request".into()));
            }
        }
    }
}

/// The engine's answer as a layout in full-image pixels.
fn layout(result: &OcrResult, recognized: (u32, u32), image: (u32, u32)) -> Result<TextLayout> {
    let read = |what| move |e| backend(what, e);
    // A missing angle (no text, or the engine did not measure one) is 0.
    let angle = result
        .TextAngle()
        .and_then(|a| a.Value())
        .map_or(0.0, |a| a as f32);
    let lines = result.Lines().map_err(read("lines"))?;
    let mut out = TextLayout::new(image.0, image.1);
    for i in 0..lines.Size().map_err(read("lines"))? {
        let line = lines.GetAt(i).map_err(read("line"))?;
        let words = line.Words().map_err(read("words"))?;
        let count = words.Size().map_err(read("words"))?;
        let mut kept = Vec::with_capacity(count as usize);
        for j in 0..count {
            let word = words.GetAt(j).map_err(read("word"))?;
            let text = word.Text().map_err(read("word text"))?.to_string_lossy();
            let text = text.trim();
            if text.is_empty() {
                continue;
            }
            let r = word.BoundingRect().map_err(read("word box"))?;
            let rect = Rect::new(r.X, r.Y, r.Width, r.Height);
            kept.push(TextWord {
                text: text.to_string(),
                quad: prepare::to_image(rect, recognized, image, angle),
            });
        }
        if !kept.is_empty() {
            out.lines.push(TextLine { words: kept });
        }
    }
    Ok(out)
}

/// Lets one request at a time into the engine. A ticket names the request
/// holding it, so a late completion of a request that was given up never
/// frees the engine for the request after it.
struct Gate {
    state: Mutex<GateState>,
    free: Condvar,
}

struct GateState {
    next: u64,
    holder: Option<(u64, Instant)>,
}

static GATE: Gate = Gate {
    state: Mutex::new(GateState {
        next: 1,
        holder: None,
    }),
    free: Condvar::new(),
};

impl Gate {
    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Wait until no request holds the engine (or the holder has run past
    /// [`REQUEST_TIMEOUT`] and counts as lost), then take it.
    fn acquire(&self, cancel: &CancelToken) -> Result<u64> {
        let mut state = self.lock();
        loop {
            let free = state
                .holder
                .is_none_or(|(_, since)| since.elapsed() >= REQUEST_TIMEOUT);
            if free {
                break;
            }
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            state = self
                .free
                .wait_timeout(state, POLL)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        let ticket = state.next;
        state.next += 1;
        state.holder = Some((ticket, Instant::now()));
        Ok(ticket)
    }

    fn release(&self, ticket: u64) {
        let mut state = self.lock();
        if state.holder.is_some_and(|(held, _)| held == ticket) {
            state.holder = None;
            drop(state);
            self.free.notify_all();
        }
    }
}

/// Runs the calling thread at the lowest normal priority until dropped,
/// then restores the priority it had. Not `Send`: it is dropped on the
/// thread it lowered.
struct LowPriority(Option<i32>, PhantomData<*const ()>);

impl LowPriority {
    fn enter() -> Self {
        // SAFETY: GetCurrentThread returns a pseudo handle for the calling
        // thread that needs no closing; GetThreadPriority and
        // SetThreadPriority only read and change that thread's scheduling
        // priority and touch no memory of ours.
        let old = unsafe {
            let thread = GetCurrentThread();
            let old = GetThreadPriority(thread);
            if old == PRIORITY_ERROR || old <= THREAD_PRIORITY_LOWEST.0 {
                None
            } else {
                SetThreadPriority(thread, THREAD_PRIORITY_LOWEST)
                    .ok()
                    .map(|()| old)
            }
        };
        Self(old, PhantomData)
    }
}

impl Drop for LowPriority {
    fn drop(&mut self) {
        if let Some(old) = self.0 {
            // SAFETY: the same pseudo handle and call as in `enter`. The
            // guard is not Send, so this runs on the thread that lowered
            // its own priority and restores exactly that.
            unsafe {
                let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY(old));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_lets_one_request_in_and_ignores_stale_tickets() {
        let cancel = CancelToken::new();
        let first = GATE.acquire(&cancel).unwrap();
        // A second request waits; once cancelled it gives up promptly.
        let waiter = std::thread::spawn(|| {
            let cancel = CancelToken::new();
            let token = cancel.clone();
            let canceller = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(30));
                token.cancel();
            });
            let started = Instant::now();
            let got = GATE.acquire(&cancel);
            canceller.join().unwrap();
            (got.is_err(), started.elapsed())
        });
        let (gave_up, waited) = waiter.join().unwrap();
        assert!(gave_up, "the second request must not get the engine");
        assert!(waited < Duration::from_secs(5), "{waited:?}");
        // A stale ticket does not free the engine; the right one does.
        GATE.release(first + 1000);
        assert!(GATE.lock().holder.is_some());
        GATE.release(first);
        let next = GATE.acquire(&cancel).unwrap();
        assert!(next > first);
        GATE.release(next);
    }

    #[test]
    fn low_priority_is_restored() {
        // SAFETY: reads the calling thread's priority through its pseudo
        // handle; no memory is touched.
        let before = unsafe { GetThreadPriority(GetCurrentThread()) };
        {
            let _low = LowPriority::enter();
            // SAFETY: as above.
            let during = unsafe { GetThreadPriority(GetCurrentThread()) };
            assert!(during <= THREAD_PRIORITY_LOWEST.0.max(before), "{during}");
        }
        // SAFETY: as above.
        let after = unsafe { GetThreadPriority(GetCurrentThread()) };
        assert_eq!(before, after);
    }
}

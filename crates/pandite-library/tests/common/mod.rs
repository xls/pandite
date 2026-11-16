//! Test support: a counting fake decoder and a fake file system, both
//! driven by latches (a condition variable and a flag the test flips), and
//! a harness that collects events through the wake callback.
//!
//! Nothing here sleeps to wait for an outcome. Every wait blocks on a
//! condition variable or a channel and has a generous deadline that only
//! turns a hang into a failure; a passing test never depends on timing.

#![allow(dead_code)]

use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, Error, ImageBuf, ImageMeta,
    Orientation, Registry, Result, Source,
};
use pandite_library::{Event, FileAccess, FileInfo, Library, LibraryConfig, StdFiles};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

/// Deadline that turns a hang into a test failure. Never reached when the
/// code is correct.
pub const WAIT: Duration = Duration::from_secs(30);
/// Bound for calls that must return at once (they take microseconds; a
/// blocked call never returns). Generous so a loaded machine cannot fail it.
pub const QUICK: Duration = Duration::from_secs(10);

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Wait on `cv` until `done` holds or `WAIT` passes.
fn wait_until<T>(m: &Mutex<T>, cv: &Condvar, mut done: impl FnMut(&T) -> bool) -> bool {
    let deadline = Instant::now() + WAIT;
    let mut g = lock(m);
    while !done(&g) {
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        g = cv
            .wait_timeout(g, deadline - now)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    true
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

// ----- temporary folder -----

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder under the system temp directory, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pandite-library-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write a fake image file: the fake decoder reads its size from it.
    pub fn image(&self, name: &str, width: u32, height: u32) -> PathBuf {
        self.file(name, format!("FAKE {width} {height}").as_bytes())
    }

    pub fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path.join(name);
        std::fs::write(&path, bytes).expect("write temp file");
        path
    }

    /// `count` fake images named img00.fake, img01.fake, ...
    pub fn images(&self, count: usize, width: u32, height: u32) -> Vec<PathBuf> {
        (0..count)
            .map(|i| self.image(&img(i), width, height))
            .collect()
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// The file name of fake image `i`.
pub fn img(i: usize) -> String {
    format!("img{i:02}.fake")
}

/// File names of fake images.
pub fn imgs(indexes: &[usize]) -> Vec<String> {
    indexes.iter().map(|&i| img(i)).collect()
}

// ----- fake decoder -----

/// One decode call: the file name and whether it was a thumbnail decode.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Call {
    pub name: String,
    pub thumb: bool,
}

impl Call {
    pub fn full(name: &str) -> Self {
        Self {
            name: name.into(),
            thumb: false,
        }
    }

    pub fn thumb(name: &str) -> Self {
        Self {
            name: name.into(),
            thumb: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Ok,
    Failed,
    Cancelled,
}

#[derive(Default)]
struct DecState {
    starts: Vec<Call>,
    /// Every decode in start order with its target size.
    targets: Vec<(String, Option<(u32, u32)>)>,
    ends: Vec<(Call, End)>,
    holds: HashSet<Call>,
    failing: HashSet<String>,
}

/// Decodes "FAKE <w> <h>" files into solid images. Counts every call in
/// order, blocks a held call until released or cancelled, and fails on
/// demand.
#[derive(Default)]
pub struct FakeDecoder {
    state: Mutex<DecState>,
    cv: Condvar,
    /// Decode reduced images cheaply for a target size (staged loading).
    reduced: AtomicBool,
    /// Held decodes ignore their cancel token, like a decoder inside a
    /// call that cannot be interrupted.
    deaf: AtomicBool,
}

impl FakeDecoder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn registry(self: &Arc<Self>) -> Arc<Registry> {
        let mut reg = Registry::new();
        reg.add_decoder(Arc::clone(self) as Arc<dyn DecoderProvider>);
        Arc::new(reg)
    }

    /// The next decodes of `call` block until [`FakeDecoder::release`] or
    /// until their cancel token is set.
    pub fn hold(&self, call: Call) {
        lock(&self.state).holds.insert(call);
    }

    pub fn release(&self, call: &Call) {
        lock(&self.state).holds.remove(call);
        self.cv.notify_all();
    }

    pub fn release_all(&self) {
        lock(&self.state).holds.clear();
        self.cv.notify_all();
    }

    /// Held decodes stop only on release, not on cancellation.
    pub fn ignore_cancel(&self) {
        self.deaf.store(true, Ordering::SeqCst);
    }

    pub fn fail(&self, name: &str) {
        lock(&self.state).failing.insert(name.into());
    }

    /// Report cheap reduced decodes: a target size then yields an image
    /// that just covers it.
    pub fn enable_reduced(&self) {
        self.reduced.store(true, Ordering::SeqCst);
    }

    /// Every decode so far with its target size, in start order.
    pub fn targets(&self) -> Vec<(String, Option<(u32, u32)>)> {
        lock(&self.state).targets.clone()
    }

    pub fn starts(&self) -> Vec<Call> {
        lock(&self.state).starts.clone()
    }

    /// Names of full decodes in start order.
    pub fn full_starts(&self) -> Vec<String> {
        self.starts()
            .into_iter()
            .filter(|c| !c.thumb)
            .map(|c| c.name)
            .collect()
    }

    /// Names of thumbnail decodes in start order.
    pub fn thumb_starts(&self) -> Vec<String> {
        self.starts()
            .into_iter()
            .filter(|c| c.thumb)
            .map(|c| c.name)
            .collect()
    }

    pub fn count(&self, call: &Call) -> usize {
        lock(&self.state)
            .starts
            .iter()
            .filter(|c| *c == call)
            .count()
    }

    pub fn wait_started(&self, call: &Call) -> bool {
        wait_until(&self.state, &self.cv, |s| s.starts.contains(call))
    }

    /// Wait until `call` started `n` times.
    pub fn wait_starts(&self, call: &Call, n: usize) -> bool {
        wait_until(&self.state, &self.cv, |s| {
            s.starts.iter().filter(|c| *c == call).count() >= n
        })
    }

    /// How the first decode of `call` ended.
    pub fn wait_end(&self, call: &Call) -> Option<End> {
        let mut end = None;
        wait_until(&self.state, &self.cv, |s| {
            end = s.ends.iter().find(|(c, _)| c == call).map(|(_, e)| *e);
            end.is_some()
        });
        end
    }

    fn finish(&self, mut g: MutexGuard<'_, DecState>, call: Call, end: End) {
        g.ends.push((call, end));
        drop(g);
        self.cv.notify_all();
    }
}

fn parse(source: &Source) -> Result<(u32, u32)> {
    let text = std::str::from_utf8(source.bytes()).map_err(|_| Error::Corrupt("utf8".into()))?;
    let mut parts = text.split_whitespace().skip(1);
    let mut num = || -> Result<u32> {
        parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| Error::Corrupt("size".into()))
    };
    Ok((num()?, num()?))
}

impl DecoderProvider for FakeDecoder {
    fn id(&self) -> &'static str {
        "fake"
    }

    fn name(&self) -> &'static str {
        "Fake image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["fake"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"FAKE")
    }

    fn probe(&self, source: &Source) -> Result<ImageMeta> {
        let (width, height) = parse(source)?;
        Ok(ImageMeta {
            width,
            height,
            format: "fake",
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: false,
            orientation: Orientation::Normal,
        })
    }

    fn reduced_decode(&self, _source: &Source) -> bool {
        self.reduced.load(Ordering::SeqCst)
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> Result<Content> {
        let call = Call {
            name: source.path().map(name_of).unwrap_or_default(),
            thumb: options.target_size.is_some(),
        };
        let mut g = lock(&self.state);
        g.starts.push(call.clone());
        g.targets.push((call.name.clone(), options.target_size));
        self.cv.notify_all();
        // A held decode waits on the latch. The cancel token is a plain
        // flag with no notification, so it is checked every millisecond;
        // the outcome does not depend on that period.
        while g.holds.contains(&call) {
            if options.cancel.is_cancelled() && !self.deaf.load(Ordering::SeqCst) {
                self.finish(g, call, End::Cancelled);
                return Err(Error::Cancelled);
            }
            g = self
                .cv
                .wait_timeout(g, Duration::from_millis(1))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        if g.failing.contains(&call.name) {
            self.finish(g, call, End::Failed);
            return Err(Error::Corrupt("fake failure".into()));
        }
        drop(g);
        let (mut w, mut h) = parse(source)?;
        if let (Some((tw, th)), true) = (options.target_size, self.reduced.load(Ordering::SeqCst)) {
            // Just cover the target, never enlarge.
            let s = (f64::from(tw) / f64::from(w))
                .max(f64::from(th) / f64::from(h))
                .min(1.0);
            w = ((f64::from(w) * s).ceil() as u32).max(1);
            h = ((f64::from(h) * s).ceil() as u32).max(1);
        }
        let shade = call.name.bytes().fold(0u8, |a, b| a.wrapping_add(b));
        let image = ImageBuf::solid_rgba8(w, h, [shade, 255 - shade, 128, 255])?;
        self.finish(lock(&self.state), call, End::Ok);
        Ok(Content::Still(image))
    }
}

// ----- fake file system -----

#[derive(Default)]
struct FilesState {
    reads: Vec<String>,
    returned: Vec<String>,
    infos: Vec<String>,
    stalled: HashSet<String>,
    info_stalled: HashSet<String>,
    cloud: HashSet<String>,
    list_stalled: bool,
    lists: usize,
    released: bool,
}

/// `std::fs` with injected trouble: reads of chosen files stall (ignoring
/// cancellation, like a read blocked inside the operating system), chosen
/// files report as not local, and the folder listing can stall.
#[derive(Default)]
pub struct FakeFiles {
    state: Mutex<FilesState>,
    cv: Condvar,
}

impl FakeFiles {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn stall_read(&self, name: &str) {
        lock(&self.state).stalled.insert(name.into());
    }

    pub fn release_read(&self, name: &str) {
        lock(&self.state).stalled.remove(name);
        self.cv.notify_all();
    }

    /// Metadata lookups of `name` block until released.
    pub fn stall_info(&self, name: &str) {
        lock(&self.state).info_stalled.insert(name.into());
    }

    pub fn release_info(&self, name: &str) {
        lock(&self.state).info_stalled.remove(name);
        self.cv.notify_all();
    }

    pub fn wait_info_started(&self, name: &str) -> bool {
        wait_until(&self.state, &self.cv, |s| s.infos.iter().any(|n| n == name))
    }

    pub fn set_cloud(&self, name: &str) {
        lock(&self.state).cloud.insert(name.into());
    }

    pub fn stall_listing(&self) {
        lock(&self.state).list_stalled = true;
    }

    pub fn release_listing(&self) {
        lock(&self.state).list_stalled = false;
        self.cv.notify_all();
    }

    /// Open every latch for good (test teardown).
    pub fn release_all(&self) {
        let mut g = lock(&self.state);
        g.released = true;
        g.stalled.clear();
        g.info_stalled.clear();
        g.list_stalled = false;
        drop(g);
        self.cv.notify_all();
    }

    pub fn reads(&self, name: &str) -> usize {
        lock(&self.state)
            .reads
            .iter()
            .filter(|n| *n == name)
            .count()
    }

    pub fn returned(&self, name: &str) -> usize {
        lock(&self.state)
            .returned
            .iter()
            .filter(|n| *n == name)
            .count()
    }

    pub fn wait_read_started(&self, name: &str) -> bool {
        wait_until(&self.state, &self.cv, |s| s.reads.iter().any(|n| n == name))
    }

    pub fn wait_read_returned(&self, name: &str) -> bool {
        wait_until(&self.state, &self.cv, |s| {
            s.returned.iter().any(|n| n == name)
        })
    }

    pub fn wait_listing_started(&self) -> bool {
        wait_until(&self.state, &self.cv, |s| s.lists > 0)
    }

    /// Folder listings that started.
    pub fn lists(&self) -> usize {
        lock(&self.state).lists
    }

    /// Metadata lookups of single files that started.
    pub fn infos(&self) -> usize {
        lock(&self.state).infos.len()
    }

    fn localize(&self, mut info: FileInfo) -> FileInfo {
        if lock(&self.state)
            .cloud
            .contains(info.name.to_string_lossy().as_ref())
        {
            info.local = false;
        }
        info
    }
}

impl FileAccess for FakeFiles {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        let mut g = lock(&self.state);
        g.lists += 1;
        self.cv.notify_all();
        while g.list_stalled && !g.released {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        drop(g);
        let listed = StdFiles.list_dir(dir)?;
        Ok(listed.into_iter().map(|i| self.localize(i)).collect())
    }

    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        let name = name_of(path);
        let mut g = lock(&self.state);
        g.infos.push(name.clone());
        self.cv.notify_all();
        while g.info_stalled.contains(&name) && !g.released {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        drop(g);
        Ok(self.localize(StdFiles.file_info(path)?))
    }

    fn read_header(&self, path: &Path, len: usize) -> Option<io::Result<Vec<u8>>> {
        StdFiles.read_header(path, len)
    }

    fn read(&self, path: &Path, cancel: &CancelToken) -> Result<Source> {
        let name = name_of(path);
        let mut g = lock(&self.state);
        g.reads.push(name.clone());
        self.cv.notify_all();
        while g.stalled.contains(&name) && !g.released {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        drop(g);
        let result = StdFiles.read(path, cancel);
        lock(&self.state).returned.push(name);
        self.cv.notify_all();
        result
    }
}

// ----- harness -----

/// Sends once when dropped: tells the test that the library's shared
/// state (which owns the wake callback) is gone.
struct Sentinel(Sender<()>);

impl Drop for Sentinel {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// A library over a temp folder with the fakes, plus an event log fed by
/// the wake callback. Teardown opens every latch before dropping the
/// library, so no thread stays blocked at process exit.
pub struct Harness {
    pub dir: TempDir,
    pub dec: Arc<FakeDecoder>,
    pub files: Arc<FakeFiles>,
    lib: Option<Arc<Library>>,
    wake_rx: Receiver<()>,
    gone_rx: Receiver<()>,
    wakes: Arc<AtomicUsize>,
    pub events: Vec<Event>,
}

impl Harness {
    pub fn new(tag: &str, tune: impl FnOnce(&mut LibraryConfig)) -> Self {
        let dir = TempDir::new(tag);
        let dec = FakeDecoder::new();
        let files = FakeFiles::new();
        let (wake_tx, wake_rx) = mpsc::channel();
        let (gone_tx, gone_rx) = mpsc::channel();
        let wakes = Arc::new(AtomicUsize::new(0));
        let mut config = LibraryConfig {
            workers: 1,
            read_ahead: 2,
            background_thumbnails: false,
            thumb_margin: 0,
            thumb_size: 8,
            files: Arc::clone(&files) as Arc<dyn FileAccess>,
            ..LibraryConfig::default()
        };
        tune(&mut config);
        let counter = Arc::clone(&wakes);
        let sentinel = Sentinel(gone_tx);
        let wake = Arc::new(move || {
            // The closure owns the sentinel: it is dropped with the
            // library's shared state.
            let _owned = &sentinel;
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = wake_tx.send(());
        });
        let lib = Library::new(dec.registry(), config, wake).expect("library starts");
        Self {
            dir,
            dec,
            files,
            lib: Some(Arc::new(lib)),
            wake_rx,
            gone_rx,
            wakes,
            events: Vec::new(),
        }
    }

    pub fn lib(&self) -> &Arc<Library> {
        self.lib.as_ref().expect("library present")
    }

    /// Hand the library over (to drop it elsewhere).
    pub fn take_lib(&mut self) -> Arc<Library> {
        self.lib.take().expect("library present")
    }

    pub fn wakes(&self) -> usize {
        self.wakes.load(Ordering::SeqCst)
    }

    /// Block until the wake callback fires.
    pub fn wait_wake(&self) -> bool {
        self.wake_rx.recv_timeout(WAIT).is_ok()
    }

    /// Wait until the library is idle and no wake has come for 100 ms,
    /// then move the queued events into the log. A wake of finished work
    /// can come a moment after the library reports idle.
    #[allow(dead_code)]
    pub fn settle(&mut self) {
        self.idle();
        while self
            .wake_rx
            .recv_timeout(Duration::from_millis(100))
            .is_ok()
        {}
        self.pump();
    }

    /// Block until the library's shared state is dropped.
    pub fn wait_gone(&self) -> bool {
        self.gone_rx.recv_timeout(WAIT).is_ok()
    }

    /// Move queued events into the log.
    pub fn pump(&mut self) {
        if let Some(lib) = &self.lib {
            self.events.extend(lib.poll_events());
        }
    }

    /// Sleep on the wake channel until some logged event matches `pred`.
    pub fn wait_for(&mut self, pred: impl Fn(&Event) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            if self.events.iter().any(&pred) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match self.wake_rx.recv_timeout(deadline - now) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.pump();
                    return self.events.iter().any(&pred);
                }
            }
        }
    }

    pub fn count_events(&self, pred: impl Fn(&Event) -> bool) -> usize {
        self.events.iter().filter(|e| pred(e)).count()
    }

    pub fn idle(&self) {
        assert!(self.lib().wait_idle(WAIT), "library did not become idle");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.files.release_all();
        self.dec.release_all();
        self.lib = None;
    }
}

/// Run `f` on a helper thread and wait at most `bound` for it. `None`
/// means the call blocked.
pub fn run_bounded<T: Send + 'static>(
    bound: Duration,
    f: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(bound).ok()
}

//! Test support: recording fakes for the platform traits, a file system
//! with stall latches that records the calling thread, and a harness that
//! drives the application core the way the window shell does.
//!
//! Nothing here sleeps to wait for an outcome. Waits block on the wake
//! channel or a condition variable with a generous deadline that only turns
//! a hang into a failure. Teardown opens every latch.

#![allow(dead_code)]

use pandite_app::app::{App, Handled, Platform, Startup, WindowState};
use pandite_app::disk::{Disk, StdDisk};
use pandite_app::platform::{
    Clipboard, DialogKind, DialogReply, Dialogs, ImageWriter, WindowControl,
};
use pandite_app::settings::Settings;
use pandite_core::{
    CancelToken, Content, ContentKind, DecodeOptions, DecoderProvider, DrawCmd, DrawList,
    EncodeOptions, EncoderProvider, Error, ImageBuf, ImageMeta, Orientation, PixelFormat, Quad,
    Rect, Registry, Result as CoreResult, Size, Source, TextLayout, TextLine, TextMeasure,
    TextProvider, TextWord, TextureId, TextureSink,
};
use pandite_library::{FileAccess, FileInfo};
use pandite_ui::{Action, CursorShape, InputEvent, Key, Modifiers};
use std::collections::{HashSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

/// Deadline that turns a hang into a failure. Never reached when the code
/// is correct.
pub const WAIT: Duration = Duration::from_secs(30);
/// Bound for calls that must return at once (they take microseconds; a
/// blocked call never returns).
pub const QUICK: Duration = Duration::from_secs(10);

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ----- watchdog ---------------------------------------------------------

/// Aborts the test process when not disarmed before `bound`: a call that
/// should return at once but blocks fails the run instead of hanging it.
pub struct Watchdog {
    done: Arc<(Mutex<bool>, Condvar)>,
}

impl Watchdog {
    pub fn arm(what: &'static str, bound: Duration) -> Self {
        let done = Arc::new((Mutex::new(false), Condvar::new()));
        let flag = Arc::clone(&done);
        thread::spawn(move || {
            let (m, cv) = &*flag;
            let deadline = Instant::now() + bound;
            let mut g = lock(m);
            while !*g {
                let now = Instant::now();
                if now >= deadline {
                    eprintln!("WATCHDOG: {what} blocked for more than {bound:?}");
                    std::process::abort();
                }
                g = cv
                    .wait_timeout(g, deadline - now)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
        });
        Self { done }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let (m, cv) = &*self.done;
        *lock(m) = true;
        cv.notify_all();
    }
}

// ----- temporary folders and images -----------------------------------------

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh folder under the system temp directory, removed on drop.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "pandite-app-{tag}-{}-{}",
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

    pub fn join(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// An RGBA8 image whose pixel at (x, y) is `f(x, y)`.
pub fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> ImageBuf {
    let mut data = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&f(x, y));
        }
    }
    ImageBuf::new(w, h, PixelFormat::Rgba8, data).unwrap()
}

/// Every pixel distinct: red counts up, green marks the row.
pub fn numbered(w: u32, h: u32) -> ImageBuf {
    image(w, h, |x, y| {
        [(y * w + x + 1) as u8 * 7, (y * 40) as u8, 200, 255]
    })
}

pub fn write_png(path: &Path, img: &ImageBuf) {
    let mut out = Vec::new();
    pandite_codecs::PngEncoder
        .encode(img, &EncodeOptions::default(), &mut out)
        .unwrap();
    std::fs::write(path, out).unwrap();
}

/// Decode a file from disk with the full registry.
pub fn decode(path: &Path) -> (ImageBuf, &'static str) {
    let reg = pandite_codecs::default_registry();
    let source = Source::from_path(path).unwrap();
    let id = reg.decoder_for(&source).unwrap().id();
    let content = reg.decode(&source, &DecodeOptions::default()).unwrap();
    (pandite_ops::to_rgba8(content.first_image()).unwrap(), id)
}

pub fn pixels(img: &ImageBuf) -> Vec<[u8; 4]> {
    img.data()
        .chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

// ----- file system with latches ---------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call {
    pub op: &'static str,
    pub name: String,
    pub thread: ThreadId,
}

#[derive(Default)]
struct DiskState {
    calls: Vec<Call>,
    returned: Vec<(&'static str, String)>,
    stall_read: HashSet<String>,
    /// Stalls whole-file reads only; head reads go through.
    stall_full_read: HashSet<String>,
    stall_write: HashSet<String>,
    released: bool,
}

/// `std::fs` with injected stalls (reads and writes of chosen file names
/// block until released, ignoring cancellation like a read stuck in the
/// operating system) and a record of every call with its thread.
#[derive(Default)]
pub struct TestDisk {
    state: Mutex<DiskState>,
    cv: Condvar,
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl TestDisk {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn stall_read(&self, name: &str) {
        lock(&self.state).stall_read.insert(name.into());
    }

    pub fn stall_write(&self, name: &str) {
        lock(&self.state).stall_write.insert(name.into());
    }

    /// Whole-file reads of `name` block until released; head reads (the
    /// size probe) go through.
    pub fn stall_full_read(&self, name: &str) {
        lock(&self.state).stall_full_read.insert(name.into());
    }

    pub fn release_full_read(&self, name: &str) {
        lock(&self.state).stall_full_read.remove(name);
        self.cv.notify_all();
    }

    /// Wait until a call `op` on `name` returned.
    pub fn wait_returned(&self, op: &str, name: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut g = lock(&self.state);
        loop {
            if g.returned.iter().any(|(o, n)| *o == op && n == name) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    pub fn release_all(&self) {
        let mut g = lock(&self.state);
        g.released = true;
        g.stall_read.clear();
        g.stall_full_read.clear();
        g.stall_write.clear();
        drop(g);
        self.cv.notify_all();
    }

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.state).calls.clone()
    }

    /// Reads (full reads and head reads) of `name` that started.
    pub fn reads(&self, name: &str) -> usize {
        lock(&self.state)
            .calls
            .iter()
            .filter(|c| (c.op == "read" || c.op == "read_head") && c.name == name)
            .count()
    }

    /// Reads of `name` that returned.
    pub fn returned(&self, name: &str) -> usize {
        lock(&self.state)
            .returned
            .iter()
            .filter(|(op, n)| (*op == "read" || *op == "read_head") && n == name)
            .count()
    }

    pub fn wait_read_started(&self, name: &str) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut g = lock(&self.state);
        loop {
            if g.calls.iter().any(|c| c.op == "read" && c.name == name) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn enter(&self, op: &'static str, path: &Path, stall: bool) -> String {
        let name = name_of(path);
        let mut g = lock(&self.state);
        g.calls.push(Call {
            op,
            name: name.clone(),
            thread: thread::current().id(),
        });
        self.cv.notify_all();
        loop {
            let set = if op == "write_atomic" {
                &g.stall_write
            } else {
                &g.stall_read
            };
            let full = op == "read" && g.stall_full_read.contains(&name);
            if !stall || g.released || (!set.contains(&name) && !full) {
                break;
            }
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        name
    }

    fn leave(&self, op: &'static str, name: String) {
        lock(&self.state).returned.push((op, name));
        self.cv.notify_all();
    }
}

impl FileAccess for TestDisk {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        let name = self.enter("list_dir", dir, false);
        let r = StdDisk.list_dir(dir);
        self.leave("list_dir", name);
        r
    }

    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        let name = self.enter("file_info", path, false);
        let r = StdDisk.file_info(path);
        self.leave("file_info", name);
        r
    }

    fn read(&self, path: &Path, cancel: &CancelToken) -> CoreResult<Source> {
        let name = self.enter("read", path, true);
        let r = StdDisk.read(path, cancel);
        self.leave("read", name);
        r
    }
}

impl Disk for TestDisk {
    fn read_head(&self, path: &Path, max: usize) -> io::Result<Vec<u8>> {
        let name = self.enter("read_head", path, true);
        let r = StdDisk.read_head(path, max);
        self.leave("read_head", name);
        r
    }

    fn read_small(&self, path: &Path) -> io::Result<Vec<u8>> {
        let name = self.enter("read_small", path, false);
        let r = StdDisk.read_small(path);
        self.leave("read_small", name);
        r
    }

    fn write_atomic(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let name = self.enter("write_atomic", path, true);
        let r = StdDisk.write_atomic(path, data);
        self.leave("write_atomic", name);
        r
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        let name = self.enter("rename", from, false);
        let r = StdDisk.rename(from, to);
        self.leave("rename", name);
        r
    }

    fn create_dir_all(&self, dir: &Path) -> io::Result<()> {
        let name = self.enter("create_dir_all", dir, false);
        let r = StdDisk.create_dir_all(dir);
        self.leave("create_dir_all", name);
        r
    }
}

// ----- platform fakes ------------------------------------------------------------

#[derive(Debug, Default)]
pub struct WindowLog {
    pub titles: Vec<String>,
    pub fullscreen: Vec<bool>,
    pub cursors: Vec<CursorShape>,
    pub ime: Vec<bool>,
    pub client_rects: Vec<Rect>,
    pub redraws: usize,
    pub work_area: Option<Rect>,
}

#[derive(Clone)]
pub struct FakeWindow(pub Arc<Mutex<WindowLog>>);

impl WindowControl for FakeWindow {
    fn request_redraw(&self) {
        lock(&self.0).redraws += 1;
    }
    fn set_title(&self, title: &str) {
        lock(&self.0).titles.push(title.to_string());
    }
    fn set_fullscreen(&self, fullscreen: bool) {
        lock(&self.0).fullscreen.push(fullscreen);
    }
    fn set_cursor(&self, cursor: CursorShape) {
        lock(&self.0).cursors.push(cursor);
    }
    fn set_ime_allowed(&self, allowed: bool) {
        lock(&self.0).ime.push(allowed);
    }
    fn set_client_rect(&self, rect: Rect) {
        lock(&self.0).client_rects.push(rect);
    }
    fn work_area(&self) -> Option<Rect> {
        lock(&self.0).work_area
    }
}

/// The text and image clipboard of the tests. Nothing reaches the real
/// system clipboard.
#[derive(Clone, Default)]
pub struct FakeClipboard {
    pub text: Arc<Mutex<Option<String>>>,
    pub images: Arc<ImageBoard>,
}

impl Clipboard for FakeClipboard {
    fn set_text(&mut self, text: &str) -> Result<(), String> {
        *lock(&self.text) = Some(text.to_string());
        Ok(())
    }
    fn get_text(&mut self) -> Result<String, String> {
        lock(&self.text).clone().ok_or_else(|| "empty".into())
    }
    fn image_writer(&mut self) -> Box<dyn ImageWriter> {
        lock(&self.images.state).writers += 1;
        Box::new(FakeImageWriter(Arc::clone(&self.images)))
    }
}

#[derive(Debug, Default)]
pub struct ImageState {
    /// Writers the core asked for.
    pub writers: usize,
    /// The thread of every `set_image` call, in order.
    pub threads: Vec<ThreadId>,
    /// Images that reached the clipboard.
    pub images: Vec<ImageBuf>,
    /// Calls that ended cancelled.
    pub cancelled: usize,
    /// While set, `set_image` blocks like a hand-over stuck on another
    /// program, ignoring cancellation, until released.
    pub hold: bool,
}

/// The image side of [`FakeClipboard`]: a record of every call with its
/// thread and a latch that holds calls.
#[derive(Default)]
pub struct ImageBoard {
    pub state: Mutex<ImageState>,
    cv: Condvar,
}

impl ImageBoard {
    pub fn hold(&self) {
        lock(&self.state).hold = true;
    }

    pub fn release(&self) {
        lock(&self.state).hold = false;
        self.cv.notify_all();
    }

    /// The images on the clipboard so far, oldest first.
    pub fn images(&self) -> Vec<ImageBuf> {
        lock(&self.state).images.clone()
    }

    pub fn threads(&self) -> Vec<ThreadId> {
        lock(&self.state).threads.clone()
    }

    pub fn writers(&self) -> usize {
        lock(&self.state).writers
    }

    /// Wait until `n` calls started.
    pub fn wait_started(&self, n: usize) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut g = lock(&self.state);
        while g.threads.len() < n {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }
}

struct FakeImageWriter(Arc<ImageBoard>);

impl ImageWriter for FakeImageWriter {
    fn set_image(self: Box<Self>, image: &ImageBuf, cancel: &CancelToken) -> CoreResult<()> {
        let board = &self.0;
        let mut g = lock(&board.state);
        g.threads.push(thread::current().id());
        board.cv.notify_all();
        while g.hold {
            g = board.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
        if cancel.is_cancelled() {
            g.cancelled += 1;
            return Err(Error::Cancelled);
        }
        g.images.push(image.clone());
        board.cv.notify_all();
        Ok(())
    }
}

#[derive(Default)]
pub struct DialogLog {
    pub shown: Vec<DialogKind>,
    pub answers: VecDeque<Vec<PathBuf>>,
}

/// Answers dialogs from a queue at once (an empty answer is a cancel).
#[derive(Clone, Default)]
pub struct FakeDialogs(pub Arc<Mutex<DialogLog>>);

impl Dialogs for FakeDialogs {
    fn show(&mut self, kind: DialogKind, reply: DialogReply) {
        let answer = {
            let mut g = lock(&self.0);
            g.shown.push(kind);
            g.answers.pop_front().unwrap_or_default()
        };
        reply(answer);
    }
}

/// Text measurement without a font: every character is half the size wide.
pub struct Measure;

impl TextMeasure for Measure {
    fn measure(&self, text: &str, size: f32) -> Size {
        Size::new(text.chars().count() as f32 * size * 0.5, size * 1.25)
    }
}

/// Records texture uploads and removals.
#[derive(Default)]
pub struct Sink {
    pub uploads: Vec<TextureId>,
    pub removes: Vec<TextureId>,
}

impl TextureSink for Sink {
    fn upload(&mut self, id: TextureId, _image: &ImageBuf, _alpha_mask: bool) {
        self.uploads.push(id);
    }
    fn remove(&mut self, id: TextureId) {
        self.removes.push(id);
    }
}

/// True when the draw list has a text run containing `needle`.
pub fn has_text(list: &DrawList, needle: &str) -> bool {
    list.cmds
        .iter()
        .any(|c| matches!(c, DrawCmd::Text { text, .. } if text.contains(needle)))
}

// ----- harness ---------------------------------------------------------------------

/// How a harness starts.
pub struct Config {
    pub settings: Settings,
    pub settings_path: Option<PathBuf>,
    pub open: Option<PathBuf>,
    pub window: WindowState,
    pub size_to_image: bool,
    pub work_area: Option<Rect>,
    /// Decoders to use instead of the default registry.
    pub registry: Option<Arc<Registry>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            settings_path: None,
            open: None,
            window: WindowState {
                size: Size::new(1000.0, 800.0),
                ..WindowState::default()
            },
            size_to_image: false,
            work_area: Some(Rect::new(0.0, 0.0, 1920.0, 1040.0)),
            registry: None,
        }
    }
}

/// One content update the core handed to the shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub generation: u64,
    pub full_resolution: bool,
    pub logical: Option<(u32, u32)>,
    /// Pixel size of the content, `None` for "show nothing".
    pub dims: Option<(u32, u32)>,
    /// The first pixel of the content, to tell images apart.
    pub first_pixel: Option<[u8; 4]>,
}

/// The application core with fakes, driven like the shell drives it.
pub struct Harness {
    pub app: Option<App>,
    pub disk: Arc<TestDisk>,
    pub window: Arc<Mutex<WindowLog>>,
    pub clipboard: Arc<Mutex<Option<String>>>,
    /// The image side of the fake clipboard.
    pub images: Arc<ImageBoard>,
    pub dialogs: Arc<Mutex<DialogLog>>,
    pub registry: Arc<Registry>,
    pub now: Duration,
    /// Every content update taken by [`Harness::frame`], oldest first.
    pub updates: Vec<Update>,
    /// Report full-resolution content as presented right after
    /// [`Harness::frame`] takes it, the way the shell does after drawing.
    /// Off: only [`Harness::present`] reports it.
    pub auto_present: bool,
    /// The newest full-resolution content generation taken.
    last_full: Option<u64>,
    wake_rx: Receiver<()>,
}

impl Harness {
    pub fn new(config: Config) -> Self {
        Self::with_disk(config, TestDisk::new())
    }

    pub fn with_disk(config: Config, disk: Arc<TestDisk>) -> Self {
        let (tx, wake_rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let window = Arc::new(Mutex::new(WindowLog {
            work_area: config.work_area,
            ..WindowLog::default()
        }));
        let clipboard = FakeClipboard::default();
        let dialogs = FakeDialogs::default();
        let registry = config
            .registry
            .clone()
            .unwrap_or_else(|| Arc::new(pandite_codecs::default_registry()));
        let platform = Platform {
            disk: Arc::clone(&disk) as Arc<dyn Disk>,
            clipboard: Box::new(clipboard.clone()),
            dialogs: Box::new(dialogs.clone()),
            window: Box::new(FakeWindow(Arc::clone(&window))),
            waker: Arc::new(move || {
                let _ = lock(&tx).send(());
            }),
        };
        let startup = Startup {
            registry: Arc::clone(&registry),
            settings: config.settings,
            settings_path: config.settings_path,
            open: config.open,
            window: config.window,
            size_to_image: config.size_to_image,
        };
        let app = App::new(startup, platform).expect("app starts");
        Self {
            app: Some(app),
            disk,
            window,
            clipboard: clipboard.text,
            images: clipboard.images,
            dialogs: dialogs.0,
            registry,
            now: Duration::from_millis(1),
            updates: Vec::new(),
            auto_present: true,
            last_full: None,
            wake_rx,
        }
    }

    /// Report the newest full-resolution content taken as presented.
    pub fn present(&mut self) {
        let now = self.now;
        if let Some(generation) = self.last_full {
            self.app().content_presented(generation, now);
        }
    }

    pub fn app(&mut self) -> &mut App {
        self.app.as_mut().expect("app running")
    }

    pub fn advance(&mut self, ms: u64) {
        self.now += Duration::from_millis(ms);
    }

    /// Build one frame (and sync textures), as the shell does on redraw.
    pub fn frame(&mut self) -> DrawList {
        let now = self.now;
        let mut sink = Sink::default();
        let app = self.app();
        app.sync_textures(now, &mut sink);
        let mut list = DrawList::new();
        app.frame(now, &Measure, &mut list);
        let mut taken = Vec::new();
        while let Some(update) = app.take_content() {
            let image = update.content.as_ref().map(|c| c.first_image().clone());
            taken.push(Update {
                generation: update.generation,
                full_resolution: update.full_resolution,
                logical: update.logical,
                dims: image.as_ref().map(|i| (i.width(), i.height())),
                first_pixel: image
                    .as_ref()
                    .and_then(|i| i.data().get(..4))
                    .map(|p| [p[0], p[1], p[2], p[3]]),
            });
        }
        if let Some(last) = taken.last() {
            // A newer update replaces older ones; only full content counts.
            self.last_full =
                (last.full_resolution && last.dims.is_some()).then_some(last.generation);
        }
        self.updates.extend(taken);
        if self.auto_present {
            self.present();
        }
        list
    }

    /// Pump and draw frames until `pred` holds, sleeping on the wake
    /// channel in between. Frames collect content updates.
    pub fn wait_frames(&mut self, mut pred: impl FnMut(&App, &[Update]) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            self.frame();
            if pred(self.app.as_ref().unwrap(), &self.updates) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match self.wake_rx.recv_timeout(deadline - now) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// The first frame is on screen: the startup image starts loading.
    pub fn start(&mut self) -> DrawList {
        let list = self.frame();
        let now = self.now;
        self.app().first_frame_shown(now);
        list
    }

    pub fn pump(&mut self) {
        let now = self.now;
        self.app().pump(now);
    }

    /// Pump until `pred` holds, sleeping on the wake channel in between.
    pub fn wait_until(&mut self, mut pred: impl FnMut(&App) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            if pred(self.app.as_ref().unwrap()) {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match self.wake_rx.recv_timeout(deadline - now) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// Draw frames (uploading thumbnails) until `n` thumbnails are
    /// uploaded.
    pub fn wait_thumbnails(&mut self, n: usize) -> bool {
        let deadline = Instant::now() + WAIT;
        loop {
            self.pump();
            self.frame();
            if self.app.as_ref().unwrap().uploaded_thumbnails() >= n {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            match self.wake_rx.recv_timeout(deadline - now) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// Wait until `path` is the image on screen and no work runs.
    pub fn wait_shown(&mut self, path: &Path) -> bool {
        let path = path.to_path_buf();
        self.wait_until(|a| a.shown_path() == Some(path.as_path()) && !a.is_busy())
    }

    pub fn wait_idle(&mut self) -> bool {
        self.wait_until(|a| !a.is_busy())
    }

    pub fn input(&mut self, event: InputEvent) {
        let now = self.now;
        self.app().input(event, now);
    }

    pub fn key(&mut self, key: Key, mods: Modifiers) {
        self.input(InputEvent::KeyDown {
            key,
            mods,
            repeat: false,
        });
        self.input(InputEvent::KeyUp { key, mods });
    }

    pub fn act(&mut self, action: Action) -> Handled {
        let now = self.now;
        self.app().handle_action(action, now)
    }

    pub fn answer_dialog(&mut self, paths: Vec<PathBuf>) {
        lock(&self.dialogs).answers.push_back(paths);
    }

    pub fn clipboard(&self) -> Option<String> {
        lock(&self.clipboard).clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.disk.release_all();
        self.images.release();
        if let Some(app) = self.app.take() {
            let _ = app.shutdown(Duration::from_millis(500));
        }
    }
}

/// A folder with `names` as small distinct PNG images (`w` x `h`).
pub fn folder(tag: &str, names: &[&str], w: u32, h: u32) -> (TempDir, Vec<PathBuf>) {
    let dir = TempDir::new(tag);
    let paths = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let p = dir.join(n);
            let shade = (i as u8).wrapping_mul(60);
            write_png(
                &p,
                &image(w, h, |x, y| [shade, (x * 9) as u8, (y * 9) as u8, 255]),
            );
            p
        })
        .collect();
    (dir, paths)
}

// ----- text recognizer -------------------------------------------------------------

/// One call into [`FakeText`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextCall {
    pub op: &'static str,
    pub thread: ThreadId,
    /// The image size `recognize` was given (0 x 0 for `available`).
    pub dims: (u32, u32),
}

#[derive(Default)]
struct FakeTextState {
    available: bool,
    hold: bool,
    /// Words per line: text and box as fractions of the image size, so
    /// the layout fits any image it is given.
    lines: Vec<Vec<(String, [f32; 4])>>,
    calls: Vec<TextCall>,
    started: usize,
    cancelled: usize,
    finished: usize,
}

/// A text recognizer for tests. It returns a fixed layout scaled to the
/// image it reads, records every call with its thread, and with
/// [`FakeText::hold`] blocks inside `recognize` until released, returning
/// `Cancelled` as soon as the token is set.
#[derive(Default)]
pub struct FakeText {
    state: Mutex<FakeTextState>,
    cv: Condvar,
}

impl FakeText {
    /// A working recognizer that finds `lines`: words with their boxes
    /// as fractions `[x, y, w, h]` of the image size.
    pub fn new(lines: &[&[(&str, [f32; 4])]]) -> Arc<Self> {
        let fake = Self::default();
        {
            let mut g = lock(&fake.state);
            g.available = true;
            g.lines = lines
                .iter()
                .map(|l| l.iter().map(|(t, r)| (t.to_string(), *r)).collect())
                .collect();
        }
        Arc::new(fake)
    }

    /// A recognizer that reports itself missing (no OCR language).
    pub fn unavailable() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The default registry plus this recognizer and `decoders`.
    pub fn registry(self: &Arc<Self>, decoders: &[Arc<dyn DecoderProvider>]) -> Arc<Registry> {
        let mut reg = pandite_codecs::default_registry();
        for d in decoders {
            reg.add_decoder(Arc::clone(d));
        }
        reg.add_text_provider(Arc::clone(self) as Arc<dyn TextProvider>);
        Arc::new(reg)
    }

    pub fn hold(&self) {
        lock(&self.state).hold = true;
    }

    pub fn release(&self) {
        lock(&self.state).hold = false;
        self.cv.notify_all();
    }

    pub fn calls(&self) -> Vec<TextCall> {
        lock(&self.state).calls.clone()
    }

    /// `recognize` calls so far.
    pub fn recognitions(&self) -> Vec<TextCall> {
        self.calls()
            .into_iter()
            .filter(|c| c.op == "recognize")
            .collect()
    }

    fn wait(&self, done: impl Fn(&FakeTextState) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut g = lock(&self.state);
        while !done(&g) {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    /// Wait until `n` recognitions started.
    pub fn wait_started(&self, n: usize) -> bool {
        self.wait(|s| s.started >= n)
    }

    /// Wait until `n` held recognitions stopped because they were
    /// cancelled.
    pub fn wait_cancelled(&self, n: usize) -> bool {
        self.wait(|s| s.cancelled >= n)
    }

    /// Recognitions that returned a layout.
    pub fn finished(&self) -> usize {
        lock(&self.state).finished
    }

    fn record(&self, op: &'static str, dims: (u32, u32)) {
        lock(&self.state).calls.push(TextCall {
            op,
            thread: thread::current().id(),
            dims,
        });
        self.cv.notify_all();
    }
}

impl TextProvider for FakeText {
    fn id(&self) -> &'static str {
        "fake-text"
    }

    fn name(&self) -> &'static str {
        "Test recognizer"
    }

    fn available(&self) -> bool {
        self.record("available", (0, 0));
        lock(&self.state).available
    }

    fn recognize(&self, image: &ImageBuf, cancel: &CancelToken) -> CoreResult<TextLayout> {
        let (w, h) = (image.width(), image.height());
        self.record("recognize", (w, h));
        let mut g = lock(&self.state);
        if !g.available {
            return Err(Error::Unavailable("test recognizer is off".into()));
        }
        g.started += 1;
        self.cv.notify_all();
        while g.hold {
            if cancel.is_cancelled() {
                g.cancelled += 1;
                drop(g);
                self.cv.notify_all();
                return Err(Error::Cancelled);
            }
            g = self
                .cv
                .wait_timeout(g, Duration::from_millis(1))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        let (fw, fh) = (w as f32, h as f32);
        let lines = g
            .lines
            .iter()
            .map(|l| TextLine {
                words: l
                    .iter()
                    .map(|(text, [x, y, bw, bh])| TextWord {
                        text: text.clone(),
                        quad: Quad::from_rect(Rect::new(x * fw, y * fh, bw * fw, bh * fh)),
                    })
                    .collect(),
            })
            .collect();
        g.finished += 1;
        drop(g);
        self.cv.notify_all();
        Ok(TextLayout {
            width: w,
            height: h,
            lines,
        })
    }
}

// ----- staged decoder ------------------------------------------------------------

/// The full image of a staged test file: a pattern every pixel of which
/// differs from its neighbors, so a crop, rotation or downscale shows.
pub fn staged_pattern(w: u32, h: u32) -> ImageBuf {
    image(w, h, |x, y| {
        [
            (x % 251) as u8,
            (y % 241) as u8,
            ((x * 7 + y * 13) % 256) as u8,
            255,
        ]
    })
}

/// Write a staged test file: "STAGED <w> <h>".
pub fn write_staged(path: &Path, w: u32, h: u32) {
    std::fs::write(path, format!("STAGED {w} {h}")).unwrap();
}

#[derive(Default)]
struct GateState {
    hold_full: HashSet<String>,
    hold_reduced: HashSet<String>,
    started: Vec<(String, bool)>,
    cancelled: Vec<String>,
    finished: Vec<(String, bool)>,
}

/// Decodes "STAGED <w> <h>" files (extension `stg`): a cheap reduced
/// decode for a target size and a full decode of [`staged_pattern`], each
/// of which waits on a latch the test opens. A held decode stops with
/// `Cancelled` as soon as its token is set, and records that.
#[derive(Default)]
pub struct GatedDecoder {
    state: Mutex<GateState>,
    cv: Condvar,
}

impl GatedDecoder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The default registry plus this decoder.
    pub fn registry(self: &Arc<Self>) -> Arc<Registry> {
        let mut reg = pandite_codecs::default_registry();
        reg.add_decoder(Arc::clone(self) as Arc<dyn DecoderProvider>);
        Arc::new(reg)
    }

    pub fn hold_full(&self, name: &str) {
        lock(&self.state).hold_full.insert(name.into());
    }

    pub fn release_full(&self, name: &str) {
        lock(&self.state).hold_full.remove(name);
        self.cv.notify_all();
    }

    pub fn hold_reduced(&self, name: &str) {
        lock(&self.state).hold_reduced.insert(name.into());
    }

    pub fn release_reduced(&self, name: &str) {
        lock(&self.state).hold_reduced.remove(name);
        self.cv.notify_all();
    }

    pub fn release_all(&self) {
        let mut g = lock(&self.state);
        g.hold_full.clear();
        g.hold_reduced.clear();
        drop(g);
        self.cv.notify_all();
    }

    fn wait(&self, done: impl Fn(&GateState) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut g = lock(&self.state);
        while !done(&g) {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            g = self
                .cv
                .wait_timeout(g, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        true
    }

    /// Wait until a full (`full`) or reduced decode of `name` started.
    pub fn wait_started(&self, name: &str, full: bool) -> bool {
        self.wait(|s| s.started.iter().any(|(n, f)| n == name && *f == full))
    }

    /// Wait until a held full decode of `name` stopped because it was
    /// cancelled.
    pub fn wait_cancelled(&self, name: &str) -> bool {
        self.wait(|s| s.cancelled.iter().any(|n| n == name))
    }

    /// Wait until a full (`full`) or reduced decode of `name` returned an
    /// image.
    pub fn wait_finished(&self, name: &str, full: bool) -> bool {
        self.wait(|s| s.finished.iter().any(|(n, f)| n == name && *f == full))
    }

    /// Full decodes of `name` that finished with an image.
    pub fn full_finished(&self, name: &str) -> usize {
        lock(&self.state)
            .finished
            .iter()
            .filter(|(n, f)| n == name && *f)
            .count()
    }
}

fn staged_size(source: &Source) -> CoreResult<(u32, u32)> {
    let text = std::str::from_utf8(source.bytes()).map_err(|_| Error::Corrupt("utf8".into()))?;
    let mut parts = text.split_whitespace().skip(1);
    let mut num = || -> CoreResult<u32> {
        parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| Error::Corrupt("size".into()))
    };
    Ok((num()?, num()?))
}

impl DecoderProvider for GatedDecoder {
    fn id(&self) -> &'static str {
        "staged"
    }

    fn name(&self) -> &'static str {
        "Staged test image"
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["stg"]
    }

    fn sniff(&self, header: &[u8]) -> bool {
        header.starts_with(b"STAGED")
    }

    fn probe(&self, source: &Source) -> CoreResult<ImageMeta> {
        let (width, height) = staged_size(source)?;
        Ok(ImageMeta {
            width,
            height,
            format: "staged",
            kind: ContentKind::Still,
            frame_count: 1,
            has_alpha: false,
            orientation: Orientation::Normal,
        })
    }

    fn reduced_decode(&self, _source: &Source) -> bool {
        true
    }

    fn decode(&self, source: &Source, options: &DecodeOptions) -> CoreResult<Content> {
        let name = source
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let (w, h) = staged_size(source)?;
        let full = options.target_size.is_none();
        let mut g = lock(&self.state);
        g.started.push((name.clone(), full));
        self.cv.notify_all();
        loop {
            let held = if full {
                g.hold_full.contains(&name)
            } else {
                g.hold_reduced.contains(&name)
            };
            if !held {
                break;
            }
            if options.cancel.is_cancelled() {
                if full {
                    g.cancelled.push(name.clone());
                }
                drop(g);
                self.cv.notify_all();
                return Err(Error::Cancelled);
            }
            g = self
                .cv
                .wait_timeout(g, Duration::from_millis(1))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        drop(g);
        let image = match options.target_size {
            None => staged_pattern(w, h),
            Some((tw, th)) => {
                // Cover the target: the smallest scale that reaches it on
                // one axis, sampling the full pattern.
                let s = (f64::from(tw) / f64::from(w))
                    .max(f64::from(th) / f64::from(h))
                    .min(1.0);
                let (rw, rh) = (
                    ((f64::from(w) * s).ceil() as u32).max(1),
                    ((f64::from(h) * s).ceil() as u32).max(1),
                );
                let full = staged_pattern(w, h);
                image(rw, rh, |x, y| {
                    let sx = (x * w / rw).min(w - 1);
                    let sy = (y * h / rh).min(h - 1);
                    let i = ((sy * w + sx) * 4) as usize;
                    let p = &full.data()[i..i + 4];
                    [p[0], p[1], p[2], p[3]]
                })
            }
        };
        lock(&self.state).finished.push((name, full));
        self.cv.notify_all();
        Ok(Content::Still(image))
    }
}

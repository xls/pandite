//! The application core: every behavior of pandite as a state machine.
//!
//! [`App`] owns the interface ([`Ui`]), the folder [`Library`], the edit
//! stack of the image on screen, the settings and the bookkeeping of
//! background jobs. The shell feeds it input events and wakes; it answers
//! with draw lists, content to upload and calls on the [`WindowControl`].
//!
//! Threading: every method runs on the event-loop thread and returns at
//! once. File reads, folder listings, decodes, encodes, writes, bulk work,
//! image copies to the clipboard and text recognition run on library
//! workers or on the job threads in [`jobs`], which report through a
//! channel and the [`Waker`]. A job stuck in a file system call or a
//! clipboard hand-over is never waited for, also not when the app quits.

mod jobs;
mod text;
mod thumbs;

use crate::disk::Disk;
use crate::edit::{self, Edit, EditStack, Undo};
use crate::platform::{Clipboard, DialogKind, Dialogs, Filter, Waker, WindowControl};
use crate::settings::{self, Settings, TextRecognition, ThemeChoice, WindowPlacement};
use jobs::{Copied, CopyJob, Outbox};
use pandite_core::{
    CancelToken, Color, Content, DrawList, EncodeOptions, Error, ImageBuf, PixelFormat, Point,
    Rect, Registry, Size, TextLayout, TextMeasure, TextureSink, ViewState,
};
use pandite_library::{Event, Fetch, FileAccess, KeptThumbnails, Library, LibraryConfig, Reduced};
use pandite_ui::{
    Action, ConfirmRequest, CursorShape, EncoderInfo, ImageInfo, InputEvent, Loading, Progress,
    TextView, ThemeKind, ThumbState, Ui, UiContext, ZOOM_STEP,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;
use text::{TextKey, TextState, TEXT_DELAY};
use thumbs::Thumbs;

/// Messages from job threads to the core.
pub(crate) enum Msg {
    /// A native dialog closed; `paths` is empty when it was cancelled.
    Dialog { id: u64, paths: Vec<PathBuf> },
    /// An edit job finished.
    EditDone {
        image: u64,
        edit: Edit,
        result: pandite_core::Result<ImageBuf>,
    },
    /// A Save As job finished.
    Saved { id: u64, result: Result<(), String> },
    /// The size of the file opened at startup.
    Probed { path: PathBuf, size: (u32, u32) },
    /// A folder or unknown path was turned into an image path.
    Resolved {
        original: PathBuf,
        found: Result<PathBuf, String>,
    },
    /// A bulk job made progress.
    BulkProgress {
        id: u64,
        done: usize,
        total: usize,
        name: String,
    },
    /// A bulk job ended.
    BulkDone {
        id: u64,
        summary: String,
        destination: PathBuf,
    },
    /// Image copy `id` ended.
    Copied { id: u64, outcome: Copied },
    /// Text recognition job `ticket` of shown image `image` ended.
    TextDone {
        ticket: u64,
        image: u64,
        key: TextKey,
        result: pandite_core::Result<TextLayout>,
    },
}

/// What the shell must put on screen as content.
#[derive(Clone, Debug)]
pub struct ContentUpdate {
    /// Increases with every update; a later update replaces an earlier one.
    pub generation: u64,
    /// The new content, or `None` to show nothing.
    pub content: Option<Content>,
    /// True when the content is the image at full resolution; false for a
    /// reduced stage shown while the full image decodes.
    pub full_resolution: bool,
    /// For a reduced stage: the full image's size. The renderer draws the
    /// reduced content as if it had this size, so the full image replaces
    /// it without moving the view.
    pub logical: Option<(u32, u32)>,
}

/// What the shell needs to draw one frame besides the draw list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameOut {
    /// How the content is placed.
    pub view: ViewState,
    /// Where the content may draw (the window minus the gallery strip).
    pub viewport: Rect,
    /// Window background color.
    pub background: Color,
}

/// The window as the shell found it when the core started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowState {
    /// Client size in physical pixels.
    pub size: Size,
    /// DPI scale factor.
    pub scale: f32,
    /// The operating system theme, when known.
    pub system_theme: Option<ThemeKind>,
    /// Client rectangle in physical pixels of the virtual desktop, when
    /// known (outer position plus client size).
    pub placement: Option<Rect>,
    /// The window starts maximized.
    pub maximized: bool,
    /// The window starts in full screen.
    pub fullscreen: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            size: Size::new(1280.0, 800.0),
            scale: 1.0,
            system_theme: None,
            placement: None,
            maximized: false,
            fullscreen: false,
        }
    }
}

/// Everything the core needs from the platform.
pub struct Platform {
    /// File system access for the library and the jobs.
    pub disk: Arc<dyn Disk>,
    /// The system clipboard.
    pub clipboard: Box<dyn Clipboard>,
    /// Native file dialogs.
    pub dialogs: Box<dyn Dialogs>,
    /// The window.
    pub window: Box<dyn WindowControl>,
    /// Wakes the event loop from any thread.
    pub waker: Waker,
}

/// How the core starts.
pub struct Startup {
    /// Decoders and encoders.
    pub registry: Arc<Registry>,
    /// Loaded settings.
    pub settings: Settings,
    /// Where to save settings; `None` never writes them (smoke runs).
    pub settings_path: Option<PathBuf>,
    /// The image to open once the first frame is on screen.
    pub open: Option<PathBuf>,
    /// The window.
    pub window: WindowState,
    /// Size the window to the first image when its size is known.
    pub size_to_image: bool,
    /// Thumbnails of the last folder from the previous session
    /// ([`ShutdownReport::thumbnails`]). Used when this session opens the
    /// same folder; dropped otherwise.
    pub thumbnails: Option<KeptThumbnails>,
}

/// What [`App::handle_action`] did with an action.
#[derive(Clone, Debug, PartialEq)]
pub enum Handled {
    /// The action ran (or started a job or dialog).
    Done,
    /// The action does not apply right now (nothing open, nothing to undo).
    Ignored,
    /// Feed this clipboard text back as [`InputEvent::Text`].
    Paste(String),
}

/// What [`App::shutdown`] did.
#[derive(Clone, Debug)]
pub struct ShutdownReport {
    /// `Some(true)` when the settings were written, `Some(false)` when the
    /// write failed, `None` when it was not attempted or did not finish in
    /// time.
    pub settings_saved: Option<bool>,
    /// The thumbnails of the last folder, within the thumbnail cache
    /// limit, for the next session ([`Startup::thumbnails`]). Drop them to
    /// free the memory.
    pub thumbnails: Option<KeptThumbnails>,
}

/// The file the user wants to see.
#[derive(Clone, Debug, PartialEq)]
struct Target {
    path: PathBuf,
    index: usize,
}

/// A load in progress.
#[derive(Clone, Debug)]
struct LoadState {
    path: PathBuf,
    name: String,
    since: Duration,
    cloud: bool,
    /// The file named on the command line before any image was shown.
    startup: bool,
}

/// The image on screen.
struct Shown {
    path: PathBuf,
    name: String,
    /// What the renderer shows: the full decode, or a reduced stand-in.
    content: Arc<Content>,
    base: ImageBuf,
    edits: EditStack,
    id: u64,
    told_first_frame: bool,
    /// The full image's size while `content` is a reduced stand-in. Edits
    /// and saves wait until the full image replaces it.
    standin: Option<(u32, u32)>,
    /// The content generation that first carried this image at full
    /// resolution.
    full_gen: Option<u64>,
    /// When a frame with that content was first presented.
    presented_at: Option<Duration>,
    /// The text read from `base`, once recognition ended (an empty layout
    /// when it found nothing or failed).
    text: Option<ShownText>,
}

/// Recognized text attached to the image on screen.
struct ShownText {
    /// Names this layout on this image for the interface.
    id: u64,
    layout: Arc<TextLayout>,
}

impl Shown {
    fn animated(&self) -> bool {
        matches!(*self.content, Content::Animation(_))
    }

    fn current(&self) -> &ImageBuf {
        self.edits.current(&self.base)
    }

    /// The size the view works with: the full image's, also while a
    /// reduced stand-in is on screen.
    fn display_dims(&self) -> (u32, u32) {
        match self.standin {
            Some(dims) => dims,
            None => {
                let image = self.current();
                (image.width(), image.height())
            }
        }
    }

    /// The update that puts this image on screen.
    fn update(&self) -> (Content, Option<(u32, u32)>) {
        if self.edits.is_empty() || self.standin.is_some() {
            ((*self.content).clone(), self.standin)
        } else {
            (Content::Still(self.current().clone()), None)
        }
    }
}

/// Why a dialog was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DialogPurpose {
    Open,
    SaveAs,
    BulkFiles,
    BulkFolder,
    BulkDestination,
}

/// A save waiting for its confirmation or for running edits.
#[derive(Clone, Debug)]
struct PendingSave {
    target: PathBuf,
    encoder: String,
}

/// A save on a worker.
#[derive(Clone, Debug)]
struct Saving {
    id: u64,
    target: PathBuf,
    name: String,
    image: u64,
    overwrite: bool,
    animated: bool,
}

/// A bulk job on a worker.
#[derive(Clone, Debug)]
struct BulkRun {
    id: u64,
    cancel: CancelToken,
    done: usize,
    total: usize,
    label: String,
}

/// An image copy on a worker.
#[derive(Clone, Debug)]
struct CopyRun {
    id: u64,
    /// The shown image it copies.
    image: u64,
    /// The content generation on screen when it started: the picture.
    picture: u64,
    cancel: CancelToken,
}

/// Copies of at least this many pixels say so when they start; smaller
/// ones finish before a toast could be read.
const COPY_NOTICE_PIXELS: u64 = 12_000_000;

/// Largest image (in bytes of pixel data) the edit commands accept.
const MAX_EDIT_BYTES: usize = 1 << 30;

/// How long after the first frame the window may still size itself to
/// the first image. A slow file (a cloud download) arrives after the user
/// arranged the window; then it stays as it is.
const AUTO_SIZE_WINDOW: Duration = Duration::from_secs(3);

/// First-open window sizing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AutoSize {
    Off,
    Waiting,
    Done,
}

/// Everything except the interface, so a [`UiContext`] can borrow it while
/// the [`Ui`] is borrowed mutably.
struct State {
    registry: Arc<Registry>,
    library: Library,
    disk: Arc<dyn Disk>,
    clipboard: Box<dyn Clipboard>,
    dialogs: Box<dyn Dialogs>,
    window: Box<dyn WindowControl>,
    out: Outbox,
    inbox: Receiver<Msg>,

    settings: Settings,
    settings_path: Option<PathBuf>,
    encoders: Vec<EncoderInfo<'static>>,

    window_size: Size,
    scale: f32,
    system_theme: Option<ThemeKind>,
    fullscreen: bool,
    maximized: bool,
    normal: Option<Rect>,
    auto_size: AutoSize,
    first_frame_at: Duration,
    clock: Duration,
    startup_path: Option<PathBuf>,
    pending_open: Option<PathBuf>,
    started: bool,
    /// The path handed to the library last and the clock time it was.
    load_started: Option<(PathBuf, Duration)>,
    /// The full image replaced its stand-in: start the edits that waited.
    resume_edits: bool,
    /// The full image replaced its stand-in: start the save that waited.
    resume_save: bool,

    target: Option<Target>,
    count: usize,
    folder: Option<PathBuf>,
    shown: Option<Shown>,
    loading: Option<LoadState>,
    view: ViewState,
    content_rect: Rect,
    next_image_id: u64,
    content_gen: u64,
    content_out: Option<ContentUpdate>,
    notice: Option<String>,
    thumbs: Thumbs,
    text: TextState,

    next_id: u64,
    dialogs_open: HashMap<u64, DialogPurpose>,
    confirms: HashMap<u64, PendingSave>,
    pending_save: Option<PendingSave>,
    saving: Option<Saving>,
    bulk: Option<BulkRun>,
    /// The image copy on a worker.
    copying: Option<CopyRun>,
    /// The shown image a copy waits for: its full resolution, its edits
    /// or the copy that runs.
    copy_wanted: Option<u64>,
    /// The content generation the renderer shows and the animation frame
    /// of it on screen.
    frame_on_screen: Option<(u64, usize)>,

    title: String,
    busy_text: String,
    bulk_text: String,
    cursor: Option<CursorShape>,
    ime: bool,
    exit: bool,
    dirty: bool,
}

/// The application core. See the module documentation.
pub struct App {
    ui: Ui,
    st: State,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The path as people write it: without the `\\?\` prefix Windows uses
/// for long paths.
pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        text.into_owned()
    }
}

/// True when two paths name the same file, compared without touching the
/// disk: case-insensitive on Windows and macOS, exact elsewhere.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let (a, b) = (display_path(a), display_path(b));
    if cfg!(any(windows, target_os = "macos")) {
        let norm = |s: &str| s.replace('/', std::path::MAIN_SEPARATOR_STR).to_lowercase();
        norm(&a) == norm(&b)
    } else {
        a == b
    }
}

impl App {
    /// Start the core. Spawns the library workers; reads nothing. The
    /// startup image (if any) opens on [`App::first_frame_shown`], after
    /// the window shows its first frame.
    pub fn new(startup: Startup, platform: Platform) -> pandite_core::Result<App> {
        let Startup {
            registry,
            settings,
            settings_path,
            open,
            window,
            size_to_image,
            thumbnails,
        } = startup;
        let config = LibraryConfig {
            read_ahead: settings.read_ahead as usize,
            wrap_around: settings.wrap_around,
            full_budget: (settings.image_cache_mb as usize) << 20,
            thumb_budget: (settings.thumbnail_cache_mb as usize) << 20,
            thumb_size: 256,
            shutdown_grace: Duration::from_millis(100),
            downscale: Arc::new(|image: &ImageBuf, max: u32| {
                let small = pandite_ops::thumbnail(image, max);
                // Thumbnails of deep images become 8-bit: a quarter of the
                // memory, and the strip cannot show more anyway.
                pandite_ops::to_rgba8(&small).unwrap_or(small)
            }),
            files: Arc::clone(&platform.disk) as Arc<dyn FileAccess>,
            // Staged loading: a quick reduced image first, full resolution
            // for the image on screen only, read-ahead at window size.
            screen_size: Some(screen_size(window.size)),
            kept_thumbnails: thumbnails,
            ..LibraryConfig::default()
        };
        let library = Library::new(Arc::clone(&registry), config, Arc::clone(&platform.waker))?;
        let (tx, inbox): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
        let encoders = registry
            .encoders()
            .iter()
            .map(|e| EncoderInfo {
                id: e.id(),
                name: short_format_name(e.name()),
                has_quality: e.has_quality(),
            })
            .collect();
        let auto_size = if open.is_some() && size_to_image {
            AutoSize::Waiting
        } else {
            AutoSize::Off
        };
        let text = TextState::new(
            registry.text_providers().to_vec(),
            settings.ocr == TextRecognition::Auto,
        );
        let st = State {
            registry,
            library,
            disk: platform.disk,
            clipboard: platform.clipboard,
            dialogs: platform.dialogs,
            window: platform.window,
            out: Outbox::new(tx, platform.waker),
            inbox,
            settings,
            settings_path,
            encoders,
            window_size: window.size,
            scale: window.scale.max(0.25),
            system_theme: window.system_theme,
            fullscreen: window.fullscreen,
            maximized: window.maximized,
            normal: window.placement,
            auto_size,
            first_frame_at: Duration::ZERO,
            clock: Duration::ZERO,
            startup_path: open.clone(),
            pending_open: open.clone(),
            started: false,
            load_started: None,
            resume_edits: false,
            resume_save: false,
            target: None,
            count: 0,
            folder: open
                .as_deref()
                .and_then(Path::parent)
                .map(Path::to_path_buf),
            shown: None,
            // The loading screen shows from the first frame, before any
            // byte of the file is read.
            loading: open.as_ref().map(|p| LoadState {
                path: p.clone(),
                name: file_name(p),
                since: Duration::ZERO,
                cloud: false,
                startup: true,
            }),
            view: ViewState::default(),
            content_rect: Rect::new(0.0, 0.0, window.size.w, window.size.h),
            next_image_id: 1,
            content_gen: 0,
            content_out: None,
            notice: None,
            thumbs: Thumbs::default(),
            text,
            next_id: 1,
            dialogs_open: HashMap::new(),
            confirms: HashMap::new(),
            pending_save: None,
            saving: None,
            bulk: None,
            copying: None,
            copy_wanted: None,
            frame_on_screen: None,
            title: String::new(),
            busy_text: String::new(),
            bulk_text: String::new(),
            cursor: None,
            ime: false,
            exit: false,
            dirty: true,
        };
        let mut app = App { ui: Ui::new(), st };
        app.st.update_title();
        Ok(app)
    }

    // ----- queries ----------------------------------------------------------

    /// The interface state machine.
    pub fn ui(&self) -> &Ui {
        &self.ui
    }

    /// The folder library.
    pub fn library(&self) -> &Library {
        &self.st.library
    }

    /// Record whether this computer has a discrete GPU, for the next start.
    pub fn set_gpu_discrete_seen(&mut self, seen: bool) {
        self.st.settings.gpu_discrete_seen = seen;
    }

    /// Current settings, with the window placement as it is now.
    pub fn settings(&self) -> Settings {
        self.st.final_settings()
    }

    /// The current view (display mode, zoom and pan).
    pub fn view(&self) -> ViewState {
        self.st.view
    }

    /// True in full screen.
    pub fn fullscreen(&self) -> bool {
        self.st.fullscreen
    }

    /// The window title.
    pub fn title(&self) -> &str {
        &self.st.title
    }

    /// The path of the image on screen.
    pub fn shown_path(&self) -> Option<&Path> {
        self.st.shown.as_ref().map(|s| s.path.as_path())
    }

    /// True while the image on screen is a reduced stand-in and its full
    /// resolution is still decoding.
    pub fn shows_reduced(&self) -> bool {
        self.st.shown.as_ref().is_some_and(|s| s.standin.is_some())
    }

    /// The size of the image on screen as the view sees it: the full
    /// image's size, also while a reduced stand-in is shown.
    pub fn display_dims(&self) -> Option<(u32, u32)> {
        self.st.shown.as_ref().map(Shown::display_dims)
    }

    /// The image on screen after edits (the first frame of an animation).
    /// While a reduced stand-in is shown, the stand-in.
    pub fn shown_image(&self) -> Option<&ImageBuf> {
        self.st.shown.as_ref().map(Shown::current)
    }

    /// True when the image on screen has edits that are not saved.
    pub fn is_edited(&self) -> bool {
        self.st.shown.as_ref().is_some_and(|s| s.edits.is_edited())
    }

    /// The text recognized in the image on screen, once recognition ended
    /// (empty when it found none). Word outlines are in pixels of the
    /// full-resolution image as decoded, before view edits.
    pub fn shown_text(&self) -> Option<&TextLayout> {
        self.st.shown.as_ref()?.text.as_ref().map(|t| &*t.layout)
    }

    /// True while text recognition runs for the image on screen.
    pub fn reading_text(&self) -> bool {
        self.st.text.job.is_some()
    }

    /// True while text recognition of the image on screen runs or waits
    /// for its start time.
    pub fn text_pending(&self) -> bool {
        self.st.text.job.is_some() || self.st.text_due().is_some()
    }

    /// True while edits or a save run in the background.
    pub fn is_busy(&self) -> bool {
        self.st.saving.is_some()
            || self.st.pending_save.is_some()
            || self.st.shown.as_ref().is_some_and(|s| s.edits.is_busy())
    }

    /// True while an image copy runs or waits (for the full resolution,
    /// for edits, or for the copy before it).
    pub fn copying_image(&self) -> bool {
        self.st.copying.is_some() || self.st.copy_wanted.is_some()
    }

    /// True while a bulk job runs.
    pub fn bulk_running(&self) -> bool {
        self.st.bulk.is_some()
    }

    /// The path the user wants to see (shown or loading).
    pub fn target_path(&self) -> Option<&Path> {
        self.st.target.as_ref().map(|t| t.path.as_path())
    }

    /// The folder index of the target image.
    pub fn index(&self) -> Option<usize> {
        self.st.target.as_ref().map(|t| t.index)
    }

    /// Number of images in the folder.
    pub fn count(&self) -> usize {
        self.st.count
    }

    /// When the library was asked for `path` last, on the clock passed to
    /// the core (the start of the load the timing report measures).
    pub fn load_started_at(&self, path: &Path) -> Option<Duration> {
        self.st
            .load_started
            .as_ref()
            .filter(|(p, _)| p == path)
            .map(|(_, at)| *at)
    }

    /// The file being loaded, when a load runs.
    pub fn loading_path(&self) -> Option<&Path> {
        self.st.loading.as_ref().map(|l| l.path.as_path())
    }

    /// The persistent message, when one shows.
    pub fn notice(&self) -> Option<&str> {
        self.st.notice.as_deref()
    }

    /// Gallery thumbnail states.
    pub fn thumbnails(&self) -> &[ThumbState] {
        self.st.thumbs.states()
    }

    /// Number of thumbnail textures uploaded to the renderer.
    pub fn uploaded_thumbnails(&self) -> usize {
        self.st.thumbs.uploaded()
    }

    /// True once the core asked to close.
    pub fn exit_requested(&self) -> bool {
        self.st.exit
    }

    /// The interface theme in effect.
    pub fn theme(&self) -> ThemeKind {
        self.st.theme()
    }

    /// The window background color.
    pub fn background(&self) -> Color {
        let ctx = UiContext {
            theme: self.st.theme(),
            ..UiContext::default()
        };
        Ui::clear_color(&ctx)
    }

    /// Something changed since the last frame: draw one.
    pub fn needs_frame(&self) -> bool {
        self.st.dirty || self.ui.wants_redraw()
    }

    /// When the next timed frame is due, on the clock passed as `now`.
    /// Text recognition of the image on screen starts at such a frame.
    pub fn next_frame_at(&self) -> Option<Duration> {
        let ui = self.ui.next_redraw_at();
        match (ui, self.st.text_due()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // ----- events from the shell -------------------------------------------

    /// The first frame is on screen: start opening the startup image.
    pub fn first_frame_shown(&mut self, now: Duration) {
        if self.st.started {
            return;
        }
        self.st.started = true;
        self.st.first_frame_at = now;
        self.st.clock = now;
        if let Some(path) = self.st.pending_open.take() {
            self.st.open_path(path, now, true);
        }
    }

    /// A frame showing content `generation` (from
    /// [`ContentUpdate::generation`]) at full resolution was presented.
    /// Call after every such frame; repeats cost nothing. Text recognition
    /// of the image waits for this.
    pub fn content_presented(&mut self, generation: u64, now: Duration) {
        if let Some(shown) = self.st.shown.as_mut() {
            let full = shown.standin.is_none() && shown.full_gen.is_some_and(|g| generation >= g);
            if full && shown.presented_at.is_none() {
                shown.presented_at = Some(now);
            }
        }
        self.st.poll_text(now);
    }

    /// The renderer shows content `generation` (from
    /// [`ContentUpdate::generation`]) with animation frame `frame` (0 for a
    /// still). Call after each frame; an image copy takes the frame on
    /// screen.
    pub fn frame_on_screen(&mut self, generation: u64, frame: usize) {
        self.st.frame_on_screen = Some((generation, frame));
    }

    /// Open a file or folder (dropped on the window, for example).
    pub fn open(&mut self, path: PathBuf, now: Duration) {
        self.st.open_path(path, now, false);
        self.st.dirty = true;
        self.st.update_title();
        self.st.poll_text(now);
        self.after_change();
    }

    /// Feed one input event.
    pub fn input(&mut self, event: InputEvent, now: Duration) {
        let mut queue = vec![event];
        while let Some(event) = queue.pop() {
            self.st.refresh_text();
            let (actions, rect) = {
                let ctx = context(&self.st, &self.ui, now);
                let rect = self.ui.content_rect(&ctx);
                (self.ui.handle_event(event, &ctx), rect)
            };
            self.st.content_rect = rect;
            for action in actions {
                if let Handled::Paste(text) = self.handle_action(action, now) {
                    queue.push(InputEvent::Text(text));
                }
            }
        }
        // Hover, focus and pressed states change without an action.
        self.st.dirty = true;
        self.st.poll_text(now);
        self.after_change();
    }

    /// Carry out one action. Every [`Action`] variant has an arm here.
    pub fn handle_action(&mut self, action: Action, now: Duration) -> Handled {
        let st = &mut self.st;
        st.dirty = true;
        let handled = match action {
            Action::Open => {
                let kind = DialogKind::OpenFile {
                    start_dir: st.folder.clone(),
                    filters: vec![st.image_filter()],
                };
                st.open_dialog(DialogPurpose::Open, kind);
                Handled::Done
            }
            Action::SaveAs => st.save_as_dialog(&mut self.ui, now),
            Action::CopyPath => st.copy_path(&mut self.ui, now),
            Action::CopyImage => st.copy_image(&mut self.ui, now),
            Action::CopyText(text) => {
                st.drop_image_copy();
                match st.clipboard.set_text(&text) {
                    Ok(()) => Handled::Done,
                    Err(_) => {
                        self.ui.toast("Could not copy", now);
                        Handled::Done
                    }
                }
            }
            Action::RequestPaste => match st.clipboard.get_text() {
                Ok(text) if !text.is_empty() => Handled::Paste(text),
                _ => Handled::Ignored,
            },
            Action::RotateLeft => st.push_edit(Edit::RotateLeft, &mut self.ui, now),
            Action::RotateRight => st.push_edit(Edit::RotateRight, &mut self.ui, now),
            Action::FlipHorizontal => st.push_edit(Edit::FlipHorizontal, &mut self.ui, now),
            Action::FlipVertical => st.push_edit(Edit::FlipVertical, &mut self.ui, now),
            // Crop mode, the resize and bulk dialogs live in the interface;
            // these notices only need a new frame.
            Action::StartCrop => Handled::Done,
            Action::CancelCrop => Handled::Done,
            Action::OpenResizeDialog => Handled::Done,
            Action::OpenBulkDialog => Handled::Done,
            Action::ApplyCrop(rect) => {
                st.push_edit(Edit::Crop(edit::pixel_rect(rect)), &mut self.ui, now)
            }
            Action::ApplyResize(spec) => {
                st.push_edit(Edit::Resize(edit::resize_spec(spec)), &mut self.ui, now)
            }
            Action::PickBulkFiles => {
                let kind = DialogKind::OpenFiles {
                    start_dir: st.folder.clone(),
                    filters: vec![st.image_filter()],
                };
                st.open_dialog(DialogPurpose::BulkFiles, kind);
                Handled::Done
            }
            Action::PickBulkFolder => {
                let kind = DialogKind::PickFolder {
                    start_dir: st.folder.clone(),
                    title: "Choose a folder of images".into(),
                };
                st.open_dialog(DialogPurpose::BulkFolder, kind);
                Handled::Done
            }
            Action::PickBulkDestination => {
                let kind = DialogKind::PickFolder {
                    start_dir: st.folder.clone(),
                    title: "Choose where to save".into(),
                };
                st.open_dialog(DialogPurpose::BulkDestination, kind);
                Handled::Done
            }
            Action::RunBulk(job) => st.run_bulk(job, &mut self.ui, now),
            Action::CancelBulk => match st.bulk.take() {
                Some(run) => {
                    run.cancel.cancel();
                    self.ui.toast("Bulk job stopped", now);
                    Handled::Done
                }
                None => Handled::Ignored,
            },
            Action::Undo => st.undo(),
            Action::FitToWindow => {
                st.view.set_fit();
                Handled::Done
            }
            Action::OneToOne { anchor } => match st.display_size() {
                Some(size) => {
                    let vp = st.content_rect;
                    st.view
                        .set_one_to_one(size, vp, anchor.unwrap_or_else(|| vp.center()));
                    Handled::Done
                }
                None => Handled::Ignored,
            },
            Action::ZoomIn => st.zoom(ZOOM_STEP, None),
            Action::ZoomOut => st.zoom(1.0 / ZOOM_STEP, None),
            Action::ZoomAt { factor, anchor } => st.zoom(factor, Some(anchor)),
            Action::PanBy { dx, dy } => match st.display_size() {
                Some(size) => {
                    st.view.pan_by(dx, dy, size, st.content_rect.size());
                    Handled::Done
                }
                None => Handled::Ignored,
            },
            Action::ToggleFullscreen => {
                st.fullscreen = !st.fullscreen;
                st.window.set_fullscreen(st.fullscreen);
                Handled::Done
            }
            Action::Next => st.step(1, now),
            Action::Previous => st.step(-1, now),
            Action::GoTo(index) => {
                if index < st.library.len() {
                    st.go_to(index, now);
                    Handled::Done
                } else {
                    Handled::Ignored
                }
            }
            Action::ToggleGallery => {
                st.settings.gallery_visible = !st.settings.gallery_visible;
                st.save_settings_soon();
                Handled::Done
            }
            Action::ConfirmResult { id, accepted } => match st.confirms.remove(&id) {
                Some(save) if accepted => {
                    st.request_save(save, &mut self.ui, now);
                    Handled::Done
                }
                Some(_) => {
                    self.ui.toast("Not saved", now);
                    Handled::Done
                }
                None => Handled::Ignored,
            },
            Action::Quit => {
                st.exit = true;
                Handled::Done
            }
        };
        st.update_title();
        handled
    }

    /// Take library events and job results. Call after every wake. Returns
    /// true when the screen should be redrawn.
    pub fn pump(&mut self, now: Duration) -> bool {
        self.st.clock = now;
        let events = self.st.library.poll_events();
        let mut changed = !events.is_empty();
        for event in events {
            self.st.on_library_event(event);
        }
        let had_text = self.st.text_id();
        while let Ok(msg) = self.st.inbox.try_recv() {
            changed = true;
            self.on_msg(msg, now);
        }
        changed |= self.st.check_target();
        self.st.poll_text(now);
        if changed {
            self.st.dirty = true;
            self.st.update_title();
        }
        if self.st.text_id() != had_text {
            // Text arrived under a resting pointer: the cursor over a word
            // becomes a text cursor without waiting for a move.
            self.input(InputEvent::Tick, now);
        }
        self.after_change();
        changed
    }

    /// The client area changed size (physical pixels). `maximized` is the
    /// window's state now.
    pub fn resized(&mut self, width: u32, height: u32, maximized: bool) {
        let st = &mut self.st;
        st.window_size = Size::new(width as f32, height as f32);
        st.maximized = maximized;
        if width > 0 && height > 0 {
            let (w, h) = screen_size(st.window_size);
            st.library.set_screen_size(w, h);
        }
        if width > 0 && height > 0 && !maximized && !st.fullscreen {
            let mut normal = st.normal.unwrap_or_default();
            normal.w = width as f32;
            normal.h = height as f32;
            st.normal = Some(normal);
        }
        st.dirty = true;
    }

    /// The window moved: `x`, `y` is the client area's top-left corner in
    /// physical desktop pixels.
    pub fn moved(&mut self, x: i32, y: i32, maximized: bool) {
        let st = &mut self.st;
        st.maximized = maximized;
        if !maximized && !st.fullscreen && st.window_size.w > 0.0 && st.window_size.h > 0.0 {
            // Minimized windows on Windows report -32000.
            if x > -30_000 && y > -30_000 {
                let mut normal =
                    st.normal
                        .unwrap_or(Rect::new(0.0, 0.0, st.window_size.w, st.window_size.h));
                normal.x = x as f32;
                normal.y = y as f32;
                st.normal = Some(normal);
            }
        }
    }

    /// The DPI scale factor changed.
    pub fn scale_changed(&mut self, scale: f32) {
        if scale.is_finite() && scale > 0.0 {
            self.st.scale = scale;
            self.st.dirty = true;
        }
    }

    /// The operating system theme changed.
    pub fn system_theme_changed(&mut self, theme: Option<ThemeKind>) {
        self.st.system_theme = theme;
        self.st.dirty = true;
    }

    /// The full screen state changed outside the core (the window manager).
    pub fn fullscreen_changed(&mut self, fullscreen: bool) {
        if self.st.fullscreen != fullscreen {
            self.st.fullscreen = fullscreen;
            self.st.dirty = true;
        }
    }

    /// The user closed the window.
    pub fn request_exit(&mut self) {
        self.st.exit = true;
    }

    /// Build one frame: the interface draw list plus the view to draw the
    /// content with.
    pub fn frame(
        &mut self,
        now: Duration,
        measure: &dyn TextMeasure,
        list: &mut DrawList,
    ) -> FrameOut {
        // A recognition whose start time came wakes the loop for a frame.
        self.st.poll_text(now);
        self.st.refresh_text();
        let (viewport, background, range) = {
            let ctx = context(&self.st, &self.ui, now);
            let viewport = self.ui.content_rect(&ctx);
            self.ui.build(&ctx, measure, list);
            (
                viewport,
                Ui::clear_color(&ctx),
                self.ui.gallery_visible_range(&ctx),
            )
        };
        self.st.content_rect = viewport;
        self.st.thumbs.set_visible(range, &self.st.library);
        self.st.dirty = false;
        FrameOut {
            view: self.st.view,
            viewport,
            background,
        }
    }

    /// Upload interface icons and gallery thumbnails, and free textures far
    /// out of view. Call before drawing each frame. Returns true when a
    /// texture changed.
    pub fn sync_textures(&mut self, now: Duration, sink: &mut dyn TextureSink) -> bool {
        self.st.refresh_text();
        let icons = {
            let ctx = context(&self.st, &self.ui, now);
            self.ui.sync_textures(&ctx, sink)
        };
        let thumbs = self.st.thumbs.sync(&self.st.library, sink);
        icons || thumbs
    }

    /// The GPU device was replaced: upload every texture and the content
    /// again.
    pub fn gpu_reset(&mut self) {
        self.ui.invalidate_textures();
        self.st.thumbs.invalidate();
        if let Some(shown) = &self.st.shown {
            let (content, logical) = shown.update();
            self.st.push_staged(Some(content), logical);
        }
        self.st.dirty = true;
    }

    /// The content to show, when it changed since the last call.
    pub fn take_content(&mut self) -> Option<ContentUpdate> {
        self.st.content_out.take()
    }

    /// Write the settings (on a worker, waiting at most `timeout`), stop
    /// background work and drop the library without joining threads stuck
    /// in file reads. The report hands back the thumbnails of the last
    /// folder. Returns at once when nothing is stuck and after at most
    /// `timeout` plus the library's short grace otherwise.
    pub fn shutdown(self, timeout: Duration) -> ShutdownReport {
        let App { ui: _, mut st } = self;
        if let Some(run) = &st.bulk {
            run.cancel.cancel();
        }
        // A copy that has not reached the clipboard stops there; one in the
        // middle of the hand-over ends with the process.
        st.drop_image_copy();
        st.text.cancel_job();
        let mut report = ShutdownReport {
            settings_saved: None,
            thumbnails: st.library.export_thumbnails(),
        };
        if let Some(path) = st.settings_path.clone() {
            let settings = st.final_settings();
            let disk = Arc::clone(&st.disk);
            let (tx, rx) = mpsc::channel();
            let spawned = jobs::spawn("pandite-settings", move || {
                let _ = tx.send(settings::save(&*disk, &path, &settings).is_ok());
            });
            if spawned.is_ok() {
                report.settings_saved = rx.recv_timeout(timeout).ok();
            }
        }
        // Dropping the library cancels its work and detaches stuck workers.
        drop(st);
        report
    }

    // ----- internals --------------------------------------------------------

    fn after_change(&mut self) {
        let now = self.st.clock;
        if std::mem::take(&mut self.st.resume_edits) {
            self.st.start_edit_job(&mut self.ui, now);
        }
        if std::mem::take(&mut self.st.resume_save) {
            if let Some(save) = self.st.pending_save.take() {
                self.st.request_save(save, &mut self.ui, now);
            }
        }
        if self.st.copy_wanted.is_some() {
            self.st.start_copy(&mut self.ui, now);
        }
        let st = &mut self.st;
        let cursor = self.ui.cursor();
        if st.cursor != Some(cursor) {
            st.cursor = Some(cursor);
            st.window.set_cursor(cursor);
        }
        let ime = self.ui.wants_text_input();
        if ime != st.ime {
            st.ime = ime;
            st.window.set_ime_allowed(ime);
        }
        if st.dirty {
            st.window.request_redraw();
        }
    }

    fn on_msg(&mut self, msg: Msg, now: Duration) {
        let st = &mut self.st;
        match msg {
            Msg::Dialog { id, paths } => {
                let Some(purpose) = st.dialogs_open.remove(&id) else {
                    return;
                };
                if paths.is_empty() {
                    return;
                }
                match purpose {
                    DialogPurpose::Open => {
                        let path = paths[0].clone();
                        st.open_path(path, now, false);
                    }
                    DialogPurpose::SaveAs => {
                        st.save_target_chosen(paths[0].clone(), &mut self.ui, now)
                    }
                    DialogPurpose::BulkFiles => self.ui.add_bulk_files(paths),
                    DialogPurpose::BulkFolder => self.ui.set_bulk_folder(paths[0].clone()),
                    DialogPurpose::BulkDestination => {
                        self.ui.set_bulk_destination(paths[0].clone());
                    }
                }
            }
            Msg::EditDone {
                image,
                edit,
                result,
            } => st.edit_done(image, edit, result, &mut self.ui, now),
            Msg::Saved { id, result } => st.saved(id, result, &mut self.ui, now),
            Msg::Probed { path, size } => {
                if st.startup_path.as_deref() == Some(path.as_path()) {
                    st.auto_size_to(size);
                }
            }
            Msg::Resolved { original, found } => {
                let wanted =
                    st.target.as_ref().is_some_and(|t| t.path == original) && st.loading.is_some();
                if !wanted {
                    return;
                }
                match found {
                    Ok(path) => {
                        if st.startup_path.as_deref() == Some(original.as_path()) {
                            st.startup_path = Some(path.clone());
                        }
                        st.start_library_open(path, now);
                    }
                    Err(err) => {
                        st.loading = None;
                        st.notice = Some(format!("Cannot open {}: {err}", file_name(&original)));
                    }
                }
            }
            Msg::BulkProgress {
                id,
                done,
                total,
                name,
            } => {
                if let Some(run) = st.bulk.as_mut().filter(|r| r.id == id) {
                    run.done = done;
                    run.total = total;
                    run.label = if name.is_empty() {
                        format!("Converting {total} images")
                    } else {
                        format!("Converted {name}")
                    };
                }
            }
            Msg::BulkDone {
                id,
                summary,
                destination,
            } => {
                if st.bulk.as_ref().is_some_and(|r| r.id == id) {
                    st.bulk = None;
                    self.ui.toast(summary, now);
                }
                // New files in the folder on screen show up in the gallery.
                if st
                    .folder
                    .as_deref()
                    .is_some_and(|f| same_path(f, &destination))
                {
                    st.library.refresh();
                }
            }
            Msg::Copied { id, outcome } => {
                let Some(run) = st.copying.take_if(|r| r.id == id) else {
                    return;
                };
                // A copy replaced by a newer one (or by a text copy) says
                // nothing; the newer one reports.
                if run.cancel.is_cancelled() {
                    return;
                }
                match outcome {
                    Copied::Done => self.ui.toast("Image copied", now),
                    Copied::Cancelled => {}
                    Copied::Failed(why) => self
                        .ui
                        .toast(format!("Could not copy the image: {why}"), now),
                }
            }
            Msg::TextDone {
                ticket,
                image,
                key,
                result,
            } => st.text_done(ticket, image, key, result),
        }
    }
}

/// The screen size for staged loading: the window's client area in
/// physical pixels (at least 1 x 1).
fn screen_size(window: Size) -> (u32, u32) {
    let side = |v: f32| {
        if v.is_finite() {
            v.round().clamp(1.0, 65_535.0) as u32
        } else {
            1
        }
    };
    (side(window.w), side(window.h))
}

/// "PNG image" becomes "PNG" for the format list.
fn short_format_name(name: &'static str) -> &'static str {
    name.strip_suffix(" image").unwrap_or(name)
}

/// Build the interface snapshot from the state.
fn context<'a>(st: &'a State, ui: &Ui, now: Duration) -> UiContext<'a> {
    let mut ctx = UiContext {
        now,
        window_size: st.window_size,
        scale_factor: st.scale,
        theme: st.theme(),
        fullscreen: st.fullscreen,
        gallery_visible: st.settings.gallery_visible,
        image: None,
        index: st.target.as_ref().map_or(0, |t| t.index),
        count: st.count,
        folder: st.folder.as_deref(),
        thumbnails: st.thumbs.states(),
        can_undo: st.shown.as_ref().is_some_and(|s| s.edits.is_edited()),
        busy: (!st.busy_text.is_empty()).then_some(Progress {
            label: &st.busy_text,
            done: 0,
            total: 0,
        }),
        bulk: st.bulk.as_ref().map(|b| Progress {
            label: &st.bulk_text,
            done: b.done as u64,
            total: b.total as u64,
        }),
        loading: st.loading.as_ref().map(|l| Loading {
            file_name: &l.name,
            elapsed: now.saturating_sub(l.since),
            progress: None,
            cloud: l.cloud,
            over_image: st.shown.is_some(),
        }),
        notice: st.notice.as_deref(),
        encoders: &st.encoders,
        text_enabled: st.text.active(),
        text: None,
    };
    if let Some(shown) = &st.shown {
        let content = ui.content_rect(&ctx);
        let (width, height) = shown.display_dims();
        let size = Size::new(width as f32, height as f32);
        ctx.image = Some(ImageInfo {
            file_name: &shown.name,
            width,
            height,
            view: st.view,
            zoom_percent: st.view.scale(size, content.size()) * 100.0,
            screen_rect: st.view.image_rect(size, content),
            edited: shown.edits.is_edited(),
        });
        // Words follow the applied edits; edits still queued or running
        // show the previous picture, and so do its words.
        ctx.text = shown
            .text
            .as_ref()
            .filter(|t| shown.standin.is_none() && !t.layout.is_empty())
            .map(|t| TextView {
                layout: &t.layout,
                to_image: shown
                    .edits
                    .transform((shown.base.width(), shown.base.height())),
                id: t.id,
            });
    }
    ctx
}

impl State {
    fn theme(&self) -> ThemeKind {
        match self.settings.theme {
            ThemeChoice::Dark => ThemeKind::Dark,
            ThemeChoice::Light => ThemeKind::Light,
            ThemeChoice::System => self.system_theme.unwrap_or(ThemeKind::Dark),
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn final_settings(&self) -> Settings {
        let mut settings = self.settings.clone();
        if let Some(normal) = self.normal.filter(|r| r.w >= 1.0 && r.h >= 1.0) {
            settings.window = Some(WindowPlacement {
                x: normal.x.round() as i32,
                y: normal.y.round() as i32,
                width: normal.w.round() as u32,
                height: normal.h.round() as u32,
                maximized: self.maximized,
                fullscreen: self.fullscreen,
            });
        }
        settings
    }

    fn save_settings_soon(&mut self) {
        let Some(path) = self.settings_path.clone() else {
            return;
        };
        let settings = self.final_settings();
        let disk = Arc::clone(&self.disk);
        let _ = jobs::spawn("pandite-settings", move || {
            let _ = settings::save(&*disk, &path, &settings);
        });
    }

    fn refresh_text(&mut self) {
        self.busy_text.clear();
        if let Some(save) = &self.saving {
            self.busy_text = format!("Saving {}", save.name);
        } else if let Some(shown) = &self.shown {
            let waiting = shown.edits.is_busy()
                || self.pending_save.is_some()
                || self.copy_wanted == Some(shown.id);
            if shown.standin.is_some() && waiting {
                self.busy_text = "Loading full resolution".into();
            } else if self.pending_save.is_some() {
                self.busy_text = "Preparing to save".into();
            } else if shown.edits.is_busy() {
                self.busy_text = "Applying edits".into();
            }
        } else if self.pending_save.is_some() {
            self.busy_text = "Preparing to save".into();
        }
        self.bulk_text.clear();
        if let Some(run) = &self.bulk {
            self.bulk_text.clone_from(&run.label);
        }
    }

    fn update_title(&mut self) {
        let title = match (&self.shown, &self.target) {
            (Some(shown), _) => {
                let edited = if shown.edits.is_edited() {
                    " (edited)"
                } else {
                    ""
                };
                format!("{}{edited} - pandite", shown.name)
            }
            (None, Some(t)) => format!("{} - pandite", file_name(&t.path)),
            (None, None) => match &self.loading {
                Some(load) => format!("{} - pandite", load.name),
                None => "pandite".to_string(),
            },
        };
        if title != self.title {
            self.window.set_title(&title);
            self.title = title;
        }
    }

    fn image_filter(&self) -> Filter {
        Filter {
            name: "Images".into(),
            extensions: self
                .registry
                .decodable_extensions()
                .into_iter()
                .map(String::from)
                .collect(),
        }
    }

    fn open_dialog(&mut self, purpose: DialogPurpose, kind: DialogKind) {
        let id = self.next_id();
        self.dialogs_open.insert(id, purpose);
        let out = self.out.clone();
        self.dialogs.show(
            kind,
            Box::new(move |paths| out.send(Msg::Dialog { id, paths })),
        );
    }

    fn display_size(&self) -> Option<Size> {
        let (w, h) = self.shown.as_ref()?.display_dims();
        Some(Size::new(w as f32, h as f32))
    }

    fn zoom(&mut self, factor: f32, anchor: Option<Point>) -> Handled {
        let Some(size) = self.display_size() else {
            return Handled::Ignored;
        };
        let vp = self.content_rect;
        self.view
            .zoom_by(factor, size, vp, anchor.unwrap_or_else(|| vp.center()));
        Handled::Done
    }

    fn push_content(&mut self, content: Option<Content>) {
        self.push_staged(content, None);
    }

    /// Hand content to the shell; `logical` marks a reduced stand-in and
    /// carries the full image's size.
    fn push_staged(&mut self, content: Option<Content>, logical: Option<(u32, u32)>) {
        self.content_gen += 1;
        self.content_out = Some(ContentUpdate {
            generation: self.content_gen,
            content,
            full_resolution: logical.is_none(),
            logical,
        });
        self.dirty = true;
    }

    // ----- opening and navigation --------------------------------------------

    fn open_path(&mut self, path: PathBuf, now: Duration, startup: bool) {
        self.notice = None;
        // The startup load has shown its loading screen since launch; keep
        // its clock so the screen does not blink.
        let since = match &self.loading {
            Some(load) if startup && load.startup => load.since,
            _ => now,
        };
        self.loading = Some(LoadState {
            path: path.clone(),
            name: file_name(&path),
            since,
            cloud: false,
            startup,
        });
        self.target = Some(Target {
            path: path.clone(),
            index: 0,
        });
        self.folder = path.parent().map(Path::to_path_buf);
        if self.registry.can_open_path(&path) {
            self.start_library_open(path, now);
        } else {
            // A folder or a file without a known extension: find out on a
            // worker what to open.
            let spawned = jobs::resolve(
                self.out.clone(),
                Arc::clone(&self.disk),
                Arc::clone(&self.registry),
                path.clone(),
            );
            if spawned.is_err() {
                self.start_library_open(path, now);
            }
        }
    }

    fn start_library_open(&mut self, path: PathBuf, now: Duration) {
        if let Some(load) = &mut self.loading {
            load.path = path.clone();
            load.name = file_name(&path);
        } else {
            self.loading = Some(LoadState {
                path: path.clone(),
                name: file_name(&path),
                since: now,
                cloud: false,
                startup: false,
            });
        }
        self.target = Some(Target {
            path: path.clone(),
            index: 0,
        });
        self.folder = path.parent().map(Path::to_path_buf);
        self.load_started = Some((path.clone(), now));
        self.library.open(path.clone());
        // A file decoded earlier is served from the cache without an event.
        self.check_target();
        if self.auto_size == AutoSize::Waiting {
            let _ = jobs::probe(
                self.out.clone(),
                Arc::clone(&self.disk),
                Arc::clone(&self.registry),
                path,
            );
        }
    }

    /// Show the target when the library has it, or report its failure.
    /// Events usually do this; a file served from the cache sends none.
    /// Returns true when the screen changed.
    fn check_target(&mut self) -> bool {
        let Some(target) = self.target.clone() else {
            return false;
        };
        let full_shown = self
            .shown
            .as_ref()
            .is_some_and(|s| s.path == target.path && s.standin.is_none());
        if self.loading.is_none() || full_shown {
            return false;
        }
        if self.library.path(target.index).as_deref() != Some(target.path.as_path()) {
            return false;
        }
        match self.library.get(target.index) {
            Fetch::Ready(content) => {
                self.show(target.path, content);
                true
            }
            Fetch::Failed(err) => {
                self.fail(&target.path, &err);
                true
            }
            Fetch::Downloading => {
                if let Some(load) = self.loading.as_mut() {
                    load.cloud = true;
                }
                false
            }
            Fetch::Pending | Fetch::NotLocal | Fetch::Cancelled => {
                let standing_in = self.shown.as_ref().is_some_and(|s| s.path == target.path);
                match self.library.reduced(target.index) {
                    Some(reduced) if !standing_in => {
                        self.show_reduced(target.path, reduced);
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    fn index_of(&self, path: &Path) -> Option<usize> {
        self.library.paths().iter().position(|p| p == path)
    }

    fn step(&mut self, delta: isize, now: Duration) -> Handled {
        let len = self.library.len();
        if len < 2 {
            return Handled::Ignored;
        }
        let current = self.target.as_ref().map_or(0, |t| t.index.min(len - 1));
        let next = current as isize + delta;
        let index = if (0..len as isize).contains(&next) {
            next as usize
        } else if self.settings.wrap_around {
            next.rem_euclid(len as isize) as usize
        } else {
            return Handled::Ignored;
        };
        self.go_to(index, now);
        Handled::Done
    }

    fn go_to(&mut self, index: usize, now: Duration) {
        let Some(path) = self.library.path(index) else {
            return;
        };
        self.library.set_current(index);
        self.target = Some(Target {
            path: path.clone(),
            index,
        });
        self.notice = None;
        if self
            .shown
            .as_ref()
            .is_some_and(|s| s.path == path && s.standin.is_none())
        {
            self.loading = None;
            return;
        }
        self.load_started = Some((path.clone(), now));
        match self.library.get(index) {
            Fetch::Ready(content) => self.show(path, content),
            Fetch::Failed(err) => self.fail(&path, &err),
            fetch => {
                let cloud = matches!(fetch, Fetch::Downloading);
                let keep_since = self.loading.as_ref().map(|l| l.since);
                self.loading = Some(LoadState {
                    name: file_name(&path),
                    path: path.clone(),
                    // Holding a key through several files keeps one timer,
                    // so the overlay does not restart on every step.
                    since: keep_since.unwrap_or(now),
                    cloud,
                    startup: false,
                });
                // A read-ahead neighbor shows at screen resolution at once.
                if let Some(reduced) = self.library.reduced(index) {
                    if !self.shown.as_ref().is_some_and(|s| s.path == path) {
                        self.show_reduced(path, reduced);
                    }
                }
            }
        }
    }

    fn show(&mut self, path: PathBuf, content: Arc<Content>) {
        let base = content.first_image().clone();
        let dims = (base.width(), base.height());
        self.loading = None;
        self.notice = None;
        // The full image replaces its reduced stand-in: same view, and the
        // edits and the save that waited for it start now.
        if let Some(shown) = self
            .shown
            .as_mut()
            .filter(|s| s.path == path && s.standin.is_some())
        {
            shown.content = Arc::clone(&content);
            shown.base = base;
            shown.standin = None;
            self.push_content(Some((*content).clone()));
            self.mark_full_resolution();
            self.resume_waiting_work();
            return;
        }
        let id = self.next_image_id;
        self.next_image_id += 1;
        self.shown = Some(Shown {
            name: file_name(&path),
            path: path.clone(),
            content: Arc::clone(&content),
            base,
            edits: EditStack::new(),
            id,
            told_first_frame: false,
            standin: None,
            full_gen: None,
            presented_at: None,
            text: None,
        });
        self.view = ViewState::default();
        self.pending_save = None;
        self.push_content(Some((*content).clone()));
        self.mark_full_resolution();
        if self.startup_path.as_deref() == Some(path.as_path()) {
            self.auto_size_to(dims);
        }
    }

    /// The content just pushed is the image on screen at full resolution:
    /// once a frame with it is presented, its text may be read.
    fn mark_full_resolution(&mut self) {
        let generation = self.content_gen;
        if let Some(shown) = self.shown.as_mut() {
            shown.full_gen = Some(generation);
            shown.presented_at = None;
        }
    }

    /// Show a reduced stand-in for `path` while its full resolution
    /// decodes. The view is set up for the full image's size, so the swap
    /// later changes nothing on screen but the sharpness.
    fn show_reduced(&mut self, path: PathBuf, reduced: Reduced) {
        let dims = (reduced.width, reduced.height);
        let content = Arc::new(Content::Still(reduced.image));
        if let Some(shown) = self.shown.as_mut().filter(|s| s.path == path) {
            if shown.standin.is_some() {
                // A better stand-in for the same image: keep the view.
                shown.base = content.first_image().clone();
                shown.content = Arc::clone(&content);
                shown.standin = Some(dims);
                self.push_staged(Some((*content).clone()), Some(dims));
            }
            return;
        }
        let id = self.next_image_id;
        self.next_image_id += 1;
        self.shown = Some(Shown {
            name: file_name(&path),
            path: path.clone(),
            base: content.first_image().clone(),
            content: Arc::clone(&content),
            edits: EditStack::new(),
            id,
            told_first_frame: false,
            standin: Some(dims),
            full_gen: None,
            presented_at: None,
            text: None,
        });
        self.view = ViewState::default();
        self.notice = None;
        self.pending_save = None;
        self.push_staged(Some((*content).clone()), Some(dims));
        if self.startup_path.as_deref() == Some(path.as_path()) {
            self.auto_size_to(dims);
        }
    }

    /// Edits and a save that waited for the full image start.
    fn resume_waiting_work(&mut self) {
        let Some(shown) = &self.shown else {
            return;
        };
        if shown.edits.is_busy() {
            self.resume_edits = true;
        } else if self.pending_save.is_some() {
            self.resume_save = true;
        }
    }

    fn fail(&mut self, path: &Path, err: &Error) {
        self.loading = None;
        // Edits or a save waiting for this image cannot run.
        self.pending_save = None;
        if self.shown.take().is_some() {
            self.push_content(None);
        }
        self.notice = Some(format!("Cannot open {}: {err}", file_name(path)));
    }

    fn on_library_event(&mut self, event: Event) {
        match event {
            Event::Listed { len, current } => {
                self.count = len;
                self.thumbs.reset(len);
                if let Some(target) = &mut self.target {
                    match current {
                        Some(index) => target.index = index,
                        None => {
                            let path = target.path.clone();
                            if let Some(index) = self.index_of(&path) {
                                if let Some(t) = &mut self.target {
                                    t.index = index;
                                }
                            }
                        }
                    }
                }
            }
            Event::Reduced { path, .. } => {
                let Some(target) = self.target.clone() else {
                    return;
                };
                if target.path != path || self.shown.as_ref().is_some_and(|s| s.path == path) {
                    return;
                }
                let index = if self.library.path(target.index).as_deref() == Some(path.as_path()) {
                    Some(target.index)
                } else {
                    self.index_of(&path)
                };
                if let Some(reduced) = index.and_then(|i| self.library.reduced(i)) {
                    self.show_reduced(path, reduced);
                }
            }
            Event::Ready { path, .. } => {
                let Some(target) = self.target.clone() else {
                    return;
                };
                let full_shown = self
                    .shown
                    .as_ref()
                    .is_some_and(|s| s.path == path && s.standin.is_none());
                if target.path != path || full_shown {
                    return;
                }
                let index = if self.library.path(target.index).as_deref() == Some(path.as_path()) {
                    Some(target.index)
                } else {
                    self.index_of(&path)
                };
                if let Some(index) = index {
                    if let Fetch::Ready(content) = self.library.get(index) {
                        if let Some(t) = &mut self.target {
                            t.index = index;
                        }
                        self.show(path, content);
                    }
                }
            }
            Event::Failed { path, error, .. } => {
                if self.target.as_ref().is_some_and(|t| t.path == path) && self.loading.is_some() {
                    self.fail(&path, &error);
                }
            }
            Event::Downloading { path, .. } => {
                if let Some(load) = self.loading.as_mut().filter(|l| l.path == path) {
                    load.cloud = true;
                }
            }
            Event::NotLocal { index, .. } => self.thumbs.mark(index, ThumbState::NotDownloaded),
            Event::ThumbnailReady { index, .. } => self.thumbs.ready(index),
            Event::ThumbnailFailed { index, .. } => self.thumbs.mark(index, ThumbState::Failed),
            _ => {}
        }
    }

    // ----- text recognition -------------------------------------------------

    /// True when the shown image is still the one the user wants to see
    /// and can have its text read: full resolution, a still, on target.
    fn text_wanted(&self, shown: &Shown) -> bool {
        shown.standin.is_none()
            && !shown.animated()
            && self.target.as_ref().is_some_and(|t| t.path == shown.path)
    }

    /// When recognition of the shown image may start, if it is waiting.
    fn text_due(&self) -> Option<Duration> {
        if !self.text.active() || self.text.job.is_some() {
            return None;
        }
        let shown = self.shown.as_ref()?;
        if shown.text.is_some() || !self.text_wanted(shown) {
            return None;
        }
        Some(shown.presented_at? + TEXT_DELAY)
    }

    /// The id of the text attached to the shown image.
    fn text_id(&self) -> Option<u64> {
        self.shown.as_ref()?.text.as_ref().map(|t| t.id)
    }

    /// Stop a recognition nobody waits for any more, and start the one
    /// that is due.
    fn poll_text(&mut self, now: Duration) {
        if let Some(job) = &self.text.job {
            let wanted = self
                .shown
                .as_ref()
                .is_some_and(|s| s.id == job.image && self.text_wanted(s));
            if !wanted || !self.text.active() {
                self.text.cancel_job();
            }
        }
        let Some(due) = self.text_due() else {
            return;
        };
        let Some(shown) = self.shown.as_ref() else {
            return;
        };
        let key = TextKey::new(&shown.path, &shown.base);
        if let Some(layout) = self.text.cached(&key) {
            // Read earlier this session: show it at once.
            let id = self.text.next_id();
            if let Some(shown) = self.shown.as_mut() {
                shown.text = Some(ShownText { id, layout });
            }
            self.dirty = true;
            return;
        }
        if now < due {
            return;
        }
        let (image_id, image) = (shown.id, shown.base.clone());
        if !self
            .text
            .start(self.out.clone(), image_id, key.clone(), image)
        {
            // No thread: count the image as read, without text, so this
            // does not repeat on every frame.
            self.attach_text(image_id, &key, Arc::new(TextLayout::default()));
        }
    }

    /// Attach `layout` to the shown image when it is still `image_id` read
    /// from the same pixels and has no text yet (a selection on text that
    /// is already there stays).
    fn attach_text(&mut self, image_id: u64, key: &TextKey, layout: Arc<TextLayout>) {
        let id = self.text.next_id();
        if let Some(shown) = self.shown.as_mut().filter(|s| {
            s.id == image_id && s.text.is_none() && TextKey::new(&s.path, &s.base) == *key
        }) {
            shown.text = Some(ShownText { id, layout });
            self.dirty = true;
            // An earlier job answered for the same pixels: a later one
            // for them is not needed.
            if self.text.job.as_ref().is_some_and(|j| j.image == image_id) {
                self.text.cancel_job();
            }
        }
    }

    fn text_done(
        &mut self,
        ticket: u64,
        image: u64,
        key: TextKey,
        result: pandite_core::Result<TextLayout>,
    ) {
        if self.text.job.as_ref().is_some_and(|j| j.ticket == ticket) {
            self.text.job = None;
        }
        if !self.text.current(ticket) {
            // Read from pixels a save replaced since.
            return;
        }
        match result {
            Ok(layout) => {
                let layout = Arc::new(layout);
                self.text.store(key.clone(), Arc::clone(&layout));
                self.attach_text(image, &key, layout);
            }
            // Navigation stopped it; it starts again if the image returns.
            Err(Error::Cancelled) => {}
            // No recognizer or language here: no text features at all, and
            // no message (this is not the user's problem to solve).
            Err(Error::Unavailable(_)) => self.text.set_unavailable(),
            // A failure for this image: remember it as "no text" so it is
            // not tried again this session.
            Err(_) => {
                let empty = Arc::new(TextLayout::new(key.width, key.height));
                self.text.store(key.clone(), Arc::clone(&empty));
                self.attach_text(image, &key, empty);
            }
        }
    }

    // ----- first-open window size --------------------------------------------

    fn auto_size_to(&mut self, dims: (u32, u32)) {
        if self.auto_size != AutoSize::Waiting {
            return;
        }
        self.auto_size = AutoSize::Done;
        let late = self.clock.saturating_sub(self.first_frame_at) > AUTO_SIZE_WINDOW;
        if self.fullscreen || self.maximized || late {
            return;
        }
        let Some(work) = self.window.work_area() else {
            return;
        };
        // The gallery strip height at this scale: a tall window with two
        // images always shows it. The strip geometry depends on the
        // context only, so a fresh interface measures it.
        let probe = UiContext {
            window_size: Size::new(4000.0, 4000.0),
            scale_factor: self.scale,
            gallery_visible: self.settings.gallery_visible,
            count: 2,
            ..UiContext::default()
        };
        let strip = (4000.0 - Ui::new().content_rect(&probe).h).max(0.0);
        let min = Size::new(480.0 * self.scale, 360.0 * self.scale);
        let rect = settings::first_open_rect(
            work,
            Some(dims),
            self.settings.first_open_fraction,
            strip,
            min,
        );
        // The window reports its new frame position and size through
        // `moved` and `resized`.
        self.window.set_client_rect(rect);
    }

    // ----- edits -------------------------------------------------------------

    fn push_edit(&mut self, edit: Edit, ui: &mut Ui, now: Duration) -> Handled {
        let Some(shown) = &mut self.shown else {
            return Handled::Ignored;
        };
        // Each edit keeps its result for undo. An image this large would
        // need several more buffers of its size; refuse instead of running
        // out of memory.
        if shown.current().byte_size() > MAX_EDIT_BYTES {
            ui.toast("This image is too large to edit", now);
            return Handled::Ignored;
        }
        if shown.animated() && !shown.told_first_frame {
            shown.told_first_frame = true;
            ui.toast("Edits apply to the first frame", now);
        }
        shown.edits.push(edit);
        self.start_edit_job(ui, now);
        Handled::Done
    }

    fn start_edit_job(&mut self, ui: &mut Ui, now: Duration) {
        let Some(shown) = &mut self.shown else {
            return;
        };
        if shown.standin.is_some() {
            // Edits run on full-resolution data: they wait for it.
            return;
        }
        let Some((edit, input)) = shown.edits.next_job(&shown.base) else {
            return;
        };
        if jobs::edit(self.out.clone(), shown.id, edit, input).is_err() {
            let _ = shown.edits.finish(Err(Error::Backend("no thread".into())));
            ui.toast("Could not start the edit", now);
        }
    }

    fn edit_done(
        &mut self,
        image: u64,
        edit: Edit,
        result: pandite_core::Result<ImageBuf>,
        ui: &mut Ui,
        now: Duration,
    ) {
        let Some(shown) = self.shown.as_mut().filter(|s| s.id == image) else {
            return;
        };
        let before = (shown.current().width(), shown.current().height());
        match shown.edits.finish(result) {
            Ok(true) => {
                let after = (shown.current().width(), shown.current().height());
                let still = Content::Still(shown.current().clone());
                if matches!(edit, Edit::Crop(_)) {
                    self.view.set_fit();
                } else if before != after {
                    self.view.pan = Point::default();
                }
                self.push_content(Some(still));
            }
            Ok(false) => {}
            Err(err) => ui.toast(edit::describe_error(&edit, &err), now),
        }
        self.start_edit_job(ui, now);
        if self.shown.as_ref().is_some_and(|s| s.edits.is_idle()) {
            if let Some(save) = self.pending_save.take() {
                self.start_save(save, ui, now);
            }
        }
    }

    fn undo(&mut self) -> Handled {
        let Some(shown) = &mut self.shown else {
            return Handled::Ignored;
        };
        match shown.edits.undo() {
            Undo::Nothing => Handled::Ignored,
            Undo::Dequeued | Undo::DroppedRunning => Handled::Done,
            Undo::Reverted => {
                let (content, logical) = shown.update();
                self.view.pan = Point::default();
                self.push_staged(Some(content), logical);
                Handled::Done
            }
        }
    }

    // ----- save as -------------------------------------------------------------

    fn save_filters(&self, first: &str) -> Vec<Filter> {
        let mut filters: Vec<Filter> = self
            .registry
            .encoders()
            .iter()
            .map(|e| Filter {
                name: e.name().to_string(),
                extensions: e.extensions().iter().map(|s| s.to_string()).collect(),
            })
            .collect();
        if let Some(pos) = filters
            .iter()
            .position(|f| f.extensions.iter().any(|e| e == first))
        {
            let f = filters.remove(pos);
            filters.insert(0, f);
        }
        filters
    }

    fn save_as_dialog(&mut self, ui: &mut Ui, now: Duration) -> Handled {
        let Some(shown) = &self.shown else {
            ui.toast("Open an image first", now);
            return Handled::Ignored;
        };
        if self.saving.is_some() {
            ui.toast("Still saving", now);
            return Handled::Ignored;
        }
        let ext = shown
            .path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .filter(|e| self.registry.encoder_for_extension(e).is_some())
            .unwrap_or_else(|| "png".to_string());
        let stem = shown
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "image".into());
        let kind = DialogKind::SaveFile {
            start_dir: shown.path.parent().map(Path::to_path_buf),
            file_name: format!("{stem}.{ext}"),
            filters: self.save_filters(&ext),
        };
        self.open_dialog(DialogPurpose::SaveAs, kind);
        Handled::Done
    }

    fn save_target_chosen(&mut self, chosen: PathBuf, ui: &mut Ui, now: Duration) {
        let Some(shown) = &self.shown else {
            return;
        };
        // The format follows the extension; a name without a known one
        // gets the original's format (or PNG).
        let ext = chosen
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase());
        let (target, encoder) = match ext
            .as_deref()
            .and_then(|e| self.registry.encoder_for_extension(e))
        {
            Some(enc) => (chosen.clone(), enc.id().to_string()),
            None => {
                let fallback = shown
                    .path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase())
                    .and_then(|e| self.registry.encoder_for_extension(&e).cloned())
                    .or_else(|| self.registry.encoder_by_id("png").cloned());
                let Some(enc) = fallback else {
                    ui.toast("No encoder for this file type", now);
                    return;
                };
                let mut name = chosen.as_os_str().to_owned();
                name.push(".");
                name.push(enc.extensions()[0]);
                (PathBuf::from(name), enc.id().to_string())
            }
        };
        let save = PendingSave { target, encoder };
        if same_path(&save.target, &shown.path) {
            let name = shown.name.clone();
            let id = self.next_id();
            self.confirms.insert(id, save);
            ui.show_confirm(ConfirmRequest {
                id,
                title: "Replace the original?".into(),
                message: format!("Saving replaces {name} on disk with the edited image."),
                confirm_label: "Replace".into(),
                cancel_label: "Cancel".into(),
            });
        } else {
            self.request_save(save, ui, now);
        }
    }

    fn request_save(&mut self, save: PendingSave, ui: &mut Ui, now: Duration) {
        if self
            .shown
            .as_ref()
            .is_some_and(|s| s.edits.is_busy() || s.standin.is_some())
        {
            self.pending_save = Some(save);
        } else {
            self.start_save(save, ui, now);
        }
    }

    fn start_save(&mut self, save: PendingSave, ui: &mut Ui, now: Duration) {
        let Some(shown) = &self.shown else {
            return;
        };
        let id = self.next_id;
        self.next_id += 1;
        let job = jobs::SaveJob {
            id,
            encoder: save.encoder.clone(),
            image: shown.current().clone(),
            options: EncodeOptions {
                quality: self.settings.jpeg_quality.clamp(1, 100),
                lossless: save.encoder == "webp",
            },
            target: save.target.clone(),
        };
        let saving = Saving {
            id,
            name: file_name(&save.target),
            overwrite: same_path(&save.target, &shown.path),
            target: save.target,
            image: shown.id,
            animated: shown.animated(),
        };
        match jobs::save(
            self.out.clone(),
            Arc::clone(&self.disk),
            Arc::clone(&self.registry),
            job,
        ) {
            Ok(()) => self.saving = Some(saving),
            Err(_) => ui.toast("Could not start saving", now),
        }
    }

    fn saved(&mut self, id: u64, result: Result<(), String>, ui: &mut Ui, now: Duration) {
        let Some(save) = self.saving.take_if(|s| s.id == id) else {
            return;
        };
        if let Err(err) = result {
            ui.toast(format!("Could not save {}: {err}", save.name), now);
            return;
        }
        let first_frame = if save.animated {
            " (first frame only)"
        } else {
            ""
        };
        ui.toast(format!("Saved {}{first_frame}", save.name), now);
        let Some(shown) = self.shown.as_mut().filter(|s| s.id == save.image) else {
            // The user moved on; the cache must not keep the old file.
            if save.overwrite {
                self.library.invalidate(&save.target);
            }
            return;
        };
        // The picture on screen is what the file now holds: the edits are
        // saved and become the new starting point.
        let animated = shown.animated();
        let had_edits = !shown.edits.is_empty();
        let committed = shown.edits.commit(&shown.base);
        let old_path = shown.path.clone();
        if had_edits || animated {
            shown.base = committed.clone();
            shown.content = Arc::new(Content::Still(committed.clone()));
            // New pixels: words read from the old ones no longer apply and
            // are read again from the saved image.
            shown.text = None;
        }
        if !save.overwrite {
            shown.path = save.target.clone();
            shown.name = save.name.clone();
        }
        if had_edits || animated {
            self.text.forget(&old_path);
            self.text.forget(&save.target);
            if self
                .text
                .job
                .as_ref()
                .is_some_and(|j| j.image == save.image)
            {
                self.text.cancel_job();
            }
        } else if let Some(text) = &shown.text {
            // The same pixels under a new name keep their words.
            let key = TextKey::new(&shown.path, &shown.base);
            self.text.store(key, Arc::clone(&text.layout));
        }
        if save.overwrite {
            self.library.invalidate(&save.target);
        } else {
            // Save As makes the new file the one on screen.
            self.folder = save.target.parent().map(Path::to_path_buf);
            self.target = Some(Target {
                path: save.target.clone(),
                index: 0,
            });
            self.library.open(save.target.clone());
        }
        if animated {
            self.push_content(Some(Content::Still(committed)));
            self.mark_full_resolution();
        }
    }

    // ----- clipboard and bulk ----------------------------------------------------

    /// Stop the image copy that waits or runs, when the clipboard gets
    /// something else: the last copy wins.
    fn drop_image_copy(&mut self) {
        self.copy_wanted = None;
        if let Some(run) = &self.copying {
            run.cancel.cancel();
        }
    }

    /// Ctrl+C: copy the image on screen once it is ready (full resolution,
    /// edits applied). A second request while one runs never starts a
    /// second job: for the same picture it is dropped, for another one it
    /// waits until the running copy ends.
    fn copy_image(&mut self, ui: &mut Ui, now: Duration) -> Handled {
        let Some(shown) = &self.shown else {
            ui.toast("Open an image first", now);
            return Handled::Ignored;
        };
        let id = shown.id;
        let waits_for_full = shown.standin.is_some();
        if let Some(run) = &self.copying {
            if run.image == id && run.picture == self.content_gen && !run.cancel.is_cancelled() {
                ui.toast("Still copying the image", now);
                return Handled::Ignored;
            }
            // The picture changed: the running copy is out of date. It ends
            // soon; this one starts then.
            run.cancel.cancel();
        }
        self.copy_wanted = Some(id);
        if waits_for_full {
            ui.toast("Copying when the full image has loaded", now);
        }
        self.start_copy(ui, now);
        Handled::Done
    }

    /// Start the image copy that waits, when nothing holds it back. A copy
    /// for an image no longer on screen is dropped.
    fn start_copy(&mut self, ui: &mut Ui, now: Duration) {
        let Some(wanted) = self.copy_wanted else {
            return;
        };
        let Some(shown) = self.shown.as_ref().filter(|s| s.id == wanted) else {
            self.copy_wanted = None;
            return;
        };
        if self.copying.is_some() || shown.standin.is_some() || shown.edits.is_busy() {
            return;
        }
        let image = self.copy_source(shown);
        self.copy_wanted = None;
        let pixels = u64::from(image.width()) * u64::from(image.height());
        let slow = pixels >= COPY_NOTICE_PIXELS || image.format() != PixelFormat::Rgba8;
        let run = CopyRun {
            id: self.next_id(),
            image: wanted,
            picture: self.content_gen,
            cancel: CancelToken::new(),
        };
        let job = CopyJob {
            id: run.id,
            image,
            writer: self.clipboard.image_writer(),
            cancel: run.cancel.clone(),
        };
        match jobs::copy_image(self.out.clone(), job) {
            Ok(()) => {
                self.copying = Some(run);
                if slow {
                    ui.toast("Copying image...", now);
                }
            }
            Err(_) => ui.toast("Could not start copying the image", now),
        }
    }

    /// The pixels an image copy takes: what Save As would write (the
    /// current picture with its edits), or for an unedited animation the
    /// frame on screen.
    fn copy_source(&self, shown: &Shown) -> ImageBuf {
        if let (true, Content::Animation(anim)) = (shown.edits.is_empty(), &*shown.content) {
            let frame = self
                .frame_on_screen
                .filter(|(generation, _)| shown.full_gen.is_some_and(|g| *generation >= g))
                .map_or(0, |(_, frame)| frame);
            if let Some(f) = anim.frames().get(frame) {
                return f.image.clone();
            }
        }
        shown.current().clone()
    }

    fn copy_path(&mut self, ui: &mut Ui, now: Duration) -> Handled {
        self.drop_image_copy();
        let path = match (&self.shown, &self.target) {
            (Some(s), _) => s.path.clone(),
            (None, Some(t)) => t.path.clone(),
            (None, None) => {
                ui.toast("Open an image first", now);
                return Handled::Ignored;
            }
        };
        match self.clipboard.set_text(&display_path(&path)) {
            Ok(()) => ui.toast("Path copied", now),
            Err(_) => ui.toast("Could not copy the path", now),
        }
        Handled::Done
    }

    fn run_bulk(&mut self, job: pandite_ui::BulkJob, ui: &mut Ui, now: Duration) -> Handled {
        if self.bulk.is_some() {
            ui.toast("A bulk job is already running", now);
            return Handled::Ignored;
        }
        if job.source.is_empty() {
            ui.toast("Choose images to convert", now);
            return Handled::Ignored;
        }
        let id = self.next_id();
        let cancel = CancelToken::new();
        let spawned = jobs::bulk(
            self.out.clone(),
            Arc::clone(&self.disk),
            Arc::clone(&self.registry),
            id,
            job,
            cancel.clone(),
        );
        if spawned.is_err() {
            ui.toast("Could not start the bulk job", now);
            return Handled::Ignored;
        }
        self.bulk = Some(BulkRun {
            id,
            cancel,
            done: 0,
            total: 0,
            label: "Preparing".into(),
        });
        Handled::Done
    }
}

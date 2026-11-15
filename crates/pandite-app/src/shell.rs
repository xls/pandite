//! The window shell: winit event loop, wgpu renderer and native services
//! around the application core.
//!
//! The shell owns the window and the [`Renderer`]. It translates window
//! events into [`InputEvent`]s for the [`App`], wakes the core when workers
//! queue results, uploads new content on a worker thread through the
//! renderer's [`pandite_render::ContentFactory`], and draws frames only
//! when something changed: after input, after a wake, at animation
//! deadlines and when the interface asks for a timed frame. Between those
//! the event loop sleeps (`ControlFlow::Wait` or `WaitUntil`).
//!
//! On Windows the first pandite of a user session stays when its window
//! closes (see [`crate::resident`]): it ends the core, drops the renderer
//! and with them every image and texture, hides the window and keeps the
//! open GPU device and the thumbnail pixels of the last folder (for at
//! most 30 minutes). The next image opens in that process without the
//! start-up cost, and its folder's gallery without decoding again.

#[cfg(target_os = "macos")]
mod finder;
mod input;
mod native;
mod smoke;
mod timing;

use crate::app::{App, ContentUpdate, Platform, Startup, WindowState};
use crate::cli::Cli;
use crate::console;
use crate::disk::{Disk, StdDisk};
use crate::platform::Waker;
use crate::resident::{self, Reply, Request};
use crate::settings::{self, ensure_on_screen, first_open_rect, Settings, ThemeChoice};
use pandite_core::{Content, DrawList, Point, Rect, Size};
use pandite_library::KeptThumbnails;
use pandite_render::{
    Checkerboard, FrameStatus, Gpu, GpuOptions, PreparedContent, RenderInput, RenderSettings,
    Renderer, TextMeasurer,
};
use pandite_ui::{InputEvent, ThemeKind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, Ime, StartCause, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::window::{Fullscreen, Icon, Theme, Window, WindowId};

/// Exit code of a smoke run without a usable GPU.
pub const EXIT_NO_GPU: i32 = 3;

/// How long the settings file may take to load before defaults are used.
const SETTINGS_LOAD_WAIT: Duration = Duration::from_millis(400);

/// How long a window whose GPU is ready stays hidden for its image, so
/// that it appears with the image in it. A file that takes longer shows
/// the loading screen.
const SHOW_WAIT: Duration = Duration::from_millis(150);

/// How long writing the settings at exit may take.
const SETTINGS_SAVE_WAIT: Duration = Duration::from_secs(2);

/// How long the program in the background keeps the thumbnails of the
/// last folder when no image opens.
const KEEP_THUMBNAILS: Duration = Duration::from_secs(30 * 60);

/// Events sent to the event loop from other threads.
pub enum UserEvent {
    /// Workers queued results for the core.
    Wake,
    /// The GPU device opened on its start-up thread.
    GpuReady(Result<Arc<Gpu>, String>),
    /// Another pandite process asks the one in the background.
    Resident(Request),
    /// An upload finished on the upload thread.
    Prepared {
        /// The renderer the upload was made for.
        epoch: u64,
        /// The content generation it belongs to.
        generation: u64,
        /// True when the content is the full resolution image.
        full: bool,
        /// The GPU content, or why it failed.
        result: Result<PreparedContent, String>,
    },
}

/// Load settings on a worker, waiting a short while. A settings folder
/// that does not answer (a stalled network profile) gives the defaults,
/// and then nothing is saved at exit either.
fn load_settings(path: Option<PathBuf>) -> (Settings, Option<PathBuf>) {
    let Some(path) = path else {
        return (Settings::default(), None);
    };
    let (tx, rx) = mpsc::channel();
    let file = path.clone();
    let spawned = std::thread::Builder::new()
        .name("pandite-settings-load".into())
        .spawn(move || {
            let _ = tx.send(settings::load(&StdDisk, &file));
        });
    if spawned.is_err() {
        return (Settings::default(), None);
    }
    match rx.recv_timeout(SETTINGS_LOAD_WAIT) {
        Ok((settings, _note)) => (settings, Some(path)),
        Err(_) => (Settings::default(), None),
    }
}

/// True when this command line may use the pandite that waits in the
/// background, and may become it. Test and timing runs and runs with
/// their own settings folder stand alone.
pub fn may_stay(cli: &Cli) -> bool {
    cfg!(windows) && cli.smoke.is_none() && !cli.timing && cli.config_dir.is_none()
}

/// Become the program that waits in the background, when no other process
/// is. `dormant` is true while it shows no window.
fn become_resident(proxy: &EventLoopProxy<UserEvent>, dormant: &Arc<AtomicBool>) -> bool {
    let proxy = Mutex::new(proxy.clone());
    let dormant = Arc::clone(dormant);
    let send = move |request: Request| {
        proxy
            .lock()
            .is_ok_and(|p| p.send_event(UserEvent::Resident(request)).is_ok())
    };
    resident::listen(move |request| match request {
        Request::Ping => Reply::Ok,
        Request::Quit => {
            send(Request::Quit);
            Reply::Ok
        }
        // Only a hidden program takes the image; one that shows a window
        // leaves the new process to open its own.
        open @ Request::Open { .. } => {
            if dormant.swap(false, Ordering::AcqRel) && send(open) {
                Reply::Ok
            } else {
                Reply::Busy
            }
        }
    })
}

/// Run the viewer until the window closes. Returns the process exit code.
pub fn run(cli: Cli) -> i32 {
    let start = Instant::now();
    let (settings, settings_path) = if cli.smoke.is_some() {
        // Smoke runs never read or write the user's settings.
        (Settings::default(), None)
    } else {
        load_settings(settings::settings_path(cli.config_dir.as_deref()))
    };
    let event_loop = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(err) => {
            console::err(&format!("pandite: cannot start the window system: {err}"));
            return if cli.smoke.is_some() { EXIT_NO_GPU } else { 1 };
        }
    };
    let proxy = event_loop.create_proxy();
    #[cfg(target_os = "macos")]
    {
        // Finder sends a file as an event, not on the command line.
        let proxy = proxy.clone();
        finder::install(move |path| {
            let _ = proxy.send_event(UserEvent::Resident(Request::Open {
                path: Some(path),
                launched: None,
            }));
        });
    }
    let dormant = Arc::new(AtomicBool::new(false));
    let stays = may_stay(&cli) && settings.stay_ready && become_resident(&proxy, &dormant);
    if cli.background && !stays {
        // Nothing to wait for: another process does, or the setting is off.
        return 0;
    }
    if stays {
        // Do not keep the folder of the first image in use.
        let _ = std::env::set_current_dir(std::env::temp_dir());
    }
    let early_gpu = start_gpu(&proxy, start, gpu_options(&settings));
    let mut shell = Shell::new(cli, start, settings, settings_path, proxy);
    shell.early_gpu = early_gpu;
    shell.resident = stays;
    shell.dormant = dormant;
    if let Err(err) = event_loop.run_app(&mut shell) {
        console::err(&format!("pandite: event loop failed: {err}"));
        shell.exit_code = shell.exit_code.max(1);
    }
    shell.finish()
}

/// GPU options for this start. A discrete GPU seen at the last start (or
/// demanded with `PANDITE_EXPECT_DISCRETE=1`, which test runs use because
/// they read no settings) is waited for briefly when it is not listed.
fn gpu_options(settings: &Settings) -> GpuOptions {
    let demanded = std::env::var_os("PANDITE_EXPECT_DISCRETE").is_some_and(|v| v == "1");
    GpuOptions {
        expect_discrete: settings.gpu_discrete_seen || demanded,
        ..GpuOptions::default()
    }
}

/// Open the GPU device on a worker while the window is created and shown,
/// and build the shared pipelines with a tiny offscreen frame. The two
/// steps take most of the start-up time, so running them in parallel with
/// the window brings the first frame forward. Only on Windows: a DirectX 12
/// device can present to any window, and other platforms want surface
/// work on the main thread. Returns true when the worker started.
fn start_gpu(proxy: &EventLoopProxy<UserEvent>, start: Instant, options: GpuOptions) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let proxy = proxy.clone();
    let trace = std::env::var_os("PANDITE_TRACE").is_some();
    let mark = move |what: &str| {
        if trace {
            console::err(&format!(
                "pandite trace: {what} at {:.1} ms",
                start.elapsed().as_secs_f64() * 1000.0
            ));
        }
    };
    std::thread::Builder::new()
        .name("pandite-gpu-start".into())
        .spawn(move || {
            let result = Gpu::new(&options).inspect(|gpu| {
                mark("GPU device open");
                if let Ok(mut warm) =
                    Renderer::offscreen(Arc::clone(gpu), 4, 4, RenderSettings::default())
                {
                    let view = pandite_core::ViewState::default();
                    let _ = warm.render(&view, warm.full_viewport(), &DrawList::new());
                }
                mark("GPU pipelines built");
                // Staging memory for the first image's upload, paged in now.
                gpu.prepare_uploads();
                mark("GPU ready");
            });
            let _ = proxy.send_event(UserEvent::GpuReady(result.map_err(|e| e.to_string())));
        })
        .is_ok()
}

/// Content uploads: one runs on a worker at a time; the newest waiting
/// update replaces older ones.
#[derive(Default)]
struct Uploads {
    /// Counts renderers; results made for an earlier one are dropped.
    epoch: u64,
    in_flight: Option<u64>,
    queued: Option<ContentUpdate>,
    latest: u64,
    shown: u64,
    /// The shown generation is the image at full resolution.
    shown_full: bool,
}

struct Shell {
    cli: Cli,
    start: Instant,
    settings: Settings,
    settings_path: Option<PathBuf>,
    proxy: EventLoopProxy<UserEvent>,
    wake_pending: Arc<AtomicBool>,
    /// Set by a late look at the adapter list that found a discrete GPU.
    discrete_late: Arc<AtomicBool>,
    /// The GPU in use is a discrete one.
    gpu_discrete: bool,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    measurer: Option<TextMeasurer>,
    app: Option<App>,
    uploads: Uploads,
    content: Option<Content>,
    list: DrawList,
    cursor: Point,
    mods: pandite_ui::Modifiers,
    ime_preedit: bool,
    last_key_text: Option<String>,
    pending_drop: Option<PathBuf>,
    last_frame: Option<Instant>,
    anim_deadline: Option<Instant>,
    minimized: bool,
    occluded: bool,
    scale: f64,
    first_frame: bool,
    visible: bool,
    early_gpu: bool,
    /// The GPU device while no renderer uses it: before the first window
    /// session and between sessions.
    early_result: Option<Result<Arc<Gpu>, String>>,
    /// This process stays in the background when its window closes.
    resident: bool,
    /// True while the process waits without a window session.
    dormant: Arc<AtomicBool>,
    /// Decoders and encoders, kept between window sessions.
    registry: Option<Arc<pandite_core::Registry>>,
    /// Thumbnails of the last folder, kept between window sessions.
    kept: Option<KeptThumbnails>,
    /// When the kept thumbnails are freed if no image opens before.
    kept_until: Option<Instant>,
    /// When the process that asked for the current image started.
    launched: Option<SystemTime>,
    image_presented: bool,
    /// The image load started before the first frame.
    early_load: bool,
    /// The hidden window waits for its image until then.
    show_at: Option<Instant>,
    /// Take the keyboard focus when the window shows.
    focus_on_show: bool,
    last_recreate: Option<Instant>,
    image_on_screen: bool,
    trace: bool,
    exit_code: i32,
    smoke: Option<smoke::Smoke>,
    stall: Option<Arc<smoke::StallDisk>>,
    timing: Option<timing::Timing>,
    timing_printed: bool,
}

fn theme_kind(theme: Theme) -> ThemeKind {
    match theme {
        Theme::Dark => ThemeKind::Dark,
        Theme::Light => ThemeKind::Light,
    }
}

fn window_icon(size: u32) -> Option<Icon> {
    let (rgba, side) = crate::icon::window_icon_rgba(size)?;
    Icon::from_rgba(rgba, side, side).ok()
}

impl Shell {
    fn new(
        cli: Cli,
        start: Instant,
        settings: Settings,
        settings_path: Option<PathBuf>,
        proxy: EventLoopProxy<UserEvent>,
    ) -> Self {
        let smoke = cli
            .smoke
            .clone()
            .map(|path| smoke::Smoke::new(path, cli.smoke_stall, start));
        let stall = cli
            .smoke_stall
            .then(|| Arc::new(smoke::StallDisk::default()));
        let timing = cli
            .timing
            .then(|| cli.smoke.clone().or_else(|| cli.path.clone()))
            .flatten()
            .map(timing::Timing::new);
        Self {
            cli,
            start,
            settings,
            settings_path,
            proxy,
            wake_pending: Arc::new(AtomicBool::new(false)),
            discrete_late: Arc::new(AtomicBool::new(false)),
            gpu_discrete: false,
            window: None,
            renderer: None,
            measurer: None,
            app: None,
            uploads: Uploads::default(),
            content: None,
            list: DrawList::new(),
            cursor: Point::default(),
            mods: pandite_ui::Modifiers::NONE,
            ime_preedit: false,
            last_key_text: None,
            pending_drop: None,
            last_frame: None,
            anim_deadline: None,
            minimized: false,
            occluded: false,
            scale: 1.0,
            first_frame: false,
            visible: false,
            early_gpu: false,
            early_result: None,
            resident: false,
            dormant: Arc::new(AtomicBool::new(false)),
            registry: None,
            kept: None,
            kept_until: None,
            launched: Some(resident::launch_time()),
            image_presented: false,
            early_load: false,
            show_at: None,
            focus_on_show: false,
            last_recreate: None,
            image_on_screen: false,
            trace: std::env::var_os("PANDITE_TRACE").is_some(),
            exit_code: 0,
            smoke,
            stall,
            timing,
            timing_printed: false,
        }
    }

    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    /// With `PANDITE_TRACE` set, report startup milestones on standard
    /// error.
    fn trace(&self, what: &str) {
        if self.trace {
            console::err(&format!(
                "pandite trace: {what} at {:.1} ms",
                self.start.elapsed().as_secs_f64() * 1000.0
            ));
        }
    }

    /// Like [`Shell::trace`], with the time since the user started the
    /// process that asked for the image.
    fn trace_launch(&self, what: &str) {
        if self.trace {
            let since = self
                .launched
                .and_then(|at| SystemTime::now().duration_since(at).ok())
                .map_or(0.0, |d| d.as_secs_f64() * 1000.0);
            self.trace(&format!("{what} ({since:.1} ms after launch)"));
        }
    }

    fn waker(&self) -> Waker {
        let proxy = Mutex::new(self.proxy.clone());
        let pending = Arc::clone(&self.wake_pending);
        Arc::new(move || {
            // One wake event at a time: the core drains everything queued.
            if !pending.swap(true, Ordering::AcqRel) {
                if let Ok(proxy) = proxy.lock() {
                    let _ = proxy.send_event(UserEvent::Wake);
                }
            }
        })
    }

    fn render_settings(&self, background: pandite_core::Color) -> RenderSettings {
        RenderSettings {
            background,
            checkerboard: Some(Checkerboard {
                cell: (8.0 * self.scale as f32).round().max(4.0),
                ..Checkerboard::default()
            }),
            ..RenderSettings::default()
        }
    }

    /// Create the window, then start the first window session (unless the
    /// program starts in the background).
    fn create(&mut self, event_loop: &ActiveEventLoop) -> Result<(), (String, i32)> {
        let monitors = native::monitors(
            event_loop.primary_monitor(),
            event_loop.available_monitors(),
        );
        let saved = if self.smoke.is_some() {
            None
        } else {
            self.settings.window.map(|p| ensure_on_screen(p, &monitors))
        };
        let theme = match self.settings.theme {
            ThemeChoice::System => None,
            ThemeChoice::Dark => Some(Theme::Dark),
            ThemeChoice::Light => Some(Theme::Light),
        };
        let mut attrs = Window::default_attributes()
            .with_title("Pandite")
            .with_visible(false)
            .with_min_inner_size(LogicalSize::new(320.0, 240.0))
            .with_theme(theme)
            .with_window_icon(window_icon(32));
        #[cfg(windows)]
        {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs = attrs.with_taskbar_icon(window_icon(64));
        }
        #[cfg(all(unix, not(target_vendor = "apple")))]
        {
            use winit::platform::x11::WindowAttributesExtX11;
            attrs = attrs.with_name("pandite", "pandite");
        }
        let primary_work = monitors.first().map(|m| m.work);
        let mut first_rect = None;
        if let Some(p) = saved {
            // winit applies maximized and full screen after creating the
            // window, so the rectangle stays the restored one.
            attrs = attrs
                .with_position(PhysicalPosition::new(p.x, p.y))
                .with_inner_size(PhysicalSize::new(p.width, p.height))
                .with_maximized(p.maximized && !p.fullscreen)
                .with_fullscreen(p.fullscreen.then_some(Fullscreen::Borderless(None)));
        } else if let Some(work) = primary_work {
            let s = event_loop
                .primary_monitor()
                .map_or(1.0, |m| m.scale_factor()) as f32;
            let min = Size::new(480.0 * s, 360.0 * s);
            let rect = first_open_rect(work, None, self.settings.first_open_fraction, 0.0, min);
            attrs = attrs.with_inner_size(PhysicalSize::new(rect.w as u32, rect.h as u32));
            first_rect = Some(rect);
        }
        let window = event_loop
            .create_window(attrs)
            .map(Arc::new)
            .map_err(|e| (format!("cannot create the window: {e}"), 1))?;
        self.trace("window created");
        // The window appears and disappears at once, without the system's
        // open and close animation.
        native::disable_transitions(&window);
        if let Some(rect) = first_rect {
            native::place_client(&window, rect);
        }
        if let Some(p) = saved.filter(|p| !p.maximized && !p.fullscreen) {
            // A window created on a monitor with another scale factor and
            // moved to the saved one was rescaled on the way: put back the
            // saved physical size and position.
            let size = window.inner_size();
            if (size.width, size.height) != (p.width, p.height) {
                let _ = window.request_inner_size(PhysicalSize::new(p.width, p.height));
            }
            if window.outer_position().ok() != Some(PhysicalPosition::new(p.x, p.y)) {
                window.set_outer_position(PhysicalPosition::new(p.x, p.y));
            }
        }
        self.window = Some(window);
        if self.cli.background {
            self.trace("waiting in the background");
            self.dormant.store(true, Ordering::Release);
            return Ok(());
        }
        self.start_session()
    }

    /// Start the core for the window and draw the first frame, or show
    /// the window in the theme color until the GPU is ready.
    fn start_session(&mut self) -> Result<(), (String, i32)> {
        let Some(window) = self.window.clone() else {
            return Ok(());
        };
        self.scale = window.scale_factor();
        let size = window.inner_size();
        let system_theme = window.theme().map(theme_kind);
        let fullscreen = window.fullscreen().is_some();
        let maximized = window.is_maximized();
        let placement = match (window.outer_position(), size) {
            (Ok(pos), s) if s.width > 0 && s.height > 0 => Some(Rect::new(
                pos.x as f32,
                pos.y as f32,
                s.width as f32,
                s.height as f32,
            )),
            _ => None,
        };
        // Decoders, encoders and the platform text recognizer (Windows OCR,
        // or the built-in models on macOS and Linux).
        let registry = match &self.registry {
            Some(registry) => Arc::clone(registry),
            None => {
                let mut registry = pandite_codecs::default_registry();
                pandite_ocr::register_text_providers(&mut registry);
                let registry = Arc::new(registry);
                self.registry = Some(Arc::clone(&registry));
                registry
            }
        };
        let disk: Arc<dyn Disk> = match &self.stall {
            Some(stall) => Arc::clone(stall) as Arc<dyn Disk>,
            None => Arc::new(StdDisk),
        };
        let open = self.cli.smoke.clone().or_else(|| self.cli.path.clone());
        let size_to_image = self.settings.window.is_none() || self.settings.size_window_to_image;
        let startup = Startup {
            registry,
            settings: self.settings.clone(),
            settings_path: self.settings_path.clone(),
            open,
            window: WindowState {
                size: Size::new(size.width as f32, size.height as f32),
                scale: self.scale as f32,
                system_theme,
                placement,
                maximized,
                fullscreen,
            },
            size_to_image,
            // The library drops them when the image is in another folder.
            thumbnails: self.kept.take(),
        };
        self.kept_until = None;
        let platform = Platform {
            disk,
            clipboard: Box::new(native::SystemClipboard::default()),
            dialogs: Box::new(native::NativeDialogs::new(Arc::clone(&window))),
            window: Box::new(native::NativeWindow::new(Arc::clone(&window))),
            waker: self.waker(),
        };
        let app = App::new(startup, platform)
            .map_err(|e| (format!("cannot start the image library: {e}"), 1))?;
        let background = app.background();
        self.app = Some(app);
        self.trace("core started");
        if self.early_result.is_some() && self.smoke.is_none() && self.timing.is_none() {
            // The GPU is ready and the image comes first: it is read and
            // decoded while the renderer starts. Not while the GPU still
            // opens (the decoders slow that down by more than they gain),
            // and not in test and timing runs, which keep the measured
            // order (the load starts with the first frame).
            let now = self.now();
            if let Some(app) = self.app.as_mut() {
                app.first_frame_shown(now);
                self.early_load = app.loading_path().is_some();
            }
        }
        if self.early_gpu {
            native::paint_background(&window, background);
            if let Some(result) = self.early_result.take() {
                // The GPU is ready (always so when the program waited in
                // the background): the window shows with its first frame.
                return self.gpu_ready(result);
            }
            // The GPU opens on its worker: show the window now, painted in
            // the theme color, and attach the renderer when it is ready.
            window.set_visible(true);
            self.visible = true;
            self.trace("window shown");
            return Ok(());
        }
        let renderer = Renderer::for_window(
            Arc::clone(&window),
            size.width,
            size.height,
            &gpu_options(&self.settings),
            self.render_settings(background),
        )
        .map_err(|e| (format!("no usable GPU: {e}"), EXIT_NO_GPU))?;
        self.attach_renderer(renderer);
        Ok(())
    }

    /// Use `renderer` for the window and draw the first frame.
    fn attach_renderer(&mut self, renderer: Renderer) {
        let Some(window) = self.window.clone() else {
            return;
        };
        self.trace("renderer ready");
        if self.timing.is_some() {
            renderer.gpu().set_measure_uploads(true);
        }
        self.measurer = Some(renderer.text_measurer());
        self.note_gpu(&renderer);
        self.renderer = Some(renderer);
        if let Some(smoke) = &mut self.smoke {
            smoke.window_ready();
        }
        if let Some(update) = self.uploads.queued.take() {
            // The image was decoded before the renderer was ready.
            self.start_upload(update);
        }
        self.flush_content();
        if self.visible {
            self.redraw();
            self.trace("first frame");
        } else if self.early_load {
            // The image is on its way: draw once while hidden (glyphs get
            // built) and show the window when the image is in it, or
            // after a short wait with the loading screen.
            self.redraw();
            self.trace("hidden frame");
            self.show_at = Some(Instant::now() + SHOW_WAIT);
            window.request_redraw();
        } else {
            self.show();
        }
        self.flush_content();
    }

    /// Show the window with a frame in it, so it never appears blank.
    fn show(&mut self) {
        self.show_at = None;
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.visible {
            return;
        }
        // Draw once while hidden (pipelines and glyphs get built), then
        // show the window and draw it at once.
        self.redraw();
        window.set_visible(true);
        self.visible = true;
        self.trace("window shown");
        if std::mem::take(&mut self.focus_on_show) {
            window.focus_window();
        }
        self.redraw();
        self.trace("first frame");
    }

    /// The GPU worker finished: make the window's renderer from its device,
    /// or open the GPU the usual way when that fails.
    fn gpu_ready(&mut self, result: Result<Arc<Gpu>, String>) -> Result<(), (String, i32)> {
        let (Some(window), Some(app)) = (self.window.clone(), self.app.as_ref()) else {
            return Ok(());
        };
        let settings = self.render_settings(app.background());
        let size = window.inner_size();
        let early = result.and_then(|gpu| {
            let surface = gpu
                .instance()
                .create_surface(Arc::clone(&window))
                .map_err(|e| e.to_string())?;
            Renderer::with_surface(gpu, surface, size.width, size.height, settings.clone())
                .map_err(|e| e.to_string())
        });
        let renderer = match early {
            Ok(renderer) => renderer,
            Err(_) => Renderer::for_window(
                Arc::clone(&window),
                size.width,
                size.height,
                &gpu_options(&self.settings),
                settings,
            )
            .map_err(|e| (format!("no usable GPU: {e}"), EXIT_NO_GPU))?,
        };
        self.attach_renderer(renderer);
        Ok(())
    }

    /// The window closed and the process stays: end the core, free every
    /// image and texture, hide the window and keep the GPU device and the
    /// thumbnails of the last folder for the next image.
    fn sleep(&mut self) {
        if let Some(window) = &self.window {
            window.set_visible(false);
        }
        self.visible = false;
        self.sync_gpu_seen();
        if let Some(app) = self.app.take() {
            self.settings = app.settings();
            self.kept = app.shutdown(SETTINGS_SAVE_WAIT).thumbnails;
            self.kept_until = self.kept.as_ref().map(|_| Instant::now() + KEEP_THUMBNAILS);
        }
        self.measurer = None;
        self.content = None;
        self.list = DrawList::new();
        // An upload still running belongs to the renderer dropped here.
        self.uploads = Uploads {
            epoch: self.uploads.epoch + 1,
            ..Uploads::default()
        };
        if let Some(renderer) = self.renderer.take() {
            let gpu = Arc::clone(renderer.gpu());
            drop(renderer);
            gpu.release_memory();
            if !gpu.is_lost() {
                self.early_result = Some(Ok(gpu));
            }
        }
        self.pending_drop = None;
        self.last_frame = None;
        self.anim_deadline = None;
        self.minimized = false;
        self.occluded = false;
        self.first_frame = false;
        self.image_on_screen = false;
        self.image_presented = false;
        self.early_load = false;
        self.show_at = None;
        self.focus_on_show = false;
        // The kept thumbnails stay allocated through the trim: the system
        // only moves their pages out of the working set.
        resident::trim_memory();
        self.trace("waiting in the background");
        self.dormant.store(true, Ordering::Release);
    }

    /// Another pandite process handed over its image.
    fn wake(
        &mut self,
        event_loop: &ActiveEventLoop,
        path: Option<PathBuf>,
        at: Option<SystemTime>,
    ) {
        self.dormant.store(false, Ordering::Release);
        let Some(window) = self.window.clone() else {
            // The request came before the window: open the image with it.
            self.cli.background = false;
            self.cli.path = path;
            return;
        };
        if let Some(app) = self.app.as_mut() {
            // A window session runs (the request raced with one starting).
            if let Some(path) = path {
                app.open(path, self.start.elapsed());
            }
            window.focus_window();
            return;
        }
        // The session clock starts over, as in a new process.
        self.start = Instant::now();
        self.launched = at;
        self.cli.path = path;
        self.trace_launch("woke up");
        window.set_minimized(false);
        self.focus_on_show = true;
        if let Err((message, code)) = self.start_session() {
            self.fail_start(event_loop, message, code);
            return;
        }
        if self.visible && std::mem::take(&mut self.focus_on_show) {
            window.focus_window();
        }
    }

    /// Report a start-up failure and leave.
    fn fail_start(&mut self, event_loop: &ActiveEventLoop, message: String, code: i32) {
        console::err(&format!("pandite: {message}"));
        if let Some(smoke) = &mut self.smoke {
            smoke.fail(message.clone());
        } else {
            let _ = rfd::MessageDialog::new()
                .set_title("Pandite")
                .set_description(format!("Pandite cannot start: {message}"))
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
        self.exit_code = code;
        event_loop.exit();
    }

    /// Hand new content to the upload thread, or clear it at once.
    fn flush_content(&mut self) {
        while let Some(update) = self.app.as_mut().and_then(App::take_content) {
            self.uploads.latest = self.uploads.latest.max(update.generation);
            match update.content {
                None => {
                    if let Some(r) = self.renderer.as_mut() {
                        r.clear_content();
                    }
                    self.content = None;
                    self.uploads.shown = update.generation;
                    self.uploads.shown_full = false;
                    self.uploads.queued = None;
                }
                Some(_) => {
                    if self.uploads.in_flight.is_some() || self.renderer.is_none() {
                        // An upload runs, or the GPU is not ready yet: the
                        // newest image waits.
                        self.uploads.queued = Some(update);
                    } else {
                        self.start_upload(update);
                    }
                }
            }
        }
    }

    fn start_upload(&mut self, update: ContentUpdate) {
        let (Some(renderer), Some(content)) = (self.renderer.as_mut(), update.content) else {
            return;
        };
        let generation = update.generation;
        let full = update.full_resolution;
        let epoch = self.uploads.epoch;
        let factory = renderer.content_factory();
        let proxy = self.proxy.clone();
        // A reduced stand-in is drawn at the full image's size.
        let input = match update.logical {
            Some((w, h)) => RenderInput::Reduced {
                content: content.clone(),
                size: Size::new(w as f32, h as f32),
            },
            None => RenderInput::Content(content.clone()),
        };
        let spawned = std::thread::Builder::new()
            .name("pandite-upload".into())
            .spawn(move || {
                let result = factory.create(&input).map_err(|e| e.to_string());
                let _ = proxy.send_event(UserEvent::Prepared {
                    epoch,
                    generation,
                    full,
                    result,
                });
            });
        match spawned {
            Ok(_) => {
                self.uploads.in_flight = Some(generation);
                self.content = Some(content);
            }
            Err(_) => {
                // No thread: upload here instead (full resolution only; a
                // stand-in is skipped, its full image follows).
                if update.logical.is_none() && renderer.set_content(content.clone()).is_ok() {
                    self.uploads.shown = generation;
                    self.uploads.shown_full = full;
                    self.content = Some(content);
                    self.last_frame = None;
                }
            }
        }
    }

    fn prepared(
        &mut self,
        epoch: u64,
        generation: u64,
        full: bool,
        result: Result<PreparedContent, String>,
    ) {
        if epoch != self.uploads.epoch {
            // Made for a renderer that no longer exists.
            return;
        }
        self.uploads.in_flight = None;
        if let Some(next) = self.uploads.queued.take() {
            // A newer image is waiting; this one is already stale.
            self.start_upload(next);
            return;
        }
        if generation < self.uploads.latest {
            return;
        }
        match result {
            Ok(prepared) => {
                if let Some(renderer) = self.renderer.as_mut() {
                    if let Some(timing) = &mut self.timing {
                        timing.prepared(generation, full, prepared.upload_timings());
                    }
                    renderer.set_prepared(prepared);
                    self.uploads.shown = generation;
                    self.uploads.shown_full = full;
                    self.last_frame = None;
                    if !self.image_on_screen {
                        self.image_on_screen = true;
                        self.trace("first image uploaded");
                    }
                    if self.show_at.is_some() {
                        // The window waited for this image.
                        self.show();
                    }
                }
            }
            Err(err) => {
                console::err(&format!("pandite: cannot show the image: {err}"));
                if let Some(smoke) = &mut self.smoke {
                    smoke.fail(format!("upload failed: {err}"));
                }
            }
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// Replace a renderer whose GPU device was lost.
    fn recreate_renderer(&mut self) {
        let Some(background) = self.app.as_ref().map(App::background) else {
            return;
        };
        let settings = self.render_settings(background);
        let (Some(window), Some(app)) = (self.window.clone(), self.app.as_mut()) else {
            return;
        };
        let size = window.inner_size();
        match Renderer::for_window(
            Arc::clone(&window),
            size.width,
            size.height,
            &gpu_options(&self.settings),
            settings,
        ) {
            Ok(renderer) => {
                self.measurer = Some(renderer.text_measurer());
                self.renderer = Some(renderer);
                self.uploads.epoch += 1;
                self.uploads.in_flight = None;
                self.uploads.queued = None;
                app.gpu_reset();
                self.flush_content();
                window.request_redraw();
            }
            Err(err) => {
                console::err(&format!("pandite: the GPU stopped working: {err}"));
                self.renderer = None;
                if let Some(smoke) = &mut self.smoke {
                    smoke.fail(format!("GPU lost: {err}"));
                } else {
                    let _ = rfd::MessageDialog::new()
                        .set_title("Pandite")
                        .set_description(format!("The graphics device stopped working: {err}"))
                        .set_level(rfd::MessageLevel::Error)
                        .show();
                }
                app.request_exit();
                self.exit_code = 1;
            }
        }
    }

    fn redraw(&mut self) {
        let now = self.now();
        let (Some(app), Some(renderer), Some(measurer)) = (
            self.app.as_mut(),
            self.renderer.as_mut(),
            self.measurer.as_ref(),
        ) else {
            return;
        };
        let t = Instant::now();
        let dt = self.last_frame.map_or(Duration::ZERO, |last| t - last);
        let advanced = self.anim_deadline.is_some_and(|d| t >= d);
        self.last_frame = Some(t);
        self.anim_deadline = renderer.update(dt).map(|d| t + d);
        if let Some(frame) = renderer.content_frame() {
            // An image copy of an animation takes the frame on screen.
            app.frame_on_screen(self.uploads.shown, frame);
        }
        app.sync_textures(now, renderer);
        self.list.clear();
        let out = app.frame(now, measurer, &mut self.list);
        if renderer.settings().background != out.background {
            let mut settings = renderer.settings().clone();
            settings.background = out.background;
            renderer.set_settings(settings);
        }
        let status = renderer.render(&out.view, out.viewport, &self.list);
        if let Some(err) = renderer.take_texture_error() {
            console::err(&format!("pandite: {err}"));
        }
        let lost = renderer.gpu().is_lost();
        let mut presented = false;
        match status {
            Ok(status) => {
                let current = self.uploads.shown == self.uploads.latest && self.uploads.latest > 0;
                if let (Some(timing), true) = (&mut self.timing, status == FrameStatus::Drawn) {
                    if current {
                        let start = self.start;
                        timing.presented(self.uploads.shown, renderer, app, || start.elapsed());
                    }
                }
                if let Some(smoke) = &mut self.smoke {
                    smoke.frame(
                        status == FrameStatus::Drawn,
                        advanced,
                        app,
                        renderer,
                        current && self.uploads.shown_full,
                    );
                }
                if status == FrameStatus::Drawn && current && self.uploads.shown_full {
                    // The full image is on screen: background work that
                    // waited for it (text recognition) may start.
                    app.content_presented(self.uploads.shown, self.start.elapsed());
                    presented = self.visible;
                }
            }
            Err(err) => {
                let recent = self
                    .last_recreate
                    .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
                if !lost {
                    console::err(&format!("pandite: drawing failed: {err}"));
                }
                if let Some(smoke) = &mut self.smoke {
                    smoke.fail(format!("render failed: {err}"));
                } else if lost || !recent {
                    // A lost device, or a surface the renderer cannot
                    // rebuild itself: start over with a new renderer.
                    self.last_recreate = Some(Instant::now());
                    self.recreate_renderer();
                }
            }
        }
        if presented && !self.image_presented {
            self.image_presented = true;
            self.trace_launch("image on screen");
        }
        if self.smoke.is_none() && !self.timing_printed {
            if let Some(line) = self.timing.as_ref().and_then(timing::Timing::line) {
                console::out(line);
                self.timing_printed = true;
            }
        }
        if !self.first_frame && self.visible {
            // The window is on screen: only now may the startup image load.
            self.first_frame = true;
            if let Some(app) = self.app.as_mut() {
                app.first_frame_shown(self.start.elapsed());
            }
        }
    }

    fn input(&mut self, event: InputEvent) {
        let now = self.now();
        if let Some(app) = self.app.as_mut() {
            app.input(event, now);
        }
    }

    fn keyboard(&mut self, event: winit::event::KeyEvent) {
        let key = input::key(&event);
        let mods = self.mods;
        match event.state {
            ElementState::Pressed => {
                self.input(InputEvent::KeyDown {
                    key,
                    mods,
                    repeat: event.repeat,
                });
                let wants_text = self.app.as_ref().is_some_and(|a| a.ui().wants_text_input());
                // Ctrl+Alt is AltGr on many layouts and types characters.
                let shortcut = mods.ctrl && !mods.alt;
                if wants_text && !shortcut && !self.ime_preedit {
                    if let Some(text) = input::typed_text(&event) {
                        self.last_key_text = Some(text.clone());
                        self.input(InputEvent::Text(text));
                    }
                }
            }
            ElementState::Released => self.input(InputEvent::KeyUp { key, mods }),
        }
    }

    /// Remember a discrete GPU, so the next start waits for it when the
    /// system is slow to list it. Forget it only when this start waited
    /// for it in vain. A start that did not expect one and got another
    /// GPU looks again a little later, off the main thread.
    fn note_gpu(&mut self, renderer: &Renderer) {
        let discrete = renderer.gpu().adapter_summary().device_type
            == pandite_render::wgpu::DeviceType::DiscreteGpu;
        self.gpu_discrete = discrete;
        if discrete || self.settings.gpu_discrete_seen {
            self.settings.gpu_discrete_seen = discrete;
        } else if self.smoke.is_none() {
            let late = Arc::clone(&self.discrete_late);
            let _ = std::thread::Builder::new()
                .name("pandite-gpu-list".into())
                .spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    let listed = pandite_render::list_adapters(&GpuOptions::default())
                        .iter()
                        .any(|a| a.device_type == pandite_render::wgpu::DeviceType::DiscreteGpu);
                    late.store(listed, Ordering::Release);
                });
        }
        self.sync_gpu_seen();
    }

    /// Hand the remembered GPU fact to the core, which saves the settings.
    fn sync_gpu_seen(&mut self) {
        if self.discrete_late.load(Ordering::Acquire) {
            self.settings.gpu_discrete_seen = true;
        }
        let seen = self.settings.gpu_discrete_seen;
        if let Some(app) = &mut self.app {
            app.set_gpu_discrete_seen(seen);
        }
    }

    /// Free the thumbnails kept for the next window session.
    fn drop_kept(&mut self) {
        self.kept_until = None;
        if self.kept.take().is_some() {
            self.trace("kept thumbnails freed");
            resident::trim_memory();
        }
    }

    /// Report a finished smoke run and decide the exit code.
    fn finish(mut self) -> i32 {
        // The process ends: nothing is kept for a next session.
        self.kept = None;
        self.sync_gpu_seen();
        if let Some(app) = self.app.take() {
            let _ = app.shutdown(SETTINGS_SAVE_WAIT);
        }
        if let Some(smoke) = self.smoke.take() {
            let (line, code) = smoke.report(self.exit_code);
            if let Some(timing) = self.timing.as_ref().and_then(timing::Timing::line) {
                if code == 0 {
                    console::out(timing);
                }
            }
            if code == 0 {
                console::out(&line);
            } else {
                console::err(&line);
            }
            self.exit_code = code;
        }
        let code = self.exit_code;
        // Do not wait for the GPU or for stuck threads: the process ends.
        std::mem::forget(self);
        code
    }

    fn check_exit(&mut self, event_loop: &ActiveEventLoop) {
        let wants_exit = self.app.as_ref().is_some_and(App::exit_requested)
            || self.smoke.as_ref().is_some_and(smoke::Smoke::finished);
        if wants_exit {
            // A process that holds another GPU than the discrete one this
            // computer has does not stay: the next start picks the right one.
            self.sync_gpu_seen();
            let wrong_gpu = self.settings.gpu_discrete_seen && !self.gpu_discrete;
            if self.resident && self.exit_code == 0 && self.app.is_some() && !wrong_gpu {
                self.sleep();
                return;
            }
            if let Some(window) = &self.window {
                // Disappear at once; settings are written after the loop.
                window.set_visible(false);
            }
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<UserEvent> for Shell {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if matches!(cause, StartCause::ResumeTimeReached { .. }) {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        if let Err((message, code)) = self.create(event_loop) {
            self.fail_start(event_loop, message, code);
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Wake => {
                self.wake_pending.store(false, Ordering::Release);
                let now = self.now();
                if let Some(app) = self.app.as_mut() {
                    app.pump(now);
                }
                self.flush_content();
            }
            UserEvent::Resident(Request::Open { path, launched }) => {
                self.wake(event_loop, path, launched);
            }
            UserEvent::Resident(Request::Quit) => {
                self.resident = false;
                match self.app.as_mut() {
                    Some(app) => app.request_exit(),
                    None => event_loop.exit(),
                }
            }
            UserEvent::Resident(Request::Ping) => {}
            UserEvent::GpuReady(result) => {
                if self.app.is_none() {
                    // No window session yet (the window comes in `resumed`,
                    // or the program waits in the background): the next
                    // session takes the device.
                    self.early_result = Some(result);
                    if self.dormant.load(Ordering::Acquire) {
                        resident::trim_memory();
                    }
                } else if self.renderer.is_none() {
                    if let Err((message, code)) = self.gpu_ready(result) {
                        self.fail_start(event_loop, message, code);
                    }
                }
            }
            UserEvent::Prepared {
                epoch,
                generation,
                full,
                result,
            } => self.prepared(epoch, generation, full, result),
        }
        self.check_exit(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if !matches!(event, WindowEvent::Ime(_) | WindowEvent::RedrawRequested) {
            self.last_key_text = None;
        }
        match event {
            WindowEvent::CloseRequested => {
                if let Some(app) = self.app.as_mut() {
                    app.request_exit();
                }
            }
            WindowEvent::Resized(size) => {
                self.minimized = size.width == 0 || size.height == 0;
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
                let maximized = window.is_maximized();
                if let Some(app) = self.app.as_mut() {
                    app.resized(size.width, size.height, maximized);
                    app.fullscreen_changed(window.fullscreen().is_some());
                }
                if !self.minimized {
                    // Draw now so live resizing never shows a stretched frame.
                    self.redraw();
                }
            }
            WindowEvent::Moved(_) => {
                if let (Ok(pos), Some(app)) = (window.outer_position(), self.app.as_mut()) {
                    app.moved(pos.x, pos.y, window.is_maximized());
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                if let Some(app) = self.app.as_mut() {
                    app.scale_changed(scale_factor as f32);
                }
                if let (Some(r), Some(app)) = (self.renderer.as_mut(), self.app.as_ref()) {
                    let mut s = r.settings().clone();
                    s.checkerboard = Some(Checkerboard {
                        cell: (8.0 * scale_factor as f32).round().max(4.0),
                        ..Checkerboard::default()
                    });
                    s.background = app.background();
                    r.set_settings(s);
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                if let Some(app) = self.app.as_mut() {
                    app.system_theme_changed(Some(theme_kind(theme)));
                    native::paint_background(&window, app.background());
                }
                window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                self.occluded = occluded;
                if !occluded {
                    window.request_redraw();
                }
            }
            WindowEvent::Focused(false) => self.input(InputEvent::FocusLost),
            WindowEvent::DroppedFile(path) => {
                // Several files dropped at once: the first one opens.
                if self.pending_drop.is_none() {
                    self.pending_drop = Some(path);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Point::new(position.x as f32, position.y as f32);
                self.input(InputEvent::PointerMove { pos: self.cursor });
            }
            WindowEvent::CursorLeft { .. } => self.input(InputEvent::PointerLeft),
            WindowEvent::MouseInput { state, button, .. } => {
                if let Some(button) = input::button(button) {
                    let pos = self.cursor;
                    self.input(match state {
                        ElementState::Pressed => InputEvent::PointerDown { pos, button },
                        ElementState::Released => InputEvent::PointerUp { pos, button },
                    });
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let pos = self.cursor;
                self.input(InputEvent::Wheel {
                    pos,
                    delta: input::wheel(delta),
                });
            }
            WindowEvent::PinchGesture { delta, .. } => {
                let now = self.now();
                let anchor = self.cursor;
                if let Some(app) = self.app.as_mut() {
                    let factor = (1.0 + delta as f32).clamp(0.5, 2.0);
                    app.handle_action(pandite_ui::Action::ZoomAt { factor, anchor }, now);
                    window.request_redraw();
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.mods = input::modifiers(m.state());
                self.input(InputEvent::ModifiersChanged(self.mods));
            }
            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                // Synthetic presses replay held keys on focus changes.
                if !is_synthetic {
                    self.keyboard(event);
                }
            }
            WindowEvent::Ime(Ime::Preedit(text, _)) => self.ime_preedit = !text.is_empty(),
            WindowEvent::Ime(Ime::Commit(text)) => {
                self.ime_preedit = false;
                let duplicate = self.last_key_text.take().is_some_and(|t| t == text);
                if !duplicate && !text.is_empty() {
                    self.input(InputEvent::Text(text));
                }
            }
            WindowEvent::RedrawRequested if !self.minimized => self.redraw(),
            _ => {}
        }
        self.flush_content();
        self.check_exit(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = self.now();
        if self.kept_until.is_some_and(|at| at <= Instant::now()) {
            // No image opened in time: the background program frees them.
            self.drop_kept();
        }
        if let Some(path) = self.pending_drop.take() {
            if let Some(app) = self.app.as_mut() {
                app.open(path, now);
            }
        }
        if let (Some(smoke), Some(app)) = (self.smoke.as_mut(), self.app.as_mut()) {
            smoke.tick(app, self.stall.as_deref(), now);
        }
        let timed = self.timing.as_ref().is_none_or(|t| t.line().is_some());
        if let (Some(smoke), Some(renderer), Some(app), true) = (
            self.smoke.as_mut(),
            self.renderer.as_ref(),
            self.app.as_ref(),
            timed,
        ) {
            smoke.verify(renderer, self.content.as_ref(), app);
        }
        self.flush_content();
        self.check_exit(event_loop);
        if self.show_at.is_some_and(|at| at <= Instant::now()) {
            // The image takes long: show the loading screen.
            self.show();
        }
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.renderer.is_none() {
            // Waiting for the GPU, or in the background: nothing to draw.
            // In the background, wake when the kept thumbnails expire.
            event_loop.set_control_flow(match self.kept_until {
                Some(at) => ControlFlow::WaitUntil(at),
                None => ControlFlow::Wait,
            });
            return;
        }
        let app_wants = self.app.as_ref().is_some_and(App::needs_frame)
            || self.smoke.as_ref().is_some_and(smoke::Smoke::wants_frames);
        let hidden = self.minimized || self.occluded;
        if app_wants && !hidden {
            window.request_redraw();
        }
        let mut wake = if hidden { None } else { self.anim_deadline };
        if let Some(at) = self.app.as_ref().and_then(App::next_frame_at) {
            let t = self.start + at;
            wake = Some(wake.map_or(t, |w| w.min(t)));
        }
        if let Some(deadline) = self.smoke.as_ref().map(smoke::Smoke::deadline) {
            wake = Some(wake.map_or(deadline, |w| w.min(deadline)));
        }
        if let Some(at) = self.show_at {
            wake = Some(wake.map_or(at, |w| w.min(at)));
        }
        match wake {
            Some(t) if t <= Instant::now() => {
                if !hidden {
                    window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(t) => event_loop.set_control_flow(ControlFlow::WaitUntil(t)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

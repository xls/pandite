//! `pandite --smoke <image>`: the release self test.
//!
//! The run opens the real window and GPU, loads the image through the
//! normal asynchronous path, draws frames to the window and then renders
//! the same content offscreen on the same GPU device and reads the pixels
//! back: a still must show non-background pixels, an animation must show
//! two different frames. It prints one `SMOKE OK` or `SMOKE FAIL` line.
//!
//! With `--smoke-stall` every file read blocks forever. The run waits for
//! the loading screen, presses Escape and passes when the core asks to
//! close at once.

use crate::app::App;
use crate::disk::Disk;
use pandite_core::{CancelToken, Content, DrawList, Result as CoreResult, Source, ViewState};
use pandite_library::{FileAccess, FileInfo, StdFiles};
use pandite_render::{RenderSettings, Renderer};
use pandite_ui::{InputEvent, Key, Modifiers};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// The longest a smoke run may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// Frames drawn to the window with the content before the offscreen check.
const MIN_FRAMES: u32 = 3;
/// Animation frame changes seen on the window path.
const MIN_ADVANCES: u32 = 2;

/// File access whose reads never return: a cloud file that never
/// downloads.
#[derive(Default)]
pub struct StallDisk {
    started: AtomicBool,
    gate: Mutex<()>,
    never: Condvar,
}

impl StallDisk {
    /// True once some read has started (and stalled).
    pub fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }

    fn stall(&self) -> ! {
        self.started.store(true, Ordering::Release);
        let mut guard = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            guard = self.never.wait(guard).unwrap_or_else(|e| e.into_inner());
        }
    }
}

impl FileAccess for StallDisk {
    fn list_dir(&self, dir: &Path) -> io::Result<Vec<FileInfo>> {
        StdFiles.list_dir(dir)
    }

    fn file_info(&self, path: &Path) -> io::Result<FileInfo> {
        StdFiles.file_info(path)
    }

    fn read(&self, _path: &Path, _cancel: &CancelToken) -> CoreResult<Source> {
        self.stall()
    }
}

impl Disk for StallDisk {
    fn read_head(&self, _path: &Path, _max: usize) -> io::Result<Vec<u8>> {
        self.stall()
    }
}

/// Progress of a smoke run.
pub struct Smoke {
    path: PathBuf,
    stall: bool,
    start: Instant,
    window: bool,
    frames: u32,
    advances: u32,
    loading_frames: u32,
    stall_seen: Option<Instant>,
    escape_at: Option<Instant>,
    shown: Option<(u32, u32, &'static str)>,
    failure: Option<String>,
    verified: Option<String>,
}

impl Smoke {
    /// A run for `path`; `stall` selects the stalled-read variant.
    pub fn new(path: PathBuf, stall: bool, start: Instant) -> Self {
        Self {
            path,
            stall,
            start,
            window: false,
            frames: 0,
            advances: 0,
            loading_frames: 0,
            stall_seen: None,
            escape_at: None,
            shown: None,
            failure: None,
            verified: None,
        }
    }

    /// The window and renderer exist.
    pub fn window_ready(&mut self) {
        self.window = true;
    }

    /// Record a failure (the first one wins).
    pub fn fail(&mut self, reason: String) {
        if self.failure.is_none() && self.verified.is_none() {
            self.failure = Some(reason);
        }
    }

    /// True when the run has a result.
    pub fn finished(&self) -> bool {
        self.failure.is_some() || self.verified.is_some()
    }

    /// Keep drawing frames while the run waits for them.
    pub fn wants_frames(&self) -> bool {
        !self.finished()
    }

    /// When the run gives up.
    pub fn deadline(&self) -> Instant {
        self.start + TIMEOUT
    }

    /// Account for one drawn frame.
    pub fn frame(
        &mut self,
        drawn: bool,
        advanced: bool,
        app: &App,
        renderer: &Renderer,
        content_current: bool,
    ) {
        if !drawn || self.finished() {
            return;
        }
        if self.stall {
            if self.stall_seen.is_some() && app.loading_path().is_some() {
                self.loading_frames += 1;
            }
            return;
        }
        let shown = app.shown_path().is_some_and(|p| p == self.path);
        if shown && content_current && renderer.content_kind().is_some() {
            self.frames += 1;
            if advanced {
                self.advances += 1;
            }
            if let Some(image) = app.shown_image() {
                let kind = match renderer.content_kind() {
                    Some(pandite_core::ContentKind::Animation) => "animation",
                    _ => "still",
                };
                self.shown = Some((image.width(), image.height(), kind));
            }
        }
    }

    /// Timeouts, load failures and the Escape press of a stall run.
    pub fn tick(&mut self, app: &mut App, stall: Option<&StallDisk>, now: Duration) {
        if self.finished() {
            return;
        }
        if Instant::now() >= self.deadline() {
            let what = if !self.window {
                "the window"
            } else if self.stall {
                "the loading screen"
            } else if self.frames == 0 {
                "the image"
            } else {
                "animation frames"
            };
            self.fail(format!("timed out waiting for {what}"));
            return;
        }
        if !self.stall {
            if let Some(notice) = app.notice() {
                self.fail(format!("load failed: {notice}"));
            }
            return;
        }
        if stall.is_some_and(StallDisk::started) && self.stall_seen.is_none() {
            self.stall_seen = Some(Instant::now());
        }
        let Some(seen) = self.stall_seen else {
            return;
        };
        // The loading screen appears after a short delay; wait until it
        // was drawn a few times, then press Escape.
        if self.escape_at.is_none()
            && seen.elapsed() >= Duration::from_millis(400)
            && self.loading_frames >= 3
        {
            let pressed = Instant::now();
            app.input(
                InputEvent::KeyDown {
                    key: Key::Escape,
                    mods: Modifiers::NONE,
                    repeat: false,
                },
                now,
            );
            self.escape_at = Some(pressed);
            if app.exit_requested() {
                self.verified = Some(format!(
                    "escaped a stalled load after {} loading frames; exit requested in {} us",
                    self.loading_frames,
                    pressed.elapsed().as_micros()
                ));
            } else {
                self.fail("Escape on the loading screen did not close the app".into());
            }
        }
    }

    /// Once enough window frames were drawn, check the GPU output
    /// offscreen.
    pub fn verify(&mut self, renderer: &Renderer, content: Option<&Content>) {
        if self.finished() || self.stall {
            return;
        }
        let Some((w, h, kind)) = self.shown else {
            return;
        };
        let animated = kind == "animation";
        if self.frames < MIN_FRAMES || (animated && self.advances < MIN_ADVANCES) {
            return;
        }
        let Some(content) = content else {
            return;
        };
        match offscreen_check(renderer, content) {
            Ok(detail) => {
                self.verified = Some(format!(
                    "{} {w}x{h} {kind}, {} window frames, {} animation steps, {detail}, gpu {}",
                    self.path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    self.frames,
                    self.advances,
                    renderer.gpu().describe(),
                ));
            }
            Err(err) => self.fail(format!("offscreen check failed: {err}")),
        }
    }

    /// The line to print and the exit code.
    pub fn report(&self, exit_code: i32) -> (String, i32) {
        if let Some(reason) = &self.failure {
            let code = if exit_code == super::EXIT_NO_GPU {
                super::EXIT_NO_GPU
            } else {
                1
            };
            return (format!("SMOKE FAIL: {reason}"), code);
        }
        match &self.verified {
            Some(detail) => (format!("SMOKE OK: {detail}"), 0),
            None => (
                "SMOKE FAIL: the window closed before the check finished".into(),
                1,
            ),
        }
    }
}

fn hash(data: &[u8]) -> u64 {
    // FNV-1a: enough to tell two frames apart.
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// Render `content` offscreen on the window's GPU and inspect the pixels.
fn offscreen_check(window: &Renderer, content: &Content) -> Result<String, String> {
    let settings = RenderSettings::default();
    let background = settings.background;
    let mut r =
        Renderer::offscreen(window.gpu().clone(), 256, 256, settings).map_err(|e| e.to_string())?;
    r.set_content(content.clone()).map_err(|e| e.to_string())?;
    let view = ViewState::default();
    let list = DrawList::new();
    let _ = r.update(Duration::ZERO);
    r.render(&view, r.full_viewport(), &list)
        .map_err(|e| e.to_string())?;
    let first = r.read_pixels().map_err(|e| e.to_string())?;
    let differs_from_background = first.data().chunks_exact(4).any(|p| {
        p[0].abs_diff(background.r) > 8
            || p[1].abs_diff(background.g) > 8
            || p[2].abs_diff(background.b) > 8
    });
    if !differs_from_background {
        return Err("the rendered image shows only background".into());
    }
    match content {
        Content::Animation(anim) => {
            let delay = anim.frames()[0].delay;
            let _ = r.update(delay + Duration::from_millis(1));
            r.render(&view, r.full_viewport(), &list)
                .map_err(|e| e.to_string())?;
            let second = r.read_pixels().map_err(|e| e.to_string())?;
            let (a, b) = (hash(first.data()), hash(second.data()));
            if a == b {
                return Err("two animation frames rendered the same pixels".into());
            }
            Ok(format!("offscreen frames {a:016x} and {b:016x} differ"))
        }
        _ => Ok(format!("offscreen frame {:016x}", hash(first.data()))),
    }
}

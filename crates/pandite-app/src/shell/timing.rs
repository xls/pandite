//! `pandite --timing`: how long the startup image takes to reach the
//! screen, stage by stage.
//!
//! Both headline figures count from the moment the core asks the library
//! for the file (the window is visible and the GPU is ready by then) to
//! the presented frame that first shows the image: at any resolution
//! (`first_pixel_ms`) and at full resolution (`full_res_ms`). A frame
//! counts as presented once the GPU finished it: the run waits for the
//! GPU after each such frame, on the frames it measures only.
//!
//! The stage figures come from the library (file read, reduced and full
//! decode) and from the renderer (upload and mip generation of the full
//! resolution image, measured with [`pandite_render::Gpu::set_measure_uploads`]).
//! A stage that did not run prints 0.

use crate::app::App;
use pandite_render::{Renderer, UploadTimings};
use std::path::PathBuf;
use std::time::Duration;

/// Progress of one timing run.
pub struct Timing {
    path: PathBuf,
    /// The content generation measured last.
    measured: u64,
    /// Upload figures per generation waiting for its first frame.
    pending: Vec<(u64, bool, Option<UploadTimings>)>,
    first_pixel: Option<Duration>,
    full_res: Option<Duration>,
    upload_full: Option<UploadTimings>,
    upload_reduced: Option<UploadTimings>,
    line: Option<String>,
}

fn ms(d: Option<Duration>) -> String {
    format!("{:.1}", d.map_or(0.0, |d| d.as_secs_f64() * 1000.0))
}

impl Timing {
    /// A run for the startup image `path`.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            measured: 0,
            pending: Vec::new(),
            first_pixel: None,
            full_res: None,
            upload_full: None,
            upload_reduced: None,
            line: None,
        }
    }

    /// Content of `generation` was uploaded and handed to the renderer.
    pub fn prepared(&mut self, generation: u64, full: bool, upload: Option<UploadTimings>) {
        // Older content was replaced before it showed: it is never measured.
        self.pending.retain(|p| p.0 > generation);
        self.pending.push((generation, full, upload));
    }

    /// A frame showing content `generation` was presented. `now` is the
    /// shell clock. Waits for the GPU when the frame is the first one of a
    /// measured stage.
    pub fn presented(
        &mut self,
        generation: u64,
        renderer: &Renderer,
        app: &App,
        now: impl Fn() -> Duration,
    ) {
        if self.line.is_some() || generation == self.measured {
            return;
        }
        let Some(pos) = self.pending.iter().position(|p| p.0 == generation) else {
            return;
        };
        let (_, full, upload) = self.pending.remove(pos);
        self.measured = generation;
        let Some(start) = app.load_started_at(&self.path) else {
            return;
        };
        if app.shown_path() != Some(self.path.as_path()) {
            return;
        }
        // The frame is on screen once the GPU finished it.
        let _ = renderer.gpu().wait_idle();
        let at = now().saturating_sub(start);
        self.first_pixel.get_or_insert(at);
        if full {
            self.full_res = Some(at);
            self.upload_full = upload;
        } else {
            self.upload_reduced = upload;
        }
        if full {
            let stages = app.library().timings(&self.path).unwrap_or_default();
            self.line = Some(format!(
                "TIMING first_pixel_ms={} full_res_ms={} read_ms={} decode_reduced_ms={} decode_full_ms={} upload_ms={} mips_ms={} upload_reduced_ms={}",
                ms(self.first_pixel),
                ms(self.full_res),
                ms(stages.read),
                ms(stages.decode_reduced),
                ms(stages.decode_full),
                ms(self.upload_full.map(|u| u.upload)),
                ms(self.upload_full.map(|u| u.mips)),
                ms(self.upload_reduced.map(|u| u.upload + u.mips)),
            ));
        }
    }

    /// The report, once the full resolution frame was presented.
    pub fn line(&self) -> Option<&str> {
        self.line.as_deref()
    }
}

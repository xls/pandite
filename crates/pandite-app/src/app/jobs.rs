//! Background jobs of the application core: edits, Save As, size probes,
//! folder resolution, bulk conversion and settings writes.
//!
//! Each job runs on its own thread, touches the disk only through
//! [`Disk`], and reports with one [`Msg`] followed by a wake of the event
//! loop. A job whose thread is stuck in a file system call is simply never
//! heard from again; nothing waits for it.

use super::Msg;
use crate::disk::{list_images, Disk};
use crate::edit::Edit;
use crate::platform::Waker;
use pandite_core::{
    CancelToken, EncodeOptions, Error, ImageBuf, Registry, Result as CoreResult, Source,
};
use pandite_ops::{BulkJob as OpsJob, Destination, FileOutcome, OverwritePolicy};
use pandite_ui::{BulkJob, BulkSource};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::sync::Arc;

/// Bytes read from the start of a file to probe its size.
pub(crate) const PROBE_BYTES: usize = 512 * 1024;

/// The way back from a job to the core.
#[derive(Clone)]
pub(crate) struct Outbox {
    tx: Sender<Msg>,
    wake: Waker,
}

impl Outbox {
    pub(crate) fn new(tx: Sender<Msg>, wake: Waker) -> Self {
        Self { tx, wake }
    }

    /// Queue `msg` and wake the event loop.
    pub(crate) fn send(&self, msg: Msg) {
        if self.tx.send(msg).is_ok() {
            (self.wake)();
        }
    }
}

/// Start a named background thread.
pub(crate) fn spawn(name: &str, f: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .map(drop)
}

fn guarded<T>(what: &str, f: impl FnOnce() -> CoreResult<T>) -> CoreResult<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => Err(Error::Backend(format!("{what} stopped unexpectedly"))),
    }
}

/// Apply one edit.
pub(crate) fn edit(out: Outbox, image_id: u64, edit: Edit, input: ImageBuf) -> std::io::Result<()> {
    spawn("pandite-edit", move || {
        let result = guarded("the edit", || edit.apply(&input, &CancelToken::new()));
        out.send(Msg::EditDone {
            image: image_id,
            edit,
            result,
        });
    })
}

/// What a Save As job needs.
pub(crate) struct SaveJob {
    pub id: u64,
    pub encoder: String,
    pub image: ImageBuf,
    pub options: EncodeOptions,
    pub target: PathBuf,
}

/// Encode and write atomically.
pub(crate) fn save(
    out: Outbox,
    disk: Arc<dyn Disk>,
    registry: Arc<Registry>,
    job: SaveJob,
) -> std::io::Result<()> {
    spawn("pandite-save", move || {
        let result = guarded("saving", || {
            let encoder = registry
                .encoder_by_id(&job.encoder)
                .ok_or_else(|| Error::Unsupported(job.encoder.clone()))?;
            let mut bytes = Vec::new();
            encoder.encode(&job.image, &job.options, &mut bytes)?;
            disk.write_atomic(&job.target, &bytes)?;
            Ok(())
        })
        .map_err(|e| e.to_string());
        out.send(Msg::Saved { id: job.id, result });
    })
}

/// Read the head of `path` and probe its pixel size.
pub(crate) fn probe(
    out: Outbox,
    disk: Arc<dyn Disk>,
    registry: Arc<Registry>,
    path: PathBuf,
) -> std::io::Result<()> {
    spawn("pandite-probe", move || {
        let meta = guarded("probing", || {
            let bytes = disk.read_head(&path, PROBE_BYTES)?;
            let source = Source::from_bytes(bytes, Some(path.clone()));
            registry.probe(&source)
        });
        if let Ok(meta) = meta {
            out.send(Msg::Probed {
                path,
                size: (meta.width, meta.height),
            });
        }
    })
}

/// Turn a path that is not an image (a folder, a file without a known
/// extension) into the image to open.
pub(crate) fn resolve(
    out: Outbox,
    disk: Arc<dyn Disk>,
    registry: Arc<Registry>,
    path: PathBuf,
) -> std::io::Result<()> {
    spawn("pandite-resolve", move || {
        let found = match disk.file_info(&path) {
            Ok(info) if info.is_dir => list_images(&*disk, &registry, &path, false)
                .map_err(|e| e.to_string())
                .and_then(|list| {
                    list.into_iter()
                        .next()
                        .ok_or_else(|| "no images in this folder".to_string())
                }),
            Ok(_) => Ok(path.clone()),
            Err(err) => Err(err.to_string()),
        };
        out.send(Msg::Resolved {
            original: path,
            found,
        });
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Convert the interface's bulk job to the operation crate's, without
/// inputs (the folder is listed on the worker).
pub(crate) fn ops_job(job: &BulkJob, inputs: Vec<PathBuf>) -> OpsJob {
    let mut out = OpsJob::new(inputs);
    out.resize = job.resize.map(crate::edit::resize_spec);
    out.filter = pandite_ops::Filter::Lanczos3;
    out.output_format = job.format.clone();
    out.encode = EncodeOptions {
        quality: job.quality.clamp(1, 100),
        // The WebP encoder writes lossless files only.
        lossless: job.format.as_deref() == Some("webp"),
    };
    out.destination = Destination::Directory(job.destination.clone());
    out.overwrite = match job.overwrite {
        pandite_ui::OverwritePolicy::Skip => OverwritePolicy::Skip,
        pandite_ui::OverwritePolicy::Overwrite => OverwritePolicy::Overwrite,
        pandite_ui::OverwritePolicy::Rename => OverwritePolicy::Rename,
    };
    out
}

/// Run a bulk job: list a folder source, then convert every file.
pub(crate) fn bulk(
    out: Outbox,
    disk: Arc<dyn Disk>,
    registry: Arc<Registry>,
    id: u64,
    job: BulkJob,
    cancel: CancelToken,
) -> std::io::Result<()> {
    spawn("pandite-bulk", move || {
        let inputs = match &job.source {
            BulkSource::Files(files) => Ok(files.clone()),
            BulkSource::Folder { path, recursive } => {
                list_images(&*disk, &registry, path, *recursive).map_err(|e| e.to_string())
            }
        };
        let inputs = match inputs {
            Ok(inputs) if inputs.is_empty() => {
                out.send(Msg::BulkDone {
                    id,
                    summary: "No images to convert".into(),
                    destination: job.destination.clone(),
                });
                return;
            }
            Ok(inputs) => inputs,
            Err(err) => {
                out.send(Msg::BulkDone {
                    id,
                    summary: format!("Cannot read the folder: {err}"),
                    destination: job.destination.clone(),
                });
                return;
            }
        };
        out.send(Msg::BulkProgress {
            id,
            done: 0,
            total: inputs.len(),
            name: String::new(),
        });
        let ops = ops_job(&job, inputs);
        let progress_out = out.clone();
        let progress = move |p: &pandite_ops::Progress<'_>| {
            progress_out.send(Msg::BulkProgress {
                id,
                done: p.done,
                total: p.total,
                name: file_name(p.input),
            });
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            pandite_ops::run_bulk(&ops, &registry, &cancel, &progress)
        }));
        let summary = match result {
            Ok(Ok(report)) => summarize(&report),
            Ok(Err(err)) => format!("Bulk job failed: {err}"),
            Err(_) => "Bulk job stopped unexpectedly".to_string(),
        };
        out.send(Msg::BulkDone {
            id,
            summary,
            destination: job.destination.clone(),
        });
    })
}

/// One line about a finished bulk job.
pub(crate) fn summarize(report: &pandite_ops::BulkReport) -> String {
    let mut parts = vec![format!("{} saved", report.written())];
    if report.skipped() > 0 {
        parts.push(format!("{} skipped", report.skipped()));
    }
    if report.failed() > 0 {
        parts.push(format!("{} failed", report.failed()));
    }
    if report.cancelled || report.cancelled_files() > 0 {
        parts.push(format!("{} not done", report.cancelled_files()));
    }
    let mut line = format!("Bulk done: {}", parts.join(", "));
    if let Some(first) = report.files.iter().find_map(|f| match &f.outcome {
        FileOutcome::Failed(err) => Some((f.input.clone(), err.to_string())),
        _ => None,
    }) {
        line.push_str(&format!(". {}: {}", file_name(&first.0), first.1));
    }
    line
}

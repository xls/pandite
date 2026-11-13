//! Batch resize and reformat through the provider registry.
//!
//! [`run_bulk`] takes a list of files. For each file it decodes through the
//! [`Registry`], applies an optional [`ResizeSpec`], encodes with a registry
//! encoder and writes the result. Decoders and encoders arrive only through
//! the registry, so this module knows no file format.
//!
//! Behavior:
//! - Files run on a bounded pool of worker threads (see [`BulkJob::threads`]).
//!   A failure in one file is reported for that file and the batch goes on.
//! - Animated inputs: only the first frame is processed and the output is a
//!   still image. The decoder is asked for the first frame only.
//! - Output format: [`BulkJob::output_format`] names an encoder by id or
//!   extension. Without it each file keeps its format, chosen by its
//!   extension, and keeps its extension spelling.
//! - Output names come from [`BulkJob::name_pattern`]; the extension is the
//!   encoder's default one.
//! - Existing files are never replaced unless [`OverwritePolicy::Overwrite`]
//!   is set. The default, [`OverwritePolicy::Rename`], adds the first free
//!   numeric suffix (`photo_1.png`, `photo_2.png`, ...). Two inputs of one
//!   batch never get the same output name, and no output takes the path of
//!   another input of the batch.
//! - Writes are atomic: the encoded bytes go to a hidden temporary file in
//!   the destination folder, are flushed to disk, and then appear under the
//!   final name in one step (a hard link that fails if the name exists, or a
//!   rename). A crash never leaves a half-written output.
//! - Progress: the callback runs once per input, from worker threads, one
//!   call at a time, after the file is finished.
//! - Cancellation: files not yet started are reported as
//!   [`FileOutcome::Cancelled`] and files in progress stop at the next check
//!   (between rows while resizing). A file is only reported as written once
//!   it is complete under its final name.

use crate::convert::to_rgba8;
use crate::filter::Filter;
use crate::par;
use crate::resize::{resize_with, ResizeOptions, ResizeSpec};
use pandite_core::provider::extension_of;
use pandite_core::{
    CancelToken, DecodeOptions, EncodeOptions, EncoderProvider, Error, ImageBuf, PixelFormat,
    Registry, Result, Source,
};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// Where output files go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// The folder of each input file.
    BesideSource,
    /// One folder for every output. It is created when missing.
    Directory(PathBuf),
}

/// What to do when an output name is already taken on disk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverwritePolicy {
    /// Keep the existing file and write under the first free name with a
    /// numeric suffix: `photo_1.png`, `photo_2.png` and so on. The default.
    #[default]
    Rename,
    /// Keep the existing file, write nothing and report
    /// [`FileOutcome::Skipped`].
    Skip,
    /// Replace the existing file.
    Overwrite,
}

/// A batch of files to resize and/or convert.
#[derive(Clone, Debug)]
pub struct BulkJob {
    /// The files to process, in report order.
    pub inputs: Vec<PathBuf>,
    /// Resize rule, or `None` to keep the size.
    pub resize: Option<ResizeSpec>,
    /// Resampling filter for resizing.
    pub filter: Filter,
    /// Encoder id (`"png"`) or extension (`"jpg"`) for every output, or
    /// `None` to keep each file's format.
    pub output_format: Option<String>,
    /// Quality and lossless settings handed to the encoder.
    pub encode: EncodeOptions,
    /// Output folder rule.
    pub destination: Destination,
    /// Output file name without extension. `{name}` is the input file name
    /// without extension, `{n}` the 1-based position in [`BulkJob::inputs`],
    /// `{{` and `}}` are literal braces. Path separators and characters that
    /// Windows forbids in names are rejected. Example: `"{name}_small"`.
    pub name_pattern: String,
    /// Rule for names that already exist on disk.
    pub overwrite: OverwritePolicy,
    /// Most files processed at once; 0 picks the number of hardware threads,
    /// at most 8, to bound memory. Resizing inside each file shares the
    /// remaining hardware threads.
    pub threads: usize,
}

impl BulkJob {
    /// The default [`BulkJob::name_pattern`]: the input name unchanged.
    pub const DEFAULT_PATTERN: &'static str = "{name}";

    /// Most automatic workers.
    const AUTO_MAX_WORKERS: usize = 8;

    /// A job for `inputs` with defaults: no resize, Lanczos3, keep formats,
    /// default encoder settings, outputs beside the sources named
    /// `{name}`, existing files renamed around, automatic threads.
    pub fn new(inputs: Vec<PathBuf>) -> Self {
        Self {
            inputs,
            resize: None,
            filter: Filter::default(),
            output_format: None,
            encode: EncodeOptions::default(),
            destination: Destination::BesideSource,
            name_pattern: Self::DEFAULT_PATTERN.to_owned(),
            overwrite: OverwritePolicy::default(),
            threads: 0,
        }
    }
}

impl Default for BulkJob {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

/// What happened to one input.
#[derive(Debug)]
pub enum FileOutcome {
    /// The output was written completely.
    Written {
        /// Final output path.
        path: PathBuf,
        /// Output width in pixels.
        width: u32,
        /// Output height in pixels.
        height: u32,
    },
    /// The output name existed and the policy was [`OverwritePolicy::Skip`].
    Skipped {
        /// The existing file that was left alone.
        existing: PathBuf,
    },
    /// The file could not be processed. Nothing was written for it.
    Failed(Error),
    /// The batch was cancelled before this file finished. Nothing was
    /// written for it.
    Cancelled,
}

impl FileOutcome {
    /// The output path when the file was written.
    pub fn output_path(&self) -> Option<&Path> {
        match self {
            FileOutcome::Written { path, .. } => Some(path),
            _ => None,
        }
    }

    /// True when the output was written.
    pub fn is_written(&self) -> bool {
        matches!(self, FileOutcome::Written { .. })
    }
}

/// The result for one input of a batch.
#[derive(Debug)]
pub struct FileReport {
    /// The input path as given in [`BulkJob::inputs`].
    pub input: PathBuf,
    /// What happened to it.
    pub outcome: FileOutcome,
}

/// The result of [`run_bulk`]: one entry per input, in input order.
#[derive(Debug, Default)]
pub struct BulkReport {
    /// Per-file results, in the order of [`BulkJob::inputs`].
    pub files: Vec<FileReport>,
    /// True when the cancel token was set during the run.
    pub cancelled: bool,
}

impl BulkReport {
    fn count(&self, pick: fn(&FileOutcome) -> bool) -> usize {
        self.files.iter().filter(|f| pick(&f.outcome)).count()
    }

    /// Number of files written.
    pub fn written(&self) -> usize {
        self.count(|o| matches!(o, FileOutcome::Written { .. }))
    }

    /// Number of files skipped because the output existed.
    pub fn skipped(&self) -> usize {
        self.count(|o| matches!(o, FileOutcome::Skipped { .. }))
    }

    /// Number of files that failed.
    pub fn failed(&self) -> usize {
        self.count(|o| matches!(o, FileOutcome::Failed(_)))
    }

    /// Number of files not finished because of cancellation.
    pub fn cancelled_files(&self) -> usize {
        self.count(|o| matches!(o, FileOutcome::Cancelled))
    }
}

/// One progress event, sent after each input is finished.
#[derive(Debug)]
pub struct Progress<'a> {
    /// Inputs finished so far, including this one (1 to `total`).
    pub done: usize,
    /// Number of inputs in the batch.
    pub total: usize,
    /// Position of this input in [`BulkJob::inputs`].
    pub index: usize,
    /// This input's path.
    pub input: &'a Path,
    /// What happened to it.
    pub outcome: &'a FileOutcome,
}

/// Run `job` and report each file.
///
/// Returns `Err` only for problems with the job itself, before any file is
/// touched: an unknown output format ([`Error::Unsupported`]), an invalid
/// name pattern ([`Error::Invalid`]) or a destination folder that cannot be
/// created ([`Error::Io`]). Everything that goes wrong with one file is
/// reported in its [`FileReport`] and the batch continues.
///
/// `progress` is called once per input from the worker threads, never
/// concurrently, after the file is finished. Keep it short; it must not
/// panic.
pub fn run_bulk(
    job: &BulkJob,
    registry: &Registry,
    cancel: &CancelToken,
    progress: &(dyn Fn(&Progress<'_>) + Sync),
) -> Result<BulkReport> {
    let pattern = parse_pattern(&job.name_pattern)?;
    let fixed = match &job.output_format {
        Some(id) => Some(find_encoder(registry, id)?),
        None => None,
    };
    if let Destination::Directory(dir) = &job.destination {
        fs::create_dir_all(dir)?;
    }
    let total = job.inputs.len();
    let names = NameBook::new(&job.inputs);
    let plans: Vec<Mutex<Option<Result<Plan>>>> = job
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            Mutex::new(Some(plan(
                job,
                registry,
                fixed.as_ref(),
                &pattern,
                &names,
                index,
                input,
            )))
        })
        .collect();

    let hardware = par::available_threads();
    let wanted = if job.threads == 0 {
        hardware.min(BulkJob::AUTO_MAX_WORKERS)
    } else {
        job.threads
    };
    let workers = wanted.min(total).max(1);
    let ctx = Ctx {
        job,
        registry,
        cancel,
        names: &names,
        resize_threads: (hardware / workers).max(1),
    };
    let next = AtomicUsize::new(0);
    let shared = Mutex::new(Shared {
        done: 0,
        outcomes: (0..total).map(|_| None).collect(),
    });
    let work = || loop {
        let index = next.fetch_add(1, Ordering::Relaxed);
        if index >= total {
            break;
        }
        let planned = plans[index]
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let outcome = if cancel.is_cancelled() {
            FileOutcome::Cancelled
        } else {
            match planned {
                Some(Ok(plan)) => run_one(&ctx, &plan),
                Some(Err(err)) => FileOutcome::Failed(err),
                // Each index is claimed once, so the plan is always there.
                None => FileOutcome::Failed(Error::Backend("internal: file plan missing".into())),
            }
        };
        let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
        state.done += 1;
        progress(&Progress {
            done: state.done,
            total,
            index,
            input: &job.inputs[index],
            outcome: &outcome,
        });
        state.outcomes[index] = Some(outcome);
    };
    std::thread::scope(|scope| {
        let mut started = 0;
        for _ in 0..workers {
            let spawned = std::thread::Builder::new()
                .name("pandite-bulk".into())
                .spawn_scoped(scope, work);
            if spawned.is_ok() {
                started += 1;
            }
        }
        if started == 0 {
            // No thread could start: do the work here rather than not at all.
            work();
        }
    });

    let outcomes = shared
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .outcomes;
    let files = job
        .inputs
        .iter()
        .zip(outcomes)
        .map(|(input, outcome)| FileReport {
            input: input.clone(),
            outcome: outcome.unwrap_or_else(|| {
                FileOutcome::Failed(Error::Backend("file was not processed".into()))
            }),
        })
        .collect();
    Ok(BulkReport {
        files,
        cancelled: cancel.is_cancelled(),
    })
}

struct Shared {
    done: usize,
    outcomes: Vec<Option<FileOutcome>>,
}

/// Everything a worker needs, shared read-only.
struct Ctx<'a> {
    job: &'a BulkJob,
    registry: &'a Registry,
    cancel: &'a CancelToken,
    names: &'a NameBook,
    resize_threads: usize,
}

/// One input with its encoder and planned output name.
struct Plan {
    input: PathBuf,
    encoder: Arc<dyn EncoderProvider>,
    dir: PathBuf,
    stem: OsString,
    ext: OsString,
    /// Numeric suffix reserved at planning time; 0 means none.
    slot: usize,
}

impl Plan {
    /// The output path with numeric suffix `slot` (0 for none).
    fn path(&self, slot: usize) -> PathBuf {
        output_path(&self.dir, &self.stem, &self.ext, slot)
    }
}

fn output_path(dir: &Path, stem: &OsStr, ext: &OsStr, slot: usize) -> PathBuf {
    let mut name = stem.to_os_string();
    if slot > 0 {
        name.push(format!("_{slot}"));
    }
    if !ext.is_empty() {
        name.push(".");
        name.push(ext);
    }
    dir.join(name)
}

/// The encoder named by id, or failing that by extension.
fn find_encoder(registry: &Registry, id: &str) -> Result<Arc<dyn EncoderProvider>> {
    let wanted = id.trim().trim_start_matches('.').to_ascii_lowercase();
    registry
        .encoder_by_id(&wanted)
        .or_else(|| registry.encoder_for_extension(&wanted))
        .cloned()
        .ok_or_else(|| Error::Unsupported(format!("no encoder for output format {id:?}")))
}

/// Choose the encoder, folder and name for one input.
fn plan(
    job: &BulkJob,
    registry: &Registry,
    fixed: Option<&Arc<dyn EncoderProvider>>,
    pattern: &[Segment],
    names: &NameBook,
    index: usize,
    input: &Path,
) -> Result<Plan> {
    if !names.first_listing(input) {
        // Processing one file twice could read it while the other copy
        // replaces it in place.
        return Err(Error::Invalid(format!(
            "{} is listed more than once",
            input.display()
        )));
    }
    let (encoder, ext) = match fixed {
        Some(encoder) => {
            let ext = encoder
                .extensions()
                .first()
                .copied()
                .unwrap_or_else(|| encoder.id());
            (Arc::clone(encoder), OsString::from(ext))
        }
        None => {
            let encoder = extension_of(input)
                .and_then(|ext| registry.encoder_for_extension(&ext).cloned())
                .ok_or_else(|| {
                    Error::Unsupported(format!(
                        "no encoder writes the format of {}; choose an output format",
                        input.display()
                    ))
                })?;
            let ext = input.extension().unwrap_or_default().to_os_string();
            (encoder, ext)
        }
    };
    let dir = match &job.destination {
        Destination::BesideSource => match input.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
            _ => PathBuf::from("."),
        },
        Destination::Directory(dir) => dir.clone(),
    };
    let stem = build_stem(pattern, input, index);
    let slot = names.reserve_planned(input, &dir, &stem, &ext)?;
    Ok(Plan {
        input: input.to_path_buf(),
        encoder,
        dir,
        stem,
        ext,
        slot,
    })
}

/// Process one planned file, turning errors and panics into outcomes.
fn run_one(ctx: &Ctx<'_>, plan: &Plan) -> FileOutcome {
    match catch_unwind(AssertUnwindSafe(|| process(ctx, plan))) {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(Error::Cancelled)) => FileOutcome::Cancelled,
        Ok(Err(err)) => FileOutcome::Failed(err),
        Err(_) => FileOutcome::Failed(Error::Backend(
            "a provider panicked while processing this file".into(),
        )),
    }
}

fn process(ctx: &Ctx<'_>, plan: &Plan) -> Result<FileOutcome> {
    ctx.cancel.check()?;
    let source = Source::from_path(&plan.input)?;
    // Bulk conversion keeps full precision: 16-bit sources stay 16-bit
    // for formats that can store it.
    let options = DecodeOptions {
        target_size: None,
        first_frame_only: true,
        prefer_8bit: false,
        cancel: ctx.cancel.clone(),
    };
    let content = ctx.registry.decode(&source, &options)?;
    drop(source);
    // Animations: the first frame only. Dropping the content frees the rest.
    let mut image = content.first_image().clone();
    drop(content);
    if let Some(spec) = &ctx.job.resize {
        let (width, height) = spec.resolve(image.width(), image.height())?;
        let options = ResizeOptions {
            filter: ctx.job.filter,
            threads: ctx.resize_threads,
            cancel: Some(ctx.cancel.clone()),
        };
        image = resize_with(&image, width, height, &options)?;
    }
    ctx.cancel.check()?;
    let bytes = encode(&*plan.encoder, &image, &ctx.job.encode)?;
    ctx.cancel.check()?;
    match ctx.names.place(plan, &bytes, ctx.job.overwrite)? {
        Placed::Written(path) => Ok(FileOutcome::Written {
            path,
            width: image.width(),
            height: image.height(),
        }),
        Placed::Skipped(existing) => Ok(FileOutcome::Skipped { existing }),
    }
}

/// Encode to memory. An encoder that rejects a 16-bit or float image with
/// [`Error::Unsupported`] gets an 8-bit copy instead.
fn encode(
    encoder: &dyn EncoderProvider,
    image: &ImageBuf,
    options: &EncodeOptions,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    match encoder.encode(image, options, &mut out) {
        Ok(()) => {}
        Err(Error::Unsupported(_)) if image.format() != PixelFormat::Rgba8 => {
            out.clear();
            encoder.encode(&to_rgba8(image)?, options, &mut out)?;
        }
        Err(err) => return Err(err),
    }
    if out.is_empty() {
        return Err(Error::Backend(format!(
            "encoder {} produced no data",
            encoder.id()
        )));
    }
    Ok(out)
}

/// A piece of a parsed name pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    Text(String),
    Name,
    Index,
}

fn parse_pattern(pattern: &str) -> Result<Vec<Segment>> {
    let invalid = |why: &str| Error::Invalid(format!("name pattern {pattern:?}: {why}"));
    if pattern.trim().is_empty() {
        return Err(invalid("it is empty"));
    }
    let mut segments = Vec::new();
    let mut text = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                text.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                text.push('}');
            }
            '{' => {
                let mut key = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(k) => key.push(k),
                        None => return Err(invalid("a '{' is not closed")),
                    }
                }
                if !text.is_empty() {
                    segments.push(Segment::Text(std::mem::take(&mut text)));
                }
                segments.push(match key.as_str() {
                    "name" => Segment::Name,
                    "n" => Segment::Index,
                    other => return Err(invalid(&format!("unknown placeholder {{{other}}}"))),
                });
            }
            '}' => return Err(invalid("a '}' has no matching '{'")),
            c if c.is_control() || r#"/\:*?"<>|"#.contains(c) => {
                return Err(invalid(&format!("{c:?} is not allowed in file names")));
            }
            c => text.push(c),
        }
    }
    if !text.is_empty() {
        segments.push(Segment::Text(text));
    }
    if segments
        .iter()
        .all(|s| matches!(s, Segment::Text(t) if t.chars().all(|c| c == '.' || c == ' ')))
    {
        return Err(invalid("it makes no usable file name"));
    }
    Ok(segments)
}

fn build_stem(pattern: &[Segment], input: &Path, index: usize) -> OsString {
    let mut stem = OsString::new();
    for segment in pattern {
        match segment {
            Segment::Text(text) => stem.push(text),
            Segment::Name => stem.push(input.file_stem().unwrap_or(OsStr::new("image"))),
            Segment::Index => stem.push((index + 1).to_string()),
        }
    }
    stem
}

/// A path in comparable form: absolute, and case-folded on file systems
/// that ignore case by default (Windows, macOS).
fn path_key(path: &Path) -> OsString {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    if cfg!(any(windows, target_os = "macos")) {
        absolute.to_string_lossy().to_lowercase().into()
    } else {
        absolute.into_os_string()
    }
}

/// True when anything (file, folder, link) has this name.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Output names reserved by this batch.
struct NameBook {
    inner: Mutex<Names>,
}

struct Names {
    /// Every input of the batch. No output may take another input's path.
    inputs: HashSet<OsString>,
    /// Inputs planned so far, to refuse a path listed twice.
    planned: HashSet<OsString>,
    /// Output names already given to an input of this batch.
    outputs: HashSet<OsString>,
}

/// Where an output ended up.
enum Placed {
    Written(PathBuf),
    Skipped(PathBuf),
}

/// Give up after this many numeric suffixes.
const MAX_SUFFIX: usize = 100_000;

impl NameBook {
    fn new(inputs: &[PathBuf]) -> Self {
        Self {
            inner: Mutex::new(Names {
                inputs: inputs.iter().map(|p| path_key(p)).collect(),
                planned: HashSet::new(),
                outputs: HashSet::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Names> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// True the first time `input` is planned, false for a repeat.
    fn first_listing(&self, input: &Path) -> bool {
        self.lock().planned.insert(path_key(input))
    }

    /// Reserve the first output name for `input` that no other input of the
    /// batch uses, either as its input path or as its output. The input's
    /// own path is allowed (in-place output). Disk contents are checked
    /// later, at write time. Inputs are planned in order, so the result is
    /// deterministic.
    fn reserve_planned(
        &self,
        input: &Path,
        dir: &Path,
        stem: &OsStr,
        ext: &OsStr,
    ) -> Result<usize> {
        let own = path_key(input);
        let mut names = self.lock();
        for slot in 0..MAX_SUFFIX {
            let key = path_key(&output_path(dir, stem, ext, slot));
            let other_input = names.inputs.contains(&key) && key != own;
            if !other_input && names.outputs.insert(key) {
                return Ok(slot);
            }
        }
        Err(Error::Invalid(format!(
            "no free output name for {}",
            input.display()
        )))
    }

    /// Reserve `path` as an alternative name if nothing in the batch or on
    /// disk uses it.
    fn try_claim(&self, path: &Path) -> bool {
        let key = path_key(path);
        let mut names = self.lock();
        if names.inputs.contains(&key) || names.outputs.contains(&key) || exists(path) {
            return false;
        }
        names.outputs.insert(key)
    }

    /// Write `bytes` for `plan` under its final name, following `policy`.
    fn place(&self, plan: &Plan, bytes: &[u8], policy: OverwritePolicy) -> Result<Placed> {
        let temp = write_temp(&plan.dir, bytes)?;
        let placed = self.place_temp(plan, &temp, policy);
        if !matches!(placed, Ok(Placed::Written(_))) {
            // Best effort: the temporary file is hidden and harmless.
            let _ = fs::remove_file(&temp);
        }
        placed
    }

    fn place_temp(&self, plan: &Plan, temp: &Path, policy: OverwritePolicy) -> Result<Placed> {
        let mut slot = plan.slot;
        while slot < plan.slot + MAX_SUFFIX {
            let path = plan.path(slot);
            if slot != plan.slot && !self.try_claim(&path) {
                slot += 1;
                continue;
            }
            if policy == OverwritePolicy::Overwrite {
                fs::rename(temp, &path)?;
                return Ok(Placed::Written(path));
            }
            match link_new(temp, &path) {
                Ok(()) => return Ok(Placed::Written(path)),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if policy == OverwritePolicy::Skip {
                        return Ok(Placed::Skipped(path));
                    }
                    slot += 1;
                }
                Err(err) => return Err(err.into()),
            }
        }
        Err(Error::Invalid(format!(
            "no free output name for {}",
            plan.input.display()
        )))
    }
}

/// Make `temp` appear as `path` only if `path` does not exist. A hard link
/// is atomic and fails on an existing name on every platform. File systems
/// without hard links (FAT, some network shares) fall back to check, then
/// rename, which leaves a short window for another program to race.
fn link_new(temp: &Path, path: &Path) -> io::Result<()> {
    match fs::hard_link(temp, path) {
        Ok(()) => {
            // Best effort: the output is complete either way.
            let _ = fs::remove_file(temp);
            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => Err(err),
        Err(_) if exists(path) => Err(io::ErrorKind::AlreadyExists.into()),
        Err(_) => fs::rename(temp, path),
    }
}

/// Write `bytes` to a new hidden file in `dir`, flushed to disk.
fn write_temp(dir: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    for _ in 0..1000 {
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!(".pandite-{}-{seq}.tmp", std::process::id()));
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        return match written {
            Ok(()) => Ok(path),
            Err(err) => {
                let _ = fs::remove_file(&path);
                Err(err)
            }
        };
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free temporary file name",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_parse_and_reject_bad_input() {
        assert_eq!(
            parse_pattern("{name}_s{{x}}{n}").unwrap(),
            vec![Segment::Name, Segment::Text("_s{x}".into()), Segment::Index]
        );
        for bad in [
            "", "  ", "{nam}", "{name", "a}b", "a/b", r"a\b", "a:b", "x?", "..", "a\tb",
        ] {
            assert!(
                matches!(parse_pattern(bad), Err(Error::Invalid(_))),
                "{bad:?}"
            );
        }
        let stem = build_stem(
            &parse_pattern("{n}-{name}").unwrap(),
            Path::new("d/pic.v2.png"),
            4,
        );
        assert_eq!(stem, OsString::from("5-pic.v2"));
    }

    #[test]
    fn output_paths_add_suffix_before_extension() {
        let p = output_path(Path::new("out"), OsStr::new("a"), OsStr::new("png"), 0);
        assert_eq!(p, Path::new("out").join("a.png"));
        let p = output_path(Path::new("out"), OsStr::new("a"), OsStr::new("png"), 3);
        assert_eq!(p, Path::new("out").join("a_3.png"));
        let p = output_path(Path::new("out"), OsStr::new("a"), OsStr::new(""), 1);
        assert_eq!(p, Path::new("out").join("a_1"));
    }

    #[test]
    fn planning_never_reuses_names_or_other_inputs() {
        let inputs = vec![
            PathBuf::from("x/a.png"),
            PathBuf::from("x/a_1.png"),
            PathBuf::from("y/a.png"),
        ];
        let book = NameBook::new(&inputs);
        let dir = Path::new("x");
        let (stem, ext) = (OsStr::new("a"), OsStr::new("png"));
        // In place is allowed for the input itself.
        assert_eq!(book.reserve_planned(&inputs[0], dir, stem, ext).unwrap(), 0);
        // a.png is taken and a_1.png is another input: next free is a_2.
        assert_eq!(book.reserve_planned(&inputs[2], dir, stem, ext).unwrap(), 2);
    }
}

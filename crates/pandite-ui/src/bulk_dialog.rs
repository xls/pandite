//! The bulk convert and resize dialog.
//!
//! Inputs come from pick requests the application answers (files, folder,
//! destination), from the current folder or from the selected images. With
//! a selection the dialog opens with the selected images as its source.
//! The dialog collects a size rule, an output format from the context's
//! encoder list, a quality, a destination and an overwrite policy, then
//! emits a [`BulkJob`]. While the job runs it shows progress from
//! [`UiContext::bulk`].

use crate::action::{Action, BulkJob, BulkSource, OverwritePolicy, ResizeSpec};
use crate::context::UiContext;
use crate::form::{ButtonStyle, FormBuilder, FormEvent, FormHost, Id, Outcome, Tone};
use crate::resize_dialog::{parse_percent, parse_side, MAX_PERCENT};
use crate::text_field::{CharFilter, TextField};
use pandite_core::image::MAX_DIMENSION;
use std::path::{Path, PathBuf};

pub(crate) const BTN_ADD_FILES: Id = 1;
pub(crate) const BTN_ADD_FOLDER: Id = 2;
pub(crate) const BTN_CURRENT: Id = 3;
pub(crate) const CHK_RECURSIVE: Id = 4;
pub(crate) const SEG_RULE: Id = 5;
pub(crate) const F_PERCENT: Id = 6;
pub(crate) const F_FIT_W: Id = 7;
pub(crate) const F_FIT_H: Id = 8;
pub(crate) const CHK_ENLARGE: Id = 9;
pub(crate) const F_EXACT_W: Id = 10;
pub(crate) const F_EXACT_H: Id = 11;
pub(crate) const DD_FORMAT: Id = 12;
pub(crate) const F_QUALITY: Id = 13;
pub(crate) const F_DEST: Id = 14;
pub(crate) const BTN_BROWSE: Id = 15;
pub(crate) const SEG_OVERWRITE: Id = 16;
pub(crate) const BTN_CANCEL: Id = 17;
pub(crate) const BTN_RUN: Id = 18;
pub(crate) const BTN_HIDE: Id = 19;
pub(crate) const BTN_STOP: Id = 20;
pub(crate) const BTN_SELECTION: Id = 21;

/// Label of the "keep each file's format" entry.
pub(crate) const SAME_FORMAT: &str = "Same as source";

/// The size rule of a bulk job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SizeRule {
    Keep,
    Percent,
    Fit,
    Exact,
}

impl SizeRule {
    const ALL: [SizeRule; 4] = [
        SizeRule::Keep,
        SizeRule::Percent,
        SizeRule::Fit,
        SizeRule::Exact,
    ];
    const LABELS: [&'static str; 4] = ["Keep size", "Percent", "Fit in box", "Exact"];

    fn index(self) -> usize {
        match self {
            SizeRule::Keep => 0,
            SizeRule::Percent => 1,
            SizeRule::Fit => 2,
            SizeRule::Exact => 3,
        }
    }
}

const OVERWRITE: [OverwritePolicy; 3] = [
    OverwritePolicy::Skip,
    OverwritePolicy::Overwrite,
    OverwritePolicy::Rename,
];
const OVERWRITE_LABELS: [&str; 3] = ["Skip", "Replace", "Rename"];

/// The bulk dialog state.
#[derive(Clone, Debug)]
pub(crate) struct BulkDialog {
    pub source: Option<BulkSource>,
    pub recursive: bool,
    pub rule: SizeRule,
    percent: TextField,
    fit_w: TextField,
    fit_h: TextField,
    exact_w: TextField,
    exact_h: TextField,
    pub enlarge: bool,
    /// Encoder id, `None` keeps each file's format.
    pub format: Option<String>,
    quality: TextField,
    destination: TextField,
    pub overwrite: OverwritePolicy,
    /// The job was started; the dialog shows progress.
    pub running: bool,
    /// Progress was reported since the job started.
    pub seen_progress: bool,
}

impl Default for BulkDialog {
    fn default() -> Self {
        let digits = |v: &str| TextField::with_text(CharFilter::Digits, 5, v);
        Self {
            source: None,
            recursive: false,
            rule: SizeRule::Fit,
            percent: TextField::with_text(CharFilter::Decimal, 7, "50"),
            fit_w: digits("1920"),
            fit_h: digits("1080"),
            exact_w: digits("1024"),
            exact_h: digits("768"),
            enlarge: false,
            format: None,
            quality: TextField::with_text(CharFilter::Digits, 3, "90"),
            destination: TextField::new(CharFilter::Any, 1024),
            overwrite: OverwritePolicy::Skip,
            running: false,
            seen_progress: false,
        }
    }
}

impl BulkDialog {
    /// Add picked files (duplicates are skipped). Replaces a folder source.
    pub fn add_files(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }
        let mut list = match self.source.take() {
            Some(BulkSource::Files(list)) => list,
            _ => Vec::new(),
        };
        for f in files {
            if !list.contains(&f) {
                list.push(f);
            }
        }
        let parent = list.first().and_then(|f| f.parent()).map(Path::to_path_buf);
        self.source = Some(BulkSource::Files(list));
        if let Some(parent) = parent {
            self.suggest_destination(&parent);
        }
    }

    /// Use a folder as the source.
    pub fn set_folder(&mut self, folder: PathBuf) {
        if folder.as_os_str().is_empty() {
            return;
        }
        self.suggest_destination(&folder);
        self.source = Some(BulkSource::Folder {
            path: folder,
            recursive: self.recursive,
        });
    }

    /// Use the selected images of `ctx` as the source.
    pub fn use_selection(&mut self, ctx: &UiContext<'_>) {
        if ctx.selected.is_empty() {
            return;
        }
        self.source = Some(BulkSource::Selection);
        if let Some(folder) = ctx.folder {
            self.suggest_destination(folder);
        }
    }

    /// Match the source to the selection of `ctx` as the dialog opens: the
    /// selected images when there are any. Without a selection, a
    /// selection source remembered from an earlier opening is dropped and
    /// every other source stays.
    pub fn follow_selection(&mut self, ctx: &UiContext<'_>) {
        if !ctx.selected.is_empty() {
            self.use_selection(ctx);
        } else if self.source == Some(BulkSource::Selection) {
            self.source = None;
        }
    }

    /// Set the destination folder.
    pub fn set_destination(&mut self, folder: &Path) {
        self.destination.set_text(&folder.to_string_lossy());
    }

    fn suggest_destination(&mut self, folder: &Path) {
        if self.destination.text().trim().is_empty() {
            self.destination
                .set_text(&folder.join("converted").to_string_lossy());
        }
    }

    fn quality_applies(&self, ctx: &UiContext<'_>) -> bool {
        match &self.format {
            None => true,
            Some(id) => ctx
                .encoders
                .iter()
                .find(|e| e.id == id)
                .is_none_or(|e| e.has_quality),
        }
    }

    /// The label of the chosen format.
    pub fn format_label<'c>(&self, ctx: &'c UiContext<'_>) -> &'c str {
        match &self.format {
            None => SAME_FORMAT,
            Some(id) => ctx
                .encoders
                .iter()
                .find(|e| e.id == id)
                .map_or(SAME_FORMAT, |e| e.name),
        }
    }

    /// Pick entry `n` of the format list (0 is "same as source").
    pub fn choose_format(&mut self, n: usize, ctx: &UiContext<'_>) {
        self.format = if n == 0 {
            None
        } else {
            ctx.encoders.get(n - 1).map(|e| e.id.to_string())
        };
    }

    /// The job described by the dialog, or why it is incomplete.
    pub fn job(&self, ctx: &UiContext<'_>) -> Result<BulkJob, String> {
        let source = match &self.source {
            Some(BulkSource::Folder { path, .. }) => BulkSource::Folder {
                path: path.clone(),
                recursive: self.recursive,
            },
            Some(BulkSource::Selection) if !ctx.selected.is_empty() => BulkSource::Selection,
            Some(BulkSource::Selection) => return Err("Choose images or a folder.".into()),
            Some(s) if !s.is_empty() => s.clone(),
            _ => return Err("Choose images or a folder.".into()),
        };
        let size_err = || format!("Enter a width and height from 1 to {MAX_DIMENSION}.");
        let resize = match self.rule {
            SizeRule::Keep => None,
            SizeRule::Percent => Some(ResizeSpec::Percent(
                parse_percent(&self.percent).ok_or_else(|| {
                    format!("Enter a percentage above 0 and at most {MAX_PERCENT}.")
                })?,
            )),
            SizeRule::Fit => {
                let (Some(w), Some(h)) = (parse_side(&self.fit_w), parse_side(&self.fit_h)) else {
                    return Err(size_err());
                };
                Some(ResizeSpec::Fit {
                    width: w,
                    height: h,
                    enlarge: self.enlarge,
                })
            }
            SizeRule::Exact => {
                let (Some(w), Some(h)) = (parse_side(&self.exact_w), parse_side(&self.exact_h))
                else {
                    return Err(size_err());
                };
                Some(ResizeSpec::Exact {
                    width: w,
                    height: h,
                })
            }
        };
        let quality = match self.quality.text().parse::<u8>() {
            Ok(q @ 1..=100) => q,
            _ if !self.quality_applies(ctx) => 90,
            _ => return Err("Enter a quality from 1 to 100.".into()),
        };
        let dest = self.destination.text().trim();
        if dest.is_empty() {
            return Err("Choose a destination folder.".into());
        }
        if let Some(id) = &self.format {
            if !ctx.encoders.iter().any(|e| e.id == id) {
                return Err("Choose an output format.".into());
            }
        }
        Ok(BulkJob {
            source,
            resize,
            format: self.format.clone(),
            quality,
            destination: PathBuf::from(dest),
            overwrite: self.overwrite,
        })
    }

    /// Start the job when it is complete.
    pub fn start(&mut self, ctx: &UiContext<'_>) -> Option<Action> {
        let job = self.job(ctx).ok()?;
        self.running = true;
        self.seen_progress = false;
        Some(Action::RunBulk(job))
    }

    fn submit(&mut self, ctx: &UiContext<'_>) -> Outcome {
        if self.job(ctx).is_err() {
            return Outcome::Stay;
        }
        if self.overwrite == OverwritePolicy::Overwrite {
            return Outcome::ConfirmOverwrite;
        }
        match self.start(ctx) {
            Some(a) => Outcome::Emit(vec![a]),
            None => Outcome::Stay,
        }
    }

    /// React to a form event.
    pub fn on_event(&mut self, event: &FormEvent, ctx: &UiContext<'_>) -> Outcome {
        if self.running {
            return match event {
                FormEvent::Activate(BTN_STOP) => Outcome::Emit(vec![Action::CancelBulk]),
                // Hiding keeps the job running; progress moves to a small
                // indicator. Enter and Escape both hide.
                FormEvent::Activate(BTN_HIDE) | FormEvent::Cancel | FormEvent::Submit => {
                    Outcome::Close(Vec::new())
                }
                _ => Outcome::Stay,
            };
        }
        match event {
            FormEvent::Activate(BTN_ADD_FILES) => Outcome::Emit(vec![Action::PickBulkFiles]),
            FormEvent::Activate(BTN_ADD_FOLDER) => Outcome::Emit(vec![Action::PickBulkFolder]),
            FormEvent::Activate(BTN_BROWSE) => Outcome::Emit(vec![Action::PickBulkDestination]),
            FormEvent::Activate(BTN_CURRENT) => {
                if let Some(folder) = ctx.folder {
                    self.set_folder(folder.to_path_buf());
                }
                Outcome::Stay
            }
            FormEvent::Activate(BTN_SELECTION) => {
                self.use_selection(ctx);
                Outcome::Stay
            }
            FormEvent::Activate(CHK_RECURSIVE) => {
                self.recursive = !self.recursive;
                Outcome::Stay
            }
            FormEvent::Activate(CHK_ENLARGE) => {
                self.enlarge = !self.enlarge;
                Outcome::Stay
            }
            FormEvent::Activate(DD_FORMAT) => Outcome::OpenFormatMenu,
            FormEvent::Segment(SEG_RULE, i) => {
                if let Some(r) = SizeRule::ALL.get(*i) {
                    self.rule = *r;
                }
                Outcome::Stay
            }
            FormEvent::Segment(SEG_OVERWRITE, i) => {
                if let Some(o) = OVERWRITE.get(*i) {
                    self.overwrite = *o;
                }
                Outcome::Stay
            }
            FormEvent::Activate(BTN_CANCEL) | FormEvent::Cancel => Outcome::Close(Vec::new()),
            FormEvent::Activate(BTN_RUN) | FormEvent::Submit => self.submit(ctx),
            FormEvent::Copy(text) => Outcome::Emit(vec![Action::CopyText(text.clone())]),
            FormEvent::Paste => Outcome::Emit(vec![Action::RequestPaste]),
            _ => Outcome::Stay,
        }
    }

    fn source_text(&self, ctx: &UiContext<'_>) -> (String, Tone) {
        match &self.source {
            None => ("No images chosen".into(), Tone::Dim),
            Some(BulkSource::Selection) => match ctx.selected.len() {
                0 => ("No images chosen".into(), Tone::Dim),
                1 => ("1 selected image".into(), Tone::Normal),
                n => (format!("{n} selected images"), Tone::Normal),
            },
            Some(BulkSource::Files(list)) if list.len() == 1 => (
                format!(
                    "1 file: {}",
                    list[0]
                        .file_name()
                        .map_or_else(|| list[0].to_string_lossy(), |n| n.to_string_lossy())
                ),
                Tone::Normal,
            ),
            Some(BulkSource::Files(list)) if list.is_empty() => {
                ("No images chosen".into(), Tone::Dim)
            }
            Some(BulkSource::Files(list)) => (format!("{} files", list.len()), Tone::Normal),
            Some(BulkSource::Folder { path, .. }) => {
                (format!("Folder: {}", path.to_string_lossy()), Tone::Normal)
            }
        }
    }

    fn build_running(&self, ctx: &UiContext<'_>, b: &mut FormBuilder<'_>) {
        b.title("Bulk convert");
        match ctx.bulk {
            Some(p) => {
                let status = if p.total > 0 {
                    format!("Converting {} of {}", p.done.min(p.total), p.total)
                } else {
                    "Converting...".to_string()
                };
                b.text(status, Tone::Normal);
                b.progress_row(p.fraction());
                b.text(p.label.to_string(), Tone::Dim);
            }
            None => {
                b.text("Starting...", Tone::Normal);
                b.progress_row(None);
                b.text(String::new(), Tone::Dim);
            }
        }
        b.space(8.0);
        b.buttons_row(
            &[
                (BTN_STOP, "Stop", ButtonStyle::Secondary, true),
                (BTN_HIDE, "Hide", ButtonStyle::Primary, true),
            ],
            true,
        );
    }
}

impl FormHost for BulkDialog {
    fn build(&self, ctx: &UiContext<'_>, b: &mut FormBuilder<'_>) {
        if self.running {
            self.build_running(ctx, b);
            return;
        }
        b.title("Bulk convert and resize");
        let (text, tone) = self.source_text(ctx);
        b.text(text, tone);
        if !ctx.selected.is_empty() {
            let label = format!("Selected images ({})", ctx.selected.len());
            b.buttons_row(
                &[(BTN_SELECTION, label.as_str(), ButtonStyle::Secondary, true)],
                false,
            );
        }
        let mut buttons: Vec<(Id, &str, ButtonStyle, bool)> = vec![
            (BTN_ADD_FILES, "Add files...", ButtonStyle::Secondary, true),
            (
                BTN_ADD_FOLDER,
                "Add folder...",
                ButtonStyle::Secondary,
                true,
            ),
        ];
        if ctx.folder.is_some() {
            buttons.push((BTN_CURRENT, "Current folder", ButtonStyle::Secondary, true));
        }
        b.buttons_row(&buttons, false);
        let is_folder = matches!(self.source, Some(BulkSource::Folder { .. }));
        b.check_row(
            CHK_RECURSIVE,
            "Include subfolders",
            self.recursive,
            is_folder,
        );
        b.space(4.0);
        b.segmented_row(
            Some("Size"),
            SEG_RULE,
            &SizeRule::LABELS,
            self.rule.index(),
            true,
        );
        match self.rule {
            SizeRule::Keep => {}
            SizeRule::Percent => b.field_row("Scale", F_PERCENT, 90.0, Some("%"), true),
            SizeRule::Fit => {
                b.field_row("Max width", F_FIT_W, 90.0, Some("px"), true);
                b.field_row("Max height", F_FIT_H, 90.0, Some("px"), true);
                b.check_row(CHK_ENLARGE, "Enlarge smaller images", self.enlarge, true);
            }
            SizeRule::Exact => {
                b.field_row("Width", F_EXACT_W, 90.0, Some("px"), true);
                b.field_row("Height", F_EXACT_H, 90.0, Some("px"), true);
            }
        }
        b.dropdown_row("Format", DD_FORMAT, self.format_label(ctx), true);
        b.field_row(
            "Quality",
            F_QUALITY,
            60.0,
            Some("1 to 100"),
            self.quality_applies(ctx),
        );
        b.field_button_row("Save to", F_DEST, BTN_BROWSE, "Browse...", true);
        let selected = OVERWRITE
            .iter()
            .position(|o| *o == self.overwrite)
            .unwrap_or(0);
        b.segmented_row(
            Some("If file exists"),
            SEG_OVERWRITE,
            &OVERWRITE_LABELS,
            selected,
            true,
        );
        b.space(4.0);
        let valid = match self.job(ctx) {
            Ok(_) => {
                b.text("Ready.", Tone::Dim);
                true
            }
            Err(msg) => {
                b.text(msg, Tone::Error);
                false
            }
        };
        b.space(6.0);
        b.buttons_row(
            &[
                (BTN_CANCEL, "Cancel", ButtonStyle::Secondary, true),
                (BTN_RUN, "Run", ButtonStyle::Primary, valid),
            ],
            true,
        );
    }

    fn field(&self, id: Id) -> Option<&TextField> {
        Some(match id {
            F_PERCENT => &self.percent,
            F_FIT_W => &self.fit_w,
            F_FIT_H => &self.fit_h,
            F_EXACT_W => &self.exact_w,
            F_EXACT_H => &self.exact_h,
            F_QUALITY => &self.quality,
            F_DEST => &self.destination,
            _ => return None,
        })
    }

    fn field_mut(&mut self, id: Id) -> Option<&mut TextField> {
        Some(match id {
            F_PERCENT => &mut self.percent,
            F_FIT_W => &mut self.fit_w,
            F_FIT_H => &mut self.fit_h,
            F_EXACT_W => &mut self.exact_w,
            F_EXACT_H => &mut self.exact_h,
            F_QUALITY => &mut self.quality,
            F_DEST => &mut self.destination,
            _ => return None,
        })
    }
}

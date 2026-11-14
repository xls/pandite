//! The interface state machine.
//!
//! [`Ui`] owns every piece of interface state (hover, pointer capture,
//! focus, open menu and dialogs, crop selection, gallery scroll, toasts,
//! icon textures). The application calls [`Ui::handle_event`] for each
//! input event and [`Ui::build`] once per frame, passing a fresh
//! [`UiContext`] both times.

mod draw;
mod events;

use crate::action::{Action, PixelRect, ZOOM_STEP};
use crate::bulk_dialog::BulkDialog;
use crate::command::Command;
use crate::confirm::{ConfirmDialog, ConfirmOwner, ConfirmRequest};
use crate::context::UiContext;
use crate::crop::{CropDrag, CropHit, CropState, Mapping};
use crate::form::{FieldGeom, FormHost, FormLayout, FormState, Id};
use crate::gallery::{GalleryLayout, GalleryState};
use crate::icon::IconCache;
use crate::input::{CursorShape, Modifiers, PointerButton};
use crate::menu::{MenuItem, MenuOwner, MenuPick, MenuState};
use crate::metrics::{self, Metrics};
use crate::resize_dialog::{self, ResizeDialog};
use crate::text_select::{TextGeom, TextSelection};
use crate::theme::Theme;
use crate::toolbar::ToolbarLayout;
use pandite_core::{Color, DisplayMode, Point, Rect, TextureSink};
use std::collections::VecDeque;
use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

/// The broad interaction mode, for the application's information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UiMode {
    /// Viewing; shortcuts and pointer gestures act on the image.
    Normal,
    /// Crop mode; drags edit the selection.
    Crop,
    /// A dialog, the shortcut overlay or a menu holds the input.
    Modal,
}

/// A modal dialog.
#[derive(Clone, Debug)]
pub(crate) enum Dialog {
    Resize(ResizeDialog),
    Bulk(BulkDialog),
}

impl Dialog {
    pub fn host(&self) -> &dyn FormHost {
        match self {
            Dialog::Resize(d) => d,
            Dialog::Bulk(d) => d,
        }
    }

    pub fn host_mut(&mut self) -> &mut dyn FormHost {
        match self {
            Dialog::Resize(d) => d,
            Dialog::Bulk(d) => d,
        }
    }
}

/// A button of the crop bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CropButton {
    Apply,
    Cancel,
}

/// What lies under the pointer, topmost first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Target {
    None,
    MenuItem(usize),
    MenuPanel,
    Help,
    Form { confirm: bool, id: Id, index: usize },
    FormPanel,
    Backdrop,
    Toolbar(crate::command::Command),
    More,
    ToolbarBar,
    CropBar(CropButton),
    CropBarPanel,
    InfoPanel,
    Gallery(Option<usize>),
    GalleryCheck(usize),
    EmptyOpen,
    Content,
}

/// Who owns the pointer between a press and its release.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Capture {
    Toolbar(crate::command::Command),
    More,
    CropBar(CropButton),
    Menu(Option<usize>),
    Form {
        confirm: bool,
        id: Id,
        index: usize,
        field: bool,
    },
    Gallery {
        index: Option<usize>,
        /// The press was on the check box of `index`.
        check: bool,
        start: Point,
        start_scroll: f32,
        dragging: bool,
    },
    Pan {
        last: Point,
        start: Point,
        /// The pointer went past the drag threshold: a pan, not a click.
        panned: bool,
    },
    /// A left press on a recognized word: a drag from it selects text.
    Text {
        id: u64,
        /// The pressed word.
        anchor: usize,
        start: Point,
        dragging: bool,
        grip: TextGrip,
    },
    Crop,
    RightPress,
    EmptyOpen,
    Swallow,
}

/// What a held left press on text selects while the pointer moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextGrip {
    /// A single press: a drag selects from the pressed word to the word
    /// under the pointer. A release without a drag clears the selection.
    Click,
    /// The second press of a double click: the pressed word is selected,
    /// and a drag extends the selection by whole words.
    Words,
    /// The third press of a triple click: the pressed word's line is
    /// selected, and a drag extends the selection by whole lines.
    Lines,
}

/// A transient message.
#[derive(Clone, Debug)]
pub(crate) struct Toast {
    pub text: String,
    pub start: Duration,
}

/// The interface state machine. See the crate documentation for the
/// frame loop.
#[derive(Debug)]
pub struct Ui {
    now: Duration,
    last_update: Option<Duration>,
    pointer: Option<Point>,
    mods: Modifiers,
    capture: Option<(Capture, PointerButton)>,
    hover: Target,
    hover_since: Duration,
    tooltip_blocked: bool,
    last_press: Option<(Duration, Point)>,
    last_activity: Duration,
    toolbar_alpha: f32,
    crop: Option<CropState>,
    dialog: Option<Dialog>,
    dialog_form: FormState,
    bulk_memory: Option<BulkDialog>,
    confirm: Option<ConfirmDialog>,
    confirm_form: FormState,
    confirm_queue: VecDeque<(ConfirmOwner, ConfirmRequest)>,
    menu: Option<MenuState>,
    help: bool,
    gallery: GalleryState,
    toasts: Vec<Toast>,
    icons: IconCache,
    cursor: CursorShape,
    toolbar: ToolbarLayout,
    field_geoms: Vec<FieldGeom>,
    animating: bool,
    next_redraw: Option<Duration>,
    text_sel: Option<TextSelection>,
    /// Time, place and count of the last press on a word, for double and
    /// triple clicks.
    text_clicks: Option<(Duration, Point, u32)>,
    /// Where the information panel was drawn in the last frame.
    info_rect: Option<Rect>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    /// A fresh interface: toolbar visible, nothing open.
    pub fn new() -> Self {
        Self {
            now: Duration::ZERO,
            last_update: None,
            pointer: None,
            mods: Modifiers::NONE,
            capture: None,
            hover: Target::None,
            hover_since: Duration::ZERO,
            tooltip_blocked: false,
            last_press: None,
            last_activity: Duration::ZERO,
            toolbar_alpha: 1.0,
            crop: None,
            dialog: None,
            dialog_form: FormState::default(),
            bulk_memory: None,
            confirm: None,
            confirm_form: FormState::default(),
            confirm_queue: VecDeque::new(),
            menu: None,
            help: false,
            gallery: GalleryState::default(),
            toasts: Vec::new(),
            icons: IconCache::default(),
            cursor: CursorShape::Default,
            toolbar: ToolbarLayout::default(),
            field_geoms: Vec::new(),
            animating: false,
            next_redraw: None,
            text_sel: None,
            text_clicks: None,
            info_rect: None,
        }
    }

    // ----- public queries -------------------------------------------------

    /// The window area left for the image: the window minus the gallery
    /// strip when it shows. The toolbar overlays the image and does not
    /// reduce it. Pass this as the viewport to [`pandite_core::ViewState`].
    pub fn content_rect(&self, ctx: &UiContext<'_>) -> Rect {
        let m = Metrics::new(ctx.scale_factor);
        let window = ctx.window_rect();
        match self.gallery_panel(ctx, &m) {
            Some(panel) => Rect::new(window.x, window.y, window.w, (panel.y - window.y).max(0.0)),
            None => window,
        }
    }

    /// True while an animation runs (toolbar fade, gallery scroll, toast
    /// fade, loading indicator): keep drawing frames.
    pub fn wants_redraw(&self) -> bool {
        self.animating
    }

    /// When the next timed change is due (toolbar hide, tooltip, toast
    /// fade, loading overlay), on the [`UiContext::now`] clock. Schedule a
    /// frame for then even without input.
    pub fn next_redraw_at(&self) -> Option<Duration> {
        self.next_redraw
    }

    /// Gallery indices drawn in the last frame. Decode their thumbnails
    /// first and read ahead around this range.
    pub fn gallery_visible_range(&self, ctx: &UiContext<'_>) -> Range<usize> {
        let m = Metrics::new(ctx.scale_factor);
        match self.gallery_layout(ctx, &m) {
            Some(layout) => layout.visible_range(layout.clamp_scroll(self.gallery.scroll)),
            None => 0..0,
        }
    }

    /// The side in physical pixels of a gallery cell. Decode thumbnails at
    /// least this large on their short side for crisp cells; the strip
    /// crops them to squares.
    pub fn thumbnail_px(ctx: &UiContext<'_>) -> u32 {
        Metrics::new(ctx.scale_factor).cell() as u32
    }

    /// The number of thumbnails that fit whole in the gallery strip at the
    /// window width of `ctx`, with the strip's side padding: the step of
    /// Page Up and Page Down. The same number when the strip is hidden.
    /// At least 1.
    pub fn gallery_page(ctx: &UiContext<'_>) -> usize {
        let m = Metrics::new(ctx.scale_factor);
        let room = ctx.window_rect().w - 2.0 * m.gallery_pad() - m.cell();
        let pitch = m.cell() + m.cell_gap();
        if room.is_finite() && room > 0.0 {
            (room / pitch).floor() as usize + 1
        } else {
            1
        }
    }

    /// The pointer shape wanted at the pointer position.
    pub fn cursor(&self) -> CursorShape {
        self.cursor
    }

    /// Where the information panel was drawn by the last [`Ui::build`], in
    /// window pixels: anchored to the bottom right corner of
    /// [`Ui::content_rect`] with a margin. `None` when it was not drawn
    /// (hidden, no rows, crop mode, or a window too small for one row).
    pub fn info_panel_rect(&self) -> Option<Rect> {
        self.info_rect
    }

    /// True while a text field has keyboard focus. Enable the input method
    /// (IME) then, and treat Ctrl+V as a paste request.
    pub fn wants_text_input(&self) -> bool {
        if self.confirm.is_some() || self.menu.is_some() || self.help {
            return false;
        }
        match (&self.dialog, self.dialog_form.focus) {
            (Some(d), Some(f)) => d.host().field(f).is_some(),
            _ => false,
        }
    }

    /// The current interaction mode.
    pub fn mode(&self) -> UiMode {
        if self.dialog.is_some() || self.confirm.is_some() || self.help || self.menu.is_some() {
            UiMode::Modal
        } else if self.crop.is_some() {
            UiMode::Crop
        } else {
            UiMode::Normal
        }
    }

    /// The background color for the window behind the image.
    pub fn clear_color(ctx: &UiContext<'_>) -> Color {
        Theme::for_kind(ctx.theme).canvas
    }

    /// True when `cmd` can run now.
    pub fn command_enabled(&self, cmd: Command, ctx: &UiContext<'_>) -> bool {
        if cmd.needs_image() && ctx.image.is_none() {
            return false;
        }
        if self.crop.is_some() && !cmd.allowed_in_crop() {
            return false;
        }
        match cmd {
            Command::Undo => ctx.can_undo,
            Command::Previous
            | Command::Next
            | Command::PagePrevious
            | Command::PageNext
            | Command::First
            | Command::Last => ctx.count > 1,
            // A file that failed to open can go too: the folder still
            // lists it. The same holds for its selection.
            Command::MoveToTrash | Command::DeleteFile | Command::ToggleSelect => {
                ctx.image.is_some() || ctx.count > 0
            }
            Command::ClearSelection => !ctx.selected.is_empty(),
            Command::CropApply => self.crop.as_ref().is_some_and(|c| c.pixel_rect().is_some()),
            Command::CropSelectAll | Command::CropCancel => self.crop.is_some(),
            Command::SelectAllText => TextGeom::new(ctx).is_some(),
            Command::CopyText => self.selected_text(ctx).is_some(),
            _ => true,
        }
    }

    /// The selected recognized text (lines joined with `"\n"`, words with
    /// one space), when a selection exists for [`UiContext::text`]. Words
    /// a crop cut away are left out.
    pub fn selected_text(&self, ctx: &UiContext<'_>) -> Option<String> {
        let sel = self.selection(ctx)?;
        let (first, last) = sel.bounds();
        let text = TextGeom::new(ctx)?.text_of(first, last);
        (!text.is_empty()).then_some(text)
    }

    /// The text selection when it still belongs to the layout in `ctx`.
    fn selection(&self, ctx: &UiContext<'_>) -> Option<TextSelection> {
        let sel = self.text_sel?;
        let text = ctx.text?;
        let (_, last) = sel.bounds();
        (text.id == sel.id && last < text.layout.word_count()).then_some(sel)
    }

    /// True while a menu, dialog or the shortcut overlay holds the input.
    fn modal(&self) -> bool {
        self.menu.is_some() || self.help || self.confirm.is_some() || self.dialog.is_some()
    }

    /// Forget the text selection and end a text drag.
    fn clear_text_selection(&mut self) {
        self.text_sel = None;
        if let Some((Capture::Text { .. }, button)) = self.capture {
            // The release of the held button still arrives; swallow it.
            self.capture = Some((Capture::Swallow, button));
        }
    }

    // ----- public requests ------------------------------------------------

    /// Show a short message that fades out by itself, for example "Path
    /// copied". `now` is the [`UiContext::now`] clock.
    pub fn toast(&mut self, text: impl Into<String>, now: Duration) {
        self.toasts.push(Toast {
            text: text.into(),
            start: now,
        });
        if self.toasts.len() > 3 {
            self.toasts.remove(0);
        }
        self.next_redraw = Some(now);
    }

    /// Ask the user a yes or no question. The answer comes back as
    /// [`Action::ConfirmResult`]. Requests queue when one is already open.
    pub fn show_confirm(&mut self, request: ConfirmRequest) {
        let owner = ConfirmOwner::App(request.id);
        if self.confirm.is_none() {
            self.open_confirm(owner, request);
        } else {
            self.confirm_queue.push_back((owner, request));
        }
    }

    /// Answer [`Action::PickBulkFiles`]: add these files to the bulk job.
    pub fn add_bulk_files(&mut self, files: Vec<PathBuf>) {
        if let Some(b) = self.bulk_mut() {
            b.add_files(files);
        }
    }

    /// Answer [`Action::PickBulkFolder`]: read this folder.
    pub fn set_bulk_folder(&mut self, folder: PathBuf) {
        if let Some(b) = self.bulk_mut() {
            b.set_folder(folder);
        }
    }

    /// Answer [`Action::PickBulkDestination`]: write into this folder.
    pub fn set_bulk_destination(&mut self, folder: PathBuf) {
        if let Some(b) = self.bulk_mut() {
            b.set_destination(&folder);
        }
    }

    /// Run a command as if its shortcut was pressed (for native menus).
    pub fn execute(&mut self, cmd: Command, ctx: &UiContext<'_>) -> Vec<Action> {
        self.update(ctx);
        let mut out = Vec::new();
        self.run_command(cmd, ctx, None, &mut out);
        self.finish(ctx);
        out
    }

    /// Upload icon textures at the size the scale factor needs. Call before
    /// drawing each frame; it does nothing when the icons are current.
    /// Returns true when it uploaded.
    pub fn sync_textures(&mut self, ctx: &UiContext<'_>, sink: &mut dyn TextureSink) -> bool {
        let m = Metrics::new(ctx.scale_factor);
        self.icons.sync(m.icon_px(), sink)
    }

    /// Forget uploaded icons (for example after the GPU device was lost);
    /// the next [`Ui::sync_textures`] uploads them again.
    pub fn invalidate_textures(&mut self) {
        self.icons.invalidate();
    }

    /// The icon pixel size currently uploaded, 0 before the first upload.
    pub fn icon_size(&self) -> u32 {
        self.icons.uploaded_px()
    }

    // ----- internals ------------------------------------------------------

    fn bulk_mut(&mut self) -> Option<&mut BulkDialog> {
        match &mut self.dialog {
            Some(Dialog::Bulk(b)) => Some(b),
            _ => None,
        }
    }

    fn open_confirm(&mut self, owner: ConfirmOwner, request: ConfirmRequest) {
        self.menu = None;
        self.help = false;
        self.confirm_form = FormState {
            focus: Some(request.default_button()),
            ..FormState::default()
        };
        self.confirm = Some(ConfirmDialog { owner, request });
    }

    fn close_dialog(&mut self) {
        if let Some(Dialog::Bulk(mut b)) = self.dialog.take() {
            b.running = false;
            b.seen_progress = false;
            self.bulk_memory = Some(b);
        }
        self.dialog = None;
        self.dialog_form = FormState::default();
        if matches!(
            self.menu,
            Some(MenuState {
                owner: MenuOwner::FormatList,
                ..
            })
        ) {
            self.menu = None;
        }
    }

    pub(crate) fn gallery_panel(&self, ctx: &UiContext<'_>, m: &Metrics) -> Option<Rect> {
        let h = m.gallery_h();
        let win = ctx.window_rect();
        let shown = ctx.gallery_visible && ctx.count > 0 && win.h >= h + m.px(120.0);
        shown.then(|| Rect::new(win.x, win.bottom() - h, win.w, h))
    }

    pub(crate) fn gallery_layout(&self, ctx: &UiContext<'_>, m: &Metrics) -> Option<GalleryLayout> {
        self.gallery_panel(ctx, m)
            .map(|panel| GalleryLayout::new(panel, ctx.count, m))
    }

    /// True when the toolbar should be showing.
    fn toolbar_wanted(&self, ctx: &UiContext<'_>, m: &Metrics) -> bool {
        if self.crop.is_some() {
            return false;
        }
        ctx.image.is_none()
            || self.menu.is_some()
            || self.dialog.is_some()
            || self.confirm.is_some()
            || self.help
            || matches!(self.capture, Some((Capture::Toolbar(_) | Capture::More, _)))
            || self
                .pointer
                .is_some_and(|p| p.y >= 0.0 && p.y < m.toolbar_h() && p.x >= 0.0)
            || ctx.now < self.last_activity + metrics::IDLE_HIDE
    }

    fn toolbar_interactive(&self, ctx: &UiContext<'_>, m: &Metrics) -> bool {
        self.crop.is_none() && (self.toolbar_alpha > 0.5 || self.toolbar_wanted(ctx, m))
    }

    /// Advance time-based state to `ctx.now`.
    fn update(&mut self, ctx: &UiContext<'_>) {
        let now = ctx.now;
        let prev = self.last_update.filter(|&t| t <= now);
        let dt = prev.map_or(0.0, |t| (now - t).as_secs_f32());
        self.last_update = Some(now);
        self.now = now;
        let m = Metrics::new(ctx.scale_factor);
        self.toolbar.compute(ctx.window_size.w, &m);

        // A text selection ends when its layout or image goes away.
        if self.text_sel.is_some() && self.selection(ctx).is_none() {
            self.clear_text_selection();
        }
        if let Some((Capture::Text { id, .. }, _)) = self.capture {
            if TextGeom::new(ctx).is_none_or(|g| g.id != id) {
                self.clear_text_selection();
            }
        }
        // Crop mode ends when its image goes away or changes size.
        if let Some(crop) = &self.crop {
            if ctx.image.map(|i| (i.width, i.height)) != Some(crop.image) {
                self.crop = None;
                if matches!(self.capture, Some((Capture::Crop, _))) {
                    self.capture = None;
                }
            }
        }
        // A running bulk job closes its dialog when it ends.
        let mut finished = false;
        if let Some(Dialog::Bulk(b)) = &mut self.dialog {
            if b.running {
                if ctx.bulk.is_some() {
                    b.seen_progress = true;
                } else if b.seen_progress {
                    finished = true;
                }
            }
        }
        if finished {
            self.close_dialog();
        }
        if self.confirm.is_none() {
            if let Some((owner, request)) = self.confirm_queue.pop_front() {
                self.open_confirm(owner, request);
            }
        }
        if let Some(layout) = self.gallery_layout(ctx, &m) {
            self.gallery.track(&layout, ctx.index, ctx.count);
            self.gallery.animate(dt);
            self.gallery.scroll = layout.clamp_scroll(self.gallery.scroll);
        } else {
            self.gallery.reset();
        }
        let wanted = self.toolbar_wanted(ctx, &m);
        if self.crop.is_some() {
            // The crop bar replaces the toolbar.
            self.toolbar_alpha = 0.0;
        } else if wanted {
            self.toolbar_alpha += dt / metrics::FADE_IN.as_secs_f32();
        } else {
            // Outside crop mode the toolbar is unwanted only after the idle
            // timer ran out; the fade starts then, even when no frame was
            // drawn in between.
            let hide_at = self.last_activity + metrics::IDLE_HIDE;
            let from = prev.map_or(now, |p| p.max(hide_at)).min(now);
            let faded = (now - from).as_secs_f32();
            self.toolbar_alpha -= faded / metrics::FADE_OUT.as_secs_f32();
        }
        self.toolbar_alpha = self.toolbar_alpha.clamp(0.0, 1.0);
        self.toasts
            .retain(|t| now < t.start + metrics::TOAST_HOLD + metrics::TOAST_FADE);
    }

    /// Recompute the redraw schedule after an event or frame.
    fn finish(&mut self, ctx: &UiContext<'_>) {
        let m = Metrics::new(ctx.scale_factor);
        let now = ctx.now;
        let wanted = self.toolbar_wanted(ctx, &m);
        let target = if wanted { 1.0 } else { 0.0 };
        let loading_visible = ctx
            .loading
            .is_some_and(|l| l.elapsed >= metrics::LOADING_DELAY);
        let toast_fading = self
            .toasts
            .iter()
            .any(|t| now >= t.start + metrics::TOAST_HOLD);
        self.animating = (self.toolbar_alpha - target).abs() > f32::EPSILON
            || self.gallery.animating()
            || toast_fading
            || loading_visible;

        let mut next: Option<Duration> = None;
        let mut consider = |t: Duration| {
            if t > now {
                next = Some(next.map_or(t, |n: Duration| n.min(t)));
            }
        };
        if ctx.image.is_some() && self.crop.is_none() {
            consider(self.last_activity + metrics::IDLE_HIDE);
        }
        if matches!(self.hover, Target::Toolbar(_) | Target::More) && !self.tooltip_blocked {
            consider(self.hover_since + metrics::TOOLTIP_DELAY);
        }
        for t in &self.toasts {
            consider(t.start + metrics::TOAST_HOLD);
            consider(t.start + metrics::TOAST_HOLD + metrics::TOAST_FADE);
        }
        if let Some(l) = ctx.loading {
            if l.elapsed < metrics::LOADING_DELAY {
                consider(now + (metrics::LOADING_DELAY - l.elapsed));
            }
        }
        // Background work shows a small spinner; ten frames a second keep
        // it alive without running the GPU flat out during long jobs.
        let bulk_dialog_open = matches!(&self.dialog, Some(Dialog::Bulk(b)) if b.running);
        if ctx.busy.is_some() || (ctx.bulk.is_some() && !bulk_dialog_open) {
            consider(now + metrics::BUSY_FRAME);
        }
        self.next_redraw = next;
    }

    fn dialog_layout(&self, ctx: &UiContext<'_>, m: &Metrics) -> Option<FormLayout> {
        let dialog = self.dialog.as_ref()?;
        let mut layout = FormLayout::default();
        layout.compute(
            dialog.host(),
            ctx,
            m,
            ctx.window_rect(),
            self.dialog_form.scroll,
        );
        Some(layout)
    }

    fn confirm_layout(&self, ctx: &UiContext<'_>, m: &Metrics) -> Option<FormLayout> {
        let confirm = self.confirm.as_ref()?;
        let mut layout = FormLayout::default();
        layout.compute(confirm, ctx, m, ctx.window_rect(), self.confirm_form.scroll);
        Some(layout)
    }

    /// The crop bar and its two buttons.
    pub(crate) fn crop_bar(&self, ctx: &UiContext<'_>, m: &Metrics) -> (Rect, Rect, Rect) {
        let win = ctx.window_rect();
        let margin = m.px(8.0);
        let w = m.px(400.0).min((win.w - 2.0 * margin).max(m.px(60.0)));
        let h = m.px(40.0);
        let bar = Rect::new((win.x + (win.w - w) * 0.5).round(), margin, w, h);
        let bw = m.px(92.0).min(w * 0.3);
        let bh = m.control_h();
        let by = bar.y + ((h - bh) * 0.5).round();
        let pad = m.px(6.0);
        let cancel = Rect::new(bar.right() - pad - bw, by, bw, bh);
        let apply = Rect::new(cancel.x - pad - bw, by, bw, bh);
        (bar, apply, cancel)
    }

    /// The empty screen's Open button.
    pub(crate) fn empty_open_button(&self, ctx: &UiContext<'_>, m: &Metrics) -> Option<Rect> {
        if ctx.image.is_some() || ctx.count > 0 || ctx.loading.is_some() {
            return None;
        }
        let content = self.content_rect(ctx);
        let w = m.px(190.0).min(content.w);
        let h = m.px(36.0);
        Some(Rect::new(
            (content.x + (content.w - w) * 0.5).round(),
            (content.y + content.h * 0.5).round(),
            w,
            h,
        ))
    }

    /// What lies under `p`, topmost first.
    pub(crate) fn hit(&self, p: Point, ctx: &UiContext<'_>, m: &Metrics) -> Target {
        let win = ctx.window_rect();
        if let Some(menu) = &self.menu {
            if menu.rect(win, m).contains(p) {
                return match menu.hit(p, win, m) {
                    Some(i) => Target::MenuItem(i),
                    None => Target::MenuPanel,
                };
            }
            return Target::None;
        }
        if self.help {
            return Target::Help;
        }
        for (confirm, layout) in [
            (true, self.confirm_layout(ctx, m)),
            (false, self.dialog_layout(ctx, m)),
        ] {
            if let Some(layout) = layout {
                if let Some((id, index)) = layout.hit(p) {
                    return Target::Form { confirm, id, index };
                }
                return if layout.panel.contains(p) {
                    Target::FormPanel
                } else {
                    Target::Backdrop
                };
            }
        }
        if self.crop.is_some() {
            let (bar, apply, cancel) = self.crop_bar(ctx, m);
            if apply.contains(p) {
                return Target::CropBar(CropButton::Apply);
            }
            if cancel.contains(p) {
                return Target::CropBar(CropButton::Cancel);
            }
            if bar.contains(p) {
                return Target::CropBarPanel;
            }
        } else if self.toolbar_interactive(ctx, m) && self.toolbar.bar.contains(p) {
            if let Some(cmd) = self.toolbar.button_at(p) {
                return Target::Toolbar(cmd);
            }
            if self.toolbar.more.is_some_and(|r| r.contains(p)) {
                return Target::More;
            }
            return Target::ToolbarBar;
        }
        if self.info_shown(ctx).is_some_and(|r| r.contains(p)) {
            return Target::InfoPanel;
        }
        if let Some(layout) = self.gallery_layout(ctx, m) {
            if layout.panel.contains(p) {
                let scroll = self.gallery.scroll;
                return match layout.hit(p, scroll) {
                    Some(i) if layout.check_hit(i, p, scroll) => Target::GalleryCheck(i),
                    index => Target::Gallery(index),
                };
            }
        }
        if self
            .empty_open_button(ctx, m)
            .is_some_and(|r| r.contains(p))
        {
            return Target::EmptyOpen;
        }
        if self.content_rect(ctx).contains(p) {
            return Target::Content;
        }
        Target::None
    }

    /// True when the context asks for the information panel and the mode
    /// allows it (crop mode hides it, so the whole image stays editable).
    pub(crate) fn info_wanted(&self, ctx: &UiContext<'_>) -> bool {
        ctx.info_visible && !ctx.info.is_empty() && self.crop.is_none()
    }

    /// The information panel rectangle of the last frame, while the panel
    /// still shows.
    fn info_shown(&self, ctx: &UiContext<'_>) -> Option<Rect> {
        self.info_rect.filter(|_| self.info_wanted(ctx))
    }

    /// The crop mapping for the current image.
    fn crop_mapping(ctx: &UiContext<'_>) -> Option<Mapping> {
        ctx.image.as_ref().and_then(Mapping::new)
    }

    fn start_crop(&mut self, ctx: &UiContext<'_>) -> bool {
        let Some(info) = ctx.image else { return false };
        self.menu = None;
        self.help = false;
        // Crop mode owns the pointer; a text selection would only be in
        // the way.
        self.clear_text_selection();
        self.crop = Some(CropState::new((info.width, info.height)));
        true
    }

    fn open_resize(&mut self, ctx: &UiContext<'_>) -> bool {
        let Some(info) = ctx.image else { return false };
        self.menu = None;
        self.help = false;
        self.dialog = Some(Dialog::Resize(ResizeDialog::new(info.width, info.height)));
        self.dialog_form = FormState {
            focus: Some(resize_dialog::F_PERCENT),
            ..FormState::default()
        };
        true
    }

    fn open_bulk(&mut self, ctx: &UiContext<'_>) {
        self.menu = None;
        self.help = false;
        let mut dialog = self.bulk_memory.take().unwrap_or_default();
        if ctx.bulk.is_some() {
            // A hidden job is still running: show its progress, not a form
            // that could start a second one.
            dialog.running = true;
            dialog.seen_progress = true;
        } else {
            // Selected images are the source when there are any; a
            // selection remembered from an earlier opening is gone.
            dialog.follow_selection(ctx);
        }
        self.dialog = Some(Dialog::Bulk(dialog));
        self.dialog_form = FormState::default();
    }

    /// Open the context menu at `pos`.
    fn open_context_menu(&mut self, pos: Point, ctx: &UiContext<'_>) {
        let content = self.content_rect(ctx);
        let anchor = (ctx.image.is_some() && content.contains(pos)).then_some(pos);
        let mut commands: Vec<Option<Command>> = if self.crop.is_some() {
            vec![
                Some(Command::CropApply),
                Some(Command::CropSelectAll),
                Some(Command::CropCancel),
                None,
                Some(Command::Fit),
                Some(Command::ActualSize),
                Some(Command::ZoomIn),
                Some(Command::ZoomOut),
            ]
        } else {
            CONTEXT_MENU.to_vec()
        };
        // The text commands appear only while the image has text.
        if self.crop.is_none() && TextGeom::new(ctx).is_some() {
            if let Some(at) = commands.iter().position(|c| *c == Some(Command::CopyPath)) {
                commands.insert(at + 1, Some(Command::SelectAllText));
                commands.insert(at + 1, Some(Command::CopyText));
            }
        }
        let items = commands
            .iter()
            .map(|c| match c {
                Some(cmd) => self.menu_entry(*cmd, ctx),
                None => MenuItem::Separator,
            })
            .collect();
        self.menu = Some(MenuState::new(MenuOwner::Context { anchor }, pos, items));
    }

    fn menu_entry(&self, cmd: Command, ctx: &UiContext<'_>) -> MenuItem {
        let checked = match cmd {
            Command::ToggleSelect => ctx.current_selected(),
            Command::ToggleGallery => ctx.gallery_visible,
            Command::ToggleInfo => ctx.info_visible,
            Command::Fullscreen => ctx.fullscreen,
            _ => false,
        };
        MenuItem::Entry {
            label: cmd.label().to_string(),
            shortcut: cmd.shortcut_label(),
            enabled: self.command_enabled(cmd, ctx),
            checked,
            pick: MenuPick::Command(cmd),
        }
    }

    fn open_overflow_menu(&mut self, ctx: &UiContext<'_>) {
        let Some(more) = self.toolbar.more else {
            return;
        };
        let items = self
            .toolbar
            .overflow
            .iter()
            .map(|cmd| self.menu_entry(*cmd, ctx))
            .collect();
        let origin = Point::new(more.right(), more.bottom() + 2.0);
        let mut menu = MenuState::new(MenuOwner::Overflow, origin, items);
        // Align the menu's right edge with the button.
        let m = Metrics::new(ctx.scale_factor);
        let w = menu.rect(ctx.window_rect(), &m).w;
        menu.origin.x = (more.right() - w).max(0.0);
        self.menu = Some(menu);
    }

    fn open_format_menu(&mut self, ctx: &UiContext<'_>, m: &Metrics) {
        let Some(Dialog::Bulk(b)) = &self.dialog else {
            return;
        };
        let Some(layout) = self.dialog_layout(ctx, m) else {
            return;
        };
        let Some(r) = layout.rect_of(crate::bulk_dialog::DD_FORMAT) else {
            return;
        };
        let mut items = vec![MenuItem::Entry {
            label: crate::bulk_dialog::SAME_FORMAT.to_string(),
            shortcut: None,
            enabled: true,
            checked: b.format.is_none(),
            pick: MenuPick::Choice(0),
        }];
        for (i, e) in ctx.encoders.iter().enumerate() {
            items.push(MenuItem::Entry {
                label: e.name.to_string(),
                shortcut: None,
                enabled: true,
                checked: b.format.as_deref() == Some(e.id),
                pick: MenuPick::Choice(i + 1),
            });
        }
        let mut menu = MenuState::new(
            MenuOwner::FormatList,
            Point::new(r.x, r.bottom() + 2.0),
            items,
        );
        menu.min_w = r.w;
        menu.hover = Some(
            b.format
                .as_deref()
                .and_then(|id| ctx.encoders.iter().position(|e| e.id == id))
                .map_or(0, |p| p + 1),
        );
        self.menu = Some(menu);
    }

    /// Run a command. `origin` is where a context menu opened, used to
    /// zoom at that point.
    fn run_command(
        &mut self,
        cmd: Command,
        ctx: &UiContext<'_>,
        origin: Option<Point>,
        out: &mut Vec<Action>,
    ) {
        if !self.command_enabled(cmd, ctx) {
            return;
        }
        let m = Metrics::new(ctx.scale_factor);
        let step = m.pan_step();
        match cmd {
            Command::Open => out.push(Action::Open),
            Command::SaveAs => out.push(Action::SaveAs),
            Command::CopyPath => out.push(Action::CopyPath),
            Command::CopyImage => out.push(Action::CopyImage),
            Command::MoveToTrash => out.push(Action::MoveToTrash),
            Command::DeleteFile => out.push(Action::DeleteFile),
            Command::ToggleSelect => out.push(Action::ToggleSelect(ctx.index)),
            Command::ClearSelection => out.push(Action::ClearSelection),
            Command::Undo => out.push(Action::Undo),
            Command::RotateLeft => out.push(Action::RotateLeft),
            Command::RotateRight => out.push(Action::RotateRight),
            Command::FlipHorizontal => out.push(Action::FlipHorizontal),
            Command::FlipVertical => out.push(Action::FlipVertical),
            Command::Crop => {
                if self.start_crop(ctx) {
                    out.push(Action::StartCrop);
                }
            }
            Command::Resize => {
                if self.open_resize(ctx) {
                    out.push(Action::OpenResizeDialog);
                }
            }
            Command::BulkResize => {
                self.open_bulk(ctx);
                out.push(Action::OpenBulkDialog);
            }
            Command::Fit => out.push(Action::FitToWindow),
            Command::ActualSize => out.push(Action::OneToOne { anchor: origin }),
            Command::ZoomIn => out.push(match origin {
                Some(anchor) => Action::ZoomAt {
                    factor: ZOOM_STEP,
                    anchor,
                },
                None => Action::ZoomIn,
            }),
            Command::ZoomOut => out.push(match origin {
                Some(anchor) => Action::ZoomAt {
                    factor: 1.0 / ZOOM_STEP,
                    anchor,
                },
                None => Action::ZoomOut,
            }),
            Command::PanUp => out.push(Action::PanBy { dx: 0.0, dy: step }),
            Command::PanDown => out.push(Action::PanBy { dx: 0.0, dy: -step }),
            Command::PanLeft => out.push(Action::PanBy { dx: step, dy: 0.0 }),
            Command::PanRight => out.push(Action::PanBy { dx: -step, dy: 0.0 }),
            Command::Previous => out.push(Action::Previous),
            Command::Next => out.push(Action::Next),
            Command::PagePrevious => {
                let page = Self::gallery_page(ctx).min(isize::MAX as usize) as isize;
                out.push(Action::StepBy(-page));
            }
            Command::PageNext => {
                let page = Self::gallery_page(ctx).min(isize::MAX as usize) as isize;
                out.push(Action::StepBy(page));
            }
            Command::First => out.push(Action::GoTo(0)),
            Command::Last => out.push(Action::GoTo(ctx.count - 1)),
            Command::ToggleGallery => out.push(Action::ToggleGallery),
            Command::ToggleInfo => out.push(Action::ToggleInfo),
            Command::Fullscreen => out.push(Action::ToggleFullscreen),
            Command::ContextMenu => {
                let content = self.content_rect(ctx);
                let pos = self
                    .pointer
                    .filter(|p| ctx.window_rect().contains(*p))
                    .unwrap_or(content.center());
                self.open_context_menu(pos, ctx);
                if let Some(menu) = &mut self.menu {
                    menu.jump(true);
                }
            }
            Command::Help => {
                self.menu = None;
                self.help = !self.help;
            }
            Command::Cancel => self.cancel(ctx, out),
            Command::Quit => out.push(Action::Quit),
            Command::CropApply => {
                if let Some(rect) = self.crop.as_ref().and_then(CropState::pixel_rect) {
                    self.finish_crop(Some(rect), out);
                }
            }
            Command::CropSelectAll => {
                if let Some(crop) = &mut self.crop {
                    crop.select_all();
                }
            }
            Command::CropCancel => self.finish_crop(None, out),
            Command::SelectAllText => {
                if let Some(geom) = TextGeom::new(ctx) {
                    if let Some((anchor, focus)) = geom.visible_bounds() {
                        self.text_sel = Some(TextSelection {
                            id: geom.id,
                            anchor,
                            focus,
                        });
                    }
                }
            }
            Command::CopyText => {
                if let Some(text) = self.selected_text(ctx) {
                    out.push(Action::CopyText(text));
                    // Ctrl+C also copies the image; say which it was.
                    self.toast("Text copied", self.now);
                }
            }
        }
    }

    fn finish_crop(&mut self, rect: Option<PixelRect>, out: &mut Vec<Action>) {
        if self.crop.take().is_none() {
            return;
        }
        if matches!(self.capture, Some((Capture::Crop, _))) {
            self.capture = None;
        }
        self.menu = None;
        out.push(match rect {
            Some(r) => Action::ApplyCrop(r),
            None => Action::CancelCrop,
        });
    }

    /// Escape: close the topmost menu, overlay or dialog. With none open
    /// it quits, whatever else goes on (full screen, crop mode, a text
    /// selection, a load).
    fn cancel(&mut self, ctx: &UiContext<'_>, out: &mut Vec<Action>) {
        if self.menu.is_some() {
            self.menu = None;
        } else if self.help {
            self.help = false;
        } else if self.confirm.is_some() {
            self.confirm_event(&crate::form::FormEvent::Cancel, ctx, out);
        } else if self.dialog.is_some() {
            self.dialog_event(crate::form::FormEvent::Cancel, ctx, out);
        } else {
            out.push(Action::Quit);
        }
    }

    /// Double click on the image: toggle fit and 1:1.
    fn toggle_fit(&self, ctx: &UiContext<'_>, pos: Point, out: &mut Vec<Action>) {
        let Some(info) = ctx.image else { return };
        if info.view.mode == DisplayMode::Fit {
            out.push(Action::OneToOne { anchor: Some(pos) });
        } else {
            out.push(Action::FitToWindow);
        }
    }

    /// The recognized word under window point `p`, outside crop mode. Only
    /// points on the image count.
    fn word_at(&self, p: Point, ctx: &UiContext<'_>, m: &Metrics) -> Option<usize> {
        if self.crop.is_some() || !self.content_rect(ctx).contains(p) {
            return None;
        }
        let geom = TextGeom::new(ctx)?;
        if !geom.image_rect.contains(p) {
            return None;
        }
        geom.word_at(p, m.px(3.0))
    }

    /// True when the image overflows the content area (it can be panned).
    fn pannable(&self, ctx: &UiContext<'_>) -> bool {
        let Some(info) = ctx.image else { return false };
        let c = self.content_rect(ctx);
        info.screen_rect.w > c.w + 0.5 || info.screen_rect.h > c.h + 0.5
    }

    fn crop_cursor(&self, p: Point, ctx: &UiContext<'_>, m: &Metrics) -> CursorShape {
        let (Some(crop), Some(map)) = (&self.crop, Self::crop_mapping(ctx)) else {
            return CursorShape::Default;
        };
        match crop.drag {
            Some(CropDrag::New { .. }) => CursorShape::Crosshair,
            Some(CropDrag::Move { .. }) => CursorShape::Move,
            Some(CropDrag::Resize { handle, .. }) => handle.cursor(),
            None => match crop.hit(p, &map, m.handle_hit()) {
                CropHit::Handle(h) => h.cursor(),
                CropHit::Inside => CursorShape::Move,
                CropHit::Outside => CursorShape::Crosshair,
            },
        }
    }

    fn refresh_cursor(&mut self, ctx: &UiContext<'_>, m: &Metrics) {
        let field_hover = |confirm: bool, id: Id, ui: &Ui| -> bool {
            if confirm {
                false
            } else {
                ui.dialog
                    .as_ref()
                    .is_some_and(|d| d.host().field(id).is_some())
            }
        };
        self.cursor = match self.capture {
            Some((Capture::Pan { .. }, _)) => CursorShape::Grabbing,
            Some((Capture::Text { .. }, _)) => CursorShape::Text,
            Some((Capture::Gallery { dragging: true, .. }, _)) => CursorShape::Grabbing,
            Some((Capture::Crop, _)) => match self.pointer {
                Some(p) => self.crop_cursor(p, ctx, m),
                None => CursorShape::Crosshair,
            },
            Some((Capture::Form { field: true, .. }, _)) => CursorShape::Text,
            _ => match self.hover {
                Target::Toolbar(cmd) if self.command_enabled(cmd, ctx) => CursorShape::Hand,
                Target::More | Target::CropBar(_) | Target::EmptyOpen => CursorShape::Hand,
                Target::Gallery(Some(_)) | Target::GalleryCheck(_) => CursorShape::Hand,
                Target::Form { confirm, id, .. } => {
                    if field_hover(confirm, id, self) {
                        CursorShape::Text
                    } else {
                        CursorShape::Hand
                    }
                }
                Target::Content => match self.pointer {
                    Some(p) if self.crop.is_some() => self.crop_cursor(p, ctx, m),
                    _ if ctx.fullscreen && !self.toolbar_wanted(ctx, m) => CursorShape::Hidden,
                    Some(p) if self.word_at(p, ctx, m).is_some() => CursorShape::Text,
                    _ if self.pannable(ctx) => CursorShape::Grab,
                    _ => CursorShape::Default,
                },
                _ => CursorShape::Default,
            },
        };
    }
}

/// The context menu in normal mode. `None` is a separator.
const CONTEXT_MENU: [Option<Command>; 33] = [
    Some(Command::Open),
    Some(Command::SaveAs),
    Some(Command::CopyImage),
    Some(Command::CopyPath),
    None,
    Some(Command::Undo),
    None,
    Some(Command::ToggleSelect),
    Some(Command::ClearSelection),
    Some(Command::MoveToTrash),
    Some(Command::DeleteFile),
    None,
    Some(Command::RotateLeft),
    Some(Command::RotateRight),
    Some(Command::FlipHorizontal),
    Some(Command::FlipVertical),
    Some(Command::Crop),
    Some(Command::Resize),
    Some(Command::BulkResize),
    None,
    Some(Command::Fit),
    Some(Command::ActualSize),
    Some(Command::ZoomIn),
    Some(Command::ZoomOut),
    None,
    Some(Command::Previous),
    Some(Command::Next),
    None,
    Some(Command::ToggleGallery),
    Some(Command::Fullscreen),
    Some(Command::Help),
    None,
    Some(Command::Quit),
];

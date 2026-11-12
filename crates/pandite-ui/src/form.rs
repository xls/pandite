//! A small form toolkit for modal dialogs: rows of labels, text fields,
//! segmented choices, check boxes, drop-downs, buttons and progress bars.
//!
//! A dialog describes its rows with a [`FormBuilder`]. The layout uses only
//! fixed sizes (no text measurement), so it is a pure function of the
//! dialog state, the window size and the scale factor; event handling and
//! drawing compute the same rectangles. Text that might not fit is elided
//! at draw time.

use crate::context::UiContext;
use crate::icon::Icon;
use crate::input::{Key, Modifiers};
use crate::metrics::Metrics;
use crate::paint;
use crate::text_field::{FieldResponse, TextField};
use crate::theme::{fade, Theme};
use pandite_core::{DrawList, Point, Rect, TextMeasure};

/// Identifies an interactive element inside one dialog. 0 means none.
pub(crate) type Id = u16;

/// Text color role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Normal,
    Dim,
    Error,
}

/// Button look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonStyle {
    /// Filled with the accent color (the Enter default).
    Primary,
    /// Outlined.
    Secondary,
}

/// What an element is.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ElemKind {
    Title(String),
    Text(String, Tone),
    Label(&'static str),
    Suffix(&'static str),
    Field {
        id: Id,
        enabled: bool,
    },
    Button {
        id: Id,
        label: String,
        style: ButtonStyle,
        enabled: bool,
    },
    Segment {
        id: Id,
        index: usize,
        label: String,
        selected: bool,
        enabled: bool,
    },
    Check {
        id: Id,
        label: &'static str,
        checked: bool,
        enabled: bool,
    },
    Dropdown {
        id: Id,
        value: String,
        enabled: bool,
    },
    Progress(Option<f32>),
}

impl ElemKind {
    /// The interactive id and segment index, when the element takes input.
    pub fn target(&self) -> Option<(Id, usize)> {
        match *self {
            ElemKind::Field { id, enabled: true }
            | ElemKind::Button {
                id, enabled: true, ..
            }
            | ElemKind::Check {
                id, enabled: true, ..
            }
            | ElemKind::Dropdown {
                id, enabled: true, ..
            } => Some((id, 0)),
            ElemKind::Segment {
                id,
                index,
                enabled: true,
                ..
            } => Some((id, index)),
            _ => None,
        }
    }
}

/// A placed element.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Elem {
    pub rect: Rect,
    pub kind: ElemKind,
}

/// Collects rows top to bottom in dialog-local coordinates.
pub(crate) struct FormBuilder<'m> {
    m: &'m Metrics,
    width: f32,
    y: f32,
    pub elems: Vec<Elem>,
}

impl<'m> FormBuilder<'m> {
    pub fn new(m: &'m Metrics, width: f32) -> Self {
        Self {
            m,
            width,
            y: m.px(16.0),
            elems: Vec::new(),
        }
    }

    fn pad(&self) -> f32 {
        self.m.px(18.0)
    }

    fn inner_w(&self) -> f32 {
        (self.width - 2.0 * self.pad()).max(1.0)
    }

    fn push(&mut self, rect: Rect, kind: ElemKind) {
        self.elems.push(Elem { rect, kind });
    }

    /// Vertical space.
    pub fn space(&mut self, logical: f32) {
        self.y += self.m.px(logical);
    }

    /// The dialog title.
    pub fn title(&mut self, text: impl Into<String>) {
        let h = self.m.px(28.0);
        let r = Rect::new(self.pad(), self.y, self.inner_w(), h);
        self.push(r, ElemKind::Title(text.into()));
        self.y += h + self.m.px(6.0);
    }

    /// One line of text.
    pub fn text(&mut self, text: impl Into<String>, tone: Tone) {
        let h = self.m.px(22.0);
        let r = Rect::new(self.pad(), self.y, self.inner_w(), h);
        self.push(r, ElemKind::Text(text.into(), tone));
        self.y += h;
    }

    /// Text wrapped by an estimate of characters per line (no measuring,
    /// so the layout stays pure). At most `max_lines` lines.
    pub fn paragraph(&mut self, text: &str, tone: Tone, max_lines: usize) {
        let per_line = ((self.inner_w() / (self.m.text() * 0.56)) as usize).max(8);
        for line in wrap_chars(text, per_line, max_lines) {
            self.text(line, tone);
        }
    }

    fn label(&mut self, label: Option<&'static str>) -> f32 {
        let x = self.pad();
        match label {
            Some(l) => {
                let r = Rect::new(x, self.y, self.m.label_w(), self.m.row_h());
                self.push(r, ElemKind::Label(l));
                x + self.m.label_w()
            }
            None => x,
        }
    }

    fn control_y(&self) -> f32 {
        self.y + ((self.m.row_h() - self.m.control_h()) * 0.5).round()
    }

    /// A labelled text field `field_w` logical pixels wide, with an optional
    /// suffix such as "%" or "px".
    pub fn field_row(
        &mut self,
        label: &'static str,
        id: Id,
        field_w: f32,
        suffix: Option<&'static str>,
        enabled: bool,
    ) {
        let x = self.label(Some(label));
        let cy = self.control_y();
        let ch = self.m.control_h();
        let right = self.pad() + self.inner_w();
        let w = self.m.px(field_w).min(right - x);
        self.push(Rect::new(x, cy, w, ch), ElemKind::Field { id, enabled });
        if let Some(s) = suffix {
            let sx = x + w + self.m.px(8.0);
            self.push(
                Rect::new(sx, self.y, (right - sx).max(0.0), self.m.row_h()),
                ElemKind::Suffix(s),
            );
        }
        self.y += self.m.row_h();
    }

    /// A labelled wide field followed by a button (a path and "Browse").
    pub fn field_button_row(
        &mut self,
        label: &'static str,
        field: Id,
        button: Id,
        button_label: &str,
        enabled: bool,
    ) {
        let x = self.label(Some(label));
        let cy = self.control_y();
        let ch = self.m.control_h();
        let right = self.pad() + self.inner_w();
        let bw = self.m.px(84.0).min((right - x) * 0.5);
        let gap = self.m.px(6.0);
        let fw = (right - x - bw - gap).max(0.0);
        self.push(
            Rect::new(x, cy, fw, ch),
            ElemKind::Field { id: field, enabled },
        );
        self.push(
            Rect::new(x + fw + gap, cy, bw, ch),
            ElemKind::Button {
                id: button,
                label: button_label.to_string(),
                style: ButtonStyle::Secondary,
                enabled,
            },
        );
        self.y += self.m.row_h();
    }

    /// A segmented choice: one of `options`.
    pub fn segmented_row(
        &mut self,
        label: Option<&'static str>,
        id: Id,
        options: &[&str],
        selected: usize,
        enabled: bool,
    ) {
        let x = self.label(label);
        let cy = self.control_y();
        let ch = self.m.control_h();
        let right = self.pad() + self.inner_w();
        let n = options.len().max(1) as f32;
        let total = (right - x).max(0.0);
        for (i, opt) in options.iter().enumerate() {
            let x0 = (x + total * i as f32 / n).round();
            let x1 = (x + total * (i + 1) as f32 / n).round();
            self.push(
                Rect::new(x0, cy, x1 - x0, ch),
                ElemKind::Segment {
                    id,
                    index: i,
                    label: (*opt).to_string(),
                    selected: i == selected,
                    enabled,
                },
            );
        }
        self.y += self.m.row_h();
    }

    /// A check box with its label, indented to the control column.
    pub fn check_row(&mut self, id: Id, label: &'static str, checked: bool, enabled: bool) {
        let x = self.pad() + self.m.label_w();
        let right = self.pad() + self.inner_w();
        let r = Rect::new(
            x,
            self.control_y(),
            (right - x).max(0.0),
            self.m.control_h(),
        );
        self.push(
            r,
            ElemKind::Check {
                id,
                label,
                checked,
                enabled,
            },
        );
        self.y += self.m.row_h();
    }

    /// A labelled drop-down showing `value`.
    pub fn dropdown_row(&mut self, label: &'static str, id: Id, value: &str, enabled: bool) {
        let x = self.label(Some(label));
        let right = self.pad() + self.inner_w();
        let w = self.m.px(200.0).min(right - x);
        let r = Rect::new(x, self.control_y(), w, self.m.control_h());
        self.push(
            r,
            ElemKind::Dropdown {
                id,
                value: value.to_string(),
                enabled,
            },
        );
        self.y += self.m.row_h();
    }

    /// A row of buttons. `right_align` puts them at the right edge with a
    /// fixed width (dialog footers); otherwise they share the full width.
    pub fn buttons_row(&mut self, buttons: &[(Id, &str, ButtonStyle, bool)], right_align: bool) {
        let gap = self.m.px(8.0);
        let h = self.m.control_h();
        let avail = self.inner_w();
        let n = buttons.len().max(1) as f32;
        let share = ((avail - gap * (n - 1.0)) / n).max(0.0).floor();
        let bw = if right_align {
            self.m.px(96.0).min(share)
        } else {
            share
        };
        let total = bw * n + gap * (n - 1.0);
        let mut x = if right_align {
            self.pad() + self.inner_w() - total
        } else {
            self.pad()
        };
        let y = self.control_y();
        for (id, label, style, enabled) in buttons {
            self.push(
                Rect::new(x, y, bw, h),
                ElemKind::Button {
                    id: *id,
                    label: (*label).to_string(),
                    style: *style,
                    enabled: *enabled,
                },
            );
            x += bw + gap;
        }
        self.y += self.m.row_h();
    }

    /// A progress bar; `None` is indeterminate.
    pub fn progress_row(&mut self, fraction: Option<f32>) {
        let h = self.m.px(8.0);
        let r = Rect::new(self.pad(), self.y + self.m.px(6.0), self.inner_w(), h);
        self.push(r, ElemKind::Progress(fraction));
        self.y += h + self.m.px(14.0);
    }

    /// Total height including the bottom padding.
    pub fn height(&self) -> f32 {
        self.y + self.m.px(12.0)
    }
}

/// Greedy word wrap by character count.
pub(crate) fn wrap_chars(text: &str, per_line: usize, max_lines: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let needed = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };
        if needed > per_line && !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > max_lines.max(1) {
        let keep = max_lines.max(1);
        let rest = lines[keep - 1..].join(" ");
        lines.truncate(keep - 1);
        lines.push(rest);
    }
    lines
}

/// A dialog that the form toolkit can lay out and edit.
pub(crate) trait FormHost {
    /// Describe the rows.
    fn build(&self, ctx: &UiContext<'_>, b: &mut FormBuilder<'_>);
    /// The text field with this id.
    fn field(&self, id: Id) -> Option<&TextField>;
    /// The text field with this id, mutable.
    fn field_mut(&mut self, id: Id) -> Option<&mut TextField>;
}

/// The laid-out dialog in window coordinates.
#[derive(Clone, Debug, Default)]
pub(crate) struct FormLayout {
    /// The visible dialog panel.
    pub panel: Rect,
    /// Height of all rows (may exceed the panel when the window is small).
    pub content_h: f32,
    pub elems: Vec<Elem>,
}

impl FormLayout {
    /// Lay out `host` centered in `window`, scrolled by `scroll`.
    pub fn compute(
        &mut self,
        host: &dyn FormHost,
        ctx: &UiContext<'_>,
        m: &Metrics,
        window: Rect,
        scroll: f32,
    ) {
        let margin = m.px(12.0).min((window.w * 0.05).round());
        let width = m
            .dialog_w()
            .min((window.w - 2.0 * margin).max(m.px(120.0)))
            .floor();
        let mut b = FormBuilder::new(m, width);
        host.build(ctx, &mut b);
        let content_h = b.height();
        let max_h = (window.h - 2.0 * margin).max(m.px(60.0)).floor();
        let h = content_h.min(max_h);
        let x = (window.x + (window.w - width) * 0.5).round();
        let y = (window.y + (window.h - h) * 0.5).round();
        self.panel = Rect::new(x, y, width, h);
        self.content_h = content_h;
        let scroll = scroll.clamp(0.0, (content_h - h).max(0.0));
        self.elems.clear();
        for mut e in b.elems {
            e.rect.x += x;
            e.rect.y += y - scroll;
            self.elems.push(e);
        }
    }

    /// Largest scroll offset.
    pub fn max_scroll(&self) -> f32 {
        (self.content_h - self.panel.h).max(0.0)
    }

    /// The interactive element under `p`.
    pub fn hit(&self, p: Point) -> Option<(Id, usize)> {
        if !self.panel.contains(p) {
            return None;
        }
        self.elems
            .iter()
            .filter(|e| e.rect.contains(p))
            .find_map(|e| e.kind.target())
    }

    /// Focusable ids in order, without repeats.
    pub fn focus_order(&self) -> Vec<Id> {
        let mut order: Vec<Id> = Vec::new();
        for e in &self.elems {
            if let Some((id, _)) = e.kind.target() {
                if !order.contains(&id) {
                    order.push(id);
                }
            }
        }
        order
    }

    /// The first element rectangle of `id`.
    pub fn rect_of(&self, id: Id) -> Option<Rect> {
        self.elems
            .iter()
            .find(|e| e.kind.target().map(|t| t.0) == Some(id))
            .map(|e| e.rect)
    }

    /// Number of segments in group `id`.
    pub fn segment_count(&self, id: Id) -> usize {
        self.elems
            .iter()
            .filter(|e| matches!(e.kind, ElemKind::Segment { id: sid, .. } if sid == id))
            .count()
    }

    /// The selected index of segment group `id`.
    pub fn segment_selected(&self, id: Id) -> Option<usize> {
        self.elems.iter().find_map(|e| match e.kind {
            ElemKind::Segment {
                id: sid,
                index,
                selected: true,
                ..
            } if sid == id => Some(index),
            _ => None,
        })
    }

    /// True when `id` is a text field.
    pub fn is_field(&self, id: Id) -> bool {
        self.elems
            .iter()
            .any(|e| matches!(e.kind, ElemKind::Field { id: fid, .. } if fid == id))
    }

    /// The kind of the first element with interactive id `id`.
    pub fn kind_of(&self, id: Id) -> Option<&ElemKind> {
        self.elems
            .iter()
            .find(|e| e.kind.target().map(|t| t.0) == Some(id))
            .map(|e| &e.kind)
    }
}

/// Interaction state shared by every dialog.
#[derive(Clone, Debug, Default)]
pub(crate) struct FormState {
    pub focus: Option<Id>,
    pub pressed: Option<(Id, usize)>,
    pub hover: Option<(Id, usize)>,
    pub scroll: f32,
}

/// What the generic form logic asks the dialog to do.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FormEvent {
    /// A button, check box or drop-down was activated.
    Activate(Id),
    /// Segment `index` of group `id` was chosen.
    Segment(Id, usize),
    /// The text of field `id` changed.
    FieldChanged(Id),
    /// Enter outside a button: run the default action.
    Submit,
    /// Escape: close or cancel.
    Cancel,
    /// Put this text on the clipboard.
    Copy(String),
    /// Ask for clipboard text.
    Paste,
}

/// What a dialog does after an event.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Outcome {
    /// Stay open.
    Stay,
    /// Close, emitting these actions.
    Close(Vec<crate::action::Action>),
    /// Stay open, emitting these actions.
    Emit(Vec<crate::action::Action>),
    /// Ask the user to confirm replacing files, then start the job.
    ConfirmOverwrite,
    /// Open the output format list below the drop-down.
    OpenFormatMenu,
}

impl FormState {
    /// Move focus to the next (`dir` = 1) or previous focusable element.
    pub fn cycle_focus(&mut self, layout: &FormLayout, dir: i32) {
        let order = layout.focus_order();
        if order.is_empty() {
            self.focus = None;
            return;
        }
        let n = order.len() as i32;
        let current = self
            .focus
            .and_then(|f| order.iter().position(|&o| o == f))
            .map_or(if dir > 0 { -1 } else { n }, |p| p as i32);
        let next = (current + dir).rem_euclid(n);
        self.focus = Some(order[next as usize]);
    }

    /// Scroll so the focused element is inside the panel.
    pub fn scroll_to_focus(&mut self, layout: &FormLayout) {
        let Some(id) = self.focus else { return };
        let Some(r) = layout.rect_of(id) else { return };
        let panel = layout.panel;
        if r.y < panel.y {
            self.scroll -= panel.y - r.y;
        } else if r.bottom() > panel.bottom() {
            self.scroll += r.bottom() - panel.bottom();
        }
        self.scroll = self.scroll.clamp(0.0, layout.max_scroll());
    }

    /// Generic key handling. Returns the events for the dialog and whether
    /// the key was used.
    pub fn key(
        &mut self,
        host: &mut dyn FormHost,
        layout: &FormLayout,
        key: Key,
        mods: Modifiers,
    ) -> (Vec<FormEvent>, bool) {
        let mut events = Vec::new();
        if self.focus.is_some_and(|f| layout.rect_of(f).is_none()) {
            self.focus = None;
        }
        match key {
            Key::Tab => {
                self.cycle_focus(layout, if mods.shift { -1 } else { 1 });
                self.scroll_to_focus(layout);
                if let Some(f) = self.focus {
                    if let Some(field) = host.field_mut(f) {
                        field.select_all();
                    }
                }
                return (events, true);
            }
            Key::Escape => {
                events.push(FormEvent::Cancel);
                return (events, true);
            }
            _ => {}
        }
        let focus = self.focus;
        // Text fields first: they own the editing keys.
        if let Some(f) = focus {
            if layout.is_field(f) {
                if let Some(field) = host.field_mut(f) {
                    match field.handle_key(key, mods) {
                        FieldResponse::Ignored => {}
                        FieldResponse::Handled => return (events, true),
                        FieldResponse::Changed => {
                            events.push(FormEvent::FieldChanged(f));
                            return (events, true);
                        }
                        FieldResponse::Copy { text, cut } => {
                            events.push(FormEvent::Copy(text));
                            if cut {
                                events.push(FormEvent::FieldChanged(f));
                            }
                            return (events, true);
                        }
                        FieldResponse::Paste => {
                            events.push(FormEvent::Paste);
                            return (events, true);
                        }
                    }
                }
            }
        }
        match key {
            Key::Enter => {
                match focus.and_then(|f| layout.kind_of(f)) {
                    Some(ElemKind::Button { id, .. }) | Some(ElemKind::Dropdown { id, .. }) => {
                        events.push(FormEvent::Activate(*id))
                    }
                    _ => events.push(FormEvent::Submit),
                }
                (events, true)
            }
            Key::Space => {
                match focus.and_then(|f| layout.kind_of(f)) {
                    Some(ElemKind::Button { id, .. })
                    | Some(ElemKind::Check { id, .. })
                    | Some(ElemKind::Dropdown { id, .. }) => events.push(FormEvent::Activate(*id)),
                    _ => {}
                }
                (events, true)
            }
            Key::Left | Key::Right | Key::Up | Key::Down => {
                if let Some(f) = focus {
                    let n = layout.segment_count(f);
                    if n > 0 {
                        let cur = layout.segment_selected(f).unwrap_or(0);
                        let next = if matches!(key, Key::Left | Key::Up) {
                            cur.saturating_sub(1)
                        } else {
                            (cur + 1).min(n - 1)
                        };
                        if next != cur {
                            events.push(FormEvent::Segment(f, next));
                        }
                    }
                }
                (events, true)
            }
            // Other keys are swallowed: the dialog is modal.
            _ => (events, false),
        }
    }

    /// Typed text goes to the focused field.
    pub fn text(
        &mut self,
        host: &mut dyn FormHost,
        layout: &FormLayout,
        text: &str,
    ) -> Vec<FormEvent> {
        let Some(f) = self.focus else {
            return Vec::new();
        };
        if !layout.is_field(f) {
            return Vec::new();
        }
        match host.field_mut(f) {
            Some(field) => {
                if field.insert(text) {
                    vec![FormEvent::FieldChanged(f)]
                } else {
                    Vec::new()
                }
            }
            None => Vec::new(),
        }
    }
}

/// Glyph stops of a drawn field, kept from the last frame so a click can
/// place the caret.
#[derive(Clone, Debug, Default)]
pub(crate) struct FieldGeom {
    pub id: Id,
    pub text: String,
    pub font: f32,
    /// (byte offset, x from the text start) for every character boundary.
    pub stops: Vec<(usize, f32)>,
    /// Horizontal text scroll inside the field.
    pub scroll: f32,
    /// Text area inside the field.
    pub inner: Rect,
}

impl FieldGeom {
    /// The byte offset nearest to window x.
    pub fn offset_at(&self, x: f32) -> usize {
        let local = x - self.inner.x + self.scroll;
        self.stops
            .iter()
            .min_by(|a, b| (a.1 - local).abs().total_cmp(&(b.1 - local).abs()))
            .map_or(0, |s| s.0)
    }

    fn x_of(&self, offset: usize) -> f32 {
        self.stops
            .iter()
            .find(|s| s.0 == offset)
            .map_or(0.0, |s| s.1)
    }
}

/// Everything needed to draw a form.
pub(crate) struct FormPaint<'a> {
    pub theme: &'a Theme,
    pub m: &'a Metrics,
    pub measure: &'a dyn TextMeasure,
}

/// Draw a laid-out dialog.
pub(crate) fn draw(
    out: &mut DrawList,
    p: &FormPaint<'_>,
    layout: &FormLayout,
    state: &FormState,
    host: &dyn FormHost,
    geoms: &mut Vec<FieldGeom>,
) {
    let t = p.theme;
    let m = p.m;
    let panel = layout.panel;
    paint::panel(
        out,
        panel,
        t.panel_solid,
        t.border,
        t.shadow,
        m.radius() * 1.5,
        m.hairline(),
    );
    out.clip(Some(panel));
    let font = m.text();
    for e in &layout.elems {
        if e.rect.bottom() < panel.y || e.rect.y > panel.bottom() {
            continue;
        }
        let hovered = |id: Id, index: usize| state.hover == Some((id, index));
        let pressed = |id: Id, index: usize| state.pressed == Some((id, index));
        match &e.kind {
            ElemKind::Title(text) => {
                let s = paint::elide_end(p.measure, text, m.text_title(), e.rect.w);
                paint::text_left(out, p.measure, e.rect, &s, m.text_title(), t.text);
            }
            ElemKind::Text(text, tone) => {
                let color = match tone {
                    Tone::Normal => t.text,
                    Tone::Dim => t.text_dim,
                    Tone::Error => t.error,
                };
                let s = paint::elide_middle(p.measure, text, font, e.rect.w);
                paint::text_left(out, p.measure, e.rect, &s, font, color);
            }
            ElemKind::Label(text) => {
                let s = paint::elide_end(p.measure, text, font, e.rect.w - m.px(6.0));
                paint::text_left(out, p.measure, e.rect, &s, font, t.text_dim);
            }
            ElemKind::Suffix(text) => {
                paint::text_left(out, p.measure, e.rect, text, font, t.text_dim);
            }
            ElemKind::Field { id, enabled } => {
                let focused = state.focus == Some(*id);
                out.rect(e.rect, t.field, m.radius_small());
                let border = if focused { t.accent } else { t.field_border };
                let bw = if focused { m.px(2.0) } else { m.hairline() };
                out.border(e.rect, border, bw, m.radius_small());
                let Some(field) = host.field(*id) else {
                    continue;
                };
                let inner = e.rect.inset(m.px(7.0));
                let inner = Rect::new(inner.x, e.rect.y, inner.w, e.rect.h);
                let geom = field_geom(geoms, *id, field.text(), font, p.measure, inner);
                // Keep the caret visible.
                let caret_x = geom.x_of(field.caret());
                if caret_x - geom.scroll > inner.w {
                    geom.scroll = caret_x - inner.w;
                } else if caret_x < geom.scroll {
                    geom.scroll = caret_x;
                }
                let total = geom.stops.last().map_or(0.0, |s| s.1);
                geom.scroll = geom.scroll.clamp(0.0, (total - inner.w).max(0.0));
                let text_color = if *enabled { t.text } else { t.text_disabled };
                out.clip(inner.intersect(panel));
                if focused && field.has_selection() {
                    let sel = field.selection();
                    let x0 = inner.x + geom.x_of(sel.start) - geom.scroll;
                    let x1 = inner.x + geom.x_of(sel.end) - geom.scroll;
                    let sh = (font * 1.35).round();
                    let sy = (e.rect.y + (e.rect.h - sh) * 0.5).round();
                    out.rect(
                        Rect::new(x0.round(), sy, (x1 - x0).round(), sh),
                        t.selection,
                        0.0,
                    );
                }
                let line = p.measure.measure(field.text(), font);
                let ty = (e.rect.y + (e.rect.h - line.h) * 0.5).round();
                out.text(
                    Point::new((inner.x - geom.scroll).round(), ty),
                    field.text(),
                    font,
                    text_color,
                );
                if focused {
                    let cx = (inner.x + caret_x - geom.scroll).round();
                    let ch = (font * 1.3).round();
                    let cy = (e.rect.y + (e.rect.h - ch) * 0.5).round();
                    out.rect(Rect::new(cx, cy, m.hairline().max(1.0), ch), t.text, 0.0);
                }
                out.clip(Some(panel));
            }
            ElemKind::Button {
                id,
                label,
                style,
                enabled,
            } => {
                let focused = state.focus == Some(*id);
                let (fill, text_color) = match (style, enabled) {
                    (_, false) => (t.hover, t.text_disabled),
                    (ButtonStyle::Primary, true) => (t.accent, t.accent_text),
                    (ButtonStyle::Secondary, true) => (t.hover, t.text),
                };
                out.rect(e.rect, fill, m.radius_small());
                if *enabled && pressed(*id, 0) {
                    out.rect(e.rect, t.pressed, m.radius_small());
                } else if *enabled && hovered(*id, 0) {
                    out.rect(e.rect, t.hover, m.radius_small());
                }
                if focused {
                    out.border(
                        paint::outset(e.rect, m.px(2.0)),
                        t.accent,
                        m.px(2.0),
                        m.radius_small() + m.px(2.0),
                    );
                }
                let s = paint::elide_end(p.measure, label, font, e.rect.w - m.px(8.0));
                paint::text_center(out, p.measure, e.rect, &s, font, text_color);
            }
            ElemKind::Segment {
                id,
                index,
                label,
                selected,
                enabled,
            } => {
                let r = Rect::new(e.rect.x, e.rect.y, e.rect.w - m.hairline(), e.rect.h);
                let (fill, text_color) = if !*enabled {
                    (t.hover, t.text_disabled)
                } else if *selected {
                    (t.accent, t.accent_text)
                } else {
                    (t.hover, t.text)
                };
                out.rect(r, fill, m.radius_small());
                if *enabled && !*selected && (hovered(*id, *index) || pressed(*id, *index)) {
                    out.rect(r, t.hover, m.radius_small());
                }
                if state.focus == Some(*id) && *selected {
                    out.border(
                        paint::outset(r, m.px(2.0)),
                        t.accent,
                        m.px(2.0),
                        m.radius_small() + m.px(2.0),
                    );
                }
                let s = paint::elide_end(p.measure, label, font, r.w - m.px(6.0));
                paint::text_center(out, p.measure, r, &s, font, text_color);
            }
            ElemKind::Check {
                id,
                label,
                checked,
                enabled,
            } => {
                let side = m.px(18.0);
                let bx = Rect::new(
                    e.rect.x,
                    (e.rect.y + (e.rect.h - side) * 0.5).round(),
                    side,
                    side,
                );
                let color = if *enabled { t.text } else { t.text_disabled };
                if *checked {
                    out.rect(
                        bx,
                        if *enabled { t.accent } else { t.hover },
                        m.radius_small(),
                    );
                    paint::icon(
                        out,
                        bx,
                        Icon::Check,
                        m.icon(),
                        if *enabled {
                            t.accent_text
                        } else {
                            t.text_disabled
                        },
                    );
                } else {
                    out.rect(bx, t.field, m.radius_small());
                    out.border(bx, t.field_border, m.hairline(), m.radius_small());
                }
                if state.focus == Some(*id) {
                    out.border(
                        paint::outset(bx, m.px(2.0)),
                        t.accent,
                        m.px(2.0),
                        m.radius_small() + m.px(2.0),
                    );
                }
                if *enabled && hovered(*id, 0) {
                    out.rect(bx, t.hover, m.radius_small());
                }
                let lr = Rect::new(
                    bx.right() + m.px(8.0),
                    e.rect.y,
                    (e.rect.right() - bx.right() - m.px(8.0)).max(0.0),
                    e.rect.h,
                );
                let s = paint::elide_end(p.measure, label, font, lr.w);
                paint::text_left(out, p.measure, lr, &s, font, color);
            }
            ElemKind::Dropdown { id, value, enabled } => {
                out.rect(e.rect, t.field, m.radius_small());
                let focused = state.focus == Some(*id);
                out.border(
                    e.rect,
                    if focused { t.accent } else { t.field_border },
                    if focused { m.px(2.0) } else { m.hairline() },
                    m.radius_small(),
                );
                if *enabled && hovered(*id, 0) {
                    out.rect(e.rect, t.hover, m.radius_small());
                }
                let icon_box = Rect::new(e.rect.right() - e.rect.h, e.rect.y, e.rect.h, e.rect.h);
                let color = if *enabled { t.text } else { t.text_disabled };
                paint::icon(out, icon_box, Icon::ChevronDown, m.icon(), fade(color, 0.8));
                let tr = Rect::new(
                    e.rect.x + m.px(8.0),
                    e.rect.y,
                    (icon_box.x - e.rect.x - m.px(8.0)).max(0.0),
                    e.rect.h,
                );
                let s = paint::elide_end(p.measure, value, font, tr.w);
                paint::text_left(out, p.measure, tr, &s, font, color);
            }
            ElemKind::Progress(fraction) => {
                out.rect(e.rect, t.hover, e.rect.h * 0.5);
                match fraction {
                    Some(f) => {
                        let w = (e.rect.w * f.clamp(0.0, 1.0)).round();
                        if w > 0.0 {
                            out.rect(
                                Rect::new(e.rect.x, e.rect.y, w, e.rect.h),
                                t.accent,
                                e.rect.h * 0.5,
                            );
                        }
                    }
                    None => {
                        out.rect(
                            Rect::new(e.rect.x, e.rect.y, (e.rect.w * 0.25).round(), e.rect.h),
                            t.accent,
                            e.rect.h * 0.5,
                        );
                    }
                }
            }
        }
    }
    out.clip(None);
}

/// The cached geometry for field `id`, refreshed when its text changed.
fn field_geom<'g>(
    geoms: &'g mut Vec<FieldGeom>,
    id: Id,
    text: &str,
    font: f32,
    measure: &dyn TextMeasure,
    inner: Rect,
) -> &'g mut FieldGeom {
    let pos = match geoms.iter().position(|g| g.id == id) {
        Some(p) => p,
        None => {
            geoms.push(FieldGeom {
                id,
                text: String::from("\u{0}"),
                ..FieldGeom::default()
            });
            geoms.len() - 1
        }
    };
    let g = &mut geoms[pos];
    g.inner = inner;
    if g.text != text || g.font != font {
        g.text.clear();
        g.text.push_str(text);
        g.font = font;
        g.stops.clear();
        g.stops.push((0, 0.0));
        for (i, c) in text.char_indices() {
            let end = i + c.len_utf8();
            g.stops.push((end, measure.measure(&text[..end], font).w));
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_chars_limits_lines() {
        assert_eq!(
            wrap_chars("one two three four", 9, 5),
            vec!["one two", "three", "four"]
        );
        assert_eq!(
            wrap_chars("one two three four", 9, 2),
            vec!["one two", "three four"]
        );
    }
}

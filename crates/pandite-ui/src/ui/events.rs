//! Input handling: pointer, wheel, keyboard and text.

use super::{Capture, CropButton, Dialog, Target, Ui};
use crate::action::{Action, WHEEL_ZOOM_STEP};
use crate::command::{command_for_key, Command};
use crate::confirm::{Answer, ConfirmOwner};
use crate::context::UiContext;
use crate::form::{FormEvent, FormHost, FormLayout, Outcome};
use crate::input::{InputEvent, Key, Modifiers, PointerButton, WheelDelta};
use crate::menu::{MenuOwner, MenuPick};
use crate::metrics::{self, Metrics};
use crate::resize_dialog;
use crate::text_select::{TextGeom, TextSelection};
use pandite_core::Point;

/// Wheel lines per pixel of precise scrolling, before scaling.
const PIXELS_PER_LINE: f32 = 50.0;

impl Ui {
    /// Feed one input event. Returns what the user asked for, in order.
    /// `ctx` must describe the application at the time of the event.
    pub fn handle_event(&mut self, event: InputEvent, ctx: &UiContext<'_>) -> Vec<Action> {
        self.update(ctx);
        let m = Metrics::new(ctx.scale_factor);
        let mut out = Vec::new();
        match event {
            InputEvent::PointerMove { pos } => self.on_move(pos, ctx, &m, &mut out),
            InputEvent::PointerDown { pos, button } => self.on_down(pos, button, ctx, &m, &mut out),
            InputEvent::PointerUp { pos, button } => self.on_up(pos, button, ctx, &m, &mut out),
            InputEvent::PointerLeft => {
                self.pointer = None;
                if self.capture.is_none() {
                    self.set_hover(Target::None);
                }
            }
            InputEvent::Wheel { pos, delta } => self.on_wheel(pos, delta, ctx, &m, &mut out),
            InputEvent::KeyDown { key, mods, repeat } => {
                self.mods = mods;
                self.on_key(key, mods, repeat, ctx, &m, &mut out);
            }
            InputEvent::KeyUp { mods, .. } => self.mods = mods,
            InputEvent::ModifiersChanged(mods) => self.mods = mods,
            InputEvent::Text(text) => self.on_text(&text, ctx, &m, &mut out),
            InputEvent::FocusLost => {
                self.drop_capture();
                self.mods = Modifiers::NONE;
            }
            InputEvent::Tick => {}
        }
        self.toolbar.compute(ctx.window_size.w, &m);
        self.refresh_cursor(ctx, &m);
        self.finish(ctx);
        out
    }

    /// Forget the pointer capture, ending a crop drag cleanly.
    fn drop_capture(&mut self) {
        if matches!(self.capture, Some((Capture::Crop, _))) {
            if let Some(crop) = &mut self.crop {
                crop.end();
            }
        }
        self.capture = None;
        self.dialog_form.pressed = None;
        self.confirm_form.pressed = None;
    }

    pub(super) fn set_hover(&mut self, target: Target) {
        if self.hover != target {
            self.hover = target;
            self.hover_since = self.now;
            self.tooltip_blocked = false;
        }
    }

    fn wake(&mut self) {
        self.last_activity = self.now;
    }

    fn form_layout(&self, confirm: bool, ctx: &UiContext<'_>, m: &Metrics) -> Option<FormLayout> {
        if confirm {
            self.confirm_layout(ctx, m)
        } else {
            self.dialog_layout(ctx, m)
        }
    }

    fn on_move(&mut self, pos: Point, ctx: &UiContext<'_>, m: &Metrics, out: &mut Vec<Action>) {
        self.pointer = Some(pos);
        match self.capture {
            Some((
                Capture::Pan {
                    last,
                    start,
                    panned,
                },
                button,
            )) => {
                let (dx, dy) = (pos.x - last.x, pos.y - last.y);
                if dx != 0.0 || dy != 0.0 {
                    out.push(Action::PanBy { dx, dy });
                }
                let moved = (pos.x - start.x).abs().max((pos.y - start.y).abs());
                let panned = panned || moved > m.drag_threshold();
                self.capture = Some((
                    Capture::Pan {
                        last: pos,
                        start,
                        panned,
                    },
                    button,
                ));
                return;
            }
            Some((
                Capture::Text {
                    id,
                    anchor,
                    start,
                    dragging,
                },
                button,
            )) => {
                let moved = (pos.x - start.x).abs().max((pos.y - start.y).abs());
                let dragging = dragging || moved > m.drag_threshold();
                if dragging {
                    if let Some(focus) = TextGeom::new(ctx).and_then(|g| g.nearest(pos)) {
                        self.text_sel = Some(TextSelection { id, anchor, focus });
                    }
                }
                self.capture = Some((
                    Capture::Text {
                        id,
                        anchor,
                        start,
                        dragging,
                    },
                    button,
                ));
                return;
            }
            Some((Capture::Crop, _)) => {
                if let (Some(crop), Some(map)) = (&mut self.crop, Self::crop_mapping(ctx)) {
                    crop.drag_to(pos, &map, self.mods.shift);
                }
                return;
            }
            Some((
                Capture::Gallery {
                    index,
                    start,
                    start_scroll,
                    dragging,
                },
                button,
            )) => {
                let moved = (pos.x - start.x).abs().max((pos.y - start.y).abs());
                let dragging = dragging || moved > m.drag_threshold();
                if dragging {
                    if let Some(layout) = self.gallery_layout(ctx, m) {
                        let s = layout.clamp_scroll(start_scroll - (pos.x - start.x));
                        self.gallery.scroll = s;
                        self.gallery.target = s;
                    }
                }
                self.capture = Some((
                    Capture::Gallery {
                        index,
                        start,
                        start_scroll,
                        dragging,
                    },
                    button,
                ));
                return;
            }
            Some((
                Capture::Form {
                    confirm: false,
                    id,
                    field: true,
                    ..
                },
                _,
            )) => {
                if let (Some(dialog), Some(geom)) = (
                    &mut self.dialog,
                    self.field_geoms.iter().find(|g| g.id == id),
                ) {
                    let offset = geom.offset_at(pos.x);
                    if let Some(field) = dialog.host_mut().field_mut(id) {
                        field.set_caret(offset, true);
                    }
                }
                return;
            }
            _ => {}
        }
        self.wake();
        let target = self.hit(pos, ctx, m);
        // Button presses keep tracking hover, so the pressed look follows
        // the pointer and a release elsewhere cancels.
        self.set_hover(target);
        if let Some(menu) = &mut self.menu {
            if let Target::MenuItem(i) = target {
                if menu.items[i].selectable() {
                    menu.hover = Some(i);
                }
            }
        }
        self.dialog_form.hover = match target {
            Target::Form {
                confirm: false,
                id,
                index,
            } => Some((id, index)),
            _ => None,
        };
        self.confirm_form.hover = match target {
            Target::Form {
                confirm: true,
                id,
                index,
            } => Some((id, index)),
            _ => None,
        };
        self.gallery.hover = match target {
            Target::Gallery(i) => i,
            _ => None,
        };
    }

    fn on_down(
        &mut self,
        pos: Point,
        button: PointerButton,
        ctx: &UiContext<'_>,
        m: &Metrics,
        out: &mut Vec<Action>,
    ) {
        let modal = self.modal();
        if matches!(button, PointerButton::Back | PointerButton::Forward) {
            // Side buttons act on press and never hold the pointer.
            self.pointer = Some(pos);
            if !modal && self.crop.is_none() && self.capture.is_none() {
                let cmd = if button == PointerButton::Back {
                    Command::Previous
                } else {
                    Command::Next
                };
                self.run_command(cmd, ctx, None, out);
            }
            return;
        }
        if let Some((_, held)) = self.capture {
            if held != button {
                // A second button during a drag is ignored.
                self.pointer = Some(pos);
                return;
            }
            // The release of the earlier press never arrived; drop it.
            self.drop_capture();
        }
        // Hit test before the pointer and the idle timer wake the toolbar,
        // so a press where the hidden toolbar would be does not trigger an
        // unseen button.
        let target = self.hit(pos, ctx, m);
        self.pointer = Some(pos);
        self.wake();
        self.set_hover(target);
        self.tooltip_blocked = true;
        if self.menu.is_some() {
            let capture = match target {
                Target::MenuItem(i) => Capture::Menu(Some(i)),
                Target::MenuPanel => Capture::Menu(None),
                _ => {
                    self.menu = None;
                    Capture::Swallow
                }
            };
            self.capture = Some((capture, button));
            return;
        }
        if self.help {
            self.help = false;
            self.capture = Some((Capture::Swallow, button));
            return;
        }
        let capture = match target {
            Target::Form { confirm, id, index } if button == PointerButton::Left => {
                self.form_press(confirm, id, index, pos, ctx, m)
            }
            Target::Toolbar(cmd) if button == PointerButton::Left => Capture::Toolbar(cmd),
            Target::More if button == PointerButton::Left => Capture::More,
            Target::CropBar(b) if button == PointerButton::Left => Capture::CropBar(b),
            Target::Gallery(index) => match button {
                PointerButton::Left => Capture::Gallery {
                    index,
                    start: pos,
                    start_scroll: self.gallery.scroll,
                    dragging: false,
                },
                PointerButton::Right => Capture::RightPress,
                _ => Capture::Swallow,
            },
            Target::EmptyOpen if button == PointerButton::Left => Capture::EmptyOpen,
            Target::Content => match button {
                PointerButton::Left if self.crop.is_some() => {
                    match (&mut self.crop, Self::crop_mapping(ctx)) {
                        (Some(crop), Some(map)) => {
                            crop.begin(pos, &map, m.handle_hit());
                            Capture::Crop
                        }
                        _ => Capture::Swallow,
                    }
                }
                PointerButton::Left => match self.text_press(pos, ctx, m) {
                    Some(capture) => capture,
                    None => {
                        self.text_clicks = None;
                        let double = self.last_press.is_some_and(|(t, p)| {
                            self.now.saturating_sub(t) <= metrics::DOUBLE_CLICK
                                && (p.x - pos.x).abs() <= m.px(6.0)
                                && (p.y - pos.y).abs() <= m.px(6.0)
                        });
                        if double {
                            self.last_press = None;
                            self.toggle_fit(ctx, pos, out);
                        } else {
                            self.last_press = Some((self.now, pos));
                        }
                        Capture::Pan {
                            last: pos,
                            start: pos,
                            panned: false,
                        }
                    }
                },
                PointerButton::Middle => Capture::Pan {
                    last: pos,
                    start: pos,
                    panned: false,
                },
                PointerButton::Right => Capture::RightPress,
                _ => Capture::Swallow,
            },
            Target::ToolbarBar | Target::CropBarPanel | Target::InfoPanel
                if button == PointerButton::Right =>
            {
                Capture::RightPress
            }
            _ => Capture::Swallow,
        };
        self.capture = Some((capture, button));
    }

    /// A left press on the image: on a recognized word it starts a text
    /// selection (one click), selects the word (two) or its line (three).
    /// Returns `None` when the press is off text.
    fn text_press(&mut self, pos: Point, ctx: &UiContext<'_>, m: &Metrics) -> Option<Capture> {
        let word = self.word_at(pos, ctx, m)?;
        let geom = TextGeom::new(ctx)?;
        let count = match self.text_clicks {
            Some((t, p, n))
                if self.now.saturating_sub(t) <= metrics::DOUBLE_CLICK
                    && (p.x - pos.x).abs() <= m.px(6.0)
                    && (p.y - pos.y).abs() <= m.px(6.0) =>
            {
                n + 1
            }
            _ => 1,
        };
        self.text_clicks = Some((self.now, pos, count));
        // A press on text is never the first half of a fit toggle.
        self.last_press = None;
        let select = |anchor: usize, focus: usize| TextSelection {
            id: geom.id,
            anchor,
            focus,
        };
        Some(match count {
            1 => Capture::Text {
                id: geom.id,
                anchor: word,
                start: pos,
                dragging: false,
            },
            2 => {
                self.text_sel = Some(select(word, word));
                Capture::Swallow
            }
            _ => {
                let (first, last) = geom.line_bounds(word)?;
                self.text_sel = Some(select(first, last));
                Capture::Swallow
            }
        })
    }

    /// A left press on a dialog element: focus it and start a press or a
    /// text selection.
    fn form_press(
        &mut self,
        confirm: bool,
        id: crate::form::Id,
        index: usize,
        pos: Point,
        ctx: &UiContext<'_>,
        m: &Metrics,
    ) -> Capture {
        let is_field = self
            .form_layout(confirm, ctx, m)
            .is_some_and(|l| l.is_field(id));
        let shift = self.mods.shift;
        let form = if confirm {
            &mut self.confirm_form
        } else {
            &mut self.dialog_form
        };
        form.focus = Some(id);
        form.pressed = Some((id, index));
        if is_field && !confirm {
            if let (Some(dialog), Some(geom)) = (
                &mut self.dialog,
                self.field_geoms.iter().find(|g| g.id == id),
            ) {
                let offset = geom.offset_at(pos.x);
                if let Some(field) = dialog.host_mut().field_mut(id) {
                    field.set_caret(offset, shift);
                }
            }
        }
        Capture::Form {
            confirm,
            id,
            index,
            field: is_field,
        }
    }

    fn on_up(
        &mut self,
        pos: Point,
        button: PointerButton,
        ctx: &UiContext<'_>,
        m: &Metrics,
        out: &mut Vec<Action>,
    ) {
        self.pointer = Some(pos);
        let Some((capture, pressed_with)) = self.capture else {
            return;
        };
        if pressed_with != button {
            return;
        }
        self.capture = None;
        self.dialog_form.pressed = None;
        self.confirm_form.pressed = None;
        let target = self.hit(pos, ctx, m);
        self.set_hover(target);
        match capture {
            Capture::Toolbar(cmd) => {
                if target == Target::Toolbar(cmd) {
                    self.run_command(cmd, ctx, None, out);
                }
            }
            Capture::More => {
                if target == Target::More {
                    self.open_overflow_menu(ctx);
                }
            }
            Capture::CropBar(b) => {
                if target == Target::CropBar(b) {
                    let cmd = match b {
                        CropButton::Apply => Command::CropApply,
                        CropButton::Cancel => Command::CropCancel,
                    };
                    self.run_command(cmd, ctx, None, out);
                }
            }
            Capture::Menu(Some(i)) => {
                if target == Target::MenuItem(i) {
                    self.activate_menu(i, ctx, out);
                }
            }
            Capture::Menu(None) => {}
            Capture::Form {
                confirm,
                id,
                index,
                field,
            } => {
                if !field && target == (Target::Form { confirm, id, index }) {
                    let segmented = self
                        .form_layout(confirm, ctx, m)
                        .is_some_and(|l| l.segment_count(id) > 0);
                    let event = if segmented {
                        FormEvent::Segment(id, index)
                    } else {
                        FormEvent::Activate(id)
                    };
                    if confirm {
                        self.confirm_event(&event, ctx, out);
                    } else {
                        self.dialog_event(event, ctx, out);
                        // A clicked resize mode is ready for typing.
                        if segmented && id == resize_dialog::SEG_MODE {
                            if let Some(Dialog::Resize(d)) = &mut self.dialog {
                                let f = d.first_field();
                                if let Some(field) = d.field_mut(f) {
                                    field.select_all();
                                }
                                self.dialog_form.focus = Some(f);
                            }
                        }
                    }
                }
            }
            Capture::Gallery {
                index: Some(i),
                dragging: false,
                ..
            } => {
                if target == Target::Gallery(Some(i)) && self.crop.is_none() && i < ctx.count {
                    out.push(Action::GoTo(i));
                }
            }
            Capture::Gallery { .. } => {}
            Capture::RightPress => {
                if matches!(
                    target,
                    Target::Content
                        | Target::Gallery(_)
                        | Target::ToolbarBar
                        | Target::CropBarPanel
                        | Target::InfoPanel
                ) {
                    self.open_context_menu(pos, ctx);
                }
            }
            Capture::EmptyOpen => {
                if target == Target::EmptyOpen {
                    out.push(Action::Open);
                }
            }
            Capture::Crop => {
                if let Some(crop) = &mut self.crop {
                    crop.end();
                }
            }
            Capture::Text { dragging, .. } => {
                // A click on text without a drag places no selection, like
                // a click in a document.
                if !dragging {
                    self.text_sel = None;
                }
            }
            Capture::Pan { start, panned, .. } => {
                // A left click off text (not a pan) drops the selection.
                let moved = (pos.x - start.x).abs().max((pos.y - start.y).abs());
                if button == PointerButton::Left && !panned && moved <= m.drag_threshold() {
                    self.text_sel = None;
                }
            }
            Capture::Swallow => {}
        }
    }

    fn on_wheel(
        &mut self,
        pos: Point,
        delta: WheelDelta,
        ctx: &UiContext<'_>,
        m: &Metrics,
        out: &mut Vec<Action>,
    ) {
        self.pointer = Some(pos);
        let (lines, pixels) = match delta {
            WheelDelta::Lines { x, y } => ((x, y), None),
            WheelDelta::Pixels { x, y } => {
                let per_line = PIXELS_PER_LINE * m.scale;
                ((x / per_line, y / per_line), Some((x, y)))
            }
        };
        let target = self.hit(pos, ctx, m);
        let win = ctx.window_rect();
        if let Some(menu) = &mut self.menu {
            if matches!(target, Target::MenuItem(_) | Target::MenuPanel) {
                let amount = pixels.map_or(lines.1 * m.menu_item_h() * 2.0, |p| p.1);
                menu.scroll = (menu.scroll - amount).clamp(0.0, menu.max_scroll(win, m));
            }
            return;
        }
        if self.help {
            return;
        }
        if self.confirm.is_some() || self.dialog.is_some() {
            let confirm = self.confirm.is_some();
            if let Some(layout) = self.form_layout(confirm, ctx, m) {
                if layout.panel.contains(pos) {
                    let amount = pixels.map_or(lines.1 * m.row_h(), |p| p.1);
                    let form = if confirm {
                        &mut self.confirm_form
                    } else {
                        &mut self.dialog_form
                    };
                    form.scroll = (form.scroll - amount).clamp(0.0, layout.max_scroll());
                }
            }
            return;
        }
        match target {
            Target::Gallery(_) => {
                if let Some(layout) = self.gallery_layout(ctx, m) {
                    // Both axes scroll the strip; positive values reveal
                    // earlier items.
                    let amount = match pixels {
                        Some((x, y)) => x + y,
                        None => (lines.0 + lines.1) * layout.pitch * 1.5,
                    };
                    self.gallery.target = layout.clamp_scroll(self.gallery.target - amount);
                }
            }
            Target::Content if ctx.image.is_some() && lines.1 != 0.0 => {
                let factor = WHEEL_ZOOM_STEP.powf(lines.1);
                if factor.is_finite() && factor > 0.0 {
                    out.push(Action::ZoomAt {
                        factor,
                        anchor: pos,
                    });
                }
            }
            _ => {}
        }
    }

    fn on_key(
        &mut self,
        key: Key,
        mods: Modifiers,
        repeat: bool,
        ctx: &UiContext<'_>,
        m: &Metrics,
        out: &mut Vec<Action>,
    ) {
        // Escape closes the topmost menu, overlay or dialog below. With none
        // open it reaches the shortcut table and quits; a held key never
        // does, so a long press that closes a dialog stops there.
        if self.menu.is_some() {
            self.menu_key(key, ctx, m, out);
            return;
        }
        if self.help {
            if matches!(key, Key::Escape | Key::F1 | Key::Enter) {
                self.help = false;
            }
            return;
        }
        if self.confirm.is_some() || self.dialog.is_some() {
            let confirm = self.confirm.is_some();
            // Window commands stay available in dialogs.
            if let Some(cmd @ (Command::Fullscreen | Command::Quit)) =
                command_for_key(key, mods, false)
            {
                if !repeat {
                    self.run_command(cmd, ctx, None, out);
                }
                return;
            }
            let Some(layout) = self.form_layout(confirm, ctx, m) else {
                return;
            };
            let (events, _) = if confirm {
                let Some(dialog) = self.confirm.as_mut() else {
                    return;
                };
                if matches!(key, Key::Left | Key::Right) {
                    // Arrow keys move between the two buttons.
                    self.confirm_form
                        .cycle_focus(&layout, if key == Key::Left { -1 } else { 1 });
                    return;
                }
                self.confirm_form.key(dialog, &layout, key, mods)
            } else {
                let Some(dialog) = self.dialog.as_mut() else {
                    return;
                };
                self.dialog_form.key(dialog.host_mut(), &layout, key, mods)
            };
            for event in events {
                if confirm {
                    self.confirm_event(&event, ctx, out);
                } else {
                    self.dialog_event(event, ctx, out);
                }
            }
            return;
        }
        if self.crop.is_some() && !mods.ctrl && !mods.alt {
            let step = if mods.shift { 10.0 } else { 1.0 };
            let nudge = match key {
                Key::Left => Some((-step, 0.0)),
                Key::Right => Some((step, 0.0)),
                Key::Up => Some((0.0, -step)),
                Key::Down => Some((0.0, step)),
                _ => None,
            };
            if let Some((dx, dy)) = nudge {
                if let Some(crop) = &mut self.crop {
                    if crop.selection.is_some() {
                        crop.nudge(dx, dy);
                        return;
                    }
                }
            }
        }
        let Some(mut cmd) = command_for_key(key, mods, self.crop.is_some()) else {
            return;
        };
        // Ctrl+C copies the selected text while there is a selection, and
        // the image otherwise.
        if cmd == Command::CopyImage && self.command_enabled(Command::CopyText, ctx) {
            cmd = Command::CopyText;
        }
        if repeat && !cmd.repeats() {
            return;
        }
        self.run_command(cmd, ctx, None, out);
    }

    fn menu_key(&mut self, key: Key, ctx: &UiContext<'_>, m: &Metrics, out: &mut Vec<Action>) {
        let win = ctx.window_rect();
        let Some(menu) = &mut self.menu else { return };
        match key {
            Key::Up => menu.step(-1),
            Key::Down | Key::Tab => menu.step(1),
            Key::Home | Key::PageUp => menu.jump(true),
            Key::End | Key::PageDown => menu.jump(false),
            Key::Enter | Key::Space => {
                if let Some(i) = menu.hover {
                    self.activate_menu(i, ctx, out);
                }
                return;
            }
            _ => {
                // Escape and every other key close the menu.
                self.menu = None;
                return;
            }
        }
        menu.scroll_to_hover(win, m);
    }

    fn activate_menu(&mut self, i: usize, ctx: &UiContext<'_>, out: &mut Vec<Action>) {
        let Some(menu) = self.menu.take() else { return };
        let Some(pick) = menu.pick(i) else {
            // Disabled entries keep the menu open.
            self.menu = Some(menu);
            return;
        };
        match (menu.owner, pick) {
            (MenuOwner::Context { anchor }, MenuPick::Command(cmd)) => {
                let origin = match cmd {
                    Command::ZoomIn | Command::ZoomOut | Command::ActualSize => anchor,
                    _ => None,
                };
                self.run_command(cmd, ctx, origin, out);
            }
            (MenuOwner::Overflow, MenuPick::Command(cmd)) => self.run_command(cmd, ctx, None, out),
            (MenuOwner::FormatList, MenuPick::Choice(n)) => {
                if let Some(Dialog::Bulk(b)) = &mut self.dialog {
                    b.choose_format(n, ctx);
                }
                self.dialog_form.focus = Some(crate::bulk_dialog::DD_FORMAT);
            }
            _ => {}
        }
    }

    fn on_text(&mut self, text: &str, ctx: &UiContext<'_>, m: &Metrics, out: &mut Vec<Action>) {
        if self.menu.is_some() || self.help || self.confirm.is_some() {
            return;
        }
        let Some(layout) = self.dialog_layout(ctx, m) else {
            return;
        };
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let events = self.dialog_form.text(dialog.host_mut(), &layout, text);
        for event in events {
            self.dialog_event(event, ctx, out);
        }
    }

    /// Hand a form event to the open dialog and apply the outcome.
    pub(super) fn dialog_event(
        &mut self,
        event: FormEvent,
        ctx: &UiContext<'_>,
        out: &mut Vec<Action>,
    ) {
        let m = Metrics::new(ctx.scale_factor);
        let outcome = match &mut self.dialog {
            Some(Dialog::Resize(d)) => d.on_event(&event, resize_dialog::image_size(ctx)),
            Some(Dialog::Bulk(d)) => d.on_event(&event, ctx),
            None => return,
        };
        match outcome {
            Outcome::Stay => {}
            Outcome::Close(actions) => {
                self.close_dialog();
                out.extend(actions);
            }
            Outcome::Emit(actions) => out.extend(actions),
            Outcome::ConfirmOverwrite => {
                let request = crate::confirm::ConfirmRequest {
                    id: 0,
                    title: "Replace existing files?".into(),
                    message: "Files in the destination folder with the same name will be \
                              replaced. This cannot be undone."
                        .into(),
                    confirm_label: "Replace".into(),
                    cancel_label: "Cancel".into(),
                };
                self.open_confirm(ConfirmOwner::BulkOverwrite, request);
            }
            Outcome::OpenFormatMenu => self.open_format_menu(ctx, &m),
        }
        if let Some(layout) = self.dialog_layout(ctx, &m) {
            if self
                .dialog_form
                .focus
                .is_some_and(|f| layout.rect_of(f).is_none())
            {
                self.dialog_form.focus = None;
            }
            self.dialog_form.scroll = self.dialog_form.scroll.clamp(0.0, layout.max_scroll());
        }
    }

    /// Hand a form event to the confirm dialog and answer it.
    pub(super) fn confirm_event(
        &mut self,
        event: &FormEvent,
        ctx: &UiContext<'_>,
        out: &mut Vec<Action>,
    ) {
        let Some(answer) = self.confirm.as_ref().and_then(|c| c.on_event(event)) else {
            return;
        };
        let Some(dialog) = self.confirm.take() else {
            return;
        };
        self.confirm_form = Default::default();
        match dialog.owner {
            ConfirmOwner::App(id) => out.push(Action::ConfirmResult {
                id,
                accepted: answer == Answer::Yes,
            }),
            ConfirmOwner::BulkOverwrite => {
                if answer == Answer::Yes {
                    if let Some(Dialog::Bulk(b)) = &mut self.dialog {
                        if let Some(action) = b.start(ctx) {
                            out.push(action);
                        }
                    }
                }
            }
        }
    }
}

//! Frame building: turns the interface state into draw commands.

use super::{Capture, CropButton, Target, Ui};
use crate::command::{Command, SHORTCUTS};
use crate::context::{Loading, ThumbState, UiContext};
use crate::crop::Handle;
use crate::form::{self, FormPaint};
use crate::icon::Icon;
use crate::menu::MenuItem;
use crate::metrics::{self, Metrics};
use crate::paint;
use crate::text_select::{as_rect, TextGeom};
use crate::theme::{fade, Theme};
use crate::thumbnail_texture_id;
use pandite_core::{Color, DrawCmd, DrawList, Point, Rect, TextMeasure};

impl Ui {
    /// Append this frame's interface to `out`. Draw it on top of the image.
    /// Call [`Ui::sync_textures`] first so icon textures exist.
    pub fn build(&mut self, ctx: &UiContext<'_>, measure: &dyn TextMeasure, out: &mut DrawList) {
        self.update(ctx);
        let m = Metrics::new(ctx.scale_factor);
        let theme = Theme::for_kind(ctx.theme);
        let content = self.content_rect(ctx);
        let loading = ctx.loading.filter(|l| l.elapsed >= metrics::LOADING_DELAY);
        let splash = loading.filter(|l| !l.over_image);

        self.draw_text_selection(ctx, theme, content, out);
        if self.crop.is_some() {
            self.draw_crop(ctx, measure, &m, theme, out);
        }
        if let Some(l) = splash {
            self.draw_splash(ctx, &l, measure, &m, theme, content, out);
        } else if ctx.image.is_none() && ctx.count == 0 && ctx.loading.is_none() {
            self.draw_empty(ctx, measure, &m, theme, content, out);
        }
        self.draw_gallery(ctx, &m, theme, out);
        if self.crop.is_some() {
            self.draw_crop_bar(ctx, measure, &m, theme, out);
        } else {
            self.draw_toolbar(ctx, measure, &m, theme, out);
        }
        if let Some(l) = loading.filter(|l| l.over_image) {
            self.draw_loading_pill(ctx, &l, measure, &m, theme, out);
        }
        self.draw_info(ctx, measure, &m, theme, content, out);
        self.draw_bottom(ctx, measure, &m, theme, content, out);
        self.draw_dialogs(ctx, measure, &m, theme, out);
        if self.help {
            self.draw_help(ctx, measure, &m, theme, out);
        }
        self.draw_menu(ctx, measure, &m, theme, out);
        self.draw_tooltip(ctx, measure, &m, theme, out);
        self.refresh_cursor(ctx, &m);
        self.finish(ctx);
    }

    /// Translucent bands over the selected words, one per line, placed
    /// exactly where the image draws them and clipped to the image area.
    fn draw_text_selection(
        &self,
        ctx: &UiContext<'_>,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        let (Some(sel), Some(geom)) = (self.selection(ctx), TextGeom::new(ctx)) else {
            return;
        };
        let (first, last) = sel.bounds();
        let bands = geom.bands(first, last);
        let Some(area) = content.intersect(geom.image_rect) else {
            return;
        };
        if bands.is_empty() {
            return;
        }
        out.clip(Some(area));
        for band in bands {
            match as_rect(&band) {
                Some(r) => out.rect(r, t.selection, 0.0),
                None => out.quad(band.points, t.selection),
            }
        }
        out.clip(None);
    }

    fn draw_toolbar(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let alpha = self.toolbar_alpha;
        if alpha <= 0.0 {
            return;
        }
        let bar = self.toolbar.bar;
        out.rect(bar, fade(t.panel, alpha), 0.0);
        out.rect(
            Rect::new(bar.x, bar.bottom() - m.hairline(), bar.w, m.hairline()),
            fade(t.border, alpha),
            0.0,
        );
        let sep_h = (bar.h * 0.5).round();
        for x in &self.toolbar.separators {
            out.rect(
                Rect::new(*x, ((bar.h - sep_h) * 0.5).round(), m.hairline(), sep_h),
                fade(t.separator, alpha),
                0.0,
            );
        }
        let pressed_cmd = match self.capture {
            Some((Capture::Toolbar(cmd), _)) => Some(cmd),
            _ => None,
        };
        for (cmd, r) in &self.toolbar.buttons {
            let enabled = self.command_enabled(*cmd, ctx);
            let toggled = match cmd {
                Command::ToggleGallery => ctx.gallery_visible && ctx.count > 0,
                _ => false,
            };
            if toggled {
                out.rect(*r, fade(t.hover, alpha), m.radius_small());
            }
            if enabled && pressed_cmd == Some(*cmd) && self.hover == Target::Toolbar(*cmd) {
                out.rect(*r, fade(t.pressed, alpha), m.radius_small());
            } else if enabled && self.hover == Target::Toolbar(*cmd) {
                out.rect(*r, fade(t.hover, alpha), m.radius_small());
            }
            let icon = match cmd {
                Command::Fullscreen if ctx.fullscreen => Icon::ExitFullscreen,
                _ => cmd.icon().unwrap_or(Icon::More),
            };
            let tint = if enabled { t.icon } else { t.text_disabled };
            paint::icon(out, *r, icon, m.icon(), fade(tint, alpha));
        }
        if let Some(more) = self.toolbar.more {
            let open = matches!(
                &self.menu,
                Some(menu) if menu.owner == crate::menu::MenuOwner::Overflow
            );
            if self.hover == Target::More || open {
                out.rect(more, fade(t.hover, alpha), m.radius_small());
            }
            paint::icon(out, more, Icon::More, m.icon(), fade(t.icon, alpha));
        }
        self.draw_status(ctx, measure, m, t, alpha, out);
    }

    fn draw_status(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        alpha: f32,
        out: &mut DrawList,
    ) {
        let area = self.toolbar.status;
        if area.w < m.px(40.0) {
            return;
        }
        let font = m.text();
        let gap = m.px(14.0);
        let position = (ctx.count > 0)
            .then(|| format!("{} / {}", ctx.index.min(ctx.count - 1) + 1, ctx.count));
        let Some(info) = ctx.image else {
            if let Some(pos) = position {
                paint::text_right(out, measure, area, &pos, font, fade(t.text_dim, alpha));
            }
            return;
        };
        let zoom = if info.zoom_percent < 10.0 {
            format!("{:.1}%", info.zoom_percent)
        } else {
            format!("{:.0}%", info.zoom_percent)
        };
        let dims = format!("{} x {}", info.width, info.height);
        // Parts from right to left; the file name takes what is left.
        let mut parts: Vec<(String, Color)> = Vec::with_capacity(4);
        if let Some(p) = position {
            parts.push((p, t.text_dim));
        }
        parts.push((zoom, t.text_dim));
        parts.push((dims, t.text_dim));
        let mut right = area.right();
        let mut drawn = Vec::with_capacity(4);
        for (text, color) in parts {
            let w = measure.measure(&text, font).w;
            if right - w < area.x {
                break;
            }
            drawn.push((Rect::new(right - w, area.y, w, area.h), text, color));
            right -= w + gap;
        }
        for (r, text, color) in &drawn {
            paint::text_left(out, measure, *r, text, font, fade(*color, alpha));
        }
        let name_w = right - area.x;
        if name_w > m.px(30.0) {
            let name = if info.edited {
                format!("{} (edited)", info.file_name)
            } else {
                info.file_name.to_string()
            };
            let shown = paint::elide_middle(measure, &name, font, name_w);
            let w = measure.measure(&shown, font).w;
            let r = Rect::new(right - w, area.y, w, area.h);
            paint::text_left(out, measure, r, &shown, font, fade(t.text, alpha));
        }
    }

    fn draw_gallery(&self, ctx: &UiContext<'_>, m: &Metrics, t: &Theme, out: &mut DrawList) {
        let Some(layout) = self.gallery_layout(ctx, m) else {
            return;
        };
        let panel = layout.panel;
        out.rect(panel, t.panel_solid, 0.0);
        out.rect(
            Rect::new(panel.x, panel.y, panel.w, m.hairline()),
            t.border,
            0.0,
        );
        out.clip(Some(panel));
        let scroll = self.gallery.scroll;
        let radius = m.radius_small();
        for i in layout.visible_range(scroll) {
            let cell = layout.item_rect(i, scroll);
            let current = i == ctx.index;
            out.rect(cell, t.placeholder, radius);
            match ctx.thumbnail(i) {
                ThumbState::Ready { width, height } if width > 0 && height > 0 => {
                    // Cover the square cell, cropping the longer side.
                    let (w, h) = (width as f32, height as f32);
                    let uv = if w > h {
                        let u = h / w;
                        Rect::new((1.0 - u) * 0.5, 0.0, u, 1.0)
                    } else {
                        let v = w / h;
                        Rect::new(0.0, (1.0 - v) * 0.5, 1.0, v)
                    };
                    out.cmds.push(DrawCmd::Image {
                        rect: cell,
                        texture: thumbnail_texture_id(i),
                        uv,
                        tint: Color::WHITE,
                    });
                }
                ThumbState::NotDownloaded => {
                    paint::icon(out, cell, Icon::Cloud, m.icon(), t.text_dim);
                }
                ThumbState::Failed => {
                    paint::icon(out, cell, Icon::Warning, m.icon(), t.text_dim);
                }
                _ => {
                    paint::icon(out, cell, Icon::Picture, m.icon(), fade(t.text_dim, 0.6));
                }
            }
            if self.gallery.hover == Some(i) && !current {
                out.rect(cell, t.hover, radius);
            }
            if current {
                let ring = paint::outset(cell, m.px(2.0));
                out.border(ring, t.accent, m.px(2.0), radius + m.px(2.0));
            }
        }
        out.clip(None);
    }

    fn draw_crop(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let (Some(crop), Some(map)) = (&self.crop, Self::crop_mapping(ctx)) else {
            return;
        };
        let content = self.content_rect(ctx);
        let Some(sel) = crop.selection else {
            return;
        };
        let s = paint::snap(map.to_screen(sel));
        // Dim the four bands around the selection.
        let bands = [
            Rect::new(content.x, content.y, content.w, s.y - content.y),
            Rect::new(
                content.x,
                s.bottom(),
                content.w,
                content.bottom() - s.bottom(),
            ),
            Rect::new(content.x, s.y, s.x - content.x, s.h),
            Rect::new(s.right(), s.y, content.right() - s.right(), s.h),
        ];
        for b in bands {
            if let Some(r) = b.intersect(content) {
                out.rect(r, t.crop_dim, 0.0);
            }
        }
        // Rule of thirds.
        let line = m.hairline();
        for k in 1..3 {
            let fx = (s.x + s.w * k as f32 / 3.0).round();
            let fy = (s.y + s.h * k as f32 / 3.0).round();
            out.rect(Rect::new(fx, s.y, line, s.h), fade(t.crop_line, 0.35), 0.0);
            out.rect(Rect::new(s.x, fy, s.w, line), fade(t.crop_line, 0.35), 0.0);
        }
        out.border(paint::outset(s, line), t.crop_shadow, line * 3.0, 0.0);
        out.border(s, t.crop_line, line, 0.0);
        let hs = m.handle();
        for h in Handle::ALL {
            let c = h.center(s);
            let r = paint::snap(Rect::new(c.x - hs * 0.5, c.y - hs * 0.5, hs, hs));
            out.rect(paint::outset(r, line), t.crop_shadow, 0.0);
            out.rect(r, t.crop_line, 0.0);
        }
        if let Some(px) = crop.pixel_rect() {
            let text = format!("{} x {}", px.width, px.height);
            let font = m.text_small();
            let tw = measure.measure(&text, font).w + m.px(14.0);
            let th = m.px(22.0);
            let mut y = s.bottom() + m.px(8.0);
            if y + th > content.bottom() {
                y = (s.bottom() - th - m.px(8.0)).max(content.y);
            }
            let r = paint::keep_inside(
                Rect::new((s.x + (s.w - tw) * 0.5).round(), y.round(), tw.round(), th),
                content,
            );
            out.rect(r, t.tooltip, th * 0.5);
            paint::text_center(out, measure, r, &text, font, t.tooltip_text);
        }
    }

    fn draw_crop_bar(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let Some(crop) = &self.crop else { return };
        let (bar, apply, cancel) = self.crop_bar(ctx, m);
        paint::panel(
            out,
            bar,
            t.panel,
            t.border,
            t.shadow,
            m.radius(),
            m.hairline(),
        );
        let font = m.text();
        let label = match crop.pixel_rect() {
            Some(px) => format!("Crop to {} x {}", px.width, px.height),
            None => "Drag over the image to select".to_string(),
        };
        let lr = Rect::new(
            bar.x + m.px(12.0),
            bar.y,
            (apply.x - bar.x - m.px(16.0)).max(0.0),
            bar.h,
        );
        let shown = paint::elide_end(measure, &label, font, lr.w);
        paint::text_left(out, measure, lr, &shown, font, t.text);
        let can_apply = crop.pixel_rect().is_some();
        for (r, b, text, icon, primary) in [
            (apply, CropButton::Apply, "Apply", Icon::Check, true),
            (cancel, CropButton::Cancel, "Cancel", Icon::Close, false),
        ] {
            let enabled = b == CropButton::Cancel || can_apply;
            let (fill, fg) = match (primary, enabled) {
                (_, false) => (t.hover, t.text_disabled),
                (true, true) => (t.accent, t.accent_text),
                (false, true) => (t.hover, t.text),
            };
            out.rect(r, fill, m.radius_small());
            if enabled && self.hover == Target::CropBar(b) {
                out.rect(r, t.hover, m.radius_small());
            }
            let ir = Rect::new(r.x + m.px(4.0), r.y, m.icon(), r.h);
            paint::icon(out, ir, icon, m.icon(), fg);
            let tr = Rect::new(
                ir.right() + m.px(4.0),
                r.y,
                (r.right() - ir.right() - m.px(6.0)).max(0.0),
                r.h,
            );
            let shown = paint::elide_end(measure, text, font, tr.w);
            paint::text_left(out, measure, tr, &shown, font, fg);
        }
    }

    fn draw_empty(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        let Some(button) = self.empty_open_button(ctx, m) else {
            return;
        };
        let title_r = Rect::new(content.x, button.y - m.px(64.0), content.w, m.px(28.0));
        paint::text_center(
            out,
            measure,
            title_r,
            "No image open",
            m.text_title(),
            t.text,
        );
        let icon_r = Rect::new(content.x, title_r.y - m.px(30.0), content.w, m.icon());
        paint::icon(out, icon_r, Icon::Picture, m.icon(), t.text_dim);
        out.rect(button, t.accent, m.radius_small());
        if self.hover == Target::EmptyOpen {
            out.rect(button, t.hover, m.radius_small());
        }
        let ir = Rect::new(button.x + m.px(10.0), button.y, m.icon(), button.h);
        paint::icon(out, ir, Icon::Open, m.icon(), t.accent_text);
        let tr = Rect::new(
            ir.right() + m.px(8.0),
            button.y,
            button.right() - ir.right() - m.px(10.0),
            button.h,
        );
        paint::text_left(out, measure, tr, "Open image...", m.text(), t.accent_text);
        let hint_r = Rect::new(
            content.x,
            button.bottom() + m.px(10.0),
            content.w,
            m.px(22.0),
        );
        paint::text_center(
            out,
            measure,
            hint_r,
            "Ctrl+O, or drop a file on the window",
            m.text_small(),
            t.text_dim,
        );
    }

    /// A ring of dots; one dot is bright and the brightness travels round.
    fn draw_spinner(&self, center: Point, radius: f32, dot: f32, color: Color, out: &mut DrawList) {
        const DOTS: usize = 8;
        let t = self.now.as_secs_f32();
        let head = (t * 1.2).fract() * DOTS as f32;
        for i in 0..DOTS {
            let angle =
                i as f32 / DOTS as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
            let behind = (head - i as f32).rem_euclid(DOTS as f32);
            let a = 1.0 - behind / DOTS as f32 * 0.8;
            let c = Point::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            );
            let r = Rect::new(
                (c.x - dot * 0.5).round(),
                (c.y - dot * 0.5).round(),
                dot,
                dot,
            );
            out.rect(r, fade(color, a), dot * 0.5);
        }
    }

    fn loading_lines(l: &Loading<'_>) -> (String, String) {
        let verb = if l.cloud { "Downloading" } else { "Opening" };
        let mut status = String::new();
        if l.cloud {
            status.push_str("Downloading from cloud drive");
        } else {
            status.push_str("Reading file");
        }
        if let Some(p) = l.progress {
            status.push_str(&format!(" - {:.0}%", (p.clamp(0.0, 1.0) * 100.0).floor()));
        }
        let secs = l.elapsed.as_secs();
        if secs >= 2 {
            status.push_str(&format!(" - {secs} s"));
        }
        (format!("{verb} {}", l.file_name), status)
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_splash(
        &self,
        _ctx: &UiContext<'_>,
        l: &Loading<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        out.rect(content, t.canvas, 0.0);
        let cx = content.x + content.w * 0.5;
        let cy = content.y + content.h * 0.45;
        self.draw_spinner(Point::new(cx, cy), m.px(16.0), m.px(6.0), t.accent, out);
        let (title, status) = Self::loading_lines(l);
        let width = (content.w - 2.0 * m.margin()).max(0.0);
        let mut y = cy + m.px(34.0);
        let row = m.px(24.0);
        let title_r = Rect::new(content.x + m.margin(), y, width, row);
        let shown = paint::elide_middle(measure, &title, m.text_title(), width);
        paint::text_center(out, measure, title_r, &shown, m.text_title(), t.text);
        y += row;
        let font = m.text();
        if l.cloud {
            // Cloud icon, then the status, centered together.
            let icon_gap = m.px(6.0);
            let shown = paint::elide_end(
                measure,
                &status,
                font,
                (width - m.icon() - icon_gap).max(0.0),
            );
            let tw = measure.measure(&shown, font).w;
            let x0 = (cx - (m.icon() + icon_gap + tw) * 0.5).round();
            paint::icon(
                out,
                Rect::new(x0, y, m.icon(), row),
                Icon::Cloud,
                m.icon(),
                t.text_dim,
            );
            let tr = Rect::new(x0 + m.icon() + icon_gap, y, tw, row);
            paint::text_left(out, measure, tr, &shown, font, t.text_dim);
        } else {
            let status_r = Rect::new(content.x + m.margin(), y, width, row);
            let shown = paint::elide_end(measure, &status, font, width);
            paint::text_center(out, measure, status_r, &shown, font, t.text_dim);
        }
        y += row + m.px(6.0);
        if let Some(p) = l.progress {
            let bw = m.px(240.0).min(width);
            let bar = Rect::new((cx - bw * 0.5).round(), y, bw, m.px(6.0));
            out.rect(bar, t.hover, bar.h * 0.5);
            let fw = (bw * p.clamp(0.0, 1.0)).round();
            if fw > 0.0 {
                out.rect(Rect::new(bar.x, bar.y, fw, bar.h), t.accent, bar.h * 0.5);
            }
            y += m.px(18.0);
        }
        let hint_r = Rect::new(content.x + m.margin(), y, width, row);
        paint::text_center(
            out,
            measure,
            hint_r,
            "Press Esc to quit",
            m.text_small(),
            t.text_dim,
        );
    }

    fn draw_loading_pill(
        &self,
        ctx: &UiContext<'_>,
        l: &Loading<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let content = self.content_rect(ctx);
        let (title, status) = Self::loading_lines(l);
        let status = format!("{status} - Press Esc to quit");
        let font = m.text();
        let small = m.text_small();
        let max_w = (content.w - 2.0 * m.margin()).max(m.px(60.0));
        let text_w = measure
            .measure(&title, font)
            .w
            .max(measure.measure(&status, small).w);
        let spin = m.px(36.0);
        let w = (spin + text_w + m.px(16.0))
            .min(max_w)
            .min(m.px(460.0))
            .round();
        let h = m.px(48.0);
        let top = content.y + m.toolbar_h() + m.px(10.0);
        let r = Rect::new(
            (content.x + (content.w - w) * 0.5).round(),
            top.round(),
            w,
            h,
        );
        paint::panel(
            out,
            r,
            t.panel,
            t.border,
            t.shadow,
            m.radius(),
            m.hairline(),
        );
        self.draw_spinner(
            Point::new(r.x + spin * 0.5 + m.px(4.0), r.y + h * 0.5),
            m.px(9.0),
            m.px(4.0),
            t.accent,
            out,
        );
        let tx = r.x + spin + m.px(6.0);
        let tw = (r.right() - tx - m.px(10.0)).max(0.0);
        let half = h * 0.5;
        let line1 = Rect::new(tx, r.y + m.px(4.0), tw, half - m.px(4.0));
        let line2 = Rect::new(tx, r.y + half, tw, half - m.px(4.0));
        let shown = paint::elide_middle(measure, &title, font, tw);
        paint::text_left(out, measure, line1, &shown, font, t.text);
        let shown = paint::elide_end(measure, &status, small, tw);
        paint::text_left(out, measure, line2, &shown, small, t.text_dim);
        if let Some(p) = l.progress {
            let bar = Rect::new(
                r.x + m.px(6.0),
                r.bottom() - m.px(4.0),
                (r.w - m.px(12.0)) * p.clamp(0.0, 1.0),
                m.px(2.0),
            );
            if bar.w >= 1.0 {
                out.rect(paint::snap(bar), t.accent, 0.0);
            }
        }
    }

    /// The information panel: label and value rows in the bottom right
    /// corner of the content area, so above the gallery strip. Rows that
    /// do not fit the height are left out; long values are shortened in
    /// the middle, so file extensions and units stay visible.
    fn draw_info(
        &mut self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        self.info_rect = None;
        if !self.info_wanted(ctx) {
            return;
        }
        let Some(layout) = InfoLayout::new(ctx.info, measure, m, content) else {
            return;
        };
        let panel = layout.panel;
        self.info_rect = Some(panel);
        paint::panel(
            out,
            panel,
            t.panel,
            t.border,
            t.shadow,
            m.radius(),
            m.hairline(),
        );
        let inner_w = (panel.w - 2.0 * layout.pad - layout.gap).max(0.0);
        let label_w = layout.label_w.min((inner_w * 0.45).floor());
        let value_x = panel.x + layout.pad + label_w + layout.gap;
        let value_w = (panel.right() - layout.pad - value_x).max(0.0);
        out.clip(Some(panel));
        for (i, row) in ctx.info.iter().take(layout.rows).enumerate() {
            let y = panel.y + layout.pad + i as f32 * layout.row_h;
            let lr = Rect::new(panel.x + layout.pad, y, label_w, layout.row_h);
            let label = paint::elide_end(measure, &row.label, layout.label_font, lr.w);
            paint::text_left(out, measure, lr, &label, layout.label_font, t.text_dim);
            let vr = Rect::new(value_x, y, value_w, layout.row_h);
            let value = paint::elide_middle(measure, &row.value, layout.value_font, vr.w);
            paint::text_left(out, measure, vr, &value, layout.value_font, t.text);
        }
        out.clip(None);
    }

    /// Toasts, the notice, and background progress, along the bottom of
    /// the content area.
    fn draw_bottom(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        content: Rect,
        out: &mut DrawList,
    ) {
        let font = m.text();
        let h = m.px(34.0);
        let max_w = (content.w - 2.0 * m.margin()).max(m.px(40.0));
        let mut bottom = content.bottom() - m.margin();
        if let Some(notice) = ctx.notice {
            let chrome = m.icon() + m.px(30.0);
            let text_max = (max_w - chrome).max(m.px(20.0));
            let lines = paint::wrap(measure, notice, font, text_max, 3);
            let line_h = m.px(20.0);
            let text_w = lines
                .iter()
                .map(|l| measure.measure(l, font).w)
                .fold(0.0, f32::max);
            let w = (text_w + chrome).min(max_w).round();
            let ph = (lines.len().max(1) as f32 * line_h + m.px(14.0)).max(h);
            let r = Rect::new(
                (content.x + (content.w - w) * 0.5).round(),
                (bottom - ph).round(),
                w,
                ph,
            );
            paint::panel(
                out,
                r,
                t.panel,
                t.border,
                t.shadow,
                m.radius(),
                m.hairline(),
            );
            let ir = Rect::new(r.x + m.px(8.0), r.y, m.icon(), r.h);
            paint::icon(out, ir, Icon::Warning, m.icon(), t.error);
            let tx = ir.right() + m.px(8.0);
            let mut y = r.y + ((ph - lines.len() as f32 * line_h) * 0.5).round();
            for line in &lines {
                let lr = Rect::new(tx, y, (r.right() - tx - m.px(6.0)).max(0.0), line_h);
                paint::text_left(out, measure, lr, line, font, t.text);
                y += line_h;
            }
            bottom = r.y - m.px(8.0);
        }
        for toast in self.toasts.iter().rev() {
            let age = self.now.saturating_sub(toast.start);
            let alpha = if age < metrics::TOAST_HOLD {
                1.0
            } else {
                1.0 - (age - metrics::TOAST_HOLD).as_secs_f32() / metrics::TOAST_FADE.as_secs_f32()
            };
            if alpha <= 0.0 {
                continue;
            }
            let text_w = measure.measure(&toast.text, font).w;
            let w = (text_w + m.px(28.0)).min(max_w).round();
            let r = Rect::new(
                (content.x + (content.w - w) * 0.5).round(),
                (bottom - h).round(),
                w,
                h,
            );
            out.rect(r, fade(t.tooltip, alpha * 0.96), h * 0.5);
            let shown = paint::elide_end(measure, &toast.text, font, w - m.px(20.0));
            paint::text_center(out, measure, r, &shown, font, fade(t.tooltip_text, alpha));
            bottom = r.y - m.px(8.0);
        }
        // Background work: saving, or a bulk job whose dialog is hidden.
        let bulk_hidden = !matches!(&self.dialog, Some(super::Dialog::Bulk(b)) if b.running);
        let work = ctx.busy.or(ctx.bulk.filter(|_| bulk_hidden));
        if let Some(p) = work {
            let text = match p.fraction() {
                Some(f) => format!("{} - {:.0}%", p.label, (f * 100.0).floor()),
                None => p.label.to_string(),
            };
            let w = (measure.measure(&text, m.text_small()).w + m.px(40.0))
                .min(content.w * 0.5)
                .max(m.px(80.0))
                .round();
            let r = Rect::new(
                content.x + m.margin(),
                content.bottom() - m.margin() - m.px(30.0),
                w,
                m.px(30.0),
            );
            paint::panel(
                out,
                r,
                t.panel,
                t.border,
                t.shadow,
                m.radius(),
                m.hairline(),
            );
            self.draw_spinner(
                Point::new(r.x + m.px(15.0), r.y + r.h * 0.5),
                m.px(6.0),
                m.px(3.0),
                t.accent,
                out,
            );
            let tr = Rect::new(r.x + m.px(28.0), r.y, (r.w - m.px(34.0)).max(0.0), r.h);
            let shown = paint::elide_end(measure, &text, m.text_small(), tr.w);
            paint::text_left(out, measure, tr, &shown, m.text_small(), t.text);
            if let Some(f) = p.fraction() {
                let bar = Rect::new(
                    r.x + m.px(6.0),
                    r.bottom() - m.px(3.0),
                    ((r.w - m.px(12.0)) * f).round(),
                    m.px(2.0),
                );
                if bar.w >= 1.0 {
                    out.rect(bar, t.accent, 0.0);
                }
            }
        }
    }

    fn draw_dialogs(
        &mut self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let win = ctx.window_rect();
        let paint_ctx = FormPaint {
            theme: t,
            m,
            measure,
        };
        if let Some(layout) = self.dialog_layout(ctx, m) {
            out.rect(win, t.backdrop, 0.0);
            if let Some(dialog) = &self.dialog {
                form::draw(
                    out,
                    &paint_ctx,
                    &layout,
                    &self.dialog_form,
                    dialog.host(),
                    &mut self.field_geoms,
                );
            }
        }
        if let Some(layout) = self.confirm_layout(ctx, m) {
            out.rect(win, t.backdrop, 0.0);
            if let Some(confirm) = &self.confirm {
                let mut scratch = Vec::new();
                form::draw(
                    out,
                    &paint_ctx,
                    &layout,
                    &self.confirm_form,
                    confirm,
                    &mut scratch,
                );
            }
        }
    }

    fn draw_help(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let win = ctx.window_rect();
        out.rect(win, t.backdrop, 0.0);
        let rows: Vec<(String, &'static str)> = Command::ALL
            .iter()
            .filter(|cmd| ctx.text_enabled || !cmd.is_text())
            .filter_map(|cmd| {
                let keys: Vec<String> = SHORTCUTS
                    .iter()
                    .filter(|s| s.command == *cmd)
                    .map(|s| s.label())
                    .collect();
                (!keys.is_empty()).then(|| (keys.join(" / "), cmd.label()))
            })
            .collect();
        let margin = m.margin();
        let row_h = m.px(22.0);
        let col_w = m.px(340.0);
        let avail_w = (win.w - 2.0 * margin).max(m.px(100.0));
        let cols = ((avail_w / col_w) as usize).clamp(1, 3);
        let per_col = rows.len().div_ceil(cols);
        let w = (col_w * cols as f32).min(avail_w);
        let h = (m.px(56.0) + per_col as f32 * row_h + m.px(36.0))
            .min((win.h - 2.0 * margin).max(m.px(60.0)));
        let panel = Rect::new(
            (win.x + (win.w - w) * 0.5).round(),
            (win.y + (win.h - h) * 0.5).round(),
            w,
            h,
        );
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
        let title = Rect::new(
            panel.x + m.px(18.0),
            panel.y + m.px(12.0),
            panel.w - m.px(36.0),
            m.px(28.0),
        );
        paint::text_left(
            out,
            measure,
            title,
            "Keyboard shortcuts",
            m.text_title(),
            t.text,
        );
        let font = m.text();
        let cw = w / cols as f32;
        let key_w = (cw * 0.42).round();
        for (i, (keys, label)) in rows.iter().enumerate() {
            let col = i / per_col;
            let row = i % per_col;
            let x = panel.x + m.px(18.0) + col as f32 * cw;
            let y = panel.y + m.px(48.0) + row as f32 * row_h;
            let kr = Rect::new(x, y, key_w - m.px(8.0), row_h);
            let lr = Rect::new(x + key_w, y, (cw - key_w - m.px(24.0)).max(0.0), row_h);
            let k = paint::elide_end(measure, keys, m.text_small(), kr.w);
            paint::text_left(out, measure, kr, &k, m.text_small(), t.accent);
            let l = paint::elide_end(measure, label, font, lr.w);
            paint::text_left(out, measure, lr, &l, font, t.text);
        }
        let foot = Rect::new(panel.x, panel.bottom() - m.px(30.0), panel.w, m.px(24.0));
        paint::text_center(
            out,
            measure,
            foot,
            "Press Esc to close",
            m.text_small(),
            t.text_dim,
        );
        out.clip(None);
    }

    fn draw_menu(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        let Some(menu) = &self.menu else { return };
        let win = ctx.window_rect();
        let panel = menu.rect(win, m);
        paint::panel(
            out,
            panel,
            t.panel_solid,
            t.border,
            t.shadow,
            m.radius(),
            m.hairline(),
        );
        out.clip(Some(panel));
        let font = m.text();
        let gutter = m.px(30.0);
        menu.for_each_item(win, m, |i, r| {
            if r.bottom() < panel.y || r.y > panel.bottom() {
                return;
            }
            match &menu.items[i] {
                MenuItem::Separator => {
                    out.rect(
                        Rect::new(
                            r.x + m.px(10.0),
                            (r.y + r.h * 0.5).round(),
                            r.w - m.px(20.0),
                            m.hairline(),
                        ),
                        t.separator,
                        0.0,
                    );
                }
                MenuItem::Entry {
                    label,
                    shortcut,
                    enabled,
                    checked,
                    ..
                } => {
                    let inner = Rect::new(r.x + m.px(4.0), r.y, r.w - m.px(8.0), r.h);
                    if menu.hover == Some(i) && *enabled {
                        out.rect(inner, t.hover, m.radius_small());
                    }
                    let color = if *enabled { t.text } else { t.text_disabled };
                    if *checked {
                        let ir = Rect::new(inner.x, inner.y, gutter, inner.h);
                        paint::icon(out, ir, Icon::Check, m.icon(), color);
                    }
                    let sc_w = shortcut
                        .as_deref()
                        .map_or(0.0, |s| measure.measure(s, m.text_small()).w);
                    let lr = Rect::new(
                        inner.x + gutter,
                        inner.y,
                        (inner.w - gutter - sc_w - m.px(18.0)).max(0.0),
                        inner.h,
                    );
                    let shown = paint::elide_end(measure, label, font, lr.w);
                    paint::text_left(out, measure, lr, &shown, font, color);
                    if let Some(sc) = shortcut {
                        let sr = Rect::new(inner.x, inner.y, inner.w - m.px(10.0), inner.h);
                        let sc_color = if *enabled {
                            t.text_dim
                        } else {
                            t.text_disabled
                        };
                        paint::text_right(out, measure, sr, sc, m.text_small(), sc_color);
                    }
                }
            }
        });
        out.clip(None);
    }

    fn draw_tooltip(
        &self,
        ctx: &UiContext<'_>,
        measure: &dyn TextMeasure,
        m: &Metrics,
        t: &Theme,
        out: &mut DrawList,
    ) {
        if self.capture.is_some()
            || self.tooltip_blocked
            || self.mode() == super::UiMode::Modal
            || self.toolbar_alpha < 1.0
        {
            return;
        }
        if self.now < self.hover_since + metrics::TOOLTIP_DELAY {
            return;
        }
        let (text, anchor) = match self.hover {
            Target::Toolbar(cmd) => {
                let Some(r) = self.toolbar.button_rect(cmd) else {
                    return;
                };
                let mut text = match cmd {
                    Command::Fullscreen if ctx.fullscreen => {
                        format!(
                            "Leave full screen  ({})",
                            cmd.shortcut_label().unwrap_or_default()
                        )
                    }
                    _ => cmd.tooltip(),
                };
                if !self.command_enabled(cmd, ctx) && cmd.needs_image() {
                    text.push_str(" - open an image first");
                }
                (text, r)
            }
            Target::More => match self.toolbar.more {
                Some(r) => ("More commands".to_string(), r),
                None => return,
            },
            _ => return,
        };
        let font = m.text_small();
        let size = measure.measure(&text, font);
        let pad = m.px(8.0);
        let w = (size.w + 2.0 * pad).round();
        let h = (size.h + pad).round().max(m.px(24.0));
        let win = ctx.window_rect();
        let r = paint::keep_inside(
            Rect::new(
                (anchor.x + anchor.w * 0.5 - w * 0.5).round(),
                anchor.bottom() + m.px(6.0),
                w,
                h,
            ),
            win,
        );
        out.rect(r, t.tooltip, m.radius_small());
        paint::text_center(out, measure, r, &text, font, t.tooltip_text);
    }
}

/// Geometry of the information panel for one frame.
struct InfoLayout {
    /// The panel, anchored to the bottom right corner of the content area.
    panel: Rect,
    /// Rows that fit, counted from the first.
    rows: usize,
    /// Width of the widest label.
    label_w: f32,
    pad: f32,
    gap: f32,
    row_h: f32,
    label_font: f32,
    value_font: f32,
}

impl InfoLayout {
    /// The layout for `rows` in `content`, or `None` when not even one row
    /// fits. The panel keeps a margin to the content edges (less in a very
    /// small window) and stays below the toolbar when the window is tall
    /// enough for both.
    fn new(
        rows: &[crate::context::InfoRow],
        measure: &dyn TextMeasure,
        m: &Metrics,
        content: Rect,
    ) -> Option<Self> {
        let label_font = m.text_small();
        let value_font = m.text();
        let pad = m.px(10.0);
        let gap = m.px(12.0);
        let row_h = m.px(20.0);
        let margin = m
            .margin()
            .min((content.w * 0.25).floor())
            .min((content.h * 0.25).floor())
            .max(0.0);
        let top = if content.h >= m.px(240.0) {
            m.toolbar_h()
        } else {
            0.0
        };
        let avail_w = content.w - 2.0 * margin;
        let avail_h = content.h - 2.0 * margin - top;
        let fit = ((avail_h - 2.0 * pad) / row_h).floor();
        // NaN-safe: a degenerate window draws no panel.
        if !(fit >= 1.0 && avail_w >= 2.0 * pad + gap + m.px(40.0)) {
            return None;
        }
        let shown = (fit as usize).min(rows.len());
        let widest = |font: f32, pick: fn(&crate::context::InfoRow) -> &str| {
            rows.iter()
                .map(|r| measure.measure(pick(r), font).w)
                .fold(0.0f32, f32::max)
                .ceil()
        };
        let label_w = widest(label_font, |r| &r.label);
        let value_w = widest(value_font, |r| &r.value);
        let w = (2.0 * pad + label_w + gap + value_w)
            .min(m.px(460.0))
            .min(avail_w)
            .floor();
        let h = 2.0 * pad + shown as f32 * row_h;
        let panel = Rect::new(
            (content.right() - margin - w).round(),
            (content.bottom() - margin - h).round(),
            w,
            h,
        );
        Some(Self {
            panel,
            rows: shown,
            label_w,
            pad,
            gap,
            row_h,
            label_font,
            value_font,
        })
    }
}

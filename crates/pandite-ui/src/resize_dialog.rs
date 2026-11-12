//! The resize dialog: percent, best fit in a box, or an exact size with an
//! optional aspect lock. Shows the resulting size and refuses bad input.

use crate::action::{Action, ResizeSpec};
use crate::context::UiContext;
use crate::form::{ButtonStyle, FormBuilder, FormEvent, FormHost, Id, Outcome, Tone};
use crate::text_field::{CharFilter, TextField};
use pandite_core::image::MAX_DIMENSION;

pub(crate) const SEG_MODE: Id = 1;
pub(crate) const F_PERCENT: Id = 2;
pub(crate) const F_FIT_W: Id = 3;
pub(crate) const F_FIT_H: Id = 4;
pub(crate) const CHK_ENLARGE: Id = 5;
pub(crate) const F_EXACT_W: Id = 6;
pub(crate) const F_EXACT_H: Id = 7;
pub(crate) const CHK_ASPECT: Id = 8;
pub(crate) const BTN_CANCEL: Id = 9;
pub(crate) const BTN_APPLY: Id = 10;

/// Largest percentage the dialog accepts.
pub(crate) const MAX_PERCENT: f32 = 1000.0;

/// Which rule the dialog edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResizeMode {
    Percent,
    Fit,
    Exact,
}

impl ResizeMode {
    const ALL: [ResizeMode; 3] = [ResizeMode::Percent, ResizeMode::Fit, ResizeMode::Exact];
    pub const LABELS: [&'static str; 3] = ["Percent", "Fit in box", "Exact size"];

    fn index(self) -> usize {
        match self {
            ResizeMode::Percent => 0,
            ResizeMode::Fit => 1,
            ResizeMode::Exact => 2,
        }
    }
}

/// Parse a pixel size field.
pub(crate) fn parse_side(field: &TextField) -> Option<u32> {
    let v: u32 = field.text().parse().ok()?;
    (1..=MAX_DIMENSION).contains(&v).then_some(v)
}

/// Parse a percent field.
pub(crate) fn parse_percent(field: &TextField) -> Option<f32> {
    let v: f32 = field.text().parse().ok()?;
    (v.is_finite() && v > 0.0 && v <= MAX_PERCENT).then_some(v)
}

/// The resize dialog state.
#[derive(Clone, Debug)]
pub(crate) struct ResizeDialog {
    pub mode: ResizeMode,
    percent: TextField,
    fit_w: TextField,
    fit_h: TextField,
    exact_w: TextField,
    exact_h: TextField,
    pub enlarge: bool,
    pub keep_aspect: bool,
}

impl ResizeDialog {
    pub fn new(width: u32, height: u32) -> Self {
        let digits = |v: u32| TextField::with_text(CharFilter::Digits, 5, &v.to_string());
        let mut percent = TextField::with_text(CharFilter::Decimal, 7, "50");
        percent.select_all();
        Self {
            mode: ResizeMode::Percent,
            percent,
            fit_w: digits(1920),
            fit_h: digits(1080),
            exact_w: digits(width.max(1)),
            exact_h: digits(height.max(1)),
            enlarge: false,
            keep_aspect: true,
        }
    }

    /// The first field of the current mode, focused when the mode changes.
    pub fn first_field(&self) -> Id {
        match self.mode {
            ResizeMode::Percent => F_PERCENT,
            ResizeMode::Fit => F_FIT_W,
            ResizeMode::Exact => F_EXACT_W,
        }
    }

    /// The spec and its result for an image of `size`, or why it is invalid.
    pub fn validate(&self, size: Option<(u32, u32)>) -> Result<(ResizeSpec, (u32, u32)), String> {
        let spec = match self.mode {
            ResizeMode::Percent => {
                let p = parse_percent(&self.percent).ok_or_else(|| {
                    format!("Enter a percentage above 0 and at most {MAX_PERCENT}.")
                })?;
                ResizeSpec::Percent(p)
            }
            ResizeMode::Fit => {
                let (Some(w), Some(h)) = (parse_side(&self.fit_w), parse_side(&self.fit_h)) else {
                    return Err(format!(
                        "Enter a width and height from 1 to {MAX_DIMENSION}."
                    ));
                };
                ResizeSpec::Fit {
                    width: w,
                    height: h,
                    enlarge: self.enlarge,
                }
            }
            ResizeMode::Exact => {
                let (Some(w), Some(h)) = (parse_side(&self.exact_w), parse_side(&self.exact_h))
                else {
                    return Err(format!(
                        "Enter a width and height from 1 to {MAX_DIMENSION}."
                    ));
                };
                ResizeSpec::Exact {
                    width: w,
                    height: h,
                }
            }
        };
        let (w, h) = size.ok_or_else(|| "No image is open.".to_string())?;
        let result = spec
            .result_size(w, h)
            .ok_or_else(|| format!("The result would exceed {MAX_DIMENSION} pixels."))?;
        Ok((spec, result))
    }

    fn sync_aspect(&mut self, changed: Id, size: Option<(u32, u32)>) {
        let Some((w0, h0)) = size else { return };
        if !self.keep_aspect || w0 == 0 || h0 == 0 {
            return;
        }
        let ratio = f64::from(h0) / f64::from(w0);
        if changed == F_EXACT_W {
            if let Ok(w) = self.exact_w.text().parse::<u32>() {
                let h = (f64::from(w) * ratio).round().max(1.0) as u64;
                self.exact_h.set_text(&h.to_string());
            }
        } else if changed == F_EXACT_H {
            if let Ok(h) = self.exact_h.text().parse::<u32>() {
                let w = (f64::from(h) / ratio).round().max(1.0) as u64;
                self.exact_w.set_text(&w.to_string());
            }
        }
    }

    /// React to a form event. `size` is the current image size.
    pub fn on_event(&mut self, event: &FormEvent, size: Option<(u32, u32)>) -> Outcome {
        match event {
            FormEvent::Segment(SEG_MODE, i) => {
                if let Some(mode) = ResizeMode::ALL.get(*i) {
                    self.mode = *mode;
                }
                Outcome::Stay
            }
            FormEvent::FieldChanged(id) => {
                self.sync_aspect(*id, size);
                Outcome::Stay
            }
            FormEvent::Activate(CHK_ENLARGE) => {
                self.enlarge = !self.enlarge;
                Outcome::Stay
            }
            FormEvent::Activate(CHK_ASPECT) => {
                self.keep_aspect = !self.keep_aspect;
                self.sync_aspect(F_EXACT_W, size);
                Outcome::Stay
            }
            FormEvent::Activate(BTN_CANCEL) | FormEvent::Cancel => Outcome::Close(Vec::new()),
            FormEvent::Activate(BTN_APPLY) | FormEvent::Submit => match self.validate(size) {
                Ok((spec, _)) => Outcome::Close(vec![Action::ApplyResize(spec)]),
                Err(_) => Outcome::Stay,
            },
            FormEvent::Copy(text) => Outcome::Emit(vec![Action::CopyText(text.clone())]),
            FormEvent::Paste => Outcome::Emit(vec![Action::RequestPaste]),
            _ => Outcome::Stay,
        }
    }
}

/// Image size from the context.
pub(crate) fn image_size(ctx: &UiContext<'_>) -> Option<(u32, u32)> {
    ctx.image.map(|i| (i.width, i.height))
}

impl FormHost for ResizeDialog {
    fn build(&self, ctx: &UiContext<'_>, b: &mut FormBuilder<'_>) {
        let size = image_size(ctx);
        b.title("Resize image");
        match size {
            Some((w, h)) => b.text(format!("Current size: {w} x {h} px"), Tone::Dim),
            None => b.text("No image is open.", Tone::Dim),
        }
        b.space(6.0);
        b.segmented_row(
            Some("Resize by"),
            SEG_MODE,
            &ResizeMode::LABELS,
            self.mode.index(),
            true,
        );
        match self.mode {
            ResizeMode::Percent => {
                b.field_row("Scale", F_PERCENT, 90.0, Some("%"), true);
            }
            ResizeMode::Fit => {
                b.field_row("Max width", F_FIT_W, 90.0, Some("px"), true);
                b.field_row("Max height", F_FIT_H, 90.0, Some("px"), true);
                b.check_row(CHK_ENLARGE, "Enlarge smaller images", self.enlarge, true);
            }
            ResizeMode::Exact => {
                b.field_row("Width", F_EXACT_W, 90.0, Some("px"), true);
                b.field_row("Height", F_EXACT_H, 90.0, Some("px"), true);
                b.check_row(CHK_ASPECT, "Keep aspect ratio", self.keep_aspect, true);
            }
        }
        b.space(4.0);
        let valid = match self.validate(size) {
            Ok((_, (w, h))) => {
                b.text(format!("New size: {w} x {h} px"), Tone::Normal);
                true
            }
            Err(msg) => {
                b.text(msg, Tone::Error);
                false
            }
        };
        b.space(8.0);
        b.buttons_row(
            &[
                (BTN_CANCEL, "Cancel", ButtonStyle::Secondary, true),
                (BTN_APPLY, "Resize", ButtonStyle::Primary, valid),
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
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_lock_follows_width_and_height() {
        let mut d = ResizeDialog::new(4000, 3000);
        d.mode = ResizeMode::Exact;
        let size = Some((4000, 3000));
        d.exact_w.set_text("2000");
        d.on_event(&FormEvent::FieldChanged(F_EXACT_W), size);
        assert_eq!(d.exact_h.text(), "1500");
        d.exact_h.set_text("300");
        d.on_event(&FormEvent::FieldChanged(F_EXACT_H), size);
        assert_eq!(d.exact_w.text(), "400");
        d.on_event(&FormEvent::Activate(CHK_ASPECT), size);
        d.exact_w.set_text("10");
        d.on_event(&FormEvent::FieldChanged(F_EXACT_W), size);
        assert_eq!(d.exact_h.text(), "300", "unlocked height stays");
    }

    #[test]
    fn validation_messages() {
        let mut d = ResizeDialog::new(100, 100);
        d.percent.set_text("0");
        assert!(d.validate(Some((100, 100))).is_err());
        d.percent.set_text("1000.5");
        assert!(d.validate(Some((100, 100))).is_err());
        d.percent.set_text("25");
        assert_eq!(
            d.validate(Some((100, 100))).unwrap(),
            (ResizeSpec::Percent(25.0), (25, 25))
        );
        assert!(d.validate(None).is_err());
    }
}

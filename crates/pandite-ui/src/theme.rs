//! Dark and light color tables.
//!
//! The application reads the system setting and passes it as
//! [`crate::UiContext::theme`]. Every color the interface draws comes from
//! one [`Theme`]; nothing is hard-coded in the widgets.

use pandite_core::Color;

/// Which color table to use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeKind {
    /// Light text on dark panels. The default.
    #[default]
    Dark,
    /// Dark text on light panels.
    Light,
}

/// Every color the interface uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// Window background behind the image. The renderer clears with it.
    pub canvas: Color,
    /// Translucent toolbar and overlay panels.
    pub panel: Color,
    /// Opaque panels: dialogs, menus, gallery strip.
    pub panel_solid: Color,
    /// Hairline borders around panels.
    pub border: Color,
    /// Separators between groups.
    pub separator: Color,
    /// Main text.
    pub text: Color,
    /// Secondary text.
    pub text_dim: Color,
    /// Text and icons of disabled items.
    pub text_disabled: Color,
    /// Icon tint.
    pub icon: Color,
    /// Hover highlight.
    pub hover: Color,
    /// Pressed highlight.
    pub pressed: Color,
    /// Accent: current item, focus ring, primary button.
    pub accent: Color,
    /// Text on the accent color.
    pub accent_text: Color,
    /// Validation errors.
    pub error: Color,
    /// Text field background.
    pub field: Color,
    /// Text field border.
    pub field_border: Color,
    /// Selected text background.
    pub selection: Color,
    /// Tooltip background.
    pub tooltip: Color,
    /// Tooltip text.
    pub tooltip_text: Color,
    /// Dims everything behind a modal dialog.
    pub backdrop: Color,
    /// Dims the image outside the crop selection.
    pub crop_dim: Color,
    /// Crop selection outline and handles.
    pub crop_line: Color,
    /// Dark edge around crop lines so they show on light images.
    pub crop_shadow: Color,
    /// Gallery cell background and missing thumbnail fill.
    pub placeholder: Color,
    /// Drop shadow under floating panels.
    pub shadow: Color,
    /// Fill of the selection badge of a thumbnail and of the "Marked" pill:
    /// a light gray blue that reads on any photo.
    pub check_fill: Color,
    /// Outline, check mark and text of the selection badge and the
    /// "Marked" pill.
    pub check_line: Color,
}

impl Theme {
    /// The dark table.
    pub const DARK: Theme = Theme {
        canvas: Color::rgb(18, 18, 19),
        panel: Color::rgba(30, 30, 32, 232),
        panel_solid: Color::rgb(30, 30, 32),
        border: Color::rgba(255, 255, 255, 26),
        separator: Color::rgba(255, 255, 255, 30),
        text: Color::rgb(236, 236, 238),
        text_dim: Color::rgb(160, 160, 166),
        text_disabled: Color::rgb(92, 92, 98),
        icon: Color::rgb(226, 226, 230),
        hover: Color::rgba(255, 255, 255, 26),
        pressed: Color::rgba(255, 255, 255, 46),
        accent: Color::rgb(72, 148, 255),
        accent_text: Color::rgb(8, 16, 32),
        error: Color::rgb(255, 120, 112),
        field: Color::rgb(20, 20, 22),
        field_border: Color::rgba(255, 255, 255, 48),
        selection: Color::rgba(72, 148, 255, 110),
        tooltip: Color::rgb(52, 52, 56),
        tooltip_text: Color::rgb(244, 244, 246),
        backdrop: Color::rgba(0, 0, 0, 140),
        crop_dim: Color::rgba(0, 0, 0, 150),
        crop_line: Color::rgb(255, 255, 255),
        crop_shadow: Color::rgba(0, 0, 0, 150),
        placeholder: Color::rgb(44, 44, 48),
        shadow: Color::rgba(0, 0, 0, 90),
        check_fill: Color::rgb(198, 210, 226),
        check_line: Color::rgb(20, 26, 36),
    };

    /// The light table.
    pub const LIGHT: Theme = Theme {
        canvas: Color::rgb(238, 238, 240),
        panel: Color::rgba(250, 250, 251, 236),
        panel_solid: Color::rgb(250, 250, 251),
        border: Color::rgba(0, 0, 0, 30),
        separator: Color::rgba(0, 0, 0, 34),
        text: Color::rgb(28, 28, 30),
        text_dim: Color::rgb(88, 88, 94),
        text_disabled: Color::rgb(170, 170, 176),
        icon: Color::rgb(40, 40, 44),
        hover: Color::rgba(0, 0, 0, 16),
        pressed: Color::rgba(0, 0, 0, 32),
        accent: Color::rgb(10, 96, 214),
        accent_text: Color::rgb(255, 255, 255),
        error: Color::rgb(190, 30, 30),
        field: Color::rgb(255, 255, 255),
        field_border: Color::rgba(0, 0, 0, 60),
        selection: Color::rgba(10, 96, 214, 70),
        tooltip: Color::rgb(36, 36, 40),
        tooltip_text: Color::rgb(246, 246, 248),
        backdrop: Color::rgba(0, 0, 0, 90),
        crop_dim: Color::rgba(0, 0, 0, 130),
        crop_line: Color::rgb(255, 255, 255),
        crop_shadow: Color::rgba(0, 0, 0, 140),
        placeholder: Color::rgb(222, 222, 226),
        shadow: Color::rgba(0, 0, 0, 40),
        check_fill: Color::rgb(198, 210, 226),
        check_line: Color::rgb(20, 26, 36),
    };

    /// The table for `kind`.
    pub fn for_kind(kind: ThemeKind) -> &'static Theme {
        match kind {
            ThemeKind::Dark => &Theme::DARK,
            ThemeKind::Light => &Theme::LIGHT,
        }
    }

    /// Every color with its field name, for checks and documentation.
    pub fn entries(&self) -> [(&'static str, Color); 25] {
        [
            ("canvas", self.canvas),
            ("panel", self.panel),
            ("panel_solid", self.panel_solid),
            ("border", self.border),
            ("separator", self.separator),
            ("text", self.text),
            ("text_dim", self.text_dim),
            ("text_disabled", self.text_disabled),
            ("icon", self.icon),
            ("hover", self.hover),
            ("pressed", self.pressed),
            ("accent", self.accent),
            ("accent_text", self.accent_text),
            ("error", self.error),
            ("field", self.field),
            ("field_border", self.field_border),
            ("selection", self.selection),
            ("tooltip", self.tooltip),
            ("tooltip_text", self.tooltip_text),
            ("backdrop", self.backdrop),
            ("crop_dim", self.crop_dim),
            ("crop_line", self.crop_line),
            ("crop_shadow", self.crop_shadow),
            ("placeholder", self.placeholder),
            ("shadow", self.shadow),
        ]
    }
}

/// WCAG relative luminance of an opaque color.
pub fn luminance(c: Color) -> f32 {
    fn channel(v: u8) -> f32 {
        let s = f32::from(v) / 255.0;
        if s <= 0.040_45 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG contrast ratio between two opaque colors (1 to 21).
pub fn contrast(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Multiply a color's alpha by `factor` (0 to 1).
pub(crate) fn fade(c: Color, factor: f32) -> Color {
    let a = (f32::from(c.a) * factor.clamp(0.0, 1.0)).round() as u8;
    c.with_alpha(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_readable_in_both_themes() {
        for theme in [&Theme::DARK, &Theme::LIGHT] {
            assert!(contrast(theme.text, theme.panel_solid) >= 7.0);
            assert!(contrast(theme.text_dim, theme.panel_solid) >= 4.5);
            assert!(contrast(theme.accent_text, theme.accent) >= 4.5);
            assert!(contrast(theme.tooltip_text, theme.tooltip) >= 7.0);
            assert!(contrast(theme.error, theme.panel_solid) >= 4.5);
        }
    }
}

//! Drawing helpers shared by the widgets: pixel snapping, text placement,
//! elision and wrapping.

use crate::icon::Icon;
use crate::theme::fade;
use pandite_core::{Color, DrawList, Point, Rect, Size, TextMeasure};
use std::borrow::Cow;

/// Round a rectangle's edges to whole pixels.
pub(crate) fn snap(r: Rect) -> Rect {
    let x0 = r.x.round();
    let y0 = r.y.round();
    let x1 = (r.x + r.w).round();
    let y1 = (r.y + r.h).round();
    Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
}

/// Grow a rectangle by `d` on every side.
pub(crate) fn outset(r: Rect, d: f32) -> Rect {
    Rect::new(r.x - d, r.y - d, r.w + 2.0 * d, r.h + 2.0 * d)
}

/// Draw an icon of `size` pixels centered in `rect`, snapped so texels
/// land on pixels.
pub(crate) fn icon(out: &mut DrawList, rect: Rect, icon: Icon, size: f32, tint: Color) {
    let x = (rect.x + (rect.w - size) * 0.5).round();
    let y = (rect.y + (rect.h - size) * 0.5).round();
    out.image(Rect::new(x, y, size, size), icon.texture_id(), tint);
}

/// Draw text vertically centered in `rect`, starting at its left edge.
pub(crate) fn text_left(
    out: &mut DrawList,
    measure: &dyn TextMeasure,
    rect: Rect,
    text: &str,
    size: f32,
    color: Color,
) -> Size {
    let m = measure.measure(text, size);
    let y = (rect.y + (rect.h - m.h) * 0.5).round();
    out.text(Point::new(rect.x.round(), y), text, size, color);
    m
}

/// Draw text centered in `rect`.
pub(crate) fn text_center(
    out: &mut DrawList,
    measure: &dyn TextMeasure,
    rect: Rect,
    text: &str,
    size: f32,
    color: Color,
) {
    let m = measure.measure(text, size);
    let x = (rect.x + (rect.w - m.w) * 0.5).round();
    let y = (rect.y + (rect.h - m.h) * 0.5).round();
    out.text(Point::new(x, y), text, size, color);
}

/// Draw text right-aligned in `rect`.
pub(crate) fn text_right(
    out: &mut DrawList,
    measure: &dyn TextMeasure,
    rect: Rect,
    text: &str,
    size: f32,
    color: Color,
) {
    let m = measure.measure(text, size);
    let x = (rect.right() - m.w).round();
    let y = (rect.y + (rect.h - m.h) * 0.5).round();
    out.text(Point::new(x, y), text, size, color);
}

const ELLIPSIS: &str = "...";

/// `text` shortened at the end with "..." so it fits `max_w`.
pub(crate) fn elide_end<'t>(
    measure: &dyn TextMeasure,
    text: &'t str,
    size: f32,
    max_w: f32,
) -> Cow<'t, str> {
    if text.is_empty() || measure.measure(text, size).w <= max_w {
        return Cow::Borrowed(text);
    }
    // offsets[k] is the byte length of the first k characters.
    let offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let mut buf = String::with_capacity(text.len() + ELLIPSIS.len());
    let mut fits = |k: usize| {
        buf.clear();
        buf.push_str(text[..offsets[k]].trim_end());
        buf.push_str(ELLIPSIS);
        measure.measure(&buf, size).w <= max_w
    };
    // Binary search the longest prefix that fits with the ellipsis.
    let (mut lo, mut hi) = (0usize, offsets.len() - 1);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return Cow::Owned(if fits(0) {
            ELLIPSIS.to_string()
        } else {
            String::new()
        });
    }
    Cow::Owned(format!("{}{ELLIPSIS}", text[..offsets[lo]].trim_end()))
}

/// `text` shortened in the middle with "..." so it fits `max_w`. Keeps the
/// end, so file extensions stay visible.
pub(crate) fn elide_middle<'t>(
    measure: &dyn TextMeasure,
    text: &'t str,
    size: f32,
    max_w: f32,
) -> Cow<'t, str> {
    if measure.measure(text, size).w <= max_w {
        return Cow::Borrowed(text);
    }
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    // Keep `k` characters in total, split between head and tail.
    let build = |k: usize| -> String {
        let tail = k.div_ceil(2);
        let head = k - tail;
        let head_end = chars.get(head).map_or(text.len(), |c| c.0);
        let tail_start = if tail == 0 {
            text.len()
        } else {
            chars[n - tail].0
        };
        format!("{}{ELLIPSIS}{}", &text[..head_end], &text[tail_start..])
    };
    let (mut lo, mut hi) = (0usize, n.saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure.measure(&build(mid), size).w <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 {
        return elide_end(measure, text, size, max_w);
    }
    Cow::Owned(build(lo))
}

/// Break `text` into lines no wider than `max_w`, at most `max_lines`. The
/// last line is elided when text remains.
pub(crate) fn wrap(
    measure: &dyn TextMeasure,
    text: &str,
    size: f32,
    max_w: f32,
    max_lines: usize,
) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        let word = words[i];
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if measure.measure(&candidate, size).w <= max_w || current.is_empty() {
            current = candidate;
            i += 1;
        } else {
            lines.push(std::mem::take(&mut current));
            if lines.len() == max_lines {
                break;
            }
        }
    }
    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    } else if i < words.len() && !lines.is_empty() {
        let last = lines.len() - 1;
        let rest = format!("{} {}", lines[last], words[i..].join(" "));
        lines[last] = elide_end(measure, &rest, size, max_w).into_owned();
    }
    for line in &mut lines {
        if measure.measure(line, size).w > max_w {
            *line = elide_end(measure, line, size, max_w).into_owned();
        }
    }
    lines
}

/// A floating panel with a soft shadow, a fill and a hairline border.
pub(crate) fn panel(
    out: &mut DrawList,
    rect: Rect,
    fill: Color,
    border: Color,
    shadow: Color,
    radius: f32,
    hairline: f32,
) {
    let spread = (radius * 0.5).max(1.0);
    out.rect(
        Rect::new(
            rect.x - spread * 0.5,
            rect.y,
            rect.w + spread,
            rect.h + spread * 1.5,
        ),
        fade(shadow, 0.6),
        radius + spread,
    );
    out.rect(rect, fill, radius);
    out.border(rect, border, hairline, radius);
}

/// A rectangle that keeps `rect` inside `bounds` where possible (moved, not
/// resized, unless it is larger than `bounds`).
pub(crate) fn keep_inside(rect: Rect, bounds: Rect) -> Rect {
    let w = rect.w.min(bounds.w.max(0.0));
    let h = rect.h.min(bounds.h.max(0.0));
    let x = rect.x.min(bounds.right() - w).max(bounds.x);
    let y = rect.y.min(bounds.bottom() - h).max(bounds.y);
    Rect::new(x, y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed;
    impl TextMeasure for Fixed {
        fn measure(&self, text: &str, size: f32) -> Size {
            Size::new(text.chars().count() as f32 * size * 0.5, size * 1.25)
        }
    }

    #[test]
    fn elide_keeps_text_that_fits_and_shortens_the_rest() {
        // 10 px per character at size 20.
        assert_eq!(elide_end(&Fixed, "abcdef", 20.0, 60.0), "abcdef");
        assert_eq!(elide_end(&Fixed, "abcdefghij", 20.0, 60.0), "abc...");
        assert_eq!(
            elide_middle(&Fixed, "very_long_name.jpeg", 20.0, 100.0),
            "ver...jpeg"
        );
        assert_eq!(elide_end(&Fixed, "abcdef", 20.0, 5.0), "");
    }

    #[test]
    fn wrap_breaks_on_words() {
        let lines = wrap(&Fixed, "one two three four", 20.0, 90.0, 3);
        assert_eq!(lines, vec!["one two", "three", "four"]);
        let lines = wrap(&Fixed, "one two three four five six", 20.0, 90.0, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."));
    }

    #[test]
    fn keep_inside_moves_into_bounds() {
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            keep_inside(Rect::new(90.0, -5.0, 20.0, 20.0), b),
            Rect::new(80.0, 0.0, 20.0, 20.0)
        );
    }
}

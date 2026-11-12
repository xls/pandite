//! Single-line text editing state: text, caret and selection.
//!
//! The caret and the selection anchor are byte offsets that always sit on
//! character boundaries. The field never draws itself; dialogs draw it.

use crate::input::{Key, Modifiers};

/// Which characters a field accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CharFilter {
    /// Digits only.
    Digits,
    /// Digits and one decimal point.
    Decimal,
    /// Any printable character.
    Any,
}

/// What a key did to a field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldResponse {
    /// The key is not an editing key; let others handle it.
    Ignored,
    /// The key was handled; the text is unchanged.
    Handled,
    /// The text changed.
    Changed,
    /// Copy this text to the clipboard (the text may also have changed
    /// when it was a cut).
    Copy {
        /// The selected text.
        text: String,
        /// True when the selection was removed (cut).
        cut: bool,
    },
    /// Ask the application for clipboard text.
    Paste,
}

/// A single-line text field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextField {
    text: String,
    caret: usize,
    anchor: usize,
    filter: CharFilter,
    max_chars: usize,
}

impl TextField {
    /// An empty field.
    pub fn new(filter: CharFilter, max_chars: usize) -> Self {
        Self {
            text: String::new(),
            caret: 0,
            anchor: 0,
            filter,
            max_chars: max_chars.max(1),
        }
    }

    /// A field holding `text` (filtered), with the caret at the end.
    pub fn with_text(filter: CharFilter, max_chars: usize, text: &str) -> Self {
        let mut field = Self::new(filter, max_chars);
        field.set_text(text);
        field
    }

    /// The current text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The caret position in bytes.
    pub fn caret(&self) -> usize {
        self.caret
    }

    /// The selected byte range (empty when nothing is selected).
    pub fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    /// True when some text is selected.
    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    /// The selected text.
    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Replace the text (filtered) and put the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        self.text.clear();
        self.caret = 0;
        self.anchor = 0;
        self.insert(text);
    }

    /// Put the caret at byte `pos` (snapped to a character boundary). With
    /// `extend` the selection grows from the anchor.
    pub fn set_caret(&mut self, pos: usize, extend: bool) {
        let mut pos = pos.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        self.caret = pos;
        if !extend {
            self.anchor = pos;
        }
    }

    /// Select the whole text.
    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.text.len();
    }

    fn accepts(&self, c: char, decimal_seen: bool) -> bool {
        if c.is_control() {
            return false;
        }
        match self.filter {
            CharFilter::Digits => c.is_ascii_digit(),
            CharFilter::Decimal => c.is_ascii_digit() || ((c == '.' || c == ',') && !decimal_seen),
            CharFilter::Any => true,
        }
    }

    fn delete_selection(&mut self) -> bool {
        if !self.has_selection() {
            return false;
        }
        let range = self.selection();
        self.text.replace_range(range.clone(), "");
        self.caret = range.start;
        self.anchor = range.start;
        true
    }

    /// Type text at the caret, replacing the selection. Rejected
    /// characters are dropped. Returns true when the text changed.
    pub fn insert(&mut self, input: &str) -> bool {
        let mut changed = self.delete_selection();
        let mut decimal_seen = self.text.contains('.');
        let mut count = self.text.chars().count();
        for c in input.chars() {
            if count >= self.max_chars {
                break;
            }
            if !self.accepts(c, decimal_seen) {
                continue;
            }
            let c = if c == ',' && self.filter == CharFilter::Decimal {
                '.'
            } else {
                c
            };
            decimal_seen |= c == '.';
            self.text.insert(self.caret, c);
            self.caret += c.len_utf8();
            count += 1;
            changed = true;
        }
        self.anchor = self.caret;
        changed
    }

    fn prev_boundary(&self, from: usize) -> usize {
        self.text[..from]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, from: usize) -> usize {
        self.text[from..]
            .chars()
            .next()
            .map_or(from, |c| from + c.len_utf8())
    }

    fn prev_word(&self, from: usize) -> usize {
        let mut pos = from;
        // Skip separators, then the word.
        while pos > 0 {
            let p = self.prev_boundary(pos);
            if self.text[p..pos].chars().all(char::is_alphanumeric) {
                break;
            }
            pos = p;
        }
        while pos > 0 {
            let p = self.prev_boundary(pos);
            if !self.text[p..pos].chars().all(char::is_alphanumeric) {
                break;
            }
            pos = p;
        }
        pos
    }

    fn next_word(&self, from: usize) -> usize {
        let mut pos = from;
        let len = self.text.len();
        while pos < len {
            let n = self.next_boundary(pos);
            if !self.text[pos..n].chars().all(char::is_alphanumeric) {
                break;
            }
            pos = n;
        }
        while pos < len {
            let n = self.next_boundary(pos);
            if self.text[pos..n].chars().all(char::is_alphanumeric) {
                break;
            }
            pos = n;
        }
        pos
    }

    /// Apply an editing key: Left, Right, Home, End (Shift extends, Ctrl
    /// moves by word), Backspace and Delete (Ctrl removes a word), Ctrl+A,
    /// Ctrl+C, Ctrl+X and Ctrl+V.
    pub fn handle_key(&mut self, key: Key, mods: Modifiers) -> FieldResponse {
        let extend = mods.shift;
        match key {
            Key::Left => {
                let target = if self.has_selection() && !extend {
                    self.selection().start
                } else if mods.ctrl {
                    self.prev_word(self.caret)
                } else {
                    self.prev_boundary(self.caret)
                };
                self.set_caret(target, extend);
                FieldResponse::Handled
            }
            Key::Right => {
                let target = if self.has_selection() && !extend {
                    self.selection().end
                } else if mods.ctrl {
                    self.next_word(self.caret)
                } else {
                    self.next_boundary(self.caret)
                };
                self.set_caret(target, extend);
                FieldResponse::Handled
            }
            Key::Home => {
                self.set_caret(0, extend);
                FieldResponse::Handled
            }
            Key::End => {
                self.set_caret(self.text.len(), extend);
                FieldResponse::Handled
            }
            Key::Backspace => {
                if self.delete_selection() {
                    return FieldResponse::Changed;
                }
                if self.caret == 0 {
                    return FieldResponse::Handled;
                }
                let start = if mods.ctrl {
                    self.prev_word(self.caret)
                } else {
                    self.prev_boundary(self.caret)
                };
                self.text.replace_range(start..self.caret, "");
                self.caret = start;
                self.anchor = start;
                FieldResponse::Changed
            }
            Key::Delete => {
                if self.delete_selection() {
                    return FieldResponse::Changed;
                }
                if self.caret >= self.text.len() {
                    return FieldResponse::Handled;
                }
                let end = if mods.ctrl {
                    self.next_word(self.caret)
                } else {
                    self.next_boundary(self.caret)
                };
                self.text.replace_range(self.caret..end, "");
                self.anchor = self.caret;
                FieldResponse::Changed
            }
            Key::A if mods.ctrl => {
                self.select_all();
                FieldResponse::Handled
            }
            Key::C if mods.ctrl => {
                if self.has_selection() {
                    FieldResponse::Copy {
                        text: self.selected_text().to_string(),
                        cut: false,
                    }
                } else {
                    FieldResponse::Handled
                }
            }
            Key::X if mods.ctrl => {
                if self.has_selection() {
                    let text = self.selected_text().to_string();
                    self.delete_selection();
                    FieldResponse::Copy { text, cut: true }
                } else {
                    FieldResponse::Handled
                }
            }
            Key::V if mods.ctrl => FieldResponse::Paste,
            _ => FieldResponse::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_filters_and_limits() {
        let mut f = TextField::new(CharFilter::Digits, 5);
        assert!(f.insert("12a3"));
        assert_eq!(f.text(), "123");
        f.insert("4567");
        assert_eq!(f.text(), "12345");
        let mut d = TextField::new(CharFilter::Decimal, 8);
        d.insert("12,5.5");
        assert_eq!(d.text(), "12.55");
        let mut any = TextField::new(CharFilter::Any, 40);
        any.insert("C:\\Fotos\u{e9}\u{8}\r");
        assert_eq!(any.text(), "C:\\Fotos\u{e9}");
    }

    #[test]
    fn caret_moves_on_char_boundaries_and_edits() {
        let mut f = TextField::with_text(CharFilter::Any, 40, "a\u{e9}b");
        assert_eq!(f.caret(), 4);
        f.handle_key(Key::Left, Modifiers::NONE);
        assert_eq!(f.caret(), 3);
        f.handle_key(Key::Left, Modifiers::NONE);
        assert_eq!(f.caret(), 1);
        assert_eq!(
            f.handle_key(Key::Delete, Modifiers::NONE),
            FieldResponse::Changed
        );
        assert_eq!(f.text(), "ab");
        f.handle_key(Key::End, Modifiers::NONE);
        f.handle_key(Key::Backspace, Modifiers::NONE);
        assert_eq!(f.text(), "a");
    }

    #[test]
    fn selection_copy_cut_paste_and_words() {
        let mut f = TextField::with_text(CharFilter::Any, 40, "hello big world");
        f.handle_key(Key::Left, Modifiers::CTRL_SHIFT);
        assert_eq!(f.selected_text(), "world");
        assert_eq!(
            f.handle_key(Key::X, Modifiers::CTRL),
            FieldResponse::Copy {
                text: "world".into(),
                cut: true
            }
        );
        assert_eq!(f.text(), "hello big ");
        f.handle_key(Key::Backspace, Modifiers::CTRL);
        assert_eq!(f.text(), "hello ");
        assert_eq!(f.handle_key(Key::V, Modifiers::CTRL), FieldResponse::Paste);
        f.handle_key(Key::A, Modifiers::CTRL);
        f.insert("x");
        assert_eq!(f.text(), "x");
        assert_eq!(
            f.handle_key(Key::F5, Modifiers::NONE),
            FieldResponse::Ignored
        );
    }
}

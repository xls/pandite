//! Input events the application feeds into the interface.
//!
//! The application translates its window system events (winit) into these
//! plain values. Positions are physical pixels with the origin at the
//! window's top-left corner, the same space the [`pandite_core::DrawList`]
//! uses.

use pandite_core::Point;

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointerButton {
    /// The primary button.
    Left,
    /// The secondary button. Opens the context menu.
    Right,
    /// The wheel button. Drags pan the image in every mode.
    Middle,
    /// The "back" side button. Shows the previous image.
    Back,
    /// The "forward" side button. Shows the next image.
    Forward,
}

/// Keyboard modifier state.
///
/// `ctrl` is the platform command modifier: the application sets it for
/// Control on Windows and Linux and for Command on macOS.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers {
    /// Control (Command on macOS).
    pub ctrl: bool,
    /// Shift.
    pub shift: bool,
    /// Alt (Option on macOS).
    pub alt: bool,
}

impl Modifiers {
    /// No modifier held.
    pub const NONE: Modifiers = Modifiers {
        ctrl: false,
        shift: false,
        alt: false,
    };
    /// Control only.
    pub const CTRL: Modifiers = Modifiers {
        ctrl: true,
        shift: false,
        alt: false,
    };
    /// Shift only.
    pub const SHIFT: Modifiers = Modifiers {
        ctrl: false,
        shift: true,
        alt: false,
    };
    /// Alt only.
    pub const ALT: Modifiers = Modifiers {
        ctrl: false,
        shift: false,
        alt: true,
    };
    /// Control and Shift.
    pub const CTRL_SHIFT: Modifiers = Modifiers {
        ctrl: true,
        shift: true,
        alt: false,
    };
}

/// A key, named by what it means rather than where it sits.
///
/// The application maps the logical key of its keyboard event. Character
/// keys go through [`Key::from_char`], so `'+'` and the keypad plus both
/// become [`Key::Plus`] and `'R'` and `'r'` both become [`Key::R`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// The A key.
    A,
    /// The B key.
    B,
    /// The C key.
    C,
    /// The D key.
    D,
    /// The E key.
    E,
    /// The F key.
    F,
    /// The G key.
    G,
    /// The H key.
    H,
    /// The I key.
    I,
    /// The J key.
    J,
    /// The K key.
    K,
    /// The L key.
    L,
    /// The M key.
    M,
    /// The N key.
    N,
    /// The O key.
    O,
    /// The P key.
    P,
    /// The Q key.
    Q,
    /// The R key.
    R,
    /// The S key.
    S,
    /// The T key.
    T,
    /// The U key.
    U,
    /// The V key.
    V,
    /// The W key.
    W,
    /// The X key.
    X,
    /// The Y key.
    Y,
    /// The Z key.
    Z,
    /// The 0 key on the main keyboard or keypad.
    Digit0,
    /// The 1 key on the main keyboard or keypad.
    Digit1,
    /// The 2 key on the main keyboard or keypad.
    Digit2,
    /// The 3 key on the main keyboard or keypad.
    Digit3,
    /// The 4 key on the main keyboard or keypad.
    Digit4,
    /// The 5 key on the main keyboard or keypad.
    Digit5,
    /// The 6 key on the main keyboard or keypad.
    Digit6,
    /// The 7 key on the main keyboard or keypad.
    Digit7,
    /// The 8 key on the main keyboard or keypad.
    Digit8,
    /// The 9 key on the main keyboard or keypad.
    Digit9,
    /// The F1 function key.
    F1,
    /// The F2 function key.
    F2,
    /// The F3 function key.
    F3,
    /// The F4 function key.
    F4,
    /// The F5 function key.
    F5,
    /// The F6 function key.
    F6,
    /// The F7 function key.
    F7,
    /// The F8 function key.
    F8,
    /// The F9 function key.
    F9,
    /// The F10 function key.
    F10,
    /// The F11 function key.
    F11,
    /// The F12 function key.
    F12,
    /// The left arrow key.
    Left,
    /// The right arrow key.
    Right,
    /// The up arrow key.
    Up,
    /// The down arrow key.
    Down,
    /// The Home key.
    Home,
    /// The End key.
    End,
    /// The Page Up key.
    PageUp,
    /// The Page Down key.
    PageDown,
    /// The Enter or Return key.
    Enter,
    /// The Escape key.
    Escape,
    /// The Tab key.
    Tab,
    /// The Backspace key.
    Backspace,
    /// The Delete key.
    Delete,
    /// The Insert key.
    Insert,
    /// The space bar.
    Space,
    /// The plus sign, from the main keyboard or the keypad.
    Plus,
    /// The minus sign, from the main keyboard or the keypad.
    Minus,
    /// The equals sign (unshifted plus on many layouts).
    Equals,
    /// The question mark, wherever the layout puts it.
    Question,
    /// The context menu key.
    ContextMenu,
    /// Any key the interface has no use for.
    Other,
}

impl Key {
    /// Map a typed character to a key. Letters ignore case. Returns `None`
    /// for characters with no key of their own.
    pub fn from_char(c: char) -> Option<Key> {
        const LETTERS: [Key; 26] = [
            Key::A,
            Key::B,
            Key::C,
            Key::D,
            Key::E,
            Key::F,
            Key::G,
            Key::H,
            Key::I,
            Key::J,
            Key::K,
            Key::L,
            Key::M,
            Key::N,
            Key::O,
            Key::P,
            Key::Q,
            Key::R,
            Key::S,
            Key::T,
            Key::U,
            Key::V,
            Key::W,
            Key::X,
            Key::Y,
            Key::Z,
        ];
        const DIGITS: [Key; 10] = [
            Key::Digit0,
            Key::Digit1,
            Key::Digit2,
            Key::Digit3,
            Key::Digit4,
            Key::Digit5,
            Key::Digit6,
            Key::Digit7,
            Key::Digit8,
            Key::Digit9,
        ];
        match c {
            'a'..='z' => Some(LETTERS[(c as u8 - b'a') as usize]),
            'A'..='Z' => Some(LETTERS[(c as u8 - b'A') as usize]),
            '0'..='9' => Some(DIGITS[(c as u8 - b'0') as usize]),
            '+' => Some(Key::Plus),
            '-' => Some(Key::Minus),
            '=' => Some(Key::Equals),
            '?' => Some(Key::Question),
            ' ' => Some(Key::Space),
            _ => None,
        }
    }

    /// True for keys that type a symbol or digit. Their shortcuts ignore
    /// Shift, because many layouts need Shift to reach them.
    pub fn is_symbol(self) -> bool {
        matches!(
            self,
            Key::Plus
                | Key::Minus
                | Key::Equals
                | Key::Question
                | Key::Digit0
                | Key::Digit1
                | Key::Digit2
                | Key::Digit3
                | Key::Digit4
                | Key::Digit5
                | Key::Digit6
                | Key::Digit7
                | Key::Digit8
                | Key::Digit9
        )
    }

    /// The short name shown in tooltips, menus and the shortcut overlay.
    pub fn label(self) -> &'static str {
        match self {
            Key::A => "A",
            Key::B => "B",
            Key::C => "C",
            Key::D => "D",
            Key::E => "E",
            Key::F => "F",
            Key::G => "G",
            Key::H => "H",
            Key::I => "I",
            Key::J => "J",
            Key::K => "K",
            Key::L => "L",
            Key::M => "M",
            Key::N => "N",
            Key::O => "O",
            Key::P => "P",
            Key::Q => "Q",
            Key::R => "R",
            Key::S => "S",
            Key::T => "T",
            Key::U => "U",
            Key::V => "V",
            Key::W => "W",
            Key::X => "X",
            Key::Y => "Y",
            Key::Z => "Z",
            Key::Digit0 => "0",
            Key::Digit1 => "1",
            Key::Digit2 => "2",
            Key::Digit3 => "3",
            Key::Digit4 => "4",
            Key::Digit5 => "5",
            Key::Digit6 => "6",
            Key::Digit7 => "7",
            Key::Digit8 => "8",
            Key::Digit9 => "9",
            Key::F1 => "F1",
            Key::F2 => "F2",
            Key::F3 => "F3",
            Key::F4 => "F4",
            Key::F5 => "F5",
            Key::F6 => "F6",
            Key::F7 => "F7",
            Key::F8 => "F8",
            Key::F9 => "F9",
            Key::F10 => "F10",
            Key::F11 => "F11",
            Key::F12 => "F12",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Up => "Up",
            Key::Down => "Down",
            Key::Home => "Home",
            Key::End => "End",
            Key::PageUp => "PgUp",
            Key::PageDown => "PgDn",
            Key::Enter => "Enter",
            Key::Escape => "Esc",
            Key::Tab => "Tab",
            Key::Backspace => "Backspace",
            Key::Delete => "Del",
            Key::Insert => "Ins",
            Key::Space => "Space",
            Key::Plus => "+",
            Key::Minus => "-",
            Key::Equals => "=",
            Key::Question => "?",
            Key::ContextMenu => "Menu",
            Key::Other => "?",
        }
    }
}

/// How far a wheel or touchpad scrolled.
///
/// Signs follow winit: positive values move the content right and down, so
/// rolling the wheel away from the user gives a positive `y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WheelDelta {
    /// Wheel notches (lines).
    Lines {
        /// Horizontal notches.
        x: f32,
        /// Vertical notches.
        y: f32,
    },
    /// Precise touchpad movement in physical pixels.
    Pixels {
        /// Horizontal pixels.
        x: f32,
        /// Vertical pixels.
        y: f32,
    },
}

/// One input event. Positions are physical window pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    /// The pointer moved.
    PointerMove {
        /// New pointer position.
        pos: Point,
    },
    /// A mouse button went down.
    PointerDown {
        /// Pointer position.
        pos: Point,
        /// Which button.
        button: PointerButton,
    },
    /// A mouse button went up.
    PointerUp {
        /// Pointer position.
        pos: Point,
        /// Which button.
        button: PointerButton,
    },
    /// The pointer left the window.
    PointerLeft,
    /// The wheel or touchpad scrolled.
    Wheel {
        /// Pointer position.
        pos: Point,
        /// Scroll amount.
        delta: WheelDelta,
    },
    /// A key went down.
    KeyDown {
        /// The key.
        key: Key,
        /// Modifiers held with it.
        mods: Modifiers,
        /// True when the key is held and the system repeats it.
        repeat: bool,
    },
    /// A key went up.
    KeyUp {
        /// The key.
        key: Key,
        /// Modifiers held with it.
        mods: Modifiers,
    },
    /// The modifier state changed without a key event.
    ModifiersChanged(Modifiers),
    /// Text was typed or committed by an input method. Also used to deliver
    /// clipboard text after [`crate::Action::RequestPaste`].
    Text(String),
    /// The window lost keyboard focus. Drops pointer capture and held keys.
    FocusLost,
    /// Time passed. Lets animations and timers advance without input.
    Tick,
}

/// The pointer shape the interface wants at the current pointer position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The normal arrow.
    #[default]
    Default,
    /// A pointing hand over clickable items.
    Hand,
    /// A text caret over text fields.
    Text,
    /// An open hand over a pannable image.
    Grab,
    /// A closed hand while panning.
    Grabbing,
    /// A crosshair for drawing a crop selection.
    Crosshair,
    /// Four arrows for moving a crop selection.
    Move,
    /// Left-right arrows for a side handle.
    ResizeHorizontal,
    /// Up-down arrows for a top or bottom handle.
    ResizeVertical,
    /// Arrows from top-left to bottom-right.
    ResizeDiagonalDown,
    /// Arrows from bottom-left to top-right.
    ResizeDiagonalUp,
    /// No cursor (idle in full screen).
    Hidden,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_char_maps_letters_digits_and_symbols() {
        assert_eq!(Key::from_char('r'), Some(Key::R));
        assert_eq!(Key::from_char('R'), Some(Key::R));
        assert_eq!(Key::from_char('7'), Some(Key::Digit7));
        assert_eq!(Key::from_char('+'), Some(Key::Plus));
        assert_eq!(Key::from_char('-'), Some(Key::Minus));
        assert_eq!(Key::from_char('='), Some(Key::Equals));
        assert_eq!(Key::from_char('?'), Some(Key::Question));
        assert_eq!(Key::from_char('%'), None);
        assert!(Key::Plus.is_symbol());
        assert!(!Key::R.is_symbol());
    }
}

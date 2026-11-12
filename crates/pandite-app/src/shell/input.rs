//! Translate winit input into the interface's plain input events.

use pandite_ui::{Key, Modifiers, PointerButton, WheelDelta};
use winit::event::{KeyEvent, MouseButton, MouseScrollDelta};
use winit::keyboard::{Key as WKey, KeyCode, ModifiersState, NamedKey, PhysicalKey};

/// The platform command modifier is Command on macOS and Control elsewhere.
pub fn modifiers(state: ModifiersState) -> Modifiers {
    let ctrl = if cfg!(target_os = "macos") {
        state.super_key()
    } else {
        state.control_key()
    };
    Modifiers {
        ctrl,
        shift: state.shift_key(),
        alt: state.alt_key(),
    }
}

/// A mouse button, when the interface knows it.
pub fn button(button: MouseButton) -> Option<PointerButton> {
    match button {
        MouseButton::Left => Some(PointerButton::Left),
        MouseButton::Right => Some(PointerButton::Right),
        MouseButton::Middle => Some(PointerButton::Middle),
        MouseButton::Back => Some(PointerButton::Back),
        MouseButton::Forward => Some(PointerButton::Forward),
        MouseButton::Other(_) => None,
    }
}

/// A wheel or touchpad scroll.
pub fn wheel(delta: MouseScrollDelta) -> WheelDelta {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => WheelDelta::Lines { x, y },
        MouseScrollDelta::PixelDelta(p) => WheelDelta::Pixels {
            x: p.x as f32,
            y: p.y as f32,
        },
    }
}

fn named(key: NamedKey) -> Option<Key> {
    Some(match key {
        NamedKey::ArrowLeft => Key::Left,
        NamedKey::ArrowRight => Key::Right,
        NamedKey::ArrowUp => Key::Up,
        NamedKey::ArrowDown => Key::Down,
        NamedKey::Home => Key::Home,
        NamedKey::End => Key::End,
        NamedKey::PageUp => Key::PageUp,
        NamedKey::PageDown => Key::PageDown,
        NamedKey::Enter => Key::Enter,
        NamedKey::Escape => Key::Escape,
        NamedKey::Tab => Key::Tab,
        NamedKey::Backspace => Key::Backspace,
        NamedKey::Delete => Key::Delete,
        NamedKey::Insert => Key::Insert,
        NamedKey::Space => Key::Space,
        NamedKey::ContextMenu => Key::ContextMenu,
        NamedKey::F1 => Key::F1,
        NamedKey::F2 => Key::F2,
        NamedKey::F3 => Key::F3,
        NamedKey::F4 => Key::F4,
        NamedKey::F5 => Key::F5,
        NamedKey::F6 => Key::F6,
        NamedKey::F7 => Key::F7,
        NamedKey::F8 => Key::F8,
        NamedKey::F9 => Key::F9,
        NamedKey::F10 => Key::F10,
        NamedKey::F11 => Key::F11,
        NamedKey::F12 => Key::F12,
        _ => return None,
    })
}

/// The key a physical key position means on a US layout. Used when the
/// layout's character has no key of its own (Cyrillic, Greek, ...), so
/// shortcuts like Ctrl+O still work.
fn physical(code: KeyCode) -> Option<Key> {
    const LETTERS: [(KeyCode, char); 26] = [
        (KeyCode::KeyA, 'a'),
        (KeyCode::KeyB, 'b'),
        (KeyCode::KeyC, 'c'),
        (KeyCode::KeyD, 'd'),
        (KeyCode::KeyE, 'e'),
        (KeyCode::KeyF, 'f'),
        (KeyCode::KeyG, 'g'),
        (KeyCode::KeyH, 'h'),
        (KeyCode::KeyI, 'i'),
        (KeyCode::KeyJ, 'j'),
        (KeyCode::KeyK, 'k'),
        (KeyCode::KeyL, 'l'),
        (KeyCode::KeyM, 'm'),
        (KeyCode::KeyN, 'n'),
        (KeyCode::KeyO, 'o'),
        (KeyCode::KeyP, 'p'),
        (KeyCode::KeyQ, 'q'),
        (KeyCode::KeyR, 'r'),
        (KeyCode::KeyS, 's'),
        (KeyCode::KeyT, 't'),
        (KeyCode::KeyU, 'u'),
        (KeyCode::KeyV, 'v'),
        (KeyCode::KeyW, 'w'),
        (KeyCode::KeyX, 'x'),
        (KeyCode::KeyY, 'y'),
        (KeyCode::KeyZ, 'z'),
    ];
    if let Some((_, c)) = LETTERS.iter().find(|(k, _)| *k == code) {
        return Key::from_char(*c);
    }
    Some(match code {
        KeyCode::Digit0 | KeyCode::Numpad0 => Key::Digit0,
        KeyCode::Digit1 | KeyCode::Numpad1 => Key::Digit1,
        KeyCode::Digit2 | KeyCode::Numpad2 => Key::Digit2,
        KeyCode::Digit3 | KeyCode::Numpad3 => Key::Digit3,
        KeyCode::Digit4 | KeyCode::Numpad4 => Key::Digit4,
        KeyCode::Digit5 | KeyCode::Numpad5 => Key::Digit5,
        KeyCode::Digit6 | KeyCode::Numpad6 => Key::Digit6,
        KeyCode::Digit7 | KeyCode::Numpad7 => Key::Digit7,
        KeyCode::Digit8 | KeyCode::Numpad8 => Key::Digit8,
        KeyCode::Digit9 | KeyCode::Numpad9 => Key::Digit9,
        KeyCode::NumpadAdd => Key::Plus,
        KeyCode::Minus | KeyCode::NumpadSubtract => Key::Minus,
        KeyCode::Equal => Key::Equals,
        KeyCode::NumpadEnter => Key::Enter,
        _ => return None,
    })
}

/// The interface key for a keyboard event: the layout's meaning first,
/// the key position as a fallback.
pub fn key(event: &KeyEvent) -> Key {
    let logical = match &event.logical_key {
        WKey::Named(n) => named(*n),
        WKey::Character(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Key::from_char(c),
                _ => None,
            }
        }
        _ => None,
    };
    let fallback = || match event.physical_key {
        PhysicalKey::Code(code) => physical(code),
        PhysicalKey::Unidentified(_) => None,
    };
    logical.or_else(fallback).unwrap_or(Key::Other)
}

/// Typed text from a key press, without control characters.
pub fn typed_text(event: &KeyEvent) -> Option<String> {
    let text = event.text.as_ref()?;
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    (!clean.is_empty()).then_some(clean)
}

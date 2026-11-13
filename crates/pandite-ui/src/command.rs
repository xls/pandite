//! User commands and the keyboard shortcut table.
//!
//! A [`Command`] is one thing the user can trigger from the toolbar, the
//! context menu or the keyboard. The interface turns a command into
//! [`crate::Action`]s, taking the current mode and context into account.
//! [`SHORTCUTS`] is the single keyboard table; tooltips, menus and the
//! shortcut overlay read their key labels from it.

use crate::icon::Icon;
use crate::input::{Key, Modifiers};

/// A user command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// Open a file.
    Open,
    /// Save the edited image under a new name or format.
    SaveAs,
    /// Copy the current file path.
    CopyPath,
    /// Undo the last view edit.
    Undo,
    /// Rotate 90 degrees counterclockwise.
    RotateLeft,
    /// Rotate 90 degrees clockwise.
    RotateRight,
    /// Mirror left to right.
    FlipHorizontal,
    /// Mirror top to bottom.
    FlipVertical,
    /// Enter crop mode.
    Crop,
    /// Open the resize dialog.
    Resize,
    /// Open the bulk convert and resize dialog.
    BulkResize,
    /// Best fit in the window.
    Fit,
    /// One image pixel per screen pixel.
    ActualSize,
    /// Zoom in one step.
    ZoomIn,
    /// Zoom out one step.
    ZoomOut,
    /// Move the view up.
    PanUp,
    /// Move the view down.
    PanDown,
    /// Move the view left.
    PanLeft,
    /// Move the view right.
    PanRight,
    /// Previous image.
    Previous,
    /// Next image.
    Next,
    /// First image in the folder.
    First,
    /// Last image in the folder.
    Last,
    /// Show or hide the gallery strip.
    ToggleGallery,
    /// Enter or leave full screen.
    Fullscreen,
    /// Open the context menu from the keyboard.
    ContextMenu,
    /// Show the keyboard shortcut overlay.
    Help,
    /// Escape: cancel loading, close the topmost menu, dialog or mode, or
    /// leave full screen.
    Cancel,
    /// Close the application.
    Quit,
    /// Crop mode: apply the selection.
    CropApply,
    /// Crop mode: select the whole image.
    CropSelectAll,
    /// Crop mode: leave without cropping.
    CropCancel,
    /// Select all text recognized in the image.
    SelectAllText,
    /// Copy the selected text.
    CopyText,
}

impl Command {
    /// Every command, in the order the shortcut overlay lists them.
    pub const ALL: [Command; 34] = [
        Command::Open,
        Command::SaveAs,
        Command::CopyPath,
        Command::CopyText,
        Command::SelectAllText,
        Command::Undo,
        Command::RotateLeft,
        Command::RotateRight,
        Command::FlipHorizontal,
        Command::FlipVertical,
        Command::Crop,
        Command::Resize,
        Command::BulkResize,
        Command::Fit,
        Command::ActualSize,
        Command::ZoomIn,
        Command::ZoomOut,
        Command::PanUp,
        Command::PanDown,
        Command::PanLeft,
        Command::PanRight,
        Command::Previous,
        Command::Next,
        Command::First,
        Command::Last,
        Command::ToggleGallery,
        Command::Fullscreen,
        Command::ContextMenu,
        Command::Help,
        Command::Cancel,
        Command::Quit,
        Command::CropApply,
        Command::CropSelectAll,
        Command::CropCancel,
    ];

    /// The menu label.
    pub fn label(self) -> &'static str {
        match self {
            Command::Open => "Open...",
            Command::SaveAs => "Save as...",
            Command::CopyPath => "Copy file path",
            Command::Undo => "Undo",
            Command::RotateLeft => "Rotate left",
            Command::RotateRight => "Rotate right",
            Command::FlipHorizontal => "Flip horizontal",
            Command::FlipVertical => "Flip vertical",
            Command::Crop => "Crop",
            Command::Resize => "Resize...",
            Command::BulkResize => "Bulk convert and resize...",
            Command::Fit => "Fit to window",
            Command::ActualSize => "Actual size (1:1)",
            Command::ZoomIn => "Zoom in",
            Command::ZoomOut => "Zoom out",
            Command::PanUp => "Pan up",
            Command::PanDown => "Pan down",
            Command::PanLeft => "Pan left",
            Command::PanRight => "Pan right",
            Command::Previous => "Previous image",
            Command::Next => "Next image",
            Command::First => "First image",
            Command::Last => "Last image",
            Command::ToggleGallery => "Gallery strip",
            Command::Fullscreen => "Full screen",
            Command::ContextMenu => "Context menu",
            Command::Help => "Keyboard shortcuts",
            Command::Cancel => "Cancel or leave full screen",
            Command::Quit => "Quit",
            Command::CropApply => "Apply crop",
            Command::CropSelectAll => "Select whole image",
            Command::CropCancel => "Cancel crop",
            Command::SelectAllText => "Select all text",
            Command::CopyText => "Copy selected text",
        }
    }

    /// The toolbar icon, when the command has a button.
    pub fn icon(self) -> Option<Icon> {
        Some(match self {
            Command::Open => Icon::Open,
            Command::SaveAs => Icon::SaveAs,
            Command::CopyPath => Icon::CopyPath,
            Command::Undo => Icon::Undo,
            Command::RotateLeft => Icon::RotateLeft,
            Command::RotateRight => Icon::RotateRight,
            Command::FlipHorizontal => Icon::FlipHorizontal,
            Command::FlipVertical => Icon::FlipVertical,
            Command::Crop => Icon::Crop,
            Command::Resize => Icon::Resize,
            Command::BulkResize => Icon::BulkResize,
            Command::Fit => Icon::Fit,
            Command::ActualSize => Icon::ActualSize,
            Command::ZoomIn => Icon::ZoomIn,
            Command::ZoomOut => Icon::ZoomOut,
            Command::Previous => Icon::Previous,
            Command::Next => Icon::Next,
            Command::ToggleGallery => Icon::Gallery,
            Command::Fullscreen => Icon::Fullscreen,
            Command::CropApply => Icon::Check,
            Command::CropCancel => Icon::Close,
            _ => return None,
        })
    }

    /// True when the command needs an open image.
    pub fn needs_image(self) -> bool {
        matches!(
            self,
            Command::SaveAs
                | Command::CopyPath
                | Command::Undo
                | Command::RotateLeft
                | Command::RotateRight
                | Command::FlipHorizontal
                | Command::FlipVertical
                | Command::Crop
                | Command::Resize
                | Command::Fit
                | Command::ActualSize
                | Command::ZoomIn
                | Command::ZoomOut
                | Command::PanUp
                | Command::PanDown
                | Command::PanLeft
                | Command::PanRight
                | Command::CropApply
                | Command::CropSelectAll
                | Command::SelectAllText
                | Command::CopyText
        )
    }

    /// True for the commands that act on recognized text. They exist only
    /// where text recognition is on.
    pub fn is_text(self) -> bool {
        matches!(self, Command::SelectAllText | Command::CopyText)
    }

    /// True when holding the key repeats the command.
    pub fn repeats(self) -> bool {
        matches!(
            self,
            Command::Next
                | Command::Previous
                | Command::ZoomIn
                | Command::ZoomOut
                | Command::PanUp
                | Command::PanDown
                | Command::PanLeft
                | Command::PanRight
        )
    }

    /// True when the command stays available in crop mode. Commands that
    /// change the image or leave it are blocked until the crop ends.
    pub fn allowed_in_crop(self) -> bool {
        matches!(
            self,
            Command::Fit
                | Command::ActualSize
                | Command::ZoomIn
                | Command::ZoomOut
                | Command::PanUp
                | Command::PanDown
                | Command::PanLeft
                | Command::PanRight
                | Command::Fullscreen
                | Command::ToggleGallery
                | Command::ContextMenu
                | Command::Help
                | Command::Cancel
                | Command::Quit
                | Command::CropApply
                | Command::CropSelectAll
                | Command::CropCancel
        )
    }

    /// True when the command only means something in crop mode.
    pub fn crop_only(self) -> bool {
        matches!(
            self,
            Command::CropApply | Command::CropSelectAll | Command::CropCancel
        )
    }

    /// The label of the first shortcut bound to this command, for example
    /// `"Ctrl+Shift+S"`.
    pub fn shortcut_label(self) -> Option<String> {
        SHORTCUTS
            .iter()
            .find(|s| s.command == self)
            .map(Shortcut::label)
    }

    /// The tooltip text: the label plus the first shortcut.
    pub fn tooltip(self) -> String {
        let label = self.label().trim_end_matches("...");
        match self.shortcut_label() {
            Some(keys) => format!("{label}  ({keys})"),
            None => label.to_string(),
        }
    }
}

/// One keyboard binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shortcut {
    /// The key.
    pub key: Key,
    /// The modifiers that must be held. For symbol keys
    /// ([`Key::is_symbol`]) Shift is ignored when matching.
    pub mods: Modifiers,
    /// The command it runs.
    pub command: Command,
}

impl Shortcut {
    const fn new(key: Key, mods: Modifiers, command: Command) -> Self {
        Self { key, mods, command }
    }

    /// True when a key press with `mods` triggers this binding.
    pub fn matches(&self, key: Key, mods: Modifiers) -> bool {
        if self.key != key || self.mods.ctrl != mods.ctrl || self.mods.alt != mods.alt {
            return false;
        }
        key.is_symbol() || self.mods.shift == mods.shift
    }

    /// The printable form, for example `"Ctrl+Shift+S"`.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.mods.ctrl {
            s.push_str("Ctrl+");
        }
        if self.mods.alt {
            s.push_str("Alt+");
        }
        if self.mods.shift {
            s.push_str("Shift+");
        }
        s.push_str(self.key.label());
        s
    }
}

const N: Modifiers = Modifiers::NONE;
const CTRL: Modifiers = Modifiers::CTRL;
const SHIFT: Modifiers = Modifiers::SHIFT;
const ALT: Modifiers = Modifiers::ALT;
const CTRL_SHIFT: Modifiers = Modifiers::CTRL_SHIFT;

/// The global keyboard table. The first binding of a command is the one
/// shown in tooltips and menus. Dialogs and text fields handle their own
/// keys (Enter, Escape, Tab, editing keys) before this table is consulted.
/// Ctrl+C is listed for both copy commands: [`command_for_key`] returns
/// [`Command::CopyPath`], and the interface copies the selected text
/// instead while a text selection exists.
pub const SHORTCUTS: &[Shortcut] = &[
    Shortcut::new(Key::O, CTRL, Command::Open),
    Shortcut::new(Key::S, CTRL_SHIFT, Command::SaveAs),
    Shortcut::new(Key::S, CTRL, Command::SaveAs),
    Shortcut::new(Key::C, CTRL, Command::CopyPath),
    Shortcut::new(Key::C, CTRL, Command::CopyText),
    Shortcut::new(Key::A, CTRL, Command::SelectAllText),
    Shortcut::new(Key::Z, CTRL, Command::Undo),
    Shortcut::new(Key::R, SHIFT, Command::RotateLeft),
    Shortcut::new(Key::L, N, Command::RotateLeft),
    Shortcut::new(Key::R, N, Command::RotateRight),
    Shortcut::new(Key::H, N, Command::FlipHorizontal),
    Shortcut::new(Key::V, N, Command::FlipVertical),
    Shortcut::new(Key::C, N, Command::Crop),
    Shortcut::new(Key::R, CTRL, Command::Resize),
    Shortcut::new(Key::B, CTRL, Command::BulkResize),
    Shortcut::new(Key::F, N, Command::Fit),
    Shortcut::new(Key::Digit0, N, Command::Fit),
    Shortcut::new(Key::Digit1, N, Command::ActualSize),
    Shortcut::new(Key::Plus, N, Command::ZoomIn),
    Shortcut::new(Key::Equals, N, Command::ZoomIn),
    Shortcut::new(Key::Minus, N, Command::ZoomOut),
    Shortcut::new(Key::Up, N, Command::PanUp),
    Shortcut::new(Key::Up, CTRL, Command::PanUp),
    Shortcut::new(Key::Down, N, Command::PanDown),
    Shortcut::new(Key::Down, CTRL, Command::PanDown),
    Shortcut::new(Key::Left, CTRL, Command::PanLeft),
    Shortcut::new(Key::Right, CTRL, Command::PanRight),
    Shortcut::new(Key::Left, N, Command::Previous),
    Shortcut::new(Key::PageUp, N, Command::Previous),
    Shortcut::new(Key::Backspace, N, Command::Previous),
    Shortcut::new(Key::Right, N, Command::Next),
    Shortcut::new(Key::PageDown, N, Command::Next),
    Shortcut::new(Key::Space, N, Command::Next),
    Shortcut::new(Key::Home, N, Command::First),
    Shortcut::new(Key::End, N, Command::Last),
    Shortcut::new(Key::G, N, Command::ToggleGallery),
    Shortcut::new(Key::F11, N, Command::Fullscreen),
    Shortcut::new(Key::Enter, ALT, Command::Fullscreen),
    Shortcut::new(Key::ContextMenu, N, Command::ContextMenu),
    Shortcut::new(Key::F10, SHIFT, Command::ContextMenu),
    Shortcut::new(Key::F1, N, Command::Help),
    Shortcut::new(Key::Escape, N, Command::Cancel),
    Shortcut::new(Key::Q, CTRL, Command::Quit),
    Shortcut::new(Key::W, CTRL, Command::Quit),
    Shortcut::new(Key::Enter, N, Command::CropApply),
    Shortcut::new(Key::A, CTRL, Command::CropSelectAll),
    Shortcut::new(Key::Escape, N, Command::CropCancel),
];

/// The command a key press runs in the given mode, if any. In crop mode
/// crop bindings win (Enter applies, Escape cancels); outside crop mode
/// they are skipped.
pub fn command_for_key(key: Key, mods: Modifiers, cropping: bool) -> Option<Command> {
    let mut found = None;
    for s in SHORTCUTS.iter().filter(|s| s.matches(key, mods)) {
        if s.command.crop_only() {
            if cropping {
                return Some(s.command);
            }
            continue;
        }
        if found.is_none() {
            found = Some(s.command);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_keys_ignore_shift_letters_do_not() {
        assert_eq!(
            command_for_key(Key::Plus, Modifiers::SHIFT, false),
            Some(Command::ZoomIn)
        );
        assert_eq!(
            command_for_key(Key::R, Modifiers::SHIFT, false),
            Some(Command::RotateLeft)
        );
        assert_eq!(
            command_for_key(Key::R, Modifiers::NONE, false),
            Some(Command::RotateRight)
        );
        assert_eq!(
            command_for_key(Key::R, Modifiers::CTRL, false),
            Some(Command::Resize)
        );
        assert_eq!(command_for_key(Key::Enter, Modifiers::NONE, false), None);
        assert_eq!(
            command_for_key(Key::Enter, Modifiers::NONE, true),
            Some(Command::CropApply)
        );
        assert_eq!(
            command_for_key(Key::Escape, Modifiers::NONE, true),
            Some(Command::CropCancel)
        );
        assert_eq!(
            command_for_key(Key::Escape, Modifiers::NONE, false),
            Some(Command::Cancel)
        );
        // Ctrl+A selects text outside crop mode and the crop inside it;
        // Ctrl+C resolves to the path copy, which a text selection
        // overrides in the interface.
        assert_eq!(
            command_for_key(Key::A, Modifiers::CTRL, false),
            Some(Command::SelectAllText)
        );
        assert_eq!(
            command_for_key(Key::A, Modifiers::CTRL, true),
            Some(Command::CropSelectAll)
        );
        assert_eq!(
            command_for_key(Key::C, Modifiers::CTRL, false),
            Some(Command::CopyPath)
        );
        assert_eq!(
            Command::CopyText.shortcut_label().as_deref(),
            Some("Ctrl+C")
        );
    }

    #[test]
    fn labels_and_tooltips() {
        assert_eq!(
            Command::SaveAs.shortcut_label().as_deref(),
            Some("Ctrl+Shift+S")
        );
        assert_eq!(Command::Fullscreen.tooltip(), "Full screen  (F11)");
        assert_eq!(Command::Open.tooltip(), "Open  (Ctrl+O)");
    }
}

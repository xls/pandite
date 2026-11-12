//! The outside world as the application core sees it.
//!
//! The core never calls a window system, clipboard or file dialog directly.
//! It goes through these traits; the shell implements them with winit, the
//! system clipboard and native dialogs, and tests implement them with
//! recorders.

use pandite_core::Rect;
use pandite_ui::CursorShape;
use std::path::PathBuf;
use std::sync::Arc;

/// Wakes the event loop from any thread. The core hands clones to its
/// workers; after a worker queues a result it calls the waker.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Text on the system clipboard.
pub trait Clipboard {
    /// Put `text` on the clipboard.
    fn set_text(&mut self, text: &str) -> Result<(), String>;
    /// Read text from the clipboard.
    fn get_text(&mut self) -> Result<String, String>;
}

/// A file type filter for a dialog: a name and its extensions (no dots).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// Shown in the dialog, for example "PNG image".
    pub name: String,
    /// Extensions without the dot, for example `["jpg", "jpeg"]`.
    pub extensions: Vec<String>,
}

/// Which dialog to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DialogKind {
    /// Choose one image to open.
    OpenFile {
        /// Folder to start in.
        start_dir: Option<PathBuf>,
        /// Offered file types.
        filters: Vec<Filter>,
    },
    /// Choose several images.
    OpenFiles {
        /// Folder to start in.
        start_dir: Option<PathBuf>,
        /// Offered file types.
        filters: Vec<Filter>,
    },
    /// Choose a folder.
    PickFolder {
        /// Folder to start in.
        start_dir: Option<PathBuf>,
        /// Dialog title.
        title: String,
    },
    /// Choose where to save.
    SaveFile {
        /// Folder to start in.
        start_dir: Option<PathBuf>,
        /// Suggested file name.
        file_name: String,
        /// Offered file types; the first is the default.
        filters: Vec<Filter>,
    },
}

/// The answer to a dialog: the chosen paths, empty when cancelled.
pub type DialogReply = Box<dyn FnOnce(Vec<PathBuf>) + Send>;

/// Native file dialogs. Implementations must not block the caller: show
/// the dialog on another thread (or answer at once) and call `reply` with
/// the result.
pub trait Dialogs {
    /// Show `kind` and answer through `reply`.
    fn show(&mut self, kind: DialogKind, reply: DialogReply);
}

/// The application window.
pub trait WindowControl {
    /// Schedule a frame.
    fn request_redraw(&self);
    /// Set the title bar text.
    fn set_title(&self, title: &str);
    /// Enter (borderless, on the current monitor) or leave full screen.
    fn set_fullscreen(&self, fullscreen: bool);
    /// Set the pointer shape.
    fn set_cursor(&self, cursor: CursorShape);
    /// Turn the input method on while a text field has focus.
    fn set_ime_allowed(&self, allowed: bool);
    /// Move and size the window: `rect` is the client area in physical
    /// pixels of the virtual desktop.
    fn set_client_rect(&self, rect: Rect);
    /// The work area (without task bars) of the monitor the window is on,
    /// in physical pixels, when known.
    fn work_area(&self) -> Option<Rect>;
}

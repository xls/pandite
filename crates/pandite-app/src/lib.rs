//! pandite-app: the pandite image viewer.
//!
//! The crate is a library plus the `pandite` binary. The library holds a
//! testable core and a thin window shell:
//!
//! - [`app::App`]: the whole application as a state machine. It owns the
//!   interface ([`pandite_ui::Ui`]), the folder [`pandite_library::Library`],
//!   the edit stack, settings and background jobs. It reaches the outside
//!   world only through the traits in [`platform`] (clipboard, dialogs,
//!   window) and [`disk::Disk`] (file system), so tests drive it headless.
//! - [`shell`]: the winit event loop and the wgpu renderer. It turns window
//!   events into [`pandite_ui::InputEvent`]s, wakes the core when workers
//!   finish, uploads content and draws frames.
//!
//! No file is read, listed or written on the event-loop thread: the library
//! workers, the app's own job threads and the dialog threads do all of it,
//! and they report back through events plus an event-loop wake.

#![deny(unsafe_code)]

pub mod app;
pub mod cli;
pub mod console;
pub mod disk;
pub mod edit;
pub mod icon;
pub mod platform;
pub mod settings;
pub mod shell;

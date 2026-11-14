//! pandite-ui: the pandite interface, driven entirely by plain data.
//!
//! This crate has no GPU, window system or third-party dependency. It reads
//! [`InputEvent`]s and a per-frame [`UiContext`] snapshot, returns
//! [`Action`]s, and appends a [`pandite_core::DrawList`] that the renderer
//! draws on top of the image. Every behavior is testable headless.
//!
//! # Frame loop
//!
//! ```no_run
//! # use pandite_ui::*;
//! # use pandite_core::{DrawList, Size, TextMeasure, TextureSink, TextureId, ImageBuf};
//! # struct Font; impl TextMeasure for Font { fn measure(&self, t: &str, s: f32) -> Size { Size::new(t.len() as f32 * s * 0.5, s) } }
//! # struct Gpu; impl TextureSink for Gpu { fn upload(&mut self, _: TextureId, _: &ImageBuf, _: bool) {} fn remove(&mut self, _: TextureId) {} }
//! # let (font, mut gpu) = (Font, Gpu);
//! let mut ui = Ui::new();
//! let ctx = UiContext::default(); // filled from application state
//! // For each window event, translated into an InputEvent:
//! for action in ui.handle_event(InputEvent::Tick, &ctx) {
//!     // carry out the action
//! #   let _ = action;
//! }
//! // For each frame:
//! ui.sync_textures(&ctx, &mut gpu);
//! let viewport = ui.content_rect(&ctx); // where the image goes
//! let mut list = DrawList::new();
//! ui.build(&ctx, &font, &mut list);
//! // Keep drawing while ui.wants_redraw(); otherwise wake at
//! // ui.next_redraw_at(). Set the pointer to ui.cursor().
//! # let _ = viewport;
//! ```
//!
//! # Texture ids
//!
//! - Ids below [`ICON_TEXTURE_BASE`] (`1 << 40`) belong to the application
//!   (the image itself, for example).
//! - Icons use `ICON_TEXTURE_BASE + n` ([`Icon::texture_id`]); the
//!   interface uploads them through [`Ui::sync_textures`] as white alpha
//!   masks (`alpha_mask = true`) and tints them when drawing.
//! - Thumbnails use [`thumbnail_texture_id`]`(index)` =
//!   `THUMBNAIL_TEXTURE_BASE + index`. The application uploads each
//!   thumbnail under that id and marks it [`ThumbState::Ready`] in
//!   [`UiContext::thumbnails`]; the interface draws only visible ones.
//!
//! # Coordinates
//!
//! Positions and sizes are physical pixels from the window's top-left,
//! matching the draw list. Every interface length is a logical size times
//! [`UiContext::scale_factor`], rounded to whole pixels.
//!
//! # Recognized text
//!
//! When [`UiContext::text`] carries the words of the image on screen, the
//! pointer over a word is a text cursor, a left drag that starts on a word
//! selects words in reading order, a double click selects a word and a
//! triple click its line. With the button held after the second or third
//! press, a drag extends that selection by whole words or whole lines, and
//! the release ends it. A double click off text toggles fit and 1:1.
//! Ctrl+A selects everything, Ctrl+C copies the selection as
//! [`Action::CopyText`] (lines joined with `"\n"`, words with one space).
//! Without a selection Ctrl+C copies the image ([`Action::CopyImage`]).
//! Drags that start off text pan as before. Highlights are drawn over the
//! image with the same placement as the image itself.
//!
//! # Information panel
//!
//! I emits [`Action::ToggleInfo`]. While [`UiContext::info_visible`] is
//! set and [`UiContext::info`] has rows, a panel of label and value rows
//! shows in the bottom right corner of the content area, above the
//! gallery strip ([`Ui::info_panel_rect`]). It is not a dialog: keys,
//! Escape included, act as without it, and it hides in crop mode.
//!
//! # Selection
//!
//! The application keeps the set of selected files and reports their
//! folder indices in [`UiContext::selected`]. Space emits
//! [`Action::ToggleSelect`] for the image on screen and Backspace emits
//! [`Action::ClearSelection`]; neither navigates. A selected thumbnail
//! draws a checked box in its top left corner, and the thumbnail under the
//! pointer an empty one. A click on that box emits
//! [`Action::ToggleSelect`] for its index and does not show the image. The
//! image on screen draws a check badge while it is selected, and the
//! toolbar shows the count. The context menu offers both commands. Space
//! and Backspace never change the selection in crop mode, in a dialog or
//! in the shortcut overlay; in a text field they edit the text. The bulk
//! dialog opens with the selected images as its source
//! ([`BulkSource::Selection`]).
//!
//! # Escape
//!
//! Escape closes the topmost menu, shortcut overlay or dialog. When none
//! is open it emits [`Action::Quit`], also in full screen, in crop mode,
//! with a text selection, with the information panel and while a file
//! loads.

#![forbid(unsafe_code)]

mod action;
mod bulk_dialog;
mod command;
mod confirm;
mod context;
mod crop;
mod form;
mod gallery;
mod icon;
mod input;
mod menu;
mod metrics;
mod paint;
mod raster;
mod resize_dialog;
mod text_field;
mod text_select;
mod theme;
mod toolbar;
mod ui;

pub use action::{
    Action, ActionKind, BulkJob, BulkSource, OverwritePolicy, PixelRect, ResizeSpec,
    WHEEL_ZOOM_STEP, ZOOM_STEP,
};
pub use command::{command_for_key, Command, Shortcut, SHORTCUTS};
pub use confirm::ConfirmRequest;
pub use context::{
    EncoderInfo, ImageInfo, InfoRow, Loading, Progress, TextView, ThumbState, UiContext,
};
pub use icon::{Icon, ICON_TEXTURE_BASE};
pub use input::{CursorShape, InputEvent, Key, Modifiers, PointerButton, WheelDelta};
pub use text_field::{CharFilter, FieldResponse, TextField};
pub use theme::{contrast, luminance, Theme, ThemeKind};
pub use ui::{Ui, UiMode};

use pandite_core::TextureId;

/// First texture id used for gallery thumbnails.
pub const THUMBNAIL_TEXTURE_BASE: u64 = 1 << 41;

/// The texture id the application uploads the thumbnail of folder index
/// `index` under: `THUMBNAIL_TEXTURE_BASE + index`.
pub fn thumbnail_texture_id(index: usize) -> TextureId {
    TextureId(THUMBNAIL_TEXTURE_BASE + index as u64)
}

/// The folder index of a thumbnail texture id, if it is one.
pub fn thumbnail_index(id: TextureId) -> Option<usize> {
    id.0.checked_sub(THUMBNAIL_TEXTURE_BASE)
        .and_then(|i| usize::try_from(i).ok())
}

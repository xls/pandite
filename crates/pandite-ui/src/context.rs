//! The per-frame snapshot the application hands to the interface.

use crate::theme::ThemeKind;
use pandite_core::{Affine, Rect, Size, TextLayout, ViewState};
use std::path::Path;
use std::time::Duration;

/// Recognized text of the image on screen.
#[derive(Clone, Copy, Debug)]
pub struct TextView<'a> {
    /// The words, in pixels of the image the text was read from. Give a
    /// layout with at least one word; an empty one offers nothing.
    pub layout: &'a TextLayout,
    /// Maps layout pixels to pixels of the displayed image
    /// ([`ImageInfo::width`] x [`ImageInfo::height`]): the view edits made
    /// since the text was read (quarter turns, mirrors, crops, resizes).
    /// The identity for an unedited image.
    pub to_image: Affine,
    /// Names this layout on this image. Change it whenever either changes;
    /// a selection made under another id is dropped.
    pub id: u64,
}

/// Facts about the image on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageInfo<'a> {
    /// File name shown in the status readout.
    pub file_name: &'a str,
    /// Width in pixels of the displayed image, after view edits (a rotated
    /// image reports its rotated size). Crop rectangles use this space.
    pub width: u32,
    /// Height in pixels of the displayed image, after view edits.
    pub height: u32,
    /// The current view.
    pub view: ViewState,
    /// The effective zoom in percent (100 is 1:1).
    pub zoom_percent: f32,
    /// Where the image is drawn, in window pixels (normally
    /// [`ViewState::image_rect`] with [`crate::Ui::content_rect`]).
    pub screen_rect: Rect,
    /// True when unsaved view edits exist.
    pub edited: bool,
}

/// The state of one gallery thumbnail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThumbState {
    /// Not decoded yet. Drawn as a placeholder.
    #[default]
    Pending,
    /// Uploaded under [`crate::thumbnail_texture_id`] with this pixel size.
    Ready {
        /// Thumbnail width in pixels.
        width: u32,
        /// Thumbnail height in pixels.
        height: u32,
    },
    /// The file sits on a cloud drive and is not downloaded. Drawn as a
    /// cloud placeholder; the application should not force a download for
    /// a thumbnail.
    NotDownloaded,
    /// Decoding failed. Drawn as a warning placeholder.
    Failed,
}

/// Progress of background work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress<'a> {
    /// What is happening, for example "Saving photo.png" or the file being
    /// converted.
    pub label: &'a str,
    /// Units done.
    pub done: u64,
    /// Units in total, 0 when unknown.
    pub total: u64,
}

impl Progress<'_> {
    /// Done as a fraction 0 to 1, `None` when the total is unknown.
    pub fn fraction(&self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32)
    }
}

/// A file being read or downloaded on a worker thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loading<'a> {
    /// The file name being opened.
    pub file_name: &'a str,
    /// Time since the load started.
    pub elapsed: Duration,
    /// Bytes read of the total, 0 to 1, when known.
    pub progress: Option<f32>,
    /// True when the file is being downloaded from a cloud drive.
    pub cloud: bool,
    /// True when a previous image stays on screen behind the overlay. When
    /// false the overlay is the whole first screen.
    pub over_image: bool,
}

/// One line of the information panel: what a fact is and its value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InfoRow {
    /// What the value is, for example "Pixels".
    pub label: String,
    /// The value, for example "6000 x 4000".
    pub value: String,
}

impl InfoRow {
    /// A row from a label and a value.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// An encoder the bulk dialog offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncoderInfo<'a> {
    /// Stable id, returned in [`crate::BulkJob::format`].
    pub id: &'a str,
    /// Name shown in the list, for example "PNG".
    pub name: &'a str,
    /// True when the quality setting changes the output.
    pub has_quality: bool,
}

/// Everything the interface needs to know about the application for one
/// event or frame. Build it fresh each time; it borrows application data.
#[derive(Clone, Copy, Debug)]
pub struct UiContext<'a> {
    /// Monotonic time since an arbitrary start, the same clock for every
    /// call. Drives double clicks, the toolbar idle timer and animations.
    pub now: Duration,
    /// Window inner size in physical pixels.
    pub window_size: Size,
    /// DPI scale factor (1.0 at 96 DPI on Windows).
    pub scale_factor: f32,
    /// Which color table to use (follow the system setting).
    pub theme: ThemeKind,
    /// True in full screen.
    pub fullscreen: bool,
    /// The user's gallery setting. The strip shows when this is set, the
    /// folder has images and the window is tall enough.
    pub gallery_visible: bool,
    /// The image on screen, `None` when nothing is open or it failed.
    pub image: Option<ImageInfo<'a>>,
    /// Index of the current image in the folder.
    pub index: usize,
    /// Number of images in the folder.
    pub count: usize,
    /// The folder of the current image, offered by the bulk dialog.
    pub folder: Option<&'a Path>,
    /// Thumbnail state per folder index. Missing entries count as pending.
    pub thumbnails: &'a [ThumbState],
    /// True when an edit can be undone.
    pub can_undo: bool,
    /// Background work other than bulk jobs (saving, for example).
    pub busy: Option<Progress<'a>>,
    /// Progress of the running bulk job. Keep it `Some` from
    /// [`crate::Action::RunBulk`] until the job ends.
    pub bulk: Option<Progress<'a>>,
    /// A file being read or downloaded.
    pub loading: Option<Loading<'a>>,
    /// A message that stays while set (for example why a file failed).
    pub notice: Option<&'a str>,
    /// Encoders the bulk dialog offers.
    pub encoders: &'a [EncoderInfo<'a>],
    /// True when this platform reads text in images and the user allows
    /// it. The shortcut overlay lists the text commands only then.
    pub text_enabled: bool,
    /// The text recognized in the image on screen, when there is any.
    /// Words then take a text cursor, drags over them select text, and
    /// Ctrl+A and Ctrl+C select and copy it.
    pub text: Option<TextView<'a>>,
    /// The user's information panel setting. The panel shows in the
    /// bottom right corner of the content area when this is set and
    /// [`UiContext::info`] has rows. It is not a dialog: every key and
    /// pointer gesture outside it works as before.
    pub info_visible: bool,
    /// The rows of the information panel, in display order. Empty when
    /// nothing is open.
    pub info: &'a [InfoRow],
    /// Folder indices of the selected images, in ascending order without
    /// repeats. The gallery strip draws a small badge on these thumbnails,
    /// the image on screen shows a "Marked" label when its index is here,
    /// and the toolbar shows the count. The bulk dialog offers them as a
    /// source.
    pub selected: &'a [usize],
}

impl Default for UiContext<'_> {
    fn default() -> Self {
        Self {
            now: Duration::ZERO,
            window_size: Size::new(1280.0, 800.0),
            scale_factor: 1.0,
            theme: ThemeKind::Dark,
            fullscreen: false,
            gallery_visible: true,
            image: None,
            index: 0,
            count: 0,
            folder: None,
            thumbnails: &[],
            can_undo: false,
            busy: None,
            bulk: None,
            loading: None,
            notice: None,
            encoders: &[],
            text_enabled: false,
            text: None,
            info_visible: false,
            info: &[],
            selected: &[],
        }
    }
}

impl UiContext<'_> {
    /// True when the image with folder index `index` is selected.
    pub fn is_selected(&self, index: usize) -> bool {
        self.selected.binary_search(&index).is_ok()
    }

    /// True when the image on screen (folder index [`UiContext::index`])
    /// is selected.
    pub fn current_selected(&self) -> bool {
        self.is_selected(self.index)
    }

    /// The thumbnail state of `index`.
    pub fn thumbnail(&self, index: usize) -> ThumbState {
        self.thumbnails.get(index).copied().unwrap_or_default()
    }

    /// The whole window as a rectangle.
    pub fn window_rect(&self) -> Rect {
        Rect::new(
            0.0,
            0.0,
            self.window_size.w.max(0.0),
            self.window_size.h.max(0.0),
        )
    }
}

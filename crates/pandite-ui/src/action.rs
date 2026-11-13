//! What the user asked for, as plain data the application acts on.
//!
//! [`crate::Ui::handle_event`] returns a list of [`Action`]s. Each one is
//! either a request the application carries out (rotate, save, go to an
//! image), a request the application answers (pick files, paste), or a
//! notice that the interface changed its own state (a dialog opened). The
//! structs here describe resize rules and bulk jobs without depending on
//! any other pandite crate; the application converts them.

use pandite_core::image::MAX_DIMENSION;
use pandite_core::Point;
use std::path::PathBuf;

/// Zoom factor of one zoom-in or zoom-out step (keyboard, toolbar, menu).
pub const ZOOM_STEP: f32 = 1.25;

/// Zoom factor of one wheel notch over the image.
pub const WHEEL_ZOOM_STEP: f32 = 1.2;

/// A rectangle in image pixels. Always inside the image it was made for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PixelRect {
    /// Left column.
    pub x: u32,
    /// Top row.
    pub y: u32,
    /// Width in pixels, at least 1.
    pub width: u32,
    /// Height in pixels, at least 1.
    pub height: u32,
}

/// A resize rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResizeSpec {
    /// Scale both sides by this percentage (100 keeps the size).
    Percent(f32),
    /// Best fit inside a box, keeping the aspect ratio.
    Fit {
        /// Box width in pixels.
        width: u32,
        /// Box height in pixels.
        height: u32,
        /// Also scale up images that are smaller than the box.
        enlarge: bool,
    },
    /// An exact size. The aspect ratio may change.
    Exact {
        /// Width in pixels.
        width: u32,
        /// Height in pixels.
        height: u32,
    },
}

impl ResizeSpec {
    /// The size an image of `width` x `height` gets under this rule. Each
    /// side is at least 1. Returns `None` when the rule is invalid (zero or
    /// negative values) or a side would exceed
    /// [`pandite_core::image::MAX_DIMENSION`].
    pub fn result_size(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width == 0 || height == 0 {
            return None;
        }
        let (w, h) = (width as f64, height as f64);
        let (nw, nh) = match *self {
            ResizeSpec::Percent(p) => {
                if !(p.is_finite() && p > 0.0) {
                    return None;
                }
                let s = f64::from(p) / 100.0;
                (w * s, h * s)
            }
            ResizeSpec::Fit {
                width: bw,
                height: bh,
                enlarge,
            } => {
                if bw == 0 || bh == 0 {
                    return None;
                }
                let mut s = (f64::from(bw) / w).min(f64::from(bh) / h);
                if !enlarge {
                    s = s.min(1.0);
                }
                (w * s, h * s)
            }
            ResizeSpec::Exact {
                width: ew,
                height: eh,
            } => {
                if ew == 0 || eh == 0 {
                    return None;
                }
                (f64::from(ew), f64::from(eh))
            }
        };
        let nw = nw.round().max(1.0);
        let nh = nh.round().max(1.0);
        let max = f64::from(MAX_DIMENSION);
        if nw > max || nh > max {
            return None;
        }
        Some((nw as u32, nh as u32))
    }
}

/// Which images a bulk job reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BulkSource {
    /// These files.
    Files(Vec<PathBuf>),
    /// Every image the application can open in this folder.
    Folder {
        /// The folder.
        path: PathBuf,
        /// Also read subfolders.
        recursive: bool,
    },
}

impl BulkSource {
    /// True when the source names nothing.
    pub fn is_empty(&self) -> bool {
        match self {
            BulkSource::Files(files) => files.is_empty(),
            BulkSource::Folder { path, .. } => path.as_os_str().is_empty(),
        }
    }
}

/// What a bulk job does when an output file already exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverwritePolicy {
    /// Leave the existing file and skip this image.
    #[default]
    Skip,
    /// Replace the existing file.
    Overwrite,
    /// Write under a new name (for example `photo (1).png`).
    Rename,
}

/// A bulk resize and convert job.
#[derive(Clone, Debug, PartialEq)]
pub struct BulkJob {
    /// The images to read.
    pub source: BulkSource,
    /// The size rule, or `None` to keep every image's size.
    pub resize: Option<ResizeSpec>,
    /// The encoder id to write (from [`crate::EncoderInfo::id`]), or `None`
    /// to keep each file's own format.
    pub format: Option<String>,
    /// Encoder quality, 1 to 100.
    pub quality: u8,
    /// The folder that receives the output files.
    pub destination: PathBuf,
    /// What to do when an output file exists.
    pub overwrite: OverwritePolicy,
}

/// Everything the user can ask for.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Show the system open dialog and open the chosen file.
    Open,
    /// Show the system save dialog and write the edited image. The format
    /// follows the chosen extension.
    SaveAs,
    /// Put the current file's absolute path on the clipboard, then call
    /// [`crate::Ui::toast`] with a confirmation.
    CopyPath,
    /// Put the image on screen on the clipboard as pixels: full
    /// resolution, with the view edits applied (what Save As would write),
    /// the frame on screen for an animation. Do the work off the event
    /// loop and confirm with [`crate::Ui::toast`].
    CopyImage,
    /// Put this text on the clipboard (Ctrl+C or Ctrl+X in a text field).
    CopyText(String),
    /// Read the clipboard and send its text back as
    /// [`crate::InputEvent::Text`] (Ctrl+V in a text field).
    RequestPaste,
    /// Rotate the view 90 degrees counterclockwise.
    RotateLeft,
    /// Rotate the view 90 degrees clockwise.
    RotateRight,
    /// Mirror left to right.
    FlipHorizontal,
    /// Mirror top to bottom.
    FlipVertical,
    /// Notice: the interface entered crop mode.
    StartCrop,
    /// Crop to this rectangle of the displayed (edited) image. The
    /// interface has left crop mode.
    ApplyCrop(PixelRect),
    /// Notice: the interface left crop mode without cropping.
    CancelCrop,
    /// Notice: the interface opened its resize dialog.
    OpenResizeDialog,
    /// Resize the displayed image with this rule.
    ApplyResize(ResizeSpec),
    /// Notice: the interface opened its bulk dialog.
    OpenBulkDialog,
    /// Show a multi-file picker and answer with
    /// [`crate::Ui::add_bulk_files`].
    PickBulkFiles,
    /// Show a folder picker and answer with [`crate::Ui::set_bulk_folder`].
    PickBulkFolder,
    /// Show a folder picker and answer with
    /// [`crate::Ui::set_bulk_destination`].
    PickBulkDestination,
    /// Start this bulk job. Report progress through
    /// [`crate::UiContext::bulk`] until it ends.
    RunBulk(BulkJob),
    /// Stop the running bulk job.
    CancelBulk,
    /// Undo the last view edit (rotate, flip, crop, resize).
    Undo,
    /// Display mode: best fit in the content viewport.
    FitToWindow,
    /// Display mode: one image pixel per screen pixel. `anchor` is the
    /// window position to keep fixed (double click), `None` for the
    /// viewport center.
    OneToOne {
        /// Window position to keep over the same image point.
        anchor: Option<Point>,
    },
    /// Zoom in one [`ZOOM_STEP`] around the viewport center.
    ZoomIn,
    /// Zoom out one [`ZOOM_STEP`] around the viewport center.
    ZoomOut,
    /// Multiply the zoom by `factor`, keeping the image point under
    /// `anchor` (a window position) fixed. Pass both to
    /// [`pandite_core::ViewState::zoom_by`] with the content viewport.
    ZoomAt {
        /// Zoom multiplier, above 1 zooms in.
        factor: f32,
        /// Window position to keep fixed.
        anchor: Point,
    },
    /// Move the image by this many physical pixels. Pass both to
    /// [`pandite_core::ViewState::pan_by`].
    PanBy {
        /// Horizontal movement, positive right.
        dx: f32,
        /// Vertical movement, positive down.
        dy: f32,
    },
    /// Enter or leave full screen.
    ToggleFullscreen,
    /// Show the next image in the folder.
    Next,
    /// Show the previous image in the folder.
    Previous,
    /// Show the image with this index in the folder.
    GoTo(usize),
    /// Show or hide the gallery strip
    /// (flip [`crate::UiContext::gallery_visible`]).
    ToggleGallery,
    /// Show or hide the information panel
    /// (flip [`crate::UiContext::info_visible`]).
    ToggleInfo,
    /// The answer to a [`crate::ConfirmRequest`].
    ConfirmResult {
        /// The request id.
        id: u64,
        /// True when the user confirmed.
        accepted: bool,
    },
    /// Close the application.
    Quit,
}

/// The variant of an [`Action`] without its data. Used to check that every
/// action is reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ActionKind {
    /// [`Action::Open`].
    Open,
    /// [`Action::SaveAs`].
    SaveAs,
    /// [`Action::CopyPath`].
    CopyPath,
    /// [`Action::CopyImage`].
    CopyImage,
    /// [`Action::CopyText`].
    CopyText,
    /// [`Action::RequestPaste`].
    RequestPaste,
    /// [`Action::RotateLeft`].
    RotateLeft,
    /// [`Action::RotateRight`].
    RotateRight,
    /// [`Action::FlipHorizontal`].
    FlipHorizontal,
    /// [`Action::FlipVertical`].
    FlipVertical,
    /// [`Action::StartCrop`].
    StartCrop,
    /// [`Action::ApplyCrop`].
    ApplyCrop,
    /// [`Action::CancelCrop`].
    CancelCrop,
    /// [`Action::OpenResizeDialog`].
    OpenResizeDialog,
    /// [`Action::ApplyResize`].
    ApplyResize,
    /// [`Action::OpenBulkDialog`].
    OpenBulkDialog,
    /// [`Action::PickBulkFiles`].
    PickBulkFiles,
    /// [`Action::PickBulkFolder`].
    PickBulkFolder,
    /// [`Action::PickBulkDestination`].
    PickBulkDestination,
    /// [`Action::RunBulk`].
    RunBulk,
    /// [`Action::CancelBulk`].
    CancelBulk,
    /// [`Action::Undo`].
    Undo,
    /// [`Action::FitToWindow`].
    FitToWindow,
    /// [`Action::OneToOne`].
    OneToOne,
    /// [`Action::ZoomIn`].
    ZoomIn,
    /// [`Action::ZoomOut`].
    ZoomOut,
    /// [`Action::ZoomAt`].
    ZoomAt,
    /// [`Action::PanBy`].
    PanBy,
    /// [`Action::ToggleFullscreen`].
    ToggleFullscreen,
    /// [`Action::Next`].
    Next,
    /// [`Action::Previous`].
    Previous,
    /// [`Action::GoTo`].
    GoTo,
    /// [`Action::ToggleGallery`].
    ToggleGallery,
    /// [`Action::ToggleInfo`].
    ToggleInfo,
    /// [`Action::ConfirmResult`].
    ConfirmResult,
    /// [`Action::Quit`].
    Quit,
}

impl ActionKind {
    /// Every kind, in declaration order.
    pub const ALL: [ActionKind; 36] = [
        ActionKind::Open,
        ActionKind::SaveAs,
        ActionKind::CopyPath,
        ActionKind::CopyImage,
        ActionKind::CopyText,
        ActionKind::RequestPaste,
        ActionKind::RotateLeft,
        ActionKind::RotateRight,
        ActionKind::FlipHorizontal,
        ActionKind::FlipVertical,
        ActionKind::StartCrop,
        ActionKind::ApplyCrop,
        ActionKind::CancelCrop,
        ActionKind::OpenResizeDialog,
        ActionKind::ApplyResize,
        ActionKind::OpenBulkDialog,
        ActionKind::PickBulkFiles,
        ActionKind::PickBulkFolder,
        ActionKind::PickBulkDestination,
        ActionKind::RunBulk,
        ActionKind::CancelBulk,
        ActionKind::Undo,
        ActionKind::FitToWindow,
        ActionKind::OneToOne,
        ActionKind::ZoomIn,
        ActionKind::ZoomOut,
        ActionKind::ZoomAt,
        ActionKind::PanBy,
        ActionKind::ToggleFullscreen,
        ActionKind::Next,
        ActionKind::Previous,
        ActionKind::GoTo,
        ActionKind::ToggleGallery,
        ActionKind::ToggleInfo,
        ActionKind::ConfirmResult,
        ActionKind::Quit,
    ];

    /// The position of this kind in [`ActionKind::ALL`]. The match is
    /// exhaustive, so a new variant cannot be forgotten here.
    pub const fn index(self) -> usize {
        match self {
            ActionKind::Open => 0,
            ActionKind::SaveAs => 1,
            ActionKind::CopyPath => 2,
            ActionKind::CopyImage => 3,
            ActionKind::CopyText => 4,
            ActionKind::RequestPaste => 5,
            ActionKind::RotateLeft => 6,
            ActionKind::RotateRight => 7,
            ActionKind::FlipHorizontal => 8,
            ActionKind::FlipVertical => 9,
            ActionKind::StartCrop => 10,
            ActionKind::ApplyCrop => 11,
            ActionKind::CancelCrop => 12,
            ActionKind::OpenResizeDialog => 13,
            ActionKind::ApplyResize => 14,
            ActionKind::OpenBulkDialog => 15,
            ActionKind::PickBulkFiles => 16,
            ActionKind::PickBulkFolder => 17,
            ActionKind::PickBulkDestination => 18,
            ActionKind::RunBulk => 19,
            ActionKind::CancelBulk => 20,
            ActionKind::Undo => 21,
            ActionKind::FitToWindow => 22,
            ActionKind::OneToOne => 23,
            ActionKind::ZoomIn => 24,
            ActionKind::ZoomOut => 25,
            ActionKind::ZoomAt => 26,
            ActionKind::PanBy => 27,
            ActionKind::ToggleFullscreen => 28,
            ActionKind::Next => 29,
            ActionKind::Previous => 30,
            ActionKind::GoTo => 31,
            ActionKind::ToggleGallery => 32,
            ActionKind::ToggleInfo => 33,
            ActionKind::ConfirmResult => 34,
            ActionKind::Quit => 35,
        }
    }
}

impl Action {
    /// The variant without its data.
    pub fn kind(&self) -> ActionKind {
        match self {
            Action::Open => ActionKind::Open,
            Action::SaveAs => ActionKind::SaveAs,
            Action::CopyPath => ActionKind::CopyPath,
            Action::CopyImage => ActionKind::CopyImage,
            Action::CopyText(_) => ActionKind::CopyText,
            Action::RequestPaste => ActionKind::RequestPaste,
            Action::RotateLeft => ActionKind::RotateLeft,
            Action::RotateRight => ActionKind::RotateRight,
            Action::FlipHorizontal => ActionKind::FlipHorizontal,
            Action::FlipVertical => ActionKind::FlipVertical,
            Action::StartCrop => ActionKind::StartCrop,
            Action::ApplyCrop(_) => ActionKind::ApplyCrop,
            Action::CancelCrop => ActionKind::CancelCrop,
            Action::OpenResizeDialog => ActionKind::OpenResizeDialog,
            Action::ApplyResize(_) => ActionKind::ApplyResize,
            Action::OpenBulkDialog => ActionKind::OpenBulkDialog,
            Action::PickBulkFiles => ActionKind::PickBulkFiles,
            Action::PickBulkFolder => ActionKind::PickBulkFolder,
            Action::PickBulkDestination => ActionKind::PickBulkDestination,
            Action::RunBulk(_) => ActionKind::RunBulk,
            Action::CancelBulk => ActionKind::CancelBulk,
            Action::Undo => ActionKind::Undo,
            Action::FitToWindow => ActionKind::FitToWindow,
            Action::OneToOne { .. } => ActionKind::OneToOne,
            Action::ZoomIn => ActionKind::ZoomIn,
            Action::ZoomOut => ActionKind::ZoomOut,
            Action::ZoomAt { .. } => ActionKind::ZoomAt,
            Action::PanBy { .. } => ActionKind::PanBy,
            Action::ToggleFullscreen => ActionKind::ToggleFullscreen,
            Action::Next => ActionKind::Next,
            Action::Previous => ActionKind::Previous,
            Action::GoTo(_) => ActionKind::GoTo,
            Action::ToggleGallery => ActionKind::ToggleGallery,
            Action::ToggleInfo => ActionKind::ToggleInfo,
            Action::ConfirmResult { .. } => ActionKind::ConfirmResult,
            Action::Quit => ActionKind::Quit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_kind_all_is_complete_and_ordered() {
        for (i, kind) in ActionKind::ALL.iter().enumerate() {
            assert_eq!(kind.index(), i, "{kind:?} out of place");
        }
    }

    #[test]
    fn resize_spec_result_sizes() {
        assert_eq!(
            ResizeSpec::Percent(50.0).result_size(4000, 3000),
            Some((2000, 1500))
        );
        assert_eq!(
            ResizeSpec::Percent(1.0).result_size(10, 10),
            Some((1, 1)),
            "sides never drop below one pixel"
        );
        assert_eq!(ResizeSpec::Percent(0.0).result_size(10, 10), None);
        assert_eq!(ResizeSpec::Percent(f32::NAN).result_size(10, 10), None);
        assert_eq!(
            ResizeSpec::Fit {
                width: 1920,
                height: 1080,
                enlarge: false
            }
            .result_size(4000, 3000),
            Some((1440, 1080))
        );
        assert_eq!(
            ResizeSpec::Fit {
                width: 1920,
                height: 1080,
                enlarge: false
            }
            .result_size(400, 300),
            Some((400, 300))
        );
        assert_eq!(
            ResizeSpec::Fit {
                width: 1920,
                height: 1080,
                enlarge: true
            }
            .result_size(400, 300),
            Some((1440, 1080))
        );
        assert_eq!(
            ResizeSpec::Exact {
                width: 7,
                height: 9
            }
            .result_size(1, 1),
            Some((7, 9))
        );
        assert_eq!(
            ResizeSpec::Exact {
                width: 0,
                height: 9
            }
            .result_size(1, 1),
            None
        );
        assert_eq!(
            ResizeSpec::Percent(1000.0).result_size(10_000, 10),
            None,
            "result above the dimension limit"
        );
    }
}

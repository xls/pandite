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
    /// The selected images of the folder on screen
    /// ([`crate::UiContext::selected`]). The application turns this into
    /// the list of those files when the job starts.
    Selection,
}

impl BulkSource {
    /// True when the source names nothing. A selection source is never
    /// empty here: the application knows its files.
    pub fn is_empty(&self) -> bool {
        match self {
            BulkSource::Files(files) => files.is_empty(),
            BulkSource::Folder { path, .. } => path.as_os_str().is_empty(),
            BulkSource::Selection => false,
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
    /// Move the file of the current image to the trash of the system
    /// without a question (Delete), then show its neighbor. A file whose
    /// drive has no trash is removed for good instead, also without a
    /// question.
    MoveToTrash,
    /// Remove the file of the current image for good (Shift+Delete). Ask
    /// first with [`crate::Ui::show_confirm`] and a request whose
    /// [`crate::ConfirmRequest::cancel_is_default`] is set.
    ///
    /// While images are selected ([`crate::UiContext::selected`]), this
    /// action and [`Action::MoveToTrash`] act on the selected files
    /// instead, and both ask first with the number of files.
    DeleteFile,
    /// Select the image with this folder index, or unselect it when it is
    /// selected (Space on the current image). Report the result in
    /// [`crate::UiContext::selected`].
    ToggleSelect(usize),
    /// Select the image with this folder index; an image that is selected
    /// stays selected (Shift+Left and Shift+Right, before their move).
    Select(usize),
    /// Unselect every image (Backspace).
    ClearSelection,
    /// Compare the selected images (D): one selected image against the
    /// image on screen with a slider, or two to eight selected images in
    /// tiles. Report the comparison in [`crate::UiContext::compare`].
    Compare,
    /// End the comparison (D again, or Escape).
    EndCompare,
    /// Move the selected files, or the file of the current image when
    /// nothing is selected, to a folder the user picks (M). The folder
    /// picker is the confirmation; there is no question.
    MoveTo,
    /// Copy the selected files, or the file of the current image when
    /// nothing is selected, to a folder the user picks (Shift+M). The files
    /// stay where they are. The folder picker is the confirmation; there is
    /// no question.
    CopyTo,
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
    /// Show the image this many places after the current one, or before it
    /// when negative (Page Down and Page Up move one gallery page). A move
    /// past an end stops at the first or last image. From that image, the
    /// next move in the same direction wraps when the application wraps
    /// single steps.
    StepBy(isize),
    /// Show or hide the gallery strip
    /// (flip [`crate::UiContext::gallery_visible`]).
    ToggleGallery,
    /// Make the gallery thumbnails one size larger (set
    /// [`crate::UiContext::gallery_size`] to [`crate::GallerySize::larger`]),
    /// then make them again at the new [`crate::Ui::thumbnail_px`]. Only
    /// emitted while a larger size exists and the strip is not hidden.
    LargerThumbnails,
    /// Make the gallery thumbnails one size smaller, the same way as
    /// [`Action::LargerThumbnails`].
    SmallerThumbnails,
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
    /// The answer to the [`crate::Prompt`] of [`crate::UiContext::prompt`]:
    /// its button (`accepted`), or its close mark or Escape (not
    /// `accepted`).
    PromptResult {
        /// The prompt id.
        id: u64,
        /// True for the button.
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
    /// [`Action::MoveToTrash`].
    MoveToTrash,
    /// [`Action::DeleteFile`].
    DeleteFile,
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
    /// [`Action::StepBy`].
    StepBy,
    /// [`Action::ToggleGallery`].
    ToggleGallery,
    /// [`Action::ToggleInfo`].
    ToggleInfo,
    /// [`Action::ConfirmResult`].
    ConfirmResult,
    /// [`Action::Quit`].
    Quit,
    /// [`Action::ToggleSelect`].
    ToggleSelect,
    /// [`Action::ClearSelection`].
    ClearSelection,
    /// [`Action::Compare`].
    Compare,
    /// [`Action::EndCompare`].
    EndCompare,
    /// [`Action::Select`].
    Select,
    /// [`Action::MoveTo`].
    MoveTo,
    /// [`Action::CopyTo`].
    CopyTo,
    /// [`Action::LargerThumbnails`].
    LargerThumbnails,
    /// [`Action::SmallerThumbnails`].
    SmallerThumbnails,
    /// [`Action::PromptResult`].
    PromptResult,
}

impl ActionKind {
    /// Every kind, in declaration order.
    pub const ALL: [ActionKind; 49] = [
        ActionKind::Open,
        ActionKind::SaveAs,
        ActionKind::CopyPath,
        ActionKind::CopyImage,
        ActionKind::MoveToTrash,
        ActionKind::DeleteFile,
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
        ActionKind::StepBy,
        ActionKind::ToggleGallery,
        ActionKind::ToggleInfo,
        ActionKind::ConfirmResult,
        ActionKind::Quit,
        ActionKind::ToggleSelect,
        ActionKind::ClearSelection,
        ActionKind::Select,
        ActionKind::MoveTo,
        ActionKind::CopyTo,
        ActionKind::LargerThumbnails,
        ActionKind::SmallerThumbnails,
        ActionKind::PromptResult,
        ActionKind::Compare,
        ActionKind::EndCompare,
    ];

    /// The position of this kind in [`ActionKind::ALL`]. The match is
    /// exhaustive, so a new variant cannot be forgotten here.
    pub const fn index(self) -> usize {
        match self {
            ActionKind::Open => 0,
            ActionKind::SaveAs => 1,
            ActionKind::CopyPath => 2,
            ActionKind::CopyImage => 3,
            ActionKind::MoveToTrash => 4,
            ActionKind::DeleteFile => 5,
            ActionKind::CopyText => 6,
            ActionKind::RequestPaste => 7,
            ActionKind::RotateLeft => 8,
            ActionKind::RotateRight => 9,
            ActionKind::FlipHorizontal => 10,
            ActionKind::FlipVertical => 11,
            ActionKind::StartCrop => 12,
            ActionKind::ApplyCrop => 13,
            ActionKind::CancelCrop => 14,
            ActionKind::OpenResizeDialog => 15,
            ActionKind::ApplyResize => 16,
            ActionKind::OpenBulkDialog => 17,
            ActionKind::PickBulkFiles => 18,
            ActionKind::PickBulkFolder => 19,
            ActionKind::PickBulkDestination => 20,
            ActionKind::RunBulk => 21,
            ActionKind::CancelBulk => 22,
            ActionKind::Undo => 23,
            ActionKind::FitToWindow => 24,
            ActionKind::OneToOne => 25,
            ActionKind::ZoomIn => 26,
            ActionKind::ZoomOut => 27,
            ActionKind::ZoomAt => 28,
            ActionKind::PanBy => 29,
            ActionKind::ToggleFullscreen => 30,
            ActionKind::Next => 31,
            ActionKind::Previous => 32,
            ActionKind::GoTo => 33,
            ActionKind::StepBy => 34,
            ActionKind::ToggleGallery => 35,
            ActionKind::ToggleInfo => 36,
            ActionKind::ConfirmResult => 37,
            ActionKind::Quit => 38,
            ActionKind::ToggleSelect => 39,
            ActionKind::ClearSelection => 40,
            ActionKind::Select => 41,
            ActionKind::MoveTo => 42,
            ActionKind::CopyTo => 43,
            ActionKind::LargerThumbnails => 44,
            ActionKind::SmallerThumbnails => 45,
            ActionKind::PromptResult => 46,
            ActionKind::Compare => 47,
            ActionKind::EndCompare => 48,
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
            Action::MoveToTrash => ActionKind::MoveToTrash,
            Action::DeleteFile => ActionKind::DeleteFile,
            Action::ToggleSelect(_) => ActionKind::ToggleSelect,
            Action::ClearSelection => ActionKind::ClearSelection,
            Action::Compare => ActionKind::Compare,
            Action::EndCompare => ActionKind::EndCompare,
            Action::Select(_) => ActionKind::Select,
            Action::MoveTo => ActionKind::MoveTo,
            Action::CopyTo => ActionKind::CopyTo,
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
            Action::StepBy(_) => ActionKind::StepBy,
            Action::ToggleGallery => ActionKind::ToggleGallery,
            Action::LargerThumbnails => ActionKind::LargerThumbnails,
            Action::SmallerThumbnails => ActionKind::SmallerThumbnails,
            Action::ToggleInfo => ActionKind::ToggleInfo,
            Action::ConfirmResult { .. } => ActionKind::ConfirmResult,
            Action::PromptResult { .. } => ActionKind::PromptResult,
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

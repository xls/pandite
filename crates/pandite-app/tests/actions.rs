//! Every interface action has a handler, and edits stay in the view until
//! Save As.

mod support;

use pandite_app::app::Handled;
use pandite_core::Point;
use pandite_ui::{Action, ActionKind, BulkJob, BulkSource, OverwritePolicy, PixelRect, ResizeSpec};
use support::*;

/// One action of each kind. The match over [`ActionKind`] is exhaustive.
fn sample(kind: ActionKind, dir: &std::path::Path) -> Action {
    match kind {
        ActionKind::Open => Action::Open,
        ActionKind::SaveAs => Action::SaveAs,
        ActionKind::CopyPath => Action::CopyPath,
        ActionKind::CopyImage => Action::CopyImage,
        ActionKind::MoveToTrash => Action::MoveToTrash,
        ActionKind::DeleteFile => Action::DeleteFile,
        ActionKind::CopyText => Action::CopyText("some text".into()),
        ActionKind::RequestPaste => Action::RequestPaste,
        ActionKind::RotateLeft => Action::RotateLeft,
        ActionKind::RotateRight => Action::RotateRight,
        ActionKind::FlipHorizontal => Action::FlipHorizontal,
        ActionKind::FlipVertical => Action::FlipVertical,
        ActionKind::StartCrop => Action::StartCrop,
        ActionKind::ApplyCrop => Action::ApplyCrop(PixelRect {
            x: 0,
            y: 0,
            width: 2,
            height: 2,
        }),
        ActionKind::CancelCrop => Action::CancelCrop,
        ActionKind::OpenResizeDialog => Action::OpenResizeDialog,
        ActionKind::ApplyResize => Action::ApplyResize(ResizeSpec::Percent(200.0)),
        ActionKind::OpenBulkDialog => Action::OpenBulkDialog,
        ActionKind::PickBulkFiles => Action::PickBulkFiles,
        ActionKind::PickBulkFolder => Action::PickBulkFolder,
        ActionKind::PickBulkDestination => Action::PickBulkDestination,
        ActionKind::RunBulk => Action::RunBulk(BulkJob {
            source: BulkSource::Folder {
                path: dir.to_path_buf(),
                recursive: false,
            },
            resize: Some(ResizeSpec::Percent(50.0)),
            format: Some("png".into()),
            quality: 90,
            destination: dir.join("out"),
            overwrite: OverwritePolicy::Rename,
        }),
        ActionKind::CancelBulk => Action::CancelBulk,
        ActionKind::Undo => Action::Undo,
        ActionKind::FitToWindow => Action::FitToWindow,
        ActionKind::OneToOne => Action::OneToOne { anchor: None },
        ActionKind::ZoomIn => Action::ZoomIn,
        ActionKind::ZoomOut => Action::ZoomOut,
        ActionKind::ZoomAt => Action::ZoomAt {
            factor: 2.0,
            anchor: Point::new(100.0, 100.0),
        },
        ActionKind::PanBy => Action::PanBy { dx: 5.0, dy: 5.0 },
        ActionKind::ToggleFullscreen => Action::ToggleFullscreen,
        ActionKind::Next => Action::Next,
        ActionKind::Previous => Action::Previous,
        ActionKind::GoTo => Action::GoTo(0),
        ActionKind::StepBy => Action::StepBy(1),
        ActionKind::ToggleGallery => Action::ToggleGallery,
        ActionKind::ToggleInfo => Action::ToggleInfo,
        ActionKind::ConfirmResult => Action::ConfirmResult {
            id: 999,
            accepted: true,
        },
        ActionKind::Quit => Action::Quit,
        ActionKind::ToggleSelect => Action::ToggleSelect(0),
        ActionKind::ClearSelection => Action::ClearSelection,
        ActionKind::Compare => Action::Compare,
        ActionKind::EndCompare => Action::EndCompare,
        ActionKind::Select => Action::Select(1),
        ActionKind::MoveTo => Action::MoveTo,
        ActionKind::CopyTo => Action::CopyTo,
        ActionKind::LargerThumbnails => Action::LargerThumbnails,
        ActionKind::SmallerThumbnails => Action::SmallerThumbnails,
        ActionKind::PromptResult => Action::PromptResult {
            id: 1,
            accepted: true,
        },
    }
}

/// Compile-time guard: adding an `Action` variant breaks this match until
/// the test (and the handler it checks) covers it.
fn kind_of(action: &Action) -> ActionKind {
    match action {
        Action::Open => ActionKind::Open,
        Action::SaveAs => ActionKind::SaveAs,
        Action::CopyPath => ActionKind::CopyPath,
        Action::CopyImage => ActionKind::CopyImage,
        Action::MoveToTrash => ActionKind::MoveToTrash,
        Action::DeleteFile => ActionKind::DeleteFile,
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
        Action::ToggleInfo => ActionKind::ToggleInfo,
        Action::ConfirmResult { .. } => ActionKind::ConfirmResult,
        Action::Quit => ActionKind::Quit,
        Action::ToggleSelect(_) => ActionKind::ToggleSelect,
        Action::ClearSelection => ActionKind::ClearSelection,
        Action::Compare => ActionKind::Compare,
        Action::EndCompare => ActionKind::EndCompare,
        Action::Select(_) => ActionKind::Select,
        Action::MoveTo => ActionKind::MoveTo,
        Action::CopyTo => ActionKind::CopyTo,
        Action::LargerThumbnails => ActionKind::LargerThumbnails,
        Action::SmallerThumbnails => ActionKind::SmallerThumbnails,
        Action::PromptResult { .. } => ActionKind::PromptResult,
    }
}

#[test]
fn every_action_has_a_handler() {
    let (dir, paths) = folder("actions", &["a.png", "b.png", "c.png"], 8, 6);
    let mut h = Harness::new(Config {
        open: Some(paths[0].clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&paths[0]));
    assert!(h.wait_until(|a| a.count() == 3));
    *lock(&h.clipboard) = Some("pasted".into());

    let mut handled = Vec::new();
    for kind in ActionKind::ALL {
        let action = sample(kind, dir.path());
        assert_eq!(kind_of(&action), kind);
        let gallery = h.app().settings().gallery_visible;
        let size = h.app().settings().gallery_size;
        let info = h.app().info_visible();
        let fullscreen = h.app().fullscreen();
        let result = h.act(action);
        // Some actions have an effect that can be checked right here.
        match kind {
            ActionKind::ToggleGallery => {
                assert_ne!(h.app().settings().gallery_visible, gallery)
            }
            ActionKind::LargerThumbnails => {
                assert_eq!(Some(h.app().settings().gallery_size), size.larger())
            }
            ActionKind::SmallerThumbnails => {
                assert_eq!(Some(h.app().settings().gallery_size), size.smaller())
            }
            ActionKind::ToggleInfo => assert_ne!(h.app().info_visible(), info),
            ActionKind::ToggleFullscreen => assert_ne!(h.app().fullscreen(), fullscreen),
            ActionKind::CopyPath => {
                assert_eq!(h.clipboard().as_deref(), Some(paths[0].to_str().unwrap()))
            }
            ActionKind::CopyImage => {
                assert!(h.wait_until(|a| !a.copying_image()));
                assert_eq!(
                    h.images.images().len(),
                    1,
                    "the image reached the clipboard"
                );
            }
            ActionKind::MoveToTrash => {
                assert!(h.wait_until(|a| !a.deleting()));
                assert_eq!(h.disk.removals(), [("trash", "a.png".to_string())]);
                assert!(!paths[0].exists());
            }
            ActionKind::DeleteFile => {
                // It only asks; the answer comes from the dialog.
                assert_eq!(h.app().ui().mode(), pandite_ui::UiMode::Modal);
                assert_eq!(h.disk.removals().len(), 1);
            }
            ActionKind::CopyText => assert_eq!(h.clipboard().as_deref(), Some("some text")),
            ActionKind::RequestPaste => {
                assert_eq!(result, Handled::Paste("some text".into()))
            }
            ActionKind::Quit => assert!(h.app().exit_requested()),
            ActionKind::ConfirmResult => {
                assert_eq!(result, Handled::Ignored, "unknown confirmation id")
            }
            ActionKind::PromptResult => {
                assert_eq!(result, Handled::Ignored, "no prompt outside the sandbox")
            }
            ActionKind::ToggleSelect => {
                assert_eq!(h.app().selected_indices(), [0]);
                assert_eq!(h.app().selection().len(), 1);
            }
            ActionKind::ClearSelection => assert!(h.app().selection().is_empty()),
            ActionKind::Select => assert_eq!(h.app().selected_indices(), [1]),
            // The folder picker opens; the fake answers with a cancel.
            ActionKind::MoveTo => assert!(!h.app().moving()),
            ActionKind::CopyTo => {
                assert!(!h.app().copying_files());
                match lock(&h.dialogs).shown.last() {
                    Some(pandite_app::platform::DialogKind::PickFolder { title, .. }) => {
                        assert_eq!(title, "Copy 1 image to")
                    }
                    other => panic!("expected a folder picker, got {other:?}"),
                }
            }
            _ => {}
        }
        assert!(
            matches!(result, Handled::Done | Handled::Ignored | Handled::Paste(_)),
            "{kind:?}"
        );
        handled.push(kind);
        h.pump();
    }
    assert_eq!(handled.len(), 49);
    // The dialogs asked for: open, save, bulk files, bulk folder, bulk
    // destination, the folder of a move and the folder of a copy.
    assert_eq!(lock(&h.dialogs).shown.len(), 7);
    assert!(h.wait_until(|a| !a.bulk_running() && !a.is_busy()));
}

#[test]
fn edits_are_view_only_until_save() {
    let dir = TempDir::new("viewonly");
    let a = dir.join("a.png");
    write_png(&a, &numbered(4, 2));
    let before = std::fs::read(&a).unwrap();
    let modified = std::fs::metadata(&a).unwrap().modified().unwrap();
    let mut h = Harness::new(Config {
        open: Some(a.clone()),
        ..Config::default()
    });
    h.start();
    assert!(h.wait_shown(&a));
    assert!(!h.app().title().contains("edited"));

    h.act(Action::RotateRight);
    assert!(h.app().is_edited(), "the edit shows at once in the title");
    assert!(h.app().title().contains("(edited)"), "{}", h.app().title());
    assert!(h.wait_idle());
    let shown = h.app().shown_image().unwrap().clone();
    assert_eq!(
        (shown.width(), shown.height()),
        (2, 4),
        "the view is rotated"
    );
    // The file on disk did not change, and nothing else was written.
    assert_eq!(std::fs::read(&a).unwrap(), before);
    assert_eq!(std::fs::metadata(&a).unwrap().modified().unwrap(), modified);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    // Undo returns to the file's pixels.
    h.act(Action::Undo);
    assert!(!h.app().is_edited());
    assert_eq!(h.app().shown_image().unwrap().width(), 4);

    // Only Save As writes, and to the chosen file.
    h.act(Action::FlipHorizontal);
    assert!(h.wait_idle());
    let b = dir.join("b.png");
    h.answer_dialog(vec![b.clone()]);
    h.act(Action::SaveAs);
    assert!(h.wait_until(|app| !app.is_busy() && app.shown_path() == Some(b.as_path())));
    assert_eq!(
        std::fs::read(&a).unwrap(),
        before,
        "the original is untouched"
    );
    let (saved, _) = decode(&b);
    let mut expected = pixels(&numbered(4, 2));
    for row in expected.chunks_mut(4) {
        row.reverse();
    }
    assert_eq!(pixels(&saved), expected);
    assert!(!h.app().is_edited(), "saved edits are no longer pending");
    assert!(h.app().title().starts_with("b.png - Pandite"));
}

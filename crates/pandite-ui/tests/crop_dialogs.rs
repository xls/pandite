//! Gate G3: crop selection, the resize and bulk dialogs, text fields.

mod support;

use pandite_core::{DrawCmd, Point};
use pandite_ui::{
    Action, BulkJob, BulkSource, CursorShape, EncoderInfo, Key, Modifiers, OverwritePolicy,
    PixelRect, PointerButton, Progress, ResizeSpec, Theme, Ui, UiContext, UiMode,
};
use std::path::{Path, PathBuf};
use support::*;

/// A 1000 x 500 image shown 1:1 at window position (140, 108).
fn crop_ctx() -> UiContext<'static> {
    let ctx = with_image(base_ctx(), 1000, 500);
    let r = ctx.image.unwrap().screen_rect;
    assert_eq!((r.x, r.y, r.w, r.h), (140.0, 108.0, 1000.0, 500.0));
    ctx
}

/// Window position of image pixel (x, y).
fn ip(x: f32, y: f32) -> Point {
    Point::new(140.0 + x, 108.0 + y)
}

fn crop_rect(actions: &[Action]) -> PixelRect {
    match actions {
        [Action::ApplyCrop(r)] => *r,
        other => panic!("expected one ApplyCrop, got {other:?}"),
    }
}

#[test]
fn crop_drag_creates_clamped_selection() {
    let ctx = crop_ctx();
    let mut ui = Ui::new();
    frame(&mut ui, &ctx);
    assert_eq!(press_key(&mut ui, &ctx, Key::C), vec![Action::StartCrop]);
    assert_eq!(ui.mode(), UiMode::Crop);
    // From inside the image to far beyond its bottom-right corner.
    let out = drag(
        &mut ui,
        &ctx,
        ip(100.0, 50.0),
        Point::new(1275.0, 795.0),
        PointerButton::Left,
    );
    assert!(out.is_empty());
    let list = frame(&mut ui, &ctx);
    assert!(
        has_text(&list, "900 x 450"),
        "size readout: {:?}",
        texts(&list)
    );
    let theme = Theme::for_kind(ctx.theme);
    let dims = list
        .cmds
        .iter()
        .filter(|c| matches!(c, DrawCmd::Rect { color, .. } if *color == theme.crop_dim))
        .count();
    assert!(dims >= 2, "outside area dimmed");
    let r = crop_rect(&press_key(&mut ui, &ctx, Key::Enter));
    assert_eq!(
        r,
        PixelRect {
            x: 100,
            y: 50,
            width: 900,
            height: 450
        }
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // Starting left of and above the image clamps to its corner, and the
    // drag direction does not matter.
    press_key(&mut ui, &ctx, Key::C);
    drag(
        &mut ui,
        &ctx,
        ip(200.0, 100.0),
        Point::new(20.0, 60.0),
        PointerButton::Left,
    );
    let r = crop_rect(&press_key(&mut ui, &ctx, Key::Enter));
    assert_eq!(
        r,
        PixelRect {
            x: 0,
            y: 0,
            width: 200,
            height: 100
        }
    );

    // Selections follow image pixels, not screen pixels: at 50% zoom a
    // 100 pixel drag selects 200 image pixels.
    let mut half = with_image(base_ctx(), 2000, 1000);
    let info = half.image.as_mut().unwrap();
    info.screen_rect = pandite_core::Rect::new(140.0, 108.0, 1000.0, 500.0);
    let mut ui = Ui::new();
    press_key(&mut ui, &half, Key::C);
    drag(
        &mut ui,
        &half,
        ip(10.0, 10.0),
        ip(110.0, 60.0),
        PointerButton::Left,
    );
    let r = crop_rect(&press_key(&mut ui, &half, Key::Enter));
    assert_eq!((r.x, r.y, r.width, r.height), (20, 20, 200, 100));
}

#[test]
fn crop_handles_resize_selection() {
    let ctx = crop_ctx();
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    drag(
        &mut ui,
        &ctx,
        ip(100.0, 50.0),
        ip(600.0, 300.0),
        PointerButton::Left,
    );
    // Eight handles are drawn.
    let list = frame(&mut ui, &ctx);
    let theme = Theme::for_kind(ctx.theme);
    let handles = list
        .cmds
        .iter()
        .filter(|c| {
            matches!(c, DrawCmd::Rect { rect, color, .. }
                if *color == theme.crop_line && rect.w == 9.0 && rect.h == 9.0)
        })
        .count();
    assert_eq!(handles, 8);
    // The left side handle: cursor and drag.
    move_to(&mut ui, &ctx, ip(100.0, 175.0));
    assert_eq!(ui.cursor(), CursorShape::ResizeHorizontal);
    drag(
        &mut ui,
        &ctx,
        ip(100.0, 175.0),
        ip(200.0, 175.0),
        PointerButton::Left,
    );
    // The bottom-right corner handle.
    move_to(&mut ui, &ctx, ip(600.0, 300.0));
    assert_eq!(ui.cursor(), CursorShape::ResizeDiagonalDown);
    drag(
        &mut ui,
        &ctx,
        ip(600.0, 300.0),
        ip(700.0, 350.0),
        PointerButton::Left,
    );
    // Dragging inside moves the selection.
    move_to(&mut ui, &ctx, ip(400.0, 200.0));
    assert_eq!(ui.cursor(), CursorShape::Move);
    drag(
        &mut ui,
        &ctx,
        ip(400.0, 200.0),
        ip(450.0, 220.0),
        PointerButton::Left,
    );
    // Selection is now (250, 70) to (750, 370). Dragging the top edge past
    // the bottom flips it, clamped to the image.
    drag(
        &mut ui,
        &ctx,
        ip(500.0, 70.0),
        ip(500.0, 700.0),
        PointerButton::Left,
    );
    let r = crop_rect(&press_key(&mut ui, &ctx, Key::Enter));
    assert_eq!(
        r,
        PixelRect {
            x: 250,
            y: 370,
            width: 500,
            height: 130
        }
    );
    // Moving is clamped too: a selection dragged off the image stays in.
    press_key(&mut ui, &ctx, Key::C);
    drag(
        &mut ui,
        &ctx,
        ip(100.0, 100.0),
        ip(300.0, 200.0),
        PointerButton::Left,
    );
    drag(
        &mut ui,
        &ctx,
        ip(200.0, 150.0),
        Point::new(1270.0, 10.0),
        PointerButton::Left,
    );
    let r = crop_rect(&press_key(&mut ui, &ctx, Key::Enter));
    assert_eq!(
        r,
        PixelRect {
            x: 800,
            y: 0,
            width: 200,
            height: 100
        }
    );
}

#[test]
fn crop_enter_confirms_escape_cancels() {
    let ctx = crop_ctx();
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    assert!(
        press_key(&mut ui, &ctx, Key::Enter).is_empty(),
        "nothing to apply yet"
    );
    assert_eq!(ui.mode(), UiMode::Crop);
    // Editing commands are blocked while cropping.
    assert!(press_key(&mut ui, &ctx, Key::R).is_empty());
    assert!(press_key(&mut ui, &ctx, Key::Right).is_empty());
    drag(
        &mut ui,
        &ctx,
        ip(10.0, 10.0),
        ip(90.0, 90.0),
        PointerButton::Left,
    );
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::CancelCrop]
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // In full screen, Escape cancels the crop before leaving full screen.
    let mut full = ctx;
    full.fullscreen = true;
    press_key(&mut ui, &full, Key::C);
    drag(
        &mut ui,
        &full,
        ip(10.0, 10.0),
        ip(90.0, 90.0),
        PointerButton::Left,
    );
    assert_eq!(
        press_key(&mut ui, &full, Key::Escape),
        vec![Action::CancelCrop]
    );
    assert_eq!(
        press_key(&mut ui, &full, Key::Escape),
        vec![Action::ToggleFullscreen]
    );

    // Ctrl+A selects the whole image; arrows nudge; Enter applies.
    press_key(&mut ui, &ctx, Key::C);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    let r = crop_rect(&press_key(&mut ui, &ctx, Key::Enter));
    assert_eq!((r.x, r.y, r.width, r.height), (0, 0, 1000, 500));

    // The crop bar buttons do the same.
    press_key(&mut ui, &ctx, Key::C);
    drag(
        &mut ui,
        &ctx,
        ip(10.0, 10.0),
        ip(60.0, 40.0),
        PointerButton::Left,
    );
    press_key(&mut ui, &ctx, Key::Right);
    let r = crop_rect(&click_label(&mut ui, &ctx, "Apply"));
    assert_eq!((r.x, r.y, r.width, r.height), (11, 10, 50, 30));
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(
        click_label(&mut ui, &ctx, "Cancel"),
        vec![Action::CancelCrop]
    );
}

fn apply_spec(actions: &[Action]) -> ResizeSpec {
    match actions {
        [Action::ApplyResize(s)] => *s,
        other => panic!("expected one ApplyResize, got {other:?}"),
    }
}

#[test]
fn resize_dialog_percent_fit_exact() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(
        key(&mut ui, &ctx, Key::R, Modifiers::CTRL),
        vec![Action::OpenResizeDialog]
    );
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "Current size: 4000 x 3000 px"));
    assert!(has_text(&list, "New size: 2000 x 1500 px"));
    for mode in ["Percent", "Fit in box", "Exact size"] {
        assert!(text_rect_exact(&list, mode).is_some(), "{mode} offered");
    }
    // Percent: the field starts selected, so typing replaces it.
    type_text(&mut ui, &ctx, "25");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 1000 x 750 px"));
    assert_eq!(
        apply_spec(&press_key(&mut ui, &ctx, Key::Enter)),
        ResizeSpec::Percent(25.0)
    );
    assert_eq!(ui.mode(), UiMode::Normal);

    // Best fit in a box.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    click_label(&mut ui, &ctx, "Fit in box");
    type_text(&mut ui, &ctx, "800");
    press_key(&mut ui, &ctx, Key::Tab);
    type_text(&mut ui, &ctx, "800");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 800 x 600 px"));
    assert_eq!(
        apply_spec(&click_label(&mut ui, &ctx, "Resize")),
        ResizeSpec::Fit {
            width: 800,
            height: 800,
            enlarge: false
        }
    );

    // Exact size with the aspect lock on by default.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    click_label(&mut ui, &ctx, "Exact size");
    type_text(&mut ui, &ctx, "1000");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 1000 x 750 px"));
    assert_eq!(
        apply_spec(&press_key(&mut ui, &ctx, Key::Enter)),
        ResizeSpec::Exact {
            width: 1000,
            height: 750
        }
    );

    // Escape and Cancel close without resizing.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(click_label(&mut ui, &ctx, "Cancel").is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
}

#[test]
fn resize_dialog_aspect_lock() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    click_label(&mut ui, &ctx, "Exact size");
    type_text(&mut ui, &ctx, "2000");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 2000 x 1500 px"));
    // Editing the height follows the width the other way.
    press_key(&mut ui, &ctx, Key::Tab);
    type_text(&mut ui, &ctx, "600");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 800 x 600 px"));
    // Unlock, then change the height alone.
    click_label(&mut ui, &ctx, "Keep aspect ratio");
    click_label(&mut ui, &ctx, "600");
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "100");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 800 x 100 px"));
    // Locking again restores the ratio from the width.
    click_label(&mut ui, &ctx, "Keep aspect ratio");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 800 x 600 px"));
    click_label(&mut ui, &ctx, "Keep aspect ratio");
    click_label(&mut ui, &ctx, "600");
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "100");
    assert_eq!(
        apply_spec(&press_key(&mut ui, &ctx, Key::Enter)),
        ResizeSpec::Exact {
            width: 800,
            height: 100
        }
    );
}

#[test]
fn resize_dialog_rejects_bad_input() {
    let ctx = image_ctx();
    let theme = Theme::for_kind(ctx.theme);
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    // Letters are refused, leaving an empty field.
    type_text(&mut ui, &ctx, "abc");
    let list = frame(&mut ui, &ctx);
    let err = "Enter a percentage above 0 and at most 1000.";
    assert!(has_text(&list, err), "{:?}", texts(&list));
    assert_eq!(text_color(&list, err), Some(theme.error));
    assert!(press_key(&mut ui, &ctx, Key::Enter).is_empty());
    assert_eq!(ui.mode(), UiMode::Modal, "dialog stays open");
    // Zero and too large are refused; the disabled button does nothing.
    for bad in ["0", "5000", "0.0"] {
        key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
        type_text(&mut ui, &ctx, bad);
        assert!(has_text(&frame(&mut ui, &ctx), err), "{bad} accepted");
        assert!(press_key(&mut ui, &ctx, Key::Enter).is_empty());
        assert!(click_label(&mut ui, &ctx, "Resize").is_empty());
    }
    // A result above the size limit is refused too (a 20000 pixel wide
    // image at 500%).
    let big = with_image(base_ctx(), 20_000, 10_000);
    key(&mut ui, &big, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &big, "500");
    assert!(has_text(
        &frame(&mut ui, &big),
        "The result would exceed 65535 pixels."
    ));
    assert!(press_key(&mut ui, &big, Key::Enter).is_empty());
    // Exact sizes must be 1 to 65535.
    click_label(&mut ui, &ctx, "Exact size");
    type_text(&mut ui, &ctx, "70000");
    let side_err = "Enter a width and height from 1 to 65535.";
    assert!(has_text(&frame(&mut ui, &ctx), side_err));
    assert!(press_key(&mut ui, &ctx, Key::Enter).is_empty());
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "0");
    assert!(has_text(&frame(&mut ui, &ctx), side_err));
    // Fixing the value makes it valid again.
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "40");
    assert_eq!(
        apply_spec(&press_key(&mut ui, &ctx, Key::Enter)),
        ResizeSpec::Exact {
            width: 40,
            height: 30
        }
    );
}

const ENCODERS: &[EncoderInfo<'static>] = &[
    EncoderInfo {
        id: "png",
        name: "PNG",
        has_quality: false,
    },
    EncoderInfo {
        id: "jpeg",
        name: "JPEG",
        has_quality: true,
    },
    EncoderInfo {
        id: "webp",
        name: "WebP",
        has_quality: true,
    },
];

#[test]
fn bulk_dialog_collects_job() {
    let mut ctx = image_ctx();
    ctx.encoders = ENCODERS;
    ctx.folder = Some(Path::new("C:/photos"));
    let mut ui = Ui::new();
    assert_eq!(
        key(&mut ui, &ctx, Key::B, Modifiers::CTRL),
        vec![Action::OpenBulkDialog]
    );
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "No images chosen"));
    assert!(has_text(&list, "Choose images or a folder."));
    assert!(
        click_label(&mut ui, &ctx, "Run").is_empty(),
        "incomplete job"
    );

    // Files through a pick request the application answers.
    assert_eq!(
        click_label(&mut ui, &ctx, "Add files..."),
        vec![Action::PickBulkFiles]
    );
    ui.add_bulk_files(vec![
        PathBuf::from("C:/photos/a.jpg"),
        PathBuf::from("C:/photos/b.png"),
    ]);
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, "2 files"));
    // The destination is suggested next to the inputs.
    assert!(has_text(&list, "converted"));

    // A folder replaces the files.
    assert_eq!(
        click_label(&mut ui, &ctx, "Add folder..."),
        vec![Action::PickBulkFolder]
    );
    ui.set_bulk_folder(PathBuf::from("D:/in"));
    assert!(has_text(&frame(&mut ui, &ctx), "Folder: D:/in"));
    click_label(&mut ui, &ctx, "Include subfolders");

    // Size rule: percent.
    click_label(&mut ui, &ctx, "Percent");
    click_label(&mut ui, &ctx, "50");
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "40");

    // Output format from the encoder list.
    click_label(&mut ui, &ctx, "Same as source");
    let menu = frame(&mut ui, &ctx);
    for e in ENCODERS {
        assert!(
            text_rect_exact(&menu, e.name).is_some(),
            "{} listed",
            e.name
        );
    }
    click_label(&mut ui, &ctx, "WebP");
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "WebP").is_some());

    // Quality.
    click_label(&mut ui, &ctx, "90");
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    type_text(&mut ui, &ctx, "75");

    // Destination through a pick request.
    assert_eq!(
        click_label(&mut ui, &ctx, "Browse..."),
        vec![Action::PickBulkDestination]
    );
    ui.set_bulk_destination(PathBuf::from("D:/out"));

    // Overwrite choice.
    click_label(&mut ui, &ctx, "Rename");
    assert!(has_text(&frame(&mut ui, &ctx), "Ready."));
    let expected = BulkJob {
        source: BulkSource::Folder {
            path: PathBuf::from("D:/in"),
            recursive: true,
        },
        resize: Some(ResizeSpec::Percent(40.0)),
        format: Some("webp".into()),
        quality: 75,
        destination: PathBuf::from("D:/out"),
        overwrite: OverwritePolicy::Rename,
    };
    assert_eq!(
        click_label(&mut ui, &ctx, "Run"),
        vec![Action::RunBulk(expected.clone())]
    );

    // Progress view.
    let mut running = ctx;
    running.bulk = Some(Progress {
        label: "b.png",
        done: 3,
        total: 10,
    });
    let list = frame(&mut ui, &running);
    assert!(has_text(&list, "Converting 3 of 10"));
    assert_eq!(
        click_label(&mut ui, &running, "Stop"),
        vec![Action::CancelBulk]
    );
    // The dialog closes when the job ends.
    frame(&mut ui, &ctx);
    assert_eq!(ui.mode(), UiMode::Normal);

    // Reopening keeps the settings; "Replace" asks before running.
    key(&mut ui, &ctx, Key::B, Modifiers::CTRL);
    assert!(has_text(&frame(&mut ui, &ctx), "Folder: D:/in"));
    click_label(&mut ui, &ctx, "Replace");
    assert!(click_label(&mut ui, &ctx, "Run").is_empty());
    assert!(has_text(&frame(&mut ui, &ctx), "Replace existing files?"));
    let out = press_key(&mut ui, &ctx, Key::Enter);
    assert_eq!(
        out,
        vec![Action::RunBulk(BulkJob {
            overwrite: OverwritePolicy::Overwrite,
            ..expected
        })]
    );
}

#[test]
fn text_field_editing() {
    let ctx = image_ctx();
    let theme = Theme::for_kind(ctx.theme);
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert!(ui.wants_text_input());
    // Typing replaces the selected "50"; letters are dropped.
    type_text(&mut ui, &ctx, "1a23");
    assert!(has_text(&frame(&mut ui, &ctx), "New size: 4920 x 3690 px"));
    // Backspace removes the character before the caret.
    press_key(&mut ui, &ctx, Key::Backspace);
    let list = frame(&mut ui, &ctx);
    assert!(text_rect_exact(&list, "12").is_some());
    // A caret is drawn: a one pixel wide bar in the text color.
    assert!(list.cmds.iter().any(|c| matches!(
        c,
        DrawCmd::Rect { rect, color, .. } if rect.w == 1.0 && rect.h > 10.0 && *color == theme.text
    )));
    // Home, type, Right, Delete.
    press_key(&mut ui, &ctx, Key::Home);
    type_text(&mut ui, &ctx, "9");
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "912").is_some());
    press_key(&mut ui, &ctx, Key::Right);
    press_key(&mut ui, &ctx, Key::Delete);
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "91").is_some());
    press_key(&mut ui, &ctx, Key::Left);
    press_key(&mut ui, &ctx, Key::Left);
    type_text(&mut ui, &ctx, "5");
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "591").is_some());
    // Shift+End selects to the end; typing replaces the selection.
    press_key(&mut ui, &ctx, Key::Home);
    key(&mut ui, &ctx, Key::End, Modifiers::SHIFT);
    type_text(&mut ui, &ctx, "7");
    assert!(text_rect_exact(&frame(&mut ui, &ctx), "7").is_some());
    // Clipboard: copy asks the application, paste arrives as text.
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("7".into())]
    );
    assert_eq!(
        key(&mut ui, &ctx, Key::V, Modifiers::CTRL),
        vec![Action::RequestPaste]
    );
    type_text(&mut ui, &ctx, "33");
    // A click places the caret: after the first digit.
    let r = text_rect_exact(&frame(&mut ui, &ctx), "33").unwrap();
    click(&mut ui, &ctx, Point::new(r.x + r.w * 0.5, r.center().y));
    type_text(&mut ui, &ctx, ".");
    assert_eq!(
        apply_spec(&press_key(&mut ui, &ctx, Key::Enter)),
        ResizeSpec::Percent(3.3)
    );
    assert!(!ui.wants_text_input());
}

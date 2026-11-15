//! Escape quits unless a dialog is open; Ctrl+C copies the image, or the
//! selected text; Ctrl+Shift+C copies the file path.

mod support;

use pandite_core::{Point, Size};
use pandite_ui::{
    Action, BulkSource, Command, ConfirmRequest, InputEvent, Key, Loading, Modifiers,
    PointerButton, Ui, UiMode,
};
use std::path::PathBuf;
use support::*;

fn held(ui: &mut Ui, ctx: &pandite_ui::UiContext<'_>, k: Key, mods: Modifiers) -> Vec<Action> {
    ui.handle_event(
        InputEvent::KeyDown {
            key: k,
            mods,
            repeat: true,
        },
        ctx,
    )
}

#[test]
fn escape_quits_without_dialog() {
    let quit = vec![Action::Quit];
    // An image in a window.
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), quit);
    assert_eq!(ui.mode(), UiMode::Normal);
    // Full screen: Escape quits instead of leaving full screen.
    let mut full = ctx;
    full.fullscreen = true;
    assert_eq!(press_key(&mut Ui::new(), &full, Key::Escape), quit);
    // Nothing open at all.
    let mut empty = base_ctx();
    empty.count = 0;
    assert_eq!(press_key(&mut Ui::new(), &empty, Key::Escape), quit);
    // The first screen while the first image loads.
    let mut splash = base_ctx();
    splash.count = 1;
    splash.loading = Some(Loading {
        file_name: "first.jpg",
        elapsed: ms(900),
        progress: None,
        cloud: true,
        over_image: false,
    });
    assert_eq!(press_key(&mut Ui::new(), &splash, Key::Escape), quit);
    // A later image still loading over the one shown.
    let mut over = image_ctx();
    over.loading = Some(Loading {
        file_name: "next.jpg",
        elapsed: ms(20),
        progress: None,
        cloud: false,
        over_image: true,
    });
    assert_eq!(press_key(&mut Ui::new(), &over, Key::Escape), quit);
    // Crop mode, also during a drag of the selection.
    let mut ui = Ui::new();
    assert_eq!(press_key(&mut ui, &ctx, Key::C), vec![Action::StartCrop]);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), quit);
    let p = Point::new(500.0, 300.0);
    move_to(&mut ui, &ctx, p);
    press(&mut ui, &ctx, p, PointerButton::Left);
    move_to(&mut ui, &ctx, Point::new(700.0, 500.0));
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), quit);
    // The shortcut overlay is closed by Escape; then Escape quits.
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::F1);
    assert_eq!(ui.mode(), UiMode::Modal);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), quit);
    // A held Escape never quits; only a fresh press does.
    let mut ui = Ui::new();
    assert!(held(&mut ui, &ctx, Key::Escape, Modifiers::NONE).is_empty());
    // Ctrl+Q and Ctrl+W are the same quit.
    assert_eq!(key(&mut ui, &ctx, Key::Q, Modifiers::CTRL), quit);
    assert_eq!(key(&mut ui, &ctx, Key::W, Modifiers::CTRL), quit);
    // The shortcut overlay lists Escape with its new meaning.
    assert_eq!(Command::Cancel.shortcut_label().as_deref(), Some("Esc"));
    press_key(&mut ui, &ctx, Key::F1);
    let list = frame(&mut ui, &ctx);
    assert!(
        has_text(&list, "Close dialog or quit"),
        "{:?}",
        texts(&list)
    );
    assert!(!has_text(&list, "leave full screen"));
}

#[test]
fn escape_closes_dialog_first() {
    let ctx = image_ctx();
    // The resize dialog: Escape closes it, nothing else happens; the next
    // Escape quits.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert_eq!(ui.mode(), UiMode::Modal);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);

    // A held Escape that closed a dialog stops there.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    let first = ui.handle_event(
        InputEvent::KeyDown {
            key: Key::Escape,
            mods: Modifiers::NONE,
            repeat: false,
        },
        &ctx,
    );
    assert!(first.is_empty());
    for _ in 0..5 {
        assert!(held(&mut ui, &ctx, Key::Escape, Modifiers::NONE).is_empty());
    }
    assert_eq!(ui.mode(), UiMode::Normal);

    // The bulk dialog with its overwrite question on top: Escape answers
    // the question only, the next one closes the dialog, the third quits.
    let mut bulk_ctx = ctx;
    bulk_ctx.folder = None;
    let mut ui = Ui::new();
    key(&mut ui, &bulk_ctx, Key::B, Modifiers::CTRL);
    ui.add_bulk_files(vec![PathBuf::from("C:/a.jpg")]);
    ui.set_bulk_destination(PathBuf::from("C:/out"));
    // Choose "Replace" for existing files, then Run: the question opens.
    click_label(&mut ui, &bulk_ctx, "Replace");
    let run = click_label(&mut ui, &bulk_ctx, "Run");
    assert!(
        run.is_empty(),
        "the overwrite question comes first: {run:?}"
    );
    assert!(has_text(
        &frame(&mut ui, &bulk_ctx),
        "Replace existing files?"
    ));
    assert!(press_key(&mut ui, &bulk_ctx, Key::Escape).is_empty());
    let list = frame(&mut ui, &bulk_ctx);
    assert!(!has_text(&list, "Replace existing files?"));
    assert_eq!(ui.mode(), UiMode::Modal, "the bulk dialog stays");
    assert!(press_key(&mut ui, &bulk_ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
    assert_eq!(
        press_key(&mut ui, &bulk_ctx, Key::Escape),
        vec![Action::Quit]
    );
    // The bulk job description stays for the next time.
    key(&mut ui, &bulk_ctx, Key::B, Modifiers::CTRL);
    let out = click_label(&mut ui, &bulk_ctx, "Run");
    assert!(out.is_empty());
    assert!(press_key(&mut ui, &bulk_ctx, Key::Enter)
        .iter()
        .any(|a| matches!(a, Action::RunBulk(job) if job.source == BulkSource::Files(vec![PathBuf::from("C:/a.jpg")]))));

    // An application question: Escape answers no and closes it only.
    let mut ui = Ui::new();
    ui.show_confirm(ConfirmRequest::new(9, "Replace?", "Replace the file?"));
    assert_eq!(
        press_key(&mut ui, &ctx, Key::Escape),
        vec![Action::ConfirmResult {
            id: 9,
            accepted: false
        }]
    );
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);

    // A menu: Escape closes it.
    let mut ui = Ui::new();
    let p = Point::new(500.0, 300.0);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    assert_eq!(ui.mode(), UiMode::Modal);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);

    // A dialog over crop mode and a loading screen closes first too.
    let mut busy = ctx;
    busy.fullscreen = true;
    busy.loading = Some(Loading {
        file_name: "next.jpg",
        elapsed: ms(900),
        progress: None,
        cloud: false,
        over_image: true,
    });
    let mut ui = Ui::new();
    press_key(&mut ui, &busy, Key::C);
    press(&mut ui, &busy, p, PointerButton::Right);
    release(&mut ui, &busy, p, PointerButton::Right);
    assert!(press_key(&mut ui, &busy, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Crop);
    assert_eq!(press_key(&mut ui, &busy, Key::Escape), vec![Action::Quit]);
    // Ctrl+Q quits from inside a dialog, as before.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert_eq!(
        key(&mut ui, &ctx, Key::Q, Modifiers::CTRL),
        vec![Action::Quit]
    );
}

#[test]
fn ctrl_c_copies_image() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
    // A held Ctrl+C copies once.
    assert!(held(&mut ui, &ctx, Key::C, Modifiers::CTRL).is_empty());
    // Also in full screen and while the next image loads.
    let mut busy = ctx;
    busy.fullscreen = true;
    busy.loading = Some(Loading {
        file_name: "next.jpg",
        elapsed: ms(900),
        progress: None,
        cloud: false,
        over_image: true,
    });
    assert_eq!(
        key(&mut Ui::new(), &busy, Key::C, Modifiers::CTRL),
        vec![Action::CopyImage]
    );
    // The context menu offers it with its key.
    let p = Point::new(500.0, 300.0);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    let list = frame(&mut ui, &ctx);
    assert!(text_rect_exact(&list, "Copy image").is_some());
    assert_eq!(
        Command::CopyImage.shortcut_label().as_deref(),
        Some("Ctrl+C")
    );
    assert_eq!(
        click_label(&mut ui, &ctx, "Copy image"),
        vec![Action::CopyImage]
    );
    // Nothing to copy without an image, and nothing in crop mode (the
    // selection is not applied yet).
    let mut empty = base_ctx();
    empty.count = 0;
    assert!(key(&mut Ui::new(), &empty, Key::C, Modifiers::CTRL).is_empty());
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Crop);
    assert!(key(&mut ui, &ctx, Key::C, Modifiers::CTRL).is_empty());
    // In a text field Ctrl+C copies the field's selected text.
    let mut ui = Ui::new();
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    key(&mut ui, &ctx, Key::A, Modifiers::CTRL);
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL),
        vec![Action::CopyText("50".into())]
    );
    // The help overlay lists both copies.
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::F1);
    let mut wide = ctx;
    wide.window_size = Size::new(1600.0, 1000.0);
    let list = frame(&mut ui, &wide);
    assert!(has_text(&list, "Copy image"));
    assert!(has_text(&list, "Copy file path"));
    assert!(has_text(&list, "Ctrl+Shift+C"));
}

#[test]
fn ctrl_shift_c_copies_path() {
    let ctx = image_ctx();
    let mut ui = Ui::new();
    assert_eq!(
        key(&mut ui, &ctx, Key::C, Modifiers::CTRL_SHIFT),
        vec![Action::CopyPath]
    );
    assert!(held(&mut ui, &ctx, Key::C, Modifiers::CTRL_SHIFT).is_empty());
    // The toolbar button stays and names the new key.
    assert_eq!(
        Command::CopyPath.shortcut_label().as_deref(),
        Some("Ctrl+Shift+C")
    );
    assert_eq!(
        Command::CopyPath.tooltip(),
        "Copy file path  (Ctrl+Shift+C)"
    );
    // The context menu entry shows the key too.
    let p = Point::new(500.0, 300.0);
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    let list = frame(&mut ui, &ctx);
    assert!(text_rect_exact(&list, "Ctrl+Shift+C").is_some());
    assert_eq!(
        click_label(&mut ui, &ctx, "Copy file path"),
        vec![Action::CopyPath]
    );
    // Nothing to copy without an image.
    let mut empty = base_ctx();
    empty.count = 0;
    assert!(key(&mut Ui::new(), &empty, Key::C, Modifiers::CTRL_SHIFT).is_empty());
}

#[test]
fn q_quits_and_is_a_letter_in_dialogs() {
    let quit = vec![Action::Quit];
    let ctx = image_ctx();
    // In the view and in crop mode, Q closes the program like Escape.
    assert_eq!(press_key(&mut Ui::new(), &ctx, Key::Q), quit);
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Crop);
    assert_eq!(press_key(&mut ui, &ctx, Key::Q), quit);
    // Ctrl+Q still quits.
    assert_eq!(key(&mut Ui::new(), &ctx, Key::Q, Modifiers::CTRL), quit);
    // In a question, a plain Q does not quit; Ctrl+Q does.
    let mut ui = Ui::new();
    ui.show_confirm(ConfirmRequest::new(9, "Replace?", "Replace the file?"));
    assert_eq!(ui.mode(), UiMode::Modal);
    assert!(!press_key(&mut ui, &ctx, Key::Q).contains(&Action::Quit));
    assert_eq!(ui.mode(), UiMode::Modal);
    assert_eq!(key(&mut ui, &ctx, Key::Q, Modifiers::CTRL), quit);
}

//! The information panel: drawn from the context rows, anchored to the
//! bottom left of the content area above the help hint, inert to pointer
//! gestures, hidden in crop mode, and never in the way of Escape.

mod support;

use pandite_ui::{Action, InfoRow, Key, PointerButton, Ui, UiMode};
use support::*;

fn rows() -> Vec<InfoRow> {
    vec![
        InfoRow::new("File", "photo.jpg"),
        InfoRow::new("Pixels", "4000 x 3000"),
        InfoRow::new("Format", "JPEG"),
    ]
}

#[test]
fn info_panel_draws_rows_and_stays_out_of_the_way() {
    let rows = rows();
    let mut ctx = image_ctx();
    ctx.info_visible = true;
    ctx.info = &rows;
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    let r = ui.info_panel_rect().expect("panel drawn");
    let content = ui.content_rect(&ctx);
    assert_eq!(r.x, content.x + 16.0);
    // Above the help hint in the bottom left corner.
    let hint = text_rect_exact(&list, "F1 / ? help").expect("the help hint shows");
    assert!(r.bottom() < hint.y, "{r:?} above {hint:?}");
    assert!(content.bottom() - r.bottom() < 40.0);
    // The toolbar shows the name and size too: look inside the panel.
    for t in ["File", "photo.jpg", "4000 x 3000", "JPEG"] {
        let inside = list.cmds.iter().any(|c| {
            matches!(c, pandite_core::DrawCmd::Text { pos, text, .. }
                if text == t && r.contains(*pos))
        });
        assert!(inside, "{t} inside the panel");
    }

    // The wheel and a click over the panel do not reach the image.
    let p = r.center();
    assert!(wheel(&mut ui, &ctx, p, 1.0).is_empty());
    assert!(click(&mut ui, &ctx, p).is_empty());
    // A right click opens the context menu as anywhere else.
    press(&mut ui, &ctx, p, PointerButton::Right);
    release(&mut ui, &ctx, p, PointerButton::Right);
    assert_eq!(ui.mode(), UiMode::Modal);
    press_key(&mut ui, &ctx, Key::Escape);
    assert_eq!(ui.mode(), UiMode::Normal);

    // I asks to hide it; Escape quits with it on screen.
    assert_eq!(press_key(&mut ui, &ctx, Key::I), vec![Action::ToggleInfo]);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);

    // Crop mode hides it until the crop ends.
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Crop);
    frame(&mut ui, &ctx);
    assert_eq!(ui.info_panel_rect(), None);
    press_key(&mut ui, &ctx, Key::C);
    frame(&mut ui, &ctx);
    assert!(ui.info_panel_rect().is_some());

    // Off, or without rows: nothing.
    ctx.info_visible = false;
    frame(&mut ui, &ctx);
    assert_eq!(ui.info_panel_rect(), None);
    assert_eq!(
        wheel(&mut ui, &ctx, p, 1.0).len(),
        1,
        "the image zooms there again"
    );
    ctx.info_visible = true;
    ctx.info = &[];
    frame(&mut ui, &ctx);
    assert_eq!(ui.info_panel_rect(), None);
}

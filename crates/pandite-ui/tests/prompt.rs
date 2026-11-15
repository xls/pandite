//! The prompt: a message with one button and a close mark at the bottom
//! of the content area. The button, the close mark and Escape answer it;
//! everything else acts as without it, and without one nothing changes.

mod support;

use pandite_ui::{Action, Icon, Key, Modifiers, PointerButton, Prompt, Ui, UiContext, UiMode};
use support::*;

const MESSAGE: &str = "Allow Pandite to show the other images in this folder";

fn prompted() -> UiContext<'static> {
    let mut ctx = image_ctx();
    ctx.prompt = Some(Prompt {
        id: 4,
        message: MESSAGE,
        button: "Allow...",
    });
    ctx
}

fn answer(accepted: bool) -> Vec<Action> {
    vec![Action::PromptResult { id: 4, accepted }]
}

#[test]
fn prompt_buttons_and_escape_answer_it() {
    let ctx = prompted();
    let mut ui = Ui::new();
    let list = frame(&mut ui, &ctx);
    assert!(has_text(&list, MESSAGE), "{:?}", texts(&list));
    let content = ui.content_rect(&ctx);
    let button = text_rect_exact(&list, "Allow...").expect("button drawn");
    assert!(
        button.bottom() < content.bottom(),
        "inside the content area"
    );

    // The button accepts.
    assert_eq!(click(&mut ui, &ctx, button.center()), answer(true));

    // The close mark dismisses. It is the only close mark on screen.
    let closes = icon_draws(&list, Icon::Close);
    assert_eq!(closes.len(), 1);
    let close = closes[0].0.center();
    assert_eq!(click(&mut ui, &ctx, close), answer(false));

    // A press that leaves the button before the release answers nothing.
    move_to(&mut ui, &ctx, button.center());
    press(&mut ui, &ctx, button.center(), PointerButton::Left);
    move_to(&mut ui, &ctx, content.center());
    assert!(release(&mut ui, &ctx, content.center(), PointerButton::Left).is_empty());

    // The message itself is inert: no click, no zoom reaches the image.
    let text = text_rect(&list, MESSAGE).unwrap().center();
    assert!(click(&mut ui, &ctx, text).is_empty());
    assert!(wheel(&mut ui, &ctx, text, 1.0).is_empty());
    // A right click there opens the context menu as anywhere else.
    press(&mut ui, &ctx, text, PointerButton::Right);
    release(&mut ui, &ctx, text, PointerButton::Right);
    assert_eq!(ui.mode(), UiMode::Modal);

    // Escape closes the menu first, then dismisses the prompt instead of
    // quitting.
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), answer(false));

    // A dialog takes Escape before the prompt does.
    key(&mut ui, &ctx, Key::R, Modifiers::CTRL);
    assert_eq!(ui.mode(), UiMode::Modal);
    assert!(press_key(&mut ui, &ctx, Key::Escape).is_empty());
    assert_eq!(ui.mode(), UiMode::Normal);

    // Other keys act as without the prompt.
    assert_eq!(press_key(&mut ui, &ctx, Key::Right), vec![Action::Next]);
}

#[test]
fn without_a_prompt_nothing_changes() {
    let plain = image_ctx();
    let ctx = prompted();
    let mut ui = Ui::new();
    let before = frame(&mut ui, &plain);
    assert!(!has_text(&before, MESSAGE));
    assert!(icon_draws(&before, Icon::Close).is_empty());
    assert_eq!(press_key(&mut ui, &plain, Key::Escape), vec![Action::Quit]);

    // The prompt leaves no trace once the context drops it: the spot of
    // its button is the image again, and Escape quits.
    let list = frame(&mut ui, &ctx);
    let button = text_rect_exact(&list, "Allow...").unwrap().center();
    let after = frame(&mut ui, &plain);
    assert_eq!(texts(&after), texts(&before));
    assert!(click(&mut ui, &plain, button).is_empty());
    assert_eq!(
        kinds(&wheel(&mut ui, &plain, button, 1.0)),
        [pandite_ui::ActionKind::ZoomAt]
    );
    assert_eq!(press_key(&mut ui, &plain, Key::Escape), vec![Action::Quit]);
}

#[test]
fn prompt_hides_in_crop_mode_and_sits_above_the_notice() {
    let mut ctx = prompted();
    let mut ui = Ui::new();
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Crop);
    let list = frame(&mut ui, &ctx);
    assert!(!has_text(&list, MESSAGE), "crop mode hides it");
    assert_eq!(press_key(&mut ui, &ctx, Key::Escape), vec![Action::Quit]);
    press_key(&mut ui, &ctx, Key::C);
    assert_eq!(ui.mode(), UiMode::Normal);

    // With a notice, both show: the notice at the bottom, the prompt
    // above it.
    ctx.notice = Some("Cannot open b.png: broken");
    let list = frame(&mut ui, &ctx);
    let notice = text_rect(&list, "Cannot open").expect("notice drawn");
    let message = text_rect(&list, MESSAGE).expect("prompt drawn");
    assert!(message.bottom() < notice.y, "{message:?} above {notice:?}");

    // A narrow window shortens the message and keeps both controls.
    let mut narrow = ctx;
    narrow.window_size = pandite_core::Size::new(360.0, 600.0);
    let list = frame(&mut ui, &narrow);
    assert!(!has_text(&list, MESSAGE));
    let button = text_rect_exact(&list, "Allow...").expect("button kept");
    assert!(button.right() <= 360.0);
    assert_eq!(icon_draws(&list, Icon::Close).len(), 1);
    assert_eq!(click(&mut ui, &narrow, button.center()), answer(true));
}

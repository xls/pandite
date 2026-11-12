//! The confirm dialog (for example "Replace the original file?").

use crate::context::UiContext;
use crate::form::{ButtonStyle, FormBuilder, FormEvent, FormHost, Id, Tone};
use crate::text_field::TextField;

pub(crate) const BTN_NO: Id = 1;
pub(crate) const BTN_YES: Id = 2;

/// A question the application asks through [`crate::Ui::show_confirm`].
/// The answer arrives as [`crate::Action::ConfirmResult`] with the same id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmRequest {
    /// Chosen by the application to match the answer.
    pub id: u64,
    /// Short title, for example "Replace file?".
    pub title: String,
    /// The question in full. Long text wraps over up to four lines.
    pub message: String,
    /// Label of the confirming button, for example "Replace".
    pub confirm_label: String,
    /// Label of the cancelling button, for example "Cancel".
    pub cancel_label: String,
}

impl ConfirmRequest {
    /// A request with "OK" and "Cancel" buttons.
    pub fn new(id: u64, title: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            message: message.into(),
            confirm_label: "OK".into(),
            cancel_label: "Cancel".into(),
        }
    }
}

/// Who asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfirmOwner {
    /// The application; answer with `ConfirmResult`.
    App(u64),
    /// The bulk dialog's overwrite check; accept starts the job.
    BulkOverwrite,
}

/// An open confirm dialog.
#[derive(Clone, Debug)]
pub(crate) struct ConfirmDialog {
    pub owner: ConfirmOwner,
    pub request: ConfirmRequest,
}

/// The user's answer, when given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    Yes,
    No,
}

impl ConfirmDialog {
    pub fn on_event(&self, event: &FormEvent) -> Option<Answer> {
        match event {
            FormEvent::Activate(BTN_YES) | FormEvent::Submit => Some(Answer::Yes),
            FormEvent::Activate(BTN_NO) | FormEvent::Cancel => Some(Answer::No),
            _ => None,
        }
    }
}

impl FormHost for ConfirmDialog {
    fn build(&self, _ctx: &UiContext<'_>, b: &mut FormBuilder<'_>) {
        b.title(self.request.title.clone());
        b.paragraph(&self.request.message, Tone::Normal, 4);
        b.space(10.0);
        b.buttons_row(
            &[
                (
                    BTN_NO,
                    &self.request.cancel_label,
                    ButtonStyle::Secondary,
                    true,
                ),
                (
                    BTN_YES,
                    &self.request.confirm_label,
                    ButtonStyle::Primary,
                    true,
                ),
            ],
            true,
        );
    }

    fn field(&self, _id: Id) -> Option<&TextField> {
        None
    }

    fn field_mut(&mut self, _id: Id) -> Option<&mut TextField> {
        None
    }
}

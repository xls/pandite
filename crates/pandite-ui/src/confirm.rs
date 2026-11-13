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
    /// The cancelling button is the default: it has the focus and the
    /// accent color when the dialog opens, and Enter outside a button
    /// cancels. Set it for a question whose yes cannot be undone.
    pub cancel_is_default: bool,
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
            cancel_is_default: false,
        }
    }

    /// The button that has the focus when the dialog opens.
    pub(crate) fn default_button(&self) -> Id {
        if self.cancel_is_default {
            BTN_NO
        } else {
            BTN_YES
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
            FormEvent::Activate(BTN_YES) => Some(Answer::Yes),
            FormEvent::Submit if !self.request.cancel_is_default => Some(Answer::Yes),
            FormEvent::Activate(BTN_NO) | FormEvent::Submit | FormEvent::Cancel => Some(Answer::No),
            _ => None,
        }
    }
}

impl FormHost for ConfirmDialog {
    fn build(&self, _ctx: &UiContext<'_>, b: &mut FormBuilder<'_>) {
        b.title(self.request.title.clone());
        b.paragraph(&self.request.message, Tone::Normal, 4);
        b.space(10.0);
        // The default button takes the accent color.
        let (no_style, yes_style) = if self.request.cancel_is_default {
            (ButtonStyle::Primary, ButtonStyle::Secondary)
        } else {
            (ButtonStyle::Secondary, ButtonStyle::Primary)
        };
        b.buttons_row(
            &[
                (BTN_NO, &self.request.cancel_label, no_style, true),
                (BTN_YES, &self.request.confirm_label, yes_style, true),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_default_answers_no_to_enter() {
        let mut request = ConfirmRequest::new(1, "Delete?", "Delete a.png for good?");
        let ask = |request: &ConfirmRequest| ConfirmDialog {
            owner: ConfirmOwner::App(request.id),
            request: request.clone(),
        };
        assert_eq!(request.default_button(), BTN_YES);
        assert_eq!(
            ask(&request).on_event(&FormEvent::Submit),
            Some(Answer::Yes)
        );
        request.cancel_is_default = true;
        let dialog = ask(&request);
        assert_eq!(request.default_button(), BTN_NO);
        assert_eq!(dialog.on_event(&FormEvent::Submit), Some(Answer::No));
        assert_eq!(dialog.on_event(&FormEvent::Cancel), Some(Answer::No));
        assert_eq!(
            dialog.on_event(&FormEvent::Activate(BTN_NO)),
            Some(Answer::No)
        );
        assert_eq!(
            dialog.on_event(&FormEvent::Activate(BTN_YES)),
            Some(Answer::Yes),
            "only the confirming button itself says yes"
        );
    }
}

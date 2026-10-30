//! What a notification carries back to the app when it is clicked, and the one question the wall's
//! notifications ask before they are raised — the rest of `ToastArguments.cs` (`For`, `IsNews`).
//! The PARSE is [`notification_target::parse`](crate::notification_target::parse): this writes exactly what that reads.
//!
//! On Windows a toast's arguments are strings (`chat=42`, `board=1&family=3`). On Linux a
//! notification carries an ACTION and a typed TARGET, so `For` answers the action (without the
//! `app.` prefix, as [`notification_target`](crate::notification_target)'s constants are) and the target the shell hands to
//! `GNotification::set_default_action_and_target_value("app." + action, target)`: an `x` (i64) for
//! a chat, an `x` for the wall's family, and NO target for a wall whose family is not known — which
//! [`notification_target::parse`](crate::notification_target::parse) reads back as the wall shown.
//!
//! A NOTE IS NEWS BY THE BADGE'S OWN RULE (`fc_text::board::is_unread`): the badge and the
//! notification answer the same question and must not disagree. A tombstone is never news.

use fc_core::protocol::NoteDto;
use fc_text::board::{Marks, is_unread};

use crate::notification_target::{OPEN_BOARD, OPEN_CHAT, TargetValue};
use crate::notifications::Toast;

/// The action a notification raises when clicked, and its target (`ToastArguments.For`'s answer).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToastArguments {
    /// [`OPEN_CHAT`] or [`OPEN_BOARD`] — without the `app.` prefix.
    pub action: &'static str,
    /// The chat's id, or the wall's family's; `None` for a wall whose family is not known.
    pub target: Option<TargetValue>,
}

impl ToastArguments {
    /// The action as the shell registers it: `app.open-chat`, `app.open-board`.
    pub fn detailed_action(&self) -> String {
        format!("app.{}", self.action)
    }
}

/// What a click on this toast opens (`ToastArguments.For`): its chat, or its family's wall, or the
/// wall shown.
pub fn for_toast(toast: &Toast) -> ToastArguments {
    match (toast.chat_id, toast.family_id) {
        (Some(chat_id), _) => ToastArguments {
            action: OPEN_CHAT,
            target: Some(TargetValue::Int64(chat_id)),
        },
        (None, Some(family_id)) => ToastArguments {
            action: OPEN_BOARD,
            target: Some(TargetValue::Int64(family_id)),
        },
        (None, None) => ToastArguments {
            action: OPEN_BOARD,
            target: None,
        },
    }
}

/// Whether a note is worth a notification: what the badge would count, and never a tombstone
/// (`ToastArguments.IsNews`).
pub fn is_news(note: &NoteDto, marks: Marks) -> bool {
    !note.deleted && is_unread(note.id, note.content_seq, marks)
}

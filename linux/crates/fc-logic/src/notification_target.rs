//! What a clicked notification opens (shell-design §14.6, §15.3–15.4, Appendix B item 6) — the
//! parsing rules of `ToastArguments.cs`, for a `GNotification`'s action and its `GVariant` target.
//!
//! On Windows a toast carries `chat=42` or `board=1&family=3` back as strings. On Linux a
//! notification carries an ACTION and a typed TARGET: `app.open-chat` with the chat id as an `x`
//! (i64), `app.open-board` with the family's id as an `x`, and on a ringing call
//! `app.answer-call` / `app.decline-call` with the call id as an `s`. The shell reads the variant
//! into [`TargetValue`] (`Variant::get::<i64>()`, `::<String>()`) and asks [`parse`].
//!
//! AN ARGUMENT IS UNTRUSTED INPUT. It comes back from the desktop — through D-Bus activation even
//! when the app had quit — and a click on a notification the app never raised, or one an older
//! build wrote differently, must open nothing rather than panic or open the wrong chat. Anything
//! that is not a positive id is ignored: a chat id must be a positive number; a family id that is
//! not one is no family, and the wall shown opens; a call id must be there at all — whether it
//! names the call ringing is the call engine's question, which applies a call action only to the
//! call it holds (docs/protocol.md, "Identity: the client names the call").
//!
//! `ToastArguments.For` and `IsNews` are `toast_arguments` (`for_toast`, `is_news`), which writes
//! exactly what [`parse`] reads.

/// The four actions a notification of this app carries, without the `app.` prefix the shell
/// registers them under (`gio::SimpleAction` names).
pub const OPEN_CHAT: &str = "open-chat";
pub const OPEN_BOARD: &str = "open-board";
pub const ANSWER_CALL: &str = "answer-call";
pub const DECLINE_CALL: &str = "decline-call";

/// A notification's target as the shell reads it out of the `GVariant`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TargetValue {
    /// An `x` — what this app writes for a chat or a family.
    Int64(i64),
    /// An `s` — what this app writes for a call. (A chat or family id that arrives as text is
    /// read too, digits only, as Windows reads its arguments: tolerance for an older or another
    /// writer, never a different meaning.)
    Str(String),
    /// Any other type: nothing this app wrote.
    Other,
}

/// What a clicked chat or wall notification opens: a chat, or — with no chat — the wall; and, for
/// the wall, WHOSE (docs/protocol.md, "Several families": a notification opens the app in its
/// family). The C#'s `ToastTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToastTarget {
    pub chat_id: Option<i64>,
    pub family_id: Option<i64>,
}

impl ToastTarget {
    pub fn chat(chat_id: i64) -> Self {
        ToastTarget {
            chat_id: Some(chat_id),
            family_id: None,
        }
    }

    pub fn board(family_id: Option<i64>) -> Self {
        ToastTarget {
            chat_id: None,
            family_id,
        }
    }

    pub fn is_board(&self) -> bool {
        self.chat_id.is_none()
    }
}

/// What a notification's action asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NotificationTarget {
    /// Open a chat, or a family's wall.
    Open(ToastTarget),
    /// Answer the ringing call with this id (the Call window comes up).
    Answer(String),
    /// Decline it (`call_end` `decline`, nothing shown).
    Decline(String),
}

/// The action and its target, or `None` for anything this app did not write.
pub fn parse(action: &str, target: Option<&TargetValue>) -> Option<NotificationTarget> {
    let action = action.strip_prefix("app.").unwrap_or(action);
    match action {
        OPEN_CHAT => {
            positive_id(target?).map(|chat_id| NotificationTarget::Open(ToastTarget::chat(chat_id)))
        }
        // Whose wall, when it says — a family id that is not a positive number is no family, and
        // the wall shown opens.
        OPEN_BOARD => Some(NotificationTarget::Open(ToastTarget::board(
            target.and_then(positive_id),
        ))),
        ANSWER_CALL => call_id(target?).map(NotificationTarget::Answer),
        DECLINE_CALL => call_id(target?).map(NotificationTarget::Decline),
        _ => None,
    }
}

/// A positive id: an `x` above zero, or text that is nothing but ASCII digits (.NET's
/// `NumberStyles.None` — no sign, no space, no separator) naming one.
pub fn positive_id(target: &TargetValue) -> Option<i64> {
    let id = match target {
        TargetValue::Int64(id) => *id,
        TargetValue::Str(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
            text.parse().ok()?
        }
        _ => return None,
    };
    (id > 0).then_some(id)
}

/// A call id: text, and not empty.
fn call_id(target: &TargetValue) -> Option<String> {
    match target {
        TargetValue::Str(text) if !text.is_empty() => Some(text.clone()),
        _ => None,
    }
}

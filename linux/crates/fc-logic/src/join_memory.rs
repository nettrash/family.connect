//! Whose join request this device was waiting on, and the composer's two assistant doors
//! (`JoinMemory.cs`).

use std::cell::Cell;

use fc_core::protocol::AssistantDto;

use crate::assistant_consent;
use crate::groups::chat_kinds;

/// Whose join request this device was waiting on (`IAwaitingJoin`). A refusal is never said by the
/// server — the request just vanishes from `GET /me` — so only a client that REMEMBERS waiting can
/// tell "declined" from "never asked" (web `session.rs` `awaiting_join`, ios
/// `AppSettings.joinPending`), and a relaunch must not forget it: the shell keeps it in its
/// settings file.
pub trait AwaitingJoin {
    fn user_id(&self) -> Option<i64>;

    fn set_user_id(&self, user_id: Option<i64>);
}

/// For a session that need not outlive its process — the tests (`MemoryAwaitingJoin`).
#[derive(Debug, Default)]
pub struct MemoryAwaitingJoin(Cell<Option<i64>>);

impl MemoryAwaitingJoin {
    pub fn new() -> Self {
        MemoryAwaitingJoin::default()
    }
}

impl AwaitingJoin for MemoryAwaitingJoin {
    fn user_id(&self) -> Option<i64> {
        self.0.get()
    }

    fn set_user_id(&self, user_id: Option<i64>) {
        self.0.set(user_id);
    }
}

/// The composer's two assistant doors (web `composer.rs` `offers_ai` and `can_draw`, the Mac's
/// sparkles and paintbrush): "Ask the assistant" appends `@ai` in the family chat or a group
/// (docs/protocol.md, "Groups") of a server that has an assistant; "Ask for a picture" puts
/// `/draw` first in the assistant's own chat when it can draw. Neither while a message is being
/// edited — an edit rewrites what was said, and a request cannot be added to it.
///
/// The C#'s named tuple `(AskAssistant, AskPicture)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AssistantButtons {
    pub ask_assistant: bool,
    pub ask_picture: bool,
}

impl AssistantButtons {
    /// `AssistantButtons.Offered(chatKind, assistant, editing)`.
    pub fn offered(
        chat_kind: Option<&str>,
        assistant: Option<&AssistantDto>,
        editing: bool,
    ) -> AssistantButtons {
        // A server that names no processor offers no assistant at all: a consent screen with a
        // hole where the recipient goes is not consent (docs/protocol.md, "Consenting to the
        // assistant"), so the doors that would type `@ai` are not there either.
        let named = assistant.is_some_and(|assistant| {
            assistant_consent::is_available(assistant.processor.as_deref())
        });
        AssistantButtons {
            ask_assistant: chat_kinds::is_shared(chat_kind) && named && !editing,
            ask_picture: chat_kind == Some(chat_kinds::ASSISTANT)
                && assistant.is_some_and(|assistant| assistant.images)
                && named
                && !editing,
        }
    }
}

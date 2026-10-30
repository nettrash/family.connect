//! The assistant's answers while they are being written: the text streamed so far, and the ones
//! that stopped early (docs/protocol.md, `ai_delta` and `ai_error`; `AssistantAnswers.cs`).
//!
//! COSMETIC, AND NEVER IN THE CACHE. The finished answer arrives as `message_edited` and replaces
//! all of it, so streamed text lives here and is drawn after the held body — and a delta for a row
//! already finished (it has an `edit_seq`), or for a row this device does not hold, is late and
//! ignored.
//!
//! A FINISHED ANSWER IS A FINISHED ANSWER, whatever went before it: an edit clears both the text
//! and the failure, and a row that carries an `edit_seq` is never drawn as failed, however it
//! reached the cache.
//!
//! A FAILURE REMEMBERS ITS SENTENCE (docs/protocol.md, `ai_error`'s `reason`): a refusal by the
//! provider's own filter says so, and anything else says "ask again" — for exactly as long as the
//! row is failed, so a redraw never turns one sentence into the other.
//!
//! One thread (the GLib main context), so no lock: the C#'s `lock` guarded the socket's thread.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use fc_core::protocol::{AiErrorReason, MessageDto};
use fc_core::rt::Event;
use fc_core::text::string_catalog::StringCatalog;

#[derive(Default)]
pub struct AssistantAnswers {
    written: RefCell<HashMap<i64, String>>,
    /// Each stopped answer, with the reason its frame gave — `None` for every failure that is not
    /// a refusal.
    stopped: RefCell<HashMap<i64, Option<AiErrorReason>>>,
    version: Cell<u64>,
    /// Something about a chat's answers changed.
    pub changed: Event<i64>,
}

impl AssistantAnswers {
    pub fn new() -> Self {
        AssistantAnswers::default()
    }

    /// Goes up on every change — part of what a conversation compares before drawing again.
    pub fn version(&self) -> u64 {
        self.version.get()
    }

    pub fn delta(&self, chat_id: i64, message_id: i64, text: &str, held: Option<&MessageDto>) {
        let Some(held) = held else {
            return;
        };
        if held.edit_seq.is_some() || text.is_empty() {
            return;
        }
        self.written
            .borrow_mut()
            .entry(message_id)
            .or_default()
            .push_str(text);
        self.touch(chat_id);
    }

    /// It stopped early: the row keeps what arrived, and says so — and why, when the frame said.
    pub fn stopped(&self, chat_id: i64, message_id: i64, reason: Option<AiErrorReason>) {
        self.stopped.borrow_mut().insert(message_id, reason);
        self.touch(chat_id);
    }

    pub fn finished(&self, message: &MessageDto) {
        // Both, not whichever answers first: an answer that streamed AND stopped forgets both.
        let wrote = self.written.borrow_mut().remove(&message.id).is_some();
        let stopped = self.stopped.borrow_mut().remove(&message.id).is_some();
        if wrote || stopped {
            self.touch(message.chat_id);
        }
    }

    /// The body as drawn: the held one, and — until the answer is finished — what has streamed
    /// after it.
    pub fn body_of(&self, message: &MessageDto) -> String {
        if message.edit_seq.is_some() {
            return message.body.clone();
        }
        match self.written.borrow().get(&message.id) {
            Some(so_far) => format!("{}{so_far}", message.body),
            None => message.body.clone(),
        }
    }

    /// Whether what is drawn for this row includes streamed words the finished row has not yet
    /// replaced — the model's raw text, before the server's link filter (docs/protocol.md, "How
    /// sources are shown"). True for an answer that stopped part-way too, whose words are never
    /// replaced.
    pub fn is_writing(&self, message: &MessageDto) -> bool {
        message.edit_seq.is_none() && self.written.borrow().contains_key(&message.id)
    }

    /// Whether the row says it failed — which sentence it says is
    /// [`AssistantAnswers::failure_sentence`].
    pub fn failed(&self, message: &MessageDto) -> bool {
        message.edit_seq.is_none() && self.stopped.borrow().contains_key(&message.id)
    }

    /// What a failed row says — on the bubble, under what streamed, and to a screen reader — or
    /// `None` while it has not failed.
    pub fn failure_sentence(
        &self,
        message: &MessageDto,
        say: &dyn StringCatalog,
    ) -> Option<String> {
        if message.edit_seq.is_some() {
            return None;
        }
        let reason = *self.stopped.borrow().get(&message.id)?;
        Some(Self::sentence(reason, say))
    }

    /// The failure sentence for a reason: the provider's refusal is not worth asking again in the
    /// same words.
    pub fn sentence(reason: Option<AiErrorReason>, say: &dyn StringCatalog) -> String {
        match reason {
            Some(AiErrorReason::Refused) => say
                .get("The assistant's provider refused that. Try putting it another way.")
                .to_string(),
            None => say.get("Couldn't answer that. Ask again.").to_string(),
        }
    }

    pub fn clear(&self) {
        self.written.borrow_mut().clear();
        self.stopped.borrow_mut().clear();
        self.version.set(self.version.get() + 1);
    }

    fn touch(&self, chat_id: i64) {
        self.version.set(self.version.get() + 1);
        self.changed.emit(&chat_id);
    }
}

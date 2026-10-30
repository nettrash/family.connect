//! Who is typing where — the line under a conversation's name (`Typing.cs`).
//!
//! A TYPING FRAME IS MOMENTARY. It stands for five seconds — the phones' and the web's TTL — and is
//! never stored. The reader's own frames (from another device) and a blocked member's are not news,
//! and a message from somebody ends their typing: they stopped, and said it.
//!
//! BY MEMBER, NOT BY WHOSE FRAME CAME LAST, so "Anna, Bob" does not keep swapping while somebody
//! reads it. The sentences are the Apple client's — "%@ is typing…" and "%@ are typing…" over the
//! names — because those are the ones translated into all nine languages.
//!
//! One thread (the GLib main context), so no lock: the C#'s guarded the socket's thread.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::MessageDto;
use fc_core::rt::Clock;
use fc_core::store::{ChatStore, StoreResult};
use fc_core::text::string_catalog::StringCatalog;
use time::OffsetDateTime;

/// Who is typing where.
pub struct TypingRoster {
    chats: Rc<ChatStore>,
    clock: Rc<dyn Clock>,
    say: Rc<dyn StringCatalog>,
    typing: RefCell<HashMap<i64, HashMap<i64, OffsetDateTime>>>,
}

impl std::fmt::Debug for TypingRoster {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypingRoster")
            .field("chats", &self.typing.borrow().len())
            .finish_non_exhaustive()
    }
}

impl TypingRoster {
    /// How long one `typing` frame stands.
    pub const LASTS: Duration = Duration::from_secs(5);

    /// The roster over the cache it asks who is blocked and who is who, the clock that says what
    /// time it is (the C#'s `clock`), and the language its line is said in (`words`; the C#'s
    /// default was English).
    pub fn new(chats: Rc<ChatStore>, clock: Rc<dyn Clock>, say: Rc<dyn StringCatalog>) -> Self {
        TypingRoster {
            chats,
            clock,
            say,
            typing: RefCell::new(HashMap::new()),
        }
    }

    /// A `typing` frame.
    pub fn heard(&self, chat_id: i64, user_id: i64) -> StoreResult<()> {
        if user_id == self.chats.reader() || self.chats.is_blocked(user_id)? {
            return Ok(());
        }
        let now = self.clock.now();
        self.typing
            .borrow_mut()
            .entry(chat_id)
            .or_default()
            .insert(user_id, now);
        Ok(())
    }

    /// A message arrived: whoever sent it has stopped typing.
    pub fn spoke(&self, message: &MessageDto) {
        if let Some(who) = self.typing.borrow_mut().get_mut(&message.chat_id) {
            who.remove(&message.sender_id);
        }
    }

    /// Who is typing in this chat now, in member order.
    pub fn typists(&self, chat_id: i64) -> Vec<i64> {
        let at = self.clock.now();
        let mut typists: Vec<i64> =
            self.typing
                .borrow()
                .get(&chat_id)
                .map_or_else(Vec::new, |who| {
                    who.iter()
                        .filter(|(_, since)| at - **since < TypingRoster::LASTS)
                        .map(|(user_id, _)| *user_id)
                        .collect()
                });
        typists.sort_unstable();
        typists
    }

    /// The line to draw, or nothing.
    pub fn line(&self, chat_id: i64) -> StoreResult<String> {
        let say = &*self.say;
        let mut names = Vec::new();
        for id in self.typists(chat_id) {
            names.push(match self.chats.member(id)? {
                Some(member) if !member.display_name.is_empty() => member.display_name,
                _ => say.get("Someone").to_string(),
            });
        }
        Ok(match names.len() {
            0 => String::new(),
            1 => say.format("%@ is typing…", &[&names[0]]),
            _ => say.format("%@ are typing…", &[&names.join(", ")]),
        })
    }
}

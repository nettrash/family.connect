//! What each chat's composer holds while the reader is somewhere else (`ComposerDrafts.cs`) — the
//! web client's `drafts`: half a thought kept for the chat it was written in, and given back when
//! the reader returns.
//!
//! WORDS ONLY, AND ONLY WORDS WORTH KEEPING. A draft that is nothing but white space is no draft,
//! and saving one forgets the last. Sending takes the draft with it.
//!
//! A CHAT THAT IS GONE TAKES ITS DRAFT WITH IT — a direct chat deleted, a family left — so a later
//! chat that reused nothing still opens empty, and the store does not grow with chats nobody can
//! open.
//!
//! In memory, as the web client keeps them: a draft is this sitting's, not a document.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Default, Clone)]
pub struct ComposerDrafts {
    drafts: HashMap<i64, String>,
}

impl ComposerDrafts {
    pub fn new() -> Self {
        ComposerDrafts::default()
    }

    /// Keep what a chat's composer holds as the reader leaves it. White space is .NET's
    /// `string.IsNullOrWhiteSpace` — Unicode's `White_Space`, which `char::is_whitespace` is.
    pub fn save(&mut self, chat_id: i64, text: &str) {
        if text.chars().all(char::is_whitespace) {
            self.drafts.remove(&chat_id);
        } else {
            self.drafts.insert(chat_id, text.to_string());
        }
    }

    /// What to put back in a chat's composer as the reader returns: its draft, or nothing.
    pub fn of(&self, chat_id: i64) -> &str {
        self.drafts.get(&chat_id).map_or("", String::as_str)
    }

    /// A message was sent from this chat: what was being written went with it.
    pub fn sent(&mut self, chat_id: i64) {
        self.drafts.remove(&chat_id);
    }

    /// Keep only the drafts of chats that still exist.
    pub fn retain(&mut self, chat_ids: impl IntoIterator<Item = i64>) {
        let living: HashSet<i64> = chat_ids.into_iter().collect();
        self.drafts.retain(|chat_id, _| living.contains(chat_id));
    }

    pub fn count(&self) -> usize {
        self.drafts.len()
    }
}

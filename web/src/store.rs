//! What the app holds, and the rules for changing it.
//!
//! Kept apart from the views so the rules can be TESTED without a browser
//! rendering anything: every function here is pure, takes the state and
//! what arrived, and says what the state becomes. The parts that need a
//! browser (fetch, socket, storage) live in api/socket/session and are the
//! only places that do.

use std::collections::HashMap;

use crate::model::{ChatListItem, Message};

/// One chat's messages, oldest first — which is the order they are drawn in
/// and NOT the order `before_id` pages arrive in.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Thread {
    pub messages: Vec<Message>,
    /// The oldest id held, which is what the next history page asks before.
    /// None when nothing is held.
    pub oldest: Option<i64>,
    /// Whether the last page came back full, i.e. there may be more.
    pub more_above: bool,
}

impl Thread {
    /// Fold in whatever arrived, in any order, without duplicating.
    ///
    /// The protocol delivers the same message on several paths — a history
    /// page, the socket, and the ack of this device's own send — so "have I
    /// seen this" is the question every path asks. Identity is the SERVER
    /// ID; a pending row that has none yet is matched by `client_msg_id`,
    /// which is what turns the optimistic bubble into the real one instead
    /// of leaving two.
    pub fn apply(&mut self, incoming: Message) {
        if let Some(existing) = self
            .messages
            .iter_mut()
            .find(|held| held.id == incoming.id && incoming.id != 0)
        {
            *existing = incoming;
            return;
        }
        if let Some(client_msg_id) = incoming.client_msg_id.as_deref() {
            if let Some(pending) = self
                .messages
                .iter_mut()
                .find(|held| held.id == 0 && held.client_msg_id.as_deref() == Some(client_msg_id))
            {
                *pending = incoming;
                self.resort();
                return;
            }
        }
        self.messages.push(incoming);
        self.resort();
    }

    /// Oldest first, with pending rows (id 0) after everything the server
    /// has numbered — they were written last, whatever id they end up with.
    fn resort(&mut self) {
        self.messages.sort_by_key(|message| {
            if message.id == 0 {
                i64::MAX
            } else {
                message.id
            }
        });
        self.oldest = self
            .messages
            .iter()
            .map(|message| message.id)
            .filter(|id| *id != 0)
            .min();
    }

    /// The newest id the server has numbered — the read marker to report,
    /// and the cursor a catch-up asks after.
    pub fn newest_server_id(&self) -> Option<i64> {
        self.messages
            .iter()
            .map(|message| message.id)
            .filter(|id| *id != 0)
            .max()
    }
}

/// Everything the signed-in app knows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Store {
    pub my_user_id: i64,
    pub chats: Vec<ChatListItem>,
    pub threads: HashMap<i64, Thread>,
    pub names: HashMap<i64, String>,
    /// chat → (member → when their last `typing` frame arrived, in
    /// milliseconds). Timestamps rather than a plain list because a typing
    /// frame says "still typing" and never says "stopped": without an
    /// expiry, one frame from somebody who then closed their laptop leaves
    /// "X is typing…" on the screen for ever. The apps prune at 5 s and so
    /// does this (ios ChatSyncCoordinator.typingByChat).
    pub typing: HashMap<i64, HashMap<i64, f64>>,
}

/// How long a `typing` frame stands before it is forgotten, in
/// milliseconds. The same 5 s the phone clients use.
pub const TYPING_TTL_MS: f64 = 5_000.0;

/// How often this client may SEND one, in milliseconds. The same 4 s the
/// phone clients throttle to: a frame per keystroke would be a frame per
/// keystroke times every member.
pub const TYPING_THROTTLE_MS: f64 = 4_000.0;

impl Store {
    /// A message arrived, from any path.
    ///
    /// The unread count moves only for a message from SOMEBODY ELSE in a
    /// chat that is not open — the same rule every client in this product
    /// follows, and the reason it is here rather than in a view is that the
    /// view would have to be looking to apply it.
    pub fn apply_message(&mut self, message: Message, open_chat: Option<i64>) {
        let chat_id = message.chat_id;
        let is_mine = message.sender_id == self.my_user_id;
        let is_open = open_chat == Some(chat_id);
        let is_new = self
            .threads
            .get(&chat_id)
            .map(|thread| !thread.messages.iter().any(|held| held.id == message.id))
            .unwrap_or(true)
            && message.id != 0;

        // Somebody who has just said something is not still typing.
        if let Some(typing) = self.typing.get_mut(&chat_id) {
            typing.remove(&message.sender_id);
        }

        if let Some(item) = self.chats.iter_mut().find(|item| item.chat.id == chat_id) {
            item.last_message = Some(message.clone());
            if is_new && !is_mine && !is_open {
                item.unread_count += 1;
            }
        }
        self.threads.entry(chat_id).or_default().apply(message);
    }

    /// This device has read up to here: the badge goes, and the marker
    /// moves MONOTONICALLY — a stale report must never walk it backwards.
    pub fn mark_read(&mut self, chat_id: i64, up_to: i64) {
        if let Some(item) = self.chats.iter_mut().find(|item| item.chat.id == chat_id) {
            item.unread_count = 0;
            item.last_read_message_id = item.last_read_message_id.max(up_to);
        }
    }

    /// Somebody's `read` frame. Only the CALLER'S own marker is theirs to
    /// apply; another member's read says nothing about this device's badge.
    pub fn apply_read_frame(&mut self, chat_id: i64, user_id: i64, up_to: i64) {
        if user_id != self.my_user_id {
            return;
        }
        if let Some(item) = self.chats.iter_mut().find(|item| item.chat.id == chat_id) {
            item.last_read_message_id = item.last_read_message_id.max(up_to);
            // Read on another device: this one's badge follows, and only
            // ever downwards to zero when the marker caught up with the
            // newest thing here.
            let newest = self
                .threads
                .get(&chat_id)
                .and_then(|thread| thread.newest_server_id())
                .unwrap_or(0);
            if item.last_read_message_id >= newest {
                item.unread_count = 0;
            }
        }
    }

    /// A `typing` frame arrived. `now` is passed in rather than read so
    /// this stays testable without a clock.
    pub fn set_typing(&mut self, chat_id: i64, user_id: i64, now: f64) {
        if user_id == self.my_user_id {
            return;
        }
        self.typing.entry(chat_id).or_default().insert(user_id, now);
    }

    /// Forget everybody whose last frame is older than the TTL. Called on a
    /// tick, and again whenever the answer is about to be drawn.
    pub fn prune_typing(&mut self, now: f64) {
        for typing in self.typing.values_mut() {
            typing.retain(|_, stamp| now - *stamp < TYPING_TTL_MS);
        }
        self.typing.retain(|_, typing| !typing.is_empty());
    }

    /// Who is typing here, by name, oldest frame first so the line does not
    /// reshuffle itself while somebody reads it.
    pub fn typing_names(&self, chat_id: i64, now: f64) -> Vec<String> {
        let Some(typing) = self.typing.get(&chat_id) else {
            return Vec::new();
        };
        let mut live: Vec<(&i64, &f64)> = typing
            .iter()
            .filter(|(_, stamp)| now - **stamp < TYPING_TTL_MS)
            .collect();
        live.sort_by(|left, right| {
            right
                .1
                .partial_cmp(left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(left.0.cmp(right.0))
        });
        live.into_iter()
            .map(|(user, _)| {
                self.names
                    .get(user)
                    .cloned()
                    .unwrap_or_else(|| "Someone".to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn message(id: i64, sender: i64, body: &str) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id: sender,
            client_msg_id: None,
            body: body.into(),
            created_at: "2026-08-19T17:03:12Z".into(),
            edited_at: None,
        }
    }

    fn pending(client_msg_id: &str, body: &str) -> Message {
        Message {
            id: 0,
            chat_id: 42,
            sender_id: 7,
            client_msg_id: Some(client_msg_id.into()),
            body: body.into(),
            created_at: "2026-08-19T17:04:00Z".into(),
            edited_at: None,
        }
    }

    fn store() -> Store {
        Store {
            my_user_id: 7,
            chats: vec![ChatListItem {
                chat: crate::model::Chat {
                    id: 42,
                    kind: "family".into(),
                    title: Some("The Smiths".into()),
                    peer_user_id: None,
                },
                last_message: None,
                unread_count: 0,
                last_read_message_id: 0,
            }],
            ..Store::default()
        }
    }

    /// The SAME message arrives on three paths — a history page, the socket
    /// and this device's own ack. It is one message, not three.
    #[wasm_bindgen_test]
    fn a_message_seen_twice_is_one_message() {
        let mut thread = Thread::default();
        thread.apply(message(100, 9, "Dinner at 7?"));
        thread.apply(message(100, 9, "Dinner at 7?"));
        assert_eq!(thread.messages.len(), 1);
        thread.apply(message(101, 7, "Six works"));
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.oldest, Some(100));
        assert_eq!(thread.newest_server_id(), Some(101));
    }

    /// The optimistic bubble BECOMES the real one. Matched on
    /// `client_msg_id`, because that is the only thing the two copies share
    /// before the server has given it an id.
    #[wasm_bindgen_test]
    fn an_ack_replaces_the_pending_row_rather_than_adding_one() {
        let mut thread = Thread::default();
        thread.apply(pending("8f14e45f", "Six works"));
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.newest_server_id(), None, "nothing is numbered yet");

        let mut acked = message(101, 7, "Six works");
        acked.client_msg_id = Some("8f14e45f".into());
        thread.apply(acked);
        assert_eq!(thread.messages.len(), 1, "one bubble, not two");
        assert_eq!(thread.messages[0].id, 101);
        assert_eq!(thread.newest_server_id(), Some(101));
    }

    /// A history page arrives newest-first and out of order relative to
    /// what is held; the thread is drawn oldest-first regardless, with the
    /// not-yet-numbered rows last.
    #[wasm_bindgen_test]
    fn a_thread_is_ordered_oldest_first_with_pending_rows_at_the_end() {
        let mut thread = Thread::default();
        thread.apply(pending("later", "typing this now"));
        thread.apply(message(103, 9, "c"));
        thread.apply(message(101, 9, "a"));
        thread.apply(message(102, 9, "b"));
        let ids: Vec<i64> = thread.messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![101, 102, 103, 0]);
        assert_eq!(thread.oldest, Some(101));
    }

    /// The unread rule: somebody else's message, in a chat nobody is
    /// looking at.
    #[wasm_bindgen_test]
    fn unread_counts_only_somebody_elses_message_in_a_chat_that_is_not_open() {
        let mut store = store();
        store.apply_message(message(100, 9, "Dinner?"), None);
        assert_eq!(store.chats[0].unread_count, 1);

        // My own message never counts.
        store.apply_message(message(101, 7, "Six works"), None);
        assert_eq!(store.chats[0].unread_count, 1);

        // Nor does one in the chat that is open.
        store.apply_message(message(102, 9, "See you"), Some(42));
        assert_eq!(store.chats[0].unread_count, 1);

        // Nor the same message twice.
        store.apply_message(message(100, 9, "Dinner?"), None);
        assert_eq!(store.chats[0].unread_count, 1);
    }

    #[wasm_bindgen_test]
    fn reading_clears_the_badge_and_the_marker_never_walks_backwards() {
        let mut store = store();
        store.apply_message(message(100, 9, "Dinner?"), None);
        store.mark_read(42, 100);
        assert_eq!(store.chats[0].unread_count, 0);
        assert_eq!(store.chats[0].last_read_message_id, 100);

        // A stale report — a response still in flight while the reader
        // reads on — must not move the marker back.
        store.mark_read(42, 50);
        assert_eq!(store.chats[0].last_read_message_id, 100);
    }

    /// A `read` frame is only about ITS OWN user. Another member reading is
    /// not this device's badge.
    #[wasm_bindgen_test]
    fn another_members_read_frame_leaves_this_badge_alone() {
        let mut store = store();
        store.apply_message(message(100, 9, "Dinner?"), None);
        store.apply_read_frame(42, 9, 100);
        assert_eq!(store.chats[0].unread_count, 1, "that was somebody else");

        // My own, from another device: the badge follows.
        store.apply_read_frame(42, 7, 100);
        assert_eq!(store.chats[0].unread_count, 0);
        assert_eq!(store.chats[0].last_read_message_id, 100);
    }

    /// A partial read from another device leaves what is still unread.
    #[wasm_bindgen_test]
    fn a_partial_read_elsewhere_does_not_clear_the_badge() {
        let mut store = store();
        store.apply_message(message(100, 9, "one"), None);
        store.apply_message(message(101, 9, "two"), None);
        assert_eq!(store.chats[0].unread_count, 2);
        store.apply_read_frame(42, 7, 100);
        assert_eq!(store.chats[0].unread_count, 2, "101 is still unread");
        store.apply_read_frame(42, 7, 101);
        assert_eq!(store.chats[0].unread_count, 0);
    }

    #[wasm_bindgen_test]
    fn typing_is_other_people_and_stops_when_they_say_something() {
        let mut store = store();
        store.names.insert(9, "Anna".into());
        store.set_typing(42, 9, 1_000.0);
        // My own typing frame is not news to me.
        store.set_typing(42, 7, 1_000.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Anna".to_string()]);

        store.apply_message(message(100, 9, "Dinner?"), Some(42));
        assert!(
            store.typing_names(42, 1_100.0).is_empty(),
            "somebody who just spoke is not still typing"
        );
    }

    /// A `typing` frame says "still typing" and NEVER says "stopped". With
    /// no expiry, one frame from somebody who then shut their laptop would
    /// leave "X is typing…" on the screen for ever.
    #[wasm_bindgen_test]
    fn a_typing_frame_expires_on_its_own() {
        let mut store = store();
        store.names.insert(9, "Anna".into());
        store.set_typing(42, 9, 1_000.0);
        assert_eq!(
            store.typing_names(42, 1_000.0 + TYPING_TTL_MS - 1.0).len(),
            1
        );
        assert!(
            store.typing_names(42, 1_000.0 + TYPING_TTL_MS).is_empty(),
            "past the TTL it is forgotten"
        );

        // And the prune actually drops it, so the map does not grow with
        // every member who ever typed.
        store.prune_typing(1_000.0 + TYPING_TTL_MS);
        assert!(store.typing.is_empty());
    }

    /// A fresh frame from the same member RENEWS rather than adding a
    /// second entry — the frame means "still".
    #[wasm_bindgen_test]
    fn a_second_frame_renews_the_first() {
        let mut store = store();
        store.names.insert(9, "Anna".into());
        store.set_typing(42, 9, 1_000.0);
        store.set_typing(42, 9, 4_000.0);
        assert_eq!(store.typing_names(42, 6_000.0), vec!["Anna".to_string()]);
        assert_eq!(store.typing.get(&42).map(HashMap::len), Some(1));
    }

    #[wasm_bindgen_test]
    fn a_sender_who_is_not_in_the_roster_still_has_a_name() {
        let mut store = store();
        store.set_typing(42, 11, 1_000.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Someone".to_string()]);
    }
}

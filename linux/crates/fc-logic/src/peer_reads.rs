//! How far the other person in each direct chat has read — the seen tick's marker
//! (docs/protocol.md, the `read` frame; `PeerReads.cs`).
//!
//! ONLY A LIVE FRAME CARRIES IT. `GET /chats` answers the reader's OWN marker and nobody else's,
//! so this is what the frames said since this device connected, kept in memory as the web client
//! keeps it. The router already passes only a direct chat's.
//!
//! MONOTONIC, as every read marker is: a frame delayed behind a newer one must not take a tick
//! back.
//!
//! One thread (the GLib main context), so no lock: the C#'s `lock` guarded the socket's thread.

use std::cell::RefCell;
use std::collections::HashMap;

use fc_core::rt::Event;

#[derive(Default)]
pub struct PeerReads {
    markers: RefCell<HashMap<i64, i64>>,
    /// A chat's marker moved forward.
    pub changed: Event<i64>,
}

impl PeerReads {
    pub fn new() -> Self {
        PeerReads::default()
    }

    /// Take a marker; answers whether it moved anything.
    pub fn apply(&self, chat_id: i64, last_read_message_id: i64) -> bool {
        {
            let mut markers = self.markers.borrow_mut();
            if markers
                .get(&chat_id)
                .is_some_and(|&held| held >= last_read_message_id)
            {
                return false;
            }
            markers.insert(chat_id, last_read_message_id);
        }
        self.changed.emit(&chat_id);
        true
    }

    /// The marker for a chat: 0 while no frame has said.
    pub fn up_to(&self, chat_id: i64) -> i64 {
        self.markers.borrow().get(&chat_id).copied().unwrap_or(0)
    }

    /// A session ended: somebody else's markers are not the next person's.
    pub fn clear(&self) {
        self.markers.borrow_mut().clear();
    }
}

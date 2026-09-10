//! What the app holds, and the rules for changing it.
//!
//! Kept apart from the views so the rules can be TESTED without a browser
//! rendering anything: every function here is pure, takes the state and
//! what arrived, and says what the state becomes. The parts that need a
//! browser (fetch, socket, storage) live in api/socket/session and are the
//! only places that do.

use std::collections::{HashMap, HashSet};

use crate::model::{
    Assistant, ChatListItem, Me, Member, Mention, Message, Poll, PollOption, Reaction, ReplyParent,
    ReplyTo, Roster, User,
};

/// A quote's cut: 120 Unicode scalar values, the server's own cut
/// (fc_text::excerpt), redone here when an edit to a quoted message is
/// applied (docs/protocol.md, "Editing").
pub fn excerpt(body: &str) -> String {
    fc_text::excerpt::excerpt(body).to_string()
}

/// What folding a server copy into a thread did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrival {
    /// A server id this thread did not hold — including a pending bubble
    /// turning into the real message.
    New,
    /// A copy of something already held.
    Known,
}

/// Which path a message arrived by — which decides what else it moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// A live `message` frame: counts as unread, marks a mention, and
    /// raises a thread root's count.
    Frame,
    /// An `after_id` catch-up page: raises a thread root's count, and counts
    /// as unread ONLY what is newer than the chat's last message in the
    /// `GET /chats` read before it — that read already counted everything up
    /// to there, and counting it again would double every badge, while a
    /// message landing after it would otherwise never be counted at all.
    CatchUp {
        /// The chat's `last_message.id` in the list read, 0 when none.
        counted_up_to: i64,
    },
    /// The answer to this device's own send: raises a thread root's count.
    Answer,
    /// A bubble drawn before the server has it: moves nothing.
    Pending,
}

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
    /// Whether a page of history has been folded in. A thread that only
    /// frames have made holds the newest messages and NOTHING of what came
    /// before them — not even how much that is — so opening it reads the
    /// newest page rather than catching up after what it holds.
    pub paged: bool,
}

/// Whether `incoming` is a newer sequence value than `held` — absent on the
/// incoming side is never newer, absent on the held side is always older.
fn newer(incoming: Option<i64>, held: Option<i64>) -> bool {
    match (incoming, held) {
        (Some(incoming), Some(held)) => incoming > held,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

/// Fold a SERVER copy of a message into the one held, under the protocol's
/// three guards — each part of a message changes on its own sequence, and a
/// copy fetched before a change but delivered after it must not undo it.
///
/// - the body (and everything else an edit can carry: the assistant's
///   picture arrives as an attachment this way) only when the incoming
///   `edit_seq` is at least the held one, absent counting as 0;
/// - reactions only when the incoming `reaction_seq` is greater;
/// - the poll only when the incoming `poll_seq` is greater;
/// - `reply_count` from ANY server copy: it is recomputed on every read, so
///   whatever the server last said is the truth (docs/protocol.md,
///   "Threads").
fn merge(held: &mut Message, incoming: Message) {
    let take_reactions = newer(incoming.reaction_seq, held.reaction_seq);
    let take_poll = newer(
        incoming.poll.as_ref().map(|poll| poll.poll_seq),
        held.poll.as_ref().map(|poll| poll.poll_seq),
    );
    let (reactions, reaction_seq) = if take_reactions {
        (incoming.reactions.clone(), incoming.reaction_seq)
    } else {
        (held.reactions.take(), held.reaction_seq)
    };
    let poll = if take_poll {
        incoming.poll.clone()
    } else {
        held.poll.take()
    };
    let reply_count = incoming.reply_count;
    if incoming.edit_seq() >= held.edit_seq() {
        *held = incoming;
    }
    held.reactions = reactions;
    held.reaction_seq = reaction_seq;
    held.poll = poll;
    held.reply_count = reply_count;
}

impl Thread {
    /// Fold in whatever arrived, in any order, without duplicating.
    ///
    /// The protocol delivers the same message on several paths — a history
    /// page, the socket, the answer to this device's own send — so "have I
    /// seen this" is the question every path asks. Identity is the SERVER
    /// ID; a pending row that has none yet is matched by `client_msg_id`,
    /// which is what turns the optimistic bubble into the real one instead
    /// of leaving two.
    pub fn apply(&mut self, incoming: Message) -> Arrival {
        if incoming.id != 0 {
            if let Some(held) = self.messages.iter_mut().find(|held| held.id == incoming.id) {
                merge(held, incoming);
                return Arrival::Known;
            }
        }
        if let Some(client_msg_id) = incoming.client_msg_id.as_deref() {
            if let Some(pending) = self
                .messages
                .iter_mut()
                .find(|held| held.id == 0 && held.client_msg_id.as_deref() == Some(client_msg_id))
            {
                let arrival = if incoming.id == 0 {
                    Arrival::Known
                } else {
                    Arrival::New
                };
                *pending = incoming;
                self.resort();
                return arrival;
            }
        }
        let arrival = if incoming.id == 0 {
            Arrival::Known
        } else {
            Arrival::New
        };
        self.messages.push(incoming);
        self.resort();
        arrival
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

    pub fn get(&self, id: i64) -> Option<&Message> {
        self.messages
            .iter()
            .find(|message| message.id == id && id != 0)
    }

    fn get_mut(&mut self, id: i64) -> Option<&mut Message> {
        self.messages
            .iter_mut()
            .find(|message| message.id == id && id != 0)
    }
}

/// A message this device wrote that the server has not confirmed yet.
///
/// Its bubble is in the thread under id 0; this is the other half — what
/// the sender needs to try again, and what the bubble needs to say about
/// it (docs/protocol.md, "Sending on an unreliable network").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Outgoing {
    pub chat_id: i64,
    pub client_msg_id: String,
    pub body: String,
    /// The message this one answers, if any.
    pub reply_to_message_id: Option<i64>,
    /// The members named, resolved from the text when Send was pressed —
    /// riding on the row so a retry re-sends exactly the same list.
    pub mentions: Vec<Mention>,
    /// A poll's options; the body is then its question.
    pub poll: Option<Vec<String>>,
    /// Tries whose outcome was UNKNOWN. A refusal is not counted here: it
    /// ends the row outright.
    pub attempts: u32,
    /// Why it will not be tried again until somebody asks. None while it
    /// is still queued — which is what an unknown outcome leaves it.
    pub failed: Option<String>,
}

/// What a person asked to send. Everything but the chat and the body is
/// optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub body: String,
    pub reply_to_message_id: Option<i64>,
    pub mentions: Vec<Mention>,
    pub poll: Option<Vec<String>>,
}

/// How many unknown outcomes a message may have before it is shown as
/// failed. The same six the phone clients allow.
pub const MAX_SEND_ATTEMPTS: u32 = 6;

/// A chat's three catch-up cursors (docs/protocol.md, "Semantics", resync
/// step 3). ONLY a live frame and a catch-up page may move them — never a
/// history page and never the answer to this device's own request.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Cursors {
    pub reaction: i64,
    pub edit: i64,
    pub poll: i64,
    /// Whether this chat's catch-up has finished on the CURRENT connection.
    /// Until it has, a frame does not move a cursor: a frame for seq 130
    /// arriving while the catch-up is still fetching from 100 would, if the
    /// catch-up then failed, leave the stored cursor past 101–129 for good.
    /// Stricter than the protocol asks, and one redundant page is the
    /// cheaper mistake.
    pub caught_up: bool,
}

/// Which of a chat's cursors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feed {
    Reactions,
    Edits,
    Polls,
}

/// A chain of replies read on its own surface. Held APART from the chat's
/// own thread: its rows may sit outside the window the chat holds — a root
/// older than it, a reply newer — and folding them in would move the
/// chat's paging cursor past everything between (docs/protocol.md,
/// "Threads").
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ThreadView {
    pub chat_id: i64,
    pub root_id: i64,
    /// Which opening this is. A thread read in flight when the reader opened
    /// ANOTHER thread belongs to that other one, and is dropped rather than
    /// folded into — or re-rooting — the surface now showing.
    pub generation: u64,
    pub thread: Thread,
}

impl ThreadView {
    /// Whether a message belongs on this surface: the root, or a reply
    /// rooted at it.
    pub fn includes(&self, message: &Message) -> bool {
        message.chat_id == self.chat_id
            && (message.id == self.root_id || message.thread_root_id == Some(self.root_id))
    }
}

/// The family chat's open polls, as last read — a plain read, not a cursor.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct OpenPolls {
    pub chat_id: i64,
    pub messages: Vec<Message>,
}

/// Everything the signed-in app knows.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Store {
    pub my_user_id: i64,
    pub chats: Vec<ChatListItem>,
    pub threads: HashMap<i64, Thread>,
    pub names: HashMap<i64, String>,
    /// The live roster, which the mention suggestions offer from.
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    /// The reader's own block list — replaced whole on every resync.
    pub blocked: HashSet<i64>,
    /// Hidden rows the reader has peeked at. Per row, per device, never on
    /// the wire, and gone on reload (docs/protocol.md, "Blocking a member").
    pub revealed: HashSet<i64>,
    /// Quotes of a blocked member the reader has peeked at: (message, level)
    /// with level 0 the quote and 1 the parent under it — each its own
    /// reveal, like a row's, and like it gone on reload.
    pub revealed_quotes: HashSet<(i64, u8)>,
    pub support_contact: Option<String>,
    /// What this device has written and the server has not confirmed, in
    /// the order it was written — which is the order it is sent in.
    pub outbox: Vec<Outgoing>,
    /// chat → (member → when their last `typing` frame arrived, in
    /// milliseconds). Timestamps rather than a plain list because a typing
    /// frame says "still typing" and never says "stopped": without an
    /// expiry, one frame from somebody who then closed their laptop leaves
    /// "X is typing…" on the screen for ever. The apps prune at 5 s and so
    /// does this (ios ChatSyncCoordinator.typingByChat).
    pub typing: HashMap<i64, HashMap<i64, f64>>,
    pub cursors: HashMap<i64, Cursors>,
    /// The peer's read marker in a DIRECT chat — the highest id they have
    /// reported, which is what a "seen" tick compares against.
    pub peer_read: HashMap<i64, i64>,
    /// Assistant answers that stopped early (`ai_error`).
    pub ai_failed: HashSet<i64>,
    /// What was being typed in a chat the reader left.
    pub drafts: HashMap<i64, String>,
    pub thread_view: Option<ThreadView>,
    /// How many threads have been opened this session — the next
    /// `ThreadView::generation`. Counted here rather than on the view, which
    /// a close throws away: a count that started again at every open would
    /// hand a stale read the generation of the thread opened after it.
    pub thread_openings: u64,
    pub open_polls: Option<OpenPolls>,
    /// chat → the last message the most recent `GET /chats` listed, which
    /// its unread count already includes. A frame for a message at or below
    /// it is one the list has counted: the socket and the list race, and a
    /// frame landing after the answer that included its message must not
    /// count it a second time.
    pub listed: HashMap<i64, i64>,
}

/// How long a `typing` frame stands before it is forgotten, in
/// milliseconds. The same 5 s the phone clients use.
pub const TYPING_TTL_MS: f64 = 5_000.0;

/// How often this client may SEND one, in milliseconds. The same 4 s the
/// phone clients throttle to: a frame per keystroke would be a frame per
/// keystroke times every member.
pub const TYPING_THROTTLE_MS: f64 = 4_000.0;

impl Store {
    pub fn item(&self, chat_id: i64) -> Option<&ChatListItem> {
        self.chats.iter().find(|item| item.chat.id == chat_id)
    }

    fn item_mut(&mut self, chat_id: i64) -> Option<&mut ChatListItem> {
        self.chats.iter_mut().find(|item| item.chat.id == chat_id)
    }

    /// The list as it is drawn: the family chat first, then everything else
    /// newest conversation first (ios MacChatView).
    pub fn sorted_chats(&self) -> Vec<ChatListItem> {
        let mut chats = self.chats.clone();
        let newest = |item: &ChatListItem| -> i64 {
            item.last_message
                .as_ref()
                .map(|message| {
                    if message.id == 0 {
                        i64::MAX
                    } else {
                        message.id
                    }
                })
                .unwrap_or(0)
        };
        chats.sort_by(|left, right| {
            right
                .chat
                .is_family()
                .cmp(&left.chat.is_family())
                .then(newest(right).cmp(&newest(left)))
        });
        chats
    }

    /// `GET /me`: who this is, and the block list — REPLACED, never merged.
    pub fn apply_me(&mut self, me: &Me) {
        self.my_user_id = me.user.id;
        self.names.insert(me.user.id, me.user.display_name.clone());
        self.blocked = me.blocked_user_ids.iter().copied().collect();
        self.support_contact = me.support_contact.clone();
    }

    /// `GET /families/mine`: the roster, the names, the assistant, and the
    /// block list again (it rides on both reads).
    pub fn apply_roster(&mut self, roster: &Roster) {
        self.names.extend(roster.names());
        self.members = roster.members.clone();
        self.assistant = roster.assistant.clone();
        self.blocked = roster.blocked_user_ids.iter().copied().collect();
    }

    /// `GET /chats`, MERGED into what this client holds rather than laid
    /// over it (ios ChatSyncCoordinator: the marker kept monotonic, live
    /// bumps above the list's snapshot added back).
    ///
    /// The list is a snapshot taken when the server answered, and this
    /// client went on living while it was in flight: a read it reported, a
    /// message that arrived, a mention. So per chat:
    /// - the read marker is the larger of the two — it never walks back;
    /// - the unread count is the server's, plus what arrived after its
    ///   snapshot (held messages from somebody else newer than both the
    ///   snapshot's last message and the marker) — or, when this device has
    ///   read past the snapshot's marker, a recount of what is still above
    ///   it, where the held window covers the marker;
    /// - the preview is whichever last message is newer.
    pub fn merge_chats(&mut self, fresh: Vec<ChatListItem>) {
        let me = self.my_user_id;
        self.listed = fresh
            .iter()
            .map(|item| {
                let last = item.last_message.as_ref().map(|m| m.id).unwrap_or(0);
                (item.chat.id, last)
            })
            .collect();
        let merged = fresh
            .into_iter()
            .map(|mut item| {
                let Some(held) = self.item(item.chat.id) else {
                    return item;
                };
                let snapshot_last = item.last_message.as_ref().map(|m| m.id).unwrap_or(0);
                let marker = item.last_read_message_id.max(held.last_read_message_id);
                let thread = self.threads.get(&item.chat.id);
                let held_newest = thread.and_then(Thread::newest_server_id).unwrap_or(0);
                let newest = snapshot_last.max(held_newest);
                if marker >= newest {
                    item.unread_count = 0;
                    item.mentioned = false;
                } else if let Some(thread) = thread {
                    let after = |floor: i64| {
                        thread
                            .messages
                            .iter()
                            .filter(move |m| m.id > floor && m.sender_id != me)
                    };
                    let covers = thread.oldest.is_some_and(|oldest| oldest <= marker);
                    if held.last_read_message_id > item.last_read_message_id && covers {
                        item.unread_count = after(marker).count() as i64;
                    } else {
                        let raced: Vec<&Message> = after(snapshot_last.max(marker)).collect();
                        item.unread_count += raced.len() as i64;
                        if raced.iter().any(|m| {
                            !self.blocked.contains(&m.sender_id)
                                && m.mentions().iter().any(|mention| mention.user_id == me)
                        }) {
                            item.mentioned = true;
                        }
                    }
                }
                item.last_read_message_id = marker;
                if let Some(last) = &held.last_message {
                    if last.id == 0 || last.id > snapshot_last {
                        item.last_message = Some(last.clone());
                    }
                }
                item
            })
            .collect();
        self.chats = merged;
    }

    /// Rows a reload left in the outbox, back in it — each with its bubble,
    /// so a message the person saw as "Sending…" goes on being sent rather
    /// than vanishing with the page (docs/protocol.md, "Sending on an
    /// unreliable network").
    pub fn restore_outbox(&mut self, rows: Vec<Outgoing>) {
        for row in rows {
            if self
                .outbox
                .iter()
                .any(|held| held.client_msg_id == row.client_msg_id)
            {
                continue;
            }
            let failed = row.failed.clone();
            let draft = Draft {
                body: row.body.clone(),
                reply_to_message_id: row.reply_to_message_id,
                mentions: row.mentions.clone(),
                poll: row.poll.clone(),
            };
            self.queue_send(row.chat_id, row.client_msg_id.clone(), draft);
            if let Some(restored) = self.row_mut(&row.client_msg_id) {
                restored.attempts = row.attempts;
                restored.failed = failed;
            }
        }
    }

    /// A message arrived, from any path but a history page.
    ///
    /// A frame, a catch-up page and the answer to this device's own send
    /// deliver what is newer than everything held, and only those raise a
    /// thread root's count: a history page or a thread read delivers replies
    /// the root's own recomputed copy already includes (docs/protocol.md,
    /// "Threads"). Only a frame moves an unread figure — see `Via`.
    ///
    /// The unread count moves only for a message from SOMEBODY ELSE in a
    /// chat nobody is reading — the rule every client in this product
    /// follows, and the reason it is here rather than in a view is that the
    /// view would have to be looking to apply it.
    pub fn apply_message(&mut self, message: Message, reading: Option<i64>, via: Via) {
        let chat_id = message.chat_id;
        let is_mine = message.sender_id == self.my_user_id;
        let is_reading = reading == Some(chat_id);
        let names_me = !is_mine
            && !self.blocked.contains(&message.sender_id)
            && message
                .mentions()
                .iter()
                .any(|mention| mention.user_id == self.my_user_id);

        // Somebody who has just said something is not still typing.
        if let Some(typing) = self.typing.get_mut(&chat_id) {
            typing.remove(&message.sender_id);
        }

        // One of mine, numbered: the server has it, whichever path said so
        // first — the POST's own answer, or the `message` frame the same
        // send fans out to this socket. Matched on the protocol's dedup key,
        // (chat, sender, client_msg_id), and nothing looser.
        if is_mine && message.id != 0 {
            if let Some(client_msg_id) = message.client_msg_id.as_deref() {
                self.outbox
                    .retain(|row| !(row.chat_id == chat_id && row.client_msg_id == client_msg_id));
            }
        }

        let mut view_arrival = None;
        if let Some(view) = self.thread_view.as_mut() {
            if view.includes(&message) || view.thread.get(message.id).is_some() {
                view_arrival = Some(view.thread.apply(message.clone()));
            }
        }
        let thread_root = message.thread_root_id;
        let message_id = message.id;
        let arrival = self
            .threads
            .entry(chat_id)
            .or_default()
            .apply(message.clone());

        // Each copy of the root counts a reply the first time THAT copy sees
        // it: the thread read may already have delivered — and counted — a
        // reply the chat's own window is only now receiving.
        if via != Via::Pending {
            if let Some(root) = thread_root {
                if arrival == Arrival::New {
                    self.bump_root(chat_id, root, false);
                }
                if view_arrival == Some(Arrival::New) {
                    self.bump_root(chat_id, root, true);
                }
            }
        }
        let counts = match via {
            Via::Frame => message_id > self.listed.get(&chat_id).copied().unwrap_or(0),
            Via::CatchUp { counted_up_to } => message_id > counted_up_to,
            Via::Answer | Via::Pending => false,
        };

        if let Some(item) = self.item_mut(chat_id) {
            let newer_than_preview = item
                .last_message
                .as_ref()
                .is_none_or(|last| last.id == 0 || message.id == 0 || message.id >= last.id);
            if newer_than_preview {
                item.last_message = Some(message.clone());
            }
            if counts && arrival == Arrival::New && !is_mine && !is_reading {
                item.unread_count += 1;
                if names_me {
                    item.mentioned = true;
                }
            }
        }
    }

    /// A reply arrived live: one copy of its root — the chat's own, or the
    /// thread surface's — says one more.
    fn bump_root(&mut self, chat_id: i64, root: i64, on_surface: bool) {
        let held = if on_surface {
            self.thread_view
                .as_mut()
                .filter(|view| view.chat_id == chat_id)
                .and_then(|view| view.thread.get_mut(root))
        } else {
            self.threads
                .get_mut(&chat_id)
                .and_then(|thread| thread.get_mut(root))
        };
        if let Some(held) = held {
            held.reply_count = Some(held.reply_count.unwrap_or(0) + 1);
        }
    }

    /// Server copies read for the thread surface, folded into whatever the
    /// chat's own window already HOLDS of them — the recomputed reply count
    /// and any edit — and nothing else: a row the window does not hold stays
    /// out of it, so no paging cursor moves (docs/protocol.md, "Threads").
    pub fn refresh_held(&mut self, chat_id: i64, messages: &[Message]) {
        let Some(thread) = self.threads.get_mut(&chat_id) else {
            return;
        };
        for message in messages {
            if thread.get(message.id).is_some() {
                thread.apply(message.clone());
            }
        }
    }

    /// A page of a thread read (`GET …/thread`), for the opening whose
    /// surface asked for it. The chat's own window takes the copies of rows
    /// it already holds (`refresh_held`); the surface takes the page only
    /// while it is still that opening's — a read in flight when the reader
    /// opened another thread would otherwise land in, and on its first page
    /// re-root, the thread now showing. Answers whether the surface is
    /// still this opening's, i.e. whether reading on is worth it.
    pub fn apply_thread_page(
        &mut self,
        chat_id: i64,
        generation: u64,
        first_page: bool,
        page: Vec<Message>,
    ) -> bool {
        self.refresh_held(chat_id, &page);
        let Some(view) = self.thread_view.as_mut() else {
            return false;
        };
        if view.generation != generation || view.chat_id != chat_id {
            return false;
        }
        // The first page answers with the ROOT first, whichever message
        // was named — which is how a client holding only a reply learns
        // what the chain is rooted at.
        if first_page {
            if let Some(root) = page.first() {
                view.root_id = root.id;
            }
        }
        for message in page {
            view.thread.apply(message);
        }
        true
    }

    /// A page of history: folded in, and nothing else — no unread, no
    /// preview, no reply count, no cursor.
    pub fn apply_history(&mut self, chat_id: i64, page: Vec<Message>, more_above: bool) {
        let thread = self.threads.entry(chat_id).or_default();
        for message in page {
            thread.apply(message);
        }
        thread.more_above = more_above;
        thread.paged = true;
    }

    /// Where opening `chat_id` picks up: after the newest message held, and
    /// counting as unread only what the list has not (`counted_up_to`) —
    /// when a page of it has ever been read. None means "read the newest
    /// page": nothing is held, or only what frames brought, above a history
    /// never fetched.
    pub fn resume_point(&self, chat_id: i64) -> Option<(i64, i64)> {
        let newest = self
            .threads
            .get(&chat_id)
            .filter(|thread| thread.paged)
            .and_then(Thread::newest_server_id)?;
        Some((newest, self.counted_up_to(chat_id)))
    }

    /// An edit — the `message_edited` frame, or a page of the edits
    /// catch-up. Applied only to what is HELD: an edit to a message outside
    /// the window has nothing to change, and inserting it would move the
    /// paging cursor. Quotes of it held here are re-cut from the new body,
    /// the way the server will cut them on its next read.
    pub fn apply_edit(&mut self, message: Message) {
        let chat_id = message.chat_id;
        let id = message.id;
        // The list's preview is a copy too, held or not (ios
        // ChatSyncCoordinator `applyEdit` updates the row's preview).
        if let Some(item) = self.item_mut(chat_id) {
            if let Some(last) = item.last_message.as_mut() {
                if last.id == id && message.edit_seq() >= last.edit_seq() {
                    last.body = message.body.clone();
                    last.edited_at = message.edited_at.clone();
                    last.edit_seq = message.edit_seq;
                }
            }
        }
        let applied_body = |thread: &mut Thread| -> Option<String> {
            let held = thread.get_mut(id)?;
            merge(held, message.clone());
            Some(held.body.clone())
        };
        let mut body = self.threads.get_mut(&chat_id).and_then(applied_body);
        if let Some(view) = self.thread_view.as_mut() {
            if view.chat_id == chat_id {
                body = applied_body(&mut view.thread).or(body);
            }
        }
        if let Some(open) = self.open_polls.as_mut() {
            if let Some(held) = open.messages.iter_mut().find(|held| held.id == id) {
                merge(held, message.clone());
            }
        }
        // A finished answer is a finished answer, whatever went before it.
        self.ai_failed.remove(&id);
        let Some(body) = body else { return };
        let recut = excerpt(&body);
        let requote = |thread: &mut Thread| {
            for quoting in thread.messages.iter_mut() {
                if let Some(quote) = quoting.reply_to.as_mut() {
                    if quote.message_id == id {
                        quote.excerpt = recut.clone();
                    }
                    if let Some(parent) = quote.parent.as_mut() {
                        if parent.message_id == id {
                            parent.excerpt = recut.clone();
                        }
                    }
                }
            }
        };
        if let Some(thread) = self.threads.get_mut(&chat_id) {
            requote(thread);
        }
        if let Some(view) = self.thread_view.as_mut() {
            requote(&mut view.thread);
        }
    }

    /// A message's full reaction state, applied only when its `reaction_seq`
    /// is newer than the one held — so an out-of-order frame cannot undo a
    /// newer one. States for messages not held are dropped: history paging
    /// re-delivers them on the messages themselves.
    pub fn apply_reactions(
        &mut self,
        chat_id: i64,
        message_id: i64,
        seq: i64,
        reactions: Vec<Reaction>,
    ) {
        let apply = |held: &mut Message| {
            if newer(Some(seq), held.reaction_seq) {
                held.reaction_seq = Some(seq);
                held.reactions = Some(reactions.clone());
            }
        };
        self.each_copy(chat_id, message_id, apply);
    }

    /// A poll's full state, applied only when its `poll_seq` is newer.
    pub fn apply_poll(&mut self, chat_id: i64, message_id: i64, poll: Poll) {
        let apply = |held: &mut Message| {
            let current = held.poll.as_ref().map(|poll| poll.poll_seq);
            if newer(Some(poll.poll_seq), current) {
                held.poll = Some(poll.clone());
            }
        };
        self.each_copy(chat_id, message_id, apply);
    }

    /// Every copy of one message this client holds: the chat's, the thread
    /// surface's, and the open-polls list's.
    fn each_copy(&mut self, chat_id: i64, message_id: i64, mut apply: impl FnMut(&mut Message)) {
        if let Some(held) = self
            .threads
            .get_mut(&chat_id)
            .and_then(|thread| thread.get_mut(message_id))
        {
            apply(held);
        }
        if let Some(view) = self.thread_view.as_mut() {
            if view.chat_id == chat_id {
                if let Some(held) = view.thread.get_mut(message_id) {
                    apply(held);
                }
            }
        }
        if let Some(open) = self.open_polls.as_mut() {
            if open.chat_id == chat_id {
                if let Some(held) = open.messages.iter_mut().find(|held| held.id == message_id) {
                    apply(held);
                }
            }
        }
    }

    /// Text the assistant is still writing. COSMETIC — the finished row
    /// arrives as `message_edited` and replaces all of it — so a delta for
    /// a row already finished (it has an `edit_seq`) is late and ignored.
    pub fn apply_ai_delta(&mut self, chat_id: i64, message_id: i64, text: &str) {
        self.each_copy(chat_id, message_id, |held| {
            if held.edit_seq.is_none() {
                held.body.push_str(text);
            }
        });
    }

    /// The assistant stopped early: the row keeps what arrived, and says so.
    pub fn apply_ai_error(&mut self, message_id: i64) {
        self.ai_failed.insert(message_id);
    }

    /// Move a catch-up cursor forward — from a catch-up page always, from a
    /// live frame only once this chat has caught up on this connection.
    pub fn advance_cursor(&mut self, chat_id: i64, feed: Feed, seq: i64, from_frame: bool) {
        let cursors = self.cursors.entry(chat_id).or_default();
        if from_frame && !cursors.caught_up {
            return;
        }
        let cursor = match feed {
            Feed::Reactions => &mut cursors.reaction,
            Feed::Edits => &mut cursors.edit,
            Feed::Polls => &mut cursors.poll,
        };
        *cursor = (*cursor).max(seq);
    }

    /// A person pressed Send. The bubble draws NOW, under id 0 — quoting
    /// what it answers, if it answers anything — and the row joins the END
    /// of the outbox; `apply_message` settles both when the server's copy
    /// arrives.
    pub fn queue_send(&mut self, chat_id: i64, client_msg_id: String, draft: Draft) {
        let reply_to = draft
            .reply_to_message_id
            .and_then(|id| self.local_quote(chat_id, id));
        // The chain a pending reply belongs to, decided the server's way —
        // the quoted message's own root, or the quoted message itself — so
        // the thread surface shows it (and any failure of it) at once.
        let thread_root_id = draft.reply_to_message_id.map(|id| {
            self.find(chat_id, id)
                .and_then(|quoted| quoted.thread_root_id)
                .unwrap_or(id)
        });
        self.outbox.push(Outgoing {
            chat_id,
            client_msg_id: client_msg_id.clone(),
            body: draft.body.clone(),
            reply_to_message_id: draft.reply_to_message_id,
            mentions: draft.mentions.clone(),
            poll: draft.poll.clone(),
            attempts: 0,
            failed: None,
        });
        let pending = Message {
            id: 0,
            chat_id,
            sender_id: self.my_user_id,
            client_msg_id: Some(client_msg_id),
            body: draft.body,
            reply_to,
            thread_root_id,
            mentions: (!draft.mentions.is_empty()).then_some(draft.mentions),
            // A poll that is not numbered yet cannot be voted on: options
            // with no ids draw as the question and its choices, disabled.
            poll: draft.poll.map(|options| Poll {
                poll_seq: 0,
                closed: false,
                options: options
                    .into_iter()
                    .map(|text| PollOption {
                        id: 0,
                        text,
                        votes: Vec::new(),
                    })
                    .collect(),
            }),
            ..Message::default()
        };
        self.apply_message(pending, None, Via::Pending);
    }

    /// The quote a pending reply draws until the server's own arrives,
    /// built the way the server builds it.
    fn local_quote(&self, chat_id: i64, id: i64) -> Option<ReplyTo> {
        let quoted = self.find(chat_id, id)?;
        Some(ReplyTo {
            message_id: quoted.id,
            sender_id: quoted.sender_id,
            excerpt: excerpt(&quoted.body),
            parent: quoted.reply_to.as_ref().map(|quote| ReplyParent {
                message_id: quote.message_id,
                sender_id: quote.sender_id,
                excerpt: quote.excerpt.clone(),
            }),
        })
    }

    /// What a catch-up of `chat_id` must not count again: everything up to
    /// the list's last message, and everything already held (see
    /// `Via::CatchUp`).
    pub fn counted_up_to(&self, chat_id: i64) -> i64 {
        let listed = self
            .item(chat_id)
            .and_then(|item| item.last_message.as_ref())
            .map(|message| message.id)
            .unwrap_or(0);
        let held = self
            .threads
            .get(&chat_id)
            .and_then(Thread::newest_server_id)
            .unwrap_or(0);
        listed.max(held)
    }

    /// A message this client holds, wherever it holds it.
    pub fn find(&self, chat_id: i64, id: i64) -> Option<&Message> {
        self.threads
            .get(&chat_id)
            .and_then(|thread| thread.get(id))
            .or_else(|| {
                self.thread_view
                    .as_ref()
                    .filter(|view| view.chat_id == chat_id)
                    .and_then(|view| view.thread.get(id))
            })
    }

    /// What to send next: the OLDEST row still queued. A failed row is
    /// stepped over — it waits for a person — so one refusal does not hold
    /// back everything written after it.
    pub fn next_to_send(&self) -> Option<Outgoing> {
        self.outbox.iter().find(|row| row.failed.is_none()).cloned()
    }

    /// The server's answer to this row's own POST: the row goes, whatever
    /// else is true, and the message it answered with takes the bubble's
    /// place. Removed by `client_msg_id` HERE rather than left to
    /// `apply_message`'s sender check, because a row that outlived its
    /// answer would be sent again, and again, for ever.
    pub fn settle(&mut self, client_msg_id: &str, message: Message, reading: Option<i64>) {
        self.outbox.retain(|row| row.client_msg_id != client_msg_id);
        self.apply_message(message, reading, Via::Answer);
    }

    /// The server read the send and refused it. The row fails NOW, with
    /// the reason, and nobody tries it again until a person asks.
    pub fn refuse(&mut self, client_msg_id: &str, reason: String) {
        if let Some(row) = self.row_mut(client_msg_id) {
            row.failed = Some(reason);
        }
    }

    /// Nobody knows whether it landed. That is counted, and shown as a
    /// failure only once the attempts run out — an unknown outcome is not a
    /// red bubble (docs/protocol.md). Returns the count while the row is
    /// still queued, and None once it has failed or is gone.
    pub fn note_unknown(&mut self, client_msg_id: &str) -> Option<u32> {
        let row = self.row_mut(client_msg_id)?;
        row.attempts += 1;
        if row.attempts >= MAX_SEND_ATTEMPTS {
            row.failed = Some("Not sent. Check your connection and try again.".to_string());
            return None;
        }
        Some(row.attempts)
    }

    /// A person pressed Retry: queued again with a fresh count, and IN
    /// PLACE, so it goes before anything written after it.
    pub fn retry(&mut self, client_msg_id: &str) {
        if let Some(row) = self.row_mut(client_msg_id) {
            row.attempts = 0;
            row.failed = None;
        }
    }

    /// A person gave up on it: the row goes, and so does its bubble.
    pub fn discard(&mut self, client_msg_id: &str) {
        let Some(index) = self
            .outbox
            .iter()
            .position(|row| row.client_msg_id == client_msg_id)
        else {
            return;
        };
        let row = self.outbox.remove(index);
        let is_it = |message: &Message| {
            message.id == 0 && message.client_msg_id.as_deref() == Some(client_msg_id)
        };
        let Some(thread) = self.threads.get_mut(&row.chat_id) else {
            return;
        };
        thread.messages.retain(|message| !is_it(message));
        // The list's preview was this bubble; it goes back to what the
        // thread now ends with.
        let newest = thread.messages.last().cloned();
        if let Some(item) = self.item_mut(row.chat_id) {
            if item.last_message.as_ref().is_some_and(is_it) {
                item.last_message = newest;
            }
        }
    }

    /// The rows of one chat that have FAILED, by `client_msg_id`, with why.
    /// A pending bubble that is not in here is still being sent.
    pub fn failed_sends(&self, chat_id: i64) -> HashMap<String, String> {
        self.outbox
            .iter()
            .filter(|row| row.chat_id == chat_id)
            .filter_map(|row| {
                row.failed
                    .clone()
                    .map(|reason| (row.client_msg_id.clone(), reason))
            })
            .collect()
    }

    fn row_mut(&mut self, client_msg_id: &str) -> Option<&mut Outgoing> {
        self.outbox
            .iter_mut()
            .find(|row| row.client_msg_id == client_msg_id)
    }

    /// This device has read up to here: the badge and the "@" go, and the
    /// marker moves MONOTONICALLY — a stale report must never walk it
    /// backwards.
    pub fn mark_read(&mut self, chat_id: i64, up_to: i64) {
        if let Some(item) = self.item_mut(chat_id) {
            item.unread_count = 0;
            item.mentioned = false;
            item.last_read_message_id = item.last_read_message_id.max(up_to);
        }
    }

    /// A `read` frame. My OWN marker from another device moves mine and may
    /// clear this badge; somebody else's is a "seen" fact only in a DIRECT
    /// chat, where there is one peer and the marker means one person — in
    /// the family chat it is roster data no bubble draws.
    pub fn apply_read_frame(&mut self, chat_id: i64, user_id: i64, up_to: i64) {
        if user_id != self.my_user_id {
            if self.item(chat_id).is_some_and(|item| item.chat.is_direct()) {
                let marker = self.peer_read.entry(chat_id).or_insert(0);
                *marker = (*marker).max(up_to);
            }
            return;
        }
        let me = self.my_user_id;
        let thread = self.threads.get(&chat_id);
        let held_newest = thread.and_then(Thread::newest_server_id).unwrap_or(0);
        let Some(item) = self.chats.iter_mut().find(|item| item.chat.id == chat_id) else {
            return;
        };
        let marker = item.last_read_message_id.max(up_to);
        item.last_read_message_id = marker;
        let newest = held_newest.max(item.last_message.as_ref().map(|m| m.id).unwrap_or(0));
        // Read on another device: this one's badge follows — to zero when the
        // marker caught up with the newest thing known, and otherwise to
        // what is still above it, when the window held covers the marker
        // (ios ChatSyncCoordinator recounts held rows above it).
        if marker >= newest {
            item.unread_count = 0;
            item.mentioned = false;
        } else if let Some(thread) =
            thread.filter(|thread| thread.oldest.is_some_and(|oldest| oldest <= marker))
        {
            item.unread_count = thread
                .messages
                .iter()
                .filter(|message| message.id > marker && message.sender_id != me)
                .count() as i64;
        }
    }

    /// `member_blocked` — full state, applied as a set. Blocking hides a
    /// direct chat from the blocker's list (the server stops listing it
    /// too); unblocking brings it back on the next `GET /chats`.
    pub fn set_blocked(&mut self, user_id: i64, blocked: bool) {
        if blocked {
            self.blocked.insert(user_id);
            self.chats
                .retain(|item| !(item.chat.is_direct() && item.chat.peer_user_id == Some(user_id)));
        } else {
            self.blocked.remove(&user_id);
        }
    }

    pub fn member_joined(&mut self, user: &User) {
        self.names.insert(user.id, user.display_name.clone());
        if !self.members.iter().any(|member| member.id == user.id) {
            self.members.push(Member {
                id: user.id,
                display_name: user.display_name.clone(),
                username: user.username.clone(),
                role: Some("member".to_string()),
                deleted: false,
            });
        }
    }

    /// Somebody left. They are no longer offered as somebody to name, but
    /// their name stays on everything they said.
    pub fn member_left(&mut self, user_id: i64) {
        self.members.retain(|member| member.id != user_id);
    }

    /// An account was deleted: the tombstone is WRITTEN, deliberately — the
    /// one frame whose job is to wipe what is stored.
    pub fn member_deleted(&mut self, member: &Member) {
        self.members.retain(|held| held.id != member.id);
        self.names.insert(member.id, member.display_name.clone());
    }

    pub fn family_owner(&mut self, user_id: i64) {
        for member in self.members.iter_mut() {
            member.role = Some(
                if member.id == user_id {
                    "owner"
                } else {
                    "member"
                }
                .to_string(),
            );
        }
    }

    /// How many of the family chat's open polls this reader has not voted
    /// in — the badge's number (ios OpenPollsBadge). A closed poll never
    /// counts, nor a question the reader has chosen not to see.
    pub fn unanswered_polls(&self, chat_id: i64) -> usize {
        let mut seen = HashSet::new();
        let held = self
            .threads
            .get(&chat_id)
            .map(|thread| thread.messages.as_slice())
            .unwrap_or(&[]);
        let listed = self
            .open_polls
            .as_ref()
            .filter(|open| open.chat_id == chat_id)
            .map(|open| open.messages.as_slice())
            .unwrap_or(&[]);
        held.iter()
            .chain(listed.iter())
            .filter(|message| message.id != 0 && seen.insert(message.id))
            .filter(|message| !self.blocked.contains(&message.sender_id))
            .filter_map(|message| message.poll.as_ref())
            .filter(|poll| !poll.closed)
            .filter(|poll| {
                !poll
                    .options
                    .iter()
                    .any(|option| option.votes.contains(&self.my_user_id))
            })
            .count()
    }

    /// A `typing` frame arrived. `now` is passed in rather than read so
    /// this stays testable without a clock. Mine, and a blocked member's,
    /// are not news.
    pub fn set_typing(&mut self, chat_id: i64, user_id: i64, now: f64) {
        if user_id == self.my_user_id || self.blocked.contains(&user_id) {
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

    /// Who is typing here, by name, in a steady order.
    pub fn typing_names(&self, chat_id: i64, now: f64) -> Vec<String> {
        let Some(typing) = self.typing.get(&chat_id) else {
            return Vec::new();
        };
        let mut live: Vec<(&i64, &f64)> = typing
            .iter()
            .filter(|(_, stamp)| now - **stamp < TYPING_TTL_MS)
            .collect();
        // By member, not by whose frame came last, so "Anna and Bob" does
        // not keep swapping while somebody reads it (ios typingNames).
        live.sort_by_key(|(user, _)| **user);
        live.into_iter()
            .map(|(user, _)| self.name_of(*user))
            .collect()
    }

    /// Whoever this is, by name — "Someone" rather than a number when the
    /// roster does not know them.
    pub fn name_of(&self, user_id: i64) -> String {
        self.names
            .get(&user_id)
            .cloned()
            .unwrap_or_else(|| "Someone".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Chat, PollOption};
    use wasm_bindgen_test::*;

    const ME: i64 = 7;
    const ANNA: i64 = 9;

    fn message(id: i64, sender: i64, body: &str) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id: sender,
            body: body.into(),
            created_at: "2026-08-19T17:03:12Z".into(),
            ..Message::default()
        }
    }

    fn pending(client_msg_id: &str, body: &str) -> Message {
        Message {
            id: 0,
            chat_id: 42,
            sender_id: ME,
            client_msg_id: Some(client_msg_id.into()),
            body: body.into(),
            created_at: "2026-08-19T17:04:00Z".into(),
            ..Message::default()
        }
    }

    fn chat(id: i64, kind: &str) -> ChatListItem {
        ChatListItem {
            chat: Chat {
                id,
                kind: kind.into(),
                title: Some("The Smiths".into()),
                peer_user_id: (kind == "direct").then_some(ANNA),
            },
            last_message: None,
            unread_count: 0,
            last_read_message_id: 0,
            max_reaction_seq: None,
            max_edit_seq: None,
            max_poll_seq: None,
            mentioned: false,
        }
    }

    fn store() -> Store {
        Store {
            my_user_id: ME,
            chats: vec![chat(42, "family")],
            ..Store::default()
        }
    }

    fn text(body: &str) -> Draft {
        Draft {
            body: body.into(),
            ..Draft::default()
        }
    }

    /// The SAME message arrives on three paths — a history page, the socket
    /// and this device's own answer. It is one message, not three.
    #[wasm_bindgen_test]
    fn a_message_seen_twice_is_one_message() {
        let mut thread = Thread::default();
        assert_eq!(
            thread.apply(message(100, ANNA, "Dinner at 7?")),
            Arrival::New
        );
        assert_eq!(
            thread.apply(message(100, ANNA, "Dinner at 7?")),
            Arrival::Known
        );
        assert_eq!(thread.messages.len(), 1);
        thread.apply(message(101, ME, "Six works"));
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.oldest, Some(100));
        assert_eq!(thread.newest_server_id(), Some(101));
    }

    /// The optimistic bubble BECOMES the real one, and that is a NEW arrival
    /// — the first time the server's id is seen.
    #[wasm_bindgen_test]
    fn an_answer_replaces_the_pending_row_rather_than_adding_one() {
        let mut thread = Thread::default();
        thread.apply(pending("8f14e45f", "Six works"));
        assert_eq!(thread.newest_server_id(), None, "nothing is numbered yet");

        let mut acked = message(101, ME, "Six works");
        acked.client_msg_id = Some("8f14e45f".into());
        assert_eq!(thread.apply(acked), Arrival::New);
        assert_eq!(thread.messages.len(), 1, "one bubble, not two");
        assert_eq!(thread.messages[0].id, 101);
    }

    #[wasm_bindgen_test]
    fn a_thread_is_ordered_oldest_first_with_pending_rows_at_the_end() {
        let mut thread = Thread::default();
        thread.apply(pending("later", "typing this now"));
        thread.apply(message(103, ANNA, "c"));
        thread.apply(message(101, ANNA, "a"));
        thread.apply(message(102, ANNA, "b"));
        let ids: Vec<i64> = thread.messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![101, 102, 103, 0]);
        assert_eq!(thread.oldest, Some(101));
    }

    // --- The three guards -------------------------------------------------

    /// THE EDIT GUARD, and it is load-bearing: a page fetched BEFORE an edit
    /// but delivered after it must not restore the old words.
    #[wasm_bindgen_test]
    fn an_older_copy_never_undoes_an_edit() {
        let mut thread = Thread::default();
        let mut edited = message(100, ANNA, "Dinner at 8");
        edited.edit_seq = Some(5);
        edited.edited_at = Some("…".into());
        thread.apply(edited);

        thread.apply(message(100, ANNA, "Dinner at 7?")); // the stale page
        assert_eq!(thread.messages[0].body, "Dinner at 8");
        assert_eq!(thread.messages[0].edit_seq, Some(5));

        // An equal seq is the same edit, and applies.
        let mut again = message(100, ANNA, "Dinner at 8");
        again.edit_seq = Some(5);
        thread.apply(again);
        assert_eq!(thread.messages[0].body, "Dinner at 8");

        let mut later = message(100, ANNA, "Dinner at 9");
        later.edit_seq = Some(6);
        thread.apply(later);
        assert_eq!(thread.messages[0].body, "Dinner at 9");
    }

    #[wasm_bindgen_test]
    fn reactions_and_polls_only_move_forward() {
        let mut thread = Thread::default();
        let mut reacted = message(100, ANNA, "Pizza or pasta?");
        reacted.reactions = Some(vec![Reaction {
            user_id: ME,
            emoji: "❤️".into(),
        }]);
        reacted.reaction_seq = Some(10);
        reacted.poll = Some(Poll {
            poll_seq: 20,
            closed: false,
            options: vec![PollOption {
                id: 5,
                text: "Pizza".into(),
                votes: vec![ME],
            }],
        });
        thread.apply(reacted);

        // A stale copy with no reactions and an older poll changes neither.
        let mut stale = message(100, ANNA, "Pizza or pasta?");
        stale.poll = Some(Poll {
            poll_seq: 19,
            closed: false,
            options: vec![PollOption {
                id: 5,
                text: "Pizza".into(),
                votes: vec![],
            }],
        });
        thread.apply(stale);
        let held = &thread.messages[0];
        assert_eq!(held.reactions().len(), 1, "no data is not cleared");
        assert_eq!(
            held.poll.as_ref().map(|p| p.options[0].votes.clone()),
            Some(vec![ME])
        );
    }

    /// Reply counts: any SERVER copy says the truth, and a live reply adds
    /// one — but a history page's replies do not.
    #[wasm_bindgen_test]
    fn a_live_reply_raises_its_roots_count_and_a_page_does_not() {
        let mut store = store();
        store.apply_history(42, vec![message(100, ANNA, "Dinner?")], false);

        let mut reply = message(101, ANNA, "at 7");
        reply.thread_root_id = Some(100);
        store.apply_history(42, vec![reply.clone()], false);
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            None
        );

        let mut live_reply = message(102, ME, "works");
        live_reply.thread_root_id = Some(100);
        store.apply_message(live_reply.clone(), None, Via::Frame);
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(1)
        );

        // The same reply again is not a second reply.
        store.apply_message(live_reply, None, Via::Frame);
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(1)
        );

        // And the root's own server copy overwrites with the recomputed truth.
        let mut root = message(100, ANNA, "Dinner?");
        root.reply_count = Some(2);
        store.apply_history(42, vec![root], false);
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(2)
        );
    }

    /// The count is the one part an OLDER copy still overrules: a history
    /// page fetched before the root was edited keeps the edited words out,
    /// and still brings the recomputed number of replies in.
    #[wasm_bindgen_test]
    fn an_older_copy_keeps_the_edit_but_brings_the_true_reply_count() {
        let mut thread = Thread::default();
        let mut edited_root = message(100, ANNA, "Dinner at 8");
        edited_root.edit_seq = Some(5);
        edited_root.reply_count = Some(1);
        thread.apply(edited_root);

        let mut older_page = message(100, ANNA, "Dinner at 7?");
        older_page.reply_count = Some(3);
        thread.apply(older_page);

        let held = thread.get(100).expect("held");
        assert_eq!(held.body, "Dinner at 8", "the edit stands");
        assert_eq!(held.reply_count, Some(3), "the server's count wins");
    }

    // --- Unread, mentions and the list ------------------------------------

    #[wasm_bindgen_test]
    fn unread_counts_only_somebody_elses_new_message_in_a_chat_not_being_read() {
        let mut store = store();
        store.apply_message(message(100, ANNA, "Dinner?"), None, Via::Frame);
        assert_eq!(store.chats[0].unread_count, 1);
        store.apply_message(message(101, ME, "Six works"), None, Via::Frame);
        assert_eq!(store.chats[0].unread_count, 1, "mine never counts");
        store.apply_message(message(102, ANNA, "See you"), Some(42), Via::Frame);
        assert_eq!(store.chats[0].unread_count, 1, "nor one being read");
        store.apply_message(message(100, ANNA, "Dinner?"), None, Via::Frame);
        assert_eq!(store.chats[0].unread_count, 1, "nor the same message twice");
    }

    /// A catch-up page after `GET /chats` must not count what that read
    /// already counted — but its replies still raise their root's count,
    /// because the root this client holds predates them.
    #[wasm_bindgen_test]
    fn a_catch_up_page_moves_no_badge_but_counts_its_replies() {
        let mut store = store();
        store.apply_history(42, vec![message(100, ANNA, "Dinner?")], false);
        store.chats[0].unread_count = 3; // what GET /chats just said

        let mut reply = message(101, ANNA, "at 7");
        reply.thread_root_id = Some(100);
        store.chats[0].last_message = Some(message(101, ANNA, "at 7"));
        store.apply_message(reply, None, Via::CatchUp { counted_up_to: 101 });

        assert_eq!(store.chats[0].unread_count, 3, "the server's count stands");
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(1)
        );
        assert_eq!(store.threads[&42].newest_server_id(), Some(101));
    }

    /// The "@" mark: a live message naming ME, from somebody I have not
    /// blocked, in a chat I am not reading — and reading clears it.
    #[wasm_bindgen_test]
    fn a_live_mention_marks_the_row_until_it_is_read() {
        let mut store = store();
        let mut naming = message(100, ANNA, "@Me are you in?");
        naming.mentions = Some(vec![Mention {
            user_id: ME,
            name: "Me".into(),
        }]);
        store.apply_message(naming.clone(), Some(42), Via::Frame);
        assert!(
            !store.chats[0].mentioned,
            "read at once by a reader looking"
        );

        naming.id = 101;
        store.apply_message(naming.clone(), None, Via::Frame);
        assert!(store.chats[0].mentioned);
        store.mark_read(42, 101);
        assert!(!store.chats[0].mentioned);

        store.blocked.insert(ANNA);
        naming.id = 102;
        store.apply_message(naming, None, Via::Frame);
        assert!(
            !store.chats[0].mentioned,
            "a blocked member's mention wakes nobody"
        );
    }

    #[wasm_bindgen_test]
    fn reading_clears_the_badge_and_the_marker_never_walks_backwards() {
        let mut store = store();
        store.apply_message(message(100, ANNA, "Dinner?"), None, Via::Frame);
        store.mark_read(42, 100);
        assert_eq!(store.chats[0].unread_count, 0);
        store.mark_read(42, 50);
        assert_eq!(store.chats[0].last_read_message_id, 100);
    }

    /// My own `read` frame from another device is mine; somebody else's is
    /// a "seen" fact — in a direct chat only.
    #[wasm_bindgen_test]
    fn a_read_frame_is_my_marker_or_the_peers_seen_mark() {
        let mut store = store();
        store.chats.push(chat(43, "direct"));
        store.apply_message(message(100, ANNA, "Dinner?"), None, Via::Frame);
        store.apply_read_frame(42, ANNA, 100);
        assert_eq!(store.chats[0].unread_count, 1, "that was somebody else");
        assert!(
            !store.peer_read.contains_key(&42),
            "the family chat has no seen mark"
        );

        store.apply_read_frame(42, ME, 100);
        assert_eq!(store.chats[0].unread_count, 0);

        store.apply_read_frame(43, ANNA, 55);
        store.apply_read_frame(43, ANNA, 40);
        assert_eq!(store.peer_read.get(&43), Some(&55), "monotonic");
    }

    #[wasm_bindgen_test]
    fn a_partial_read_elsewhere_does_not_clear_the_badge() {
        let mut store = store();
        store.apply_message(message(100, ANNA, "one"), None, Via::Frame);
        store.apply_message(message(101, ANNA, "two"), None, Via::Frame);
        store.apply_read_frame(42, ME, 100);
        assert_eq!(store.chats[0].unread_count, 1, "101 is still unread");
        store.apply_read_frame(42, ME, 101);
        assert_eq!(store.chats[0].unread_count, 0);
    }

    #[wasm_bindgen_test]
    fn the_family_chat_is_first_and_the_rest_are_newest_first() {
        let mut store = store();
        let mut older = chat(43, "direct");
        older.last_message = Some(message(50, ANNA, "old"));
        let mut newer = chat(44, "ai");
        newer.last_message = Some(message(90, ANNA, "new"));
        store.chats.push(older);
        store.chats.push(newer);
        store.chats[0].last_message = Some(message(10, ANNA, "oldest"));
        let order: Vec<i64> = store
            .sorted_chats()
            .iter()
            .map(|item| item.chat.id)
            .collect();
        assert_eq!(order, vec![42, 44, 43]);
    }

    // --- Edits ------------------------------------------------------------

    /// An edit re-cuts every quote of the message this client holds, the way
    /// the server will cut it on its next read.
    #[wasm_bindgen_test]
    fn an_edit_recuts_the_quotes_of_it() {
        let mut store = store();
        let mut reply = message(101, ME, "sure");
        reply.reply_to = Some(ReplyTo {
            message_id: 100,
            sender_id: ANNA,
            excerpt: "Dinner at 7?".into(),
            parent: None,
        });
        store.apply_history(42, vec![message(100, ANNA, "Dinner at 7?"), reply], false);

        let mut edited = message(100, ANNA, &"x".repeat(200));
        edited.edit_seq = Some(3);
        store.apply_edit(edited);

        let quote = store.threads[&42]
            .get(101)
            .and_then(|m| m.reply_to.clone())
            .expect("quote");
        assert_eq!(
            quote.excerpt.chars().count(),
            fc_text::excerpt::MAX_EXCERPT_CHARS
        );
    }

    /// An edit to a message outside the window changes nothing and inserts
    /// nothing — inserting would move the paging cursor.
    #[wasm_bindgen_test]
    fn an_edit_to_a_message_not_held_is_dropped() {
        let mut store = store();
        store.apply_history(42, vec![message(100, ANNA, "held")], true);
        let mut elsewhere = message(5, ANNA, "old news");
        elsewhere.edit_seq = Some(1);
        store.apply_edit(elsewhere);
        assert_eq!(store.threads[&42].messages.len(), 1);
        assert_eq!(store.threads[&42].oldest, Some(100));
    }

    // --- The assistant ----------------------------------------------------

    #[wasm_bindgen_test]
    fn deltas_grow_the_answer_until_the_finished_row_replaces_it() {
        let mut store = store();
        store.apply_message(message(100, 2, ""), Some(42), Via::Frame);
        store.apply_ai_delta(42, 100, "Sure — ");
        store.apply_ai_delta(42, 100, "seven.");
        assert_eq!(
            store.threads[&42].get(100).map(|m| m.body.as_str()),
            Some("Sure — seven.")
        );

        let mut done = message(100, 2, "Sure — seven works.");
        done.edit_seq = Some(1);
        store.apply_edit(done);
        store.apply_ai_delta(42, 100, " late");
        assert_eq!(
            store.threads[&42].get(100).map(|m| m.body.as_str()),
            Some("Sure — seven works."),
            "a delta after the finished row is late and ignored"
        );
    }

    // --- Cursors ----------------------------------------------------------

    /// Frames move a cursor only once the chat has caught up on this
    /// connection; a catch-up page always does, and only forwards.
    #[wasm_bindgen_test]
    fn a_frame_does_not_move_a_cursor_before_the_catch_up_has_finished() {
        let mut store = store();
        store.advance_cursor(42, Feed::Reactions, 130, true);
        assert_eq!(store.cursors.get(&42).map(|c| c.reaction).unwrap_or(0), 0);

        store.advance_cursor(42, Feed::Reactions, 100, false);
        store.cursors.get_mut(&42).expect("cursors").caught_up = true;
        store.advance_cursor(42, Feed::Reactions, 130, true);
        store.advance_cursor(42, Feed::Reactions, 120, false);
        assert_eq!(store.cursors[&42].reaction, 130, "only ever forwards");
    }

    // --- Blocking ---------------------------------------------------------

    #[wasm_bindgen_test]
    fn blocking_hides_the_direct_chat_and_the_list_is_replaced_not_merged() {
        let mut store = store();
        store.chats.push(chat(43, "direct"));
        store.set_blocked(ANNA, true);
        assert!(
            store.item(43).is_none(),
            "a direct chat with somebody blocked is not listed"
        );
        assert!(store.item(42).is_some(), "the family chat stays");

        let me = Me {
            user: User {
                id: ME,
                username: "me".into(),
                display_name: "Me".into(),
                deleted: false,
            },
            family: None,
            blocked_user_ids: vec![11],
            support_contact: None,
        };
        store.apply_me(&me);
        assert_eq!(store.blocked, HashSet::from([11]), "replaced whole");
    }

    #[wasm_bindgen_test]
    fn a_blocked_members_typing_is_not_news() {
        let mut store = store();
        store.blocked.insert(ANNA);
        store.set_typing(42, ANNA, 1_000.0);
        assert!(store.typing_names(42, 1_100.0).is_empty());
    }

    // --- Polls ------------------------------------------------------------

    /// The badge counts open polls I have not voted in — not closed ones,
    /// not a blocked member's, and each poll once.
    #[wasm_bindgen_test]
    fn the_polls_badge_counts_what_is_still_mine_to_answer() {
        let mut store = store();
        let poll = |id: i64, sender: i64, votes: Vec<i64>, closed: bool| {
            let mut asking = message(id, sender, "Pizza or pasta?");
            asking.poll = Some(Poll {
                poll_seq: 1,
                closed,
                options: vec![PollOption {
                    id: 1,
                    text: "Pizza".into(),
                    votes,
                }],
            });
            asking
        };
        store.apply_history(
            42,
            vec![
                poll(100, ANNA, vec![], false),
                poll(101, ANNA, vec![ME], false),
                poll(102, ANNA, vec![], true),
                poll(103, 11, vec![], false),
            ],
            false,
        );
        store.blocked.insert(11);
        store.open_polls = Some(OpenPolls {
            chat_id: 42,
            messages: vec![
                poll(100, ANNA, vec![], false),
                poll(90, ANNA, vec![], false),
            ],
        });
        assert_eq!(store.unanswered_polls(42), 2, "100 (once) and 90");
    }

    #[wasm_bindgen_test]
    fn a_poll_state_reaches_every_copy_held() {
        let mut store = store();
        let mut asking = message(100, ANNA, "Pizza or pasta?");
        asking.poll = Some(Poll {
            poll_seq: 1,
            closed: false,
            options: vec![PollOption {
                id: 1,
                text: "Pizza".into(),
                votes: vec![],
            }],
        });
        store.apply_history(42, vec![asking.clone()], false);
        store.open_polls = Some(OpenPolls {
            chat_id: 42,
            messages: vec![asking],
        });
        store.apply_poll(
            42,
            100,
            Poll {
                poll_seq: 2,
                closed: true,
                options: vec![],
            },
        );
        assert!(store.threads[&42]
            .get(100)
            .and_then(|m| m.poll.as_ref())
            .is_some_and(|p| p.closed));
        assert!(store
            .open_polls
            .as_ref()
            .is_some_and(|open| open.messages[0].poll.as_ref().is_some_and(|p| p.closed)));
    }

    // --- The thread surface -----------------------------------------------

    /// A live reply lands on the open thread surface too, and raises the
    /// root's count there — the surface may hold a root the chat does not.
    #[wasm_bindgen_test]
    fn a_live_reply_reaches_the_open_thread_surface() {
        let mut store = store();
        let mut root = message(10, ANNA, "Dinner?");
        root.reply_count = Some(1);
        let mut first = message(11, ME, "at 7");
        first.thread_root_id = Some(10);
        let mut view = ThreadView {
            chat_id: 42,
            root_id: 10,
            generation: 1,
            thread: Thread::default(),
        };
        view.thread.apply(root);
        view.thread.apply(first);
        store.thread_view = Some(view);

        let mut second = message(200, ANNA, "8?");
        second.thread_root_id = Some(10);
        store.apply_message(second, Some(42), Via::Frame);

        let view = store.thread_view.as_ref().expect("still open");
        assert_eq!(view.thread.messages.len(), 3);
        assert_eq!(view.thread.get(10).and_then(|m| m.reply_count), Some(2));
        assert_eq!(
            store.threads[&42].oldest,
            Some(200),
            "the chat's own window did not gain the old root"
        );
    }

    // --- The outbox -------------------------------------------------------

    #[wasm_bindgen_test]
    fn a_send_draws_at_once_and_queues_in_the_order_written() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("first"));
        store.queue_send(42, "b".into(), text("second"));

        let thread = &store.threads[&42];
        assert_eq!(thread.messages.len(), 2);
        assert!(thread.messages.iter().all(|message| message.id == 0));
        assert!(thread
            .messages
            .iter()
            .all(|message| message.sender_id == ME));
        assert_eq!(
            store.next_to_send().map(|row| row.client_msg_id),
            Some("a".to_string())
        );
        assert_eq!(
            store.chats[0].unread_count, 0,
            "my own message is not unread"
        );
    }

    /// A pending reply draws its quote at once, cut the server's way.
    #[wasm_bindgen_test]
    fn a_pending_reply_quotes_what_it_answers() {
        let mut store = store();
        store.apply_history(42, vec![message(100, ANNA, "Dinner at 7?")], false);
        store.queue_send(
            42,
            "a".into(),
            Draft {
                body: "Six works".into(),
                reply_to_message_id: Some(100),
                ..Draft::default()
            },
        );
        let pending = store.threads[&42].messages.last().expect("pending");
        let quote = pending.reply_to.as_ref().expect("a quote");
        assert_eq!((quote.message_id, quote.sender_id), (100, ANNA));
        assert_eq!(quote.excerpt, "Dinner at 7?");
        assert_eq!(store.outbox[0].reply_to_message_id, Some(100));
    }

    #[wasm_bindgen_test]
    fn the_answer_settles_the_row_and_takes_the_bubbles_place() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("Six works"));
        let mut delivered = message(101, ME, "Six works");
        delivered.client_msg_id = Some("a".into());
        store.settle("a", delivered, Some(42));
        assert!(store.outbox.is_empty());
        let ids: Vec<i64> = store.threads[&42].messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![101]);
    }

    #[wasm_bindgen_test]
    fn an_answer_settles_its_row_whatever_it_says_about_the_sender() {
        let mut store = store();
        store.my_user_id = 0; // `/me` has not answered yet
        store.queue_send(42, "a".into(), text("Six works"));
        let mut delivered = message(101, ME, "Six works");
        delivered.client_msg_id = Some("a".into());
        store.settle("a", delivered, Some(42));
        assert!(store.outbox.is_empty());
    }

    #[wasm_bindgen_test]
    fn the_sockets_copy_landing_first_settles_the_row_too() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("Six works"));
        let mut theirs = message(100, ANNA, "Six works");
        theirs.client_msg_id = Some("a".into());
        store.apply_message(theirs, Some(42), Via::Frame);
        assert_eq!(store.outbox.len(), 1, "another sender's key is another key");

        let mut mine = message(101, ME, "Six works");
        mine.client_msg_id = Some("a".into());
        store.apply_message(mine, Some(42), Via::Frame);
        assert!(store.outbox.is_empty(), "my own copy settled it");
    }

    #[wasm_bindgen_test]
    fn unknown_is_not_failed_until_the_attempts_run_out() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("first"));
        store.queue_send(42, "b".into(), text("second"));
        for attempt in 1..MAX_SEND_ATTEMPTS {
            assert_eq!(store.note_unknown("a"), Some(attempt));
            assert!(store.failed_sends(42).is_empty());
        }
        assert_eq!(store.note_unknown("a"), None);
        assert!(store.failed_sends(42).contains_key("a"));
        assert_eq!(
            store.next_to_send().map(|row| row.client_msg_id),
            Some("b".to_string())
        );
        assert_eq!(store.note_unknown("gone"), None);
    }

    #[wasm_bindgen_test]
    fn a_refusal_fails_at_once_with_its_reason() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("first"));
        store.refuse("a", "Not sent: blocked.".into());
        assert_eq!(
            store.failed_sends(42).get("a").map(String::as_str),
            Some("Not sent: blocked.")
        );
        assert_eq!(store.next_to_send(), None);
    }

    #[wasm_bindgen_test]
    fn retry_requeues_in_place_with_a_fresh_count() {
        let mut store = store();
        store.queue_send(42, "a".into(), text("first"));
        store.queue_send(42, "b".into(), text("second"));
        store.note_unknown("a");
        store.refuse("a", "Not sent.".into());
        store.retry("a");
        let next = store.next_to_send().expect("something to send");
        assert_eq!(next.client_msg_id, "a");
        assert_eq!(next.attempts, 0);
    }

    #[wasm_bindgen_test]
    fn discard_drops_the_row_its_bubble_and_its_preview() {
        let mut store = store();
        store.apply_message(message(100, ANNA, "Dinner?"), Some(42), Via::Frame);
        store.queue_send(42, "a".into(), text("never mind"));
        store.discard("a");
        assert!(store.outbox.is_empty());
        let ids: Vec<i64> = store.threads[&42].messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![100]);
        assert_eq!(
            store.chats[0].last_message.as_ref().map(|m| m.id),
            Some(100)
        );
    }

    // --- Typing -----------------------------------------------------------

    #[wasm_bindgen_test]
    fn typing_is_other_people_and_stops_when_they_say_something() {
        let mut store = store();
        store.names.insert(ANNA, "Anna".into());
        store.set_typing(42, ANNA, 1_000.0);
        store.set_typing(42, ME, 1_000.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Anna".to_string()]);
        store.apply_message(message(100, ANNA, "Dinner?"), Some(42), Via::Frame);
        assert!(store.typing_names(42, 1_100.0).is_empty());
    }

    #[wasm_bindgen_test]
    fn a_typing_frame_expires_on_its_own() {
        let mut store = store();
        store.names.insert(ANNA, "Anna".into());
        store.set_typing(42, ANNA, 1_000.0);
        assert_eq!(
            store.typing_names(42, 1_000.0 + TYPING_TTL_MS - 1.0).len(),
            1
        );
        assert!(store.typing_names(42, 1_000.0 + TYPING_TTL_MS).is_empty());
        store.prune_typing(1_000.0 + TYPING_TTL_MS);
        assert!(store.typing.is_empty());
    }

    #[wasm_bindgen_test]
    fn a_sender_who_is_not_in_the_roster_still_has_a_name() {
        let mut store = store();
        store.set_typing(42, 11, 1_000.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Someone".to_string()]);
    }

    // --- The roster -------------------------------------------------------

    #[wasm_bindgen_test]
    fn the_roster_follows_joins_leaves_deletions_and_a_new_owner() {
        let mut store = store();
        store.member_joined(&User {
            id: 11,
            username: "junior".into(),
            display_name: "Junior".into(),
            deleted: false,
        });
        assert_eq!(store.name_of(11), "Junior");
        assert!(store.members.iter().any(|m| m.id == 11));

        store.family_owner(11);
        assert_eq!(
            store
                .members
                .iter()
                .find(|m| m.id == 11)
                .and_then(|m| m.role.clone())
                .as_deref(),
            Some("owner")
        );

        store.member_left(11);
        assert!(
            !store.members.iter().any(|m| m.id == 11),
            "not offered any more"
        );
        assert_eq!(
            store.name_of(11),
            "Junior",
            "but still named on what they said"
        );

        store.member_deleted(&Member {
            id: 11,
            display_name: "Deleted account".into(),
            username: String::new(),
            role: None,
            deleted: true,
        });
        assert_eq!(
            store.name_of(11),
            "Deleted account",
            "the tombstone is written"
        );
    }

    // --- The list and the socket, racing --------------------------------

    /// A list read is a snapshot, and this client went on living while it
    /// was in flight. The marker never walks back; a message that arrived
    /// after the snapshot keeps its count and its "@"; a read reported here
    /// past the snapshot's marker is recounted rather than undone.
    #[wasm_bindgen_test]
    fn the_chat_list_is_merged_not_laid_over() {
        let snapshot = |last: i64, unread: i64, marker: i64| {
            let mut item = chat(42, "family");
            item.last_message = Some(message(last, ANNA, "x"));
            item.unread_count = unread;
            item.last_read_message_id = marker;
            item
        };

        let held = || {
            let mut held = store();
            for id in 100..=103 {
                held.apply_message(message(id, ANNA, "x"), None, Via::Frame);
            }
            held
        };

        // Read to the end here; the server had not heard yet.
        let mut read_here = held();
        read_here.mark_read(42, 103);
        read_here.merge_chats(vec![snapshot(103, 3, 100)]);
        assert_eq!(
            read_here.chats[0].last_read_message_id, 103,
            "never backwards"
        );
        assert_eq!(read_here.chats[0].unread_count, 0, "and nothing is unread");

        // A message landed after the snapshot, naming me.
        let mut raced = held();
        let mut naming = message(104, ANNA, "@Me?");
        naming.mentions = Some(vec![Mention {
            user_id: ME,
            name: "Me".into(),
        }]);
        raced.apply_message(naming, None, Via::Frame);
        raced.merge_chats(vec![snapshot(103, 3, 100)]);
        assert_eq!(
            raced.chats[0].unread_count, 4,
            "the server's three and mine"
        );
        assert!(raced.chats[0].mentioned);
        assert_eq!(
            raced.chats[0].last_message.as_ref().map(|m| m.id),
            Some(104),
            "the newer preview stands"
        );

        // Read part of the way on another device, past the snapshot's marker.
        let mut elsewhere = held();
        elsewhere.apply_read_frame(42, ME, 102);
        elsewhere.merge_chats(vec![snapshot(103, 3, 100)]);
        assert_eq!(elsewhere.chats[0].last_read_message_id, 102);
        assert_eq!(
            elsewhere.chats[0].unread_count, 1,
            "only 103 is still unread"
        );
    }

    /// The socket and the list race. A frame whose message the list's
    /// answer already counted — landed before the snapshot, its frame after
    /// the answer — is not counted a second time; a newer one is.
    #[wasm_bindgen_test]
    fn a_frame_the_list_already_counted_is_not_counted_twice() {
        let mut store = store();
        let mut listed = chat(42, "family");
        listed.last_message = Some(message(105, ANNA, "five"));
        listed.unread_count = 3;
        listed.last_read_message_id = 102;
        store.merge_chats(vec![listed]);

        store.apply_message(message(105, ANNA, "five"), None, Via::Frame);
        assert_eq!(store.chats[0].unread_count, 3);
        store.apply_message(message(106, ANNA, "six"), None, Via::Frame);
        assert_eq!(store.chats[0].unread_count, 4);
    }

    /// A catch-up counts exactly what the list did not: past its last
    /// message, and not anything held.
    #[wasm_bindgen_test]
    fn a_catch_up_counts_only_what_the_list_had_not() {
        let mut store = store();
        store.apply_history(42, vec![message(100, ANNA, "Dinner?")], false);
        let mut listed = chat(42, "family");
        listed.last_message = Some(message(102, ANNA, "b"));
        listed.unread_count = 2;
        listed.last_read_message_id = 100;
        store.merge_chats(vec![listed]);
        let counted = store.counted_up_to(42);
        assert_eq!(counted, 102);

        for id in 101..=104 {
            store.apply_message(
                message(id, ANNA, "x"),
                None,
                Via::CatchUp {
                    counted_up_to: counted,
                },
            );
        }
        assert_eq!(
            store.chats[0].unread_count, 4,
            "the list's two, and the two after it"
        );
    }

    /// My own read from another device: the badge follows to what is still
    /// above the marker when the window held covers it — and is left alone
    /// when nothing held can say, rather than cleared.
    #[wasm_bindgen_test]
    fn my_read_elsewhere_recounts_what_is_still_above_it() {
        let mut held = store();
        for id in 100..=103 {
            held.apply_message(message(id, ANNA, "x"), None, Via::Frame);
        }
        held.apply_read_frame(42, ME, 101);
        assert_eq!(held.chats[0].unread_count, 2);

        let mut unheld = store();
        unheld.chats[0].last_message = Some(message(110, ANNA, "x"));
        unheld.chats[0].unread_count = 5;
        unheld.apply_read_frame(42, ME, 105);
        assert_eq!(
            unheld.chats[0].unread_count, 5,
            "nothing held to count, so the count stands"
        );
        unheld.apply_read_frame(42, ME, 110);
        assert_eq!(unheld.chats[0].unread_count, 0);
    }

    /// "Anna and Bob", whoever typed last — by member, so it does not keep
    /// swapping while somebody reads it.
    #[wasm_bindgen_test]
    fn the_typing_line_is_in_member_order() {
        let mut store = store();
        store.names.insert(ANNA, "Anna".into());
        store.names.insert(11, "Bob".into());
        store.set_typing(42, 11, 1_000.0);
        store.set_typing(42, ANNA, 1_050.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Anna", "Bob"]);
        store.set_typing(42, 11, 1_080.0);
        assert_eq!(store.typing_names(42, 1_100.0), vec!["Anna", "Bob"]);
    }

    // --- The outbox, across a reload --------------------------------------

    /// A reload in the middle of sending: every row back, with its count and
    /// its failure, each with its bubble, in the order written — and
    /// restoring twice is restoring once.
    #[wasm_bindgen_test]
    fn a_reload_puts_the_outbox_back_bubbles_and_all() {
        let mut before = store();
        before.queue_send(42, "a".into(), text("first"));
        before.queue_send(42, "b".into(), text("second"));
        before.note_unknown("a");
        before.refuse("b", "Not sent: blocked.".into());
        let json = serde_json::to_string(&before.outbox).expect("encodes");
        let rows: Vec<Outgoing> = serde_json::from_str(&json).expect("decodes");

        let mut after = store();
        after.restore_outbox(rows.clone());
        after.restore_outbox(rows);

        assert_eq!(after.outbox, before.outbox);
        let bubbles: Vec<(i64, Option<&str>)> = after.threads[&42]
            .messages
            .iter()
            .map(|m| (m.id, m.client_msg_id.as_deref()))
            .collect();
        assert_eq!(bubbles, vec![(0, Some("a")), (0, Some("b"))]);
        assert_eq!(
            after.failed_sends(42).get("b").map(String::as_str),
            Some("Not sent: blocked.")
        );
        assert_eq!(
            after.next_to_send().map(|row| row.client_msg_id),
            Some("a".to_string())
        );
    }

    // --- Threads, review round --------------------------------------------

    /// A reply written on the thread surface shows THERE at once — while it
    /// is sending, and if it fails — rooted the server's way: replying to a
    /// reply is still the root's chain.
    #[wasm_bindgen_test]
    fn a_pending_reply_joins_the_open_thread_at_once() {
        let mut store = store();
        let mut root = message(10, ANNA, "Dinner?");
        root.reply_count = Some(1);
        let mut first = message(11, ANNA, "at 7");
        first.thread_root_id = Some(10);
        store.apply_history(42, vec![root.clone(), first.clone()], false);
        let mut view = ThreadView {
            chat_id: 42,
            root_id: 10,
            generation: 1,
            thread: Thread::default(),
        };
        view.thread.apply(root);
        view.thread.apply(first);
        store.thread_view = Some(view);

        store.queue_send(
            42,
            "a".into(),
            Draft {
                body: "8?".into(),
                reply_to_message_id: Some(11),
                ..Draft::default()
            },
        );
        let view = store.thread_view.as_ref().expect("open");
        let row = view
            .thread
            .messages
            .iter()
            .find(|m| m.client_msg_id.as_deref() == Some("a"))
            .expect("the pending reply is on the surface");
        assert_eq!(row.thread_root_id, Some(10));
        assert_eq!(
            view.thread.get(10).and_then(|m| m.reply_count),
            Some(1),
            "a pending row counts nothing"
        );
    }

    /// A thread read lands only on the opening that asked for it. A read in
    /// flight when the reader opened another thread neither joins nor
    /// re-roots the one now showing — while the chat's own window still
    /// takes its copies of the rows it HOLDS, and nothing more.
    #[wasm_bindgen_test]
    fn a_thread_read_lands_only_on_its_own_opening() {
        let mut store = store();
        let mut root_a = message(10, ANNA, "A?");
        root_a.reply_count = Some(1);
        store.apply_history(42, vec![root_a.clone(), message(50, ANNA, "later")], false);
        store.thread_view = Some(ThreadView {
            chat_id: 42,
            root_id: 20,
            generation: 2,
            thread: Thread::default(),
        });

        let mut fresh_root = root_a;
        fresh_root.reply_count = Some(3);
        let mut reply = message(300, ANNA, "a reply");
        reply.thread_root_id = Some(10);
        assert!(
            !store.apply_thread_page(42, 1, true, vec![fresh_root, reply]),
            "A's read stops"
        );
        let view = store.thread_view.as_ref().expect("B is open");
        assert_eq!(view.root_id, 20, "not re-rooted");
        assert!(
            view.thread.messages.is_empty(),
            "nothing of A's on B's surface"
        );
        assert_eq!(
            store.threads[&42].get(10).and_then(|m| m.reply_count),
            Some(3),
            "the chat's own copy of A's root took the true count"
        );
        assert!(
            store.threads[&42].get(300).is_none(),
            "a row the window does not hold stays out of it"
        );
        assert_eq!(store.threads[&42].oldest, Some(10));

        assert!(store.apply_thread_page(42, 2, true, vec![message(20, ANNA, "B?")]));
        assert_eq!(
            store
                .thread_view
                .as_ref()
                .map(|view| view.thread.messages.len()),
            Some(1)
        );
    }

    /// Each copy of a root counts a reply the first time THAT copy sees it.
    /// The thread read already delivered — and its root already counts — a
    /// reply whose frame reaches the chat's window only now.
    #[wasm_bindgen_test]
    fn each_copy_of_a_root_counts_a_reply_once() {
        let mut store = store();
        let mut root = message(10, ANNA, "Dinner?");
        root.reply_count = Some(1);
        store.apply_history(42, vec![root.clone()], false);
        let mut reply = message(200, ANNA, "8?");
        reply.thread_root_id = Some(10);
        let mut surface_root = root;
        surface_root.reply_count = Some(2);
        let mut view = ThreadView {
            chat_id: 42,
            root_id: 10,
            generation: 1,
            thread: Thread::default(),
        };
        view.thread.apply(surface_root);
        view.thread.apply(reply.clone());
        store.thread_view = Some(view);

        store.apply_message(reply, None, Via::Frame);

        assert_eq!(
            store.threads[&42].get(10).and_then(|m| m.reply_count),
            Some(2),
            "the chat's copy counts it"
        );
        assert_eq!(
            store
                .thread_view
                .as_ref()
                .and_then(|view| view.thread.get(10))
                .and_then(|m| m.reply_count),
            Some(2),
            "the surface's copy had already"
        );
    }

    /// The list's preview is a copy too: an edit to a chat's last message
    /// reaches it whether or not the chat's messages are loaded — under the
    /// same guard, and without inserting anything.
    #[wasm_bindgen_test]
    fn an_edit_to_the_last_message_reaches_the_preview_even_when_not_held() {
        let mut store = store();
        store.chats[0].last_message = Some(message(100, ANNA, "Dinner at 7?"));
        let mut edited = message(100, ANNA, "Dinner at 8?");
        edited.edit_seq = Some(3);
        edited.edited_at = Some("2026-08-19T17:05:00Z".into());
        store.apply_edit(edited);
        assert_eq!(
            store.chats[0]
                .last_message
                .as_ref()
                .map(|m| m.body.as_str()),
            Some("Dinner at 8?")
        );

        let mut older = message(100, ANNA, "Dinner at 6?");
        older.edit_seq = Some(2);
        store.apply_edit(older);
        assert_eq!(
            store.chats[0]
                .last_message
                .as_ref()
                .map(|m| m.body.as_str()),
            Some("Dinner at 8?"),
            "an older edit does not walk it back"
        );
        assert!(!store.threads.contains_key(&42), "nothing was inserted");
    }

    /// Opening a chat picks up after what it holds only when a page of it
    /// was ever read. Frames alone hold the newest messages and nothing of
    /// the history above them: that chat opens on its newest page, or it
    /// would show a handful of rows, no way to fetch more, and a divider
    /// that could never find its first unread message.
    #[wasm_bindgen_test]
    fn only_a_paged_chat_resumes_after_what_it_holds() {
        let mut store = store();
        assert_eq!(store.resume_point(42), None, "nothing held");
        store.apply_message(message(140, ANNA, "live"), None, Via::Frame);
        assert_eq!(store.resume_point(42), None, "frames alone");
        store.apply_history(42, vec![message(100, ANNA, "old")], true);
        store.chats[0].last_message = Some(message(150, ANNA, "listed"));
        assert_eq!(store.resume_point(42), Some((140, 150)));
    }
}

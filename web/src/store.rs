//! What the app holds, and the rules for changing it.
//!
//! Kept apart from the views so the rules can be TESTED without a browser
//! rendering anything: every function here is pure, takes the state and
//! what arrived, and says what the state becomes. The parts that need a
//! browser (fetch, socket, storage) live in api/socket/session and are the
//! only places that do.

use fc_text::i18n::t;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::model::{
    AiFailure, Assistant, Chat, ChatListItem, Family, Invitation, JoinRequest, Me, Member, Mention,
    Message, Poll, PollOption, Reaction, ReplyParent, ReplyTo, Report, Roster, User,
};
use crate::staged::{OutgoingItem, Prepared, StagedBytes};

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
        /// The cursor the pass OPENED with: everything this client held is
        /// at or below it. A thread root ABOVE it arrives on this very
        /// pass, carrying a recomputed count that already holds every
        /// reply riding with it, so counting those replies again doubles
        /// it — "8 replies" under a message with four, on every first
        /// sync, where this is 0 (docs/protocol.md, "Threads" -> Live).
        held_through: i64,
    },
    /// The answer to this device's own send: raises a thread root's count.
    Answer,
    /// A bubble drawn before the server has it: moves nothing.
    Pending,
}

/// A thread's messages, SHARED: cloning them is a reference count, not a
/// copy of every message.
///
/// The whole app is drawn again after every change (live.rs), and every
/// draw used to take a deep copy of every chat's every message — the state
/// itself, then the open chat's list again for its pane — so a chat that had
/// gathered a few hundred messages made every typing frame, every catch-up
/// page and every tick of a presence change cost a few hundred message
/// copies, twice. Shared, a draw hands the pane the very list the store
/// holds; the pane can tell "nothing changed" by the pointer alone; and the
/// store copies a list only when it changes one that a pane is still
/// holding (`make_mut`, copy-on-write) — once per change, not once per draw.
///
/// Reads go through `Deref` to the `Vec`, so everything that only looks —
/// `len`, `iter`, indexing, `as_slice` — reads exactly as it did.
#[derive(Debug, Default, Clone)]
pub struct Messages(Rc<Vec<Message>>);

impl Messages {
    /// The list to change. A copy only when a draw still holds this one.
    pub fn make_mut(&mut self) -> &mut Vec<Message> {
        Rc::make_mut(&mut self.0)
    }
}

impl std::ops::Deref for Messages {
    type Target = Vec<Message>;

    fn deref(&self) -> &Vec<Message> {
        &self.0
    }
}

impl From<Vec<Message>> for Messages {
    fn from(messages: Vec<Message>) -> Self {
        Messages(Rc::new(messages))
    }
}

/// The same list is equal without looking at a single message — the answer
/// a pane asks for on every draw — and two different lists are compared the
/// way two `Vec`s are (a `Message` is not `Eq`, so `Rc`'s own `==` would
/// always look).
impl PartialEq for Messages {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

/// One chat's messages, oldest first — which is the order they are drawn in
/// and NOT the order `before_id` pages arrive in.
///
/// Held IN that order at all times — numbered rows by id, then the pending
/// rows (id 0) in the order they were written — so a message is found by a
/// binary search and goes in where it belongs, rather than every arrival
/// searching the whole list and sorting it again: a catch-up of five hundred
/// messages into a chat holding five hundred was a quarter of a million
/// comparisons and five hundred sorts.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Thread {
    pub messages: Messages,
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
    ///
    /// The order it leaves is the one a stable sort by id (pending rows
    /// last) would: `resort`, which this used to call after every arrival,
    /// is kept as the reference the tests hold it to.
    pub fn apply(&mut self, incoming: Message) -> Arrival {
        if incoming.id != 0 {
            if let Some(index) = self.index_of(incoming.id) {
                merge(&mut self.messages.make_mut()[index], incoming);
                return Arrival::Known;
            }
        }
        if let Some(client_msg_id) = incoming.client_msg_id.as_deref() {
            // Pending rows are the tail, after every numbered one.
            let pending_from = self.numbered();
            if let Some(offset) = self.messages[pending_from..]
                .iter()
                .position(|held| held.client_msg_id.as_deref() == Some(client_msg_id))
            {
                let arrival = if incoming.id == 0 {
                    Arrival::Known
                } else {
                    Arrival::New
                };
                let messages = self.messages.make_mut();
                if incoming.id == 0 {
                    // Still pending: it keeps its place among the pending.
                    messages[pending_from + offset] = incoming;
                } else {
                    // Numbered now: out of the pending tail, into the
                    // numbered rows where its id puts it.
                    messages.remove(pending_from + offset);
                    let at = insertion_point(messages, incoming.id);
                    messages.insert(at, incoming);
                }
                self.oldest = self.oldest_numbered();
                return arrival;
            }
        }
        let arrival = if incoming.id == 0 {
            Arrival::Known
        } else {
            Arrival::New
        };
        let messages = self.messages.make_mut();
        if incoming.id == 0 {
            // The newest pending row: last of all.
            messages.push(incoming);
        } else {
            let at = insertion_point(messages, incoming.id);
            messages.insert(at, incoming);
        }
        self.oldest = self.oldest_numbered();
        arrival
    }

    /// Oldest first, with pending rows (id 0) after everything the server
    /// has numbered — they were written last, whatever id they end up with.
    /// The order `apply` keeps; only the tests sort any more, to check it.
    #[cfg(test)]
    fn resort(&mut self) {
        self.messages.make_mut().sort_by_key(|message| {
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

    /// How many rows the server has numbered — where the pending tail
    /// starts.
    fn numbered(&self) -> usize {
        self.messages.partition_point(|message| message.id != 0)
    }

    fn oldest_numbered(&self) -> Option<i64> {
        self.messages
            .first()
            .map(|message| message.id)
            .filter(|id| *id != 0)
    }

    /// Where the numbered row `id` is held, by binary search.
    fn index_of(&self, id: i64) -> Option<usize> {
        if id == 0 {
            return None;
        }
        let at = insertion_point(&self.messages, id);
        (self.messages.get(at).map(|message| message.id) == Some(id)).then_some(at)
    }

    /// The newest id the server has numbered — the read marker to report,
    /// and the cursor a catch-up asks after.
    pub fn newest_server_id(&self) -> Option<i64> {
        self.messages
            .iter()
            .rev()
            .map(|message| message.id)
            .find(|id| *id != 0)
    }

    pub fn get(&self, id: i64) -> Option<&Message> {
        self.index_of(id).map(|index| &self.messages[index])
    }

    /// The numbered row `id`, to change — the list copied first if a draw
    /// still holds it (`Messages::make_mut`), and only when it is here.
    pub fn get_mut(&mut self, id: i64) -> Option<&mut Message> {
        let index = self.index_of(id)?;
        Some(&mut self.messages.make_mut()[index])
    }
}

/// Where a numbered row with `id` goes among `messages` — held oldest first
/// with the pending rows last: after every numbered row below it, and before
/// everything else.
pub fn insertion_point(messages: &[Message], id: i64) -> usize {
    messages.partition_point(|held| held.id != 0 && held.id < id)
}

/// A message this device wrote that the server has not confirmed yet.
///
/// Its bubble is in the thread under id 0; this is the other half — what
/// the sender needs to try again, and what the bubble needs to say about
/// it (docs/protocol.md, "Sending on an unreliable network").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
    /// Its attachments, in the order they were chosen — the order the
    /// server keeps (docs/protocol.md, "Photos, videos, audio, files and
    /// locations"). Uploaded one at a time before the message is sent.
    #[serde(default)]
    pub items: Vec<OutgoingItem>,
    /// This message is a STICKER: its one attachment is sent with
    /// `sticker: true`, and its bytes go up exactly as they are
    /// (docs/protocol.md, "Sending one"). On the row, so a retry — and a
    /// reload — sends the same flag.
    #[serde(default)]
    pub sticker: bool,
    /// This message is a VIDEO MESSAGE: its one video is sent with
    /// `round: true` and drawn as a circle (docs/protocol.md, "Video
    /// messages"). On the row, like `sticker`, so a retry sends the same
    /// flag; a row kept by a build from before it reads as an ordinary one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub round: bool,
    /// This message is an ANIMATION: its one attachment — an animated
    /// picture made into a silent looping video — is sent with
    /// `animation: true` (docs/protocol.md, "Animations"). On the row, like
    /// `round`, so a retry sends the same flag; a row kept by a build from
    /// before it reads as an ordinary video.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub animation: bool,
    /// Tries whose outcome was UNKNOWN. A refusal is not counted here: it
    /// ends the row outright.
    pub attempts: u32,
    /// Why it will not be tried again until somebody asks. None while it
    /// is still queued — which is what an unknown outcome leaves it.
    pub failed: Option<String>,
}

/// What a person asked to send. Everything but the chat and the body is
/// optional — and with attachments, the body may be empty: a photo needs no
/// caption.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub body: String,
    pub reply_to_message_id: Option<i64>,
    pub mentions: Vec<Mention>,
    pub poll: Option<Vec<String>>,
    pub attachments: Vec<Prepared>,
    /// Sent as a sticker: one picture, no words, no bubble.
    pub sticker: bool,
    /// Sent as a video message: one square video, no words, drawn round.
    pub round: bool,
}

/// Whether `draft` goes as an ANIMATION (docs/protocol.md, "Animations"):
/// its one attachment an animated picture this client made into a video,
/// and nothing that is a sticker, a video message or a poll. An animation
/// staged beside other attachments goes as the ordinary video it is — the
/// server refuses `animation` on anything but exactly one attachment, and
/// every client draws a message of several as what it otherwise is. A
/// caption is allowed.
pub fn sends_as_animation(draft: &Draft) -> bool {
    !draft.sticker
        && !draft.round
        && draft.poll.is_none()
        && matches!(draft.attachments.as_slice(), [only] if only.animation && only.kind == "video")
}

/// Why a queued message whose bytes a reload took fails at once — it has
/// nothing left to upload, and waiting would only look like sending.
/// A function, not a const: a translated string is not a constant.
pub fn lost_in_reload() -> &'static str {
    t("Not sent: its attachments were lost when the page reloaded. Attach them again.")
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

/// A voice message that was not sent (the plan for #79,
/// docs/audio-video-messages-2026-10-04.md, S2.8): a recording stopped by
/// something other than the person — a call, a hidden tab, the chat left,
/// the microphone going away — or still in review when its chat was left.
///
/// It waits in a row of its own above the box, with the reply it was
/// recorded under and the caption it was given, and leaves only by its own
/// Send — never carried by another, so a recording nobody finished deciding
/// about cannot ride out with the next message, and its caption never leaves
/// without it. Kept for the life of the tab and written nowhere: this client
/// keeps nothing a person said or wrote past the tab (docs/protocol.md, "A
/// browser is a client too"), which asks before closing while one waits.
#[derive(Debug, Clone, PartialEq)]
pub struct NotSent {
    /// This tab's own number for it — what its row is keyed by and its
    /// buttons name. Never sent anywhere.
    pub id: u64,
    /// The recording, as it would have been staged.
    pub note: Prepared,
    /// How long it is, as the recorder measured it — said on its row
    /// whatever the note turned out to be.
    pub duration_ms: i64,
    /// The message it was recorded in answer to.
    pub reply_to_message_id: Option<i64>,
    /// The words that go with it; empty for none.
    pub caption: String,
}

/// What this client holds of ONE family: its roster and assistant, its
/// board and sticker pack, and its owner's lists (docs/protocol.md,
/// "Several families"). The family on screen keeps its own in the
/// [`Store`]'s fields of the same names, where every screen reads them; the
/// others wait in [`Store::away`], and showing another family swaps.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Held {
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    pub board: crate::board::Board,
    pub pack: crate::pack::Pack,
    pub join_requests: Vec<JoinRequest>,
    pub reports: Vec<Report>,
    pub sent_invitations: Vec<Invitation>,
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
    /// The family on screen, with the owner's switches for the assistant —
    /// and with this account's own `role` in it. Its entry in
    /// [`Store::families`] is kept equal to it.
    pub family: Option<crate::model::Family>,
    /// Every family this account is in, the home family first, each with
    /// this account's role in it (`GET /me`'s `families`; docs/protocol.md,
    /// "Several families").
    pub families: Vec<crate::model::Family>,
    /// Everything held of the families NOT on screen, by family id: their
    /// rosters, boards, packs and owner's lists, kept current by their
    /// frames and resyncs so that showing one is instant.
    pub away: HashMap<i64, Held>,
    /// The invitations this account has been sent and not answered, oldest
    /// first.
    pub invitations: Vec<Invitation>,
    /// The family on screen's pending invitations, for its owner.
    pub sent_invitations: Vec<Invitation>,
    /// The family this device showed last, as it keeps it for the account —
    /// shown first after a reload while the account is still in it.
    pub preferred_family: Option<i64>,
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
    /// Assistant answers that stopped early (`ai_error`), each with what it
    /// says about it — remembered for exactly as long as the failure is.
    pub ai_failed: HashMap<i64, AiFailure>,
    /// The text of recordings this member asked for, by attachment id
    /// (docs/protocol.md, "Transcripts on request") — kept for the life of
    /// the tab, so reopening a chat shows them without asking again, and
    /// NEVER written to storage: they are words somebody said, and this
    /// client keeps nobody's words past the tab ("A browser is a client
    /// too"). A sign-out takes them with the rest of the store.
    pub transcripts: fc_text::transcript::Transcripts,
    /// What was being typed in a chat the reader left.
    pub drafts: HashMap<i64, String>,
    pub thread_view: Option<ThreadView>,
    /// How many threads have been opened this session — the next
    /// `ThreadView::generation`. Counted here rather than on the view, which
    /// a close throws away: a count that started again at every open would
    /// hand a stale read the generation of the thread opened after it.
    pub thread_openings: u64,
    pub open_polls: Option<OpenPolls>,
    /// provisional id → the bytes a queued attachment still has to upload.
    /// Memory only; see staged.rs.
    pub bytes: HashMap<i64, StagedBytes>,
    /// The last provisional id handed out. They count DOWN from -1, so none
    /// can ever be taken for one of the server's.
    pub last_provisional: i64,
    /// chat → what the composer has staged there and not sent yet.
    pub staged: HashMap<i64, Vec<Prepared>>,
    /// chat → its voice messages that were not sent, oldest first. Memory
    /// only, like the staged ones; see [`NotSent`].
    pub not_sent: HashMap<i64, Vec<NotSent>>,
    /// The last [`NotSent::id`] handed out.
    pub last_not_sent: u64,
    /// The server's id → the provisional one its bubble drew under, so a
    /// bubble that has just been acked draws from the same bytes rather
    /// than fetching back what this device uploaded.
    pub aliases: HashMap<i64, i64>,
    /// chat → the last message the most recent `GET /chats` listed, which
    /// its unread count already includes. A frame for a message at or below
    /// it is one the list has counted: the socket and the list race, and a
    /// frame landing after the answer that included its message must not
    /// count it a second time.
    pub listed: HashMap<i64, i64>,
    /// The family board (board.rs).
    pub board: crate::board::Board,
    /// The family's sticker pack (pack.rs).
    pub pack: crate::pack::Pack,
    /// How long and how big a video message may be on this server — None on
    /// a server from before video messages, which is offered none
    /// (docs/protocol.md, "Video messages"; round_video.rs).
    pub round: Option<crate::round_video::RoundLimits>,
    /// The whole of the last `GET /me` — the role, a join request still
    /// waiting, the server's switches — and None until it has answered.
    pub account: Option<Me>,
    /// The owner's pending join requests, as last read.
    pub join_requests: Vec<crate::model::JoinRequest>,
    /// The owner's open reports, oldest first, as last read.
    pub reports: Vec<crate::model::Report>,
    /// This account asked to join a family and is waiting on its owner —
    /// what tells a request that vanished (declined) from one never made.
    pub awaiting_join: bool,
    /// The request this account was waiting on was declined: said once, on
    /// the family gate, until the next join or create.
    pub join_declined: bool,
    /// Bumped by every frame that changes who is in the family or who owns
    /// it — `member_joined`, `member_left`, `member_deleted`,
    /// `family_owner`. A leave dialog names who inherits, a prediction any
    /// of them can change, so it is built again from a fresh read when one
    /// lands (docs/protocol.md, `next_owner_user_id`).
    pub roster_changes: u64,
    /// Bumped by every answer to the owner's own change of the family. A
    /// roster read that was asked for BEFORE one does not overwrite it.
    pub family_writes: u64,
    /// Bumped by every `/me` taken in. A poll's answer that lands after
    /// another `/me` — the resync an approval set off — is older than what
    /// is held, and is not taken.
    pub account_reads: u64,
    /// Bumped by every change this client makes to the chat list itself — a
    /// group created, changed or removed, by a frame or by this device's own
    /// answer. A `GET /chats` takes it as it starts
    /// ([`Store::merge_chats_from`]).
    pub chat_changes: u64,
    /// chat → the [`Store::chat_changes`] its last such change was. A list
    /// that started before it is older than this chat's row, and neither
    /// drops it nor brings it back.
    pub chat_touched: HashMap<i64, u64>,
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

    /// The list as it is drawn: the chats of the family on screen — the
    /// family chat first, which is THAT family's, then everything else
    /// newest conversation first (ios MacChatView; docs/protocol.md,
    /// "Several families": a client shows in the current family exactly the
    /// chats that name it).
    pub fn sorted_chats(&self) -> Vec<ChatListItem> {
        let mut chats: Vec<ChatListItem> = self
            .chats
            .iter()
            .filter(|item| self.listed_here(&item.chat))
            .cloned()
            .collect();
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

    /// When this member agreed that their words may go to the model, or
    /// none if they have not (docs/protocol.md, "Consenting to the
    /// assistant"). Held on the account the last `/me` brought, which is
    /// where the server's own answer lives.
    pub fn assistant_consent_at(&self) -> Option<&str> {
        self.account
            .as_ref()
            .and_then(|me| me.assistant_consent_at.as_deref())
    }

    /// Record the answer the server just gave, so the composer stops
    /// asking — or starts again — without waiting for the next `/me`.
    ///
    /// Withdrawing it withdraws the lookup consent with it, as the server
    /// does in the same write: that one may only stand on this one
    /// (docs/protocol.md, `POST /me/assistant-consent`).
    pub fn set_assistant_consent(&mut self, at: Option<String>) {
        if let Some(account) = self.account.as_mut() {
            if at.is_none() {
                account.assistant_lookup_consent_at = None;
            }
            account.assistant_consent_at = at;
        }
    }

    /// When this member agreed that the assistant may send queries it
    /// writes from their words to the lookup providers, or none
    /// (docs/protocol.md, "Looking things up"). The views read it off the
    /// account they are handed; this is the tests' window on it.
    #[cfg(test)]
    pub fn assistant_lookup_consent_at(&self) -> Option<&str> {
        self.account
            .as_ref()
            .and_then(|me| me.assistant_lookup_consent_at.as_deref())
    }

    /// Record the lookup answer the server just gave.
    pub fn set_assistant_lookup_consent(&mut self, at: Option<String>) {
        if let Some(account) = self.account.as_mut() {
            account.assistant_lookup_consent_at = at;
        }
    }

    /// `GET /me`: who this is, the families it is in, and the block list —
    /// REPLACED, never merged. A family no longer listed is put away
    /// ([`Store::forget_family`]); the family on screen stays while it is
    /// still listed, else the one this device showed last, else the first
    /// (fc_text::families::current_after). Answers whether the family on
    /// screen changed, which the app answers by putting away whatever pane
    /// was open on the old one.
    pub fn apply_me(&mut self, me: &Me) -> bool {
        // Only a SECOND answer can say the family changed. The first one of
        // a session meets a store that holds no family yet — but may hold
        // the outbox a reload put back, bubbles and all, which a "change"
        // would wipe (and a row that then failed would fail on no bubble).
        let second = self.account.is_some() && self.my_user_id == me.user.id;
        let before = self.current_family_id();
        let listed = families_of_me(me);
        let ids: Vec<i64> = listed.iter().map(|family| family.id).collect();
        if second {
            let gone: Vec<i64> = self
                .families
                .iter()
                .map(|family| family.id)
                .chain(before)
                .filter(|id| !ids.contains(id))
                .collect();
            for family_id in gone {
                self.forget_family(family_id);
            }
        }
        self.families = listed;
        self.my_user_id = me.user.id;
        self.names.insert(me.user.id, me.user.display_name.clone());
        self.blocked = me.blocked_user_ids.iter().copied().collect();
        self.support_contact = me.support_contact.clone();
        self.invitations = me.invitations.clone();
        self.account = Some(me.clone());
        self.account_reads += 1;
        let chosen =
            fc_text::families::current_after(&ids, self.current_family_id(), self.preferred_family);
        self.swap_to(chosen);
        // `/me`'s copy is as fresh as any: the shown family's settings and
        // this account's role in it come from it.
        self.family = chosen.and_then(|id| self.families.iter().find(|f| f.id == id).cloned());
        self.sync_view();
        // A join request is only ever answered by vanishing: the account
        // that was waiting and now has neither a family nor a request was
        // declined (ios AppSession.apply(me:)).
        match (&me.family, &me.pending_join_request) {
            (Some(_), _) => {
                self.awaiting_join = false;
                self.join_declined = false;
            }
            (None, Some(_)) => {
                self.awaiting_join = true;
                self.join_declined = false;
            }
            (None, None) => {
                if self.awaiting_join {
                    self.join_declined = true;
                }
                self.awaiting_join = false;
            }
        }
        second && before != chosen
    }

    /// Everything that belonged to ONE family this account WAS in, gone: it
    /// left, it was removed, or the family was deleted. Nothing of that
    /// family's feeds will ever reach this client again to tidy it — no
    /// tombstone for its notes, no read for its chats — so it goes now: its
    /// roster, board, pack and owner's lists, and every chat listed in it
    /// alone — its family chat, its groups, its assistant's chat. A direct
    /// chat stays while it still names another family this account holds,
    /// listed there and no longer here (docs/protocol.md, "Several
    /// families"). What is the PERSON's stays: the block list, and an outbox
    /// whose rows for the chats that are left go on.
    pub fn forget_family(&mut self, family_id: i64) {
        let home = self.home_family_id();
        let ids: Vec<i64> = self.chats.iter().map(|item| item.chat.id).collect();
        for chat_id in ids {
            let Some(item) = self.item(chat_id) else {
                continue;
            };
            let named = fc_text::families::families_of(item.chat.family_ids.as_deref(), home);
            if !named.contains(&family_id) {
                continue;
            }
            let rest: Vec<i64> = named.into_iter().filter(|id| *id != family_id).collect();
            if rest.is_empty() {
                self.remove_chat(chat_id);
            } else if let Some(item) = self.item_mut(chat_id) {
                item.chat.family_ids = Some(rest);
            }
        }
        if self.current_family_id() == Some(family_id) {
            // The roster, the board, the pack and the owner's lists were
            // that family's, whole.
            drop(self.take_held());
            self.family = None;
            self.thread_view = None;
            self.open_polls = None;
            self.sync_view();
        }
        self.away.remove(&family_id);
        self.families.retain(|family| family.id != family_id);
    }

    /// `member_left` naming THIS account: out of that family, and only that
    /// one. The family on screen falls back to the first left — or to none,
    /// and the gate. Answers whether the family on screen changed.
    pub fn leave_family(&mut self, family_id: i64) -> bool {
        let before = self.current_family_id();
        self.forget_family(family_id);
        let ids: Vec<i64> = self.families.iter().map(|family| family.id).collect();
        let chosen = fc_text::families::current_after(&ids, self.current_family_id(), None);
        self.swap_to(chosen);
        before != chosen
    }

    /// The family on screen, by id.
    pub fn current_family_id(&self) -> Option<i64> {
        self.family.as_ref().map(|family| family.id)
    }

    /// The home family — the first `/me` lists, which is what a chat that
    /// names no family means (docs/protocol.md, `Chat`).
    pub fn home_family_id(&self) -> Option<i64> {
        self.families
            .first()
            .map(|family| family.id)
            .or_else(|| self.current_family_id())
    }

    /// Every family this account is in, as the switcher lists them — the
    /// one on screen as it now stands.
    pub fn family_list(&self) -> Vec<Family> {
        self.families
            .iter()
            .map(|family| match &self.family {
                Some(shown) if shown.id == family.id => shown.clone(),
                _ => family.clone(),
            })
            .collect()
    }

    /// A family's name, for a notification about it.
    pub fn family_name(&self, family_id: i64) -> Option<String> {
        self.family_list()
            .into_iter()
            .find(|family| family.id == family_id)
            .map(|family| family.name)
    }

    /// Whether this account owns any of its families — and so may not start
    /// another (docs/protocol.md, "Getting another family").
    pub fn owns_a_family(&self) -> bool {
        self.family_list().iter().any(Family::is_owner)
    }

    /// The families a chat is listed in.
    pub fn chat_families(&self, chat: &Chat) -> Vec<i64> {
        fc_text::families::families_of(chat.family_ids.as_deref(), self.home_family_id())
    }

    /// Whether a chat is shown in the family on screen. With no family on
    /// screen every chat is: there is nothing to tell them apart by.
    pub fn listed_here(&self, chat: &Chat) -> bool {
        match self.current_family_id() {
            Some(family_id) => fc_text::families::listed_in(
                chat.family_ids.as_deref(),
                family_id,
                self.home_family_id(),
            ),
            None => true,
        }
    }

    /// The family to show a chat in: the one on screen when the chat is
    /// listed there, else the first that lists it — which is also the
    /// family an upload for it is made in (docs/protocol.md, "Uploads name
    /// their family").
    pub fn family_of_chat(&self, chat_id: i64) -> Option<i64> {
        let Some(item) = self.item(chat_id) else {
            return self.current_family_id();
        };
        if self.listed_here(&item.chat) {
            return self.current_family_id();
        }
        self.chat_families(&item.chat).first().copied()
    }

    /// What is unread in one family: the unread of the chats listed in it —
    /// a direct chat counts in each family that lists it.
    pub fn unread_in(&self, family_id: i64) -> i64 {
        let home = self.home_family_id();
        self.chats
            .iter()
            .filter(|item| {
                fc_text::families::listed_in(item.chat.family_ids.as_deref(), family_id, home)
            })
            .map(|item| item.unread_count)
            .sum()
    }

    /// Show another family: what the one on screen holds goes away, and
    /// what was held of that one comes out. Answers whether it changed —
    /// not for a family this account is not in, nor for the one on screen.
    pub fn switch_family(&mut self, family_id: i64) -> bool {
        if self.current_family_id() == Some(family_id)
            || !self.families.iter().any(|family| family.id == family_id)
        {
            return false;
        }
        self.swap_to(Some(family_id));
        self.thread_view = None;
        self.open_polls = None;
        true
    }

    /// Run `apply` on one family's state, wherever it is held — on screen or
    /// away — leaving the family on screen as it was. None for a family
    /// this account is not in.
    pub fn with_family<R>(
        &mut self,
        family_id: i64,
        apply: impl FnOnce(&mut Store) -> R,
    ) -> Option<R> {
        let shown = self.current_family_id();
        if shown == Some(family_id) {
            return Some(apply(self));
        }
        if !self.families.iter().any(|family| family.id == family_id) {
            return None;
        }
        let (thread_view, open_polls) = (self.thread_view.take(), self.open_polls.take());
        self.swap_to(Some(family_id));
        let answer = apply(self);
        self.swap_to(shown);
        self.thread_view = thread_view;
        self.open_polls = open_polls;
        Some(answer)
    }

    /// One family's board, on screen or away.
    pub fn board_of(&self, family_id: i64) -> Option<&crate::board::Board> {
        if self.current_family_id() == Some(family_id) {
            return Some(&self.board);
        }
        self.away.get(&family_id).map(|held| &held.board)
    }

    pub fn board_mut(&mut self, family_id: i64) -> Option<&mut crate::board::Board> {
        if self.current_family_id() == Some(family_id) {
            return Some(&mut self.board);
        }
        self.away.get_mut(&family_id).map(|held| &mut held.board)
    }

    /// One family's sticker pack, on screen or away.
    pub fn pack_of(&self, family_id: i64) -> Option<&crate::pack::Pack> {
        if self.current_family_id() == Some(family_id) {
            return Some(&self.pack);
        }
        self.away.get(&family_id).map(|held| &held.pack)
    }

    pub fn pack_mut(&mut self, family_id: i64) -> Option<&mut crate::pack::Pack> {
        if self.current_family_id() == Some(family_id) {
            return Some(&mut self.pack);
        }
        self.away.get_mut(&family_id).map(|held| &mut held.pack)
    }

    /// The board an answer was asked of — `family`'s, or the one on screen
    /// when none was named.
    pub fn board_for(&mut self, family: Option<i64>) -> Option<&mut crate::board::Board> {
        match family {
            Some(family_id) => self.board_mut(family_id),
            None => Some(&mut self.board),
        }
    }

    /// The pack an answer was asked of, the same way.
    pub fn pack_for(&mut self, family: Option<i64>) -> Option<&mut crate::pack::Pack> {
        match family {
            Some(family_id) => self.pack_mut(family_id),
            None => Some(&mut self.pack),
        }
    }

    /// A note this device's own change was answered with, onto the board it
    /// is on: the one it names, else the one asked of.
    pub fn apply_note(&mut self, family: Option<i64>, note: crate::model::Note) {
        if let Some(board) = self.board_for(note.family_id.or(family)) {
            board.apply(note);
        }
    }

    /// Every board held, on screen and away.
    pub fn boards_mut(&mut self) -> impl Iterator<Item = &mut crate::board::Board> {
        std::iter::once(&mut self.board).chain(self.away.values_mut().map(|held| &mut held.board))
    }

    /// Every pack held, on screen and away.
    pub fn packs_mut(&mut self) -> impl Iterator<Item = &mut crate::pack::Pack> {
        std::iter::once(&mut self.pack).chain(self.away.values_mut().map(|held| &mut held.pack))
    }

    /// One family's roster, on screen or away: (user id, name) for each
    /// live member — what the owner's "Invite…" picks from.
    pub fn roster_of(&self, family_id: i64) -> Vec<(i64, String)> {
        let members = if self.current_family_id() == Some(family_id) {
            &self.members
        } else {
            match self.away.get(&family_id) {
                Some(held) => &held.members,
                None => return Vec::new(),
            }
        };
        members
            .iter()
            .filter(|member| !member.deleted)
            .map(|member| (member.id, member.display_name.clone()))
            .collect()
    }

    /// Whom the owner of the family on screen may invite: the members of
    /// their OTHER families who are not in this one.
    pub fn invitable(&self) -> Vec<(i64, String)> {
        let Some(shown) = self.current_family_id() else {
            return Vec::new();
        };
        let here: Vec<i64> = self.members.iter().map(|member| member.id).collect();
        let others: Vec<Vec<(i64, String)>> = self
            .families
            .iter()
            .filter(|family| family.id != shown)
            .map(|family| self.roster_of(family.id))
            .collect();
        fc_text::families::invitable(&here, &others, self.my_user_id)
    }

    /// An `invitation` frame: to ME, onto my list; otherwise to the owner
    /// of the family it is into, onto that family's list. Replaced when
    /// held, never doubled.
    pub fn apply_invitation(&mut self, invitation: Invitation) {
        let upsert = |list: &mut Vec<Invitation>, invitation: Invitation| match list
            .iter_mut()
            .find(|held| held.id == invitation.id)
        {
            Some(held) => *held = invitation,
            None => list.push(invitation),
        };
        if invitation.user.id == self.my_user_id {
            // And on the account, which the family gate draws from.
            if let Some(account) = self.account.as_mut() {
                upsert(&mut account.invitations, invitation.clone());
            }
            upsert(&mut self.invitations, invitation);
            return;
        }
        let family_id = invitation.family_id;
        self.with_family(family_id, |store| {
            upsert(&mut store.sent_invitations, invitation)
        });
    }

    /// `invitation_removed`, or this device's own answer: off every list.
    pub fn remove_invitation(&mut self, invitation_id: i64) {
        self.invitations.retain(|held| held.id != invitation_id);
        if let Some(account) = self.account.as_mut() {
            account.invitations.retain(|held| held.id != invitation_id);
        }
        self.sent_invitations
            .retain(|held| held.id != invitation_id);
        for held in self.away.values_mut() {
            held.sent_invitations
                .retain(|held| held.id != invitation_id);
        }
    }

    /// What the family on screen holds, taken out.
    fn take_held(&mut self) -> Held {
        Held {
            members: std::mem::take(&mut self.members),
            assistant: self.assistant.take(),
            board: std::mem::take(&mut self.board),
            pack: std::mem::take(&mut self.pack),
            join_requests: std::mem::take(&mut self.join_requests),
            reports: std::mem::take(&mut self.reports),
            sent_invitations: std::mem::take(&mut self.sent_invitations),
        }
    }

    fn put_held(&mut self, held: Held) {
        self.members = held.members;
        self.assistant = held.assistant;
        self.board = held.board;
        self.pack = held.pack;
        self.join_requests = held.join_requests;
        self.reports = held.reports;
        self.sent_invitations = held.sent_invitations;
    }

    /// The family on screen becomes `family_id`: what the old one holds is
    /// kept away — while the account is still in it — and the new one's is
    /// brought out, or begun.
    fn swap_to(&mut self, family_id: Option<i64>) {
        let shown = self.current_family_id();
        if shown == family_id {
            return;
        }
        self.write_back();
        let held = self.take_held();
        if let Some(shown) = shown.filter(|id| self.families.iter().any(|f| f.id == *id)) {
            self.away.insert(shown, held);
        }
        let next = family_id
            .and_then(|id| self.away.remove(&id))
            .unwrap_or_default();
        self.put_held(next);
        self.family = family_id.and_then(|id| self.families.iter().find(|f| f.id == id).cloned());
        self.sync_view();
    }

    /// The family on screen, written back into the list of families.
    fn write_back(&mut self) {
        if let Some(shown) = &self.family {
            if let Some(entry) = self.families.iter_mut().find(|f| f.id == shown.id) {
                *entry = shown.clone();
            }
        }
    }

    /// The account as the screens read it: the family on screen and this
    /// account's role in it — `/me`'s own `family` and `role` are the home
    /// family's, which is what an app from before several families sees.
    fn sync_view(&mut self) {
        self.write_back();
        let family = self.family.clone();
        if let Some(account) = self.account.as_mut() {
            account.role = family.as_ref().and_then(|family| family.role.clone());
            account.family = family;
        }
    }

    /// This account's role in the family on screen, as a roster or a
    /// `family_owner` frame said it.
    fn set_my_role(&mut self, role: &str) {
        match self.family.as_mut() {
            Some(family) => {
                family.role = Some(role.to_string());
                self.sync_view();
            }
            None => {
                if let Some(account) = self.account.as_mut() {
                    account.role = Some(role.to_string());
                }
            }
        }
    }

    /// Whether this account is the family's owner, as `/me` last said — kept
    /// true to `family_owner` frames in between.
    pub fn is_owner(&self) -> bool {
        self.account.as_ref().is_some_and(Me::is_owner)
    }

    /// `GET /families/mine`: the roster, the names, the assistant, and the
    /// block list again (it rides on both reads). Its `family` is the
    /// fresher copy — the owner's invite code rides on it — and replaces the
    /// one `/me` gave.
    pub fn apply_roster(&mut self, roster: &Roster) {
        self.names.extend(roster.names());
        // A read that finds the family changed — somebody in or out, an
        // owner moved, which a socket that was down never heard as frames —
        // counts as the frames would have. One that finds it as it was does
        // not, so a leave dialog's own read does not set off another.
        let shape = |members: &[Member]| -> Vec<(i64, Option<String>)> {
            let mut shape: Vec<_> = members
                .iter()
                .map(|member| (member.id, member.role.clone()))
                .collect();
            shape.sort();
            shape
        };
        if shape(&self.members) != shape(&roster.members) {
            self.roster_changes += 1;
        }
        self.members = roster.members.clone();
        self.assistant = roster.assistant.clone();
        // Both limits or neither: a server that has packs always sends the
        // two, and one that sends neither predates them.
        self.pack.limits = roster
            .max_pack_items
            .zip(roster.max_pack_item_bytes)
            .map(|(items, bytes)| crate::pack::Limits { items, bytes });
        // The same for video messages: both keys, or a server without them.
        self.round = crate::round_video::RoundLimits::of(
            roster.max_round_video_ms,
            roster.max_round_video_bytes,
        );
        self.blocked = roster.blocked_user_ids.iter().copied().collect();
        // The roster names every member's role, this account's too — and it
        // is fresher than the `/me` it may follow, and than a `family_owner`
        // frame this tab missed while its socket was down.
        if let Some(family) = roster.family.as_ref().filter(|family| {
            self.family
                .as_ref()
                .is_some_and(|held| held.id == family.id)
        }) {
            // A family read carries no role: this account's is kept, and
            // set again just below from the roster itself.
            let role = self.family.as_ref().and_then(|held| held.role.clone());
            self.family = Some(Family {
                role: family.role.clone().or(role),
                ..family.clone()
            });
            self.sync_view();
        }
        if let Some(role) = roster
            .members
            .iter()
            .find(|member| member.id == self.my_user_id)
            .and_then(|member| member.role.clone())
        {
            self.set_my_role(&role);
        }
    }

    /// The family's settings, as the owner's own change answered them.
    /// A PATCH answers without the invite code unless the caller owns the
    /// family, so the code already held is kept rather than blanked.
    pub fn apply_family(&mut self, family: crate::model::Family) {
        self.family_writes += 1;
        let held = self.family.as_ref().filter(|held| held.id == family.id);
        let code = held.and_then(|held| held.invite_code.clone());
        let role = held.and_then(|held| held.role.clone());
        let family = crate::model::Family {
            invite_code: family.invite_code.or(code),
            role: family.role.or(role),
            ..family
        };
        self.family = Some(family);
        self.sync_view();
    }

    /// A `family_plan` frame: the family it names will be deleted at
    /// `delete_at` — or, with none, no longer will — wherever that family is
    /// held, on screen or away (docs/protocol.md, "Paid families"). The
    /// banner over its chat list follows.
    pub fn apply_plan(&mut self, family_id: i64, delete_at: Option<String>) {
        if let Some(family) = self.family.as_mut().filter(|held| held.id == family_id) {
            family.delete_at = delete_at.clone();
        }
        for family in self.families.iter_mut().filter(|held| held.id == family_id) {
            family.delete_at = delete_at.clone();
        }
        self.sync_view();
    }

    /// A member's birthday, as the owner's own change left it — or mine.
    pub fn set_birthday(&mut self, user_id: i64, birthday: Option<crate::model::Birthday>) {
        if let Some(member) = self.members.iter_mut().find(|member| member.id == user_id) {
            member.birthday = birthday;
        }
        if let Some(account) = self
            .account
            .as_mut()
            .filter(|account| account.user.id == user_id)
        {
            account.user.birthday = birthday;
        }
    }

    /// The owner's invite code, rotated: the one held, and the account's.
    pub fn set_invite_code(&mut self, code: String) {
        if let Some(family) = self.family.as_mut() {
            family.invite_code = Some(code.clone());
        }
        self.sync_view();
        if let Some(family) = self.account.as_mut().and_then(|a| a.family.as_mut()) {
            family.invite_code = Some(code);
        }
    }

    /// A profile picture's version — 0 when it was taken away.
    pub fn set_avatar_version(&mut self, user_id: i64, version: i64) {
        if let Some(member) = self.members.iter_mut().find(|member| member.id == user_id) {
            member.avatar_version = version;
        }
        if let Some(account) = self
            .account
            .as_mut()
            .filter(|account| account.user.id == user_id)
        {
            account.user.avatar_version = version;
        }
    }

    /// My own profile as an endpoint of mine answered it — a new picture, a
    /// birthday — into both places it is drawn from.
    pub fn apply_my_user(&mut self, user: crate::model::User) {
        self.names.insert(user.id, user.display_name.clone());
        if let Some(member) = self.members.iter_mut().find(|member| member.id == user.id) {
            member.avatar_version = user.avatar_version;
            member.birthday = user.birthday;
        }
        if let Some(account) = self.account.as_mut() {
            account.user = user;
        }
    }

    /// A list asked for just now — the tests' shorthand for
    /// [`Store::merge_chats_from`].
    #[cfg(test)]
    pub fn merge_chats(&mut self, fresh: Vec<ChatListItem>) -> Vec<i64> {
        self.merge_chats_from(fresh, self.chat_changes)
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
    ///
    /// A chat that is no longer listed is gone from the list, whatever its
    /// kind (resync step 2, amended 2026-10-22) — and a GROUP that is no
    /// longer listed is gone for good, left, removed from or deleted while
    /// this client slept, so everything held for it goes too
    /// ([`Store::remove_chat`]). A direct chat is only hidden by a block and
    /// comes back whole on unblock, so what is held of it stays. Answers the
    /// groups dropped, for the caller to close one that is open.
    ///
    /// `began` is what [`Store::chat_changes`] was when the list was asked
    /// for. A chat this client added, changed or removed SINCE — a
    /// group made, a `chat_updated`, a `chat_removed` that landed while the
    /// list was in flight — is newer than the list's snapshot of it: a group
    /// added since is kept although the list does not have it, one changed
    /// since keeps the chat it holds, and one removed since is not brought
    /// back (the list races the frames as it races the messages).
    pub fn merge_chats_from(&mut self, mut fresh: Vec<ChatListItem>, began: u64) -> Vec<i64> {
        let me = self.my_user_id;
        let newer: HashSet<i64> = self
            .chat_touched
            .iter()
            .filter(|(_, at)| **at > began)
            .map(|(chat_id, _)| *chat_id)
            .collect();
        fresh.retain(|item| !newer.contains(&item.chat.id) || self.item(item.chat.id).is_some());
        let kept: Vec<ChatListItem> = self
            .chats
            .iter()
            .filter(|held| newer.contains(&held.chat.id))
            .filter(|held| !fresh.iter().any(|item| item.chat.id == held.chat.id))
            .cloned()
            .collect();
        let gone: Vec<i64> = self
            .chats
            .iter()
            .filter(|held| held.chat.is_group() && !newer.contains(&held.chat.id))
            .filter(|held| !fresh.iter().any(|item| item.chat.id == held.chat.id))
            .map(|held| held.chat.id)
            .collect();
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
                if newer.contains(&item.chat.id) {
                    item.chat = held.chat.clone();
                }
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
        self.chats.extend(kept);
        for chat_id in &gone {
            self.remove_chat(*chat_id);
        }
        gone
    }

    /// Rows a reload left in the outbox, back in it — each with its bubble,
    /// so a message the person saw as "Sending…" goes on being sent rather
    /// than vanishing with the page (docs/protocol.md, "Sending on an
    /// unreliable network").
    ///
    /// Its attachments come back without their bytes, which a reload does
    /// not keep. Uploads that had landed are good as they are; a row with
    /// bytes still to upload fails at once, saying why — a location is the
    /// exception, being three numbers the row still has. Every item gets a
    /// fresh provisional id: the old session's could meet this one's.
    pub fn restore_outbox(&mut self, rows: Vec<Outgoing>) {
        for mut row in rows {
            if self
                .outbox
                .iter()
                .any(|held| held.client_msg_id == row.client_msg_id)
            {
                continue;
            }
            for item in row.items.iter_mut() {
                item.provisional_id = self.next_provisional();
            }
            let lost = row
                .items
                .iter()
                .any(|item| item.attachment_id.is_none() && !item.survives_reload());
            if lost {
                row.failed = Some(lost_in_reload().to_string());
            }
            self.enqueue(row);
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
                if let Some(index) = self
                    .outbox
                    .iter()
                    .position(|row| row.chat_id == chat_id && row.client_msg_id == client_msg_id)
                {
                    let row = self.outbox.remove(index);
                    self.forget_bytes(&row);
                }
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
        // reply the chat's own window is only now receiving. A catch-up is
        // the exception, and only for a root it brought along itself: that
        // copy's count already holds the replies on the page.
        if let Some(root) = thread_root {
            let counts = match via {
                Via::Frame | Via::Answer => true,
                Via::CatchUp { held_through, .. } => root <= held_through,
                Via::Pending => false,
            };
            if counts {
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
            Via::CatchUp { counted_up_to, .. } => message_id > counted_up_to,
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

    /// How much of `chat_id` is unread as it opens: the list's count, or
    /// what the rows now held say is above the reader's own marker — the
    /// larger. With the socket down the list is a snapshot from before the
    /// messages that just loaded; trusting it alone drew "1 new message"
    /// over forty and marked all of them read. The marker only ever lags,
    /// and a lagging marker counts MORE as new, which is the safe side.
    pub fn opening_unread(&self, chat_id: i64) -> i64 {
        let Some(item) = self.item(chat_id) else {
            return 0;
        };
        let above_marker = self.threads.get(&chat_id).map_or(0, |thread| {
            thread
                .messages
                .iter()
                .filter(|message| {
                    message.id > item.last_read_message_id && message.sender_id != self.my_user_id
                })
                .count() as i64
        });
        item.unread_count.max(above_marker)
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
            // Looked for first: a list a pane still holds is copied by the
            // first change to it, and an edit nobody quoted changes none.
            let quotes_it = |message: &Message| {
                message.reply_to.as_ref().is_some_and(|quote| {
                    quote.message_id == id
                        || quote
                            .parent
                            .as_ref()
                            .is_some_and(|parent| parent.message_id == id)
                })
            };
            if !thread.messages.iter().any(quotes_it) {
                return;
            }
            for quoting in thread.messages.make_mut().iter_mut() {
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

    /// The assistant stopped early: the row keeps what arrived, and says so
    /// — and says WHY when the frame did, the latest frame's word winning.
    pub fn apply_ai_error(&mut self, message_id: i64, failure: AiFailure) {
        self.ai_failed.insert(message_id, failure);
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
    /// what it answers, if it answers anything, and drawing its attachments
    /// from this device's own bytes under provisional ids — and the row
    /// joins the END of the outbox; `apply_message` settles both when the
    /// server's copy arrives. Answers the provisional ids given, in order.
    pub fn queue_send(&mut self, chat_id: i64, client_msg_id: String, draft: Draft) -> Vec<i64> {
        let animation = sends_as_animation(&draft);
        let mut items = Vec::with_capacity(draft.attachments.len());
        for prepared in &draft.attachments {
            let provisional = self.next_provisional();
            items.push(OutgoingItem::new(prepared, provisional));
            self.bytes.insert(
                provisional,
                StagedBytes {
                    file: prepared.file.clone(),
                    preview: prepared.preview.clone(),
                },
            );
        }
        let ids = items.iter().map(|item| item.provisional_id).collect();
        self.enqueue(Outgoing {
            chat_id,
            client_msg_id,
            body: draft.body,
            reply_to_message_id: draft.reply_to_message_id,
            mentions: draft.mentions,
            poll: draft.poll,
            items,
            sticker: draft.sticker,
            round: draft.round,
            animation,
            attempts: 0,
            failed: None,
        });
        ids
    }

    fn next_provisional(&mut self) -> i64 {
        self.last_provisional = self.last_provisional.min(0) - 1;
        self.last_provisional
    }

    /// A voice message kept as not sent in `chat_id` (the plan's S2.8), at
    /// the end of that chat's rows. Returns its number.
    pub fn park(
        &mut self,
        chat_id: i64,
        note: Prepared,
        duration_ms: i64,
        reply_to_message_id: Option<i64>,
        caption: String,
    ) -> u64 {
        self.last_not_sent += 1;
        let id = self.last_not_sent;
        self.not_sent.entry(chat_id).or_default().push(NotSent {
            id,
            note,
            duration_ms,
            reply_to_message_id,
            caption,
        });
        id
    }

    /// The not-sent voice message `id` of `chat_id`, taken out — to be sent
    /// by its own Send, or deleted by its ✕.
    pub fn take_not_sent(&mut self, chat_id: i64, id: u64) -> Option<NotSent> {
        let rows = self.not_sent.get_mut(&chat_id)?;
        let at = rows.iter().position(|row| row.id == id)?;
        let taken = rows.remove(at);
        if rows.is_empty() {
            self.not_sent.remove(&chat_id);
        }
        Some(taken)
    }

    /// `chat_id` has been left with `draft` in its box and the composer
    /// answering `reply_to_message_id`: every voice note still in review
    /// there — staged, and not sent — becomes not sent (the plan's S2.8),
    /// and the first takes the words along as its caption. Whatever else is
    /// staged stays staged, as it always has in this tab. Returns whether
    /// anything was kept so, which is whether the box's words went too.
    pub fn park_review(
        &mut self,
        chat_id: i64,
        draft: &str,
        reply_to_message_id: Option<i64>,
    ) -> bool {
        let Some(staged) = self.staged.get_mut(&chat_id) else {
            return false;
        };
        let (notes, rest): (Vec<Prepared>, Vec<Prepared>) =
            staged.drain(..).partition(Prepared::is_voice_note);
        *staged = rest;
        if staged.is_empty() {
            self.staged.remove(&chat_id);
        }
        let mut caption = fc_text::composer::trimmed_for_send(draft)
            .unwrap_or("")
            .to_string();
        let parked = !notes.is_empty();
        for note in notes {
            let duration_ms = note.duration_ms.unwrap_or(0);
            self.park(
                chat_id,
                note,
                duration_ms,
                reply_to_message_id,
                std::mem::take(&mut caption),
            );
        }
        parked
    }

    /// Whether this tab holds something nobody else has and the tab's end
    /// would lose: a message not sent yet, something staged, a voice message
    /// that was not sent. A recording in progress is the recorder's to say
    /// (`recorder::in_progress`).
    pub fn holds_unsent(&self) -> bool {
        !self.outbox.is_empty()
            || self.staged.values().any(|items| !items.is_empty())
            || self.not_sent.values().any(|rows| !rows.is_empty())
    }

    /// A row into the outbox, and its bubble into the thread.
    fn enqueue(&mut self, row: Outgoing) {
        let chat_id = row.chat_id;
        let reply_to = row
            .reply_to_message_id
            .and_then(|id| self.local_quote(chat_id, id));
        // The chain a pending reply belongs to, decided the server's way —
        // the quoted message's own root, or the quoted message itself — so
        // the thread surface shows it (and any failure of it) at once.
        let thread_root_id = row.reply_to_message_id.map(|id| {
            self.find(chat_id, id)
                .and_then(|quoted| quoted.thread_root_id)
                .unwrap_or(id)
        });
        // A sticker's pending bubble is already the sticker: drawn bare,
        // from this tab's own bytes, before the server has said anything —
        // and a video message's is already the circle (the plan for #79,
        // S5.6).
        let attachments: Vec<crate::model::Attachment> = row
            .items
            .iter()
            .map(|item| crate::model::Attachment {
                sticker: row.sticker,
                round: row.round,
                animation: row.animation,
                ..item.as_attachment()
            })
            .collect();
        let pending = Message {
            id: 0,
            chat_id,
            sender_id: self.my_user_id,
            client_msg_id: Some(row.client_msg_id.clone()),
            body: row.body.clone(),
            reply_to,
            thread_root_id,
            mentions: (!row.mentions.is_empty()).then(|| row.mentions.clone()),
            attachments: (!attachments.is_empty()).then_some(attachments),
            // A poll that is not numbered yet cannot be voted on: options
            // with no ids draw as the question and its choices, disabled.
            poll: row.poll.clone().map(|options| Poll {
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
        self.outbox.push(row);
        self.apply_message(pending, None, Via::Pending);
    }

    /// One attachment of a queued message landed on the server. That is
    /// progress, and the row's count of unknown outcomes starts again: a
    /// network that just carried an upload is one worth trying again.
    pub fn landed(&mut self, client_msg_id: &str, provisional_id: i64, attachment_id: i64) {
        let Some(row) = self.row_mut(client_msg_id) else {
            return;
        };
        if let Some(item) = row
            .items
            .iter_mut()
            .find(|item| item.provisional_id == provisional_id)
        {
            item.attachment_id = Some(attachment_id);
            row.attempts = 0;
            self.aliases.insert(attachment_id, provisional_id);
        }
    }

    /// The server swept this row's uploads before the message claimed them
    /// (`attachment_expired`): upload them again — possible only while every
    /// item still has its bytes, or is a location, which needs none, or a
    /// sticker, whose bytes are the pack's to fetch again.
    /// Answers whether the row can go on.
    pub fn expire_uploads(&mut self, client_msg_id: &str) -> bool {
        let Some(row) = self
            .outbox
            .iter()
            .find(|row| row.client_msg_id == client_msg_id)
        else {
            return false;
        };
        let recoverable = !row.items.is_empty()
            && row.items.iter().all(|item| {
                item.survives_reload()
                    || self
                        .bytes
                        .get(&item.provisional_id)
                        .is_some_and(|bytes| bytes.file.is_some())
            });
        if !recoverable {
            return false;
        }
        if let Some(row) = self.row_mut(client_msg_id) {
            for item in row.items.iter_mut() {
                item.attachment_id = None;
            }
        }
        true
    }

    /// The bytes a row no longer needs, gone with it.
    fn forget_bytes(&mut self, row: &Outgoing) {
        for item in &row.items {
            self.bytes.remove(&item.provisional_id);
        }
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
        if let Some(index) = self
            .outbox
            .iter()
            .position(|row| row.client_msg_id == client_msg_id)
        {
            let row = self.outbox.remove(index);
            self.forget_bytes(&row);
        }
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
            // Two sentences, so the second is the apps' own.
            row.failed = Some(format!(
                "{} {}",
                t("Not sent."),
                t("Check your connection and try again.")
            ));
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
        self.forget_bytes(&row);
        let is_it = |message: &Message| {
            message.id == 0 && message.client_msg_id.as_deref() == Some(client_msg_id)
        };
        let Some(thread) = self.threads.get_mut(&row.chat_id) else {
            return;
        };
        thread.messages.make_mut().retain(|message| !is_it(message));
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

    /// `chat_updated` — a group created, renamed, or its members or admin
    /// changed — and the answer to this device's own create or change: the
    /// chat whole, into the list or over the one held, with everything else
    /// on its row kept (docs/protocol.md, "Groups"). A group that no longer
    /// lists this member is one they are out of, and goes.
    pub fn upsert_chat(&mut self, chat: Chat) {
        self.touch_chat(chat.id);
        if chat.is_group() && !chat.group_member_ids().contains(&self.my_user_id) {
            self.remove_chat(chat.id);
            return;
        }
        match self.item_mut(chat.id) {
            Some(item) => item.chat = chat,
            // Nothing unread: a member just added starts with the group's
            // newest message read, and the creator has written nothing yet.
            None => self.chats.push(ChatListItem {
                chat,
                ..ChatListItem::default()
            }),
        }
    }

    /// `chat_removed` — a group this member left, was removed from, or that
    /// was deleted — or this device's own leave or delete answered: the
    /// chat, its messages, its cursors and anything being written there,
    /// gone (docs/protocol.md, "Groups"). What waits in the outbox for it
    /// would be refused `chat_not_found`, on a bubble nobody can see any
    /// more, so it goes with its bytes. Answers whether anything was held.
    pub fn remove_chat(&mut self, chat_id: i64) -> bool {
        self.touch_chat(chat_id);
        let held = self.item(chat_id).is_some() || self.threads.contains_key(&chat_id);
        self.chats.retain(|item| item.chat.id != chat_id);
        self.threads.remove(&chat_id);
        self.cursors.remove(&chat_id);
        self.listed.remove(&chat_id);
        self.typing.remove(&chat_id);
        self.peer_read.remove(&chat_id);
        self.drafts.remove(&chat_id);
        self.staged.remove(&chat_id);
        self.not_sent.remove(&chat_id);
        let gone: Vec<i64> = self
            .outbox
            .iter()
            .filter(|row| row.chat_id == chat_id)
            .flat_map(|row| row.items.iter().map(|item| item.provisional_id))
            .collect();
        for provisional in gone {
            self.bytes.remove(&provisional);
        }
        self.outbox.retain(|row| row.chat_id != chat_id);
        if self
            .thread_view
            .as_ref()
            .is_some_and(|view| view.chat_id == chat_id)
        {
            self.thread_view = None;
        }
        if self
            .open_polls
            .as_ref()
            .is_some_and(|polls| polls.chat_id == chat_id)
        {
            self.open_polls = None;
        }
        held
    }

    /// This chat's row changed here, after any list now in flight began.
    fn touch_chat(&mut self, chat_id: i64) {
        self.chat_changes += 1;
        self.chat_touched.insert(chat_id, self.chat_changes);
    }

    /// Somebody arrived. `password_reset` is the flag the family OWNER's
    /// copy of the frame carries — whether they may reset the newcomer's
    /// password, false when the newcomer is also in a family they do not
    /// own (docs/protocol.md, "Several families"). Everybody else's copy,
    /// and an older server's, carries none, and absent says NOTHING: a flag
    /// already held for that member (the roster a resync read a moment ago
    /// can beat the frame here) is kept, never clobbered with an unknown.
    /// Only a newcomer with no flag at all falls back to the old rule,
    /// where the reset is offered.
    pub fn member_joined(&mut self, user: &User, password_reset: Option<bool>) {
        self.roster_changes += 1;
        self.names.insert(user.id, user.display_name.clone());
        match self.members.iter_mut().find(|member| member.id == user.id) {
            Some(member) => {
                if password_reset.is_some() {
                    member.password_reset = password_reset;
                }
            }
            None => self.members.push(Member {
                id: user.id,
                display_name: user.display_name.clone(),
                username: user.username.clone(),
                role: Some("member".to_string()),
                deleted: false,
                avatar_version: user.avatar_version,
                birthday: user.birthday,
                password_reset,
            }),
        }
    }

    /// Somebody left. They are no longer offered as somebody to name, but
    /// their name stays on everything they said.
    pub fn member_left(&mut self, user_id: i64) {
        self.roster_changes += 1;
        self.members.retain(|member| member.id != user_id);
    }

    /// An account was deleted: the tombstone is WRITTEN, deliberately — the
    /// one frame whose job is to wipe what is stored.
    pub fn member_deleted(&mut self, member: &Member) {
        self.roster_changes += 1;
        self.members.retain(|held| held.id != member.id);
        self.names.insert(member.id, member.display_name.clone());
    }

    pub fn family_owner(&mut self, user_id: i64) {
        self.roster_changes += 1;
        if self.account.is_some() {
            self.set_my_role(if self.my_user_id == user_id {
                "owner"
            } else {
                "member"
            });
        }
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

    /// Every unread message, across every chat — the number a running
    /// client puts in front of its reader (the apps put it on their icon,
    /// a tab puts it in its title; ios Core/UnreadBadge.swift).
    pub fn unread_total(&self) -> i64 {
        self.chats.iter().map(|item| item.unread_count).sum()
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
            .unwrap_or_else(|| t("Someone").to_string())
    }
}

/// The families `/me` lists — or, from a server that predates several
/// families, the one it names, with the role it names (docs/protocol.md,
/// "Apps from before this section").
pub fn families_of_me(me: &Me) -> Vec<Family> {
    if !me.families.is_empty() {
        return me.families.clone();
    }
    me.family
        .clone()
        .map(|family| Family {
            role: family.role.clone().or_else(|| me.role.clone()),
            ..family
        })
        .into_iter()
        .collect()
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

    /// `Thread::apply` as it was before it kept the order itself: a search
    /// of every row, then a sort of every row — the reference the binary
    /// search and the insert are held to.
    fn apply_by_sorting(thread: &mut Thread, incoming: Message) -> Arrival {
        if incoming.id != 0 {
            if let Some(held) = thread
                .messages
                .make_mut()
                .iter_mut()
                .find(|held| held.id == incoming.id)
            {
                merge(held, incoming);
                return Arrival::Known;
            }
        }
        if let Some(client_msg_id) = incoming.client_msg_id.clone() {
            if let Some(pending) = thread.messages.make_mut().iter_mut().find(|held| {
                held.id == 0 && held.client_msg_id.as_deref() == Some(client_msg_id.as_str())
            }) {
                let arrival = if incoming.id == 0 {
                    Arrival::Known
                } else {
                    Arrival::New
                };
                *pending = incoming;
                thread.resort();
                return arrival;
            }
        }
        let arrival = if incoming.id == 0 {
            Arrival::Known
        } else {
            Arrival::New
        };
        thread.messages.make_mut().push(incoming);
        thread.resort();
        arrival
    }

    /// Every kind of arrival, in an order nobody would choose: history and
    /// frames out of order, copies of what is held (an older edit and a
    /// newer one), pending bubbles written, written again, and numbered —
    /// some of them only after the server's copy of a LATER message.
    /// Whatever arrives, the thread ends up exactly as searching and
    /// sorting after every arrival left it.
    #[wasm_bindgen_test]
    fn a_thread_keeps_the_order_a_sort_after_every_arrival_would() {
        let mut seed: u64 = 0x5eed;
        let mut next = |below: u64| {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (seed >> 33) % below
        };
        let mut kept = Thread::default();
        let mut sorted = Thread::default();
        for step in 0..600 {
            let incoming = match next(6) {
                // A numbered message, often one already held.
                0..=2 => {
                    let id = 1 + next(150) as i64;
                    let mut message = message(id, ANNA, &format!("copy {step} of {id}"));
                    message.edit_seq = Some(next(4) as i64);
                    message
                }
                // A bubble of mine, maybe one already pending.
                3 => pending(&format!("c{}", next(40)), &format!("draft {step}")),
                // The server's copy of one of my bubbles.
                _ => {
                    let mut numbered = pending(&format!("c{}", next(40)), "sent");
                    numbered.id = 1 + next(150) as i64;
                    numbered
                }
            };
            assert_eq!(
                kept.apply(incoming.clone()),
                apply_by_sorting(&mut sorted, incoming),
                "the same answer at step {step}"
            );
            assert_eq!(kept, sorted, "the same thread after step {step}");
        }
        assert!(kept.messages.len() > 100, "the walk built a real thread");
        assert!(kept.messages.iter().any(|message| message.id == 0));
        assert_eq!(kept.oldest, sorted.oldest);
        assert_eq!(kept.newest_server_id(), sorted.newest_server_id());
        for id in 0..160 {
            assert_eq!(
                kept.get(id).map(|message| &message.body),
                sorted
                    .messages
                    .iter()
                    .find(|message| message.id == id && id != 0)
                    .map(|message| &message.body),
                "row {id} found by the binary search"
            );
        }
    }

    /// A draw holds the very list the store holds — equal by the pointer,
    /// without a message compared — and the store's next change to it
    /// copies it, leaving the draw's copy as it was.
    #[wasm_bindgen_test]
    fn a_threads_messages_are_shared_until_they_change() {
        let mut thread = Thread::default();
        thread.apply(message(1, ANNA, "one"));
        let drawn = thread.messages.clone();
        assert!(
            Rc::ptr_eq(&drawn.0, &thread.messages.0),
            "shared, not copied"
        );
        assert_eq!(drawn, thread.messages);

        thread.apply(message(2, ANNA, "two"));
        assert_eq!(drawn.len(), 1, "what was drawn is left as it was");
        assert_eq!(thread.messages.len(), 2);
        assert_ne!(drawn, thread.messages);

        // Two separate lists holding the same messages are still equal.
        let copy = Messages::from(thread.messages.to_vec());
        assert!(!Rc::ptr_eq(&copy.0, &thread.messages.0));
        assert_eq!(copy, thread.messages);

        // An edit changes the store's list and not the one being drawn.
        let mut store = store();
        store.threads.insert(42, thread);
        let before = store.threads[&42].messages.clone();
        let mut edited = message(1, ANNA, "one, edited");
        edited.edit_seq = Some(1);
        store.apply_edit(edited);
        assert_eq!(store.threads[&42].messages[0].body, "one, edited");
        assert_eq!(before[0].body, "one", "the draw's list is untouched");
    }

    fn chat(id: i64, kind: &str) -> ChatListItem {
        ChatListItem {
            chat: Chat {
                id,
                kind: kind.into(),
                title: Some("The Smiths".into()),
                peer_user_id: (kind == "direct").then_some(ANNA),
                ..Chat::default()
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
        store.apply_message(
            reply,
            None,
            Via::CatchUp {
                counted_up_to: 101,
                held_through: 100,
            },
        );

        assert_eq!(store.chats[0].unread_count, 3, "the server's count stands");
        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(1)
        );
        assert_eq!(store.threads[&42].newest_server_id(), Some(101));
    }

    /// THE FIRST SYNC. Root and replies arrive on one catch-up page, and
    /// the root's own copy already counts them — so counting them again is
    /// what drew "8 replies" under a message with four (docs/protocol.md,
    /// "Threads" -> Live). The cursor the pass opened with is what tells
    /// the two cases apart: a root at or below it was already held.
    #[wasm_bindgen_test]
    fn a_catch_up_that_brought_the_root_itself_does_not_count_its_replies() {
        let mut store = store();
        let mut root = message(100, ANNA, "Still coming Sunday?");
        root.reply_count = Some(2);
        let mut first = message(101, ANNA, "Sunday works");
        first.thread_root_id = Some(100);
        let mut second = message(102, ANNA, "I'll bring the bread");
        second.thread_root_id = Some(100);

        for arriving in [root, first, second] {
            store.apply_message(
                arriving,
                None,
                Via::CatchUp {
                    counted_up_to: 0,
                    held_through: 0, // a first sync holds nothing
                },
            );
        }

        assert_eq!(
            store.threads[&42].get(100).and_then(|m| m.reply_count),
            Some(2),
            "the root rode with the page, so its recomputed count already holds both replies"
        );
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

    /// A failed answer remembers WHICH failure it was, for exactly as long
    /// as it remembers that it failed: a refusal stays a refusal through
    /// every redraw, a later frame's word wins, and a finished row or a
    /// sign-out forgets both at once (docs/protocol.md, "The assistant").
    #[wasm_bindgen_test]
    fn a_failed_answer_remembers_why_until_it_is_finished() {
        let mut store = store();
        store.apply_message(message(100, 2, ""), Some(42), Via::Frame);
        store.apply_message(message(101, 2, "Half an"), Some(42), Via::Frame);
        store.apply_ai_error(100, AiFailure::Refused);
        store.apply_ai_error(101, AiFailure::Failed);
        assert_eq!(store.ai_failed.get(&100), Some(&AiFailure::Refused));
        assert_eq!(store.ai_failed.get(&101), Some(&AiFailure::Failed));

        store.apply_ai_error(101, AiFailure::Refused);
        assert_eq!(
            store.ai_failed.get(&101),
            Some(&AiFailure::Refused),
            "the latest frame's word wins"
        );

        let mut done = message(100, 2, "Here it is after all.");
        done.edit_seq = Some(1);
        store.apply_edit(done);
        assert_eq!(
            store.ai_failed.get(&100),
            None,
            "a finished answer is a finished answer"
        );
        assert_eq!(store.ai_failed.get(&101), Some(&AiFailure::Refused));

        // A paid family's month used up is remembered the same way, with
        // the day it renews.
        let spent = AiFailure::AllowanceUsed {
            renews_at: Some("2026-12-01T00:00:00Z".into()),
        };
        store.apply_ai_error(101, spent.clone());
        assert_eq!(store.ai_failed.get(&101), Some(&spent));
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
                ..Default::default()
            },
            family: None,
            blocked_user_ids: vec![11],
            support_contact: None,
            ..Default::default()
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
        store.member_joined(
            &User {
                id: 11,
                username: "junior".into(),
                display_name: "Junior".into(),
                deleted: false,
                ..Default::default()
            },
            None,
        );
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
            ..Default::default()
        });
        assert_eq!(
            store.name_of(11),
            "Deleted account",
            "the tombstone is written"
        );
    }

    /// The owner's copy of `member_joined` says whether the newcomer's
    /// password is theirs to reset — false for somebody also in a family
    /// they do not own, whom the server would refuse with
    /// `member_in_other_families`. A copy without the flag (everybody
    /// else's, or one racing a resync that already read the roster) never
    /// turns a known `false` back into the old rule's "offered".
    #[wasm_bindgen_test]
    fn a_newcomers_reset_flag_is_kept_and_never_clobbered() {
        let mut store = store();
        let junior = User {
            id: 11,
            display_name: "Junior".into(),
            ..Default::default()
        };
        let flag = |store: &Store| {
            store
                .members
                .iter()
                .find(|m| m.id == 11)
                .and_then(|m| m.password_reset)
        };

        store.member_joined(&junior, Some(false));
        assert_eq!(flag(&store), Some(false), "a newcomer takes the flag");

        store.member_joined(&junior, None);
        assert_eq!(flag(&store), Some(false), "absent keeps what is held");

        store.member_joined(&junior, Some(true));
        assert_eq!(flag(&store), Some(true), "present replaces it");
        assert_eq!(
            store.members.iter().filter(|m| m.id == 11).count(),
            1,
            "still one row"
        );

        // A newcomer the frame says nothing about: the old rule, offered.
        store.member_joined(
            &User {
                id: 12,
                ..Default::default()
            },
            None,
        );
        assert_eq!(
            store
                .members
                .iter()
                .find(|m| m.id == 12)
                .map(|m| m.password_reset),
            Some(None)
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
                    held_through: 100,
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

    /// A reload's rows fail AT the reload when their bytes did not survive
    /// it — shown at once, not whenever the outbox next reaches them — and
    /// a row whose uploads all landed, or that is only a place, does not.
    #[wasm_bindgen_test]
    fn a_reload_fails_at_once_what_it_cannot_upload() {
        let mut before = store();
        before.queue_send(
            42,
            "photo".into(),
            Draft {
                attachments: vec![Prepared {
                    kind: "photo".into(),
                    mime: "image/jpeg".into(),
                    file: Some(web_sys::Blob::new().expect("a blob")),
                    ..Prepared::default()
                }],
                ..Draft::default()
            },
        );
        before.queue_send(
            42,
            "landed".into(),
            Draft {
                attachments: vec![Prepared {
                    kind: "photo".into(),
                    mime: "image/jpeg".into(),
                    file: Some(web_sys::Blob::new().expect("a blob")),
                    ..Prepared::default()
                }],
                ..Draft::default()
            },
        );
        let landed = before.outbox[1].items[0].provisional_id;
        before.landed("landed", landed, 501);
        before.queue_send(
            42,
            "place".into(),
            Draft {
                attachments: vec![Prepared::location(1.0, 2.0, None)],
                ..Draft::default()
            },
        );

        let mut after = store();
        after.restore_outbox(before.outbox.clone());

        let failed = after.failed_sends(42);
        assert_eq!(
            failed.get("photo").map(String::as_str),
            Some(lost_in_reload())
        );
        assert!(
            !failed.contains_key("landed"),
            "its upload is good for the grace"
        );
        assert!(!failed.contains_key("place"), "a place needs no bytes");
        let ids: Vec<i64> = after
            .outbox
            .iter()
            .flat_map(|row| &row.items)
            .map(|item| item.provisional_id)
            .collect();
        assert!(ids.iter().all(|id| *id < 0), "{ids:?}");
        let mut distinct = ids.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), ids.len(), "each its own");
    }

    /// A chat opening on a stale list count still sees what the rows it just
    /// loaded say is new — above the reader's own marker, from somebody else.
    #[wasm_bindgen_test]
    fn an_opening_counts_at_least_what_it_can_see_is_new() {
        let mut store = store();
        store.chats[0].unread_count = 1; // the list, from before the messages
        store.chats[0].last_read_message_id = 100;
        let page: Vec<Message> = (99..=110)
            .map(|id| message(id, if id == 105 { ME } else { ANNA }, "x"))
            .collect();
        store.apply_history(42, page, false);
        assert_eq!(
            store.opening_unread(42),
            9,
            "101..=110 from Anna, 105 being mine"
        );
        store.chats[0].unread_count = 30; // the list knows of more than loaded
        assert_eq!(store.opening_unread(42), 30);
        assert_eq!(store.opening_unread(7), 0, "no such chat");
    }
    /// Another family is another wall: nothing of the old one's feed will
    /// tombstone its notes, so they go, and the next read is a whole one —
    /// but the badge's marks are the person's, and stay.
    #[wasm_bindgen_test]
    fn a_new_family_starts_a_new_wall_and_keeps_the_marks() {
        let family = |id: i64| crate::model::Family {
            id,
            name: "The Smiths".into(),
            ai_history: true,
            ai_vision: false,
            ai_history_photos: false,
            ..Default::default()
        };
        let me = |family_id: Option<i64>| Me {
            user: User {
                id: 7,
                username: "me".into(),
                display_name: "Me".into(),
                deleted: false,
                ..Default::default()
            },
            family: family_id.map(family),
            blocked_user_ids: Vec::new(),
            support_contact: None,
            ..Default::default()
        };
        let mut store = Store::default();
        store.apply_me(&me(Some(3)));
        store.board.take_marks(
            7,
            fc_text::board::Marks {
                note_id: 5,
                content_seq: 50,
            },
        );
        store.board.apply_full(
            vec![crate::model::Note {
                id: 1,
                board_seq: 10,
                author_id: Some(7),
                text: Some("old wall".into()),
                color: Some("yellow".into()),
                x: Some(0.1),
                y: Some(0.1),
                ..Default::default()
            }],
            10,
        );
        store.apply_me(&me(Some(3)));
        assert_eq!(store.board.notes.len(), 1, "the same family keeps its wall");
        store.apply_me(&me(Some(4)));
        assert!(store.board.notes.is_empty() && !store.board.loaded);
        assert_eq!(store.board.cursor, 0);
        // A board is a family's, and so are its marks: the new wall starts
        // with none until its own are taken from what the browser kept
        // (sync::take_kept_marks).
        assert_eq!(store.board.marks, fc_text::board::Marks::default());
        assert_eq!(store.board.marks_for, 0);
    }

    fn account(family: Option<i64>, pending: bool) -> Me {
        Me {
            user: User {
                id: ME,
                username: "me".into(),
                display_name: "Me".into(),
                ..Default::default()
            },
            family: family.map(|id| crate::model::Family {
                id,
                name: "The Smiths".into(),
                ..Default::default()
            }),
            pending_join_request: pending.then(|| crate::model::PendingJoin {
                family_id: 3,
                family_name: "The Smiths".into(),
                created_at: None,
            }),
            ..Default::default()
        }
    }

    /// A refusal is never said: the request just vanishes from `/me`. Only
    /// an account that was WAITING has been declined.
    #[wasm_bindgen_test]
    fn a_declined_request_is_told_by_its_vanishing() {
        let mut store = Store::default();
        store.apply_me(&account(None, false));
        assert!(!store.join_declined, "never asked is not declined");
        store.apply_me(&account(None, true));
        assert!(store.awaiting_join && !store.join_declined);
        store.apply_me(&account(None, true));
        assert!(store.awaiting_join, "still waiting");
        store.apply_me(&account(None, false));
        assert!(store.join_declined && !store.awaiting_join);
        store.apply_me(&account(None, false));
        assert!(
            store.join_declined,
            "said until the next join, not only once"
        );
        store.apply_me(&account(Some(3), false));
        assert!(!store.join_declined && !store.awaiting_join);
    }

    #[wasm_bindgen_test]
    fn an_approved_request_is_not_a_declined_one() {
        let mut store = Store::default();
        store.apply_me(&account(None, true));
        store.apply_me(&account(Some(3), false));
        assert!(!store.join_declined && !store.awaiting_join);
    }

    /// The owner's lists belong to the family they were read in.
    #[wasm_bindgen_test]
    fn leaving_forgets_the_owners_lists() {
        let mut store = Store::default();
        store.apply_me(&account(Some(3), false));
        store.join_requests = vec![crate::model::JoinRequest {
            id: 1,
            user: User::default(),
            created_at: None,
        }];
        store.reports = vec![crate::model::Report {
            id: 2,
            reporter: User::default(),
            reported: User::default(),
            reason: "spam".into(),
            created_at: None,
            message_id: None,
            message_excerpt: None,
            message_attachments: Vec::new(),
        }];
        store.apply_me(&account(None, false));
        assert!(store.join_requests.is_empty() && store.reports.is_empty());
    }

    /// Taking the picture away changes the version and nothing else — the
    /// birthday stays (ios rebuilds the user and loses it) — and the same
    /// the other way round.
    #[wasm_bindgen_test]
    fn a_picture_and_a_birthday_change_only_themselves() {
        let mut store = Store::default();
        let mut me = account(Some(3), false);
        me.user.avatar_version = 4;
        me.user.birthday = Some(crate::model::Birthday { month: 3, day: 14 });
        store.apply_me(&me);
        store.members = vec![Member {
            id: ME,
            avatar_version: 4,
            birthday: Some(crate::model::Birthday { month: 3, day: 14 }),
            ..Default::default()
        }];
        store.set_avatar_version(ME, 0);
        let held = &store.account.as_ref().unwrap().user;
        assert_eq!(held.avatar_version, 0);
        assert_eq!(
            held.birthday,
            Some(crate::model::Birthday { month: 3, day: 14 })
        );
        assert_eq!(store.members[0].avatar_version, 0);
        assert!(store.members[0].birthday.is_some());
        store.set_birthday(ME, None);
        assert_eq!(store.account.as_ref().unwrap().user.birthday, None);
        assert_eq!(store.members[0].birthday, None);
        assert_eq!(store.members[0].avatar_version, 0);
    }

    #[wasm_bindgen_test]
    fn a_rotated_code_replaces_both_copies() {
        let mut store = Store::default();
        let mut me = account(Some(3), false);
        me.role = Some("owner".into());
        store.apply_me(&me);
        store.set_invite_code("NEWCODE2".into());
        assert_eq!(
            store.family.as_ref().unwrap().invite_code.as_deref(),
            Some("NEWCODE2")
        );
        assert_eq!(
            store
                .account
                .as_ref()
                .unwrap()
                .family
                .as_ref()
                .unwrap()
                .invite_code
                .as_deref(),
            Some("NEWCODE2")
        );
    }

    /// A leave dialog is rebuilt when ownership moves under it; this is
    /// what it watches.
    #[wasm_bindgen_test]
    fn ownership_moving_is_counted() {
        let mut store = Store::default();
        let mut me = account(Some(3), false);
        me.role = Some("owner".into());
        store.apply_me(&me);
        let before = store.roster_changes;
        store.family_owner(ANNA);
        assert_eq!(store.roster_changes, before + 1);
        store.member_joined(
            &User {
                id: 12,
                ..Default::default()
            },
            None,
        );
        store.member_left(12);
        assert_eq!(
            store.roster_changes,
            before + 3,
            "joins and leaves move the heir too"
        );
        assert!(!store.is_owner());
    }

    /// A reload with something unsent: the rows come back with their
    /// bubbles before `/me` has answered, and that first answer — which
    /// finds no family held yet — is not a family CHANGE that wipes them.
    #[wasm_bindgen_test]
    fn a_reload_keeps_its_unsent_bubbles() {
        let mut before = store();
        before.queue_send(42, "a".into(), text("still going"));
        let rows = before.outbox.clone();
        let mut after = Store {
            my_user_id: ME,
            ..Store::default()
        };
        after.restore_outbox(rows);
        let changed = after.apply_me(&account(Some(3), false));
        assert!(!changed, "the first answer is not a change");
        assert_eq!(after.outbox.len(), 1);
        assert!(
            after.threads[&42]
                .messages
                .iter()
                .any(|m| m.client_msg_id.as_deref() == Some("a")),
            "the bubble is still there to show how it goes"
        );
    }

    /// Leaving takes the family chat and the direct chats, and whatever
    /// was on its way to them — it can never go now — but what belongs to
    /// another family this account is still in stays: here its assistant's
    /// chat, with its history and its unsent rows (docs/protocol.md,
    /// "Several families": the `ai` chat is per family).
    #[wasm_bindgen_test]
    fn leaving_keeps_only_what_is_the_persons() {
        let mut store = store();
        let mut theirs = chat(50, "ai");
        theirs.chat.family_ids = Some(vec![4]);
        store.chats.push(theirs);
        let mut both = account(Some(3), false);
        both.families = vec![
            crate::model::Family {
                id: 3,
                name: "The Smiths".into(),
                ..Default::default()
            },
            crate::model::Family {
                id: 4,
                name: "Ours".into(),
                ..Default::default()
            },
        ];
        store.apply_me(&both);
        store.queue_send(42, "family".into(), text("to the family"));
        store.queue_send(50, "ai".into(), text("to the assistant"));
        store.drafts.insert(42, "half a thought".into());
        store.drafts.insert(50, "a question".into());
        let voice = || Prepared {
            kind: "audio".into(),
            duration_ms: Some(3_000),
            ..Prepared::default()
        };
        store.park(42, voice(), 3_000, None, String::new());
        store.park(50, voice(), 3_000, None, String::new());
        let mut left = account(Some(4), false);
        left.families = vec![both.families[1].clone()];
        assert!(store.apply_me(&left), "a change");
        let rows: Vec<&str> = store
            .outbox
            .iter()
            .map(|row| row.client_msg_id.as_str())
            .collect();
        assert_eq!(rows, vec!["ai"]);
        assert!(store.threads.contains_key(&50) && !store.threads.contains_key(&42));
        assert!(store.item(50).is_some() && store.item(42).is_none());
        assert_eq!(
            store.drafts.get(&50).map(String::as_str),
            Some("a question")
        );
        assert!(!store.drafts.contains_key(&42));
        // A voice message that was not sent goes with its chat too (#79).
        assert!(!store.not_sent.contains_key(&42));
        assert_eq!(store.not_sent.get(&50).map(Vec::len), Some(1));
    }

    fn group(id: i64, members: &[i64], admin: i64) -> Chat {
        Chat {
            id,
            kind: "group".into(),
            title: Some("Cousins".into()),
            member_ids: Some(members.to_vec()),
            admin_user_id: Some(admin),
            ..Chat::default()
        }
    }

    /// `chat_updated` puts a group in the list — a member just added sees
    /// it at once — and over the one held, keeping what the row knows; a
    /// group that no longer lists this member is one they are out of
    /// (docs/protocol.md, "Groups").
    #[wasm_bindgen_test]
    fn a_group_frame_adds_the_group_and_changes_it_in_place() {
        let mut store = store();
        store.upsert_chat(group(77, &[ANNA, ME], ANNA));
        let held = store.item(77).expect("listed at once");
        assert_eq!(held.unread_count, 0);
        assert_eq!(held.chat.title.as_deref(), Some("Cousins"));
        store.chats[1].unread_count = 2;
        let mut renamed = group(77, &[ANNA, ME, 11], ANNA);
        renamed.title = Some("The cousins".into());
        store.upsert_chat(renamed);
        assert_eq!(store.chats.len(), 2, "the same row");
        let held = store.item(77).expect("still listed");
        assert_eq!(held.chat.title.as_deref(), Some("The cousins"));
        assert_eq!(held.chat.group_member_ids(), &[ANNA, ME, 11]);
        assert_eq!(held.unread_count, 2, "what the row knew stays");
        // Below the family chat, as every other chat.
        assert!(store.sorted_chats()[0].chat.is_family());

        store.drafts.insert(77, "see you".into());
        store.upsert_chat(group(77, &[ANNA, 11], ANNA));
        assert!(store.item(77).is_none(), "out of it");
        assert!(!store.drafts.contains_key(&77));
    }

    /// `chat_removed` takes the group and everything held for it: its
    /// messages, its cursors, what was being written there and what was on
    /// its way — which would be refused `chat_not_found` now — and the
    /// thread or the open polls showing it. Nothing else is touched.
    #[wasm_bindgen_test]
    fn a_removed_group_takes_everything_held_for_it() {
        let mut store = store();
        store.upsert_chat(group(77, &[ANNA, ME], ME));
        store.apply_message(
            Message {
                chat_id: 77,
                ..message(500, ANNA, "hello")
            },
            None,
            Via::Frame,
        );
        store.queue_send(77, "g".into(), text("to the group"));
        store.queue_send(42, "f".into(), text("to the family"));
        store.drafts.insert(77, "half".into());
        store.drafts.insert(42, "kept".into());
        store.park(
            77,
            Prepared {
                kind: "audio".into(),
                duration_ms: Some(3_000),
                ..Prepared::default()
            },
            3_000,
            None,
            String::new(),
        );
        store.thread_view = Some(ThreadView {
            chat_id: 77,
            ..ThreadView::default()
        });
        assert!(store.threads.contains_key(&77));

        assert!(store.remove_chat(77));
        assert!(store.item(77).is_none());
        assert!(!store.threads.contains_key(&77));
        assert!(!store.drafts.contains_key(&77) && !store.not_sent.contains_key(&77));
        let rows: Vec<&str> = store
            .outbox
            .iter()
            .map(|row| row.client_msg_id.as_str())
            .collect();
        assert_eq!(rows, vec!["f"]);
        assert!(store.thread_view.is_none());
        assert_eq!(store.drafts.get(&42).map(String::as_str), Some("kept"));
        assert!(!store.remove_chat(77), "nothing left to take");
    }

    /// `GET /chats` is the reconciler: a group no longer in it is gone for
    /// good and takes what was held for it, whatever frame was missed; a
    /// direct chat no longer in it (a block hides it) leaves the list and
    /// keeps its history for the unblock (docs/protocol.md, resync step 2).
    #[wasm_bindgen_test]
    fn a_group_missing_from_the_list_is_gone_and_a_direct_chat_only_hidden() {
        let mut store = store();
        store.chats.push(chat(43, "direct"));
        store.upsert_chat(group(77, &[ANNA, ME], ANNA));
        for chat_id in [43, 77] {
            store.apply_message(
                Message {
                    chat_id,
                    ..message(600 + chat_id, ANNA, "hi")
                },
                None,
                Via::Frame,
            );
        }
        let gone = store.merge_chats(vec![chat(42, "family")]);
        assert_eq!(gone, vec![77]);
        assert!(store.item(43).is_none() && store.item(77).is_none());
        assert!(
            store.threads.contains_key(&43),
            "a direct chat's history stays"
        );
        assert!(!store.threads.contains_key(&77));
    }

    /// A list asked for BEFORE a group was made, or changed, does not drop
    /// it or put its old self back when it lands; one asked for before a
    /// group was removed does not bring it back. A list asked for after
    /// either is the reconciler again.
    #[wasm_bindgen_test]
    fn a_list_older_than_a_group_change_does_not_undo_it() {
        let mut store = store();
        let began = store.chat_changes;
        store.upsert_chat(group(77, &[ANNA, ME], ANNA));
        let gone = store.merge_chats_from(vec![chat(42, "family")], began);
        assert!(gone.is_empty());
        assert!(store.item(77).is_some(), "made while the list was out");

        let began = store.chat_changes;
        let mut renamed = group(77, &[ANNA, ME], ANNA);
        renamed.title = Some("The cousins".into());
        store.upsert_chat(renamed);
        let mut stale = chat(77, "group");
        stale.chat = group(77, &[ANNA, ME], ANNA);
        stale.unread_count = 3;
        stale.last_message = Some(Message {
            chat_id: 77,
            ..message(800, ANNA, "see you")
        });
        store.merge_chats_from(vec![chat(42, "family"), stale.clone()], began);
        let held = store.item(77).expect("kept");
        assert_eq!(
            held.chat.title.as_deref(),
            Some("The cousins"),
            "the newer chat"
        );
        assert_eq!(held.unread_count, 3, "the list's counts");

        let began = store.chat_changes;
        store.remove_chat(77);
        store.merge_chats_from(vec![chat(42, "family"), stale.clone()], began);
        assert!(store.item(77).is_none(), "removed while the list was out");

        // Asked for after the change: the list decides.
        store.merge_chats(vec![chat(42, "family"), stale]);
        assert!(store.item(77).is_some());
        store.merge_chats(vec![chat(42, "family")]);
        assert!(store.item(77).is_none());
    }

    /// Leaving the family takes every group too: a rejoin does not put a
    /// member back in one (docs/protocol.md, `POST /families/leave`) — and,
    /// since the assistant's chat is per family, that family's `ai` chat
    /// with it (docs/protocol.md, "Several families").
    #[wasm_bindgen_test]
    fn leaving_the_family_takes_its_groups() {
        let mut store = store();
        store.apply_me(&account(Some(3), false));
        store.chats.push(chat(50, "ai"));
        store.upsert_chat(group(77, &[ANNA, ME], ME));
        store.drafts.insert(77, "half".into());
        store.forget_family(3);
        assert!(store.item(77).is_none() && store.item(50).is_none());
        assert!(store.item(42).is_none(), "its family chat");
        assert!(!store.drafts.contains_key(&77));
        assert!(store.family.is_none() && store.families.is_empty());
    }

    /// The roster is fresher than `/me` about this account's own role — and
    /// than a `family_owner` frame missed while the socket was down.
    #[wasm_bindgen_test]
    fn the_roster_says_who_owns_the_family() {
        let mut store = store();
        store.apply_me(&account(Some(3), false));
        assert!(!store.is_owner());
        let before = store.roster_changes;
        let roster = Roster {
            members: vec![Member {
                id: ME,
                role: Some("owner".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        store.apply_roster(&roster);
        assert!(store.is_owner());
        assert_eq!(store.roster_changes, before + 1, "a changed family counts");
        store.apply_roster(&roster);
        assert_eq!(store.roster_changes, before + 1, "the same family does not");
    }

    fn sticker_draft(source: i64) -> Draft {
        Draft {
            attachments: vec![Prepared {
                kind: "photo".into(),
                mime: "image/webp".into(),
                size: 18_234,
                width: Some(512),
                height: Some(512),
                file: Some(web_sys::Blob::new().expect("a blob")),
                source_attachment_id: Some(source),
                ..Prepared::default()
            }],
            sticker: true,
            ..Draft::default()
        }
    }

    /// A sticker is a message like any other in the outbox — one row, one
    /// bubble, drawn at once — and its bubble is already the sticker: the
    /// flag is on the pending attachment, not waiting for the server.
    #[wasm_bindgen_test]
    fn a_queued_sticker_draws_as_one_at_once() {
        let mut store = store();
        store.queue_send(42, "s".into(), sticker_draft(71));
        let row = &store.outbox[0];
        assert!(row.sticker && row.body.is_empty() && row.items.len() == 1);
        assert_eq!(row.items[0].source_attachment_id, Some(71));
        assert!(!row.items[0].has_preview, "a sticker has no preview");
        let pending = &store.threads[&42].messages[0];
        let drawn = pending
            .sticker()
            .expect("the pending bubble is the sticker");
        assert_eq!(drawn.id, row.items[0].provisional_id);
        assert_eq!(drawn.mime.as_deref(), Some("image/webp"));
        // An ordinary photo beside it is not one.
        store.queue_send(
            42,
            "p".into(),
            Draft {
                attachments: vec![Prepared {
                    kind: "photo".into(),
                    mime: "image/jpeg".into(),
                    file: Some(web_sys::Blob::new().expect("a blob")),
                    ..Prepared::default()
                }],
                ..Draft::default()
            },
        );
        assert!(!store.outbox[1].sticker);
        assert!(store.threads[&42].messages[1].sticker().is_none());
    }

    /// A reload keeps no bytes — and a queued sticker goes all the same:
    /// its bytes are the pack's, and the upload fetches them again. The row
    /// comes back queued, flag and all; a photo beside it fails as before.
    #[wasm_bindgen_test]
    fn a_queued_sticker_survives_a_reload() {
        let mut before = store();
        before.queue_send(42, "s".into(), sticker_draft(71));
        let kept = serde_json::to_string(&before.outbox).expect("the outbox is kept as JSON");
        let rows: Vec<Outgoing> = serde_json::from_str(&kept).expect("and read back");

        let mut after = store();
        after.restore_outbox(rows);
        assert!(after.failed_sends(42).is_empty(), "still on its way");
        let row = after.next_to_send().expect("queued");
        assert!(row.sticker, "the flag rides on the row");
        assert_eq!(row.items[0].source_attachment_id, Some(71));
        assert!(after.bytes.is_empty(), "the bytes did not come back");
        assert!(after.threads[&42].messages[0].sticker().is_some());
        // Swept before the message claimed it: it can go up again, from
        // the pack, with no bytes held.
        let provisional = row.items[0].provisional_id;
        after.landed("s", provisional, 501);
        assert!(after.expire_uploads("s"));
        assert_eq!(after.outbox[0].items[0].attachment_id, None);

        // A row a build from before stickers kept reads as an ordinary one.
        let old: Outgoing = serde_json::from_str(
            r#"{"chat_id": 42, "client_msg_id": "old", "body": "hi", "reply_to_message_id": null,
                "mentions": [], "poll": null, "attempts": 0, "failed": null,
                "items": [{"provisional_id": -1, "kind": "photo", "mime": "image/jpeg", "size": 3,
                           "width": null, "height": null, "duration_ms": null, "name": null,
                           "latitude": null, "longitude": null, "accuracy_m": null,
                           "has_preview": false, "attachment_id": null}]}"#,
        )
        .expect("an older row reads");
        assert!(!old.sticker);
        assert_eq!(old.items[0].source_attachment_id, None);
    }

    fn round_draft() -> Draft {
        Draft {
            attachments: vec![Prepared {
                kind: "video".into(),
                mime: "video/mp4".into(),
                size: 1_649_700,
                width: Some(480),
                height: Some(480),
                duration_ms: Some(23_400),
                file: Some(web_sys::Blob::new().expect("a blob")),
                preview: Some(web_sys::Blob::new().expect("a poster")),
                ..Prepared::default()
            }],
            round: true,
            ..Draft::default()
        }
    }

    /// A VIDEO MESSAGE in the outbox is one row and one bubble, drawn at
    /// once — and its bubble is already the circle, from this tab's own
    /// poster (the plan for #79, S5.6). The flag rides on the row through a
    /// reload, like a sticker's; unlike a sticker's, its bytes are this
    /// tab's alone, so after a reload it fails as any video does.
    #[wasm_bindgen_test]
    fn a_queued_video_message_draws_round_at_once() {
        let mut queued = store();
        queued.queue_send(42, "v".into(), round_draft());
        let row = &queued.outbox[0];
        assert!(row.round && !row.sticker && row.body.is_empty());
        let pending = &queued.threads[&42].messages[0];
        let drawn = pending
            .round_video()
            .expect("the pending bubble is the circle");
        assert_eq!(drawn.id, row.items[0].provisional_id);
        assert!(drawn.has_preview, "drawn from the local poster");
        assert!(!drawn.sticker);
        // An ordinary video beside it is not one.
        let mut plain = round_draft();
        plain.round = false;
        queued.queue_send(42, "p".into(), plain);
        assert!(!queued.outbox[1].round);
        assert!(queued.threads[&42].messages[1].round_video().is_none());
        let kept = serde_json::to_value(&queued.outbox[1]).expect("encodes");
        assert!(kept.get("round").is_none(), "written only when true");

        let kept = serde_json::to_string(&queued.outbox).expect("the outbox is kept as JSON");
        let rows: Vec<Outgoing> = serde_json::from_str(&kept).expect("and read back");
        let mut after = store();
        after.restore_outbox(rows);
        assert!(after.outbox[0].round, "the flag rides on the row");
        assert!(
            after.threads[&42].messages[0].round_video().is_some(),
            "and the bubble is still round"
        );
        assert_eq!(
            after.failed_sends(42).len(),
            2,
            "a video's bytes do not survive a reload"
        );
    }

    fn animation_draft(body: &str) -> Draft {
        Draft {
            body: body.into(),
            attachments: vec![Prepared {
                kind: "video".into(),
                mime: "video/mp4".into(),
                size: 182_330,
                width: Some(480),
                height: Some(270),
                duration_ms: Some(2_100),
                file: Some(web_sys::Blob::new().expect("a blob")),
                preview: Some(web_sys::Blob::new().expect("a poster")),
                animation: true,
                ..Prepared::default()
            }],
            ..Draft::default()
        }
    }

    /// AN ANIMATION in the outbox is one row carrying the flag, and its
    /// pending bubble is already the animation — caption or not
    /// (docs/protocol.md, "Animations"). Beside another attachment it is the
    /// ordinary video it otherwise is: the flag is one attachment's or
    /// nobody's. The flag rides on the row through a reload.
    #[wasm_bindgen_test]
    fn a_queued_animation_carries_its_flag_alone_and_draws_as_one() {
        let mut queued = store();
        queued.queue_send(42, "a".into(), animation_draft(""));
        queued.queue_send(42, "c".into(), animation_draft("look"));
        assert!(queued.outbox[0].animation && !queued.outbox[0].round);
        assert!(queued.outbox[1].animation, "a caption is allowed");
        let drawn = queued.threads[&42].messages[0]
            .animation()
            .expect("the pending bubble is the animation");
        assert_eq!(drawn.id, queued.outbox[0].items[0].provisional_id);
        assert!(drawn.has_preview && !drawn.round && !drawn.sticker);
        assert!(queued.threads[&42].messages[1].animation().is_some());

        let mut two = animation_draft("");
        two.attachments.push(two.attachments[0].clone());
        queued.queue_send(42, "two".into(), two);
        assert!(!queued.outbox[2].animation, "never on two attachments");
        assert!(queued.threads[&42].messages[2].animation().is_none());
        let kept = serde_json::to_value(&queued.outbox[2]).expect("encodes");
        assert!(kept.get("animation").is_none(), "written only when true");
        let mut still = animation_draft("");
        still.attachments[0].animation = false;
        assert!(!sends_as_animation(&still));
        let mut round = animation_draft("");
        round.round = true;
        assert!(!sends_as_animation(&round), "never beside round");

        let kept = serde_json::to_string(&queued.outbox).expect("the outbox is kept as JSON");
        let rows: Vec<Outgoing> = serde_json::from_str(&kept).expect("and read back");
        let mut after = store();
        after.restore_outbox(rows);
        assert!(after.outbox[0].animation, "the flag rides on the row");
        assert!(after.threads[&42].messages[0].animation().is_some());
    }

    /// The pack's limits ride on the roster, and are the capability: with
    /// them there are stickers, without them — a server from before packs —
    /// there are none. And the pack is the FAMILY's: leaving takes it, the
    /// way it takes the wall.
    /// A server with video messages says both keys; the store keeps them
    /// as one pair, and a server that sends neither — or only one — offers
    /// none (docs/protocol.md, "Video messages").
    #[wasm_bindgen_test]
    fn the_roster_says_how_long_and_big_a_video_message_may_be() {
        let mut store = Store::default();
        store.apply_roster(&Roster {
            max_round_video_ms: Some(60_000),
            max_round_video_bytes: Some(12_582_912),
            ..Roster::default()
        });
        assert_eq!(
            store.round,
            Some(crate::round_video::RoundLimits {
                max_ms: 60_000,
                max_bytes: 12_582_912
            })
        );
        store.apply_roster(&Roster {
            max_round_video_ms: Some(60_000),
            ..Roster::default()
        });
        assert_eq!(store.round, None, "one key is not a server that has them");
        store.apply_roster(&Roster::default());
        assert_eq!(store.round, None, "an older server");
    }

    #[wasm_bindgen_test]
    fn the_roster_offers_the_pack_and_leaving_takes_it() {
        let mut store = store();
        store.apply_me(&account(Some(3), false));
        store.apply_roster(&Roster::default());
        assert!(!store.pack.is_offered(), "an older server offers none");
        store.apply_roster(&Roster {
            max_pack_items: Some(200),
            max_pack_item_bytes: Some(524_288),
            ..Roster::default()
        });
        assert_eq!(
            store.pack.limits,
            Some(crate::pack::Limits {
                items: 200,
                bytes: 524_288
            })
        );
        store.pack.apply_full(
            vec![crate::model::PackItem {
                id: 5,
                pack_seq: 12,
                added_by: Some(ANNA),
                attachment: Some(crate::model::Attachment {
                    id: 71,
                    kind: "photo".into(),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            12,
        );
        // A BLOCK hides nothing of the pack: an item is a picture the
        // family keeps, not something a person said.
        store.set_blocked(ANNA, true);
        assert_eq!(store.pack.listed().len(), 1);
        assert_eq!(store.pack.panel().len(), 1);

        store.apply_me(&account(Some(4), false));
        assert!(store.pack.items.is_empty() && !store.pack.loaded);
        assert_eq!(store.pack.cursor, 0, "the next read is a whole one");
        assert!(
            !store.pack.is_offered(),
            "until the new family's roster says"
        );
    }

    /// The lookup consent stands only on the assistant consent: granting
    /// that one again leaves it alone, withdrawing it takes this one with
    /// it — as the server does in the same write — and the lookup answer
    /// on its own touches nothing else.
    #[wasm_bindgen_test]
    fn withdrawing_the_assistant_withdraws_the_lookups() {
        let mut store = Store {
            account: Some(Me::default()),
            ..Store::default()
        };
        store.set_assistant_consent(Some("2026-10-03T09:00:00Z".into()));
        store.set_assistant_lookup_consent(Some("2026-10-03T09:30:00Z".into()));
        assert_eq!(
            store.assistant_lookup_consent_at(),
            Some("2026-10-03T09:30:00Z")
        );
        store.set_assistant_consent(Some("2026-10-03T09:00:00Z".into()));
        assert!(store.assistant_lookup_consent_at().is_some());
        store.set_assistant_lookup_consent(None);
        assert!(store.assistant_lookup_consent_at().is_none());
        assert!(store.assistant_consent_at().is_some(), "only its own");
        store.set_assistant_lookup_consent(Some("2026-10-03T09:40:00Z".into()));
        store.set_assistant_consent(None);
        assert!(store.assistant_consent_at().is_none());
        assert!(store.assistant_lookup_consent_at().is_none());
    }

    // --- several families (#88; docs/protocol.md, "Several families") -----

    fn fam(id: i64, name: &str, role: &str) -> crate::model::Family {
        crate::model::Family {
            id,
            name: name.into(),
            role: Some(role.into()),
            ..Default::default()
        }
    }

    /// `/me` from a server that knows several families: the list, home
    /// first, each with this account's own role.
    fn several(families: &[(i64, &str, &str)]) -> Me {
        let listed: Vec<crate::model::Family> = families
            .iter()
            .map(|(id, name, role)| fam(*id, name, role))
            .collect();
        Me {
            user: User {
                id: ME,
                username: "me".into(),
                display_name: "Me".into(),
                ..Default::default()
            },
            family: listed.first().cloned(),
            role: families.first().map(|(_, _, role)| role.to_string()),
            families: listed,
            member_families_enabled: true,
            max_families_per_account: Some(5),
            ..Default::default()
        }
    }

    fn listed_in(id: i64, kind: &str, family_ids: &[i64]) -> ChatListItem {
        let mut item = chat(id, kind);
        item.chat.family_ids = Some(family_ids.to_vec());
        item
    }

    fn member(id: i64, name: &str) -> Member {
        Member {
            id,
            display_name: name.into(),
            role: Some("member".into()),
            ..Default::default()
        }
    }

    /// The model is a LIST: the home family is shown first, the role read
    /// is THAT family's, and a server from before several families is one
    /// family with the role it names.
    #[wasm_bindgen_test]
    fn the_account_is_in_a_list_of_families_and_shows_one() {
        let mut store = Store::default();
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        assert_eq!(store.current_family_id(), Some(3), "the home family");
        assert_eq!(store.families.len(), 2);
        assert!(!store.is_owner(), "a member of the family on screen");
        assert!(store.owns_a_family(), "but the owner of another");
        assert_eq!(store.family_name(4).as_deref(), Some("Ours"));

        let mut old = Store::default();
        let mut me = account(Some(3), false);
        me.role = Some("owner".into());
        old.apply_me(&me);
        assert_eq!(old.families.len(), 1, "a server that predates the list");
        assert!(old.is_owner());
        assert_eq!(old.families[0].role.as_deref(), Some("owner"));
    }

    /// The family on screen stays while it is listed, a reload opens the one
    /// this device showed last, and when it goes the first left is shown.
    #[wasm_bindgen_test]
    fn the_family_on_screen_stays_until_it_is_gone() {
        let mut store = Store {
            preferred_family: Some(4),
            ..Store::default()
        };
        let both = several(&[(3, "Parents", "member"), (4, "Ours", "owner")]);
        store.apply_me(&both);
        assert_eq!(store.current_family_id(), Some(4), "shown here last");
        assert!(store.is_owner(), "this account's role in THAT family");
        assert!(store.switch_family(3));
        assert!(!store.apply_me(&both), "the same family is no change");
        assert_eq!(store.current_family_id(), Some(3));
        assert!(
            store.apply_me(&several(&[(4, "Ours", "owner")])),
            "the family on screen went: a change"
        );
        assert_eq!(store.current_family_id(), Some(4));
        assert!(store.apply_me(&several(&[])), "none left");
        assert!(store.family.is_none() && store.account.as_ref().unwrap().family.is_none());
        assert!(!store.switch_family(3), "not a family this account is in");
    }

    /// The chat list is the family on screen's: its family chat first — THAT
    /// family's, not the other's — and a direct chat in every family it
    /// names. Unread counts per family count a direct chat in each.
    #[wasm_bindgen_test]
    fn the_list_shows_the_chats_of_the_family_on_screen() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        let mut parents = listed_in(42, "family", &[3]);
        parents.unread_count = 2;
        let mut ours = listed_in(43, "family", &[4]);
        ours.unread_count = 5;
        let mut anna = listed_in(60, "direct", &[3, 4]);
        anna.unread_count = 1;
        store.chats = vec![
            ours,
            parents,
            anna,
            listed_in(50, "ai", &[3]),
            listed_in(51, "ai", &[4]),
        ];
        let ids = |store: &Store| -> Vec<i64> {
            store
                .sorted_chats()
                .iter()
                .map(|item| item.chat.id)
                .collect()
        };
        let shown = ids(&store);
        assert_eq!(shown[0], 42, "the family chat of the family on screen");
        assert!(shown.contains(&60) && shown.contains(&50));
        assert!(!shown.contains(&43) && !shown.contains(&51));
        store.switch_family(4);
        let shown = ids(&store);
        assert_eq!(shown[0], 43);
        assert!(shown.contains(&60) && shown.contains(&51) && !shown.contains(&50));
        assert_eq!(store.unread_in(3), 3, "the family chat and the direct chat");
        assert_eq!(store.unread_in(4), 6, "the direct chat counts in both");
        assert_eq!(
            store.unread_total(),
            8,
            "the tab's title counts each chat once"
        );
        assert_eq!(
            store.family_of_chat(42),
            Some(3),
            "not listed here: its own"
        );
        assert_eq!(store.family_of_chat(60), Some(4), "listed here: here");
    }

    /// Switching keeps each family's roster, board and pack where they were:
    /// what was on screen waits away, and comes back as it was.
    #[wasm_bindgen_test]
    fn switching_keeps_every_familys_roster_board_and_pack() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        store.members = vec![member(ME, "Me"), member(ANNA, "Anna")];
        store.board.apply_full(
            vec![crate::model::Note {
                id: 1,
                board_seq: 10,
                author_id: Some(ANNA),
                text: Some("Milk".into()),
                color: Some("yellow".into()),
                x: Some(0.1),
                y: Some(0.1),
                family_id: Some(3),
                ..Default::default()
            }],
            10,
        );
        store.switch_family(4);
        assert!(store.members.is_empty() && store.board.notes.is_empty());
        assert_eq!(store.board_of(3).map(|board| board.notes.len()), Some(1));
        store.members = vec![member(ME, "Me"), member(12, "Cleo")];
        store.switch_family(3);
        assert_eq!(store.members.len(), 2);
        assert_eq!(store.members[1].display_name, "Anna");
        assert_eq!(store.board.notes.len(), 1);
        assert_eq!(store.roster_of(4).len(), 2);
    }

    /// A frame for another family is applied to THAT family — the one on
    /// screen is left as it was — and its owner's change is that family's.
    #[wasm_bindgen_test]
    fn a_frame_is_applied_to_the_family_it_names() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "member")]));
        store.members = vec![member(ME, "Me")];
        let joined = store.with_family(4, |store| {
            store.member_joined(
                &User {
                    id: 12,
                    display_name: "Cleo".into(),
                    ..Default::default()
                },
                None,
            )
        });
        assert!(joined.is_some());
        assert_eq!(store.members.len(), 1, "not the family on screen");
        assert_eq!(store.roster_of(4), vec![(12, "Cleo".to_string())]);
        assert!(store.with_family(9, |_| ()).is_none(), "a family not held");
        store.with_family(4, |store| store.family_owner(ME));
        assert!(!store.is_owner(), "the family on screen is unchanged");
        assert!(store.families[1].is_owner(), "the owner of THAT family");
        store.switch_family(4);
        assert!(store.is_owner());
        assert!(store.account.as_ref().unwrap().is_owner());
    }

    /// `member_left` naming ME: out of that family only — its chats go, a
    /// direct chat stays while it names another family held (and is no
    /// longer listed in the one gone), and the family on screen falls back
    /// to the first left.
    #[wasm_bindgen_test]
    fn leaving_one_family_keeps_the_rest() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        store.switch_family(4);
        store.chats = vec![
            listed_in(42, "family", &[3]),
            listed_in(43, "family", &[4]),
            listed_in(60, "direct", &[3, 4]),
            listed_in(61, "direct", &[4]),
            listed_in(51, "ai", &[4]),
        ];
        store.drafts.insert(61, "half".into());
        assert!(store.leave_family(4), "it was on screen");
        assert_eq!(store.current_family_id(), Some(3));
        assert!(store.item(43).is_none() && store.item(61).is_none() && store.item(51).is_none());
        assert!(!store.drafts.contains_key(&61));
        assert_eq!(
            store.item(60).and_then(|item| item.chat.family_ids.clone()),
            Some(vec![3]),
            "the direct chat stays, listed where it still is"
        );
        assert!(store.item(42).is_some());
        assert!(!store.away.contains_key(&4));
        assert!(!store.leave_family(9), "a family not held changes nothing");
        assert!(store.leave_family(3));
        assert!(store.family.is_none() && store.chats.is_empty(), "the gate");
    }

    /// Invitations: mine onto my list (and the account's, which the gate
    /// draws), an owner's onto THAT family's, never doubled — and a removal
    /// takes it off every list.
    #[wasm_bindgen_test]
    fn invitations_land_on_the_right_list_and_go_from_all() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        let invitation = |id: i64, family_id: i64, user: i64| Invitation {
            id,
            family_id,
            family_name: "Ivanovs".into(),
            invited_by: User {
                id: ANNA,
                display_name: "Anna".into(),
                ..Default::default()
            },
            user: User {
                id: user,
                display_name: "Somebody".into(),
                ..Default::default()
            },
            created_at: None,
        };
        store.apply_invitation(invitation(1, 9, ME));
        store.apply_invitation(invitation(1, 9, ME));
        assert_eq!(store.invitations.len(), 1, "never doubled");
        assert_eq!(store.account.as_ref().unwrap().invitations.len(), 1);
        store.apply_invitation(invitation(2, 4, 12));
        assert!(
            store.sent_invitations.is_empty(),
            "not the family on screen's"
        );
        assert_eq!(store.away[&4].sent_invitations.len(), 1);
        store.remove_invitation(1);
        store.remove_invitation(2);
        assert!(store.invitations.is_empty());
        assert!(store.account.as_ref().unwrap().invitations.is_empty());
        assert!(store.away[&4].sent_invitations.is_empty());
    }

    /// Whom the owner may invite: the members of their OTHER families who
    /// are not in this one.
    #[wasm_bindgen_test]
    fn the_owner_invites_from_their_other_families() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        store.members = vec![member(ME, "Me"), member(ANNA, "Anna"), member(11, "Bob")];
        store.switch_family(4);
        store.members = vec![member(ME, "Me"), member(11, "Bob")];
        assert_eq!(store.invitable(), vec![(ANNA, "Anna".to_string())]);
    }

    /// A note this device's change was answered with goes on the board it
    /// is on — even after a switch.
    #[wasm_bindgen_test]
    fn an_answered_note_lands_on_its_own_board() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        store.board.apply_full(Vec::new(), 0);
        store.switch_family(4);
        store.apply_note(
            Some(3),
            crate::model::Note {
                id: 1,
                board_seq: 1,
                author_id: Some(ME),
                text: Some("Milk".into()),
                color: Some("yellow".into()),
                x: Some(0.1),
                y: Some(0.1),
                ..Default::default()
            },
        );
        assert!(store.board.notes.is_empty());
        assert_eq!(store.board_of(3).map(|board| board.notes.len()), Some(1));
    }

    // --- paid families (#89; docs/protocol.md, "Paid families") ----------

    /// A `family_plan` frame sets the deadline of the family it NAMES — on
    /// screen or away — and one without `delete_at` clears it, so the banner
    /// comes and goes with it.
    #[wasm_bindgen_test]
    fn a_plan_frame_sets_and_clears_the_named_familys_deadline() {
        let mut store = Store {
            my_user_id: ME,
            ..Store::default()
        };
        store.apply_me(&several(&[(3, "Parents", "member"), (4, "Ours", "owner")]));
        assert_eq!(store.current_family_id(), Some(3));

        store.apply_plan(4, Some("2026-11-14T09:00:00Z".into()));
        let deadline = |store: &Store, id: i64| {
            store
                .family_list()
                .into_iter()
                .find(|family| family.id == id)
                .and_then(|family| family.delete_at)
        };
        assert_eq!(deadline(&store, 4).as_deref(), Some("2026-11-14T09:00:00Z"));
        assert_eq!(deadline(&store, 3), None, "only the family it names");
        assert_eq!(
            store.family.as_ref().and_then(|f| f.delete_at.clone()),
            None
        );

        // Shown, it is the family on screen that carries it — and the
        // account's view of it.
        store.switch_family(4);
        assert_eq!(
            store.family.as_ref().and_then(|f| f.delete_at.as_deref()),
            Some("2026-11-14T09:00:00Z")
        );
        store.apply_plan(4, None);
        assert_eq!(
            store.family.as_ref().and_then(|f| f.delete_at.clone()),
            None
        );
        assert_eq!(deadline(&store, 4), None, "renewed: no deadline anywhere");
        assert_eq!(
            store
                .account
                .as_ref()
                .and_then(|me| me.family.as_ref())
                .and_then(|f| f.delete_at.clone()),
            None
        );

        store.apply_plan(4, Some("2026-11-20T09:00:00Z".into()));
        assert_eq!(
            store
                .account
                .as_ref()
                .and_then(|me| me.family.as_ref())
                .and_then(|f| f.delete_at.as_deref()),
            Some("2026-11-20T09:00:00Z")
        );
        // A family this account is not in changes nothing.
        store.apply_plan(9, Some("2026-11-20T09:00:00Z".into()));
        assert_eq!(store.families.len(), 2);
    }

    /// `/me` and the family reads carry the new keys, and a server from
    /// before paid families sends none of them.
    #[wasm_bindgen_test]
    fn the_paid_keys_are_read_and_absent_is_the_old_behaviour() {
        let me: Me = serde_json::from_str(
            r#"{"user": {"id": 7, "username": "me", "display_name": "Me"},
                "families": [{"id": 3, "name": "Ivanovs", "role": "owner", "plan": "paid",
                              "member_ceiling": 25, "delete_at": "2026-11-14T09:00:00Z"}],
                "subscriptions_enabled": true,
                "subscription_products": ["me.nettrash.FamilyConnect.family.monthly"],
                "app_account_token": "6f1c0000-0000-4000-8000-000000000000",
                "unpaid_days": 14, "paid_family_limits": {"members": 25},
                "subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly",
                                 "environment": "Sandbox", "expires_at": "2026-11-01T00:00:00Z",
                                 "active": true, "renews": false}}"#,
        )
        .expect("reads");
        assert!(me.subscriptions_enabled);
        assert!(me.may_start_family(), "an active subscription");
        assert_eq!(me.families[0].member_ceiling, Some(25));
        assert_eq!(
            me.families[0].delete_at.as_deref(),
            Some("2026-11-14T09:00:00Z")
        );

        let ended: Me = serde_json::from_str(
            r#"{"user": {"id": 7, "username": "me", "display_name": "Me"},
                "subscriptions_enabled": true,
                "subscription": {"product_id": "x", "environment": "Production",
                                 "expires_at": "2026-10-01T00:00:00Z", "active": false}}"#,
        )
        .expect("reads");
        assert!(!ended.may_start_family(), "one that has ended");
        let never: Me = serde_json::from_str(
            r#"{"user": {"id": 7, "username": "me", "display_name": "Me"},
                "subscriptions_enabled": true, "subscription": null}"#,
        )
        .expect("reads");
        assert!(!never.may_start_family(), "none at all");

        let before: Me = serde_json::from_str(
            r#"{"user": {"id": 7, "username": "me", "display_name": "Me"},
                "families": [{"id": 3, "name": "Ivanovs", "role": "owner"}]}"#,
        )
        .expect("reads");
        assert!(!before.subscriptions_enabled);
        assert!(before.may_start_family(), "a server that does not charge");
        assert_eq!(before.families[0].member_ceiling, None);
        assert_eq!(before.families[0].delete_at, None);
    }
}

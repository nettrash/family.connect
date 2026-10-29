//! The chats and what was said in them, as this device holds them — a port of `ChatStore.cs`.
//!
//! Three protocol rules shape every write here. A MESSAGE IS IMMUTABLE except through two guarded
//! paths — an edit under `edit_seq` and reactions under `reaction_seq` — and both apply the WHOLE
//! message rather than the field they seem to be about, because the assistant's picture answer
//! arrives as an attachment added by an edit and a body-only merge draws nothing. The READ MARKER
//! is monotonic (`max(stored, received)`): a response still in flight while the reader is reading
//! must never walk it backwards. And the three `max_*_seq` cursors are HIGH-WATER MARKS that never
//! go down, so a chat whose polls retention has swept still reports one.
//!
//! Apple counterpart: the SwiftData stores behind `ChatSyncCoordinator`. Android: `ChatRepository`
//! + the DAOs.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

use rusqlite::{Connection, OptionalExtension, Row, named_params};

use super::database::{Database, StoreResult};
use super::times;
use crate::protocol::{
    AttachmentDto, CallRecordDto, ChatDto, ChatRowDto, MemberDto, MentionDto, MessageDto, PollDto,
    ReactionDto, ReplyToDto, UserDto, wire,
};

/// HOW a reaction, a poll state, a message, a note or a pack item reached this client, which
/// decides whether it may move a catch-up cursor (docs/protocol.md, "Best-effort delivery").
///
/// **Only a live frame and a catch-up page may move a chat cursor.** A state that arrives by any
/// other route — embedded on a fetched `Message`, or in the answer to this client's own reaction,
/// vote or close — is EVIDENCE about one message and is applied under the per-message guard alone.
/// Advancing the cursor from evidence would step the chat past states for messages this device has
/// never seen, and `after_seq` can never look back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SeqRoute {
    /// A frame off the socket.
    LiveFrame,

    /// A page of a catch-up — `…/messages?after_id=`, `…/reactions?after_seq=` or
    /// `…/polls?after_seq=` — or the answer to this device's own send: a new reply on it raises
    /// its root's count.
    CatchUpPage,

    /// Anything else: a page of history, an edit, a thread read, a fetched message, or the answer
    /// to our own reaction or vote. It moves no cursor and counts nothing.
    Evidence,
}

/// How many messages a page of [`ChatStore::messages`] holds when the caller has no better number
/// (the C#'s default `limit`).
pub const PAGE: usize = 50;

/// The chats and what was said in them, as this device holds them.
pub struct ChatStore {
    database: Rc<Database>,
    /// Who is reading. A message of THEIR OWN never counts as unread, and there is no other
    /// question the store asks about them — one cache belongs to one account, because a sign-out
    /// wipes it (see [`Database::wipe_all`]).
    reader: Box<dyn Fn() -> i64>,
    /// The family being shown (docs/protocol.md, "Several families"); `None` while there is none
    /// to say.
    family: Box<dyn Fn() -> Option<i64>>,
    /// The account's HOME family, as the session says now — the one a chat with no `family_ids` is
    /// listed in (cached before them, or from a server that predates them); `None` while there is
    /// none to say.
    home: Box<dyn Fn() -> Option<i64>>,
    kept: RefCell<Kept>,
}

impl fmt::Debug for ChatStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatStore")
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

// THE ROSTER, THE BLOCK LIST AND EACH CHAT'S KIND, kept rather than asked for. A conversation asks
// who wrote each bubble and whether they are blocked once per bubble, and a redraw of a long chat
// was hundreds of queries. These are small tables that change a few times a day, and every write
// to them goes through this type, which bumps the database's generation as it starts
// (`Database::roster_generation` / `chats_generation` — on the DATABASE, so a second store on the
// same file, or a wipe, makes this copy stale too). Everything here is read and written inside an
// operation, so a reader can never keep a copy older than a write it raced: on one thread the
// write has finished, and its bump has landed, before the next read can look.
#[derive(Debug)]
struct Kept {
    roster_at: i64,
    roster_family: Option<i64>,
    roster: Option<Rc<[MemberDto]>>,
    roster_by_id: Option<HashMap<i64, MemberDto>>,
    directory_by_id: Option<HashMap<i64, MemberDto>>,
    blocked: Option<Rc<[i64]>>,
    blocked_set: Option<HashSet<i64>>,
    kinds_at: i64,
    kinds: HashMap<i64, String>,
}

impl Default for Kept {
    fn default() -> Self {
        Kept {
            roster_at: -1,
            roster_family: None,
            roster: None,
            roster_by_id: None,
            directory_by_id: None,
            blocked: None,
            blocked_set: None,
            kinds_at: -1,
            kinds: HashMap::new(),
        }
    }
}

impl ChatStore {
    /// A store over `database` read by nobody in particular (id 0), showing no family and with no
    /// home family named — `new ChatStore(database)`.
    pub fn new(database: &Rc<Database>) -> Self {
        ChatStore {
            database: Rc::clone(database),
            reader: Box::new(|| 0),
            family: Box::new(|| None),
            home: Box::new(|| None),
            kept: RefCell::new(Kept::default()),
        }
    }

    /// The same store, read by whoever `me` says (the C#'s `me`).
    pub fn with_me(mut self, me: impl Fn() -> i64 + 'static) -> Self {
        self.reader = Box::new(me);
        self
    }

    /// The same store, showing the family `current` says (the C#'s `current`).
    pub fn with_current(mut self, current: impl Fn() -> Option<i64> + 'static) -> Self {
        self.family = Box::new(current);
        self
    }

    /// The same store, with the home family `home_family` says (the C#'s `homeFamily`).
    pub fn with_home_family(mut self, home_family: impl Fn() -> Option<i64> + 'static) -> Self {
        self.home = Box::new(home_family);
        self
    }

    /// Who is reading, for the callers that also have to know — the frame router asks the store
    /// rather than being told a second time, because two answers to "who am I" is one too many.
    pub fn reader(&self) -> i64 {
        (self.reader)()
    }

    /// The family being shown, as the session says now — or `None`, which reads and writes the
    /// roster the way a cache from before several families held it: one family, on `members`
    /// alone.
    pub fn family(&self) -> Option<i64> {
        (self.family)()
    }

    /// The account's home family, as the session says now — or `None` (see [`ChatStore::family`]).
    pub fn home(&self) -> Option<i64> {
        (self.home)()
    }

    fn connection(&self) -> &Connection {
        self.database.connection()
    }

    // ---- the list ------------------------------------------------------------------------------

    /// What `GET /chats` answered IS the list — but a chat it does not carry is HIDDEN, not
    /// forgotten.
    ///
    /// A direct chat with somebody the reader has blocked is not listed, for the blocker alone,
    /// and **comes back whole on unblock — nothing about it is deleted** (docs/protocol.md,
    /// "Blocking a member"). Deleting the row would take its messages with it, so an unblock would
    /// hand the reader an empty chat where a conversation used to be.
    ///
    /// **A LIST READ OLDER THAN A CHANGE DOES NOT UNDO IT.** `since` is the
    /// [`list_mark`](ChatStore::list_mark) taken when the request STARTED: a group created,
    /// renamed or re-membered after it ([`upsert`](ChatStore::upsert)) is not hidden for being
    /// missing from it, and keeps its newer name, members and admin while still taking the
    /// answer's counts and preview; a group removed after it ([`remove`](ChatStore::remove)) is not
    /// written back. A later request decides normally. Apple, Android and the web keep the same
    /// guard. `None` — no mark — applies the answer whole.
    pub fn replace(&self, rows: &[ChatRowDto], since: Option<i64>) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.chats_changed();
        let transaction = self.connection().unchecked_transaction()?;
        let touched = since.map_or_else(HashSet::new, |mark| self.database.touched_since(mark));
        let keep: HashSet<i64> = rows
            .iter()
            .map(|row| row.chat.id)
            .chain(touched.iter().copied())
            .collect();
        for id in self.ids(&transaction)? {
            if !keep.contains(&id) {
                transaction
                    .prepare_cached("UPDATE chats SET listed = 0 WHERE chat_id = $id")?
                    .execute(named_params! { "$id": id })?;
            }
        }
        for listed in rows {
            let newer;
            let mut row = listed;
            if touched.contains(&listed.chat.id) {
                // Changed since this answer was taken: gone stays gone, and a newer chat keeps its
                // own fields.
                let Some(held) = self.held_chat(&transaction, listed.chat.id)? else {
                    continue;
                };
                newer = ChatRowDto {
                    chat: held,
                    ..listed.clone()
                };
                row = &newer;
            }
            self.write(&transaction, row)?;
            if let Some(preview) = &row.last_message {
                // A preview is a TRIMMED message: no reactions, no poll, no quote. It is stored so
                // the row can draw itself, and a fuller copy from a page overwrites it — which is
                // why the merge below is per-field and not a replace.
                self.apply_one(&transaction, preview, true, SeqRoute::CatchUpPage, true)?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The list: every chat the last list read carried, newest activity first, each with its
    /// newest message as the preview.
    pub fn chats(&self) -> StoreResult<Vec<ChatRowDto>> {
        let _held = self.database.hold();
        // Hidden chats are not on the list and count towards nothing: "there is nothing here for
        // a client to count" (docs/protocol.md, "Blocking a member").
        let mut rows = self
            .connection()
            .prepare_cached(
                "SELECT * FROM chats WHERE listed = 1 ORDER BY last_message_at DESC, chat_id",
            )?
            .query_map([], read_chat)?
            .collect::<Result<Vec<_>, _>>()?;
        for row in &mut rows {
            row.last_message = self.newest(row.chat.id)?;
        }
        Ok(rows)
    }

    /// One chat's row, listed or not: a blocked chat still has a read marker and cursors, and the
    /// thread it holds is still there to come back to.
    pub fn chat(&self, chat_id: i64) -> StoreResult<Option<ChatRowDto>> {
        let _held = self.database.hold();
        let row = self
            .connection()
            .prepare_cached("SELECT * FROM chats WHERE chat_id = $id")?
            .query_row(named_params! { "$id": chat_id }, read_chat)
            .optional()?;
        match row {
            Some(mut row) => {
                row.last_message = self.newest(chat_id)?;
                Ok(Some(row))
            }
            None => Ok(None),
        }
    }

    /// Whether this chat is on the list — false while the reader has its peer blocked.
    pub fn is_listed(&self, chat_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        let listed: Option<Option<i64>> = self
            .connection()
            .prepare_cached("SELECT listed FROM chats WHERE chat_id = $id")?
            .query_row(named_params! { "$id": chat_id }, |row| row.get(0))
            .optional()?;
        Ok(listed.flatten() == Some(1))
    }

    /// The mark a `GET /chats` request takes as it STARTS, handed back to
    /// [`replace`](ChatStore::replace) with its answer, so an answer older than a change to a chat
    /// cannot undo it.
    pub fn list_mark(&self) -> i64 {
        let _held = self.database.hold();
        self.database.chat_touches()
    }

    /// A chat as a `chat_updated` frame or a group endpoint's answer states it: its name, its
    /// members and its admin, written whole and put on the list (docs/protocol.md, "Groups"). A
    /// chat this device has never held starts with nothing unread, as the server starts a member
    /// just added; the counts, the marker and the cursors of one it holds are not touched — none of
    /// them is on a `Chat`, and the next list read brings the server's.
    ///
    /// A DIRECT CHAT WITH SOMEBODY THIS READER HAS BLOCKED STAYS OFF THE LIST. The server sends a
    /// direct chat's `chat_updated` without asking about blocks — a held chat moving family when
    /// the reader leaves one, say — and the list read and [`set_blocked`](ChatStore::set_blocked)
    /// both keep such a chat hidden, for the blocker alone (docs/protocol.md, "Blocking a
    /// member"); a frame relisting it would show it until the next `GET /chats`. The row is still
    /// written: it comes back whole, with whatever the frame said, on unblock.
    pub fn upsert(&self, chat: &ChatDto) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.chats_changed();
        let (members, admin, families) = group_parameters(chat);
        self.connection()
            .prepare_cached(
                "INSERT INTO chats (chat_id, kind, title, peer_user_id, member_ids_json, admin_user_id, family_ids_json, listed)
VALUES ($id, $kind, $title, $peer, $members, $admin, $families,
        CASE WHEN $kind = 'direct' AND $peer IN (SELECT user_id FROM blocked) THEN 0 ELSE 1 END)
ON CONFLICT(chat_id) DO UPDATE SET
    kind = excluded.kind, title = excluded.title, peer_user_id = excluded.peer_user_id,
    member_ids_json = excluded.member_ids_json, admin_user_id = excluded.admin_user_id,
    family_ids_json = excluded.family_ids_json,
    listed = excluded.listed",
            )?
            .execute(named_params! {
                "$id": chat.id,
                "$kind": chat.kind,
                "$title": chat.title,
                "$peer": chat.peer_user_id,
                "$members": members,
                "$admin": admin,
                "$families": families,
            })?;
        self.database.touched(chat.id);
        Ok(())
    }

    /// A GROUP this reader is no longer in — left, removed from, or deleted (docs/protocol.md,
    /// `chat_removed`): the chat, its messages and anything still queued for it go. Answers
    /// whether anything went.
    ///
    /// A GROUP ONLY. The family chat is untouched by everything groups added, and a direct or `ai`
    /// chat never leaves this way — so a frame naming one of those drops nothing. A list read that
    /// no longer carries a chat HIDES it ([`replace`](ChatStore::replace)), whatever its kind; this
    /// is the frame's stronger statement, that it is gone.
    ///
    /// Deleted rather than hidden, unlike a blocked direct chat, because nothing brings it back
    /// whole: a member the admin adds again is handed the group's history by the server, and reads
    /// it in as any chat this device never held. The outbox rows go too — a send to a chat its
    /// sender is no longer in can only be refused — and their staged bytes are the caller's to
    /// clean up, as a discarded row's are.
    pub fn remove(&self, chat_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        self.database.chats_changed();
        let transaction = self.connection().unchecked_transaction()?;
        let kind: Option<Option<String>> = transaction
            .prepare_cached("SELECT kind FROM chats WHERE chat_id = $id")?
            .query_row(named_params! { "$id": chat_id }, |row| row.get(0))
            .optional()?;
        match kind.flatten().as_deref() {
            None => {
                // Never held, or already gone: still a fact a list read taken before it must not
                // undo.
                self.database.touched(chat_id);
                return Ok(false);
            }
            Some(kind) if kind != "group" => return Ok(false),
            Some(_) => {}
        }
        self.database.touched(chat_id);
        for statement in [
            "DELETE FROM outbox WHERE chat_id = $id",
            "DELETE FROM messages WHERE chat_id = $id",
            "DELETE FROM chats WHERE chat_id = $id",
        ] {
            transaction
                .prepare_cached(statement)?
                .execute(named_params! { "$id": chat_id })?;
        }
        transaction.commit()?;
        Ok(true)
    }

    /// How many messages are unread across every chat — the badge.
    pub fn unread(&self) -> StoreResult<i64> {
        let _held = self.database.hold();
        Ok(self
            .chats()?
            .iter()
            .map(|row| i64::from(row.unread_count))
            .sum())
    }

    /// The caller's own read marker, applied MONOTONICALLY. The unread count is the other half of
    /// it and is recomputed here from what this device holds, which is the best it can do between
    /// reads: the server's own number arrives with the next list.
    ///
    /// The "@" mark goes once the marker passes the newest held message that would raise it —
    /// one that names this reader, from somebody else they have not blocked. The C# asks about
    /// any message with mentions, so a later message naming somebody else kept the mark on a chat
    /// with nothing unread (measured on .NET 10), which docs/protocol.md ("The mark on the chat
    /// row": "a chat with `mentioned` always has an unread count") rules out.
    pub fn mark_read(&self, chat_id: i64, last_read_message_id: i64) -> StoreResult<()> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE chats
   SET last_read_message_id = MAX(last_read_message_id, $read),
       -- AT THE BOTTOM OF WHAT THIS DEVICE HOLDS, the count is 0: there is nothing
       -- left here to read, and the server's own number arrives with the next list.
       -- Short of that, SUBTRACT what was just read rather than recounting what is
       -- held — a recount undercounts whenever history is shorter than the unread run,
       -- which on a fresh install is always.
       unread_count = CASE
           WHEN MAX(chats.last_read_message_id, $read) >=
                COALESCE((SELECT MAX(message_id) FROM messages
                           WHERE messages.chat_id = chats.chat_id), 0)
           THEN 0
           ELSE MAX(0, unread_count - (
               SELECT COUNT(*) FROM messages
                WHERE messages.chat_id = chats.chat_id
                  AND messages.sender_id <> $me
                  AND messages.message_id > chats.last_read_message_id
                  AND messages.message_id <= $read))
           END,
       -- Cleared once the marker passes the newest held message that MARKS: one naming this
       -- reader, from somebody else they have not blocked (see `touch`).
       mentioned = CASE
           WHEN $read >= (SELECT COALESCE(MAX(message_id), 0) FROM messages
                           WHERE messages.chat_id = chats.chat_id
                             AND messages.sender_id <> $me
                             AND messages.sender_id NOT IN (SELECT user_id FROM blocked)
                             AND EXISTS (
                                 SELECT 1 FROM json_each(CASE WHEN json_valid(messages.mentions_json)
                                                              THEN messages.mentions_json END)
                                  WHERE json_extract(json_each.value, '$.user_id') = $me))
           THEN 0 ELSE mentioned END
 WHERE chat_id = $chat",
            )?
            .execute(named_params! {
                "$chat": chat_id,
                "$read": last_read_message_id,
                "$me": self.reader(),
            })?;
        Ok(())
    }

    /// The three catch-up cursors, as high-water marks. Absent on the wire means "nothing of that
    /// kind has ever happened here", which is 0 — and a 0 that arrives after a 124 is not a reset.
    ///
    /// THIS IS THE ONLY WAY THEY MOVE. The columns hold what THIS DEVICE has applied, and the
    /// `max_*_seq` on a `GET /chats` row is a different number with the same name: the SERVER's
    /// maximum. Step 3's gate compares the two — "when the chat's `max_reaction_seq` from step 2
    /// exceeds the locally stored reaction cursor" — so storing the server's mark in the cursor
    /// column makes that test false forever and the catch-up never runs again. It did exactly that
    /// in `win/` until a resync test asked for a page and got none.
    pub fn advance(
        &self,
        chat_id: i64,
        reaction_seq: Option<i64>,
        edit_seq: Option<i64>,
        poll_seq: Option<i64>,
    ) -> StoreResult<()> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE chats
   SET max_reaction_seq = MAX(max_reaction_seq, $reaction),
       max_edit_seq     = MAX(max_edit_seq, $edit),
       max_poll_seq     = MAX(max_poll_seq, $poll)
 WHERE chat_id = $chat",
            )?
            .execute(named_params! {
                "$chat": chat_id,
                "$reaction": reaction_seq.unwrap_or(0),
                "$edit": edit_seq.unwrap_or(0),
                "$poll": poll_seq.unwrap_or(0),
            })?;
        Ok(())
    }

    // ---- what was said ---------------------------------------------------------------------

    /// One message, from a page, a frame or an ack. Answers whether anything changed — false for
    /// an answer this device had already applied.
    ///
    /// **ONLY A LIVE FRAME MAY RAISE THE UNREAD COUNT**, and only for somebody else's message above
    /// this reader's marker. A PAGE must not: the count on the list read that preceded it already
    /// includes every message that page is about, so counting them again doubles the badge — and
    /// the count is never RECOMPUTED from what is held either, because local history is not the
    /// whole of it (retention has swept some, paging has not fetched the rest) and a recount is a
    /// badge that silently falls to one the moment anything arrives. Apple and the web both
    /// increment; so does this.
    ///
    /// `in_sequence` is false for a copy that says nothing about the messages before it — the REST
    /// answer to a send or an edit, a copy from the edits feed or frame. It is held and drawn, and
    /// it is not where the next catch-up starts ([`catch_up_cursor`](ChatStore::catch_up_cursor))
    /// until a page or a frame delivers it again. (The C#'s defaults: `CatchUpPage`, `true`.)
    pub fn apply(
        &self,
        message: &MessageDto,
        route: SeqRoute,
        in_sequence: bool,
    ) -> StoreResult<bool> {
        let _held = self.database.hold();
        let transaction = self.connection().unchecked_transaction()?;
        let changed = self.apply_one(&transaction, message, false, route, in_sequence)?;
        transaction.commit()?;
        Ok(changed)
    }

    /// A page of history, applied in order. Answers how many messages changed.
    pub fn apply_page(
        &self,
        messages: &[MessageDto],
        route: SeqRoute,
        in_sequence: bool,
    ) -> StoreResult<usize> {
        let _held = self.database.hold();
        let transaction = self.connection().unchecked_transaction()?;
        let mut changed = 0;
        for message in messages {
            if self.apply_one(&transaction, message, false, route, in_sequence)? {
                changed += 1;
            }
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// Server copies read for somewhere other than the window — a thread read — folded into the
    /// rows this device already HOLDS (their recomputed reply count, any edit) and nothing else. A
    /// row the cache does not hold stays out of it: the cache holds pages, and a root older than
    /// them or a reply newer would move where the next page starts, so it would skip everything
    /// between (docs/protocol.md, "Threads"). Answers how many held rows changed.
    pub fn refresh(&self, messages: &[MessageDto]) -> StoreResult<usize> {
        let _held = self.database.hold();
        let transaction = self.connection().unchecked_transaction()?;
        let mut changed = 0;
        for message in messages {
            // Out of sequence, for the same reason: a preview it refreshes is still not where the
            // catch-up starts.
            if self.read_message(&transaction, message.id)?.is_some()
                && self.apply_one(&transaction, message, false, SeqRoute::Evidence, false)?
            {
                changed += 1;
            }
        }
        transaction.commit()?;
        Ok(changed)
    }

    /// Reactions, from the frame or the catch-up: complete state, never a delta, and applied only
    /// when the seq is newer than the one held. (The C#'s default route: `LiveFrame`.)
    ///
    /// THE CHAT IS A PARAMETER BECAUSE THE MESSAGE MAY BE UNKNOWN. A state for a message this
    /// device does not hold is DROPPED — history paging re-delivers it embedded on the message
    /// itself — and the chat cursor moves all the same, because a live frame and a catch-up page
    /// are each a complete statement about that chat's sequence and a cursor left behind asks for
    /// a page that answers the same nothing. Evidence moves nothing (see [`SeqRoute`]), so the
    /// route is the whole of the rule and not a label.
    pub fn apply_reactions(
        &self,
        chat_id: i64,
        message_id: i64,
        reaction_seq: i64,
        reactions: &[ReactionDto],
        route: SeqRoute,
    ) -> StoreResult<bool> {
        let _held = self.database.hold();
        if route != SeqRoute::Evidence {
            self.advance(chat_id, Some(reaction_seq), None, None)?;
        }
        let Some(held) = self.message(message_id)? else {
            return Ok(false);
        };
        if reaction_seq < held.reaction_seq.unwrap_or(0) {
            return Ok(false);
        }
        self.connection()
            .prepare_cached(
                "UPDATE messages SET reaction_seq = $seq, reactions_json = $reactions WHERE message_id = $id",
            )?
            .execute(named_params! {
                "$seq": reaction_seq,
                "$reactions": wire::encode(reactions),
                "$id": message_id,
            })?;
        Ok(true)
    }

    /// A poll's state, under its own sequence — and its chat's cursor, as above. (The C#'s default
    /// route: `LiveFrame`.)
    pub fn apply_poll(
        &self,
        chat_id: i64,
        message_id: i64,
        poll: &PollDto,
        route: SeqRoute,
    ) -> StoreResult<bool> {
        let _held = self.database.hold();
        if route != SeqRoute::Evidence {
            self.advance(chat_id, None, None, Some(poll.poll_seq))?;
        }
        let Some(held) = self.message(message_id)? else {
            return Ok(false);
        };
        if poll.poll_seq < held.poll.as_ref().map_or(0, |held| held.poll_seq) {
            return Ok(false);
        }
        self.connection()
            .prepare_cached("UPDATE messages SET poll_json = $poll WHERE message_id = $id")?
            .execute(named_params! { "$poll": wire::encode(poll), "$id": message_id })?;
        Ok(true)
    }

    /// One message, as this device holds it.
    pub fn message(&self, message_id: i64) -> StoreResult<Option<MessageDto>> {
        let _held = self.database.hold();
        self.read_message(self.connection(), message_id)
    }

    /// A page of one chat, NEWEST FIRST — the direction a chat view pages in, backwards from where
    /// the reader is. (The C#'s default `limit` is [`PAGE`].)
    pub fn messages(
        &self,
        chat_id: i64,
        before_id: Option<i64>,
        limit: usize,
    ) -> StoreResult<Vec<MessageDto>> {
        let _held = self.database.hold();
        let messages = match before_id {
            None => self
                .connection()
                .prepare_cached(
                    "SELECT * FROM messages WHERE chat_id = $chat ORDER BY message_id DESC LIMIT $limit",
                )?
                .query_map(
                    named_params! { "$chat": chat_id, "$limit": limit },
                    read_message,
                )?
                .collect::<Result<Vec<_>, _>>()?,
            Some(before) => self
                .connection()
                .prepare_cached(
                    "SELECT * FROM messages WHERE chat_id = $chat AND message_id < $before ORDER BY message_id DESC LIMIT $limit",
                )?
                .query_map(
                    named_params! { "$chat": chat_id, "$limit": limit, "$before": before },
                    read_message,
                )?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(messages)
    }

    /// One chain, oldest first: the root and everything that names it. A chain is read far more
    /// often than it is written, which is why the root is stored rather than walked.
    pub fn thread(&self, root_id: i64) -> StoreResult<Vec<MessageDto>> {
        let _held = self.database.hold();
        let messages = self
            .connection()
            .prepare_cached(
                "SELECT * FROM messages WHERE message_id = $root OR thread_root_id = $root ORDER BY message_id",
            )?
            .query_map(named_params! { "$root": root_id }, read_message)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// Every poll this device holds for one chat, oldest first — what the unanswered count reads.
    pub fn polls(&self, chat_id: i64) -> StoreResult<Vec<MessageDto>> {
        let _held = self.database.hold();
        let messages = self
            .connection()
            .prepare_cached(
                "SELECT * FROM messages WHERE chat_id = $chat AND poll_json IS NOT NULL ORDER BY message_id",
            )?
            .query_map(named_params! { "$chat": chat_id }, read_message)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(messages)
    }

    /// The newest message of a chat — the row's preview.
    pub fn newest(&self, chat_id: i64) -> StoreResult<Option<MessageDto>> {
        let _held = self.database.hold();
        Ok(self.messages(chat_id, None, 1)?.into_iter().next())
    }

    /// One chat's message catch-up cursor: the newest message delivered IN SEQUENCE, by a page or a
    /// live frame — never a `GET /chats` preview, which would make `after_id` the server's newest
    /// id and skip everything before it for good, nor any other copy that says nothing about the
    /// messages before it (docs/protocol.md, "Best-effort delivery", step 3). `None` when the chat
    /// holds nothing in sequence: there is no hole to fill, and opening it reads the newest page.
    pub fn catch_up_cursor(&self, chat_id: i64) -> StoreResult<Option<i64>> {
        let _held = self.database.hold();
        // Down the chat's own index, stopping at the first row in sequence — which is almost
        // always the newest — rather than a MAX over every row the chat holds, on every open.
        Ok(self
            .connection()
            .prepare_cached(
                "SELECT message_id FROM messages WHERE chat_id = $chat AND sequenced = 1 ORDER BY message_id DESC LIMIT 1",
            )?
            .query_row(named_params! { "$chat": chat_id }, |row| row.get(0))
            .optional()?)
    }

    /// Every chat's [`catch_up_cursor`](ChatStore::catch_up_cursor) at once — what a connection
    /// takes as it opens.
    pub fn catch_up_cursors(&self) -> StoreResult<BTreeMap<i64, i64>> {
        let _held = self.database.hold();
        let cursors = self
            .connection()
            .prepare_cached(
                "SELECT chat_id, MAX(message_id) FROM messages WHERE sequenced = 1 GROUP BY chat_id",
            )?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<BTreeMap<i64, i64>, _>>()?;
        Ok(cursors)
    }

    // ---- sets an older build cached without the flags (schema step 8) -------------------------

    /// Whether a set written by a build that did not know the flags COULD have lost one: a message
    /// with no body and exactly one attachment, a photo (a sticker,
    /// [`MessageDto::sticker_picture`]) or a video (a circle, [`MessageDto::round_video`]),
    /// carrying neither flag. Anything else draws the same with or without them.
    pub fn could_have_lost_a_flag(message: &MessageDto) -> bool {
        message.body.is_empty()
            && matches!(
                message.media(),
                [only] if (only.kind == "photo" || only.kind == "video") && !only.sticker && !only.round
            )
    }

    /// The held messages an older build may have cached as a plain photo or video that is really a
    /// STICKER or a VIDEO MESSAGE, newest first, at most `limit` — what the resync reads once more
    /// (iOS `repairUnknownRoundFlags`, Android `MessageDao.roundFlagUnknown`; Windows also asks
    /// about photos, because it learned stickers in #58, after the 1.1 the Store has, and a set
    /// that build cached lost `sticker` too).
    ///
    /// Every unknown row that is NOT a candidate is settled on the way, with no request: a message
    /// with words, one with no attachment, an album, a voice note, a set that already carries a
    /// flag. So the scan only ever shrinks, and a pass that finds nothing to ask about costs one
    /// query next time.
    pub fn flag_repair_candidates(&self, limit: usize) -> StoreResult<Vec<MessageDto>> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE messages SET attachments_know_flags = 1
 WHERE attachments_know_flags = 0 AND (attachments_json IS NULL OR body <> '')",
            )?
            .execute([])?;
        let mut found = Vec::new();
        while found.len() < limit {
            let scanned = self
                .connection()
                .prepare_cached(
                    "SELECT * FROM messages WHERE attachments_know_flags = 0
 ORDER BY message_id DESC LIMIT $limit OFFSET $skip",
                )?
                .query_map(
                    named_params! { "$limit": limit, "$skip": found.len() },
                    read_message,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            if scanned.is_empty() {
                break;
            }
            let whole = scanned.len() >= limit;
            for message in scanned {
                if !ChatStore::could_have_lost_a_flag(&message) {
                    self.settle_flags(message.id)?;
                } else if found.len() < limit {
                    found.push(message);
                }
            }
            if !whole {
                break;
            }
        }
        Ok(found)
    }

    /// A row the repair asked about, or need not ask about: never asked about again.
    pub fn settle_flags(&self, message_id: i64) -> StoreResult<()> {
        let _held = self.database.hold();
        self.connection()
            .prepare_cached(
                "UPDATE messages SET attachments_know_flags = 1 WHERE message_id = $id",
            )?
            .execute(named_params! { "$id": message_id })?;
        Ok(())
    }

    /// The server's copy of a held message's set, written over the one an older build cached — and
    /// nothing else about the row: no cursor moves (the row may lie outside the run the catch-up
    /// has delivered, and a cursor dragged to it would skip the gap for good), no count, no
    /// ordering. A copy with no set settles the row and changes nothing. Answers whether the held
    /// set changed.
    pub fn repair_media(&self, server: &MessageDto) -> StoreResult<bool> {
        let _held = self.database.hold();
        if server.attachments.is_none() && server.attachment.is_none() {
            self.settle_flags(server.id)?;
            return Ok(false);
        }
        let encoded = wire::encode(server.media());
        let changed = self
            .connection()
            .prepare_cached(
                "UPDATE messages SET attachments_json = $attachments, attachments_know_flags = 1
 WHERE message_id = $id AND attachments_json IS NOT $attachments",
            )?
            .execute(named_params! { "$attachments": encoded, "$id": server.id })?
            > 0;
        self.settle_flags(server.id)?;
        Ok(changed)
    }

    // ---- who -------------------------------------------------------------------------------

    /// The roster, replaced whole — of the family being shown, when there is one (see
    /// [`family`](ChatStore::family)). A member who LEFT and an account that was DELETED both keep
    /// their row: their messages and notes keep their authors, and a name has to resolve for an
    /// old message. (The C#'s `Replace(members, former)`; no former members is `&[]`.)
    pub fn replace_members(&self, members: &[MemberDto], former: &[MemberDto]) -> StoreResult<()> {
        let _held = self.database.hold();
        self.replace_members_of(self.family(), members, former)
    }

    /// ONE family's roster, replaced whole (docs/protocol.md, "Several families"): the person's own
    /// fields — name, picture, birthday, whether the account is gone — on `members`, which every
    /// family shares, and the family's — the role, whether they left it, whether its owner may
    /// reset their password — on that family's rows alone. `None` is the one family a cache from
    /// before several families held, and writes the person's row only. (The C#'s
    /// `Replace(familyId, members, former)`.)
    pub fn replace_members_of(
        &self,
        family_id: Option<i64>,
        members: &[MemberDto],
        former: &[MemberDto],
    ) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        let transaction = self.connection().unchecked_transaction()?;
        if let Some(held) = family_id {
            transaction
                .prepare_cached("DELETE FROM family_members WHERE family_id = $family")?
                .execute(named_params! { "$family": held })?;
        }
        for member in members.iter().chain(former) {
            transaction
                .prepare_cached(
                    "INSERT INTO members (user_id, username, display_name, avatar_version, role,
                     has_left, deleted, birthday_month, birthday_day)
VALUES ($id, $username, $name, $avatar, $role, $left, $deleted, $month, $day)
ON CONFLICT(user_id) DO UPDATE SET
    username = excluded.username, display_name = excluded.display_name,
    avatar_version = excluded.avatar_version, role = excluded.role,
    has_left = excluded.has_left, deleted = excluded.deleted,
    birthday_month = excluded.birthday_month, birthday_day = excluded.birthday_day",
                )?
                .execute(named_params! {
                    "$id": member.id,
                    "$username": member.username,
                    "$name": member.display_name,
                    "$avatar": member.avatar_version,
                    "$role": member.role,
                    "$left": member.has_left,
                    "$deleted": member.deleted,
                    "$month": member.birthday.map(|birthday| birthday.month),
                    "$day": member.birthday.map(|birthday| birthday.day),
                })?;
            if let Some(family) = family_id {
                transaction
                    .prepare_cached(
                        "INSERT OR REPLACE INTO family_members (family_id, user_id, role, has_left, password_reset)
VALUES ($family, $id, $role, $left, $reset)",
                    )?
                    .execute(named_params! {
                        "$family": family,
                        "$id": member.id,
                        "$role": member.role,
                        "$left": member.has_left || member.deleted,
                        "$reset": member.password_reset,
                    })?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The roster of the family being shown — present and former members alike. One copy is
    /// handed to every caller until the table changes, and nobody can change it: it is shared, and
    /// a slice behind an `Rc` has no `&mut` to give.
    pub fn members(&self) -> StoreResult<Rc<[MemberDto]>> {
        let _held = self.database.hold();
        let mut kept = self.kept.borrow_mut();
        self.fresh(&mut kept);
        self.roster(&mut kept)
    }

    /// ONE family's roster, named — what an owner's "Invite…" reads for the members of their OTHER
    /// families. Empty for a family this device has no roster of.
    pub fn members_of(&self, family_id: i64) -> StoreResult<Vec<MemberDto>> {
        let _held = self.database.hold();
        self.read_members(Some(family_id), true)
    }

    /// The families this device holds a roster for — every one a resync read, until one is dropped
    /// ([`Database::drop_family`]).
    pub fn roster_families(&self) -> StoreResult<Vec<i64>> {
        let _held = self.database.hold();
        let ids = self
            .connection()
            .prepare_cached("SELECT DISTINCT family_id FROM family_members ORDER BY family_id")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    /// The member behind an id, or `None` when this device has never heard of them — which
    /// happens, and is the caller's to name ("Deleted account" is a CLIENT's word, not the
    /// server's).
    ///
    /// The family being shown first, and then ANYBODY this device has a row for: a name has to
    /// resolve for a message from another family's chat (a notification, a direct chat listed
    /// elsewhere), and the person is the same person.
    pub fn member(&self, user_id: i64) -> StoreResult<Option<MemberDto>> {
        let _held = self.database.hold();
        let mut kept = self.kept.borrow_mut();
        self.fresh(&mut kept);
        if kept.roster_by_id.is_none() {
            let roster = self.roster(&mut kept)?;
            kept.roster_by_id = Some(by_id(&roster));
        }
        if let Some(known) = kept.roster_by_id.as_ref().and_then(|by| by.get(&user_id)) {
            return Ok(Some(known.clone()));
        }
        if kept.directory_by_id.is_none() {
            kept.directory_by_id = Some(by_id(&self.read_members(None, false)?));
        }
        Ok(kept
            .directory_by_id
            .as_ref()
            .and_then(|by| by.get(&user_id))
            .cloned())
    }

    /// A member who JOINED, as the frame carries them — a `User`, so a newcomer's role is
    /// "member". A REJOIN lands here too: the row survived their leaving, and this is what takes
    /// the flag back off.
    ///
    /// The narrow writer, not the roster one: an absent field must never clear a stored one, so a
    /// frame that says nothing about a birthday leaves the one this device knows alone — and a
    /// frame that says nothing about a ROLE leaves the role of somebody already here alone: a
    /// family started in the app is announced by a `member_joined` naming its creator, who is its
    /// OWNER, and that frame may land after the roster read that said so (docs/protocol.md,
    /// "Several families"). Only a new row, or one that had left, becomes "member". The exception
    /// is [`deleted`](ChatStore::deleted), whose whole job is to wipe. `family_id` is the family
    /// the frame names — never "my family" (docs/protocol.md, "Several families") — and the one
    /// shown when it names none. `password_reset` is the family OWNER's flag for the newcomer —
    /// false when they are also in a family the owner does not own, where the server refuses the
    /// reset with `member_in_other_families`. It comes on the owner's copy of `member_joined` and
    /// on the member an approval answers with; `None` (everybody else's copy, an older server)
    /// keeps the flag already stored, the same narrow rule as the birthday, and only a row with no
    /// flag at all falls back to the old rule, where the reset is offered.
    pub fn joined(
        &self,
        user: &UserDto,
        family_id: Option<i64>,
        password_reset: Option<bool>,
    ) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        let transaction = self.connection().unchecked_transaction()?;
        transaction
            .prepare_cached(
                "INSERT INTO members (user_id, username, display_name, avatar_version, role,
                     has_left, deleted, birthday_month, birthday_day)
VALUES ($id, $username, $name, $avatar, 'member', 0, 0, $month, $day)
ON CONFLICT(user_id) DO UPDATE SET
    username = excluded.username, display_name = excluded.display_name,
    avatar_version = excluded.avatar_version,
    -- A rejoin is a member again, and never the owner by arriving; somebody still here keeps their role.
    role = CASE WHEN members.has_left = 0 AND members.role IS NOT NULL THEN members.role ELSE 'member' END,
    has_left = 0, deleted = 0,
    birthday_month = COALESCE(excluded.birthday_month, members.birthday_month),
    birthday_day = COALESCE(excluded.birthday_day, members.birthday_day)",
            )?
            .execute(named_params! {
                "$id": user.id,
                "$username": user.username,
                "$name": user.display_name,
                "$avatar": user.avatar_version,
                "$month": user.birthday.map(|birthday| birthday.month),
                "$day": user.birthday.map(|birthday| birthday.day),
            })?;
        if let Some(joined) = family_id.or_else(|| self.family()) {
            transaction
                .prepare_cached(
                    "INSERT INTO family_members (family_id, user_id, role, has_left, password_reset)
VALUES ($family, $id, 'member', 0, $reset)
ON CONFLICT(family_id, user_id) DO UPDATE SET
    -- Never a downgrade: the creator of a family started in the app is its owner (see above).
    role = CASE WHEN family_members.has_left = 0 AND family_members.role IS NOT NULL
                THEN family_members.role ELSE 'member' END,
    has_left = 0,
    password_reset = COALESCE(excluded.password_reset, family_members.password_reset)",
                )?
                .execute(named_params! {
                    "$family": joined,
                    "$id": user.id,
                    "$reset": password_reset,
                })?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// A member who LEFT a family — the one `family_id` names, else the one shown. Their row stays,
    /// and their name keeps resolving for the messages and notes they left behind; only the flag
    /// and the role change.
    pub fn left(&self, user_id: i64, family_id: Option<i64>) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        // Per family once rosters are kept per family; on the person's row while the cache is
        // still one from before.
        match family_id.or_else(|| self.family()) {
            Some(left) if self.has_family_rosters()? => {
                self.connection()
                    .prepare_cached(
                        "UPDATE family_members SET has_left = 1, role = NULL WHERE family_id = $family AND user_id = $id",
                    )?
                    .execute(named_params! { "$family": left, "$id": user_id })?;
            }
            _ => {
                self.connection()
                    .prepare_cached(
                        "UPDATE members SET has_left = 1, role = NULL WHERE user_id = $id",
                    )?
                    .execute(named_params! { "$id": user_id })?;
            }
        }
        Ok(())
    }

    /// An account that was DELETED, written as the tombstone the frame carries.
    ///
    /// THE ONE WRITE WHOSE JOB IS TO WIPE. The `member_deleted` frame carries the whole tombstone —
    /// the placeholder name, `avatar_version: 0`, no birthday — "because that is exactly what a
    /// client has to overwrite", and it is applied deliberately rather than through the ordinary
    /// upsert, which everywhere else must never let an absent field clear a stored one
    /// (docs/protocol.md, server frames). An account deleted leaves EVERY family, so every family's
    /// row says so — and a second frame for a second family writes the same tombstone again,
    /// harmlessly.
    pub fn deleted(&self, tombstone: &MemberDto) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        let transaction = self.connection().unchecked_transaction()?;
        transaction
            .prepare_cached(
                "INSERT INTO members (user_id, username, display_name, avatar_version, role,
                     has_left, deleted, birthday_month, birthday_day)
VALUES ($id, $username, $name, $avatar, NULL, 1, 1, NULL, NULL)
ON CONFLICT(user_id) DO UPDATE SET
    username = excluded.username, display_name = excluded.display_name,
    avatar_version = 0, role = NULL, has_left = 1, deleted = 1,
    birthday_month = NULL, birthday_day = NULL",
            )?
            .execute(named_params! {
                "$id": tombstone.id,
                "$username": tombstone.username,
                "$name": tombstone.display_name,
                "$avatar": 0,
            })?;
        transaction
            .prepare_cached(
                "UPDATE family_members SET has_left = 1, role = NULL WHERE user_id = $id",
            )?
            .execute(named_params! { "$id": tombstone.id })?;
        transaction.commit()?;
        Ok(())
    }

    /// A family's new owner, as the `family_owner` frame names them — in the family it names, else
    /// the one shown: one owner, so whoever held it stops holding it in the same write.
    pub fn set_owner(&self, user_id: i64, family_id: Option<i64>) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        match family_id.or_else(|| self.family()) {
            Some(owned) if self.has_family_rosters()? => {
                self.connection()
                    .prepare_cached(
                        "UPDATE family_members
   SET role = CASE WHEN user_id = $id THEN 'owner' ELSE 'member' END
 WHERE family_id = $family AND has_left = 0",
                    )?
                    .execute(named_params! { "$family": owned, "$id": user_id })?;
            }
            _ => {
                self.connection()
                    .prepare_cached(
                        "UPDATE members
   SET role = CASE WHEN user_id = $id THEN 'owner' ELSE 'member' END
 WHERE has_left = 0 AND deleted = 0",
                    )?
                    .execute(named_params! { "$id": user_id })?;
            }
        }
        Ok(())
    }

    // ---- who this reader will not see -----------------------------------------------------------

    /// The caller's own block list, replaced WHOLE — it is complete state on every read that
    /// carries it, and an absent list means nobody rather than "leave what you hold alone".
    ///
    /// This deliberately does NOT relist or hide a chat. The list read that follows it decides
    /// what is on the list — the server omits a blocked peer's direct chat from `GET /chats` for
    /// the blocker alone — and two writers for one fact would fight over it. The frame path is
    /// where a client applies the consequence itself, because there no list read is coming: see
    /// [`set_blocked`](ChatStore::set_blocked).
    pub fn replace_blocked(&self, user_ids: &[i64]) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        let transaction = self.connection().unchecked_transaction()?;
        transaction
            .prepare_cached("DELETE FROM blocked")?
            .execute([])?;
        let mut seen = HashSet::new();
        for &id in user_ids {
            if seen.insert(id) {
                transaction
                    .prepare_cached("INSERT OR IGNORE INTO blocked (user_id) VALUES ($id)")?
                    .execute(named_params! { "$id": id })?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// One member's block state, as the `member_blocked` frame states it: a BOOLEAN carrying full
    /// current state, so an unblock is the same frame with `false`.
    ///
    /// The direct chat follows it here, and only here. The frame reaches the blocker's own devices
    /// and nothing else is coming — no list read, no error — so a client applies the consequence it
    /// would otherwise wait for: the chat leaves the list on a block and comes back WHOLE on an
    /// unblock, because nothing about it was ever deleted.
    pub fn set_blocked(&self, user_id: i64, blocked: bool) -> StoreResult<()> {
        let _held = self.database.hold();
        self.database.roster_changed();
        let transaction = self.connection().unchecked_transaction()?;
        transaction
            .prepare_cached(if blocked {
                "INSERT OR IGNORE INTO blocked (user_id) VALUES ($id)"
            } else {
                "DELETE FROM blocked WHERE user_id = $id"
            })?
            .execute(named_params! { "$id": user_id })?;
        transaction
            .prepare_cached(
                "UPDATE chats SET listed = $listed WHERE kind = 'direct' AND peer_user_id = $id",
            )?
            .execute(named_params! { "$listed": !blocked, "$id": user_id })?;
        transaction.commit()?;
        Ok(())
    }

    /// Everyone this reader has blocked, whether or not the roster can name them — ascending, and
    /// shared like the roster.
    pub fn blocked(&self) -> StoreResult<Rc<[i64]>> {
        let _held = self.database.hold();
        let mut kept = self.kept.borrow_mut();
        self.fresh(&mut kept);
        self.blocked_list(&mut kept)
    }

    /// Whether this reader has blocked that member. What a client DRAWS for one is the hidden row:
    /// in the family chat their messages still arrive, still count and may still be the preview,
    /// because the count is the other half of the read marker (docs/protocol.md, "Blocking a
    /// member").
    pub fn is_blocked(&self, user_id: i64) -> StoreResult<bool> {
        let _held = self.database.hold();
        let mut kept = self.kept.borrow_mut();
        self.fresh(&mut kept);
        if kept.blocked_set.is_none() {
            let blocked = self.blocked_list(&mut kept)?;
            kept.blocked_set = Some(blocked.iter().copied().collect());
        }
        Ok(kept
            .blocked_set
            .as_ref()
            .is_some_and(|set| set.contains(&user_id)))
    }

    /// A chat's KIND — "family", "direct", "ai", "group" — or `None` while this device holds no row
    /// for it. What `chat(chat_id)?.chat.kind` answers, without reading the row and its newest
    /// message to get there: a conversation asks it of every bubble it draws.
    ///
    /// An unknown chat is not remembered as unknown: the list read that brings its row is a write,
    /// and the next ask reads it.
    pub fn kind(&self, chat_id: i64) -> StoreResult<Option<String>> {
        let _held = self.database.hold();
        let mut kept = self.kept.borrow_mut();
        self.fresh(&mut kept);
        if let Some(known) = kept.kinds.get(&chat_id) {
            return Ok(Some(known.clone()));
        }
        let kind: Option<Option<String>> = self
            .connection()
            .prepare_cached("SELECT kind FROM chats WHERE chat_id = $id")?
            .query_row(named_params! { "$id": chat_id }, |row| row.get(0))
            .optional()?;
        let Some(kind) = kind.flatten() else {
            return Ok(None);
        };
        kept.kinds.insert(chat_id, kind.clone());
        Ok(Some(kind))
    }

    // ---- what is kept in memory ------------------------------------------------------------

    /// Drop what is kept when a write has happened since it was read.
    fn fresh(&self, kept: &mut Kept) {
        if kept.roster_at != self.database.roster_generation() {
            kept.roster_at = self.database.roster_generation();
            kept.roster = None;
            kept.roster_by_id = None;
            kept.directory_by_id = None;
            kept.blocked = None;
            kept.blocked_set = None;
        }
        let family = self.family();
        if kept.roster_family != family {
            // Another family is shown: its roster, not the last one's.
            kept.roster_family = family;
            kept.roster = None;
            kept.roster_by_id = None;
        }
        if kept.kinds_at != self.database.chats_generation() {
            kept.kinds_at = self.database.chats_generation();
            kept.kinds.clear();
        }
    }

    /// The kept roster of the family shown, read when there is none.
    fn roster(&self, kept: &mut Kept) -> StoreResult<Rc<[MemberDto]>> {
        if let Some(roster) = &kept.roster {
            return Ok(Rc::clone(roster));
        }
        let roster: Rc<[MemberDto]> = self.read_members(self.family(), false)?.into();
        kept.roster = Some(Rc::clone(&roster));
        Ok(roster)
    }

    /// The kept block list, read when there is none.
    fn blocked_list(&self, kept: &mut Kept) -> StoreResult<Rc<[i64]>> {
        if let Some(blocked) = &kept.blocked {
            return Ok(Rc::clone(blocked));
        }
        let blocked: Rc<[i64]> = self
            .connection()
            .prepare_cached("SELECT user_id FROM blocked ORDER BY user_id")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?
            .into();
        kept.blocked = Some(Rc::clone(&blocked));
        Ok(blocked)
    }

    /// The roster of `family_id`, or — `None`, or a cache whose rosters were never kept per family
    /// (one built before several families, not yet read again) — every person on `members`, as it
    /// always was.
    fn read_members(&self, family_id: Option<i64>, scoped: bool) -> StoreResult<Vec<MemberDto>> {
        let per_family = match family_id {
            Some(family) if scoped || self.has_family_rosters()? => Some(family),
            _ => None,
        };
        let members = match per_family {
            Some(family) => self
                .connection()
                .prepare_cached(
                    "SELECT m.user_id, m.username, m.display_name, m.avatar_version, f.role, f.has_left, m.deleted,
       m.birthday_month, m.birthday_day, f.password_reset
  FROM members m JOIN family_members f ON f.user_id = m.user_id
 WHERE f.family_id = $family
 ORDER BY m.display_name, m.user_id",
                )?
                .query_map(named_params! { "$family": family }, read_member)?
                .collect::<Result<Vec<_>, _>>()?,
            None => self
                .connection()
                .prepare_cached(
                    "SELECT user_id, username, display_name, avatar_version, role, has_left, deleted,
       birthday_month, birthday_day, NULL AS password_reset
  FROM members ORDER BY display_name, user_id",
                )?
                .query_map([], read_member)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(members)
    }

    /// Whether any roster is kept per family yet.
    fn has_family_rosters(&self) -> StoreResult<bool> {
        let exists: i64 = self
            .connection()
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM family_members)")?
            .query_row([], |row| row.get(0))?;
        Ok(exists == 1)
    }

    // ---- rows ------------------------------------------------------------------------------

    fn ids(&self, connection: &Connection) -> StoreResult<Vec<i64>> {
        let ids = connection
            .prepare_cached("SELECT chat_id FROM chats")?
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<i64>, _>>()?;
        Ok(ids)
    }

    fn read_message(
        &self,
        connection: &Connection,
        message_id: i64,
    ) -> StoreResult<Option<MessageDto>> {
        Ok(connection
            .prepare_cached("SELECT * FROM messages WHERE message_id = $id")?
            .query_row(named_params! { "$id": message_id }, read_message)
            .optional()?)
    }

    /// One list row. Everything on it is the server's, EXCEPT the three catch-up cursors, which
    /// are not written at all — they are this device's own (see [`advance`](ChatStore::advance)).
    fn write(&self, connection: &Connection, row: &ChatRowDto) -> StoreResult<()> {
        let (members, admin, families) = group_parameters(&row.chat);
        connection
            .prepare_cached(
                "INSERT INTO chats (chat_id, kind, title, peer_user_id, member_ids_json, admin_user_id, family_ids_json,
                   unread_count, last_read_message_id, mentioned,
                   max_reaction_seq, max_edit_seq, max_poll_seq, last_message_at)
-- The three cursors start at zero on a chat this device has never held, and a list
-- read NEVER writes them: see the note below.
VALUES ($id, $kind, $title, $peer, $members, $admin, $families, $unread, $read, $mentioned, 0, 0, 0, $at)
ON CONFLICT(chat_id) DO UPDATE SET
    kind = excluded.kind, title = excluded.title, peer_user_id = excluded.peer_user_id,
    -- A group's members and admin are the server's, whole, on every read that carries the chat.
    member_ids_json = excluded.member_ids_json, admin_user_id = excluded.admin_user_id,
    -- And which families list it, the same way: a direct chat's list shrinks and grows with the families
    -- the two share (docs/protocol.md, \"Several families\").
    family_ids_json = excluded.family_ids_json,
    -- Back on the list: an unblock relists the chat it hid.
    listed = 1,
    -- THE SERVER'S NUMBER IS THE AUTHORITY, PLUS WHAT RACED IT. The server counted
    -- up to the preview it sent, so anything this device holds ABOVE that preview
    -- arrived while the read was in flight and is on nobody's answer: a frame landing
    -- mid-read would otherwise vanish from the badge until the next list.
    unread_count = excluded.unread_count + (
        SELECT COUNT(*) FROM messages
         WHERE messages.chat_id = chats.chat_id
           AND messages.sender_id <> $me
           AND messages.message_id > MAX(
               $snapshot,
               MAX(chats.last_read_message_id, excluded.last_read_message_id))),
    -- MONOTONIC: a list read that crossed a `read` frame must not walk the marker
    -- back, for the same reason the server applies it that way.
    last_read_message_id = MAX(chats.last_read_message_id, excluded.last_read_message_id),
    -- A mention this device raised and has not read is not cleared by an answer that
    -- did not see it; `mark_read` is what takes it off, and only once the marker has
    -- passed the message that named the reader.
    mentioned = CASE
        WHEN excluded.mentioned THEN 1
        WHEN chats.mentioned AND COALESCE(
            (SELECT MAX(message_id) FROM messages
              WHERE messages.chat_id = chats.chat_id), 0) >
            MAX(chats.last_read_message_id, excluded.last_read_message_id)
        THEN 1 ELSE 0 END,
    last_message_at = MAX(chats.last_message_at, excluded.last_message_at)",
            )?
            .execute(named_params! {
                "$id": row.chat.id,
                "$kind": row.chat.kind,
                "$title": row.chat.title,
                "$peer": row.chat.peer_user_id,
                "$members": members,
                "$admin": admin,
                "$families": families,
                "$unread": row.unread_count,
                "$me": self.reader(),
                // The newest message the server's own answer carried: the line its count was drawn
                // to.
                "$snapshot": row.last_message.as_ref().map_or(0, |last| last.id),
                "$read": row.last_read_message_id,
                // `mentioned` rides only when an unread message names the caller — absent, never
                // false.
                "$mentioned": row.mentioned == Some(true),
                "$at": row
                    .last_message
                    .as_ref()
                    .and_then(|last| times::instant(Some(&last.created_at)))
                    .unwrap_or(0),
            })?;
        Ok(())
    }

    /// One message, merged under the guards. A PREVIEW never overwrites a fuller copy's reactions,
    /// poll or quote — it does not carry them, and absent means "not included here", not
    /// "cleared".
    fn apply_one(
        &self,
        connection: &Connection,
        message: &MessageDto,
        is_preview: bool,
        route: SeqRoute,
        in_sequence: bool,
    ) -> StoreResult<bool> {
        let held = self.read_message(connection, message.id)?;
        let is_new = held.is_none();
        if let Some(held) = held {
            // AN EDIT IS GUARDED: overwrite only when the incoming seq is at least the one held,
            // or a history page fetched before an edit and delivered after it quietly restores
            // the old text and two devices disagree about what was said.
            let incoming = message.edit_seq.unwrap_or(0);
            let stored = held.edit_seq.unwrap_or(0);
            if incoming < stored {
                return Ok(false);
            }
            if is_preview && incoming == stored {
                // Nothing new, and a preview carries less than what is held.
                return Ok(false);
            }
        }
        let attachments = if message.attachments.is_none() && message.attachment.is_none() {
            None
        } else {
            Some(wire::encode(message.media()))
        };
        connection
            .prepare_cached(
                "INSERT INTO messages (message_id, chat_id, sender_id, client_msg_id, body, created_at,
                      edited_at, edit_seq, reply_to_id, reply_to_json, thread_root_id, reply_count,
                      reaction_seq, reactions_json, attachments_json, mentions_json,
                      poll_json, call_json, sequenced, attachments_know_flags)
VALUES ($id, $chat, $sender, $client, $body, $created,
        $edited, $editSeq, $reply, $quote, $root, $replies,
        $reactionSeq, $reactions, $attachments, $mentions, $poll, $call, $sequenced, 1)
ON CONFLICT(message_id) DO UPDATE SET
    body = excluded.body, edited_at = excluded.edited_at, edit_seq = excluded.edit_seq,
    -- In sequence once a page or a frame has delivered it, and a later preview, answer or thread read cannot take
    -- that back.
    sequenced = MAX(messages.sequenced, excluded.sequenced),
    reply_count = excluded.reply_count,
    -- These three are only overwritten by an answer that HAS them: a preview and a
    -- page carry different amounts of one message, and absent is not empty.
    reactions_json = COALESCE(excluded.reactions_json, messages.reactions_json),
    attachments_json = COALESCE(excluded.attachments_json, messages.attachments_json),
    -- A set THIS build decoded knows the sticker and video-message flags (schema step 8): a copy that carries
    -- one settles a row an older build wrote, and a copy that carries none leaves the row as it was.
    attachments_know_flags = CASE WHEN excluded.attachments_json IS NULL
                                  THEN messages.attachments_know_flags ELSE 1 END,
    poll_json = COALESCE(excluded.poll_json, messages.poll_json),
    reaction_seq = MAX(COALESCE(excluded.reaction_seq, 0),
                       COALESCE(messages.reaction_seq, 0)),
    mentions_json = COALESCE(excluded.mentions_json, messages.mentions_json),
    -- The server RECOMPUTES a quote on every read, so a newer copy is the truer one
    -- (an edit of the quoted message changes its excerpt) — and an answer that carries
    -- NO quote is a chat-list preview, which knows nothing about one, not a reply that
    -- has lost it. Absent is not empty here, exactly as above.
    reply_to_json = COALESCE(excluded.reply_to_json, messages.reply_to_json)",
            )?
            .execute(named_params! {
                "$id": message.id,
                "$chat": message.chat_id,
                "$sender": message.sender_id,
                "$client": message.client_msg_id,
                "$body": message.body,
                "$created": times::instant(Some(&message.created_at)).unwrap_or(0),
                "$edited": times::instant(message.edited_at.as_deref()),
                "$editSeq": message.edit_seq,
                "$reply": message.reply_to.as_ref().map(|quote| quote.message_id),
                // Both levels, as the wire wrote them: an excerpt this device does not keep is a
                // reply drawn as "Someone" with nothing under it, which is what a quote exists to
                // prevent.
                "$quote": message.reply_to.as_ref().map(wire::encode),
                "$root": message.thread_root_id,
                "$replies": message.reply_count,
                "$reactionSeq": message.reaction_seq,
                "$reactions": message.reactions.as_ref().map(wire::encode),
                "$attachments": attachments,
                "$mentions": message.mentions.as_ref().map(wire::encode),
                "$poll": message.poll.as_ref().map(wire::encode),
                "$call": message.call.as_ref().map(wire::encode),
                "$sequenced": in_sequence && !is_preview,
            })?;
        // A NEW REPLY RAISES ITS ROOT'S COUNT, once, on whichever route first brings it: a live
        // frame, the after_id catch-up, or the answer to this device's own send. Never a page of
        // history, an edit or a thread read — they arrive as Evidence — and never a preview: those
        // deliver replies the root's recomputed copy already includes, and counting them would
        // show two where there is one (docs/protocol.md, "Threads"). A root this device does not
        // hold has nothing to raise; the next copy of it brings the true count.
        if is_new
            && !is_preview
            && route != SeqRoute::Evidence
            && let Some(root) = message.thread_root_id
        {
            connection
                .prepare_cached(
                    "UPDATE messages SET reply_count = COALESCE(reply_count, 0) + 1 WHERE message_id = $root AND chat_id = $chat",
                )?
                .execute(named_params! { "$root": root, "$chat": message.chat_id })?;
        }
        self.touch(connection, message, is_new && route == SeqRoute::LiveFrame)?;
        Ok(true)
    }

    /// The chat's ordering and its unread count, after a message landed. An EDIT moves neither: it
    /// never re-notifies, never bumps a count and never moves the chat's ordering.
    ///
    /// The count is INCREMENTED, never recomputed, and only for a message that is new to this
    /// device, arrived as a live frame, was sent by somebody else and sits above this reader's
    /// marker. Every one of those four is load-bearing: a page would double the badge the list read
    /// already set, an edit would re-notify, your own send would count against you, and a message
    /// below the marker was read before it arrived.
    ///
    /// THE "@" MARK IS THE SERVER'S FILTER, and only that: a message above the marker that NAMES
    /// THIS READER, from somebody else they have not blocked (docs/protocol.md, "The mark on the
    /// chat row"; the Mac's `ChatSyncCoordinator` and the web client ask the same). The C# raised
    /// it for any message carrying mentions — of anybody, from a blocked member, or the reader's
    /// own — which the next `GET /chats` then took away again (measured on .NET 10); the protocol
    /// wins, and the port follows it.
    fn touch(
        &self,
        connection: &Connection,
        message: &MessageDto,
        counts: bool,
    ) -> StoreResult<()> {
        let me = self.reader();
        connection
            .prepare_cached(
                "UPDATE chats
   SET last_message_at = MAX(last_message_at, $at),
       unread_count = unread_count + CASE
           WHEN $counts AND $sender <> $me AND $id > last_read_message_id
           THEN 1 ELSE 0 END,
       mentioned = CASE WHEN $namesMe AND $sender <> $me AND $id > last_read_message_id
                             AND $sender NOT IN (SELECT user_id FROM blocked)
                        THEN 1 ELSE mentioned END
 WHERE chat_id = $chat",
            )?
            .execute(named_params! {
                "$chat": message.chat_id,
                "$id": message.id,
                "$counts": counts && message.edit_seq.is_none(),
                "$sender": message.sender_id,
                "$me": me,
                "$at": if message.edit_seq.is_none() {
                    times::instant(Some(&message.created_at)).unwrap_or(0)
                } else {
                    0
                },
                "$namesMe": message
                    .mentions
                    .as_ref()
                    .is_some_and(|mentions| mentions.iter().any(|mention| mention.user_id == me)),
            })?;
        Ok(())
    }

    /// The chat fields of a row this device holds, or `None`.
    fn held_chat(&self, connection: &Connection, chat_id: i64) -> StoreResult<Option<ChatDto>> {
        Ok(connection
            .prepare_cached("SELECT * FROM chats WHERE chat_id = $id")?
            .query_row(named_params! { "$id": chat_id }, read_chat)
            .optional()?
            .map(|row| row.chat))
    }
}

/// A group's members and admin as parameters — NULL for every other kind, which carries neither —
/// and the families the chat is listed in, NULL from a server that predates several families.
fn group_parameters(chat: &ChatDto) -> (Option<String>, Option<i64>, Option<String>) {
    (
        chat.member_ids.as_ref().map(wire::encode),
        chat.admin_user_id,
        chat.family_ids.as_ref().map(wire::encode),
    )
}

fn by_id(members: &[MemberDto]) -> HashMap<i64, MemberDto> {
    members
        .iter()
        .map(|member| (member.id, member.clone()))
        .collect()
}

/// A JSON column, decoded — `None` for NULL, and for JSON this build cannot read (the C#'s
/// `Wire.Decode` answers null for both).
fn json<T: serde::de::DeserializeOwned>(
    row: &Row<'_>,
    column: &str,
) -> rusqlite::Result<Option<T>> {
    Ok(row
        .get::<_, Option<String>>(column)?
        .and_then(|text| wire::decode(&text)))
}

fn read_chat(row: &Row<'_>) -> rusqlite::Result<ChatRowDto> {
    // Back to the wire's shape: absent rather than 0.
    let optional = |column: &str| -> rusqlite::Result<Option<i64>> {
        let value: i64 = row.get(column)?;
        Ok((value != 0).then_some(value))
    };
    Ok(ChatRowDto {
        chat: ChatDto {
            id: row.get("chat_id")?,
            kind: row.get("kind")?,
            title: row.get("title")?,
            peer_user_id: row.get("peer_user_id")?,
            member_ids: json(row, "member_ids_json")?,
            admin_user_id: row.get("admin_user_id")?,
            family_ids: json(row, "family_ids_json")?,
        },
        last_message: None,
        unread_count: row.get("unread_count")?,
        last_read_message_id: row.get("last_read_message_id")?,
        max_reaction_seq: optional("max_reaction_seq")?,
        max_edit_seq: optional("max_edit_seq")?,
        max_poll_seq: optional("max_poll_seq")?,
        // Absent rather than false.
        mentioned: (row.get::<_, i64>("mentioned")? == 1).then_some(true),
    })
}

fn read_message(row: &Row<'_>) -> rusqlite::Result<MessageDto> {
    let attachments: Option<Vec<AttachmentDto>> = json(row, "attachments_json")?;
    let attachment = attachments
        .as_ref()
        .and_then(|attachments| attachments.first().cloned());
    // The quote as it was delivered, both levels. A row cached before the snapshot was kept (schema
    // step 3) has only the id: it is drawn as a quote with no author and no words until a page
    // redelivers it, which is better than a reply that answers nothing.
    let reply_to = match row.get::<_, Option<String>>("reply_to_json")? {
        Some(quote) => wire::decode::<ReplyToDto>(&quote),
        None => row
            .get::<_, Option<i64>>("reply_to_id")?
            .map(|message_id| ReplyToDto {
                message_id,
                sender_id: 0,
                excerpt: String::new(),
                parent: None,
            }),
    };
    Ok(MessageDto {
        id: row.get("message_id")?,
        chat_id: row.get("chat_id")?,
        sender_id: row.get("sender_id")?,
        client_msg_id: row.get("client_msg_id")?,
        body: row.get::<_, Option<String>>("body")?.unwrap_or_default(),
        created_at: times::rfc3339(row.get("created_at")?).unwrap_or_default(),
        reactions: json::<Vec<ReactionDto>>(row, "reactions_json")?,
        reaction_seq: row.get("reaction_seq")?,
        reply_to,
        thread_root_id: row.get("thread_root_id")?,
        // The C#'s `(int)` cast: a count past i32 wraps there too.
        reply_count: row
            .get::<_, Option<i64>>("reply_count")?
            .map(|replies| replies as i32),
        mentions: json::<Vec<MentionDto>>(row, "mentions_json")?,
        edited_at: times::rfc3339(row.get("edited_at")?),
        edit_seq: row.get("edit_seq")?,
        attachments,
        attachment,
        poll: json::<PollDto>(row, "poll_json")?,
        call: json::<CallRecordDto>(row, "call_json")?,
    })
}

fn read_member(row: &Row<'_>) -> rusqlite::Result<MemberDto> {
    let month: Option<i32> = row.get("birthday_month")?;
    let day: Option<i32> = row.get("birthday_day")?;
    Ok(MemberDto {
        id: row.get("user_id")?,
        username: row.get("username")?,
        display_name: row.get("display_name")?,
        avatar_version: row.get("avatar_version")?,
        role: row.get("role")?,
        has_left: row.get::<_, i64>("has_left")? == 1,
        deleted: row.get::<_, i64>("deleted")? == 1,
        birthday: match (month, day) {
            (Some(month), Some(day)) => Some(crate::protocol::BirthdayDto { month, day }),
            _ => None,
        },
        password_reset: row
            .get::<_, Option<i64>>("password_reset")?
            .map(|reset| reset == 1),
    })
}

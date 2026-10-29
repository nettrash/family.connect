//! One open chat: the window of messages the reader is looking at, and the four things looking at
//! it does — page back, report the read, say you are typing, and send (`Conversation.cs`).
//!
//! THE CACHE IS THE STATE. This model holds no copy of the messages: [`ConversationModel::bubbles`]
//! is a query, so a live frame that lands in the store is drawn by the next read and there is no
//! second list to keep in step. What it DOES hold is what the cache has no business knowing — how
//! far back the reader has paged, which hidden bubbles they asked to see, and when this device last
//! said "typing".
//!
//! THE READ MARKER GOES FORWARD ONLY, AND ONLY WHEN THERE IS SOMETHING TO REPORT. It is monotonic
//! per USER, shared across that person's devices, and the server keeps the maximum ever reported —
//! so a device that reports an older id achieves nothing but a request, and a device that reports
//! on every redraw costs the family's server a request per redraw.
//!
//! A REVEAL BELONGS TO THE CONVERSATION, NOT TO THE BUBBLE. Held here, it outlives the bubble being
//! drawn again — which is the difference between tapping "show" and tapping it again after every
//! arriving message.
//!
//! TYPING IS MOMENTARY. One frame every four seconds at most (the apps' own throttle, inside the
//! server's three-second one), and never while the socket is down: a typing frame is not worth
//! queueing, because by the time it went it would be a lie.
//!
//! Every method takes `&self` (the state it keeps is in cells), so the shell can hold the model in
//! an `Rc` and run a read, a page back and a send side by side on the main context, as the C# ran
//! them on the UI thread. Anything that touches the cache answers a `StoreResult` — `Err` is the
//! cache refusing, the C#'s exception — around the C#'s own answer.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::{
    ApiClient, ApiError, FrameSender, MessageDto, QueueOptions, SendPipeline, client_frames,
    error_codes,
};
use fc_core::rt::Clock;
use fc_core::store::{ChatStore, OutboxRow, OutboxStore, SeqRoute, StoreResult};
use fc_text::reactions::{Reaction, toggle_reaction};
use time::OffsetDateTime;

use crate::new_line_keys;
use crate::poll_voting;
use crate::quotes::QuoteLevel;

/// One message as the conversation draws it: the message, and what the reader may see.
#[derive(Debug, Clone, PartialEq)]
pub struct Bubble {
    pub message: MessageDto,
    /// Sent by somebody this reader has blocked, so it is drawn as the hidden row — a line saying
    /// something was said, and not what. Unlike the chat list's, THIS one can be revealed.
    pub hidden: bool,
    /// Whether the reader asked to see it after all.
    pub revealed: bool,
    /// Whether it is this reader's own.
    pub mine: bool,
}

impl Bubble {
    pub fn new(message: MessageDto, hidden: bool, revealed: bool, mine: bool) -> Self {
        Bubble {
            message,
            hidden,
            revealed,
            mine,
        }
    }

    /// Whether the words are drawn: not hidden, or hidden and asked for.
    pub fn reads(&self) -> bool {
        !self.hidden || self.revealed
    }
}

/// A server's reaction list as `fc_text::reactions` reads one.
pub(crate) fn reactions_of(message: &MessageDto) -> Vec<Reaction> {
    message
        .reactions
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .map(|reaction| Reaction::new(reaction.user_id, &reaction.emoji))
        .collect()
}

/// One open chat.
pub struct ConversationModel {
    chat_id: i64,
    chats: Rc<ChatStore>,
    api: Rc<ApiClient>,
    sending: Rc<SendPipeline>,
    socket: Rc<dyn FrameSender>,
    clock: Rc<dyn Clock>,
    outbox: Option<Rc<OutboxStore>>,
    revealed: RefCell<HashSet<i64>>,
    quote_revealed: RefCell<HashSet<(i64, QuoteLevel)>>,
    window: Cell<usize>,
    said_typing: Cell<Option<OffsetDateTime>>,
    reported: Cell<i64>,
    may_have_older: Cell<bool>,
}

impl std::fmt::Debug for ConversationModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConversationModel")
            .field("chat_id", &self.chat_id)
            .field("window", &self.window.get())
            .finish_non_exhaustive()
    }
}

impl ConversationModel {
    /// How many messages a page is, here and on the wire.
    pub const PAGE: usize = 50;

    /// At most one `typing` frame per chat per this long.
    pub const TYPING_EVERY: Duration = Duration::from_secs(4);

    /// How far back a jump may widen the window: ten pages, and then it says it cannot.
    pub const JUMP_LIMIT: usize = ConversationModel::PAGE * 10;

    /// `new ConversationModel(chatId, chats, api, sending, socket, clock)` — the clock the typing
    /// throttle reads (the C#'s `clock`, `UtcNow` by default). Reads the chat's read marker, as the
    /// C# constructor did. With no outbox, [`pending`](Self::pending) is always empty
    /// ([`with_outbox`](Self::with_outbox) gives it one).
    pub fn new(
        chat_id: i64,
        chats: Rc<ChatStore>,
        api: Rc<ApiClient>,
        sending: Rc<SendPipeline>,
        socket: Rc<dyn FrameSender>,
        clock: Rc<dyn Clock>,
    ) -> StoreResult<Self> {
        let reported = chats
            .chat(chat_id)?
            .map_or(0, |chat| chat.last_read_message_id);
        Ok(ConversationModel {
            chat_id,
            chats,
            api,
            sending,
            socket,
            clock,
            outbox: None,
            revealed: RefCell::new(HashSet::new()),
            quote_revealed: RefCell::new(HashSet::new()),
            window: Cell::new(ConversationModel::PAGE),
            said_typing: Cell::new(None),
            reported: Cell::new(reported),
            may_have_older: Cell::new(true),
        })
    }

    /// The same model, with the outbox its [`pending`](Self::pending) rows come from (the C#'s
    /// `outbox`).
    pub fn with_outbox(mut self, outbox: Rc<OutboxStore>) -> Self {
        self.outbox = Some(outbox);
        self
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// Whether there may be older messages than the ones held: the button's enabled state.
    pub fn may_have_older(&self) -> bool {
        self.may_have_older.get()
    }

    /// The messages the reader is looking at, oldest first.
    pub fn bubbles(&self) -> StoreResult<Vec<Bubble>> {
        let me = self.chats.reader();
        let held = self.chats.messages(self.chat_id, None, self.window.get())?;
        let blocked = self.chats.blocked()?;
        let revealed = self.revealed.borrow();
        // The store answers newest first, because that is the cheap end of the index; a
        // conversation reads the other way.
        Ok(held
            .into_iter()
            .rev()
            .map(|message| {
                let hidden = message.sender_id != me && blocked.contains(&message.sender_id);
                let shown = hidden && revealed.contains(&message.id);
                let mine = message.sender_id == me;
                Bubble::new(message, hidden, shown, mine)
            })
            .collect())
    }

    /// Open it: the newest page. Nothing is fetched when the cache already holds this chat in
    /// sequence — the resync's own catch-up brings it up to date, and a read here would race it.
    /// The list's preview is not holding the chat, nor is the answer to a send: each is one row
    /// with nothing known under it, the catch-up does not page a chat that has only those, and so
    /// it is read from the newest page down.
    pub async fn open(&self) -> StoreResult<Option<ApiError>> {
        self.window.set(ConversationModel::PAGE);
        if self.chats.catch_up_cursor(self.chat_id)?.is_some() {
            return Ok(None);
        }
        let page = match self
            .api
            .messages(self.chat_id, None, Some(page_size()))
            .await
            .into_result()
        {
            Ok(page) => page,
            Err(error) => return Ok(Some(error)),
        };
        let messages = page.messages.unwrap_or_default();
        // History, not catch-up: a reply on it is already in its root's recomputed count.
        self.chats.apply_page(&messages, SeqRoute::Evidence, true)?;
        // A short page IS the beginning of the chat: there is nothing older to ask for.
        self.may_have_older
            .set(messages.len() >= ConversationModel::PAGE);
        Ok(None)
    }

    /// Page BACKWARDS: older than the oldest message held. A history read must never move a
    /// catch-up cursor — it is older news by definition, and a cursor stepped back by it would
    /// re-read what has already been applied (or, worse, skip what has not).
    pub async fn older(&self) -> StoreResult<Option<ApiError>> {
        let held = self.chats.messages(self.chat_id, None, self.window.get())?;
        let Some(oldest) = held.last().map(|message| message.id) else {
            return self.open().await;
        };
        // Widen the window first: what the cache already holds below the oldest DRAWN message is
        // paged in without a request at all, which is what makes reopening a chat cheap.
        let deeper = self
            .chats
            .messages(self.chat_id, Some(oldest), ConversationModel::PAGE)?;
        self.window.set(self.window.get() + ConversationModel::PAGE);
        if !deeper.is_empty() {
            self.may_have_older.set(true);
            return Ok(None);
        }
        let page = match self
            .api
            .messages(self.chat_id, Some(oldest), Some(page_size()))
            .await
            .into_result()
        {
            Ok(page) => page,
            Err(error) => return Ok(Some(error)),
        };
        let messages = page.messages.unwrap_or_default();
        self.chats.apply_page(&messages, SeqRoute::Evidence, true)?;
        self.may_have_older
            .set(messages.len() >= ConversationModel::PAGE);
        Ok(None)
    }

    /// Report what the reader has read, at most once per newest message. Answers whether it
    /// reported anything — a device with nothing new to say makes no request.
    ///
    /// `seen`: the newest message the reader has actually BEEN SHOWN, for a window that is not at
    /// the bottom. `None` means "everything held", which is what a window scrolled to the newest
    /// message means — and what a window scrolled up the history most certainly does not: a reader
    /// who opened a chat at an old position and never came down has not read the rest of it, and a
    /// client that said so would clear a badge nobody looked at. Clamped to what is held, so a
    /// window that names an id from somewhere else cannot report a message this device has never
    /// seen.
    pub async fn read(&self, seen: Option<i64>) -> StoreResult<bool> {
        let held = self
            .chats
            .newest(self.chat_id)?
            .map_or(0, |newest| newest.id);
        let newest = seen.map_or(held, |shown| shown.min(held));
        let marker = self.reported.get().max(
            self.chats
                .chat(self.chat_id)?
                .map_or(0, |chat| chat.last_read_message_id),
        );
        if newest <= marker {
            return Ok(false);
        }
        // Locally FIRST: the badge goes out as the reader looks at it, whatever the network is
        // doing. The server keeps the maximum ever reported, so a repeat is harmless and a failure
        // costs nothing that the next report will not fix.
        self.chats.mark_read(self.chat_id, newest)?;
        self.reported.set(newest);
        let _ = self.api.mark_read(self.chat_id, newest).await;
        Ok(true)
    }

    /// Say that this reader is typing, if it is worth saying. Answers whether a frame went.
    pub async fn typing(&self) -> bool {
        if !self.socket.is_connected() {
            return false;
        }
        let now = self.clock.now();
        if self
            .said_typing
            .get()
            .is_some_and(|last| now - last < ConversationModel::TYPING_EVERY)
        {
            return false;
        }
        self.said_typing.set(Some(now));
        self.socket
            .try_send(client_frames::typing(self.chat_id))
            .await
    }

    /// Send. The row is written down BEFORE anything is tried and the outbox owns it from there, so
    /// an interrupted send is a bubble that can be finished rather than nothing at all. Its lines go
    /// out as line feeds, whatever the text field kept them as (#95). The caller flushes
    /// (`FlushTrigger::Queued`).
    pub fn send(&self, body: &str, options: &QueueOptions) -> StoreResult<OutboxRow> {
        self.sending
            .enqueue(self.chat_id, &new_line_keys::for_wire(body), options)
    }

    /// Send a sticker: ONE staged file, no words, and the flag — a message like any other from here
    /// on, so the outbox, the dedup key, the upload it owes and "Sending on an unreliable network"
    /// are all unchanged (docs/protocol.md, "Sending one"). It may be a reply, which is how one
    /// answers something with a sticker.
    pub fn send_sticker(
        &self,
        staged_file: &str,
        reply_to_message_id: Option<i64>,
    ) -> StoreResult<OutboxRow> {
        self.send(
            "",
            &QueueOptions {
                reply_to_message_id,
                pending_files: vec![staged_file.to_string()],
                sticker: true,
                ..QueueOptions::default()
            },
        )
    }

    /// Send a VIDEO MESSAGE: ONE staged square video with its poster, no words, and the flag
    /// (docs/protocol.md, "Video messages") — the sticker's pattern, so the outbox, the dedup key,
    /// the upload it owes and its retries are a message's. It may be a reply. The caller offers it
    /// only where `GET /families/mine` named `max_round_video_ms`: an older server would ignore the
    /// flag and deliver a square video.
    pub fn send_round(
        &self,
        staged_file: &str,
        reply_to_message_id: Option<i64>,
    ) -> StoreResult<OutboxRow> {
        self.send(
            "",
            &QueueOptions {
                reply_to_message_id,
                pending_files: vec![staged_file.to_string()],
                round: true,
                ..QueueOptions::default()
            },
        )
    }

    /// Show a hidden bubble after all. Per message, and it outlives the redraw.
    pub fn reveal(&self, message_id: i64) {
        self.revealed.borrow_mut().insert(message_id);
    }

    /// Hide it again.
    pub fn hide(&self, message_id: i64) {
        self.revealed.borrow_mut().remove(&message_id);
    }

    /// Show one hidden level of one quote after all — the same one-tap rule as a hidden bubble, per
    /// level, because a quote of a blocked member is hidden at each level on its own
    /// (docs/protocol.md, "Blocking a member").
    pub fn reveal_quote(&self, message_id: i64, level: QuoteLevel) {
        self.quote_revealed.borrow_mut().insert((message_id, level));
    }

    /// Whether that level has been asked for.
    pub fn quote_revealed(&self, message_id: i64, level: QuoteLevel) -> bool {
        self.quote_revealed.borrow().contains(&(message_id, level))
    }

    // ---- finding a message the reader asked for -------------------------------------------------

    /// Whether the window being drawn holds this message.
    pub fn draws(&self, message_id: i64) -> StoreResult<bool> {
        Ok(self
            .chats
            .messages(self.chat_id, None, self.window.get())?
            .iter()
            .any(|message| message.id == message_id))
    }

    /// Widen the window until it draws `message_id`, and answer whether it does.
    ///
    /// FROM THE CACHE ALONE. A click on a quote asks for a message this device was already told
    /// about — the excerpt is there so the quote can be drawn without it — and a request per click
    /// is not what that click is for. So: drawn already, or drawn by widening over rows this device
    /// holds, or not drawn at all, which is what the reader is told. A quote can name a message
    /// retention has swept, or one older than anything this install ever held, and neither is an
    /// error (docs/protocol.md, "Replies").
    pub fn draw_to(&self, message_id: i64) -> StoreResult<bool> {
        if self.draws(message_id)? {
            return Ok(true);
        }
        if self
            .chats
            .message(message_id)?
            .is_none_or(|held| held.chat_id != self.chat_id)
        {
            return Ok(false);
        }
        while self.window.get() < ConversationModel::JUMP_LIMIT {
            // A short read is the end of what the cache holds: widening again would draw the same
            // rows and loop.
            if self
                .chats
                .messages(self.chat_id, None, self.window.get())?
                .len()
                < self.window.get()
            {
                return Ok(false);
            }
            self.window.set(self.window.get() + ConversationModel::PAGE);
            if self.draws(message_id)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // ---- what is still on its way -----------------------------------------------------------------

    /// This chat's own messages that have not landed — waiting, sending, or refused — oldest first.
    /// They are drawn under the newest message until the ack or the feed brings the real one and
    /// the row goes; a refused one stays, with a way to try again or give up.
    pub fn pending(&self) -> StoreResult<Vec<OutboxRow>> {
        match &self.outbox {
            Some(outbox) => outbox.for_chat(self.chat_id),
            None => Ok(Vec::new()),
        }
    }

    /// Try a refused or waiting row again, now — the budget starts over. The caller flushes.
    pub fn retry(&self, client_msg_id: &str) -> StoreResult<()> {
        match &self.outbox {
            Some(outbox) => outbox.retry(client_msg_id),
            None => Ok(()),
        }
    }

    /// Give up on a row. It never reached the server, so nobody else has to know.
    pub fn discard(&self, client_msg_id: &str) -> StoreResult<bool> {
        match &self.outbox {
            Some(outbox) => outbox.discard(client_msg_id),
            None => Ok(false),
        }
    }

    // ---- reactions and edits ----------------------------------------------------------------------

    /// React, or take a reaction off: the server's set is a STATE-SET and not a toggle, so the tap
    /// is decided here — the emoji already held means DELETE, anything else means PUT
    /// (`fc_text::reactions::toggle_reaction`). The answer is the message's whole list, applied as
    /// EVIDENCE: it may not move the chat's catch-up cursor.
    ///
    /// Nothing is drawn before the server answers. An optimistic list would have to live in the
    /// cache without a sequence of its own, and the next frame for the same message would be judged
    /// against a list the server never minted.
    pub async fn react(&self, message_id: i64, emoji: &str) -> StoreResult<Option<ApiError>> {
        let Some(held) = self.chats.message(message_id)? else {
            return Ok(None);
        };
        let toggle = toggle_reaction(&reactions_of(&held), self.chats.reader(), emoji);
        let answer = if toggle.removing {
            self.api.unreact(self.chat_id, message_id).await
        } else {
            self.api.react(self.chat_id, message_id, emoji).await
        };
        match answer.into_result() {
            Err(error) => Ok(Some(error)),
            Ok(answer) => {
                self.chats.apply_reactions(
                    self.chat_id,
                    message_id,
                    answer.reaction_seq,
                    &answer.reactions,
                    SeqRoute::Evidence,
                )?;
                Ok(None)
            }
        }
    }

    // ---- polls ------------------------------------------------------------------------------------

    /// A tap on one of a poll's options: the one held retracts, any other casts (`poll_voting`).
    pub async fn vote(&self, message_id: i64, option_id: i64) -> StoreResult<Option<ApiError>> {
        let Some(held) = self.chats.message(message_id)? else {
            return Ok(None);
        };
        let answer =
            poll_voting::tap(&self.api, &self.chats, self.chat_id, &held, option_id).await?;
        Ok(answer.error().cloned())
    }

    /// Close one of the reader's own polls.
    pub async fn close_poll(&self, message_id: i64) -> StoreResult<Option<ApiError>> {
        let Some(held) = self.chats.message(message_id)? else {
            return Ok(None);
        };
        let answer = poll_voting::close(&self.api, &self.chats, self.chat_id, &held).await?;
        Ok(answer.error().cloned())
    }

    /// Whether a bubble's words may be edited: the reader's own, with words to edit. A call
    /// record's body is a placeholder nobody wrote, and a poll's question is the poll.
    ///
    /// NEVER A STICKER, and said here in as many words rather than left to follow from its empty
    /// body: the server refuses the edit (`validation`), because it would put words on a message
    /// drawn with no bubble to hold them (docs/protocol.md, "And it cannot be edited"). NOR A VIDEO
    /// MESSAGE, for the same reason (docs/protocol.md, "Video messages"; S5.4) — even one some
    /// server stored words beside, which is DRAWN as the ordinary video it otherwise is
    /// (`MessageDto::round_video` asks for no body) but which the server, refusing an edit to any
    /// message whose attachment carries `round: true`, would still refuse.
    pub fn may_edit(bubble: &Bubble) -> bool {
        let message = &bubble.message;
        bubble.mine
            && message.sticker_picture().is_none()
            && !message.media().iter().any(|attachment| attachment.round)
            && message.call.is_none()
            && message.poll.is_none()
            && !message.body.is_empty()
    }

    /// Replace the words of one of the reader's messages. Trimmed, as the server trims them
    /// (`str::trim`); an empty body is refused here, and the words it already has send nothing at
    /// all.
    pub async fn edit(&self, message_id: i64, body: &str) -> StoreResult<Option<ApiError>> {
        let wire = new_line_keys::for_wire(body);
        let trimmed = wire.trim();
        if trimmed.is_empty() {
            return Ok(Some(ApiError::new(
                error_codes::MESSAGE_EMPTY,
                "a message cannot be empty",
                400,
            )));
        }
        if self
            .chats
            .message(message_id)?
            .is_some_and(|held| held.body == trimmed)
        {
            return Ok(None);
        }
        match self
            .api
            .edit_message(self.chat_id, message_id, trimmed)
            .await
            .into_result()
        {
            Err(error) => Ok(Some(error)),
            Ok(answer) => {
                self.chats
                    .apply(&answer.message, SeqRoute::Evidence, false)?;
                Ok(None)
            }
        }
    }
}

/// [`ConversationModel::PAGE`] as the wire's `limit`.
fn page_size() -> i32 {
    i32::try_from(ConversationModel::PAGE).unwrap_or(i32::MAX)
}

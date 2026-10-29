//! One chain of replies on a surface of its own: the root at the top, the replies below it in
//! order, and a composer whose every send answers the root (docs/protocol.md, "Threads"; web
//! `thread_panel.rs`, ios `ThreadView`; `Threads.cs`).
//!
//! OPENED AT ONCE ON WHAT IS HELD, THEN READ. A reply names its root, so a chain opened from one is
//! rooted before anything is asked; the read's first page answers with the ROOT first, whichever
//! message was named, and re-roots it.
//!
//! THE READ STAYS OUT OF THE CACHE. The cache holds pages, and a root older than them or a reply
//! newer would move where the next page starts. Its rows are kept here, and the cache takes only
//! its copies of rows it already holds — their recomputed count and any edit
//! (`ChatStore::refresh`).
//!
//! EACH COPY OF THE ROOT COUNTS A REPLY ONCE. The cache raises its own copy on the routes that
//! count; this surface raises its copy the first time IT sees a reply — whatever brought it — and a
//! row both hold is drawn from the fresher.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

use fc_core::protocol::{ApiClient, ApiError, MessageDto};
use fc_core::store::{ChatStore, OutboxRow, OutboxStore, SeqRoute, StoreResult};
use fc_text::reactions::toggle_reaction;

use crate::conversation::{Bubble, reactions_of};
use crate::quotes::QuoteLevel;

/// One chain of replies on a surface of its own.
pub struct ThreadModel {
    chat_id: i64,
    named: i64,
    chats: Rc<ChatStore>,
    api: Rc<ApiClient>,
    outbox: Option<Rc<OutboxStore>>,
    revealed: RefCell<HashSet<i64>>,
    quote_revealed: RefCell<HashSet<(i64, QuoteLevel)>>,
    /// This surface's copies, by id — which is also the order they are drawn in.
    read: RefCell<BTreeMap<i64, MessageDto>>,
    root_id: Cell<i64>,
    loaded: Cell<bool>,
    failure: RefCell<Option<ApiError>>,
}

impl std::fmt::Debug for ThreadModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadModel")
            .field("chat_id", &self.chat_id)
            .field("root_id", &self.root_id.get())
            .field("read", &self.read.borrow().len())
            .finish_non_exhaustive()
    }
}

impl ThreadModel {
    /// How many rows one thread read asks for.
    pub const PAGE: i32 = 50;

    /// `new ThreadModel(chatId, messageId, chats, api)`: rooted at once on what the cache holds —
    /// a reply names its root; anything else is one. With no outbox, [`pending`](Self::pending) is
    /// always empty ([`with_outbox`](Self::with_outbox) gives it one).
    pub fn new(
        chat_id: i64,
        message_id: i64,
        chats: Rc<ChatStore>,
        api: Rc<ApiClient>,
    ) -> StoreResult<Self> {
        let root_id = chats
            .message(message_id)?
            .and_then(|held| held.thread_root_id)
            .unwrap_or(message_id);
        // What the cache holds of the chain is where the surface starts — not news, so none of it
        // raises the root.
        let read: BTreeMap<i64, MessageDto> = chats
            .thread(root_id)?
            .into_iter()
            .filter(|message| message.chat_id == chat_id)
            .map(|message| (message.id, message))
            .collect();
        Ok(ThreadModel {
            chat_id,
            named: message_id,
            chats,
            api,
            outbox: None,
            revealed: RefCell::new(HashSet::new()),
            quote_revealed: RefCell::new(HashSet::new()),
            read: RefCell::new(read),
            root_id: Cell::new(root_id),
            loaded: Cell::new(false),
            failure: RefCell::new(None),
        })
    }

    /// The same surface, with the outbox its [`pending`](Self::pending) rows come from.
    pub fn with_outbox(mut self, outbox: Rc<OutboxStore>) -> Self {
        self.outbox = Some(outbox);
        self
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// The chain's root — decided from what is held, and settled by the read's first page.
    pub fn root_id(&self) -> i64 {
        self.root_id.get()
    }

    /// True once the whole chain has been read.
    pub fn loaded(&self) -> bool {
        self.loaded.get()
    }

    /// Why the last read stopped, or `None`. What was drawn before stays drawn.
    pub fn failure(&self) -> Option<ApiError> {
        self.failure.borrow().clone()
    }

    /// The chain, page by page — root first, then every reply, `after_id` looped until a short
    /// page.
    pub async fn load(&self) -> StoreResult<Option<ApiError>> {
        let mut after: Option<i64> = None;
        loop {
            let answer = self
                .api
                .thread(self.chat_id, self.named, after, ThreadModel::PAGE)
                .await;
            let page = match answer.into_result() {
                Ok(page) => page.messages.unwrap_or_default(),
                Err(error) => {
                    *self.failure.borrow_mut() = Some(error.clone());
                    return Ok(Some(error));
                }
            };
            if after.is_none()
                && let Some(first) = page.first()
            {
                // The ROOT comes first, whichever message was named.
                self.root_id.set(first.id);
            }
            self.chats.refresh(&page)?;
            {
                let mut read = self.read.borrow_mut();
                for message in &page {
                    let merged = merge(read.get(&message.id), message);
                    read.insert(message.id, merged);
                }
            }
            if page.len() < usize::try_from(ThreadModel::PAGE).unwrap_or(usize::MAX) {
                break;
            }
            // A cursor that cannot move ends the loop, whatever the server says about the page
            // being full.
            let next = page.iter().map(|message| message.id).max().unwrap_or(0);
            if after.is_some_and(|previous| next <= previous) {
                break;
            }
            after = Some(next);
        }
        *self.failure.borrow_mut() = None;
        self.loaded.set(true);
        Ok(None)
    }

    /// The rows, oldest first, each drawn from the fresher of this surface's copy and the cache's
    /// — folding in first whatever the cache gained since: a live reply, this reader's own
    /// delivered one, a catch-up.
    pub fn bubbles(&self) -> StoreResult<Vec<Bubble>> {
        let held: HashMap<i64, MessageDto> = self
            .chats
            .thread(self.root_id.get())?
            .into_iter()
            .filter(|message| message.chat_id == self.chat_id)
            .map(|message| (message.id, message))
            .collect();
        let mut ids: Vec<i64> = held.keys().copied().collect();
        ids.sort_unstable();
        for id in ids {
            if !self.read.borrow().contains_key(&id) {
                self.heard(&held[&id]);
            }
        }
        let me = self.chats.reader();
        let blocked = self.chats.blocked()?;
        let revealed = self.revealed.borrow();
        Ok(self
            .read
            .borrow()
            .values()
            .map(|copy| match held.get(&copy.id) {
                Some(cached) => merge(Some(copy), cached),
                None => copy.clone(),
            })
            .map(|message| {
                let hidden = message.sender_id != me && blocked.contains(&message.sender_id);
                let shown = hidden && revealed.contains(&message.id);
                let mine = message.sender_id == me;
                Bubble::new(message, hidden, shown, mine)
            })
            .collect())
    }

    /// A message this surface has not seen: on it if it belongs — the root, or a reply rooted at it
    /// — and a reply new HERE raises this surface's copy of the root. Answers whether it belonged.
    pub fn heard(&self, message: &MessageDto) -> bool {
        let root_id = self.root_id.get();
        if message.chat_id != self.chat_id
            || (message.id != root_id && message.thread_root_id != Some(root_id))
        {
            return false;
        }
        let mut read = self.read.borrow_mut();
        let is_new = !read.contains_key(&message.id);
        let merged = merge(read.get(&message.id), message);
        read.insert(message.id, merged);
        if is_new
            && message.thread_root_id == Some(root_id)
            && let Some(root) = read.get_mut(&root_id)
        {
            root.reply_count = Some(root.reply_count.unwrap_or(0) + 1);
        }
        true
    }

    /// How many replies the surface draws: every row but the root.
    pub fn replies(&self, bubbles: &[Bubble]) -> usize {
        bubbles
            .iter()
            .filter(|bubble| bubble.message.id != self.root_id.get())
            .count()
    }

    /// This reader's replies still on their way that answer the chain — the root, or any row of
    /// it.
    pub fn pending(&self) -> StoreResult<Vec<OutboxRow>> {
        let Some(outbox) = &self.outbox else {
            return Ok(Vec::new());
        };
        let mut chain: HashSet<i64> = self.read.borrow().keys().copied().collect();
        chain.extend(
            self.chats
                .thread(self.root_id.get())?
                .iter()
                .map(|message| message.id),
        );
        chain.insert(self.root_id.get());
        Ok(outbox
            .for_chat(self.chat_id)?
            .into_iter()
            .filter(|row| {
                row.reply_to_message_id
                    .is_some_and(|answered| chain.contains(&answered))
            })
            .collect())
    }

    /// Show a hidden row after all — this surface's own, for as long as it lives.
    pub fn reveal(&self, message_id: i64) {
        self.revealed.borrow_mut().insert(message_id);
    }

    pub fn hide(&self, message_id: i64) {
        self.revealed.borrow_mut().remove(&message_id);
    }

    /// Show one hidden level of one quote, on this panel — its own reveal, exactly as its hidden
    /// bubbles are its own (docs/protocol.md, "Blocking a member").
    pub fn reveal_quote(&self, message_id: i64, level: QuoteLevel) {
        self.quote_revealed.borrow_mut().insert((message_id, level));
    }

    /// Whether that level has been asked for here.
    pub fn quote_revealed(&self, message_id: i64, level: QuoteLevel) -> bool {
        self.quote_revealed.borrow().contains(&(message_id, level))
    }

    /// Whether this chain draws that message at all — what a click on a quote needs to know.
    pub fn draws(&self, message_id: i64) -> StoreResult<bool> {
        Ok(self
            .bubbles()?
            .iter()
            .any(|bubble| bubble.message.id == message_id))
    }

    /// React on a row of the chain, held by the cache or not: decided against the freshest copy,
    /// and the answer applied to both — the cache as evidence, this surface's copy under its
    /// sequence.
    pub async fn react(&self, message_id: i64, emoji: &str) -> StoreResult<Option<ApiError>> {
        let copy = match self.chats.message(message_id)? {
            Some(held) => held,
            None => match self.read.borrow().get(&message_id) {
                Some(kept) => kept.clone(),
                None => return Ok(None),
            },
        };
        let toggle = toggle_reaction(&reactions_of(&copy), self.chats.reader(), emoji);
        let answer = if toggle.removing {
            self.api.unreact(self.chat_id, message_id).await
        } else {
            self.api.react(self.chat_id, message_id, emoji).await
        };
        let answer = match answer.into_result() {
            Ok(answer) => answer,
            Err(error) => return Ok(Some(error)),
        };
        self.chats.apply_reactions(
            self.chat_id,
            message_id,
            answer.reaction_seq,
            &answer.reactions,
            SeqRoute::Evidence,
        )?;
        if let Some(kept) = self.read.borrow_mut().get_mut(&message_id)
            && answer.reaction_seq >= kept.reaction_seq.unwrap_or(0)
        {
            kept.reactions = Some(answer.reactions);
            kept.reaction_seq = Some(answer.reaction_seq);
        }
        Ok(None)
    }
}

/// Two copies of one row: the words from the one with the newer edit — an older copy never undoes
/// an edit, though it still brings the true count — and the reactions from the one with the newer
/// reaction sequence.
fn merge(kept: Option<&MessageDto>, incoming: &MessageDto) -> MessageDto {
    let Some(kept) = kept else {
        return incoming.clone();
    };
    let mut merged = if incoming.edit_seq.unwrap_or(0) >= kept.edit_seq.unwrap_or(0) {
        incoming.clone()
    } else {
        MessageDto {
            reply_count: incoming.reply_count,
            ..kept.clone()
        }
    };
    if kept.reaction_seq.unwrap_or(0) > merged.reaction_seq.unwrap_or(0) {
        merged.reactions = kept.reactions.clone();
        merged.reaction_seq = kept.reaction_seq;
    }
    merged
}

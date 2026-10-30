//! The family chat's open polls, as they stand — a list of its own, because a decision nobody can
//! find is a decision nobody makes (docs/protocol.md, "Finding the open ones"; `OpenPolls.cs`).
//!
//! OLDEST FIRST, BY MESSAGE ID, and never by `poll_seq`: ordering by the last change would
//! reshuffle the list under the reader every time somebody voted.
//!
//! A PLAIN READ. The rows are kept here and NOT written into the cache: they are messages from
//! anywhere in the chat's history, and the cache holds pages. A poll frame lands in the cache
//! alone, so each row is drawn with whichever state is newer, its own or the cache's.
//!
//! RE-READ AFTER EVERY ACTION. The answer to a vote would patch one row, but a poll its author
//! closed while the list was open belongs off it, and one request gets both right.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use fc_core::protocol::{ApiClient, ApiError, MessageDto};
use fc_core::store::{ChatStore, StoreResult};
use fc_core::text::polls;

use crate::poll_voting::{self, PollAnswer};

/// Which of the two actions a row is asked for.
enum Act {
    Vote(i64),
    Close,
}

/// The family chat's open polls.
pub struct OpenPollsModel {
    chat_id: i64,
    chats: Rc<ChatStore>,
    api: Rc<ApiClient>,
    revealed: RefCell<HashSet<i64>>,
    listed: RefCell<Vec<MessageDto>>,
    loaded: Cell<bool>,
    failure: RefCell<Option<ApiError>>,
}

impl std::fmt::Debug for OpenPollsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenPollsModel")
            .field("chat_id", &self.chat_id)
            .field("listed", &self.listed.borrow().len())
            .finish_non_exhaustive()
    }
}

impl OpenPollsModel {
    pub fn new(chat_id: i64, chats: Rc<ChatStore>, api: Rc<ApiClient>) -> Self {
        OpenPollsModel {
            chat_id,
            chats,
            api,
            revealed: RefCell::new(HashSet::new()),
            listed: RefCell::new(Vec::new()),
            loaded: Cell::new(false),
            failure: RefCell::new(None),
        }
    }

    pub fn chat_id(&self) -> i64 {
        self.chat_id
    }

    /// True once a read has answered; a failed one leaves what was there before.
    pub fn loaded(&self) -> bool {
        self.loaded.get()
    }

    /// Why the last read failed, or `None`.
    pub fn failure(&self) -> Option<ApiError> {
        self.failure.borrow().clone()
    }

    pub async fn load(&self) -> Option<ApiError> {
        match self.api.open_polls(self.chat_id).await.into_result() {
            Err(error) => {
                *self.failure.borrow_mut() = Some(error.clone());
                Some(error)
            }
            Ok(answer) => {
                let mut listed: Vec<MessageDto> = answer
                    .messages
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|message| message.poll.is_some())
                    .collect();
                listed.sort_by_key(|message| message.id);
                *self.listed.borrow_mut() = listed;
                *self.failure.borrow_mut() = None;
                self.loaded.set(true);
                None
            }
        }
    }

    /// The rows, each with the newest state this device knows.
    pub fn messages(&self) -> StoreResult<Vec<MessageDto>> {
        let listed = self.listed.borrow().clone();
        listed.into_iter().map(|row| self.freshest(row)).collect()
    }

    /// A poll by somebody this reader has blocked is the hidden row here too, or the block would
    /// have a second door. Revealing it is this list's, for as long as the list lives.
    pub fn is_hidden(&self, message: &MessageDto) -> StoreResult<bool> {
        Ok(message.sender_id != self.chats.reader()
            && self.chats.is_blocked(message.sender_id)?
            && !self.revealed.borrow().contains(&message.id))
    }

    pub fn reveal(&self, message_id: i64) {
        self.revealed.borrow_mut().insert(message_id);
    }

    /// The badge: every poll the cache holds for this chat and every one listed, counted once.
    pub fn unanswered(&self) -> StoreResult<usize> {
        let held = self.chats.polls(self.chat_id)?;
        let blocked = self.chats.blocked()?;
        Ok(polls::unanswered(
            &held,
            &self.listed.borrow(),
            self.chats.reader(),
            |user_id| blocked.contains(&user_id),
        ))
    }

    pub async fn vote(&self, message_id: i64, option_id: i64) -> StoreResult<Option<ApiError>> {
        self.act(message_id, Act::Vote(option_id)).await
    }

    pub async fn close(&self, message_id: i64) -> StoreResult<Option<ApiError>> {
        self.act(message_id, Act::Close).await
    }

    async fn act(&self, message_id: i64, act: Act) -> StoreResult<Option<ApiError>> {
        let Some(row) = self
            .messages()?
            .into_iter()
            .find(|row| row.id == message_id)
        else {
            return Ok(None);
        };
        let answer = match act {
            Act::Vote(option_id) => {
                poll_voting::tap(&self.api, &self.chats, self.chat_id, &row, option_id).await?
            }
            Act::Close => poll_voting::close(&self.api, &self.chats, self.chat_id, &row).await?,
        };
        let error = match answer {
            // Nothing was sent — a closed poll, somebody else's to close — so there is nothing to
            // re-read.
            PollAnswer::Unsent => return Ok(None),
            PollAnswer::Failed(error) => Some(error),
            PollAnswer::Applied(poll) => {
                // Kept even if the re-read below fails: the vote happened.
                for held in self.listed.borrow_mut().iter_mut() {
                    if held.id == message_id
                        && held
                            .poll
                            .as_ref()
                            .is_none_or(|was| poll.poll_seq >= was.poll_seq)
                    {
                        held.poll = Some(poll.clone());
                    }
                }
                None
            }
        };
        let reread = self.load().await;
        Ok(error.or(reread))
    }

    fn freshest(&self, row: MessageDto) -> StoreResult<MessageDto> {
        let listed_seq = row.poll.as_ref().map_or(0, |poll| poll.poll_seq);
        Ok(
            match self.chats.message(row.id)?.and_then(|held| held.poll) {
                Some(cached) if cached.poll_seq > listed_seq => MessageDto {
                    poll: Some(cached),
                    ..row
                },
                _ => row,
            },
        )
    }
}

//! A vote, a retraction and a close — for a poll in the conversation and for one on the open-polls
//! list, which may be a message this device has never paged back to (docs/protocol.md, "Polls";
//! `PollVoting.cs`).
//!
//! THE TAP IS DECIDED HERE. The server's vote is a state-set: re-PUTting the option already held is
//! a no-op, so a client that always PUT would draw a row as "yours, tap to clear" and then do
//! nothing when it was tapped. The option held means DELETE; any other means PUT.
//!
//! THE ANSWER IS EVIDENCE. It is the poll's whole state, applied under the message's own `poll_seq`
//! guard and never moving the chat's catch-up cursor — the same rule a reaction's answer keeps.
//! Nothing is drawn before it arrives: an optimistic state would have no sequence the server
//! minted, and the next frame would be judged against it.

use fc_core::protocol::{ApiClient, ApiError, ApiResult, MessageDto, MessagePollDto, PollDto};
use fc_core::store::{ChatStore, SeqRoute, StoreResult};
use fc_core::text::polls;

/// What a tap or a close came to (the C#'s `(ApiError? Error, PollDto? Poll)`).
#[derive(Debug, Clone, PartialEq)]
pub enum PollAnswer {
    /// Nothing was sent: a closed poll, an option it never had, somebody else's to close.
    Unsent,
    /// The server refused, or the request never arrived.
    Failed(ApiError),
    /// The poll's whole state as the server now holds it, applied to the cache.
    Applied(PollDto),
}

impl PollAnswer {
    pub fn error(&self) -> Option<&ApiError> {
        match self {
            PollAnswer::Failed(error) => Some(error),
            _ => None,
        }
    }

    pub fn poll(&self) -> Option<&PollDto> {
        match self {
            PollAnswer::Applied(poll) => Some(poll),
            _ => None,
        }
    }
}

/// A tap on one of a poll's options: the one held retracts, any other casts.
pub async fn tap(
    api: &ApiClient,
    chats: &ChatStore,
    chat_id: i64,
    message: &MessageDto,
    option_id: i64,
) -> StoreResult<PollAnswer> {
    let Some(poll) = message.poll.as_ref().filter(|_| polls::votable(message)) else {
        return Ok(PollAnswer::Unsent);
    };
    if poll.options.iter().all(|option| option.id != option_id) {
        return Ok(PollAnswer::Unsent);
    }
    let answer = if polls::tap_retracts(poll, option_id, chats.reader()) {
        api.unvote(chat_id, message.id).await
    } else {
        api.vote(chat_id, message.id, option_id).await
    };
    settle(chats, chat_id, answer)
}

/// The author's alone — the family owner does not outrank authorship here — and one-way.
pub async fn close(
    api: &ApiClient,
    chats: &ChatStore,
    chat_id: i64,
    message: &MessageDto,
) -> StoreResult<PollAnswer> {
    if !may_close(message, chats.reader()) {
        return Ok(PollAnswer::Unsent);
    }
    let answer = api.close_poll(chat_id, message.id).await;
    settle(chats, chat_id, answer)
}

/// Whether this reader is offered "Close poll": their own, numbered, still open.
pub fn may_close(message: &MessageDto, reader: i64) -> bool {
    message.sender_id == reader && polls::votable(message)
}

fn settle(
    chats: &ChatStore,
    chat_id: i64,
    answer: ApiResult<MessagePollDto>,
) -> StoreResult<PollAnswer> {
    match answer.into_result() {
        Err(error) => Ok(PollAnswer::Failed(error)),
        Ok(answer) => {
            chats.apply_poll(chat_id, answer.message_id, &answer.poll, SeqRoute::Evidence)?;
            Ok(PollAnswer::Applied(answer.poll))
        }
    }
}

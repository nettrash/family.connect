//! What `PollsModelTests` builds: the roster its class constructor writes, and the poll documents
//! its tests hold and serve.

use fc_core::protocol::{MemberDto, MessageDto, PollDto, PollOptionDto};
use fc_core::store::{SeqRoute, StoreResult};

use super::chat::{CHAT, ChatRig};

pub const SENT: &str = "2026-09-14T10:00:00Z";
pub const NOT_MEMBER: &str = r#"{"error": {"code": "not_chat_member", "message": "no"}}"#;

/// The class constructor's roster: Anna (the reader), Bob, Dora (blocked), a nameless member and a
/// deleted Gus; Eve and Fay gone.
pub fn rig() -> StoreResult<ChatRig> {
    let rig = ChatRig::new()?;
    rig.chats.replace_members(
        &[
            MemberDto::new(7, "anna", "Anna"),
            MemberDto::new(11, "bob", "Bob"),
            MemberDto::new(13, "dora", "Dora"),
            MemberDto::new(16, "nameless", ""),
            MemberDto {
                deleted: true,
                ..MemberDto::new(17, "ghost", "Gus")
            },
        ],
        &[
            MemberDto {
                has_left: true,
                deleted: true,
                ..MemberDto::new(14, "gone", "Eve")
            },
            MemberDto {
                has_left: true,
                ..MemberDto::new(15, "left", "Fay")
            },
        ],
    )?;
    rig.chats.set_blocked(13, true)?;
    Ok(rig)
}

/// `Poll(seq, closed, pizza, pasta)`.
pub fn poll(seq: i64, closed: bool, pizza: &[i64], pasta: &[i64]) -> PollDto {
    PollDto {
        poll_seq: seq,
        closed,
        options: vec![
            PollOptionDto {
                id: 1,
                text: "Pizza".to_string(),
                votes: pizza.to_vec(),
            },
            PollOptionDto {
                id: 2,
                text: "Pasta".to_string(),
                votes: pasta.to_vec(),
            },
        ],
    }
}

fn votes(voters: &[i64]) -> String {
    voters
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `State(messageId, seq, closed, pizza, pasta)`: a vote's or a close's answer.
pub fn state(message_id: i64, seq: i64, closed: bool, pizza: &[i64], pasta: &[i64]) -> String {
    format!(
        r#"{{"message_id": {message_id}, "poll": {{"poll_seq": {seq}, "closed": {closed}, "options": [{{"id": 1, "text": "Pizza", "votes": [{}]}}, {{"id": 2, "text": "Pasta", "votes": [{}]}}]}}}}"#,
        votes(pizza),
        votes(pasta)
    )
}

/// `Row(id, sender, seq, pizza)`: one row of the open-polls list.
pub fn row(id: i64, sender: i64, seq: i64, pizza: &[i64]) -> String {
    format!(
        r#"{{"id": {id}, "chat_id": 42, "sender_id": {sender}, "body": "Q{id}", "created_at": "{SENT}", "poll": {{"poll_seq": {seq}, "closed": false, "options": [{{"id": 1, "text": "Pizza", "votes": [{}]}}, {{"id": 2, "text": "Pasta", "votes": []}}]}}}}"#,
        votes(pizza)
    )
}

/// `List(rows…)`.
pub fn list(rows: &[String]) -> String {
    format!(r#"{{"messages": [{}]}}"#, rows.join(", "))
}

/// `Hold(id, poll, sender)`: a poll message in the cache, as a catch-up page brings one.
pub fn hold(rig: &ChatRig, id: i64, poll: PollDto, sender: i64) -> StoreResult<()> {
    rig.chats.apply(
        &MessageDto {
            poll: Some(poll),
            ..MessageDto::new(id, CHAT, sender, None, "Dinner?", SENT)
        },
        SeqRoute::CatchUpPage,
        true,
    )?;
    Ok(())
}

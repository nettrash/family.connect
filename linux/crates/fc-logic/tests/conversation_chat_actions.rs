//! `ChatActionsTests`, the conversation's half: what is still on its way, reactions, edits, and who
//! may edit. Who is typing is `typing_chat_actions.rs`; what a reply quotes, `quotes_chat_actions.rs`.

mod common;

use std::rc::Rc;

use fc_core::protocol::{
    CallRecordDto, MessageDto, PollDto, QueueOptions, ReactionDto, error_codes,
};
use fc_core::store::{OutboxRow, SeqRoute, StoreResult};
use fc_logic::conversation::{Bubble, ConversationModel};
use fc_text::emoji::QUICK_REACTIONS;
use fc_text::reactions::{Reaction, my_reaction};
use serde_json::Value;
use time::OffsetDateTime;

use common::chat::{CHAT, ChatRig, ME, actions_rig as rig, socket};
use common::server::Server;

const SENT: &str = "2026-09-13T10:00:00Z";
const HEART: &str = QUICK_REACTIONS[0];
const UP: &str = QUICK_REACTIONS[1];

fn send(chat: &ConversationModel, body: &str) -> StoreResult<OutboxRow> {
    chat.send(body, &QueueOptions::default())
}

fn field(body: &str, name: &str) -> String {
    let parsed: Value = serde_json::from_str(body).expect("a JSON body");
    parsed[name].as_str().expect("a string").to_string()
}

fn mine(rig: &ChatRig, id: i64) -> StoreResult<Option<String>> {
    let reactions: Vec<Reaction> = rig
        .chats
        .message(id)?
        .unwrap()
        .reactions
        .unwrap_or_default()
        .iter()
        .map(|reaction| Reaction::new(reaction.user_id, &reaction.emoji))
        .collect();
    Ok(my_reaction(&reactions, ME).map(str::to_string))
}

fn dinner() -> MessageDto {
    MessageDto {
        reactions: Some(vec![ReactionDto::new(9, UP)]),
        reaction_seq: Some(3),
        ..MessageDto::new(1, CHAT, 9, None, "Dinner", SENT)
    }
}

const EDITED: &str = r#"{"message": {"id": 1, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "Dinner at 8?", "created_at": "2026-09-13T10:00:00Z", "edited_at": "2026-09-13T10:05:00Z", "edit_seq": 9}}"#;

#[test]
fn what_is_still_on_its_way_is_this_chats_own_and_can_be_retried_or_given_up() -> StoreResult<()> {
    let rig = rig()?;
    let chat = rig.open(&Server::new())?;
    let first = send(&chat, "Dinner at 7?")?;
    let second = send(&chat, "…or 8?")?;
    rig.outbox.queue(&OutboxRow {
        queued_at: Some(OffsetDateTime::UNIX_EPOCH),
        ..OutboxRow::new("elsewhere", 99, "not here")
    })?;

    let keys = |chat: &ConversationModel| -> StoreResult<Vec<String>> {
        Ok(chat
            .pending()?
            .into_iter()
            .map(|row| row.client_msg_id)
            .collect())
    };
    assert_eq!(
        vec![first.client_msg_id.clone(), second.client_msg_id.clone()],
        keys(&chat)?
    );

    rig.outbox
        .refuse(&second.client_msg_id, error_codes::BLOCKED)?;
    assert!(chat.pending()?[1].failed());
    chat.retry(&second.client_msg_id)?;
    assert!(!chat.pending()?[1].failed());

    assert!(chat.discard(&first.client_msg_id)?);
    assert_eq!(vec![second.client_msg_id], keys(&chat)?);
    Ok(())
}

#[test]
fn a_conversation_without_an_outbox_has_nothing_pending() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    let api = rig.api(&server);
    let socket = socket(false);
    let sending = rig.pipeline(&socket, &api);
    let chat = ConversationModel::new(
        CHAT,
        rig.chats.clone(),
        api,
        sending,
        Rc::new(socket),
        Rc::new(rig.clock.clone()),
    )?;
    send(&chat, "queued all the same")?;
    assert!(chat.pending()?.is_empty());
    Ok(())
}

/// THE TAP IS DECIDED HERE: an emoji the reader does not hold is a PUT with it, the one they hold
/// is a DELETE — and the answer, the whole list, is what the cache takes.
#[test]
fn reacting_sets_what_is_not_mine_and_removes_what_is() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.apply(&dinner(), SeqRoute::CatchUpPage, true)?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/1/reaction",
        &[
            (
                200,
                Some(&format!(
                    r#"{{"message_id": 1, "reaction_seq": 4, "reactions": [{{"user_id": 9, "emoji": "{UP}"}}, {{"user_id": 7, "emoji": "{HEART}"}}]}}"#
                )),
            ),
            (
                200,
                Some(&format!(
                    r#"{{"message_id": 1, "reaction_seq": 5, "reactions": [{{"user_id": 9, "emoji": "{UP}"}}]}}"#
                )),
            ),
        ],
    );
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.react(1, HEART))?);
    // The body is JSON, and the encoder may escape an emoji: what the server reads is the decoded
    // string, so that is what is compared.
    let bodies = server.bodies();
    assert_eq!(1, bodies.len());
    assert_eq!(HEART, field(&bodies[0], "emoji"));
    assert_eq!(Some(HEART.to_string()), mine(&rig, 1)?);
    assert_eq!(Some(4), rig.chats.message(1)?.unwrap().reaction_seq);

    assert_eq!(None, rig.drive(chat.react(1, HEART))?);
    // A DELETE carries no body, so there is still exactly one.
    assert_eq!(1, server.bodies().len());
    assert_eq!(2, server.asked().len());
    assert_eq!(None, mine(&rig, 1)?);
    assert_eq!(Some(5), rig.chats.message(1)?.unwrap().reaction_seq);
    // The answer to this client's own write is EVIDENCE: the chat's catch-up cursor stays where the
    // frames and pages left it, or the next catch-up would skip states this device never saw.
    assert!(rig.chats.chat(CHAT)?.unwrap().max_reaction_seq.unwrap_or(0) < 4);
    Ok(())
}

#[test]
fn a_refused_reaction_changes_nothing() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.apply(&dinner(), SeqRoute::CatchUpPage, true)?;
    let server = Server::new();
    server.on_status(
        "/chats/42/messages/1/reaction",
        403,
        Some(r#"{"error": {"code": "blocked", "message": "no"}}"#),
    );
    let chat = rig.open(&server)?;

    let error = rig.drive(chat.react(1, HEART))?;

    assert_eq!(
        Some(error_codes::BLOCKED),
        error.as_ref().map(|error| error.code.as_str())
    );
    let held = rig.chats.message(1)?.unwrap();
    assert_eq!(1, held.reactions.unwrap().len());
    assert_eq!(Some(3), held.reaction_seq);
    Ok(())
}

#[test]
fn editing_sends_trimmed_words_and_the_words_it_already_has_send_nothing() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.apply(
        &MessageDto::new(1, CHAT, ME, Some("c-1"), "Dinner at 7?", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let server = Server::new();
    server.on("/chats/42/messages/1", EDITED);
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.edit(1, "  Dinner at 8?  "))?);
    let bodies = server.bodies();
    assert_eq!(1, bodies.len());
    assert_eq!("Dinner at 8?", field(&bodies[0], "body"));
    let held = rig.chats.message(1)?.unwrap();
    assert_eq!("Dinner at 8?", held.body);
    assert_eq!(Some(9), held.edit_seq);
    assert!(rig.chats.chat(CHAT)?.unwrap().max_edit_seq.unwrap_or(0) < 9);

    assert_eq!(None, rig.drive(chat.edit(1, "Dinner at 8?"))?);
    assert_eq!(
        Some(error_codes::MESSAGE_EMPTY),
        rig.drive(chat.edit(1, "   "))?
            .as_ref()
            .map(|error| error.code.as_str())
    );
    assert_eq!(1, server.asked().len());
    Ok(())
}

/// #95: a text box keeps a line break as a lone CR; a send and an edit put a LF on the wire, as
/// every client does.
#[test]
fn lines_go_out_as_line_feeds() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.apply(
        &MessageDto::new(1, CHAT, ME, Some("c-1"), "Dinner\nat 7?", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let server = Server::new();
    server.on(
        "/chats/42/messages/1",
        r#"{"message": {"id": 1, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-1", "body": "Dinner\nat 8?", "created_at": "2026-09-13T10:00:00Z", "edited_at": "2026-09-13T10:05:00Z", "edit_seq": 9}}"#,
    );
    let chat = rig.open(&server)?;

    assert_eq!(
        "Milk\neggs\n\nbread",
        send(&chat, "Milk\reggs\r\rbread")?.body
    );

    // The words it already has, typed again in a text box, send nothing.
    assert_eq!(None, rig.drive(chat.edit(1, "Dinner\rat 7?"))?);
    assert!(server.asked().is_empty());
    assert_eq!(None, rig.drive(chat.edit(1, "Dinner\rat 8?"))?);
    let bodies = server.bodies();
    assert_eq!(1, bodies.len());
    assert_eq!("Dinner\nat 8?", field(&bodies[0], "body"));
    Ok(())
}

/// The answer to an edit is one message with nothing known under it: held and drawn, and not where
/// the next catch-up starts (docs/protocol.md, "Best-effort delivery", step 3).
#[test]
fn an_edits_answer_is_not_where_the_catch_up_starts() -> StoreResult<()> {
    let rig = rig()?;
    rig.chats.apply(
        &MessageDto::new(1, CHAT, ME, Some("c-1"), "Dinner at 7?", SENT),
        SeqRoute::CatchUpPage,
        false,
    )?;
    let server = Server::new();
    server.on("/chats/42/messages/1", EDITED);
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.edit(1, "Dinner at 8?"))?);

    assert_eq!("Dinner at 8?", rig.chats.message(1)?.unwrap().body);
    assert_eq!(None, rig.chats.catch_up_cursor(CHAT)?);
    Ok(())
}

#[test]
fn only_the_readers_own_words_may_be_edited() {
    let of = |sender: i64, body: &str, call: Option<CallRecordDto>, poll: Option<PollDto>| {
        Bubble::new(
            MessageDto {
                call,
                poll,
                ..MessageDto::new(1, CHAT, sender, None, body, SENT)
            },
            false,
            false,
            sender == ME,
        )
    };
    let missed = CallRecordDto {
        outcome: "missed".to_string(),
        ..CallRecordDto::default()
    };

    assert!(ConversationModel::may_edit(&of(ME, "words", None, None)));
    assert!(!ConversationModel::may_edit(&of(11, "words", None, None)));
    assert!(!ConversationModel::may_edit(&of(ME, "", None, None)));
    assert!(!ConversationModel::may_edit(&of(
        ME,
        "words",
        Some(missed),
        None
    )));
    assert!(!ConversationModel::may_edit(&of(
        ME,
        "words",
        None,
        Some(PollDto {
            poll_seq: 1,
            ..PollDto::default()
        })
    )));
}

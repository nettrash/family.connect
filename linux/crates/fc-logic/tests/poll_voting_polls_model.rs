//! `PollsModelTests`, the tap's half (`PollVoting`, through the conversation): what a tap sends and
//! who may close. The open-polls list is `open_polls_polls_model.rs`; the draft and the words were
//! ported with layer 0 (`poll_draft_polls_model.rs`, `poll_text_polls_model.rs`).

mod common;

use fc_core::protocol::{MessageDto, PollDto, error_codes};
use fc_core::store::{SeqRoute, StoreResult};
use fc_core::text::polls;
use fc_logic::poll_voting;

use common::chat::{CHAT, ME};
use common::polls::{SENT, hold, poll, rig, state};
use common::server::Server;

fn my_option(poll: &PollDto) -> Option<i64> {
    polls::my_option(poll, ME)
}

/// THE TAP IS DECIDED HERE: an option not held is a PUT naming it, the one held is a DELETE — and
/// the answer, the whole poll, is what the cache takes, as evidence that moves no cursor.
#[test]
fn a_tap_casts_what_is_not_held_and_retracts_what_is() -> StoreResult<()> {
    let rig = rig()?;
    hold(&rig, 1, poll(3, false, &[], &[9]), 9)?;
    let server = Server::new();
    server.then(
        "/chats/42/messages/1/vote",
        &[
            (200, Some(&state(1, 4, false, &[ME], &[9]))),
            (200, Some(&state(1, 5, false, &[], &[9]))),
        ],
    );
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.vote(1, 1))?);
    assert_eq!(vec!["{\"option_id\":1}"], server.bodies());
    let held = rig.chats.message(1)?.unwrap().poll.unwrap();
    assert_eq!(Some(1), my_option(&held));
    assert_eq!(4, held.poll_seq);

    assert_eq!(None, rig.drive(chat.vote(1, 1))?);
    // A DELETE carries no body, so there is still exactly one.
    assert_eq!(1, server.bodies().len());
    assert_eq!(2, server.asked().len());
    let held = rig.chats.message(1)?.unwrap().poll.unwrap();
    assert_eq!(None, my_option(&held));
    assert_eq!(5, held.poll_seq);
    assert!(rig.chats.chat(CHAT)?.unwrap().max_poll_seq.unwrap_or(0) < 4);
    Ok(())
}

#[test]
fn changing_your_mind_puts_the_other_option() -> StoreResult<()> {
    let rig = rig()?;
    hold(&rig, 1, poll(3, false, &[ME], &[]), 9)?;
    let server = Server::new();
    server.on("/chats/42/messages/1/vote", &state(1, 4, false, &[], &[ME]));

    assert_eq!(None, rig.drive(rig.open(&server)?.vote(1, 2))?);

    assert_eq!(vec!["{\"option_id\":2}"], server.bodies());
    assert_eq!(
        Some(2),
        my_option(&rig.chats.message(1)?.unwrap().poll.unwrap())
    );
    Ok(())
}

/// A closed poll is a result, an option the poll never had is nothing to send, and a message not
/// held is not a poll.
#[test]
fn nothing_is_sent_for_a_closed_poll_a_strange_option_or_a_message_that_is_not_a_poll()
-> StoreResult<()> {
    let rig = rig()?;
    hold(&rig, 1, poll(3, true, &[], &[]), 9)?;
    hold(&rig, 2, poll(3, false, &[], &[]), 9)?;
    rig.chats.apply(
        &MessageDto::new(3, CHAT, 9, None, "just words", SENT),
        SeqRoute::CatchUpPage,
        true,
    )?;
    let server = Server::new();
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.vote(1, 1))?);
    assert_eq!(None, rig.drive(chat.vote(2, 9))?);
    assert_eq!(None, rig.drive(chat.vote(3, 1))?);
    assert_eq!(None, rig.drive(chat.vote(77, 1))?);
    assert_eq!(None, rig.drive(chat.close_poll(3))?);
    assert_eq!(None, rig.drive(chat.close_poll(77))?);

    assert!(server.asked().is_empty());
    Ok(())
}

#[test]
fn a_refused_vote_changes_nothing() -> StoreResult<()> {
    let rig = rig()?;
    hold(&rig, 1, poll(3, false, &[], &[]), 9)?;
    let server = Server::new();
    server.on_status(
        "/chats/42/messages/1/vote",
        409,
        Some(r#"{"error": {"code": "poll_closed", "message": "closed"}}"#),
    );

    let error = rig.drive(rig.open(&server)?.vote(1, 1))?;

    assert_eq!(
        Some(error_codes::POLL_CLOSED),
        error.as_ref().map(|error| error.code.as_str())
    );
    let held = rig.chats.message(1)?.unwrap().poll.unwrap();
    assert_eq!(3, held.poll_seq);
    assert_eq!(None, my_option(&held));
    Ok(())
}

/// Closing is the AUTHOR's — the family owner does not outrank authorship — and only while the poll
/// is open.
#[test]
fn only_the_author_closes_an_open_poll() -> StoreResult<()> {
    let rig = rig()?;
    hold(&rig, 1, poll(3, false, &[], &[]), ME)?;
    hold(&rig, 2, poll(3, false, &[], &[]), 9)?;
    let server = Server::new();
    server.on(
        "/chats/42/messages/1/poll/close",
        &state(1, 6, true, &[], &[]),
    );
    let chat = rig.open(&server)?;

    assert_eq!(None, rig.drive(chat.close_poll(2))?);
    assert!(server.asked().is_empty());

    assert_eq!(None, rig.drive(chat.close_poll(1))?);
    assert_eq!(1, server.asked().len());
    let held = rig.chats.message(1)?.unwrap().poll.unwrap();
    assert!(held.closed);
    assert_eq!(6, held.poll_seq);

    // Closed now: there is nothing more to close.
    assert_eq!(None, rig.drive(chat.close_poll(1))?);
    assert_eq!(1, server.asked().len());

    let mine = MessageDto {
        poll: Some(poll(1, false, &[], &[])),
        ..MessageDto::new(9, CHAT, ME, None, "Q", SENT)
    };
    assert!(poll_voting::may_close(&mine, ME));
    assert!(!poll_voting::may_close(&mine, 11));
    assert!(!poll_voting::may_close(
        &MessageDto {
            id: 0,
            ..mine.clone()
        },
        ME
    ));
    assert!(!poll_voting::may_close(
        &MessageDto {
            poll: Some(PollDto {
                closed: true,
                ..mine.poll.clone().unwrap()
            }),
            ..mine.clone()
        },
        ME
    ));
    Ok(())
}

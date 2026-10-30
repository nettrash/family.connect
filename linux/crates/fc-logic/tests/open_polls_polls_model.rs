//! `PollsModelTests`, the open-polls list's half (`OpenPollsModel`): oldest first, the newest state
//! drawn, the badge, and the re-read after every action.

mod common;

use fc_core::protocol::error_codes;
use fc_core::store::StoreResult;
use fc_core::text::polls;
use fc_logic::open_polls::OpenPollsModel;

use common::chat::{CHAT, ChatRig, ME};
use common::polls::{NOT_MEMBER, hold, list, poll, rig, row, state};
use common::server::Server;

fn model(rig: &ChatRig, server: &Server) -> OpenPollsModel {
    OpenPollsModel::new(CHAT, rig.chats.clone(), rig.api(server))
}

/// Oldest first by id, whatever order the answer came in; each row drawn with the newest state this
/// device knows; a blocked member's poll hidden until revealed.
#[test]
fn the_list_is_oldest_first_newest_state_and_hides_the_blocked() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server.on(
        "/chats/42/polls/open",
        &list(&[row(30, 9, 2, &[]), row(10, 13, 1, &[]), row(20, ME, 3, &[])]),
    );
    let model = model(&rig, &server);
    assert!(!model.loaded());

    assert_eq!(None, rig.drive(model.load()));

    assert!(model.loaded());
    assert_eq!(
        vec![10, 20, 30],
        model
            .messages()?
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>()
    );
    // A frame reached the cache after the list was read: the newer state is the one drawn...
    hold(&rig, 30, poll(4, false, &[ME], &[]), 9)?;
    assert_eq!(
        Some(1),
        polls::my_option(model.messages()?[2].poll.as_ref().unwrap(), ME)
    );
    // ...and an older one is not.
    hold(&rig, 20, poll(2, false, &[11], &[]), 9)?;
    let second = model.messages()?.remove(1).poll.unwrap();
    assert_eq!(None, polls::my_option(&second, ME));
    assert_eq!(3, second.poll_seq);

    assert!(model.is_hidden(&model.messages()?[0])?);
    assert!(!model.is_hidden(&model.messages()?[1])?);
    model.reveal(10);
    assert!(!model.is_hidden(&model.messages()?[0])?);
    Ok(())
}

/// The badge counts the cache's polls and the list's, once each: here only the reader's own, still
/// unanswered.
#[test]
fn the_badge_counts_what_is_still_unanswered() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server.on(
        "/chats/42/polls/open",
        &list(&[row(30, 9, 2, &[]), row(10, 13, 1, &[]), row(20, ME, 3, &[])]),
    );
    let model = model(&rig, &server);
    hold(&rig, 30, poll(4, false, &[ME], &[]), 9)?;
    hold(&rig, 40, poll(1, false, &[], &[]), 9)?;
    assert_eq!(1, model.unanswered()?);

    rig.drive(model.load());

    // 20 (the reader's own) and 40; 30 was answered in the newer copy, 10 is a blocked member's.
    assert_eq!(2, model.unanswered()?);
    Ok(())
}

/// A vote from the list is re-read after — and a re-read that fails keeps the vote it answered,
/// says why, and leaves the message out of the cache it never belonged to.
#[test]
fn a_vote_on_the_list_is_re_read_and_survives_a_re_read_that_fails() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server
        .then(
            "/chats/42/polls/open",
            &[
                (200, Some(&list(&[row(10, 9, 1, &[])]))),
                (403, Some(NOT_MEMBER)),
            ],
        )
        .on(
            "/chats/42/messages/10/vote",
            &state(10, 2, false, &[ME], &[]),
        );
    let model = model(&rig, &server);
    rig.drive(model.load());

    let error = rig.drive(model.vote(10, 1))?;

    assert_eq!(
        Some(error_codes::NOT_CHAT_MEMBER),
        error.map(|e| e.code).as_deref()
    );
    assert_eq!(
        Some(error_codes::NOT_CHAT_MEMBER),
        model.failure().map(|e| e.code).as_deref()
    );
    assert_eq!(
        2,
        server
            .asked()
            .iter()
            .filter(|path| path.starts_with("/api/v1/chats/42/polls/open"))
            .count()
    );
    let rows = model.messages()?;
    assert_eq!(1, rows.len());
    let held = rows[0].poll.as_ref().unwrap();
    assert_eq!(2, held.poll_seq);
    assert_eq!(Some(1), polls::my_option(held, ME));
    assert_eq!(None, rig.chats.message(10)?);
    Ok(())
}

/// An answer OLDER than the row the list holds does not wind it back, even when the re-read after
/// it fails.
#[test]
fn an_older_answer_does_not_wind_the_list_back() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server
        .then(
            "/chats/42/polls/open",
            &[
                (200, Some(&list(&[row(10, 9, 5, &[11])]))),
                (403, Some(NOT_MEMBER)),
            ],
        )
        .on(
            "/chats/42/messages/10/vote",
            &state(10, 3, false, &[ME], &[]),
        );
    let model = model(&rig, &server);
    rig.drive(model.load());

    rig.drive(model.vote(10, 1))?;

    let rows = model.messages()?;
    assert_eq!(1, rows.len());
    let held = rows[0].poll.as_ref().unwrap();
    assert_eq!(5, held.poll_seq);
    assert_eq!(vec![11], held.options[0].votes);
    Ok(())
}

#[test]
fn closing_from_the_list_takes_the_poll_off_it() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server
        .then(
            "/chats/42/polls/open",
            &[
                (200, Some(&list(&[row(20, ME, 1, &[]), row(30, 9, 1, &[])]))),
                (200, Some(&list(&[row(30, 9, 1, &[])]))),
            ],
        )
        .on(
            "/chats/42/messages/20/poll/close",
            &state(20, 3, true, &[], &[]),
        );
    let model = model(&rig, &server);
    rig.drive(model.load());

    assert_eq!(None, rig.drive(model.close(20))?);
    assert_eq!(None, model.failure());
    assert_eq!(
        vec![30],
        model
            .messages()?
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>()
    );

    // Somebody else's, and one no longer listed: nothing is sent.
    let asked = server.asked().len();
    assert_eq!(None, rig.drive(model.close(30))?);
    assert_eq!(None, rig.drive(model.vote(20, 1))?);
    assert_eq!(asked, server.asked().len());
    Ok(())
}

#[test]
fn a_list_that_cannot_be_read_says_so_and_keeps_what_it_had() -> StoreResult<()> {
    let rig = rig()?;
    let server = Server::new();
    server.then(
        "/chats/42/polls/open",
        &[
            (200, Some(&list(&[row(10, 9, 1, &[])]))),
            (403, Some(NOT_MEMBER)),
            (200, Some(&list(&[]))),
        ],
    );
    let model = model(&rig, &server);
    rig.drive(model.load());

    assert_eq!(
        Some(error_codes::NOT_CHAT_MEMBER),
        rig.drive(model.load()).map(|e| e.code).as_deref()
    );
    assert!(model.loaded());
    assert_eq!(1, model.messages()?.len());

    assert_eq!(None, rig.drive(model.load()));
    assert_eq!(None, model.failure());
    assert!(model.messages()?.is_empty());
    Ok(())
}

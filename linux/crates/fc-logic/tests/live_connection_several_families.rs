//! `SeveralFamiliesTests.FramesAboutTheReadersFamiliesReachTheSession`, whose subject is
//! `LiveConnection`: the live connection hands the session what the frames say about families — an
//! invitation is listed, and a `member_left` naming the reader drops THAT family's rows and shows what
//! is left.

mod common;

use std::cell::Cell;
use std::rc::Rc;

use fc_core::protocol::{
    ChatDto, ChatRowDto, FrameRouter, FrameSender, MemoryTokenStore, Resync, SendPipeline,
    ServerFrame,
};
use fc_core::store::{BoardStore, ChatStore, Database, OutboxStore, StoreResult};
use fc_core::testing::FakeClock;
use fc_logic::app_session::{AppSession, CurrentFamily, MemoryCurrentFamily};
use fc_logic::live_connection::{LiveConnection, LiveParts};
use time::macros::datetime;

use common::server::Server;
use common::session::{Pool, Wire, api_with};

const SMITHS: &str = r#"{"id": 3, "name": "The Smiths", "role": "member"}"#;
const IVANOVS: &str = r#"{"id": 5, "name": "Ivanovs", "role": "owner"}"#;
const INVITATION: &str = r#"{"id": 4, "family_id": 9, "family_name": "Sidorovs", "invited_by": {"id": 11, "username": "carl", "display_name": "Carl"}, "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#;

fn me(families: &str) -> String {
    format!(
        r#"{{"user": {{"id": 7, "username": "anna", "display_name": "Anna"}}, "blocked_user_ids": [],
         "families": [{families}], "pending_join_requests": [], "invitations": [],
         "member_families_enabled": true, "max_families_per_account": 5}}"#
    )
}

fn frame(json: &str) -> ServerFrame {
    ServerFrame::parse(json).expect("a frame")
}

#[test]
fn frames_about_the_readers_families_reach_the_session() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let clock = FakeClock::at(datetime!(2026-10-22 10:00 UTC));
    let pool = Pool::new(&clock);
    let server = Server::new();
    server.then(
        "/me",
        &[
            (200, Some(&me(&format!("{SMITHS}, {IVANOVS}")))),
            (200, Some(&me(SMITHS))),
        ],
    );
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server), &clock, tokens.clone());
    let remembered = Rc::new(MemoryCurrentFamily::new());
    remembered.remember(7, 5);
    let session = Rc::new(
        AppSession::new(api.clone(), tokens, cache.clone()).with_current_family(remembered),
    );
    pool.run(session.refresh())?;
    let (on_chats, on_board) = (session.clone(), session.clone());
    let chats = Rc::new(
        ChatStore::new(&cache)
            .with_me(|| 7)
            .with_current(move || on_chats.state().current_family_id),
    );
    let board =
        Rc::new(BoardStore::new(&cache).with_current(move || on_board.state().current_family_id));
    chats.replace(
        &[ChatRowDto::new(ChatDto {
            family_ids: Some(vec![5]),
            ..ChatDto::new(43, "family", "Ivanovs")
        })],
        None,
    )?;
    // This socket never connects, so no pass runs under the frames.
    let socket = Wire::new();
    let sender: Rc<dyn FrameSender> = Rc::new(socket.clone());
    let sending = Rc::new(SendPipeline::from_api(
        sender,
        Rc::new(OutboxStore::new(&cache)),
        chats.clone(),
        api.clone(),
        Rc::new(clock.clone()),
    ));
    let live = LiveConnection::new(
        session.clone(),
        LiveParts {
            socket: Rc::new(socket.clone()),
            sending,
            router: Rc::new(FrameRouter::new(chats.clone(), board.clone(), None)),
            resync: Rc::new(Resync::new(api, chats.clone(), board)),
        },
        pool.spawn(),
    );

    socket.hear(&frame(&format!(
        r#"{{"type": "invitation", "invitation": {INVITATION}}}"#
    )));
    let state = session.state();
    assert_eq!(1, state.all_invitations().len());
    assert_eq!(4, state.all_invitations()[0].id);
    socket.hear(&frame(
        r#"{"type": "invitation_removed", "invitation_id": 4}"#,
    ));
    assert!(session.state().all_invitations().is_empty());

    let moved = Rc::new(Cell::new(false));
    let flag = moved.clone();
    session.on_changed().subscribe(move |state| {
        if state.current_family_id == Some(3) {
            flag.set(true);
        }
    });
    socket.hear(&frame(
        r#"{"type": "member_left", "family_id": 5, "user_id": 7}"#,
    ));
    pool.until(|| moved.get());
    assert_eq!(None, chats.chat(43)?);
    assert_eq!(
        vec![3],
        session
            .state()
            .all_families()
            .iter()
            .map(|family| family.id)
            .collect::<Vec<_>>()
    );
    pool.run(live.dispose());
    Ok(())
}

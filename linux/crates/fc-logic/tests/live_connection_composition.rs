//! This port's own (no C# counterpart): the composition the app uses — `LiveParts::compose` over a
//! real `ChatSocket` and `fc_core::testing::FakeConnector` — so the carry-forward's recipe is pinned
//! end to end: a refused upgrade with `401` signs the user out once (the protocol's rule, which
//! `win/` did not keep — carry-forward, "Found in the other clients" 10), and a connection's frames
//! reach the cache through the router while the connection starts a pass.

mod common;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use fc_core::protocol::{ChatDto, ChatRowDto, ChatSocket, MemoryTokenStore, TokenStore};
use fc_core::rt::{TransportError, WebSocketConnector};
use fc_core::store::{BoardStore, ChatStore, Database, OutboxStore, StoreResult};
use fc_core::testing::{FakeClock, FakeConnector, FakeWebSocket};
use fc_logic::app_session::{AppSession, Gate, SessionEnd};
use fc_logic::live_connection::{Link, LiveConnection, LiveParts};
use time::macros::datetime;

use common::server::Server;
use common::session::{Pool, api_with};

const ME: &str = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
     "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "member",
     "blocked_user_ids": []}"#;

struct Rig {
    pool: Pool,
    session: Rc<AppSession>,
    live: LiveConnection,
    chats: Rc<ChatStore>,
    tokens: Rc<MemoryTokenStore>,
}

fn build(connector: &FakeConnector) -> StoreResult<Rig> {
    let cache = Database::open_in_memory()?;
    let clock = FakeClock::at(datetime!(2026-10-29 10:00 UTC));
    let pool = Pool::new(&clock);
    let server = Server::new();
    server
        .on("/me", ME)
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [], "blocked_user_ids": []}"#,
        )
        .on(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#,
        )
        .on("/chats/42/messages", r#"{"messages": []}"#);
    let tokens = Rc::new(MemoryTokenStore::new(Some("t0ken")));
    let api = api_with(Rc::new(server), &clock, tokens.clone());
    let session = Rc::new(AppSession::new(api.clone(), tokens.clone(), cache.clone()));
    pool.run(session.refresh())?;
    assert_eq!(Gate::Member, session.state().gate);
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| 7));
    chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let link: Rc<dyn WebSocketConnector> = Rc::new(connector.clone());
    let url = api.socket_url();
    let socket = Rc::new(ChatSocket::new(
        link,
        move || url.clone(),
        tokens.clone(),
        Rc::new(clock.clone()),
    ));
    let parts = LiveParts::compose(
        socket,
        api,
        chats.clone(),
        Rc::new(BoardStore::new(&cache)),
        None,
        Rc::new(OutboxStore::new(&cache)),
        Rc::new(clock.clone()),
        None,
    );
    let live = LiveConnection::new(session.clone(), parts, pool.spawn());
    Ok(Rig {
        pool,
        session,
        live,
        chats,
        tokens,
    })
}

#[test]
fn an_upgrade_refused_with_401_signs_out_once_and_stops() -> StoreResult<()> {
    let connector = FakeConnector::new();
    connector.then_refuse(TransportError::UpgradeRefused(401));
    let rig = build(&connector)?;
    let endings = Rc::new(RefCell::new(Vec::new()));
    let heard = endings.clone();
    rig.session
        .on_ended()
        .subscribe(move |end| heard.borrow_mut().push(*end));
    let down = Rc::new(Cell::new(false));
    let flag = down.clone();
    rig.live.on_link_changed().subscribe(move |link| {
        if *link == Link::Down {
            flag.set(true);
        }
    });

    rig.live.start();
    rig.pool.until(|| down.get());

    assert_eq!(vec![SessionEnd::Expired], *endings.borrow());
    assert_eq!(None, rig.tokens.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    // The bearer went in the header, and nobody knocked again with a token that is gone.
    assert_eq!(1, connector.opened().len());
    assert_eq!(Some("t0ken"), connector.opened()[0].1.as_deref());
    rig.pool.run(rig.live.dispose());
    Ok(())
}

#[test]
fn a_connections_frames_reach_the_cache_and_the_connection_starts_a_pass() -> StoreResult<()> {
    let connector = FakeConnector::new();
    let wire = FakeWebSocket::new(&[
        r#"{"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 9, "body": "Dinner at 7?", "created_at": "2026-10-29T10:00:00Z"}}"#,
    ])
    .held_open();
    connector.then(wire.clone());
    let rig = build(&connector)?;
    let passes = Rc::new(Cell::new(0));
    let counted = passes.clone();
    rig.live.on_resynced().subscribe(move |report| {
        assert!(report.complete());
        counted.set(counted.get() + 1);
    });

    rig.live.start();
    rig.pool.until(|| passes.get() == 1);

    assert_eq!(Link::Up, rig.live.link());
    assert_eq!(
        "Dinner at 7?",
        rig.chats.message(1338)?.expect("heard").body
    );
    rig.pool.run(rig.live.dispose());
    assert!(wire.is_closed());
    assert_eq!(Link::Down, rig.live.link());
    Ok(())
}

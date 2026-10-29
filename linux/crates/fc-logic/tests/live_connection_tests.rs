//! `LiveConnectionTests`: the policy over the socket — when it runs, what a connection means, and
//! what a connection that keeps flapping is not allowed to cost.
//!
//! The C# waited on the thread pool with a five-second patience; here everything runs on one
//! `LocalPool` with a fake clock that moves only when nothing else can (`common::session::Pool`), so
//! "give a second loop every chance to appear" is running the pool until it stalls.
//!
//! Adapted: `APassThatThrowsDoesNotStopTheNextOne`. The C#'s pass "threw" because its handler threw
//! an exception the client did not catch; the Rust transport has no exceptions — a handler's failure
//! is an `ApiError` the pass reports (`AFailedPassSaysSoAndTheNextConnectionTriesAgain`) — so the
//! pass that throws here is one the CACHE refuses mid-way (a trigger aborts the chat list's write),
//! which is the `Err` a Rust pass answers. The rule pinned is the C#'s: it reports nothing, and the
//! next connection starts another.

mod common;

use std::cell::{Cell, RefCell};
use std::pin::pin;
use std::rc::Rc;

use fc_core::protocol::resync::Report;
use fc_core::protocol::send_pipeline::Post;
use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{
    ApiError, ApiResult, ChatDto, ChatRowDto, FrameRouter, FrameSender, MemoryTokenStore,
    MessageDto, MessageResponse, QueueOptions, Resync, SendPipeline, ServerFrame, TokenStore,
};
use fc_core::rt::{Cancel, LocalBoxFuture};
use fc_core::store::{BoardStore, ChatStore, Database, OutboxStore, SeqRoute, StoreResult};
use fc_core::testing::FakeClock;
use fc_logic::app_session::{AppSession, Gate, SessionEnd};
use fc_logic::assistant_consent;
use fc_logic::live_connection::{Link, LiveConnection, LiveParts};
use time::macros::datetime;

use common::server::Server;
use common::session::{Pool, Wire, api_with};

const TOKEN: &str =
    r#"{"token": "t0ken", "user": {"id": 7, "username": "anna", "display_name": "Anna"}}"#;

const ME: &str = r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
         "family": {"id": 3, "name": "The Smiths", "ai_history": true}, "role": "member",
         "blocked_user_ids": []}"#;

const FAMILY: &str = r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true},
         "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
         "blocked_user_ids": []}"#;

const NO_FAMILY: &str =
    r#"{"user": {"id": 7, "username": "anna", "display_name": "Anna"}, "blocked_user_ids": []}"#;

struct Rig {
    pool: Pool,
    session: Rc<AppSession>,
    live: LiveConnection,
    wire: Wire,
    chats: Rc<ChatStore>,
    outbox: Rc<OutboxStore>,
    sending: Rc<SendPipeline>,
    server: Server,
    tokens: Rc<MemoryTokenStore>,
    posts: Rc<Cell<usize>>,
    cache: Rc<Database>,
}

fn default_server() -> Server {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", ME)
        .on("/families/mine", FAMILY)
        .on("/chats", r#"{"chats": []}"#);
    server
}

fn build(server: Server) -> StoreResult<Rig> {
    build_over(server, Database::open_in_memory()?)
}

fn build_over(server: Server, cache: Rc<Database>) -> StoreResult<Rig> {
    let clock = FakeClock::at(datetime!(2026-09-12 10:00 UTC));
    let pool = Pool::new(&clock);
    let tokens = Rc::new(MemoryTokenStore::new(None));
    let api = api_with(Rc::new(server.clone()), &clock, tokens.clone());
    let chats = Rc::new(ChatStore::new(&cache).with_me(|| 7));
    let board = Rc::new(BoardStore::new(&cache));
    let outbox = Rc::new(OutboxStore::new(&cache));
    let wire = Wire::new();
    let posts = Rc::new(Cell::new(0));
    let counted = posts.clone();
    let post: Post = Rc::new(move |row| {
        counted.set(counted.get() + 1);
        let answer = ApiResult::success(
            MessageResponse {
                message: MessageDto::new(
                    1338,
                    row.chat_id,
                    7,
                    Some(&row.client_msg_id),
                    &row.body,
                    "2026-09-12T10:00:00Z",
                ),
            },
            201,
        );
        Box::pin(std::future::ready(answer)) as LocalBoxFuture<'static, _>
    });
    let socket: Rc<dyn FrameSender> = Rc::new(wire.clone());
    let sending = Rc::new(SendPipeline::new(
        socket,
        outbox.clone(),
        chats.clone(),
        post,
        Rc::new(clock.clone()),
    ));
    let session = Rc::new(AppSession::new(api.clone(), tokens.clone(), cache.clone()));
    let resync =
        Rc::new(Resync::new(api, chats.clone(), board.clone()).with_sending(sending.clone()));
    let router = Rc::new(FrameRouter::new(chats.clone(), board, None));
    let live = LiveConnection::new(
        session.clone(),
        LiveParts {
            socket: Rc::new(wire.clone()),
            sending: sending.clone(),
            router,
            resync,
        },
        pool.spawn(),
    );
    Ok(Rig {
        pool,
        session,
        live,
        wire,
        chats,
        outbox,
        sending,
        server,
        tokens,
        posts,
        cache,
    })
}

impl Rig {
    fn sign_in(&self) -> StoreResult<()> {
        assert_eq!(
            None,
            self.pool.run(self.session.sign_in("anna", "hunter2"))?
        );
        Ok(())
    }

    /// `await Wire.Running`.
    fn running(&self) {
        self.pool.until(|| self.wire.running());
    }

    /// `NextPass(live, act)`: the report of the pass `act` sets going.
    fn next_pass(&self, act: impl FnOnce()) -> Report {
        let pass: Rc<RefCell<Option<Report>>> = Rc::default();
        let heard = pass.clone();
        let subscription = self.live.on_resynced().subscribe(move |report| {
            heard.borrow_mut().get_or_insert_with(|| report.clone());
        });
        act();
        self.pool.until(|| pass.borrow().is_some());
        self.live.on_resynced().unsubscribe(subscription);
        pass.take().expect("a pass")
    }

    fn dispose(&self) {
        self.pool.run(self.live.dispose());
    }
}

/// A PASS PUTS THE FAMILY'S ASSISTANT IN THE SESSION. `GET /families/mine` is the only read that
/// names it, and this pass is the only thing that runs that read on a connect — so a client that
/// left it in the report had NO assistant in its state until somebody opened the Family screen, and
/// with no assistant there is no consent card in Settings and no consent line over the composer: no
/// way at all to agree that your words may go to the model (docs/protocol.md, "Consenting to the
/// assistant").
#[test]
fn a_pass_gives_the_session_the_familys_assistant() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", ME)
        .on(
            "/families/mine",
            r#"{"family": {"id": 3, "name": "The Smiths", "ai_history": true},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "owner"}],
                 "blocked_user_ids": [],
                 "assistant": {"user_id": 99, "display_name": "Assistant", "mention": "@ai",
                               "processor": "Azure OpenAI (Sweden Central)"}}"#,
        )
        .on("/chats", r#"{"chats": []}"#);
    let rig = build(server)?;
    rig.sign_in()?;
    assert_eq!(None, rig.session.state().assistant);

    rig.live.start();
    let report = rig.next_pass(|| rig.wire.comes());

    assert!(report.complete());
    assert_eq!(
        Some("Azure OpenAI (Sweden Central)"),
        report
            .assistant
            .as_ref()
            .and_then(|assistant| assistant.processor.as_deref())
    );
    // And in the state the screens read, without the Family screen ever being opened.
    let state = rig.session.state();
    let processor = state
        .assistant
        .as_ref()
        .and_then(|assistant| assistant.processor.as_deref());
    assert_eq!(Some("Azure OpenAI (Sweden Central)"), processor);
    assert!(assistant_consent::is_required(
        Some("ai"),
        None,
        processor,
        state.assistant_consent_at.as_deref()
    ));
    rig.dispose();
    Ok(())
}

/// A PASS PUTS THE VIDEO MESSAGE'S LIMITS IN THE SESSION (docs/protocol.md, "Video messages"), where
/// the composer reads whether to offer a way of recording one: kept through a pass that could not
/// read the family's document, and taken away by one that read it without them — an older server.
#[test]
fn a_pass_gives_the_session_the_video_messages_limits() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/me", ME)
        .then(
            "/families/mine",
            &[
                (
                    200,
                    Some(
                        r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [], "blocked_user_ids": [],
                     "max_round_video_ms": 60000, "max_round_video_bytes": 12582912}"#,
                    ),
                ),
                (502, Some("<html>502</html>")),
                (502, Some("<html>502</html>")),
                (
                    200,
                    Some(r#"{"family": {"id": 3, "name": "The Smiths"}, "members": [], "blocked_user_ids": []}"#),
                ),
            ],
        )
        .on("/chats", r#"{"chats": []}"#);
    let rig = build(server)?;
    rig.sign_in()?;
    assert_eq!(None, rig.session.state().round_video);
    let limits = fc_core::protocol::RoundVideoLimits {
        max_ms: 60_000,
        max_bytes: 12_582_912,
    };

    rig.live.start();
    assert!(rig.next_pass(|| rig.wire.comes()).complete());
    assert_eq!(Some(limits), rig.session.state().round_video);

    rig.wire.goes();
    assert!(!rig.next_pass(|| rig.wire.comes()).complete());
    assert_eq!(Some(limits), rig.session.state().round_video);

    rig.wire.goes();
    assert!(rig.next_pass(|| rig.wire.comes()).complete());
    assert_eq!(None, rig.session.state().round_video);
    rig.dispose();
    Ok(())
}

/// THE SOCKET RUNS ONLY WHILE THERE IS SOMETHING TO LISTEN TO: an account at the family gate has no
/// chats, and a loop reconnecting on its behalf is a client hammering a door with no room behind it.
#[test]
fn nothing_listens_until_there_is_a_family_to_listen_to() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .then("/me", &[(200, Some(NO_FAMILY)), (200, Some(ME))])
        .on("/families/mine", FAMILY)
        .on("/chats", r#"{"chats": []}"#);
    let rig = build(server)?;

    rig.sign_in()?;
    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    rig.live.start();
    rig.pool.settle();
    assert_eq!(0, rig.wire.runs());
    assert_eq!(Link::Down, rig.live.link());

    // The join is approved: the gate moves, and the socket follows it without being asked.
    assert_eq!(None, rig.pool.run(rig.session.refresh())?);
    rig.running();
    assert_eq!(1, rig.wire.runs());
    assert_eq!(Link::Connecting, rig.live.link());

    // And starting again is not starting a second one: the window may say this on every
    // appearance, and two loops on one socket would be two sockets and two resyncs. Everything set
    // going runs before insisting there is none.
    rig.live.start();
    rig.live.start();
    rig.pool.settle();
    assert_eq!(1, rig.wire.runs());

    rig.dispose();
    Ok(())
}

/// THE CURSORS ARE TAKEN WHEN THE CONNECTION OPENS, before a frame on it can be applied
/// (docs/protocol.md, "Best-effort delivery", step 3). The pass starts later — here, only once the
/// gate lets the socket run — and a live message that landed in between is not where it starts:
/// what was missed while the socket was down is below it.
#[test]
fn the_pass_starts_from_what_was_held_when_the_connection_opened() -> StoreResult<()> {
    let page = (11..=30)
        .map(|id| {
            format!(
                r#"{{"id": {id}, "chat_id": 42, "sender_id": 9, "body": "hi", "created_at": "2026-09-12T10:00:00Z"}}"#
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .then("/me", &[(200, Some(NO_FAMILY)), (200, Some(ME))])
        .on("/families/mine", FAMILY)
        .on(
            "/chats",
            r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#,
        )
        .on(
            "/chats/42/messages",
            &format!(r#"{{"messages": [{page}]}}"#),
        );
    let rig = build(server)?;
    rig.sign_in()?;
    assert_eq!(Gate::NoFamily, rig.session.state().gate);
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    let held: Vec<MessageDto> = (1..=10)
        .map(|id| MessageDto::new(id, 42, 9, None, "hi", "2026-09-12T10:00:00Z"))
        .collect();
    rig.chats.apply_page(&held, SeqRoute::CatchUpPage, true)?;

    // The connection opens with 10 held, and a live message lands on it...
    rig.wire.comes();
    rig.chats.apply(
        &MessageDto::new(30, 42, 9, None, "hi", "2026-09-12T10:00:00Z"),
        SeqRoute::LiveFrame,
        true,
    )?;

    // ...before the pass that connection asked for gets to run.
    let session = rig.session.clone();
    let spawn = rig.pool.spawn();
    let report = rig.next_pass(move || {
        spawn(Box::pin(async move {
            let _ = session.refresh().await;
        }));
    });

    assert!(report.complete());
    assert!(
        rig.server
            .asked()
            .contains(&"/api/v1/chats/42/messages?after_id=10&limit=50".to_string())
    );
    for id in 11..=30 {
        assert!(rig.chats.message(id)?.is_some(), "message {id}");
    }
    rig.dispose();
    Ok(())
}

/// A pass that THREW is a pass that failed, and no more than that: the next connection starts
/// another. A loop that stopped reading because one answer was nonsense would be a client that never
/// recovers. (Adapted: the module's comment.)
#[test]
fn a_pass_that_throws_does_not_stop_the_next_one() -> StoreResult<()> {
    let cache = Database::open_in_memory()?;
    let routed = Rc::new(Cell::new(0));
    let server = Server::new();
    let (counted, held) = (routed.clone(), cache.clone());
    server
        .on("/auth/login", TOKEN)
        .on("/me", ME)
        .on("/families/mine", FAMILY)
        // The first pass asks for a chat list whose write the cache refuses; the second gets one it
        // keeps.
        .on_async("/chats", move || {
            counted.set(counted.get() + 1);
            let answer = if counted.get() == 1 {
                held.connection()
                    .execute_batch(
                        "CREATE TRIGGER nothing_there BEFORE INSERT ON chats
                         BEGIN SELECT RAISE(ABORT, 'nothing there'); END;",
                    )
                    .expect("a trigger");
                r#"{"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths"}}]}"#
            } else {
                held.connection()
                    .execute_batch("DROP TRIGGER IF EXISTS nothing_there;")
                    .expect("no trigger");
                r#"{"chats": []}"#
            };
            Box::pin(std::future::ready(Ok((200, Some(answer.to_string())))))
        });
    let rig = build_over(server, cache)?;
    rig.sign_in()?;
    rig.running();

    let reports: Rc<RefCell<Vec<Report>>> = Rc::default();
    let heard = reports.clone();
    rig.live
        .on_resynced()
        .subscribe(move |report| heard.borrow_mut().push(report.clone()));
    let failures = Rc::new(Cell::new(0));
    let failed = failures.clone();
    rig.live
        .on_cache_failed()
        .subscribe(move |_| failed.set(failed.get() + 1));
    rig.wire.comes();
    rig.wire.goes();
    // The first pass has TAKEN its connection before the next one arrives — the same precondition
    // `connections_that_arrive_during_a_pass_become_one_more_pass` states.
    rig.pool.until(|| routed.get() == 1);
    let second = rig.next_pass(|| rig.wire.comes());

    assert!(second.complete());
    // The pass that threw reported nothing at all — it never reached the end — and the one after it
    // did. Its failure is the cache's, for the log.
    assert_eq!(2, routed.get());
    assert_eq!(1, reports.borrow().len());
    assert_eq!(1, failures.get());
    rig.dispose();
    Ok(())
}

/// STARTING AND STOPPING ARE SERIALISED. A stop used to clear its fields and only then wait for the
/// socket loop to wind down, so a start arriving in that window found nothing running and put a
/// SECOND loop on the same socket — one object with one live connection field, and the two would
/// have fought over it. A gate that moves away and back (signed out, signed in; removed, re-approved)
/// is exactly that window.
#[test]
fn a_gate_that_moves_away_and_back_never_runs_two_sockets_at_once() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.running();

    rig.pool.run(async {
        for _ in 0..5 {
            // Not awaited at first, on purpose: this is the window handler's own shape — the gate
            // moved inside somebody else's handler, and a window event may not wait on a socket.
            let mut stopping = pin!(rig.live.stop());
            let _ = futures::poll!(stopping.as_mut());
            rig.live.start();
            stopping.await;
            rig.live.start_now().await;
        }
    });
    rig.pool.settle();

    assert_eq!(1, rig.wire.overlap());
    assert_eq!(Link::Connecting, rig.live.link());

    rig.dispose();
    assert_eq!(0, rig.wire.live());
    Ok(())
}

/// EVERY connection starts a pass, not just the first: what a client missed while the socket was
/// down is exactly what the pass reads.
#[test]
fn every_connection_starts_a_pass() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.running();

    let first = rig.next_pass(|| rig.wire.comes());
    assert!(first.complete());
    assert!(first.signed);
    assert_eq!(Link::Up, rig.live.link());

    rig.wire.goes();
    assert_eq!(Link::Connecting, rig.live.link());

    let second = rig.next_pass(|| rig.wire.comes());
    assert!(second.complete());

    rig.dispose();
    assert_eq!(Link::Down, rig.live.link());
    assert!(rig.wire.stopped());
    Ok(())
}

/// AND RESYNCS DO NOT STACK. A flapping proxy can raise three connections in a second; three
/// overlapping passes would page the same chats three times and fight over the cursors. A pass in
/// flight coalesces what arrives during it into ONE more pass.
#[test]
fn connections_that_arrive_during_a_pass_become_one_more_pass() -> StoreResult<()> {
    let held = Cancel::new();
    let stuck = Rc::new(Cell::new(false));
    let reads = Rc::new(Cell::new(0));
    // `/me` is the first read of a pass: the SECOND one (the first belongs to the sign-in) is held
    // open, so every connection that follows arrives while a pass is in flight.
    let (release, flagged, counted) = (held.clone(), stuck.clone(), reads.clone());
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        .on("/families/mine", FAMILY)
        .on("/chats", r#"{"chats": []}"#)
        .on_async("/me", move || {
            counted.set(counted.get() + 1);
            let wait = (counted.get() == 2).then(|| {
                flagged.set(true);
                release.clone()
            });
            Box::pin(async move {
                if let Some(wait) = wait {
                    wait.cancelled().await;
                }
                Ok((200, Some(ME.to_string())))
            })
        });
    let rig = build(server)?;
    rig.sign_in()?;
    rig.running();

    let passes = Rc::new(Cell::new(0));
    let counted = passes.clone();
    rig.live
        .on_resynced()
        .subscribe(move |_| counted.set(counted.get() + 1));

    // The first connection starts the pass that is now stuck on `/me`…
    rig.wire.comes();
    // …and it IS stuck before anything else arrives: four connections that all land before the pass
    // takes the first one are ONE pass — correctly, since none of them arrived during a pass.
    rig.pool.until(|| stuck.get());
    // …and three more arrive while it is stuck.
    rig.wire.goes();
    rig.wire.comes();
    rig.wire.goes();
    rig.wire.comes();
    rig.wire.goes();
    rig.wire.comes();
    held.cancel();

    rig.pool.until(|| passes.get() == 2);
    // Give a third pass every chance to appear, and then insist there is none.
    rig.pool.settle();
    assert_eq!(2, passes.get());

    rig.dispose();
    Ok(())
}

/// AN EXPIRED SESSION IS NOT A NETWORK ERROR: the socket's `4401` lands exactly where a REST `401`
/// lands, and it lands once — otherwise a device whose account was deleted sits behind a
/// "Connecting…" banner looking signed in.
#[test]
fn the_socket_saying_the_session_is_gone_ends_it_once_and_stops_listening() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.running();
    let endings = Rc::new(RefCell::new(Vec::new()));
    let heard = endings.clone();
    rig.session
        .on_ended()
        .subscribe(move |end| heard.borrow_mut().push(*end));
    let stopped = Rc::new(Cell::new(false));
    let down = stopped.clone();
    rig.live.on_link_changed().subscribe(move |link| {
        if *link == Link::Down {
            down.set(true);
        }
    });

    rig.wire.expires();
    rig.wire.expires();

    rig.pool.until(|| stopped.get());
    assert_eq!(vec![SessionEnd::Expired], *endings.borrow());
    assert_eq!(None, rig.tokens.token());
    assert_eq!(Gate::SignedOut, rig.session.state().gate);
    assert!(rig.wire.stopped());

    rig.dispose();
    Ok(())
}

#[test]
fn frames_reach_the_cache_through_the_router() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;

    rig.wire.hear(&ServerFrame::Message(MessageDto::new(
        1338,
        42,
        9,
        None,
        "Dinner at 7?",
        "2026-09-12T10:00:00Z",
    )));

    assert_eq!(
        "Dinner at 7?",
        rig.chats.message(1338)?.expect("heard").body
    );

    rig.dispose();
    // And a disposed connection is not still listening to a socket somebody else holds.
    rig.wire.hear(&ServerFrame::Message(MessageDto::new(
        1339,
        42,
        9,
        None,
        "…or 8?",
        "2026-09-12T10:01:00Z",
    )));
    assert_eq!(None, rig.chats.message(1339)?);
    Ok(())
}

/// THE OUTBOX HAS THREE TRIGGERS AND THE SOCKET IS ONLY ONE OF THEM: a returning network and a
/// window coming forward flush it too, or a message sits unsent under a full signal bar.
#[test]
fn a_returning_network_flushes_the_outbox_without_waiting_for_a_socket() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.chats.replace(
        &[ChatRowDto::new(ChatDto::new(42, "family", "The Smiths"))],
        None,
    )?;
    rig.sending
        .enqueue(42, "Dinner at 7?", &QueueOptions::default())?;

    // No socket at all: the trigger is the network coming back.
    assert_eq!(
        1,
        rig.pool
            .run(rig.live.flush(FlushTrigger::ConnectivityRestored))?
    );
    assert_eq!(1, rig.posts.get());
    // Posted, delivered, and the row is gone.
    assert!(rig.outbox.all()?.is_empty());

    rig.dispose();
    Ok(())
}

#[test]
fn a_failed_pass_says_so_and_the_next_connection_tries_again() -> StoreResult<()> {
    let server = Server::new();
    server
        .on("/auth/login", TOKEN)
        // TWICE, because a GET is tried once more after a transient failure — one 502 is a read
        // that succeeds on the retry, which is the point of the retry.
        .then(
            "/me",
            &[
                (200, Some(ME)),
                (502, Some("<html>502</html>")),
                (502, Some("<html>502</html>")),
                (200, Some(ME)),
            ],
        )
        .on("/families/mine", FAMILY)
        .on("/chats", r#"{"chats": []}"#);
    let rig = build(server)?;
    rig.sign_in()?;
    rig.running();
    let failures: Rc<RefCell<Vec<ApiError>>> = Rc::default();
    let heard = failures.clone();
    rig.live
        .on_resync_failed()
        .subscribe(move |error| heard.borrow_mut().push(error.clone()));

    let failed = rig.next_pass(|| rig.wire.comes());
    assert!(!failed.complete());
    assert_eq!(1, failures.borrow().len());
    assert!(failures.borrow()[0].transient());
    // The flush ran anyway: it is not a step.
    assert!(failed.flushed);

    rig.wire.goes();
    let worked = rig.next_pass(|| rig.wire.comes());
    assert!(worked.complete());

    rig.dispose();
    Ok(())
}

#[test]
fn a_refresh_on_demand_is_a_pass_and_two_at_once_are_still_one() -> StoreResult<()> {
    let rig = build(default_server())?;
    rig.sign_in()?;
    rig.running();

    let report = rig.next_pass(|| {
        rig.live.refresh();
        rig.live.refresh();
        rig.live.refresh();
    });
    assert!(report.complete());

    rig.dispose();
    let _ = &rig.cache;
    Ok(())
}

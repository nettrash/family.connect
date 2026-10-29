//! The realtime loop: what it does with frames, with a dropped connection, and with a session
//! that expired under it (docs/protocol.md, "WebSocket protocol") — a port of
//! `win/tests/FamilyConnect.Core.Tests/Protocol/ChatSocketTests.cs`, test for test, then the
//! cases the port added: an upgrade refused with `401` (the Mac's rule, which `win/` lacked), the
//! write deadline run on the fake clock, a loop dropped mid-connection, and the two traits.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;
use std::time::Duration;

use fc_core::protocol::chat_socket::{DURABLE_AFTER, SESSION_EXPIRED_CODE};
use fc_core::protocol::send_rules::ACK_DEADLINE;
use fc_core::protocol::{
    ChatSocket, FrameSender, LiveSocket, MemoryTokenStore, ReconnectBackoff, ServerFrame,
    client_frames,
};
use fc_core::rt::{Cancel, Clock, LocalBoxFuture, TransportError};
use fc_core::testing::{FakeClock, FakeConnector, FakeWebSocket, drive};
use futures::executor::LocalPool;
use futures::future::{Either, join, select};
use futures::task::LocalSpawnExt;
use time::OffsetDateTime;
use time::macros::datetime;

const WHERE: &str = "wss://chat.example.com/api/v1/ws";

const ONE_MESSAGE: &str = r#"
    {"type": "message", "message": {"id": 1338, "chat_id": 42, "sender_id": 7,
     "client_msg_id": null, "body": "Dinner at 7?", "created_at": "…"}}
    "#;

/// What the loop did, as the C# tests' `Loop` recorded it.
#[derive(Debug, Default)]
struct Log {
    waits: Vec<Duration>,
    frames: Vec<ServerFrame>,
    connects: usize,
    disconnects: usize,
    expired: usize,
}

/// The C# tests' injected `wait` and `now` in one: a wait is RECORDED rather than slept, and ends
/// at once; after `stop_after` of them the loop is asked to stop. The time is moved only by the
/// test.
struct ScriptClock {
    now: Cell<OffsetDateTime>,
    log: Rc<RefCell<Log>>,
    stop_after: usize,
    stop: Cancel,
}

impl Clock for ScriptClock {
    fn now(&self) -> OffsetDateTime {
        self.now.get()
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        let waited = {
            let mut log = self.log.borrow_mut();
            log.waits.push(duration);
            log.waits.len()
        };
        if waited >= self.stop_after {
            self.stop.cancel();
        }
        Box::pin(async {})
    }
}

/// Run a future to its end. Nothing in these tests waits on a clock; the fake clock is here so a
/// loop that stalls fails the test rather than hanging it.
fn finish<T>(future: impl Future<Output = T>) -> T {
    drive(&FakeClock::at(datetime!(2026-09-12 10:00 UTC)), future)
}

/// The backoff at its ceilings — 1, 2, 4, … — so the waits can be read off exactly.
fn ceilings() -> ReconnectBackoff {
    ReconnectBackoff::with_random(|ceiling| ceiling)
}

/// Subscribe a log to every event of a socket.
fn listen(socket: &ChatSocket, log: &Rc<RefCell<Log>>) {
    let frames = log.clone();
    socket
        .on_frame()
        .subscribe(move |frame| frames.borrow_mut().frames.push(frame.clone()));
    let disconnects = log.clone();
    socket
        .on_disconnected()
        .subscribe(move |()| disconnects.borrow_mut().disconnects += 1);
    let expired = log.clone();
    socket
        .on_session_expired()
        .subscribe(move |()| expired.borrow_mut().expired += 1);
}

/// One loop over a script of connections. The waits are recorded rather than slept, and the clock
/// is moved by `lasted` for each connection that came up — which is what decides whether the
/// ceiling resets. Past the end of the script the loop is stopped (and one idle connection is
/// opened), as the C# harness did.
fn run(
    script: impl FnOnce(&FakeConnector),
    lasted: Duration,
    stop_after: usize,
) -> (Log, FakeConnector) {
    let log = Rc::new(RefCell::new(Log::default()));
    let stop = Cancel::new();
    let connector = FakeConnector::new();
    script(&connector);
    connector.cancel_when_empty(&stop);
    let clock = Rc::new(ScriptClock {
        now: Cell::new(datetime!(2026-09-12 10:00 UTC)),
        log: log.clone(),
        stop_after,
        stop: stop.clone(),
    });
    let socket = ChatSocket::new(
        Rc::new(connector.clone()),
        || WHERE.to_string(),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        clock.clone(),
    )
    .with_backoff(ceilings());
    listen(&socket, &log);
    let (connects, moved) = (log.clone(), clock.clone());
    socket.on_connected().subscribe(move |()| {
        connects.borrow_mut().connects += 1;
        // Each connection "lasts" this long before it drops.
        moved.now.set(moved.now.get() + lasted);
    });
    finish(socket.run(stop));
    let log = std::mem::take(&mut *log.borrow_mut());
    (log, connector)
}

#[test]
fn the_upgrade_carries_the_token_in_a_header_and_nowhere_else() {
    let (_, connector) = run(
        |c| {
            c.then(FakeWebSocket::new(&[ONE_MESSAGE]));
        },
        Duration::ZERO,
        1,
    );
    let (url, bearer) = connector.opened()[0].clone();
    assert_eq!(url, WHERE);
    assert_eq!(bearer.as_deref(), Some("t0ken"));
    assert!(!url.contains("t0ken"));
    // And the socket is closed when the connection ends, every time.
    assert!(connector.sockets()[0].is_closed());
}

#[test]
fn frames_arrive_in_order_and_unknown_ones_are_skipped() {
    let (log, _) = run(
        |c| {
            c.then(FakeWebSocket::new(&[
                ONE_MESSAGE,
                r#"{"type": "fireworks", "colour": "green"}"#,
                r#"{"type": "typing", "chat_id": 42, "user_id": 9}"#,
                "not json at all",
                r#"{"type": "pong"}"#,
            ]));
        },
        Duration::ZERO,
        1,
    );
    assert_eq!(log.frames.len(), 3);
    assert!(matches!(log.frames[0], ServerFrame::Message(_)));
    assert!(matches!(log.frames[1], ServerFrame::Typing { .. }));
    assert!(matches!(log.frames[2], ServerFrame::Pong));
}

#[test]
fn a_dropped_connection_comes_back_with_the_backoffs_own_delays() {
    let (log, connector) = run(
        |c| {
            c.then(FakeWebSocket::new(&[ONE_MESSAGE]))
                .then(FakeWebSocket::new(&[ONE_MESSAGE]))
                .then(FakeWebSocket::new(&[ONE_MESSAGE]));
        },
        Duration::ZERO,
        3,
    );
    // Three connections, three drops, and the ceilings climbing: none of them lasted.
    assert_eq!(log.connects, 3);
    assert_eq!(log.disconnects, 3);
    assert_eq!(
        log.waits,
        [
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(4)
        ]
    );
    assert_eq!(log.frames.len(), 3);
    assert!(
        connector.sockets()[..3]
            .iter()
            .all(FakeWebSocket::is_closed)
    );
}

/// A COMPLETED HANDSHAKE IS NOT ENOUGH. A proxy that accepts the upgrade and drops it at once used
/// to reset the ceiling every cycle, so the socket reconnected about twice a second for ever and
/// each cycle fired a full resync.
#[test]
fn a_connection_that_lasted_resets_the_ceiling_and_one_that_did_not_does_not() {
    let two = |c: &FakeConnector| {
        c.then(FakeWebSocket::new(&[ONE_MESSAGE]))
            .then(FakeWebSocket::new(&[ONE_MESSAGE]));
    };
    let (lasting, _) = run(two, DURABLE_AFTER, 2);
    // Both connections carried traffic for long enough, so both start again from the floor.
    assert_eq!(
        lasting.waits,
        [Duration::from_secs(1), Duration::from_secs(1)]
    );

    let (flapping, _) = run(two, Duration::from_secs(1), 2);
    assert_eq!(
        flapping.waits,
        [Duration::from_secs(1), Duration::from_secs(2)]
    );
}

#[test]
fn an_upgrade_that_never_completed_earns_nothing_and_is_not_a_connection() {
    let (log, _) = run(
        |c| {
            c.then_refuse(TransportError::Unreachable("refused".into()))
                .then_refuse(TransportError::Unreachable("refused".into()));
        },
        Duration::ZERO,
        2,
    );
    assert_eq!(log.connects, 0);
    // Nothing came up, so nothing went down: a refused dial is not a disconnection to show.
    assert_eq!(log.disconnects, 0);
    assert_eq!(log.waits, [Duration::from_secs(1), Duration::from_secs(2)]);
}

/// `4401` is not a network error: the session expired mid-connection, so the client signs out
/// rather than retrying a token that will never work again.
#[test]
fn a_session_that_expired_mid_connection_signs_out_and_stops_trying() {
    let (log, connector) = run(
        |c| {
            c.then(FakeWebSocket::new(&[ONE_MESSAGE]).closing_with(SESSION_EXPIRED_CODE))
                .then(FakeWebSocket::new(&[ONE_MESSAGE]));
        },
        Duration::ZERO,
        usize::MAX,
    );
    assert_eq!(log.expired, 1);
    // One connection only: the loop returned instead of reconnecting.
    assert_eq!(connector.sockets().len(), 1);
    assert!(log.waits.is_empty());
}

#[test]
fn an_ordinary_close_is_not_a_sign_out() {
    let (log, _) = run(
        |c| {
            // WebSocketCloseStatus.NormalClosure.
            c.then(FakeWebSocket::new(&[ONE_MESSAGE]).closing_with(1000));
        },
        Duration::ZERO,
        1,
    );
    assert_eq!(log.expired, 0);
    assert_eq!(log.waits.len(), 1);
}

#[test]
fn a_frame_is_written_only_while_there_is_a_connection() {
    let scripted = FakeWebSocket::new(&[ONE_MESSAGE]).held_open();
    let connector = FakeConnector::new();
    connector.then(scripted.clone());
    let log = Rc::new(RefCell::new(Log::default()));
    let stop = Cancel::new();
    let clock = Rc::new(ScriptClock {
        now: Cell::new(datetime!(2026-09-12 10:00 UTC)),
        log: log.clone(),
        stop_after: usize::MAX,
        stop: stop.clone(),
    });
    let socket = Rc::new(
        ChatSocket::new(
            Rc::new(connector),
            || WHERE.to_string(),
            Rc::new(MemoryTokenStore::new(Some("t0ken"))),
            clock,
        )
        .with_backoff(ReconnectBackoff::with_random(|_| 0.0)),
    );
    let connected = Rc::new(Cell::new(false));
    let flag = connected.clone();
    socket.on_connected().subscribe(move |()| flag.set(true));
    let mut pool = LocalPool::new();
    let ended = Rc::new(Cell::new(false));
    let (looping, cancel, done) = (socket.clone(), stop.clone(), ended.clone());
    pool.spawner()
        .spawn_local(async move {
            looping.run(cancel).await;
            done.set(true);
        })
        .expect("the loop is spawned");

    pool.run_until_stalled();
    assert!(connected.get());
    assert!(socket.is_connected());
    assert!(pool.run_until(socket.try_send(client_frames::typing(42))));
    assert_eq!(scripted.sent(), [r#"{"type":"typing","chat_id":42}"#]);

    // And once it is gone, a write reports that it did not go rather than failing: the outbox
    // decides what to do about it.
    scripted.release();
    stop.cancel();
    pool.run_until_stalled();
    assert!(ended.get(), "the loop has wound down");
    assert!(!socket.is_connected());
    assert!(!pool.run_until(socket.try_send(client_frames::typing(42))));
}

#[test]
fn cancelling_stops_the_loop_without_waiting_or_reconnecting() {
    let log = Rc::new(RefCell::new(Log::default()));
    let stop = Cancel::new();
    let opens = Rc::new(Cell::new(0));
    let connector = FakeConnector::new();
    let (counted, cancel) = (opens.clone(), stop.clone());
    connector
        .on_connect(move || {
            counted.set(counted.get() + 1);
            cancel.cancel();
        })
        .then(FakeWebSocket::new(&[ONE_MESSAGE]));
    let clock = Rc::new(ScriptClock {
        now: Cell::new(datetime!(2026-09-12 10:00 UTC)),
        log: log.clone(),
        stop_after: usize::MAX,
        stop: Cancel::new(),
    });
    let socket = ChatSocket::new(
        Rc::new(connector),
        || WHERE.to_string(),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        clock,
    );
    finish(socket.run(stop));
    assert_eq!(opens.get(), 1);
    assert!(log.borrow().waits.is_empty());
}

// ---- what the port added ----------------------------------------------------------------------

/// An upgrade REFUSED with `401` is a session that is gone, before the connection opened: "a bad
/// token fails the upgrade with `401`" and "a `401` means the session is gone" (docs/protocol.md,
/// "WebSocket protocol", "Authentication"). The Mac signs out on it; `win/` knocked again every
/// thirty seconds for ever (measured on .NET 10).
#[test]
fn an_upgrade_refused_with_401_signs_out_and_stops_trying() {
    let (log, connector) = run(
        |c| {
            c.then_refuse(TransportError::UpgradeRefused(401))
                .then(FakeWebSocket::new(&[ONE_MESSAGE]));
        },
        Duration::ZERO,
        usize::MAX,
    );
    assert_eq!(log.expired, 1);
    assert_eq!(log.connects, 0);
    // Nothing came up, so nothing went down.
    assert_eq!(log.disconnects, 0);
    assert!(log.waits.is_empty());
    assert_eq!(connector.opened().len(), 1);
}

/// Any other refusal of the upgrade — a proxy's 502, a 403 — is a network failure like any other,
/// and the ceiling climbs.
#[test]
fn an_upgrade_refused_with_anything_else_is_tried_again() {
    let (log, _) = run(
        |c| {
            c.then_refuse(TransportError::UpgradeRefused(502))
                .then_refuse(TransportError::UpgradeRefused(403));
        },
        Duration::ZERO,
        2,
    );
    assert_eq!(log.expired, 0);
    assert_eq!(log.waits, [Duration::from_secs(1), Duration::from_secs(2)]);
}

/// The C# adapter sent `Authorization` only for a token that was there: an empty one is none.
#[test]
fn no_token_is_no_bearer() {
    let connector = FakeConnector::new();
    let stop = Cancel::new();
    connector.on_connect({
        let stop = stop.clone();
        move || stop.cancel()
    });
    connector.then(FakeWebSocket::new(&[]));
    let socket = ChatSocket::new(
        Rc::new(connector.clone()),
        || WHERE.to_string(),
        Rc::new(MemoryTokenStore::new(Some(""))),
        Rc::new(FakeClock::at(datetime!(2026-09-12 10:00 UTC))),
    );
    finish(socket.run(stop));
    assert_eq!(connector.opened(), [(WHERE.to_string(), None)]);
}

/// The ack deadline is the WRITE deadline too: a socket whose TCP connection is dead absorbs a
/// write, and after ten seconds the write is reported as not gone — run here on the fake clock.
#[test]
fn a_write_that_outlasts_the_ack_deadline_did_not_go() {
    let clock = FakeClock::at(datetime!(2026-09-12 10:00 UTC));
    let dead = FakeWebSocket::new(&[]).held_open().stalled_sends();
    let connector = FakeConnector::new();
    connector.then(dead.clone());
    let socket = ChatSocket::new(
        Rc::new(connector),
        || WHERE.to_string(),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(clock.clone()),
    );
    let stop = Cancel::new();
    let started = clock.now();
    let ((), went) = drive(
        &clock,
        join(socket.run(stop.clone()), async {
            let went = socket.try_send(client_frames::typing(42)).await;
            stop.cancel();
            went
        }),
    );
    assert!(!went);
    assert_eq!(dead.sent(), [r#"{"type":"typing","chat_id":42}"#]);
    assert_eq!(clock.now() - started, ACK_DEADLINE);
    assert!(dead.is_closed());
}

/// Dropping the loop's future mid-connection stops it at once: nothing is live any more, the
/// socket is closed, and — unlike a cancel — nothing is raised. Frames that arrive on an open
/// connection are handed over as they come.
#[test]
fn dropping_the_loop_closes_the_socket_and_raises_nothing() {
    let open = FakeWebSocket::new(&[]).held_open();
    let connector = FakeConnector::new();
    connector.then(open.clone());
    let socket = ChatSocket::new(
        Rc::new(connector),
        || WHERE.to_string(),
        Rc::new(MemoryTokenStore::new(Some("t0ken"))),
        Rc::new(FakeClock::at(datetime!(2026-09-12 10:00 UTC))),
    );
    let log = Rc::new(RefCell::new(Log::default()));
    listen(&socket, &log);
    let looping = Box::pin(socket.run(Cancel::new()));
    let watched = Box::pin(async {
        // One poll of the loop has connected it by now.
        assert!(socket.is_connected());
        open.push(r#"{"type": "pong"}"#);
        // Let the loop read it.
        while log.borrow().frames.is_empty() {
            yield_once().await;
        }
    });
    match finish(select(looping, watched)) {
        Either::Left(_) => panic!("the loop ended on its own"),
        Either::Right(((), looping)) => drop(looping),
    }
    assert!(!socket.is_connected());
    assert!(open.is_closed());
    let log = log.borrow();
    assert!(matches!(log.frames[..], [ServerFrame::Pong]));
    assert_eq!(log.disconnects, 0);
    assert_eq!(log.expired, 0);
}

/// One yield to the executor.
fn yield_once() -> impl Future<Output = ()> {
    let mut yielded = false;
    std::future::poll_fn(move |cx| {
        if yielded {
            std::task::Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            std::task::Poll::Pending
        }
    })
}

/// The loop as the layer above it holds it: a `LiveSocket`, whose writes are a `FrameSender`'s.
#[test]
fn the_socket_is_a_live_socket_and_a_frame_sender() {
    let connector = FakeConnector::new();
    let stop = Cancel::new();
    connector
        .then(FakeWebSocket::new(&[ONE_MESSAGE]))
        .cancel_when_empty(&stop);
    let socket: Rc<dyn LiveSocket> = Rc::new(
        ChatSocket::new(
            Rc::new(connector),
            || WHERE.to_string(),
            Rc::new(MemoryTokenStore::new(Some("t0ken"))),
            Rc::new(FakeClock::at(datetime!(2026-09-12 10:00 UTC))),
        )
        .with_backoff(ReconnectBackoff::with_random(|_| 0.0)),
    );
    let frames = Rc::new(Cell::new(0));
    let (counted, connects) = (frames.clone(), Rc::new(Cell::new(0)));
    socket
        .on_frame()
        .subscribe(move |_| counted.set(counted.get() + 1));
    let seen = connects.clone();
    socket
        .on_connected()
        .subscribe(move |()| seen.set(seen.get() + 1));
    let sender: &dyn FrameSender = socket.as_ref();
    assert!(!sender.is_connected());
    assert!(!finish(sender.try_send(client_frames::ping())));
    finish(socket.run(stop));
    // The scripted connection and the idle one past the end of the script.
    assert_eq!(connects.get(), 2);
    assert_eq!(frames.get(), 1);
    assert!(!socket.is_connected());
}

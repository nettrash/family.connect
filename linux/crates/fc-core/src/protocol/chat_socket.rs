//! The realtime connection: one socket at a time, reconnected with full jitter, with the frames
//! handed to whoever is listening (docs/protocol.md, "WebSocket protocol") — a port of
//! `win/src/FamilyConnect.Core/Protocol/ChatSocket.cs`.
//!
//! Three rules this loop exists to keep. A CONNECTION THAT DIED ON ARRIVAL EARNS NOTHING: a proxy
//! can accept the upgrade and drop it at once, and forgiving the ceiling at handshake made the
//! socket reconnect roughly twice a second for ever, each cycle firing a full resync — so the
//! reset is judged by how long the connection LASTED. An UNKNOWN FRAME IS IGNORED, because a newer
//! server may add one and dropping the connection over it would be worse than skipping it. And
//! `4401` IS NOT A NETWORK ERROR: the session expired mid-connection, so the client signs out
//! instead of retrying a token that will never work again.
//!
//! A fourth, which `win/` did not keep and the Mac does: an upgrade REFUSED WITH `401` is the
//! same fact before the connection opened — "a bad token fails the upgrade with `401`"
//! (docs/protocol.md, "WebSocket protocol"), and "a `401` means the session is gone"
//! ("Authentication"). `win/`'s loop read it as a network failure and knocked again with the same
//! dead token every thirty seconds for ever (measured on .NET 10: `ClientWebSocket` throws a
//! `WebSocketException` with no close status, so `CloseCode == 4401` never matched); a session
//! revoked while the laptop slept left the window at "Connecting…" with no resync to find out.
//!
//! Keeping the connection alive is the transport's: the server pings at the protocol level every
//! 30 s and drops a socket idle for 75 s, and the platform's WebSocket answers those pings and
//! pings in its turn (the C# set `KeepAliveInterval` to 30 s on `ClientWebSocket`). A `pong`
//! frame is a frame like any other here.
//!
//! Apple counterpart: `ChatSocket`. Android: `ChatSocketImpl`. Web: `web/src/socket.rs`.

use std::cell::RefCell;
use std::pin::pin;
use std::rc::Rc;
use std::time::Duration;

use futures_util::future::{Either, select};

use super::api_result::TokenStore;
use super::frames::ServerFrame;
use super::reconnect_backoff::ReconnectBackoff;
use super::send_rules::ACK_DEADLINE;
use crate::rt::{
    Cancel, Clock, Event, LocalBoxFuture, TransportError, WebSocket, WebSocketConnector,
};

/// The close code the server sends when the session expires mid-connection.
pub const SESSION_EXPIRED_CODE: u16 = 4401;

/// The status that refuses an upgrade whose token is no good (docs/protocol.md, "WebSocket
/// protocol").
pub const UPGRADE_UNAUTHORIZED: u16 = 401;

/// How long a connection has to last to earn its reset. Shorter than this and the ceiling keeps
/// climbing, which is what stops a flapping proxy becoming a resync storm.
pub const DURABLE_AFTER: Duration = Duration::from_secs(5);

/// What the send pipeline needs of a socket: whether it can write, a write that reports rather
/// than fails, and the frames coming back (`IFrameSender`).
///
/// A seam and not an abstraction for its own sake: the pipeline's tests are about the ack deadline
/// and the fall back to REST, and running a reconnect loop to exercise them would test the wrong
/// thing twice.
pub trait FrameSender {
    /// Whether a frame can be written right now.
    fn is_connected(&self) -> bool;

    /// Write one frame: whether it went. `false` is not a refusal — the outbox decides what that
    /// means.
    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool>;

    /// Every frame this client understood, in arrival order (`event Action<ServerFrame> Frame`).
    fn on_frame(&self) -> &Event<ServerFrame>;
}

/// The realtime connection as the app above it uses one: the frames, the three things that happen
/// TO a connection, and the loop that keeps it (`ILiveSocket`).
///
/// A seam, so that the layer deciding WHEN to connect and what a connection means can be tested
/// without a socket at all. [`ChatSocket`] is the one implementation; a test's fake is the other.
pub trait LiveSocket: FrameSender {
    /// A connection came up. The resync starts here, and so does the outbox flush.
    fn on_connected(&self) -> &Event<()>;

    /// It went away. Whether it comes back is the loop's business.
    fn on_disconnected(&self) -> &Event<()>;

    /// The session is gone: sign out, and do not reconnect.
    fn on_session_expired(&self) -> &Event<()>;

    /// Connect, read, reconnect — until `cancel` is raised or the session expires.
    fn run(&self, cancel: Cancel) -> LocalBoxFuture<'_, ()>;
}

/// The one implementation of [`LiveSocket`].
pub struct ChatSocket {
    connector: Rc<dyn WebSocketConnector>,
    url: Box<dyn Fn() -> String>,
    tokens: Rc<dyn TokenStore>,
    clock: Rc<dyn Clock>,
    backoff: RefCell<ReconnectBackoff>,
    live: RefCell<Option<Rc<dyn WebSocket>>>,
    frame: Event<ServerFrame>,
    connected: Event<()>,
    disconnected: Event<()>,
    session_expired: Event<()>,
}

impl std::fmt::Debug for ChatSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No URL and no token: the family's logging rule.
        f.debug_struct("ChatSocket")
            .field("connected", &self.is_connected())
            .finish_non_exhaustive()
    }
}

/// The `finally` of one connection, and what dropping the loop's future mid-connection does
/// instead: nothing is live any more, and the socket is closed.
struct Connection<'a> {
    live: &'a RefCell<Option<Rc<dyn WebSocket>>>,
    socket: Option<Rc<dyn WebSocket>>,
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        *self.live.borrow_mut() = None;
        if let Some(socket) = self.socket.take() {
            socket.close();
        }
    }
}

/// `operation`, unless `cancel` is raised first: `None` is the `OperationCanceledException` the C#
/// awaits threw. An operation that is ready is taken even when the flag is up, as a C# task that
/// completed was.
async fn unless_cancelled<T>(operation: LocalBoxFuture<'_, T>, cancel: &Cancel) -> Option<T> {
    match select(operation, pin!(cancel.cancelled())).await {
        Either::Left((value, _)) => Some(value),
        Either::Right(((), _)) => None,
    }
}

impl ChatSocket {
    /// `new ChatSocket(open, url, tokens, backoff, wait, now)`: the connector opens each socket,
    /// `url` is asked afresh for each connection (the server may have changed), and the clock is
    /// both the C#'s `now` and its `wait`. The backoff is 1 s doubling to 30 s, uniformly
    /// jittered, unless [`with_backoff`](Self::with_backoff) gives another.
    pub fn new(
        connector: Rc<dyn WebSocketConnector>,
        url: impl Fn() -> String + 'static,
        tokens: Rc<dyn TokenStore>,
        clock: Rc<dyn Clock>,
    ) -> Self {
        ChatSocket {
            connector,
            url: Box::new(url),
            tokens,
            clock,
            backoff: RefCell::new(ReconnectBackoff::default()),
            live: RefCell::new(None),
            frame: Event::new(),
            connected: Event::new(),
            disconnected: Event::new(),
            session_expired: Event::new(),
        }
    }

    /// The reconnect schedule — a test's pinned random source.
    pub fn with_backoff(self, backoff: ReconnectBackoff) -> Self {
        *self.backoff.borrow_mut() = backoff;
        self
    }

    /// Whether a frame can be written right now.
    pub fn is_connected(&self) -> bool {
        self.live.borrow().is_some()
    }

    /// Every frame this client understood, in arrival order.
    pub fn on_frame(&self) -> &Event<ServerFrame> {
        &self.frame
    }

    /// A connection came up. The resync starts here, and so does the outbox flush. Raised before
    /// the first frame of the connection is read.
    pub fn on_connected(&self) -> &Event<()> {
        &self.connected
    }

    /// It went away — raised only for a connection that came up. Whether it comes back is this
    /// loop's business.
    pub fn on_disconnected(&self) -> &Event<()> {
        &self.disconnected
    }

    /// The session is gone: sign out, and do not reconnect.
    pub fn on_session_expired(&self) -> &Event<()> {
        &self.session_expired
    }

    /// Connect, read frames, and reconnect when the connection goes — until `cancel` is raised or
    /// the session expires.
    ///
    /// Raising `cancel` ends it as the C# cancellation token did: a connection that was up raises
    /// [`on_disconnected`](Self::on_disconnected), and nothing waits or reconnects. Dropping the
    /// future instead stops it at once and raises nothing; the socket is closed either way.
    pub async fn run(&self, cancel: Cancel) {
        while !cancel.is_cancelled() {
            let url = (self.url)();
            // No token is no header: the C# adapter sent `Authorization` only for a non-empty one.
            let token = self.tokens.token().filter(|token| !token.is_empty());
            let mut connected_at = None;
            let mut expired = false;
            let mut connection = Connection {
                live: &self.live,
                socket: None,
            };
            match unless_cancelled(self.connector.connect(&url, token.as_deref()), &cancel).await {
                // The caller asked. Not a failure, and nothing to retry.
                None => {}
                // Any transport failure is the same thing here: no connection — except an upgrade
                // the server refused because the session is gone.
                Some(Err(error)) => {
                    expired = error == TransportError::UpgradeRefused(UPGRADE_UNAUTHORIZED);
                }
                Some(Ok(socket)) => {
                    let socket: Rc<dyn WebSocket> = Rc::from(socket);
                    connection.socket = Some(socket.clone());
                    connected_at = Some(self.clock.now());
                    *self.live.borrow_mut() = Some(socket.clone());
                    self.connected.emit(&());
                    let mut asked_to_stop = false;
                    while !cancel.is_cancelled() {
                        match unless_cancelled(socket.receive(), &cancel).await {
                            None => {
                                asked_to_stop = true;
                                break;
                            }
                            Some(None) => break,
                            // An unknown type parses to `None` and is skipped: a client that threw
                            // would drop the connection over something it was free to ignore.
                            Some(Some(text)) => {
                                if let Some(frame) = ServerFrame::parse(&text) {
                                    self.frame.emit(&frame);
                                }
                            }
                        }
                    }
                    // What it says about the session is decided by the close code, not by how the
                    // connection ended — unless the caller stopped it mid-read, which says nothing.
                    if !asked_to_stop {
                        expired = socket.close_code() == Some(SESSION_EXPIRED_CODE);
                    }
                }
            }
            drop(connection);
            if connected_at.is_some() {
                self.disconnected.emit(&());
            }
            if expired {
                // NOT a network error: the token will never work again, so retrying it would be a
                // client hammering a door it has no key to.
                self.session_expired.emit(&());
                return;
            }
            if cancel.is_cancelled() {
                return;
            }
            if ReconnectBackoff::earns_reset(connected_at, self.clock.now(), DURABLE_AFTER) {
                self.backoff.borrow_mut().reset();
            }
            let delay = self.backoff.borrow_mut().next_delay();
            if unless_cancelled(self.clock.sleep(delay), &cancel)
                .await
                .is_none()
            {
                return;
            }
        }
    }

    /// Write one frame, with the ack deadline as the WRITE deadline too: a socket whose TCP
    /// connection is dead absorbs a write, and the keepalive horizon is far longer than a person's
    /// patience (docs/protocol.md, "Sending on an unreliable network").
    ///
    /// Whether it went. `false` is not a refusal — the outbox decides what that means.
    pub async fn try_send(&self, frame: String) -> bool {
        let socket = self.live.borrow().clone();
        let Some(socket) = socket else {
            return false;
        };
        match select(socket.send(frame), self.clock.sleep(ACK_DEADLINE)).await {
            Either::Left((sent, _)) => sent.is_ok(),
            Either::Right(((), _)) => false,
        }
    }
}

impl FrameSender for ChatSocket {
    fn is_connected(&self) -> bool {
        ChatSocket::is_connected(self)
    }

    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool> {
        Box::pin(ChatSocket::try_send(self, frame))
    }

    fn on_frame(&self) -> &Event<ServerFrame> {
        ChatSocket::on_frame(self)
    }
}

impl LiveSocket for ChatSocket {
    fn on_connected(&self) -> &Event<()> {
        ChatSocket::on_connected(self)
    }

    fn on_disconnected(&self) -> &Event<()> {
        ChatSocket::on_disconnected(self)
    }

    fn on_session_expired(&self) -> &Event<()> {
        ChatSocket::on_session_expired(self)
    }

    fn run(&self, cancel: Cancel) -> LocalBoxFuture<'_, ()> {
        Box::pin(ChatSocket::run(self, cancel))
    }
}

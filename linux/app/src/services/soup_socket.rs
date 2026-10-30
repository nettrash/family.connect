//! fc-core's [`WebSocketConnector`] and [`WebSocket`] over libsoup 3 (shell-design §2.4;
//! carry-forward "For the shell's libsoup seams", WebSocket — implemented exactly as specified
//! there):
//!
//! - **Connect**: `soup_session_websocket_connect_async` on a `GET` of the socket URL (its host in
//!   ASCII, `soup_http::ascii_url`), `Authorization: Bearer <token>` only when a token is given, no
//!   subprotocols, origin NULL, never the token in the URL; cancelled when the future is dropped
//!   (a `GioFuture`).
//! - **A refused upgrade** — the message's HTTP status is anything but 101 — is
//!   `TransportError::UpgradeRefused(status)`. **401 MUST arrive as `UpgradeRefused(401)`: it is
//!   what signs the user out.** Any other failure is `soup_http::classify`'s.
//! - **On success**: `keepalive-interval` 30 s, `keepalive-pong-timeout` 45 s (a dead peer is
//!   noticed inside the server's 75 s idle window — the protocol's and the Mac's rule),
//!   `max-incoming-payload-size` 0 (the 128 KiB default would close a big frame with 1009).
//! - **Receiving is a QUEUE owned by the socket**, filled by the `message` signal from the moment
//!   the connection exists — so a frame that arrives before the first `receive()` is kept (fc-core
//!   raises Connected first). A BINARY frame is read as lossy UTF-8. A dropped `receive` future
//!   loses nothing (it takes a frame only when it completes). `receive` answers `None` once the
//!   socket is closed AND the queue is drained.
//! - **`closed`** records the close code (0 → `None`) and wakes the reader. Which code ends the
//!   session (only 4401) is fc-core's business, not this seam's.
//! - **`send`** = `send_text` while OPEN, `Ok` at once; `Err` otherwise.
//! - **`close()`** is idempotent: 1001 ("going away") if OPEN, nothing otherwise; a pending
//!   `receive` answers `None` at once and a later `send` fails. Dropping the socket does the same,
//!   and keeps the connection alive until the closing handshake ends (or 5 s), so the server hears
//!   1001 rather than a torn connection.
//! - **Never log the socket URL** (it carries no token, but the rule is simpler kept whole).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use fc_core::rt::{LocalBoxFuture, TransportError, WebSocket, WebSocketConnector};
use glib::translate::IntoGlib;
use soup::prelude::*;

use super::soup_http::{ascii_url, classify};

/// Seconds between pings the client sends.
pub const KEEPALIVE_INTERVAL: u32 = 30;
/// Seconds a ping may wait for its pong before the connection is closed as dead.
pub const KEEPALIVE_PONG_TIMEOUT: u32 = 45;
/// The close code of a client going away (RFC 6455 §7.4.1).
pub const GOING_AWAY: u16 = 1001;
/// How long a dropped socket's closing handshake may take before it is let go.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

/// Opens the live socket for fc-core's `ChatSocket` (`rt::WebSocketConnector`).
#[derive(Debug, Clone)]
pub struct SoupSockets {
    session: soup::Session,
}

impl SoupSockets {
    /// Over `session` — the API's (`SoupHttp::session`), so both share one connection pool and one
    /// user agent.
    pub fn new(session: soup::Session) -> SoupSockets {
        SoupSockets { session }
    }
}

impl WebSocketConnector for SoupSockets {
    fn connect(
        &self,
        url: &str,
        bearer: Option<&str>,
    ) -> LocalBoxFuture<'_, Result<Box<dyn WebSocket>, TransportError>> {
        let session = self.session.clone();
        let url = ascii_url(url);
        let authorization = bearer.map(|token| format!("Bearer {token}"));
        Box::pin(async move {
            let url = url
                .ok_or_else(|| TransportError::Other("the socket's address is not a URL".into()))?;
            let message = soup::Message::new("GET", &url)
                .map_err(|_| TransportError::Other("the socket's address is not a URL".into()))?;
            if let (Some(value), Some(headers)) = (&authorization, message.request_headers()) {
                headers.append("Authorization", value);
            }
            let connected = session
                .websocket_connect_async_future(&message, None, &[], glib::Priority::DEFAULT)
                .await;
            match connected {
                Ok(connection) => {
                    let socket: Box<dyn WebSocket> = Box::new(SoupWebSocket::new(connection));
                    Ok(socket)
                }
                Err(error) => Err(refusal(message.status().into_glib(), &error)),
            }
        })
    }
}

/// A failed connect as fc-core reads it: the HTTP status when the server answered with one that is
/// not 101 (`UpgradeRefused`), else the transport's error.
pub fn refusal(status: i32, error: &glib::Error) -> TransportError {
    match u16::try_from(status) {
        // libsoup's own statuses below 100 are not HTTP answers (0: none yet).
        Ok(status) if (100..=999).contains(&status) && status != 101 => {
            TransportError::UpgradeRefused(status)
        }
        _ => classify(error),
    }
}

/// What the signal handlers and the reader share.
#[derive(Default)]
struct Shared {
    queue: VecDeque<String>,
    /// Closed by the peer, by a failure, or by [`WebSocket::close`].
    closed: bool,
    code: Option<u16>,
    waiting: Vec<Waker>,
}

impl Shared {
    fn wake(&mut self) {
        for waker in self.waiting.drain(..) {
            waker.wake();
        }
    }
}

/// One open socket (`rt::WebSocket`).
pub struct SoupWebSocket {
    connection: soup::WebsocketConnection,
    shared: Rc<RefCell<Shared>>,
}

impl std::fmt::Debug for SoupWebSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No URL: the logging rule.
        f.debug_struct("SoupWebSocket")
            .field("state", &self.connection.state())
            .field("code", &self.shared.borrow().code)
            .finish_non_exhaustive()
    }
}

impl SoupWebSocket {
    /// Takes over a connection just opened: sets the keepalive and the payload limit, and starts
    /// queueing what arrives.
    pub fn new(connection: soup::WebsocketConnection) -> SoupWebSocket {
        connection.set_keepalive_interval(KEEPALIVE_INTERVAL);
        connection.set_keepalive_pong_timeout(KEEPALIVE_PONG_TIMEOUT);
        connection.set_max_incoming_payload_size(0);
        let shared = Rc::new(RefCell::new(Shared::default()));
        {
            let shared = Rc::clone(&shared);
            connection.connect_message(move |_, kind, bytes| {
                let text = kind == soup::WebsocketDataType::Text.into_glib()
                    || kind == soup::WebsocketDataType::Binary.into_glib();
                if text {
                    let mut shared = shared.borrow_mut();
                    shared
                        .queue
                        .push_back(String::from_utf8_lossy(bytes).into_owned());
                    shared.wake();
                }
            });
        }
        {
            let shared = Rc::clone(&shared);
            connection.connect_closed(move |connection| {
                let mut shared = shared.borrow_mut();
                shared.closed = true;
                shared.code = match connection.close_code() {
                    0 => None,
                    code => Some(code),
                };
                shared.wake();
            });
        }
        connection.connect_error(|_, error| {
            // The kind only: a protocol error's text may quote what the peer sent.
            crate::fc_debug!("socket: {}", classify(error));
        });
        SoupWebSocket { connection, shared }
    }

    /// The connection's keepalive and limit, as set (tests).
    #[cfg(test)]
    pub fn settings(&self) -> (u32, u32, u64) {
        (
            self.connection.keepalive_interval(),
            self.connection.keepalive_pong_timeout(),
            self.connection.max_incoming_payload_size(),
        )
    }

    fn close_now(&self) -> bool {
        let open = self.connection.state() == soup::WebsocketState::Open;
        if open {
            self.connection.close(GOING_AWAY, None);
        }
        let mut shared = self.shared.borrow_mut();
        shared.closed = true;
        shared.wake();
        open
    }
}

impl WebSocket for SoupWebSocket {
    fn send(&self, text: String) -> LocalBoxFuture<'_, Result<(), TransportError>> {
        let sent = if self.connection.state() == soup::WebsocketState::Open
            && !self.shared.borrow().closed
        {
            self.connection.send_text(&text);
            Ok(())
        } else {
            Err(TransportError::Unreachable(
                "the socket is not open".to_string(),
            ))
        };
        Box::pin(std::future::ready(sent))
    }

    fn receive(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(Receive {
            shared: Rc::clone(&self.shared),
        })
    }

    fn close_code(&self) -> Option<u16> {
        self.shared.borrow().code
    }

    fn close(&self) {
        self.close_now();
    }
}

impl Drop for SoupWebSocket {
    fn drop(&mut self) {
        if self.close_now() {
            // Hold the connection until its closing handshake ends, so the peer reads 1001.
            let connection = self.connection.clone();
            glib::MainContext::ref_thread_default().spawn_local(async move {
                let start = std::time::Instant::now();
                while connection.state() != soup::WebsocketState::Closed
                    && start.elapsed() < CLOSE_GRACE
                {
                    glib::timeout_future(Duration::from_millis(50)).await;
                }
                drop(connection);
            });
        }
    }
}

/// [`WebSocket::receive`]: the next queued frame, `None` once closed and drained. Takes a frame
/// only when it completes, so dropping it loses nothing.
struct Receive {
    shared: Rc<RefCell<Shared>>,
}

impl Future for Receive {
    type Output = Option<String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut shared = self.shared.borrow_mut();
        if let Some(text) = shared.queue.pop_front() {
            return Poll::Ready(Some(text));
        }
        if shared.closed {
            return Poll::Ready(None);
        }
        if !shared.waiting.iter().any(|w| w.will_wake(cx.waker())) {
            shared.waiting.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashMap;

    use futures_util::future::{Either, select};

    const PATH: &str = "/api/v1/ws";

    /// Runs `body` to completion on a fresh main context made the thread's default — the
    /// server and the client both live on it, as everything lives on GTK's in the app.
    fn on_main_context<F: Future<Output = ()>>(body: impl FnOnce() -> F) {
        let context = glib::MainContext::new();
        context
            .with_thread_default(|| context.block_on(body()))
            .expect("the context is free");
    }

    async fn pause(ms: u64) {
        glib::timeout_future(Duration::from_millis(ms)).await;
    }

    /// Polls `condition` every 10 ms for up to 3 s.
    async fn until(condition: impl Fn() -> bool) -> bool {
        for _ in 0..300 {
            if condition() {
                return true;
            }
            pause(10).await;
        }
        condition()
    }

    /// A local WebSocket server (libsoup's own): `Bearer good` is let in, anything else is
    /// answered 401 before the handshake; `greet` runs on each accepted connection.
    struct Loopback {
        _server: soup::Server,
        url: String,
        connections: Rc<RefCell<Vec<soup::WebsocketConnection>>>,
        authorizations: Rc<RefCell<Vec<Option<String>>>>,
        closed_with: Rc<Cell<Option<u16>>>,
    }

    fn loopback(greet: impl Fn(&soup::WebsocketConnection) + 'static) -> Loopback {
        let server = soup::Server::builder().build();
        let authorizations = Rc::new(RefCell::new(Vec::new()));
        {
            let authorizations = Rc::clone(&authorizations);
            server.add_early_handler(Some(PATH), move |_, message, _, _: HashMap<&str, &str>| {
                let given = message
                    .request_headers()
                    .and_then(|headers| headers.one("Authorization"))
                    .map(|value| value.to_string());
                if given.as_deref() != Some("Bearer good") {
                    message.set_status(401, None);
                }
                authorizations.borrow_mut().push(given);
            });
        }
        let connections = Rc::new(RefCell::new(Vec::new()));
        let closed_with = Rc::new(Cell::new(None));
        {
            let (connections, closed_with) = (Rc::clone(&connections), Rc::clone(&closed_with));
            server.add_websocket_handler(Some(PATH), None, &[], move |_, _, _, connection| {
                let closed_with = Rc::clone(&closed_with);
                connection.connect_closed(move |connection| {
                    closed_with.set(Some(connection.close_code()));
                });
                greet(connection);
                connections.borrow_mut().push(connection.clone());
            });
        }
        server
            .listen_local(0, soup::ServerListenOptions::IPV4_ONLY)
            .expect("listen on loopback");
        let port = server.uris().first().expect("a uri").port();
        Loopback {
            _server: server,
            url: format!("ws://127.0.0.1:{port}{PATH}"),
            connections,
            authorizations,
            closed_with,
        }
    }

    fn connector() -> SoupSockets {
        SoupSockets::new(soup::Session::new())
    }

    #[test]
    fn frames_that_arrive_before_the_first_receive_are_queued_and_a_dropped_receive_loses_none() {
        on_main_context(|| async {
            let server = loopback(|connection| {
                connection.send_text("one");
                connection.send_text("two");
            });
            let socket = connector()
                .connect(&server.url, Some("good"))
                .await
                .expect("connected");
            // Nobody reads for a while: the frames wait in the socket's own queue.
            pause(200).await;
            assert_eq!(socket.receive().await.as_deref(), Some("one"));
            assert_eq!(socket.receive().await.as_deref(), Some("two"));
            // A receive given up before anything arrives takes nothing with it.
            match select(socket.receive(), Box::pin(pause(100))).await {
                Either::Left(_) => panic!("nothing was sent"),
                Either::Right(_) => {}
            }
            server.connections.borrow()[0].send_text("four");
            assert_eq!(socket.receive().await.as_deref(), Some("four"));
            assert_eq!(socket.close_code(), None);
            assert_eq!(
                server.authorizations.borrow().as_slice(),
                [Some("Bearer good".to_string())]
            );
        });
    }

    #[test]
    fn the_keepalive_and_the_payload_limit_are_set_on_every_connection() {
        on_main_context(|| async {
            let server = loopback(|_| {});
            let connection = soup::Session::new()
                .websocket_connect_async_future(
                    &{
                        let message = soup::Message::new("GET", &server.url).unwrap();
                        message
                            .request_headers()
                            .unwrap()
                            .append("Authorization", "Bearer good");
                        message
                    },
                    None,
                    &[],
                    glib::Priority::DEFAULT,
                )
                .await
                .expect("connected");
            let socket = SoupWebSocket::new(connection);
            assert_eq!(socket.settings(), (30, 45, 0));
            // A frame far past libsoup's 128 KiB default arrives whole.
            let big = "x".repeat(300 * 1024);
            server.connections.borrow()[0].send_text(&big);
            assert_eq!(socket.receive().await.map(|t| t.len()), Some(big.len()));
        });
    }

    #[test]
    fn a_401_upgrade_is_upgrade_refused_401_and_no_token_sends_no_header() {
        on_main_context(|| async {
            let server = loopback(|_| {});
            let refused = connector().connect(&server.url, Some("stale")).await;
            assert_eq!(refused.err(), Some(TransportError::UpgradeRefused(401)));
            let anonymous = connector().connect(&server.url, None).await;
            assert_eq!(anonymous.err(), Some(TransportError::UpgradeRefused(401)));
            assert_eq!(
                server.authorizations.borrow().as_slice(),
                [Some("Bearer stale".to_string()), None]
            );
            // A path with no socket behind it: the server's 404 is a refusal too.
            let elsewhere = server.url.replace(PATH, "/nowhere");
            assert_eq!(
                connector().connect(&elsewhere, Some("good")).await.err(),
                Some(TransportError::UpgradeRefused(404))
            );
        });
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        on_main_context(|| async {
            let server = loopback(|_| {});
            let url = server.url.clone();
            drop(server);
            pause(50).await;
            let failed = connector().connect(&url, Some("good")).await;
            assert!(
                matches!(failed, Err(TransportError::Unreachable(_))),
                "{:?}",
                failed.err()
            );
        });
    }

    #[test]
    fn the_servers_close_code_is_recorded_and_receive_answers_none_after_the_queue() {
        on_main_context(|| async {
            let server = loopback(|connection| {
                connection.send_text("last words");
            });
            let socket = connector()
                .connect(&server.url, Some("good"))
                .await
                .expect("connected");
            assert!(until(|| !server.connections.borrow().is_empty()).await);
            server.connections.borrow()[0].close(4401, Some("session expired"));
            assert_eq!(socket.receive().await.as_deref(), Some("last words"));
            assert_eq!(socket.receive().await, None);
            assert_eq!(socket.close_code(), Some(4401));
            assert!(socket.send("late".into()).await.is_err());
        });
    }

    #[test]
    fn close_is_idempotent_and_says_going_away_and_so_does_a_drop() {
        on_main_context(|| async {
            let server = loopback(|_| {});
            let socket = connector()
                .connect(&server.url, Some("good"))
                .await
                .expect("connected");
            assert!(socket.send("hello".into()).await.is_ok());
            let waiting = socket.receive();
            socket.close();
            socket.close();
            assert_eq!(waiting.await, None);
            assert!(socket.send("after".into()).await.is_err());
            assert!(until(|| server.closed_with.get().is_some()).await);
            assert_eq!(server.closed_with.get(), Some(GOING_AWAY));

            let second = loopback(|_| {});
            let dropped = connector()
                .connect(&second.url, Some("good"))
                .await
                .expect("connected");
            drop(dropped);
            assert!(until(|| second.closed_with.get().is_some()).await);
            assert_eq!(second.closed_with.get(), Some(GOING_AWAY));
        });
    }

    /// A one-shot WebSocket server written by hand on a std thread — libsoup's binding cannot send
    /// a BINARY frame — that accepts one handshake, sends `frames` (opcode, payload) unmasked and
    /// waits for the client to go.
    fn raw_server(frames: Vec<(u8, Vec<u8>)>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                if stream.read(&mut byte).unwrap_or(0) == 0 {
                    return;
                }
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request).into_owned();
            let key = request
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("sec-websocket-key")
                        .then(|| value.trim().to_string())
                })
                .unwrap();
            let digest = glib::compute_checksum_for_string(
                glib::ChecksumType::Sha1,
                format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11"),
            )
            .unwrap();
            let raw: Vec<u8> = (0..digest.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&digest[i..i + 2], 16).unwrap())
                .collect();
            let accept = glib::base64_encode(&raw);
            let mut out = format!(
                "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
            )
            .into_bytes();
            for (opcode, payload) in frames {
                out.push(0x80 | opcode);
                out.push(payload.len() as u8);
                out.extend_from_slice(&payload);
            }
            stream.write_all(&out).unwrap();
            let mut rest = [0u8; 256];
            while stream.read(&mut rest).unwrap_or(0) > 0 {}
        });
        format!("ws://127.0.0.1:{port}{PATH}")
    }

    #[test]
    fn a_binary_frame_is_read_as_lossy_utf8_in_its_place() {
        on_main_context(|| async {
            let url = raw_server(vec![
                (0x1, b"text".to_vec()),
                (0x2, b"thr\xffee".to_vec()),
                (0x1, b"after".to_vec()),
            ]);
            let socket = connector().connect(&url, None).await.expect("connected");
            assert_eq!(socket.receive().await.as_deref(), Some("text"));
            assert_eq!(socket.receive().await.as_deref(), Some("thr\u{fffd}ee"));
            assert_eq!(socket.receive().await.as_deref(), Some("after"));
            socket.close();
        });
    }

    #[test]
    fn a_refusal_is_read_from_the_status() {
        let error = glib::Error::new(gio::IOErrorEnum::ConnectionRefused, "x");
        assert_eq!(refusal(401, &error), TransportError::UpgradeRefused(401));
        assert_eq!(refusal(200, &error), TransportError::UpgradeRefused(200));
        assert_eq!(refusal(503, &error), TransportError::UpgradeRefused(503));
        assert!(matches!(refusal(0, &error), TransportError::Unreachable(_)));
        assert!(matches!(
            refusal(101, &error),
            TransportError::Unreachable(_)
        ));
        // libsoup's transport pseudo-statuses are not answers.
        assert!(matches!(refusal(7, &error), TransportError::Unreachable(_)));
    }
}

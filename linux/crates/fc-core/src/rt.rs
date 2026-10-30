//! The seams between the portable core and the platform: what the C# gets injected as an
//! `HttpClient`, an `IWebSocket`, a clock and a `CancellationToken`.
//!
//! Everything here is single-threaded: futures are not `Send`, and the app drives them on the GLib
//! main context (libsoup for HTTP and the WebSocket, GLib for time). Cancelling a request is
//! dropping its future; [`Cancel`] is only for work that checks part-way, as the C# checks
//! `ct.IsCancellationRequested`, and can cut a wait short. [`Event`] is what a C# `event` is on
//! one thread.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use time::OffsetDateTime;

/// A future that need not be `Send`, boxed so a trait can return it.
pub type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Patch => "PATCH",
            Method::Delete => "DELETE",
        }
    }
}

/// One HTTP request. Bodies are whole, as the C# client's are (`ReadOnlyMemoryContent`,
/// `ReadAsByteArrayAsync`): the largest is an attachment the server caps anyway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    /// Absolute, already joined to the server's base URL.
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<Vec<u8>>,
    /// The longest the transport may take; `None` for none at all (the C# client's
    /// `Timeout.InfiniteTimeSpan`, with its deadlines kept per request by the caller).
    pub timeout: Option<Duration>,
}

impl HttpRequest {
    pub fn new(method: Method, url: impl Into<String>) -> Self {
        HttpRequest {
            method,
            url: url.into(),
            headers: Vec::new(),
            body: None,
            timeout: None,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub fn body(mut self, content_type: &str, bytes: Vec<u8>) -> Self {
        self.headers
            .push(("Content-Type".into(), content_type.into()));
        self.body = Some(bytes);
        self
    }

    /// Every value of the headers of that name, compared without case as HTTP does, in the order
    /// they were added — what a test asks of a request it was sent.
    pub fn header_values(&self, name: &str) -> Vec<&str> {
        self.headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// The first header of that name, compared without case as HTTP does.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Why a request or a socket never got an answer. A status code, however bad, is an answer to a
/// REQUEST and is never one of these — with one exception, the socket's upgrade, which has no
/// answer but `101` ([`UpgradeRefused`](Self::UpgradeRefused)).
///
/// The strings are for a log: an implementation never puts a token, a name or a URL with a path
/// in them (the family's logging rule).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// The request's own deadline passed.
    Timeout,
    /// No connection: DNS, refused, reset, no network.
    Unreachable(String),
    /// The TLS handshake failed or the certificate was refused.
    Tls(String),
    /// The WebSocket upgrade was answered with this HTTP status instead of `101 Switching
    /// Protocols`. Only a [`WebSocketConnector`] produces it. `401` is a session that is gone
    /// (docs/protocol.md, "WebSocket protocol": "a bad token fails the upgrade with `401`"), and
    /// the socket stops rather than knock again with the same dead token.
    UpgradeRefused(u16),
    Other(String),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportError::Timeout => f.write_str("timed out"),
            TransportError::Unreachable(why) => write!(f, "unreachable: {why}"),
            TransportError::Tls(why) => write!(f, "TLS: {why}"),
            TransportError::UpgradeRefused(status) => {
                write!(f, "the upgrade was answered with {status}")
            }
            TransportError::Other(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for TransportError {}

/// HTTP, as the platform does it (libsoup in the app, [`crate::testing::FakeHttp`] in tests).
pub trait Http {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>>;
}

/// Opens the live socket. The token goes in `Authorization: Bearer` — a native client can set
/// headers (docs/protocol.md, "WebSocket protocol"); the subprotocols are the browser's workaround.
/// `bearer` is `None` when there is no token, and then no `Authorization` header is sent at all.
///
/// An upgrade answered with anything but `101` is [`TransportError::UpgradeRefused`] with that
/// status; every other failure to connect is one of the other variants.
pub trait WebSocketConnector {
    fn connect(
        &self,
        url: &str,
        bearer: Option<&str>,
    ) -> LocalBoxFuture<'_, Result<Box<dyn WebSocket>, TransportError>>;
}

/// One open socket (the C# `IWebSocket` after `ConnectAsync`).
///
/// A `send` and a `receive` may be pending at the same time — the reader waits on `receive` for
/// as long as the connection lives, and the outbox writes beside it.
pub trait WebSocket {
    fn send(&self, text: String) -> LocalBoxFuture<'_, Result<(), TransportError>>;
    /// The next text frame, or `None` once the connection has closed.
    fn receive(&self) -> LocalBoxFuture<'_, Option<String>>;
    /// Why it closed, once it has.
    fn close_code(&self) -> Option<u16>;
    /// Close it now, if it is not closed already: what the C# did with `Dispose` when a
    /// connection's loop ended. A pending `receive` then answers `None` and a later `send` fails.
    /// Idempotent. The default does nothing, for a socket that closes itself on drop.
    fn close(&self) {}
}

/// Wall time and waiting, so no test ever waits for real.
pub trait Clock {
    fn now(&self) -> OffsetDateTime;
    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()>;
}

/// A cancellation flag for work that checks part-way, and that a wait can be cut short by —
/// [`cancelled`](Self::cancelled) is a future that ends when it is raised. Clones share the flag.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Rc<CancelState>);

#[derive(Debug, Default)]
struct CancelState {
    raised: Cell<bool>,
    waiting: RefCell<Vec<Waker>>,
}

impl Cancel {
    pub fn new() -> Self {
        Cancel::default()
    }

    /// Raise it, and wake everything waiting on [`cancelled`](Self::cancelled).
    pub fn cancel(&self) {
        self.0.raised.set(true);
        let waiting = std::mem::take(&mut *self.0.waiting.borrow_mut());
        for waker in waiting {
            waker.wake();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.raised.get()
    }

    /// A future that ends once the flag is raised — at once if it already is.
    pub fn cancelled(&self) -> Cancelled {
        Cancelled(self.0.clone())
    }
}

/// [`Cancel::cancelled`].
#[derive(Debug)]
pub struct Cancelled(Rc<CancelState>);

impl Future for Cancelled {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0.raised.get() {
            return Poll::Ready(());
        }
        let mut waiting = self.0.waiting.borrow_mut();
        if !waiting.iter().any(|waker| waker.will_wake(cx.waker())) {
            waiting.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

/// What a C# `event` is, on one thread: handlers called in the order they subscribed, each with
/// the same value. Emitting calls the handlers subscribed WHEN IT STARTED — a handler may
/// subscribe or unsubscribe (itself included) while it runs, as a C# multicast delegate's
/// invocation list is fixed when it is invoked.
pub struct Event<A: ?Sized> {
    handlers: RefCell<Vec<(u64, Rc<dyn Fn(&A)>)>>,
    next: Cell<u64>,
}

/// A handler's place on an [`Event`], for [`Event::unsubscribe`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Subscription(u64);

impl<A: ?Sized> Default for Event<A> {
    fn default() -> Self {
        Event {
            handlers: RefCell::new(Vec::new()),
            next: Cell::new(0),
        }
    }
}

impl<A: ?Sized> fmt::Debug for Event<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Event")
            .field("handlers", &self.handlers.borrow().len())
            .finish()
    }
}

impl<A: ?Sized> Event<A> {
    pub fn new() -> Self {
        Event::default()
    }

    /// `event += handler`.
    pub fn subscribe(&self, handler: impl Fn(&A) + 'static) -> Subscription {
        let id = self.next.get();
        self.next.set(id + 1);
        self.handlers.borrow_mut().push((id, Rc::new(handler)));
        Subscription(id)
    }

    /// `event -= handler`: whether it was still subscribed.
    pub fn unsubscribe(&self, subscription: Subscription) -> bool {
        let mut handlers = self.handlers.borrow_mut();
        let before = handlers.len();
        handlers.retain(|(id, _)| *id != subscription.0);
        handlers.len() != before
    }

    /// `event?.Invoke(value)`.
    pub fn emit(&self, value: &A) {
        let handlers: Vec<Rc<dyn Fn(&A)>> = self
            .handlers
            .borrow()
            .iter()
            .map(|(_, handler)| handler.clone())
            .collect();
        for handler in handlers {
            handler(value);
        }
    }

    /// How many handlers are subscribed.
    pub fn len(&self) -> usize {
        self.handlers.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.handlers.borrow().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt;

    #[test]
    fn a_cancel_wakes_whoever_waits_on_it_and_answers_at_once_after() {
        let cancel = Cancel::new();
        let mut pool = LocalPool::new();
        let ended = Rc::new(Cell::new(false));
        let (waiting, flag) = (cancel.cancelled(), ended.clone());
        pool.spawner()
            .spawn_local(async move {
                waiting.await;
                flag.set(true);
            })
            .unwrap();
        pool.run_until_stalled();
        assert!(!ended.get());
        cancel.clone().cancel();
        pool.run_until_stalled();
        assert!(ended.get());
        assert!(cancel.is_cancelled());
        pool.run_until(cancel.cancelled());
    }

    #[test]
    fn an_event_calls_every_handler_in_order_and_one_unsubscribed_is_not_called() {
        let event: Event<i32> = Event::new();
        let heard = Rc::new(RefCell::new(Vec::new()));
        let (first, second) = (heard.clone(), heard.clone());
        let one = event.subscribe(move |value| first.borrow_mut().push(("one", *value)));
        event.subscribe(move |value| second.borrow_mut().push(("two", *value)));
        event.emit(&7);
        assert!(event.unsubscribe(one));
        assert!(!event.unsubscribe(one));
        event.emit(&8);
        assert_eq!(*heard.borrow(), vec![("one", 7), ("two", 7), ("two", 8)]);
        assert_eq!(event.len(), 1);
    }

    #[test]
    fn a_handler_may_subscribe_while_the_event_is_emitting() {
        let event: Rc<Event<()>> = Rc::new(Event::new());
        let calls = Rc::new(Cell::new(0));
        let (inner, counted) = (event.clone(), calls.clone());
        event.subscribe(move |_| {
            counted.set(counted.get() + 1);
            let again = counted.clone();
            inner.subscribe(move |_| again.set(again.get() + 100));
        });
        event.emit(&());
        // The handler added during the first emit is not part of it.
        assert_eq!(calls.get(), 1);
        event.emit(&());
        assert_eq!(calls.get(), 102);
    }
}

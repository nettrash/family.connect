//! Fakes of the [`crate::rt`] seams for the tests of this crate, `fc-logic` and the app: a clock
//! that moves only when told to ([`FakeClock`]) and one that never waits at all ([`StillClock`]),
//! an HTTP transport that answers from a script and keeps what it was sent ([`FakeHttp`]) and one
//! that answers by route ([`FakeServer`]), a WebSocket and its connector that play a script of
//! connections, the send pipeline's socket ([`FakeFrameSender`]), and [`drive`], which runs a
//! future to its end on the fake clock without waiting for anything real.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::fmt;
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

use time::OffsetDateTime;

use crate::protocol::{FrameSender, ServerFrame};
use crate::rt::{
    Cancel, Clock, Event, Http, HttpRequest, HttpResponse, LocalBoxFuture, Method, TransportError,
    WebSocket, WebSocketConnector,
};

#[derive(Debug)]
struct ClockState {
    now: OffsetDateTime,
    /// The sleeps not yet ended: deadline, the sleep's own id, and who to wake. A sleep that is
    /// dropped before its deadline takes its entry with it, so an abandoned deadline (the losing
    /// side of a race) never counts as waiting and is never advanced to.
    waiting: Vec<(OffsetDateTime, u64, Waker)>,
    next_sleep: u64,
    /// Every wait asked of [`Clock::sleep`], oldest first.
    slept: Vec<Duration>,
}

/// A clock that stands still until [`FakeClock::advance`]; a `sleep` ends when the clock has been
/// moved past its deadline (and the executor is run again). Clones share the time.
#[derive(Debug, Clone)]
pub struct FakeClock(Rc<RefCell<ClockState>>);

impl FakeClock {
    pub fn at(now: OffsetDateTime) -> Self {
        FakeClock(Rc::new(RefCell::new(ClockState {
            now,
            waiting: Vec::new(),
            next_sleep: 0,
            slept: Vec::new(),
        })))
    }

    pub fn advance(&self, by: Duration) {
        let due: Vec<Waker> = {
            let mut state = self.0.borrow_mut();
            state.now += by;
            let now = state.now;
            let (due, later): (Vec<_>, Vec<_>) =
                state.waiting.drain(..).partition(|(at, _, _)| *at <= now);
            state.waiting = later;
            due.into_iter().map(|(_, _, waker)| waker).collect()
        };
        for waker in due {
            waker.wake();
        }
    }

    /// How many sleeps have not ended yet.
    pub fn sleepers(&self) -> usize {
        self.0.borrow().waiting.len()
    }

    /// Move the clock to the earliest deadline a sleep is waiting for, waking it (and any other
    /// due then); `false` when nothing is waiting.
    pub fn advance_to_next(&self) -> bool {
        let next = {
            let state = self.0.borrow();
            let Some(earliest) = state.waiting.iter().map(|(at, _, _)| *at).min() else {
                return false;
            };
            earliest - state.now
        };
        // Never negative — `advance` takes every due sleep off the list — but a conversion that
        // could fail is not unwrapped.
        self.advance(Duration::try_from(next).unwrap_or(Duration::ZERO));
        true
    }

    /// Every wait asked of [`Clock::sleep`] so far, oldest first — what the C# tests recorded with
    /// an injected `wait`.
    pub fn slept(&self) -> Vec<Duration> {
        self.0.borrow().slept.clone()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> OffsetDateTime {
        self.0.borrow().now
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        let (until, id) = {
            let mut state = self.0.borrow_mut();
            state.slept.push(duration);
            state.next_sleep += 1;
            (state.now + duration, state.next_sleep)
        };
        Box::pin(Sleep {
            clock: self.0.clone(),
            until,
            id,
        })
    }
}

struct Sleep {
    clock: Rc<RefCell<ClockState>>,
    until: OffsetDateTime,
    id: u64,
}

impl Future for Sleep {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.clock.borrow_mut();
        let id = self.id;
        state.waiting.retain(|(_, sleep, _)| *sleep != id);
        if state.now >= self.until {
            return Poll::Ready(());
        }
        state.waiting.push((self.until, id, cx.waker().clone()));
        Poll::Pending
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Ok(mut state) = self.clock.try_borrow_mut() {
            let id = self.id;
            state.waiting.retain(|(_, sleep, _)| *sleep != id);
        }
    }
}

#[derive(Debug)]
struct Scripted {
    method: Method,
    /// Matched against the request URL's path and query: equal, or a prefix followed by `?`.
    path: String,
    answer: Result<HttpResponse, TransportError>,
}

/// A standing answer: any method, any number of times, by the path alone (query aside).
#[derive(Debug)]
struct Served {
    path: String,
    status: u16,
    json: String,
}

/// An HTTP transport that answers from a script, first match first, each answer used once; then
/// from the standing answers of [`serve`](Self::serve); a request neither matches is
/// `Unreachable`. Clones share the script and the log.
#[derive(Debug, Clone, Default)]
pub struct FakeHttp {
    script: Rc<RefCell<VecDeque<Scripted>>>,
    served: Rc<RefCell<Vec<Served>>>,
    sent: Rc<RefCell<Vec<HttpRequest>>>,
    /// How long each request waits before it is answered, in the order they are sent, on the
    /// clock given with it.
    delays: Rc<RefCell<VecDeque<(FakeClock, Duration)>>>,
}

impl FakeHttp {
    pub fn new() -> Self {
        FakeHttp::default()
    }

    /// Answer the next `method path` with `status` and a JSON (or empty) body.
    pub fn respond(&self, method: Method, path: &str, status: u16, json: &str) -> &Self {
        self.answer(method, path, Ok(json_response(status, json, &[])))
    }

    /// Answer the next `method path` with exactly this.
    pub fn answer(
        &self,
        method: Method,
        path: &str,
        answer: Result<HttpResponse, TransportError>,
    ) -> &Self {
        self.script.borrow_mut().push_back(Scripted {
            method,
            path: path.to_string(),
            answer,
        });
        self
    }

    /// Answer EVERY request for `path` — any method, the query aside, as often as it is asked —
    /// with `status` and a JSON (or empty) body, when nothing in the script matches it first. The
    /// C# tests' routing `Server`.
    pub fn serve(&self, path: &str, status: u16, json: &str) -> &Self {
        self.served.borrow_mut().push(Served {
            path: path.to_string(),
            status,
            json: json.to_string(),
        });
        self
    }

    /// The next request not yet sent waits `by` on `clock` before it is answered — and only then
    /// takes its answer from the script, so a request whose deadline passes first never uses one.
    /// Delays are taken in the order requests are sent; a request with none answers at once.
    pub fn delay(&self, clock: &FakeClock, by: Duration) -> &Self {
        self.delays.borrow_mut().push_back((clock.clone(), by));
        self
    }

    /// Every request sent so far, oldest first.
    pub fn sent(&self) -> Vec<HttpRequest> {
        self.sent.borrow().clone()
    }

    /// The path and query of every request sent so far, oldest first (`PathAndQuery`).
    pub fn asked(&self) -> Vec<String> {
        self.sent
            .borrow()
            .iter()
            .map(|request| path_of(&request.url).to_string())
            .collect()
    }

    /// Scripted answers nobody asked for.
    pub fn unused(&self) -> usize {
        self.script.borrow().len()
    }

    fn answer_for(&self, method: Method, path: &str) -> Result<HttpResponse, TransportError> {
        let mut script = self.script.borrow_mut();
        let found = script.iter().position(|s| {
            s.method == method
                && (s.path == path
                    || (path.starts_with(&s.path) && path[s.path.len()..].starts_with('?')))
        });
        if let Some(scripted) = found.and_then(|i| script.remove(i)) {
            return scripted.answer;
        }
        let bare = path.split('?').next().unwrap_or(path);
        if let Some(served) = self.served.borrow().iter().find(|s| s.path == bare) {
            return Ok(json_response(served.status, &served.json, &[]));
        }
        Err(TransportError::Unreachable(format!(
            "nothing scripted for {} {path}",
            method.as_str()
        )))
    }
}

/// A response with a JSON (or, for `""`, no) body and the extra headers given — what a test
/// scripts with [`FakeHttp::answer`] when it needs a header such as `Retry-After`.
pub fn json_response(status: u16, json: &str, headers: &[(&str, &str)]) -> HttpResponse {
    let mut all: Vec<(String, String)> = headers
        .iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
    if !json.is_empty() {
        all.push(("Content-Type".to_string(), "application/json".to_string()));
    }
    HttpResponse {
        status,
        headers: all,
        body: json.as_bytes().to_vec(),
    }
}

/// The path and query of an absolute URL: everything from the first `/` after `://`.
fn path_of(url: &str) -> &str {
    let rest = url.find("://").map_or(url, |i| &url[i + 3..]);
    rest.find('/').map_or("/", |i| &rest[i..])
}

impl Http for FakeHttp {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let path = path_of(&request.url).to_string();
        let method = request.method;
        self.sent.borrow_mut().push(request);
        let delay = self.delays.borrow_mut().pop_front();
        match delay {
            None => {
                let answer = self.answer_for(method, &path);
                Box::pin(async move { answer })
            }
            Some((clock, by)) => Box::pin(async move {
                clock.sleep(by).await;
                self.answer_for(method, &path)
            }),
        }
    }
}

// ---- the socket -------------------------------------------------------------------------------

/// What a [`FakeWebSocket`] does with a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sends {
    Accepted,
    Failing,
    /// Never finishes: a dead TCP connection absorbing a write.
    Stalled,
}

#[derive(Debug)]
struct SocketState {
    frames: RefCell<VecDeque<String>>,
    close_code: Cell<Option<u16>>,
    held_open: Cell<bool>,
    receiver: RefCell<Option<Waker>>,
    sends: Cell<Sends>,
    sent: RefCell<Vec<String>>,
    closed: Cell<bool>,
}

/// One scripted connection — the C# tests' `Scripted` socket: the frames it hands over, then how
/// it ends. Each frame is handed over after one yield, because a real receive yields and a fake
/// that answered synchronously would invent an ordering the transport cannot have. After the
/// last frame `receive` answers `None` (the connection closed), unless the socket is
/// [`held_open`](Self::held_open). Clones share the socket, so a test keeps one and the
/// connector hands out another.
#[derive(Debug, Clone)]
pub struct FakeWebSocket(Rc<SocketState>);

impl FakeWebSocket {
    pub fn new(frames: &[&str]) -> Self {
        FakeWebSocket(Rc::new(SocketState {
            frames: RefCell::new(frames.iter().map(|frame| frame.to_string()).collect()),
            close_code: Cell::new(None),
            held_open: Cell::new(false),
            receiver: RefCell::new(None),
            sends: Cell::new(Sends::Accepted),
            sent: RefCell::new(Vec::new()),
            closed: Cell::new(false),
        }))
    }

    /// The close code it reports — `4401` for a session that expired mid-connection.
    pub fn closing_with(self, code: u16) -> Self {
        self.0.close_code.set(Some(code));
        self
    }

    /// After its frames it stays open — `receive` waits — until [`release`](Self::release),
    /// [`push`](Self::push) or `close`.
    pub fn held_open(self) -> Self {
        self.0.held_open.set(true);
        self
    }

    /// Every write fails.
    pub fn failing_sends(self) -> Self {
        self.0.sends.set(Sends::Failing);
        self
    }

    /// Every write waits for ever (and is still recorded).
    pub fn stalled_sends(self) -> Self {
        self.0.sends.set(Sends::Stalled);
        self
    }

    /// Let a held-open connection close: the waiting `receive` answers `None`.
    pub fn release(&self) {
        self.0.held_open.set(false);
        self.wake_receiver();
    }

    /// One more frame from the server, on a connection that is still open.
    pub fn push(&self, frame: &str) {
        self.0.frames.borrow_mut().push_back(frame.to_string());
        self.wake_receiver();
    }

    /// Every frame written to it, oldest first.
    pub fn sent(&self) -> Vec<String> {
        self.0.sent.borrow().clone()
    }

    /// Whether it was closed — the C# `Disposed`.
    pub fn is_closed(&self) -> bool {
        self.0.closed.get()
    }

    fn wake_receiver(&self) {
        let waiting = self.0.receiver.borrow_mut().take();
        if let Some(waker) = waiting {
            waker.wake();
        }
    }
}

/// `receive`'s future: one yield before each frame, then the frame; the end of a held-open
/// connection waits.
struct Receive {
    socket: Rc<SocketState>,
    yielded: bool,
}

impl Future for Receive {
    type Output = Option<String>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        if self.socket.closed.get() {
            return Poll::Ready(None);
        }
        if !self.socket.frames.borrow().is_empty() {
            if !self.yielded {
                self.yielded = true;
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }
            return Poll::Ready(self.socket.frames.borrow_mut().pop_front());
        }
        if self.socket.held_open.get() {
            *self.socket.receiver.borrow_mut() = Some(cx.waker().clone());
            return Poll::Pending;
        }
        Poll::Ready(None)
    }
}

impl WebSocket for FakeWebSocket {
    fn send(&self, text: String) -> LocalBoxFuture<'_, Result<(), TransportError>> {
        if self.0.closed.get() {
            return Box::pin(async { Err(TransportError::Other("the socket is closed".into())) });
        }
        self.0.sent.borrow_mut().push(text);
        match self.0.sends.get() {
            Sends::Accepted => Box::pin(async { Ok(()) }),
            Sends::Failing => Box::pin(async { Err(TransportError::Other("reset".into())) }),
            Sends::Stalled => Box::pin(std::future::pending()),
        }
    }

    fn receive(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(Receive {
            socket: self.0.clone(),
            yielded: false,
        })
    }

    fn close_code(&self) -> Option<u16> {
        self.0.close_code.get()
    }

    fn close(&self) {
        self.0.closed.set(true);
        self.wake_receiver();
    }
}

#[derive(Default)]
struct ConnectorState {
    script: RefCell<VecDeque<Result<FakeWebSocket, TransportError>>>,
    opened: RefCell<Vec<(String, Option<String>)>>,
    sockets: RefCell<Vec<FakeWebSocket>>,
    cancel_when_empty: RefCell<Option<Cancel>>,
    on_connect: RefCell<Option<Rc<dyn Fn()>>>,
}

/// Opens the scripted connections in order — the C# tests' `open` callback over a queue of
/// `Scripted` sockets — and keeps every URL and token it was asked with. Past the end of the
/// script it refuses with `Unreachable`, or, after [`cancel_when_empty`](Self::cancel_when_empty),
/// raises that flag and opens an idle connection that closes at once, as the C# harness did so
/// that a loop that misbehaves still ends. Clones share the script.
#[derive(Clone, Default)]
pub struct FakeConnector(Rc<ConnectorState>);

impl fmt::Debug for FakeConnector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeConnector")
            .field("scripted", &self.0.script.borrow().len())
            .field("opened", &self.0.opened.borrow().len())
            .finish_non_exhaustive()
    }
}

impl FakeConnector {
    pub fn new() -> Self {
        FakeConnector::default()
    }

    /// The next connection opens, and is this socket.
    pub fn then(&self, socket: FakeWebSocket) -> &Self {
        self.0.script.borrow_mut().push_back(Ok(socket));
        self
    }

    /// The next connection never opens.
    pub fn then_refuse(&self, error: TransportError) -> &Self {
        self.0.script.borrow_mut().push_back(Err(error));
        self
    }

    /// Past the end of the script: raise `cancel`, and open an idle connection.
    pub fn cancel_when_empty(&self, cancel: &Cancel) -> &Self {
        *self.0.cancel_when_empty.borrow_mut() = Some(cancel.clone());
        self
    }

    /// Run `hook` at the start of every connect, before its outcome — the C# tests' `open`
    /// callback doing something of its own.
    pub fn on_connect(&self, hook: impl Fn() + 'static) -> &Self {
        *self.0.on_connect.borrow_mut() = Some(Rc::new(hook));
        self
    }

    /// The URL and the bearer token of every connect, oldest first.
    pub fn opened(&self) -> Vec<(String, Option<String>)> {
        self.0.opened.borrow().clone()
    }

    /// Every socket handed out, oldest first.
    pub fn sockets(&self) -> Vec<FakeWebSocket> {
        self.0.sockets.borrow().clone()
    }
}

impl WebSocketConnector for FakeConnector {
    fn connect(
        &self,
        url: &str,
        bearer: Option<&str>,
    ) -> LocalBoxFuture<'_, Result<Box<dyn WebSocket>, TransportError>> {
        self.0
            .opened
            .borrow_mut()
            .push((url.to_string(), bearer.map(str::to_string)));
        let hook = self.0.on_connect.borrow().clone();
        if let Some(hook) = hook {
            hook();
        }
        let next = self.0.script.borrow_mut().pop_front();
        let outcome = match next {
            Some(outcome) => outcome,
            None => match self.0.cancel_when_empty.borrow().clone() {
                Some(cancel) => {
                    cancel.cancel();
                    Ok(FakeWebSocket::new(&[]))
                }
                None => Err(TransportError::Unreachable("nothing scripted".into())),
            },
        };
        let outcome = outcome.map(|socket| {
            self.0.sockets.borrow_mut().push(socket.clone());
            Box::new(socket) as Box<dyn WebSocket>
        });
        Box::pin(async move { outcome })
    }
}

// ---- a clock that never waits ----------------------------------------------------------------

/// A clock whose time never moves and whose every sleep ends at once — the C# tests'
/// `now: () => Now, wait: (_, _) => Task.CompletedTask`: a deadline that is "the same decision ten
/// seconds later" and a suite that finishes, with the time a row records exactly the one given.
/// Clones share the record of what was slept.
#[derive(Debug, Clone)]
pub struct StillClock {
    now: OffsetDateTime,
    slept: Rc<RefCell<Vec<Duration>>>,
}

impl StillClock {
    pub fn at(now: OffsetDateTime) -> Self {
        StillClock {
            now,
            slept: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Every wait asked of [`Clock::sleep`] so far, oldest first.
    pub fn slept(&self) -> Vec<Duration> {
        self.slept.borrow().clone()
    }
}

impl Clock for StillClock {
    fn now(&self) -> OffsetDateTime {
        self.now
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        self.slept.borrow_mut().push(duration);
        Box::pin(std::future::ready(()))
    }
}

// ---- a server that answers by route -----------------------------------------------------------

/// One route of a [`FakeServer`]: given the path and query asked, an answer — a status and a JSON
/// body, or `None` for no body — or `None` to let the next route answer.
pub type Route = Rc<dyn Fn(&str) -> Option<(u16, Option<String>)>>;

#[derive(Default)]
struct ServerState {
    routes: RefCell<Vec<Route>>,
    asked: RefCell<Vec<String>>,
}

/// An HTTP transport that answers by ROUTE — the C# resync tests' `Server` handler: each request is
/// offered to the routes in the order they were added, and the first that answers wins; a request
/// no route answers never arrives (`Unreachable`, the C# handler's exception). A route is a
/// closure, so it may do something of its own as it answers — apply a live frame mid-pass, say, or
/// reopen the socket — which a script cannot. Every path asked is kept, in order. Clones share the
/// routes and the record.
#[derive(Clone, Default)]
pub struct FakeServer(Rc<ServerState>);

impl fmt::Debug for FakeServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeServer")
            .field("routes", &self.0.routes.borrow().len())
            .field("asked", &self.0.asked.borrow().len())
            .finish()
    }
}

impl FakeServer {
    pub fn new() -> Self {
        FakeServer::default()
    }

    /// A route of its own (the C#'s `On`).
    pub fn on(&self, route: impl Fn(&str) -> Option<(u16, Option<String>)> + 'static) -> &Self {
        self.0.routes.borrow_mut().push(Rc::new(route));
        self
    }

    /// A fixed `200` answer for the one endpoint `what` names — `"/me"` is `/api/v1/me` — matched
    /// against the WHOLE path, query aside (the C#'s `Always`). Anchored for the C#'s reason: a
    /// route that matched a substring answered `/chats/42/messages` with `/me`'s body.
    pub fn always(&self, what: &str, json: &str) -> &Self {
        self.always_with(what, json, 200)
    }

    /// [`always`](Self::always) with a status of its own.
    pub fn always_with(&self, what: &str, json: &str, status: u16) -> &Self {
        let endpoint = format!("/api/v1{what}");
        let json = json.to_string();
        self.on(move |path| {
            (path.split('?').next() == Some(endpoint.as_str()))
                .then(|| (status, Some(json.clone())))
        })
    }

    /// The path and query of every request so far, oldest first.
    pub fn asked(&self) -> Vec<String> {
        self.0.asked.borrow().clone()
    }

    /// Forget what was asked so far (`Asked.Clear()`).
    pub fn clear_asked(&self) {
        self.0.asked.borrow_mut().clear();
    }
}

impl Http for FakeServer {
    fn send(
        &self,
        request: HttpRequest,
    ) -> LocalBoxFuture<'_, Result<HttpResponse, TransportError>> {
        let path = path_of(&request.url).to_string();
        self.0.asked.borrow_mut().push(path.clone());
        // Copied out first, so a route may add routes (or ask again) while it answers.
        let routes: Vec<Route> = self.0.routes.borrow().clone();
        let answer = routes
            .iter()
            .find_map(|route| route(&path))
            .map(|(status, json)| json_response(status, json.as_deref().unwrap_or(""), &[]))
            .ok_or_else(|| TransportError::Unreachable(format!("no route for {path}")));
        Box::pin(async move { answer })
    }
}

// ---- a socket the send pipeline writes to -----------------------------------------------------

struct FrameSenderState {
    connected: Cell<bool>,
    writes_go_through: Cell<bool>,
    sent: RefCell<Vec<String>>,
    answers: RefCell<Option<Rc<dyn Fn(&str) -> Option<ServerFrame>>>>,
    frame: Event<ServerFrame>,
}

/// A socket whose answers the test decides, frame by frame — the C# send-pipeline tests' `Fake`
/// [`FrameSender`]. Connected and taking writes until told otherwise; an answer is raised on the
/// same turn as the write, before it returns — deterministic, and an ordering the wire can produce:
/// the pipeline registers its waiter before the frame goes. Clones share the socket.
#[derive(Clone)]
pub struct FakeFrameSender(Rc<FrameSenderState>);

impl Default for FakeFrameSender {
    fn default() -> Self {
        FakeFrameSender(Rc::new(FrameSenderState {
            connected: Cell::new(true),
            writes_go_through: Cell::new(true),
            sent: RefCell::new(Vec::new()),
            answers: RefCell::new(None),
            frame: Event::new(),
        }))
    }
}

impl fmt::Debug for FakeFrameSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeFrameSender")
            .field("connected", &self.0.connected.get())
            .field("sent", &self.0.sent.borrow().len())
            .finish_non_exhaustive()
    }
}

impl FakeFrameSender {
    pub fn new() -> Self {
        FakeFrameSender::default()
    }

    /// Whether a frame can be written (`IsConnected`).
    pub fn set_connected(&self, connected: bool) {
        self.0.connected.set(connected);
    }

    /// Whether a write goes (`WritesGoThrough`); one that does not answers `false`.
    pub fn set_writes_go_through(&self, go: bool) {
        self.0.writes_go_through.set(go);
    }

    /// What to answer each written frame with, if anything — `None` is silence (`Answers`).
    pub fn answer_with(&self, answers: impl Fn(&str) -> Option<ServerFrame> + 'static) {
        *self.0.answers.borrow_mut() = Some(Rc::new(answers));
    }

    /// A frame from the server, unasked (`Raise`).
    pub fn raise(&self, frame: &ServerFrame) {
        self.0.frame.emit(frame);
    }

    /// Every frame written, oldest first — those that did not go included.
    pub fn sent(&self) -> Vec<String> {
        self.0.sent.borrow().clone()
    }
}

impl FrameSender for FakeFrameSender {
    fn is_connected(&self) -> bool {
        self.0.connected.get()
    }

    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool> {
        self.0.sent.borrow_mut().push(frame.clone());
        if !self.0.writes_go_through.get() {
            return Box::pin(std::future::ready(false));
        }
        let answers = self.0.answers.borrow().clone();
        if let Some(answer) = answers.and_then(|answers| answers(&frame)) {
            self.0.frame.emit(&answer);
        }
        Box::pin(std::future::ready(true))
    }

    fn on_frame(&self) -> &Event<ServerFrame> {
        &self.0.frame
    }
}

// ---- driving a future -------------------------------------------------------------------------

/// A waker that remembers it was woken.
struct Flag(AtomicBool);

impl Wake for Flag {
    fn wake(self: Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Run `future` to its end on this thread, moving `clock` to the next sleep's deadline whenever
/// nothing else can move — so a test of a deadline or a backoff takes no real time at all, and
/// every wait it asked for is in [`FakeClock::slept`].
///
/// Panics when the future is STUCK: not finished, nothing woke it, and no sleep is waiting on the
/// clock — a test that would otherwise hang for ever.
pub fn drive<F: Future>(clock: &FakeClock, future: F) -> F::Output {
    let mut future = pin!(future);
    let flag = Arc::new(Flag(AtomicBool::new(false)));
    let waker = Waker::from(flag.clone());
    let mut cx = Context::from_waker(&waker);
    loop {
        flag.0.store(false, Ordering::SeqCst);
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
        if flag.0.load(Ordering::SeqCst) {
            continue;
        }
        assert!(
            clock.advance_to_next(),
            "the future is stuck: nothing woke it and no sleep is waiting on the clock"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::LocalPool;
    use futures::task::LocalSpawnExt;
    use std::cell::Cell;
    use time::macros::datetime;

    #[test]
    fn a_sleep_ends_only_when_the_clock_passes_it() {
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let mut pool = LocalPool::new();
        let woke = Rc::new(Cell::new(false));
        let (sleep, flag) = (clock.sleep(Duration::from_secs(5)), woke.clone());
        pool.spawner()
            .spawn_local(async move {
                sleep.await;
                flag.set(true)
            })
            .unwrap();
        pool.run_until_stalled();
        assert!(!woke.get());
        assert_eq!(clock.sleepers(), 1);
        clock.advance(Duration::from_secs(4));
        pool.run_until_stalled();
        assert!(!woke.get());
        clock.advance(Duration::from_secs(1));
        pool.run_until_stalled();
        assert!(woke.get());
        assert_eq!(clock.sleepers(), 0);
        assert_eq!(clock.now(), datetime!(2026-10-28 12:00:05 UTC));
    }

    #[test]
    fn the_fake_transport_answers_from_its_script_once_each_and_keeps_what_was_sent() {
        let http = FakeHttp::new();
        http.respond(Method::Get, "/chats", 200, r#"{"chats":[]}"#);
        let mut pool = LocalPool::new();
        let first = pool.run_until(http.send(HttpRequest::new(
            Method::Get,
            "https://fc.example/chats?family_id=3",
        )));
        assert_eq!(first.unwrap().body, br#"{"chats":[]}"#);
        let second =
            pool.run_until(http.send(HttpRequest::new(Method::Get, "https://fc.example/chats")));
        assert!(matches!(second, Err(TransportError::Unreachable(_))));
        assert_eq!(http.sent().len(), 2);
        assert_eq!(http.unused(), 0);
    }

    #[test]
    fn an_abandoned_sleep_is_not_waited_for() {
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let mut pool = LocalPool::new();
        let mut sleep = clock.sleep(Duration::from_secs(5));
        assert!(pool.run_until(futures::future::poll_fn(|cx| Poll::Ready(
            sleep.as_mut().poll(cx).is_pending()
        ))));
        assert_eq!(clock.sleepers(), 1);
        drop(sleep);
        assert_eq!(clock.sleepers(), 0);
        assert!(!clock.advance_to_next());
        assert_eq!(clock.slept(), [Duration::from_secs(5)]);
    }

    #[test]
    fn drive_moves_the_clock_to_each_deadline_in_turn() {
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let (first, second) = (clock.sleep(Duration::from_secs(3)), clock.clone());
        drive(&clock, async move {
            first.await;
            second.sleep(Duration::from_secs(2)).await;
        });
        assert_eq!(clock.now(), datetime!(2026-10-28 12:00:05 UTC));
    }

    #[test]
    #[should_panic(expected = "stuck")]
    fn drive_fails_a_future_nothing_can_finish() {
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        drive(&clock, std::future::pending::<()>());
    }

    #[test]
    fn a_delayed_answer_is_looked_up_only_when_its_wait_ends() {
        let clock = FakeClock::at(datetime!(2026-10-28 12:00 UTC));
        let http = FakeHttp::new();
        http.delay(&clock, Duration::from_secs(1))
            .serve("/chats", 200, r#"{"chats":[]}"#);
        let asking = http.send(HttpRequest::new(
            Method::Get,
            "https://fc.example/chats?x=1",
        ));
        // Scripted after the request went out, before its wait ended: the script still answers.
        http.respond(Method::Get, "/chats", 503, "");
        let answer = drive(&clock, asking).expect("an answer");
        assert_eq!(answer.status, 503);
        let served = drive(
            &clock,
            http.send(HttpRequest::new(Method::Post, "https://fc.example/chats")),
        );
        assert_eq!(served.expect("an answer").status, 200);
        assert_eq!(http.asked(), ["/chats?x=1", "/chats"]);
    }

    #[test]
    fn a_prefix_matches_only_up_to_the_query() {
        assert_eq!(
            path_of("https://fc.example:8443/chats/7/messages?before=9"),
            "/chats/7/messages?before=9"
        );
        assert_eq!(path_of("https://fc.example"), "/");
        let http = FakeHttp::new();
        http.respond(Method::Get, "/chats", 200, "");
        let mut pool = LocalPool::new();
        let longer =
            pool.run_until(http.send(HttpRequest::new(Method::Get, "https://fc.example/chats/7")));
        assert!(longer.is_err(), "/chats must not answer /chats/7");
    }
}

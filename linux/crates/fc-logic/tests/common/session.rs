//! What the session layer's tests share: a client over a token store the test also holds (the C#
//! tests' `new ApiClient(…, tokens)` beside `new AppSession(api, tokens, cache)`), a pool that runs
//! what the code under test spawns while a fake clock stands still unless everything waits on it,
//! and `LiveConnectionTests`' `Wire` — a socket that is told what to do rather than connecting to
//! anything.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use fc_core::protocol::{
    ApiClient, FrameSender, LiveSocket, MemoryTokenStore, ServerFrame, ServerUrl,
};
use fc_core::rt::{Cancel, Event, Http, LocalBoxFuture};
use fc_core::testing::FakeClock;
use fc_logic::calls::Spawn;

/// A client over `http` whose token store the test holds too.
pub fn api_with(
    http: Rc<dyn Http>,
    clock: &FakeClock,
    tokens: Rc<MemoryTokenStore>,
) -> Rc<ApiClient> {
    Rc::new(ApiClient::new(
        http,
        ServerUrl::normalise(Some("chat.example.com")).expect("a server"),
        tokens,
        Rc::new(clock.clone()),
    ))
}

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

/// A `LocalPool` for what the code under test spawns, and the fake clock it reads: the clock moves
/// to the next sleep's deadline only when NOTHING else can move — so a test of a retry's pause takes
/// no real time, and a request held open is never timed out behind the test's back.
pub struct Pool {
    pool: RefCell<LocalPool>,
    pub clock: FakeClock,
}

impl Pool {
    pub fn new(clock: &FakeClock) -> Pool {
        Pool {
            pool: RefCell::new(LocalPool::new()),
            clock: clock.clone(),
        }
    }

    /// What the code under test is given to spawn with.
    pub fn spawn(&self) -> Spawn {
        let spawner = self.pool.borrow().spawner();
        Rc::new(move |future| spawner.spawn_local(future).expect("the pool takes work"))
    }

    /// Everything set going, run until nothing more can move without the clock.
    pub fn settle(&self) {
        self.pool.borrow_mut().run_until_stalled();
    }

    /// Run until `done` says so, moving the clock only when nothing else can move.
    pub fn until(&self, done: impl Fn() -> bool) {
        loop {
            self.settle();
            if done() {
                return;
            }
            assert!(
                self.clock.advance_to_next(),
                "stuck: not done, nothing can move, and nothing sleeps on the clock"
            );
        }
    }

    /// `await future`, with everything it sets going run beside it.
    pub fn run<F: Future>(&self, future: F) -> F::Output {
        let mut future = pin!(future);
        let flag = Arc::new(Flag(AtomicBool::new(true)));
        let waker = Waker::from(flag.clone());
        let mut cx = Context::from_waker(&waker);
        loop {
            if flag.0.swap(false, Ordering::SeqCst)
                && let Poll::Ready(output) = future.as_mut().poll(&mut cx)
            {
                return output;
            }
            self.settle();
            if flag.0.load(Ordering::SeqCst) {
                continue;
            }
            assert!(
                self.clock.advance_to_next(),
                "stuck: the future waits, nothing can move, and nothing sleeps on the clock"
            );
        }
    }
}

#[derive(Default)]
struct WireState {
    runs: Cell<usize>,
    live: Cell<usize>,
    overlap: Cell<usize>,
    stopped: Cell<bool>,
    connected: Cell<bool>,
    sent: RefCell<Vec<String>>,
    frame: Event<ServerFrame>,
    connected_event: Event<()>,
    disconnected: Event<()>,
    session_expired: Event<()>,
}

/// A socket that is told what to do rather than connecting to anything (`LiveConnectionTests.Wire`).
/// Clones share the socket.
#[derive(Clone, Default)]
pub struct Wire(Rc<WireState>);

impl Wire {
    pub fn new() -> Wire {
        Wire::default()
    }

    /// How many times the loop was entered.
    pub fn runs(&self) -> usize {
        self.0.runs.get()
    }

    /// The most loops that were ever running at once.
    pub fn overlap(&self) -> usize {
        self.0.overlap.get()
    }

    /// How many loops are running now.
    pub fn live(&self) -> usize {
        self.0.live.get()
    }

    /// Whether a loop was ever told to stop.
    pub fn stopped(&self) -> bool {
        self.0.stopped.get()
    }

    /// Whether the loop is running (`Running` completed).
    pub fn running(&self) -> bool {
        self.0.runs.get() > 0
    }

    pub fn sent(&self) -> Vec<String> {
        self.0.sent.borrow().clone()
    }

    /// A connection came up.
    pub fn comes(&self) {
        self.0.connected.set(true);
        self.0.connected_event.emit(&());
    }

    /// It went away.
    pub fn goes(&self) {
        self.0.connected.set(false);
        self.0.disconnected.emit(&());
    }

    /// The socket says the session is gone.
    pub fn expires(&self) {
        self.0.session_expired.emit(&());
    }

    /// A frame arrived.
    pub fn hear(&self, frame: &ServerFrame) {
        self.0.frame.emit(frame);
    }
}

impl FrameSender for Wire {
    fn is_connected(&self) -> bool {
        self.0.connected.get()
    }

    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool> {
        self.0.sent.borrow_mut().push(frame);
        let connected = self.0.connected.get();
        Box::pin(std::future::ready(connected))
    }

    fn on_frame(&self) -> &Event<ServerFrame> {
        &self.0.frame
    }
}

impl LiveSocket for Wire {
    fn on_connected(&self) -> &Event<()> {
        &self.0.connected_event
    }

    fn on_disconnected(&self) -> &Event<()> {
        &self.0.disconnected
    }

    fn on_session_expired(&self) -> &Event<()> {
        &self.0.session_expired
    }

    fn run(&self, cancel: Cancel) -> LocalBoxFuture<'_, ()> {
        let state = Rc::clone(&self.0);
        Box::pin(async move {
            state.runs.set(state.runs.get() + 1);
            state.live.set(state.live.get() + 1);
            state.overlap.set(state.overlap.get().max(state.live.get()));
            cancel.cancelled().await;
            state.stopped.set(true);
            state.live.set(state.live.get() - 1);
        })
    }
}

//! The live client: the socket, the reconnect resync, the outbox and the frame router under one
//! policy — which is the only thing here that is a decision rather than a wire (`LiveConnection.cs`).
//!
//! THE SOCKET RUNS ONLY WHILE THERE IS SOMETHING TO LISTEN TO. Signed in AND in a family: an
//! account at the gate has no chats, and a loop that reconnected on its behalf would be a client
//! hammering a door with no room behind it. The gate moving is what starts and stops it — a
//! sign-in, a sign-out, a join approved, a removal.
//!
//! A CONNECTION MEANS A RESYNC, AND RESYNCS DO NOT STACK. Every connection starts one, not just the
//! first: what a client missed while the socket was down is exactly what the pass reads. But a
//! flapping proxy can raise three connections in a second, and three overlapping passes would page
//! the same chats three times and fight over the cursors — so a pass in flight COALESCES the
//! connections that arrive during it into one more pass, and no more than one.
//!
//! AN EXPIRED SESSION IS NOT A NETWORK ERROR. The socket closing with `4401` — or refusing its
//! upgrade with `401`, which `win/` read as a network failure and retried for ever (carry-forward,
//! "Found in the other clients" 10) — lands exactly where a REST `401` lands, and it lands once:
//! otherwise a device whose account was deleted sits behind a "Connecting…" banner looking signed in
//! until something makes a REST call.
//!
//! THE OUTBOX HAS THREE TRIGGERS AND THE SOCKET IS ONLY ONE OF THEM. A returning network and a
//! window coming forward flush it too, because waiting for the next reconnect to come round on its
//! own is what makes a message sit unsent under a full signal bar.
//!
//! **How the app builds it** (`linux/docs/carry-forward.md`, "For fc-logic's LiveConnection and the
//! app's composition") — on the GLib main context, once per signed-in server:
//!
//! ```text
//! let socket = Rc::new(ChatSocket::new(connector, move || api.socket_url(), tokens, clock));
//! let parts = LiveParts::compose(socket, api, chats, board, Some(pack), outbox, clock,
//!                                Some(media_outbox.pump()));    // pipeline FIRST, then router, resync
//! let live = LiveConnection::new(session.clone(), parts, spawn); // spawn = glib spawn_local
//! live.on_cache_failed().subscribe(|error| log ids and counts only);
//! live.start();                                                  // and on every window appearance
//! spawn(live.flush(FlushTrigger::…)) for connectivity, activation, the timer, a retry, a new row
//! ```
//!
//! What it spawns: while the gate lets it run, ONE future that runs the socket's loop and the
//! passes side by side (the C#'s `RunAsync` and `PassesAsync`), ended by an `rt::Cancel` when the
//! gate closes — [`stop`](LiveConnection::stop) waits for it to wind down, so a sign-out cannot
//! leave a pass writing into a cache the next account is about to use; and, for a `member_joined` or
//! `member_left` naming the reader, the session's re-read. Every failure of the cache on the way —
//! a frame the router could not write, a send the pipeline could not record, a snapshot, a pass — is
//! raised on [`on_cache_failed`](LiveConnection::on_cache_failed) for the shell to log; none of them
//! stops the socket (carry-forward, "Found in the other clients" 16).

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::{Pin, pin};
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use futures_util::future::{Either, join, select};

use fc_core::protocol::resync::Report;
use fc_core::protocol::send_pipeline::Uploads;
use fc_core::protocol::send_rules::FlushTrigger;
use fc_core::protocol::{
    ApiClient, ApiError, ChatSocket, FrameRouter, FrameSender, LiveSocket, Resync, SendPipeline,
};
use fc_core::rt::{Cancel, Clock, Event, Subscription};
use fc_core::store::{BoardStore, ChatStore, OutboxStore, PackStore, StoreError, StoreResult};

use crate::app_session::{AppSession, SessionState};
use crate::calls::Spawn;

/// What the window draws in the corner, and nothing more subtle than this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Link {
    /// No socket, and not trying: signed out, or at the family gate.
    Down,
    /// Trying. The loop is running and the connection is not up.
    Connecting,
    /// Up, and the pass that follows a connection has been asked for.
    Up,
}

/// The core's pieces, built in the order they must be built (carry-forward): the send pipeline
/// FIRST — it subscribes to the socket's frames itself, as the C# did in its constructor — then the
/// router, which [`LiveConnection::new`] subscribes after it, then the resync over the pipeline and
/// the pack.
pub struct LiveParts {
    pub socket: Rc<dyn LiveSocket>,
    pub sending: Rc<SendPipeline>,
    pub router: Rc<FrameRouter>,
    pub resync: Rc<Resync>,
}

impl std::fmt::Debug for LiveParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveParts").finish_non_exhaustive()
    }
}

impl LiveParts {
    /// The app's composition over a real socket (`Connection.cs`): `SendPipeline::from_api(socket,
    /// outbox, chats, api, clock).with_uploads(media pump)`, `FrameRouter::new(chats, board, pack)`,
    /// `Resync::new(api, chats, board).with_sending(pipeline).with_pack(pack)`.
    pub fn compose(
        socket: Rc<ChatSocket>,
        api: Rc<ApiClient>,
        chats: Rc<ChatStore>,
        board: Rc<BoardStore>,
        pack: Option<Rc<PackStore>>,
        outbox: Rc<OutboxStore>,
        clock: Rc<dyn Clock>,
        uploads: Option<Uploads>,
    ) -> LiveParts {
        let sender: Rc<dyn FrameSender> = socket.clone();
        let mut pipeline =
            SendPipeline::from_api(sender, outbox, Rc::clone(&chats), Rc::clone(&api), clock);
        if let Some(uploads) = uploads {
            pipeline = pipeline.with_uploads(uploads);
        }
        let sending = Rc::new(pipeline);
        let router = Rc::new(FrameRouter::new(
            Rc::clone(&chats),
            Rc::clone(&board),
            pack.clone(),
        ));
        let mut resync = Resync::new(api, chats, board).with_sending(Rc::clone(&sending));
        if let Some(pack) = pack {
            resync = resync.with_pack(pack);
        }
        LiveParts {
            socket,
            sending,
            router,
            resync: Rc::new(resync),
        }
    }
}

/// The live client (`LiveConnection`). See the module's comment.
pub struct LiveConnection {
    inner: Rc<Inner>,
    subscriptions: RefCell<Vec<Unsubscribe>>,
}

/// One event subscription, to be undone when the connection is disposed of.
type Unsubscribe = Box<dyn Fn()>;

struct Inner {
    session: Rc<AppSession>,
    socket: Rc<dyn LiveSocket>,
    resync: Rc<Resync>,
    sending: Rc<SendPipeline>,
    spawn: Spawn,
    /// Raised once per connection, and never more than once at a time: the coalescing rule IS this
    /// flag being one bit (the C#'s semaphore of capacity one).
    due: Signal,
    /// STARTING AND STOPPING ARE SERIALISED, and it took a review round to notice why. A stop used
    /// to clear its fields and only THEN wait for the socket loop to wind down, so a start arriving
    /// in that window found nothing running and put a SECOND loop on the same socket — one object
    /// with one live connection field, and the two would have fought over it. A gate movement away
    /// and back (signed out, signed in; removed, re-approved) is exactly that window.
    lifecycle: Turnstile,
    life: RefCell<Option<Life>>,
    link: Cell<Link>,
    link_changed: Event<Link>,
    resynced: Event<Report>,
    resync_failed: Event<ApiError>,
    cache_failed: Event<StoreError>,
}

/// What runs while the gate lets it: the flag that ends it, and the sign that it has.
#[derive(Clone)]
struct Life {
    cancel: Cancel,
    wound_down: Cancel,
}

impl std::fmt::Debug for LiveConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveConnection")
            .field("link", &self.inner.link.get())
            .field("running", &self.inner.life.borrow().is_some())
            .finish_non_exhaustive()
    }
}

impl LiveConnection {
    /// `new LiveConnection(session, socket, resync, sending, router)`: listens to the socket, the
    /// session and the router from here on — the router to the socket's frames (after the pipeline,
    /// which subscribed itself when it was made). `spawn` starts the work the C# started with
    /// `Task.Run`: `glib::MainContext::spawn_local` in the app, a `LocalPool`'s spawner in a test.
    pub fn new(session: Rc<AppSession>, parts: LiveParts, spawn: Spawn) -> LiveConnection {
        let LiveParts {
            socket,
            sending,
            router,
            resync,
        } = parts;
        let inner = Rc::new(Inner {
            session: Rc::clone(&session),
            socket: Rc::clone(&socket),
            resync,
            sending: Rc::clone(&sending),
            spawn,
            due: Signal::default(),
            lifecycle: Turnstile::default(),
            life: RefCell::new(None),
            link: Cell::new(Link::Down),
            link_changed: Event::new(),
            resynced: Event::new(),
            resync_failed: Event::new(),
            cache_failed: Event::new(),
        });
        let mut undo: Vec<Unsubscribe> = Vec::new();

        // Every frame reaches the cache through the router; a frame it could not write is logged,
        // and the socket goes on (`GET /chats` and the catch-up feeds reconcile it).
        let weak = Rc::downgrade(&inner);
        let heard_by = Rc::clone(&router);
        let id = socket.on_frame().subscribe(move |frame| {
            if let Err(error) = heard_by.hear(frame)
                && let Some(inner) = weak.upgrade()
            {
                inner.cache_failed.emit(&error);
            }
        });
        undo.push(unsubscribe_from(&socket, id, |socket| socket.on_frame()));

        let weak = Rc::downgrade(&inner);
        let id = socket.on_connected().subscribe(move |()| {
            if let Some(inner) = weak.upgrade() {
                inner.on_connected();
            }
        });
        undo.push(unsubscribe_from(&socket, id, |socket| {
            socket.on_connected()
        }));

        let weak = Rc::downgrade(&inner);
        let id = socket.on_disconnected().subscribe(move |()| {
            if let Some(inner) = weak.upgrade() {
                inner.publish(Link::Connecting);
            }
        });
        undo.push(unsubscribe_from(&socket, id, |socket| {
            socket.on_disconnected()
        }));

        let weak = Rc::downgrade(&inner);
        let id = socket.on_session_expired().subscribe(move |()| {
            if let Some(inner) = weak.upgrade()
                && let Err(error) = inner.session.session_gone()
            {
                inner.cache_failed.emit(&error);
            }
        });
        undo.push(unsubscribe_from(&socket, id, |socket| {
            socket.on_session_expired()
        }));

        let weak = Rc::downgrade(&inner);
        let id = session.on_changed().subscribe(move |state| {
            if let Some(inner) = weak.upgrade() {
                Inner::gate_moved(&inner, state);
            }
        });
        let held = Rc::clone(&session);
        undo.push(Box::new(move || {
            held.on_changed().unsubscribe(id);
        }));

        // The reader is in a family (docs/protocol.md, "Several families"): one this session does
        // not hold is listed by a fresh `GET /me`, and its roster, wall, pack and chats are read by
        // a pass. Spawned — a frame handler never waits on a request.
        let weak = Rc::downgrade(&inner);
        let id = router.on_joined_family().subscribe(move |family_id| {
            if let Some(inner) = weak.upgrade() {
                let family_id = *family_id;
                let task = Rc::clone(&inner);
                (inner.spawn)(Box::pin(async move {
                    match task.session.joined_family(family_id).await {
                        Ok(true) => task.refresh(),
                        Ok(false) => {}
                        // The next pass's `GET /me` lists it anyway.
                        Err(error) => task.cache_failed.emit(&error),
                    }
                }));
            }
        });
        undo.push(unsubscribe_from(&router, id, |router| {
            router.on_joined_family()
        }));

        // The reader is out of a family: its rows go, and the session shows what is left.
        let weak = Rc::downgrade(&inner);
        let id = router.on_left_family().subscribe(move |family_id| {
            if let Some(inner) = weak.upgrade() {
                let family_id = *family_id;
                let task = Rc::clone(&inner);
                (inner.spawn)(Box::pin(async move {
                    // The next pass's `GET /me` drops it anyway.
                    if let Err(error) = task.session.family_gone(family_id).await {
                        task.cache_failed.emit(&error);
                    }
                }));
            }
        });
        undo.push(unsubscribe_from(&router, id, |router| {
            router.on_left_family()
        }));

        let to = Rc::clone(&session);
        let id = router
            .on_invitation_changed()
            .subscribe(move |invitation| to.apply_invitation(invitation));
        undo.push(unsubscribe_from(&router, id, |router| {
            router.on_invitation_changed()
        }));

        let to = Rc::clone(&session);
        let id = router
            .on_invitation_removed()
            .subscribe(move |invitation_id| to.invitation_gone(*invitation_id));
        undo.push(unsubscribe_from(&router, id, |router| {
            router.on_invitation_removed()
        }));

        // A send answered by a frame the cache could not record: nothing is lost (the row stays
        // queued, and the next flush sends it again under the same key) — logged with the rest.
        let weak = Rc::downgrade(&inner);
        let id = sending.on_cache_failed().subscribe(move |error| {
            if let Some(inner) = weak.upgrade() {
                inner.cache_failed.emit(error);
            }
        });
        undo.push(unsubscribe_from(&sending, id, |sending| {
            sending.on_cache_failed()
        }));

        LiveConnection {
            inner,
            subscriptions: RefCell::new(undo),
        }
    }

    /// What the corner of the window says.
    pub fn link(&self) -> Link {
        self.inner.link.get()
    }

    /// The link changed (`LinkChanged`).
    pub fn on_link_changed(&self) -> &Event<Link> {
        &self.inner.link_changed
    }

    /// A reconnect pass finished, and what it managed (`Resynced`).
    pub fn on_resynced(&self) -> &Event<Report> {
        &self.inner.resynced
    }

    /// A pass could not finish (`ResyncFailed`). Transient means it will be tried again on the next
    /// one.
    pub fn on_resync_failed(&self) -> &Event<ApiError> {
        &self.inner.resync_failed
    }

    /// The cache refused a write somewhere under this connection — a frame, a send's answer, a
    /// snapshot, a pass, the session's re-read. For the shell's log (ids and counts only); nothing
    /// stops over it.
    pub fn on_cache_failed(&self) -> &Event<StoreError> {
        &self.inner.cache_failed
    }

    /// Start listening, if the session says there is anything to listen to. Idempotent: the window
    /// may call it on every appearance. Fire and forget — [`start_now`](Self::start_now) is the
    /// same, awaitable.
    pub fn start(&self) {
        Inner::start(&self.inner);
    }

    /// The same start, awaitable (`StartAsync`) — which is what a test wants and what the
    /// fire-and-forget [`start`](Self::start) is.
    pub async fn start_now(&self) {
        Inner::start_now(&self.inner).await;
    }

    /// Stop, and wait for the socket's loop and the passes to finish — so that a sign-out cannot
    /// leave a pass writing into a cache the next account is about to use.
    pub async fn stop(&self) {
        self.inner.stop().await;
    }

    /// The network came back, or the window came forward. Flushes the outbox without waiting for a
    /// reconnect to come round on its own. Answers how many rows went.
    pub async fn flush(&self, trigger: FlushTrigger) -> StoreResult<usize> {
        self.inner.sending.flush(trigger).await
    }

    /// A pass, on demand — what a pull-to-refresh is. Two at once are still one.
    pub fn refresh(&self) {
        self.inner.refresh();
    }

    /// The network came back: the socket dials now instead of finishing its backoff (the Mac's
    /// `networkRestored`). The outbox's own flush is [`flush`](Self::flush).
    pub fn reconnect_now(&self) {
        self.inner.socket.kick();
    }

    /// Stop listening to the socket, the session and the router, and stop (`DisposeAsync`).
    pub async fn dispose(&self) {
        self.unsubscribe();
        self.inner.stop().await;
    }

    fn unsubscribe(&self) {
        for undo in self.subscriptions.borrow_mut().drain(..) {
            undo();
        }
    }
}

impl Drop for LiveConnection {
    /// Dropped without [`dispose`](Self::dispose): nothing is listened to any more, and whatever runs
    /// is told to end — it cannot be waited for here.
    fn drop(&mut self) {
        self.unsubscribe();
        if let Some(life) = self.inner.life.borrow().as_ref() {
            life.cancel.cancel();
        }
    }
}

/// An undo for a subscription on an event that `owner` holds.
fn unsubscribe_from<T: ?Sized + 'static, A: ?Sized + 'static>(
    owner: &Rc<T>,
    subscription: Subscription,
    event: fn(&T) -> &Event<A>,
) -> Unsubscribe {
    let owner = Rc::clone(owner);
    Box::new(move || {
        event(&owner).unsubscribe(subscription);
    })
}

impl Inner {
    fn start(inner: &Rc<Inner>) {
        let task = Rc::clone(inner);
        (inner.spawn)(Box::pin(async move { Inner::start_now(&task).await }));
    }

    async fn start_now(inner: &Rc<Inner>) {
        {
            let _turn = inner.lifecycle.enter().await;
            if !inner.session.state().can_chat() || inner.life.borrow().is_some() {
                return;
            }
            let life = Life {
                cancel: Cancel::new(),
                wound_down: Cancel::new(),
            };
            *inner.life.borrow_mut() = Some(life.clone());
            let task = Rc::clone(inner);
            (inner.spawn)(Box::pin(async move {
                let socket = Rc::clone(&task.socket);
                join(socket.run(life.cancel.clone()), task.passes(&life.cancel)).await;
                life.wound_down.cancel();
            }));
        }
        // Outside the turnstile: a subscriber is somebody else's code, and holding the gate across
        // it is how a window handler ends up waiting on a socket.
        inner.publish(Link::Connecting);
    }

    async fn stop(&self) {
        {
            let _turn = self.lifecycle.enter().await;
            let Some(life) = self.life.borrow().clone() else {
                return;
            };
            life.cancel.cancel();
            life.wound_down.cancelled().await;
            // Cleared only once the loop has actually WOUND DOWN, and inside the turnstile, so a
            // start that follows cannot find a socket that looks free while it is still running.
            *self.life.borrow_mut() = None;
        }
        self.publish(Link::Down);
    }

    fn refresh(&self) {
        // One already due is the whole point.
        self.due.raise();
    }

    fn on_connected(&self) {
        // Raised before the socket reads a single frame on this connection: the cursors taken here
        // are the ones the pass must start from, before a live message can move them past what was
        // missed.
        if let Err(error) = self.resync.snapshot() {
            self.cache_failed.emit(&error);
        }
        self.publish(Link::Up);
        self.refresh();
    }

    fn gate_moved(inner: &Rc<Inner>, state: &SessionState) {
        if state.can_chat() {
            Inner::start(inner);
            return;
        }
        // Signed out, removed, or waiting at the gate: nothing to listen to. Spawned — the gate
        // moved inside somebody else's handler, and a window event may not wait on a socket.
        let task = Rc::clone(inner);
        (inner.spawn)(Box::pin(async move { task.stop().await }));
    }

    /// The passes, one at a time, for as long as the socket runs (`PassesAsync`).
    async fn passes(&self, cancel: &Cancel) {
        while !cancel.is_cancelled() {
            match select(pin!(self.due.wait()), pin!(cancel.cancelled())).await {
                Either::Left(((), _)) => {}
                Either::Right(((), _)) => return,
            }
            // Cancelling a pass is dropping it: the cursors it started from go to the next one.
            let report = match select(pin!(self.resync.run()), pin!(cancel.cancelled())).await {
                Either::Left((report, _)) => report,
                Either::Right(((), _)) => return,
            };
            match report {
                Ok(report) => self.apply(&report),
                // A pass the cache refused is a pass that failed. The next connection starts
                // another, and a client that stopped reading because one page went wrong would be
                // a client that never recovers.
                Err(error) => self.cache_failed.emit(&error),
            }
        }
    }

    /// What a pass found, given to the session.
    fn apply(&self, report: &Report) {
        if let Some(error) = &report.stopped {
            self.resync_failed.emit(error);
        }
        if let Some(me) = &report.me {
            // Which families there are, as step 1 found them: one that appeared or went while the
            // socket was down is listed, or dropped with its rows, here.
            if let Err(error) = self.session.apply(me, None) {
                self.cache_failed.emit(&error);
            }
        }
        let shown = report.shown_family.is_none()
            || report.shown_family == self.session.state().current_family_id;
        if report.has_family && shown {
            // The one read that names the assistant is this pass's. Without this the session had
            // none until somebody opened the Family screen, which is where the only other read of
            // that document lives — and with no assistant there is no consent card in Settings and
            // no consent line over the composer.
            self.session.apply_assistant(report.assistant.clone());
        }
        if report.family_read && shown {
            // The video message's limits — or their absence, an older server — from the same read.
            self.session.apply_round_video(report.round_video);
        }
        self.resynced.emit(report);
    }

    fn publish(&self, next: Link) {
        if self.link.replace(next) == next {
            return;
        }
        self.link_changed.emit(&next);
    }
}

/// A one-bit signal: raised any number of times, taken once (the C#'s `SemaphoreSlim(0, 1)`).
#[derive(Default)]
struct Signal {
    raised: Cell<bool>,
    waiting: RefCell<Option<Waker>>,
}

impl Signal {
    fn raise(&self) {
        self.raised.set(true);
        if let Some(waker) = self.waiting.borrow_mut().take() {
            waker.wake();
        }
    }

    /// Ends once the signal is raised, and lowers it.
    fn wait(&self) -> impl Future<Output = ()> + '_ {
        std::future::poll_fn(move |cx| {
            if self.raised.replace(false) {
                return Poll::Ready(());
            }
            *self.waiting.borrow_mut() = Some(cx.waker().clone());
            Poll::Pending
        })
    }
}

/// One at a time, in the order they came (the C#'s `SemaphoreSlim(1, 1)` around starting and
/// stopping), on one thread.
#[derive(Default)]
struct Turnstile {
    taken: Cell<bool>,
    queue: RefCell<VecDeque<Rc<Cell<Option<Waker>>>>>,
}

impl Turnstile {
    fn enter(&self) -> Entering<'_> {
        Entering {
            turnstile: self,
            place: None,
        }
    }

    fn leave(&self) {
        self.taken.set(false);
        if let Some(next) = self.queue.borrow().front()
            && let Some(waker) = next.take()
        {
            waker.wake();
        }
    }
}

/// Waiting for a [`Turnstile`]: in the queue until it is this one's turn.
struct Entering<'a> {
    turnstile: &'a Turnstile,
    place: Option<Rc<Cell<Option<Waker>>>>,
}

impl<'a> Future for Entering<'a> {
    type Output = Turn<'a>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Turn<'a>> {
        let turnstile = self.turnstile;
        let first = match &self.place {
            None => turnstile.queue.borrow().is_empty(),
            Some(place) => turnstile
                .queue
                .borrow()
                .front()
                .is_some_and(|front| Rc::ptr_eq(front, place)),
        };
        if first && !turnstile.taken.get() {
            if let Some(place) = self.place.take() {
                turnstile.queue.borrow_mut().pop_front();
                drop(place);
            }
            turnstile.taken.set(true);
            return Poll::Ready(Turn { turnstile });
        }
        let place = self
            .place
            .get_or_insert_with(|| {
                let place = Rc::new(Cell::new(None));
                turnstile.queue.borrow_mut().push_back(Rc::clone(&place));
                place
            })
            .clone();
        place.set(Some(cx.waker().clone()));
        Poll::Pending
    }
}

impl Drop for Entering<'_> {
    /// A waiter dropped before its turn leaves the queue, and wakes the next if it was first.
    fn drop(&mut self) {
        if let Some(place) = self.place.take() {
            let mut queue = self.turnstile.queue.borrow_mut();
            let was_first = queue.front().is_some_and(|front| Rc::ptr_eq(front, &place));
            queue.retain(|held| !Rc::ptr_eq(held, &place));
            let next = if was_first && !self.turnstile.taken.get() {
                queue.front().and_then(|front| front.take())
            } else {
                None
            };
            drop(queue);
            if let Some(waker) = next {
                waker.wake();
            }
        }
    }
}

/// A turn at a [`Turnstile`], given back when dropped.
struct Turn<'a> {
    turnstile: &'a Turnstile,
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.turnstile.leave();
    }
}

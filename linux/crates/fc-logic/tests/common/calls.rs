//! The call engine's rig — `CallEngineTests`' `Wire`, `Media`, `Clock` and `Build`, in Rust.
//!
//! THE C# RAN EVERY CONTINUATION INLINE: its fakes answered with completed tasks and its clock's
//! delays completed synchronously when the test passed them, so a statement after `Hear`, `Pass` or
//! `Gather` saw everything that followed from it. Here the engine spawns that work, so every rig
//! action runs the pool until nothing more can move ([`Rig::settle`]) before it returns — the same
//! moment the C# test looked.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use serde_json::Value;
use time::OffsetDateTime;
use time::macros::datetime;

use fc_core::protocol::{
    ApiError, ApiResult, FrameSender, IceCandidate, IceServerDto, IceServersResponse, ServerFrame,
};
use fc_core::rt::{Clock, Event, LocalBoxFuture};
use fc_core::text::string_catalog::EnglishCatalog;
use fc_logic::calls::{CallEngine, CallMedia, MediaGrant, RingTone};

/// The socket: takes every frame, unless told to refuse the next few (`Wire`). Only the frames
/// that went are kept.
#[derive(Default)]
pub struct Wire {
    /// How many sends to refuse before the socket takes frames again.
    pub refusals: Cell<u32>,
    pub sent: RefCell<Vec<String>>,
    frame: Event<ServerFrame>,
}

impl FrameSender for Wire {
    fn is_connected(&self) -> bool {
        true
    }

    fn try_send(&self, frame: String) -> LocalBoxFuture<'_, bool> {
        if self.refusals.get() > 0 {
            self.refusals.set(self.refusals.get() - 1);
            return Box::pin(std::future::ready(false));
        }
        self.sent.borrow_mut().push(frame);
        Box::pin(std::future::ready(true))
    }

    fn on_frame(&self) -> &Event<ServerFrame> {
        &self.frame
    }
}

/// The media (`Media`): answers at once, as told, and writes down what it was asked.
pub struct Media {
    pub grant: Cell<MediaGrant>,
    /// Holds the microphone question open, as a person reading the prompt does.
    asking: RefCell<Option<oneshot::Receiver<MediaGrant>>>,
    pub connects: Cell<bool>,
    pub offer: RefCell<Option<String>>,
    pub answer: RefCell<Option<String>>,
    pub accepts_remote: Cell<bool>,
    /// A candidate found while the offer or answer is being described — where a WebRTC stack
    /// starts finding them (`WhileDescribing = () => Gather(…)`).
    pub while_describing: RefCell<Option<String>>,
    /// What `on-ice-candidate` gives here: the halves of a candidate the fake finds
    /// (`Gather`'s `"0"` and `0`, unless a test says otherwise).
    pub gathered_halves: RefCell<(Option<String>, Option<i32>)>,
    pub connected: Cell<bool>,
    /// Reports its own `close` as a failure, every time — a media breaking the contract.
    pub fails_on_close: Cell<bool>,
    pub log: RefCell<Vec<String>>,
    pub added: RefCell<Vec<IceCandidate>>,
    candidate_gathered: Event<IceCandidate>,
    came_up: Event<()>,
    failed: Event<()>,
    media_changed: Event<()>,
}

impl Default for Media {
    fn default() -> Self {
        Media {
            grant: Cell::new(MediaGrant::Microphone),
            asking: RefCell::new(None),
            connects: Cell::new(true),
            offer: RefCell::new(Some("v=offer".to_string())),
            answer: RefCell::new(Some("v=answer".to_string())),
            accepts_remote: Cell::new(true),
            while_describing: RefCell::new(None),
            gathered_halves: RefCell::new((Some("0".to_string()), Some(0))),
            connected: Cell::new(false),
            fails_on_close: Cell::new(false),
            log: RefCell::new(Vec::new()),
            added: RefCell::new(Vec::new()),
            candidate_gathered: Event::new(),
            came_up: Event::new(),
            failed: Event::new(),
            media_changed: Event::new(),
        }
    }
}

impl Media {
    /// `Asking = new TaskCompletionSource<MediaGrant>()`: the next `open` waits for the sender.
    pub fn ask(&self) -> oneshot::Sender<MediaGrant> {
        let (answer, question) = oneshot::channel();
        *self.asking.borrow_mut() = Some(question);
        answer
    }

    fn say(&self, line: String) {
        self.log.borrow_mut().push(line);
    }

    pub fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    pub fn added(&self) -> Vec<IceCandidate> {
        self.added.borrow().clone()
    }

    fn describing(&self) {
        let found = self.while_describing.borrow().clone();
        if let Some(candidate) = found {
            self.gather(&candidate);
        }
    }

    /// `Gather`: this device found a candidate.
    pub fn gather(&self, candidate: &str) {
        let (mid, index) = self.gathered_halves.borrow().clone();
        self.candidate_gathered
            .emit(&IceCandidate::new(candidate, mid.as_deref(), index));
    }

    /// `Up`: the media came up.
    pub fn up(&self) {
        self.connected.set(true);
        self.came_up.emit(&());
    }

    /// `Die`: the media failed.
    pub fn die(&self) {
        self.failed.emit(&());
    }

    /// `Tracks`: a stream appeared.
    pub fn tracks(&self) {
        self.media_changed.emit(&());
    }
}

impl CallMedia for Media {
    fn open(&self, video: bool) -> LocalBoxFuture<'static, MediaGrant> {
        self.say(if video { "open video" } else { "open" }.to_string());
        match self.asking.borrow_mut().take() {
            Some(question) => Box::pin(async move { question.await.unwrap_or(MediaGrant::None) }),
            None => Box::pin(std::future::ready(self.grant.get())),
        }
    }

    fn connect(
        &self,
        servers: Vec<IceServerDto>,
        receive_video_only: bool,
    ) -> LocalBoxFuture<'static, bool> {
        let only = if receive_video_only {
            " receive-only"
        } else {
            ""
        };
        self.say(format!("connect {}{only}", servers.len()));
        Box::pin(std::future::ready(self.connects.get()))
    }

    fn create_offer(&self) -> LocalBoxFuture<'static, Option<String>> {
        self.say("offer".to_string());
        self.describing();
        Box::pin(std::future::ready(self.offer.borrow().clone()))
    }

    fn set_remote(&self, is_offer: bool, sdp: String) -> LocalBoxFuture<'static, bool> {
        let kind = if is_offer { "offer" } else { "answer" };
        self.say(format!("remote {kind} {sdp}"));
        Box::pin(std::future::ready(self.accepts_remote.get()))
    }

    fn create_answer(&self) -> LocalBoxFuture<'static, Option<String>> {
        self.say("answer".to_string());
        self.describing();
        Box::pin(std::future::ready(self.answer.borrow().clone()))
    }

    fn add_candidate(&self, candidate: IceCandidate) -> LocalBoxFuture<'static, ()> {
        self.added.borrow_mut().push(candidate);
        Box::pin(std::future::ready(()))
    }

    fn is_connected(&self) -> bool {
        self.connected.get()
    }

    fn set_muted(&self, muted: bool) {
        self.say(format!("muted {muted}"));
    }

    fn set_camera(&self, on: bool) {
        self.say(format!("camera {on}"));
    }

    fn ring(&self, tone: RingTone) {
        self.say(format!("ring {tone:?}"));
    }

    fn stop_ringing(&self) {
        self.say("quiet".to_string());
    }

    fn close(&self) {
        self.say("close".to_string());
        if self.fails_on_close.get() {
            self.failed.emit(&());
        }
    }

    fn on_candidate_gathered(&self) -> &Event<IceCandidate> {
        &self.candidate_gathered
    }

    fn on_connected(&self) -> &Event<()> {
        &self.came_up
    }

    fn on_failed(&self) -> &Event<()> {
        &self.failed
    }

    fn on_media_changed(&self) -> &Event<()> {
        &self.media_changed
    }
}

struct Waiter {
    id: u64,
    after: Duration,
    done: bool,
    waker: Option<Waker>,
}

struct ClockState {
    now: OffsetDateTime,
    waiting: Vec<Waiter>,
    next: u64,
}

/// A clock a test moves (`Clock`): every wait lasts until the test says that much time has passed
/// — [`pass`](Self::pass) ends EVERY wait of exactly that length and no other, and the time of day
/// moves only when set. A wait the engine gave up on (its guard cancelled) is no longer waiting.
#[derive(Clone)]
pub struct CallClock(Rc<RefCell<ClockState>>);

impl CallClock {
    pub fn new() -> Self {
        CallClock(Rc::new(RefCell::new(ClockState {
            now: datetime!(2026-09-14 12:00 UTC),
            waiting: Vec::new(),
            next: 0,
        })))
    }

    /// `Clock.Now += by`.
    pub fn forward(&self, by: Duration) {
        self.0.borrow_mut().now += by;
    }

    /// `Waiting(after)`: how many waits of exactly that length have not ended.
    pub fn waiting(&self, after: Duration) -> usize {
        self.0
            .borrow()
            .waiting
            .iter()
            .filter(|waiter| waiter.after == after && !waiter.done)
            .count()
    }

    /// `Pass(after)`: every wait of exactly that length ends.
    pub fn pass(&self, after: Duration) {
        let woken: Vec<Waker> = {
            let mut state = self.0.borrow_mut();
            state
                .waiting
                .iter_mut()
                .filter(|waiter| waiter.after == after && !waiter.done)
                .filter_map(|waiter| {
                    waiter.done = true;
                    waiter.waker.take()
                })
                .collect()
        };
        for waker in woken {
            waker.wake();
        }
    }
}

impl Default for CallClock {
    fn default() -> Self {
        CallClock::new()
    }
}

impl Clock for CallClock {
    fn now(&self) -> OffsetDateTime {
        self.0.borrow().now
    }

    fn sleep(&self, duration: Duration) -> LocalBoxFuture<'static, ()> {
        let id = {
            let mut state = self.0.borrow_mut();
            state.next += 1;
            let id = state.next;
            state.waiting.push(Waiter {
                id,
                after: duration,
                done: false,
                waker: None,
            });
            id
        };
        Box::pin(Wait {
            clock: self.0.clone(),
            id,
        })
    }
}

struct Wait {
    clock: Rc<RefCell<ClockState>>,
    id: u64,
}

impl Future for Wait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut state = self.clock.borrow_mut();
        let Some(waiter) = state.waiting.iter_mut().find(|waiter| waiter.id == self.id) else {
            return Poll::Ready(());
        };
        if waiter.done {
            return Poll::Ready(());
        }
        waiter.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        if let Ok(mut state) = self.clock.try_borrow_mut() {
            let id = self.id;
            state.waiting.retain(|waiter| waiter.id != id);
        }
    }
}

/// `Rig`: the engine and everything it was built over.
pub struct Rig {
    pub engine: CallEngine,
    pub socket: Rc<Wire>,
    pub media: Rc<Media>,
    pub clock: CallClock,
    /// Every `GET /calls/ice` asked (`Asked`).
    pub asked: Rc<RefCell<Vec<String>>>,
    pool: RefCell<LocalPool>,
}

/// `Theirs`: the far side's candidate.
pub fn theirs() -> IceCandidate {
    IceCandidate::new("candidate:theirs", Some("0"), Some(0))
}

/// The STUN server `GET /calls/ice` answers with unless a test says otherwise.
pub fn one_stun_server() -> ApiResult<IceServersResponse> {
    ApiResult::success(
        IceServersResponse {
            ice_servers: Some(vec![IceServerDto {
                urls: Some(vec!["stun:stun.example.com".to_string()]),
                username: None,
                credential: None,
            }]),
            ttl_secs: None,
        },
        200,
    )
}

/// `ApiResult.Failure(new ApiError(code, "no", 403))`.
pub fn refused(code: &str) -> ApiResult<IceServersResponse> {
    ApiResult::failure(ApiError::new(code, "no", 403))
}

/// `ApiResult.Failure(ApiError.Transport("down"))`.
pub fn unreachable() -> ApiResult<IceServersResponse> {
    ApiResult::failure(ApiError::transport("down"))
}

/// `Offer(callId, video)`: a call to this device in chat 42 from user 9.
pub fn offer(call_id: &str, video: bool) -> ServerFrame {
    ServerFrame::CallOffer {
        call_id: call_id.to_string(),
        chat_id: 42,
        from_user_id: 9,
        sdp: "v=theirs".to_string(),
        video,
    }
}

pub fn ringing(call_id: &str) -> ServerFrame {
    ServerFrame::CallRinging {
        call_id: call_id.to_string(),
    }
}

pub fn answer(call_id: &str, sdp: &str) -> ServerFrame {
    ServerFrame::CallAnswer {
        call_id: call_id.to_string(),
        sdp: sdp.to_string(),
    }
}

pub fn ice(call_id: &str, candidate: IceCandidate) -> ServerFrame {
    ServerFrame::CallIce {
        call_id: call_id.to_string(),
        candidate,
    }
}

pub fn end(call_id: &str, reason: &str) -> ServerFrame {
    ServerFrame::CallEnd {
        call_id: call_id.to_string(),
        reason: reason.to_string(),
    }
}

/// `new ServerFrame.Error(code, message, null, callId)`.
pub fn refusal(code: &str, message: &str, call_id: &str) -> ServerFrame {
    ServerFrame::Error {
        code: code.to_string(),
        detail: message.to_string(),
        client_msg_id: None,
        call_id: Some(call_id.to_string()),
    }
}

/// `Say`: English.
pub fn say() -> EnglishCatalog {
    EnglishCatalog::new()
}

/// A frame as JSON (`Frame`).
pub fn frame(text: &str) -> Value {
    serde_json::from_str(text).expect("a frame is JSON")
}

/// A spawned future's end, for a test that awaits it later (`var placing = …; await placing`).
pub struct Pending(Rc<Cell<bool>>);

impl Pending {
    pub fn done(&self) -> bool {
        self.0.get()
    }
}

impl Rig {
    /// `Build()`: the servers answer one STUN server.
    pub fn build() -> Rig {
        Rig::build_with(one_stun_server())
    }

    /// `Build(servers)`.
    pub fn build_with(servers: ApiResult<IceServersResponse>) -> Rig {
        let socket = Rc::new(Wire::default());
        let media = Rc::new(Media::default());
        let clock = CallClock::new();
        let asked = Rc::new(RefCell::new(Vec::new()));
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        let ice_servers = {
            let asked = asked.clone();
            Rc::new(move || {
                asked.borrow_mut().push("/calls/ice".to_string());
                let answer = servers.clone();
                Box::pin(std::future::ready(answer))
                    as LocalBoxFuture<'static, ApiResult<IceServersResponse>>
            })
        };
        let ids = Cell::new(0);
        let engine = CallEngine::new(
            socket.clone(),
            ice_servers,
            media.clone(),
            Rc::new(clock.clone()),
            Rc::new(move |future| spawner.spawn_local(future).expect("the pool takes work")),
        )
        .with_call_ids(move || {
            ids.set(ids.get() + 1);
            format!("call-{}", ids.get())
        });
        Rig {
            engine,
            socket,
            media,
            clock,
            asked,
            pool: RefCell::new(pool),
        }
    }

    /// Everything the last action set going, run until nothing more can move.
    pub fn settle(&self) {
        self.pool.borrow_mut().run_until_stalled();
    }

    /// Spawn a future of the engine's and run until it waits — `var task = Engine.SomethingAsync()`.
    pub fn start(&self, future: impl Future<Output = ()> + 'static) -> Pending {
        let done = Rc::new(Cell::new(false));
        let flag = done.clone();
        self.pool
            .borrow()
            .spawner()
            .spawn_local(async move {
                future.await;
                flag.set(true);
            })
            .expect("the pool takes work");
        self.settle();
        Pending(done)
    }

    /// `await Engine.PlaceAsync(chat, peer, video)`.
    pub fn place(&self, chat_id: i64, peer_user_id: i64, video: bool) {
        let placing = self.start(self.engine.place(chat_id, peer_user_id, video));
        assert!(placing.done(), "placing waits for nothing here");
    }

    /// `await Engine.AnswerAsync()`.
    pub fn answer(&self) {
        let answering = self.start(self.engine.answer());
        assert!(answering.done(), "answering waits for nothing here");
    }

    /// `await Engine.LeaveAsync()`.
    pub fn leave(&self) {
        let leaving = self.start(self.engine.leave());
        assert!(leaving.done(), "leaving waits for nothing here");
    }

    /// `Engine.Hear(frame)`.
    pub fn hear(&self, frame: ServerFrame) {
        self.engine.hear(&frame);
        self.settle();
    }

    pub fn end(&self) {
        self.engine.end();
        self.settle();
    }

    pub fn decline(&self) {
        self.engine.decline();
        self.settle();
    }

    /// `Clock.Pass(after)`.
    pub fn pass(&self, after: Duration) {
        self.clock.pass(after);
        self.settle();
    }

    /// `Media.Gather(candidate)`.
    pub fn gather(&self, candidate: &str) {
        self.media.gather(candidate);
        self.settle();
    }

    /// `Media.Up()`.
    pub fn up(&self) {
        self.media.up();
        self.settle();
    }

    /// `Media.Die()`.
    pub fn die(&self) {
        self.media.die();
        self.settle();
    }

    /// `Media.Tracks()`.
    pub fn tracks(&self) {
        self.media.tracks();
        self.settle();
    }

    /// `Socket.Sent`.
    pub fn sent(&self) -> Vec<String> {
        self.socket.sent.borrow().clone()
    }

    /// The `n`th frame sent, as JSON.
    pub fn sent_frame(&self, n: usize) -> Value {
        frame(&self.sent()[n])
    }

    /// `Types(rig)`: each frame's type, in order.
    pub fn types(&self) -> Vec<String> {
        self.sent()
            .iter()
            .map(|sent| frame(sent)["type"].as_str().unwrap_or_default().to_string())
            .collect()
    }

    /// `LastReason(rig)`: the last frame's `reason`.
    pub fn last_reason(&self) -> Option<String> {
        let sent = self.sent();
        let last = sent.last().expect("something was sent");
        frame(last)["reason"].as_str().map(str::to_string)
    }

    /// `Engine.Call!`.
    pub fn call(&self) -> fc_logic::calls::CallState {
        self.engine.call().expect("a call is held")
    }
}

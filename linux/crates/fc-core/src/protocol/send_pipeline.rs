//! Everything between "somebody pressed send" and "the server has it" (docs/protocol.md, "Sending
//! on an unreliable network") — a port of `win/src/FamilyConnect.Core/Protocol/SendPipeline.cs`.
//!
//! A SEND HAS THREE OUTCOMES: delivered, refused, and unknown. Unknown is by far the most common on
//! a bad network and is the one that must never be shown — so the row is written down BEFORE
//! anything moves, the frame is given [`ACK_DEADLINE`] to be answered, an unanswered frame falls
//! back to `POST /chats/{id}/messages` with the SAME `client_msg_id` (which is what makes the
//! repeat safe), and only a terminal code marks the row failed.
//!
//! A RETURNING NETWORK IS A TRIGGER. Waiting for the next reconnect to come round on its own is what
//! makes a message sit unsent under a full signal bar, so the app flushes on connectivity, on the
//! socket connecting, on the window coming forward and on a timer — and the flush runs FIRST in a
//! resync rather than last.
//!
//! **How the app wires it.** One pipeline per signed-in server, over the same socket the router
//! hears ([`SendPipeline::from_api`]); it listens to the socket's frames itself from the moment it
//! is made, as the C# subscribed in its constructor, and stops listening when it is dropped. The app
//! spawns [`SendPipeline::flush`] on the main context for every [`FlushTrigger`] — connectivity
//! restored, the window activated, a timer, a retry, a row just queued — and the resync runs one
//! first on every connection. Two flushes at once are safe: the second answers 0 at once.
//!
//! Cancelling a flush is dropping its future (the C#'s token): the row it was on stays queued, and
//! the next flush sends it again under the same dedup key.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::rc::{Rc, Weak};

use futures_channel::oneshot;
use futures_util::future::{Either, select};

use super::api_client::ApiClient;
use super::api_error::ApiError;
use super::api_result::ApiResult;
use super::chat_socket::FrameSender;
use super::dtos::{MentionDto, MessageDto};
use super::frames::{ServerFrame, client_frames};
use super::reconnect_backoff::ReconnectBackoff;
use super::requests::MessageResponse;
use super::send_rules::{ACK_DEADLINE, FlushTrigger, Outcome};
use crate::rt::{Clock, Event, LocalBoxFuture, Subscription};
use crate::store::{ChatStore, OutboxRow, OutboxStore, SeqRoute, StoreError, StoreResult};

/// The REST fallback: one row, posted with its own dedup key — the client's `send_message` in the
/// app, a script in a test.
pub type Post = Rc<dyn Fn(OutboxRow) -> LocalBoxFuture<'static, ApiResult<MessageResponse>>>;

/// The uploads queued media rows owe, pushed BEFORE anything is posted (the media pump's
/// `push`). A cache that refuses one of its writes ends the flush, as the C#'s exception did.
pub type Uploads = Rc<dyn Fn() -> LocalBoxFuture<'static, StoreResult<()>>>;

/// What a send may carry beyond its chat and its body — [`SendPipeline::enqueue`]'s optional
/// parameters in `win/`. An empty list is none: the row stores `None`, as the C# did.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueueOptions {
    pub reply_to_message_id: Option<i64>,
    pub attachment_ids: Vec<i64>,
    pub pending_files: Vec<String>,
    pub poll_options: Vec<String>,
    pub mentions: Vec<MentionDto>,
    pub sticker: bool,
    pub round: bool,
    pub animation: bool,
}

/// What came back for a frame: the message, or a refusal. (The message is boxed: it is most of
/// the enum's size, and the refusal is the common case on a bad network.)
#[derive(Debug, Clone)]
enum Answered {
    Message(Box<MessageDto>),
    Error(ApiError),
}

/// Everything between "somebody pressed send" and "the server has it".
pub struct SendPipeline {
    inner: Rc<Inner>,
    subscription: Subscription,
}

struct Inner {
    socket: Rc<dyn FrameSender>,
    outbox: Rc<OutboxStore>,
    chats: Rc<ChatStore>,
    post: Post,
    backoff: RefCell<ReconnectBackoff>,
    /// The time and the ack deadline's wait: a real clock in the app and a fake in a test — a
    /// suite that sleeps the deadline is a suite nobody runs.
    clock: Rc<dyn Clock>,
    next_id: RefCell<Rc<dyn Fn() -> String>>,
    /// The uploads a queued media row owes, pushed BEFORE anything is posted. A flush that skipped
    /// them would pass over every media row for ever: the row may not be posted while it owes a
    /// byte, and nothing else is going to move those bytes.
    uploads: RefCell<Option<Uploads>>,
    /// Sends waiting for an `ack` or an `error`, by dedup key. A key whose sender is gone has been
    /// answered already, and a second answer for it changes nothing (the C#'s `TrySetResult`).
    waiting: RefCell<HashMap<String, Option<oneshot::Sender<Answered>>>>,
    /// One send at a time per row: two passes would each upload the remainder.
    flushing: Cell<bool>,
    last_trigger: Cell<Option<FlushTrigger>>,
    refused: Event<(OutboxRow, ApiError)>,
    cache_failed: Event<StoreError>,
}

impl std::fmt::Debug for SendPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SendPipeline")
            .field("flushing", &self.inner.flushing.get())
            .field("waiting", &self.inner.waiting.borrow().len())
            .finish_non_exhaustive()
    }
}

impl SendPipeline {
    /// `new SendPipeline(socket, outbox, chats, post, …)`: the socket a frame goes out on first,
    /// the queue, the cache an answered send is applied to, the REST fallback, and the clock that
    /// says what time it is and waits out the ack deadline. The backoff is the shared one second
    /// doubling to thirty, uniformly jittered, and the dedup key a real UUID, unless
    /// [`with_backoff`](Self::with_backoff) and [`with_next_id`](Self::with_next_id) say otherwise.
    ///
    /// It listens to the socket's frames from here on — two of them answer a send.
    pub fn new(
        socket: Rc<dyn FrameSender>,
        outbox: Rc<OutboxStore>,
        chats: Rc<ChatStore>,
        post: Post,
        clock: Rc<dyn Clock>,
    ) -> Self {
        let inner = Rc::new(Inner {
            socket: Rc::clone(&socket),
            outbox,
            chats,
            post,
            backoff: RefCell::new(ReconnectBackoff::default()),
            clock,
            // A REAL UUID: the server takes the dedup key as given, and a client that invents
            // something shorter finds out on a collision, in a family's chat.
            next_id: RefCell::new(Rc::new(uuid_v4)),
            uploads: RefCell::new(None),
            waiting: RefCell::new(HashMap::new()),
            flushing: Cell::new(false),
            last_trigger: Cell::new(None),
            refused: Event::new(),
            cache_failed: Event::new(),
        });
        let listening: Weak<Inner> = Rc::downgrade(&inner);
        let subscription = socket.on_frame().subscribe(move |frame| {
            if let Some(inner) = listening.upgrade()
                && let Err(error) = inner.heard(frame)
            {
                // Nothing to hand it back to: the row stays queued, and the next flush sends it
                // again under the same key, which the server dedups.
                inner.cache_failed.emit(&error);
            }
        });
        SendPipeline {
            inner,
            subscription,
        }
    }

    /// The pipeline over a live client: the REST fallback is that client's own
    /// [`send_message`](ApiClient::send_message), with the row's options.
    pub fn from_api(
        socket: Rc<dyn FrameSender>,
        outbox: Rc<OutboxStore>,
        chats: Rc<ChatStore>,
        api: Rc<ApiClient>,
        clock: Rc<dyn Clock>,
    ) -> Self {
        let post: Post = Rc::new(move |row: OutboxRow| {
            let api = Rc::clone(&api);
            Box::pin(async move {
                api.send_message(
                    row.chat_id,
                    &row.client_msg_id,
                    &row.body,
                    &send_options(&row),
                )
                .await
            })
        });
        SendPipeline::new(socket, outbox, chats, post, clock)
    }

    /// The outbox's own per-row schedule — a test's pinned random source.
    pub fn with_backoff(self, backoff: ReconnectBackoff) -> Self {
        *self.inner.backoff.borrow_mut() = backoff;
        self
    }

    /// Where dedup keys come from — a test's fixed id.
    pub fn with_next_id(self, next_id: impl Fn() -> String + 'static) -> Self {
        *self.inner.next_id.borrow_mut() = Rc::new(next_id);
        self
    }

    /// The uploads a queued media row owes, pushed at the start of every flush (the media pump).
    pub fn with_uploads(self, uploads: Uploads) -> Self {
        *self.inner.uploads.borrow_mut() = Some(uploads);
        self
    }

    /// A row was refused for good, and the user has to be told: `(row, error)`.
    pub fn on_refused(&self) -> &Event<(OutboxRow, ApiError)> {
        &self.inner.refused
    }

    /// A frame answered a send and the cache could not record it. Nothing is lost — the row stays
    /// queued, and the next flush sends it again under the same key — but it is the app's to log
    /// (the C# let the exception out of the socket's frame event, which ended the socket's loop).
    pub fn on_cache_failed(&self) -> &Event<StoreError> {
        &self.inner.cache_failed
    }

    /// Why the last flush ran, for a log and a test: the trigger is recorded rather than acted on
    /// — every one of them means the same thing here ("try now").
    pub fn last_trigger(&self) -> Option<FlushTrigger> {
        self.inner.last_trigger.get()
    }

    /// Write a send down and try nothing yet. Answers the row as queued — the bubble is drawn from
    /// this, before anything has moved; the flush is a separate act, which is what lets it run
    /// first in a resync and on every trigger.
    pub fn enqueue(
        &self,
        chat_id: i64,
        body: &str,
        options: &QueueOptions,
    ) -> StoreResult<OutboxRow> {
        fn some<T: Clone>(items: &[T]) -> Option<Vec<T>> {
            (!items.is_empty()).then(|| items.to_vec())
        }
        let next_id = Rc::clone(&self.inner.next_id.borrow());
        let row = OutboxRow {
            client_msg_id: next_id(),
            chat_id,
            body: body.to_string(),
            reply_to_message_id: options.reply_to_message_id,
            attachment_ids: some(&options.attachment_ids),
            pending_files: some(&options.pending_files),
            poll_options: some(&options.poll_options),
            mentions: some(&options.mentions),
            queued_at: Some(self.inner.clock.now()),
            // NOTHING ELSE ABOUT THE SEND PATH IS NEW for a sticker: the row, the dedup key, the
            // uploads it owes and the retry rules are a message's. This bit is the whole of it.
            sticker: options.sticker,
            // And for a video message the same: one more bit on the row, and the send is a
            // message's.
            round: options.round,
            // And for an animation: one more bit, and the send is a message's.
            animation: options.animation,
            ..OutboxRow::default()
        };
        self.inner.outbox.queue(&row)?;
        Ok(row)
    }

    /// Try everything that is due. Answers how many landed.
    ///
    /// THE SAME SEND MUST NOT RUN TWICE AT ONCE: two passes over one row would each upload the
    /// remainder, and the second would post a message the first had already posted — so a flush
    /// that finds another still running answers 0 at once.
    pub async fn flush(&self, trigger: FlushTrigger) -> StoreResult<usize> {
        let inner = &self.inner;
        inner.last_trigger.set(Some(trigger));
        if inner.flushing.replace(true) {
            return Ok(0);
        }
        // Released however the flush ends — finished, failed, or dropped part-way.
        let _flushing = Flushing(&inner.flushing);
        let uploads = inner.uploads.borrow().clone();
        if let Some(uploads) = uploads {
            // FIRST: a row that owes a byte is passed over below, so the bytes have to move here
            // or they never move at all.
            uploads().await?;
        }
        let mut landed = 0;
        for row in inner.outbox.due(inner.clock.now())? {
            if row.owes_uploads() {
                // Until every upload has landed this row must never be posted: a message claiming
                // no attachments is a text message, and the server would take it.
                continue;
            }
            if self.send(&row).await? {
                landed += 1;
            }
        }
        Ok(landed)
    }

    /// One row: the socket first, then REST for whatever the socket could not answer for.
    async fn send(&self, row: &OutboxRow) -> StoreResult<bool> {
        let inner = &self.inner;
        // No answer at all is NOT a refusal — it says only that the frame was not acknowledged,
        // which is the case the REST path exists for.
        match self.over_socket(row).await {
            Some(Answered::Message(acked)) => return inner.delivered(&acked),
            Some(Answered::Error(refused)) if !refused.transient() => {
                return inner.failed(row, &refused);
            }
            _ => {}
        }
        let post = Rc::clone(&inner.post);
        match post(row.clone()).await.into_result() {
            Ok(response) => inner.delivered(&response.message),
            Err(error) => inner.failed(row, &error),
        }
    }

    /// The frame, and the ack deadline. A `send` with no answer inside the window is unanswered —
    /// and an `error` frame carrying a TRANSIENT code is treated exactly as no answer at all.
    async fn over_socket(&self, row: &OutboxRow) -> Option<Answered> {
        let inner = &self.inner;
        if !inner.socket.is_connected() {
            return None;
        }
        let (sender, mut receiver) = oneshot::channel();
        inner
            .waiting
            .borrow_mut()
            .insert(row.client_msg_id.clone(), Some(sender));
        // Out of the waiting list however this ends — answered, timed out, or dropped.
        let _waiting = Waiting {
            waiting: &inner.waiting,
            key: &row.client_msg_id,
        };
        let frame = client_frames::send(
            row.chat_id,
            &row.client_msg_id,
            &row.body,
            &send_options(row),
        );
        if !inner.socket.try_send(frame).await {
            // The write itself did not go — a socket whose connection is dead absorbs one, so this
            // is as much of an answer as the deadline expiring.
            return None;
        }
        // An answer already in hand beats the deadline, always: the server may answer before the
        // write even returns. `select` polls its first future first, which would happen to give
        // the same — but "happens to" is not a rule to rely on, so it is written out.
        if let Ok(Some(answer)) = receiver.try_recv() {
            return Some(answer);
        }
        match select(receiver, inner.clock.sleep(ACK_DEADLINE)).await {
            Either::Left((Ok(answer), _)) => Some(answer),
            Either::Left((Err(_), _)) | Either::Right(_) => None,
        }
    }
}

impl Drop for SendPipeline {
    fn drop(&mut self) {
        self.inner.socket.on_frame().unsubscribe(self.subscription);
    }
}

impl Inner {
    /// Every frame the socket hands over; two of them answer a send.
    fn heard(&self, frame: &ServerFrame) -> StoreResult<()> {
        match frame {
            ServerFrame::Ack {
                client_msg_id,
                value,
            } => self.answer(client_msg_id, Answered::Message(Box::new(value.clone()))),
            ServerFrame::Error {
                code,
                detail,
                client_msg_id: Some(id),
                ..
            } => self.answer(id, Answered::Error(ApiError::coded(code, detail))),
            // This device's own send, arriving as an ordinary message because the ack was lost:
            // still a delivery, and the row goes.
            ServerFrame::Message(message) => match &message.client_msg_id {
                Some(id) => self.answer(id, Answered::Message(Box::new(message.clone()))),
                None => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn answer(&self, client_msg_id: &str, answered: Answered) -> StoreResult<()> {
        let pending = self
            .waiting
            .borrow_mut()
            .get_mut(client_msg_id)
            .map(Option::take);
        match pending {
            Some(Some(sender)) => {
                // The waiter may have gone between the two lines that matter; then the next flush
                // finds the row and sends it again, safely.
                let _ = sender.send(answered);
                return Ok(());
            }
            // Answered already: a second answer for the same send changes nothing.
            Some(None) => return Ok(()),
            None => {}
        }
        // Nobody is waiting: an ack that arrived after the deadline, or after a relaunch. The row
        // is still here, and it is still delivered.
        if let Answered::Message(message) = answered
            && self.outbox.find(client_msg_id)?.is_some()
        {
            self.delivered(&message)?;
        }
        Ok(())
    }

    fn delivered(&self, message: &MessageDto) -> StoreResult<bool> {
        // Out of sequence: the answer to this device's own send says nothing about the messages
        // before it — a send answered while the socket was reconnecting would otherwise become
        // where the catch-up starts. Its frame, or the next catch-up, delivers it again in
        // sequence. (The route is the C#'s default, a catch-up page's: a new reply on it raises
        // its root's count.)
        self.chats.apply(message, SeqRoute::CatchUpPage, false)?;
        if let Some(id) = &message.client_msg_id {
            self.outbox.delivered(id)?;
        }
        Ok(true)
    }

    fn failed(&self, row: &OutboxRow, error: &ApiError) -> StoreResult<bool> {
        let what = self.outbox.failed(
            &row.client_msg_id,
            error,
            self.clock.now(),
            &mut self.backoff.borrow_mut(),
            // A client that has thrown its copy away has no way to recover an expired upload.
            row.holds_bytes(),
        )?;
        if what == Outcome::ReuploadAndRetry {
            // The one refusal with a way out: the ids are dead, the bytes are not. The row owes its
            // uploads again, and the media pump pushes them before it may be posted.
            self.outbox.reupload(&row.client_msg_id)?;
        }
        if what == Outcome::Failed {
            // The one case the user is told about, and the only one with a retry affordance.
            self.refused.emit(&(row.clone(), error.clone()));
        }
        Ok(false)
    }
}

/// The `send` frame's options and the REST body's, from the row — so the two paths cannot drift
/// apart.
fn send_options(row: &OutboxRow) -> client_frames::SendOptions<'_> {
    client_frames::SendOptions {
        reply_to_message_id: row.reply_to_message_id,
        attachment_ids: row.attachment_ids.as_deref().unwrap_or(&[]),
        poll_options: row.poll_options.as_deref().unwrap_or(&[]),
        mentions: row.mentions.as_deref().unwrap_or(&[]),
        sticker: row.sticker,
        round: row.round,
        animation: row.animation,
    }
}

/// The C#'s `finally { flushing.Release(); }`.
struct Flushing<'a>(&'a Cell<bool>);

impl Drop for Flushing<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

/// The C#'s `finally { waiting.Remove(row.ClientMsgId); }`.
struct Waiting<'a> {
    waiting: &'a RefCell<HashMap<String, Option<oneshot::Sender<Answered>>>>,
    key: &'a str,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        if let Ok(mut waiting) = self.waiting.try_borrow_mut() {
            waiting.remove(self.key);
        }
    }
}

/// A version-4 UUID, lower-case and hyphenated, as .NET's `Guid.NewGuid().ToString()` writes one:
/// 122 random bits from the system (`/dev/urandom`), with the version and variant bits set.
///
/// Where the system's source cannot be read, std's per-process random hasher keys stand in — seeded
/// from the operating system once, and mixed with the time and a counter so that no two calls in
/// one process answer the same.
pub fn uuid_v4() -> String {
    let mut bytes = [0u8; 16];
    let read =
        std::fs::File::open("/dev/urandom").and_then(|mut source| source.read_exact(&mut bytes));
    if read.is_err() {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static CALLS: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let call = CALLS.fetch_add(1, Ordering::Relaxed);
        for (half, chunk) in bytes.chunks_mut(8).enumerate() {
            let mut hasher = RandomState::new().build_hasher();
            hasher.write_u128(nanos);
            hasher.write_u64(call);
            hasher.write_usize(half);
            chunk.copy_from_slice(&hasher.finish().to_le_bytes());
        }
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

//! Everything a signed-in session does on its own: the first load, the
//! socket and its reconnects, the resync every connection owes, the frames
//! that arrive, and the small timers. The app starts it and a sign-out
//! stops it; nothing a person clicks lives here.

use std::cell::RefCell;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::{FutureExt, SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message as WsMessage, State};
use wasm_bindgen_futures::spawn_local;
use yew::Callback;

use crate::api::{self, ApiError};
use crate::live::{AppState, Live};
use crate::outbox::{self, Wake};
use crate::socket::{self, ClientFrame, ServerFrame};
use crate::store::{self, Feed, Via};

/// How many messages a page asks for. The protocol's default is 50 and its
/// ceiling 200; 50 is a screenful and a bit on any window.
pub const PAGE: u32 = 50;

/// How many rows a catch-up page of reactions, edits or polls asks for.
const FEED_PAGE: u32 = 100;

/// The heartbeat. A ping every beat — and a socket that has said nothing
/// at all since the last one, not even its pong, is DEAD, however open the
/// browser thinks it is. A laptop that slept leaves exactly that behind,
/// and a browser has no other way to find out.
const PING_MS: u32 = 30_000;

/// How often a stale "is typing" line is swept away.
const TYPING_SWEEP_MS: u32 = 2_000;

/// Milliseconds since the page loaded. The clock everything time-based
/// here reads — monotonic, unlike the wall clock, so a machine waking from
/// sleep with a corrected clock cannot make a typing indicator immortal.
pub fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_default()
}

/// Wall-clock milliseconds — for comparing against a message's timestamp,
/// which the monotonic clock above cannot do.
pub fn wall_ms() -> f64 {
    js_sys::Date::now()
}

/// Whether anybody can see the page. A message landing in the open chat of
/// a tab nobody is looking at has not been read.
pub fn page_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .map(|document| !document.hidden())
        .unwrap_or(true)
}

/// Whether this window is the one in front — a page visible on a second
/// screen while its reader types somewhere else is not being read.
fn page_focused() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.has_focus().ok())
        .unwrap_or(true)
}

/// The chat being READ: the open one, while its window is in front and
/// visible, AND its reader is at the newest message — somebody three
/// screens up in the history has not read what just landed below the fold,
/// and a read marker, once sent, is wrong on every device for good (ios
/// ChatPresence; MacConversationView reads only with the newest on screen).
pub fn reading(state: &AppState) -> Option<i64> {
    state
        .open_chat
        .filter(|_| state.at_newest && page_visible() && page_focused())
}

/// How the app reaches the session's long-lived tasks. Closed when the
/// session ends, which stops every one of them at once rather than at its
/// next tick.
pub struct Channels {
    /// Frames for the socket.
    pub frames: mpsc::UnboundedSender<ClientFrame>,
    /// "Look at the outbox again."
    pub wake: mpsc::UnboundedSender<Wake>,
    /// "Dial now": the network is back, stop waiting out the backoff.
    pub redial: mpsc::UnboundedSender<()>,
}

impl Channels {
    pub fn close(&self) {
        self.frames.close_channel();
        self.wake.close_channel();
        self.redial.close_channel();
    }
}

pub type Shared<T> = Rc<RefCell<T>>;

pub fn wake(channels: &Shared<Option<Channels>>, why: Wake) {
    if let Some(open) = channels.borrow().as_ref() {
        let _ = open.wake.unbounded_send(why);
    }
}

/// The network may be back: send what is stuck, and dial NOW if the socket
/// is down rather than at the end of its backoff (docs/protocol.md, "A
/// returning network is a trigger").
pub fn network_back(live: &Live, channels: &Shared<Option<Channels>>) {
    let connected = live.read(|state| state.connected);
    if let Some(open) = channels.borrow().as_ref() {
        let _ = open.wake.unbounded_send(Wake::Network);
        if !connected {
            let _ = open.redial.unbounded_send(());
        }
    }
}

/// What a 401 does. The session is gone, so sign out — but only if it is
/// still THIS session: a 401 for a token somebody already signed out of
/// must not sign out whoever signed in after them.
pub fn expiry(live: &Live, session: u64, sign_out: &Callback<bool>) -> Rc<dyn Fn()> {
    let live = live.clone();
    let sign_out = sign_out.clone();
    Rc::new(move || {
        if live.is_live(session) {
            sign_out.emit(false);
        }
    })
}

/// Start everything a signed-in session runs: the first load, the socket,
/// the outbox and the typing sweep. Each stops when the session ends.
pub fn start_session(
    live: &Live,
    token: String,
    channels: &Shared<Option<Channels>>,
    sign_out: &Callback<bool>,
) {
    let session = live.session();
    let (frames, frames_out) = mpsc::unbounded();
    let (wake, wake_in) = mpsc::unbounded();
    let (redial, redial_in) = mpsc::unbounded();
    *channels.borrow_mut() = Some(Channels {
        frames,
        wake: wake.clone(),
        redial,
    });
    let expired = expiry(live, session, sign_out);

    {
        let live = live.clone();
        let token = token.clone();
        let expired = expired.clone();
        spawn_local(async move { resync(&live, session, &token, &expired).await });
    }
    spawn_local(keep_socket(
        live.clone(),
        session,
        token.clone(),
        frames_out,
        redial_in,
        wake,
        expired.clone(),
    ));
    let upload_token = token.clone();
    spawn_local(outbox::drain(
        live.clone(),
        session,
        wake_in,
        move |row| {
            let token = token.clone();
            async move { api::send_message(&token, &row).await }
        },
        move |job: outbox::Upload| {
            let token = upload_token.clone();
            async move { upload(&token, job).await }
        },
        |attempts, floor| {
            let ceiling = outbox::backoff_ceiling_ms(attempts);
            let jittered = outbox::jittered_ms(ceiling, js_sys::Math::random());
            gloo_timers::future::TimeoutFuture::new(jittered.max(floor as u32))
        },
        move || expired(),
    ));
    spawn_local(sweep_typing(live.clone(), session));
}

/// One attachment up, and its preview after it.
///
/// The preview is best effort — a message may go without it — but not
/// best effort ONCE: a video's poster is the one picture with no second
/// source, and a poster that failed once is a grey tile for every
/// recipient for good (docs/protocol.md, "A preview may be uploaded again,
/// later"). So a failed one is tried twice more, beside the send rather
/// than in front of it, from the bytes this tab still holds.
async fn upload(token: &str, job: outbox::Upload) -> Result<crate::model::Attachment, ApiError> {
    // A picked file is the person's own, and they may have moved, changed or
    // deleted it since. The browser then fails the upload like a dropped
    // network — and it would be retried as one, and end as "check your
    // connection". Asked first, it fails as what it is.
    if let Some(file) = &job.bytes.file {
        if !crate::prep::readable(file).await {
            return Err(ApiError::Server {
                code: outbox::LOCAL_FILE_GONE.to_string(),
                message: "The file changed or is gone.".to_string(),
            });
        }
    }
    let attachment = api::upload_attachment(token, &job.item, job.bytes.file.as_ref()).await?;
    if let Some(preview) = job.bytes.preview {
        if api::upload_preview(token, attachment.id, &preview)
            .await
            .is_err()
        {
            let token = token.to_string();
            let id = attachment.id;
            spawn_local(async move {
                for wait in [2_000, 8_000] {
                    gloo_timers::future::TimeoutFuture::new(wait).await;
                    if api::upload_preview(&token, id, &preview).await.is_ok() {
                        break;
                    }
                }
            });
        }
    }
    Ok(attachment)
}

/// What every connection owes, in the protocol's order (docs/protocol.md,
/// "Semantics", resync): who this is and the block list, the roster, the
/// chat list with its authoritative counts, and then — per chat this client
/// holds messages of — what it missed: messages after the newest held, and
/// the reactions, edits and polls that changed past each cursor.
pub async fn resync(live: &Live, session: u64, token: &str, expired: &Rc<dyn Fn()>) {
    match api::me(token).await {
        Ok(me) => {
            live.update(session, |state| state.store.apply_me(&me));
        }
        Err(ApiError::Unauthorized) => {
            expired();
            return;
        }
        Err(error) => {
            live.update(session, |state| state.failure = Some(error.detail()));
        }
    }
    // An account in no family has no roster; that is a real state rather
    // than a failure worth showing.
    if let Ok(roster) = api::family(token).await {
        live.update(session, |state| state.store.apply_roster(&roster));
    }
    if !refresh_chats(live, session, token, expired).await {
        return;
    }
    catch_up(live, session, token).await;
    report_read(live, session, token).await;
}

/// The chat list, as the server has it now — MERGED into what is held
/// (`Store::merge_chats`), then the chat being read reported read, because
/// the list may say it has unread messages this reader is looking at.
/// Answers whether it was read.
pub async fn refresh_chats(live: &Live, session: u64, token: &str, expired: &Rc<dyn Fn()>) -> bool {
    match api::chats(token).await {
        Ok(chats) => {
            live.update(session, |state| {
                state.store.merge_chats(chats);
                state.failure = None;
            });
            report_read(live, session, token).await;
            true
        }
        Err(ApiError::Unauthorized) => {
            expired();
            false
        }
        Err(error) => {
            live.update(session, |state| state.failure = Some(error.detail()));
            false
        }
    }
}

/// What arrived while this client was not listening, for every chat it
/// holds messages of.
///
/// Messages: `after_id` from the newest held, the cursor belonging to the
/// LOOP — read once, then advanced by what each page returned, so a live
/// message landing mid-loop cannot jump it past the rest. Then each of the
/// three feeds, but only where the chat list says the server is ahead of
/// the cursor held. A chat never opened holds nothing to have a hole in.
async fn catch_up(live: &Live, session: u64, token: &str) {
    let held: Vec<(i64, i64, i64, [Option<i64>; 3])> = live.read(|state| {
        state
            .store
            .threads
            .iter()
            .filter_map(|(chat_id, thread)| {
                let newest = thread.newest_server_id()?;
                let item = state.store.item(*chat_id)?;
                Some((
                    *chat_id,
                    newest,
                    state.store.counted_up_to(*chat_id),
                    [item.max_reaction_seq, item.max_edit_seq, item.max_poll_seq],
                ))
            })
            .collect()
    });
    for (chat_id, newest, counted, highs) in held {
        if !catch_up_messages(live, session, token, chat_id, newest, counted).await {
            return;
        }
        for (feed, high) in [Feed::Reactions, Feed::Edits, Feed::Polls]
            .into_iter()
            .zip(highs)
        {
            let cursor = live.read(|state| {
                let cursors = state
                    .store
                    .cursors
                    .get(&chat_id)
                    .copied()
                    .unwrap_or_default();
                match feed {
                    Feed::Reactions => cursors.reaction,
                    Feed::Edits => cursors.edit,
                    Feed::Polls => cursors.poll,
                }
            });
            if high.is_some_and(|high| high > cursor)
                && !catch_up_feed(live, session, token, chat_id, feed, cursor).await
            {
                return;
            }
        }
        live.update(session, |state| {
            state.store.cursors.entry(chat_id).or_default().caught_up = true;
        });
    }
    // A chat with nothing held has nothing to catch up: its cursors start
    // wherever the first frames take them.
    live.update(session, |state| {
        let ids: Vec<i64> = state.store.chats.iter().map(|item| item.chat.id).collect();
        for chat_id in ids {
            if !state.store.threads.contains_key(&chat_id) {
                state.store.cursors.entry(chat_id).or_default().caught_up = true;
            }
        }
    });
}

/// Answers whether the loop finished — a stale session or a failed page
/// ends the whole catch-up. `counted` is what the chat list already counted
/// as unread (see `Via::CatchUp`).
pub async fn catch_up_messages(
    live: &Live,
    session: u64,
    token: &str,
    chat_id: i64,
    newest: i64,
    counted: i64,
) -> bool {
    let mut after = newest;
    loop {
        let Ok(page) = api::messages_after(token, chat_id, after, PAGE).await else {
            return false;
        };
        if page.is_empty() {
            return true;
        }
        let short = (page.len() as u32) < PAGE;
        after = page.iter().map(|message| message.id).fold(after, i64::max);
        let applied = live.update(session, |state| {
            let reading = reading(state);
            for message in page {
                state.store.apply_message(
                    message,
                    reading,
                    Via::CatchUp {
                        counted_up_to: counted,
                    },
                );
            }
        });
        if applied.is_none() {
            return false;
        }
        if short {
            return true;
        }
    }
}

/// One feed, looped until a short page. The cursor advances with every page
/// EVEN for messages this client does not hold — their states are dropped,
/// and history paging re-delivers them embedded on the messages.
async fn catch_up_feed(
    live: &Live,
    session: u64,
    token: &str,
    chat_id: i64,
    feed: Feed,
    mut cursor: i64,
) -> bool {
    loop {
        let (count, high) = match feed {
            Feed::Reactions => {
                let Ok(page) = api::reactions(token, chat_id, cursor, FEED_PAGE).await else {
                    return false;
                };
                let high = page.iter().map(|row| row.reaction_seq).max();
                let count = page.len();
                live.update(session, |state| {
                    for row in page {
                        state.store.apply_reactions(
                            chat_id,
                            row.message_id,
                            row.reaction_seq,
                            row.reactions,
                        );
                    }
                });
                (count, high)
            }
            Feed::Edits => {
                let Ok(page) = api::edits(token, chat_id, cursor, FEED_PAGE).await else {
                    return false;
                };
                let high = page.iter().filter_map(|message| message.edit_seq).max();
                let count = page.len();
                live.update(session, |state| {
                    for message in page {
                        state.store.apply_edit(message);
                    }
                });
                (count, high)
            }
            Feed::Polls => {
                let Ok(page) = api::polls(token, chat_id, cursor, FEED_PAGE).await else {
                    return false;
                };
                let high = page.iter().map(|row| row.poll.poll_seq).max();
                let count = page.len();
                live.update(session, |state| {
                    for row in page {
                        state.store.apply_poll(chat_id, row.message_id, row.poll);
                    }
                });
                (count, high)
            }
        };
        if let Some(high) = high {
            cursor = cursor.max(high);
            if live
                .update(session, |state| {
                    state.store.advance_cursor(chat_id, feed, high, false)
                })
                .is_none()
            {
                return false;
            }
        }
        if (count as u32) < FEED_PAGE {
            return true;
        }
    }
}

/// The chat being read is read up to its newest message: the badge goes,
/// and the server hears it over REST — the one path that works whether or
/// not the socket is up. Only while somebody can see it, and only when
/// there is something new to say.
pub async fn report_read(live: &Live, session: u64, token: &str) {
    let Some((chat_id, newest)) = live.read(|state| {
        let chat_id = reading(state)?;
        let newest = state.store.threads.get(&chat_id)?.newest_server_id()?;
        let item = state.store.item(chat_id)?;
        (item.unread_count > 0 || item.last_read_message_id < newest).then_some((chat_id, newest))
    }) else {
        return;
    };
    if live
        .update(session, |state| state.store.mark_read(chat_id, newest))
        .is_none()
    {
        return;
    }
    let _ = api::post_read(token, chat_id, newest).await;
}

/// The socket, for as long as `session` is the session: dial, run, and
/// dial again. A browser tab lives for days, and a socket that dropped once
/// and stayed dropped is a chat that silently stops arriving.
async fn keep_socket(
    live: Live,
    session: u64,
    token: String,
    mut frames: mpsc::UnboundedReceiver<ClientFrame>,
    mut redial: mpsc::UnboundedReceiver<()>,
    wake: mpsc::UnboundedSender<Wake>,
    expired: Rc<dyn Fn()>,
) {
    // Waits in a row. A socket that opened and later dropped starts the
    // count again, so the first dial after a drop is quick and only a
    // server that keeps refusing is left longer and longer alone.
    let mut failures = 0u32;
    while live.is_live(session) {
        let url = socket::url_for(&crate::session::origin());
        let protocols = socket::protocols_for(&token);
        let opened = match WebSocket::open_with_protocols(&url, &protocols) {
            Ok(ws) => run_socket(ws, &live, session, &token, &mut frames, &wake, &expired).await,
            Err(_) => false,
        };
        failures = if opened {
            1
        } else {
            failures.saturating_add(1)
        };
        let wait =
            outbox::jittered_ms(outbox::backoff_ceiling_ms(failures), js_sys::Math::random());
        let mut nap = Box::pin(gloo_timers::future::TimeoutFuture::new(wait).fuse());
        futures::select! {
            () = nap => {}
            again = redial.next() => if again.is_none() {
                return;
            },
        }
    }
}

fn encode(frame: &ClientFrame) -> WsMessage {
    WsMessage::Text(serde_json::to_string(frame).expect("a client frame always encodes"))
}

/// One connection, until it closes. Returns whether it ever OPENED, so the
/// caller can tell a socket refused at the door, which grows the backoff,
/// from one that worked and then dropped.
///
/// ONE loop over both directions rather than a reader task and a writer
/// task: two tasks would have to share the socket halves through a
/// `RefCell` held across an await, which is a borrow panic waiting for the
/// first time anything re-enters. `select!` needs neither.
async fn run_socket(
    mut ws: WebSocket,
    live: &Live,
    session: u64,
    token: &str,
    frames: &mut mpsc::UnboundedReceiver<ClientFrame>,
    wake: &mpsc::UnboundedSender<Wake>,
    expired: &Rc<dyn Fn()>,
) -> bool {
    // The handshake. gloo's sink is ready once the socket has left
    // CONNECTING, which it leaves either open or refused — and the server
    // refuses a bad token BEFORE the upgrade, so OPEN means authenticated
    // too. Nothing is said until then: a frame written to a refused socket
    // is thrown away, with a complaint in the console on every redial.
    let handshake = futures::future::poll_fn(|cx| ws.poll_ready_unpin(cx)).await;
    if handshake.is_err() || !matches!(ws.state(), State::Open) {
        return false;
    }
    // Anything said while the socket was down is stale now — a `typing`
    // from a minute ago is a lie — and is dropped rather than delivered late
    // (docs/protocol.md, "A browser is a client too").
    while frames.try_recv().is_ok() {}
    // The resync runs BESIDE the socket, not before it: frames are applied
    // the moment they arrive, and the chat-list merge and the catch-up's
    // `counted_up_to` are what keep the two from counting a message twice
    // or not at all. Holding every frame behind four round trips was a badge
    // that drifted and a typing line that arrived late.
    {
        let live = live.clone();
        let token = token.to_string();
        let wake = wake.clone();
        let expired = expired.clone();
        spawn_local(async move { opened(&live, session, &token, &wake, &expired).await });
    }

    let (mut write, read) = ws.split();
    let mut read = read.fuse();
    let mut beat = gloo_timers::future::IntervalStream::new(PING_MS).fuse();
    let mut pinged_at = now_ms();
    let mut heard_at = pinged_at;
    loop {
        futures::select! {
            incoming = read.next() => {
                let text = match incoming {
                    Some(Ok(WsMessage::Text(text))) => text,
                    // Nothing on this wire is binary: stepped over.
                    Some(Ok(WsMessage::Bytes(_))) => continue,
                    // Closed, or failed.
                    Some(Err(_)) | None => break,
                };
                heard_at = now_ms();
                let Some(frame) = socket::decode(&text) else { continue };
                if let Some(answer) = apply_frame(live, session, token, expired, frame) {
                    if write.send(encode(&answer)).await.is_err() {
                        break;
                    }
                }
            }
            outgoing = frames.next() => {
                // None: the session ended.
                let Some(frame) = outgoing else { break };
                if write.send(encode(&frame)).await.is_err() {
                    break;
                }
            }
            _ = beat.next() => {
                // Not a word since the last ping, not even its pong.
                if heard_at < pinged_at {
                    break;
                }
                pinged_at = now_ms();
                if write.send(encode(&ClientFrame::Ping)).await.is_err() {
                    break;
                }
            }
        }
    }
    live.update(session, |state| {
        state.connected = false;
        // A new connection owes a new catch-up before frames may move a
        // cursor again.
        for cursors in state.store.cursors.values_mut() {
            cursors.caught_up = false;
        }
    });
    true
}

/// The socket is open. The outbox FIRST — "Step 4 is not a step"
/// (docs/protocol.md): whatever is stuck goes out before any read that
/// might fail on the same network. Then the resync.
async fn opened(
    live: &Live,
    session: u64,
    token: &str,
    wake: &mpsc::UnboundedSender<Wake>,
    expired: &Rc<dyn Fn()>,
) {
    live.update(session, |state| state.connected = true);
    let _ = wake.unbounded_send(Wake::Network);
    resync(live, session, token, expired).await;
}

/// One frame from the server, into the state. Returns a frame to answer
/// with, if any: a message landing in the chat somebody is reading has been
/// read, and says so.
fn apply_frame(
    live: &Live,
    session: u64,
    token: &str,
    expired: &Rc<dyn Fn()>,
    frame: ServerFrame,
) -> Option<ClientFrame> {
    let refresh = || {
        let live = live.clone();
        let token = token.to_string();
        let expired = expired.clone();
        spawn_local(async move {
            refresh_chats(&live, session, &token, &expired).await;
        });
    };
    match frame {
        ServerFrame::Message { message } => {
            let chat_id = message.chat_id;
            let (answer, unknown_chat) = live.update(session, |state| {
                let reading = reading(state);
                let id = message.id;
                let theirs = message.sender_id != state.store.my_user_id;
                let unknown_chat = state.store.item(chat_id).is_none();
                state.store.apply_message(message, reading, Via::Frame);
                let answer = (reading == Some(chat_id) && theirs).then(|| {
                    state.store.mark_read(chat_id, id);
                    ClientFrame::Read {
                        chat_id,
                        last_read_message_id: id,
                    }
                });
                (answer, unknown_chat)
            })?;
            // A chat the list has never heard of: somebody has just started
            // a direct chat with this person. Fetched, not guessed at.
            if unknown_chat {
                refresh();
            }
            answer
        }
        ServerFrame::MessageEdited { message } => {
            live.update(session, |state| {
                let chat_id = message.chat_id;
                let seq = message.edit_seq;
                state.store.apply_edit(message);
                if let Some(seq) = seq {
                    state.store.advance_cursor(chat_id, Feed::Edits, seq, true);
                }
            });
            None
        }
        ServerFrame::Read {
            chat_id,
            user_id,
            last_read_message_id,
        } => {
            live.update(session, |state| {
                state
                    .store
                    .apply_read_frame(chat_id, user_id, last_read_message_id)
            });
            None
        }
        ServerFrame::Typing { chat_id, user_id } => {
            live.update(session, |state| {
                state.store.set_typing(chat_id, user_id, now_ms())
            });
            None
        }
        ServerFrame::Reaction {
            chat_id,
            message_id,
            reaction_seq,
            reactions,
        } => {
            live.update(session, |state| {
                state
                    .store
                    .apply_reactions(chat_id, message_id, reaction_seq, reactions);
                state
                    .store
                    .advance_cursor(chat_id, Feed::Reactions, reaction_seq, true);
            });
            None
        }
        ServerFrame::Poll {
            chat_id,
            message_id,
            poll,
        } => {
            live.update(session, |state| {
                let seq = poll.poll_seq;
                state.store.apply_poll(chat_id, message_id, poll);
                state.store.advance_cursor(chat_id, Feed::Polls, seq, true);
            });
            None
        }
        ServerFrame::AiDelta {
            chat_id,
            message_id,
            text,
        } => {
            live.update(session, |state| {
                state.store.apply_ai_delta(chat_id, message_id, &text)
            });
            None
        }
        ServerFrame::AiError { message_id, .. } => {
            live.update(session, |state| state.store.apply_ai_error(message_id));
            None
        }
        ServerFrame::MemberJoined { user } => {
            live.update(session, |state| state.store.member_joined(&user));
            None
        }
        ServerFrame::MemberLeft { user_id } => {
            live.update(session, |state| state.store.member_left(user_id));
            None
        }
        ServerFrame::MemberDeleted { member } => {
            live.update(session, |state| state.store.member_deleted(&member));
            // Their direct chat goes with them, both halves.
            refresh();
            None
        }
        ServerFrame::FamilyOwner { user_id } => {
            live.update(session, |state| state.store.family_owner(user_id));
            None
        }
        ServerFrame::MemberBlocked { user_id, blocked } => {
            live.update(session, |state| state.store.set_blocked(user_id, blocked));
            // An unblock brings the direct chat back whole — from the list.
            if !blocked {
                refresh();
            }
            None
        }
        // Everything this client has not learned yet, `pong` included —
        // stepped over, which is the protocol's compatibility rule.
        ServerFrame::Unknown => None,
    }
}

/// "X is typing…" goes when its frame is TYPING_TTL_MS old: a typing frame
/// says "still", never "stopped".
async fn sweep_typing(live: Live, session: u64) {
    while live.is_live(session) {
        gloo_timers::future::TimeoutFuture::new(TYPING_SWEEP_MS).await;
        let now = now_ms();
        // Only when something actually went, or every tick would draw the
        // whole app again for nothing.
        let stale = live.read(|state| {
            state
                .store
                .typing
                .values()
                .flat_map(|typing| typing.values())
                .any(|stamp| now - stamp >= store::TYPING_TTL_MS)
        });
        if stale {
            live.update(session, |state| state.store.prune_typing(now));
        }
    }
}

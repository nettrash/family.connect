//! Family Connect in a browser.
//!
//! A web client of the protocol in docs/protocol.md — see "A browser is a
//! client too" for what a browser does differently: it is served from the
//! same origin as the API, registers no device and takes no push, keeps its
//! session token in `sessionStorage` so closing the tab is a sign-out, and
//! SENDS over REST while only listening on the socket.
//!
//! This first slice is the core loop: sign in, the chat list with unread
//! counts, the family chat and direct chats with history paging, sending
//! through an outbox that survives a bad network, and the live socket for
//! messages, reads and typing. Attachments, the board, polls, the assistant
//! and calls are not here — and are not stubbed either, because a stub is a
//! claim that something works.

mod api;
mod live;
mod model;
mod outbox;
mod session;
mod socket;
mod store;
mod views;

#[cfg(test)]
mod layout_tests;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::{FutureExt, SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message as WsMessage, State};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use api::ApiError;
use live::{AppState, Live};
use model::{ChatListItem, Message};
use outbox::Wake;
use socket::{ClientFrame, ServerFrame};
use views::chat_list::ChatList;
use views::conversation::Conversation;
use views::login::Login;

/// How many messages a page asks for. The protocol's default is 50 and its
/// ceiling 200; 50 is a screenful and a bit on any window.
const PAGE: u32 = 50;

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
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_default()
}

/// Whether anybody can see the page. A message landing in the open chat of
/// a tab nobody is looking at has not been read.
fn page_visible() -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .map(|document| !document.hidden())
        .unwrap_or(true)
}

/// The chat being READ: the open one, while the page can be seen.
fn reading(state: &AppState) -> Option<i64> {
    state.open_chat.filter(|_| page_visible())
}

/// How the app reaches the session's long-lived tasks. Closed when the
/// session ends, which stops every one of them at once rather than at its
/// next tick.
struct Channels {
    /// Frames for the socket.
    frames: mpsc::UnboundedSender<ClientFrame>,
    /// "Look at the outbox again."
    wake: mpsc::UnboundedSender<Wake>,
    /// "Dial now": the network is back, stop waiting out the backoff.
    redial: mpsc::UnboundedSender<()>,
}

impl Channels {
    fn close(&self) {
        self.frames.close_channel();
        self.wake.close_channel();
        self.redial.close_channel();
    }
}

type Shared<T> = Rc<RefCell<T>>;

fn wake(channels: &Shared<Option<Channels>>, why: Wake) {
    if let Some(open) = channels.borrow().as_ref() {
        let _ = open.wake.unbounded_send(why);
    }
}

/// The network may be back: send what is stuck, and dial NOW if the socket
/// is down rather than at the end of its backoff (docs/protocol.md, "A
/// returning network is a trigger").
fn network_back(live: &Live, channels: &Shared<Option<Channels>>) {
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
fn expiry(live: &Live, session: u64, sign_out: &Callback<bool>) -> Rc<dyn Fn()> {
    let live = live.clone();
    let sign_out = sign_out.clone();
    Rc::new(move || {
        if live.is_live(session) {
            sign_out.emit(false);
        }
    })
}

/// What one render draws, read out of the state in one go.
struct Screen {
    chats: Vec<ChatListItem>,
    names: HashMap<i64, String>,
    open_chat: Option<i64>,
    messages: Vec<Message>,
    more_above: bool,
    my_user_id: i64,
    typing: Vec<String>,
    failed: HashMap<String, String>,
    connected: bool,
    failure: Option<String>,
}

impl Screen {
    fn of(state: &AppState) -> Screen {
        let thread = state
            .open_chat
            .and_then(|chat_id| state.store.threads.get(&chat_id));
        Screen {
            chats: state.store.chats.clone(),
            names: state.store.names.clone(),
            open_chat: state.open_chat,
            messages: thread
                .map(|thread| thread.messages.clone())
                .unwrap_or_default(),
            more_above: thread.is_some_and(|thread| thread.more_above),
            my_user_id: state.store.my_user_id,
            typing: state
                .open_chat
                .map(|chat_id| state.store.typing_names(chat_id, now_ms()))
                .unwrap_or_default(),
            failed: state
                .open_chat
                .map(|chat_id| state.store.failed_sends(chat_id))
                .unwrap_or_default(),
            connected: state.connected,
            failure: state.failure.clone(),
        }
    }
}

#[function_component(App)]
fn app() -> Html {
    let redraw = use_force_update();
    let live = (*use_memo((), move |_| {
        Live::new(
            AppState {
                token: session::token(),
                ..AppState::default()
            },
            Rc::new(move || redraw.force_update()),
        )
    }))
    .clone();
    let channels: Shared<Option<Channels>> = use_mut_ref(|| None);
    let last_typing = use_mut_ref(HashMap::<i64, f64>::new);

    // Signing out, from anywhere: a 401 and the button do the same thing.
    let sign_out = {
        let live = live.clone();
        Callback::from(move |revoke: bool| {
            if revoke {
                if let Some(held) = live.read(|state| state.token.clone()) {
                    spawn_local(async move { api::logout(&held).await });
                }
            }
            session::clear();
            live.end_session();
        })
    };

    let on_signed_in = {
        let live = live.clone();
        Callback::from(move |fresh: String| {
            session::set_token(&fresh);
            live.now(|state| state.token = Some(fresh));
        })
    };

    // Everything a session runs for as long as it is the session. Re-run
    // when the token changes, which is exactly sign-in and sign-out; the
    // teardown closes the channels, which stops every task at once.
    let token = live.read(|state| state.token.clone());
    {
        let live = live.clone();
        let channels = channels.clone();
        let sign_out = sign_out.clone();
        use_effect_with(token.clone(), move |held| {
            if let Some(held) = held.clone() {
                start_session(&live, held, &channels, &sign_out);
            }
            move || {
                if let Some(open) = channels.borrow_mut().take() {
                    open.close();
                }
            }
        });
    }

    // A network that comes back and a tab that comes back are both the
    // moment to send what is stuck and to dial now. A tab that comes back
    // is also somebody looking at the open chat again, which reads it.
    {
        let live = live.clone();
        let channels = channels.clone();
        use_effect_with((), move |_| {
            let back = {
                let live = live.clone();
                let channels = channels.clone();
                Closure::<dyn Fn()>::new(move || network_back(&live, &channels))
            };
            let shown = Closure::<dyn Fn()>::new(move || {
                if !page_visible() {
                    return;
                }
                network_back(&live, &channels);
                let (session, token) = live.read(|state| (state.session, state.token.clone()));
                if let Some(token) = token {
                    let live = live.clone();
                    spawn_local(async move { report_read(&live, session, &token).await });
                }
            });
            let window = web_sys::window().expect("a window");
            let document = window.document().expect("a document");
            let _ =
                window.add_event_listener_with_callback("online", back.as_ref().unchecked_ref());
            let _ = document.add_event_listener_with_callback(
                "visibilitychange",
                shown.as_ref().unchecked_ref(),
            );
            move || {
                let _ = window
                    .remove_event_listener_with_callback("online", back.as_ref().unchecked_ref());
                let _ = document.remove_event_listener_with_callback(
                    "visibilitychange",
                    shown.as_ref().unchecked_ref(),
                );
            }
        });
    }

    // Opening a chat: fetch its newest page, then report it read.
    let select_chat = {
        let live = live.clone();
        let sign_out = sign_out.clone();
        Callback::from(move |chat_id: i64| {
            let (session, token) = live.now(|state| {
                state.open_chat = Some(chat_id);
                (state.session, state.token.clone())
            });
            let Some(token) = token else { return };
            let live = live.clone();
            let expired = expiry(&live, session, &sign_out);
            spawn_local(async move {
                match api::messages(&token, chat_id, None, PAGE).await {
                    Ok(page) => {
                        let full = page.len() as u32 == PAGE;
                        live.update(session, |state| {
                            let thread = state.store.threads.entry(chat_id).or_default();
                            for message in page {
                                thread.apply(message);
                            }
                            thread.more_above = full;
                            state.failure = None;
                        });
                        report_read(&live, session, &token).await;
                    }
                    Err(ApiError::Unauthorized) => expired(),
                    Err(error) => {
                        live.update(session, |state| state.failure = Some(error.detail()));
                    }
                }
            });
        })
    };

    let load_more = {
        let live = live.clone();
        Callback::from(move |_: ()| {
            let Some((session, token, chat_id, before)) = live.read(|state| {
                let chat_id = state.open_chat?;
                let before = state.store.threads.get(&chat_id)?.oldest?;
                Some((state.session, state.token.clone()?, chat_id, before))
            }) else {
                return;
            };
            let live = live.clone();
            spawn_local(async move {
                if let Ok(page) = api::messages(&token, chat_id, Some(before), PAGE).await {
                    let full = page.len() as u32 == PAGE;
                    live.update(session, |state| {
                        let thread = state.store.threads.entry(chat_id).or_default();
                        for message in page {
                            thread.apply(message);
                        }
                        thread.more_above = full;
                    });
                }
            });
        })
    };

    // The throttle: at most one frame every TYPING_THROTTLE_MS per chat,
    // the same 4 s the phone clients use. A frame per keystroke would be a
    // frame per keystroke times every member of the family.
    let typing = {
        let live = live.clone();
        let channels = channels.clone();
        Callback::from(move |_: ()| {
            // Momentary: said only while the socket is up, never saved up.
            let Some(chat_id) = live.read(|state| state.open_chat.filter(|_| state.connected))
            else {
                return;
            };
            let now = now_ms();
            let mut last = last_typing.borrow_mut();
            if last
                .get(&chat_id)
                .is_some_and(|sent| now - sent < store::TYPING_THROTTLE_MS)
            {
                return;
            }
            last.insert(chat_id, now);
            if let Some(open) = channels.borrow().as_ref() {
                let _ = open.frames.unbounded_send(ClientFrame::Typing { chat_id });
            }
        })
    };

    // Send: the bubble draws now and the row joins the outbox, whose sender
    // takes it from there (outbox.rs). Nothing here waits for a socket.
    let send = {
        let live = live.clone();
        let channels = channels.clone();
        Callback::from(move |body: String| {
            let queued = live.now(|state| {
                let chat_id = state.open_chat?;
                state
                    .store
                    .queue_send(chat_id, uuid::Uuid::new_v4().to_string(), body);
                Some(())
            });
            if queued.is_some() {
                wake(&channels, Wake::Queued);
            }
        })
    };

    // A person asking again is as good a sign as any that it might work
    // now, so it cuts short whatever backoff the sender is sitting in.
    let retry = {
        let live = live.clone();
        let channels = channels.clone();
        Callback::from(move |client_msg_id: String| {
            live.now(|state| state.store.retry(&client_msg_id));
            wake(&channels, Wake::Network);
        })
    };

    let discard = {
        let live = live.clone();
        Callback::from(move |client_msg_id: String| {
            live.now(|state| state.store.discard(&client_msg_id));
        })
    };

    if token.is_none() {
        return html! { <Login on_signed_in={on_signed_in} /> };
    }

    let screen = live.read(Screen::of);
    let sign_out_click = {
        let sign_out = sign_out.clone();
        Callback::from(move |_| sign_out.emit(true))
    };

    html! {
        <div class="app">
            <header class="bar">
                <span class="brand">{ "Family Connect" }</span>
                // Live updates are paused, and the bar says so. Sending is
                // not: that goes over REST whether the socket is up or not.
                if !screen.connected {
                    <span class="status" role="status">{ "Connecting…" }</span>
                }
                <button class="signout" onclick={sign_out_click}>{ "Sign out" }</button>
            </header>
            if let Some(message) = screen.failure.clone() {
                <p class="error" role="alert">{ message }</p>
            }
            <div class="split">
                <ChatList
                    chats={screen.chats}
                    names={screen.names.clone()}
                    selected={screen.open_chat}
                    on_select={select_chat}
                />
                if let Some(chat_id) = screen.open_chat {
                    // Keyed by chat, so switching chats is a fresh pane:
                    // its scroll position and "pinned to the newest"
                    // state belong to the chat they were for.
                    <Conversation
                        key={chat_id.to_string()}
                        messages={screen.messages}
                        my_user_id={screen.my_user_id}
                        names={screen.names}
                        typing={screen.typing}
                        sending_disabled={false}
                        on_send={send}
                        on_typing={typing}
                        can_load_more={screen.more_above}
                        on_load_more={load_more}
                        failed={screen.failed}
                        on_retry={retry}
                        on_discard={discard}
                    />
                } else {
                    <section class="conversation empty">
                        <p>{ "Pick a chat." }</p>
                    </section>
                }
            </div>
        </div>
    }
}

/// Start everything a signed-in session runs: the first loads, the socket,
/// the outbox and the typing sweep. Each stops when the session ends.
fn start_session(
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

    spawn_local(load_session(
        live.clone(),
        session,
        token.clone(),
        expired.clone(),
    ));
    spawn_local(keep_socket(
        live.clone(),
        session,
        token.clone(),
        frames_out,
        redial_in,
        wake,
        expired.clone(),
    ));
    spawn_local(outbox::drain(
        live.clone(),
        session,
        wake_in,
        move |row| {
            let token = token.clone();
            async move { api::send_message(&token, row.chat_id, &row.client_msg_id, &row.body).await }
        },
        |attempts| {
            let ceiling = outbox::backoff_ceiling_ms(attempts);
            gloo_timers::future::TimeoutFuture::new(outbox::jittered_ms(
                ceiling,
                js_sys::Math::random(),
            ))
        },
        move || expired(),
    ));
    spawn_local(sweep_typing(live.clone(), session));
}

/// Who this is, everybody's name, and the chat list — the first things a
/// signed-in app needs.
async fn load_session(live: Live, session: u64, token: String, expired: Rc<dyn Fn()>) {
    match api::me(&token).await {
        Ok(me) => {
            live.update(session, |state| {
                state.store.my_user_id = me.user.id;
                state.store.names.insert(me.user.id, me.user.display_name);
            });
        }
        Err(ApiError::Unauthorized) => {
            expired();
            return;
        }
        Err(error) => {
            live.update(session, |state| state.failure = Some(error.detail()));
        }
    }
    // Every sender and every direct chat is drawn by a name out of this.
    // An account in no family has nobody else to name, which is a real
    // state rather than a failure worth showing.
    if let Ok(roster) = api::family(&token).await {
        live.update(session, |state| state.store.names.extend(roster.names()));
    }
    refresh_chats(&live, session, &token, &expired).await;
}

/// The chat list, as the server has it now — the unread counts included,
/// which is why it is fetched AFTER a catch-up and never before one: the
/// catch-up counts what it applies, and the server's count already has it.
async fn refresh_chats(live: &Live, session: u64, token: &str, expired: &Rc<dyn Fn()>) {
    match api::chats(token).await {
        Ok(chats) => {
            live.update(session, |state| {
                state.store.chats = chats;
                state.failure = None;
            });
        }
        Err(ApiError::Unauthorized) => expired(),
        Err(error) => {
            live.update(session, |state| state.failure = Some(error.detail()));
        }
    }
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
    // Frames arriving meanwhile wait in the socket's own queue.
    opened(live, session, token, wake, expired).await;

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
    live.update(session, |state| state.connected = false);
    true
}

/// The socket is open. What a (re)connect owes, and the outbox FIRST —
/// "Step 4 is not a step" (docs/protocol.md): whatever is stuck goes out
/// before any read that might fail on the same network. Then what was
/// missed, then the chat list with the server's counts, then the read.
async fn opened(
    live: &Live,
    session: u64,
    token: &str,
    wake: &mpsc::UnboundedSender<Wake>,
    expired: &Rc<dyn Fn()>,
) {
    live.update(session, |state| state.connected = true);
    let _ = wake.unbounded_send(Wake::Network);
    catch_up(live, session, token).await;
    refresh_chats(live, session, token, expired).await;
    report_read(live, session, token).await;
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
    match frame {
        ServerFrame::Message { message } => {
            let chat_id = message.chat_id;
            let (answer, unknown_chat) = live.update(session, |state| {
                let reading = reading(state);
                let id = message.id;
                let theirs = message.sender_id != state.store.my_user_id;
                let unknown_chat = !state.store.chats.iter().any(|item| item.chat.id == chat_id);
                state.store.apply_message(message, reading);
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
                let live = live.clone();
                let token = token.to_string();
                let expired = expired.clone();
                spawn_local(async move { refresh_chats(&live, session, &token, &expired).await });
            }
            answer
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
        // Everything this client has not learned yet, `pong` included —
        // stepped over, which is the protocol's compatibility rule.
        ServerFrame::Unknown => None,
    }
}

/// What arrived while the socket was down, for every chat this client holds
/// messages of.
///
/// `after_id` from the newest message HELD — not from the chat's own
/// cursor, because the hole is exactly between the two. Looped until a
/// short page, which is how the protocol says to read it. A chat never
/// opened holds nothing to have a hole in; it is fetched when it is opened.
async fn catch_up(live: &Live, session: u64, token: &str) {
    let held: Vec<(i64, i64)> = live.read(|state| {
        state
            .store
            .threads
            .iter()
            .filter_map(|(chat_id, thread)| {
                thread.newest_server_id().map(|newest| (*chat_id, newest))
            })
            .collect()
    });
    for (chat_id, mut after) in held {
        loop {
            let Ok(page) = api::messages_after(token, chat_id, after, PAGE).await else {
                break;
            };
            if page.is_empty() {
                break;
            }
            let short = (page.len() as u32) < PAGE;
            after = page.iter().map(|message| message.id).fold(after, i64::max);
            let applied = live.update(session, |state| {
                let reading = reading(state);
                for message in page {
                    state.store.apply_message(message, reading);
                }
            });
            if applied.is_none() {
                return;
            }
            if short {
                break;
            }
        }
    }
}

/// The chat being read is read up to its newest message: the badge goes,
/// and the server hears it over REST — the one path that works whether or
/// not the socket is up. Only while somebody can see it, and only when
/// there is something new to say.
async fn report_read(live: &Live, session: u64, token: &str) {
    let Some((chat_id, newest)) = live.read(|state| {
        let chat_id = reading(state)?;
        let newest = state.store.threads.get(&chat_id)?.newest_server_id()?;
        let item = state
            .store
            .chats
            .iter()
            .find(|item| item.chat.id == chat_id)?;
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

// These tests run in a BROWSER, not in node: everything under test here
// reaches for `window` — storage, the location, the performance clock —
// and node has none of them. `wasm-pack test --headless --chrome`.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    yew::Renderer::<App>::new().render();
}

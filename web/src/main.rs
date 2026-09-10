//! Family Connect in a browser.
//!
//! A web client of the protocol in docs/protocol.md — see "A browser is a
//! client too" for what a browser does differently: it is served from the
//! same origin as the API, registers no device and takes no push, and keeps
//! its session token in `sessionStorage` so closing the tab is a sign-out.
//!
//! This first slice is the core loop: sign in, the chat list with unread
//! counts, the family chat and direct chats with history paging, sending,
//! and the live socket for messages, acks, reads and typing. Attachments,
//! the board, polls, the assistant and calls are not here — and are not
//! stubbed either, because a stub is a claim that something works.

mod api;
mod model;
mod session;
mod socket;
mod store;
mod views;

use std::collections::HashMap;
use std::rc::Rc;

use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message as WsMessage};
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use store::Store;
use views::chat_list::ChatList;
use views::conversation::Conversation;
use views::login::Login;

/// How many messages a page asks for. The protocol's default is 50 and its
/// ceiling 200; 50 is a screenful and a bit on any window.
const PAGE: u32 = 50;

/// The heartbeat. The protocol has a `ping` frame; a browser tab that is
/// backgrounded for a while otherwise learns its socket is gone only when
/// it next tries to say something, which is exactly when it matters most.
const PING_MS: u32 = 30_000;

/// How long to wait before dialling again after the socket drops. Fixed
/// rather than backed off, because a browser tab has one socket and a
/// person is usually looking at it: five seconds is short enough to feel
/// like nothing happened and long enough not to hammer a server that is
/// restarting.
const RECONNECT_MS: u32 = 5_000;

/// Milliseconds since the page loaded. The clock everything time-based
/// here reads — monotonic, unlike the wall clock, so a machine waking from
/// sleep with a corrected clock cannot make a typing indicator immortal.
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map(|performance| performance.now())
        .unwrap_or_default()
}

#[function_component(App)]
fn app() -> Html {
    let token = use_state(session::token);
    let store = use_state(Store::default);
    let open_chat = use_state(|| Option::<i64>::None);
    let failure = use_state(|| Option::<String>::None);
    // The sink the socket loop reads from, so a view can send a frame
    // without holding the socket itself.
    let outbox = use_state(|| Option::<mpsc::UnboundedSender<socket::ClientFrame>>::None);

    // Signing out, from anywhere: a 401 and the button do the same thing.
    let sign_out = {
        let token = token.clone();
        let store = store.clone();
        let open_chat = open_chat.clone();
        let outbox = outbox.clone();
        Callback::from(move |revoke: bool| {
            if revoke {
                if let Some(held) = (*token).clone() {
                    spawn_local(async move { api::logout(&held).await });
                }
            }
            session::clear();
            outbox.set(None);
            open_chat.set(None);
            store.set(Store::default());
            token.set(None);
        })
    };

    let on_signed_in = {
        let token = token.clone();
        Callback::from(move |fresh: String| {
            session::set_token(&fresh);
            token.set(Some(fresh));
        })
    };

    // Everything the app needs the moment it has a token: who this is, the
    // chat list, and the socket. Re-run when the token changes, which is
    // exactly sign-in and sign-out.
    {
        let token = token.clone();
        let store = store.clone();
        let outbox = outbox.clone();
        let failure = failure.clone();
        let sign_out = sign_out.clone();
        let open_chat = open_chat.clone();
        use_effect_with((*token).clone(), move |held| {
            let Some(held) = held.clone() else {
                return;
            };
            {
                let store = store.clone();
                let failure = failure.clone();
                let sign_out = sign_out.clone();
                let held = held.clone();
                spawn_local(async move {
                    match api::me(&held).await {
                        Ok(me) => {
                            let mut next = (*store).clone();
                            next.my_user_id = me.user.id;
                            next.names.insert(me.user.id, me.user.display_name.clone());
                            store.set(next);
                        }
                        Err(api::ApiError::Unauthorized) => sign_out.emit(false),
                        Err(error) => failure.set(Some(error.detail())),
                    }
                    match api::chats(&held).await {
                        Ok(chats) => {
                            let mut next = (*store).clone();
                            next.chats = chats;
                            store.set(next);
                        }
                        Err(api::ApiError::Unauthorized) => sign_out.emit(false),
                        Err(error) => failure.set(Some(error.detail())),
                    }
                });
            }
            // The socket, and a channel for anything that wants to send on
            // it. One task owns the socket; everybody else owns a sender.
            //
            // The channel OUTLIVES any one connection, which is the whole
            // point: a send that happens while the socket is down is queued
            // rather than lost, and the next connection drains it.
            let (sender, receiver) = mpsc::unbounded::<socket::ClientFrame>();
            outbox.set(Some(sender.clone()));
            {
                let store = store.clone();
                let open_chat = open_chat.clone();
                let held = held.clone();
                spawn_local(async move {
                    // The receiver is owned HERE and lent to each
                    // connection in turn, so a frame queued while the
                    // socket is down survives to be sent by the next one.
                    let mut receiver = receiver;
                    // Dial, run, and dial again. A browser tab lives for
                    // days: a socket that dropped once and stayed dropped
                    // is a chat that silently stops arriving, which is
                    // worse than one that never connected at all.
                    loop {
                        let url = socket::url_for(&session::origin(), &held);
                        if let Ok(ws) = WebSocket::open(&url) {
                            // THE CATCH-UP, before anything else: the
                            // frames missed while the socket was down
                            // are fetched with `after_id`, which is
                            // what the protocol provides them for. A
                            // reconnect without it is a hole in the
                            // conversation nothing later fills.
                            catch_up(&held, &store, *open_chat).await;
                            run_socket(ws, &mut receiver, &store, &open_chat).await;
                        }
                        // Idle a moment, then dial again.
                        gloo_timers::future::TimeoutFuture::new(RECONNECT_MS).await;
                    }
                });
            }
            // The heartbeat, for as long as this token is the token.
            {
                let sender = sender.clone();
                spawn_local(async move {
                    loop {
                        gloo_timers::future::TimeoutFuture::new(PING_MS).await;
                        if sender.unbounded_send(socket::ClientFrame::Ping).is_err() {
                            return;
                        }
                    }
                });
            }
            // And the pruner, so "X is typing…" cannot get stuck: a typing
            // frame says "still", never "stopped".
            {
                let store = store.clone();
                spawn_local(async move {
                    loop {
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        let mut next = (*store).clone();
                        let before = next.typing.clone();
                        next.prune_typing(now_ms());
                        // Only when something actually went, or every tick
                        // would re-render the whole app for nothing.
                        if next.typing != before {
                            store.set(next);
                        }
                    }
                });
            }
        });
    }

    // Opening a chat: fetch its newest page, then report it read.
    let select_chat = {
        let token = token.clone();
        let store = store.clone();
        let open_chat = open_chat.clone();
        let outbox = outbox.clone();
        let sign_out = sign_out.clone();
        Callback::from(move |chat_id: i64| {
            open_chat.set(Some(chat_id));
            let Some(held) = (*token).clone() else {
                return;
            };
            let store = store.clone();
            let outbox = outbox.clone();
            let sign_out = sign_out.clone();
            spawn_local(async move {
                match api::messages(&held, chat_id, None, PAGE).await {
                    Ok(page) => {
                        let full = page.len() as u32 == PAGE;
                        let mut next = (*store).clone();
                        let thread = next.threads.entry(chat_id).or_default();
                        for message in page {
                            thread.apply(message);
                        }
                        thread.more_above = full;
                        let newest = thread.newest_server_id();
                        if let Some(newest) = newest {
                            next.mark_read(chat_id, newest);
                        }
                        store.set(next);
                        views::conversation::scroll_to_newest();
                        if let Some(newest) = newest {
                            // Both legs: the frame is the fast one, and the
                            // POST is what survives a socket that is not up.
                            if let Some(sink) = (*outbox).clone() {
                                let _ = sink.unbounded_send(socket::ClientFrame::Read {
                                    chat_id,
                                    last_read_message_id: newest,
                                });
                            }
                            let _ = api::post_read(&held, chat_id, newest).await;
                        }
                    }
                    Err(api::ApiError::Unauthorized) => sign_out.emit(false),
                    Err(_) => {}
                }
            });
        })
    };

    let load_more = {
        let token = token.clone();
        let store = store.clone();
        let open_chat = open_chat.clone();
        Callback::from(move |_: ()| {
            let (Some(held), Some(chat_id)) = ((*token).clone(), *open_chat) else {
                return;
            };
            let Some(before) = store.threads.get(&chat_id).and_then(|thread| thread.oldest) else {
                return;
            };
            let store = store.clone();
            spawn_local(async move {
                if let Ok(page) = api::messages(&held, chat_id, Some(before), PAGE).await {
                    let full = page.len() as u32 == PAGE;
                    let mut next = (*store).clone();
                    let thread = next.threads.entry(chat_id).or_default();
                    for message in page {
                        thread.apply(message);
                    }
                    thread.more_above = full;
                    store.set(next);
                }
            });
        })
    };

    // The throttle: at most one frame every TYPING_THROTTLE_MS per chat,
    // the same 4 s the phone clients use. A frame per keystroke would be a
    // frame per keystroke times every member of the family.
    let last_typing = use_mut_ref(HashMap::<i64, f64>::new);
    let typing = {
        let open_chat = open_chat.clone();
        let outbox = outbox.clone();
        let last_typing = last_typing.clone();
        Callback::from(move |_: ()| {
            let (Some(chat_id), Some(sink)) = (*open_chat, (*outbox).clone()) else {
                return;
            };
            let now = now_ms();
            let mut last = last_typing.borrow_mut();
            if let Some(sent) = last.get(&chat_id) {
                if now - *sent < store::TYPING_THROTTLE_MS {
                    return;
                }
            }
            last.insert(chat_id, now);
            let _ = sink.unbounded_send(socket::ClientFrame::Typing { chat_id });
        })
    };

    let send = {
        let token = token.clone();
        let store = store.clone();
        let open_chat = open_chat.clone();
        let outbox = outbox.clone();
        Callback::from(move |body: String| {
            let (Some(held), Some(chat_id)) = ((*token).clone(), *open_chat) else {
                return;
            };
            let client_msg_id = uuid::Uuid::new_v4().to_string();
            // The bubble draws NOW, under id 0, and the ack turns it into
            // the real one (store::Thread::apply).
            let mut next = (*store).clone();
            next.apply_message(
                model::Message {
                    id: 0,
                    chat_id,
                    sender_id: next.my_user_id,
                    client_msg_id: Some(client_msg_id.clone()),
                    body: body.clone(),
                    created_at: String::new(),
                    edited_at: None,
                },
                Some(chat_id),
            );
            store.set(next);
            views::conversation::scroll_to_newest();

            // The socket when it is up, REST when it is not — the same
            // client_msg_id either way, which is what makes a retry
            // idempotent rather than a duplicate (docs/protocol.md).
            if let Some(sink) = (*outbox).clone() {
                if sink
                    .unbounded_send(socket::ClientFrame::Send {
                        chat_id,
                        client_msg_id: client_msg_id.clone(),
                        body: body.clone(),
                    })
                    .is_ok()
                {
                    return;
                }
            }
            let store = store.clone();
            spawn_local(async move {
                if let Ok(message) = api::send_message(&held, chat_id, &client_msg_id, &body).await
                {
                    let mut next = (*store).clone();
                    next.apply_message(message, Some(chat_id));
                    store.set(next);
                }
            });
        })
    };

    let Some(_held) = (*token).clone() else {
        return html! { <Login on_signed_in={on_signed_in} /> };
    };

    let current = *open_chat;
    let thread = current
        .and_then(|chat_id| store.threads.get(&chat_id).cloned())
        .unwrap_or_default();
    let names: HashMap<i64, String> = store.names.clone();
    let sign_out_click = {
        let sign_out = sign_out.clone();
        Callback::from(move |_| sign_out.emit(true))
    };

    html! {
        <div class="app">
            <header class="bar">
                <span class="brand">{ "Family Connect" }</span>
                <button class="signout" onclick={sign_out_click}>{ "Sign out" }</button>
            </header>
            if let Some(message) = (*failure).clone() {
                <p class="error" role="alert">{ message }</p>
            }
            <div class="split">
                <ChatList
                    chats={store.chats.clone()}
                    selected={current}
                    on_select={select_chat}
                />
                if current.is_some() {
                    <Conversation
                        messages={thread.messages.clone()}
                        my_user_id={store.my_user_id}
                        names={Rc::new(names).as_ref().clone()}
                        typing={current.map(|id| store.typing_names(id, now_ms())).unwrap_or_default()}
                        sending_disabled={false}
                        on_send={send}
                        on_typing={typing}
                        can_load_more={thread.more_above}
                        on_load_more={load_more}
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

/// Fetch what arrived while the socket was down, for the chat in front of
/// the reader.
///
/// `after_id` from the newest message this client HOLDS — not from the
/// chat's own cursor, because the hole is exactly between the two. Looped
/// until a short page, which is how the protocol says to read it.
async fn catch_up(token: &str, store: &UseStateHandle<Store>, open_chat: Option<i64>) {
    let Some(chat_id) = open_chat else { return };
    let Some(mut after) = store
        .threads
        .get(&chat_id)
        .and_then(|thread| thread.newest_server_id())
    else {
        return;
    };
    loop {
        let Ok(page) = api::messages_after(token, chat_id, after, PAGE).await else {
            return;
        };
        let short = (page.len() as u32) < PAGE;
        if page.is_empty() {
            return;
        }
        let mut next = (**store).clone();
        for message in page {
            after = after.max(message.id);
            next.apply_message(message, Some(chat_id));
        }
        store.set(next);
        views::conversation::scroll_to_newest();
        if short {
            return;
        }
    }
}

/// One connection, until it closes.
///
/// ONE loop over both directions rather than a reader task and a writer
/// task: two tasks would have to share the socket halves through a
/// `RefCell` held across an await, which is a borrow panic waiting for the
/// first time anything re-enters. `select!` needs neither.
async fn run_socket(
    ws: WebSocket,
    outbox: &mut mpsc::UnboundedReceiver<socket::ClientFrame>,
    store: &UseStateHandle<Store>,
    open_chat: &UseStateHandle<Option<i64>>,
) {
    let (mut write, read) = ws.split();
    let mut read = read.fuse();
    loop {
        futures::select! {
            incoming = read.next() => {
                let Some(Ok(WsMessage::Text(text))) = incoming else {
                    // Closed, or something this client cannot read at all.
                    return;
                };
                let Some(frame) = socket::decode(&text) else { continue };
                let mut next = (**store).clone();
                match frame {
                    socket::ServerFrame::Message { message }
                    | socket::ServerFrame::Ack { message, .. } => {
                        next.apply_message(message, **open_chat);
                    }
                    socket::ServerFrame::Read { chat_id, user_id, last_read_message_id } => {
                        next.apply_read_frame(chat_id, user_id, last_read_message_id)
                    }
                    socket::ServerFrame::Typing { chat_id, user_id } => {
                        next.set_typing(chat_id, user_id, now_ms())
                    }
                    // Everything this client has not learned yet — stepped
                    // over, which is the protocol's compatibility rule.
                    socket::ServerFrame::Unknown => continue,
                }
                store.set(next);
                views::conversation::scroll_to_newest();
            }
            outgoing = outbox.next() => {
                let Some(frame) = outgoing else { return };
                let Ok(text) = serde_json::to_string(&frame) else { continue };
                if write.send(WsMessage::Text(text)).await.is_err() {
                    return;
                }
            }
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

//! Family Connect in a browser.
//!
//! A web client of the protocol in docs/protocol.md — see "A browser is a
//! client too" for what a browser does differently: it is served from the
//! same origin as the API, registers no device and takes no push, keeps its
//! session token in `sessionStorage` so closing the tab is a sign-out, and
//! SENDS over REST while only listening on the socket.
//!
//! It is being brought to the macOS app's feature set phase by phase. This
//! one is the conversation itself: formatting, links and large emoji,
//! replies with their quotes, threads, reactions, mentions, the assistant
//! with its streamed answers and pictures, polls and the open-polls list,
//! edits, seen ticks in direct chats, the unread divider, reports and
//! blocks — on top of sign-in, the chat list, history and an outbox that
//! survives a bad network. What is not built yet is not stubbed either,
//! because a stub is a claim that something works.

mod actions;
mod api;
mod live;
mod model;
mod outbox;
mod session;
mod socket;
mod store;
mod sync;
mod time;
mod timeline;
mod views;

#[cfg(test)]
mod layout_tests;

use std::collections::HashMap;
use std::rc::Rc;

use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use actions::{Action, Actions};
use live::{AppState, Live};
use store::ThreadView;
use sync::{network_back, page_visible, report_read, start_session, Channels, Shared};
use views::chat_list::ChatList;
use views::conversation::Conversation;
use views::login::Login;
use views::open_polls::OpenPollsPanel;
use views::thread_panel::ThreadPanel;

#[function_component(App)]
fn app() -> Html {
    let redraw = use_force_update();
    let live = (*use_memo((), move |_| {
        let mut state = AppState {
            token: session::token(),
            ..AppState::default()
        };
        // A reload in the middle of sending: the rows go back in the outbox,
        // bubbles and all, and the sender takes them from there.
        if state.token.is_some() {
            if let Some((user_id, rows)) = session::outbox() {
                state.store.my_user_id = user_id;
                state.store.restore_outbox(rows);
            }
        }
        Live::new(state, Rc::new(move || redraw.force_update()))
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

    // The outbox, kept for a reload whenever it changes.
    {
        let (user_id, outbox) =
            live.read(|state| (state.store.my_user_id, state.store.outbox.clone()));
        let signed_in = token_is_set(&live);
        use_effect_with((user_id, outbox), move |(user_id, outbox)| {
            if signed_in {
                session::save_outbox(*user_id, outbox);
            }
        });
    }

    // Closing the tab with something still unsent loses it — the session,
    // and its outbox, go with the tab — so the browser asks first. A reload
    // asks too (a page cannot tell the two apart), and loses nothing.
    {
        let live = live.clone();
        use_effect_with((), move |_| {
            let guard = Closure::<dyn Fn(web_sys::BeforeUnloadEvent)>::new(
                move |event: web_sys::BeforeUnloadEvent| {
                    if live.read(|state| !state.store.outbox.is_empty()) {
                        event.prevent_default();
                        event.set_return_value("unsent");
                    }
                },
            );
            let window = web_sys::window().expect("a window");
            let _ = window
                .add_event_listener_with_callback("beforeunload", guard.as_ref().unchecked_ref());
            move || {
                let _ = window.remove_event_listener_with_callback(
                    "beforeunload",
                    guard.as_ref().unchecked_ref(),
                );
            }
        });
    }

    // A network that comes back and a tab that comes back are both the
    // moment to send what is stuck and to dial now. A tab that comes back —
    // or a window brought to the front — is also somebody looking at the
    // open chat again, which reads it.
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
            let _ =
                window.add_event_listener_with_callback("focus", shown.as_ref().unchecked_ref());
            move || {
                let _ = window
                    .remove_event_listener_with_callback("online", back.as_ref().unchecked_ref());
                let _ = document.remove_event_listener_with_callback(
                    "visibilitychange",
                    shown.as_ref().unchecked_ref(),
                );
                let _ = window
                    .remove_event_listener_with_callback("focus", shown.as_ref().unchecked_ref());
            }
        });
    }

    let on_action = Actions {
        live: live.clone(),
        channels: channels.clone(),
        sign_out: sign_out.clone(),
        last_typing,
    }
    .callback();

    if token.is_none() {
        return html! { <Login on_signed_in={on_signed_in} /> };
    }

    let now = sync::wall_ms();
    let state = live.read(|state| state.clone());
    let store = &state.store;
    let sign_out_click = {
        let sign_out = sign_out.clone();
        Callback::from(move |_| sign_out.emit(true))
    };
    let dismiss = on_action.reform(|_: MouseEvent| Action::DismissNotice);
    let open_item = state
        .open_chat
        .and_then(|chat_id| store.item(chat_id).cloned());
    let assistant_user_id = store.assistant.as_ref().map(|assistant| assistant.user_id);

    let side_panel = if let Some(view) = &store.thread_view {
        let ThreadView {
            chat_id, root_id, ..
        } = *view;
        let item = store.item(chat_id);
        html! {
            <ThreadPanel
                {chat_id}
                {root_id}
                messages={view.thread.messages.clone()}
                my_user_id={store.my_user_id}
                is_family_chat={item.is_some_and(|item| item.chat.is_family())}
                is_ai_chat={item.is_some_and(|item| item.chat.is_ai())}
                names={store.names.clone()}
                members={store.members.clone()}
                assistant={store.assistant.clone()}
                blocked={store.blocked.clone()}
                revealed={store.revealed.clone()}
                revealed_quotes={store.revealed_quotes.clone()}
                failed={store.failed_sends(chat_id)}
                ai_failed={store.ai_failed.clone()}
                on_action={on_action.clone()}
            />
        }
    } else if let Some(open) = &store.open_polls {
        html! {
            <OpenPollsPanel
                messages={open.messages.clone()}
                my_user_id={store.my_user_id}
                names={store.names.clone()}
                blocked={store.blocked.clone()}
                revealed={store.revealed.clone()}
                member_count={store.members.len()}
                member_ids={store.members.iter().map(|member| member.id).collect::<std::collections::HashSet<i64>>()}
                {assistant_user_id}
                on_action={on_action.clone()}
            />
        }
    } else {
        Html::default()
    };

    html! {
        <div class="app">
            <header class="bar">
                <span class="brand">{ "Family Connect" }</span>
                // Live updates are paused, and the bar says so. Sending is
                // not: that goes over REST whether the socket is up or not.
                if !state.connected {
                    <span class="status" role="status">{ "Connecting…" }</span>
                }
                <button class="signout" onclick={sign_out_click}>{ "Sign out" }</button>
            </header>
            if let Some(message) = state.failure.clone() {
                <p class="error" role="alert">
                    { message }
                    <button class="link" onclick={dismiss.clone()} aria-label="Dismiss">{ "✕" }</button>
                </p>
            } else if let Some(message) = state.notice.clone() {
                <p class="notice" role="status">
                    { message }
                    <button class="link" onclick={dismiss} aria-label="Dismiss">{ "✕" }</button>
                </p>
            }
            <div class={classes!("split", (store.thread_view.is_some() || store.open_polls.is_some()).then_some("with-panel"))}>
                <ChatList
                    chats={store.sorted_chats()}
                    names={store.names.clone()}
                    blocked={store.blocked.clone()}
                    my_user_id={store.my_user_id}
                    selected={state.open_chat}
                    on_select={on_action.reform(Action::SelectChat)}
                    now_ms={now}
                />
                if let Some(item) = open_item {
                    // Keyed by chat, so switching chats is a fresh pane: its
                    // scroll position, its "pinned to the newest" state and
                    // its unread anchor belong to the chat they were for.
                    <Conversation
                        key={item.chat.id.to_string()}
                        messages={store.threads.get(&item.chat.id).map(|thread| thread.messages.clone()).unwrap_or_default()}
                        can_load_more={store.threads.get(&item.chat.id).is_some_and(|thread| thread.more_above)}
                        my_user_id={store.my_user_id}
                        names={store.names.clone()}
                        members={store.members.clone()}
                        assistant={store.assistant.clone()}
                        blocked={store.blocked.clone()}
                        revealed={store.revealed.clone()}
                        revealed_quotes={store.revealed_quotes.clone()}
                        opening={state.opening.filter(|opening| opening.chat_id == item.chat.id)}
                        failed={store.failed_sends(item.chat.id)}
                        ai_failed={store.ai_failed.clone()}
                        peer_read={store.peer_read.get(&item.chat.id).copied().unwrap_or(0)}
                        typing={store.typing_names(item.chat.id, sync::now_ms())}
                        unanswered_polls={store.unanswered_polls(item.chat.id)}
                        draft={store.drafts.get(&item.chat.id).cloned().unwrap_or_default()}
                        support_contact={store.support_contact.clone()}
                        on_action={on_action.clone()}
                        now_ms={now}
                        item={item.clone()}
                    />
                } else {
                    <section class="conversation empty">
                        <p>{ "Pick a chat." }</p>
                    </section>
                }
                { side_panel }
            </div>
        </div>
    }
}

fn token_is_set(live: &Live) -> bool {
    live.read(|state| state.token.is_some())
}

// These tests run in a BROWSER, not in node: everything under test here
// reaches for `window` — storage, the location, the performance clock —
// and node has none of them.
#[cfg(test)]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    yew::Renderer::<App>::new().render();
}

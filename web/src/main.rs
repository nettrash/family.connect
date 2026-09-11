//! Family Connect in a browser.
//!
//! A web client of the protocol in docs/protocol.md — see "A browser is a
//! client too" for what a browser does differently: it is served from the
//! same origin as the API, registers no device and takes no push, keeps its
//! session token in `sessionStorage` so closing the tab is a sign-out, and
//! SENDS over REST while only listening on the socket.
//!
//! It is being brought to the macOS app's feature set phase by phase: the
//! conversation itself (formatting, links and large emoji, replies with
//! their quotes, threads, reactions, mentions, the assistant with its
//! streamed answers and pictures, polls, edits, seen ticks, the unread
//! divider, reports and blocks, over an outbox that survives a bad
//! network); attachments; the family board; and the account — signing up,
//! the family gate for an account in none, settings, the owner's family
//! pane and everybody's profile pictures. What is not built yet is not
//! stubbed either, because a stub is a claim that something works.

mod actions;
mod api;
mod board;
mod calls;
mod live;
mod location;
mod media;
mod model;
mod outbox;
mod prep;
mod recorder;
mod session;
mod socket;
mod staged;
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
use calls::Calls;
use live::{AppState, Live, Panel};
use media::MediaLoader;
use store::ThreadView;
use sync::{network_back, page_visible, report_read, start_session, Channels, Shared};
use views::board::BoardPane;
use views::call::CallPanel;
use views::chat_list::ChatList;
use views::conversation::Conversation;
use views::dialog::Confirm;
use views::family::FamilyPane;
use views::gate::{FamilyGate, PendingApproval};
use views::login::Login;
use views::open_polls::OpenPollsPanel;
use views::settings::SettingsPane;
use views::thread_panel::ThreadPanel;
use views::viewer::Viewer;

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
    // Signing out from the bar or Settings asks first (ios MacSettingsView
    // "Log out?"): one stray click would otherwise end the session, and
    // with it anything this tab had not sent yet.
    let confirming_sign_out = use_state(|| false);
    let last_typing = use_mut_ref(HashMap::<i64, f64>::new);
    let media = {
        let live = live.clone();
        (*use_memo((), move |_| MediaLoader::new(live))).clone()
    };
    // The call this tab could be on: one peer connection, the ringing, and
    // the four frames that carry it (calls.rs). Held for the app's life,
    // empty between calls.
    let calls = {
        let live = live.clone();
        (*use_memo((), move |_| Calls::new(live, calls::Wire::new()))).clone()
    };

    // Signing out, from anywhere: a 401 and the button do the same thing.
    let sign_out = {
        let live = live.clone();
        let media = media.clone();
        let calls = calls.clone();
        Callback::from(move |revoke: bool| {
            // A call does not outlive the session it was placed in, and the
            // other side is owed the reason rather than a silence.
            calls.end();
            if revoke {
                if let Some(held) = live.read(|state| state.token.clone()) {
                    spawn_local(async move { api::logout(&held).await });
                }
            }
            session::clear();
            media.clear();
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
        let confirming = confirming_sign_out.clone();
        let calls = calls.clone();
        use_effect_with(token.clone(), move |held| {
            // A question asked of the last session is not this one's.
            confirming.set(false);
            if let Some(held) = held.clone() {
                start_session(&live, held, &channels, &sign_out, &calls);
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
                    // Something unsent, or something staged to send: both
                    // live in this tab only.
                    if live.read(|state| {
                        !state.store.outbox.is_empty() || !state.store.staged.is_empty()
                    }) {
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

    // The call goes with the tab — but only once the tab is really going.
    // `pagehide` is that moment; `beforeunload` is a QUESTION, and a person
    // who answers it with "stay" would otherwise be left holding a call the
    // far side has been told is over.
    {
        let calls = calls.clone();
        use_effect_with((), move |_| {
            let leaving = Closure::<dyn Fn()>::new(move || calls.hang_up_on_unload());
            let window = web_sys::window().expect("a window");
            let _ = window
                .add_event_listener_with_callback("pagehide", leaving.as_ref().unchecked_ref());
            move || {
                let _ = window.remove_event_listener_with_callback(
                    "pagehide",
                    leaving.as_ref().unchecked_ref(),
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
                // A board left open is shown again, to whoever came back.
                if token.is_some() {
                    sync::mark_board_shown(&live, session);
                }
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

    // Another tab of this browser showed the board: its marks are this
    // tab's too, so the badge here comes down without waiting to reconnect.
    {
        let live = live.clone();
        use_effect_with((), move |_| {
            let heard = Closure::<dyn Fn(web_sys::StorageEvent)>::new(
                move |event: web_sys::StorageEvent| {
                    let me = live.read(|state| state.store.my_user_id);
                    if me != 0 && event.key() == Some(session::board_marks_key(me)) {
                        let kept = session::board_marks(me);
                        live.now(|state| state.store.board.take_marks(me, kept));
                    }
                },
            );
            let window = web_sys::window().expect("a window");
            let _ =
                window.add_event_listener_with_callback("storage", heard.as_ref().unchecked_ref());
            move || {
                let _ = window
                    .remove_event_listener_with_callback("storage", heard.as_ref().unchecked_ref());
            }
        });
    }

    let on_action = Actions {
        live: live.clone(),
        channels: channels.clone(),
        sign_out: sign_out.clone(),
        last_typing,
        media: media.clone(),
        calls: calls.clone(),
    }
    .callback();

    // The board, while it is the pane in front: shown whenever what it
    // shows changes — a note arriving, a note rewritten; never a drag, which
    // changes neither mark — and as it opens (MacBoardView marks what the
    // window shows, not only where it was opened).
    {
        let board_open = live.read(|state| state.board_open);
        let marks = live.read(|state| state.store.board.marks_if_shown());
        let on_action = on_action.clone();
        use_effect_with((board_open, marks), move |(open, _)| {
            if *open {
                on_action.emit(Action::BoardShown);
            }
        });
    }

    if token.is_none() {
        return html! { <Login on_signed_in={on_signed_in} /> };
    }

    let now = sync::wall_ms();
    let state = live.read(|state| state.clone());
    let store = &state.store;
    let sign_out_click = {
        let confirming = confirming_sign_out.clone();
        Callback::from(move |_: MouseEvent| confirming.set(true))
    };

    // An account in no family: the gate — or, with a request waiting on an
    // owner, the waiting room. Until `/me` has answered nobody knows which,
    // and the shell below says it is connecting.
    if let Some(account) = store
        .account
        .clone()
        .filter(|account| account.family.is_none())
    {
        let on_sign_out = {
            let sign_out = sign_out.clone();
            Callback::from(move |_: ()| sign_out.emit(true))
        };
        return if account.pending_join_request.is_some() {
            html! { <PendingApproval on_action={on_action.clone()} {on_sign_out} /> }
        } else {
            html! {
                <FamilyGate
                    {account}
                    declined={store.join_declined}
                    notice={state.notice.clone()}
                    failure={state.failure.clone()}
                    on_action={on_action.clone()}
                    {on_sign_out}
                />
            }
        };
    }
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
                family={store.family.clone()}
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

    let viewer = state.viewing.clone().map(|viewing| {
        html! {
            <Viewer
                items={viewing.items}
                index={viewing.index}
                on_step={on_action.reform(Action::StepViewer)}
                on_close={on_action.reform(|_: ()| Action::CloseViewer)}
            />
        }
    });

    // The way onto the board, with what is new on it since this browser
    // last showed it (docs/protocol.md, "Board") — for a member of a family,
    // which is the only kind of account that has one.
    let board_button = store.family.is_some().then(|| {
        let unread = store.board.unread();
        let open = on_action.reform(|_: MouseEvent| Action::OpenBoard);
        let label = match unread {
            0 => "Board".to_string(),
            1 => "Board, 1 new note".to_string(),
            count => format!("Board, {count} new notes"),
        };
        html! {
            <button
                class={classes!("board-button", state.board_open.then_some("is-active"))}
                aria-pressed={if state.board_open { "true" } else { "false" }}
                aria-label={label}
                onclick={open}
            >
                { "Board" }
                if unread > 0 {
                    <span class="badge" aria-hidden="true">{ unread }</span>
                }
            </button>
        }
    });

    // The family and the settings, for an account in one — each a pane in
    // the chat's place, as the board is.
    let panel_button = |panel: Panel, label: &'static str| {
        let open = state.panel == Some(panel);
        let toggle = on_action.reform(move |_: MouseEvent| {
            if open {
                Action::ClosePanel
            } else {
                Action::OpenPanel(panel)
            }
        });
        html! {
            <button
                class={classes!("board-button", open.then_some("is-active"))}
                aria-pressed={if open { "true" } else { "false" }}
                onclick={toggle}
            >{ label }</button>
        }
    };
    let family_button = store
        .family
        .is_some()
        .then(|| panel_button(Panel::Family, "Family"));
    let settings_button = store
        .account
        .is_some()
        .then(|| panel_button(Panel::Settings, "Settings"));
    let close_panel = on_action.reform(|_: ()| Action::ClosePanel);
    let panel = match (state.panel, store.account.clone(), store.family.clone()) {
        (Some(Panel::Settings), Some(account), family) => Some(html! {
            <SettingsPane
                {account}
                {family}
                roster_changes={store.roster_changes}
                on_action={on_action.clone()}
                on_close={close_panel.clone()}
                on_sign_out={{
                    let confirming = confirming_sign_out.clone();
                    Callback::from(move |_: ()| confirming.set(true))
                }}
            />
        }),
        (Some(Panel::Family), Some(account), Some(family)) => Some(html! {
            <FamilyPane
                {account}
                {family}
                members={store.members.clone()}
                assistant={store.assistant.clone()}
                blocked={store.blocked.clone()}
                join_requests={store.join_requests.clone()}
                reports={store.reports.clone()}
                support_contact={store.support_contact.clone()}
                on_action={on_action.clone()}
                on_close={close_panel.clone()}
            />
        }),
        _ => None,
    };

    html! {
        <ContextProvider<Calls> context={calls.clone()}>
        <ContextProvider<MediaLoader> context={media}>
        <div class="app">
            <header class="bar">
                <span class="brand">{ "Family Connect" }</span>
                // Live updates are paused, and the bar says so. Sending is
                // not: that goes over REST whether the socket is up or not.
                if !state.connected {
                    <span class="status" role="status">{ "Connecting…" }</span>
                }
                <span class="bar-actions">
                    { board_button.unwrap_or_default() }
                    { family_button.unwrap_or_default() }
                    { settings_button.unwrap_or_default() }
                    <button class="signout" onclick={sign_out_click}>{ "Sign out" }</button>
                </span>
            </header>
            // A call is a BAND under the bar rather than a panel over the
            // page: a call is not a reason to cover the Send button, or the
            // call buttons, or anything else somebody may want while they
            // talk.
            if let Some(call) = state.call.clone() {
                <CallPanel
                    name={store.names.get(&call.peer_user_id).cloned().unwrap_or_else(|| "Someone".to_string())}
                    avatar_version={store.members.iter().find(|member| member.id == call.peer_user_id).map_or(0, |member| member.avatar_version)}
                    {call}
                    on_action={on_action.clone()}
                />
            }
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
                    avatars={store.members.iter().map(|member| (member.id, member.avatar_version)).collect::<HashMap<i64, i64>>()}
                    blocked={store.blocked.clone()}
                    my_user_id={store.my_user_id}
                    selected={state.open_chat}
                    on_select={on_action.reform(Action::SelectChat)}
                    now_ms={now}
                />
                if let Some(panel) = panel {
                    { panel }
                } else if state.board_open && store.family.is_some() {
                    <BoardPane
                        notes={store.board.drawn()}
                        loaded={store.board.loaded}
                        my_user_id={store.my_user_id}
                        names={store.names.clone()}
                        blocked={store.blocked.clone()}
                        revealed={store.board.revealed.clone()}
                        pinning={store.board.pinning}
                        now_minute={(now / 60_000.0).floor() as i64}
                        on_action={on_action.clone()}
                    />
                } else if let Some(item) = open_item {
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
                        calls_enabled={store.account.as_ref().is_some_and(|account| account.calls_enabled)}
                        video_calls_enabled={store.account.as_ref().is_some_and(|account| account.video_calls_enabled)}
                        on_call={state.call.as_ref().is_some_and(|call| call.stage != calls::Stage::Ended)}
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
                        staged={store.staged.get(&item.chat.id).cloned().unwrap_or_default()}
                        family={store.family.clone()}
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
            { viewer.unwrap_or_default() }
            if *confirming_sign_out {
                <Confirm
                    title="Sign out?"
                    message={sign_out_message(!store.outbox.is_empty() || !store.staged.is_empty())}
                    confirm="Sign Out"
                    on_confirm={{
                        let confirming = confirming_sign_out.clone();
                        let sign_out = sign_out.clone();
                        Callback::from(move |_: ()| {
                            confirming.set(false);
                            sign_out.emit(true);
                        })
                    }}
                    on_cancel={{
                        let confirming = confirming_sign_out.clone();
                        Callback::from(move |_: ()| confirming.set(false))
                    }}
                />
            }
        </div>
        </ContextProvider<MediaLoader>>
        </ContextProvider<Calls>>
    }
}

/// What signing out means for this tab: the messages are the server's, the
/// session is the tab's — and what the tab has not sent yet goes with it.
fn sign_out_message(unsent: bool) -> String {
    let lead = "Messages stay on the family server; this tab forgets its session.";
    if unsent {
        format!("{lead} What hasn't been sent yet is lost.")
    } else {
        lead.to_string()
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

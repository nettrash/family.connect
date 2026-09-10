//! One chat: its messages, and the box to add to them.

use std::collections::{HashMap, HashSet};

use web_sys::Element;
use yew::prelude::*;

use crate::actions::Action;
use crate::live::Opening;
use crate::model::{Assistant, ChatListItem, Member, Message};
use crate::store::Draft;
use crate::time;
use crate::timeline::{self, Context};
use crate::views::bubble::Bubble;
use crate::views::composer::{resolve_mentions, Composer, Editing, Replying};
use crate::views::poll::PollComposer;
use crate::views::report::{ReportDialog, ReportTarget};

#[derive(Properties, PartialEq)]
pub struct ConversationProps {
    pub item: ChatListItem,
    pub messages: Vec<Message>,
    pub my_user_id: i64,
    /// userId → the name to draw. A sender who is not in the roster reads
    /// as "Someone" rather than as a number.
    pub names: HashMap<i64, String>,
    pub members: Vec<Member>,
    pub assistant: Option<Assistant>,
    pub blocked: HashSet<i64>,
    pub revealed: HashSet<i64>,
    /// Quote levels of a blocked member peeked at: (message, 0 the quote or
    /// 1 its parent).
    pub revealed_quotes: HashSet<(i64, u8)>,
    /// This chat's unread state once its messages were in — None while they
    /// are loading (`AppState::opening`).
    pub opening: Option<Opening>,
    /// This device's messages that will not be sent unless somebody asks,
    /// by `client_msg_id`, with why.
    pub failed: HashMap<String, String>,
    pub ai_failed: HashSet<i64>,
    /// The peer's read marker, in a direct chat.
    pub peer_read: i64,
    /// Who is typing here, already resolved to names.
    pub typing: Vec<String>,
    /// There is older history to fetch.
    pub can_load_more: bool,
    /// Open polls in this chat the reader has not voted in.
    pub unanswered_polls: usize,
    /// What was being typed here when the reader last left.
    pub draft: String,
    pub support_contact: Option<String>,
    pub on_action: Callback<Action>,
    /// Wall-clock now, for the day pills.
    pub now_ms: f64,
}

/// How close to the bottom still counts as "reading the newest", in
/// pixels. A reader a line or two up is still following along; one who has
/// scrolled a screen away is reading history and must not be yanked down.
const PINNED_SLACK_PX: i32 = 48;

/// How long a jumped-to message stays tinted, in milliseconds — the apps'
/// 1.6 s.
const HIGHLIGHT_MS: u32 = 1_600;

/// Whether the list is scrolled to its newest message, give or take the
/// slack.
fn is_at_bottom(element: &Element) -> bool {
    element.scroll_top() + element.client_height() >= element.scroll_height() - PINNED_SLACK_PX
}

/// A row's identity across renders: my own messages by `client_msg_id`, so
/// the bubble drawn before the server numbered it IS the bubble after (the
/// server keeps that id unique per sender and chat), and everybody else's
/// by server id. Keyed, "Earlier messages" inserts rows above the ones on
/// screen instead of rewriting every row in place — which is what lets the
/// browser keep the reader where they were.
pub fn row_key(message: &Message, my_user_id: i64) -> String {
    match &message.client_msg_id {
        Some(client_msg_id) if message.sender_id == my_user_id => format!("c{client_msg_id}"),
        _ => format!("m{}", message.id),
    }
}

#[function_component(Conversation)]
pub fn conversation(props: &ConversationProps) -> Html {
    let chat_id = props.item.chat.id;
    let list = use_node_ref();
    // Whether the reader is at the newest message. Starts true, so the rows
    // arriving while the chat loads stay at the bottom; the scroll handler
    // keeps it honest after that.
    let pinned = use_mut_ref(|| true);
    // Where this chat OPENED: None until decided, then the first unread row
    // and the count to draw over it — or no anchor, for the newest. Decided
    // ONCE, when the messages are in and from the unread state as it stood
    // before anything read it (`AppState::opening`), so the divider does
    // not walk while they read (ios UnreadAnchor). The pane is keyed by
    // chat, so a new chat is a new decision.
    let opened = use_state(|| Option::<Option<(i64, i64)>>::None);
    {
        let opened = opened.clone();
        let messages = props.messages.clone();
        let me = props.my_user_id;
        let opening = props.opening;
        use_effect_with((opening, props.messages.len()), move |_| {
            if opened.is_some() {
                return;
            }
            let Some(opening) = opening else {
                return;
            };
            opened.set(Some(
                timeline::open_anchor(
                    &messages,
                    opening.unread_count,
                    opening.last_read_message_id,
                    me,
                )
                .map(|id| (id, opening.unread_count)),
            ));
        });
    }
    let anchor = (*opened).flatten();

    let replying = use_state(|| Option::<i64>::None);
    let editing = use_state(|| Option::<i64>::None);
    // The message being edited NOW, for an answer that lands after the
    // render that asked: a handle reads what it was when it was made.
    let editing_now = use_mut_ref(|| Option::<i64>::None);
    *editing_now.borrow_mut() = *editing;
    let report = use_state(|| Option::<ReportTarget>::None);
    let poll_open = use_state(|| false);
    let highlight = use_state(|| Option::<i64>::None);
    let at_newest = use_state(|| true);

    // OPENING: at the unread divider when there is one, otherwise at the
    // newest — once, after the rows are in the DOM. Then, and only then,
    // the view says whether its reader is at the newest message, which is
    // what lets the chat be read at all: a divider two screens up leaves
    // the newest below the fold, and the chat unread until they get there.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let at_newest = at_newest.clone();
        let on_action = props.on_action.clone();
        use_effect_with(*opened, move |opened| {
            let Some(anchor) = opened else {
                return;
            };
            let Some(element) = list.cast::<Element>() else {
                return;
            };
            let divider = anchor.and_then(|_| {
                web_sys::window()?
                    .document()?
                    .get_element_by_id("unread-divider")
            });
            match divider {
                Some(divider) => divider.scroll_into_view(),
                None => element.set_scroll_top(element.scroll_height()),
            }
            let bottom = is_at_bottom(&element);
            *pinned.borrow_mut() = bottom;
            at_newest.set(bottom);
            on_action.emit(Action::AtNewest(bottom));
        });
    }

    // FOLLOW THE CONVERSATION — after the DOM has the new message, and only
    // for a reader who was already at the bottom. A message landing must
    // not scroll somebody reading history, and must not leave somebody
    // following along one message short. An effect rather than a call
    // beside the state change: a call there runs BEFORE the re-render.
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let newest = props.messages.last().map(|message| {
            (
                message.id,
                message.client_msg_id.clone(),
                message.body.len(),
            )
        });
        use_effect_with((props.messages.len(), newest), move |_| {
            if *pinned.borrow() {
                if let Some(element) = list.cast::<Element>() {
                    element.set_scroll_top(element.scroll_height());
                }
            }
        });
    }

    // EARLIER MESSAGES land above the reader, and what they were reading
    // stays where it was on the screen: the scroll moves down by exactly
    // what was added above. Done here rather than left to the browser's
    // scroll anchoring, which Safari does not do (styles.css).
    {
        let list = list.clone();
        let pinned = pinned.clone();
        let before = use_mut_ref(|| (None::<i64>, 0));
        let oldest = props
            .messages
            .iter()
            .map(|message| message.id)
            .find(|id| *id != 0);
        use_effect_with((props.messages.len(), oldest), move |(_, oldest)| {
            let Some(element) = list.cast::<Element>() else {
                return;
            };
            let height = element.scroll_height();
            let (was_oldest, was_height) = *before.borrow();
            if let (Some(now), Some(was)) = (*oldest, was_oldest) {
                if now < was && !*pinned.borrow() {
                    element.set_scroll_top(element.scroll_top() + height - was_height);
                }
            }
            *before.borrow_mut() = (*oldest, height);
        });
    }

    let on_scroll = {
        let list = list.clone();
        let pinned = pinned.clone();
        let at_newest = at_newest.clone();
        let on_action = props.on_action.clone();
        // Until the opening is decided the scroll is the loading rows being
        // followed, not a reader arriving anywhere.
        let decided = opened.is_some();
        Callback::from(move |_: Event| {
            if let Some(element) = list.cast::<Element>() {
                let bottom = is_at_bottom(&element);
                *pinned.borrow_mut() = bottom;
                if *at_newest != bottom {
                    at_newest.set(bottom);
                    if decided {
                        on_action.emit(Action::AtNewest(bottom));
                    }
                }
            }
        })
    };

    let jump_to_newest = {
        let list = list.clone();
        let pinned = pinned.clone();
        Callback::from(move |_: MouseEvent| {
            *pinned.borrow_mut() = true;
            if let Some(element) = list.cast::<Element>() {
                element.set_scroll_top(element.scroll_height());
            }
        })
    };

    // A quote's click: show the quoted message, tinted for a moment — if it
    // is here at all.
    let on_jump = {
        let highlight = highlight.clone();
        let pinned = pinned.clone();
        Callback::from(move |id: i64| {
            let Some(target) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&format!("m-{id}")))
            else {
                return;
            };
            *pinned.borrow_mut() = false;
            target.scroll_into_view();
            highlight.set(Some(id));
            let highlight = highlight.clone();
            gloo_timers::callback::Timeout::new(HIGHLIGHT_MS, move || highlight.set(None)).forget();
        })
    };

    let is_family = props.item.chat.is_family();
    let is_ai = props.item.chat.is_ai();
    let assistant_user_id = props.assistant.as_ref().map(|assistant| assistant.user_id);
    let context = Context {
        my_user_id: props.my_user_id,
        is_family_chat: is_family,
        is_ai_chat: is_ai,
        assistant_user_id,
        blocked: &props.blocked,
        first_unread_id: anchor.map(|(id, _)| id),
        peer_read_up_to: props.peer_read,
    };
    let rows = timeline::rows(&props.messages, &context);
    // In the assistant's own chat, anything not mine is the assistant's —
    // drawn with the chat's own name, not looked up in a roster it is not in.
    let mut names = props.names.clone();
    if is_ai {
        if let Some(title) = props.item.chat.title.clone() {
            for message in &props.messages {
                if message.sender_id != props.my_user_id {
                    names.insert(message.sender_id, title.clone());
                }
            }
        }
    }

    let on_reply = {
        let replying = replying.clone();
        let editing = editing.clone();
        Callback::from(move |id: i64| {
            editing.set(None);
            replying.set(Some(id));
        })
    };
    let on_edit = {
        let replying = replying.clone();
        let editing = editing.clone();
        Callback::from(move |id: i64| {
            replying.set(None);
            editing.set(Some(id));
        })
    };
    let on_report = {
        let report = report.clone();
        let names = names.clone();
        Callback::from(move |(user_id, message_id): (i64, Option<i64>)| {
            report.set(Some(ReportTarget {
                user_id,
                name: names
                    .get(&user_id)
                    .cloned()
                    .unwrap_or_else(|| "Someone".to_string()),
                message_id,
            }));
        })
    };

    let replying_to = replying.and_then(|id| {
        let message = props.messages.iter().find(|message| message.id == id)?;
        let hidden = timeline::is_hidden_by_block(message, props.my_user_id, &props.blocked)
            && !props.revealed.contains(&id);
        Some(Replying {
            message_id: id,
            name: names
                .get(&message.sender_id)
                .cloned()
                .unwrap_or_else(|| "Someone".to_string()),
            excerpt: if hidden {
                String::new()
            } else {
                crate::store::excerpt(&message.body)
            },
        })
    });
    let edit_banner = editing.and_then(|id| {
        let message = props.messages.iter().find(|message| message.id == id)?;
        Some(Editing {
            message_id: id,
            body: message.body.clone(),
        })
    });

    let on_send = {
        let on_action = props.on_action.clone();
        let replying = replying.clone();
        let pinned = pinned.clone();
        Callback::from(move |mut draft: Draft| {
            draft.reply_to_message_id = *replying;
            replying.set(None);
            // Your own message is always shown, wherever you were: a send
            // that disappeared below the fold reads as a send that did not
            // happen.
            *pinned.borrow_mut() = true;
            on_action.emit(Action::Send { chat_id, draft });
        })
    };
    // Edit mode ends when the server has the edit, not when Save is
    // pressed: a refused or lost edit keeps the words in the box (ios
    // MacConversationView clears only once `edit` answers true).
    let on_save_edit = {
        let on_action = props.on_action.clone();
        let editing = editing.clone();
        let editing_now = editing_now.clone();
        Callback::from(move |(message_id, body): (i64, String)| {
            let done = {
                let editing = editing.clone();
                let editing_now = editing_now.clone();
                Callback::from(move |_: ()| {
                    if *editing_now.borrow() == Some(message_id) {
                        editing.set(None);
                    }
                })
            };
            on_action.emit(Action::SaveEdit {
                chat_id,
                message_id,
                body,
                done,
            });
        })
    };

    let poll_dialog = (*poll_open).then(|| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let poll_open = poll_open.clone();
            let replying = replying.clone();
            let pinned = pinned.clone();
            let members = props.members.clone();
            Callback::from(move |(question, options): (String, Vec<String>)| {
                poll_open.set(false);
                *pinned.borrow_mut() = true;
                // A question names members the way a message does, and the
                // list is fixed at send (ios MacConversationView resolves
                // them from the question).
                let mentions = resolve_mentions(&question, &members, is_family);
                on_action.emit(Action::Send {
                    chat_id,
                    draft: Draft {
                        body: question,
                        reply_to_message_id: *replying,
                        mentions,
                        poll: Some(options),
                    },
                });
                replying.set(None);
            })
        };
        let on_cancel = {
            let poll_open = poll_open.clone();
            Callback::from(move |_: ()| poll_open.set(false))
        };
        html! { <PollComposer {on_submit} {on_cancel} /> }
    });

    let report_dialog = (*report).clone().map(|target| {
        let on_submit = {
            let on_action = props.on_action.clone();
            let report = report.clone();
            let target = target.clone();
            Callback::from(move |reason: String| {
                report.set(None);
                on_action.emit(Action::Report {
                    user_id: target.user_id,
                    message_id: target.message_id,
                    reason,
                });
            })
        };
        let on_cancel = {
            let report = report.clone();
            Callback::from(move |_: ()| report.set(None))
        };
        html! {
            <ReportDialog
                {target}
                support_contact={props.support_contact.clone()}
                {on_submit}
                {on_cancel}
            />
        }
    });

    let divider_count = anchor.map(|(_, count)| count).unwrap_or(0);
    let member_count = props.members.len();
    let title = props.item.chat.display_title(
        props
            .item
            .chat
            .peer_user_id
            .and_then(|peer| props.names.get(&peer))
            .map(String::as_str),
    );

    html! {
        <section class="conversation">
            <header class="conversation-bar">
                <h2 class="conversation-title">{ title }</h2>
                if is_family {
                    <button class="open-polls" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::ShowOpenPolls { chat_id }))
                    }>
                        { "Open polls" }
                        if props.unanswered_polls > 0 {
                            <span class="badge">{ props.unanswered_polls }</span>
                        }
                    </button>
                }
            </header>
            <div class="messages" ref={list} onscroll={on_scroll}>
                if props.can_load_more {
                    <button class="load-more" onclick={
                        let on_action = props.on_action.clone();
                        Callback::from(move |_: MouseEvent| on_action.emit(Action::LoadMore { chat_id }))
                    }>{ "Earlier messages" }</button>
                }
                // The rows in a list of their OWN: keys only count in a list
                // where every child has one, and "Earlier messages" above
                // has none — mixed in with it, the rows would be matched up by
                // position after all, and a message landing below would hand
                // each bubble's state (an open menu) to its neighbour.
                <>
                { for rows.iter().map(|row| {
                    let message = row.message;
                    let revealed = props.revealed.contains(&message.id);
                    let quote_revealed = props.revealed_quotes.contains(&(message.id, 0));
                    let parent_revealed = props.revealed_quotes.contains(&(message.id, 1));
                    let failed = message
                        .client_msg_id
                        .as_ref()
                        .and_then(|id| props.failed.get(id))
                        .cloned()
                        .filter(|_| message.id == 0);
                    html! {
                        <key={row_key(message, props.my_user_id)}>
                            if let Some(day) = row.day_above {
                                <div class="day-pill" role="separator">{ time::day_label(day, props.now_ms) }</div>
                            }
                            if row.unread_divider_above {
                                <div id="unread-divider" class="unread-divider" role="separator">
                                    { if divider_count == 1 { "1 new message".to_string() } else { format!("{divider_count} new messages") } }
                                </div>
                            }
                            <div class={classes!("row", (*highlight == Some(message.id)).then_some("is-highlighted"))}>
                                <Bubble
                                    message={message.clone()}
                                    my_user_id={props.my_user_id}
                                    names={names.clone()}
                                    blocked={props.blocked.clone()}
                                    {quote_revealed}
                                    {parent_revealed}
                                    hidden={row.hidden && !revealed}
                                    shows_sender={row.shows_sender || (row.hidden && revealed && is_family)}
                                    run_end={row.run_end}
                                    seen={row.seen}
                                    awaited={row.awaited}
                                    ai_failed={props.ai_failed.contains(&message.id)}
                                    {failed}
                                    is_family_chat={is_family}
                                    is_ai_chat={is_ai}
                                    {assistant_user_id}
                                    in_thread={false}
                                    {member_count}
                                    member_ids={props.members.iter().map(|member| member.id).collect::<HashSet<i64>>()}
                                    on_action={props.on_action.clone()}
                                    on_reply={on_reply.clone()}
                                    on_edit={on_edit.clone()}
                                    on_report={on_report.clone()}
                                    on_jump={on_jump.clone()}
                                />
                            </div>
                        </>
                    }
                }) }
                </>
            </div>
            if !*at_newest {
                <button class="jump-newest" onclick={jump_to_newest} aria-label="Jump to the newest message">{ "↓" }</button>
            }
            if !props.typing.is_empty() {
                <p class="typing" aria-live="polite">{ typing_line(&props.typing) }</p>
            }
            <Composer
                key={format!("composer-{chat_id}")}
                {chat_id}
                is_family_chat={is_family}
                is_ai_chat={is_ai}
                my_user_id={props.my_user_id}
                members={props.members.clone()}
                blocked={props.blocked.clone()}
                assistant={props.assistant.clone()}
                replying={replying_to}
                editing={edit_banner}
                initial={props.draft.clone()}
                on_send={on_send}
                on_save_edit={on_save_edit}
                on_cancel={{
                    let replying = replying.clone();
                    let editing = editing.clone();
                    Callback::from(move |_: ()| {
                        replying.set(None);
                        editing.set(None);
                    })
                }}
                on_typing={props.on_action.reform(move |_: ()| Action::Typing { chat_id })}
                on_draft={props.on_action.reform(move |text: String| Action::SaveDraft { chat_id, text })}
                on_new_poll={{
                    let poll_open = poll_open.clone();
                    Callback::from(move |_: ()| poll_open.set(true))
                }}
            />
            { poll_dialog.unwrap_or_default() }
            { report_dialog.unwrap_or_default() }
        </section>
    }
}

/// "Anna is typing…", "Anna and Bob are typing…", "Several people are
/// typing…" — the third because a list of five names is a line nobody
/// reads, and the count is the part that matters.
pub fn typing_line(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} is typing…"),
        [one, two] => format!("{one} and {two} are typing…"),
        _ => "Several people are typing…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn the_typing_line_names_one_or_two_and_counts_the_rest() {
        assert_eq!(typing_line(&[]), "");
        assert_eq!(typing_line(&["Anna".to_string()]), "Anna is typing…");
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string()]),
            "Anna and Bob are typing…"
        );
        assert_eq!(
            typing_line(&["Anna".to_string(), "Bob".to_string(), "Gran".to_string()]),
            "Several people are typing…"
        );
    }
}

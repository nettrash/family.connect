//! One message, drawn — the same bubble on every surface that draws a
//! message: the chat, the thread, the open-polls list. A second, simpler
//! renderer would be a second place for the rules to drift
//! (docs/protocol.md, "Threads"; ios MacViews/MacMessageRow.swift).

use std::collections::{HashMap, HashSet};

use yew::prelude::*;

use crate::actions::Action;
use crate::model::{Call, Message};
use crate::time;
use crate::views::body::Body;
use crate::views::poll::PollView;
use crate::views::reactions::{chips, details, EmojiPicker, QUICK_REACTIONS};

/// The page's body text size, in CSS pixels (styles.css `body`).
const BODY_PX: f64 = 15.0;

/// The heart a double-click toggles.
pub const HEART: &str = "\u{2764}\u{FE0F}";

/// A call record's line — never its English placeholder body — worded by
/// outcome, direction and kind the way the apps word it
/// (fc_text::call_record, ported from ios Core/Calls/CallRecordText.swift).
/// `mine` is whether this device's person placed the call.
pub fn call_record_line(call: &Call, mine: bool) -> String {
    fc_text::call_record::label(&call.outcome, call.duration_secs, call.video, mine)
}

/// One line saying what an attachment is, until the next phase draws it.
fn attachment_line(message: &Message) -> Option<String> {
    let attachments = message.attachments();
    if attachments.is_empty() {
        return None;
    }
    let names: Vec<String> = attachments
        .iter()
        .map(|attachment| match attachment.kind.as_str() {
            "photo" => "📷 Photo".to_string(),
            "video" => "🎬 Video".to_string(),
            "audio" => "🎤 Voice message".to_string(),
            "location" => "📍 Location".to_string(),
            _ => format!(
                "📎 {}",
                attachment.name.clone().unwrap_or_else(|| "File".into())
            ),
        })
        .collect();
    Some(names.join("  "))
}

#[derive(Properties, PartialEq)]
pub struct BubbleProps {
    pub message: Message,
    pub my_user_id: i64,
    pub names: HashMap<i64, String>,
    pub blocked: HashSet<i64>,
    /// Drawn hidden — a blocked sender, not yet revealed.
    pub hidden: bool,
    /// The quote, and the parent under it, peeked at — each level its own
    /// reveal, held by the app rather than the bubble so that it outlives
    /// the bubble being drawn again (ios MacMessageRow: owned by the
    /// conversation).
    #[prop_or_default]
    pub quote_revealed: bool,
    #[prop_or_default]
    pub parent_revealed: bool,
    pub shows_sender: bool,
    pub run_end: bool,
    pub seen: bool,
    pub awaited: bool,
    pub ai_failed: bool,
    /// Why a send of mine failed, if it did.
    pub failed: Option<String>,
    pub is_family_chat: bool,
    pub is_ai_chat: bool,
    pub assistant_user_id: Option<i64>,
    /// Drawn on the thread surface: no editing, no reporting, no thread chip.
    pub in_thread: bool,
    /// Whether "Reply" means anything here. The open-polls list has no
    /// composer, and a menu item that silently does nothing is worse than
    /// none.
    #[prop_or(true)]
    pub can_reply: bool,
    /// The live roster's size, for a poll's "N of M voted".
    pub member_count: usize,
    /// The live roster — who a mention may open a chat with.
    pub member_ids: HashSet<i64>,
    pub on_action: Callback<Action>,
    /// Asked to reply to this message, or to edit it — the composer's
    /// business, so the bubble only says so.
    pub on_reply: Callback<i64>,
    pub on_edit: Callback<i64>,
    pub on_report: Callback<(i64, Option<i64>)>,
    /// Asked to show the quoted message.
    pub on_jump: Callback<i64>,
}

fn name_of(names: &HashMap<i64, String>, user: i64) -> String {
    names
        .get(&user)
        .cloned()
        .unwrap_or_else(|| "Someone".to_string())
}

#[function_component(Bubble)]
pub fn bubble(props: &BubbleProps) -> Html {
    let menu_open = use_state(|| false);
    let picker_open = use_state(|| false);
    let reactors_open = use_state(|| false);

    let message = &props.message;
    let me = props.my_user_id;
    let chat_id = message.chat_id;
    let id = message.id;
    let mine = message.sender_id == me;
    let acked = id != 0;
    let is_assistant =
        Some(message.sender_id) == props.assistant_user_id || (props.is_ai_chat && !mine);
    let is_other_member = !mine && !is_assistant;
    let emit = |action: Action| {
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| on_action.emit(action.clone()))
    };

    let toggle_menu = {
        let menu_open = menu_open.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            menu_open.set(!*menu_open);
        })
    };
    // A menu item that acts closes the menu first — or it stays open over
    // whatever the action changed (a row hidden by a block, a thread opened
    // beside it) and the next click on "⋯" merely closes it.
    let act = |action: Action| {
        let on_action = props.on_action.clone();
        let menu_open = menu_open.clone();
        Callback::from(move |_: MouseEvent| {
            menu_open.set(false);
            on_action.emit(action.clone());
        })
    };
    let close_menu = {
        let menu_open = menu_open.clone();
        Callback::from(move |_: MouseEvent| menu_open.set(false))
    };
    let report = {
        let on_report = props.on_report.clone();
        let menu_open = menu_open.clone();
        let sender = message.sender_id;
        Callback::from(move |_: MouseEvent| {
            menu_open.set(false);
            on_report.emit((sender, Some(id)));
        })
    };
    let can_report = acked && is_other_member && !props.in_thread;

    if props.hidden {
        // The placeholder and the timestamp and NOTHING else: no name, no
        // avatar, no attachment, no chips (docs/protocol.md, "Blocking a
        // member"). One click reveals it, for this row, on this device. Its
        // menu offers Report and Unblock and nothing more — Copy would put
        // the hidden words on the clipboard (ios MacMessageRow).
        return html! {
            <article id={format!("m-{id}")} class="bubble is-hidden">
                <button class="link" onclick={emit(Action::Reveal { message_id: id })}>
                    { "Hidden — blocked member" }
                </button>
                <div class="bubble-foot">
                    <span class="meta">{ time::clock(&message.created_at) }</span>
                    if is_other_member {
                        <button class="more" aria-label="Message actions" aria-haspopup="menu" onclick={toggle_menu}>{ "⋯" }</button>
                    }
                </div>
                if *menu_open && is_other_member {
                    <div class="menu" role="menu" onmouseleave={close_menu}>
                        <div class="menu-section">{ "Safety" }</div>
                        if can_report {
                            <button role="menuitem" onclick={report}>{ "Report…" }</button>
                        }
                        <button role="menuitem"
                            onclick={act(Action::Block { user_id: message.sender_id, blocked: false })}>
                            { "Unblock" }
                        </button>
                    </div>
                }
            </article>
        };
    }

    let my_reaction = message
        .reactions()
        .iter()
        .find(|reaction| reaction.user_id == me)
        .map(|reaction| reaction.emoji.clone());
    // Double-click is the heart — joined, or taken back if it is mine.
    let on_double = {
        let on_action = props.on_action.clone();
        let mine_heart = my_reaction.as_deref() == Some(HEART);
        Callback::from(move |_: MouseEvent| {
            if !acked {
                return;
            }
            on_action.emit(if mine_heart {
                Action::Unreact {
                    chat_id,
                    message_id: id,
                }
            } else {
                Action::React {
                    chat_id,
                    message_id: id,
                    emoji: HEART.to_string(),
                }
            })
        })
    };

    let quote = message.reply_to.as_ref().map(|quote| {
        let hidden_level = |sender: i64, revealed: bool| {
            sender != me && props.blocked.contains(&sender) && !revealed
        };
        let reveal = |level: u8| {
            let on_action = props.on_action.clone();
            Callback::from(move |event: MouseEvent| {
                event.stop_propagation();
                on_action.emit(Action::RevealQuote {
                    message_id: id,
                    level,
                });
            })
        };
        let jump = {
            let on_jump = props.on_jump.clone();
            let target = quote.message_id;
            Callback::from(move |_: MouseEvent| on_jump.emit(target))
        };
        html! {
            <div class="quote" role="button" onclick={jump}>
                if let Some(parent) = &quote.parent {
                    <div class="quote-parent">
                        if hidden_level(parent.sender_id, props.parent_revealed) {
                            <button class="link" onclick={reveal(1)}>{ "which replied to a hidden message" }</button>
                        } else {
                            <span class="quote-name">{ name_of(&props.names, parent.sender_id) }</span>
                            <span class="quote-text">{ &parent.excerpt }</span>
                        }
                    </div>
                }
                if hidden_level(quote.sender_id, props.quote_revealed) {
                    <button class="link" onclick={reveal(0)}>{ "Replying to a hidden message" }</button>
                } else {
                    <span class="quote-name">{ name_of(&props.names, quote.sender_id) }</span>
                    <span class="quote-text">{ &quote.excerpt }</span>
                }
            </div>
        }
    });

    let reaction_chips = chips(message.reactions(), me);
    let chip_row = (!reaction_chips.is_empty()).then(|| {
        html! {
            <div class="chips">
                { for reaction_chips.iter().map(|chip| {
                    // A click never takes a reaction away: on a chip I am
                    // not part of it joins; on mine it shows who reacted,
                    // where my own row is the explicit remove (ios
                    // MacMessageRow — undoing what you never meant to do is
                    // worse than one more click).
                    let join = if chip.mine {
                        let reactors_open = reactors_open.clone();
                        Callback::from(move |_: MouseEvent| reactors_open.set(true))
                    } else {
                        emit(Action::React {
                            chat_id,
                            message_id: id,
                            emoji: chip.emoji.clone(),
                        })
                    };
                    html! {
                        <button
                            class={classes!("chip", chip.mine.then_some("is-mine"))}
                            onclick={join}
                            disabled={!acked}
                        >
                            { &chip.emoji }
                            if chip.count > 1 { <span class="chip-count">{ chip.count }</span> }
                        </button>
                    }
                }) }
            </div>
        }
    });

    let reactors = (*reactors_open).then(|| {
        let close = {
            let reactors_open = reactors_open.clone();
            Callback::from(move |_: MouseEvent| reactors_open.set(false))
        };
        // The chips' own order, "You" first, a blocked reactor counted on
        // the chip and not named here (fc_text::reactions::reaction_details).
        let rows = details(message.reactions(), &props.names, me, &props.blocked);
        html! {
            <div class="popover" role="dialog" aria-label="Who reacted">
                { for rows.iter().map(|row| {
                    let own = my_reaction.as_deref() == Some(row.emoji.as_str());
                    html! {
                        <div class="reactor">
                            <span>{ &row.emoji }</span>
                            <span>{ row.names.join(", ") }</span>
                            if own {
                                <button class="link" onclick={emit(Action::Unreact { chat_id, message_id: id })}>
                                    { "Click to remove" }
                                </button>
                            }
                        </div>
                    }
                }) }
                <button class="link" onclick={close}>{ "Close" }</button>
            </div>
        }
    });

    let can_view_thread = acked
        && !props.in_thread
        && (message.thread_root_id.is_some() || message.reply_count.is_some());
    let can_edit =
        acked && mine && !props.in_thread && !message.body.is_empty() && message.call.is_none();
    // Choosing the emoji that is already mine takes it off — the menu and
    // the picker TOGGLE, the Mac's `toggleReaction`; only the chips never
    // remove.
    let toggle = |emoji: String| {
        if my_reaction.as_deref() == Some(emoji.as_str()) {
            Action::Unreact {
                chat_id,
                message_id: id,
            }
        } else {
            Action::React {
                chat_id,
                message_id: id,
                emoji,
            }
        }
    };

    let menu = (*menu_open).then(|| {
        let reply = {
            let on_reply = props.on_reply.clone();
            let menu_open = menu_open.clone();
            Callback::from(move |_: MouseEvent| {
                menu_open.set(false);
                on_reply.emit(id);
            })
        };
        let edit = {
            let on_edit = props.on_edit.clone();
            let menu_open = menu_open.clone();
            Callback::from(move |_: MouseEvent| {
                menu_open.set(false);
                on_edit.emit(id);
            })
        };
        let more = {
            let picker_open = picker_open.clone();
            let menu_open = menu_open.clone();
            Callback::from(move |_: MouseEvent| {
                menu_open.set(false);
                picker_open.set(true);
            })
        };
        let who = {
            let reactors_open = reactors_open.clone();
            let menu_open = menu_open.clone();
            Callback::from(move |_: MouseEvent| {
                menu_open.set(false);
                reactors_open.set(true);
            })
        };
        let copy = {
            let body = message.body.clone();
            let menu_open = menu_open.clone();
            Callback::from(move |_: MouseEvent| {
                menu_open.set(false);
                if let Some(window) = web_sys::window() {
                    let _ = window.navigator().clipboard().write_text(&body);
                }
            })
        };
        let blocked_sender = props.blocked.contains(&message.sender_id);
        let failed = props.failed.is_some();
        let client_msg_id = message.client_msg_id.clone().unwrap_or_default();
        html! {
            <div class="menu" role="menu" onmouseleave={close_menu}>
                if acked {
                    <div class="menu-reactions">
                        { for QUICK_REACTIONS.iter().map(|emoji| html! {
                            <button role="menuitem"
                                class={classes!("menu-emoji", (my_reaction.as_deref() == Some(*emoji)).then_some("is-mine"))}
                                onclick={act(toggle(emoji.to_string()))}>
                                { *emoji }
                            </button>
                        }) }
                        <button role="menuitem" class="link" onclick={more}>{ "More reactions…" }</button>
                    </div>
                    if !message.reactions().is_empty() {
                        <button role="menuitem" onclick={who}>{ "See who reacted" }</button>
                    }
                    if props.can_reply {
                        <button role="menuitem" onclick={reply}>{ "Reply" }</button>
                    }
                    if can_view_thread {
                        <button role="menuitem" onclick={act(Action::OpenThread { chat_id, message_id: id })}>
                            { "View thread" }
                        </button>
                    }
                    if can_edit {
                        <button role="menuitem" onclick={edit}>{ "Edit" }</button>
                    }
                }
                if !message.body.is_empty() {
                    <button role="menuitem" onclick={copy}>{ "Copy" }</button>
                }
                if failed {
                    <button role="menuitem" onclick={act(Action::Retry(client_msg_id.clone()))}>{ "Try Again" }</button>
                    <button role="menuitem" class="danger" onclick={act(Action::Discard(client_msg_id))}>{ "Delete" }</button>
                }
                if is_other_member {
                    <div class="menu-section">{ "Safety" }</div>
                    if can_report {
                        <button role="menuitem" onclick={report.clone()}>{ "Report…" }</button>
                    }
                    if blocked_sender {
                        <button role="menuitem"
                            onclick={act(Action::Block { user_id: message.sender_id, blocked: false })}>
                            { "Unblock" }
                        </button>
                    } else {
                        <button role="menuitem" class="danger"
                            onclick={act(Action::Block { user_id: message.sender_id, blocked: true })}>
                            { "Block" }
                        </button>
                    }
                }
            </div>
        }
    });

    let picker = (*picker_open).then(|| {
        let on_pick = {
            let on_action = props.on_action.clone();
            let picker_open = picker_open.clone();
            let mine_now = my_reaction.clone();
            Callback::from(move |emoji: String| {
                picker_open.set(false);
                on_action.emit(if mine_now.as_deref() == Some(emoji.as_str()) {
                    Action::Unreact {
                        chat_id,
                        message_id: id,
                    }
                } else {
                    Action::React {
                        chat_id,
                        message_id: id,
                        emoji,
                    }
                });
            })
        };
        let on_close = {
            let picker_open = picker_open.clone();
            Callback::from(move |_: ()| picker_open.set(false))
        };
        html! { <EmojiPicker {on_pick} {on_close} /> }
    });

    let meta = if let Some(reason) = props.failed.clone() {
        let client_msg_id = message.client_msg_id.clone().unwrap_or_default();
        html! {
            <span class="meta send-failed" role="alert">
                { reason }
                <button class="link" onclick={emit(Action::Retry(client_msg_id.clone()))}>{ "Retry" }</button>
                <button class="link" onclick={emit(Action::Discard(client_msg_id))}>{ "Discard" }</button>
            </span>
        }
    } else if !acked {
        html! { <span class="meta sending">{ "Sending…" }</span> }
    } else if props.run_end || message.is_edited() {
        html! {
            <span class="meta">
                { time::clock(&message.created_at) }
                if message.is_edited() { <span class="edited">{ " · edited" }</span> }
                if mine && !props.is_family_chat {
                    <span class={classes!("tick", props.seen.then_some("is-seen"))}
                          aria-label={if props.seen { "Seen" } else { "Sent" }}>
                        { if props.seen { " ✓✓" } else { " ✓" } }
                    </span>
                }
            </span>
        }
    } else {
        Html::default()
    };

    // The ladder was drawn against a phone's 17-point body text; scaled to
    // this page's 15px the way the Mac scales it to 13, so the proportion
    // is the phone's.
    let emoji_size = fc_text::emoji::display_font_size_for_body(&message.body, BODY_PX);
    let body = if let Some(call) = &message.call {
        let missed_incoming = call.outcome == "missed" && !mine;
        html! {
            <p class={classes!("call-record", missed_incoming.then_some("is-missed"))}>
                { if call.video { "📹 " } else { "📞 " } }
                { call_record_line(call, mine) }
            </p>
        }
    } else if props.awaited {
        if props.ai_failed {
            html! { <p class="body ai-failed">{ "Couldn't answer that. Ask again." }</p> }
        } else {
            html! { <p class="body awaiting" aria-label="The assistant is answering">{ "▍" }</p> }
        }
    } else if let Some(size) = emoji_size {
        // One to four emoji: drawn large, bare, and as themselves — no
        // markdown, no links, no balloon (ios MacMessageRow `isEmojiOnly`).
        html! {
            <p class="emoji-only" style={format!("font-size:{size:.1}px")}>{ &message.body }</p>
        }
    } else {
        html! {
            <>
                if !message.body.is_empty() {
                    <Body
                        text={message.body.clone()}
                        mentions={message.mentions().to_vec()}
                        my_user_id={me}
                        member_ids={props.member_ids.clone()}
                        on_open_direct={props.on_action.reform(|user_id| Action::OpenDirect { user_id })}
                    />
                }
                if props.ai_failed {
                    <p class="ai-failed">{ "Couldn't answer that. Ask again." }</p>
                }
            </>
        }
    };

    html! {
        <article
            id={format!("m-{id}")}
            class={classes!(
                "bubble",
                mine.then_some("is-mine"),
                (!acked).then_some("is-pending"),
                props.failed.is_some().then_some("is-failed"),
                message.call.is_some().then_some("is-call"),
                emoji_size.is_some().then_some("is-emoji-only"),
            )}
            ondblclick={on_double}
        >
            if props.shows_sender {
                <span class="sender">{ name_of(&props.names, message.sender_id) }</span>
            }
            { quote.unwrap_or_default() }
            { body }
            if let Some(line) = attachment_line(message) {
                <p class="attachments-line">{ line }</p>
            }
            if message.poll.is_some() {
                <PollView
                    message={message.clone()}
                    my_user_id={me}
                    member_count={props.member_count}
                    names={props.names.clone()}
                    blocked={props.blocked.clone()}
                    on_vote={props.on_action.reform(move |option_id| Action::Vote { chat_id, message_id: id, option_id })}
                    on_unvote={props.on_action.reform(move |_| Action::Unvote { chat_id, message_id: id })}
                    on_close={props.on_action.reform(move |_| Action::ClosePoll { chat_id, message_id: id })}
                />
            }
            { chip_row.unwrap_or_default() }
            if let Some(count) = message.reply_count.filter(|_| !props.in_thread) {
                <button class="thread-chip" onclick={emit(Action::OpenThread { chat_id, message_id: id })}
                        aria-label={format!("{count} replies")} title="Opens the thread">
                    { format!("↩ {count} {} ›", if count == 1 { "reply" } else { "replies" }) }
                </button>
            }
            <div class="bubble-foot">
                { meta }
                <button class="more" aria-label="Message actions" aria-haspopup="menu" onclick={toggle_menu}>{ "⋯" }</button>
            </div>
            { menu.unwrap_or_default() }
            { reactors.unwrap_or_default() }
            { picker.unwrap_or_default() }
        </article>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The ported wording, through the web's own model type — the full
    /// table is pinned in fc_text::call_record against the Swift tests.
    #[wasm_bindgen_test]
    fn a_call_record_reads_the_way_the_apps_word_it() {
        let call = |outcome: &str, seconds: Option<i64>, video: bool| Call {
            outcome: outcome.into(),
            duration_secs: seconds,
            video,
        };
        assert_eq!(
            call_record_line(&call("completed", Some(222), false), true),
            "Voice call · 3:42"
        );
        assert_eq!(
            call_record_line(&call("missed", None, false), true),
            "No answer"
        );
        assert_eq!(
            call_record_line(&call("missed", None, true), false),
            "Missed video call"
        );
    }

    use crate::model::Reaction;
    use std::cell::RefCell;
    use std::rc::Rc;
    use wasm_bindgen::JsCast;
    use web_sys::{Element, HtmlElement};

    const ME: i64 = 7;
    const ANNA: i64 = 9;

    fn message(id: i64, sender: i64, body: &str) -> Message {
        Message {
            id,
            chat_id: 42,
            sender_id: sender,
            client_msg_id: Some(format!("c{id}")),
            body: body.into(),
            created_at: if id == 0 {
                String::new()
            } else {
                "2026-09-10T10:00:00Z".into()
            },
            ..Message::default()
        }
    }

    fn props(message: Message, actions: Rc<RefCell<Vec<Action>>>) -> BubbleProps {
        BubbleProps {
            message,
            my_user_id: ME,
            names: HashMap::from([(ANNA, "Anna".to_string())]),
            blocked: HashSet::new(),
            hidden: false,
            quote_revealed: false,
            parent_revealed: false,
            shows_sender: true,
            run_end: true,
            seen: false,
            awaited: false,
            ai_failed: false,
            failed: None,
            is_family_chat: true,
            is_ai_chat: false,
            assistant_user_id: Some(2),
            in_thread: false,
            can_reply: true,
            member_count: 3,
            member_ids: HashSet::from([ANNA]),
            on_action: Callback::from(move |action: Action| actions.borrow_mut().push(action)),
            on_reply: Callback::noop(),
            on_edit: Callback::noop(),
            on_report: Callback::noop(),
            on_jump: Callback::noop(),
        }
    }

    async fn render(props: BubbleProps) -> (Element, yew::AppHandle<Bubble>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle = yew::Renderer::<Bubble>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        (root, handle)
    }

    fn text(root: &Element) -> String {
        root.text_content().unwrap_or_default()
    }

    /// Delivered, on its way, and not sent: three bubbles that must not look
    /// alike — and the failed one can be tried again or given up.
    #[wasm_bindgen_test]
    async fn a_bubble_says_whether_it_was_delivered() {
        let actions = Rc::new(RefCell::new(Vec::new()));

        let (root, handle) = render(props(message(101, ME, "delivered"), actions.clone())).await;
        assert!(!text(&root).contains("Sending"), "{}", text(&root));
        handle.destroy();
        root.remove();

        let (root, handle) = render(props(message(0, ME, "on its way"), actions.clone())).await;
        assert!(text(&root).contains("Sending…"), "{}", text(&root));
        assert!(root.query_selector(".bubble.is-pending").unwrap().is_some());
        handle.destroy();
        root.remove();

        let mut failed = props(message(0, ME, "refused"), actions.clone());
        failed.failed = Some("Not sent: blocked.".into());
        let (root, handle) = render(failed).await;
        assert!(text(&root).contains("Not sent: blocked."));
        let buttons = root.query_selector_all(".send-failed button").unwrap();
        buttons
            .item(0)
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        buttons
            .item(1)
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert_eq!(
            *actions.borrow(),
            vec![Action::Retry("c0".into()), Action::Discard("c0".into())]
        );
        handle.destroy();
        root.remove();
    }

    /// A hidden row draws the placeholder and the time and NOTHING else —
    /// no name, no words, no chips — and one click asks to reveal it.
    #[wasm_bindgen_test]
    async fn a_hidden_row_shows_nothing_of_its_sender() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let mut blocked = message(101, ANNA, "something unkind");
        blocked.reactions = Some(vec![Reaction {
            user_id: ME,
            emoji: "👍".into(),
        }]);
        let mut hidden = props(blocked, actions.clone());
        hidden.hidden = true;
        let (root, handle) = render(hidden).await;
        let shown = text(&root);
        assert!(shown.contains("Hidden — blocked member"));
        assert!(
            !shown.contains("unkind") && !shown.contains("Anna") && !shown.contains("👍"),
            "{shown}"
        );
        root.query_selector("button")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert_eq!(*actions.borrow(), vec![Action::Reveal { message_id: 101 }]);
        handle.destroy();
        root.remove();
    }

    /// Chips, the thread affordance, and a call record that never shows its
    /// English placeholder body.
    #[wasm_bindgen_test]
    async fn chips_the_thread_chip_and_a_call_record_draw() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let mut answered = message(101, ANNA, "Dinner?");
        answered.reactions = Some(vec![
            Reaction {
                user_id: ANNA,
                emoji: "👍".into(),
            },
            Reaction {
                user_id: ME,
                emoji: "👍".into(),
            },
        ]);
        answered.reply_count = Some(2);
        let (root, handle) = render(props(answered, actions.clone())).await;
        let chip = root
            .query_selector(".chip.is-mine")
            .unwrap()
            .expect("my chip");
        assert!(chip.text_content().unwrap_or_default().contains('2'));
        assert!(text(&root).contains("2 replies"));
        root.query_selector(".thread-chip")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
        assert_eq!(
            actions.borrow().last(),
            Some(&Action::OpenThread {
                chat_id: 42,
                message_id: 101
            })
        );
        handle.destroy();
        root.remove();

        let mut record = message(102, ANNA, "Missed voice call");
        record.call = Some(Call {
            outcome: "completed".into(),
            duration_secs: Some(65),
            video: false,
        });
        let (root, handle) = render(props(record, actions.clone())).await;
        assert!(text(&root).contains("Voice call · 1:05"));
        assert!(
            !text(&root).contains("Missed"),
            "the placeholder body is never shown"
        );
        handle.destroy();
        root.remove();
    }

    /// A menu item that ACTS closes the menu — found end to end: after
    /// "Block" the menu stayed open under the hidden row, and after a reveal
    /// the next "⋯" merely closed it again.
    #[wasm_bindgen_test]
    async fn choosing_from_the_menu_closes_it() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let (root, handle) = render(props(message(101, ANNA, "Dinner?"), actions.clone())).await;
        let click = |selector: &str| {
            root.query_selector(selector)
                .unwrap()
                .unwrap_or_else(|| panic!("{selector}"))
                .dyn_into::<HtmlElement>()
                .unwrap()
                .click();
        };
        for item in [".menu-emoji", ".menu .danger"] {
            click(".more");
            gloo_timers::future::TimeoutFuture::new(20).await;
            click(item);
            gloo_timers::future::TimeoutFuture::new(20).await;
            assert!(
                root.query_selector(".menu").unwrap().is_none(),
                "{item} left the menu open"
            );
        }
        assert!(matches!(actions.borrow()[0], Action::React { .. }));
        assert_eq!(
            actions.borrow()[1],
            Action::Block {
                user_id: ANNA,
                blocked: true
            }
        );
        handle.destroy();
        root.remove();
    }

    /// Double-click is the heart.
    #[wasm_bindgen_test]
    async fn a_double_click_sends_the_heart() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let (root, handle) = render(props(message(101, ANNA, "Dinner?"), actions.clone())).await;
        let bubble = root.query_selector("article").unwrap().unwrap();
        let event = web_sys::MouseEvent::new("dblclick").unwrap();
        bubble.dispatch_event(&event).unwrap();
        assert_eq!(
            *actions.borrow(),
            vec![Action::React {
                chat_id: 42,
                message_id: 101,
                emoji: HEART.to_string()
            }]
        );
        handle.destroy();
        root.remove();
    }

    fn click(root: &Element, selector: &str) {
        root.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} is on the page"))
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
    }

    fn click_text(root: &Element, selector: &str, label: &str) {
        let found = root.query_selector_all(selector).unwrap();
        for index in 0..found.length() {
            let element = found.item(index).unwrap();
            if element.text_content().unwrap_or_default().trim() == label {
                element.dyn_into::<HtmlElement>().unwrap().click();
                return;
            }
        }
        panic!("no {selector} reading {label:?}");
    }

    fn labels(root: &Element, selector: &str) -> Vec<String> {
        let found = root.query_selector_all(selector).unwrap();
        (0..found.length())
            .map(|index| {
                found
                    .item(index)
                    .unwrap()
                    .text_content()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect()
    }

    async fn settle() {
        gloo_timers::future::TimeoutFuture::new(20).await;
    }

    /// The menu and the picker TOGGLE — choosing the emoji that is already
    /// mine takes it off, the Mac's `toggleReaction` — while a chip never
    /// removes: mine shows who reacted, somebody else's joins.
    #[wasm_bindgen_test]
    async fn the_menu_toggles_my_reaction_and_my_chip_shows_who_reacted() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let mut reacted = message(101, ANNA, "Dinner?");
        let mine = QUICK_REACTIONS[0].to_string();
        let theirs = QUICK_REACTIONS[1].to_string();
        reacted.reactions = Some(vec![
            Reaction {
                user_id: ME,
                emoji: mine.clone(),
            },
            Reaction {
                user_id: ANNA,
                emoji: theirs.clone(),
            },
        ]);
        let (root, handle) = render(props(reacted, actions.clone())).await;

        click(&root, ".more");
        settle().await;
        click(&root, ".menu-emoji.is-mine");
        settle().await;
        click(&root, ".more");
        settle().await;
        click_text(&root, ".menu-emoji", &theirs);
        settle().await;
        assert_eq!(
            *actions.borrow(),
            vec![
                Action::Unreact {
                    chat_id: 42,
                    message_id: 101
                },
                Action::React {
                    chat_id: 42,
                    message_id: 101,
                    emoji: theirs.clone()
                },
            ]
        );

        actions.borrow_mut().clear();
        click(&root, ".chip.is-mine");
        settle().await;
        assert!(actions.borrow().is_empty(), "my chip takes nothing away");
        assert!(
            root.query_selector(".popover").unwrap().is_some(),
            "it shows who reacted"
        );
        click_text(&root, ".chip:not(.is-mine)", &theirs);
        settle().await;
        assert_eq!(
            *actions.borrow(),
            vec![Action::React {
                chat_id: 42,
                message_id: 101,
                emoji: theirs
            }]
        );
        handle.destroy();
        root.remove();
    }

    /// A hidden row's menu offers Report and Unblock and NOTHING else —
    /// Copy would put the hidden words on the clipboard.
    #[wasm_bindgen_test]
    async fn a_hidden_row_offers_report_and_unblock_only() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let reported = Rc::new(RefCell::new(Vec::new()));
        let mut hidden = props(message(101, ANNA, "something unkind"), actions.clone());
        hidden.hidden = true;
        hidden.blocked = HashSet::from([ANNA]);
        hidden.on_report = {
            let reported = reported.clone();
            Callback::from(move |target: (i64, Option<i64>)| reported.borrow_mut().push(target))
        };
        let (root, handle) = render(hidden).await;

        click(&root, ".more");
        settle().await;
        assert_eq!(
            labels(&root, ".menu [role=menuitem]"),
            vec!["Report…", "Unblock"]
        );
        click_text(&root, ".menu [role=menuitem]", "Report…");
        settle().await;
        assert_eq!(*reported.borrow(), vec![(ANNA, Some(101))]);
        click(&root, ".more");
        settle().await;
        click_text(&root, ".menu [role=menuitem]", "Unblock");
        settle().await;
        assert_eq!(
            *actions.borrow(),
            vec![Action::Block {
                user_id: ANNA,
                blocked: false
            }]
        );
        assert!(!text(&root).contains("unkind"));
        handle.destroy();
        root.remove();
    }

    /// Each level of a hidden quote is its own reveal, asked of the app —
    /// which holds it, so it outlives this bubble being drawn again.
    #[wasm_bindgen_test]
    async fn each_level_of_a_hidden_quote_reveals_on_its_own() {
        use crate::model::{ReplyParent, ReplyTo};
        let actions = Rc::new(RefCell::new(Vec::new()));
        let mut reply = message(101, ME, "no");
        reply.reply_to = Some(ReplyTo {
            message_id: 100,
            sender_id: ANNA,
            excerpt: "the quote".into(),
            parent: Some(ReplyParent {
                message_id: 99,
                sender_id: ANNA,
                excerpt: "the parent".into(),
            }),
        });
        let mut quoted = props(reply, actions.clone());
        quoted.blocked = HashSet::from([ANNA]);
        let (root, handle) = render(quoted.clone_for_test()).await;
        assert!(!text(&root).contains("the quote") && !text(&root).contains("the parent"));
        click_text(&root, ".quote button", "Replying to a hidden message");
        click_text(&root, ".quote button", "which replied to a hidden message");
        assert_eq!(
            *actions.borrow(),
            vec![
                Action::RevealQuote {
                    message_id: 101,
                    level: 0
                },
                Action::RevealQuote {
                    message_id: 101,
                    level: 1
                },
            ]
        );
        handle.destroy();
        root.remove();

        let mut revealed = quoted;
        revealed.quote_revealed = true;
        let (root, handle) = render(revealed).await;
        assert!(text(&root).contains("the quote"), "{}", text(&root));
        assert!(
            !text(&root).contains("the parent"),
            "the parent is its own reveal"
        );
        handle.destroy();
        root.remove();
    }

    /// Where Reply would do nothing — the open-polls list has no composer —
    /// it is not offered.
    #[wasm_bindgen_test]
    async fn reply_is_offered_only_where_it_does_something() {
        let actions = Rc::new(RefCell::new(Vec::new()));
        let (root, handle) = render(props(message(101, ANNA, "Dinner?"), actions.clone())).await;
        click(&root, ".more");
        settle().await;
        assert!(labels(&root, ".menu [role=menuitem]").contains(&"Reply".to_string()));
        handle.destroy();
        root.remove();

        let mut no_reply = props(message(101, ANNA, "Dinner?"), actions);
        no_reply.can_reply = false;
        let (root, handle) = render(no_reply).await;
        click(&root, ".more");
        settle().await;
        assert!(!labels(&root, ".menu [role=menuitem]").contains(&"Reply".to_string()));
        handle.destroy();
        root.remove();
    }

    impl BubbleProps {
        fn clone_for_test(&self) -> BubbleProps {
            BubbleProps {
                message: self.message.clone(),
                my_user_id: self.my_user_id,
                names: self.names.clone(),
                blocked: self.blocked.clone(),
                hidden: self.hidden,
                quote_revealed: self.quote_revealed,
                parent_revealed: self.parent_revealed,
                shows_sender: self.shows_sender,
                run_end: self.run_end,
                seen: self.seen,
                awaited: self.awaited,
                ai_failed: self.ai_failed,
                failed: self.failed.clone(),
                is_family_chat: self.is_family_chat,
                is_ai_chat: self.is_ai_chat,
                assistant_user_id: self.assistant_user_id,
                in_thread: self.in_thread,
                can_reply: self.can_reply,
                member_count: self.member_count,
                member_ids: self.member_ids.clone(),
                on_action: self.on_action.clone(),
                on_reply: self.on_reply.clone(),
                on_edit: self.on_edit.clone(),
                on_report: self.on_report.clone(),
                on_jump: self.on_jump.clone(),
            }
        }
    }
}

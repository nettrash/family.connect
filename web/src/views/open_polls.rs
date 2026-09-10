//! The family chat's open polls, as they stand — a surface of its own
//! rather than a banner in the scroll, because a decision nobody can find is
//! a decision nobody makes (docs/protocol.md, "Finding the open ones").
//!
//! Oldest first, by message id, so voting never reshuffles the list under
//! the reader. A blocked member's poll is the hidden row here too.

use std::collections::{HashMap, HashSet};

use yew::prelude::*;

use crate::actions::Action;
use crate::model::Message;
use crate::timeline;
use crate::views::bubble::Bubble;

#[derive(Properties, PartialEq)]
pub struct OpenPollsProps {
    pub messages: Vec<Message>,
    pub my_user_id: i64,
    pub names: HashMap<i64, String>,
    pub blocked: HashSet<i64>,
    pub revealed: HashSet<i64>,
    pub member_count: usize,
    pub member_ids: HashSet<i64>,
    pub assistant_user_id: Option<i64>,
    pub on_action: Callback<Action>,
}

#[function_component(OpenPollsPanel)]
pub fn open_polls_panel(props: &OpenPollsProps) -> Html {
    let close = {
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| on_action.emit(Action::HideOpenPolls))
    };
    let on_key = {
        let on_action = props.on_action.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                on_action.emit(Action::HideOpenPolls);
            }
        })
    };
    let mut messages = props.messages.clone();
    messages.sort_by_key(|message| message.id);
    html! {
        <aside class="thread-panel open-polls-panel" aria-label="Open polls" onkeydown={on_key}>
            <header class="thread-bar">
                <h2>{ "Open polls" }</h2>
                <button class="link" onclick={close} aria-label="Close">{ "✕" }</button>
            </header>
            <div class="thread-messages">
                if messages.is_empty() {
                    <p class="empty-note">{ "Nothing to decide" }</p>
                }
                // Keyed, in a list of their own (see views/conversation.rs).
                <>
                { for messages.iter().map(|message| {
                    let hidden = timeline::is_hidden_by_block(message, props.my_user_id, &props.blocked)
                        && !props.revealed.contains(&message.id);
                    html! {
                        <div key={format!("p{}", message.id)} class="row">
                            <Bubble
                                message={message.clone()}
                                my_user_id={props.my_user_id}
                                names={props.names.clone()}
                                blocked={props.blocked.clone()}
                                {hidden}
                                shows_sender={message.sender_id != props.my_user_id && !hidden}
                                run_end={true}
                                seen={false}
                                awaited={false}
                                ai_failed={false}
                                failed={None::<String>}
                                is_family_chat={true}
                                is_ai_chat={false}
                                assistant_user_id={props.assistant_user_id}
                                in_thread={true}
                                can_reply={false}
                                member_count={props.member_count}
                                member_ids={props.member_ids.clone()}
                                on_action={props.on_action.clone()}
                                on_reply={Callback::from(|_: i64| {})}
                                on_edit={Callback::from(|_: i64| {})}
                                on_report={Callback::from(|_: (i64, Option<i64>)| {})}
                                on_jump={Callback::from(|_: i64| {})}
                            />
                        </div>
                    }
                }) }
                </>
            </div>
        </aside>
    }
}

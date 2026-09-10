//! The list of chats down the side: the family chat always, direct chats
//! once they exist, each with its unread count.

use yew::prelude::*;

use crate::model::ChatListItem;

#[derive(Properties, PartialEq)]
pub struct ChatListProps {
    pub chats: Vec<ChatListItem>,
    pub selected: Option<i64>,
    pub on_select: Callback<i64>,
}

#[function_component(ChatList)]
pub fn chat_list(props: &ChatListProps) -> Html {
    html! {
        <nav class="chat-list" aria-label="Chats">
            { for props.chats.iter().map(|item| {
                let chat_id = item.chat.id;
                let selected = props.selected == Some(chat_id);
                let on_select = props.on_select.clone();
                let onclick = Callback::from(move |_| on_select.emit(chat_id));
                let preview = item
                    .last_message
                    .as_ref()
                    .map(|message| message.body.clone())
                    .unwrap_or_default();
                html! {
                    <button
                        class={classes!("chat-row", selected.then_some("is-selected"))}
                        aria-current={selected.then_some("true")}
                        {onclick}
                    >
                        <span class="chat-title">{ item.chat.display_title(None) }</span>
                        // A caption-less photo has an EMPTY body on the wire,
                        // so a preview that is blank is a real state — the
                        // row simply has no second line rather than a stray
                        // empty one.
                        if !preview.is_empty() {
                            <span class="chat-preview">{ preview }</span>
                        }
                        if item.unread_count > 0 {
                            <span class="badge" aria-label={format!("{} unread", item.unread_count)}>
                                { item.unread_count }
                            </span>
                        }
                    </button>
                }
            }) }
        </nav>
    }
}

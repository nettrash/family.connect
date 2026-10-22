//! A chain of replies on a surface of its own — the root at the top, the
//! replies below in order, and a composer whose every send answers the ROOT
//! whatever the reader was looking at (docs/protocol.md, "Threads").
//!
//! Rows draw exactly as in the chat — the same bubble, the same quotes,
//! the same hidden-row rule — minus what the apps leave out here: editing,
//! reporting, attachments and polls. A STICKER is not left out (ios
//! ThreadView): it is its own message with nothing to stage, and it goes
//! where every send from here goes — to the root. Nor is the consent
//! question: what is sent from here reaches the model exactly when it
//! would from the chat, and is asked about first exactly as there.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::i18n::{t, tn};
use wasm_bindgen::JsCast;
use web_sys::{HtmlElement, HtmlTextAreaElement};
use yew::prelude::*;

use crate::actions::Action;
use crate::model::{AiFailure, Assistant, Family, Member, PackItem};
use crate::pack::Gate;
use crate::store::{Draft, Messages};
use crate::timeline;
use crate::views::bubble::Bubble;
use crate::views::composer::{Composer, Pictures};
use crate::views::consent::AssistantConsentDialog;
use crate::views::conversation::row_key;
use crate::views::stickers::StickerMenu;
use fc_text::assistant_pictures::Candidate;

#[derive(Properties, PartialEq)]
pub struct ThreadPanelProps {
    pub chat_id: i64,
    pub root_id: i64,
    /// The chain as held, root first.
    /// The thread's rows, shared with the store (`store::Messages`).
    pub messages: Messages,
    pub my_user_id: i64,
    /// The chat's kind as the server sent it — a thread lives in the chat
    /// of its root, and offers what that chat offers (fc_text::groups).
    pub chat_kind: AttrValue,
    pub names: HashMap<i64, String>,
    /// The family's roster, for the faces.
    pub members: Vec<Member>,
    /// Whom a mention here may name and a poll counts — the family's
    /// members, or in a group the group's (`Chat::members_of`).
    pub chat_members: Vec<Member>,
    pub assistant: Option<Assistant>,
    pub blocked: HashSet<i64>,
    pub revealed: HashSet<i64>,
    pub revealed_quotes: HashSet<(i64, u8)>,
    pub failed: HashMap<String, String>,
    pub ai_failed: HashMap<i64, AiFailure>,
    /// The reader's family, whose switches decide what may go to the
    /// assistant.
    #[prop_or_default]
    pub family: Option<Family>,
    /// The family's sticker pack, in the order the panel shows it — and
    /// None where there is no sticker button: a server that predates packs,
    /// or an account in no family (as `ConversationProps::stickers`).
    #[prop_or_default]
    pub stickers: Option<Vec<PackItem>>,
    /// Whether this member has agreed that what they send the assistant
    /// may go to the model (docs/protocol.md, "Consenting to the
    /// assistant") — as `ConversationProps::agreed_to_assistant`.
    #[prop_or_default]
    pub agreed_to_assistant: bool,
    /// The text of recordings this member asked for, as this tab holds it
    /// — as `ConversationProps::transcripts`.
    #[prop_or_default]
    pub transcripts: fc_text::transcript::Transcripts,
    pub on_action: Callback<Action>,
}

#[function_component(ThreadPanel)]
pub fn thread_panel(props: &ThreadPanelProps) -> Html {
    let chat_id = props.chat_id;
    let root_id = props.root_id;
    let assistant_user_id = props.assistant.as_ref().map(|assistant| assistant.user_id);
    let close = {
        let on_action = props.on_action.clone();
        Callback::from(move |_: MouseEvent| on_action.emit(Action::CloseThread))
    };
    // Esc closes the thread — but never over a draft (#106, 2.3): closing
    // would lose it, so with words in the box the first Esc only leaves the
    // box, onto the panel, and a second closes it.
    let panel_ref = use_node_ref();
    let on_key = {
        let on_action = props.on_action.clone();
        let panel_ref = panel_ref.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() != "Escape" {
                return;
            }
            let drafted = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlTextAreaElement>().ok())
                .filter(|area| !area.value().trim().is_empty());
            if let Some(area) = drafted {
                event.prevent_default();
                let _ = area.blur();
                if let Some(panel) = panel_ref.cast::<HtmlElement>() {
                    let _ = panel.focus();
                }
                return;
            }
            on_action.emit(Action::CloseThread);
        })
    };
    // Every send from here answers the root.
    let on_send = {
        let on_action = props.on_action.clone();
        Callback::from(move |mut draft: Draft| {
            draft.reply_to_message_id = Some(root_id);
            on_action.emit(Action::Send { chat_id, draft });
        })
    };
    // The consent screen, raised from this surface as the chat raises it
    // (views/conversation.rs): by a send — words, or a sticker — that
    // would reach the model before this member has agreed.
    let consent_open = use_state(|| false);
    // The sticker button, inside the field as in the chat (#106), wherever
    // the chat has one — every chat on a server that has packs, the
    // assistant's included. One click sends,
    // and what it sends answers the root like everything else here. In a
    // thread of the assistant's chat it goes through the consent question
    // and never round it, by the chat's own test (`pack::send_gate`);
    // where the server names nobody nothing is sent, and the strip above
    // the box already says so.
    let sticker_gate = crate::pack::send_gate(
        fc_text::groups::known(&props.chat_kind),
        props.assistant.as_ref(),
        props.agreed_to_assistant,
    );
    let sticker_menu = props.stickers.clone().map(|items| {
        let on_pick = {
            let on_action = props.on_action.clone();
            let consent_open = consent_open.clone();
            Callback::from(move |item_id: i64| match sticker_gate {
                Gate::Send => on_action.emit(Action::SendSticker {
                    chat_id,
                    item_id,
                    reply_to_message_id: Some(root_id),
                }),
                Gate::Ask => consent_open.set(true),
                Gate::Withheld => {}
            })
        };
        html! {
            <StickerMenu {items} busy={None::<String>} {on_pick} on_busy={Callback::noop()} />
        }
    });
    // "Show text" under a recording: a thread's rows are the chat's, and
    // so is what they offer (`Transcription`).
    let transcription =
        crate::views::bubble::Transcription::of(props.assistant.as_ref(), props.family.as_ref(), {
            let consent_open = consent_open.clone();
            Callback::from(move |()| consent_open.set(true))
        });
    let consent_dialog = consent_open
        .then(|| {
            props
                .assistant
                .as_ref()
                .and_then(|assistant| assistant.processor.clone())
        })
        .flatten()
        .map(|processor| {
            let on_agree = {
                let on_action = props.on_action.clone();
                let consent_open = consent_open.clone();
                Callback::from(move |with_lookups: bool| {
                    consent_open.set(false);
                    on_action.emit(Action::agreement(with_lookups));
                })
            };
            let on_cancel = {
                let consent_open = consent_open.clone();
                Callback::from(move |_: ()| consent_open.set(false))
            };
            let family = props.family.as_ref();
            html! {
                <AssistantConsentDialog
                    {processor}
                    family_history={family.is_some_and(|family| family.ai_history)}
                    family_vision={family.is_some_and(|family| family.ai_vision)}
                    transcribe={props.assistant.as_ref().is_some_and(|assistant| assistant.transcribe)}
                    lookups={props.assistant.as_ref().map(|assistant| assistant.lookups.clone()).unwrap_or_default()}
                    {on_agree}
                    {on_cancel}
                />
            }
        });
    let noop_jump = Callback::from(|_: i64| {});
    // Every send here replies to the root, so an `@ai` here is pointed at
    // the ROOT'S photos — and says so, as the chat's own composer does
    // (docs/protocol.md, "What a client's family-chat composer must say").
    let pictures = Pictures {
        staged: Vec::new(),
        quoted: props
            .messages
            .iter()
            .find(|message| message.id == root_id)
            .map(|root| {
                root.attachments()
                    .iter()
                    .map(|attachment| {
                        Candidate::of_attachment(
                            &attachment.kind,
                            attachment.mime.as_deref().unwrap_or(""),
                            attachment.size.map(|size| size.max(0) as u64),
                            attachment.has_preview,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default(),
        server_can_see: props
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.vision),
        server_can_draw: props.assistant.as_ref().is_some_and(|assistant| {
            assistant.images
                && assistant
                    .draw
                    .as_deref()
                    .unwrap_or(fc_text::assistant::DRAW_TOKEN)
                    == fc_text::assistant::DRAW_TOKEN
        }),
        family_allows: props.family.as_ref().is_some_and(|family| family.ai_vision),
        family_history: props.family.as_ref().is_none_or(|family| family.ai_history),
        family_history_photos: props
            .family
            .as_ref()
            .is_some_and(|family| family.ai_history_photos),
    };
    // "Reply" on any row here is the composer below: whatever the row, a
    // reply from this surface answers the root (ios ThreadView focuses its
    // composer).
    let focus = use_state(|| 0u32);
    let on_reply = {
        let focus = focus.clone();
        Callback::from(move |_: i64| focus.set(*focus + 1))
    };
    let replies = props.messages.len().saturating_sub(1);
    // Once a draw, for every row (conversation::QuotedKinds), and the shared
    // sets every bubble is handed.
    let quoted = crate::views::conversation::QuotedKinds::of(&props.messages);
    let names = Rc::new(props.names.clone());
    let blocked = Rc::new(props.blocked.clone());
    let member_ids: Rc<HashSet<i64>> =
        Rc::new(props.chat_members.iter().map(|member| member.id).collect());
    html! {
        <aside ref={panel_ref} class="thread-panel" aria-label={t("Thread")} tabindex="-1" onkeydown={on_key}>
            <header class="thread-bar">
                <h2>{ tn("%lld replies", replies as i64) }</h2>
                <button class="link" onclick={close} aria-label={t("Close the thread")}>{ "✕" }</button>
            </header>
            <div class="thread-messages">
                { for props.messages.iter().map(|message| {
                    let hidden = timeline::is_hidden_by_block(message, props.my_user_id, &props.blocked)
                        && !props.revealed.contains(&message.id);
                    let failed = message
                        .client_msg_id
                        .as_ref()
                        .and_then(|id| props.failed.get(id))
                        .cloned()
                        .filter(|_| message.id == 0);
                    html! {
                        <div key={row_key(message, props.my_user_id)} class={classes!("row", (message.id == root_id).then_some("is-root"))}>
                            <Bubble
                                message={message.clone()}
                                my_user_id={props.my_user_id}
                                names={names.clone()}
                                blocked={blocked.clone()}
                                quote_revealed={props.revealed_quotes.contains(&(message.id, 0))}
                                parent_revealed={props.revealed_quotes.contains(&(message.id, 1))}
                                quote_round={quoted.round(message)}
                                quote_animation={quoted.animation(message)}
                                {hidden}
                                shows_sender={fc_text::groups::offers(&props.chat_kind).sender_names && message.sender_id != props.my_user_id && !hidden}
                                sender_avatar_version={props.members.iter().find(|member| member.id == message.sender_id).map_or(0, |member| member.avatar_version)}
                                run_end={true}
                                seen={false}
                                awaited={false}
                                ai_failed={props.ai_failed.get(&message.id).copied()}
                                {failed}
                                chat_kind={props.chat_kind.clone()}
                                {assistant_user_id}
                                in_thread={true}
                                member_count={props.chat_members.len()}
                                member_ids={member_ids.clone()}
                                on_action={props.on_action.clone()}
                                on_reply={on_reply.clone()}
                                on_edit={Callback::from(|_: i64| {})}
                                on_report={Callback::from(|_: (i64, Option<i64>)| {})}
                                on_jump={noop_jump.clone()}
                                transcription={transcription.clone()}
                                transcripts={props.transcripts.of(message.attachments().iter().map(|attachment| attachment.id))}
                            />
                        </div>
                    }
                }) }
            </div>
            <Composer
                key={format!("thread-composer-{root_id}")}
                {chat_id}
                chat_kind={props.chat_kind.clone()}
                my_user_id={props.my_user_id}
                members={props.chat_members.clone()}
                blocked={props.blocked.clone()}
                assistant={props.assistant.clone()}
                agreed_to_assistant={props.agreed_to_assistant}
                on_review_consent={{
                    let consent_open = consent_open.clone();
                    Callback::from(move |_: ()| consent_open.set(true))
                }}
                replying={None}
                editing={None}
                initial={String::new()}
                {on_send}
                on_save_edit={Callback::from(|_: (i64, String)| {})}
                on_cancel={Callback::from(|_: ()| {})}
                on_typing={props.on_action.reform(move |_: ()| Action::Typing { chat_id })}
                on_draft={Callback::from(|_: String| {})}
                in_thread={true}
                stickers={sticker_menu.unwrap_or_default()}
                focus={*focus}
                {pictures}
            />
            { consent_dialog.unwrap_or_default() }
        </aside>
    }
}

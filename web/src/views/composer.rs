//! The box a message is written in — with the rules of the Apple app's
//! composer (fc_text::mentions, ::assistant, ::composer): the "@" that
//! offers the roster, who a text names when Send is pressed, the `@ai` and
//! `/draw` buttons, the 4,000-character limit, and the reply and edit
//! banners.

use std::collections::HashSet;

use fc_text::{assistant, composer, mentions};
use web_sys::HtmlTextAreaElement;
use yew::prelude::*;

use crate::model::{Assistant, Member, Mention};
use crate::store::Draft;

/// The most members one message may name — the server refuses more with
/// `validation`, and neither app caps, so theirs would fail. Here the
/// first twenty named, in the order they appear, are the list.
pub const MAX_MENTIONS: usize = 20;

/// What a reply banner shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Replying {
    pub message_id: i64,
    pub name: String,
    /// Empty when the quoted row is hidden behind a block.
    pub excerpt: String,
}

/// The message being edited.
#[derive(Debug, Clone, PartialEq)]
pub struct Editing {
    pub message_id: i64,
    pub body: String,
}

#[derive(Properties, PartialEq)]
pub struct ComposerProps {
    pub chat_id: i64,
    pub is_family_chat: bool,
    pub is_ai_chat: bool,
    pub my_user_id: i64,
    pub members: Vec<Member>,
    pub blocked: HashSet<i64>,
    pub assistant: Option<Assistant>,
    pub replying: Option<Replying>,
    pub editing: Option<Editing>,
    /// What was being typed here when the reader last left.
    pub initial: String,
    pub on_send: Callback<Draft>,
    pub on_save_edit: Callback<(i64, String)>,
    /// Esc, or a banner's ✕: the reply or the edit is given up.
    pub on_cancel: Callback<()>,
    pub on_typing: Callback<()>,
    /// The words in the box when the composer goes, kept for the reader's
    /// return.
    pub on_draft: Callback<String>,
    pub on_new_poll: Callback<()>,
    /// On the thread surface: no polls, no edits.
    #[prop_or_default]
    pub in_thread: bool,
    /// Moves the cursor into the box each time it changes — a row's
    /// "Reply" on a surface whose every send is already a reply.
    #[prop_or_default]
    pub focus: u32,
}

/// Who the text names, resolved against the whole live roster at send —
/// a name typed by hand mentions too, a name deleted after picking does
/// not — and only in the family chat, where a mention means anything.
pub fn resolve_mentions(body: &str, members: &[Member], is_family_chat: bool) -> Vec<Mention> {
    if !is_family_chat {
        return Vec::new();
    }
    let roster: Vec<mentions::Member> = members
        .iter()
        .filter(|member| !member.deleted)
        .map(|member| mentions::Member {
            user_id: member.id,
            name: &member.display_name,
        })
        .collect();
    mentions::resolve(body, &roster)
        .into_iter()
        .take(MAX_MENTIONS)
        .map(|member| Mention {
            user_id: member.user_id,
            name: member.name.to_string(),
        })
        .collect()
}

#[function_component(Composer)]
pub fn composer(props: &ComposerProps) -> Html {
    let text = use_state(|| props.initial.clone());
    let notice = use_state(|| Option::<String>::None);
    let active = use_state(|| 0usize);
    let area = use_node_ref();
    // The draft in progress when an edit began — set aside, and back when
    // the edit is done, the way the apps do it.
    let aside = use_mut_ref(|| Option::<String>::None);
    // What is in the box, for the unmount below to hand back.
    let latest = use_mut_ref(String::new);
    *latest.borrow_mut() = (*text).clone();

    {
        let text = text.clone();
        let aside = aside.clone();
        let editing = props.editing.clone();
        use_effect_with(
            editing.as_ref().map(|edit| edit.message_id),
            move |_| match editing {
                Some(edit) => {
                    if aside.borrow().is_none() {
                        *aside.borrow_mut() = Some((*text).clone());
                    }
                    text.set(edit.body);
                }
                None => {
                    if let Some(kept) = aside.borrow_mut().take() {
                        text.set(kept);
                    }
                }
            },
        );
    }
    {
        let on_draft = props.on_draft.clone();
        let latest = latest.clone();
        let aside = aside.clone();
        use_effect_with((), move |_| {
            move || {
                // Leaving mid-edit keeps the draft that was set aside, not
                // the edited message's own words.
                let kept = aside
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| latest.borrow().clone());
                on_draft.emit(kept);
            }
        });
    }
    // The reply banner puts the cursor where the answer goes — and so does
    // a surface asking for it.
    {
        let area = area.clone();
        use_effect_with(
            (
                props.replying.as_ref().map(|reply| reply.message_id),
                props.focus,
            ),
            move |(replying, focus)| {
                if replying.is_some() || *focus > 0 {
                    if let Some(area) = area.cast::<HtmlTextAreaElement>() {
                        let _ = area.focus();
                    }
                }
            },
        );
    }

    let editing = props.editing.clone();
    let suggestions: Vec<Member> = if props.is_family_chat && editing.is_none() {
        match mentions::query(&text) {
            Some(query) => {
                let roster: Vec<mentions::Member> = props
                    .members
                    .iter()
                    .filter(|member| !member.deleted)
                    .map(|member| mentions::Member {
                        user_id: member.id,
                        name: &member.display_name,
                    })
                    .collect();
                let mut excluding: Vec<i64> = props.blocked.iter().copied().collect();
                excluding.push(props.my_user_id);
                let offered: HashSet<i64> = mentions::candidates(&roster, query, &excluding)
                    .into_iter()
                    .map(|member| member.user_id)
                    .collect();
                props
                    .members
                    .iter()
                    .filter(|member| offered.contains(&member.id))
                    .cloned()
                    .collect()
            }
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };

    let accept = {
        let text = text.clone();
        let area = area.clone();
        Callback::from(move |name: String| {
            text.set(mentions::accept(&text, &name));
            if let Some(area) = area.cast::<HtmlTextAreaElement>() {
                let _ = area.focus();
            }
        })
    };

    let send = {
        let text = text.clone();
        let notice = notice.clone();
        let on_send = props.on_send.clone();
        let on_save_edit = props.on_save_edit.clone();
        let on_cancel = props.on_cancel.clone();
        let members = props.members.clone();
        let is_family = props.is_family_chat;
        let editing = editing.clone();
        Callback::from(move |_: ()| {
            let Some(body) = composer::trimmed_for_send(&text).map(str::to_string) else {
                return;
            };
            notice.set(None);
            if let Some(edit) = &editing {
                // Saving what was already there is done at once: nothing to
                // send, and nothing to stay in edit mode for.
                if body == edit.body {
                    on_cancel.emit(());
                } else {
                    on_save_edit.emit((edit.message_id, body));
                }
                return;
            }
            let mentioned = resolve_mentions(&body, &members, is_family);
            text.set(String::new());
            on_send.emit(Draft {
                body,
                mentions: mentioned,
                ..Draft::default()
            });
        })
    };

    let on_input = {
        let text = text.clone();
        let notice = notice.clone();
        let on_typing = props.on_typing.clone();
        let active = active.clone();
        Callback::from(move |event: InputEvent| {
            let area: HtmlTextAreaElement = event.target_unchecked_into();
            let value = area.value();
            // Over the limit — a paste, usually — is cut between Characters
            // and said so, rather than refused later by the server.
            let value = match composer::clamping(&value) {
                Some(clamped) => {
                    notice.set(Some(composer::Notice::Clamped.english()));
                    let clamped = clamped.to_string();
                    area.set_value(&clamped);
                    clamped
                }
                None => value,
            };
            // Only when there is something to be typing. Clearing the box is
            // not typing, and a frame for it would tell the family somebody
            // is writing when they have just given up.
            if !value.trim().is_empty() {
                on_typing.emit(());
            }
            active.set(0);
            text.set(value);
        })
    };

    // Enter sends, Shift+Enter makes a line, Esc gives up a reply or an
    // edit — and while the roster is offered, the arrows walk it and Enter
    // or Tab takes the highlighted name.
    let on_key = {
        let send = send.clone();
        let accept = accept.clone();
        let active = active.clone();
        let on_cancel = props.on_cancel.clone();
        let offered: Vec<String> = suggestions
            .iter()
            .map(|member| member.display_name.clone())
            .collect();
        Callback::from(move |event: KeyboardEvent| {
            let key = event.key();
            if !offered.is_empty() {
                match key.as_str() {
                    "ArrowDown" => {
                        event.prevent_default();
                        active.set((*active + 1) % offered.len());
                        return;
                    }
                    "ArrowUp" => {
                        event.prevent_default();
                        active.set((*active + offered.len() - 1) % offered.len());
                        return;
                    }
                    "Enter" | "Tab" if !event.shift_key() => {
                        event.prevent_default();
                        accept.emit(offered[(*active).min(offered.len() - 1)].clone());
                        return;
                    }
                    _ => {}
                }
            }
            if key == "Enter" && !event.shift_key() && !event.is_composing() {
                event.prevent_default();
                send.emit(());
            } else if key == "Escape" {
                on_cancel.emit(());
            }
        })
    };

    let ask_assistant = {
        let text = text.clone();
        Callback::from(move |_: MouseEvent| text.set(assistant::with_assistant_mention(&text)))
    };
    let ask_picture = {
        let text = text.clone();
        Callback::from(move |_: MouseEvent| text.set(assistant::with_draw_token(&text)))
    };
    let cancel = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |_: MouseEvent| on_cancel.emit(()))
    };

    let has_assistant = props.assistant.is_some();
    let can_draw = props.is_ai_chat
        && props
            .assistant
            .as_ref()
            .is_some_and(|assistant| assistant.images);
    let offers_ai = props.is_family_chat && has_assistant && editing.is_none();
    let offers_poll = props.is_family_chat && !props.in_thread && editing.is_none();
    let empty = composer::trimmed_for_send(&text).is_none();

    html! {
        <div class="composer-wrap">
            if let Some(reply) = props.replying.clone() {
                <div class="composer-banner">
                    <span class="banner-text">
                        { format!("Replying to {}", reply.name) }
                        if !reply.excerpt.is_empty() { { format!(": {}", reply.excerpt) } }
                    </span>
                    <button class="link" onclick={cancel.clone()} aria-label="Cancel reply">{ "✕" }</button>
                </div>
            }
            if editing.is_some() {
                <div class="composer-banner">
                    <span class="banner-text">{ "Editing message" }</span>
                    <button class="link" onclick={cancel} aria-label="Cancel editing">{ "✕" }</button>
                </div>
            }
            if let Some(message) = (*notice).clone() {
                <p class="composer-notice" role="status">{ message }</p>
            }
            if !suggestions.is_empty() {
                <div class="suggestions" role="listbox" aria-label="Members">
                    { for suggestions.iter().enumerate().map(|(index, member)| {
                        let accept = accept.clone();
                        let name = member.display_name.clone();
                        html! {
                            <button
                                role="option"
                                class={classes!((index == *active).then_some("is-active"))}
                                aria-selected={(index == *active).to_string()}
                                onmousedown={Callback::from(move |event: MouseEvent| {
                                    event.prevent_default();
                                    accept.emit(name.clone());
                                })}
                            >
                                { &member.display_name }
                                if !member.username.is_empty() {
                                    <span class="meta">{ format!("  @{}", member.username) }</span>
                                }
                            </button>
                        }
                    }) }
                </div>
            }
            <div class="composer">
                if offers_poll {
                    <button class="tool" title="New poll" aria-label="New poll"
                        onclick={let on_new_poll = props.on_new_poll.clone(); Callback::from(move |_: MouseEvent| on_new_poll.emit(()))}>
                        { "📊" }
                    </button>
                }
                if offers_ai {
                    <button class="tool" title="Ask the assistant" aria-label="Ask the assistant" onclick={ask_assistant}>{ "✨" }</button>
                }
                if can_draw && editing.is_none() {
                    <button class="tool" title="Ask for a picture" aria-label="Ask for a picture" onclick={ask_picture}>{ "🎨" }</button>
                }
                <textarea
                    ref={area}
                    aria-label="Message"
                    rows="2"
                    value={(*text).clone()}
                    oninput={on_input}
                    onkeydown={on_key}
                />
                <button
                    onclick={let send = send.clone(); Callback::from(move |_: MouseEvent| send.emit(()))}
                    disabled={empty}
                >
                    { if editing.is_some() { "Save" } else { "Send" } }
                </button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    fn member(id: i64, name: &str) -> Member {
        Member {
            id,
            display_name: name.into(),
            username: name.to_lowercase(),
            role: Some("member".into()),
            deleted: false,
        }
    }

    /// At send, the text is resolved against the roster — longest name
    /// first, so "@Anna Lee" is Anna Lee and not also Anna — and only in
    /// the family chat.
    #[wasm_bindgen_test]
    fn mentions_are_resolved_from_the_text_at_send() {
        let roster = vec![member(9, "Anna"), member(12, "Anna Lee"), member(7, "Me")];
        let found = resolve_mentions("@Anna Lee and @Anna, dinner?", &roster, true);
        assert_eq!(
            found,
            vec![
                Mention {
                    user_id: 12,
                    name: "Anna Lee".into()
                },
                Mention {
                    user_id: 9,
                    name: "Anna".into()
                },
            ]
        );
        assert!(
            resolve_mentions("@Anna", &roster, false).is_empty(),
            "a direct chat names nobody"
        );
        assert!(resolve_mentions("no names here", &roster, true).is_empty());
    }

    /// The server refuses a twenty-first; so this never sends one.
    #[wasm_bindgen_test]
    fn no_more_than_twenty_are_named() {
        let roster: Vec<Member> = (1..=25).map(|n| member(n, &format!("M{n:02}"))).collect();
        let body: String = (1..=25).map(|n| format!("@M{n:02} ")).collect();
        assert_eq!(resolve_mentions(&body, &roster, true).len(), MAX_MENTIONS);
    }

    /// Saving an edit that changed nothing is done at once — nothing to
    /// send, and no edit mode to be stuck in.
    #[wasm_bindgen_test]
    async fn saving_an_unchanged_edit_leaves_edit_mode_and_sends_nothing() {
        use std::cell::{Cell, RefCell};
        use std::rc::Rc;
        use wasm_bindgen::JsCast;
        use web_sys::HtmlElement;

        let cancelled = Rc::new(Cell::new(0));
        let saved = Rc::new(RefCell::new(Vec::new()));
        let props = ComposerProps {
            chat_id: 42,
            is_family_chat: true,
            is_ai_chat: false,
            my_user_id: 7,
            members: Vec::new(),
            blocked: HashSet::new(),
            assistant: None,
            replying: None,
            editing: Some(Editing {
                message_id: 5,
                body: "hello".into(),
            }),
            initial: String::new(),
            on_send: Callback::noop(),
            on_save_edit: {
                let saved = saved.clone();
                Callback::from(move |edit: (i64, String)| saved.borrow_mut().push(edit))
            },
            on_cancel: {
                let cancelled = cancelled.clone();
                Callback::from(move |_: ()| cancelled.set(cancelled.get() + 1))
            },
            on_typing: Callback::noop(),
            on_draft: Callback::noop(),
            on_new_poll: Callback::noop(),
            in_thread: false,
            focus: 0,
        };
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle = yew::Renderer::<Composer>::with_root_and_props(root.clone(), props).render();
        gloo_timers::future::TimeoutFuture::new(20).await;
        let save = || {
            let buttons = root.query_selector_all(".composer button").unwrap();
            let last = buttons.item(buttons.length() - 1).unwrap();
            assert_eq!(last.text_content().unwrap_or_default(), "Save");
            last.dyn_into::<HtmlElement>().unwrap().click();
        };

        save();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(cancelled.get(), 1);
        assert!(saved.borrow().is_empty());

        let area: HtmlTextAreaElement = root
            .query_selector("textarea")
            .unwrap()
            .unwrap()
            .dyn_into()
            .unwrap();
        area.set_value("hello!");
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        gloo_timers::future::TimeoutFuture::new(20).await;
        save();
        gloo_timers::future::TimeoutFuture::new(20).await;
        assert_eq!(*saved.borrow(), vec![(5, "hello!".to_string())]);
        assert_eq!(cancelled.get(), 1);

        handle.destroy();
        root.remove();
    }
}

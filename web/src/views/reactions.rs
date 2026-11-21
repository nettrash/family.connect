//! Reactions: the chips under a bubble, "See who reacted", and the full
//! "More reactions…" catalogue — drawn from the rules ported from the Apple
//! app (fc_text::reactions, fc_text::emoji), so a message's reactions read
//! the same everywhere.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use fc_text::emoji::EMOJI_CATALOG;
pub use fc_text::emoji::QUICK_REACTIONS;
use fc_text::emoji_search::{self, Annotations};
use fc_text::i18n::{t, Lang};
use fc_text::reactions::{self as rules, ReactionDetail};
use futures::future::{select, Either, LocalBoxFuture, Shared};
use futures::FutureExt;
use gloo_net::http::Request;
use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::{AbortController, AbortSignal, HtmlElement, HtmlInputElement};
use yew::prelude::*;

use crate::model::Reaction;

/// One chip under a bubble.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub emoji: String,
    pub count: usize,
    pub mine: bool,
}

fn to_rules(reactions: &[Reaction]) -> Vec<rules::Reaction> {
    reactions
        .iter()
        .map(|reaction| rules::Reaction::new(reaction.user_id, &reaction.emoji))
        .collect()
}

/// The chips, in the order each emoji was first chosen — not by popularity,
/// which would reshuffle them under the reader — each counting everybody,
/// a blocked member included.
pub fn chips(reactions: &[Reaction], me: i64) -> Vec<Chip> {
    rules::reaction_chips(&to_rules(reactions), me)
        .into_iter()
        .map(|chip| Chip {
            emoji: chip.emoji,
            count: chip.count,
            mine: chip.includes_me,
        })
        .collect()
}

/// The rows of "See who reacted": the chips' order, "You" first, and a
/// blocked reactor left out of the NAMES (the chip still counts them).
pub fn details(
    reactions: &[Reaction],
    names: &HashMap<i64, String>,
    me: i64,
    blocked: &HashSet<i64>,
) -> Vec<ReactionDetail> {
    rules::reaction_details(&to_rules(reactions), names, me, blocked)
}

#[derive(Properties, PartialEq)]
pub struct EmojiPickerProps {
    pub my_user_id: i64,
    pub on_pick: Callback<String>,
    pub on_close: Callback<()>,
}

type LocaleLoad = Shared<LocalBoxFuture<'static, Option<Rc<Annotations>>>>;

thread_local! {
    // Each language shares one request and keeps its parsed catalogue for the page.
    static LOCALE_LOADS: RefCell<HashMap<Lang, LocaleLoad>> = RefCell::new(HashMap::new());
}

fn locale_url(lang: Lang) -> String {
    format!(
        "emoji-search/{}.tsv?v={}",
        lang.tag(),
        env!("FC_EMOJI_SEARCH_VERSION")
    )
}

async fn read_locale(lang: Lang, signal: AbortSignal) -> Option<Annotations> {
    let response = Request::get(&locale_url(lang))
        .abort_signal(Some(&signal))
        .send()
        .await
        .ok()?;
    if !response.ok() {
        return None;
    }
    Annotations::parse(lang, &response.text().await.ok()?)
}

async fn fetch_locale(lang: Lang) -> Option<Rc<Annotations>> {
    let abort = AbortController::new().ok()?;
    match select(
        Box::pin(read_locale(lang, abort.signal())),
        TimeoutFuture::new(10_000),
    )
    .await
    {
        Either::Left((result, _)) => result.map(Rc::new),
        Either::Right(_) => {
            abort.abort();
            None
        }
    }
}

fn start_locale_load(lang: Lang) -> LocaleLoad {
    LOCALE_LOADS.with(|loads| {
        loads
            .borrow_mut()
            .entry(lang)
            .or_insert_with(|| {
                async move {
                    let result = fetch_locale(lang).await;
                    // A later opening can retry a failed or timed-out request.
                    if result.is_none() {
                        LOCALE_LOADS.with(|loads| loads.borrow_mut().remove(&lang));
                    }
                    result
                }
                .boxed_local()
                .shared()
            })
            .clone()
    })
}

#[hook]
fn use_locale(lang: Lang) -> (Option<Rc<Annotations>>, bool) {
    let finished = use_state(|| None);
    let loaded = LOCALE_LOADS.with(|loads| {
        loads
            .borrow()
            .get(&lang)
            .and_then(|load| load.peek())
            .cloned()
            .flatten()
    });
    let waiting = lang != Lang::En && loaded.is_none() && *finished != Some(lang);
    use_effect_with(lang, move |&lang| {
        let active = Rc::new(Cell::new(true));
        if lang != Lang::En {
            let active = active.clone();
            spawn_local(async move {
                start_locale_load(lang).await;
                if active.get() {
                    finished.set(Some(lang));
                }
            });
        }
        move || active.set(false)
    });
    (loaded, waiting)
}

/// Search stays above one scrollable catalogue; category shortcuts stay below.
#[function_component(EmojiPicker)]
pub fn emoji_picker(props: &EmojiPickerProps) -> Html {
    let query = use_state(String::new);
    let input = use_node_ref();
    let content = use_node_ref();
    let pending_section = use_mut_ref(|| None::<String>);
    {
        let content = content.clone();
        use_effect_with((*query).clone(), move |_| {
            if let Some(content) = content.cast::<web_sys::Element>() {
                content.set_scroll_top(0);
            }
        });
    }
    {
        let content = content.clone();
        let pending_section = pending_section.clone();
        // The category sections are back in the DOM after clearing a search.
        use_effect(move || {
            let Some(section) = pending_section.borrow_mut().take() else {
                return;
            };
            let Some(content) = content.cast::<web_sys::Element>() else {
                return;
            };
            let selector = format!("[data-emoji-section='{section}']");
            if let Ok(Some(section)) = content.query_selector(&selector) {
                section.scroll_into_view_with_bool(true);
                if let Ok(Some(first)) = section.query_selector("button") {
                    if let Some(button) = first.dyn_ref::<HtmlElement>() {
                        let _ = button.focus();
                    }
                }
            }
        });
    }
    let (local, waiting) = use_locale(fc_text::i18n::lang());
    let local = local.as_deref();
    let recent = crate::session::reaction_recents(props.my_user_id);
    let searching = !query.trim().is_empty();
    let results = emoji_search::search(&query, local);
    let change = {
        let query = query.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            query.set(input.value());
        })
    };
    let clear = {
        let query = query.clone();
        let input = input.clone();
        Callback::from(move |_: MouseEvent| {
            query.set(String::new());
            if let Some(input) = input.cast::<HtmlInputElement>() {
                let _ = input.focus();
            }
        })
    };
    let close = {
        let on_close = props.on_close.clone();
        Callback::from(move |_: MouseEvent| on_close.emit(()))
    };
    let pick = |emoji: &str| {
        let chosen = emoji.to_string();
        let on_pick = props.on_pick.clone();
        let user_id = props.my_user_id;
        Callback::from(move |_: MouseEvent| {
            crate::session::remember_reaction(user_id, &chosen);
            on_pick.emit(chosen.clone());
        })
    };
    let jump = |section: String| {
        let query = query.clone();
        let pending_section = pending_section.clone();
        Callback::from(move |_: MouseEvent| {
            *pending_section.borrow_mut() = Some(section.clone());
            query.set(String::new());
        })
    };
    html! {
        <crate::views::dialog::Modal title={t("More reactions")} class={classes!("emoji-picker")} on_cancel={props.on_close.clone()}>
            <div class="emoji-picker-body">
                <div class="emoji-search">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true">
                        <circle cx="10" cy="10" r="6" /><path d="m15 15 5 5" />
                    </svg>
                    <input ref={input} type="search" value={(*query).clone()} oninput={change}
                        aria-label={t("Search emoji")} placeholder={t("Search emoji")} autocomplete="off" />
                    if !query.is_empty() {
                        <button type="button" class="emoji-clear" aria-label={t("Clear search")} onclick={clear}>{ "×" }</button>
                    }
                </div>
                <div ref={content.clone()} class="emoji-content">
                    if searching {
                        <section aria-label={t("Search results")}>
                            <h3>{ t("Search results") }</h3>
                            if results.is_empty() {
                                <p class="emoji-empty" role="status">{ if waiting { t("Loading…") } else { t("No emoji found") } }</p>
                            }
                            <div class="emoji-grid">
                                { for results.iter().map(|found| {
                                    let name = AttrValue::from(found.name.to_owned());
                                    html! { <button type="button" title={name.clone()} aria-label={name}
                                        data-emoji={found.emoji} onclick={pick(found.emoji)}>{ found.emoji }</button> }
                                }) }
                            </div>
                        </section>
                    } else {
                        if !recent.is_empty() {
                            <section data-emoji-section="recent" aria-label={t("Recent emoji")}>
                                <h3>{ t("Recent emoji") }</h3>
                                <div class="emoji-grid">
                                    { for recent.iter().map(|emoji| {
                                        let name = AttrValue::from(emoji_search::name(emoji, local).unwrap_or(emoji).to_owned());
                                        html! { <button type="button" title={name.clone()} aria-label={name}
                                            data-emoji={emoji.clone()} onclick={pick(emoji)}>{ emoji }</button> }
                                    }) }
                                </div>
                            </section>
                        }
                        { for EMOJI_CATALOG.iter().enumerate().map(|(index, group)| html! {
                            <section data-emoji-section={index.to_string()} aria-label={t(group.name)}>
                                <h3>{ t(group.name) }</h3>
                                <div class="emoji-grid">
                                    { for group.emoji.iter().map(|emoji| {
                                        let name = AttrValue::from(emoji_search::name(emoji, local).unwrap_or(emoji).to_owned());
                                        html! { <button type="button" title={name.clone()} aria-label={name}
                                            data-emoji={*emoji} onclick={pick(emoji)}>{ *emoji }</button> }
                                    }) }
                                </div>
                            </section>
                        }) }
                    }
                </div>
                <nav class="emoji-tabs" aria-label={t("Emoji categories")}>
                    if !recent.is_empty() {
                        <button type="button" title={t("Recent emoji")} aria-label={t("Recent emoji")}
                            onclick={jump("recent".into())}>{ "◷" }</button>
                    }
                    { for EMOJI_CATALOG.iter().enumerate().map(|(index, group)| html! {
                        <button type="button" title={t(group.name)} aria-label={t(group.name)}
                            onclick={jump(index.to_string())}>{ group.emoji[0] }</button>
                    }) }
                </nav>
                <div class="dialog-actions">
                    <button type="button" class="secondary" onclick={close}>{ t("Cancel") }</button>
                </div>
            </div>
        </crate::views::dialog::Modal>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use wasm_bindgen_test::*;
    use web_sys::Element;

    use crate::fake_server::{Answer, FakeServer};

    struct Picker {
        root: Element,
        handle: yew::AppHandle<EmojiPicker>,
        picked: Rc<RefCell<Vec<String>>>,
        cancelled: Rc<Cell<u32>>,
    }

    impl Picker {
        async fn mount(user_id: i64) -> Self {
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let picked = Rc::new(RefCell::new(Vec::new()));
            let cancelled = Rc::new(Cell::new(0));
            let props = EmojiPickerProps {
                my_user_id: user_id,
                on_pick: {
                    let picked = picked.clone();
                    Callback::from(move |emoji| picked.borrow_mut().push(emoji))
                },
                on_close: {
                    let cancelled = cancelled.clone();
                    Callback::from(move |_| cancelled.set(cancelled.get() + 1))
                },
            };
            let handle =
                yew::Renderer::<EmojiPicker>::with_root_and_props(root.clone(), props).render();
            settle().await;
            Self {
                root,
                handle,
                picked,
                cancelled,
            }
        }

        fn input(&self) -> HtmlInputElement {
            self.root
                .query_selector("input[type=search]")
                .unwrap()
                .unwrap()
                .dyn_into()
                .unwrap()
        }

        async fn search(&self, query: &str) {
            let input = self.input();
            input.set_value(query);
            input
                .dispatch_event(&web_sys::InputEvent::new("input").unwrap())
                .unwrap();
            settle().await;
        }

        fn click(&self, selector: &str) {
            self.root
                .query_selector(selector)
                .unwrap()
                .unwrap()
                .dyn_into::<HtmlElement>()
                .unwrap()
                .click();
        }

        fn finish(self) {
            self.handle.destroy();
            self.root.remove();
        }
    }

    async fn settle() {
        gloo_timers::future::TimeoutFuture::new(20).await;
    }

    #[wasm_bindgen_test]
    async fn search_empty_state_clear_and_choice_reach_the_callback() {
        fc_text::i18n::use_lang(fc_text::i18n::Lang::En);
        let picker = Picker::mount(0).await;
        assert_eq!(
            picker
                .root
                .query_selector_all(".emoji-content section")
                .unwrap()
                .length(),
            8
        );
        picker.search("no-such-emoji-8372").await;
        assert_eq!(
            picker
                .root
                .query_selector_all(".emoji-content button")
                .unwrap()
                .length(),
            0
        );
        assert_eq!(
            picker
                .root
                .query_selector("[role=status]")
                .unwrap()
                .unwrap()
                .text_content()
                .unwrap(),
            "No emoji found"
        );
        picker.click(".emoji-clear");
        settle().await;
        assert_eq!(picker.input().value(), "");
        assert_eq!(
            picker
                .root
                .query_selector_all(".emoji-content section")
                .unwrap()
                .length(),
            8
        );
        picker.search("  :HEART:  ").await;
        let first = picker
            .root
            .query_selector(".emoji-content button")
            .unwrap()
            .unwrap();
        assert_eq!(first.get_attribute("data-emoji").as_deref(), Some("❤️"));
        let nodes = picker
            .root
            .query_selector_all(".emoji-content button")
            .unwrap();
        let emoji: HashSet<_> = (0..nodes.length())
            .map(|index| {
                nodes
                    .item(index)
                    .unwrap()
                    .dyn_into::<Element>()
                    .unwrap()
                    .get_attribute("data-emoji")
                    .unwrap()
            })
            .collect();
        assert_eq!(emoji.len(), nodes.length() as usize);
        picker.click(".emoji-content [data-emoji='❤️']");
        assert_eq!(&*picker.picked.borrow(), &["❤️"]);
        assert_eq!(picker.cancelled.get(), 0);
        picker.finish();
    }

    #[wasm_bindgen_test]
    async fn replacing_or_clearing_a_search_resets_the_scroll_position() {
        crate::layout_tests::install_stylesheet();
        fc_text::i18n::use_lang(Lang::En);
        let picker = Picker::mount(0).await;
        let content = picker
            .root
            .query_selector(".emoji-content")
            .unwrap()
            .unwrap();

        picker.search("a").await;
        content.set_scroll_top(300);
        assert!(content.scroll_top() > 0);
        picker.search("e").await;
        assert!(content.scroll_height() - content.client_height() >= 300);
        assert_eq!(content.scroll_top(), 0);

        content.set_scroll_top(300);
        assert!(content.scroll_top() > 0);
        picker.click(".emoji-clear");
        settle().await;
        assert_eq!(content.scroll_top(), 0);
        picker.finish();
    }

    #[wasm_bindgen_test]
    async fn category_navigation_clears_query_and_focuses_the_first_emoji() {
        let picker = Picker::mount(0).await;
        picker.search("fire").await;
        picker.click("nav [aria-label='Hearts']");
        settle().await;
        assert_eq!(picker.input().value(), "");
        let first_heart = picker
            .root
            .query_selector("section[aria-label='Hearts'] .emoji-grid button")
            .unwrap()
            .unwrap();
        let document = web_sys::window().unwrap().document().unwrap();
        assert_eq!(document.active_element(), Some(first_heart));
        assert_eq!(
            picker
                .root
                .query_selector_all(".emoji-content section")
                .unwrap()
                .length(),
            8
        );
        assert!(picker.picked.borrow().is_empty());
        assert_eq!(picker.cancelled.get(), 0);
        picker.finish();
    }

    #[wasm_bindgen_test]
    async fn category_shortcuts_with_recents_fit_a_320_pixel_picker() {
        crate::layout_tests::install_stylesheet();
        let user_id = 991003;
        crate::session::remember_reaction(user_id, "🔥");
        let picker = Picker::mount(user_id).await;
        let dialog = picker
            .root
            .query_selector(".emoji-picker")
            .unwrap()
            .unwrap();
        dialog.set_attribute("style", "width: 320px").unwrap();
        let nav = picker.root.query_selector(".emoji-tabs").unwrap().unwrap();
        assert_eq!(nav.query_selector_all("button").unwrap().length(), 9);
        assert_eq!(dialog.client_width(), 320);
        assert_eq!(nav.scroll_width(), nav.client_width());
        picker.finish();
        web_sys::window()
            .unwrap()
            .local_storage()
            .unwrap()
            .unwrap()
            .remove_item("fc.reaction.recents.991003")
            .unwrap();
    }

    #[wasm_bindgen_test]
    async fn picker_uses_localised_names_and_keyboard_dismissal() {
        let server = FakeServer::answering(|_| {
            Answer::Bytes(
                200,
                include_bytes!("../../text/data/emoji-search/ru.tsv").to_vec(),
                "text/tab-separated-values",
            )
        });
        fc_text::i18n::use_lang(Lang::Ru);
        let picker = Picker::mount(0).await;
        assert_eq!(
            picker.input().get_attribute("aria-label").as_deref(),
            Some("Поиск эмодзи")
        );
        let document = web_sys::window().unwrap().document().unwrap();
        assert_eq!(
            document.active_element().unwrap(),
            picker.input().unchecked_into::<Element>()
        );
        picker.search("ОГОНЬ").await;
        let fire = picker
            .root
            .query_selector(".emoji-content [data-emoji='🔥']")
            .unwrap()
            .unwrap();
        assert_eq!(fire.get_attribute("aria-label").as_deref(), Some("огонь"));
        let root = format!("{}/", document.location().unwrap().origin().unwrap());
        assert_eq!(
            server.lines(&[&format!("{root}emoji-search/")]),
            [format!("GET {root}{}", locale_url(Lang::Ru))]
        );
        let init = web_sys::KeyboardEventInit::new();
        init.set_key("Escape");
        init.set_bubbles(true);
        picker
            .input()
            .dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(picker.cancelled.get(), 1);
        assert!(picker.picked.borrow().is_empty());
        picker.finish();
        fc_text::i18n::use_lang(fc_text::i18n::Lang::En);
    }

    #[wasm_bindgen_test]
    async fn english_needs_no_request_and_failed_locale_retries_then_stays_cached() {
        let requests = Rc::new(Cell::new(0));
        let server = FakeServer::answering({
            let requests = requests.clone();
            move |request| {
                if !request.path.ends_with(&locale_url(Lang::Fr)) {
                    return Answer::Nothing;
                }
                let previous = requests.replace(requests.get() + 1);
                if previous == 0 {
                    Answer::Nothing
                } else {
                    Answer::Bytes(
                        200,
                        include_bytes!("../../text/data/emoji-search/fr.tsv").to_vec(),
                        "text/tab-separated-values",
                    )
                }
            }
        });
        fc_text::i18n::use_lang(Lang::En);
        let picker = Picker::mount(0).await;
        let root = format!(
            "{}/",
            web_sys::window().unwrap().location().origin().unwrap()
        );
        let prefix = format!("{root}emoji-search/");
        assert!(server.lines(&[&prefix]).is_empty());
        picker.finish();

        let fire_label = |picker: &Picker| {
            picker
                .root
                .query_selector(".emoji-content [data-emoji='🔥']")
                .unwrap()
                .unwrap()
                .get_attribute("aria-label")
                .unwrap()
        };
        fc_text::i18n::use_lang(Lang::Fr);
        let picker = Picker::mount(0).await;
        picker.search("fire").await;
        assert_eq!(fire_label(&picker), "fire");
        let request = format!("GET {root}{}", locale_url(Lang::Fr));
        assert_eq!(
            server.lines(&[&prefix]).as_slice(),
            std::slice::from_ref(&request)
        );
        picker.finish();

        let picker = Picker::mount(0).await;
        picker.search("feu").await;
        assert_eq!(fire_label(&picker), "feu");
        picker.search("fire").await;
        assert_eq!(fire_label(&picker), "feu");
        assert_eq!(server.lines(&[&prefix]), [request.clone(), request.clone()]);
        picker.finish();

        let picker = Picker::mount(0).await;
        picker.search("feu").await;
        assert_eq!(fire_label(&picker), "feu");
        assert_eq!(server.lines(&[&prefix]), [request.clone(), request]);
        picker.finish();
        fc_text::i18n::use_lang(Lang::En);
    }

    #[wasm_bindgen_test]
    async fn recents_are_per_account_survive_reopening_and_clear_on_logout() {
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        let first_key = "fc.reaction.recents.991001";
        let other_key = "fc.reaction.recents.991002";
        storage.set_item(first_key, "not json").unwrap();
        assert!(crate::session::reaction_recents(991001).is_empty());
        crate::session::remember_reaction(991001, "🔥");
        crate::session::remember_reaction(991002, "❤️");
        let picker = Picker::mount(991001).await;
        let recent = picker
            .root
            .query_selector("[data-emoji-section=recent]")
            .unwrap()
            .unwrap();
        assert_eq!(recent.query_selector_all("button").unwrap().length(), 1);
        assert_eq!(
            recent
                .query_selector("button")
                .unwrap()
                .unwrap()
                .get_attribute("data-emoji")
                .as_deref(),
            Some("🔥")
        );
        picker.search("heart").await;
        assert_eq!(crate::session::reaction_recents(991001), vec!["🔥"]);
        picker.click(".emoji-content [data-emoji='❤️']");
        assert_eq!(crate::session::reaction_recents(991001), vec!["❤️", "🔥"]);
        assert_eq!(crate::session::reaction_recents(991002), vec!["❤️"]);
        picker.finish();
        let reopened = Picker::mount(991001).await;
        assert_eq!(
            reopened
                .root
                .query_selector("[data-emoji-section=recent] button")
                .unwrap()
                .unwrap()
                .get_attribute("data-emoji")
                .as_deref(),
            Some("❤️")
        );
        reopened.finish();
        storage.set_item("fc.emoji-test.keep", "unrelated").unwrap();
        crate::session::clear();
        assert_eq!(storage.get_item(first_key).unwrap(), None);
        assert_eq!(storage.get_item(other_key).unwrap(), None);
        assert_eq!(
            storage.get_item("fc.emoji-test.keep").unwrap().as_deref(),
            Some("unrelated")
        );
        storage.remove_item("fc.emoji-test.keep").unwrap();
        assert!(crate::session::reaction_recents(0).is_empty());
        storage
            .set_item("fc.reaction.recents.-1", "[\"🔥\"]")
            .unwrap();
        assert!(crate::session::reaction_recents(-1).is_empty());
        crate::session::remember_reaction(-1, "❤️");
        assert_eq!(
            storage
                .get_item("fc.reaction.recents.-1")
                .unwrap()
                .as_deref(),
            Some("[\"🔥\"]")
        );
        storage.remove_item("fc.reaction.recents.-1").unwrap();
    }

    fn reaction(user_id: i64, emoji: &str) -> Reaction {
        Reaction {
            user_id,
            emoji: emoji.into(),
        }
    }

    /// The ported rules, through the web's own model type.
    #[wasm_bindgen_test]
    fn chips_keep_first_seen_order_count_everyone_and_mark_mine() {
        let list = vec![reaction(9, "👍"), reaction(7, "❤️"), reaction(11, "👍")];
        let chips = chips(&list, 7);
        assert_eq!(
            chips,
            vec![
                Chip {
                    emoji: "👍".into(),
                    count: 2,
                    mine: false
                },
                Chip {
                    emoji: "❤️".into(),
                    count: 1,
                    mine: true
                },
            ]
        );
        let names = HashMap::from([(9, "Anna".to_string())]);
        let rows = details(&list, &names, 7, &HashSet::from([11]));
        assert_eq!(
            rows[0].names,
            vec!["Anna".to_string()],
            "the blocked reactor is not named"
        );
        assert_eq!(rows[1].names, vec!["You".to_string()]);
    }
}

//! Stickers — the CHAT kind: a small picture sent as its own message, the
//! way a messenger's stickers are (docs/protocol.md, "Sticker pack"). Not
//! the board's notes, which are what the word means in views/board.rs.
//!
//! Four things are drawn here, and the protocol's drawing rules are why
//! each looks the way it does:
//! - a sticker IN A CHAT: no bubble, one fixed box, fitted whole, from the
//!   original bytes — so its transparency shows the chat behind it and an
//!   animated one moves, which an `<img>` does by itself;
//! - the PANEL the composer opens: the family's pack as a grid, where one
//!   click sends — no caption, nothing to confirm;
//! - a sticker SHOWN LARGER, with "Add to family stickers" when the pack
//!   does not hold it;
//! - the pack ON THE FAMILY SCREEN, where it is added to and removed from.
//!
//! None of them ever asks for a preview: a preview is a JPEG, and a sticker
//! drawn from one would be a still picture on a white square.

use fc_text::i18n::{t, t1, t2};
use fc_text::pack as rules;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlInputElement;
use yew::prelude::*;

use crate::actions::Action;
use crate::media::{use_media, MediaLoader, Variant};
use crate::model::{Attachment, PackItem};
use crate::pack::Limits;
use crate::views::dialog::{Confirm, Modal};
use crate::views::quiet::LiveRegion;

/// What a sticker is called where it has to be called something: its label
/// when whoever added it gave one, and the word otherwise.
pub fn spoken(label: Option<&str>) -> String {
    label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map_or_else(|| t("Sticker").to_string(), str::to_string)
}

/// What the button that removes a sticker is called: the sentence whole,
/// in the reader's language, with the label after it when there is one —
/// never a verb glued to a noun, which only English lets anybody do.
pub fn removal(label: Option<&str>) -> String {
    match label.map(str::trim).filter(|label| !label.is_empty()) {
        Some(label) => t1("Remove sticker: %@", label),
        None => t("Remove sticker").to_string(),
    }
}

/// What is wrong with a label as typed, in words — or nothing. At most 64
/// characters, counted the way the SERVER counts them: Unicode scalar
/// values, after trimming (fc_text::pack::label). Said beside the field
/// and before any request: an `<input maxlength>` would count UTF-16 units
/// instead, and cut sixty-four emoji off at thirty-two without a word.
pub fn label_refusal(raw: &str) -> Option<&'static str> {
    rules::label(raw)
        .is_err()
        .then(|| t("A description is at most 64 characters."))
}

#[derive(Properties, PartialEq)]
pub struct TileProps {
    pub attachment: Attachment,
    /// Asked to show it larger.
    pub on_open: Callback<Attachment>,
}

/// A sticker in a chat: the picture alone, in the one box every sticker is
/// drawn in (fc_text::pack::BOX) — larger than an emoji, smaller than a
/// photograph, never at the picture's own pixel size. `object-fit: contain`
/// is "fitted whole and never cropped"; the box has no background, so what
/// is transparent in the picture is the chat.
#[function_component(StickerTile)]
pub fn sticker_tile(props: &TileProps) -> Html {
    let attachment = &props.attachment;
    // The ORIGINAL, whatever `has_preview` says: it can be true by
    // inheritance, and a preview is a JPEG.
    let url = use_media(attachment.id, Variant::Sticker, true);
    let open = {
        let on_open = props.on_open.clone();
        let attachment = attachment.clone();
        Callback::from(move |_: MouseEvent| on_open.emit(attachment.clone()))
    };
    // Waiting for bytes that are on their way — a server id being fetched,
    // and just as much an outbox id whose bytes a reload took and the
    // upload is fetching again (sync.rs): without the spinner that one is
    // an empty box with nothing to say it is coming. A send that FAILED is
    // not waiting for anything, and the stylesheet stops the spinner there
    // (`.bubble.is-failed`).
    let loading = url.is_none();
    let label = t("Sticker");
    html! {
        <button
            class={classes!("chat-sticker", loading.then_some("is-loading"))}
            style={format!("width:{0:.0}px;height:{0:.0}px", rules::BOX)}
            onclick={open}
            // Its own words, whole: "Open %@" with the noun lower-cased is
            // English grammar, and wrong wherever a noun keeps its capital.
            aria-label={t("Open sticker")}
        >
            if let Some(url) = url {
                <img src={url} alt={label} draggable="false" />
            }
        </button>
    }
}

#[derive(Properties, PartialEq)]
pub struct MenuProps {
    /// The pack, in panel order — recently used first (`Pack::panel`).
    pub items: Vec<PackItem>,
    /// A sticker may not be sent right now, and the reason why.
    pub busy: Option<String>,
    /// One was clicked: send it.
    pub on_pick: Callback<i64>,
    /// Said instead of opening the panel when `busy`.
    pub on_busy: Callback<String>,
}

/// The composer's sticker button, and the panel it opens above the box.
#[function_component(StickerMenu)]
pub fn sticker_menu(props: &MenuProps) -> Html {
    let open = use_state(|| false);
    let toggle = {
        let open = open.clone();
        let busy = props.busy.clone();
        let on_busy = props.on_busy.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            if let Some(reason) = busy.clone() {
                on_busy.emit(reason);
                return;
            }
            open.set(!*open);
        })
    };
    let close = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(false))
    };
    let on_key = {
        let open = open.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                event.stop_propagation();
                open.set(false);
            }
        })
    };
    // ONE click sends, and the panel gets out of the way of the message it
    // just sent. There is no second step: a send that needed one would not
    // need a flag.
    let pick = {
        let open = open.clone();
        let on_pick = props.on_pick.clone();
        Callback::from(move |id: i64| {
            open.set(false);
            on_pick.emit(id);
        })
    };
    // Opened, the panel takes the focus, so Escape and Tab reach it.
    {
        let is_open = *open;
        use_effect_with(is_open, move |is_open| {
            if *is_open {
                if let Some(panel) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| document.query_selector(".pack-panel").ok().flatten())
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = panel.focus();
                }
            }
        });
    }
    html! {
        <div class="attach pack-menu">
            <button class="tool" title={t("Stickers")} aria-label={t("Stickers")}
                    aria-haspopup="dialog" aria-expanded={(*open).to_string()} onclick={toggle}>
                { "🙂" }
            </button>
            if *open {
                // A click anywhere else closes it — on a touch screen there
                // is no mouse to leave.
                <div class="menu-backdrop" onclick={close} aria-hidden="true"></div>
                <div class="pack-panel" role="dialog" aria-label={t("Stickers")}
                     tabindex="-1" onkeydown={on_key}>
                    if props.items.is_empty() {
                        <p class="footnote">
                            { t("No stickers yet. Anyone in the family can add one on the Family screen.") }
                        </p>
                    } else {
                        <div class="pack-grid">
                            { for props.items.iter().map(|item| html! {
                                <PanelCell key={item.id} item={item.clone()} on_pick={pick.clone()} />
                            }) }
                        </div>
                    }
                </div>
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct CellProps {
    item: PackItem,
    on_pick: Callback<i64>,
}

/// One sticker of the panel. Not clickable until its picture is here: what
/// is sent is a copy of these very bytes, and a click on an empty square
/// would be a send of something nobody has seen.
#[function_component(PanelCell)]
fn panel_cell(props: &CellProps) -> Html {
    let Some(picture) = props.item.attachment.as_ref() else {
        return Html::default();
    };
    let url = use_media(picture.id, Variant::Sticker, true);
    let id = props.item.id;
    let send = props.on_pick.reform(move |_: MouseEvent| id);
    let name = spoken(props.item.label.as_deref());
    html! {
        <button class={classes!("pack-cell", url.is_none().then_some("is-loading"))}
                onclick={send} disabled={url.is_none()} aria-label={name.clone()} title={name}>
            if let Some(url) = url {
                <img src={url} alt="" draggable="false" />
            }
        </button>
    }
}

#[derive(Properties, PartialEq)]
pub struct ViewProps {
    /// The sticker from the chat.
    pub attachment: Attachment,
    /// The pack items it COULD be a copy of — the same size and type
    /// (`Pack::candidates`). The bytes decide among them, here.
    pub candidates: Vec<PackItem>,
    /// Whether this server has packs and this account a family: without
    /// both there is nothing to add to, and nothing is offered.
    pub offered: bool,
    /// A sticker is already on its way into the pack.
    pub adding: bool,
    pub on_action: Callback<Action>,
}

/// A sticker shown larger — and "Add to family stickers" when the family's
/// pack does not hold it. "Holds it" is decided HERE, from bytes this tab
/// already has: nothing on the wire names the item a message was sent
/// from. A wrong guess costs nothing — the server answers a claim of bytes
/// the pack already holds with the item that was there.
#[function_component(StickerView)]
pub fn sticker_view(props: &ViewProps) -> Html {
    let loader = use_context::<MediaLoader>();
    let attachment = &props.attachment;
    let url = use_media(attachment.id, Variant::Sticker, true);
    // None while the bytes are being compared: nothing is offered, and
    // nothing is claimed, until it is known either way.
    let in_pack = use_state(|| Option::<bool>::None);
    let error = use_state(|| Option::<String>::None);
    let asked = use_state(|| false);
    // The few words it may be given on its way in — adding offers a label
    // by this door as by the Family screen's.
    let label = use_state(String::new);
    {
        let in_pack = in_pack.clone();
        let candidates: Vec<i64> = props
            .candidates
            .iter()
            .filter_map(|item| item.attachment.as_ref().map(|picture| picture.id))
            .collect();
        let sticker = attachment.id;
        use_effect_with(
            (sticker, candidates, url.is_some()),
            move |(sticker, candidates, here)| {
                // Which comparison this is: one overtaken by another — the
                // pack changed under it — says nothing when it lands.
                let current = std::rc::Rc::new(std::cell::Cell::new(true));
                if candidates.is_empty() {
                    in_pack.set(Some(false));
                } else if *here {
                    in_pack.set(None);
                    let (sticker, candidates) = (*sticker, candidates.clone());
                    let still = current.clone();
                    spawn_local(async move {
                        let held = holds(loader, sticker, candidates).await;
                        if still.get() {
                            in_pack.set(Some(held));
                        }
                    });
                }
                move || current.set(false)
            },
        );
    }
    let close = props.on_action.reform(|_: ()| Action::CloseSticker);
    let too_long = label_refusal(&label);
    let on_label = {
        let label = label.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            label.set(input.value());
        })
    };
    let keep = {
        let on_action = props.on_action.clone();
        let attachment = attachment.clone();
        let error = error.clone();
        let asked = asked.clone();
        let label = label.clone();
        Callback::from(move |_: MouseEvent| {
            // Over-long is refused HERE, in words, and nothing is asked of
            // the server.
            if label_refusal(&label).is_some() {
                return;
            }
            error.set(None);
            asked.set(true);
            let error = error.clone();
            let asked = asked.clone();
            on_action.emit(Action::KeepSticker {
                attachment: attachment.clone(),
                label: (*label).clone(),
                done: Callback::from(move |failure: Option<String>| {
                    asked.set(false);
                    error.set(failure);
                }),
            });
        })
    };
    let busy = *asked || props.adding;
    html! {
        <Modal title={t("Sticker")} class="chat-sticker-view" on_cancel={close.clone()} busy={*asked}>
            <div class="chat-sticker-stage">
                if let Some(url) = url {
                    <img src={url} alt={t("Sticker")} draggable="false" />
                } else {
                    <p class="footnote">{ t("Loading…") }</p>
                }
            </div>
            // Quiet while a voice message is being recorded behind it (the
            // plan for #79, S6) — like every live region of the app's.
            if let Some(message) = (*error).clone() {
                <LiveRegion class="error" role="alert">{ message }</LiveRegion>
            }
            if props.offered && *in_pack == Some(true) {
                <p class="footnote">{ t("Already in the family's stickers.") }</p>
            }
            if props.offered && *in_pack == Some(false) {
                <input
                    class="pack-label"
                    type="text"
                    placeholder={t("Description (optional)")}
                    aria-label={t("Description (optional)")}
                    aria-invalid={too_long.is_some().to_string()}
                    value={(*label).clone()}
                    disabled={busy}
                    oninput={on_label}
                />
                if let Some(message) = too_long {
                    <LiveRegion class="error" role="alert">{ message }</LiveRegion>
                }
            }
            <div class="dialog-actions">
                if props.offered && *in_pack == Some(false) {
                    <button class="primary" disabled={busy || too_long.is_some()} onclick={keep}>
                        { if busy { t("Adding…") } else { t("Add to family stickers") } }
                    </button>
                }
                <button class="secondary" disabled={*asked}
                        onclick={close.reform(|_: MouseEvent| ())}>{ t("Close") }</button>
            </div>
        </Modal>
    }
}

/// Whether the pack holds `sticker`: whether any candidate's bytes are its
/// bytes. A picture that cannot be had — either side's — is not a match,
/// which offers the add; the server's `200` is the backstop for that.
async fn holds(loader: Option<MediaLoader>, sticker: i64, candidates: Vec<i64>) -> bool {
    let Some(loader) = loader else {
        return false;
    };
    let Some(mine) = loader.held(sticker, Variant::Sticker) else {
        return false;
    };
    for candidate in candidates {
        let (heard, hearing) = futures::channel::oneshot::channel::<bool>();
        let heard = std::cell::RefCell::new(Some(heard));
        loader.load(
            candidate,
            Variant::Sticker,
            Callback::from(move |url: Option<String>| {
                if let Some(heard) = heard.borrow_mut().take() {
                    let _ = heard.send(url.is_some());
                }
            }),
        );
        if !hearing.await.unwrap_or(false) {
            continue;
        }
        if let Some(theirs) = loader.held(candidate, Variant::Sticker) {
            if crate::prep::same_bytes(&mine, &theirs).await {
                return true;
            }
        }
    }
    false
}

#[derive(Properties, PartialEq)]
pub struct SectionProps {
    /// The pack in the order it was added to (`Pack::listed`).
    pub items: Vec<PackItem>,
    pub limits: Limits,
    pub my_user_id: i64,
    pub owner: bool,
    /// A sticker is on its way in.
    pub adding: bool,
    pub on_action: Callback<Action>,
}

/// The family's pack on the Family screen: everybody may add, and whoever
/// added one — or the family's owner — may remove it. A blocked member's
/// stickers are here like anybody's: an item is a picture the family
/// keeps, not something a person said.
#[function_component(PackSection)]
pub fn pack_section(props: &SectionProps) -> Html {
    let label = use_state(String::new);
    let error = use_state(|| Option::<String>::None);
    let removing = use_state(|| Option::<i64>::None);
    let count = props.items.len();
    let room = rules::has_room(count, props.limits.items);

    let picked = {
        let on_action = props.on_action.clone();
        let label = label.clone();
        let error = error.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let file = input.files().and_then(|list| list.get(0));
            // Emptied once read: picking the same file again is a change.
            input.set_value("");
            let Some(file) = file else {
                return;
            };
            error.set(None);
            let error = error.clone();
            let written = label.clone();
            on_action.emit(Action::AddSticker {
                file,
                label: (*label).clone(),
                done: Callback::from(move |failure: Option<String>| {
                    // The words stay until the sticker is in: a refusal
                    // leaves them to go with the next try.
                    if failure.is_none() {
                        written.set(String::new());
                    }
                    error.set(failure);
                }),
            });
        })
    };
    let on_label = {
        let label = label.clone();
        Callback::from(move |event: InputEvent| {
            let input: HtmlInputElement = event.target_unchecked_into();
            label.set(input.value());
        })
    };
    let confirm = removing.map(|item_id| {
        let remove = {
            let on_action = props.on_action.clone();
            let removing = removing.clone();
            let error = error.clone();
            Callback::from(move |_: ()| {
                removing.set(None);
                error.set(None);
                let error = error.clone();
                on_action.emit(Action::RemoveSticker {
                    item_id,
                    done: Callback::from(move |failure: Option<String>| error.set(failure)),
                });
            })
        };
        let cancel = {
            let removing = removing.clone();
            Callback::from(move |_: ()| removing.set(None))
        };
        html! {
            <Confirm
                title={t("Remove this sticker?")}
                message={t("Messages already sent with it keep it.")}
                confirm={t("Remove")}
                on_confirm={remove}
                on_cancel={cancel}
            />
        }
    });
    // A label over the limit is refused beside the field, in words, and
    // the picker is shut until it fits: nothing is picked, and nothing is
    // sent, for a description the server would refuse.
    let too_long = label_refusal(&label);
    let can_add = room && !props.adding && too_long.is_none();
    html! {
        <section class="group" aria-labelledby="family-stickers">
            <h3 id="family-stickers">{ t("Stickers") }</h3>
            <div class="setting-row">
                <span class="muted">
                    { t2("%lld of %lld", &count.to_string(), &props.limits.items.to_string()) }
                </span>
                <span class="row-actions">
                    <input
                        class="pack-label"
                        type="text"
                        placeholder={t("Description (optional)")}
                        aria-label={t("Description (optional)")}
                        aria-invalid={too_long.is_some().to_string()}
                        value={(*label).clone()}
                        oninput={on_label}
                    />
                    <label class={classes!("button-like", (!can_add).then_some("is-disabled"))}>
                        { if props.adding { t("Adding…") } else { t("Add Sticker…") } }
                        <input
                            type="file"
                            // Anything the browser can decode may be MADE
                            // into a sticker; a WebP or PNG already is one.
                            accept="image/webp,image/png,image/*"
                            class="visually-hidden"
                            disabled={!can_add}
                            onchange={picked}
                        />
                    </label>
                </span>
            </div>
            if !room {
                <p class="footnote">{ crate::actions::pack_full() }</p>
            }
            if let Some(message) = too_long {
                <p class="error" role="alert">{ message }</p>
            }
            if let Some(message) = (*error).clone() {
                <p class="error" role="alert">{ message }</p>
            }
            if !props.items.is_empty() {
                <ul class="pack-grid">
                    { for props.items.iter().map(|item| {
                        let removable = rules::may_remove(item.added_by, props.my_user_id, props.owner);
                        let id = item.id;
                        let ask = {
                            let removing = removing.clone();
                            Callback::from(move |_: MouseEvent| removing.set(Some(id)))
                        };
                        html! {
                            <li key={id} class="pack-item">
                                <PackPicture item={item.clone()} />
                                if removable {
                                    <button class="staged-remove" onclick={ask}
                                            aria-label={removal(item.label.as_deref())}>
                                        { "✕" }
                                    </button>
                                }
                            </li>
                        }
                    }) }
                </ul>
            }
            <p class="footnote">
                { t("Anyone in the family can add a sticker. Whoever added one, or the family owner, can remove it.") }
            </p>
            { confirm.unwrap_or_default() }
        </section>
    }
}

#[derive(Properties, PartialEq)]
struct PictureProps {
    item: PackItem,
}

/// One sticker of the pack, as the Family screen lists it.
#[function_component(PackPicture)]
fn pack_picture(props: &PictureProps) -> Html {
    let Some(picture) = props.item.attachment.as_ref() else {
        return Html::default();
    };
    let url = use_media(picture.id, Variant::Sticker, true);
    let name = spoken(props.item.label.as_deref());
    html! {
        <span class={classes!("pack-cell", url.is_none().then_some("is-loading"))} title={name.clone()}>
            if let Some(url) = url {
                <img src={url} alt={name} draggable="false" />
            }
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use wasm_bindgen_test::*;
    use web_sys::{Blob, Element, HtmlElement};

    use crate::live::{AppState, Live};

    const ME: i64 = 7;
    const ANNA: i64 = 9;

    #[derive(Properties, PartialEq)]
    struct HarnessProps {
        loader: MediaLoader,
        children: Html,
    }

    /// What the app gives every view: the tab's media cache.
    #[function_component(Harness)]
    fn harness(props: &HarnessProps) -> Html {
        html! {
            <ContextProvider<MediaLoader> context={props.loader.clone()}>
                { props.children.clone() }
            </ContextProvider<MediaLoader>>
        }
    }

    fn loader() -> MediaLoader {
        MediaLoader::new(Live::new(
            AppState {
                token: Some("t".into()),
                ..AppState::default()
            },
            Rc::new(|| {}),
        ))
    }

    fn bytes(text: &str) -> Blob {
        let parts = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text));
        Blob::new_with_str_sequence(&parts).expect("a blob")
    }

    fn picture(id: i64, size: i64) -> Attachment {
        Attachment {
            id,
            kind: "photo".into(),
            mime: Some("image/webp".into()),
            size: Some(size),
            ..Attachment::default()
        }
    }

    fn item(id: i64, added_by: i64, label: Option<&str>) -> PackItem {
        PackItem {
            id,
            pack_seq: id,
            added_by: Some(added_by),
            attachment: Some(picture(70 + id, 11)),
            label: label.map(str::to_string),
            ..PackItem::default()
        }
    }

    async fn render(loader: &MediaLoader, children: Html) -> (Element, yew::AppHandle<Harness>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let handle = yew::Renderer::<Harness>::with_root_and_props(
            root.clone(),
            HarnessProps {
                loader: loader.clone(),
                children,
            },
        )
        .render();
        settle().await;
        (root, handle)
    }

    async fn settle() {
        gloo_timers::future::TimeoutFuture::new(40).await;
    }

    fn click(root: &Element, selector: &str) {
        root.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} is on the page"))
            .dyn_into::<HtmlElement>()
            .unwrap()
            .click();
    }

    /// Type into a field the way somebody does: the value, and the event
    /// Yew listens for.
    fn type_into(root: &Element, selector: &str, value: &str) {
        let field: HtmlInputElement = root
            .query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} is on the page"))
            .dyn_into()
            .unwrap();
        field.set_value(value);
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        field
            .dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
    }

    fn count(root: &Element, selector: &str) -> u32 {
        root.query_selector_all(selector).unwrap().length()
    }

    fn text(root: &Element) -> String {
        root.text_content().unwrap_or_default()
    }

    /// The panel is the pack as a grid, and ONE click sends: no caption, no
    /// second step, and the panel gets out of the way. A sticker whose
    /// picture has not arrived is not clickable — what is sent is a copy of
    /// bytes this tab holds.
    #[wasm_bindgen_test]
    async fn one_click_in_the_panel_sends() {
        let loader = loader();
        loader.seed(75, Variant::Sticker, bytes("party cat"));
        let picked = Rc::new(RefCell::new(Vec::new()));
        let on_pick = {
            let picked = picked.clone();
            Callback::from(move |id: i64| picked.borrow_mut().push(id))
        };
        let (root, handle) = render(
            &loader,
            html! {
                <StickerMenu
                    items={vec![item(5, ANNA, Some("party cat")), item(6, ME, None)]}
                    busy={None::<String>}
                    {on_pick}
                    on_busy={Callback::noop()}
                />
            },
        )
        .await;
        assert_eq!(count(&root, ".pack-panel"), 0, "closed until asked for");
        click(&root, ".pack-menu .tool");
        settle().await;
        assert_eq!(count(&root, ".pack-cell"), 2);
        let ready = root
            .query_selector(".pack-cell:not([disabled])")
            .unwrap()
            .expect("the one whose picture is here");
        assert_eq!(
            ready.get_attribute("aria-label").as_deref(),
            Some("party cat"),
            "its label is what a screen reader says"
        );
        assert_eq!(count(&root, ".pack-cell[disabled]"), 1);
        assert_eq!(
            root.query_selector(".pack-cell[disabled]")
                .unwrap()
                .unwrap()
                .get_attribute("aria-label")
                .as_deref(),
            Some("Sticker")
        );
        ready.dyn_into::<HtmlElement>().unwrap().click();
        settle().await;
        assert_eq!(*picked.borrow(), vec![5], "sent by the one click");
        assert_eq!(count(&root, ".pack-panel"), 0, "and the panel is gone");
        assert_eq!(
            count(&root, "textarea, input"),
            0,
            "nothing to caption it with"
        );
        handle.destroy();
        root.remove();
    }

    /// An empty pack says where stickers come from; and while something
    /// else has the composer, the button says why instead of opening.
    #[wasm_bindgen_test]
    async fn an_empty_pack_says_so_and_a_busy_composer_says_why() {
        let loader = loader();
        let (root, handle) = render(
            &loader,
            html! {
                <StickerMenu items={Vec::<PackItem>::new()} busy={None::<String>}
                             on_pick={Callback::noop()} on_busy={Callback::noop()} />
            },
        )
        .await;
        click(&root, ".pack-menu .tool");
        settle().await;
        assert!(text(&root).contains("No stickers yet."), "{}", text(&root));
        assert_eq!(count(&root, ".pack-cell"), 0);
        handle.destroy();
        root.remove();

        let said = Rc::new(RefCell::new(Vec::new()));
        let on_busy = {
            let said = said.clone();
            Callback::from(move |reason: String| said.borrow_mut().push(reason))
        };
        let (root, handle) = render(
            &loader,
            html! {
                <StickerMenu items={vec![item(5, ANNA, None)]} busy={Some("Finish first.".to_string())}
                             on_pick={Callback::noop()} {on_busy} />
            },
        )
        .await;
        click(&root, ".pack-menu .tool");
        settle().await;
        assert_eq!(count(&root, ".pack-panel"), 0);
        assert_eq!(*said.borrow(), vec!["Finish first.".to_string()]);
        handle.destroy();
        root.remove();
    }

    fn section(items: Vec<PackItem>, max: i64, owner: bool, on_action: Callback<Action>) -> Html {
        html! {
            <PackSection
                {items}
                limits={Limits { items: max, bytes: 524_288 }}
                my_user_id={ME}
                {owner}
                adding={false}
                {on_action}
            />
        }
    }

    /// THE PERMISSION RULE: anybody may add; whoever added a sticker, or
    /// the family's owner, may remove it — and nobody else is offered the
    /// button. Removing asks first, and says what it does not take.
    #[wasm_bindgen_test]
    async fn the_adder_or_the_owner_may_remove_and_is_asked_first() {
        let loader = loader();
        let actions = Rc::new(RefCell::new(Vec::new()));
        let on_action = {
            let actions = actions.clone();
            Callback::from(move |action: Action| actions.borrow_mut().push(action))
        };
        let pack = vec![item(5, ANNA, Some("party cat")), item(6, ME, None)];

        let (root, handle) = render(
            &loader,
            section(pack.clone(), 200, false, on_action.clone()),
        )
        .await;
        assert_eq!(
            count(&root, ".pack-item"),
            2,
            "everybody sees the whole pack"
        );
        assert_eq!(
            count(&root, ".pack-item .staged-remove"),
            1,
            "a member: their own"
        );
        assert_eq!(
            root.query_selector(".pack-item .staged-remove")
                .unwrap()
                .unwrap()
                .get_attribute("aria-label")
                .as_deref(),
            Some("Remove sticker")
        );
        assert!(text(&root).contains("2 of 200"), "{}", text(&root));
        assert!(
            root.query_selector("input[type=file]:not([disabled])")
                .unwrap()
                .is_some(),
            "and anybody may add"
        );
        click(&root, ".pack-item .staged-remove");
        settle().await;
        assert!(actions.borrow().is_empty(), "asked first");
        assert!(text(&root).contains("Remove this sticker?"));
        assert!(text(&root).contains("Messages already sent with it keep it."));
        click(&root, ".dialog .danger-button");
        settle().await;
        assert!(
            matches!(
                actions.borrow()[..],
                [Action::RemoveSticker { item_id: 6, .. }]
            ),
            "{:?}",
            actions.borrow()
        );
        handle.destroy();
        root.remove();

        let (root, handle) =
            render(&loader, section(pack.clone(), 200, true, on_action.clone())).await;
        assert_eq!(
            count(&root, ".pack-item .staged-remove"),
            2,
            "the owner: anybody's"
        );
        handle.destroy();
        root.remove();

        // A full pack is refused at the picker, in words.
        let (root, handle) = render(&loader, section(pack, 2, false, on_action)).await;
        assert!(root
            .query_selector("input[type=file][disabled]")
            .unwrap()
            .is_some());
        assert!(text(&root).contains("sticker pack is full"));
        handle.destroy();
        root.remove();
    }

    /// ADDING OFFERS A LABEL, at most 64 characters counted as the server
    /// counts them — scalar values, after trimming — and one that is over
    /// is refused IN WORDS beside the field, with the picker shut, before
    /// anything is picked or sent. Sixty-four emoji are sixty-four: no
    /// `maxlength` cuts them at thirty-two UTF-16 pairs.
    #[wasm_bindgen_test]
    async fn a_label_is_offered_and_an_over_long_one_is_refused_in_words() {
        let loader = loader();
        let (root, handle) = render(
            &loader,
            section(vec![item(5, ANNA, None)], 200, false, Callback::noop()),
        )
        .await;
        let field = root
            .query_selector("input.pack-label")
            .unwrap()
            .expect("the label is offered");
        assert!(
            field.get_attribute("maxlength").is_none(),
            "counted here, not by the browser's UTF-16 units"
        );
        let picker = |root: &Element| {
            root.query_selector("input[type=file]:not([disabled])")
                .unwrap()
                .is_some()
        };
        let refused = "A description is at most 64 characters.";
        assert!(picker(&root) && !text(&root).contains(refused));

        type_into(&root, "input.pack-label", &"😀".repeat(64));
        settle().await;
        assert!(picker(&root), "sixty-four characters, whatever they weigh");
        assert!(!text(&root).contains(refused));

        type_into(
            &root,
            "input.pack-label",
            &format!("  {}  ", "й".repeat(64)),
        );
        settle().await;
        assert!(picker(&root), "trimmed before it is counted");

        type_into(&root, "input.pack-label", &"й".repeat(65));
        settle().await;
        assert!(text(&root).contains(refused), "{}", text(&root));
        assert!(!picker(&root), "nothing can be picked for it");
        assert_eq!(
            root.query_selector("input.pack-label")
                .unwrap()
                .unwrap()
                .get_attribute("aria-invalid")
                .as_deref(),
            Some("true")
        );
        handle.destroy();
        root.remove();

        // The other door — a sticker from a chat, kept — offers it too, and
        // carries it; over-long, it asks nothing of anybody.
        loader.seed(90, Variant::Sticker, bytes("party cat"));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let on_action = {
            let actions = actions.clone();
            Callback::from(move |action: Action| actions.borrow_mut().push(action))
        };
        let add = ".dialog-actions button:not(.secondary)";
        let (root, handle) = render(&loader, view(Vec::new(), true, on_action)).await;
        type_into(&root, "input.pack-label", &"x".repeat(65));
        settle().await;
        assert!(text(&root).contains(refused), "{}", text(&root));
        assert!(root
            .query_selector(&format!("{add}[disabled]"))
            .unwrap()
            .is_some());
        type_into(&root, "input.pack-label", " party cat ");
        settle().await;
        assert!(!text(&root).contains(refused));
        click(&root, add);
        settle().await;
        assert!(
            matches!(&actions.borrow()[..],
                [Action::KeepSticker { attachment, label, .. }]
                    if attachment.id == 90 && label == " party cat "),
            "{:?}",
            actions.borrow()
        );
        handle.destroy();
        root.remove();
    }

    fn view(candidates: Vec<PackItem>, offered: bool, on_action: Callback<Action>) -> Html {
        html! {
            <StickerView
                attachment={picture(90, 11)}
                {candidates}
                {offered}
                adding={false}
                {on_action}
            />
        }
    }

    #[derive(Properties, PartialEq)]
    struct RecordingProps {
        on: bool,
    }

    /// A voice message being recorded behind the view, the way the
    /// conversation says so.
    #[function_component(Recording)]
    fn recording(props: &RecordingProps) -> Html {
        crate::views::quiet::use_quiet_while(props.on);
        Html::default()
    }

    /// Opened over a chat while a voice message is being recorded, the view's
    /// refusals — a description too long, an add that failed — are quiet
    /// like every live region of the app's: nothing is spoken into the note
    /// (the plan for #79, S6). With nothing recording they speak.
    #[wasm_bindgen_test]
    async fn the_views_refusals_are_quiet_while_a_voice_message_is_recorded() {
        use crate::views::quiet::QuietRoot;
        for on in [false, true] {
            let loader = loader();
            loader.seed(90, Variant::Sticker, bytes("party cat"));
            let finish = Rc::new(RefCell::new(None::<Callback<Option<String>>>));
            let on_action = {
                let finish = finish.clone();
                Callback::from(move |action: Action| {
                    if let Action::KeepSticker { done, .. } = action {
                        *finish.borrow_mut() = Some(done);
                    }
                })
            };
            let (root, handle) = render(
                &loader,
                html! {
                    <QuietRoot>
                        <Recording {on} />
                        { view(Vec::new(), true, on_action) }
                    </QuietRoot>
                },
            )
            .await;
            settle().await;
            let expected = if on {
                (None, Some("off".to_string()))
            } else {
                (Some("alert".to_string()), None)
            };
            let said = |root: &Element| {
                let error = root
                    .query_selector(".error")
                    .unwrap()
                    .expect("a refusal on the page");
                (
                    error.get_attribute("role"),
                    error.get_attribute("aria-live"),
                )
            };
            type_into(&root, ".pack-label", &"a".repeat(65));
            settle().await;
            assert_eq!(said(&root), expected, "too long, recording={on}");
            type_into(&root, ".pack-label", "cat");
            settle().await;
            click(&root, ".dialog-actions button:not(.secondary)");
            settle().await;
            let done = finish.borrow_mut().take().expect("asked to keep it");
            done.emit(Some("Couldn't add the sticker.".into()));
            settle().await;
            assert_eq!(said(&root), expected, "a failed add, recording={on}");
            handle.destroy();
            root.remove();
        }
    }

    /// Shown larger, a sticker offers "Add to family stickers" only when
    /// the pack does not hold it — decided from the BYTES, since nothing on
    /// the wire says which item a message was sent from.
    #[wasm_bindgen_test]
    async fn a_sticker_offers_to_be_kept_only_when_the_pack_lacks_it() {
        let loader = loader();
        loader.seed(90, Variant::Sticker, bytes("party cat"));
        loader.seed(75, Variant::Sticker, bytes("party cat"));
        loader.seed(76, Variant::Sticker, bytes("other cat!"));
        let actions = Rc::new(RefCell::new(Vec::new()));
        let on_action = {
            let actions = actions.clone();
            Callback::from(move |action: Action| actions.borrow_mut().push(action))
        };
        let add = ".dialog-actions button:not(.secondary)";

        // Same size, same type, other bytes: not in the pack.
        let (root, handle) = render(
            &loader,
            view(vec![item(6, ANNA, None)], true, on_action.clone()),
        )
        .await;
        settle().await;
        assert_eq!(count(&root, ".chat-sticker-stage img"), 1, "drawn larger");
        assert_eq!(count(&root, add), 1, "{}", text(&root));
        assert!(text(&root).contains("Add to family stickers"));
        click(&root, add);
        settle().await;
        assert!(
            matches!(&actions.borrow()[..], [Action::KeepSticker { attachment, .. }] if attachment.id == 90),
            "{:?}",
            actions.borrow()
        );
        handle.destroy();
        root.remove();

        // The same bytes: it is there already, and nothing is offered.
        let (root, handle) = render(
            &loader,
            view(
                vec![item(6, ANNA, None), item(5, ANNA, None)],
                true,
                on_action.clone(),
            ),
        )
        .await;
        settle().await;
        assert_eq!(count(&root, add), 0, "{}", text(&root));
        assert!(text(&root).contains("Already in the family's stickers."));
        handle.destroy();
        root.remove();

        // Nothing it could be: offered at once.
        let (root, handle) = render(&loader, view(Vec::new(), true, on_action.clone())).await;
        assert_eq!(count(&root, add), 1);
        handle.destroy();
        root.remove();

        // A server with no packs: shown larger, and nothing to add it to.
        let (root, handle) = render(&loader, view(Vec::new(), false, on_action)).await;
        assert_eq!(count(&root, ".chat-sticker-stage img"), 1);
        assert_eq!(count(&root, add), 0);
        assert!(!text(&root).contains("family's stickers"));
        handle.destroy();
        root.remove();
    }

    #[wasm_bindgen_test]
    fn a_sticker_is_called_by_its_label_or_by_the_word() {
        assert_eq!(spoken(Some("party cat")), "party cat");
        assert_eq!(spoken(Some("  ")), "Sticker");
        assert_eq!(spoken(None), "Sticker");
        assert_eq!(removal(None), "Remove sticker");
        assert_eq!(removal(Some(" ")), "Remove sticker");
        assert_eq!(removal(Some("party cat")), "Remove sticker: party cat");
    }

    /// EVERY WORD A SCREEN READER SAYS ABOUT A STICKER IS IN THE READER'S
    /// LANGUAGE. The two labels here were once built from "Open %@" and
    /// "Remove %@" — keys the web has in English only — so a German reader
    /// heard English on a German page. Checked against the shipped
    /// catalogue, for each of the nine languages.
    #[wasm_bindgen_test]
    fn a_stickers_spoken_labels_are_translated_in_all_nine_languages() {
        use fc_text::i18n::{use_lang, Lang};
        let english = ["Open sticker", "Remove sticker", "Remove sticker: %@"];
        let nine = [
            Lang::En,
            Lang::De,
            Lang::Es,
            Lang::Fr,
            Lang::Ja,
            Lang::Ru,
            Lang::Sr,
            Lang::SrLatn,
            Lang::ZhHans,
        ];
        for lang in nine {
            use_lang(lang);
            let said = [
                t("Open sticker").to_string(),
                removal(None),
                t("Remove sticker: %@").to_string(),
            ];
            for (said, english) in said.iter().zip(english) {
                assert!(!said.is_empty());
                if lang != Lang::En {
                    assert_ne!(said, english, "{lang:?} says it in English");
                }
            }
            assert!(
                removal(Some("party cat")).contains("party cat"),
                "{lang:?} lost the label"
            );
        }
        use_lang(Lang::En);
    }
}

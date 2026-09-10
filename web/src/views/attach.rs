//! The ways something is attached, and what is waiting to be sent: the
//! paperclip's menu, the staging strip, the recording bar, and the clipboard
//! and drag-and-drop readers behind the conversation's doors (the Mac's
//! MacConversationView attach menu, StagedAttachment, ClipboardAttachment
//! and DroppedAttachment).
//!
//! Every door ends in the same place — `prep::prepare` and the ten-item
//! staging cap — so none of them can skip the downscale, the ceiling or the
//! animated-GIF rule, and none can differ from the others in ways nobody
//! notices until a send fails.

use fc_text::media;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{DataTransfer, File, HtmlInputElement, Url};
use yew::prelude::*;

use crate::staged::Prepared;

#[derive(Properties, PartialEq)]
pub struct MenuProps {
    /// Poll is offered — the family chat, never a thread.
    pub offers_poll: bool,
    /// Nothing may be attached right now, and the reason why.
    pub busy: Option<String>,
    pub on_files: Callback<Vec<File>>,
    pub on_paste: Callback<()>,
    pub on_record: Callback<()>,
    pub on_location: Callback<()>,
    pub on_poll: Callback<()>,
    /// Said instead of opening the menu when `busy`.
    pub on_busy: Callback<String>,
    /// "Show the Assistant a Photo…" is offered — the assistant's chat, on
    /// a server that can see, in a family that allows it; ABSENT otherwise,
    /// never a door that lies (docs/protocol.md, "Pictures").
    #[prop_or_default]
    pub offers_pictures: bool,
    #[prop_or_default]
    pub on_pictures: Callback<Vec<File>>,
}

/// The paperclip, and what it offers.
#[function_component(AttachMenu)]
pub fn attach_menu(props: &MenuProps) -> Html {
    let open = use_state(|| false);
    let picker = use_node_ref();
    let pictures = use_node_ref();
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
    let item = |label: &'static str, action: Callback<()>| {
        let open = open.clone();
        let onclick = Callback::from(move |_: MouseEvent| {
            open.set(false);
            action.emit(());
        });
        html! { <button role="menuitem" {onclick}>{ label }</button> }
    };
    let pick = {
        let picker = picker.clone();
        Callback::from(move |_: ()| {
            if let Some(input) = picker.cast::<HtmlInputElement>() {
                input.set_value("");
                input.click();
            }
        })
    };
    let pick_pictures = {
        let pictures = pictures.clone();
        Callback::from(move |_: ()| {
            if let Some(input) = pictures.cast::<HtmlInputElement>() {
                input.set_value("");
                input.click();
            }
        })
    };
    let picked_pictures = {
        let on_pictures = props.on_pictures.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            // The four the model is shown (MacFilePicker.pickPictures).
            let files = input
                .files()
                .map(|list| {
                    (0..list
                        .length()
                        .min(fc_text::assistant_pictures::MAX_PER_QUESTION as u32))
                        .filter_map(|index| list.get(index))
                        .collect()
                })
                .unwrap_or_default();
            // Emptied once read: picking the same file again is a change.
            input.set_value("");
            on_pictures.emit(files);
        })
    };
    let picked = {
        let on_files = props.on_files.clone();
        Callback::from(move |event: Event| {
            let input: HtmlInputElement = event.target_unchecked_into();
            let files = input
                .files()
                .map(|list| {
                    (0..list.length())
                        .filter_map(|index| list.get(index))
                        .collect()
                })
                .unwrap_or_default();
            // Emptied once read: picking the same file again is a change,
            // and what was chosen last time is not chosen again.
            input.set_value("");
            on_files.emit(files);
        })
    };
    let close = {
        let open = open.clone();
        Callback::from(move |_: MouseEvent| open.set(false))
    };
    let on_menu_key = {
        let open = open.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                open.set(false);
            }
        })
    };
    // Opened, the menu takes the focus, so Escape and the arrows reach it.
    {
        let is_open = *open;
        use_effect_with(is_open, move |is_open| {
            if *is_open {
                if let Some(first) = web_sys::window()
                    .and_then(|window| window.document())
                    .and_then(|document| {
                        document
                            .query_selector(".attach-menu [role=menuitem]")
                            .ok()
                            .flatten()
                    })
                    .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
                {
                    let _ = first.focus();
                }
            }
        });
    }
    html! {
        <div class="attach">
            <button class="tool" title="Attach a photo, video or file" aria-label="Attach"
                    aria-haspopup="menu" aria-expanded={(*open).to_string()} onclick={toggle}>
                { "📎" }
            </button>
            <input ref={picker} type="file" multiple=true class="hidden-picker" onchange={picked}
                   aria-hidden="true" tabindex="-1" />
            <input ref={pictures} type="file" multiple=true accept="image/*" class="hidden-picker"
                   onchange={picked_pictures} aria-hidden="true" tabindex="-1" />
            if *open {
                // A click anywhere else closes it — on a touch screen there
                // is no mouse to leave.
                <div class="menu-backdrop" onclick={close.clone()} aria-hidden="true"></div>
                <div class="menu attach-menu" role="menu" onmouseleave={close} onkeydown={on_menu_key}>
                    if props.offers_pictures {
                        { item("Show the Assistant a Photo…", pick_pictures) }
                    }
                    { item("Attach a File…", pick) }
                    { item("Paste", props.on_paste.clone()) }
                    { item("Record Audio", props.on_record.clone()) }
                    { item("Location", props.on_location.clone()) }
                    if props.offers_poll {
                        { item("Poll", props.on_poll.clone()) }
                    }
                </div>
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct StripProps {
    pub items: Vec<Prepared>,
    pub on_remove: Callback<usize>,
}

/// What is staged, each with its own ✕ — sent with whatever the box says
/// as the caption, or with nothing (StagedAttachment).
#[function_component(StagingStrip)]
pub fn staging_strip(props: &StripProps) -> Html {
    if props.items.is_empty() {
        return Html::default();
    }
    html! {
        <div class="staging" aria-label="Attachments to send">
            { for props.items.iter().enumerate().map(|(index, item)| {
                let remove = props.on_remove.reform(move |_: MouseEvent| index);
                html! {
                    <div class="staged" key={index}>
                        <Thumb item={item.clone()} />
                        <span class="staged-label">{ label(item) }</span>
                        <button class="staged-remove" onclick={remove} aria-label={format!("Remove {}", label(item))}>{ "✕" }</button>
                    </div>
                }
            }) }
        </div>
    }
}

/// What a staged item is called on its chip.
pub fn label(item: &Prepared) -> String {
    match item.kind.as_str() {
        "audio" if item.name.is_none() => format!(
            "Voice note · {}",
            media::time_label(item.duration_ms.unwrap_or(0) as f64 / 1000.0)
        ),
        "location" => "Location".to_string(),
        kind => {
            let name = media::display_name(kind, item.name.as_deref()).into_owned();
            if kind == "file" || kind == "audio" {
                format!(
                    "{} · {}",
                    name,
                    media::display_size(item.size.max(0) as u64)
                )
            } else {
                name
            }
        }
    }
}

#[derive(Properties, PartialEq)]
struct ThumbProps {
    item: Prepared,
}

/// A staged item's picture — its preview, under a URL of its own for as
/// long as the chip is on screen — or its kind's glyph.
#[function_component(Thumb)]
fn thumb(props: &ThumbProps) -> Html {
    let url = use_memo(props.item.preview.clone(), |preview| {
        preview
            .as_ref()
            .and_then(|blob| Url::create_object_url_with_blob(blob).ok())
    });
    {
        let url = url.clone();
        use_effect_with(url, |url| {
            let url = (**url).clone();
            move || {
                if let Some(url) = url {
                    let _ = Url::revoke_object_url(&url);
                }
            }
        });
    }
    match (*url).clone() {
        Some(url) => html! { <img class="staged-thumb" src={url} alt="" /> },
        None => {
            let glyph = match props.item.kind.as_str() {
                "audio" => "🎤",
                "video" => "🎬",
                "location" => "📍",
                "photo" => "🖼",
                _ => "📄",
            };
            html! { <span class="staged-thumb" aria-hidden="true">{ glyph }</span> }
        }
    }
}

#[derive(Properties, PartialEq)]
pub struct RecordingProps {
    /// When it started, in wall-clock milliseconds.
    pub started_ms: f64,
    pub on_cancel: Callback<()>,
    pub on_stop: Callback<()>,
}

/// A voice note being recorded: how long so far, Cancel, and Stop — which
/// Return presses, and which the five-minute ceiling presses by itself. The
/// clock ticks HERE, so a recording redraws this bar and not the chat.
#[function_component(RecordingBar)]
pub fn recording_bar(props: &RecordingProps) -> Html {
    let stop = use_node_ref();
    let elapsed = use_state(|| 0.0_f64);
    {
        let stop = stop.clone();
        use_effect_with((), move |_| {
            if let Some(button) = stop.cast::<web_sys::HtmlElement>() {
                let _ = button.focus();
            }
        });
    }
    {
        let elapsed = elapsed.clone();
        let on_stop = props.on_stop.clone();
        let started = props.started_ms;
        use_effect_with(started, move |started| {
            let started = *started;
            let stopped = std::cell::Cell::new(false);
            let ticker = gloo_timers::callback::Interval::new(250, move || {
                let now = js_sys::Date::now() - started;
                if now >= f64::from(media::VOICE_MAX_SECONDS) * 1000.0 {
                    if !stopped.replace(true) {
                        on_stop.emit(());
                    }
                    return;
                }
                elapsed.set(now);
            });
            move || drop(ticker)
        });
    }
    let on_key = {
        let on_cancel = props.on_cancel.clone();
        Callback::from(move |event: KeyboardEvent| {
            if event.key() == "Escape" {
                on_cancel.emit(());
            }
        })
    };
    html! {
        // A group with a name, not a live region: the clock changing every
        // second is not news to announce every second.
        <div class="recording" role="group" aria-label="Recording a voice note" onkeydown={on_key}>
            <span class="recording-dot" aria-hidden="true">{ "●" }</span>
            <span aria-live="off">{ format!("Recording {}", media::time_label(*elapsed / 1000.0)) }</span>
            <button class="secondary" onclick={props.on_cancel.reform(|_: MouseEvent| ())}>{ "Cancel" }</button>
            <button ref={stop} onclick={props.on_stop.reform(|_: MouseEvent| ())}>{ "Stop" }</button>
        </div>
    }
}

/// What a drop carries: the files to attach, and whether a folder was in
/// it — refused, like the Mac refuses one (DroppedAttachment.decide).
pub fn dropped_files(data: &DataTransfer) -> (Vec<File>, bool) {
    let items = data.items();
    let mut files = Vec::new();
    let mut folders = false;
    for index in 0..items.length() {
        let Some(item) = items.get(index) else {
            continue;
        };
        if item.kind() != "file" {
            continue;
        }
        let is_folder = item
            .webkit_get_as_entry()
            .ok()
            .flatten()
            .is_some_and(|entry| entry.is_directory());
        if is_folder {
            folders = true;
            continue;
        }
        if let Ok(Some(file)) = item.get_as_file() {
            files.push(file);
        }
    }
    (files, folders)
}

/// The links a drop carries, one per line — into the draft as words, since
/// there is nothing to attach: the bytes are on somebody else's server.
pub fn dropped_links(data: &DataTransfer) -> String {
    let list = data.get_data("text/uri-list").unwrap_or_default();
    web_links(&list)
}

/// The web links of a `text/uri-list`: comments and blank lines skipped,
/// and nothing but http(s) — a `file:` URL is somebody's own disk, and its
/// path has no business in a message (DroppedAttachment takes only links
/// that are not files).
pub fn web_links(list: &str) -> String {
    list.lines()
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.starts_with("https://") || lower.starts_with("http://")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether a drag is carrying anything this window can take.
pub fn drag_carries_something(data: &DataTransfer) -> bool {
    let types = data.types();
    (0..types.length()).any(|index| {
        types
            .get(index)
            .as_string()
            .is_some_and(|kind| kind == "Files" || kind == "text/uri-list")
    })
}

/// What the clipboard held, asked for by the menu's Paste.
pub enum Clip {
    Files(Vec<File>),
    Text(String),
    Nothing,
    /// The browser would not let the page read it.
    Denied,
}

/// Read the clipboard, by the paste rule (fc_text::media::paste_decision).
pub async fn read_clipboard() -> Clip {
    let Some(window) = web_sys::window() else {
        return Clip::Denied;
    };
    // Absent outside a secure context — a server on http:// — where calling
    // into it would throw rather than answer.
    let navigator = window.navigator();
    let readable = js_sys::Reflect::get(&navigator, &wasm_bindgen::JsValue::from_str("clipboard"))
        .ok()
        .filter(|clipboard| !clipboard.is_undefined() && !clipboard.is_null())
        .and_then(|clipboard| {
            js_sys::Reflect::get(&clipboard, &wasm_bindgen::JsValue::from_str("read")).ok()
        })
        .is_some_and(|read| read.is_function());
    if !readable {
        return Clip::Denied;
    }
    let clipboard = navigator.clipboard();
    let Ok(items) = JsFuture::from(clipboard.read()).await else {
        return Clip::Denied;
    };
    let items: js_sys::Array = items.unchecked_into();
    let mut files = Vec::new();
    let mut text = String::new();
    for item in items.iter() {
        let item: web_sys::ClipboardItem = item.unchecked_into();
        let offered: Vec<String> = item
            .types()
            .iter()
            .filter_map(|kind| kind.as_string())
            .collect();
        if let Some(kind) = media::chosen_paste_type(&offered) {
            if let Ok(blob) = JsFuture::from(item.get_type(&kind)).await {
                let parts = js_sys::Array::of1(&blob);
                let options = web_sys::FilePropertyBag::new();
                options.set_type(&kind);
                if let Ok(file) = File::new_with_blob_sequence_and_options(
                    &parts,
                    &media::pasted_name(&kind),
                    &options,
                ) {
                    files.push(file);
                }
            }
        }
        if text.is_empty() && offered.iter().any(|kind| kind == "text/plain") {
            if let Ok(blob) = JsFuture::from(item.get_type("text/plain")).await {
                let blob: web_sys::Blob = blob.unchecked_into();
                if let Ok(words) = JsFuture::from(blob.text()).await {
                    text = words.as_string().unwrap_or_default();
                }
            }
        }
    }
    let names: Vec<String> = files.iter().map(File::name).collect();
    match media::paste_decision(&names, &text) {
        media::PasteDecision::Attach => Clip::Files(files),
        media::PasteDecision::Type => Clip::Text(text),
        media::PasteDecision::Nothing => Clip::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn a_dropped_link_is_words_and_a_dropped_path_is_nothing() {
        assert_eq!(
            web_links("# a comment\nhttps://example.com/a\nfile:///Users/anna/secret\n\nHTTP://EXAMPLE.ORG"),
            "https://example.com/a\nHTTP://EXAMPLE.ORG"
        );
        assert_eq!(web_links("file:///tmp/folder"), "");
    }

    #[wasm_bindgen_test]
    fn a_staged_item_says_what_it_is() {
        let voice = Prepared {
            kind: "audio".into(),
            duration_ms: Some(83_000),
            ..Prepared::default()
        };
        assert_eq!(label(&voice), "Voice note · 1:23");
        let track = Prepared {
            kind: "audio".into(),
            name: Some("song.mp3".into()),
            size: 4_200_000,
            ..Prepared::default()
        };
        assert_eq!(label(&track), "song.mp3 · 4.2 MB");
        let file = Prepared {
            kind: "file".into(),
            name: Some("receipts.pdf".into()),
            size: 182_734,
            ..Prepared::default()
        };
        assert_eq!(label(&file), "receipts.pdf · 183 KB");
        let photo = Prepared {
            kind: "photo".into(),
            ..Prepared::default()
        };
        assert_eq!(label(&photo), "Photo");
        assert_eq!(label(&Prepared::location(1.0, 2.0, None)), "Location");
    }
}

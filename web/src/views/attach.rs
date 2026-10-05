//! The ways something is attached, and what is waiting to be sent: the
//! paperclip's menu, the staging strip — a voice note on it playable — the
//! recording row, the voice messages that were not sent, and the clipboard
//! and drag-and-drop readers behind the conversation's doors (the Mac's
//! MacConversationView attach menu, StagedAttachment, ClipboardAttachment
//! and DroppedAttachment).
//!
//! Every door ends in the same place — `prep::prepare` and the ten-item
//! staging cap — so none of them can skip the downscale, the ceiling or the
//! animated-GIF rule, and none can differ from the others in ways nobody
//! notices until a send fails.

use fc_text::i18n::{t, t1};
use fc_text::{media, record};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, DataTransfer, File, HtmlInputElement, HtmlMediaElement, Url};
use yew::prelude::*;

use crate::awake::ScreenAwake;
use crate::recorder::{self, Meter};
use crate::staged::Prepared;
use crate::store::NotSent;
use crate::views::dialog::Confirm;

/// "You can play this after recording." — what a play control says while a
/// voice message is being recorded, instead of playing (S1.7): nothing of
/// the app's plays into a note.
pub const PLAY_AFTER: &str = "You can play this after recording.";

/// Everything playing on the page paused — a voice note in a bubble, a
/// video, a staged note — but `except`: one thing plays at a time, and a
/// recording starting pauses whatever does (S1.7, S2.2, S2.7).
pub fn pause_all_playing(except: Option<&HtmlMediaElement>) {
    let Some(found) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.query_selector_all("audio, video").ok())
    else {
        return;
    };
    for index in 0..found.length() {
        let Some(player) = found
            .item(index)
            .and_then(|node| node.dyn_into::<HtmlMediaElement>().ok())
        else {
            continue;
        };
        if except.is_some_and(|keep| keep == &player) || player.paused() {
            continue;
        }
        let _ = player.pause();
    }
}

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
    /// Why "Record Voice Message" cannot record right now — a call is on, or
    /// a voice message that was not sent waits (the plan for #79, S1.5).
    /// DIMMED, not disabled: it stays in the menu, focusable, carries the
    /// reason, and is still chosen — `on_record` is what refuses, and says
    /// why.
    #[prop_or_default]
    pub record_dimmed: Option<String>,
    /// "Record Voice Message" is offered at all: never in the assistant's
    /// chat, where every message is a consented model call that is only
    /// ever shown `[voice note]` (S1.5, Decision 24).
    #[prop_or(true)]
    pub offers_record: bool,
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
    // "Record Voice Message", DIMMED where it cannot record — which it still
    // says when chosen, through the same door: the recorder's start is what
    // refuses, and the menu only shows that it will.
    let dimmed_reason_id = use_memo((), |_| crate::views::dialog::fresh_id("dimmed-reason"));
    let record = {
        let open = open.clone();
        let on_record = props.on_record.clone();
        let onclick = Callback::from(move |_: MouseEvent| {
            open.set(false);
            on_record.emit(());
        });
        match props.record_dimmed.clone() {
            _ if !props.offers_record => Html::default(),
            None => {
                html! { <button role="menuitem" {onclick}>{ t("Record Voice Message") }</button> }
            }
            Some(reason) => html! {
                <>
                    <button role="menuitem" class="is-dimmed" aria-disabled="true"
                            aria-describedby={(*dimmed_reason_id).clone()} title={reason.clone()} {onclick}>
                        { t("Record Voice Message") }
                    </button>
                    <span id={(*dimmed_reason_id).clone()} hidden=true>{ reason }</span>
                </>
            },
        }
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
            <button class="tool" title={t("Attach a photo, video or file")} aria-label={t("Attach")}
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
                        { item(t("Show the Assistant a Photo…"), pick_pictures) }
                    }
                    { item(t("Attach a File…"), pick) }
                    { item(t("Paste"), props.on_paste.clone()) }
                    { record }
                    { item(t("Location"), props.on_location.clone()) }
                    if props.offers_poll {
                        { item(t("Poll"), props.on_poll.clone()) }
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
    /// A voice message is being recorded: a staged note's ▶ says so instead
    /// of playing (S1.7).
    #[prop_or_default]
    pub recording: bool,
    /// Where a play control says why it did not play.
    #[prop_or_default]
    pub on_explain: Callback<String>,
}

/// What is staged, each with its own ✕ — sent with whatever the box says
/// as the caption, or with nothing (StagedAttachment). A voice note can be
/// listened to before it goes (the plan for #79, S2.7).
#[function_component(StagingStrip)]
pub fn staging_strip(props: &StripProps) -> Html {
    if props.items.is_empty() {
        return Html::default();
    }
    html! {
        <div class="staging" aria-label={t("Attachments to send")}>
            { for props.items.iter().enumerate().map(|(index, item)| {
                let remove = props.on_remove.reform(move |_: ()| index);
                if item.is_voice_note() {
                    return html! {
                        <VoiceChip
                            key={index}
                            item={item.clone()}
                            on_remove={remove}
                            recording={props.recording}
                            on_explain={props.on_explain.clone()}
                        />
                    };
                }
                html! {
                    <div class="staged" key={index}>
                        <Thumb item={item.clone()} />
                        <span class="staged-label">{ label(item) }</span>
                        <button class="staged-remove" onclick={remove.reform(|_: MouseEvent| ())} aria-label={t1("Remove %@", &label(item))}>{ "✕" }</button>
                    </div>
                }
            }) }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct VoiceChipProps {
    item: Prepared,
    on_remove: Callback<()>,
    recording: bool,
    on_explain: Callback<String>,
}

/// A voice note in review (S2.7): "[▶] Voice message · 0:42 [✕]", and while
/// it plays "[❚❚] 0:12 / 0:42". ✕ — "Delete recording" — asks first at ten
/// seconds or more.
#[function_component(VoiceChip)]
fn voice_chip(props: &VoiceChipProps) -> Html {
    let playing = use_state(|| Option::<f64>::None);
    let asking = use_state(|| false);
    let duration_ms = props.item.duration_ms.unwrap_or(0).max(0);
    let total = duration_ms as f64 / 1000.0;
    let words = match *playing {
        Some(at) => format!(
            "{} / {}",
            media::time_label(at.floor()),
            media::time_label(total)
        ),
        None => label(&props.item),
    };
    let remove = {
        let on_remove = props.on_remove.clone();
        let asking = asking.clone();
        let asks = duration_ms >= record::DELETE_ASKS_FROM_MS as i64;
        Callback::from(move |_: MouseEvent| {
            if asks {
                asking.set(true);
            } else {
                on_remove.emit(());
            }
        })
    };
    html! {
        <div class="staged is-voice">
            <LocalAudio
                blob={props.item.file.clone()}
                dimmed={props.recording}
                on_explain={props.on_explain.clone()}
                on_progress={{
                    let playing = playing.clone();
                    Callback::from(move |at: Option<f64>| playing.set(at))
                }}
            />
            <span class="staged-label">{ words }</span>
            <button class="staged-remove" onclick={remove} aria-label={t("Delete recording")}>{ "✕" }</button>
            if *asking {
                <Confirm
                    title={t("Delete this recording?")}
                    confirm={t("Delete")}
                    cancel={AttrValue::from(t("Keep"))}
                    on_confirm={{
                        let on_remove = props.on_remove.clone();
                        let asking = asking.clone();
                        Callback::from(move |_: ()| {
                            asking.set(false);
                            on_remove.emit(());
                        })
                    }}
                    on_cancel={{
                        let asking = asking.clone();
                        Callback::from(move |_: ()| asking.set(false))
                    }}
                />
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
pub struct LocalAudioProps {
    /// The recording's own bytes, on this device.
    pub blob: Option<Blob>,
    /// A voice message is being recorded: say why instead of playing (S1.7).
    #[prop_or_default]
    pub dimmed: bool,
    #[prop_or_default]
    pub on_explain: Callback<String>,
    /// Where it is while it plays, in seconds — None once it stops.
    #[prop_or_default]
    pub on_progress: Callback<Option<f64>>,
}

/// ▶ and ❚❚ for a recording that is still on this device — a voice note in
/// review, one that was not sent (S2.7, S2.8). It plays the LOCAL bytes,
/// pausing anything else that plays; and on a phone's browser it keeps the
/// screen on while it plays, so the lock does not hide the tab halfway
/// through a long note (S1.7).
#[function_component(LocalAudio)]
pub fn local_audio(props: &LocalAudioProps) -> Html {
    let player = use_node_ref();
    let playing = use_state(|| false);
    let awake = use_mut_ref(|| Option::<ScreenAwake>::None);
    let reason_id = use_memo((), |_| crate::views::dialog::fresh_id("play-reason"));
    let url = use_memo(props.blob.clone(), |blob| {
        blob.as_ref()
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
    let toggle = {
        let player = player.clone();
        let dimmed = props.dimmed;
        let on_explain = props.on_explain.clone();
        Callback::from(move |_: MouseEvent| {
            if dimmed {
                on_explain.emit(t(PLAY_AFTER).to_string());
                return;
            }
            let Some(audio) = player.cast::<HtmlMediaElement>() else {
                return;
            };
            // Asked of the player itself, not of the last drawing: a second
            // press before its `play` has even been heard is a pause.
            if !audio.paused() {
                let _ = audio.pause();
                return;
            }
            pause_all_playing(Some(&audio));
            if audio.ended() {
                audio.set_current_time(0.0);
            }
            if let Ok(started) = audio.play() {
                wasm_bindgen_futures::spawn_local(async move {
                    let _ = JsFuture::from(started).await;
                });
            }
        })
    };
    let on_play = {
        let playing = playing.clone();
        let awake = awake.clone();
        let on_progress = props.on_progress.clone();
        Callback::from(move |event: Event| {
            playing.set(true);
            if phone_like() {
                *awake.borrow_mut() = Some(ScreenAwake::hold());
            }
            if let Some(audio) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlMediaElement>().ok())
            {
                on_progress.emit(Some(audio.current_time()));
            }
        })
    };
    let on_time = {
        let on_progress = props.on_progress.clone();
        Callback::from(move |event: Event| {
            if let Some(audio) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlMediaElement>().ok())
            {
                if !audio.paused() {
                    on_progress.emit(Some(audio.current_time()));
                }
            }
        })
    };
    let on_stopped = {
        let playing = playing.clone();
        let awake = awake.clone();
        let on_progress = props.on_progress.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
            awake.borrow_mut().take();
            on_progress.emit(None);
        })
    };
    let (glyph, word) = if *playing {
        ("❚❚", t("Pause"))
    } else {
        ("▶", t("Play"))
    };
    html! {
        <>
            <button
                type="button"
                class={classes!("local-play", props.dimmed.then_some("is-dimmed"))}
                aria-label={word}
                aria-disabled={props.dimmed.then_some("true")}
                aria-describedby={props.dimmed.then(|| (*reason_id).clone())}
                disabled={url.is_none()}
                onclick={toggle}
            >
                <span aria-hidden="true">{ glyph }</span>
            </button>
            if props.dimmed {
                <span id={(*reason_id).clone()} hidden=true>{ t(PLAY_AFTER) }</span>
            }
            <audio ref={player} src={(*url).clone().unwrap_or_default()} preload="auto"
                   onplay={on_play} ontimeupdate={on_time} onpause={on_stopped.clone()}
                   onended={on_stopped} />
        </>
    }
}

/// A phone's or a tablet's browser — a coarse pointer is what one has —
/// where the screen is kept on while a recording plays (S1.7, S8.8).
pub(crate) fn phone_like() -> bool {
    web_sys::window()
        .and_then(|window| window.match_media("(pointer: coarse)").ok().flatten())
        .is_some_and(|query| query.matches())
}

/// What a staged item is called on its chip.
pub fn label(item: &Prepared) -> String {
    match item.kind.as_str() {
        "audio" if item.name.is_none() => t1(
            "Voice message · %@",
            &media::time_label(item.duration_ms.unwrap_or(0) as f64 / 1000.0),
        ),
        "location" => t("Location").to_string(),
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
    /// When it started, by the recorder's clock (`recorder::now_ms`).
    pub started_ms: f64,
    /// How loud it is — None for a recorder with no tap (the browser's own
    /// `MediaRecorder`), which draws the dot only and warns of no silence
    /// (S2.9).
    #[prop_or_default]
    pub meter: Option<Meter>,
    /// Started with words typed or items staged: the slot is its Stop (S1.3
    /// row 3), and the row draws no Stop of its own.
    #[prop_or_default]
    pub beside_draft: bool,
    pub on_delete: Callback<()>,
    pub on_stop: Callback<()>,
    /// Five minutes: the recorder stops itself, into review (S2.5).
    pub on_cap: Callback<()>,
    /// 4:30: "30 seconds left" is shown here — and said, once.
    #[prop_or_default]
    pub on_warning: Callback<()>,
    /// Three seconds in with nothing heard above digital silence (`true`),
    /// and sound at last after that (`false`).
    #[prop_or_default]
    pub on_silence: Callback<bool>,
}

/// How often the row looks at the clock and the meter — the apps' 200 ms.
const ROW_TICK_MS: u32 = 200;

/// The recording row (S2.4): it takes the field's place in the composer's
/// own row, at the same height — Delete, a red dot, the time in monospaced
/// digits and the level meter, Stop — before the slot, which is the Send
/// arrow. The clock ticks HERE, so a recording redraws this row and not the
/// chat. Esc, which is Stop, is the composer's to hear: focus is on the slot.
#[function_component(RecordingRow)]
pub fn recording_row(props: &RecordingProps) -> Html {
    let elapsed = use_state(|| 0.0_f64);
    let lit = use_state(|| 0usize);
    {
        let elapsed = elapsed.clone();
        let lit = lit.clone();
        let meter = props.meter.clone();
        let on_cap = props.on_cap.clone();
        let on_warning = props.on_warning.clone();
        let on_silence = props.on_silence.clone();
        let started = props.started_ms;
        use_effect_with(started, move |started| {
            let started = *started;
            let capped = std::cell::Cell::new(false);
            let warned = std::cell::Cell::new(false);
            let silent = std::cell::Cell::new(false);
            let tick = move || {
                let now = recorder::now_ms() - started;
                elapsed.set(now.max(0.0));
                if let Some(meter) = &meter {
                    lit.set(recorder::lit_bars(meter.take_peak()));
                    let quiet = now >= record::SILENCE_WARNING_AFTER_MS as f64 && !meter.heard();
                    if quiet != silent.get() {
                        silent.set(quiet);
                        on_silence.emit(quiet);
                    }
                }
                if now >= record::VOICE_WARNING_MS as f64 && !warned.replace(true) {
                    on_warning.emit(());
                }
                if now >= record::VOICE_CAP_MS as f64 && !capped.replace(true) {
                    on_cap.emit(());
                }
            };
            tick();
            let ticker = gloo_timers::callback::Interval::new(ROW_TICK_MS, tick);
            move || drop(ticker)
        });
    }
    let warning = *elapsed >= record::VOICE_WARNING_MS as f64;
    html! {
        // A group with a name, not a live region: the clock changing every
        // second is not news to announce every second.
        <div class="recording" role="group" aria-label={t("Recording a voice message")}>
            <button type="button" class="secondary recording-delete" aria-label={t("Delete recording")}
                    title={t("Delete recording")} onclick={props.on_delete.reform(|_: MouseEvent| ())}>
                { icon(TRASH) }
                <span class="button-word">{ t("Delete") }</span>
            </button>
            <span class="recording-dot" aria-hidden="true"></span>
            <span class={classes!("recording-time", warning.then_some("is-warning"))} aria-live="off">
                { media::time_label((*elapsed / 1000.0).floor()) }
            </span>
            if warning {
                // Words as well as colour (WCAG 1.4.1), in the meter's place.
                <span class="recording-left">{ t("30 seconds left") }</span>
            } else if props.meter.is_some() {
                <span class="recording-meter" aria-hidden="true">
                    { for (0..5).map(|bar| html! {
                        <span class={classes!("bar", (bar < *lit).then_some("is-lit"))}></span>
                    }) }
                </span>
            }
            if !props.beside_draft {
                <button type="button" class="secondary recording-stop" aria-label={t("Stop recording")}
                        title={t("Stop recording")} onclick={props.on_stop.reform(|_: MouseEvent| ())}>
                    { icon(STOP) }
                    <span class="button-word">{ t("Stop") }</span>
                </button>
            }
        </div>
    }
}

/// The recording row's pictures, for the phone's width, where its buttons
/// are icons with the same labels (S2.4).
const TRASH: &str = "M6 19a2 2 0 0 0 2 2h8a2 2 0 0 0 2-2V7H6zM19 4h-3.5l-1-1h-5l-1 1H5v2h14z";
const STOP: &str = "M7 7h10v10H7z";

fn icon(path: &'static str) -> Html {
    html! {
        <svg class="row-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
            <path fill="currentColor" d={path} />
        </svg>
    }
}

#[derive(Properties, PartialEq)]
pub struct NotSentProps {
    pub row: NotSent,
    /// "Replying to Anna: Dinner?" — the reply it was recorded under, the
    /// way the composer's banner says one; None when it has none, or the
    /// message is not on screen.
    #[prop_or_default]
    pub quote: Option<String>,
    pub on_send: Callback<NotSent>,
    pub on_delete: Callback<NotSent>,
    /// A voice message is being recorded: its ▶ says so instead of playing.
    #[prop_or_default]
    pub recording: bool,
    #[prop_or_default]
    pub on_explain: Callback<String>,
}

/// A voice message that was not sent (the plan for #79, S2.8): "Voice
/// message not sent · 0:42", the reply it was recorded under and its
/// caption, with its ▶ to listen to it first, its own Send — which sends it
/// with THAT reply and caption and nothing else — and its own ✕, which asks
/// first at ten seconds or more.
#[function_component(NotSentRow)]
pub fn not_sent_row(props: &NotSentProps) -> Html {
    let asking = use_state(|| false);
    let row = props.row.clone();
    let label = t1(
        "Voice message not sent · %@",
        &media::time_label(row.duration_ms.max(0) as f64 / 1000.0),
    );
    let send = {
        let on_send = props.on_send.clone();
        let row = row.clone();
        Callback::from(move |_: MouseEvent| on_send.emit(row.clone()))
    };
    let delete = {
        let on_delete = props.on_delete.clone();
        let asking = asking.clone();
        let row = row.clone();
        let asks = row.duration_ms >= fc_text::record::DELETE_ASKS_FROM_MS as i64;
        Callback::from(move |_: MouseEvent| {
            if asks {
                asking.set(true);
            } else {
                on_delete.emit(row.clone());
            }
        })
    };
    html! {
        <div class="not-sent" role="group" aria-label={label.clone()}>
            <span class="not-sent-glyph" aria-hidden="true">{ "🎤" }</span>
            <span class="not-sent-text">
                <span class="not-sent-label">{ label }</span>
                if let Some(quote) = props.quote.clone() {
                    <span class="not-sent-quote">{ quote }</span>
                }
                if !row.caption.is_empty() {
                    <span class="not-sent-caption">{ row.caption.clone() }</span>
                }
            </span>
            <LocalAudio
                blob={row.note.file.clone()}
                dimmed={props.recording}
                on_explain={props.on_explain.clone()}
            />
            <button class="link not-sent-send" aria-label={t("Send voice message")} onclick={send}>{ t("Send") }</button>
            <button class="link not-sent-delete" aria-label={t("Delete recording")} onclick={delete}>{ "✕" }</button>
            if *asking {
                <Confirm
                    title={t("Delete this recording?")}
                    confirm={t("Delete")}
                    cancel={AttrValue::from(t("Keep"))}
                    on_confirm={{
                        let on_delete = props.on_delete.clone();
                        let asking = asking.clone();
                        Callback::from(move |_: ()| {
                            asking.set(false);
                            on_delete.emit(row.clone());
                        })
                    }}
                    on_cancel={{
                        let asking = asking.clone();
                        Callback::from(move |_: ()| asking.set(false))
                    }}
                />
            }
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
        // "Voice message", as every client calls it (S10: renamed from
        // "Voice note").
        assert_eq!(label(&voice), "Voice message · 1:23");
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

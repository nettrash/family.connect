//! A message's attachments, drawn: the photos and videos as one tile or a
//! pile, and the files, recordings and places as rows under it — the Mac's
//! composition (MacMessageRow.attachmentStack, AttachmentAlbum), from the
//! same rules (fc_text::media).
//!
//! Everything is sized from the attachment's METADATA, never from the bytes
//! once they land, so a row is the height it will be from the moment it is
//! drawn and a picture arriving does not shove the chat.

use std::collections::{HashMap, HashSet};

use fc_text::i18n::{t, t1, t2, tn};
use fc_text::media;
use fc_text::transcript::State as TranscriptState;
use wasm_bindgen::JsCast;
use web_sys::HtmlAudioElement;
use yew::prelude::*;

use crate::awake::ScreenAwake;
use crate::media::{download, use_media, MediaLoader, Variant};
use crate::model::Attachment;
use crate::views::attach::{phone_like, PLAY_AFTER};
use crate::views::quiet::{use_quiet, use_quiet_explain, LiveRegion};

#[derive(Properties, PartialEq)]
pub struct StackProps {
    pub attachments: Vec<Attachment>,
    /// On the sender's own, tinted balloon.
    pub mine: bool,
    /// Open the viewer on these media, at this one.
    pub on_open: Callback<(Vec<Attachment>, usize)>,
    /// Something to say under the composer — a download that failed.
    #[prop_or_default]
    pub on_notice: Callback<String>,
    /// The text of these recordings, on request — None where none is
    /// offered and none is held.
    #[prop_or_default]
    pub transcribing: Option<Transcribing>,
}

/// One message's share of "Show text" (docs/protocol.md, "Transcripts on
/// request"): which of its recordings offer it, what this device holds for
/// them, and where a press goes.
#[derive(Clone, PartialEq, Default)]
pub struct Transcribing {
    /// The attachments "Show text" is drawn under
    /// (`fc_text::transcript::offers_show_text`).
    pub offered: HashSet<i64>,
    /// What this device holds for this message's attachments.
    pub held: HashMap<i64, TranscriptState>,
    /// "Show text", or "Try Again", by attachment id.
    pub on_show: Callback<i64>,
    /// "Hide text", by attachment id.
    pub on_hide: Callback<i64>,
}

/// One attachment exactly as it always drew; several as a pile of the
/// photos and videos (two or more) with the rest stacked under it, in the
/// order they were sent.
#[function_component(AttachmentStack)]
pub fn attachment_stack(props: &StackProps) -> Html {
    let media: Vec<Attachment> = props
        .attachments
        .iter()
        .filter(|attachment| media::is_media(&attachment.kind))
        .cloned()
        .collect();
    let rows: Vec<Attachment> = props
        .attachments
        .iter()
        .filter(|attachment| !media::is_media(&attachment.kind))
        .cloned()
        .collect();
    let open = |index: usize| {
        let on_open = props.on_open.clone();
        let media = media.clone();
        Callback::from(move |_: ()| on_open.emit((media.clone(), index)))
    };
    // A video's text goes under the picture, as a recording's goes under
    // its player — one block per video, in the order they were sent. In a
    // pile of two or more videos each block is marked with where its video
    // stands in the pile, a number every language reads, so that no text
    // is left wondering whose it is.
    let videos: Vec<(usize, i64)> = media
        .iter()
        .enumerate()
        .filter(|(_, attachment)| attachment.kind == "video")
        .map(|(index, attachment)| (index + 1, attachment.id))
        .collect();
    let marked = videos.len() >= 2;
    let video_text = props.transcribing.as_ref().map(|transcribing| {
        html! {
            { for videos.iter().map(|&(place, id)| html! {
                <TranscriptBlock
                    key={id}
                    state={transcribing.held.get(&id).cloned()}
                    offered={transcribing.offered.contains(&id)}
                    on_show={transcribing.on_show.reform(move |()| id)}
                    on_hide={transcribing.on_hide.reform(move |()| id)}
                    marker={marked.then(|| format!("▶ {place}"))}
                />
            }) }
        }
    });
    html! {
        <div class="attachments">
            if media.len() >= 2 {
                <Album items={media.clone()} mine={props.mine} on_open={open(0)} />
            } else if let Some(single) = media.first() {
                <Tile attachment={single.clone()} mine={props.mine} on_open={open(0)} />
            }
            { video_text.unwrap_or_default() }
            { for rows.iter().map(|attachment| html! {
                <Row
                    key={attachment.id}
                    attachment={attachment.clone()}
                    mine={props.mine}
                    on_notice={props.on_notice.clone()}
                    transcribing={props.transcribing.clone()}
                />
            }) }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct TileProps {
    attachment: Attachment,
    mine: bool,
    on_open: Callback<()>,
}

/// Which bytes a tile draws: the preview when there is one; a photo's own
/// bytes when there is not; and for a video without a poster, nothing —
/// a tile never downloads a whole video to draw itself (docs/protocol.md,
/// "A browser is a client too").
pub(crate) fn tile_source(attachment: &Attachment) -> Option<Variant> {
    if attachment.has_preview {
        Some(Variant::Preview)
    } else if attachment.kind == "photo" {
        Some(Variant::Original)
    } else {
        None
    }
}

/// A lone photo or video, at its own shape and capped at 320 either way.
#[function_component(Tile)]
fn tile(props: &TileProps) -> Html {
    let attachment = &props.attachment;
    let source = tile_source(attachment);
    let url = use_media(
        attachment.id,
        source.unwrap_or(Variant::Preview),
        source.is_some(),
    );
    let (width, height) = media::tile_size(attachment.width, attachment.height);
    let video = attachment.kind == "video";
    let open = props.on_open.reform(|_: MouseEvent| ());
    let label = if video { t("Video") } else { t("Photo") };
    // A spinner only for bytes that are on their way: a video with no poster
    // has none to wait for, and an outbox id whose bytes a reload took will
    // never have any.
    let loading = url.is_none() && source.is_some() && attachment.id > 0;
    html! {
        <button
            class={classes!("tile", props.mine.then_some("on-tint"), loading.then_some("is-loading"))}
            // Its own shape at up to 320 wide, and narrower where the pane
            // is — a thread panel, a phone — without losing the shape.
            style={format!("width:{width:.0}px;aspect-ratio:{width:.0}/{height:.0}")}
            onclick={open}
            aria-label={t1("Open %@", &label.to_lowercase())}
        >
            if let Some(url) = url {
                <img src={url} alt={label} draggable="false" />
            }
            if video {
                <span class="play" aria-hidden="true">{ "▶" }</span>
            }
        </button>
    }
}

#[derive(Properties, PartialEq)]
struct AlbumProps {
    items: Vec<Attachment>,
    mine: bool,
    on_open: Callback<()>,
}

/// Two or more photos and videos as a pile: the first at its own shape on
/// the card, the second and third peeking out behind it, the count in a
/// corner (MacAlbumStack).
#[function_component(Album)]
fn album(props: &AlbumProps) -> Html {
    let first = &props.items[0];
    let card = media::card_size(first.width, first.height, media::TILE_MAX);
    let open = props.on_open.reform(|_: MouseEvent| ());
    let behind = |index: usize, layer: media::Layer| {
        props.items.get(index).map(|item| {
            let offset = layer.offset(card);
            html! {
                // Tilted about its centre and lifted; shrunk about its TOP
                // edge by the card inside — two boxes, because one CSS
                // transform has one origin.
                <div
                    class="album-layer"
                    style={format!("transform:translateY(-{offset:.2}px) rotate({:.2}deg)", layer.tilt)}
                >
                    <div class="album-shrink" style={format!("transform:scale({:.2})", layer.scale)}>
                        <Card attachment={item.clone()} mine={props.mine} />
                    </div>
                </div>
            }
        })
    };
    html! {
        <button
            class="album"
            style={format!("width:{:.0}px;padding-top:{:.0}px", card.0, media::PEEK)}
            onclick={open}
            aria-label={tn("Album, 1 of %lld", props.items.len() as i64)}
        >
            <div class="album-card" style={format!("aspect-ratio:{:.0}/{:.0}", card.0, card.1)}>
                { behind(2, media::Layer::THIRD).unwrap_or_default() }
                { behind(1, media::Layer::SECOND).unwrap_or_default() }
                <div class="album-top">
                    <Card attachment={first.clone()} mine={props.mine} />
                    if first.kind == "video" {
                        <span class="play" aria-hidden="true">{ "▶" }</span>
                    }
                    <span class="album-count">{ format!("⧉ {}", props.items.len()) }</span>
                </div>
            </div>
        </button>
    }
}

#[derive(Properties, PartialEq)]
struct CardProps {
    attachment: Attachment,
    mine: bool,
}

/// One card of a pile: the item's picture filled into the card's frame.
#[function_component(Card)]
fn card(props: &CardProps) -> Html {
    let source = tile_source(&props.attachment);
    let url = use_media(
        props.attachment.id,
        source.unwrap_or(Variant::Preview),
        source.is_some(),
    );
    html! {
        <div class={classes!("card", props.mine.then_some("on-tint"))}>
            if let Some(url) = url {
                <img src={url} alt="" draggable="false" />
            }
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct RowProps {
    attachment: Attachment,
    mine: bool,
    on_notice: Callback<String>,
    #[prop_or_default]
    transcribing: Option<Transcribing>,
}

/// A file, a recording or a place: read, not looked at.
#[function_component(Row)]
fn row(props: &RowProps) -> Html {
    match props.attachment.kind.as_str() {
        "audio" => {
            let id = props.attachment.id;
            let text = props.transcribing.as_ref().map(|transcribing| {
                html! {
                    <TranscriptBlock
                        state={transcribing.held.get(&id).cloned()}
                        offered={transcribing.offered.contains(&id)}
                        on_show={transcribing.on_show.reform(move |()| id)}
                        on_hide={transcribing.on_hide.reform(move |()| id)}
                    />
                }
            });
            html! {
                <>
                    <AudioPlayer attachment={props.attachment.clone()} mine={props.mine} />
                    { text.unwrap_or_default() }
                </>
            }
        }
        "location" => {
            html! { <LocationRow attachment={props.attachment.clone()} mine={props.mine} /> }
        }
        _ => html! {
            <FileRow
                attachment={props.attachment.clone()}
                mine={props.mine}
                on_notice={props.on_notice.clone()}
            />
        },
    }
}

/// A long name with its middle given up, so both its start and its
/// extension stay readable — the Mac's `.truncationMode(.middle)`.
pub fn middle_truncate(name: &str, room: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= room || room < 5 {
        return name.to_string();
    }
    let tail = (room - 1) / 3;
    let head = room - 1 - tail;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// What a download is saved as: the attachment's name, or one made from
/// its kind and type.
pub fn file_name(attachment: &Attachment) -> String {
    if let Some(name) = attachment.name.as_deref().filter(|name| !name.is_empty()) {
        return name.to_string();
    }
    let extension = match attachment.mime.as_deref().map(media::essence).as_deref() {
        Some("image/jpeg") => "jpg",
        Some("image/png") => "png",
        Some("image/heic") => "heic",
        Some("video/quicktime") => "mov",
        Some("video/mp4") => "mp4",
        Some("audio/mp4") => "m4a",
        Some("audio/mpeg") => "mp3",
        Some("audio/wav") => "wav",
        Some("audio/ogg") => "ogg",
        _ => "bin",
    };
    format!("{}-{}.{extension}", attachment.kind, attachment.id.max(0))
}

#[derive(Properties, PartialEq)]
struct FileRowProps {
    attachment: Attachment,
    mine: bool,
    on_notice: Callback<String>,
}

/// A document: its name and its size, and a click that saves it.
#[function_component(FileRow)]
fn file_row(props: &FileRowProps) -> Html {
    let loader = use_context::<MediaLoader>();
    let busy = use_state(|| false);
    let attachment = props.attachment.clone();
    let name = media::display_name(&attachment.kind, attachment.name.as_deref()).into_owned();
    let size = attachment
        .size
        .map(|size| media::display_size(size.max(0) as u64))
        .unwrap_or_default();
    let save = {
        let busy = busy.clone();
        let on_notice = props.on_notice.clone();
        Callback::from(move |_: MouseEvent| {
            let Some(loader) = loader.clone() else { return };
            if *busy {
                return;
            }
            busy.set(true);
            let busy = busy.clone();
            let on_notice = on_notice.clone();
            let file = file_name(&attachment);
            loader.load(
                attachment.id,
                Variant::Original,
                Callback::from(move |url: Option<String>| {
                    busy.set(false);
                    match url {
                        Some(url) => download(&url, &file),
                        None => on_notice.emit(t("The file could not be downloaded.").to_string()),
                    }
                }),
            );
        })
    };
    html! {
        <button class={classes!("file-row", props.mine.then_some("on-tint"))} onclick={save}
                title={name.clone()} aria-label={t1("Save %@", &name)}>
            <span class="file-icon" aria-hidden="true">{ "📄" }</span>
            <span class="file-text">
                <span class="file-name">{ middle_truncate(&name, 40) }</span>
                <span class="file-size">
                    { if *busy { t("Preparing…").to_string() } else { size } }
                </span>
            </span>
        </button>
    }
}

#[derive(Properties, PartialEq)]
struct AudioProps {
    attachment: Attachment,
    mine: bool,
}

/// A recording: play and pause, a scrubber, the time gone and the whole —
/// and deliberately no waveform (docs/protocol.md, "Audio").
#[function_component(AudioPlayer)]
fn audio_player(props: &AudioProps) -> Html {
    let player = use_node_ref();
    // Asked to play — and whether it IS playing, which only the player
    // itself can say: a fetch that failed, or a browser that refused to
    // start playing outside the click, must not leave the button on ⏸
    // with nothing playing.
    let asked = use_state(|| false);
    let playing = use_state(|| false);
    let elapsed = use_state(|| 0.0_f64);
    let url = use_media(props.attachment.id, Variant::Original, *asked);
    let total = (props.attachment.duration_ms.unwrap_or(0) as f64 / 1000.0).max(0.1);
    // While a voice message is being recorded nothing of the app's plays
    // (the plan for #79, S1.7): the ▶ is dimmed — not disabled — and says
    // why when pressed, on the recording pane's notice line; a screen reader
    // hears the same sentence with the control (S6).
    let dimmed = use_quiet();
    let explain = use_quiet_explain();
    let reason_id = use_memo((), |_| crate::views::dialog::fresh_id("play-reason"));

    let start = {
        let asked = asked.clone();
        move |audio: &HtmlAudioElement| {
            if let Ok(promise) = audio.play() {
                let asked = asked.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Refused — Safari outside a click, usually. Asked again
                    // with the bytes here, the click itself starts it.
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                        asked.set(false);
                    }
                });
            }
        }
    };
    // The bytes land after the first click: play them then.
    {
        let player = player.clone();
        let asked_now = *asked;
        let start = start.clone();
        use_effect_with(url.clone(), move |url| {
            if url.is_some() && asked_now {
                if let Some(audio) = player.cast::<HtmlAudioElement>() {
                    start(&audio);
                }
            }
        });
    }

    let toggle = {
        let player = player.clone();
        let asked = asked.clone();
        let elapsed = elapsed.clone();
        let loaded = url.is_some();
        let is_playing = *playing;
        let explain = explain.clone();
        Callback::from(move |_: MouseEvent| {
            if dimmed {
                explain.emit(t(PLAY_AFTER).to_string());
                return;
            }
            let audio = player.cast::<HtmlAudioElement>();
            if is_playing {
                if let Some(audio) = audio {
                    let _ = audio.pause();
                }
                asked.set(false);
                return;
            }
            if *asked && !loaded {
                // Still fetching: a second click gives it up.
                asked.set(false);
                return;
            }
            asked.set(true);
            if let Some(audio) = audio.filter(|_| loaded) {
                if *elapsed >= total - 0.2 {
                    audio.set_current_time(0.0);
                }
                start(&audio);
            }
        })
    };
    let on_time = {
        let elapsed = elapsed.clone();
        Callback::from(move |event: Event| {
            if let Some(audio) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlAudioElement>().ok())
            {
                elapsed.set(audio.current_time());
            }
        })
    };
    // On a phone's browser the screen is kept on while a voice note plays,
    // so the lock does not hide the tab halfway through a long one (the plan
    // for #79, S1.7, S8.8); let go of when it stops, ends or goes.
    let awake = use_mut_ref(|| Option::<ScreenAwake>::None);
    let on_play = {
        let playing = playing.clone();
        let awake = awake.clone();
        Callback::from(move |_: Event| {
            playing.set(true);
            if phone_like() {
                *awake.borrow_mut() = Some(ScreenAwake::hold());
            }
        })
    };
    let on_pause = {
        let playing = playing.clone();
        let awake = awake.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
            awake.borrow_mut().take();
        })
    };
    let on_ended = {
        let playing = playing.clone();
        let asked = asked.clone();
        let elapsed = elapsed.clone();
        let awake = awake.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
            awake.borrow_mut().take();
            asked.set(false);
            elapsed.set(total);
        })
    };
    let on_scrub = {
        let player = player.clone();
        let elapsed = elapsed.clone();
        Callback::from(move |event: InputEvent| {
            let input: web_sys::HtmlInputElement = event.target_unchecked_into();
            let to = input.value().parse::<f64>().unwrap_or(0.0);
            elapsed.set(to);
            if let Some(audio) = player.cast::<HtmlAudioElement>() {
                audio.set_current_time(to);
            }
        })
    };
    let loading = *asked && !*playing && url.is_none();
    let (glyph, label) = if *playing {
        ("⏸", t("Pause"))
    } else if loading {
        ("…", t("Loading"))
    } else {
        ("▶", t("Play"))
    };
    html! {
        <div class={classes!("audio", props.mine.then_some("on-tint"))}
             aria-label={t1("Audio, %@", &media::time_label(total))}>
            <button class={classes!("audio-toggle", dimmed.then_some("is-dimmed"))}
                    onclick={toggle} aria-label={label}
                    aria-disabled={dimmed.then_some("true")}
                    aria-describedby={dimmed.then(|| (*reason_id).clone())}>{ glyph }</button>
            if dimmed {
                <span id={(*reason_id).clone()} hidden=true>{ t(PLAY_AFTER) }</span>
            }
            <div class="audio-track">
                <input
                    type="range"
                    min="0"
                    max={format!("{total:.2}")}
                    step="0.1"
                    value={format!("{:.2}", elapsed.min(total))}
                    oninput={on_scrub}
                    aria-label={t("Position")}
                />
                <div class="audio-times">
                    <span>{ media::time_label(*elapsed) }</span>
                    <span>{ media::time_label(total) }</span>
                </div>
            </div>
            <audio ref={player} src={url.unwrap_or_default()} preload="auto"
                   ontimeupdate={on_time} onplay={on_play} onpause={on_pause} onended={on_ended} />
        </div>
    }
}

#[derive(Properties, PartialEq)]
struct LocationProps {
    attachment: Attachment,
    mine: bool,
}

/// A place: its label, the numbers, and a hand-off to a map. No map is
/// drawn here — no tile provider is contacted on a family's behalf
/// (docs/protocol.md, "Locations": a client may draw the pin, the label
/// and a hand-off instead).
#[function_component(LocationRow)]
fn location_row(props: &LocationProps) -> Html {
    let attachment = &props.attachment;
    let name = media::display_name("location", attachment.name.as_deref()).into_owned();
    let place = attachment.latitude.zip(attachment.longitude);
    let line = place
        .map(|(latitude, longitude)| {
            media::location_line(latitude, longitude, attachment.accuracy_m)
        })
        .unwrap_or_default();
    let open = place.map(|(latitude, longitude)| {
        let url = media::maps_url(latitude, longitude, attachment.name.as_deref());
        Callback::from(move |_: MouseEvent| {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target_and_features(
                    &url,
                    "_blank",
                    "noopener,noreferrer",
                );
            }
        })
    });
    html! {
        <button class={classes!("location-row", props.mine.then_some("on-tint"))}
                onclick={open.unwrap_or_default()} disabled={place.is_none()}
                aria-label={t2("%@. %@. Open in Maps", &name, &line)}>
            <span class="pin" aria-hidden="true">{ "📍" }</span>
            <span class="file-text">
                <span class="file-name">{ name }</span>
                if !line.is_empty() {
                    <span class="file-size">{ line }</span>
                }
            </span>
            if place.is_some() {
                <span class="open-maps">{ t("Open in Maps") }</span>
            }
        </button>
    }
}

#[derive(Properties, PartialEq)]
pub struct TranscriptProps {
    /// What this device holds for the recording, if anything.
    pub state: Option<TranscriptState>,
    /// Whether "Show text" is offered for it at all.
    pub offered: bool,
    pub on_show: Callback<()>,
    pub on_hide: Callback<()>,
    /// Which of a pile's videos this is the text of — `▶ 2` — where there
    /// is more than one.
    #[prop_or_default]
    pub marker: Option<String>,
}

/// Under a recording's player, or a video's picture: "Show text", then
/// "Getting the text…", then the text itself — selectable, and labelled as
/// the recording's so that a screen reader does not read it as the message
/// — with "Hide text".
/// Silence is "No speech", an answer and not a failure; a failure is one
/// line, with "Try Again" only where asking again could help.
#[function_component(TranscriptBlock)]
pub fn transcript_block(props: &TranscriptProps) -> Html {
    let show = {
        let on_show = props.on_show.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            on_show.emit(());
        })
    };
    let hide = {
        let on_hide = props.on_hide.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            on_hide.emit(());
        })
    };
    let marker = props.marker.as_ref().map(|marker| {
        html! { <span class="transcript-marker">{ marker.clone() }</span> }
    });
    let show_button = |label: &'static str| {
        html! {
            <button type="button" class="link transcript-action" onclick={show.clone()}>{ label }</button>
        }
    };
    match &props.state {
        None if props.offered => html! {
            <div class="transcript">{ marker.clone().unwrap_or_default() }{ show_button(t("Show text")) }</div>
        },
        None => Html::default(),
        Some(TranscriptState::Asking) => html! {
            <div class="transcript">{ marker.clone().unwrap_or_default() }
                <LiveRegion class="transcript-status" role="status" busy=true>{ t("Getting the text…") }</LiveRegion>
            </div>
        },
        Some(TranscriptState::Hidden(_)) => html! {
            <div class="transcript">{ marker.clone().unwrap_or_default() }{ show_button(t("Show text")) }</div>
        },
        Some(TranscriptState::Shown(said)) => {
            // Selecting a word is a double-click, and a double-click on a
            // bubble is a heart: the text keeps its own.
            let keep = Callback::from(|event: MouseEvent| event.stop_propagation());
            let lang = said
                .language
                .as_deref()
                .filter(|code| {
                    (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphabetic())
                })
                .map(str::to_ascii_lowercase);
            html! {
                <div class="transcript">{ marker.clone().unwrap_or_default() }
                    <div class="transcript-text" role="group"
                        aria-label={t("Text of the recording")} ondblclick={keep}>
                        if said.is_silence() {
                            <p class="is-silence">{ t("No speech") }</p>
                        } else {
                            <p {lang}>{ said.text.clone() }</p>
                        }
                    </div>
                    <button type="button" class="link transcript-action" onclick={hide}>{ t("Hide text") }</button>
                </div>
            }
        }
        Some(TranscriptState::Failed(failure)) => html! {
            <div class="transcript">{ marker.clone().unwrap_or_default() }
                <LiveRegion class="transcript-failure" role="status">{ failure.sentence() }</LiveRegion>
                if failure.may_retry() && props.offered {
                    { show_button(t("Try Again")) }
                }
            </div>
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn a_long_name_gives_up_its_middle() {
        assert_eq!(middle_truncate("short.pdf", 40), "short.pdf");
        let long = format!("{}.pdf", "Quarterly report ".repeat(5));
        let cut = middle_truncate(&long, 20);
        assert_eq!(cut.chars().count(), 20);
        assert!(
            cut.starts_with("Quarterly re") && cut.ends_with("t .pdf"),
            "{cut}"
        );
        assert!(cut.contains('…'));
    }

    #[wasm_bindgen_test]
    fn a_download_is_named_for_what_it_is() {
        let named = Attachment {
            id: 9,
            kind: "file".into(),
            name: Some("receipts.pdf".into()),
            ..Attachment::default()
        };
        assert_eq!(file_name(&named), "receipts.pdf");
        let photo = Attachment {
            id: 34,
            kind: "photo".into(),
            mime: Some("image/jpeg".into()),
            ..Attachment::default()
        };
        assert_eq!(file_name(&photo), "photo-34.jpg");
        let clip = Attachment {
            id: 35,
            kind: "video".into(),
            mime: Some("video/quicktime".into()),
            ..Attachment::default()
        };
        assert_eq!(file_name(&clip), "video-35.mov");
    }

    #[derive(Properties, PartialEq)]
    struct QuietTranscriptProps {
        state: TranscriptState,
        recording: bool,
    }

    /// A recording's text line beside a recording somewhere in the app.
    #[function_component(QuietTranscript)]
    fn quiet_transcript(props: &QuietTranscriptProps) -> Html {
        use crate::views::quiet::QuietRoot;
        #[derive(Properties, PartialEq)]
        struct RecordingProps {
            on: bool,
        }
        #[function_component(Recording)]
        fn recording(props: &RecordingProps) -> Html {
            crate::views::quiet::use_quiet_while(props.on);
            Html::default()
        }
        html! {
            <QuietRoot>
                <Recording on={props.recording} />
                <TranscriptBlock
                    state={Some(props.state.clone())}
                    offered=true
                    on_show={Callback::noop()}
                    on_hide={Callback::noop()}
                />
            </QuietRoot>
        }
    }

    /// "Getting the text…" and a failure are live regions of the app's: quiet
    /// while a voice message is being recorded, so a transcript finishing
    /// meanwhile is not spoken into the note (the plan for #79, S6).
    #[wasm_bindgen_test]
    async fn a_recordings_text_line_is_quiet_while_a_voice_message_is_recorded() {
        use fc_text::transcript::Failure;
        for (state, line) in [
            (TranscriptState::Asking, ".transcript-status"),
            (
                TranscriptState::Failed(Failure::TryAgain),
                ".transcript-failure",
            ),
        ] {
            for recording in [false, true] {
                let document = web_sys::window().unwrap().document().unwrap();
                let root = document.create_element("div").unwrap();
                document.body().unwrap().append_child(&root).unwrap();
                let handle = yew::Renderer::<QuietTranscript>::with_root_and_props(
                    root.clone(),
                    QuietTranscriptProps {
                        state: state.clone(),
                        recording,
                    },
                )
                .render();
                gloo_timers::future::TimeoutFuture::new(30).await;
                let said = root.query_selector(line).unwrap().expect(line);
                let quiet = (said.get_attribute("role"), said.get_attribute("aria-live"));
                if recording {
                    assert_eq!(quiet, (None, Some("off".into())), "{line}: quiet");
                } else {
                    assert_eq!(quiet, (Some("status".into()), None), "{line}: loud");
                }
                handle.destroy();
                root.remove();
            }
        }
    }

    #[derive(Properties, PartialEq)]
    struct WithLoaderProps {
        loader: MediaLoader,
        attachment: Attachment,
    }

    #[function_component(WithLoader)]
    fn with_loader(props: &WithLoaderProps) -> Html {
        html! {
            <ContextProvider<MediaLoader> context={props.loader.clone()}>
                <AttachmentStack attachments={vec![props.attachment.clone()]} mine=false
                                 on_open={Callback::noop()} />
            </ContextProvider<MediaLoader>>
        }
    }

    /// On a phone's browser the screen is kept on while a received voice
    /// note plays, and let go of when it is paused (the plan for #79, S1.7,
    /// S8.8: playback pauses with a hidden tab, so the lock must not cut a
    /// long note short). On a desktop it is not asked for.
    #[wasm_bindgen_test]
    async fn a_voice_note_keeps_a_phones_screen_on_while_it_plays() {
        use crate::awake::testing::FakeWakeLock;
        use wasm_bindgen::JsValue;
        let run = |source: &str| {
            js_sys::Function::new_no_args(source)
                .call0(&JsValue::NULL)
                .unwrap()
        };
        for phone in [true, false] {
            let screen = FakeWakeLock::install();
            if phone {
                run("window.__fcMatchMedia = window.matchMedia; \
                     window.matchMedia = (query) => ({ matches: query === '(pointer: coarse)', \
                       media: query, addListener() {}, removeListener() {}, \
                       addEventListener() {}, removeEventListener() {} });");
            }
            let rate = fc_text::wav::VOICE_RATE;
            let samples: Vec<f32> = (0..2 * rate)
                .map(|at| (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.3)
                .collect();
            let bytes = fc_text::wav::encode(&samples, rate);
            let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
            let options = web_sys::BlobPropertyBag::new();
            options.set_type("audio/wav");
            let blob =
                web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap();
            let loader = MediaLoader::new(crate::live::Live::new(
                crate::live::AppState {
                    token: Some("t".into()),
                    ..Default::default()
                },
                std::rc::Rc::new(|| {}),
            ));
            loader.seed(31, Variant::Original, blob);
            let attachment = Attachment {
                id: 31,
                kind: "audio".into(),
                mime: Some("audio/wav".into()),
                duration_ms: Some(2_000),
                ..Attachment::default()
            };
            let document = web_sys::window().unwrap().document().unwrap();
            let root = document.create_element("div").unwrap();
            document.body().unwrap().append_child(&root).unwrap();
            let handle = yew::Renderer::<WithLoader>::with_root_and_props(
                root.clone(),
                WithLoaderProps { loader, attachment },
            )
            .render();
            gloo_timers::future::TimeoutFuture::new(50).await;
            let toggle = root
                .query_selector(".audio-toggle")
                .unwrap()
                .expect("a play button")
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap();
            let player = root
                .query_selector("audio")
                .unwrap()
                .unwrap()
                .dyn_into::<HtmlAudioElement>()
                .unwrap();
            toggle.click();
            let mut waited = 0;
            while player.paused() && waited < 2_000 {
                gloo_timers::future::TimeoutFuture::new(20).await;
                waited += 20;
            }
            assert!(!player.paused(), "it plays");
            gloo_timers::future::TimeoutFuture::new(50).await;
            assert_eq!(screen.asked(), u32::from(phone), "phone={phone}: asked");
            let _ = player.pause();
            gloo_timers::future::TimeoutFuture::new(50).await;
            assert_eq!(
                screen.released(),
                u32::from(phone),
                "phone={phone}: let go of"
            );
            handle.destroy();
            root.remove();
            if phone {
                run("window.matchMedia = window.__fcMatchMedia; delete window.__fcMatchMedia;");
            }
        }
    }

    #[derive(Properties, PartialEq)]
    struct RecordingBesideProps {
        loader: MediaLoader,
        attachment: Attachment,
        recording: bool,
        said: Callback<String>,
    }

    /// A received voice note beside a recording somewhere in the app, the
    /// recording pane's notice line heard.
    #[function_component(RecordingBeside)]
    fn recording_beside(props: &RecordingBesideProps) -> Html {
        #[derive(Properties, PartialEq)]
        struct PaneProps {
            on: bool,
            said: Callback<String>,
        }
        #[function_component(Pane)]
        fn pane(props: &PaneProps) -> Html {
            crate::views::quiet::use_quiet_while(props.on);
            crate::views::quiet::use_quiet_reason(props.said.clone());
            Html::default()
        }
        html! {
            <crate::views::quiet::QuietRoot>
                <Pane on={props.recording} said={props.said.clone()} />
                <WithLoader loader={props.loader.clone()} attachment={props.attachment.clone()} />
            </crate::views::quiet::QuietRoot>
        }
    }

    /// NOTHING OF THE APP'S PLAYS WHILE A VOICE MESSAGE IS RECORDED (the plan
    /// for #79, S1.7, S6): a received voice note's ▶ is dimmed — not
    /// disabled — with the reason given to a screen reader beside it
    /// (`aria-describedby`); pressed, it plays nothing and says "You can
    /// play this after recording." on the recording pane's line. Once the
    /// recording is over it plays.
    #[wasm_bindgen_test]
    async fn a_voice_notes_play_is_dimmed_while_a_voice_message_is_recorded() {
        let rate = fc_text::wav::VOICE_RATE;
        let samples: Vec<f32> = (0..2 * rate)
            .map(|at| (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.3)
            .collect();
        let bytes = fc_text::wav::encode(&samples, rate);
        let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
        let options = web_sys::BlobPropertyBag::new();
        options.set_type("audio/wav");
        let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap();
        let loader = MediaLoader::new(crate::live::Live::new(
            crate::live::AppState {
                token: Some("t".into()),
                ..Default::default()
            },
            std::rc::Rc::new(|| {}),
        ));
        loader.seed(32, Variant::Original, blob);
        let attachment = Attachment {
            id: 32,
            kind: "audio".into(),
            mime: Some("audio/wav".into()),
            duration_ms: Some(2_000),
            ..Attachment::default()
        };
        let heard = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let said = {
            let heard = heard.clone();
            Callback::from(move |text: String| heard.borrow_mut().push(text))
        };
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().append_child(&root).unwrap();
        let mut handle = yew::Renderer::<RecordingBeside>::with_root_and_props(
            root.clone(),
            RecordingBesideProps {
                loader: loader.clone(),
                attachment: attachment.clone(),
                recording: true,
                said: said.clone(),
            },
        )
        .render();
        gloo_timers::future::TimeoutFuture::new(50).await;
        let toggle = root
            .query_selector(".audio-toggle")
            .unwrap()
            .expect("a play button")
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap();
        assert_eq!(
            toggle.get_attribute("aria-disabled").as_deref(),
            Some("true")
        );
        assert!(!toggle.has_attribute("disabled"), "dimmed, not disabled");
        assert!(toggle.class_list().contains("is-dimmed"));
        let reason = toggle
            .get_attribute("aria-describedby")
            .and_then(|id| document.get_element_by_id(&id))
            .and_then(|reason| reason.text_content());
        assert_eq!(
            reason.as_deref(),
            Some("You can play this after recording.")
        );
        let player = root
            .query_selector("audio")
            .unwrap()
            .unwrap()
            .dyn_into::<HtmlAudioElement>()
            .unwrap();
        toggle.click();
        gloo_timers::future::TimeoutFuture::new(300).await;
        assert!(player.paused(), "it plays nothing");
        assert_eq!(toggle.get_attribute("aria-label").as_deref(), Some("Play"));
        assert_eq!(
            heard.borrow().as_slice(),
            ["You can play this after recording.".to_string()]
        );

        // The recording over: it plays.
        handle.update(RecordingBesideProps {
            loader,
            attachment,
            recording: false,
            said,
        });
        gloo_timers::future::TimeoutFuture::new(50).await;
        assert!(toggle.get_attribute("aria-disabled").is_none());
        assert!(toggle.get_attribute("aria-describedby").is_none());
        toggle.click();
        let mut waited = 0;
        while player.paused() && waited < 2_000 {
            gloo_timers::future::TimeoutFuture::new(20).await;
            waited += 20;
        }
        assert!(!player.paused(), "it plays once the recording is over");
        let _ = player.pause();
        assert_eq!(heard.borrow().len(), 1);
        handle.destroy();
        root.remove();
    }

    /// Which bytes a tile asks for — never a whole video.
    #[wasm_bindgen_test]
    fn a_tile_never_downloads_a_video_to_draw_itself() {
        let photo = |has_preview| Attachment {
            kind: "photo".into(),
            has_preview,
            ..Attachment::default()
        };
        let video = |has_preview| Attachment {
            kind: "video".into(),
            has_preview,
            ..Attachment::default()
        };
        assert_eq!(tile_source(&photo(true)), Some(Variant::Preview));
        assert_eq!(tile_source(&photo(false)), Some(Variant::Original));
        assert_eq!(tile_source(&video(true)), Some(Variant::Preview));
        assert_eq!(tile_source(&video(false)), None);
    }
}

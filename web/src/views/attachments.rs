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
use wasm_bindgen::closure::Closure;
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
    /// Who is looking — whose unplayed dots a voice message draws; 0 draws
    /// none.
    #[prop_or_default]
    pub my_user_id: i64,
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
                    my_user_id={props.my_user_id}
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
    #[prop_or_default]
    my_user_id: i64,
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
                    <AudioPlayer attachment={props.attachment.clone()} mine={props.mine}
                                 my_user_id={props.my_user_id} />
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
    /// Who is looking — whose unplayed dots these are; 0 draws none.
    #[prop_or_default]
    my_user_id: i64,
}

/// The bars a voice message is drawn with: its 48 levels as sent
/// (docs/protocol.md, "A voice note's waveform"), or the neutral placeholder
/// where it has none or it cannot be read.
pub const VOICE_BARS: usize = fc_text::waveform::LEVELS;

/// The voice bubble's glyphs, drawn inline.
pub(crate) const PLAY_PATH: &str =
    "M8 5.14v13.72a1 1 0 0 0 1.5.86l11-6.86a1 1 0 0 0 0-1.72l-11-6.86A1 1 0 0 0 8 5.14z";
pub(crate) const PAUSE_PATH: &str =
    "M7 5h3a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zm7 0h3a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1z";

pub(crate) fn glyph(path: &'static str) -> Html {
    html! {
        <svg class="voice-glyph" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
            <path fill="currentColor" d={path} />
        </svg>
    }
}

/// A waveform's bars: each `(2 + level) / 17` of the full height, the first
/// `played` of them in the accent colour (docs/protocol.md, "A voice note's
/// waveform", readers). Drawn alike by the bubble, the review chip and the
/// not-sent chip.
pub(crate) fn waveform_bars(levels: &[u8], played: usize) -> Html {
    html! {
        { for levels.iter().enumerate().map(|(index, level)| {
            let height = fc_text::waveform::bar_fraction(*level) * 100.0;
            html! {
                <i class={classes!((index < played).then_some("is-played"))}
                   style={format!("height:{height:.1}%")}></i>
            }
        }) }
    }
}

/// A speed as its chip says it — "1×", "1.5×", "2×" — in the reader's
/// language (a comma where the language writes one).
pub fn speed_label(speed: f64) -> String {
    if speed == 1.5 {
        t("1.5×").to_string()
    } else if speed == 2.0 {
        t("2×").to_string()
    } else {
        t("1×").to_string()
    }
}

/// The event every voice bubble on the page hears when the speed changes —
/// from a chip, or the message menu's "Playback speed".
pub const SPEED_EVENT: &str = "fc-voice-speed";

/// The marker of a voice bubble's player, which a change of speed applies to
/// at once, playing or not.
const VOICE_PLAYER: &str = "data-voice-note";

/// Set the speed voice messages play at on this device, and apply it to
/// every voice bubble now — playing ones too — and to their chips.
pub fn set_voice_speed(speed: f64) {
    crate::session::set_voice_speed(speed);
    let speed = crate::session::voice_speed();
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Ok(found) = document.query_selector_all(&format!("audio[{VOICE_PLAYER}]")) {
        for index in 0..found.length() {
            if let Some(player) = found
                .item(index)
                .and_then(|node| node.dyn_into::<web_sys::HtmlMediaElement>().ok())
            {
                player.set_playback_rate(speed);
            }
        }
    }
    if let (Some(window), Ok(event)) = (web_sys::window(), web_sys::Event::new(SPEED_EVENT)) {
        let _ = window.dispatch_event(&event);
    }
}

/// How far a key moves a voice message's position, in seconds.
const SEEK_STEP: f64 = 5.0;

/// Where a key on the waveform puts the position of a message `total`
/// seconds long that is at `at` — None for a key that does not move it.
pub fn seek_by_key(key: &str, at: f64, total: f64) -> Option<f64> {
    let to = match key {
        "ArrowRight" | "ArrowUp" => at + SEEK_STEP,
        "ArrowLeft" | "ArrowDown" => at - SEEK_STEP,
        "PageUp" => at + total / 10.0,
        "PageDown" => at - total / 10.0,
        "Home" => 0.0,
        "End" => total,
        _ => return None,
    };
    Some(to.clamp(0.0, total))
}

/// Where a pointer `x` across a waveform from `left` that is `width` wide
/// puts the position of a message `total` seconds long.
pub fn seek_by_pointer(x: f64, left: f64, width: f64, total: f64) -> f64 {
    if !(width > 0.0) || !x.is_finite() {
        return 0.0;
    }
    (((x - left) / width).clamp(0.0, 1.0) * total).clamp(0.0, total)
}

/// A pointer down on a voice message's waveform.
struct WaveDrag {
    /// Where it came down.
    x: f64,
    y: f64,
    /// The position before it came down — what a scroll leaves it at.
    before: f64,
    /// Whether it is seeking: a mouse at once; a finger once it has moved
    /// along the waveform.
    seeking: bool,
}

/// How far, in CSS pixels, a finger moves along the waveform before it is
/// seeking rather than perhaps starting a scroll.
const SEEK_SLOP: f64 = 8.0;

/// Whether a finger `dx` across and `dy` down from where it landed has
/// moved ALONG the waveform — enough, and more across than down.
pub fn along_the_wave(dx: f64, dy: f64) -> bool {
    dx.abs() >= SEEK_SLOP && dx.abs() > dy.abs()
}

/// A voice message (the approved design for #79): a round accent play
/// button; the waveform the sender measured, played bars in the accent
/// colour as it plays, which a tap or a drag seeks and a screen reader
/// adjusts as a slider; the time gone while it plays and the whole at rest,
/// in tabular digits; a speed chip once it has started, paused part way
/// included — 1×, 1.5×, 2×, the device's own; and an unplayed dot until this device has played it, never
/// on one's own. "Show text" goes under it, as before.
#[function_component(AudioPlayer)]
fn audio_player(props: &AudioProps) -> Html {
    let player = use_node_ref();
    let wave = use_node_ref();
    // Asked to play — and whether it IS playing, which only the player
    // itself can say: a fetch that failed, or a browser that refused to
    // start playing outside the click, must not leave the button on ⏸
    // with nothing playing.
    let asked = use_state(|| false);
    let playing = use_state(|| false);
    let elapsed = use_state(|| 0.0_f64);
    let speed = use_state(crate::session::voice_speed);
    // A seek made before the bytes are here: where the first play starts.
    let seek_to = use_mut_ref(|| Option::<f64>::None);
    // A finger or a mouse down on the waveform, until it is lifted.
    let dragging = use_mut_ref(|| Option::<WaveDrag>::None);
    let url = use_media(props.attachment.id, Variant::Original, *asked);
    let duration_ms = props.attachment.duration_ms.unwrap_or(0).max(0);
    let total = (duration_ms as f64 / 1000.0).max(0.1);
    let id = props.attachment.id;
    let me = props.my_user_id;
    let watches = !props.mine && me != 0 && id > 0;
    let played = use_state(|| crate::session::voice_played(me, id));
    {
        let played = played.clone();
        use_effect_with((me, id), move |(me, id)| {
            played.set(crate::session::voice_played(*me, *id));
        });
    }
    // A change of speed anywhere — another bubble's chip, the menu — is
    // this bubble's too.
    {
        let speed = speed.clone();
        use_effect_with((), move |_| {
            let heard = Closure::<dyn Fn()>::new(move || speed.set(crate::session::voice_speed()));
            let window = web_sys::window();
            if let Some(window) = &window {
                let _ = window
                    .add_event_listener_with_callback(SPEED_EVENT, heard.as_ref().unchecked_ref());
            }
            move || {
                if let Some(window) = window {
                    let _ = window.remove_event_listener_with_callback(
                        SPEED_EVENT,
                        heard.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    // While a voice message is being recorded nothing of the app's plays
    // (the plan for #79, S1.7): the ▶ is dimmed — not disabled — and says
    // why when pressed, on the recording pane's notice line; a screen reader
    // hears the same sentence with the control (S6).
    let dimmed = use_quiet();
    let explain = use_quiet_explain();
    let reason_id = use_memo((), |_| crate::views::dialog::fresh_id("play-reason"));
    let state_id = use_memo((), |_| crate::views::dialog::fresh_id("voice-state"));

    let start = {
        let asked = asked.clone();
        let seek_to = seek_to.clone();
        move |audio: &HtmlAudioElement| {
            if let Some(at) = seek_to.borrow_mut().take() {
                audio.set_current_time(at);
            }
            audio.set_playback_rate(crate::session::voice_speed());
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
        let seek_to = seek_to.clone();
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
            let from = if *elapsed >= total - 0.2 {
                0.0
            } else {
                *elapsed
            };
            *seek_to.borrow_mut() = Some(from);
            if let Some(audio) = audio.filter(|_| loaded) {
                start(&audio);
            }
        })
    };
    // Seeking: the position moves at once, and the player with it when its
    // bytes are here — or where the first play starts when they are not.
    let seek = {
        let player = player.clone();
        let elapsed = elapsed.clone();
        let seek_to = seek_to.clone();
        let loaded = url.is_some();
        Callback::from(move |to: f64| {
            let to = to.clamp(0.0, total);
            elapsed.set(to);
            match player.cast::<HtmlAudioElement>().filter(|_| loaded) {
                Some(audio) => audio.set_current_time(to),
                // Without the bytes — before Play, or while they are on their
                // way after it — the position is where the first play starts
                // (`start`), not where Play was pressed.
                None => *seek_to.borrow_mut() = Some(to),
            }
        })
    };
    let at_pointer = {
        let wave = wave.clone();
        move |event: &PointerEvent| {
            let rect = wave
                .cast::<web_sys::Element>()
                .map(|element| element.get_bounding_client_rect());
            rect.map(|rect| {
                seek_by_pointer(
                    f64::from(event.client_x()),
                    rect.left(),
                    rect.width(),
                    total,
                )
            })
        }
    };
    // A mouse seeks where it presses. A finger (or a pen) may be starting
    // a scroll of the chat — the waveform lets the browser pan it
    // vertically — so it seeks only once it has moved ALONG the waveform,
    // or when it is lifted where it landed, a tap; and when the browser
    // takes the touch for a scroll (`pointercancel`), the position is
    // what it was before the finger came down.
    let on_wave_down = {
        let seek = seek.clone();
        let dragging = dragging.clone();
        let at_pointer = at_pointer.clone();
        let player = player.clone();
        let loaded = url.is_some();
        let before = *elapsed;
        Callback::from(move |event: PointerEvent| {
            if event.button() != 0 {
                return;
            }
            let touch = matches!(event.pointer_type().as_str(), "touch" | "pen");
            let before = player
                .cast::<HtmlAudioElement>()
                .filter(|_| loaded)
                .map_or(before, |audio| audio.current_time());
            *dragging.borrow_mut() = Some(WaveDrag {
                x: f64::from(event.client_x()),
                y: f64::from(event.client_y()),
                before,
                seeking: !touch,
            });
            if let Some(target) = event
                .target()
                .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
            {
                let _ = target.set_pointer_capture(event.pointer_id());
            }
            if !touch {
                if let Some(to) = at_pointer(&event) {
                    seek.emit(to);
                }
            }
        })
    };
    let on_wave_move = {
        let seek = seek.clone();
        let dragging = dragging.clone();
        let at_pointer = at_pointer.clone();
        Callback::from(move |event: PointerEvent| {
            let seeking = {
                let mut drag = dragging.borrow_mut();
                let Some(drag) = drag.as_mut() else { return };
                if !drag.seeking {
                    drag.seeking = along_the_wave(
                        f64::from(event.client_x()) - drag.x,
                        f64::from(event.client_y()) - drag.y,
                    );
                }
                drag.seeking
            };
            if seeking {
                if let Some(to) = at_pointer(&event) {
                    seek.emit(to);
                }
            }
        })
    };
    let on_wave_up = {
        let seek = seek.clone();
        let dragging = dragging.clone();
        Callback::from(move |event: PointerEvent| {
            // A tap: lifted where it landed, never having moved along.
            if dragging
                .borrow_mut()
                .take()
                .is_some_and(|drag| !drag.seeking)
            {
                if let Some(to) = at_pointer(&event) {
                    seek.emit(to);
                }
            }
        })
    };
    let on_wave_cancel = {
        let seek = seek.clone();
        let dragging = dragging.clone();
        Callback::from(move |_: PointerEvent| {
            if let Some(drag) = dragging.borrow_mut().take().filter(|drag| drag.seeking) {
                seek.emit(drag.before);
            }
        })
    };
    let on_wave_key = {
        let seek = seek.clone();
        let at = *elapsed;
        Callback::from(move |event: KeyboardEvent| {
            if let Some(to) = seek_by_key(&event.key(), at, total) {
                event.prevent_default();
                seek.emit(to);
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
        let played = played.clone();
        Callback::from(move |event: Event| {
            playing.set(true);
            if let Some(audio) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlAudioElement>().ok())
            {
                audio.set_playback_rate(crate::session::voice_speed());
            }
            if watches {
                crate::session::mark_voice_played(me, id);
                played.set(true);
            }
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
            // Back at rest: the whole length, no bar played.
            elapsed.set(0.0);
        })
    };
    let cycle_speed = Callback::from(move |event: MouseEvent| {
        event.stop_propagation();
        set_voice_speed(crate::session::next_voice_speed(
            crate::session::voice_speed(),
        ));
    });
    let keep = Callback::from(|event: MouseEvent| event.stop_propagation());

    let loading = *asked && !*playing && url.is_none();
    let (path, label) = if *playing {
        (PAUSE_PATH, t("Pause"))
    } else if loading {
        (PLAY_PATH, t("Loading"))
    } else {
        (PLAY_PATH, t("Play"))
    };
    let levels = fc_text::waveform::levels_or_placeholder(props.attachment.waveform.as_deref());
    let started = *playing || *elapsed > 0.0;
    let played_bars = if started {
        fc_text::waveform::played_bars(
            (*elapsed * 1000.0).max(0.0) as u64,
            duration_ms as u64,
            VOICE_BARS,
        )
    } else {
        0
    };
    let shown = if started { *elapsed } else { total };
    let length = media::time_label(total);
    let name = if props.attachment.name.is_none() {
        t1("Voice message, %@", &length)
    } else {
        t1("Audio, %@", &length)
    };
    let unplayed = watches && !*played;
    let described: Vec<String> = [
        watches.then(|| (*state_id).clone()),
        dimmed.then(|| (*reason_id).clone()),
    ]
    .into_iter()
    .flatten()
    .collect();
    let speed_now = speed_label(*speed);
    html! {
        <div class={classes!("audio", props.mine.then_some("on-tint"), (*playing).then_some("is-playing"))}
             role="group" aria-label={name}>
            <button class={classes!("audio-toggle", dimmed.then_some("is-dimmed"), loading.then_some("is-loading"))}
                    onclick={toggle} aria-label={label}
                    aria-disabled={dimmed.then_some("true")}
                    aria-describedby={(!described.is_empty()).then(|| described.join(" "))}>
                { glyph(path) }
            </button>
            if dimmed {
                <span id={(*reason_id).clone()} hidden=true>{ t(PLAY_AFTER) }</span>
            }
            if watches {
                <span id={(*state_id).clone()} hidden=true>
                    { if unplayed { t("Not played") } else { t("Played") } }
                </span>
            }
            <div
                ref={wave}
                class="audio-wave"
                role="slider"
                tabindex="0"
                aria-label={t("Position")}
                aria-valuemin="0"
                aria-valuemax={format!("{:.0}", total.round())}
                aria-valuenow={format!("{:.0}", elapsed.clamp(0.0, total).round())}
                aria-valuetext={media::time_label(if started { *elapsed } else { 0.0 })}
                onpointerdown={on_wave_down}
                onpointermove={on_wave_move}
                onpointerup={on_wave_up}
                onpointercancel={on_wave_cancel}
                onkeydown={on_wave_key}
                onclick={keep.clone()}
                ondblclick={keep}
            >
                { waveform_bars(&levels, played_bars) }
            </div>
            <div class="audio-meta">
                <span class="audio-time">{ media::time_label(shown) }</span>
                if unplayed {
                    <span class="audio-dot" aria-hidden="true"></span>
                }
                // Once started and until it is back at rest — paused part way
                // included, as on iOS, Android and Windows.
                if started {
                    <button type="button" class="audio-speed" onclick={cycle_speed}
                            aria-label={t1("Playback speed, %@", &speed_now)}>
                        { speed_now.clone() }
                    </button>
                }
            </div>
            <audio ref={player} src={url.unwrap_or_default()} preload="auto"
                   data-playback="true" data-voice-note="true"
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

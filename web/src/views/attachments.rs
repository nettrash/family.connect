//! A message's attachments, drawn: the photos and videos as one tile or a
//! pile, and the files, recordings and places as rows under it — the Mac's
//! composition (MacMessageRow.attachmentStack, AttachmentAlbum), from the
//! same rules (fc_text::media).
//!
//! Everything is sized from the attachment's METADATA, never from the bytes
//! once they land, so a row is the height it will be from the moment it is
//! drawn and a picture arriving does not shove the chat.

use fc_text::media;
use wasm_bindgen::JsCast;
use web_sys::HtmlAudioElement;
use yew::prelude::*;

use crate::media::{download, use_media, MediaLoader, Variant};
use crate::model::Attachment;

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
    html! {
        <div class="attachments">
            if media.len() >= 2 {
                <Album items={media.clone()} mine={props.mine} on_open={open(0)} />
            } else if let Some(single) = media.first() {
                <Tile attachment={single.clone()} mine={props.mine} on_open={open(0)} />
            }
            { for rows.iter().map(|attachment| html! {
                <Row
                    key={attachment.id}
                    attachment={attachment.clone()}
                    mine={props.mine}
                    on_notice={props.on_notice.clone()}
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
    let label = if video { "Video" } else { "Photo" };
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
            aria-label={format!("Open {}", label.to_lowercase())}
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
            aria-label={format!("Album, 1 of {}", props.items.len())}
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
}

/// A file, a recording or a place: read, not looked at.
#[function_component(Row)]
fn row(props: &RowProps) -> Html {
    match props.attachment.kind.as_str() {
        "audio" => {
            html! { <AudioPlayer attachment={props.attachment.clone()} mine={props.mine} /> }
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
                        None => on_notice.emit("The file could not be downloaded.".to_string()),
                    }
                }),
            );
        })
    };
    html! {
        <button class={classes!("file-row", props.mine.then_some("on-tint"))} onclick={save}
                title={name.clone()} aria-label={format!("Save {name}")}>
            <span class="file-icon" aria-hidden="true">{ "📄" }</span>
            <span class="file-text">
                <span class="file-name">{ middle_truncate(&name, 40) }</span>
                <span class="file-size">
                    { if *busy { "Preparing…".to_string() } else { size } }
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
        Callback::from(move |_: MouseEvent| {
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
    let on_play = {
        let playing = playing.clone();
        Callback::from(move |_: Event| playing.set(true))
    };
    let on_pause = {
        let playing = playing.clone();
        Callback::from(move |_: Event| playing.set(false))
    };
    let on_ended = {
        let playing = playing.clone();
        let asked = asked.clone();
        let elapsed = elapsed.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
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
        ("⏸", "Pause")
    } else if loading {
        ("…", "Loading")
    } else {
        ("▶", "Play")
    };
    html! {
        <div class={classes!("audio", props.mine.then_some("on-tint"))}
             aria-label={format!("Audio, {}", media::time_label(total))}>
            <button class="audio-toggle" onclick={toggle} aria-label={label}>{ glyph }</button>
            <div class="audio-track">
                <input
                    type="range"
                    min="0"
                    max={format!("{total:.2}")}
                    step="0.1"
                    value={format!("{:.2}", elapsed.min(total))}
                    oninput={on_scrub}
                    aria-label="Position"
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
                aria-label={format!("{name}. {line}. Open in Maps")}>
            <span class="pin" aria-hidden="true">{ "📍" }</span>
            <span class="file-text">
                <span class="file-name">{ name }</span>
                if !line.is_empty() {
                    <span class="file-size">{ line }</span>
                }
            </span>
            if place.is_some() {
                <span class="open-maps">{ "Open in Maps" }</span>
            }
        </button>
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

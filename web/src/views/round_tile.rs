//! A video message, drawn: the circle (the plan for #79,
//! docs/audio-video-messages-2026-10-04.md, S5; docs/protocol.md, "Video
//! messages").
//!
//! No balloon round it, like a sticker. 200 across in a window under 720 px
//! and 240 from 720 px (`fc_text::record::round_diameter`), the square
//! poster filling the circle — a neutral disc of the same size until it
//! lands, so the row never changes height. Only the POSTER is fetched to draw
//! it: a tile never downloads a video to draw itself (docs/protocol.md, "A
//! browser is a client too").
//!
//! A tap fetches the original and plays it in place, at the same size, with
//! sound; a ring runs round the edge; another tap pauses; at the end it
//! returns to the poster and its unplayed dot goes. A tapped circle shows a
//! loading ring at once, a second tap while it loads gives up, and a failure
//! leaves the poster with a line that says so. On Safari, which refuses to
//! start playing outside the tap, it returns to its play glyph when the bytes
//! are in and the next tap plays them — the voice note's own pattern.
//! Never autoplay, muted or otherwise (S5.3, Decision 21).

use fc_text::i18n::{t, t1};
use fc_text::media;
use fc_text::record::{self, WidthClass};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;
use yew::prelude::*;

use crate::awake::ScreenAwake;
use crate::media::{use_media, MediaLoader, Variant};
use crate::model::Attachment;
use crate::views::attach::{phone_like, PLAY_AFTER};
use crate::views::attachments::{Transcribing, TranscriptBlock};
use crate::views::quiet::{use_quiet, use_quiet_explain};

/// The style that sizes a circle: both diameters, the window decides which
/// (styles.css `.round-frame`, under and from 720 px).
pub fn diameters() -> String {
    format!(
        "--round-compact:{}px;--round-regular:{}px",
        record::round_diameter(WidthClass::Compact),
        record::round_diameter(WidthClass::Regular),
    )
}

/// How far round the ring is, 0 to 1, at `elapsed` of `total` seconds —
/// stepped once a second where motion is reduced (S6), swept otherwise.
pub fn ring_progress(elapsed: f64, total: f64, reduced_motion: bool) -> f64 {
    if !(total > 0.0) || !elapsed.is_finite() {
        return 0.0;
    }
    let at = if reduced_motion {
        elapsed.floor()
    } else {
        elapsed
    };
    (at / total).clamp(0.0, 1.0)
}

fn reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}

#[derive(Properties, PartialEq)]
pub struct RoundProps {
    pub attachment: Attachment,
    /// Who is looking — whose unplayed dots these are (S5.2).
    pub my_user_id: i64,
    /// Sent from this device: no dot — there is nothing new in one's own.
    pub mine: bool,
    /// Not on the server yet: drawn from this tab's own poster, with a thin
    /// neutral ring while it goes up (S5.6).
    #[prop_or_default]
    pub sending: bool,
    /// "Open Full Screen": the viewer, with scrubbing (S5.4).
    pub on_open: Callback<Attachment>,
    /// "Show text" under it, outside its gestures (S5.5).
    #[prop_or_default]
    pub transcribing: Option<Transcribing>,
}

/// The circle, its expand control while it plays, the failed line, and the
/// text of what was said under it.
#[function_component(RoundVideoTile)]
pub fn round_video_tile(props: &RoundProps) -> Html {
    let attachment = &props.attachment;
    let id = attachment.id;
    let me = props.my_user_id;
    let poster = use_media(id, Variant::Preview, attachment.has_preview);
    let loader = use_context::<MediaLoader>();
    let player = use_node_ref();
    let frame = use_node_ref();
    // Asked to play, the bytes once they are here, whether it IS playing —
    // which only the player can say — and whether it has started since it
    // last ended (the poster shows until it has).
    let asked = use_state(|| false);
    let url = use_state(|| Option::<String>::None);
    let playing = use_state(|| false);
    let started = use_state(|| false);
    let failed = use_state(|| false);
    let elapsed = use_state(|| 0.0_f64);
    // A fetch given up by a second tap must not land afterwards.
    let fetch = use_mut_ref(|| 0_u32);
    // Where the last start began, so a double click — the heart — leaves the
    // circle where it was.
    let resumed_from = use_mut_ref(|| 0.0_f64);
    let played = use_state(|| session_played(me, id));
    {
        let played = played.clone();
        use_effect_with((me, id), move |(me, id)| {
            played.set(session_played(*me, *id));
        });
    }
    let total = (attachment.duration_ms.unwrap_or(0) as f64 / 1000.0).max(0.1);
    let dimmed = use_quiet();
    let explain = use_quiet_explain();
    let not_played_id = use_memo((), |_| crate::views::dialog::fresh_id("round-not-played"));
    let reason_id = use_memo((), |_| crate::views::dialog::fresh_id("round-reason"));

    let start = {
        let asked = asked.clone();
        let resumed_from = resumed_from.clone();
        move |video: &HtmlVideoElement| {
            if video.ended() || video.current_time() >= video.duration() - 0.05 {
                video.set_current_time(0.0);
            }
            *resumed_from.borrow_mut() = video.current_time();
            if let Ok(promise) = video.play() {
                let asked = asked.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Refused — Safari outside the tap, usually. Back to the
                    // play glyph with the bytes here: the next tap plays.
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                        asked.set(false);
                    }
                });
            }
        }
    };
    // The bytes land after the tap: play them then.
    {
        let player = player.clone();
        let asked_now = *asked;
        let start = start.clone();
        use_effect_with((*url).clone(), move |url| {
            if url.is_some() && asked_now {
                if let Some(video) = player.cast::<HtmlVideoElement>() {
                    start(&video);
                }
            }
        });
    }
    // A circle stops when its row scrolls out of view (S5.3).
    {
        let player = player.clone();
        let frame = frame.clone();
        use_effect_with(*playing, move |playing| {
            let observer = (*playing)
                .then(|| {
                    let player = player.clone();
                    let gone =
                        Closure::<dyn Fn(js_sys::Array)>::new(move |entries: js_sys::Array| {
                            let out = entries.iter().any(|entry| {
                                entry
                                    .dyn_into::<web_sys::IntersectionObserverEntry>()
                                    .is_ok_and(|entry| !entry.is_intersecting())
                            });
                            if out {
                                if let Some(video) = player.cast::<HtmlVideoElement>() {
                                    let _ = video.pause();
                                }
                            }
                        });
                    let observer =
                        web_sys::IntersectionObserver::new(gone.as_ref().unchecked_ref()).ok()?;
                    observer.observe(&frame.cast::<web_sys::Element>()?);
                    Some((observer, gone))
                })
                .flatten();
            move || {
                if let Some((observer, _gone)) = observer {
                    observer.disconnect();
                }
            }
        });
    }

    let toggle = {
        let player = player.clone();
        let asked = asked.clone();
        let url = url.clone();
        let failed = failed.clone();
        let fetch = fetch.clone();
        let resumed_from = resumed_from.clone();
        let started = started.clone();
        let is_playing = *playing;
        let explain = explain.clone();
        let start = start.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            if dimmed {
                explain.emit(t(PLAY_AFTER).to_string());
                return;
            }
            let video = player.cast::<HtmlVideoElement>();
            // The second click of a double click is the heart's, and puts
            // back what the first one did: a pause goes back to where the
            // play began.
            let second_of_double = event.detail() >= 2;
            if is_playing {
                if let Some(video) = video {
                    let _ = video.pause();
                    if second_of_double {
                        let back = *resumed_from.borrow();
                        video.set_current_time(back);
                        if back <= 0.0 {
                            started.set(false);
                        }
                    }
                }
                asked.set(false);
                return;
            }
            if *asked && url.is_none() {
                // Still fetching: a second tap gives it up.
                *fetch.borrow_mut() += 1;
                asked.set(false);
                return;
            }
            failed.set(false);
            asked.set(true);
            if url.is_some() {
                if let Some(video) = video {
                    start(&video);
                }
                return;
            }
            let Some(loader) = loader.clone() else {
                failed.set(true);
                asked.set(false);
                return;
            };
            *fetch.borrow_mut() += 1;
            let this_fetch = *fetch.borrow();
            let fetch = fetch.clone();
            let url = url.clone();
            let failed = failed.clone();
            let asked = asked.clone();
            loader.load(
                id,
                Variant::Original,
                Callback::from(move |landed: Option<String>| {
                    if *fetch.borrow() != this_fetch {
                        return;
                    }
                    match landed {
                        Some(landed) => url.set(Some(landed)),
                        None => {
                            failed.set(true);
                            asked.set(false);
                        }
                    }
                }),
            );
        })
    };
    // On a phone's browser the screen stays on while it plays (S1.7).
    let awake = use_mut_ref(|| Option::<ScreenAwake>::None);
    let on_play = {
        let playing = playing.clone();
        let started = started.clone();
        let awake = awake.clone();
        Callback::from(move |_: Event| {
            playing.set(true);
            started.set(true);
            if phone_like() {
                *awake.borrow_mut() = Some(ScreenAwake::hold());
            }
        })
    };
    let on_pause = {
        let playing = playing.clone();
        let asked = asked.clone();
        let awake = awake.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
            asked.set(false);
            awake.borrow_mut().take();
        })
    };
    let on_ended = {
        let playing = playing.clone();
        let started = started.clone();
        let asked = asked.clone();
        let elapsed = elapsed.clone();
        let awake = awake.clone();
        let played = played.clone();
        Callback::from(move |_: Event| {
            playing.set(false);
            started.set(false);
            asked.set(false);
            elapsed.set(0.0);
            awake.borrow_mut().take();
            crate::session::mark_round_played(me, id);
            played.set(true);
        })
    };
    let on_time = {
        let elapsed = elapsed.clone();
        Callback::from(move |event: Event| {
            if let Some(video) = event
                .target()
                .and_then(|target| target.dyn_into::<HtmlVideoElement>().ok())
            {
                elapsed.set(video.current_time());
            }
        })
    };
    let open = {
        let on_open = props.on_open.clone();
        let player = player.clone();
        let attachment = attachment.clone();
        Callback::from(move |event: MouseEvent| {
            event.stop_propagation();
            if let Some(video) = player.cast::<HtmlVideoElement>() {
                let _ = video.pause();
            }
            on_open.emit(attachment.clone());
        })
    };

    let loading = *asked && !*playing && url.is_none();
    let unplayed = !props.mine && !*played && id > 0;
    let duration = media::time_label(total);
    let progress = ring_progress(*elapsed, total, reduced_motion());
    let ring = if props.sending {
        Some(("is-sending", 25.0))
    } else if loading {
        Some(("is-loading", 25.0))
    } else if *started {
        Some(("is-progress", progress * 100.0))
    } else {
        None
    };
    let keep = Callback::from(|event: MouseEvent| event.stop_propagation());
    html! {
        <div class="round-video" style={diameters()}>
            <div
                ref={frame}
                class={classes!(
                    "round-frame",
                    poster.is_none().then_some("is-waiting"),
                    loading.then_some("is-loading"),
                    (*playing).then_some("is-playing"),
                    (*failed).then_some("is-failed"),
                )}
            >
                <button
                    type="button"
                    class={classes!("round-face", dimmed.then_some("is-dimmed"))}
                    onclick={toggle}
                    aria-label={t1("Video message, %@", &duration)}
                    aria-pressed={if *playing { "true" } else { "false" }}
                    aria-disabled={dimmed.then_some("true")}
                    aria-describedby={
                        let mut said = Vec::new();
                        if unplayed { said.push((*not_played_id).clone()); }
                        if dimmed { said.push((*reason_id).clone()); }
                        (!said.is_empty()).then(|| said.join(" "))
                    }
                >
                    if let Some(poster) = poster.clone().filter(|_| !*started) {
                        <img class="round-poster" src={poster} alt="" draggable="false" />
                    }
                    if let Some(url) = (*url).clone() {
                        <video
                            ref={player.clone()}
                            class={classes!("round-player", (!*started).then_some("is-idle"))}
                            src={url}
                            preload="auto"
                            playsinline=true
                            data-playback="true"
                            onplay={on_play}
                            onpause={on_pause}
                            onended={on_ended}
                            ontimeupdate={on_time}
                        />
                    }
                    if !*playing && !loading {
                        <span class="round-play" aria-hidden="true">{ "▶" }</span>
                    }
                    <span class="round-duration" aria-hidden="true">
                        { if *started { media::time_label(*elapsed) } else { duration.clone() } }
                        if unplayed {
                            <span class="round-dot"></span>
                        }
                    </span>
                    if let Some((kind, length)) = ring {
                        <svg class={classes!("round-ring", kind)} viewBox="0 0 100 100" aria-hidden="true">
                            <circle cx="50" cy="50" r="48.5" pathLength="100"
                                stroke-dasharray={format!("{length:.2} 100")}
                                transform="rotate(-90 50 50)" />
                        </svg>
                    }
                </button>
                if *started {
                    <button type="button" class="round-expand" onclick={open}
                            title={t("Open Full Screen")} aria-label={t("Open Full Screen")}>
                        <span aria-hidden="true">{ "⤢" }</span>
                    </button>
                }
            </div>
            if unplayed {
                <span id={(*not_played_id).clone()} hidden=true>{ t("Not played") }</span>
            }
            if dimmed {
                <span id={(*reason_id).clone()} hidden=true>{ t(PLAY_AFTER) }</span>
            }
            if *failed {
                <p class="round-failed" role="status" ondblclick={keep.clone()}>{ t("Couldn't load the video. Tap to try again.") }</p>
            }
            if let Some(transcribing) = &props.transcribing {
                <div class="round-transcript" ondblclick={keep}>
                    <TranscriptBlock
                        state={transcribing.held.get(&id).cloned()}
                        offered={transcribing.offered.contains(&id)}
                        on_show={transcribing.on_show.reform(move |()| id)}
                        on_hide={transcribing.on_hide.reform(move |()| id)}
                    />
                </div>
            }
        </div>
    }
}

fn session_played(me: i64, id: i64) -> bool {
    crate::session::round_played(me, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// The ring sweeps with the time played, and steps once a second where
    /// motion is reduced (S6) — never past the edge, never backwards past
    /// its start.
    #[wasm_bindgen_test]
    fn the_ring_follows_the_time_and_steps_under_reduced_motion() {
        assert_eq!(ring_progress(0.0, 20.0, false), 0.0);
        assert!((ring_progress(5.5, 20.0, false) - 0.275).abs() < 1e-9);
        assert!((ring_progress(5.5, 20.0, true) - 0.25).abs() < 1e-9);
        assert_eq!(ring_progress(25.0, 20.0, false), 1.0);
        assert_eq!(ring_progress(-1.0, 20.0, false), 0.0);
        assert_eq!(ring_progress(3.0, 0.0, false), 0.0);
        assert_eq!(ring_progress(f64::NAN, 20.0, false), 0.0);
    }

    /// Both diameters come from the shared rule: 200 and 240.
    #[wasm_bindgen_test]
    fn the_circle_takes_its_sizes_from_the_shared_rule() {
        assert_eq!(diameters(), "--round-compact:200px;--round-regular:240px");
    }
}

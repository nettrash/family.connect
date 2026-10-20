//! An animation, drawn (issue #96, docs/protocol.md "Animations" → "How it is
//! drawn"): a PICTURE that moves, not a video.
//!
//! In the box a photograph of its shape would take, with no play button, no
//! duration, no sound control and no scrubber — only a small "GIF" in a
//! corner. It plays by itself, muted, and loops for ever while it is on
//! screen; scrolled just off it, it is paused where it is — scrolling back
//! and forth across the edge does not take the video down and fetch and
//! decode it again each time — and scrolled further away its `<video>` is
//! taken out altogether, which lets go of its decoder. It shows its poster
//! instead, still, when the system asks for less motion
//! (`prefers-reduced-motion: reduce`) and when it is above 8 MiB — the bound
//! on the one exception to "a tile never downloads a video"
//! (docs/protocol.md, "A browser is a client too"). A click opens it larger,
//! playing, muted and looping, without controls.
//!
//! Its bytes come as every attachment's do — fetched with the session's
//! bearer header into an object URL, never a URL with a token in it — under
//! a bound of their own (`Variant::Animation`). Neither `<video>` is part of
//! the app's playback (crate::now_playing): a muted picture moving must
//! never pause a voice note somebody is listening to.

use fc_text::animation_source::plays_in_tile;
use fc_text::i18n::{t, t1};
use fc_text::media;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::HtmlVideoElement;
use yew::prelude::*;

use crate::media::{use_media, Variant};
use crate::model::Attachment;

/// The system's "less motion" setting, as a query this page can ask and
/// listen to.
fn motion_query() -> Option<web_sys::MediaQueryList> {
    web_sys::window()?
        .match_media("(prefers-reduced-motion: reduce)")
        .ok()
        .flatten()
}

/// Whether the system asks for less motion, right now.
pub fn reduced_motion() -> bool {
    motion_query().is_some_and(|query| query.matches())
}

/// Whether the system asks for less motion — read once as the component
/// is first drawn, and drawn again when the setting changes.
///
/// It used to be read on every draw, which honoured a change "from the next
/// draw" only because every bubble was drawn again whenever anything at all
/// changed. Now that a bubble is drawn again only when something of its own
/// changes, the next draw could be a long way off; the query's own `change`
/// is what says the setting moved, and honours it at once.
#[hook]
pub fn use_reduced_motion() -> bool {
    let reduced = use_state(reduced_motion);
    {
        let reduced = reduced.clone();
        use_effect_with((), move |_| {
            let query = motion_query();
            let heard = query.as_ref().map(|query| {
                let asked = query.clone();
                let heard = Closure::<dyn Fn()>::new(move || reduced.set(asked.matches()));
                let _ = query
                    .add_event_listener_with_callback("change", heard.as_ref().unchecked_ref());
                heard
            });
            move || {
                if let (Some(query), Some(heard)) = (query, heard) {
                    let _ = query.remove_event_listener_with_callback(
                        "change",
                        heard.as_ref().unchecked_ref(),
                    );
                }
            }
        });
    }
    *reduced
}

/// How far below or above the list a tile may be and still fetch its
/// picture — about a screen, so a reader scrolling at an ordinary pace
/// meets pictures that are already there.
pub const FETCH_AHEAD_PX: u32 = 800;

/// How far off the list an animation may be and keep its `<video>`, paused:
/// a reader scrolling back finds it where it stopped, without a fetch and a
/// decode; further away it is taken out and lets go of its decoder.
pub const KEEP_PLAYER_PX: u32 = 300;

/// Whether the element behind `node` is on screen, as an
/// `IntersectionObserver` sees it — false until it has said, so nothing is
/// fetched for a tile that was never looked at.
#[hook]
pub fn use_on_screen(node: NodeRef) -> bool {
    use_within(node, None)
}

/// Whether the element behind `node` is within `margin_px` of being on
/// screen — of the list it scrolls in (`.messages`, `.thread-messages`), or
/// of the window where it is in neither.
///
/// The LIST, where there is one, because the margin is the observer's root
/// margin: with the window as the root, the list scrolling inside it clips
/// a row by its own edges and the margin never reaches past them.
#[hook]
pub fn use_near_screen(node: NodeRef, margin_px: u32) -> bool {
    use_within(node, Some(margin_px))
}

#[hook]
fn use_within(node: NodeRef, margin_px: Option<u32>) -> bool {
    let visible = use_state(|| false);
    {
        let visible = visible.clone();
        use_effect_with((), move |_| {
            let seen = Closure::<dyn Fn(js_sys::Array)>::new(move |entries: js_sys::Array| {
                // The last word on it is the one that holds.
                if let Some(entry) = entries
                    .iter()
                    .filter_map(|entry| entry.dyn_into::<web_sys::IntersectionObserverEntry>().ok())
                    .next_back()
                {
                    visible.set(entry.is_intersecting());
                }
            });
            let element = node.cast::<web_sys::Element>();
            let observer = match margin_px {
                None => web_sys::IntersectionObserver::new(seen.as_ref().unchecked_ref()),
                Some(margin) => {
                    let options = web_sys::IntersectionObserverInit::new();
                    let list = element
                        .as_ref()
                        .and_then(|element| element.closest(".messages, .thread-messages").ok())
                        .flatten();
                    options.set_root(list.as_ref());
                    options.set_root_margin(&format!("{margin}px 0px"));
                    web_sys::IntersectionObserver::new_with_options(
                        seen.as_ref().unchecked_ref(),
                        &options,
                    )
                }
            }
            .ok()
            .and_then(|observer| {
                observer.observe(element.as_ref()?);
                Some(observer)
            });
            move || {
                if let Some(observer) = observer {
                    observer.disconnect();
                }
                drop(seen);
            }
        });
    }
    *visible
}

/// Muted for certain — the property, which a browser's autoplay rules go by,
/// and not only the attribute — and playing while `on_screen`, paused where
/// it is otherwise. A refused play (a browser that will not, a tab in the
/// background) is not an error: the picture simply waits, still, on its
/// first frame.
fn play_muted(player: &NodeRef, on_screen: bool) {
    let Some(video) = player.cast::<HtmlVideoElement>() else {
        return;
    };
    video.set_muted(true);
    video.set_default_muted(true);
    video.set_loop(true);
    if !on_screen {
        let _ = video.pause();
        return;
    }
    if let Ok(promise) = video.play() {
        wasm_bindgen_futures::spawn_local(async move {
            let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
        });
    }
}

#[derive(Properties, PartialEq)]
pub struct TileProps {
    pub attachment: Attachment,
    pub mine: bool,
    pub on_open: Callback<()>,
}

/// An animation in a chat: the tile a photo of its shape would be, moving
/// while it is on screen, its poster otherwise, and "GIF" in its corner.
#[function_component(AnimationTile)]
pub fn animation_tile(props: &TileProps) -> Html {
    let attachment = &props.attachment;
    let frame = use_node_ref();
    let player = use_node_ref();
    let on_screen = use_on_screen(frame.clone());
    // On screen or just off it: the video is kept — playing on screen,
    // paused off it (`play_muted`).
    let kept = use_near_screen(frame.clone(), KEEP_PLAYER_PX) || on_screen;
    let plays = plays_in_tile(attachment.size, use_reduced_motion());
    let poster = use_media(attachment.id, Variant::Preview, attachment.has_preview);
    let moving = use_media(attachment.id, Variant::Animation, plays && kept);
    let moving = moving.filter(|_| plays && kept);
    {
        let player = player.clone();
        use_effect_with((moving.clone(), on_screen), move |(moving, on_screen)| {
            if moving.is_some() {
                play_muted(&player, *on_screen);
            }
        });
    }
    let (width, height) = media::tile_size(attachment.width, attachment.height);
    let open = props.on_open.reform(|_: MouseEvent| ());
    let word = t("GIF");
    let loading = moving.is_none()
        && poster.is_none()
        && attachment.id > 0
        && (attachment.has_preview || (plays && kept));
    html! {
        <button
            ref={frame}
            class={classes!("tile", "is-animation", props.mine.then_some("on-tint"), loading.then_some("is-loading"))}
            style={format!("width:{width:.0}px;aspect-ratio:{width:.0}/{height:.0}")}
            onclick={open}
            aria-label={t1("Open %@", word)}
        >
            if let Some(url) = moving {
                <video
                    ref={player}
                    src={url}
                    poster={poster.unwrap_or_default()}
                    muted=true
                    loop=true
                    autoplay=true
                    playsinline=true
                    disablepictureinpicture=""
                    disableremoteplayback=""
                    preload="auto"
                    aria-hidden="true"
                />
            } else if let Some(url) = poster {
                <img src={url} alt={word} draggable="false" />
            }
            <span class="gif-label" aria-hidden="true">{ word }</span>
        </button>
    }
}

#[derive(Properties, PartialEq)]
pub struct ViewProps {
    pub attachment: Attachment,
}

/// An animation opened larger: playing, muted and looping, with no player
/// controls — even for a reader who asked for less motion, who asked for it
/// by opening it.
#[function_component(AnimationView)]
pub fn animation_view(props: &ViewProps) -> Html {
    let attachment = &props.attachment;
    let player = use_node_ref();
    let full = use_media(attachment.id, Variant::Original, true);
    let poster = use_media(attachment.id, Variant::Preview, attachment.has_preview);
    {
        let player = player.clone();
        use_effect_with(full.clone(), move |full| {
            if full.is_some() {
                play_muted(&player, true);
            }
        });
    }
    html! {
        <div class="viewer-video viewer-animation">
            if let Some(url) = full {
                <video
                    ref={player}
                    src={url}
                    poster={poster.unwrap_or_default()}
                    muted=true
                    loop=true
                    autoplay=true
                    playsinline=true
                    disablepictureinpicture=""
                    aria-label={t("GIF")}
                />
            } else {
                if let Some(poster) = poster {
                    <img src={poster} alt="" draggable="false" />
                }
                <p class="viewer-loading is-over">{ t("Loading video…") }</p>
            }
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    use wasm_bindgen_test::*;
    use web_sys::{Blob, Element};

    use crate::live::{AppState, Live};
    use crate::media::MediaLoader;

    #[derive(Properties, PartialEq)]
    struct HarnessProps {
        loader: MediaLoader,
        attachment: Attachment,
    }

    #[function_component(Harness)]
    fn harness(props: &HarnessProps) -> Html {
        html! {
            <ContextProvider<MediaLoader> context={props.loader.clone()}>
                <AnimationTile attachment={props.attachment.clone()} mine={false}
                               on_open={Callback::noop()} />
            </ContextProvider<MediaLoader>>
        }
    }

    fn loader() -> MediaLoader {
        let loader = MediaLoader::new(Live::new(
            AppState {
                token: Some("t".into()),
                ..AppState::default()
            },
            Rc::new(|| {}),
        ));
        let blob = |text: &str| {
            Blob::new_with_str_sequence(&js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(text)))
                .unwrap()
        };
        loader.seed(93, Variant::Preview, blob("the poster"));
        loader.seed(93, Variant::Animation, blob("the video"));
        loader
    }

    fn animation(size: i64) -> Attachment {
        Attachment {
            id: 93,
            kind: "video".into(),
            mime: Some("video/mp4".into()),
            size: Some(size),
            width: Some(480),
            height: Some(270),
            duration_ms: Some(2_100),
            has_preview: true,
            animation: true,
            ..Attachment::default()
        }
    }

    async fn render(attachment: Attachment) -> (Element, yew::AppHandle<Harness>) {
        let document = web_sys::window().unwrap().document().unwrap();
        let root = document.create_element("div").unwrap();
        document.body().unwrap().prepend_with_node_1(&root).unwrap();
        let handle = yew::Renderer::<Harness>::with_root_and_props(
            root.clone(),
            HarnessProps {
                loader: loader(),
                attachment,
            },
        )
        .render();
        // The observer says it is on screen, and the tile draws again.
        for _ in 0..25 {
            gloo_timers::future::TimeoutFuture::new(20).await;
            if root.query_selector("video").unwrap().is_some() {
                break;
            }
        }
        (root, handle)
    }

    /// ON SCREEN, IT PLAYS BY ITSELF: a muted, looping `<video>` that is not
    /// the app's playback — nothing a voice note is paused for. Above 8 MiB,
    /// or for a reader who asked for less motion, it is its poster, still,
    /// with the label (docs/protocol.md, "How it is drawn").
    #[wasm_bindgen_test]
    async fn a_tile_plays_muted_on_screen_and_is_its_poster_otherwise() {
        let (root, handle) = render(animation(182_330)).await;
        let video: HtmlVideoElement = root
            .query_selector(".tile.is-animation video")
            .unwrap()
            .expect("it plays by itself")
            .dyn_into()
            .unwrap();
        assert!(video.muted() && video.loop_(), "muted, and for ever");
        assert!(!video.has_attribute(crate::now_playing::PLAYBACK));
        assert!(!video.controls());
        assert!(root.query_selector(".gif-label").unwrap().is_some());
        handle.destroy();
        root.remove();

        let (root, handle) = render(animation(8_388_609)).await;
        assert!(
            root.query_selector("video").unwrap().is_none(),
            "above 8 MiB"
        );
        assert!(root
            .query_selector(".tile.is-animation img")
            .unwrap()
            .is_some());
        handle.destroy();
        root.remove();

        let answer = js_sys::Function::new_with_args(
            "query",
            "return {matches: true, media: query, addEventListener() {}, removeEventListener() {}};",
        );
        let _less = crate::webcodecs::testing::Stand::in_for("matchMedia", &answer);
        let (root, handle) = render(animation(182_330)).await;
        assert!(
            root.query_selector("video").unwrap().is_none(),
            "less motion"
        );
        assert!(root
            .query_selector(".tile.is-animation img")
            .unwrap()
            .is_some());
        assert!(root.query_selector(".gif-label").unwrap().is_some());
        handle.destroy();
        root.remove();
    }
}

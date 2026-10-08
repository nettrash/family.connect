//! Video messages, received — Phase 2 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md, S5, S4's last column, S8.7 and
//! S8.8): the circle draws from its poster alone, plays in place on a tap,
//! says when it cannot load, gives a fetch up on a second tap, takes its
//! unplayed dot away at the end and leaves a double click (the heart) where
//! it found it; one thing plays at a time, and a hidden tab, a call, an
//! output that went and a chat that closed all pause it; and it is 200
//! across under 720 px and 240 from 720 px.
//!
//! Tested in a real browser against the shipped stylesheet. The video is a
//! real one — the H.264 fixture the encoder's tests use — handed to the
//! media cache the way a fetch would, so that nothing here needs a server.

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlElement, HtmlMediaElement, HtmlVideoElement};
use yew::prelude::*;

use crate::layout_tests::{query, IntoHtml};
use crate::media::{MediaLoader, Variant};
use crate::model::Attachment;
use crate::now_playing::testing::Outputs;
use crate::views::round_tile::RoundVideoTile;

const WITHIN: &[u8] = include_bytes!("../text/fixtures/within-h264-720p30.mp4");
const ME: i64 = 9301;

fn run(source: &str) {
    js_sys::Function::new_no_args(source)
        .call0(&JsValue::NULL)
        .expect("running a stand-in");
}

fn blob(bytes: &[u8], mime: &str) -> web_sys::Blob {
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).expect("a blob")
}

/// A 1×1 JPEG-ish stand-in for the poster: what is checked is that it is
/// drawn, not what it shows.
fn poster() -> web_sys::Blob {
    blob(b"poster", "image/jpeg")
}

/// A media cache — signed in (`token`) or not.
fn loader(token: Option<&str>) -> MediaLoader {
    MediaLoader::new(crate::live::Live::new(
        crate::live::AppState {
            token: token.map(str::to_string),
            ..Default::default()
        },
        std::rc::Rc::new(|| {}),
    ))
}

fn round(id: i64) -> Attachment {
    Attachment {
        id,
        kind: "video".into(),
        mime: Some("video/mp4".into()),
        size: Some(WITHIN.len() as i64),
        width: Some(480),
        height: Some(480),
        duration_ms: Some(23_400),
        has_preview: true,
        round: true,
        ..Default::default()
    }
}

#[derive(Properties, PartialEq)]
struct HostProps {
    loader: MediaLoader,
    attachment: Attachment,
    #[prop_or_default]
    mine: bool,
    #[prop_or_default]
    on_call: bool,
    #[prop_or_default]
    opened: Callback<Attachment>,
}

/// The circle as a chat draws it: inside the media cache, with the chat's
/// now-playing owner beside it.
#[function_component(Host)]
fn host(props: &HostProps) -> Html {
    crate::now_playing::use_now_playing(props.on_call);
    html! {
        <ContextProvider<MediaLoader> context={props.loader.clone()}>
            <RoundVideoTile
                attachment={props.attachment.clone()}
                my_user_id={ME}
                mine={props.mine}
                on_open={props.opened.clone()}
            />
        </ContextProvider<MediaLoader>>
    }
}

fn mount() -> HtmlElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let root: HtmlElement = document.create_element("div").unwrap().dyn_into_html();
    // On screen: a circle that is not in view stops (S5.3).
    root.set_attribute(
        "style",
        "position:fixed;top:0;left:0;width:600px;height:560px;z-index:50;background:#fff",
    )
    .unwrap();
    document.body().unwrap().append_child(&root).unwrap();
    root
}

fn render(root: &HtmlElement, props: HostProps) -> yew::AppHandle<Host> {
    yew::Renderer::<Host>::with_root_and_props(root.clone().into(), props).render()
}

async fn until(ms: u32, check: impl Fn() -> bool) -> bool {
    for _ in 0..ms / 20 {
        if check() {
            return true;
        }
        TimeoutFuture::new(20).await;
    }
    check()
}

fn face(root: &Element) -> HtmlElement {
    query(root, ".round-face").dyn_into_html()
}

fn video(root: &Element) -> Option<HtmlVideoElement> {
    root.query_selector("video.round-player")
        .ok()
        .flatten()
        .and_then(|found| found.dyn_into().ok())
}

/// A click the way a mouse makes one — `detail` 1 for a click, 2 for the
/// second half of a double click.
fn click(element: &HtmlElement, detail: i32) {
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_detail(detail);
    let event = web_sys::MouseEvent::new_with_mouse_event_init_dict("click", &init).unwrap();
    element.dispatch_event(&event).unwrap();
}

fn forget_played() {
    crate::session::clear();
}

/// THE CIRCLE IS DRAWN FROM ITS POSTER ALONE (S5.2): the poster, the
/// duration capsule, the play disc and — on somebody else's, until this
/// device has played it — the dot, said to a screen reader as "Not
/// played". Nothing of the video is fetched to draw it. Mine has no dot.
#[wasm_bindgen_test]
async fn a_circle_draws_from_its_poster_with_its_time_and_its_dot() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9101, Variant::Preview, poster());
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache.clone(),
            attachment: round(9101),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    let circle = face(&root);
    assert_eq!(
        circle.get_attribute("aria-label").as_deref(),
        Some("Video message, 0:23")
    );
    assert_eq!(
        circle.get_attribute("aria-pressed").as_deref(),
        Some("false")
    );
    assert!(root.query_selector("img.round-poster").unwrap().is_some());
    assert_eq!(
        query(&root, ".round-duration").text_content().as_deref(),
        Some("0:23")
    );
    assert!(root.query_selector(".round-play").unwrap().is_some());
    assert!(
        root.query_selector(".round-dot").unwrap().is_some(),
        "the dot"
    );
    let described = circle.get_attribute("aria-describedby").expect("described");
    let said = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .get_element_by_id(&described)
        .expect("the value");
    assert_eq!(said.text_content().as_deref(), Some("Not played"));
    assert!(video(&root).is_none(), "no video to draw it");
    assert!(
        cache.get(9101, Variant::Original).is_none(),
        "and nothing of it fetched"
    );
    handle.destroy();

    // Mine: nothing new in one's own.
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9101),
            mine: true,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    assert!(root.query_selector(".round-dot").unwrap().is_none());
    assert!(face(&root).get_attribute("aria-describedby").is_none());
    handle.destroy();
    root.remove();
}

/// A TAP PLAYS IT IN PLACE, WITH SOUND (S5.3): the original fetched into an
/// inline `<video playsinline>` that is the app's playback, the ring running
/// round, the expand control there while it plays — and at the end it is
/// back to the poster with its dot gone, here and for this account on this
/// device.
#[wasm_bindgen_test]
async fn a_tap_plays_it_in_place_and_the_end_takes_the_dot() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9102, Variant::Preview, poster());
    cache.seed(9102, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::<i64>::new()));
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9102),
            mine: false,
            on_call: false,
            opened: {
                let opened = opened.clone();
                Callback::from(move |attachment: Attachment| {
                    opened.borrow_mut().push(attachment.id)
                })
            },
        },
    );
    TimeoutFuture::new(50).await;
    click(&face(&root), 1);
    assert!(until(2_000, || video(&root).is_some()).await, "fetched");
    let player = video(&root).unwrap();
    // The fixture is half a second long: slowed, so that it can be seen
    // playing.
    player.set_playback_rate(0.1);
    assert!(player.has_attribute("playsinline"));
    assert!(player.has_attribute(crate::now_playing::PLAYBACK));
    assert!(!player.muted(), "with sound");
    assert!(
        until(3_000, || face(&root)
            .get_attribute("aria-pressed")
            .as_deref()
            == Some("true"))
        .await,
        "it plays"
    );
    assert!(root
        .query_selector(".round-ring.is-progress")
        .unwrap()
        .is_some());
    assert!(
        root.query_selector(".round-poster").unwrap().is_none(),
        "the video, not the poster"
    );
    let expand: HtmlElement = query(&root, ".round-expand").dyn_into_html();
    assert_eq!(
        expand.get_attribute("aria-label").as_deref(),
        Some("Open Full Screen")
    );
    // To the end.
    player.set_playback_rate(4.0);
    assert!(
        until(6_000, || root
            .query_selector(".round-dot")
            .unwrap()
            .is_none())
        .await,
        "the end takes the dot"
    );
    assert_eq!(
        face(&root).get_attribute("aria-pressed").as_deref(),
        Some("false")
    );
    assert!(
        root.query_selector(".round-poster").unwrap().is_some(),
        "back to the poster"
    );
    assert!(
        crate::session::round_played(ME, 9102),
        "remembered by the device"
    );
    let described = face(&root)
        .get_attribute("aria-describedby")
        .and_then(|id| web_sys::window()?.document()?.get_element_by_id(&id))
        .and_then(|said| said.text_content());
    assert_eq!(described.as_deref(), Some("Played"), "said as played now");
    assert!(
        !crate::session::round_played(ME + 1, 9102),
        "for this account"
    );

    // Played again, then opened full screen: it pauses, and the viewer opens.
    player.set_playback_rate(0.1);
    click(&face(&root), 1);
    assert!(
        until(3_000, || face(&root)
            .get_attribute("aria-pressed")
            .as_deref()
            == Some("true"))
        .await
    );
    let expand: HtmlElement = query(&root, ".round-expand").dyn_into_html();
    expand.click();
    TimeoutFuture::new(30).await;
    assert!(player.paused(), "paused for the viewer");
    assert_eq!(*opened.borrow(), vec![9102]);
    handle.destroy();
    root.remove();
    forget_played();
}

/// A FETCH THAT FAILS LEAVES THE POSTER AND SAYS SO, and a tap tries again
/// (S5.3). Here the fetch cannot be made at all: nobody is signed in.
#[wasm_bindgen_test]
async fn a_circle_that_cannot_load_says_so_and_a_tap_tries_again() {
    forget_played();
    let cache = loader(None);
    cache.seed(9103, Variant::Preview, poster());
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9103),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    click(&face(&root), 1);
    assert!(
        until(2_000, || root
            .query_selector(".round-failed")
            .unwrap()
            .is_some())
        .await,
        "the failed line"
    );
    assert_eq!(
        query(&root, ".round-failed").text_content().as_deref(),
        Some("Couldn't load the video. Tap to try again.")
    );
    assert!(
        root.query_selector(".round-poster").unwrap().is_some(),
        "over the poster"
    );
    assert!(root.query_selector(".round-play").unwrap().is_some());
    // A tap tries again — and fails again, saying so again.
    click(&face(&root), 1);
    assert!(
        until(2_000, || root
            .query_selector(".round-failed")
            .unwrap()
            .is_some())
        .await
    );
    handle.destroy();
    root.remove();
}

/// A SECOND TAP WHILE IT LOADS GIVES IT UP (S5.3) — and what the fetch
/// brings afterwards, a failure here, lands nowhere: no failed line for a
/// fetch nobody waits for any more.
#[wasm_bindgen_test]
async fn a_second_tap_while_it_loads_gives_it_up() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9104, Variant::Preview, poster());
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9104),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    // A network that answers late: the fetch fails, but only after a while.
    run("window.__fcFetch = window.fetch; \
         window.fetch = () => new Promise((_, no) => setTimeout(() => no(new TypeError('offline')), 400));");
    click(&face(&root), 1);
    TimeoutFuture::new(20).await;
    assert!(
        root.query_selector(".round-frame.is-loading")
            .unwrap()
            .is_some(),
        "loading"
    );
    assert!(root
        .query_selector(".round-ring.is-loading")
        .unwrap()
        .is_some());
    click(&face(&root), 1);
    TimeoutFuture::new(20).await;
    assert!(root
        .query_selector(".round-frame.is-loading")
        .unwrap()
        .is_none());
    // The fetch comes back, failed, and is ignored.
    TimeoutFuture::new(800).await;
    run("window.fetch = window.__fcFetch; delete window.__fcFetch;");
    assert!(root.query_selector(".round-failed").unwrap().is_none());
    assert!(video(&root).is_none());
    assert!(root.query_selector(".round-play").unwrap().is_some());
    handle.destroy();
    root.remove();
}

/// A DOUBLE CLICK IS THE HEART, and leaves the circle where it found it:
/// the second click takes back what the first one started — playing, back
/// to its start and its poster (S5.3).
#[wasm_bindgen_test]
async fn a_double_click_leaves_the_circle_as_it_was() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9105, Variant::Preview, poster());
    cache.seed(9105, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9105),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    // Fetched once, so both halves of the double click find the bytes.
    click(&face(&root), 1);
    assert!(until(3_000, || video(&root).is_some_and(|video| !video.paused())).await);
    let player = video(&root).unwrap();
    player.set_playback_rate(0.1);
    click(&face(&root), 1);
    assert!(until(1_000, || player.paused()).await);
    player.set_current_time(0.0);
    TimeoutFuture::new(50).await;

    click(&face(&root), 1);
    assert!(
        until(2_000, || !player.paused()).await,
        "the first click plays"
    );
    TimeoutFuture::new(150).await;
    click(&face(&root), 2);
    assert!(
        until(1_000, || player.paused()).await,
        "the second takes it back"
    );
    TimeoutFuture::new(50).await;
    assert!(
        player.current_time() < 0.05,
        "at its start: {}",
        player.current_time()
    );
    assert!(
        root.query_selector(".round-poster").unwrap().is_some(),
        "and its poster"
    );
    assert!(
        root.query_selector(".round-dot").unwrap().is_some(),
        "still not played"
    );
    handle.destroy();
    root.remove();
}

/// The tab hidden, the way a browser hides it, until this is dropped.
struct HiddenTab;

impl HiddenTab {
    fn now() -> HiddenTab {
        run(
            "Object.defineProperty(document, 'hidden', { get: () => true, configurable: true }); \
             Object.defineProperty(document, 'visibilityState', \
               { get: () => 'hidden', configurable: true }); \
             document.dispatchEvent(new Event('visibilitychange'));",
        );
        HiddenTab
    }
}

impl Drop for HiddenTab {
    fn drop(&mut self) {
        run("delete document.hidden; delete document.visibilityState; \
             document.dispatchEvent(new Event('visibilitychange'));");
    }
}

/// An `<audio>` playing the fixture's sound — the app's playback when
/// `ours`, a call's far end when not.
async fn playing(root: &Element, ours: bool) -> HtmlMediaElement {
    let document = web_sys::window().unwrap().document().unwrap();
    let player: HtmlMediaElement = document
        .create_element("video")
        .unwrap()
        .dyn_into()
        .unwrap();
    if ours {
        player
            .set_attribute(crate::now_playing::PLAYBACK, "true")
            .unwrap();
    }
    player.set_attribute("playsinline", "").unwrap();
    player.set_loop(true);
    let url = web_sys::Url::create_object_url_with_blob(&blob(WITHIN, "video/mp4")).unwrap();
    player.set_src(&url);
    root.append_child(&player).unwrap();
    let _ = player.play();
    // Its `play` heard, and its time moving — not only asked to play, which
    // `paused` already says before the event is fired.
    assert!(
        until(3_000, || !player.paused() && player.current_time() > 0.0).await,
        "it plays"
    );
    player
}

/// ONE THING PLAYS AT A TIME, AND WHAT STOPS IT DOES (S4, S5.3): another of
/// the app's players starting pauses the first; a hidden tab, a call, an
/// output that went and the chat closing pause everything of the app's — an
/// output that came does not — and a call's own media is never touched.
#[wasm_bindgen_test]
async fn the_now_playing_owner_pauses_for_what_the_plan_says() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9106, Variant::Preview, poster());
    let root = mount();
    let props = |on_call: bool| HostProps {
        loader: cache.clone(),
        attachment: round(9106),
        mine: false,
        on_call,
        opened: Callback::noop(),
    };
    let outputs = Outputs::listing(Some(&["speakers", "headphones"]));
    let mut handle = render(&root, props(false));
    TimeoutFuture::new(50).await;
    let call = playing(&root, false).await;

    // One at a time.
    let first = playing(&root, true).await;
    let second = playing(&root, true).await;
    assert!(until(1_000, || first.paused()).await, "one thing at a time");
    assert!(!second.paused());
    assert!(!call.paused(), "never the call's");

    // A hidden tab.
    {
        let _hidden = HiddenTab::now();
        assert!(
            until(1_000, || second.paused()).await,
            "a hidden tab pauses"
        );
    }
    assert!(!call.paused());

    // An output that CAME plays on; one that WENT pauses.
    let _ = second.play();
    assert!(until(2_000, || !second.paused()).await);
    outputs.now(Some(&["speakers", "headphones", "bluetooth"]));
    run("navigator.mediaDevices.dispatchEvent(new Event('devicechange'));");
    TimeoutFuture::new(200).await;
    assert!(!second.paused(), "an output that came plays on");
    outputs.now(Some(&["speakers"]));
    run("navigator.mediaDevices.dispatchEvent(new Event('devicechange'));");
    assert!(
        until(1_000, || second.paused()).await,
        "headphones out pause"
    );
    // A browser that will not say: paused.
    let _ = second.play();
    assert!(until(2_000, || !second.paused()).await);
    outputs.now(None);
    run("navigator.mediaDevices.dispatchEvent(new Event('devicechange'));");
    assert!(until(1_000, || second.paused()).await, "unknown is paused");
    assert!(!call.paused());

    // A call.
    let _ = second.play();
    assert!(until(2_000, || !second.paused()).await);
    handle.update(props(true));
    assert!(until(1_000, || second.paused()).await, "a call pauses");
    handle.update(props(false));
    TimeoutFuture::new(50).await;

    // The chat closing.
    let _ = second.play();
    assert!(until(2_000, || !second.paused()).await);
    handle.destroy();
    assert!(until(1_000, || second.paused()).await, "the chat closed");
    assert!(!call.paused(), "and the call plays on");

    // Gone with the chat: nothing pauses any more.
    let _ = second.play();
    let _ = first.play();
    TimeoutFuture::new(300).await;
    assert!(!second.paused() && !first.paused(), "no owner, no rule");
    for player in [first, second, call] {
        let _ = player.pause();
        player.remove();
    }
    drop(outputs);
    handle = render(&root, props(false));
    handle.destroy();
    root.remove();
}

/// The shipped stylesheet in a frame `width` wide — a window that size —
/// with `body` in it; answers the frame's document.
async fn framed(width: u32, body: &str) -> (web_sys::HtmlIFrameElement, web_sys::Document) {
    let document = web_sys::window().unwrap().document().unwrap();
    let frame: web_sys::HtmlIFrameElement = document
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    frame
        .set_attribute(
            "style",
            &format!("position:fixed;top:0;left:0;width:{width}px;height:600px;border:0"),
        )
        .unwrap();
    let html = format!(
        "<!doctype html><html><head><style>{}</style></head><body>{body}</body></html>",
        include_str!("../styles.css")
    );
    frame.set_attribute("srcdoc", &html).unwrap();
    document.body().unwrap().append_child(&frame).unwrap();
    assert!(
        until(3_000, || frame
            .content_document()
            .and_then(|inner| inner.query_selector(".round-face").ok().flatten())
            .is_some())
        .await,
        "the frame drew"
    );
    TimeoutFuture::new(30).await;
    let inner = frame.content_document().expect("the frame's document");
    (frame, inner)
}

/// 200 ACROSS UNDER 720 PX, 240 FROM 720 PX, AND ROUND (S5.2): no balloon
/// round it, a circle clipping what is in it, the capsule and the play disc
/// inside, and never wider than the pane it is in.
#[wasm_bindgen_test]
async fn a_circle_is_200_under_720_and_240_from_720_and_has_no_balloon() {
    let markup = format!(
        r#"<div class="messages"><div class="row">
             <article class="bubble is-media-only is-round-video" id="theirs">
               <div class="round-video" style="{}">
                 <div class="round-frame"><button class="round-face">
                   <span class="round-play">▶</span>
                   <span class="round-duration">0:23<span class="round-dot"></span></span>
                 </button></div>
               </div>
             </article></div></div>"#,
        crate::views::round_tile::diameters()
    );
    for (width, diameter) in [(400, 200.0), (719, 200.0), (720, 240.0), (1024, 240.0)] {
        let (frame, inner) = framed(width, &markup).await;
        let window = inner.default_view().expect("the frame's window");
        let style = |element: &Element, property: &str| {
            window
                .get_computed_style(element)
                .unwrap()
                .unwrap()
                .get_property_value(property)
                .unwrap()
        };
        let pick = |selector: &str| inner.query_selector(selector).unwrap().expect(selector);
        let circle = pick(".round-face").get_bounding_client_rect();
        assert_eq!(
            (circle.width(), circle.height()),
            (diameter, diameter),
            "at {width} px"
        );
        assert_eq!(style(&pick(".round-face"), "border-top-left-radius"), "50%");
        assert_eq!(style(&pick(".round-face"), "overflow"), "hidden");
        let bubble = pick("#theirs");
        assert_eq!(
            style(&bubble, "background-color"),
            "rgba(0, 0, 0, 0)",
            "no balloon"
        );
        let capsule = pick(".round-duration").get_bounding_client_rect();
        assert!(capsule.bottom() <= circle.bottom() && capsule.top() >= circle.top());
        let play = pick(".round-play").get_bounding_client_rect();
        assert_eq!((play.width(), play.height()), (48.0, 48.0));
        assert!(
            ((play.left() + play.width() / 2.0) - (circle.left() + circle.width() / 2.0)).abs()
                < 1.0,
            "the play disc in the middle"
        );
        frame.remove();
    }
    // In a pane narrower than the circle, it shrinks rather than spill —
    // keeping 6 px on each side for the ring outside its edge.
    let (frame, inner) = framed(
        1024,
        &format!(
            r#"<div style="width:150px"><div class="round-video" style="{}">
                 <div class="round-frame"><button class="round-face"></button></div></div></div>"#,
            crate::views::round_tile::diameters()
        ),
    )
    .await;
    let circle = inner
        .query_selector(".round-face")
        .unwrap()
        .unwrap()
        .get_bounding_client_rect();
    assert_eq!((circle.width(), circle.height()), (138.0, 138.0));
    frame.remove();
}

/// IN A CHAT (S5.2, S5.7): a video message is drawn round among the rest,
/// a reply quoting it says "Video message" in its quote, and so does the
/// composer's banner when replying to it.
#[wasm_bindgen_test]
async fn a_chat_draws_the_circle_and_says_video_message_where_it_is_quoted() {
    use crate::layout_tests::{message, pane, props_with, recorder};
    let root = pane();
    let (_log, on_action) = recorder();
    let mut circle = message(1);
    circle.body = String::new();
    circle.attachments = Some(vec![round(9107)]);
    let mut reply = message(3);
    reply.body = "lovely".into();
    reply.reply_to = Some(crate::model::ReplyTo {
        message_id: 1,
        sender_id: circle.sender_id,
        excerpt: String::new(),
        parent: None,
    });
    let props = props_with(vec![circle, message(2), reply], None, on_action);
    let handle = yew::Renderer::<crate::views::conversation::Conversation>::with_root_and_props(
        root.clone().into(),
        props,
    )
    .render();
    TimeoutFuture::new(80).await;
    let bubble = query(&root, "#m-1");
    assert!(bubble.class_list().contains("is-round-video"));
    assert!(bubble.query_selector(".round-face").unwrap().is_some());
    assert_eq!(
        query(&root, "#m-3 .quote-text").text_content().as_deref(),
        Some("Video message")
    );
    // Replying to it.
    query(&bubble, ".more").dyn_into_html().click();
    TimeoutFuture::new(30).await;
    crate::layout_tests::click_labelled(&bubble, ".menu [role=menuitem]", "Reply");
    TimeoutFuture::new(50).await;
    let banner = query(&root, ".composer-banner .banner-text")
        .text_content()
        .unwrap_or_default();
    assert!(banner.ends_with(": Video message"), "{banner}");
    handle.destroy();
    root.remove();
}

#[derive(Properties, PartialEq)]
struct QuietHostProps {
    loader: MediaLoader,
    said: Callback<String>,
}

#[function_component(Recording)]
fn recording(props: &QuietHostProps) -> Html {
    crate::views::quiet::use_quiet_while(true);
    crate::views::quiet::use_quiet_reason(props.said.clone());
    html! {
        <ContextProvider<MediaLoader> context={props.loader.clone()}>
            <RoundVideoTile attachment={round(9108)} my_user_id={ME} mine={false} on_open={Callback::noop()} />
        </ContextProvider<MediaLoader>>
    }
}

#[function_component(QuietHost)]
fn quiet_host(props: &QuietHostProps) -> Html {
    html! {
        <crate::views::quiet::QuietRoot>
            <Recording loader={props.loader.clone()} said={props.said.clone()} />
        </crate::views::quiet::QuietRoot>
    }
}

/// NOTHING OF THE APP'S PLAYS INTO A NOTE (S1.7): while a voice message is
/// recorded the circle is dimmed — not disabled — says why when tapped, and
/// fetches and plays nothing.
#[wasm_bindgen_test]
async fn a_circle_is_dimmed_while_a_voice_message_is_recorded() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9108, Variant::Preview, poster());
    cache.seed(9108, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let said = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
    let handle = yew::Renderer::<QuietHost>::with_root_and_props(
        root.clone().into(),
        QuietHostProps {
            loader: cache,
            said: {
                let said = said.clone();
                Callback::from(move |text: String| said.borrow_mut().push(text))
            },
        },
    )
    .render();
    TimeoutFuture::new(80).await;
    let circle = face(&root);
    assert_eq!(
        circle.get_attribute("aria-disabled").as_deref(),
        Some("true")
    );
    assert!(!circle.has_attribute("disabled"), "dimmed, not disabled");
    click(&circle, 1);
    TimeoutFuture::new(200).await;
    assert_eq!(
        *said.borrow(),
        vec!["You can play this after recording.".to_string()]
    );
    assert!(video(&root).is_none(), "nothing fetched, nothing played");
    handle.destroy();
    root.remove();
}

#[derive(Properties, PartialEq)]
struct PlayersProps {
    loader: MediaLoader,
}

/// A voice note in a bubble and the viewer's video, as the app draws them.
#[function_component(Players)]
fn players(props: &PlayersProps) -> Html {
    let note = Attachment {
        id: 9110,
        kind: "audio".into(),
        mime: Some("audio/mp4".into()),
        duration_ms: Some(2_000),
        ..Default::default()
    };
    html! {
        <ContextProvider<MediaLoader> context={props.loader.clone()}>
            <crate::views::attachments::AttachmentStack
                attachments={vec![note]}
                mine={false}
                on_open={Callback::noop()}
            />
            <crate::views::viewer::Viewer
                items={vec![round(9111)]}
                index={0}
                on_step={Callback::noop()}
                on_close={Callback::noop()}
            />
        </ContextProvider<MediaLoader>>
    }
}

/// EVERY PLAYER OF THE APP'S IS THE APP'S PLAYBACK — a voice note in a
/// bubble, the viewer's video, a circle — so one thing plays at a time
/// among them, and what pauses playback pauses them (S5.3). A call's own
/// media is not marked, and the owner never touches it.
#[wasm_bindgen_test]
async fn every_player_of_the_apps_is_marked_as_its_playback_and_no_call_is() {
    let cache = loader(Some("t"));
    cache.seed(9111, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let handle = yew::Renderer::<Players>::with_root_and_props(
        root.clone().into(),
        PlayersProps { loader: cache },
    )
    .render();
    TimeoutFuture::new(80).await;
    let marked = |selector: &str| {
        root.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("{selector} is drawn"))
            .has_attribute(crate::now_playing::PLAYBACK)
    };
    assert!(marked(".audio audio"), "a voice note");
    assert!(marked(".viewer-video video"), "the viewer's video");
    handle.destroy();
    root.remove();

    // A staged voice message's own player, in the chat.
    let pane = crate::layout_tests::pane();
    let (_log, on_action) = crate::layout_tests::recorder();
    let mut props = crate::layout_tests::props_with(Vec::new(), None, on_action);
    props.staged = vec![crate::staged::Prepared {
        kind: "audio".into(),
        mime: "audio/wav".into(),
        size: 4,
        duration_ms: Some(2_000),
        file: Some(blob(b"RIFF", "audio/wav")),
        ..Default::default()
    }];
    let handle = yew::Renderer::<crate::views::conversation::Conversation>::with_root_and_props(
        pane.clone().into(),
        props,
    )
    .render();
    TimeoutFuture::new(80).await;
    assert!(
        query(&pane, ".staged.is-voice audio").has_attribute(crate::now_playing::PLAYBACK),
        "a staged voice message"
    );
    handle.destroy();
    pane.remove();

    let markup = include_str!("views/call.rs");
    assert!(
        !markup.contains("data-playback"),
        "a call's own media is never the app's playback"
    );
}

/// A CIRCLE STOPS WHEN ITS ROW SCROLLS OUT OF VIEW (S5.3).
#[wasm_bindgen_test]
async fn a_circle_scrolled_out_of_view_stops() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9112, Variant::Preview, poster());
    cache.seed(9112, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9112),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    click(&face(&root), 1);
    assert!(until(2_000, || video(&root).is_some()).await);
    let player = video(&root).unwrap();
    player.set_playback_rate(0.1);
    assert!(
        until(3_000, || face(&root)
            .get_attribute("aria-pressed")
            .as_deref()
            == Some("true"))
        .await
    );
    TimeoutFuture::new(200).await;
    assert!(!player.paused(), "in view, it plays on");
    // Scrolled away.
    root.style().set_property("top", "4000px").unwrap();
    assert!(
        until(2_000, || player.paused()).await,
        "out of view, it stops"
    );
    handle.destroy();
    root.remove();
}

/// SAFARI REFUSES TO START PLAYING OUTSIDE THE TAP (S5.3, S8.8): the
/// bytes land after it, the refused start leaves the circle on its play
/// glyph — not on a loading ring with nothing coming — and the next tap,
/// with the bytes here, plays them. The refusal is made the way Safari makes
/// it: `play()` answering with a rejected promise.
#[wasm_bindgen_test]
async fn a_refused_start_returns_to_the_play_glyph_and_the_next_tap_plays() {
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9113, Variant::Preview, poster());
    cache.seed(9113, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: round(9113),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    run("window.__fcPlay = HTMLMediaElement.prototype.play; \
         HTMLMediaElement.prototype.play = function () { \
           return Promise.reject(new DOMException('not allowed', 'NotAllowedError')); };");
    click(&face(&root), 1);
    assert!(
        until(2_000, || video(&root).is_some()).await,
        "the bytes are in"
    );
    TimeoutFuture::new(100).await;
    assert!(video(&root).unwrap().paused(), "refused");
    assert!(
        root.query_selector(".round-play:not(.is-gone)")
            .unwrap()
            .is_some(),
        "back to the play glyph"
    );
    assert!(root
        .query_selector(".round-frame.is-loading")
        .unwrap()
        .is_none());
    run("HTMLMediaElement.prototype.play = window.__fcPlay; delete window.__fcPlay;");
    let player = video(&root).unwrap();
    player.set_playback_rate(0.1);
    click(&face(&root), 1);
    assert!(
        until(2_000, || face(&root)
            .get_attribute("aria-pressed")
            .as_deref()
            == Some("true"))
        .await,
        "the next tap plays"
    );
    handle.destroy();
    root.remove();
}

/// The ring's radius in its own 100-unit box (round_tile.rs).
const RING_RADIUS_UNITS: f64 = 48.5;

/// How many of `selector` are inside `root`.
fn count(root: &Element, selector: &str) -> u32 {
    root.query_selector_all(selector).unwrap().length()
}

/// What a ring DRAWS, rather than what is in the page: the ring rasterised
/// as the page draws it — its own attributes, with the stroke's width, cap
/// and vector effect as the shipped stylesheet computes them — then walked
/// round at its radius. Answers how many separate arcs it shows and how
/// thick its stroke is across, in CSS pixels.
async fn drawn_ring(ring: &Element) -> (usize, f64) {
    use std::f64::consts::PI;
    let window = web_sys::window().unwrap();
    let size = ring.get_bounding_client_rect().width().round();
    assert!(size > 100.0, "the ring is laid out: {size}");
    let circle = crate::layout_tests::query(ring, "circle");
    let computed = window.get_computed_style(&circle).unwrap().unwrap();
    let property = |name: &str| computed.get_property_value(name).unwrap();
    let copy: Element = ring.clone_node_with_deep(true).unwrap().unchecked_into();
    copy.remove_attribute("class").unwrap();
    copy.set_attribute("xmlns", "http://www.w3.org/2000/svg")
        .unwrap();
    copy.set_attribute("width", &size.to_string()).unwrap();
    copy.set_attribute("height", &size.to_string()).unwrap();
    crate::layout_tests::query(&copy, "circle")
        .set_attribute(
            "style",
            &format!(
                "fill:none;stroke:#000;stroke-width:{};stroke-linecap:{};vector-effect:{}",
                property("stroke-width"),
                property("stroke-linecap"),
                property("vector-effect"),
            ),
        )
        .unwrap();
    let raster = js_sys::Function::new_with_args(
        "svg, size",
        "return new Promise((ok, no) => { \
           const url = URL.createObjectURL(new Blob([svg], {type: 'image/svg+xml'})); \
           const image = new Image(); \
           image.onload = () => { \
             const canvas = document.createElement('canvas'); \
             canvas.width = size; canvas.height = size; \
             const context = canvas.getContext('2d'); \
             context.drawImage(image, 0, 0, size, size); \
             URL.revokeObjectURL(url); \
             ok(context.getImageData(0, 0, size, size).data); \
           }; \
           image.onerror = () => no(new Error('the ring did not draw')); \
           image.src = url; \
         });",
    );
    let promise: js_sys::Promise = raster
        .call2(
            &JsValue::NULL,
            &JsValue::from_str(&copy.outer_html()),
            &JsValue::from_f64(size),
        )
        .unwrap()
        .unchecked_into();
    let pixels: js_sys::Uint8ClampedArray = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .expect("the ring rasterised")
        .unchecked_into();
    let pixels = pixels.to_vec();
    let side = size as i64;
    let alpha = |x: f64, y: f64| {
        let (x, y) = (x.round() as i64, y.round() as i64);
        if x < 0 || y < 0 || x >= side || y >= side {
            return 0;
        }
        pixels[((y * side + x) * 4 + 3) as usize]
    };
    // The circle's radius, in pixels: its `r` in the ring's own box.
    let units = ring
        .get_attribute("viewBox")
        .and_then(|view| view.split_whitespace().nth(2)?.parse::<f64>().ok())
        .unwrap_or(size);
    let radius: f64 = circle.get_attribute("r").unwrap().parse().unwrap();
    let radius = radius * size / units;
    let centre = size / 2.0;
    const STEPS: usize = 720;
    let on: Vec<bool> = (0..STEPS)
        .map(|step| {
            let angle = step as f64 * 2.0 * PI / STEPS as f64;
            alpha(centre + radius * angle.cos(), centre + radius * angle.sin()) > 100
        })
        .collect();
    let starts = (0..STEPS)
        .filter(|&step| on[step] && !on[(step + STEPS - 1) % STEPS])
        .count();
    let arcs = if starts == 0 && on.iter().all(|&lit| lit) {
        1
    } else {
        starts
    };
    // Across the stroke, wherever it is drawn.
    let mut widths: Vec<f64> = (0..STEPS)
        .step_by(4)
        .filter(|&step| on[step])
        .map(|step| {
            let angle = step as f64 * 2.0 * PI / STEPS as f64;
            (-40..=40)
                .map(|tenth| radius + f64::from(tenth) * 0.2)
                .filter(|&at| alpha(centre + at * angle.cos(), centre + at * angle.sin()) > 127)
                .count() as f64
                * 0.2
        })
        .collect();
    widths.sort_by(f64::total_cmp);
    let width = widths.get(widths.len() / 2).copied().unwrap_or(0.0);
    (arcs, width)
}

/// ONE RING, DRAWN AS ONE ARC, 3 ACROSS (S5.3). While it loads, a single
/// loading ring over the poster; while it plays, a single accent ring
/// running round the edge, and no play disc. And each ring DRAWS as one
/// arc 3 pixels wide: the stylesheet once drew the ring with
/// `vector-effect: non-scaling-stroke`, under which Chrome lays a dash
/// pattern out in screen pixels and ignores `pathLength`, so one arc of
/// "25 of 100" became a row of dashes round the circle — the "multiple
/// indicators" seen in a browser.
#[wasm_bindgen_test]
async fn one_ring_at_a_time_and_each_draws_one_arc_three_across() {
    crate::layout_tests::install_stylesheet();
    forget_played();
    let cache = loader(Some("t"));
    cache.seed(9120, Variant::Preview, poster());
    cache.seed(9121, Variant::Preview, poster());
    cache.seed(9121, Variant::Original, blob(WITHIN, "video/mp4"));
    let root = mount();

    // Not touched: the play disc, and no ring.
    let handle = render(
        &root,
        HostProps {
            loader: cache.clone(),
            attachment: round(9120),
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    assert_eq!(count(&root, ".round-ring"), 0, "no ring before a tap");
    assert_eq!(count(&root, ".round-play:not(.is-gone)"), 1);

    // Loading — over a network that never answers.
    run("window.__fcFetch = window.fetch; window.fetch = () => new Promise(() => {});");
    click(&face(&root), 1);
    TimeoutFuture::new(30).await;
    assert_eq!(count(&root, ".round-ring"), 1, "one ring while it loads");
    assert_eq!(count(&root, ".round-ring.is-loading"), 1);
    assert_eq!(count(&root, ".round-frame circle"), 1);
    assert_eq!(
        count(&root, ".round-play:not(.is-gone)"),
        0,
        "no play disc under it"
    );
    assert_eq!(count(&root, "img.round-poster"), 1, "over the poster");
    let (arcs, width) = drawn_ring(&query(&root, ".round-ring")).await;
    click(&face(&root), 1);
    run("window.fetch = window.__fcFetch; delete window.__fcFetch;");
    assert_eq!(arcs, 1, "the loading ring draws as one arc");
    assert!((width - 3.0).abs() <= 1.0, "3 across, not {width}");
    handle.destroy();

    // Playing: a second long, so that the ring is well round while it plays.
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: Attachment {
                duration_ms: Some(1_000),
                ..round(9121)
            },
            mine: false,
            on_call: false,
            opened: Callback::noop(),
        },
    );
    TimeoutFuture::new(50).await;
    click(&face(&root), 1);
    assert!(until(2_000, || video(&root).is_some()).await, "fetched");
    video(&root).unwrap().set_playback_rate(0.1);
    let dash = |root: &Element| {
        root.query_selector(".round-ring.is-progress circle")
            .unwrap()
            .and_then(|circle| circle.get_attribute("stroke-dasharray"))
            .and_then(|dash| dash.split_whitespace().next()?.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    assert!(
        until(4_000, || dash(&root) >= 10.0).await,
        "the ring runs round while it plays"
    );
    video(&root).unwrap().set_playback_rate(0.0625);
    assert_eq!(
        face(&root).get_attribute("aria-pressed").as_deref(),
        Some("true")
    );
    assert_eq!(count(&root, ".round-ring"), 1, "one ring while it plays");
    assert_eq!(count(&root, ".round-ring.is-progress"), 1);
    assert_eq!(count(&root, ".round-frame circle"), 1);
    assert_eq!(
        count(&root, ".round-play:not(.is-gone)"),
        0,
        "the play disc goes"
    );
    // It FADES out (none of it under Reduce Motion), and the ring lies
    // OUTSIDE the circle — never over a face.
    let window = web_sys::window().unwrap();
    let disc = query(&root, ".round-play");
    let computed = window.get_computed_style(&disc).unwrap().unwrap();
    assert!(
        computed
            .get_property_value("transition-property")
            .unwrap()
            .contains("opacity"),
        "a fade"
    );
    assert_eq!(
        computed.get_property_value("opacity").unwrap(),
        "0",
        "gone while it plays"
    );
    let ring_box = query(&root, ".round-ring").get_bounding_client_rect();
    let face_box = face(&root).get_bounding_client_rect();
    assert!(
        (ring_box.width() - face_box.width() - 12.0).abs() < 0.5
            && (ring_box.left() - face_box.left() + 6.0).abs() < 0.5,
        "6 px past the edge all round: {} over {}",
        ring_box.width(),
        face_box.width()
    );
    let radius_px = RING_RADIUS_UNITS * ring_box.width() / 100.0;
    assert!(
        radius_px - 1.5 >= face_box.width() / 2.0,
        "the stroke's inner edge clears the circle: {radius_px}"
    );
    let (arcs, width) = drawn_ring(&query(&root, ".round-ring")).await;
    assert_eq!(arcs, 1, "the progress ring draws as one arc");
    assert!((width - 3.0).abs() <= 1.0, "3 across, not {width}");
    handle.destroy();
    root.remove();
    forget_played();
}

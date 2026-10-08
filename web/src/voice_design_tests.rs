//! The voice message as the approved design for #79 draws it, in a real
//! browser: the voice bubble — its round accent play button, the sender's
//! waveform (or the neutral placeholder), played bars as it plays, seeking by
//! a tap, a drag or a key, the speed chip and the unplayed dot — and the
//! design's sizes and colours, measured against the shipped stylesheet in
//! light and in dark.

use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlAudioElement, HtmlElement};
use yew::prelude::*;

use crate::layout_tests::query;
use crate::media::{MediaLoader, Variant};
use crate::model::Attachment;
use crate::views::attachments::{
    seek_by_key, seek_by_pointer, speed_label, AttachmentStack, VOICE_BARS,
};

const ME: i64 = 9401;

/// Wait, up to `ms`, for `check`.
async fn until(ms: u32, check: impl Fn() -> bool) -> bool {
    for _ in 0..ms / 20 {
        if check() {
            return true;
        }
        TimeoutFuture::new(20).await;
    }
    check()
}

/// `seconds` of a tone, as the WAV a voice note may be.
fn tone(seconds: u32) -> web_sys::Blob {
    let rate = fc_text::wav::VOICE_RATE;
    let samples: Vec<f32> = (0..seconds * rate)
        .map(|at| (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.3)
        .collect();
    let bytes = fc_text::wav::encode(&samples, rate);
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("audio/wav");
    web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap()
}

fn voice(id: i64, seconds: u32, waveform: Option<&str>) -> Attachment {
    Attachment {
        id,
        kind: "audio".into(),
        mime: Some("audio/wav".into()),
        duration_ms: Some(i64::from(seconds) * 1_000),
        waveform: waveform.map(str::to_string),
        ..Attachment::default()
    }
}

fn loader() -> MediaLoader {
    MediaLoader::new(crate::live::Live::new(
        crate::live::AppState {
            token: Some("t".into()),
            ..Default::default()
        },
        Rc::new(|| {}),
    ))
}

#[derive(Properties, PartialEq)]
struct HostProps {
    loader: MediaLoader,
    attachment: Attachment,
    mine: bool,
}

#[function_component(Host)]
fn host(props: &HostProps) -> Html {
    html! {
        <ContextProvider<MediaLoader> context={props.loader.clone()}>
            <AttachmentStack attachments={vec![props.attachment.clone()]} mine={props.mine}
                             on_open={Callback::noop()} my_user_id={ME} />
        </ContextProvider<MediaLoader>>
    }
}

fn mount() -> Element {
    let document = web_sys::window().unwrap().document().unwrap();
    let root = document.create_element("div").unwrap();
    // Laid out, at a phone's width, so a pointer has somewhere to land.
    root.set_attribute("style", "position:fixed;top:0;left:0;width:360px")
        .unwrap();
    document.body().unwrap().append_child(&root).unwrap();
    root
}

fn render(root: &Element, props: HostProps) -> yew::AppHandle<Host> {
    yew::Renderer::<Host>::with_root_and_props(root.clone(), props).render()
}

fn bars(root: &Element) -> Vec<Element> {
    let found = root.query_selector_all(".audio-wave > i").unwrap();
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .collect()
}

fn played(root: &Element) -> usize {
    bars(root)
        .iter()
        .filter(|bar| bar.class_list().contains("is-played"))
        .count()
}

/// What a control's `aria-describedby` says.
fn described(element: &Element) -> Option<String> {
    let id = element.get_attribute("aria-describedby")?;
    web_sys::window()?
        .document()?
        .get_element_by_id(&id)?
        .text_content()
}

fn player(root: &Element) -> HtmlAudioElement {
    query(root, "audio").dyn_into().unwrap()
}

fn toggle(root: &Element) -> HtmlElement {
    query(root, ".audio-toggle").dyn_into().unwrap()
}

fn key(at: &Element, key: &str) {
    let init = web_sys::KeyboardEventInit::new();
    init.set_key(key);
    init.set_bubbles(true);
    init.set_cancelable(true);
    at.dispatch_event(
        &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
    )
    .unwrap();
}

fn pointer(at: &Element, kind: &str, x: f64) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(1);
    init.set_button(0);
    init.set_client_x(x as i32);
    init.set_client_y(10);
    at.dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

/// A finger (`pointerType` "touch") at `x`, `y` on the waveform.
fn finger(at: &Element, kind: &str, x: f64, y: f64) {
    let init = web_sys::PointerEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_pointer_id(7);
    init.set_pointer_type("touch");
    init.set_is_primary(true);
    init.set_button(if kind == "pointermove" { -1 } else { 0 });
    init.set_client_x(x as i32);
    init.set_client_y(y as i32);
    at.dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(kind, &init).unwrap())
        .unwrap();
}

/// THE SENDER'S WAVEFORM, before anything is downloaded (docs/protocol.md,
/// "A voice note's waveform"): 48 bars, each `(2 + level) / 17` of the
/// height, from the attachment's own field — and with none, or one that
/// cannot be read, the neutral placeholder: every bar at level 4. The
/// length at rest, the group named "Voice message, 0:12", and somebody
/// else's not yet played carries the dot, said as "Not played"; one's own
/// never does.
#[wasm_bindgen_test]
async fn a_voice_message_draws_the_senders_waveform_or_the_placeholder() {
    crate::session::clear();
    let shape = "0123456789abcdef".repeat(3);
    let levels = fc_text::waveform::parse(&shape).unwrap();
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: loader(),
            attachment: voice(9501, 12, Some(&shape)),
            mine: false,
        },
    );
    TimeoutFuture::new(50).await;
    let drawn = bars(&root);
    assert_eq!(drawn.len(), VOICE_BARS);
    for (bar, level) in drawn.iter().zip(levels) {
        assert_eq!(
            bar.get_attribute("style"),
            Some(format!(
                "height:{:.1}%",
                fc_text::waveform::bar_fraction(level) * 100.0
            ))
        );
    }
    assert_eq!(played(&root), 0, "nothing played at rest");
    let group = query(&root, ".audio");
    assert_eq!(
        group.get_attribute("aria-label").as_deref(),
        Some("Voice message, 0:12")
    );
    assert_eq!(
        query(&root, ".audio-time").text_content().as_deref(),
        Some("0:12"),
        "the length at rest"
    );
    assert!(
        root.query_selector(".audio-dot").unwrap().is_some(),
        "the dot"
    );
    assert_eq!(described(&toggle(&root)).as_deref(), Some("Not played"));
    assert!(
        root.query_selector(".audio-speed").unwrap().is_none(),
        "no speed chip at rest"
    );
    handle.destroy();

    for (waveform, mine) in [(None, true), (Some("FFFF"), false)] {
        let handle = render(
            &root,
            HostProps {
                loader: loader(),
                attachment: voice(9502, 3, waveform),
                mine,
            },
        );
        TimeoutFuture::new(50).await;
        let placeholder = format!(
            "height:{:.1}%",
            fc_text::waveform::bar_fraction(fc_text::waveform::PLACEHOLDER_LEVEL) * 100.0
        );
        let drawn = bars(&root);
        assert_eq!(drawn.len(), VOICE_BARS);
        assert!(
            drawn
                .iter()
                .all(|bar| bar.get_attribute("style").as_deref() == Some(placeholder.as_str())),
            "{waveform:?}: the placeholder"
        );
        if mine {
            assert!(root.query_selector(".audio-dot").unwrap().is_none());
            assert!(
                toggle(&root).get_attribute("aria-describedby").is_none(),
                "nothing new in one's own"
            );
        }
        handle.destroy();
    }
    root.remove();
}

/// PLAYING (the approved design): the played bars take the accent colour as
/// it plays, the time counts what has gone, the speed chip shows 1× and
/// goes 1× → 1.5× (and stays while it is paused part way), applied to the player's own rate and remembered by the
/// device; the dot goes the moment it starts — "Played" from then on, for
/// this account on this device — and at the end it is back at rest.
#[wasm_bindgen_test]
async fn a_voice_message_plays_its_bars_its_speed_and_loses_its_dot() {
    crate::session::clear();
    crate::session::set_voice_speed(1.0);
    let cache = loader();
    cache.seed(9503, Variant::Original, tone(2));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: voice(9503, 2, Some(&"8".repeat(48))),
            mine: false,
        },
    );
    TimeoutFuture::new(50).await;
    toggle(&root).click();
    assert!(until(2_000, || !player(&root).paused()).await, "it plays");
    player(&root).set_playback_rate(0.25);
    assert!(
        until(1_000, || root
            .query_selector(".audio-dot")
            .unwrap()
            .is_none())
        .await,
        "the dot goes as it starts"
    );
    assert!(crate::session::voice_played(ME, 9503), "remembered");
    assert!(
        !crate::session::voice_played(ME + 1, 9503),
        "for this account"
    );
    assert_eq!(described(&toggle(&root)).as_deref(), Some("Played"));
    assert_eq!(
        toggle(&root).get_attribute("aria-label").as_deref(),
        Some("Pause")
    );
    let chip = query(&root, ".audio-speed");
    assert_eq!(chip.text_content().as_deref(), Some("1×"));
    assert_eq!(
        chip.get_attribute("aria-label").as_deref(),
        Some("Playback speed, 1×")
    );
    chip.dyn_into::<HtmlElement>().unwrap().click();
    TimeoutFuture::new(30).await;
    assert_eq!(
        query(&root, ".audio-speed").text_content().as_deref(),
        Some("1.5×")
    );
    assert_eq!(player(&root).playback_rate(), 1.5, "the player's own rate");
    assert_eq!(crate::session::voice_speed(), 1.5, "the device's");
    assert!(
        until(2_000, || played(&root) > 0).await,
        "played bars as it plays"
    );
    // Paused part way it is still underway: the chip stays — as on iOS,
    // Android and Windows, and as the design draws it — and so does the
    // time gone.
    let _ = player(&root).pause();
    assert!(
        until(1_000, || toggle(&root)
            .get_attribute("aria-label")
            .as_deref()
            == Some("Play"))
        .await,
        "paused"
    );
    TimeoutFuture::new(30).await;
    assert!(
        root.query_selector(".audio-speed").unwrap().is_some(),
        "the chip stays while paused part way"
    );
    assert!(played(&root) > 0, "its played bars too");
    toggle(&root).click();
    assert!(until(2_000, || !player(&root).paused()).await, "it resumes");
    // To the end: back at rest.
    player(&root).set_playback_rate(4.0);
    assert!(until(3_000, || player(&root).paused()).await, "it ends");
    TimeoutFuture::new(50).await;
    assert_eq!(played(&root), 0, "at rest again");
    assert_eq!(
        query(&root, ".audio-time").text_content().as_deref(),
        Some("0:02")
    );
    assert!(root.query_selector(".audio-speed").unwrap().is_none());
    // The next starts at the device's speed.
    toggle(&root).click();
    assert!(until(2_000, || !player(&root).paused()).await);
    assert_eq!(player(&root).playback_rate(), 1.5);
    let _ = player(&root).pause();
    handle.destroy();
    root.remove();
    crate::session::set_voice_speed(1.0);
    crate::session::clear();
}

/// SEEKING (the approved design): the waveform is a slider a screen reader
/// can adjust — its value the position, in seconds and as m:ss — moved by
/// the arrows, Page Up and Down, Home and End, and by a tap or a drag on
/// the bars; a position chosen before the bytes are here is where the first
/// play starts.
#[wasm_bindgen_test]
async fn the_waveform_seeks_by_key_and_by_pointer_and_play_starts_there() {
    crate::session::clear();
    let cache = loader();
    cache.seed(9504, Variant::Original, tone(12));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: voice(9504, 12, None),
            mine: true,
        },
    );
    TimeoutFuture::new(50).await;
    let wave = query(&root, ".audio-wave");
    assert_eq!(wave.get_attribute("role").as_deref(), Some("slider"));
    assert_eq!(wave.get_attribute("tabindex").as_deref(), Some("0"));
    assert_eq!(wave.get_attribute("aria-valuemax").as_deref(), Some("12"));
    assert_eq!(wave.get_attribute("aria-valuenow").as_deref(), Some("0"));
    let now = |root: &Element| {
        query(root, ".audio-wave")
            .get_attribute("aria-valuenow")
            .unwrap_or_default()
    };
    key(&wave, "ArrowRight");
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "5");
    assert_eq!(
        query(&root, ".audio-wave")
            .get_attribute("aria-valuetext")
            .as_deref(),
        Some("0:05")
    );
    key(&query(&root, ".audio-wave"), "End");
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "12");
    key(&query(&root, ".audio-wave"), "Home");
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "0");
    // A tap halfway along.
    let rect = wave.get_bounding_client_rect();
    assert!(rect.width() > 100.0, "laid out: {}", rect.width());
    let wave = query(&root, ".audio-wave");
    pointer(&wave, "pointerdown", rect.left() + rect.width() / 2.0);
    pointer(&wave, "pointerup", rect.left() + rect.width() / 2.0);
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "6");
    assert_eq!(
        query(&root, ".audio-time").text_content().as_deref(),
        Some("0:06")
    );
    assert_eq!(played(&root), VOICE_BARS / 2, "half the bars played");
    // A drag to three quarters.
    pointer(&wave, "pointerdown", rect.left() + rect.width() / 2.0);
    pointer(&wave, "pointermove", rect.left() + rect.width() * 0.75);
    pointer(&wave, "pointerup", rect.left() + rect.width() * 0.75);
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "9");
    // The first play starts where it was put.
    toggle(&root).click();
    assert!(until(2_000, || !player(&root).paused()).await, "it plays");
    assert!(
        player(&root).current_time() >= 8.9,
        "from 0:09: {}",
        player(&root).current_time()
    );
    // And a seek while it plays moves the player itself.
    key(&query(&root, ".audio-wave"), "Home");
    TimeoutFuture::new(30).await;
    assert!(player(&root).current_time() < 1.0);
    let _ = player(&root).pause();
    handle.destroy();
    root.remove();
}

/// A FINGER THAT SCROLLS THE CHAT does not seek: the waveform lets the
/// browser pan vertically, so a finger landing on it may be starting a
/// scroll — it seeks only once it moves ALONG the waveform, or when it is
/// lifted where it landed (a tap); and when the browser takes the touch for
/// a scroll (`pointercancel`), the position is what it was before.
#[wasm_bindgen_test]
async fn a_finger_scrolling_past_the_waveform_leaves_the_position_alone() {
    crate::session::clear();
    let cache = loader();
    cache.seed(9507, Variant::Original, tone(12));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache,
            attachment: voice(9507, 12, None),
            mine: true,
        },
    );
    TimeoutFuture::new(50).await;
    let now = |root: &Element| {
        query(root, ".audio-wave")
            .get_attribute("aria-valuenow")
            .unwrap_or_default()
    };
    let wave = query(&root, ".audio-wave");
    let rect = wave.get_bounding_client_rect();
    let (half, y) = (rect.left() + rect.width() / 2.0, rect.top() + 5.0);
    // A scroll: down on the bubble, a little way up, and the browser takes
    // it — at rest it stays at rest: no bar played, its whole length shown.
    finger(&wave, "pointerdown", half, y);
    finger(&wave, "pointermove", half + 2.0, y - 6.0);
    TimeoutFuture::new(30).await;
    assert_eq!(
        now(&root),
        "0",
        "not even for a moment: a playing note would jump"
    );
    finger(
        &query(&root, ".audio-wave"),
        "pointercancel",
        half + 2.0,
        y - 6.0,
    );
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "0", "not moved by a scroll");
    assert_eq!(played(&root), 0, "no bar played");
    assert_eq!(
        query(&root, ".audio-time").text_content().as_deref(),
        Some("0:12"),
        "its length, at rest"
    );
    // A tap: lifted where it landed — that seeks.
    finger(&wave, "pointerdown", half, y);
    finger(&wave, "pointerup", half, y);
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "6", "a tap seeks");
    // A drag along it seeks as it goes…
    let wave = query(&root, ".audio-wave");
    finger(&wave, "pointerdown", half, y);
    finger(
        &wave,
        "pointermove",
        rect.left() + rect.width() * 0.75,
        y + 1.0,
    );
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "9", "a drag along it seeks");
    // …and one the browser takes back after all puts it back where it was.
    finger(
        &query(&root, ".audio-wave"),
        "pointercancel",
        rect.left() + rect.width() * 0.75,
        y + 1.0,
    );
    TimeoutFuture::new(30).await;
    assert_eq!(now(&root), "6", "back where it was");
    handle.destroy();
    root.remove();
}

/// A SEEK WHILE THE SOUND IS STILL ON ITS WAY is where it starts: Play is
/// pressed, the bytes are not here yet, the position is moved — and when
/// they land it plays from the new position, not from where Play was
/// pressed.
#[wasm_bindgen_test]
async fn a_seek_while_it_loads_is_where_it_starts() {
    crate::session::clear();
    // No token: nothing is fetched, the bytes come when the test puts them
    // in the cache, and the player looks again (`media::RETRY_MS`).
    let cache = MediaLoader::new(crate::live::Live::new(
        crate::live::AppState::default(),
        Rc::new(|| {}),
    ));
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: cache.clone(),
            attachment: voice(9508, 12, None),
            mine: true,
        },
    );
    TimeoutFuture::new(50).await;
    toggle(&root).click();
    TimeoutFuture::new(30).await;
    assert_eq!(
        toggle(&root).get_attribute("aria-label").as_deref(),
        Some("Loading")
    );
    key(&query(&root, ".audio-wave"), "End");
    TimeoutFuture::new(30).await;
    key(&query(&root, ".audio-wave"), "ArrowLeft");
    TimeoutFuture::new(30).await;
    assert_eq!(
        query(&root, ".audio-wave")
            .get_attribute("aria-valuenow")
            .as_deref(),
        Some("7")
    );
    cache.seed(9508, Variant::Original, tone(12));
    assert!(
        until(5_000, || !player(&root).paused()).await,
        "it plays once the bytes land"
    );
    assert!(
        player(&root).current_time() >= 6.9,
        "from 0:07, not from 0:00: {}",
        player(&root).current_time()
    );
    let _ = player(&root).pause();
    handle.destroy();
    root.remove();
}

/// The rules under the slider and the chip.
#[wasm_bindgen_test]
fn seeking_and_the_speed_chip_by_their_rules() {
    assert_eq!(seek_by_key("ArrowRight", 3.0, 12.0), Some(8.0));
    assert_eq!(
        seek_by_key("ArrowUp", 10.0, 12.0),
        Some(12.0),
        "never past the end"
    );
    assert_eq!(
        seek_by_key("ArrowLeft", 3.0, 12.0),
        Some(0.0),
        "nor before the start"
    );
    assert_eq!(seek_by_key("PageUp", 0.0, 40.0), Some(4.0));
    assert_eq!(seek_by_key("PageDown", 10.0, 40.0), Some(6.0));
    assert_eq!(seek_by_key("Home", 7.0, 12.0), Some(0.0));
    assert_eq!(seek_by_key("End", 7.0, 12.0), Some(12.0));
    assert_eq!(seek_by_key("Enter", 7.0, 12.0), None);
    assert_eq!(seek_by_pointer(150.0, 100.0, 200.0, 40.0), 10.0);
    assert_eq!(seek_by_pointer(50.0, 100.0, 200.0, 40.0), 0.0);
    assert_eq!(seek_by_pointer(400.0, 100.0, 200.0, 40.0), 40.0);
    assert_eq!(seek_by_pointer(150.0, 100.0, 0.0, 40.0), 0.0);
    assert_eq!(speed_label(1.0), "1×");
    assert_eq!(speed_label(1.5), "1.5×");
    assert_eq!(speed_label(2.0), "2×");
}

/// The shipped stylesheet in a frame `width` wide — dark where `dark` says,
/// the frame's own `prefers-color-scheme` following the colour scheme its
/// element is given — with `body` in it.
async fn framed(
    width: u32,
    dark: bool,
    body: &str,
) -> (web_sys::HtmlIFrameElement, web_sys::Document) {
    let document = web_sys::window().unwrap().document().unwrap();
    let frame: web_sys::HtmlIFrameElement = document
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    frame
        .set_attribute(
            "style",
            &format!(
                "position:fixed;top:0;left:0;width:{width}px;height:700px;border:0;\
                 color-scheme:{}",
                if dark { "dark" } else { "light" }
            ),
        )
        .unwrap();
    let html = format!(
        "<!doctype html><html><head><style>{}</style></head><body>{body}\
         <i id=\"drawn\"></i></body></html>",
        include_str!("../styles.css")
    );
    frame.set_attribute("srcdoc", &html).unwrap();
    document.body().unwrap().append_child(&frame).unwrap();
    assert!(
        until(3_000, || frame
            .content_document()
            .and_then(|inner| inner.get_element_by_id("drawn"))
            .is_some())
        .await,
        "the frame drew"
    );
    TimeoutFuture::new(30).await;
    let inner = frame.content_document().expect("the frame's document");
    (frame, inner)
}

fn style(document: &web_sys::Document, element: &Element, property: &str) -> String {
    document
        .default_view()
        .unwrap()
        .get_computed_style(element)
        .unwrap()
        .unwrap()
        .get_property_value(property)
        .unwrap()
}

/// THE DESIGN'S SIZES AND COLOURS, in light and in dark (the approved
/// design for #79), measured against the shipped stylesheet: the voice
/// bubble's 40-unit round accent play button, its waveform's bars — the
/// played one in the accent, the rest in the muted bar colour, on a panel
/// and on one's own tinted balloon — the time in tabular digits, the dot
/// and the speed chip; the recording row's red dot and round icon buttons;
/// the review and not-sent chips' round ▶.
#[wasm_bindgen_test]
async fn the_voice_bubble_and_the_recording_row_follow_the_design_in_light_and_dark() {
    // The real bubble's markup, as it draws.
    crate::session::clear();
    let root = mount();
    let handle = render(
        &root,
        HostProps {
            loader: loader(),
            attachment: voice(9505, 42, Some(&"f".repeat(48))),
            mine: false,
        },
    );
    TimeoutFuture::new(50).await;
    // As it is while it plays: its first bar played, and the speed chip.
    query(&root, ".audio-wave > i")
        .set_attribute("class", "is-played")
        .unwrap();
    let chip = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("button")
        .unwrap();
    chip.set_attribute("class", "audio-speed").unwrap();
    chip.set_text_content(Some("1×"));
    query(&root, ".audio-meta").append_child(&chip).unwrap();
    let bubble = query(&root, ".attachments").outer_html();
    handle.destroy();
    let handle = render(
        &root,
        HostProps {
            loader: loader(),
            attachment: voice(9506, 17, None),
            mine: true,
        },
    );
    TimeoutFuture::new(50).await;
    let mine = query(&root, ".attachments").outer_html();
    handle.destroy();
    root.remove();
    let markup = format!(
        r#"<div class="messages">
             <div class="row"><article class="bubble" id="theirs">{bubble}</article></div>
             <div class="row"><article class="bubble is-mine" id="mine">{mine}</article></div>
           </div>
           <div class="composer"><div class="recording">
             <button type="button" class="secondary recording-delete"><svg class="row-icon"></svg></button>
             <span class="recording-dot"></span>
             <span class="recording-time">0:42</span>
             <span class="recording-meter"><i style="height:50%"></i></span>
           </div></div>
           <div class="staged is-voice"><button class="local-play"></button>
             <span class="chip-wave"><i class="is-played" style="height:50%"></i><i style="height:50%"></i></span>
             <span class="chip-time">0:42</span></div>"#
    );
    // (scheme, accent, unplayed bar, one's own unplayed bar, recording red)
    for (dark, tint, off, mine_off, red) in [
        (
            false,
            "rgb(47, 111, 208)",
            "rgb(185, 192, 208)",
            "rgb(143, 177, 227)",
            "rgb(229, 72, 77)",
        ),
        (
            true,
            "rgb(111, 165, 245)",
            "rgb(74, 79, 96)",
            "rgb(79, 111, 156)",
            "rgb(255, 99, 105)",
        ),
    ] {
        let (frame, inner) = framed(390, dark, &markup).await;
        assert_eq!(
            frame
                .content_window()
                .unwrap()
                .match_media("(prefers-color-scheme: dark)")
                .unwrap()
                .unwrap()
                .matches(),
            dark,
            "the frame is in the scheme asked for"
        );
        let pick = |selector: &str| inner.query_selector(selector).unwrap().expect(selector);
        let play = pick("#theirs .audio-toggle");
        let rect = play.get_bounding_client_rect();
        assert_eq!((rect.width(), rect.height()), (40.0, 40.0), "dark {dark}");
        assert_eq!(style(&inner, &play, "border-top-left-radius"), "50%");
        assert_eq!(style(&inner, &play, "background-color"), tint);
        let wave = pick("#theirs .audio-wave").get_bounding_client_rect();
        assert_eq!(wave.height(), 28.0);
        assert!(wave.left() > rect.right(), "the waveform beside the button");
        let played = pick("#theirs .audio-wave > i.is-played");
        assert_eq!(style(&inner, &played, "background-color"), tint);
        assert_eq!(
            played.get_bounding_client_rect().height(),
            28.0,
            "a full-scale bar is the whole height"
        );
        let resting = pick("#theirs .audio-wave > i:not(.is-played)");
        assert_eq!(style(&inner, &resting, "background-color"), off);
        let own = pick("#mine .audio-wave > i");
        assert_eq!(style(&inner, &own, "background-color"), mine_off);
        let placeholder = own.get_bounding_client_rect().height();
        assert!(
            (placeholder - 28.0 * 6.0 / 17.0).abs() < 0.6,
            "the placeholder stands at level 4: {placeholder}"
        );
        let time = pick("#theirs .audio-time");
        assert!(style(&inner, &time, "font-variant-numeric").contains("tabular-nums"));
        assert!(
            time.get_bounding_client_rect().top() >= wave.bottom(),
            "the time under it"
        );
        let dot = pick("#theirs .audio-dot");
        let dot_rect = dot.get_bounding_client_rect();
        assert_eq!((dot_rect.width(), dot_rect.height()), (7.0, 7.0));
        assert_eq!(style(&inner, &dot, "background-color"), tint);
        let speed = pick("#theirs .audio-speed");
        assert_eq!(style(&inner, &speed, "color"), tint);
        assert!(
            speed.get_bounding_client_rect().right()
                <= pick("#theirs .audio").get_bounding_client_rect().right() + 0.5,
            "the chip at the end of its line"
        );
        // The recording row.
        let red_dot = pick(".recording-dot");
        assert_eq!(style(&inner, &red_dot, "background-color"), red);
        assert_eq!(red_dot.get_bounding_client_rect().width(), 10.0);
        let delete = pick(".recording .recording-delete").get_bounding_client_rect();
        assert_eq!((delete.width(), delete.height()), (40.0, 40.0));
        assert_eq!(
            style(&inner, &pick(".recording-meter > i"), "background-color"),
            red
        );
        assert_eq!(
            pick(".recording-meter > i")
                .get_bounding_client_rect()
                .width(),
            3.0,
            "a live bar is 3 across, nothing of the app bar's box"
        );
        assert!(
            style(&inner, &pick(".recording-time"), "font-variant-numeric")
                .contains("tabular-nums")
        );
        // The review chip's round ▶.
        let local = pick(".staged.is-voice .local-play");
        let local_rect = local.get_bounding_client_rect();
        assert_eq!((local_rect.width(), local_rect.height()), (30.0, 30.0));
        assert_eq!(style(&inner, &local, "background-color"), tint);
        assert_eq!(
            style(
                &inner,
                &pick(".chip-wave > i.is-played"),
                "background-color"
            ),
            tint
        );
        frame.remove();
    }
}

/// ON A WIDE WINDOW THE LIVE WAVEFORM FILLS ITS STRIP: the bars the row
/// keeps for a strip of its measured width (`live_bars_for`) reach its left
/// edge in the shipped stylesheet — on a desktop as on a phone, as the
/// design's strip is full.
#[wasm_bindgen_test]
async fn the_live_waveform_fills_a_wide_windows_strip() {
    for width in [360, 1_600] {
        let (frame, inner) = framed(
            width,
            false,
            r#"<div class="composer"><div class="recording">
                 <button type="button" class="secondary recording-delete"><svg class="row-icon"></svg></button>
                 <span class="recording-dot"></span>
                 <span class="recording-time">0:42</span>
                 <span class="recording-meter" id="strip"></span>
                 <button type="button" class="secondary recording-stop"><svg class="row-icon"></svg></button>
               </div></div>"#,
        )
        .await;
        let strip = inner.get_element_by_id("strip").expect("the strip");
        let edge = strip.get_bounding_client_rect();
        let bars = crate::views::attach::live_bars_for(edge.width());
        for _ in 0..bars {
            let bar = inner.create_element("i").unwrap();
            bar.set_attribute("style", "height:50%").unwrap();
            strip.append_child(&bar).unwrap();
        }
        TimeoutFuture::new(30).await;
        let first = strip
            .first_element_child()
            .expect("a bar")
            .get_bounding_client_rect();
        assert!(edge.width() > 0.0, "laid out at {width}");
        assert!(
            first.left() <= edge.left() + 0.5,
            "at {width} px the {bars} bars reach the strip's left edge: \
             first bar at {}, strip from {} ({} wide)",
            first.left(),
            edge.left(),
            edge.width()
        );
        frame.remove();
    }
}

/// LARGER TEXT — a browser's minimum font size, a reader's zoom of text
/// alone — grows the recorder's captions with it: their line height is the
/// font's, not a fixed 14 px a forced larger word spills out of.
#[wasm_bindgen_test]
async fn the_recorders_captions_grow_with_larger_text() {
    let (frame, inner) = framed(
        600,
        false,
        r#"<style>.recorder-caption { font-size: 22px; }</style>
           <div class="recorder-item">
             <button class="recorder-control"></button>
             <span class="recorder-caption" id="caption">Voice message</span>
           </div>"#,
    )
    .await;
    let caption = inner.get_element_by_id("caption").expect("the caption");
    let height = caption.get_bounding_client_rect().height();
    assert!(
        height >= 22.0,
        "a 22 px caption is a line tall: {height} px"
    );
    frame.remove();
}

/// UNDER REDUCE MOTION nothing pulses, sweeps or fades: every animation and
/// transition the design adds is named in the stylesheet's reduced-motion
/// rules.
#[wasm_bindgen_test]
fn reduce_motion_stops_every_animation_the_design_adds() {
    let sheet = include_str!("../styles.css");
    let reduced: String = sheet
        .match_indices("@media (prefers-reduced-motion: reduce)")
        .map(|(at, _)| {
            let rest = &sheet[at..];
            let end = rest.find("\n}").unwrap_or(rest.len());
            rest[..end].to_string()
        })
        .collect();
    for selector in [
        ".recording-dot",
        ".recording,",
        ".staged.is-voice",
        ".not-sent",
        ".audio-speed",
        ".round-play",
        ".recorder-dot",
        ".recorder-slot",
        ".recorder-ring circle",
        ".round-ring.is-progress circle",
    ] {
        assert!(
            reduced.contains(selector),
            "{selector} is still under Reduce Motion"
        );
    }
}

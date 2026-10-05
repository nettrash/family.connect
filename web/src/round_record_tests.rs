//! Recording a video message — Phase 3 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md, S3, S8.7, S8.8, "Tests → Web"):
//! the live encoder, the probe, and the recorder as a person meets it.
//!
//! Tested in a real browser with a camera and a microphone that are not
//! ones: `webdriver.json` starts the test browser with Chrome's fake capture
//! devices and its permission prompt already answered. Nothing here opens a
//! real camera or microphone.

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::{HtmlVideoElement, MediaStream};

use fc_text::media_plan::{self, VideoPlan};
use fc_text::{media_probe, mp4_read};

use crate::encode::{
    self,
    testing::{read, whole},
};
use crate::recorder::{now_ms, Listening};
use crate::round_video::{self, Facing, Take};

pub(crate) fn run(source: &str) -> JsValue {
    js_sys::Function::new_no_args(source)
        .call0(&JsValue::NULL)
        .expect("running a stand-in")
}

/// Wait, up to `ms`, for `check`.
pub(crate) async fn until(ms: u32, check: impl Fn() -> bool) -> bool {
    for _ in 0..ms / 20 {
        if check() {
            return true;
        }
        TimeoutFuture::new(20).await;
    }
    check()
}

/// Whether a track has been stopped.
pub(crate) fn ended(track: &JsValue) -> bool {
    js_sys::Reflect::get(track, &JsValue::from_str("readyState"))
        .ok()
        .and_then(|state| state.as_string())
        .is_some_and(|state| state == "ended")
}

/// The fake camera, shown in a muted `<video>` on the page — the way the
/// recorder's preview shows it.
async fn preview() -> (HtmlVideoElement, MediaStream) {
    let stream = round_video::open_camera(None, Facing::User, true)
        .await
        .expect("the fake camera opens");
    assert_eq!(
        stream.get_audio_tracks().length(),
        0,
        "the microphone's track is stopped and taken out at once (S3.2)"
    );
    let document = web_sys::window().unwrap().document().unwrap();
    let video: HtmlVideoElement = document
        .create_element("video")
        .unwrap()
        .dyn_into()
        .unwrap();
    video.set_muted(true);
    video.set_attribute("playsinline", "").unwrap();
    video
        .set_attribute(
            "style",
            "position:fixed;top:0;left:0;width:160px;height:120px;z-index:60",
        )
        .unwrap();
    video.set_src_object(Some(&stream));
    document.body().unwrap().append_child(&video).unwrap();
    let _ = video.play();
    assert!(
        until(5_000, || video.video_width() > 0).await,
        "the fake camera shows a picture"
    );
    (video, stream)
}

/// THE LIVE ENCODER (Web → Phase 3; Tests → Web): the fake camera's frames,
/// cut to their centre square and encoded as they come, and the fake
/// microphone's sound beside them, make an MP4 whose index comes FIRST, whose
/// picture is 480 × 480 H.264 starting on a key frame with another at least
/// every two seconds, whose sound is AAC in one channel, and which lasts as
/// long as it was recorded — the profile, which the planner keeps as it is.
#[wasm_bindgen_test]
async fn the_live_encoder_makes_a_moov_first_480_square_h264_and_aac_clip() {
    let (video, stream) = preview().await;
    let config = round_video::picture_config()
        .await
        .expect("the test browser encodes H.264 480 × 480 in real time");
    // Made in the preview, as the recorder makes it: a browser stops the
    // camera's frames while it brings an encoder up.
    let picture = encode::LivePicture::start(&config).expect("an encoder");
    TimeoutFuture::new(1_200).await;
    let take = Take::begin(&video, picture).expect("a take begins");
    // The picture first, and the microphone a moment after it — as at
    // Record, where it opens inside the click while the frames already come.
    assert!(
        until(2_000, || take.counts().0 > 0).await,
        "the first frame"
    );
    let began = now_ms();
    TimeoutFuture::new(200).await;
    let microphone = round_video::open_microphone()
        .await
        .expect("the fake microphone opens");
    let sound = round_video::tap(&microphone, Listening::in_the_click())
        .await
        .expect("the microphone is tapped");
    take.hear(sound, Clone::clone(&microphone));
    TimeoutFuture::new(2_600).await;
    let recorded = now_ms() - began;
    let (kept, _) = take.counts();
    assert!(kept > 20, "frames were encoded as they came: {kept}");
    let clip = take.finish().await.expect("a clip");
    assert!(
        microphone.get_tracks().iter().all(|track| ended(&track)),
        "the microphone is let go of"
    );
    let bytes = whole(&clip.blob).await;
    assert_eq!(clip.blob.type_(), "video/mp4");
    let (top, movie) = read(&bytes);
    assert_eq!(top, [*b"ftyp", *b"moov", *b"mdat"], "moov first");

    let picture = movie.video().expect("a picture");
    let entry = picture.entry.as_ref().expect("its sample entry");
    assert_eq!(mp4_read::video_codec_name(entry), "h264");
    assert_eq!((entry.width, entry.height), (480, 480));
    assert_eq!(picture.rotation, Some(0), "upright pixels, no matrix");
    assert!(picture.samples[0].sync, "it starts on a key frame");
    let keys: Vec<f64> = picture
        .samples
        .iter()
        .filter(|sample| sample.sync)
        .map(|sample| sample.pts as f64 / f64::from(picture.timescale))
        .collect();
    let last = picture.samples.last().unwrap().pts as f64 / f64::from(picture.timescale);
    assert!(
        keys.len() >= 2,
        "a key frame every two seconds: {keys:?} of {} frames to {last}",
        picture.samples.len()
    );
    let mut gaps = keys
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect::<Vec<_>>();
    gaps.push(last - keys.last().unwrap());
    assert!(
        gaps.iter().all(|gap| *gap < 2.0 + 0.2),
        "never more than two seconds apart: {keys:?}"
    );
    let rate = picture.frame_rate().expect("a frame rate");
    assert!(rate <= 31.0, "at most 30 fps: {rate}");

    let sound = movie.audio().expect("sound");
    let entry = sound.entry.as_ref().expect("its sample entry");
    assert_eq!(mp4_read::audio_codec_name(entry), "aac");
    let asc = mp4_read::audio_config(&entry.config).expect("its config");
    assert_eq!(asc.channels, 1, "mono");
    assert!([44_100, 48_000].contains(&asc.sample_rate));

    let length = movie.duration_ms().expect("a length") as f64;
    assert!(
        (length - recorded).abs() < 400.0,
        "it lasts as long as it was recorded: {length} of {recorded}"
    );
    assert!((clip.duration_ms as f64 - recorded).abs() < 400.0);
    // The microphone opened after the first frame: the sound starts that
    // much into the clip, said with an empty edit in front of it.
    let lead = movie.lead_ms(sound).expect("the sound's lead");
    assert!(
        (150..1_500).contains(&lead),
        "the later track carries the lead: {lead}"
    );
    let presented = movie.presented_ms(sound).expect("the sound's length") as f64;
    assert!(
        presented > recorded - 600.0,
        "the sound runs nearly the whole clip: {presented} of {recorded}"
    );

    // The profile, to the letter: the planner would keep it as it is.
    let source = media_probe::video_source(&movie, "video/mp4", bytes.len() as u64)
        .expect("the planner reads it");
    assert_eq!(media_plan::plan_video(&source), VideoPlan::Keep);

    // And the poster: square, from the clip itself.
    let prepared = crate::prep::round_video(clip.blob.clone(), clip.duration_ms).await;
    assert_eq!(
        (prepared.kind.as_str(), prepared.mime.as_str()),
        ("video", "video/mp4")
    );
    assert_eq!((prepared.width, prepared.height), (Some(480), Some(480)));
    let poster = prepared.preview.expect("a poster");
    assert!(fc_text::media::matches_magic(
        "image/jpeg",
        &crate::prep::head(&poster, 12).await
    ));

    round_video::stop(&stream);
    video.remove();
}

/// A FRAME IS DROPPED RATHER THAN QUEUED past the encoder's few (Web →
/// Phase 3): frames offered faster than it can take them are left out, and
/// what it holds never grows past that — and the clip is still a clip.
#[wasm_bindgen_test]
async fn frames_are_dropped_rather_than_queued() {
    let (video, stream) = preview().await;
    let config = round_video::picture_config().await.unwrap();
    let mut picture = encode::LivePicture::start(&config).unwrap();
    TimeoutFuture::new(1_200).await;
    let mut queued = 0;
    for index in 0..60 {
        picture.frame(&video, 1_000.0 + f64::from(index) * 33.4);
        queued = queued.max(picture.queued());
    }
    let (kept, dropped) = picture.counts();
    assert!(dropped > 0, "the encoder could not take sixty at once");
    assert!(queued <= 8, "never more than its few waiting: {queued}");
    assert_eq!(kept + dropped, 60);
    let (track, _) = picture.finish().await.expect("still a clip");
    assert_eq!(track.samples.len(), kept);
    round_video::stop(&stream);
    video.remove();
}

/// AT MOST 30 FPS (the recording profile): a camera that delivers 60 frames
/// a second is thinned to 30 — every other frame, never a burst — while one
/// at 30, its frames a few milliseconds either side of their time, or at
/// 24, loses none.
#[wasm_bindgen_test]
async fn a_faster_camera_is_thinned_to_thirty_frames_a_second() {
    let (video, stream) = preview().await;
    let config = round_video::picture_config().await.unwrap();
    // Offered one by one, the encoder never behind: only the rate decides.
    async fn offered(video: &HtmlVideoElement, config: &js_sys::Object, times: &[f64]) -> Vec<f64> {
        let mut picture = encode::LivePicture::start(config).unwrap();
        TimeoutFuture::new(1_200).await;
        let mut kept = Vec::new();
        for at in times {
            assert!(
                until(2_000, || picture.queued() == 0).await,
                "the encoder keeps up"
            );
            if picture.frame(video, *at) == encode::Offered::Kept {
                kept.push(*at);
            }
        }
        let _ = picture.finish().await;
        kept
    }
    let sixty: Vec<f64> = (0..60)
        .map(|index| 1_000.0 + f64::from(index) * 1_000.0 / 60.0)
        .collect();
    let kept = offered(&video, &config, &sixty).await;
    assert_eq!(kept.len(), 30, "half of sixty: {kept:?}");
    assert!(
        kept.windows(2).all(|pair| pair[1] - pair[0] > 30.0),
        "every other frame: {kept:?}"
    );
    let jitter = [0.0, 4.0, -5.0, 6.0, -3.0, 5.0, -6.0, 2.0];
    let thirty: Vec<f64> = (0..48)
        .map(|index| 1_000.0 + f64::from(index) * 1_000.0 / 30.0 + jitter[index as usize % 8])
        .collect();
    assert_eq!(
        offered(&video, &config, &thirty).await.len(),
        48,
        "30 fps, unevenly: all"
    );
    let slower: Vec<f64> = (0..24)
        .map(|index| 1_000.0 + f64::from(index) * 1_000.0 / 24.0)
        .collect();
    assert_eq!(
        offered(&video, &config, &slower).await.len(),
        24,
        "24 fps: all"
    );
    round_video::stop(&stream);
    video.remove();
}

/// THE MICROPHONE WITH THE CAMERA (S3.2, S3.4): asked in the same request
/// only while the browser would still prompt for it — or, where it will not
/// say, until this device has asked once; refusals the browser holds are
/// read first, the camera's before the microphone's.
#[wasm_bindgen_test]
fn the_microphone_is_asked_with_the_camera_only_while_it_would_prompt() {
    use round_video::{asks_microphone, refused, Refusal};
    assert!(!asks_microphone(Some("granted"), false));
    assert!(!asks_microphone(Some("granted"), true));
    assert!(asks_microphone(Some("prompt"), true));
    assert!(asks_microphone(None, false));
    assert!(!asks_microphone(None, true));
    assert_eq!(
        refused(Some("denied"), Some("denied")),
        Some(Refusal::Camera)
    );
    assert_eq!(
        refused(Some("prompt"), Some("denied")),
        Some(Refusal::Microphone)
    );
    assert_eq!(refused(Some("granted"), None), None);
    assert_eq!(refused(None, None), None);
}

/// A take that is dropped — Delete, Close, the pane gone — keeps nothing,
/// and lets go of the microphone it opened.
#[wasm_bindgen_test]
async fn a_dropped_take_lets_go_of_its_microphone() {
    let (video, stream) = preview().await;
    let config = round_video::picture_config().await.unwrap();
    let take = Take::begin(&video, encode::LivePicture::start(&config).unwrap()).unwrap();
    let microphone = round_video::open_microphone().await.unwrap();
    let sound = round_video::tap(&microphone, Listening::in_the_click())
        .await
        .unwrap();
    take.hear(sound, Clone::clone(&microphone));
    TimeoutFuture::new(200).await;
    drop(take);
    assert!(ended(&microphone.get_audio_tracks().get(0)));
    round_video::stop(&stream);
    video.remove();
}

/// THE DECISION (S8.7): all three answers and not WebKit — any one missing
/// is no.
#[wasm_bindgen_test]
fn recording_needs_h264_aac_and_frame_callbacks_and_not_webkit() {
    assert!(round_video::records(false, true, true, true));
    assert!(!round_video::records(true, true, true, true), "WebKit");
    assert!(!round_video::records(false, false, true, true), "no H.264");
    assert!(!round_video::records(false, true, false, true), "no AAC");
    assert!(
        !round_video::records(false, true, true, false),
        "no frame callbacks"
    );
}

/// THE PROBE, asked of the browser (S8.7): this one records, has a camera
/// and is not WebKit; the same browser made to answer "no" to H.264, to AAC,
/// to have no frame callbacks — or to be WebKit — does not record. A
/// browser that cannot is never an exception.
#[wasm_bindgen_test]
async fn the_probe_asks_the_browser_and_any_no_is_no() {
    use crate::webcodecs::testing::{refusing, without, Lack};
    round_video::testing::forget();
    let here = round_video::probe().await;
    assert_eq!(
        here,
        round_video::Probe {
            camera: true,
            records: true
        },
        "the test browser records video messages"
    );
    assert!(!round_video::is_webkit());
    for lack in [refusing as Lack, without] {
        for class in ["VideoEncoder", "AudioEncoder"] {
            round_video::testing::forget();
            let _gone = lack(class);
            assert!(!round_video::probe().await.records, "{class}");
        }
    }
    {
        round_video::testing::forget();
        run(
            "window.__rvfc = HTMLVideoElement.prototype.requestVideoFrameCallback; \
             delete HTMLVideoElement.prototype.requestVideoFrameCallback;",
        );
        let answer = round_video::probe().await;
        run("HTMLVideoElement.prototype.requestVideoFrameCallback = window.__rvfc;");
        assert!(!answer.records, "no frame callbacks");
        assert!(answer.camera);
    }
    {
        round_video::testing::forget();
        run("Object.defineProperty(navigator, 'vendor', \
             {value: 'Apple Computer, Inc.', configurable: true});");
        let webkit = round_video::is_webkit();
        let answer = round_video::probe().await;
        run("delete navigator.vendor;");
        assert!(webkit && !answer.records, "WebKit waits for its trial");
    }
    assert!(!round_video::is_webkit(), "the stand-in is gone");
    {
        // The answer is asked once a page: a browser does not change.
        round_video::testing::forget();
        assert!(round_video::probe().await.records);
        let _gone = refusing("VideoEncoder");
        assert!(round_video::probe().await.records, "kept");
    }
    round_video::testing::forget();
}

/// The picture kept is the middle square of whatever the camera gives: of
/// 640 × 480, the 480 × 480 starting 80 across; of a portrait camera, the
/// middle of its height.
#[wasm_bindgen_test]
fn the_centre_square_is_kept() {
    use crate::encode::centre_square;
    assert_eq!(centre_square(640, 480), (80.0, 0.0, 480.0));
    assert_eq!(centre_square(480, 640), (0.0, 80.0, 480.0));
    assert_eq!(centre_square(1280, 720), (280.0, 0.0, 720.0));
    assert_eq!(centre_square(480, 480), (0.0, 0.0, 480.0));
    assert_eq!(centre_square(641, 480), (80.0, 0.0, 480.0));
}

/// The sound lines up with the picture: a microphone that opened 180 ms
/// after the first frame starts 180 ms into the clip; one that heard
/// something before it loses that much, in samples.
#[wasm_bindgen_test]
fn the_later_track_carries_the_lead() {
    assert_eq!(round_video::alignment(1_000.0, 1_180.4, 48_000), (180, 0));
    assert_eq!(round_video::alignment(1_000.0, 1_000.0, 48_000), (0, 0));
    assert_eq!(round_video::alignment(1_000.0, 950.0, 48_000), (0, 2_400));
    assert_eq!(round_video::alignment(1_000.0, 990.0, 44_100), (0, 441));
}

/// "We can't see anything" is a picture near black — a shutter, not a dim
/// room (S3.6).
#[wasm_bindgen_test]
fn near_black_is_a_shutter_not_a_dim_room() {
    let picture = |r: u8, g: u8, b: u8| -> Vec<u8> { [r, g, b, 255].repeat(64) };
    assert!(round_video::near_black(&picture(0, 0, 0)));
    assert!(round_video::near_black(&picture(10, 12, 9)));
    assert!(!round_video::near_black(&picture(40, 35, 30)), "a dim room");
    assert!(!round_video::near_black(&picture(200, 180, 170)));
    assert!(
        !round_video::near_black(&[]),
        "no picture is not a black one"
    );
    let mut half = picture(0, 0, 0);
    half[..128].copy_from_slice(&[255; 128]);
    assert!(!round_video::near_black(&half), "a lamp in a dark room");
}

/// The server's two keys, as one pair: the cap is the limit less half a
/// second, the warning ten seconds before it.
#[wasm_bindgen_test]
fn the_limits_come_from_the_server() {
    use round_video::RoundLimits;
    let limits = RoundLimits::of(Some(60_000), Some(12_582_912)).unwrap();
    assert_eq!((limits.cap_ms(), limits.warning_ms()), (59_500, 50_000));
    assert_eq!(RoundLimits::of(Some(60_000), None), None);
    assert_eq!(RoundLimits::of(None, Some(1)), None);
    assert_eq!(RoundLimits::of(Some(0), Some(1)), None);
    assert_eq!(RoundLimits::of(Some(60_000), Some(-1)), None);
}

// --- the recorder, as a person meets it ---------------------------------------------------------

mod recorder_tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gloo_timers::future::TimeoutFuture;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;
    use web_sys::{Element, HtmlElement, HtmlVideoElement};

    use super::{run, until};
    use crate::actions::Action;
    use crate::layout_tests::{
        click_labelled, fixed_root, install_stylesheet, message, props_with, query, recorder,
        IntoHtml,
    };
    use crate::recorder::testing::ClockAhead;
    use crate::round_video::{self, testing::Probed, Probe, RoundLimits};
    use crate::views::conversation::{Conversation, ConversationProps};

    const LIMITS: RoundLimits = RoundLimits {
        max_ms: 60_000,
        max_bytes: 12_582_912,
    };

    fn document() -> web_sys::Document {
        web_sys::window().unwrap().document().unwrap()
    }

    /// A pane `width × height` at the top left, the stylesheet installed —
    /// and nothing left over from a test before this one that failed with
    /// its recorder open.
    fn pane_of(width: u32, height: u32) -> HtmlElement {
        let body = document().body().unwrap();
        if let Ok(stale) = document().query_selector_all(".recorder-host") {
            for index in 0..stale.length() {
                if let Some(host) = stale.item(index) {
                    host.unchecked_into::<Element>().remove();
                }
            }
        }
        let children = body.children();
        for index in 0..children.length() {
            if let Some(child) = children.item(index) {
                let _ = child.remove_attribute("inert");
            }
        }
        install_stylesheet();
        fixed_root(&format!(
            "position:fixed;top:0;left:0;width:{width}px;height:{height}px;\
             display:grid;grid-template-rows:minmax(0,1fr);"
        ))
    }

    fn props(on_action: yew::Callback<Action>) -> ConversationProps {
        let mut props = props_with(vec![message(1), message(2)], None, on_action);
        props.round = Some(LIMITS);
        props
    }

    fn render(root: &HtmlElement, props: ConversationProps) -> yew::AppHandle<Conversation> {
        yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render()
    }

    fn door(root: &Element) -> Option<HtmlElement> {
        root.query_selector(".composer .video-door")
            .unwrap()
            .map(IntoHtml::dyn_into_html)
    }

    fn dialog() -> Option<Element> {
        document()
            .query_selector(".recorder-host .recorder[role=dialog]")
            .unwrap()
    }

    fn in_dialog(selector: &str) -> Option<HtmlElement> {
        dialog()?
            .query_selector(selector)
            .unwrap()
            .map(IntoHtml::dyn_into_html)
    }

    fn has(selector: &str) -> bool {
        dialog().is_some_and(|dialog| dialog.query_selector(selector).unwrap().is_some())
    }

    fn slot() -> HtmlElement {
        in_dialog(".recorder-slot").expect("the recorder's slot")
    }

    fn label(element: &Element) -> String {
        element.get_attribute("aria-label").unwrap_or_default()
    }

    fn status() -> String {
        in_dialog(".recorder-status")
            .and_then(|status| status.text_content())
            .unwrap_or_default()
    }

    /// What the recorder's own live region says.
    fn said() -> String {
        in_dialog(".visually-hidden[aria-live='polite']")
            .and_then(|node| node.text_content())
            .unwrap_or_default()
    }

    fn button_labelled(text: &str) -> HtmlElement {
        let dialog = dialog().expect("the recorder");
        let found = dialog.query_selector_all("button").unwrap();
        (0..found.length())
            .filter_map(|index| found.item(index))
            .map(|node| node.unchecked_into::<HtmlElement>())
            .find(|button| {
                label(button) == text || button.text_content().unwrap_or_default().trim() == text
            })
            .unwrap_or_else(|| panic!("no button {text:?} in the recorder"))
    }

    fn press(at: &Element, key: &str) {
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(key);
        init.set_bubbles(true);
        init.set_cancelable(true);
        at.dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init).unwrap(),
        )
        .unwrap();
        at.dispatch_event(
            &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keyup", &init).unwrap(),
        )
        .unwrap();
    }

    fn focused() -> Option<Element> {
        document().active_element()
    }

    /// Wait for the door, past its 600 ms guard, and open the recorder by it;
    /// then wait for Record to be ready.
    async fn open(root: &Element) {
        assert!(
            until(5_000, || door(root).is_some()).await,
            "the video button comes up once the probe has answered"
        );
        TimeoutFuture::new(650).await;
        let door = door(root).unwrap();
        let _ = door.focus();
        door.click();
        assert!(
            until(2_000, || dialog().is_some()).await,
            "the recorder opens"
        );
        assert!(
            until(8_000, || slot().get_attribute("aria-disabled").is_none()).await,
            "Record is ready once the camera shows its first frame: {}",
            status()
        );
    }

    async fn record_for(ms: u32) {
        slot().click();
        assert!(
            until(2_000, || label(&slot()) == "Stop recording").await,
            "recording"
        );
        TimeoutFuture::new(ms).await;
    }

    async fn into_review() {
        assert!(
            until(15_000, || label(&slot()) == "Send video message").await,
            "the clip in review: {}",
            status()
        );
    }

    /// Past the slot's 600 ms guard after a Stop (S1.1).
    async fn past_guard() {
        TimeoutFuture::new(650).await;
    }

    fn sent(log: &Rc<RefCell<Vec<Action>>>) -> Vec<crate::store::Draft> {
        log.borrow()
            .iter()
            .filter_map(|action| match action {
                Action::Send { draft, .. } => Some(draft.clone()),
                _ => None,
            })
            .collect()
    }

    /// THE WAY IN AND THE DIALOG (S1.4, S3.3, S3.4, S8.7): the video button
    /// sits in the empty field, labelled and with its tooltip, goes the
    /// moment a character is typed, and ignores a click for 600 ms after it
    /// appears. It opens the recorder: a modal dialog called "Video message"
    /// over the whole window, everything else inert, the focus on its slot —
    /// "Record", dimmed until the camera's first frame, with "Not recording"
    /// and, the first time on this device, "Only you can see this…". The
    /// preview is a mirror. Esc closes it, the page comes back, and so does
    /// the focus.
    #[wasm_bindgen_test]
    async fn the_video_button_opens_a_modal_recorder_and_esc_closes_it() {
        let _ = web_sys::window()
            .unwrap()
            .local_storage()
            .unwrap()
            .unwrap()
            .remove_item("fc.round.previewed");
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        assert!(until(5_000, || door(&root).is_some()).await, "the door");
        let button = door(&root).unwrap();
        assert_eq!(label(&button), "Record video message");
        assert_eq!(
            button.get_attribute("title").as_deref(),
            Some("Record a video message")
        );
        assert!(query(&root, "textarea").class_list().contains("has-door"));
        // Typed: gone, the field has its width back.
        let area: web_sys::HtmlTextAreaElement = query(&root, "textarea").dyn_into().unwrap();
        area.set_value("h");
        let init = web_sys::EventInit::new();
        init.set_bubbles(true);
        area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        assert!(
            until(500, || door(&root).is_none()).await,
            "typing hides it"
        );
        area.set_value("");
        area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
            .unwrap();
        assert!(
            until(500, || door(&root).is_some()).await,
            "back when empty"
        );
        // Back this instant: a click now is the drift of the last one.
        door(&root).unwrap().click();
        TimeoutFuture::new(100).await;
        assert!(dialog().is_none(), "ignored for 600 ms after it appears");

        open(&root).await;
        let layer = dialog().unwrap();
        assert_eq!(layer.get_attribute("aria-modal").as_deref(), Some("true"));
        assert_eq!(
            layer.get_attribute("aria-label").as_deref(),
            Some("Video message")
        );
        let style = web_sys::window()
            .unwrap()
            .get_computed_style(&layer)
            .unwrap()
            .unwrap();
        assert_eq!(style.get_property_value("position").unwrap(), "fixed");
        let app = root.parent_element().unwrap();
        assert!(
            root.has_attribute("inert") || app.has_attribute("inert"),
            "the rest of the page is inert"
        );
        assert_eq!(
            focused().as_ref(),
            Some(slot().unchecked_ref::<Element>()),
            "focus on the slot"
        );
        assert_eq!(label(&slot()), "Record");
        assert!(status().contains("Not recording"), "{}", status());
        assert!(status().contains("Only you can see this until you start recording."));
        let preview = in_dialog("video.recorder-preview").expect("the preview");
        let mirror = web_sys::window()
            .unwrap()
            .get_computed_style(&preview)
            .unwrap()
            .unwrap()
            .get_property_value("transform")
            .unwrap();
        assert!(mirror.starts_with("matrix(-1"), "mirrored: {mirror}");
        let preview: HtmlVideoElement = preview.unchecked_into();
        assert!(preview.muted(), "the preview is never heard");
        assert!(
            said().contains("Camera ready"),
            "said once the first frame is in: {}",
            said()
        );
        assert!(!round_video::in_progress(), "nothing recorded yet");

        press(&slot(), "Escape");
        assert!(
            until(1_000, || dialog().is_none()).await,
            "Esc closes the preview"
        );
        assert!(!root.has_attribute("inert") && !app.has_attribute("inert"));
        assert_eq!(
            focused().as_ref(),
            door(&root)
                .as_ref()
                .map(|door| door.unchecked_ref::<Element>())
        );
        handle.destroy();
        root.remove();
    }

    /// RECORD → STOP → REVIEW → SEND (S3.4): Record starts at once and is
    /// said; a Stop inside 600 ms is the second half of the Record click and
    /// does nothing; Stop after it makes the clip — the camera off — and
    /// REVIEW says "Video message · 0:0…" with Delete, Retake and Send. While
    /// it waits there, closing the tab would lose it (the `beforeunload`
    /// guard asks). Space plays and pauses it WHEREVER the focus is — on
    /// Retake, it does not retake. Send hands it to the outbox as a video
    /// message: one 480 × 480 video with its poster, round, carrying the
    /// reply; the recorder closes and "Video message sent" is said.
    #[wasm_bindgen_test]
    async fn record_stop_review_and_send_a_video_message() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;

        slot().click();
        assert!(until(2_000, || label(&slot()) == "Stop recording").await);
        assert!(
            round_video::in_progress(),
            "a recording the tab must not drop"
        );
        assert!(said().contains("Recording video"), "{}", said());
        assert!(has(".recorder-ring.is-recording"), "the red ring");
        slot().click();
        TimeoutFuture::new(100).await;
        assert_eq!(
            label(&slot()),
            "Stop recording",
            "a Stop inside the guard is ignored"
        );
        TimeoutFuture::new(1_400).await;
        slot().click();
        into_review().await;
        assert!(
            in_dialog("video.recorder-preview").is_none(),
            "the camera is off"
        );
        assert!(
            status().starts_with("Video message · 0:0"),
            "the status: {}",
            status()
        );
        assert!(round_video::in_progress(), "REVIEW is asked about too");
        button_labelled("Delete");
        let retake = button_labelled("Retake");
        let player: HtmlVideoElement = in_dialog("video.recorder-review").unwrap().unchecked_into();
        assert!(player.src().starts_with("blob:"));
        let mirror = web_sys::window()
            .unwrap()
            .get_computed_style(&player)
            .unwrap()
            .unwrap()
            .get_property_value("transform")
            .unwrap();
        assert_eq!(mirror, "none", "REVIEW is the file as it is sent");
        let _ = retake.focus();
        // Caught on the way down: the focused control never hears it, and
        // nothing of the browser's own happens with it.
        run(
            "window.__heard = 0; document.querySelector('.recorder-retake') \
             .addEventListener('keydown', () => window.__heard++);",
        );
        let init = web_sys::KeyboardEventInit::new();
        init.set_key(" ");
        init.set_bubbles(true);
        init.set_cancelable(true);
        let went_through = retake
            .dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keyup", &init).unwrap(),
            )
            .unwrap();
        assert!(!went_through, "the keyup's default is taken");
        press(&retake, " ");
        assert_eq!(
            run("return window.__heard").as_f64(),
            Some(0.0),
            "Retake never hears Space"
        );
        assert!(until(2_000, || !player.paused()).await, "Space plays");
        assert_eq!(label(&slot()), "Send video message", "and does not retake");
        press(&retake, " ");
        assert!(until(1_000, || player.paused()).await, "Space pauses");

        past_guard().await;
        slot().click();
        assert!(until(1_000, || dialog().is_none()).await, "Send closes it");
        assert!(!round_video::in_progress());
        let drafts = sent(&log);
        assert_eq!(drafts.len(), 1);
        let draft = &drafts[0];
        assert!(draft.round, "sent as a video message");
        assert!(draft.body.is_empty());
        assert_eq!(draft.attachments.len(), 1, "alone");
        let clip = &draft.attachments[0];
        assert_eq!(
            (clip.kind.as_str(), clip.mime.as_str()),
            ("video", "video/mp4")
        );
        assert_eq!((clip.width, clip.height), (Some(480), Some(480)));
        assert!(clip.preview.is_some(), "with its poster");
        assert!(clip
            .duration_ms
            .is_some_and(|ms| (1_000..3_000).contains(&ms)));
        assert!(query(&root, ".round-said")
            .text_content()
            .unwrap_or_default()
            .contains("Video message sent"));
        handle.destroy();
        root.remove();
    }

    /// UNDER A SECOND (S3.4): back to the preview, "That video was too
    /// short.", nothing sent and nothing held.
    #[wasm_bindgen_test]
    async fn a_stop_under_a_second_is_too_short() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(750).await;
        slot().click();
        assert!(
            until(2_000, || label(&slot()) == "Record").await,
            "back to the preview"
        );
        assert!(
            status().contains("That video was too short."),
            "{}",
            status()
        );
        assert!(said().contains("That video was too short."));
        assert!(!round_video::in_progress());
        assert!(
            in_dialog("video.recorder-preview").is_some(),
            "the camera still on"
        );
        assert!(sent(&log).is_empty());
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// ESC IN REVIEW ALWAYS ASKS (S3.4): "Delete video message?" — Keep
    /// keeps it, Delete closes the recorder with nothing sent. Delete under
    /// ten seconds needs no question.
    #[wasm_bindgen_test]
    async fn esc_in_review_asks_and_a_short_delete_does_not() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(1_300).await;
        slot().click();
        into_review().await;
        press(&slot(), "Escape");
        assert!(
            until(1_000, || document()
                .body()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("Delete video message?"))
            .await,
            "Esc asks"
        );
        click_labelled(&document().body().unwrap(), ".recorder button", "Keep");
        TimeoutFuture::new(50).await;
        assert_eq!(label(&slot()), "Send video message", "kept");
        button_labelled("Delete").click();
        assert!(
            until(1_000, || dialog().is_none()).await,
            "under ten seconds: at once"
        );
        assert!(sent(&log).is_empty() && !round_video::in_progress());
        handle.destroy();
        root.remove();
    }

    /// THE LENGTH LIMIT (S1.1, S3.4): at 50 s the ring turns orange and "10
    /// seconds left" is shown and said; at 59.5 s the recording stops into
    /// REVIEW with "Recording stopped at one minute." — never sent by it.
    #[wasm_bindgen_test]
    async fn the_limit_warns_at_fifty_seconds_and_stops_into_review() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(1_200).await;
        let mut ahead = ClockAhead::by(50_000.0);
        assert!(
            until(1_000, || status().contains("10 seconds left")).await,
            "{}",
            status()
        );
        assert!(said().contains("10 seconds left"));
        assert!(has(".recorder-ring.is-warning"), "orange");
        ahead.more(9_500.0);
        into_review().await;
        assert!(
            status().contains("Recording stopped at one minute."),
            "{}",
            status()
        );
        assert!(sent(&log).is_empty(), "a limit never sends");
        drop(ahead);
        // A minute of it: Delete asks.
        button_labelled("Delete").click();
        TimeoutFuture::new(50).await;
        click_labelled(&document().body().unwrap(), ".recorder button", "Delete");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// INTERRUPTIONS (S4): a hidden tab or a call stops a recording into
    /// REVIEW, and keeps a clip in review; the preview just closes.
    #[wasm_bindgen_test]
    async fn a_hidden_tab_or_a_call_stops_into_review_and_closes_a_preview() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let mut handle = render(&root, props(on_action.clone()));
        open(&root).await;
        record_for(1_300).await;
        run(
            "Object.defineProperty(document, 'hidden', { get: () => true, configurable: true }); \
             Object.defineProperty(document, 'visibilityState', \
               { get: () => 'hidden', configurable: true }); \
             document.dispatchEvent(new Event('visibilitychange'));",
        );
        into_review().await;
        run("delete document.hidden; delete document.visibilityState; \
             document.dispatchEvent(new Event('visibilitychange'));");
        let mut calling = props(on_action.clone());
        calling.on_call = true;
        handle.update(calling);
        TimeoutFuture::new(100).await;
        assert_eq!(label(&slot()), "Send video message", "a call keeps REVIEW");
        let app = root.parent_element().unwrap();
        // …under the call, whose bar must be answerable: the recorder steps
        // aside and gives the page back until the call is over.
        assert!(dialog().unwrap().has_attribute("hidden"), "under the call");
        assert!(!root.has_attribute("inert") && !app.has_attribute("inert"));
        handle.update(props(on_action.clone()));
        TimeoutFuture::new(100).await;
        assert!(!dialog().unwrap().has_attribute("hidden"), "back after it");
        assert!(root.has_attribute("inert") || app.has_attribute("inert"));
        assert_eq!(label(&slot()), "Send video message", "still in REVIEW");
        button_labelled("Delete").click();
        assert!(until(1_000, || dialog().is_none()).await);

        // A call closes the preview, and a recording stops into review.
        open(&root).await;
        let mut calling = props(on_action.clone());
        calling.on_call = true;
        handle.update(calling);
        assert!(
            until(1_000, || dialog().is_none()).await,
            "the preview closes"
        );
        handle.update(props(on_action.clone()));
        open(&root).await;
        record_for(1_300).await;
        let mut calling = props(on_action.clone());
        calling.on_call = true;
        handle.update(calling);
        into_review().await;
        assert!(sent(&log).is_empty(), "an interruption never sends");
        button_labelled("Delete").click();
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// INTERRUPTED IN THE FIRST SECOND (S4: "under 1.0 s, 'not sent' means
    /// deleted"): a call or a hidden tab deletes the take and closes the
    /// recorder — the camera off, never a preview left running under the
    /// call; a camera that goes away leaves the camera sentence, the camera
    /// off and Record dimmed, never a preview of a stream that has ended.
    #[wasm_bindgen_test]
    async fn an_interruption_in_the_first_second_deletes_and_turns_the_camera_off() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let mut handle = render(&root, props(on_action.clone()));
        let camera_track = || {
            in_dialog("video.recorder-preview")
                .and_then(|preview| preview.unchecked_into::<HtmlVideoElement>().src_object())
                .map(|stream| {
                    stream
                        .unchecked_into::<web_sys::MediaStream>()
                        .get_video_tracks()
                        .get(0)
                })
                .expect("the camera's track")
        };

        // A call.
        open(&root).await;
        record_for(300).await;
        let track = camera_track();
        let mut calling = props(on_action.clone());
        calling.on_call = true;
        handle.update(calling);
        let closed = until(1_000, || dialog().is_none()).await;
        let off = super::ended(&track);
        handle.update(props(on_action.clone()));
        if !closed {
            press(&slot(), "Escape");
            let _ = until(1_000, || dialog().is_none()).await;
        }
        assert!(closed, "a call in the first second closes the recorder");
        assert!(off, "and the camera is off");
        assert!(!round_video::in_progress());

        // A hidden tab.
        open(&root).await;
        record_for(300).await;
        let track = camera_track();
        run(
            "Object.defineProperty(document, 'hidden', { get: () => true, configurable: true }); \
             Object.defineProperty(document, 'visibilityState', \
               { get: () => 'hidden', configurable: true }); \
             document.dispatchEvent(new Event('visibilitychange'));",
        );
        let closed = until(1_000, || dialog().is_none()).await;
        run("delete document.hidden; delete document.visibilityState; \
             document.dispatchEvent(new Event('visibilitychange'));");
        let off = super::ended(&track);
        if !closed {
            press(&slot(), "Escape");
            let _ = until(1_000, || dialog().is_none()).await;
        }
        assert!(
            closed,
            "a hidden tab in the first second closes the recorder"
        );
        assert!(off, "and the camera is off");

        // The camera gone.
        open(&root).await;
        record_for(300).await;
        let track = camera_track();
        track
            .unchecked_ref::<web_sys::EventTarget>()
            .dispatch_event(&web_sys::Event::new("ended").unwrap())
            .unwrap();
        assert!(
            until(1_000, || status()
                .contains("The camera is being used by another app."))
            .await,
            "the camera sentence: {}",
            status()
        );
        assert!(super::ended(&track), "the camera is off");
        assert!(in_dialog("video.recorder-preview").is_none(), "no preview");
        assert_eq!(
            slot().get_attribute("aria-disabled").as_deref(),
            Some("true"),
            "Record dimmed"
        );
        assert!(!round_video::in_progress());
        button_labelled("Close").click();
        assert!(until(1_000, || dialog().is_none()).await);
        assert!(sent(&log).is_empty());
        handle.destroy();
        root.remove();
    }

    /// THE SLOT'S GUARD AFTER STOP (S1.1: "Record → Stop → Send in the
    /// recorder"): the second click of a double click on Stop never sends
    /// the clip, nor starts a new recording after "That video was too
    /// short." — and once 600 ms are past, it does.
    #[wasm_bindgen_test]
    async fn the_slot_waits_out_its_guard_after_stop() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(1_200).await;
        slot().click();
        let stopped = crate::recorder::now_ms();
        let mut clicked_in_review = false;
        while crate::recorder::now_ms() - stopped < 500.0 {
            if let Some(slot) = in_dialog(".recorder-slot") {
                if label(&slot) == "Send video message" {
                    slot.click();
                    clicked_in_review = true;
                }
            }
            TimeoutFuture::new(10).await;
        }
        assert!(clicked_in_review, "REVIEW came inside the guard");
        assert!(
            sent(&log).is_empty(),
            "a double click on Stop sends nothing"
        );
        assert!(dialog().is_some());
        TimeoutFuture::new(150).await;
        slot().click();
        assert!(until(1_000, || dialog().is_none()).await, "then Send sends");
        assert_eq!(sent(&log).len(), 1);

        open(&root).await;
        record_for(750).await;
        slot().click();
        let stopped = crate::recorder::now_ms();
        let mut clicked_ready = false;
        while crate::recorder::now_ms() - stopped < 500.0 {
            if label(&slot()) == "Record" && slot().get_attribute("aria-disabled").is_none() {
                slot().click();
                clicked_ready = true;
            }
            TimeoutFuture::new(10).await;
        }
        assert!(clicked_ready, "Record was ready inside the guard");
        assert_eq!(
            label(&slot()),
            "Record",
            "no new recording from the double click"
        );
        TimeoutFuture::new(150).await;
        assert!(until(8_000, || slot().get_attribute("aria-disabled").is_none()).await);
        slot().click();
        assert!(
            until(2_000, || label(&slot()) == "Stop recording").await,
            "then Record records"
        );
        handle.destroy();
        TimeoutFuture::new(50).await;
        assert!(!round_video::in_progress(), "gone with the pane");
        root.remove();
    }

    /// THE MICROPHONE IS ASKED WITH THE CAMERA ONLY THE FIRST TIME (S3.2,
    /// S3.4): once the browser says it is allowed, opening the recorder asks
    /// for the camera alone — no microphone indicator under "Not
    /// recording"; while it would prompt, ONE request asks for both; and
    /// where the browser will not say, the device remembers it asked.
    #[wasm_bindgen_test]
    async fn the_microphone_is_asked_with_the_camera_only_the_first_time() {
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        run("window.__asks = []; \
             window.__gum = navigator.mediaDevices.getUserMedia; \
             navigator.mediaDevices.getUserMedia = function(c) { \
               window.__asks.push(!!(c && c.audio)); \
               return window.__gum.call(navigator.mediaDevices, c); }; \
             window.__query = navigator.permissions.query; \
             window.__state = {camera: 'granted', microphone: 'granted'}; \
             navigator.permissions.query = (d) => Promise.resolve({state: window.__state[d.name]});");
        let restore = || {
            run("navigator.mediaDevices.getUserMedia = window.__gum; \
                 navigator.permissions.query = window.__query;")
        };
        let asked = || -> Vec<bool> {
            let list = js_sys::Array::from(&run("return window.__asks"));
            run("window.__asks = [];");
            list.iter().map(|value| value.is_truthy()).collect()
        };
        let open_and_close = || async {
            open(&root).await;
            button_labelled("Close").click();
            assert!(until(1_000, || dialog().is_none()).await);
        };
        open_and_close().await;
        let granted = asked();
        run("window.__state = {camera: 'granted', microphone: 'prompt'};");
        open_and_close().await;
        let prompting = asked();
        run("navigator.permissions.query = () => Promise.reject(new TypeError('no'));");
        storage.remove_item("fc.round.asked").unwrap();
        open_and_close().await;
        let unknown_first = asked();
        open_and_close().await;
        let unknown_again = asked();
        restore();
        assert_eq!(granted, [false], "allowed already: the camera alone");
        assert_eq!(prompting, [true], "it would prompt: one request for both");
        assert_eq!(
            unknown_first,
            [true],
            "the browser won't say: the first time, both"
        );
        assert_eq!(unknown_again, [false], "and never again on this device");
        handle.destroy();
        root.remove();
    }

    /// A PREVIEW NOBODY USES turns the camera off after 60 s, and says so
    /// once the page is back (S1.1, S3.4).
    #[wasm_bindgen_test]
    async fn an_idle_preview_turns_the_camera_off_after_a_minute() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        let ahead = ClockAhead::by(60_500.0);
        assert!(until(2_000, || dialog().is_none()).await, "closed");
        drop(ahead);
        assert!(query(&root, ".round-said")
            .text_content()
            .unwrap_or_default()
            .contains("Camera turned off"));
        handle.destroy();
        root.remove();
    }

    /// WHERE IT IS NOT OFFERED (S1.2, S1.4, S1.5, S8.7): a browser whose
    /// probe fails has no video button, and its paperclip item and the
    /// microphone's menu item say why instead of opening; a server without
    /// the keys, or the assistant's chat, offers nothing at all; during a
    /// call the button is dimmed and says so.
    #[wasm_bindgen_test]
    async fn where_it_cannot_record_the_menus_explain_and_nothing_opens() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        {
            let _probed = Probed::as_if(Probe {
                camera: true,
                records: false,
            });
            let handle = render(&root, props(on_action.clone()));
            TimeoutFuture::new(100).await;
            assert!(door(&root).is_none(), "no button in a browser that cannot");
            query(&root, "[aria-label='Attach']")
                .dyn_into_html()
                .click();
            TimeoutFuture::new(20).await;
            let item = query(&root, ".attach-menu .is-dimmed[role=menuitem]");
            assert_eq!(item.text_content().as_deref(), Some("Record Video Message"));
            item.dyn_into_html().click();
            TimeoutFuture::new(50).await;
            assert!(dialog().is_none());
            assert!(query(&root, ".media-notice")
                .text_content()
                .unwrap_or_default()
                .contains("This browser can't record video messages. Voice messages work."));
            // The microphone's menu, by a right-click.
            let mic = query(&root, ".composer .slot");
            let init = web_sys::MouseEventInit::new();
            init.set_bubbles(true);
            init.set_cancelable(true);
            mic.dispatch_event(
                &web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap(),
            )
            .unwrap();
            TimeoutFuture::new(20).await;
            let found = root
                .query_selector_all(".slot-menu [role=menuitem]")
                .unwrap();
            let item = (0..found.length())
                .filter_map(|index| found.item(index))
                .map(|node| node.unchecked_into::<HtmlElement>())
                .find(|item| item.text_content().as_deref() == Some("Record Video Message"))
                .expect("the menu's video item");
            assert_eq!(
                item.get_attribute("aria-disabled").as_deref(),
                Some("true"),
                "dimmed in a browser that cannot"
            );
            let reason = document()
                .get_element_by_id(&item.get_attribute("aria-describedby").expect("its reason"))
                .and_then(|reason| reason.text_content())
                .unwrap_or_default();
            assert_eq!(
                reason,
                "This browser can't record video messages. Voice messages work."
            );
            item.click();
            TimeoutFuture::new(50).await;
            assert!(dialog().is_none(), "the menu item explains too");
            handle.destroy();
        }
        {
            // A server from before video messages: nothing at all.
            let _probed = Probed::as_if(Probe {
                camera: true,
                records: true,
            });
            let mut older = props(on_action.clone());
            older.round = None;
            let handle = render(&root, older);
            TimeoutFuture::new(100).await;
            assert!(door(&root).is_none());
            query(&root, "[aria-label='Attach']")
                .dyn_into_html()
                .click();
            TimeoutFuture::new(20).await;
            assert!(!query(&root, ".attach-menu")
                .text_content()
                .unwrap_or_default()
                .contains("Record Video Message"));
            handle.destroy();
            // The assistant's chat: none either.
            let mut assistant = props(on_action.clone());
            assistant.item.chat.kind = "ai".into();
            let handle = render(&root, assistant);
            TimeoutFuture::new(100).await;
            assert!(door(&root).is_none());
            handle.destroy();
            // No camera: none.
            drop(_probed);
            let _probed = Probed::as_if(Probe {
                camera: false,
                records: true,
            });
            let handle = render(&root, props(on_action.clone()));
            TimeoutFuture::new(100).await;
            assert!(door(&root).is_none(), "a device without a camera");
            handle.destroy();
        }
        {
            // During a call: dimmed, and saying why.
            let _probed = Probed::as_if(Probe {
                camera: true,
                records: true,
            });
            let mut calling = props(on_action.clone());
            calling.on_call = true;
            let handle = render(&root, calling);
            assert!(until(1_000, || door(&root).is_some()).await);
            let button = door(&root).unwrap();
            assert_eq!(
                button.get_attribute("aria-disabled").as_deref(),
                Some("true")
            );
            TimeoutFuture::new(650).await;
            button.click();
            TimeoutFuture::new(50).await;
            assert!(dialog().is_none());
            assert!(query(&root, ".media-notice")
                .text_content()
                .unwrap_or_default()
                .contains("You can record a message after the call."));
            // The paperclip's item too: dimmed, saying why, never opening.
            let dismiss = query(&root, ".media-notice [aria-label='Dismiss']").dyn_into_html();
            dismiss.click();
            TimeoutFuture::new(30).await;
            query(&root, "[aria-label='Attach']")
                .dyn_into_html()
                .click();
            TimeoutFuture::new(20).await;
            let found = root
                .query_selector_all(".attach-menu .is-dimmed[role=menuitem]")
                .unwrap();
            let item = (0..found.length())
                .filter_map(|index| found.item(index))
                .map(|node| node.unchecked_into::<HtmlElement>())
                .find(|item| item.text_content().as_deref() == Some("Record Video Message"))
                .expect("dimmed during a call");
            item.click();
            TimeoutFuture::new(50).await;
            assert!(dialog().is_none(), "not during a call");
            assert!(query(&root, ".media-notice")
                .text_content()
                .unwrap_or_default()
                .contains("You can record a message after the call."));
            handle.destroy();
        }
        root.remove();
    }

    /// A CAMERA REFUSED (S3.2): the recorder opens straight to the refusal —
    /// asked first, no prompt raised for the other — with the camera off,
    /// Record dimmed and "Record a voice message instead" offered; a
    /// microphone refused already offers nothing else. A combined request
    /// the browser will not say more of is the combined sentence.
    #[wasm_bindgen_test]
    async fn a_refusal_says_which_and_asks_nothing_more() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        assert!(until(5_000, || door(&root).is_some()).await);
        run("window.__asked = 0; \
             window.__gum = navigator.mediaDevices.getUserMedia; \
             navigator.mediaDevices.getUserMedia = function() { window.__asked++; \
               return Promise.reject(Object.assign(new Error('no'), {name: 'NotAllowedError'})); }; \
             window.__query = navigator.permissions.query; \
             window.__state = {camera: 'denied', microphone: 'prompt'}; \
             navigator.permissions.query = (d) => Promise.resolve({state: window.__state[d.name]});");
        let restore = || {
            run("navigator.mediaDevices.getUserMedia = window.__gum; \
                 navigator.permissions.query = window.__query;")
        };
        TimeoutFuture::new(650).await;
        door(&root).unwrap().click();
        let camera = "Family needs permission to use your camera. Allow it in your browser's settings for this site.";
        if !until(2_000, || status().contains(camera)).await {
            restore();
            panic!("the camera refusal: {}", status());
        }
        let asked = run("return window.__asked").as_f64();
        assert_eq!(asked, Some(0.0), "no prompt raised");
        assert_eq!(
            slot().get_attribute("aria-disabled").as_deref(),
            Some("true")
        );
        button_labelled("Record a voice message instead");
        assert!(in_dialog("video").is_none(), "the camera is off");
        button_labelled("Close").click();
        assert!(until(1_000, || dialog().is_none()).await);

        run("window.__state = {camera: 'prompt', microphone: 'denied'};");
        TimeoutFuture::new(650).await;
        door(&root).unwrap().click();
        let microphone = "Family needs permission to use your microphone. Allow it in your browser's settings for this site.";
        let found = until(2_000, || status().contains(microphone)).await;
        let voice_offered = dialog()
            .map(|dialog| dialog.text_content().unwrap_or_default())
            .is_some_and(|text| text.contains("Record a voice message instead"))
            || in_dialog(".recorder-voice").is_some();
        button_labelled("Close").click();
        assert!(until(1_000, || dialog().is_none()).await);

        // Neither denied yet, and the request refused: the browser will not
        // say which — the combined sentence.
        // …and the device has not asked for both before, or it would ask
        // for the camera alone.
        web_sys::window()
            .unwrap()
            .local_storage()
            .unwrap()
            .unwrap()
            .remove_item("fc.round.asked")
            .unwrap();
        run(
            "window.__state = {camera: 'prompt', microphone: 'prompt'}; \
             navigator.permissions.query = () => Promise.reject(new TypeError('no'));",
        );
        TimeoutFuture::new(650).await;
        door(&root).unwrap().click();
        let both = "Family needs permission to use your camera and microphone. Allow them in your browser's settings for this site.";
        let combined = until(2_000, || status().contains(both)).await;
        let asked = run("return window.__asked").as_f64();
        button_labelled("Close").click();
        restore();
        assert!(found, "the microphone refusal: {}", status());
        assert!(!voice_offered, "nothing else is offered");
        assert!(combined, "the combined sentence");
        assert_eq!(asked, Some(1.0), "one request for both");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// "RECORD A VOICE MESSAGE INSTEAD" (S3.4): one tap undoes a mis-tap on
    /// the video button — the camera closes and a hands-free voice
    /// recording starts; while a voice message that was not sent waits, it
    /// is dimmed and says why.
    #[wasm_bindgen_test]
    async fn the_preview_offers_a_voice_message_instead() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action.clone()));
        open(&root).await;
        button_labelled("Record a voice message instead").click();
        assert!(
            until(1_000, || dialog().is_none()).await,
            "the camera closes"
        );
        assert!(
            until(5_000, || root
                .query_selector(".composer .recording")
                .unwrap()
                .is_some())
            .await,
            "a voice recording runs"
        );
        query(&root, ".composer .slot").dyn_into_html().click();
        TimeoutFuture::new(300).await;
        handle.destroy();

        let mut waiting = props(on_action);
        waiting.not_sent = vec![crate::store::NotSent {
            id: 1,
            note: crate::staged::Prepared {
                kind: "audio".into(),
                mime: "audio/mp4".into(),
                duration_ms: Some(4_000),
                ..Default::default()
            },
            duration_ms: 4_000,
            reply_to_message_id: None,
            caption: String::new(),
        }];
        let handle = render(&root, waiting);
        open(&root).await;
        let voice = button_labelled("Record a voice message instead");
        assert_eq!(
            voice.get_attribute("aria-disabled").as_deref(),
            Some("true")
        );
        voice.click();
        TimeoutFuture::new(50).await;
        assert!(dialog().is_some(), "dimmed: it stays");
        assert!(status().contains("Send or delete the voice message that wasn't sent first."));
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// THE LAYOUT AT PHONE WIDTHS (S3.3, S8.8): the recorder, measured
    /// against the pane it lies over — the circle as S3.3's arithmetic says
    /// and inside the pane, nothing wider than the window, and in a pane
    /// shorter than 480 (a phone on its side; this test window is 413 tall)
    /// the controls in a column at the trailing edge. Then the same markup in
    /// frames the size of phones held upright, where the control row lies
    /// exactly where the composer's row was and the slot in the Send
    /// button's place.
    #[wasm_bindgen_test]
    async fn the_recorder_lies_over_the_pane_at_phone_widths() {
        use crate::views::round_recorder::geometry;
        let viewport = web_sys::window()
            .unwrap()
            .inner_height()
            .unwrap()
            .as_f64()
            .unwrap();
        let mut markup = String::new();
        for (width, height) in [(640u32, 360u32), (360, 413), (756, 413)] {
            let height = height.min(viewport as u32);
            let root = pane_of(width, height);
            let (_log, on_action) = recorder();
            let handle = render(&root, props(on_action));
            open(&root).await;
            let shape = geometry(f64::from(width), f64::from(height), 0.0);
            let circle = in_dialog(".recorder-circle")
                .unwrap()
                .get_bounding_client_rect();
            assert_eq!(
                (circle.width(), circle.height()),
                (shape.diameter, shape.diameter),
                "at {width} × {height}"
            );
            assert!(
                circle.top() >= 0.0
                    && circle.bottom() <= f64::from(height)
                    && circle.left() >= 0.0
                    && circle.right() <= f64::from(width),
                "inside the pane at {width} × {height}"
            );
            let layer = dialog().unwrap();
            assert!(
                layer.scroll_width() <= layer.client_width(),
                "no sideways scroll"
            );
            let slot_rect = slot().get_bounding_client_rect();
            assert!(
                slot_rect.right() <= f64::from(width) && slot_rect.bottom() <= f64::from(height)
            );
            assert!(shape.column && layer.class_list().contains("is-column"));
            assert!(
                slot_rect.left() >= circle.right(),
                "the controls beside the circle"
            );
            markup = layer.outer_html();
            press(&slot(), "Escape");
            assert!(until(1_000, || dialog().is_none()).await);
            handle.destroy();
            root.remove();
        }
        // A pane taller than the window: the slot, focused below the fold,
        // must not scroll the recorder up to show itself.
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        assert_eq!(dialog().unwrap().scroll_top(), 0, "clipped, never scrolled");
        assert!(
            in_dialog(".recorder-status")
                .unwrap()
                .get_bounding_client_rect()
                .top()
                >= 0.0
        );
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();

        // Upright phones, in frames of their size: the recorder's own
        // markup, measured for that pane — 64 tall at the bottom, the
        // composer's row.
        let markup = markup.replace("recorder is-column", "recorder");
        for (width, height) in [(320u32, 568u32), (360, 640), (390, 844)] {
            let shape = geometry(f64::from(width), f64::from(height), 0.0);
            assert!(!shape.column);
            let style = format!(
                "--pane-left:0px;--pane-top:0px;--pane-width:{width}px;--pane-height:{height}px;\
                 --row-height:64px;--row-bottom:0px;--circle:{}px",
                shape.diameter
            );
            let start = markup.find("style=\"").unwrap() + 7;
            let end = start + markup[start..].find('"').unwrap();
            let sized = format!("{}{}{}", &markup[..start], style, &markup[end..]);
            let (frame, inner) = framed(width, height, &sized).await;
            let pick = |selector: &str| {
                inner
                    .query_selector(selector)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{selector}"))
                    .get_bounding_client_rect()
            };
            let circle = pick(".recorder-circle");
            assert_eq!(circle.width(), shape.diameter, "at {width}");
            assert!(circle.top() >= 0.0 && circle.bottom() <= f64::from(height));
            assert!(circle.left() >= 0.0 && circle.right() <= f64::from(width));
            let status = pick(".recorder-status");
            assert!(
                status.bottom() <= circle.top(),
                "the status above the circle"
            );
            let row = pick(".recorder-row");
            assert_eq!(
                (row.top(), row.bottom()),
                (f64::from(height) - 64.0, f64::from(height))
            );
            let slot = pick(".recorder-slot");
            assert_eq!(
                slot.right(),
                f64::from(width) - 16.0,
                "the Send button's place"
            );
            assert!((slot.top() + slot.height() / 2.0 - (f64::from(height) - 32.0)).abs() < 1.0);
            assert!(
                row.top() >= circle.bottom(),
                "the controls under the circle"
            );
            let layer = inner.query_selector(".recorder").unwrap().unwrap();
            let page = inner.document_element().unwrap();
            assert!(
                page.scroll_width() <= width as i32,
                "no sideways scroll: page {} recorder {} / {} at {width}",
                page.scroll_width(),
                layer.scroll_width(),
                layer.client_width()
            );
            frame.remove();
        }
    }

    /// THE REPLY GOES WITH IT (S1.5, S3.3): the composer's primed reply is
    /// quoted in the recorder over the control row and carried by the video;
    /// its ✕ drops it, there and in the composer.
    #[wasm_bindgen_test]
    async fn the_reply_rides_with_the_video_and_its_cross_drops_it() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        TimeoutFuture::new(80).await;
        let reply_to_first = || async {
            let bubble = query(&root, "#m-1");
            query(&bubble, ".more").dyn_into_html().click();
            TimeoutFuture::new(30).await;
            click_labelled(&bubble, ".menu [role=menuitem]", "Reply");
            TimeoutFuture::new(50).await;
        };
        reply_to_first().await;
        open(&root).await;
        let banner = in_dialog(".recorder-banner").expect("the reply, quoted");
        assert!(banner
            .text_content()
            .unwrap_or_default()
            .starts_with("Replying to"));
        record_for(1_300).await;
        slot().click();
        into_review().await;
        assert!(in_dialog(".recorder-banner").is_some(), "in every state");
        past_guard().await;
        slot().click();
        assert!(until(1_000, || dialog().is_none()).await);
        let drafts = sent(&log);
        assert_eq!(drafts.len(), 1);
        assert_eq!(
            drafts[0].reply_to_message_id,
            Some(1),
            "the reply went with it"
        );
        assert!(
            root.query_selector(".composer-banner").unwrap().is_none(),
            "and is taken off the composer"
        );

        reply_to_first().await;
        open(&root).await;
        button_labelled("Cancel reply").click();
        assert!(
            until(500, || in_dialog(".recorder-banner").is_none()).await,
            "dropped"
        );
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        assert!(root.query_selector(".composer-banner").unwrap().is_none());
        handle.destroy();
        root.remove();
    }

    /// TAB GOES ROUND THE RECORDER and never out of it (S3.4): from the last
    /// control to the first, and back.
    #[wasm_bindgen_test]
    async fn tab_goes_round_the_recorder() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        let tab = |at: &Element, shift: bool| {
            let init = web_sys::KeyboardEventInit::new();
            init.set_key("Tab");
            init.set_shift_key(shift);
            init.set_bubbles(true);
            init.set_cancelable(true);
            at.dispatch_event(
                &web_sys::KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
                    .unwrap(),
            )
            .unwrap();
        };
        let close = button_labelled("Close");
        let _ = slot().focus();
        tab(&slot(), false);
        assert_eq!(
            focused().as_ref(),
            Some(close.unchecked_ref::<Element>()),
            "round to the first"
        );
        tab(&close, true);
        assert_eq!(
            focused().as_ref(),
            Some(slot().unchecked_ref::<Element>()),
            "and back to the last"
        );
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// A DESKTOP WITH MORE THAN ONE CAMERA offers "Choose camera", a menu of
    /// their names; the choice is remembered on the device, and a chosen
    /// camera that is not there gives way to one that is (S3.5).
    #[wasm_bindgen_test]
    async fn more_than_one_camera_offers_a_choice_that_is_remembered() {
        let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
        let _ = storage.remove_item("fc.round.camera");
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        run("window.__enum = navigator.mediaDevices.enumerateDevices; \
             navigator.mediaDevices.enumerateDevices = () => Promise.resolve([ \
               {kind: 'videoinput', deviceId: 'front', label: 'Front Camera'}, \
               {kind: 'videoinput', deviceId: 'gone', label: 'USB Camera'}, \
               {kind: 'audioinput', deviceId: 'mic', label: 'Microphone'}]);");
        open(&root).await;
        let found = until(2_000, || {
            dialog().is_some_and(|dialog| {
                dialog
                    .query_selector("[aria-label='Choose camera']")
                    .unwrap()
                    .is_some()
            })
        })
        .await;
        if !found {
            run("navigator.mediaDevices.enumerateDevices = window.__enum;");
            panic!("Choose camera is offered");
        }
        button_labelled("Choose camera").click();
        TimeoutFuture::new(30).await;
        let menu = in_dialog(".recorder-cameras[role=menu]").expect("the menu");
        let names = menu.text_content().unwrap_or_default();
        click_labelled(&menu, "[role=menuitem]", "USB Camera");
        let back = until(8_000, || {
            in_dialog(".recorder-cameras").is_none()
                && in_dialog("video.recorder-preview").is_some()
                && slot().get_attribute("aria-disabled").is_none()
        })
        .await;
        run("navigator.mediaDevices.enumerateDevices = window.__enum;");
        assert_eq!(names, "Front CameraUSB Camera");
        assert_eq!(
            storage.get_item("fc.round.camera").unwrap().as_deref(),
            Some("gone")
        );
        assert!(back, "a camera that is not there gives way: {}", status());
        let _ = storage.remove_item("fc.round.camera");
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// WHEN IT CANNOT WORK AS PLANNED (S3.6): a camera another app holds is
    /// said in place of the picture, Record dimmed and a voice message
    /// offered; a picture that stays black for two seconds adds "We can't
    /// see anything…" — and Record stays usable: a dark room is no error.
    #[wasm_bindgen_test]
    async fn a_busy_camera_and_a_black_picture_are_said() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        assert!(until(5_000, || door(&root).is_some()).await);
        run("window.__gum = navigator.mediaDevices.getUserMedia; \
             navigator.mediaDevices.getUserMedia = () => Promise.reject( \
               Object.assign(new Error('busy'), {name: 'NotReadableError'}));");
        TimeoutFuture::new(650).await;
        door(&root).unwrap().click();
        let busy = until(3_000, || {
            status().contains("The camera is being used by another app.")
        })
        .await;
        let dimmed = dialog().is_some() && slot().get_attribute("aria-disabled").is_some();
        let voice = dialog().is_some_and(|dialog| {
            dialog
                .query_selector("[aria-label='Record a voice message instead']")
                .unwrap()
                .is_some()
        });
        if dialog().is_some() {
            button_labelled("Close").click();
        }
        // A camera whose picture is black: a canvas painted black, ten
        // frames a second.
        run("const c = document.createElement('canvas'); c.width = 640; c.height = 480; \
             const g = c.getContext('2d'); \
             window.__paint = setInterval(() => { g.fillStyle = '#000'; g.fillRect(0, 0, 640, 480); }, 100); \
             g.fillStyle = '#000'; g.fillRect(0, 0, 640, 480); \
             navigator.mediaDevices.getUserMedia = () => Promise.resolve(c.captureStream(10));");
        assert!(until(1_000, || dialog().is_none()).await);
        TimeoutFuture::new(650).await;
        door(&root).unwrap().click();
        let ready = until(5_000, || {
            dialog().is_some() && slot().get_attribute("aria-disabled").is_none()
        })
        .await;
        let black = until(4_000, || {
            status().contains("We can't see anything. Is the camera turned off or covered?")
        })
        .await;
        let usable = dialog().is_some() && slot().get_attribute("aria-disabled").is_none();
        run("navigator.mediaDevices.getUserMedia = window.__gum; clearInterval(window.__paint);");
        assert!(busy, "the busy camera: {}", status());
        assert!(dimmed && voice, "Record dimmed, a voice message offered");
        assert!(ready, "a black camera still shows its first frame");
        assert!(black, "the black picture: {}", status());
        assert!(usable, "Record stays usable");
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        handle.destroy();
        root.remove();
    }

    /// TEN SECONDS OR MORE ASKS (S1.1, S3.4): Delete while recording stops
    /// first and asks "Delete video message?" — Keep goes to REVIEW, Delete
    /// back to the preview; Retake asks the same. Esc while recording is
    /// Stop, never Delete. Gone with the pane — a sign-out — nothing is held.
    #[wasm_bindgen_test]
    async fn from_ten_seconds_delete_and_retake_ask_and_esc_stops() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(1_200).await;
        press(&slot(), "Escape");
        into_review().await;
        assert!(
            in_dialog(".recorder-review").is_some(),
            "Esc stopped it into REVIEW"
        );
        button_labelled("Retake").click();
        assert!(
            until(2_000, || label(&slot()) == "Record").await,
            "under ten: at once"
        );
        assert!(until(8_000, || slot().get_attribute("aria-disabled").is_none()).await);

        record_for(1_200).await;
        let ahead = ClockAhead::by(10_000.0);
        button_labelled("Delete recording").click();
        let asks = || {
            document()
                .body()
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .contains("Delete video message?")
        };
        assert!(until(10_000, asks).await, "stopped, and asked");
        assert!(
            in_dialog(".recorder-review").is_some(),
            "the recording stopped first"
        );
        click_labelled(&document().body().unwrap(), ".recorder button", "Keep");
        assert!(until(500, || !asks()).await);
        assert_eq!(label(&slot()), "Send video message", "Keep is REVIEW");
        button_labelled("Retake").click();
        assert!(until(500, asks).await, "Retake asks from ten seconds");
        click_labelled(&document().body().unwrap(), ".recorder button", "Delete");
        assert!(
            until(2_000, || label(&slot()) == "Record").await,
            "back to the preview"
        );
        drop(ahead);
        assert!(!round_video::in_progress());

        assert!(until(8_000, || slot().get_attribute("aria-disabled").is_none()).await);
        record_for(1_200).await;
        slot().click();
        into_review().await;
        assert!(round_video::in_progress());
        handle.destroy();
        TimeoutFuture::new(50).await;
        assert!(dialog().is_none(), "gone with the pane");
        assert!(!round_video::in_progress(), "and nothing held");
        assert!(sent(&log).is_empty());
        root.remove();
    }

    /// TOO BIG FOR A VIDEO MESSAGE (S3.6): over this server's ceiling, REVIEW
    /// says so, and Send sends it as a regular video — without the flag.
    #[wasm_bindgen_test]
    async fn a_clip_over_the_ceiling_goes_as_a_regular_video() {
        let root = pane_of(600, 700);
        let (log, on_action) = recorder();
        let mut small = props(on_action);
        small.round = Some(RoundLimits {
            max_ms: 60_000,
            max_bytes: 1_000,
        });
        let handle = render(&root, small);
        open(&root).await;
        record_for(1_200).await;
        slot().click();
        into_review().await;
        assert!(
            status().contains("Too big for a video message. It will be sent as a regular video."),
            "{}",
            status()
        );
        past_guard().await;
        slot().click();
        assert!(until(1_000, || dialog().is_none()).await);
        let drafts = sent(&log);
        assert_eq!(drafts.len(), 1);
        assert!(!drafts[0].round, "a regular video");
        assert_eq!(drafts[0].attachments[0].kind, "video");
        handle.destroy();
        root.remove();
    }

    /// THE CAMERA GOES AWAY mid-recording (S4): it stops into REVIEW with
    /// what was recorded, and says why. And a desktop window that loses
    /// focus closes the PREVIEW (S3.4).
    #[wasm_bindgen_test]
    async fn a_camera_gone_stops_into_review_and_a_window_away_closes_the_preview() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        open(&root).await;
        record_for(1_300).await;
        let preview: HtmlVideoElement = in_dialog("video.recorder-preview")
            .unwrap()
            .unchecked_into();
        let stream: web_sys::MediaStream = preview.src_object().unwrap().unchecked_into();
        let track = stream.get_video_tracks().get(0);
        track
            .unchecked_ref::<web_sys::EventTarget>()
            .dispatch_event(&web_sys::Event::new("ended").unwrap())
            .unwrap();
        into_review().await;
        assert!(
            status().contains("The camera is being used by another app."),
            "{}",
            status()
        );
        button_labelled("Delete").click();
        assert!(until(1_000, || dialog().is_none()).await);

        open(&root).await;
        run(
            "window.__hasFocus = document.hasFocus; document.hasFocus = () => false; \
             window.dispatchEvent(new Event('blur'));",
        );
        let closed = until(1_000, || dialog().is_none()).await;
        run("document.hasFocus = window.__hasFocus;");
        assert!(closed, "a window away closes the preview");
        handle.destroy();
        root.remove();
    }

    /// THE OTHER WAYS IN (S1.5, S1.6): the paperclip's "Record Video
    /// Message", right below "Record Voice Message", and the microphone's
    /// menu both open the recorder — the paperclip's with words typed, too:
    /// a video message travels alone, and the words stay in the box. Closing
    /// gives the focus back to the button whose menu it was.
    #[wasm_bindgen_test]
    async fn the_paperclip_and_the_microphones_menu_open_it_too() {
        let root = pane_of(600, 700);
        let (_log, on_action) = recorder();
        let handle = render(&root, props(on_action));
        assert!(until(5_000, || door(&root).is_some()).await);
        let area: web_sys::HtmlTextAreaElement = query(&root, "textarea").dyn_into().unwrap();
        let typed = |words: &str| {
            area.set_value(words);
            let init = web_sys::EventInit::new();
            init.set_bubbles(true);
            area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
                .unwrap();
        };
        typed("see you at six");
        TimeoutFuture::new(30).await;
        let paperclip = query(&root, "[aria-label='Attach']").dyn_into_html();
        let _ = paperclip.focus();
        paperclip.click();
        TimeoutFuture::new(30).await;
        let items: Vec<String> = {
            let found = root
                .query_selector_all(".attach-menu [role=menuitem]")
                .unwrap();
            (0..found.length())
                .filter_map(|index| found.item(index))
                .map(|item| item.text_content().unwrap_or_default())
                .collect()
        };
        let voice = items
            .iter()
            .position(|item| item == "Record Voice Message")
            .unwrap();
        assert_eq!(items[voice + 1], "Record Video Message", "right below");
        let item = root
            .query_selector_all(".attach-menu [role=menuitem]")
            .unwrap()
            .item(voice as u32 + 1)
            .unwrap()
            .unchecked_into::<HtmlElement>();
        let _ = item.focus();
        item.click();
        assert!(
            until(2_000, || dialog().is_some()).await,
            "the paperclip opens it"
        );
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        TimeoutFuture::new(30).await;
        assert_eq!(
            focused().as_ref(),
            Some(paperclip.unchecked_ref::<Element>()),
            "back to the paperclip"
        );
        assert_eq!(area.value(), "see you at six", "the words stay in the box");

        // The microphone's menu, with the box empty again.
        typed("");
        TimeoutFuture::new(30).await;
        let mic = query(&root, ".composer .slot");
        let init = web_sys::MouseEventInit::new();
        init.set_bubbles(true);
        init.set_cancelable(true);
        mic.dispatch_event(
            &web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap(),
        )
        .unwrap();
        TimeoutFuture::new(30).await;
        click_labelled(&root, ".slot-menu [role=menuitem]", "Record Video Message");
        assert!(
            until(2_000, || dialog().is_some()).await,
            "the menu opens it"
        );
        press(&slot(), "Escape");
        assert!(until(1_000, || dialog().is_none()).await);
        TimeoutFuture::new(30).await;
        assert_eq!(
            focused().as_ref(),
            Some(query(&root, ".composer .slot").unchecked_ref::<Element>()),
            "back to the microphone"
        );
        handle.destroy();
        root.remove();
    }

    /// A NOTIFICATION TAPPED WHILE THE RECORDER IS OPEN opens its chat once
    /// the recorder closes — never from under a clip in review (S4).
    #[wasm_bindgen_test]
    fn a_notification_waits_for_the_recorder_to_close() {
        let done = Rc::new(RefCell::new(Vec::new()));
        let at = |name: &'static str| {
            let done = done.clone();
            move || done.borrow_mut().push(name)
        };
        round_video::when_closed(at("now"));
        assert_eq!(*done.borrow(), ["now"], "nothing open: at once");
        let open = round_video::Open::now();
        round_video::when_closed(at("first"));
        round_video::when_closed(at("latest"));
        assert_eq!(done.borrow().len(), 1, "waits");
        drop(open);
        assert_eq!(*done.borrow(), ["now", "latest"], "the latest, once closed");
    }

    /// CLOSING THE TAB OVER A RECORDING OR A CLIP IN REVIEW ASKS (S4, S8.7):
    /// the `beforeunload` guard's question covers them.
    #[wasm_bindgen_test]
    fn closing_the_tab_asks_while_a_video_message_is_held() {
        let state = crate::live::AppState::default();
        assert!(!crate::leaving_loses_something(&state));
        let held = round_video::Held::new();
        assert!(
            crate::leaving_loses_something(&state),
            "recorded and not sent"
        );
        drop(held);
        assert!(!crate::leaving_loses_something(&state));
    }

    /// The shipped stylesheet in a frame `width × height` — a window that
    /// size — with `body` in it.
    async fn framed(
        width: u32,
        height: u32,
        body: &str,
    ) -> (web_sys::HtmlIFrameElement, web_sys::Document) {
        let frame: web_sys::HtmlIFrameElement = document()
            .create_element("iframe")
            .unwrap()
            .dyn_into()
            .unwrap();
        frame
            .set_attribute(
                "style",
                &format!("position:fixed;top:0;left:0;width:{width}px;height:{height}px;border:0"),
            )
            .unwrap();
        let html = format!(
            "<!doctype html><html><head><style>{}</style></head><body>{body}</body></html>",
            include_str!("../styles.css")
        );
        frame.set_attribute("srcdoc", &html).unwrap();
        document().body().unwrap().append_child(&frame).unwrap();
        assert!(
            until(3_000, || frame
                .content_document()
                .and_then(|inner| inner.query_selector(".recorder-row").ok().flatten())
                .is_some())
            .await,
            "the frame drew"
        );
        TimeoutFuture::new(30).await;
        let inner = frame.content_document().unwrap();
        (frame, inner)
    }
}

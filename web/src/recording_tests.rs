//! Today's recorder made safe — Phase 0 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md): a voice recording never runs
//! on behind a call, a hidden tab or a chat that was left, and is never lost
//! to one either. It is handed on to be kept as a "Voice message not sent"
//! (S2.8, S4), whose row sends only itself.
//!
//! Tested in a real browser with a microphone that is not one: `webdriver.json`
//! starts the test browser with Chrome's fake capture device and its
//! permission prompt already answered. Every test lets go of what it
//! recorded, so the next starts with no microphone open.

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::Element;

use crate::actions::Action;
use crate::layout_tests::{click_labelled, message, pane, props_with, query, recorder, IntoHtml};
use crate::live::{AppState, Live};
use crate::model::{Assistant, Member};
use crate::recorder::testing::ClockAhead;
use crate::recorder::{self, Handover, Listening, Recording};
use crate::staged::Prepared;
use crate::store::NotSent;
use crate::views::conversation::{Conversation, ConversationProps};

fn run(source: &str) {
    js_sys::Function::new_no_args(source)
        .call0(&JsValue::NULL)
        .expect("running a stand-in");
}

/// The tab hidden, the way a browser hides it — `document.hidden`, its
/// `visibilityState` and the event — until this is dropped.
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

fn render(root: &web_sys::HtmlElement, props: ConversationProps) -> yew::AppHandle<Conversation> {
    yew::Renderer::<Conversation>::with_root_and_props(root.clone().into(), props).render()
}

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

async fn open_attach_menu(root: &Element) {
    query(root, "[aria-label='Attach']").dyn_into_html().click();
    TimeoutFuture::new(20).await;
}

fn menu_item(root: &Element, label: &str) -> Element {
    let found = root
        .query_selector_all(".attach-menu [role=menuitem]")
        .expect("a valid selector");
    (0..found.length())
        .filter_map(|index| found.item(index))
        .filter_map(|node| node.dyn_into::<Element>().ok())
        .find(|item| crate::layout_tests::visible_text(item).trim() == label)
        .unwrap_or_else(|| panic!("the paperclip offers {label:?}"))
}

/// A recording running in the pane: chosen from the paperclip, its row up.
async fn record(root: &Element) {
    open_attach_menu(root).await;
    menu_item(root, "Record Voice Message")
        .dyn_into_html()
        .click();
    assert!(
        until(5_000, || root
            .query_selector(".recording")
            .unwrap()
            .is_some())
        .await,
        "the recording bar comes up"
    );
    assert!(recorder::in_progress());
}

/// What the pane handed on to be kept, with the reply it carried.
fn parked(log: &Rc<RefCell<Vec<Action>>>) -> Option<(Handover, Option<i64>)> {
    log.borrow().iter().find_map(|action| match action {
        Action::Park {
            chat_id: 42,
            recording,
            reply_to_message_id,
            ..
        } => Some((recording.clone(), *reply_to_message_id)),
        _ => None,
    })
}

/// Let the handed-on recording go — it must still be one, still running.
fn let_go(handover: &Handover) {
    handover
        .take()
        .expect("the recording itself, still running")
        .cancel();
    assert!(!recorder::in_progress());
}

/// The words on the media notice, without its ✕.
fn notice(root: &Element) -> String {
    root.query_selector(".media-notice")
        .unwrap()
        .and_then(|notice| notice.text_content())
        .unwrap_or_default()
        .trim_end_matches('✕')
        .to_string()
}

/// What a dimmed control carries as its reason (`aria-describedby`).
fn reason_of(root: &Element, control: &Element) -> Option<String> {
    let id = control.get_attribute("aria-describedby")?;
    root.query_selector(&format!("#{id}"))
        .ok()
        .flatten()?
        .text_content()
}

fn voice(duration_ms: i64) -> Prepared {
    Prepared {
        kind: "audio".into(),
        mime: "audio/mp4".into(),
        size: 3,
        duration_ms: Some(duration_ms),
        file: Some(web_sys::Blob::new().expect("a blob")),
        ..Prepared::default()
    }
}

fn not_sent(id: u64, duration_ms: i64, reply: Option<i64>, caption: &str) -> NotSent {
    NotSent {
        id,
        note: voice(duration_ms),
        duration_ms,
        reply_to_message_id: reply,
        caption: caption.into(),
    }
}

/// NO RECORDING DURING A CALL, in any phase (S1.7). "Record Voice Message" is
/// DIMMED — still there, still chosen, its reason carried with it — and
/// says why instead of opening the microphone (S1.3, S1.5).
#[wasm_bindgen_test]
async fn a_call_refuses_a_recording_and_says_why() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.on_call = true;
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    open_attach_menu(&root).await;
    let item = menu_item(&root, "Record Voice Message");
    assert_eq!(item.get_attribute("aria-disabled").as_deref(), Some("true"));
    assert_eq!(
        reason_of(&root, &item).as_deref(),
        Some("You can record a message after the call.")
    );
    item.dyn_into_html().click();
    // Long enough for the fake microphone to have answered, had it been asked.
    TimeoutFuture::new(500).await;
    assert_eq!(notice(&root), "You can record a message after the call.");
    assert!(root.query_selector(".recording").unwrap().is_none());
    assert!(!recorder::in_progress(), "the microphone was never opened");
    assert!(parked(&log).is_none());
    handle.destroy();
    root.remove();
}

/// A voice message that was not sent waits for its own Send or ✕ before
/// another is recorded here (S1.3 row 9, S2.8).
#[wasm_bindgen_test]
async fn a_voice_message_not_sent_dims_the_microphone() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.not_sent = vec![not_sent(3, 4_000, None, "")];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    open_attach_menu(&root).await;
    let item = menu_item(&root, "Record Voice Message");
    assert_eq!(item.get_attribute("aria-disabled").as_deref(), Some("true"));
    item.dyn_into_html().click();
    TimeoutFuture::new(500).await;
    assert_eq!(
        notice(&root),
        "Send or delete the voice message that wasn't sent first."
    );
    assert!(!recorder::in_progress());
    handle.destroy();
    root.remove();
}

/// A HIDDEN TAB stops the recording and keeps it (S4): handed on to be
/// kept as not sent — still a recording, not thrown away — and the bar
/// goes. A window that only lost focus is still on screen, and records on.
#[wasm_bindgen_test]
async fn a_hidden_tab_keeps_the_recording_as_not_sent() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    record(&root).await;
    // Past the one-second floor: under it there is nothing to keep (S4).
    let _later = ClockAhead::by(1_100.0);
    web_sys::window()
        .unwrap()
        .dispatch_event(&web_sys::Event::new("blur").unwrap())
        .unwrap();
    TimeoutFuture::new(50).await;
    assert!(parked(&log).is_none(), "focus elsewhere is not hidden");
    assert!(root.query_selector(".recording").unwrap().is_some());

    let hidden = HiddenTab::now();
    TimeoutFuture::new(50).await;
    let (handover, _) = parked(&log).expect("handed on to be kept");
    assert!(
        root.query_selector(".recording").unwrap().is_none(),
        "and the bar goes with it"
    );
    drop(hidden);
    let_go(&handover);
    handle.destroy();
    root.remove();
}

/// A microphone granted only once the tab is hidden is let go of at once:
/// the hidden tab would have stopped it, and there is nothing in it to keep.
#[wasm_bindgen_test]
async fn a_microphone_granted_into_a_hidden_tab_is_let_go_of() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    open_attach_menu(&root).await;
    menu_item(&root, "Record Voice Message")
        .dyn_into_html()
        .click();
    let hidden = HiddenTab::now();
    TimeoutFuture::new(1_000).await;
    assert!(root.query_selector(".recording").unwrap().is_none());
    assert!(!recorder::in_progress(), "the microphone is let go of");
    assert!(parked(&log).is_none(), "and there is nothing to keep");
    drop(hidden);
    handle.destroy();
    root.remove();
}

/// A microphone that answers slowly — `ms` late — for as long as this lives:
/// the browser still asking while something else happens.
struct SlowMicrophone;

impl SlowMicrophone {
    fn by(ms: u32) -> SlowMicrophone {
        run(&format!(
            "const devices = navigator.mediaDevices; \
             const real = devices.getUserMedia.bind(devices); \
             devices.getUserMedia = (asked) => new Promise((done, failed) => \
               setTimeout(() => real(asked).then(done, failed), {ms}));"
        ));
        SlowMicrophone
    }
}

impl Drop for SlowMicrophone {
    fn drop(&mut self) {
        run("delete navigator.mediaDevices.getUserMedia;");
    }
}

/// A microphone granted only once a call has begun — it rang while the
/// browser was asking — is let go of at once (S1.7, S4): there is nothing
/// in it to keep, and no recording runs during a call.
#[wasm_bindgen_test]
async fn a_microphone_granted_into_a_call_is_let_go_of() {
    let slow = SlowMicrophone::by(400);
    let root = pane();
    let (log, on_action) = recorder();
    let mut handle = render(&root, props_with(vec![message(1)], None, on_action.clone()));
    TimeoutFuture::new(50).await;
    open_attach_menu(&root).await;
    menu_item(&root, "Record Voice Message")
        .dyn_into_html()
        .click();
    let mut ringing = props_with(vec![message(1)], None, on_action);
    ringing.on_call = true;
    handle.update(ringing);
    TimeoutFuture::new(1_200).await;
    drop(slow);
    assert!(root.query_selector(".recording").unwrap().is_none());
    assert!(!recorder::in_progress(), "the microphone is let go of");
    assert!(parked(&log).is_none(), "and there is nothing to keep");
    handle.destroy();
    root.remove();
}

/// LEAVING THE CHAT keeps the recording — it is not cancelled (S2.8, S4) —
/// with the reply it was recorded under; and the words the box leaves
/// behind go with that same reply, for a note in review to take along.
#[wasm_bindgen_test]
async fn leaving_the_chat_keeps_the_recording_with_its_reply() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    query(&root, ".bubble .more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".menu [role=menuitem]", "Reply");
    TimeoutFuture::new(20).await;
    record(&root).await;
    let _later = ClockAhead::by(1_100.0);
    handle.destroy();
    TimeoutFuture::new(20).await;
    let (handover, reply) = parked(&log).expect("kept, not cancelled");
    assert_eq!(reply, Some(1), "with the reply it was recorded under");
    assert!(
        recorder::in_progress(),
        "handed on still recording, not dropped with the pane"
    );
    let words = log.borrow().iter().find_map(|action| match action {
        Action::SaveDraft {
            chat_id: 42,
            reply_to_message_id,
            ..
        } => Some(*reply_to_message_id),
        _ => None,
    });
    assert_eq!(words, Some(Some(1)), "the box's words carry the reply too");
    let_go(&handover);
    root.remove();
}

/// A CALL — ringing, placed or answered — stops the recording and keeps it
/// (S4).
#[wasm_bindgen_test]
async fn a_call_starting_keeps_the_recording() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut handle = render(&root, props_with(vec![message(1)], None, on_action.clone()));
    TimeoutFuture::new(50).await;
    record(&root).await;
    let _later = ClockAhead::by(1_100.0);
    let mut ringing = props_with(vec![message(1)], None, on_action);
    ringing.on_call = true;
    handle.update(ringing);
    TimeoutFuture::new(50).await;
    let (handover, _) = parked(&log).expect("kept");
    assert!(root.query_selector(".recording").unwrap().is_none());
    let_go(&handover);
    handle.destroy();
    root.remove();
}

/// THE CALL BUTTONS WAIT while a recording runs (S1.7): a call over a
/// recording has no right answer.
#[wasm_bindgen_test]
async fn the_call_buttons_wait_while_recording() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.calls_enabled = true;
    asked.item.chat.kind = "direct".into();
    asked.item.chat.peer_user_id = Some(9);
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    let call = || {
        let found = root.query_selector_all(".call-buttons button").unwrap();
        (0..found.length())
            .filter_map(|index| found.item(index))
            .filter_map(|node| node.dyn_into::<Element>().ok())
            .find(|button| button.text_content().unwrap_or_default() == "Call")
            .expect("the Call button")
    };
    assert!(!call().has_attribute("disabled"));
    record(&root).await;
    assert!(call().has_attribute("disabled"), "no call over a recording");
    click_labelled(&root, ".recording button", "Delete");
    TimeoutFuture::new(50).await;
    assert!(!recorder::in_progress());
    assert!(
        !call().has_attribute("disabled"),
        "and back once it is over"
    );
    handle.destroy();
    root.remove();
}

/// A MICROPHONE THAT GOES AWAY — unplugged, its permission taken back —
/// stops the recording into "not sent", and says so (S4).
#[wasm_bindgen_test]
async fn a_microphone_that_goes_away_is_kept_and_said() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    record(&root).await;
    let _later = ClockAhead::by(1_100.0);
    let track: web_sys::MediaStreamTrack = recorder::last_stream()
        .expect("the recording's microphone")
        .get_audio_tracks()
        .get(0)
        .dyn_into()
        .expect("an audio track");
    track
        .dispatch_event(&web_sys::Event::new("ended").unwrap())
        .unwrap();
    assert!(until(1_000, || parked(&log).is_some()).await, "kept");
    TimeoutFuture::new(20).await;
    assert_eq!(notice(&root), "The recording stopped unexpectedly.");
    assert!(root.query_selector(".recording").unwrap().is_none());
    let (handover, _) = parked(&log).unwrap();
    let_go(&handover);
    handle.destroy();
    root.remove();
}

/// A NOTE STOPPED AS ITS CHAT IS LEFT — Stop pressed, the chat left while
/// the note is being finished — was in review when they left: it is kept
/// as not sent (S2.8), not dropped with the pane, and never staged into a
/// pane that is gone.
#[wasm_bindgen_test]
async fn a_note_stopped_as_its_chat_is_left_is_kept() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    record(&root).await;
    TimeoutFuture::new(1_300).await;
    click_labelled(&root, ".recording button", "Stop");
    handle.destroy();
    let kept = || {
        log.borrow().iter().find_map(|action| match action {
            Action::ParkStopped {
                chat_id: 42,
                note,
                duration_ms,
                ..
            } => Some((note.kind.clone(), *duration_ms)),
            _ => None,
        })
    };
    assert!(until(5_000, || kept().is_some()).await, "kept as not sent");
    let (kind, duration_ms) = kept().unwrap();
    assert_eq!(kind, "audio");
    assert!(duration_ms >= 1_000, "{duration_ms} ms");
    assert!(log
        .borrow()
        .iter()
        .all(|action| !matches!(action, Action::Stage { .. })));
    assert!(!recorder::in_progress());
    root.remove();
}

/// A NOTE WITH NO ROOM LEFT on the strip — ten items staged already — is
/// kept as not sent rather than thrown away, for a recording cannot be made
/// again (S2.8); the notice still says why it was not staged.
#[wasm_bindgen_test]
async fn a_note_with_no_room_left_to_stage_is_kept() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.staged = (0..10)
        .map(|_| Prepared::location(1.0, 2.0, None))
        .collect();
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    record(&root).await;
    TimeoutFuture::new(1_300).await;
    // Beside what is staged, the slot is the recording's Stop (S1.3 row 3).
    query(&root, ".composer .slot[aria-label='Stop recording']")
        .dyn_into_html()
        .click();
    let kept = || {
        log.borrow().iter().any(|action| {
            matches!(
                action,
                Action::ParkStopped {
                    chat_id: 42,
                    duration_ms,
                    ..
                } if *duration_ms >= 1_000
            )
        })
    };
    assert!(until(5_000, kept).await, "kept as not sent");
    assert!(
        notice(&root).starts_with("You can attach up to 10"),
        "{}",
        notice(&root)
    );
    assert!(log
        .borrow()
        .iter()
        .all(|action| !matches!(action, Action::Stage { .. })));
    assert!(!recorder::in_progress());
    handle.destroy();
    root.remove();
}

fn sent(log: &Rc<RefCell<Vec<Action>>>) -> Vec<(u64, Vec<i64>)> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::SendNotSent { id, mentions, .. } => Some((
                *id,
                mentions.iter().map(|mention| mention.user_id).collect(),
            )),
            _ => None,
        })
        .collect()
}

fn deleted(log: &Rc<RefCell<Vec<Action>>>) -> Vec<u64> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::DeleteNotSent { id, .. } => Some(*id),
            _ => None,
        })
        .collect()
}

/// THE NOT-SENT ROW (S2.8): its length, the reply it was recorded under and
/// its caption; its own Send, which sends it alone; and its ✕ — at once
/// under ten seconds, asking first from ten, where Keep keeps it.
#[wasm_bindgen_test]
async fn the_not_sent_row_sends_alone_and_asks_before_deleting_a_long_one() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.names.insert(9, "Anna".into());
    asked.members = vec![Member {
        id: 9,
        display_name: "Anna".into(),
        username: "anna".into(),
        role: Some("member".into()),
        ..Member::default()
    }];
    asked.not_sent = vec![
        not_sent(3, 12_000, Some(1), "for you @Anna"),
        not_sent(4, 4_000, None, ""),
    ];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    let rows = root.query_selector_all(".not-sent").unwrap();
    assert_eq!(rows.length(), 2, "each its own row");
    let long: Element = rows.item(0).unwrap().dyn_into().unwrap();
    let short: Element = rows.item(1).unwrap().dyn_into().unwrap();
    assert_eq!(
        long.get_attribute("aria-label").as_deref(),
        Some("Voice message not sent · 0:12")
    );
    let said = long.text_content().unwrap_or_default();
    assert!(said.contains("Voice message not sent · 0:12"), "{said}");
    assert!(
        said.contains("Replying to Anna: Message number 1"),
        "{said}"
    );
    assert!(said.contains("for you @Anna"), "{said}");
    assert_eq!(
        short.get_attribute("aria-label").as_deref(),
        Some("Voice message not sent · 0:04")
    );
    assert!(!short
        .text_content()
        .unwrap_or_default()
        .contains("Replying"));

    // Its own Send: this one, and its caption's mentions.
    query(&long, "[aria-label='Send voice message']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    assert_eq!(sent(&log), vec![(3, vec![9])]);

    // Under ten seconds, ✕ deletes at once…
    query(&short, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    assert_eq!(deleted(&log), vec![4]);

    // …and from ten it asks, where Keep keeps it.
    query(&long, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    let dialog = query(&root, "[role=dialog]");
    assert!(dialog
        .text_content()
        .unwrap_or_default()
        .contains("Delete this recording?"));
    click_labelled(&dialog, "button", "Keep");
    TimeoutFuture::new(20).await;
    assert!(root.query_selector("[role=dialog]").unwrap().is_none());
    assert_eq!(deleted(&log), vec![4], "kept");
    query(&long, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    click_labelled(&query(&root, "[role=dialog]"), "button", "Delete");
    TimeoutFuture::new(20).await;
    assert_eq!(deleted(&log), vec![4, 3]);
    handle.destroy();
    root.remove();
}

/// A NOT-SENT MESSAGE TO THE MODEL asks first, as every message there does
/// (docs/protocol.md, "Consenting to the assistant"): its Send raises the
/// question instead of sending.
#[wasm_bindgen_test]
async fn a_not_sent_message_to_the_model_asks_first() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.item.chat.kind = "ai".into();
    asked.assistant = Some(Assistant {
        user_id: 2,
        display_name: "Assistant".into(),
        mention: Some("@ai".into()),
        draw: Some("/draw".into()),
        vision: false,
        images: false,
        processor: Some("Microsoft — Azure OpenAI".into()),
        transcribe: false,
        transcribe_max_bytes: None,
        lookups: Vec::new(),
        greeting_weather: false,
    });
    asked.agreed_to_assistant = false;
    asked.not_sent = vec![not_sent(3, 4_000, None, "")];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    assert!(root.query_selector("[role=dialog]").unwrap().is_none());
    query(&root, ".not-sent [aria-label='Send voice message']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
    assert!(sent(&log).is_empty(), "not sent unasked");
    assert!(
        root.query_selector("[role=dialog]").unwrap().is_some(),
        "the question is asked"
    );
    handle.destroy();
    root.remove();
}

/// The pane wired the way the app wires it: its actions handled by the
/// app's own `Actions` against a `Live` store, and drawn again from that
/// store whenever it changes.
#[derive(Clone)]
struct Wired {
    live: Live,
    on_action: yew::Callback<Action>,
    redraw: Rc<RefCell<Option<yew::functional::UseForceUpdateHandle>>>,
}

impl PartialEq for Wired {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.redraw, &other.redraw)
    }
}

#[derive(yew::Properties, PartialEq)]
struct HostProps {
    wired: Wired,
}

#[yew::function_component(Host)]
fn host(props: &HostProps) -> yew::Html {
    let redraw = yew::use_force_update();
    *props.wired.redraw.borrow_mut() = Some(redraw);
    let state = props.wired.live.read(|state| state.clone());
    let mut asked = props_with(vec![message(1)], None, props.wired.on_action.clone());
    asked.names.insert(9, "Anna".into());
    asked.staged = state.store.staged.get(&42).cloned().unwrap_or_default();
    asked.not_sent = state.store.not_sent.get(&42).cloned().unwrap_or_default();
    asked.session = state.session;
    yew::html! { <Conversation ..asked /> }
}

fn wired() -> Wired {
    let redraw: Rc<RefCell<Option<yew::functional::UseForceUpdateHandle>>> = Rc::default();
    let live = Live::new(
        AppState {
            token: Some("t".into()),
            ..AppState::default()
        },
        {
            let redraw = redraw.clone();
            Rc::new(move || {
                if let Some(redraw) = redraw.borrow().as_ref() {
                    redraw.force_update();
                }
            })
        },
    );
    let on_action = crate::actions::Actions {
        live: live.clone(),
        channels: Rc::new(RefCell::new(None)),
        sign_out: yew::Callback::noop(),
        last_typing: Rc::new(RefCell::new(std::collections::HashMap::new())),
        media: crate::media::MediaLoader::new(live.clone()),
        calls: crate::calls::Calls::new(live.clone(), crate::calls::Wire::new()),
    }
    .callback();
    Wired {
        live,
        on_action,
        redraw,
    }
}

/// THE WHOLE WAY ROUND, through the app's own actions and store: a hidden
/// tab keeps the recording; it comes back as its own "not sent" row in its
/// chat, quoting the reply it was recorded under, which the box no longer
/// answers; and its Send queues it ALONE, with that reply — the box's own
/// words and what is staged stay where they are (S2.8).
#[wasm_bindgen_test]
async fn a_recording_a_hidden_tab_stopped_comes_back_as_its_own_row_and_goes_alone() {
    let root = pane();
    let wired = wired();
    let live = wired.live.clone();
    wired.on_action.emit(Action::Stage {
        chat_id: 42,
        item: Prepared::location(1.0, 2.0, None),
    });
    let handle = yew::Renderer::<Host>::with_root_and_props(
        root.clone().into(),
        HostProps {
            wired: wired.clone(),
        },
    )
    .render();
    TimeoutFuture::new(50).await;
    query(&root, ".bubble .more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".menu [role=menuitem]", "Reply");
    TimeoutFuture::new(20).await;
    record(&root).await;
    TimeoutFuture::new(1_200).await;

    let hidden = HiddenTab::now();
    assert!(
        until(5_000, || root
            .query_selector(".not-sent")
            .unwrap()
            .is_some())
        .await,
        "its row comes up"
    );
    drop(hidden);
    assert!(!recorder::in_progress(), "its microphone let go of");
    let row = query(&root, ".not-sent");
    let said = row.text_content().unwrap_or_default();
    assert!(said.contains("Voice message not sent · 0:01"), "{said}");
    assert!(said.contains("Replying to Anna"), "{said}");
    assert!(
        root.query_selector(".composer-banner").unwrap().is_none(),
        "the reply went with it"
    );

    let area = query(&root, "textarea");
    area.dyn_ref::<web_sys::HtmlTextAreaElement>()
        .unwrap()
        .set_value("dinner?");
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
    TimeoutFuture::new(20).await;
    query(&row, "[aria-label='Send voice message']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(50).await;
    live.read(|state| {
        assert_eq!(state.store.outbox.len(), 1);
        let sent = &state.store.outbox[0];
        assert_eq!(sent.body, "", "no words of the box's");
        assert_eq!(sent.reply_to_message_id, Some(1), "its own reply");
        assert_eq!(sent.items.len(), 1, "alone");
        assert_eq!(sent.items[0].kind, "audio");
        assert!(!state.store.not_sent.contains_key(&42));
        assert_eq!(state.store.staged[&42].len(), 1, "the staged place stays");
    });
    assert!(root.query_selector(".not-sent").unwrap().is_none());
    assert_eq!(
        area.dyn_ref::<web_sys::HtmlTextAreaElement>()
            .unwrap()
            .value(),
        "dinner?",
        "the box keeps its words"
    );
    handle.destroy();
    root.remove();
}

/// SOMETHING STAGED IS NEVER GUARDED (S1.1), through the app's own actions
/// and store: words sent, files pasted at once — staged by the person —
/// and the Send pressed straight after sends them. The guard is for the
/// second half of a double press, and a paste in between is not one.
#[wasm_bindgen_test]
async fn a_file_staged_straight_after_a_send_goes_with_the_next_send() {
    let root = pane();
    let wired = wired();
    let live = wired.live.clone();
    let handle = yew::Renderer::<Host>::with_root_and_props(
        root.clone().into(),
        HostProps {
            wired: wired.clone(),
        },
    )
    .render();
    TimeoutFuture::new(50).await;
    let area = query(&root, "textarea");
    area.dyn_ref::<web_sys::HtmlTextAreaElement>()
        .unwrap()
        .set_value("ok");
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
    TimeoutFuture::new(20).await;
    let started = recorder::now_ms();
    query(&root, ".composer .slot").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    live.read(|state| assert_eq!(state.store.outbox.len(), 1, "the words went"));

    // Two files pasted outside the box: staged, by the person.
    run("const files = new DataTransfer(); \
         files.items.add(new File(['one'], 'one.txt', { type: 'text/plain' })); \
         files.items.add(new File(['two'], 'two.txt', { type: 'text/plain' })); \
         document.body.dispatchEvent(new ClipboardEvent('paste', \
           { clipboardData: files, bubbles: true, cancelable: true }));");
    assert!(
        until(5_000, || live.read(|state| state
            .store
            .staged
            .get(&42)
            .is_some_and(|staged| staged.len() == 2)))
        .await,
        "staged"
    );
    TimeoutFuture::new(20).await;
    // The rules' clock put back to the Send's moment: the press below comes
    // inside its guard however slow the preparing was.
    let back = ClockAhead::by(started - recorder::now_ms());
    query(&root, ".composer .slot").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    live.read(|state| {
        assert_eq!(state.store.outbox.len(), 2, "never guarded: it went");
        assert_eq!(state.store.outbox[1].items.len(), 2);
        assert!(state.store.staged.get(&42).is_none_or(Vec::is_empty));
    });
    drop(back);
    handle.destroy();
    root.remove();
}

/// An item TAKEN OFF by the person is no more guarded than one put on
/// (S1.1): a note stopped beside a staged place — the Stop's own staging,
/// which guards a second press — then the place taken off by its ✕, and the
/// Send pressed at once sends the note.
#[wasm_bindgen_test]
async fn an_item_taken_off_after_a_stop_beside_it_leaves_a_send_that_sends() {
    let root = pane();
    let wired = wired();
    let live = wired.live.clone();
    wired.on_action.emit(Action::Stage {
        chat_id: 42,
        item: Prepared::location(1.0, 2.0, None),
    });
    let handle = yew::Renderer::<Host>::with_root_and_props(
        root.clone().into(),
        HostProps {
            wired: wired.clone(),
        },
    )
    .render();
    TimeoutFuture::new(50).await;
    record(&root).await;
    TimeoutFuture::new(1_200).await;
    let started = recorder::now_ms();
    query(&root, ".composer .slot").dyn_into_html().click();
    assert!(
        until(5_000, || live.read(|state| state
            .store
            .staged
            .get(&42)
            .is_some_and(|staged| staged.len() == 2)))
        .await,
        "the note staged beside the place"
    );
    TimeoutFuture::new(20).await;
    // The rules' clock put back to the Stop's moment: inside its guard.
    let mut back = ClockAhead::by(started - recorder::now_ms());
    let place = query(&root, ".staged:not(.is-voice) .staged-remove");
    place.dyn_into_html().click();
    TimeoutFuture::new(20).await;
    back.more(started - recorder::now_ms());
    query(&root, ".composer .slot").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    live.read(|state| {
        assert_eq!(state.store.outbox.len(), 1, "never guarded: it went");
        assert_eq!(state.store.outbox[0].items.len(), 1);
        assert_eq!(state.store.outbox[0].items[0].kind, "audio");
    });
    drop(back);
    handle.destroy();
    root.remove();
}

/// THE TAB ASKS BEFORE IT CLOSES while a voice message would be lost
/// (S2.8, S4) — one that was not sent, or one being recorded — as it
/// already did for what is unsent or staged; and signing out says what
/// goes with the session.
#[wasm_bindgen_test]
async fn closing_the_tab_asks_while_a_voice_message_would_be_lost() {
    let live = Live::new(AppState::default(), Rc::new(|| {}));
    let stop_asking = crate::ask_before_leaving(live.clone());
    let asks = || {
        let window = web_sys::window().unwrap();
        let event = window
            .document()
            .unwrap()
            .create_event("BeforeUnloadEvent")
            .unwrap();
        event.init_event_with_bubbles_and_cancelable("beforeunload", false, true);
        window.dispatch_event(&event).unwrap();
        event.default_prevented()
    };
    assert!(!asks(), "nothing to lose");
    let id = live.now(|state| {
        state
            .store
            .park(42, voice(3_000), 3_000, None, String::new())
    });
    assert!(asks(), "a voice message that was not sent");
    assert!(
        crate::sign_out_message(live.read(crate::leaving_loses_something))
            .ends_with("What hasn't been sent yet is lost."),
        "signing out says it goes"
    );
    live.now(|state| state.store.take_not_sent(42, id));
    assert!(!asks());

    let recording = Recording::start(Listening::in_the_click())
        .await
        .expect("recording starts");
    assert!(asks(), "a voice message being recorded");
    recording.cancel();
    assert!(!asks());

    stop_asking();
    live.now(|state| {
        state
            .store
            .park(42, voice(3_000), 3_000, None, String::new())
    });
    assert!(!asks(), "and nothing once the question is taken away");
}

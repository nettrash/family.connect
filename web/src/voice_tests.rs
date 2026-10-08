//! A voice message from the Send slot — Phase 1 of the plan for #79
//! (docs/audio-video-messages-2026-10-04.md): the slot records hands-free and
//! sends, Stop and Esc review, Delete asks from ten seconds, the paperclip's
//! "Record Voice Message" records beside words, the five-minute limit warns
//! and reviews, the meter lights and silence is said, nothing plays into a
//! note, and a recording's ▶ plays it before it goes (S1, S2, S6, S8.7,
//! S8.8).
//!
//! Tested in a real browser against the shipped stylesheet, with a
//! microphone that is not one: `webdriver.json` starts the test browser with
//! Chrome's fake capture device and its permission prompt already answered.
//! A recording that must be a real note — long enough to be more than the
//! recorders' 1024-byte floor — runs for over a second of real time; the
//! rest moves the recorder's own clock instead (`recorder::testing`).

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_test::*;
use web_sys::{Element, HtmlElement, HtmlMediaElement, HtmlTextAreaElement};

use crate::actions::Action;
use crate::layout_tests::{click_labelled, message, pane, props_with, query, recorder, IntoHtml};
use crate::recorder::{
    self,
    testing::{ClockAhead, StopsHeld},
};
use crate::staged::Prepared;
use crate::store::NotSent;
use crate::views::conversation::{Conversation, ConversationProps};

fn run(source: &str) -> JsValue {
    js_sys::Function::new_no_args(source)
        .call0(&JsValue::NULL)
        .expect("running a stand-in")
}

fn render(root: &HtmlElement, props: ConversationProps) -> yew::AppHandle<Conversation> {
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

fn slot(root: &Element) -> HtmlElement {
    query(root, ".composer .slot").dyn_into_html()
}

fn label(element: &Element) -> String {
    element.get_attribute("aria-label").unwrap_or_default()
}

fn row(root: &Element) -> Option<Element> {
    root.query_selector(".composer .recording").unwrap()
}

/// What the announcement node says (S6).
fn said(root: &Element) -> String {
    query(root, ".conversation > .visually-hidden[aria-live='polite']")
        .text_content()
        .unwrap_or_default()
}

/// The words on the notice line, without its ✕.
fn notice(root: &Element) -> String {
    root.query_selector(".media-notice")
        .unwrap()
        .and_then(|notice| notice.text_content())
        .unwrap_or_default()
        .trim_end_matches('✕')
        .to_string()
}

fn focused() -> Option<Element> {
    web_sys::window()?.document()?.active_element()
}

fn is_focused(element: &Element) -> bool {
    focused().as_ref() == Some(element)
}

fn textarea(root: &Element) -> HtmlTextAreaElement {
    query(root, "textarea").dyn_into().unwrap()
}

fn type_into(root: &Element, words: &str) {
    let area = textarea(root);
    area.set_value(words);
    let init = web_sys::EventInit::new();
    init.set_bubbles(true);
    area.dispatch_event(&web_sys::Event::new_with_event_init_dict("input", &init).unwrap())
        .unwrap();
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
}

/// Click the slot and wait for the recording row.
async fn start_from_the_slot(root: &Element) {
    slot(root).click();
    assert!(
        until(5_000, || row(root).is_some()).await,
        "the recording row comes up"
    );
    assert!(recorder::in_progress());
}

async fn record_from_the_paperclip(root: &Element) {
    query(root, "[aria-label='Attach']").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    click_labelled(root, ".attach-menu [role=menuitem]", "Record Voice Message");
    assert!(
        until(5_000, || row(root).is_some()).await,
        "the recording row comes up"
    );
}

/// A little over the activation guard (S1.1), for a test that starts a new
/// recording straight after the last.
fn record_guard_ms() -> u32 {
    fc_text::record::ACTIVATION_GUARD_MS as u32 + 100
}

async fn microphone_let_go() -> bool {
    until(5_000, || !recorder::in_progress()).await
}

fn staged(log: &Rc<RefCell<Vec<Action>>>) -> Vec<Prepared> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::Stage { chat_id: 42, item } => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn sent(log: &Rc<RefCell<Vec<Action>>>) -> Vec<(Prepared, Option<i64>)> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::SendRecorded {
                chat_id: 42,
                note,
                reply_to_message_id,
                ..
            } => Some((note.clone(), *reply_to_message_id)),
            _ => None,
        })
        .collect()
}

fn parked(log: &Rc<RefCell<Vec<Action>>>) -> bool {
    log.borrow()
        .iter()
        .any(|action| matches!(action, Action::Park { .. } | Action::ParkStopped { .. }))
}

/// A reply primed on message 1, the way a person primes one.
async fn reply_to_the_first(root: &Element) {
    query(root, ".bubble .more").dyn_into_html().click();
    TimeoutFuture::new(20).await;
    click_labelled(root, ".menu [role=menuitem]", "Reply");
    TimeoutFuture::new(20).await;
}

/// THE ISSUE'S "JUST BY CLICKING" (S1.3, S2.2, S2.4, S2.5): the empty box's
/// slot is the microphone; a click records hands-free — the recording row in
/// the field's place, the slot the Send arrow and focused, "Recording" said,
/// the typing line quiet — and the same slot sends: alone, at once, with the
/// reply the box was answering. "Voice message sent" is said, and the cursor
/// is back in the box, where a second Enter cannot record again.
#[wasm_bindgen_test]
async fn a_click_records_hands_free_and_the_same_slot_sends_it() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.names.insert(9, "Anna".into());
    asked.typing = vec!["Anna".into()];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    reply_to_the_first(&root).await;
    assert_eq!(label(&slot(&root)), "Record voice message");
    assert_eq!(
        query(&root, ".typing")
            .get_attribute("aria-live")
            .as_deref(),
        Some("polite")
    );

    start_from_the_slot(&root).await;
    TimeoutFuture::new(50).await;
    let arrow = slot(&root);
    assert_eq!(label(&arrow), "Send voice message");
    assert!(
        !arrow.has_attribute("disabled"),
        "never disabled while it records"
    );
    assert!(is_focused(&arrow), "focus on the slot");
    assert_eq!(said(&root), "Recording");
    assert_eq!(
        query(&root, ".typing")
            .get_attribute("aria-live")
            .as_deref(),
        Some("off"),
        "the typing line quiet: never spoken into a note"
    );
    assert!(
        query(&root, ".composer")
            .query_selector("[aria-label='Attach']")
            .unwrap()
            .is_none(),
        "the row has the paperclip's place too"
    );
    assert!(textarea(&root).hidden());
    assert!(
        root.query_selector(".composer-banner").unwrap().is_some(),
        "the reply banner above stays"
    );

    TimeoutFuture::new(1_200).await;
    arrow.click();
    assert!(until(5_000, || !sent(&log).is_empty()).await, "sent");
    let notes = sent(&log);
    assert_eq!(notes.len(), 1);
    let (note, reply) = &notes[0];
    assert_eq!(
        (note.kind.as_str(), note.mime.as_str()),
        ("audio", "audio/mp4")
    );
    assert!(note.duration_ms.unwrap_or(0) >= 1_000);
    assert_eq!(*reply, Some(1), "with the reply the box was answering");
    assert!(staged(&log).is_empty(), "sent, not staged");
    assert!(row(&root).is_none());
    assert_eq!(said(&root), "Voice message sent");
    assert!(
        root.query_selector(".composer-banner").unwrap().is_none(),
        "the reply went with it"
    );
    assert!(is_focused(&textarea(&root)), "the cursor back in the box");
    assert_eq!(label(&slot(&root)), "Record voice message");
    assert!(microphone_let_go().await);

    // Return in the empty box never records (S1.3) — asked once the
    // slot's own guard, from the Send above, has run out.
    TimeoutFuture::new(record_guard_ms()).await;
    press(&textarea(&root), "Enter");
    TimeoutFuture::new(500).await;
    assert!(row(&root).is_none());
    assert!(!recorder::in_progress());
    handle.destroy();
    root.remove();
}

/// THE ACTIVATION GUARD AND THE ONE-SECOND FLOOR (S1.1, S2.5): a second click
/// at once neither sends nor stops what the first started; a Send under a
/// second sends nothing and says "That recording was too short."; and a
/// double click on a text Send cannot start a recording.
#[wasm_bindgen_test]
async fn a_double_click_does_nothing_twice_and_under_a_second_is_too_short() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert!(
        row(&root).is_some(),
        "the second click is held by the guard"
    );
    assert!(sent(&log).is_empty());

    // Past the guard, under a second.
    TimeoutFuture::new(650).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert!(row(&root).is_none());
    assert_eq!(notice(&root), "That recording was too short.");
    assert!(microphone_let_go().await);
    TimeoutFuture::new(200).await;
    assert!(
        sent(&log).is_empty() && staged(&log).is_empty(),
        "nothing kept"
    );
    assert!(is_focused(&textarea(&root)));

    // Words sent by the slot: the microphone it becomes waits out the guard
    // — the Send's own, not one still running from the recording above.
    TimeoutFuture::new(record_guard_ms()).await;
    type_into(&root, "dinner?");
    TimeoutFuture::new(20).await;
    slot(&root).click();
    TimeoutFuture::new(20).await;
    assert!(log
        .borrow()
        .iter()
        .any(|action| matches!(action, Action::Send { .. })));
    assert_eq!(label(&slot(&root)), "Record voice message");
    slot(&root).click();
    TimeoutFuture::new(500).await;
    assert!(
        row(&root).is_none(),
        "a double click on Send records nothing"
    );
    assert!(!recorder::in_progress());
    handle.destroy();
    root.remove();
}

/// STOP AND ESC REVIEW (S2.4, S2.5, S8.7): the recording row's Stop — and Esc,
/// which is Stop now, never a cancel — stops the note into the chip above the
/// box, says "Ready to review, 0:01", and gives the box back for a caption.
#[wasm_bindgen_test]
async fn stop_and_esc_put_the_note_in_review() {
    for by in ["Stop", "Esc"] {
        let root = pane();
        let (log, on_action) = recorder();
        let handle = render(&root, props_with(vec![message(1)], None, on_action));
        TimeoutFuture::new(50).await;
        start_from_the_slot(&root).await;
        TimeoutFuture::new(1_200).await;
        if by == "Stop" {
            query(&root, ".recording button[aria-label='Stop recording']")
                .dyn_into_html()
                .click();
        } else {
            press(&slot(&root), "Escape");
        }
        assert!(
            until(5_000, || !staged(&log).is_empty()).await,
            "{by}: staged"
        );
        let note = &staged(&log)[0];
        assert_eq!(note.kind, "audio", "{by}");
        assert!(sent(&log).is_empty(), "{by}: never sent");
        assert!(!parked(&log), "{by}: never kept as not sent");
        assert!(row(&root).is_none());
        assert!(
            said(&root).starts_with("Ready to review, 0:01"),
            "{by}: {}",
            said(&root)
        );
        assert!(is_focused(&textarea(&root)), "{by}: the box, for a caption");
        assert!(microphone_let_go().await);
        handle.destroy();
        root.remove();
    }
}

/// SAID ONCE IT HAPPENED (S2.5, S6): the note is finished after the click
/// that sends or stops it, and that finishing can still come to "too short"
/// or an error — so "Voice message sent" is never said before the note is
/// handed to the outbox, nor "Ready to review" before it is in the chip.
/// Watched a tick at a time from the click until the note lands.
#[wasm_bindgen_test]
async fn sent_and_ready_to_review_are_said_only_once_they_happened() {
    for (by, words) in [("Send", "Voice message sent"), ("Stop", "Ready to review")] {
        let root = pane();
        let (log, on_action) = recorder();
        let handle = render(&root, props_with(vec![message(1)], None, on_action));
        TimeoutFuture::new(50).await;
        start_from_the_slot(&root).await;
        TimeoutFuture::new(1_200).await;
        if by == "Send" {
            slot(&root).click();
        } else {
            query(&root, ".recording button[aria-label='Stop recording']")
                .dyn_into_html()
                .click();
        }
        let landed = |log: &Rc<RefCell<Vec<Action>>>| {
            if by == "Send" {
                !sent(log).is_empty()
            } else {
                !staged(log).is_empty()
            }
        };
        let mut early = false;
        let mut ticks = 0;
        while !landed(&log) && ticks < 2_500 {
            TimeoutFuture::new(1).await;
            ticks += 1;
            if said(&root).starts_with(words) && !landed(&log) {
                early = true;
            }
        }
        assert!(landed(&log), "{by}: the note landed");
        assert!(!early, "{by}: \"{words}\" was said before it happened");
        assert!(
            until(2_000, || said(&root).starts_with(words)).await,
            "{by}: said once it happened: {}",
            said(&root)
        );
        assert!(microphone_let_go().await);
        handle.destroy();
        root.remove();
    }
}

/// DELETE (S2.5): under ten seconds it deletes at once and says so; from ten
/// the recording STOPS first — its microphone off — and "Delete this
/// recording?" asks: Keep reviews it, Delete deletes it, and Esc keeps.
#[wasm_bindgen_test]
async fn delete_is_at_once_under_ten_seconds_and_asks_from_ten() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    TimeoutFuture::new(300).await;
    click_labelled(&root, ".recording button", "Delete");
    TimeoutFuture::new(50).await;
    assert!(row(&root).is_none());
    assert!(root.query_selector("[role=dialog]").unwrap().is_none());
    assert_eq!(said(&root), "Recording deleted");
    assert!(microphone_let_go().await);
    assert!(is_focused(&textarea(&root)));

    for (answer, keeps) in [("Keep", true), ("Escape", true), ("Delete", false)] {
        let before = staged(&log).len();
        // The microphone's own guard, from the start before, has run out.
        TimeoutFuture::new(record_guard_ms()).await;
        start_from_the_slot(&root).await;
        TimeoutFuture::new(1_200).await;
        let later = ClockAhead::by(10_000.0);
        click_labelled(&root, ".recording button", "Delete");
        TimeoutFuture::new(50).await;
        let dialog = query(&root, "[role=dialog]");
        assert!(dialog
            .text_content()
            .unwrap_or_default()
            .contains("Delete this recording?"));
        assert!(row(&root).is_none(), "{answer}: stopped first");
        assert!(microphone_let_go().await, "{answer}: its microphone off");
        match answer {
            "Escape" => press(&dialog, "Escape"),
            _ => click_labelled(&dialog, "button", answer),
        }
        drop(later);
        TimeoutFuture::new(50).await;
        assert!(root.query_selector("[role=dialog]").unwrap().is_none());
        if keeps {
            assert!(
                until(5_000, || staged(&log).len() == before + 1).await,
                "{answer}: kept, in review"
            );
            assert!(
                said(&root).starts_with("Ready to review, 0:1"),
                "{}",
                said(&root)
            );
        } else {
            TimeoutFuture::new(500).await;
            assert_eq!(staged(&log).len(), before, "deleted");
            assert_eq!(said(&root), "Recording deleted");
        }
        assert!(is_focused(&textarea(&root)), "{answer}");
    }
    assert!(sent(&log).is_empty() && !parked(&log));
    handle.destroy();
    root.remove();
}

fn sent_words(log: &Rc<RefCell<Vec<Action>>>) -> bool {
    log.borrow()
        .iter()
        .any(|action| matches!(action, Action::Send { .. }))
}

/// "RECORD VOICE MESSAGE" BESIDE WORDS (S1.3 row 3, S1.5, S2.4): the words are
/// hidden behind the row and must not leave unseen, so the slot is Stop, the
/// row draws no Stop of its own, and Stop stages the note beside them. A
/// second press at once sends nothing (S1.1) — and nor does a Send pressed
/// after the guard while the note is still being finished, which for a
/// long one takes seconds: the box is busy until the note has landed beside
/// the words, which must not leave without it.
#[wasm_bindgen_test]
async fn recorded_beside_words_the_slot_is_stop_and_stages_the_note() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    type_into(&root, "for you");
    TimeoutFuture::new(20).await;
    record_from_the_paperclip(&root).await;
    TimeoutFuture::new(50).await;
    let square = slot(&root);
    assert_eq!(label(&square), "Stop recording");
    assert!(root.query_selector(".recording-stop").unwrap().is_none());
    assert!(textarea(&root).hidden());
    TimeoutFuture::new(1_200).await;

    // The note takes as long to finish as a long one does.
    let held = StopsHeld::new();
    square.click();
    // A second press at once sends nothing (S1.1).
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert!(!sent_words(&log), "the guard holds a double press");
    let area = textarea(&root);
    assert!(!area.hidden());
    assert_eq!(area.value(), "for you", "the words are still the box's");
    assert!(is_focused(&area));

    // The guard over, the note still being finished: Send waits for it.
    let later = ClockAhead::by(1_000.0);
    let send = slot(&root);
    assert_eq!(label(&send), "Send");
    assert!(
        send.has_attribute("disabled"),
        "busy until the note has landed beside the words"
    );
    send.click();
    press(&area, "Enter");
    TimeoutFuture::new(50).await;
    assert!(!sent_words(&log), "the words never leave without the note");
    assert!(staged(&log).is_empty(), "still being finished");

    drop(held);
    assert!(until(5_000, || !staged(&log).is_empty()).await);
    TimeoutFuture::new(20).await;
    assert!(sent(&log).is_empty(), "never sent beside words");
    assert!(!sent_words(&log));
    assert!(
        !slot(&root).has_attribute("disabled"),
        "landed: Send sends both"
    );
    drop(later);
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// A CHANGE THE PERSON MADE IS NEVER GUARDED (S1.1): "ok" sent, then "x"
/// typed and deleted at once, and the microphone that brought back records
/// — the guard is for the second half of a double press, and this is none.
#[wasm_bindgen_test]
async fn words_typed_and_deleted_after_a_send_leave_a_microphone_that_records() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    type_into(&root, "ok");
    TimeoutFuture::new(20).await;
    let started = recorder::now_ms();
    slot(&root).click();
    TimeoutFuture::new(20).await;
    assert!(sent_words(&log));
    assert_eq!(label(&slot(&root)), "Record voice message");
    type_into(&root, "x");
    TimeoutFuture::new(20).await;
    type_into(&root, "");
    TimeoutFuture::new(20).await;
    assert_eq!(label(&slot(&root)), "Record voice message");
    // The rules' clock put back to the Send's moment: the press below comes
    // inside its guard however slow the browser was.
    let back = ClockAhead::by(started - recorder::now_ms());
    slot(&root).click();
    assert!(
        until(5_000, || row(&root).is_some()).await,
        "never guarded: it records"
    );
    TimeoutFuture::new(record_guard_ms()).await;
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    drop(back);
    handle.destroy();
    root.remove();
}

/// FIVE MINUTES (S2.5, S2.9): at 4:30 the time turns orange and "30 seconds
/// left" is shown in the meter's place and said; at 5:00 the recorder stops
/// itself INTO REVIEW — never sent — with "Recording stopped at five
/// minutes."
#[wasm_bindgen_test]
async fn the_limit_warns_at_four_thirty_and_reviews_at_five() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    TimeoutFuture::new(1_200).await;
    assert!(root.query_selector(".recording-left").unwrap().is_none());
    let mut later = ClockAhead::by(270_000.0);
    assert!(
        until(1_000, || root
            .query_selector(".recording-left")
            .unwrap()
            .is_some())
        .await,
        "the warning shows"
    );
    assert_eq!(
        query(&root, ".recording-left").text_content().as_deref(),
        Some("30 seconds left")
    );
    assert!(query(&root, ".recording-time")
        .class_list()
        .contains("is-warning"));
    assert!(
        root.query_selector(".recording-meter").unwrap().is_none(),
        "in the meter's place"
    );
    assert_eq!(said(&root), "30 seconds left");
    assert!(sent(&log).is_empty() && staged(&log).is_empty());

    later.more(30_000.0);
    assert!(
        until(5_000, || !staged(&log).is_empty()).await,
        "into review"
    );
    drop(later);
    assert!(sent(&log).is_empty(), "never sent by the limit");
    assert_eq!(notice(&root), "Recording stopped at five minutes.");
    // Said by the announcement node (S6) — shown on the line, and hidden
    // there from screen readers, so it is said once.
    assert_eq!(said(&root), "Recording stopped at five minutes.");
    assert_eq!(
        shown_not_said(&query(&root, ".media-notice")).as_deref(),
        Some("Recording stopped at five minutes.")
    );
    assert!(row(&root).is_none());
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// A microphone that is not one, for as long as this lives: a tone through a
/// gain the test turns up and down — 0 is digital silence, a muted microphone
/// (S2.9).
struct Microphone;

impl Microphone {
    fn at(gain: f64) -> Microphone {
        run(&format!(
            "const context = new AudioContext(); \
             const tone = context.createOscillator(); tone.frequency.value = 440; \
             const gain = context.createGain(); gain.gain.value = {gain}; \
             const out = context.createMediaStreamDestination(); \
             tone.connect(gain); gain.connect(out); tone.start(); \
             window.__fcMicrophone = {{ context, gain }}; \
             navigator.mediaDevices.getUserMedia = () => Promise.resolve(out.stream);"
        ));
        Microphone
    }

    fn turn(&self, gain: f64) {
        run(&format!(
            "window.__fcMicrophone.gain.gain.setValueAtTime({gain}, \
             window.__fcMicrophone.context.currentTime);"
        ));
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        run("delete navigator.mediaDevices.getUserMedia; \
             window.__fcMicrophone.context.close(); delete window.__fcMicrophone;");
    }
}

/// The newest bar of the live waveform — its level, 0 to 15 — and how many
/// bars there are.
fn newest(root: &Element) -> (u8, u32) {
    let bars = root
        .query_selector_all(".recording-meter > i")
        .expect("a valid selector");
    let level = bars
        .item(bars.length().saturating_sub(1))
        .and_then(|bar| bar.dyn_into::<Element>().ok())
        .and_then(|bar| bar.get_attribute("data-level"))
        .and_then(|level| level.parse().ok())
        .unwrap_or(0);
    (level, bars.length())
}

/// THE LIVE WAVEFORM AND THE SILENCE WARNING (S2.9, the approved design):
/// the peak the tap hears scrolls in from the right as bars on the sent
/// waveform's own scale; three seconds of nothing above digital silence says
/// "We can't hear anything. Is the microphone muted?" — shown, said, the
/// notice line itself quiet while it records — and the line goes when sound
/// arrives. The recording goes on throughout.
#[wasm_bindgen_test]
async fn the_meter_lights_and_silence_is_said_until_sound_arrives() {
    let microphone = Microphone::at(0.0);
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    assert!(
        root.query_selector(".recording-meter").unwrap().is_some(),
        "the live waveform"
    );
    assert!(
        until(1_000, || newest(&root).1 >= 2).await,
        "it scrolls in: {} bars",
        newest(&root).1
    );
    TimeoutFuture::new(500).await;
    assert_eq!(newest(&root).0, 0, "silence stands at nothing");
    assert!(notice(&root).is_empty(), "not before three seconds");

    let later = ClockAhead::by(3_000.0);
    let silence = "We can't hear anything. Is the microphone muted?";
    assert!(until(1_000, || notice(&root) == silence).await, "shown");
    assert_eq!(said(&root), silence, "and said");
    let line = query(&root, ".media-notice");
    assert_eq!(line.get_attribute("aria-live").as_deref(), Some("off"));
    assert!(
        line.get_attribute("role").is_none(),
        "the line quiet while it records"
    );
    assert!(row(&root).is_some(), "the recording goes on");

    microphone.turn(0.5);
    assert!(
        until(2_000, || notice(&root).is_empty()).await,
        "gone once sound arrives"
    );
    assert!(
        until(2_000, || newest(&root).0 >= 12).await,
        "a loud tone stands tall: level {}",
        newest(&root).0
    );
    // The strip keeps as many bars as fill IT — on this 600 px pane more
    // than a phone's 40, so it is never half empty on a wide window — and
    // never more.
    let strip = query(&root, ".recording-meter");
    let width = strip.get_bounding_client_rect().width();
    let keep: usize = strip
        .get_attribute("data-keep")
        .and_then(|keep| keep.parse().ok())
        .unwrap_or(0);
    assert_eq!(
        keep,
        crate::views::attach::live_bars_for(width),
        "the strip's own width: {width} px"
    );
    assert!(
        keep > crate::views::attach::LIVE_BARS,
        "{keep} for {width} px"
    );
    assert!(
        newest(&root).1 as usize <= keep,
        "never more than the row keeps"
    );
    drop(later);
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
    drop(microphone);
}

/// THE BROWSER'S OWN RECORDER hands nothing over until it stops: it draws the
/// dot only — no meter — and says nothing of silence it cannot hear (S2.9).
#[wasm_bindgen_test]
async fn the_browsers_own_recorder_draws_no_meter_and_warns_of_no_silence() {
    let _no_encoder = crate::webcodecs::testing::refusing("AudioEncoder");
    let _quiet = Microphone::at(0.0);
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    assert!(root.query_selector(".recording-dot").unwrap().is_some());
    assert!(root.query_selector(".recording-meter").unwrap().is_none());
    let later = ClockAhead::by(3_500.0);
    TimeoutFuture::new(600).await;
    assert!(notice(&root).is_empty());
    drop(later);
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// THE ASSISTANT'S CHAT gets no microphone — its slot is today's disabled
/// Send — and its paperclip no "Record Voice Message" (S1.3 row 6, S1.5).
#[wasm_bindgen_test]
async fn the_assistants_chat_has_no_microphone() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.item.chat.kind = "ai".into();
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    let send = slot(&root);
    assert_eq!(label(&send), "Send");
    assert!(send.has_attribute("disabled"));
    query(&root, "[aria-label='Attach']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    let items = root
        .query_selector_all(".attach-menu [role=menuitem]")
        .unwrap();
    let labels: Vec<String> = (0..items.length())
        .filter_map(|index| items.item(index)?.dyn_into::<Element>().ok())
        .map(|item| crate::layout_tests::visible_text(&item))
        .collect();
    assert!(!labels.is_empty());
    assert!(
        !labels.iter().any(|item| item.contains("Record")),
        "{labels:?}"
    );
    handle.destroy();
    root.remove();
}

/// A DIMMED MICROPHONE SAYS WHY (S1.3 rows 7 and 9): during a call, and while
/// a voice message that was not sent waits — and records nothing.
#[wasm_bindgen_test]
async fn a_dimmed_microphone_says_why_and_records_nothing() {
    for (on_call, words) in [
        (true, "You can record a message after the call."),
        (
            false,
            "Send or delete the voice message that wasn't sent first.",
        ),
    ] {
        let root = pane();
        let (_log, on_action) = recorder();
        let mut asked = props_with(vec![message(1)], None, on_action);
        asked.on_call = on_call;
        if !on_call {
            asked.not_sent = vec![not_sent(3, voice_wav(2))];
        }
        let handle = render(&root, asked);
        TimeoutFuture::new(50).await;
        let microphone = slot(&root);
        assert_eq!(
            microphone.get_attribute("aria-disabled").as_deref(),
            Some("true")
        );
        microphone.click();
        TimeoutFuture::new(500).await;
        assert_eq!(notice(&root), words);
        assert!(row(&root).is_none());
        assert!(!recorder::in_progress(), "the microphone never opened");
        handle.destroy();
        root.remove();
    }
}

/// A tone of `seconds`, as a 16 kHz WAV voice note recorded here.
fn voice_wav(seconds: u32) -> Prepared {
    let rate = fc_text::wav::VOICE_RATE;
    let samples: Vec<f32> = (0..seconds * rate)
        .map(|at| (at as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 0.3)
        .collect();
    let bytes = fc_text::wav::encode(&samples, rate);
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes.as_slice()));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("audio/wav");
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).unwrap();
    Prepared {
        kind: "audio".into(),
        mime: "audio/wav".into(),
        size: blob.size() as i64,
        duration_ms: Some(i64::from(seconds) * 1_000),
        file: Some(blob),
        ..Prepared::default()
    }
}

fn not_sent(id: u64, note: Prepared) -> NotSent {
    let duration_ms = note.duration_ms.unwrap_or(0);
    NotSent {
        id,
        note,
        duration_ms,
        reply_to_message_id: None,
        caption: String::new(),
    }
}

fn player_of(at: &Element) -> HtmlMediaElement {
    at.query_selector("audio")
        .unwrap()
        .expect("its player")
        .dyn_into()
        .unwrap()
}

/// `window.matchMedia` answering "a phone's browser" — a coarse pointer —
/// for as long as this lives.
struct CoarsePointer;

impl CoarsePointer {
    fn now() -> CoarsePointer {
        run("window.__fcMatchMedia = window.matchMedia; \
             window.matchMedia = (query) => ({ matches: query === '(pointer: coarse)', \
               media: query, addListener() {}, removeListener() {}, \
               addEventListener() {}, removeEventListener() {} });");
        CoarsePointer
    }
}

impl Drop for CoarsePointer {
    fn drop(&mut self) {
        run("window.matchMedia = window.__fcMatchMedia; delete window.__fcMatchMedia;");
    }
}

/// REVIEW CAN BE LISTENED TO (S2.7): "[▶] Voice message · 0:02 [✕]", and while
/// it plays "[❚❚] 0:00 / 0:02" — the LOCAL bytes, and on a phone's browser the
/// screen kept on while it plays (S1.7). ✕, "Delete recording", removes it at
/// once under ten seconds and asks from ten.
#[wasm_bindgen_test]
async fn a_staged_note_plays_and_its_cross_asks_from_ten_seconds() {
    let screen = crate::awake::testing::FakeWakeLock::install();
    let _phone = CoarsePointer::now();
    let root = pane();
    let (log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action.clone());
    asked.staged = vec![voice_wav(2)];
    let mut handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    let chip = query(&root, ".staged.is-voice");
    assert_eq!(
        query(&chip, ".staged-label").text_content().as_deref(),
        Some("Voice message · 0:02")
    );
    // Something else playing — a bubble's voice note, say.
    let document = web_sys::window().unwrap().document().unwrap();
    let other: HtmlMediaElement = document
        .create_element("audio")
        .unwrap()
        .dyn_into()
        .unwrap();
    let url =
        web_sys::Url::create_object_url_with_blob(voice_wav(3).file.as_ref().unwrap()).unwrap();
    other.set_src(&url);
    root.append_child(&other).unwrap();
    let _ = other.play();
    assert!(until(2_000, || !other.paused()).await);

    let play = query(&chip, ".local-play").dyn_into_html();
    assert_eq!(label(&play), "Play");
    play.click();
    let player = player_of(&chip);
    assert!(until(2_000, || !player.paused()).await, "it plays");
    assert!(other.paused(), "one thing plays at a time");
    other.remove();
    let _ = web_sys::Url::revoke_object_url(&url);
    TimeoutFuture::new(100).await;
    assert_eq!(label(&query(&chip, ".local-play")), "Pause");
    let playing = query(&chip, ".staged-label")
        .text_content()
        .unwrap_or_default();
    assert!(playing.ends_with(" / 0:02"), "{playing}");
    assert_eq!(screen.asked(), 1, "the screen kept on while it plays");
    query(&chip, ".local-play").dyn_into_html().click();
    assert!(until(1_000, || player.paused()).await, "and pauses");
    TimeoutFuture::new(50).await;
    assert_eq!(screen.released(), 1);
    assert_eq!(
        query(&chip, ".staged-label").text_content().as_deref(),
        Some("Voice message · 0:02")
    );

    query(&chip, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    assert!(root.query_selector("[role=dialog]").unwrap().is_none());
    let unstaged = || {
        log.borrow()
            .iter()
            .filter(|action| {
                matches!(
                    action,
                    Action::Unstage {
                        chat_id: 42,
                        index: 0
                    }
                )
            })
            .count()
    };
    assert_eq!(unstaged(), 1, "under ten seconds: at once");

    let mut long = props_with(vec![message(1)], None, on_action);
    long.staged = vec![voice_wav(12)];
    handle.update(long);
    TimeoutFuture::new(50).await;
    let chip = query(&root, ".staged.is-voice");
    query(&chip, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    click_labelled(&query(&root, "[role=dialog]"), "button", "Keep");
    TimeoutFuture::new(20).await;
    assert_eq!(unstaged(), 1, "Keep keeps it");
    query(&chip, "[aria-label='Delete recording']")
        .dyn_into_html()
        .click();
    TimeoutFuture::new(20).await;
    click_labelled(&query(&root, "[role=dialog]"), "button", "Delete");
    TimeoutFuture::new(20).await;
    assert_eq!(unstaged(), 2);
    handle.destroy();
    root.remove();
}

/// The not-sent row's ▶ plays the note it keeps (S2.8).
#[wasm_bindgen_test]
async fn a_voice_message_not_sent_can_be_listened_to() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.not_sent = vec![not_sent(3, voice_wav(2))];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    let row = query(&root, ".not-sent");
    query(&row, ".local-play").dyn_into_html().click();
    let player = player_of(&row);
    assert!(until(2_000, || !player.paused()).await, "it plays");
    query(&row, ".local-play").dyn_into_html().click();
    assert!(until(1_000, || player.paused()).await);
    handle.destroy();
    root.remove();
}

/// NOTHING OF THE APP'S PLAYS INTO A NOTE (S1.7, S2.2): a recording starting
/// pauses whatever plays; while it runs a recording's ▶ is dimmed and says
/// "You can play this after recording." instead of playing; and anything
/// else that starts — a bubble's voice note — is paused at once and says so.
#[wasm_bindgen_test]
async fn while_recording_nothing_of_the_apps_plays() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut asked = props_with(vec![message(1)], None, on_action);
    asked.staged = vec![voice_wav(3)];
    let handle = render(&root, asked);
    TimeoutFuture::new(50).await;
    // Something playing already: the staged note.
    let chip = query(&root, ".staged.is-voice");
    query(&chip, ".local-play").dyn_into_html().click();
    let staged_player = player_of(&chip);
    assert!(until(2_000, || !staged_player.paused()).await);
    // …and another player on the page, as a bubble's is.
    let document = web_sys::window().unwrap().document().unwrap();
    let other: HtmlMediaElement = document
        .create_element("audio")
        .unwrap()
        .dyn_into()
        .unwrap();
    let url =
        web_sys::Url::create_object_url_with_blob(voice_wav(3).file.as_ref().unwrap()).unwrap();
    other.set_src(&url);
    root.append_child(&other).unwrap();

    // With words in the box, Record Voice Message records beside the staged
    // note — row 3, as the plan has it.
    record_from_the_paperclip(&root).await;
    assert!(
        until(1_000, || staged_player.paused()).await,
        "a recording pauses what plays"
    );

    let play = query(&chip, ".local-play");
    assert_eq!(play.get_attribute("aria-disabled").as_deref(), Some("true"));
    // Not even started and stopped again: never asked to play at all.
    let starts = Rc::new(std::cell::Cell::new(0));
    let counted = {
        let starts = starts.clone();
        wasm_bindgen::closure::Closure::<dyn Fn()>::new(move || starts.set(starts.get() + 1))
    };
    staged_player
        .add_event_listener_with_callback("play", counted.as_ref().unchecked_ref())
        .unwrap();
    play.dyn_into_html().click();
    TimeoutFuture::new(300).await;
    staged_player
        .remove_event_listener_with_callback("play", counted.as_ref().unchecked_ref())
        .unwrap();
    assert_eq!(starts.get(), 0, "a dimmed ▶ does not play");
    assert!(staged_player.paused(), "it does not play");
    assert_eq!(notice(&root), "You can play this after recording.");
    query(&root, ".media-notice .link").dyn_into_html().click();
    TimeoutFuture::new(20).await;

    let _ = other.play();
    TimeoutFuture::new(300).await;
    assert!(other.paused(), "paused at once");
    assert_eq!(notice(&root), "You can play this after recording.");

    slot(&root).click();
    assert!(microphone_let_go().await);
    TimeoutFuture::new(100).await;
    // Over: it plays again.
    let _ = other.play();
    assert!(
        until(2_000, || !other.paused()).await,
        "it plays once the recording is over"
    );
    let _ = other.pause();
    other.remove();
    let _ = web_sys::Url::revoke_object_url(&url);
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

/// UNDER A SECOND an interruption leaves nothing worth keeping (S4): the
/// recording is deleted, nothing is handed on to be kept, and the microphone
/// is let go of.
#[wasm_bindgen_test]
async fn an_interruption_under_a_second_keeps_nothing() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    let hidden = HiddenTab::now();
    TimeoutFuture::new(50).await;
    assert!(row(&root).is_none());
    assert!(microphone_let_go().await);
    drop(hidden);
    TimeoutFuture::new(200).await;
    assert!(!parked(&log), "nothing to keep");
    assert!(staged(&log).is_empty() && sent(&log).is_empty());
    handle.destroy();
    root.remove();
}

/// A THREAD'S BOX IS NOT CHANGED (S1.3): no microphone, today's Send.
#[wasm_bindgen_test]
async fn a_threads_box_keeps_todays_send() {
    use crate::views::thread_panel::{ThreadPanel, ThreadPanelProps};
    let root = pane();
    let props = ThreadPanelProps {
        chat_id: 42,
        root_id: 1,
        messages: vec![message(1)],
        my_user_id: 7,
        is_family_chat: true,
        is_ai_chat: false,
        names: Default::default(),
        members: Vec::new(),
        assistant: None,
        blocked: Default::default(),
        revealed: Default::default(),
        revealed_quotes: Default::default(),
        failed: Default::default(),
        ai_failed: Default::default(),
        family: None,
        stickers: None,
        agreed_to_assistant: true,
        transcripts: Default::default(),
        on_action: yew::Callback::noop(),
    };
    let handle =
        yew::Renderer::<ThreadPanel>::with_root_and_props(root.clone().into(), props).render();
    TimeoutFuture::new(50).await;
    assert!(
        root.query_selector(".slot").unwrap().is_none(),
        "no microphone"
    );
    let send = query(&root, ".composer > button");
    assert_eq!(send.text_content().as_deref(), Some("Send"));
    assert!(send.has_attribute("disabled"));
    handle.destroy();
    root.remove();
}

/// THE ROW IS THE COMPOSER'S OWN ROW (S2.4): the same height recording or not,
/// so the newest messages never slide under it.
#[wasm_bindgen_test]
async fn the_recording_row_keeps_the_composers_height() {
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    let height = || {
        query(&root, ".composer")
            .get_bounding_client_rect()
            .height()
    };
    let idle = height();
    start_from_the_slot(&root).await;
    TimeoutFuture::new(50).await;
    let recording = height();
    assert!((recording - idle).abs() <= 1.0, "{recording} vs {idle}");
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// THE MICROPHONE'S MENU, end to end (S1.6): a right-click on the empty box's
/// microphone, "Record Voice Message", and a hands-free recording — the slot
/// the Send arrow, as for a click.
#[wasm_bindgen_test]
async fn the_microphones_menu_records_hands_free() {
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    let init = web_sys::MouseEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_button(2);
    slot(&root)
        .dispatch_event(
            &web_sys::MouseEvent::new_with_mouse_event_init_dict("contextmenu", &init).unwrap(),
        )
        .unwrap();
    TimeoutFuture::new(20).await;
    click_labelled(&root, ".slot-menu [role=menuitem]", "Record Voice Message");
    assert!(until(5_000, || row(&root).is_some()).await, "it records");
    assert_eq!(label(&slot(&root)), "Send voice message");
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// A microphone that answers `ms` late, for as long as this lives: the
/// browser still asking while something else happens.
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

/// A microphone granted only after the recording it was asked for was
/// overtaken — the tab hidden and shown again while the browser asked, so
/// that neither a call nor a hidden tab is left to tell — is let go of: the
/// rules put the recording back to nothing, and a microphone they do not
/// know about would be one left on, with no row to stop it (S4).
#[wasm_bindgen_test]
async fn a_microphone_granted_after_the_asking_was_overtaken_is_let_go_of() {
    let slow = SlowMicrophone::by(400);
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    drop(HiddenTab::now());
    TimeoutFuture::new(1_200).await;
    drop(slow);
    assert!(row(&root).is_none());
    assert!(!recorder::in_progress(), "the microphone is let go of");
    assert!(!parked(&log), "and there is nothing to keep");
    handle.destroy();
    root.remove();
}

// --- What the checker of #79 found (the plan's S1.1, S2.4, S4, S6) ---------------

/// A control outside the pane holding the focus — a ringing call's Accept, a
/// viewer's Close — until it is dropped.
struct Elsewhere(HtmlElement);

impl Elsewhere {
    fn focused(words: &str) -> Elsewhere {
        let document = web_sys::window().unwrap().document().unwrap();
        let button: HtmlElement = document
            .create_element("button")
            .unwrap()
            .dyn_into()
            .unwrap();
        button.set_text_content(Some(words));
        document.body().unwrap().append_child(&button).unwrap();
        button.focus().unwrap();
        Elsewhere(button)
    }

    fn has_focus(&self) -> bool {
        is_focused(&self.0)
    }
}

impl Drop for Elsewhere {
    fn drop(&mut self) {
        self.0.remove();
    }
}

/// The words of every text the pane sent.
fn sends(log: &Rc<RefCell<Vec<Action>>>) -> Vec<String> {
    log.borrow()
        .iter()
        .filter_map(|action| match action {
            Action::Send { draft, .. } => Some(draft.body.clone()),
            _ => None,
        })
        .collect()
}

/// The reply a recording handed on to be kept carried, if one was handed on.
fn parked_reply(log: &Rc<RefCell<Vec<Action>>>) -> Option<Option<i64>> {
    log.borrow().iter().find_map(|action| match action {
        Action::Park {
            reply_to_message_id,
            ..
        } => Some(*reply_to_message_id),
        _ => None,
    })
}

/// Let go of every recording the pane handed on to be kept.
fn let_go_of_the_parked(log: &Rc<RefCell<Vec<Action>>>) {
    for action in log.borrow().iter() {
        if let Action::Park { recording, .. } = action {
            if let Some(running) = recording.take() {
                running.cancel();
            }
        }
    }
}

/// A pointer event at the middle of `at`.
fn pointer(at: &Element, kind: &str, name: &str) {
    let rect = at.get_bounding_client_rect();
    let init = web_sys::PointerEventInit::new();
    init.set_pointer_type(kind);
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_client_x((rect.left() + rect.width() / 2.0) as i32);
    init.set_client_y((rect.top() + rect.height() / 2.0) as i32);
    at.dispatch_event(&web_sys::PointerEvent::new_with_event_init_dict(name, &init).unwrap())
        .unwrap();
}

/// A slow press on the slot: down now, up 700 ms later — past the 600 ms
/// guard — the way `how` makes one. What comes back holds the clock there.
fn slow_press(root: &Element, how: &str) -> ClockAhead {
    let at = slot(root);
    let later = match how {
        "mouse" | "mouse, no pointerup" => {
            pointer(&at, "mouse", "pointerdown");
            let later = ClockAhead::by(700.0);
            if how == "mouse" {
                pointer(&at, "mouse", "pointerup");
            }
            // The click a press that comes up on a button ends in.
            at.click();
            later
        }
        "touch" => {
            pointer(&at, "touch", "pointerdown");
            let later = ClockAhead::by(700.0);
            pointer(&at, "touch", "pointerup");
            // …and the click a phone's browser may send after it.
            at.click();
            later
        }
        "space" => {
            press(&at, " ");
            let later = ClockAhead::by(700.0);
            // Space clicks as it comes up.
            at.click();
            later
        }
        _ => unreachable!("{how}"),
    };
    later
}

/// A CALL RINGING OVER A RECORDING is "stop → not sent", and nothing else
/// (S4). The call's band put the focus on its Accept as it rang — "Return
/// answers it" — and it stays there: the box does not take it back, where
/// Return would send the words waiting in it, which nobody can take back.
#[wasm_bindgen_test]
async fn a_call_over_a_recording_leaves_the_focus_on_its_accept() {
    let root = pane();
    let (log, on_action) = recorder();
    let mut handle = render(&root, props_with(vec![message(1)], None, on_action.clone()));
    TimeoutFuture::new(50).await;
    type_into(&root, "dinner at seven, don't tell");
    TimeoutFuture::new(20).await;
    record_from_the_paperclip(&root).await;
    let later = ClockAhead::by(1_500.0);

    let accept = Elsewhere::focused("Accept");
    let mut ringing = props_with(vec![message(1)], None, on_action);
    ringing.on_call = true;
    handle.update(ringing);
    TimeoutFuture::new(200).await;
    assert!(row(&root).is_none(), "the recording stopped");
    assert!(parked(&log), "and is kept as not sent");
    assert!(accept.has_focus(), "the focus is still on Accept");
    assert!(!is_focused(&textarea(&root)));

    // Return, as the call invites — wherever the focus is.
    press(&focused().expect("the focus somewhere"), "Enter");
    TimeoutFuture::new(100).await;
    assert!(sends(&log).is_empty(), "Return sent nothing");
    assert_eq!(textarea(&root).value(), "dinner at seven, don't tell");

    drop(later);
    let_go_of_the_parked(&log);
    assert!(microphone_let_go().await);
    drop(accept);
    handle.destroy();
    root.remove();
}

/// AN INTERRUPTION THAT CARRIES THE REPLY OFF leaves the focus alone too (S4):
/// the reply leaving the box is no reason to put the cursor in it — not even
/// after an earlier recording, sent by the person, gave it back once.
#[wasm_bindgen_test]
async fn an_interruption_that_takes_the_reply_leaves_the_focus_alone() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    TimeoutFuture::new(1_200).await;
    slot(&root).click();
    assert!(until(5_000, || !sent(&log).is_empty()).await, "sent");
    assert!(is_focused(&textarea(&root)), "the person's Send: the box");
    assert!(microphone_let_go().await);
    TimeoutFuture::new(record_guard_ms()).await;

    reply_to_the_first(&root).await;
    start_from_the_slot(&root).await;
    let later = ClockAhead::by(1_100.0);
    let elsewhere = Elsewhere::focused("Close");
    let hidden = HiddenTab::now();
    TimeoutFuture::new(200).await;
    assert!(row(&root).is_none(), "the recording stopped");
    assert_eq!(parked_reply(&log), Some(Some(1)), "kept, with its reply");
    assert!(elsewhere.has_focus(), "the focus where it was");

    drop(hidden);
    drop(later);
    let_go_of_the_parked(&log);
    assert!(microphone_let_go().await);
    drop(elsewhere);
    handle.destroy();
    root.remove();
}

/// THE FIVE-MINUTE LIMIT takes nothing from the person (S1.7, S2.7): the note
/// goes to review, and the cursor comes back to the box — for a caption —
/// only if the focus was still the composer's; from wherever the person had
/// put it meanwhile, it is not taken.
#[wasm_bindgen_test]
async fn the_limit_gives_the_box_the_focus_only_if_the_composer_had_it() {
    for away in [true, false] {
        let root = pane();
        let (log, on_action) = recorder();
        let handle = render(&root, props_with(vec![message(1)], None, on_action));
        TimeoutFuture::new(50).await;
        start_from_the_slot(&root).await;
        TimeoutFuture::new(1_200).await;
        assert!(
            is_focused(&slot(&root)),
            "focus on the slot while it records"
        );
        let elsewhere = away.then(|| Elsewhere::focused("Close"));
        let later = ClockAhead::by(300_000.0);
        assert!(
            until(5_000, || !staged(&log).is_empty()).await,
            "into review"
        );
        drop(later);
        TimeoutFuture::new(50).await;
        match &elsewhere {
            Some(elsewhere) => assert!(elsewhere.has_focus(), "left where the person put it"),
            None => assert!(is_focused(&textarea(&root)), "back in the box"),
        }
        assert!(microphone_let_go().await);
        drop(elsewhere);
        handle.destroy();
        root.remove();
    }
}

/// THE GUARD IS ASKED WHEN THE PRESS GOES DOWN (S1.1, Decision 15): a press
/// that goes down while it runs is ignored WHOLE, however long after it
/// comes up — so a slow double press on Send cannot start a recording, nor a
/// slow second press on the microphone end the recording it started. A press
/// that went down after the guard is a click like any other, and one that
/// came up off the slot is no click on it.
#[wasm_bindgen_test]
async fn a_press_that_goes_down_inside_the_guard_is_ignored_whole() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;

    // Words sent by the slot: the microphone it becomes waits out the guard.
    for (index, how) in ["mouse", "mouse, no pointerup", "touch", "space"]
        .into_iter()
        .enumerate()
    {
        type_into(&root, "hi");
        TimeoutFuture::new(20).await;
        slot(&root).click();
        TimeoutFuture::new(20).await;
        assert_eq!(sends(&log).len(), index + 1, "{how}: the words went");
        assert_eq!(label(&slot(&root)), "Record voice message");
        let later = slow_press(&root, how);
        TimeoutFuture::new(500).await;
        assert!(row(&root).is_none(), "{how}: nothing records");
        assert!(!recorder::in_progress(), "{how}: no microphone");
        drop(later);
    }

    // A recording just started: a second press at once, up after the guard,
    // neither sends it nor finds it too short.
    TimeoutFuture::new(record_guard_ms()).await;
    start_from_the_slot(&root).await;
    let later = slow_press(&root, "mouse");
    TimeoutFuture::new(300).await;
    assert!(row(&root).is_some(), "still recording");
    assert!(
        notice(&root).is_empty(),
        "nothing too short: {}",
        notice(&root)
    );
    assert!(sent(&log).is_empty());
    drop(later);
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);

    // A slow press that goes down after the guard records.
    TimeoutFuture::new(record_guard_ms()).await;
    let later = slow_press(&root, "mouse");
    assert!(
        until(5_000, || row(&root).is_some()).await,
        "a slow press records"
    );
    click_labelled(&root, ".recording button", "Delete");
    drop(later);
    assert!(microphone_let_go().await);

    // Down inside the guard, up off the slot: no click on it. A later
    // activation that comes with no press of its own — an assistive
    // technology's — is not taken for that one.
    TimeoutFuture::new(record_guard_ms()).await;
    type_into(&root, "and you?");
    TimeoutFuture::new(20).await;
    slot(&root).click();
    TimeoutFuture::new(20).await;
    assert_eq!(sends(&log).len(), 5);
    pointer(&slot(&root), "mouse", "pointerdown");
    pointer(&query(&root, ".messages"), "mouse", "pointerup");
    let later = ClockAhead::by(700.0);
    slot(&root).click();
    assert!(
        until(5_000, || row(&root).is_some()).await,
        "the later activation records"
    );
    click_labelled(&root, ".recording button", "Delete");
    drop(later);
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

/// WHILE THE MICROPHONE IS ASKED FOR the slot is a microphone that ignores
/// clicks — and a press that goes down then is ignored whole too: come up
/// once the recording runs and its guard is over, it neither sends the
/// recording nor finds it too short.
#[wasm_bindgen_test]
async fn a_press_that_goes_down_while_the_microphone_is_asked_for_is_ignored_whole() {
    let slow = SlowMicrophone::by(400);
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert!(row(&root).is_none(), "still asking");
    pointer(&slot(&root), "mouse", "pointerdown");
    assert!(
        until(5_000, || row(&root).is_some()).await,
        "the recording runs"
    );
    let later = ClockAhead::by(700.0);
    pointer(&slot(&root), "mouse", "pointerup");
    slot(&root).click();
    TimeoutFuture::new(300).await;
    assert!(row(&root).is_some(), "still recording");
    assert!(
        notice(&root).is_empty(),
        "nothing too short: {}",
        notice(&root)
    );
    assert!(sent(&log).is_empty());
    drop(later);
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    drop(slow);
    handle.destroy();
    root.remove();
}

/// "THAT RECORDING WAS TOO SHORT." IS SAID (S6, S7) — by the announcement
/// node, which is always there, and not left to the notice line, drawn with
/// the words already in it, which a screen reader may never read. The line
/// shows them hidden from screen readers, so they are said once, and stays a
/// live region: a sentence the node does not say is the line's to say.
#[wasm_bindgen_test]
async fn too_short_is_said_by_the_announcement_node_once() {
    let root = pane();
    let (_log, on_action) = recorder();
    let mut handle = render(&root, props_with(vec![message(1)], None, on_action.clone()));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    assert_eq!(said(&root), "Recording");
    TimeoutFuture::new(650).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert_eq!(said(&root), "That recording was too short.");
    assert_eq!(notice(&root), "That recording was too short.");
    let line = query(&root, ".media-notice");
    assert_eq!(
        line.get_attribute("role").as_deref(),
        Some("status"),
        "the line still a live region, for what it says next"
    );
    assert_eq!(
        shown_not_said(&line).as_deref(),
        Some("That recording was too short."),
        "shown there, not said twice"
    );
    assert!(microphone_let_go().await);

    let mut ringing = props_with(vec![message(1)], None, on_action);
    ringing.on_call = true;
    handle.update(ringing);
    TimeoutFuture::new(record_guard_ms()).await;
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert_eq!(notice(&root), "You can record a message after the call.");
    let line = query(&root, ".media-notice");
    assert_eq!(line.get_attribute("role").as_deref(), Some("status"));
    assert!(line.get_attribute("aria-live").is_none());
    assert_eq!(shown_not_said(&line), None, "said by the line itself");
    handle.destroy();
    root.remove();
}

/// The words a line shows but hides from screen readers, if any.
fn shown_not_said(line: &Element) -> Option<String> {
    line.query_selector("[aria-hidden='true']")
        .unwrap()
        .and_then(|hidden| hidden.text_content())
}

/// The conversation with the app's quiet around it, the way main.rs draws
/// it, beside one of the app's own live regions.
#[derive(yew::Properties, PartialEq)]
struct QuietHostProps {
    on_action: yew::Callback<Action>,
}

#[yew::function_component(QuietHost)]
fn quiet_host(props: &QuietHostProps) -> yew::Html {
    use crate::views::quiet::{LiveRegion, QuietRoot};
    use yew::html;
    let mut asked = props_with(
        vec![
            message(1),
            crate::model::Message {
                id: 0,
                client_msg_id: Some("c0".into()),
                ..message(2)
            },
        ],
        None,
        props.on_action.clone(),
    );
    asked
        .failed
        .insert("c0".into(), "Not sent: no connection.".into());
    html! {
        <QuietRoot>
            <LiveRegion tag="span" class="status" role="status">{ "Connecting…" }</LiveRegion>
            <Conversation ..asked />
        </QuietRoot>
    }
}

/// WHILE A RECORDING RUNS, THE APP'S OTHER LIVE REGIONS ARE QUIET (S6): a
/// send that failed, in the chat, and the app's own bar say nothing into the
/// note — still shown — and speak again once it is over.
#[wasm_bindgen_test]
async fn while_recording_the_apps_other_live_regions_are_quiet() {
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = yew::Renderer::<QuietHost>::with_root_and_props(
        root.clone().into(),
        QuietHostProps { on_action },
    )
    .render();
    TimeoutFuture::new(50).await;
    let roles = || {
        let failed = query(&root, ".send-failed");
        let bar = query(&root, ".status");
        (
            failed.get_attribute("role"),
            failed.get_attribute("aria-live"),
            bar.get_attribute("role"),
            bar.get_attribute("aria-live"),
        )
    };
    assert_eq!(
        roles(),
        (Some("alert".into()), None, Some("status".into()), None)
    );
    start_from_the_slot(&root).await;
    TimeoutFuture::new(50).await;
    assert_eq!(
        roles(),
        (None, Some("off".into()), None, Some("off".into())),
        "quiet while it records"
    );
    assert!(query(&root, ".send-failed")
        .text_content()
        .unwrap_or_default()
        .contains("Not sent: no connection."));
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    TimeoutFuture::new(50).await;
    assert_eq!(
        roles(),
        (Some("alert".into()), None, Some("status".into()), None),
        "and loud again"
    );
    handle.destroy();
    root.remove();
}

/// The conversation with the app's quiet around it, a received voice note
/// in it.
#[yew::function_component(QuietVoiceHost)]
fn quiet_voice_host(props: &QuietHostProps) -> yew::Html {
    use crate::views::quiet::QuietRoot;
    use yew::html;
    let asked = props_with(
        vec![crate::model::Message {
            attachments: Some(vec![crate::model::Attachment {
                id: 33,
                kind: "audio".into(),
                mime: Some("audio/mp4".into()),
                duration_ms: Some(4_000),
                ..crate::model::Attachment::default()
            }]),
            ..message(1)
        }],
        None,
        props.on_action.clone(),
    );
    html! {
        <QuietRoot>
            <Conversation ..asked />
        </QuietRoot>
    }
}

/// A RECEIVED VOICE NOTE'S ▶ IS DIMMED WHILE A RECORDING RUNS (S1.7, S6):
/// with the reason beside it for a screen reader, and pressed, it says
/// "You can play this after recording." on the composer's notice line
/// instead of playing.
#[wasm_bindgen_test]
async fn while_recording_a_received_voice_notes_play_is_dimmed_and_says_why() {
    let root = pane();
    let (_log, on_action) = recorder();
    let handle = yew::Renderer::<QuietVoiceHost>::with_root_and_props(
        root.clone().into(),
        QuietHostProps { on_action },
    )
    .render();
    TimeoutFuture::new(50).await;
    let toggle = query(&root, ".bubble .audio-toggle");
    assert!(toggle.get_attribute("aria-disabled").is_none());
    start_from_the_slot(&root).await;
    TimeoutFuture::new(50).await;
    let toggle = query(&root, ".bubble .audio-toggle");
    assert_eq!(
        toggle.get_attribute("aria-disabled").as_deref(),
        Some("true")
    );
    assert!(!toggle.has_attribute("disabled"));
    toggle.dyn_into_html().click();
    TimeoutFuture::new(50).await;
    assert_eq!(notice(&root), "You can play this after recording.");
    assert_eq!(
        query(&root, ".bubble .audio-toggle")
            .get_attribute("aria-label")
            .as_deref(),
        Some("Play"),
        "nothing asked for"
    );
    click_labelled(&root, ".recording button", "Delete");
    assert!(microphone_let_go().await);
    TimeoutFuture::new(50).await;
    assert!(query(&root, ".bubble .audio-toggle")
        .get_attribute("aria-disabled")
        .is_none());
    handle.destroy();
    root.remove();
}

/// A NOTE SENT GOES BEFORE WORDS TYPED WHILE IT IS FINISHED (S2.5): the Send
/// arrow hands the note to the outbox once it is finished, which for a long
/// one takes seconds; words typed meanwhile wait — the box is busy — rather
/// than overtake it.
#[wasm_bindgen_test]
async fn a_note_sent_goes_before_words_typed_while_it_is_finished() {
    let root = pane();
    let (log, on_action) = recorder();
    let handle = render(&root, props_with(vec![message(1)], None, on_action));
    TimeoutFuture::new(50).await;
    start_from_the_slot(&root).await;
    TimeoutFuture::new(1_200).await;
    let held = StopsHeld::new();
    slot(&root).click();
    TimeoutFuture::new(50).await;
    assert!(row(&root).is_none());
    type_into(&root, "and dinner?");
    TimeoutFuture::new(20).await;
    let later = ClockAhead::by(1_000.0);
    assert_eq!(label(&slot(&root)), "Send");
    assert!(slot(&root).has_attribute("disabled"), "busy");
    press(&textarea(&root), "Enter");
    TimeoutFuture::new(50).await;
    assert!(!sent_words(&log), "the words wait");
    drop(held);
    assert!(
        until(5_000, || !sent(&log).is_empty()).await,
        "the note went"
    );
    TimeoutFuture::new(20).await;
    press(&textarea(&root), "Enter");
    TimeoutFuture::new(50).await;
    let order: Vec<&str> = log
        .borrow()
        .iter()
        .filter_map(|action| match action {
            Action::SendRecorded { .. } => Some("note"),
            Action::Send { .. } => Some("words"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["note", "words"]);
    drop(later);
    assert!(microphone_let_go().await);
    handle.destroy();
    root.remove();
}

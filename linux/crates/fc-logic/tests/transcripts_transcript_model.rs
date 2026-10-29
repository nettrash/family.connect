//! `TranscriptModelTests`: "Show text", from the click to the text under the player
//! (docs/protocol.md, "Transcripts on request") — the consent question in the backdrop's order, the
//! request, the answer kept on this device, and what is drawn while it all runs.
//!
//! The C#'s `WaitUntil` polling and `TaskCompletionSource` answers are a `LocalPool` run until it
//! stalls and a [`Gate`] the held answer waits on; the C#'s maker that THROWS is one that answers
//! `Err(AttemptFailure)` (a Rust closure cannot throw); the multipart body is fc-core's, which
//! quotes its parameters (`name="audio"`) where .NET's did not (`name=audio`) — both RFC 7578.
//! The last test is Linux's own: a click dropped part-way (the C#'s `finally`).

mod common;

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::{Ready, ready};
use std::rc::Rc;

use fc_core::protocol::{ApiError, error_codes};
use fc_core::rt::LocalBoxFuture;
use fc_core::store::{Database, KeptTranscript, StoreResult, TranscriptStore};
use fc_core::testing::{FakeClock, drive};
use fc_logic::transcode_attempts::AttemptFailure;
use fc_logic::transcript_sound::{self, SoundSource, SuppliedSound, TranscriptForm};
use fc_logic::transcripts::{TranscriptLook, TranscriptModel, TranscriptPhase, transcript_rules};
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use time::macros::datetime;

use common::Gate;
use common::server::{Server, api};

const PATH: &str = "/chats/42/messages/1338/attachments/34/transcript";
const PATH_35: &str = "/chats/42/messages/1338/attachments/35/transcript";
const SAID: &str = r#"{"transcript": {"text": "Мы будем в шесть", "language": "ru"}}"#;
const NOT_AGREED: &str =
    r#"{"error": {"code": "assistant_consent_required", "message": "agree first"}}"#;

const M4A: [u8; 15] = [
    0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'M', b'4', b'A', b' ', 1, 2, 3,
];

/// The cache, the clock the client and the model both read (`DateTimeOffset.UnixEpoch`), and the
/// attachment ids the model said changed.
struct Rig {
    clock: FakeClock,
    store: Rc<TranscriptStore>,
    changed: Rc<RefCell<Vec<i64>>>,
}

impl Rig {
    fn new() -> StoreResult<Self> {
        let cache = Database::open_in_memory()?;
        Ok(Rig {
            clock: FakeClock::at(datetime!(1970-01-01 00:00 UTC)),
            store: Rc::new(TranscriptStore::new(&cache)),
            changed: Rc::default(),
        })
    }

    fn model(&self, server: &Server) -> Rc<TranscriptModel> {
        let model = Rc::new(TranscriptModel::new(
            api(server, &self.clock),
            Rc::clone(&self.store),
            Rc::new(self.clock.clone()),
        ));
        let changed = Rc::clone(&self.changed);
        model
            .on_changed()
            .subscribe(move |id| changed.borrow_mut().push(*id));
        model
    }

    fn find(&self, attachment_id: i64) -> StoreResult<Option<KeptTranscript>> {
        self.store.find(attachment_id)
    }

    /// `ShowAsync(model, asksFirst, ask: () => true)` for attachment 34, the stored form.
    fn show(&self, model: &TranscriptModel, asks_first: bool) -> StoreResult<()> {
        drive(
            &self.clock,
            model.show_stored(42, 1338, 34, move || asks_first, || ready(true)),
        )
    }

    /// `SupplyAsync(model, maker, form, asksFirst, ask)`.
    fn supply(
        &self,
        model: &TranscriptModel,
        attachment_id: i64,
        form: TranscriptForm,
        sound: Option<SoundSource>,
        asks_first: bool,
        answer: bool,
    ) -> StoreResult<()> {
        drive(
            &self.clock,
            model.show(
                42,
                1338,
                attachment_id,
                form,
                sound,
                move || asks_first,
                move || ready(answer),
            ),
        )
    }
}

fn kept(attachment_id: i64, text: &str, language: Option<&str>, supplied: bool) -> KeptTranscript {
    KeptTranscript {
        attachment_id,
        text: text.to_string(),
        language: language.map(str::to_string),
        supplied,
    }
}

/// A consent question that must never be asked (the C#'s `() => throw`).
fn never_asked() -> Ready<bool> {
    panic!("never asked")
}

/// `Maker`: a maker that counts its calls and hands back what it is told to.
struct Maker {
    calls: Rc<Cell<usize>>,
    made: Rc<dyn Fn() -> Result<SuppliedSound, AttemptFailure>>,
    max_bytes: i64,
    duration_ms: Option<i64>,
}

impl Maker {
    fn new(made: impl Fn() -> Result<SuppliedSound, AttemptFailure> + 'static) -> Self {
        Maker {
            calls: Rc::default(),
            made: Rc::new(made),
            max_bytes: 26_214_400,
            duration_ms: None,
        }
    }

    fn making(bytes: &[u8]) -> Self {
        let bytes = bytes.to_vec();
        Maker::new(move || Ok(SuppliedSound::Made(bytes.clone())))
    }

    fn with_max_bytes(mut self, max_bytes: i64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    fn with_duration(mut self, duration_ms: i64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    fn calls(&self) -> usize {
        self.calls.get()
    }

    fn source(&self) -> Option<SoundSource> {
        let calls = Rc::clone(&self.calls);
        let made = Rc::clone(&self.made);
        Some(SoundSource::new(
            self.max_bytes,
            move |_cancel| -> LocalBoxFuture<'static, Result<SuppliedSound, AttemptFailure>> {
                calls.set(calls.get() + 1);
                let answer = made();
                Box::pin(async move { answer })
            },
            self.duration_ms,
        ))
    }
}

/// A route held until `gate` opens, then answering `json` with 200.
fn held(server: &Server, path: &str, gate: &Gate, json: &'static str) {
    let gate = gate.clone();
    server.on_async(path, move || {
        let gate = gate.clone();
        Box::pin(async move {
            gate.wait().await;
            Ok((200, Some(json.to_string())))
        })
    });
}

#[test]
fn the_answer_is_shown_and_kept_on_this_device() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    assert_eq!(TranscriptLook::closed(), model.look(34));
    assert!(!model.holds(34)?);

    rig.show(&model, false)?;

    assert_eq!(TranscriptLook::open("Мы будем в шесть"), model.look(34));
    assert_eq!(
        Some(kept(34, "Мы будем в шесть", Some("ru"), false)),
        rig.find(34)?
    );
    assert!(model.holds(34)?);
    // The stored form: no body at all.
    assert_eq!(vec![format!("/api/v1{PATH}")], server.asked());
    assert!(server.bodies().is_empty());
    // "Getting the text…" was drawn before the answer, then the answer.
    assert_eq!(vec![34, 34], *rig.changed.borrow());
    Ok(())
}

/// KEPT MEANS NOT ASKED AGAIN: folded away and shown again — or shown by a model built after a
/// restart, over the same cache — the text comes from this device and no request goes anywhere.
#[test]
fn hide_and_show_again_asks_nobody() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    rig.show(&model, false)?;

    model.hide(34);
    assert_eq!(TranscriptLook::closed(), model.look(34));
    assert!(rig.find(34)?.is_some());
    rig.show(&model, false)?;
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);

    let reopened = rig.model(&server);
    assert!(reopened.holds(34)?);
    drive(
        &rig.clock,
        reopened.show_stored(42, 1338, 34, || true, never_asked),
    )?;
    assert_eq!(Some("Мы будем в шесть"), reopened.look(34).text.as_deref());
    assert_eq!(1, server.asked().len());
    Ok(())
}

/// SILENCE IS AN ANSWER: kept, and drawn as "No speech" — not as a failure, and not asked about
/// again.
#[test]
fn silence_is_kept_as_an_answer() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, r#"{"transcript": {"text": ""}}"#);
    let model = rig.model(&server);

    rig.show(&model, false)?;

    assert!(model.look(34).no_speech());
    assert_eq!("", rig.find(34)?.expect("kept").text);
    model.hide(34);
    rig.show(&model, false)?;
    assert_eq!(1, server.asked().len());
    Ok(())
}

/// ASKED FIRST when `/me` says no — and on Not Now nothing is sent and nothing is said.
#[test]
fn not_now_sends_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let asked = Cell::new(0);

    drive(
        &rig.clock,
        model.show_stored(
            42,
            1338,
            34,
            || true,
            || {
                asked.set(asked.get() + 1);
                ready(false)
            },
        ),
    )?;

    assert_eq!(1, asked.get());
    assert!(server.asked().is_empty());
    assert_eq!(TranscriptLook::closed(), model.look(34));
    assert_eq!(None, rig.find(34)?);
    Ok(())
}

/// ASKED FIRST, and on a yes the request goes.
#[test]
fn a_yes_sends_it() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);

    rig.show(&model, true)?;

    assert_eq!(1, server.asked().len());
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    Ok(())
}

/// ASKED AFTER: the server says agree first although this device thought it had — the question,
/// then the request again, ONCE; the board's backdrop does exactly this.
#[test]
fn the_servers_agree_first_asks_and_then_asks_again_once() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then(PATH, &[(403, Some(NOT_AGREED)), (200, Some(SAID))]);
    let model = rig.model(&server);
    let asked = Cell::new(0);

    drive(
        &rig.clock,
        model.show_stored(
            42,
            1338,
            34,
            || false,
            || {
                asked.set(asked.get() + 1);
                ready(true)
            },
        ),
    )?;

    assert_eq!(1, asked.get());
    assert_eq!(2, server.asked().len());
    assert_eq!(Some("Мы будем в шесть"), model.look(34).text.as_deref());
    Ok(())
}

/// A second refusal after a yes is the failure it is — said, and never asked about in a loop.
#[test]
fn a_second_agree_first_is_a_failure_not_a_loop() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(PATH, 403, Some(NOT_AGREED));
    let model = rig.model(&server);
    let asked = Cell::new(0);

    drive(
        &rig.clock,
        model.show_stored(
            42,
            1338,
            34,
            || false,
            || {
                asked.set(asked.get() + 1);
                ready(true)
            },
        ),
    )?;

    assert_eq!(1, asked.get());
    assert_eq!(2, server.asked().len());
    let look = model.look(34);
    assert_eq!(TranscriptPhase::Failed, look.phase);
    assert_eq!(
        error_codes::ASSISTANT_CONSENT_REQUIRED,
        look.error.expect("why").code
    );
    assert_eq!(None, rig.find(34)?);
    Ok(())
}

/// The provider refused: said, final, and nothing kept — asking again is the member's to try, not
/// ours.
#[test]
fn a_refusal_is_said_and_nothing_is_kept() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        PATH,
        400,
        Some(r#"{"error": {"code": "transcript_refused", "message": "no"}}"#),
    );
    let model = rig.model(&server);

    rig.show(&model, false)?;

    let look = model.look(34);
    assert_eq!(TranscriptPhase::Failed, look.phase);
    let error = look.error.expect("why");
    assert_eq!(error_codes::TRANSCRIPT_REFUSED, error.code);
    assert!(!transcript_rules::may_retry(&error));
    assert_eq!(None, rig.find(34)?);
    Ok(())
}

/// A transient failure may be retried — and the retry is an ordinary click, which then shows the
/// answer the server finished and kept in the meantime.
#[test]
fn a_failed_ask_can_be_asked_again() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then(
        PATH,
        &[
            (
                500,
                Some(r#"{"error": {"code": "internal", "message": "provider"}}"#),
            ),
            (200, Some(SAID)),
        ],
    );
    let model = rig.model(&server);

    rig.show(&model, false)?;
    assert_eq!(TranscriptPhase::Failed, model.look(34).phase);
    assert!(transcript_rules::may_retry(
        model.look(34).error.as_ref().expect("why")
    ));
    // Never repeated by itself: a write.
    assert_eq!(1, server.asked().len());

    rig.show(&model, false)?;
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    assert_eq!(2, server.asked().len());
    Ok(())
}

/// A 2xx THIS CLIENT CANNOT READ IS NOT AN ANSWER — no transcript, a transcript with no `text`, a
/// `text` of null or of the wrong type: not kept (the next click asks again), never drawn as "No
/// speech" (that is only ever what the provider ANSWERED, as `""`), and said as the generic
/// "Couldn't get the text. Try again." — the sentence the other three clients give an answer they
/// cannot decode. (The C#'s theory, one row per body.)
#[test]
fn an_answer_with_no_text_is_a_failure_and_not_kept() -> StoreResult<()> {
    for body in [
        "{}",
        r#"{"transcript": null}"#,
        r#"{"transcript": {}}"#,
        r#"{"transcript": {"language": "ru"}}"#,
        r#"{"transcript": {"text": null}}"#,
        r#"{"transcript": {"text": null, "language": "ru"}}"#,
        r#"{"transcript": {"text": 42}}"#,
    ] {
        let rig = Rig::new()?;
        let server = Server::new();
        server.then(PATH, &[(200, Some(body)), (200, Some(SAID))]);
        let model = rig.model(&server);

        rig.show(&model, false)?;

        let look = model.look(34);
        assert_eq!(TranscriptPhase::Failed, look.phase, "{body}");
        assert!(!look.no_speech(), "{body}");
        assert_eq!(None, look.text, "{body}");
        assert_eq!(None, rig.find(34)?, "{body}");
        assert!(!model.holds(34)?, "{body}");
        let error = look.error.expect("why");
        assert!(transcript_rules::may_retry(&error), "{body}");
        assert_eq!(
            "Couldn't get the text. Try again.",
            transcript_rules::failure_sentence(&error, &common::english()),
            "{body}"
        );
        // Not kept, so the next click asks again — and a readable answer then is kept.
        rig.show(&model, false)?;
        assert_eq!(2, server.asked().len(), "{body}");
        assert_eq!(
            "Мы будем в шесть",
            rig.find(34)?.expect("kept").text,
            "{body}"
        );
    }
    Ok(())
}

/// ONE ASK PER RECORDING AT A TIME: a second click while the first is still waiting — on the server
/// or on the consent question — does nothing; while it waits on the server it is drawn as "Getting
/// the text…".
#[test]
fn a_second_click_while_asking_does_nothing() -> StoreResult<()> {
    let rig = Rig::new()?;
    let gate = Gate::new();
    let server = Server::new();
    held(&server, PATH, &gate, SAID);
    let model = rig.model(&server);
    let mut pool = LocalPool::new();
    let first_done = Rc::new(Cell::new(false));
    {
        let model = Rc::clone(&model);
        let first_done = Rc::clone(&first_done);
        pool.spawner()
            .spawn_local(async move {
                model
                    .show_stored(42, 1338, 34, || false, || ready(true))
                    .await
                    .expect("the cache");
                first_done.set(true);
            })
            .expect("spawned");
    }
    pool.run_until_stalled();
    assert_eq!(1, server.asked().len());
    assert_eq!(TranscriptPhase::Asking, model.look(34).phase);
    // A second click that DID ask would wait on the held answer for ever: `drive` would say it is
    // stuck rather than hang.
    rig.show(&model, false)?;
    assert_eq!(1, server.asked().len());

    gate.open();
    pool.run_until_stalled();
    assert!(first_done.get());
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    assert_eq!(1, server.asked().len());
    Ok(())
}

/// Two recordings are two asks, and each is drawn on its own.
#[test]
fn each_recording_is_its_own() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server
        .on(PATH, SAID)
        .on(PATH_35, r#"{"transcript": {"text": "and the bread"}}"#);
    let model = rig.model(&server);

    rig.show(&model, false)?;
    drive(
        &rig.clock,
        model.show_stored(42, 1338, 35, || false, || ready(true)),
    )?;

    assert_eq!(Some("Мы будем в шесть"), model.look(34).text.as_deref());
    assert_eq!(Some("and the bread"), model.look(35).text.as_deref());
    model.hide(35);
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    Ok(())
}

/// A session that ended forgets what was on screen; the cache is wiped with everything else.
#[test]
fn clear_forgets_what_was_shown() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    rig.show(&model, false)?;
    model.clear();
    assert_eq!(TranscriptLook::closed(), model.look(34));
    Ok(())
}

/// A SESSION THAT ENDED WHILE THE ANSWER WAS ON ITS WAY keeps nothing of it: the sign-out wiped the
/// cache, and an answer landing afterwards must not write the last account's words back for the
/// next one to read without asking — least of all an answer made from supplied sound, which nobody
/// else may ever be handed. The next session's own click on the same recording is not blocked by
/// the old request either.
#[test]
fn an_answer_that_lands_after_the_session_ended_is_not_kept() -> StoreResult<()> {
    let rig = Rig::new()?;
    let gate = Gate::new();
    let holding = Rc::new(Cell::new(true));
    let server = Server::new();
    {
        let gate = gate.clone();
        let holding = Rc::clone(&holding);
        server.on_async(PATH, move || {
            let gate = gate.clone();
            if holding.get() {
                Box::pin(async move {
                    gate.wait().await;
                    Ok((200, Some(SAID.to_string())))
                })
            } else {
                Box::pin(ready(Ok((
                    200,
                    Some(r#"{"transcript": {"text": "the next account's"}}"#.to_string()),
                ))))
            }
        });
    }
    let model = rig.model(&server);
    let mut pool = LocalPool::new();
    let first_done = Rc::new(Cell::new(false));
    {
        let model = Rc::clone(&model);
        let first_done = Rc::clone(&first_done);
        pool.spawner()
            .spawn_local(async move {
                model
                    .show_stored(42, 1338, 34, || false, || ready(true))
                    .await
                    .expect("the cache");
                first_done.set(true);
            })
            .expect("spawned");
    }
    pool.run_until_stalled();
    assert_eq!(1, server.asked().len());
    assert_eq!(TranscriptPhase::Asking, model.look(34).phase);

    // Signed out while it waits.
    model.clear();
    holding.set(false);
    assert_eq!(TranscriptLook::closed(), model.look(34));

    // The next account clicks the same recording: asked afresh, not swallowed as "already asking".
    rig.show(&model, false)?;
    assert_eq!(2, server.asked().len());
    assert_eq!(Some("the next account's"), model.look(34).text.as_deref());

    // The old answer lands now — and changes nothing.
    gate.open();
    pool.run_until_stalled();
    assert!(first_done.get());
    assert_eq!(Some("the next account's"), model.look(34).text.as_deref());
    assert_eq!("the next account's", rig.find(34)?.expect("kept").text);
    Ok(())
}

/// The same, with nothing asked afresh: the old answer is neither kept nor drawn.
#[test]
fn an_answer_after_sign_out_leaves_nothing_behind() -> StoreResult<()> {
    let rig = Rig::new()?;
    let gate = Gate::new();
    let server = Server::new();
    held(&server, PATH, &gate, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);
    let mut pool = LocalPool::new();
    let first_done = Rc::new(Cell::new(false));
    {
        let model = Rc::clone(&model);
        let first_done = Rc::clone(&first_done);
        let sound = maker.source();
        pool.spawner()
            .spawn_local(async move {
                model
                    .show(
                        42,
                        1338,
                        34,
                        TranscriptForm::Supplied,
                        sound,
                        || false,
                        || ready(true),
                    )
                    .await
                    .expect("the cache");
                first_done.set(true);
            })
            .expect("spawned");
    }
    pool.run_until_stalled();
    assert_eq!(1, server.asked().len());
    model.clear();
    gate.open();
    pool.run_until_stalled();
    assert!(first_done.get());

    assert_eq!(None, rig.find(34)?);
    assert!(!model.holds(34)?);
    assert_eq!(TranscriptLook::closed(), model.look(34));
    Ok(())
}

/// Hide is for open text only: it does not wipe a failure the member has not read.
#[test]
fn hide_leaves_a_failure_alone() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        PATH,
        400,
        Some(r#"{"error": {"code": "not_transcribable", "message": "no"}}"#),
    );
    let model = rig.model(&server);
    rig.show(&model, false)?;
    model.hide(34);
    assert_eq!(TranscriptPhase::Failed, model.look(34).phase);
    Ok(())
}

// ---- the device's own sound (phase 3) -----------------------------------------------------------

/// A VIDEO'S TEXT: the device's sound goes as the one multipart `audio` part, the answer is shown,
/// and it is kept on THIS device marked as supplied — the server never kept it.
#[test]
fn supplied_sound_is_sent_and_kept_on_this_device_only() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;

    assert_eq!(1, maker.calls());
    assert_eq!(vec![format!("/api/v1{PATH}")], server.asked());
    let bodies = server.bodies();
    assert_eq!(1, bodies.len());
    let body = &bodies[0];
    assert!(body.contains("name=\"audio\""), "{body}");
    assert!(body.contains("filename=\"audio.m4a\""), "{body}");
    assert!(body.contains("Content-Type: audio/mp4"), "{body}");
    assert!(body.contains("ftypM4A"), "{body}");
    assert_eq!(TranscriptLook::open("Мы будем в шесть"), model.look(34));
    assert_eq!(
        Some(kept(34, "Мы будем в шесть", Some("ru"), true)),
        rig.find(34)?
    );

    // Kept means not made again, and not asked again.
    model.hide(34);
    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;
    assert_eq!(1, maker.calls());
    assert_eq!(1, server.asked().len());
    Ok(())
}

/// THE DEVICE CANNOT READ IT: no track, a codec it lacks — "Couldn't read the sound in this file.",
/// final, with nothing sent anywhere and nothing kept.
#[test]
fn sound_the_device_cannot_read_is_said_so() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::new(|| Ok(SuppliedSound::Failed(transcript_sound::unreadable())));

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;

    let look = model.look(34);
    assert_eq!(TranscriptPhase::Failed, look.phase);
    assert_eq!(Some(transcript_sound::unreadable()), look.error);
    assert!(!transcript_rules::may_retry(
        look.error.as_ref().expect("why")
    ));
    assert!(server.asked().is_empty());
    assert_eq!(None, rig.find(34)?);
    Ok(())
}

/// THE SIZE BOUND, HELD HERE TOO: sound over the ceiling ("too long"), empty or not MPEG-4
/// ("couldn't read") — what the server would refuse — is never uploaded; and a maker that fails
/// outright is "couldn't read", never a failure at a click.
#[test]
fn sound_the_server_would_refuse_is_never_uploaded() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let length = M4A.len() as i64;

    let over = Maker::making(&M4A).with_max_bytes(length - 1);
    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        over.source(),
        false,
        true,
    )?;
    assert_eq!(Some(transcript_sound::too_long()), model.look(34).error);
    let exact = Maker::making(&M4A).with_max_bytes(length);
    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        exact.source(),
        false,
        true,
    )?;
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    assert_eq!(1, server.asked().len());

    let other_server = Server::new();
    other_server.on(PATH_35, SAID);
    let other = rig.model(&other_server);
    let not_mpeg4 = Maker::making(b"OggS\0\0\0\0\0\0\0\0");
    rig.supply(
        &other,
        35,
        TranscriptForm::Supplied,
        not_mpeg4.source(),
        false,
        true,
    )?;
    assert_eq!(Some(transcript_sound::unreadable()), other.look(35).error);
    let empty = Maker::making(&[]);
    rig.supply(
        &other,
        35,
        TranscriptForm::Supplied,
        empty.source(),
        false,
        true,
    )?;
    assert_eq!(Some(transcript_sound::unreadable()), other.look(35).error);
    let throws = Maker::new(|| {
        Err(AttemptFailure::new(
            "InvalidOperationException",
            0x8013_1509,
        ))
    });
    rig.supply(
        &other,
        35,
        TranscriptForm::Supplied,
        throws.source(),
        false,
        true,
    )?;
    assert_eq!(Some(transcript_sound::unreadable()), other.look(35).error);
    assert_eq!(None, rig.find(35)?);
    Ok(())
}

/// TOO LONG BY ITS OWN LENGTH: a recording whose stated length could not fit the ceiling even at
/// 64 kbit/s is told so at the press — "This recording is too long to turn into text." — with
/// nothing downloaded, nothing made and nothing sent. The action was offered; only its answer says
/// why there is no text.
#[test]
fn a_recording_too_long_by_its_length_is_told_without_the_work() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A).with_duration(55 * 60_000);

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;

    assert_eq!(Some(transcript_sound::too_long()), model.look(34).error);
    assert!(!transcript_rules::may_retry(
        model.look(34).error.as_ref().expect("why")
    ));
    assert_eq!(0, maker.calls());
    assert!(server.asked().is_empty());

    // A length that fits is made and sent as before.
    let fits = Maker::making(&M4A).with_duration(50 * 60_000);
    rig.supply(
        &model,
        35,
        TranscriptForm::Supplied,
        fits.source(),
        false,
        true,
    )?;
    assert_eq!(1, fits.calls());
    Ok(())
}

/// A DOWNLOAD THAT FAILED says what it was: a lost connection is "try again" — and the next click
/// makes the sound again — while an attachment that is gone is "not available".
#[test]
fn a_failed_download_is_said_as_what_it_was() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let answers = Rc::new(RefCell::new(VecDeque::from([
        SuppliedSound::Failed(ApiError::transport("connection reset")),
        SuppliedSound::Made(M4A.to_vec()),
    ])));
    let maker = {
        let answers = Rc::clone(&answers);
        Maker::new(move || Ok(answers.borrow_mut().pop_front().expect("an answer")))
    };

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;
    assert_eq!(TranscriptPhase::Failed, model.look(34).phase);
    assert!(transcript_rules::may_retry(
        model.look(34).error.as_ref().expect("why")
    ));
    assert!(server.asked().is_empty());

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;
    assert_eq!(2, maker.calls());
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);

    let gone = rig.model(&Server::new());
    let missing = Maker::new(|| {
        Ok(SuppliedSound::Failed(ApiError::new(
            error_codes::ATTACHMENT_NOT_FOUND,
            "gone",
            404,
        )))
    });
    rig.supply(
        &gone,
        35,
        TranscriptForm::Supplied,
        missing.source(),
        false,
        true,
    )?;
    assert!(!transcript_rules::may_retry(
        gone.look(35).error.as_ref().expect("why")
    ));
    Ok(())
}

/// NOT NOW MAKES NOTHING: the consent question comes first, so nothing is downloaded or decoded for
/// a no.
#[test]
fn not_now_makes_no_sound() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        true,
        false,
    )?;

    assert_eq!(0, maker.calls());
    assert!(server.asked().is_empty());
    assert_eq!(TranscriptLook::closed(), model.look(34));
    Ok(())
}

/// AGREE FIRST, FROM THE SERVER: the question, then the request again — with the sound already
/// made, which is made ONCE per click however many times it is sent.
#[test]
fn the_sound_is_made_once_though_the_request_goes_twice() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then(PATH, &[(403, Some(NOT_AGREED)), (200, Some(SAID))]);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;

    assert_eq!(1, maker.calls());
    assert_eq!(2, server.asked().len());
    assert_eq!(2, server.bodies().len());
    assert!(rig.find(34)?.expect("kept").supplied);
    Ok(())
}

/// THE STORED FORM REFUSED AS NOT TRANSCRIBABLE — the server's ceiling or list is not what this
/// device read — falls back to the device's sound, once ("a client that sent no body may send the
/// sound track instead").
#[test]
fn the_stored_form_falls_back_to_the_devices_sound() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.then(
        PATH,
        &[
            (
                400,
                Some(r#"{"error": {"code": "not_transcribable", "message": "over"}}"#),
            ),
            (200, Some(SAID)),
        ],
    );
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Stored,
        maker.source(),
        false,
        true,
    )?;

    assert_eq!(1, maker.calls());
    assert_eq!(2, server.asked().len());
    // The first went with no body, the second with the sound.
    assert_eq!(1, server.bodies().len());
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    assert!(rig.find(34)?.expect("kept").supplied);
    Ok(())
}

/// The stored form answered is the stored form's: no sound made, and kept as the shared answer it
/// is.
#[test]
fn the_stored_form_answered_makes_no_sound() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Stored,
        maker.source(),
        false,
        true,
    )?;

    assert_eq!(0, maker.calls());
    assert!(server.bodies().is_empty());
    assert!(!rig.find(34)?.expect("kept").supplied);
    Ok(())
}

/// Other refusals of the stored form are final as they are — not a reason to upload anything — and
/// a stored form with no maker cannot fall back.
#[test]
fn only_not_transcribable_falls_back() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    server.on_status(
        PATH,
        403,
        Some(r#"{"error": {"code": "transcript_not_allowed", "message": "no"}}"#),
    );
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);

    rig.supply(
        &model,
        34,
        TranscriptForm::Stored,
        maker.source(),
        false,
        true,
    )?;
    assert_eq!(0, maker.calls());
    assert_eq!(
        error_codes::TRANSCRIPT_NOT_ALLOWED,
        model.look(34).error.expect("why").code
    );

    let plain_server = Server::new();
    plain_server.on_status(
        PATH,
        400,
        Some(r#"{"error": {"code": "not_transcribable", "message": "no"}}"#),
    );
    let plain = rig.model(&plain_server);
    rig.supply(&plain, 34, TranscriptForm::Stored, None, false, true)?;
    assert_eq!(
        error_codes::NOT_TRANSCRIBABLE,
        plain.look(34).error.expect("why").code
    );
    Ok(())
}

/// NOTHING TO ASK WITH — no form, or the device's form with no maker — is "not available" and sends
/// nothing; but text this device already holds is shown whatever the form.
#[test]
fn no_form_sends_nothing_but_held_text_is_shown() -> StoreResult<()> {
    let rig = Rig::new()?;
    let server = Server::new();
    let model = rig.model(&server);

    rig.supply(&model, 34, TranscriptForm::None, None, false, true)?;
    assert_eq!(
        error_codes::NOT_TRANSCRIBABLE,
        model.look(34).error.expect("why").code
    );
    rig.supply(&model, 34, TranscriptForm::Supplied, None, false, true)?;
    assert_eq!(
        error_codes::NOT_TRANSCRIBABLE,
        model.look(34).error.expect("why").code
    );
    assert!(server.asked().is_empty());

    rig.store.keep(
        &kept(34, "Back at six", None, true),
        datetime!(1970-01-01 00:00 UTC),
    )?;
    drive(
        &rig.clock,
        model.show(
            42,
            1338,
            34,
            TranscriptForm::None,
            None,
            || true,
            never_asked,
        ),
    )?;
    assert_eq!(TranscriptLook::open("Back at six"), model.look(34));
    assert!(server.asked().is_empty());
    Ok(())
}

/// A SUPPLIED ANSWER NEVER REPLACES THE SHARED ONE: when this device already holds the answer from
/// the stored copy, a later supplied answer for the same attachment does not overwrite it.
#[test]
fn a_supplied_answer_never_replaces_the_stored_one() -> StoreResult<()> {
    let rig = Rig::new()?;
    rig.store.keep(
        &kept(34, "from the stored copy", None, false),
        datetime!(1970-01-01 00:00 UTC),
    )?;
    let server = Server::new();
    server.on(PATH, SAID);
    let model = rig.model(&server);
    let maker = Maker::making(&M4A);
    rig.supply(
        &model,
        34,
        TranscriptForm::Supplied,
        maker.source(),
        false,
        true,
    )?;
    assert_eq!(
        Some(kept(34, "from the stored copy", None, false)),
        rig.find(34)?
    );
    Ok(())
}

/// LINUX'S OWN — the C#'s `finally`: a click whose future is dropped part-way (the view went, the
/// shell cancelled it) leaves the recording free, so the next click asks again rather than being
/// swallowed as "already asking".
#[test]
fn a_click_dropped_while_asking_lets_the_next_one_ask() -> StoreResult<()> {
    let rig = Rig::new()?;
    let gate = Gate::new();
    let server = Server::new();
    held(&server, PATH, &gate, SAID);
    let model = rig.model(&server);
    let mut pool = LocalPool::new();
    let handle = {
        let model = Rc::clone(&model);
        pool.spawner()
            .spawn_local_with_handle(async move {
                model
                    .show_stored(42, 1338, 34, || false, || ready(true))
                    .await
                    .expect("the cache");
            })
            .expect("spawned")
    };
    pool.run_until_stalled();
    assert_eq!(TranscriptPhase::Asking, model.look(34).phase);
    drop(handle);
    pool.run_until_stalled();

    gate.open();
    rig.show(&model, false)?;
    assert_eq!(2, server.asked().len());
    assert_eq!(TranscriptPhase::Open, model.look(34).phase);
    Ok(())
}

//! `ComposerStagingTests`: the strip a chat's next message takes its attachments from — prepared in
//! order, stopped at the cap, called off on request, and every chip named.
//!
//! The C#'s `Task.FromResult` preparer is a closure whose future is `ready`; a preparer that throws
//! answers `Err`; a `TaskCompletionSource` is a oneshot channel or `common::Gate`; the bounded
//! `WaitAsync(30 s)` is `run_until_stalled` — a batch that is still waiting then has failed.

mod common;

use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::future::ready;
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::Cancel;
use fc_core::text::media_text;
use fc_logic::composer_staging::{self, ComposerStaging, PrepFailure, PrepOutcome};
use fc_logic::media_outbox::StagedMedia;
use fc_logic::voice_notes;
use futures::channel::oneshot;
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

const POINT: &str = ".";

/// `new StagedMedia("photo", "image/jpeg", new byte[] { 1 }, 10, 10)`.
fn photo() -> StagedMedia {
    StagedMedia {
        width: Some(10),
        height: Some(10),
        ..StagedMedia::new("photo", "image/jpeg", vec![1])
    }
}

fn file(name: &str, bytes: Vec<u8>) -> StagedMedia {
    StagedMedia {
        name: Some(name.to_string()),
        ..StagedMedia::new("file", "text/plain", bytes)
    }
}

type Asked = Rc<RefCell<Vec<String>>>;

/// A preparer that notes each file asked for and stages it as a small text file — or refuses the
/// ones named `unreadable`.
fn preparer(
    asked: &Asked,
    unreadable: &'static [&'static str],
) -> impl Fn(&'static str, Cancel) -> std::future::Ready<Result<PrepOutcome, Infallible>> + 'static
{
    let asked = Rc::clone(asked);
    move |name, _| {
        asked.borrow_mut().push(name.to_string());
        ready(Ok(if unreadable.contains(&name) {
            PrepOutcome::refused(PrepFailure::Unreadable)
        } else {
            PrepOutcome::staged(file(name, vec![1, 2]))
        }))
    }
}

fn names(staging: &ComposerStaging) -> Vec<Option<String>> {
    staging
        .items()
        .iter()
        .map(|item| item.name.clone())
        .collect()
}

fn named(names: &[&str]) -> Vec<Option<String>> {
    names.iter().map(|name| Some(name.to_string())).collect()
}

fn asked_of(asked: &Asked) -> Vec<String> {
    asked.borrow().clone()
}

/// A batch run to its end at once.
fn run<F: std::future::Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}

#[test]
fn files_are_staged_in_the_order_they_were_picked() {
    let say = common::english();
    let staging = ComposerStaging::new();
    let asked = Asked::default();

    let said = run(staging.ingest(
        ["a.txt", "b.txt", "c.txt"],
        preparer(&asked, &[]),
        || true,
        &say,
    ));

    assert_eq!(None, said);
    assert_eq!(named(&["a.txt", "b.txt", "c.txt"]), names(&staging));
    assert!(!staging.preparing());
}

/// STOPPED AT THE CAP, not preparing the rest only to throw them away — and saying so.
#[test]
fn a_batch_stops_at_the_cap_and_says_so() {
    let say = common::english();
    let staging = ComposerStaging::new();
    for _ in 0..fc_text::media::MAX_PER_MESSAGE - 2 {
        assert!(staging.add(photo()));
    }
    let asked = Asked::default();

    let said = run(staging.ingest(["a", "b", "c", "d"], preparer(&asked, &[]), || true, &say));

    assert_eq!(fc_text::media::MAX_PER_MESSAGE, staging.len());
    assert_eq!(vec!["a", "b"], asked_of(&asked));
    assert_eq!(Some("You can attach up to 10 items.".to_string()), said);
    assert!(!staging.add(photo()));
}

#[test]
fn a_refused_file_is_said_and_the_rest_still_staged() {
    let say = common::english();
    let staging = ComposerStaging::new();

    let said = run(staging.ingest(
        ["a", "broken", "c"],
        preparer(&Asked::default(), &["broken"]),
        || true,
        &say,
    ));

    assert_eq!(named(&["a", "c"]), names(&staging));
    assert_eq!(Some("Couldn't read that file.".to_string()), said);
}

#[test]
fn a_preparer_that_throws_is_an_unreadable_file_not_a_crash() {
    let say = common::english();
    let staging = ComposerStaging::new();

    let said = run(staging.ingest(
        ["x"],
        |_: &str, _| ready(Err::<PrepOutcome, &str>("locked")),
        || true,
        &say,
    ));

    assert!(staging.is_empty());
    assert_eq!(Some("Couldn't read that file.".to_string()), said);
    assert!(!staging.preparing());
}

/// A pane that went away mid-batch: nothing more lands, and nothing is said.
#[test]
fn a_pane_that_went_away_stops_the_batch() {
    let say = common::english();
    let staging = ComposerStaging::new();
    let here = Rc::new(Cell::new(true));
    let asked = Asked::default();
    let prepare = {
        let here = Rc::clone(&here);
        let asked = Rc::clone(&asked);
        move |name: &str, _| {
            asked.borrow_mut().push(name.to_string());
            here.set(false);
            ready(Ok::<_, Infallible>(PrepOutcome::staged(photo())))
        }
    };

    let said = run(staging.ingest(["a", "b"], prepare, || here.get(), &say));

    assert_eq!(None, said);
    assert!(staging.is_empty());
    assert_eq!(vec!["a"], asked_of(&asked));
    assert!(!staging.preparing());
}

/// A pane already gone when the files arrive prepares none of them.
#[test]
fn a_pane_already_gone_prepares_nothing() {
    let say = common::english();
    let staging = ComposerStaging::new();
    let asked = Asked::default();

    let said = run(staging.ingest(["a", "b"], preparer(&asked, &[]), || false, &say));

    assert_eq!(None, said);
    assert!(asked_of(&asked).is_empty());
    assert!(staging.is_empty());
}

#[test]
fn preparing_is_the_other_busy() {
    let say = common::english();
    let staging = Rc::new(ComposerStaging::new());
    let (answer, answered) = oneshot::channel::<PrepOutcome>();
    let answered = RefCell::new(Some(answered));
    let said: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let mut pool = LocalPool::new();
    {
        let staging = Rc::clone(&staging);
        let said = Rc::clone(&said);
        pool.spawner()
            .spawn_local(async move {
                let say = common::english();
                let prepare = |_: &str, _| answered.borrow_mut().take().expect("asked once");
                *said.borrow_mut() = Some(staging.ingest(["a"], prepare, || true, &say).await);
            })
            .unwrap();
    }
    pool.run_until_stalled();

    assert!(staging.preparing());
    assert_eq!(
        Some("Wait until the current attachment is done.".to_string()),
        staging.busy_reason(false, &say)
    );
    assert_eq!(
        Some("Finish editing before attaching something.".to_string()),
        staging.busy_reason(true, &say)
    );

    answer
        .send(PrepOutcome::refused(PrepFailure::TooLarge))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(
        Some(Some("That file is over the 100 MB limit.".to_string())),
        said.borrow().clone()
    );
    assert_eq!(None, staging.busy_reason(false, &say));
}

/// A ten-minute clip dropped by mistake: Cancel reaches the preparer through its token, nothing of
/// the rest is started, nothing is said, and the composer is free again at once — not after the
/// transcode.
#[test]
fn a_batch_called_off_stops_the_preparer_and_starts_nothing_more() {
    let say = common::english();
    let staging = Rc::new(ComposerStaging::new());
    let asked = Asked::default();
    let started = common::Gate::new();
    let said: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let mut pool = LocalPool::new();
    {
        let staging = Rc::clone(&staging);
        let said = Rc::clone(&said);
        let asked = Rc::clone(&asked);
        let started = started.clone();
        pool.spawner()
            .spawn_local(async move {
                let say = common::english();
                let prepare = move |name: &'static str, cancel: Cancel| {
                    asked.borrow_mut().push(name.to_string());
                    let started = started.clone();
                    async move {
                        if name == "clip.mov" {
                            started.open();
                            // `Task.Delay(Timeout.Infinite, cancel)`: it ends only by being called
                            // off, and then as the exception the C# threw.
                            cancel.cancelled().await;
                            return Err("cancelled");
                        }
                        Ok(PrepOutcome::staged(file(name, vec![1])))
                    }
                };
                *said.borrow_mut() = Some(
                    staging
                        .ingest(["a.txt", "clip.mov", "c.txt"], prepare, || true, &say)
                        .await,
                );
            })
            .unwrap();
    }
    pool.run_until_stalled();
    assert!(started.is_open());
    assert!(staging.preparing());
    staging.cancel_preparing();

    // Bounded: a cancel that never reached the batch must FAIL here, not hang the suite on its
    // endless clip.
    pool.run_until_stalled();
    assert_eq!(Some(None), said.borrow().clone());
    assert_eq!(vec!["a.txt", "clip.mov"], asked_of(&asked));
    // What was staged before the cancel stays: it has a chip of its own to remove it by.
    assert_eq!(named(&["a.txt"]), names(&staging));
    assert!(!staging.preparing());
    assert_eq!(None, staging.busy_reason(false, &say));
}

/// A preparer that did not notice the cancel and came back with the file anyway: it is still not
/// staged. (Here its future is dropped the moment the call-off is seen.)
#[test]
fn a_batch_called_off_stages_nothing_that_came_back_anyway() {
    let staging = Rc::new(ComposerStaging::new());
    let gate = common::Gate::new();
    let said: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let mut pool = LocalPool::new();
    {
        let staging = Rc::clone(&staging);
        let said = Rc::clone(&said);
        let gate = gate.clone();
        pool.spawner()
            .spawn_local(async move {
                let say = common::english();
                let prepare = move |_: &str, _| {
                    let gate = gate.clone();
                    async move {
                        gate.wait().await;
                        Ok::<_, Infallible>(PrepOutcome::staged(photo()))
                    }
                };
                *said.borrow_mut() = Some(staging.ingest(["a", "b"], prepare, || true, &say).await);
            })
            .unwrap();
    }
    pool.run_until_stalled();
    staging.cancel_preparing();
    gate.open();
    pool.run_until_stalled();

    assert_eq!(Some(None), said.borrow().clone());
    assert!(staging.is_empty());
}

/// Cancel is for the batch in hand: with none running it does nothing, and the next batch is
/// untouched by it.
#[test]
fn calling_off_nothing_leaves_the_next_batch_alone() {
    let say = common::english();
    let staging = ComposerStaging::new();
    staging.cancel_preparing();

    let said = run(staging.ingest(["a"], preparer(&Asked::default(), &[]), || true, &say));
    staging.cancel_preparing();

    assert_eq!(None, said);
    assert_eq!(named(&["a"]), names(&staging));
    assert_eq!(
        None,
        run(staging.ingest(["b"], preparer(&Asked::default(), &[]), || true, &say))
    );
    assert_eq!(named(&["a", "b"]), names(&staging));
}

#[test]
fn taking_empties_the_strip_and_restoring_puts_it_back_in_front() {
    let staging = ComposerStaging::new();
    staging.add(file("first", vec![0]));
    let taken = staging.take_all();
    assert!(staging.is_empty());

    staging.add(file("later", vec![0]));
    staging.restore(taken);

    assert_eq!(named(&["first", "later"]), names(&staging));
    staging.remove_at(0);
    staging.remove_at(7);
    assert_eq!(named(&["later"]), names(&staging));
}

/// Leaving the chat takes the voice notes recorded here out of the strip, in their order, to wait
/// as "not sent" (S2.8) — a sound file picked from disk and everything else staged stay where they
/// are.
#[test]
fn leaving_takes_only_the_recordings_made_here() {
    let staging = ComposerStaging::new();
    let first = Rc::new(voice_notes::staged(vec![0; 2048], Duration::from_secs(3)).unwrap());
    let second = Rc::new(voice_notes::staged(vec![0; 4096], Duration::from_secs(12)).unwrap());
    let song = StagedMedia {
        duration_ms: Some(3000),
        name: Some("song.m4a".to_string()),
        ..StagedMedia::new("audio", "audio/mp4", vec![0; 2048])
    };
    staging.add(photo());
    staging.add(Rc::clone(&first));
    staging.add(song);
    staging.add(Rc::clone(&second));
    assert!(staging.holds_recordings());

    let taken = staging.take_recordings();

    assert_eq!(vec![first, second], taken);
    let kinds: Vec<String> = staging
        .items()
        .iter()
        .map(|item| item.kind.clone())
        .collect();
    assert_eq!(vec!["photo", "audio"], kinds);
    assert_eq!(Some("song.m4a".to_string()), staging.items()[1].name);
    assert!(!staging.holds_recordings());
    assert!(staging.take_recordings().is_empty());
}

#[test]
fn a_chip_names_a_picture_by_its_kind_and_a_file_by_its_name_and_size() {
    let say = common::english();
    assert_eq!("Photo", composer_staging::label(&photo(), &say, POINT));
    assert_eq!(
        "Video",
        composer_staging::label(
            &StagedMedia::new("video", "video/mp4", vec![0; 3]),
            &say,
            POINT
        )
    );
    assert_eq!(
        format!("report.pdf · {}", media_text::display_size(2048, POINT)),
        composer_staging::label(
            &StagedMedia {
                name: Some("report.pdf".to_string()),
                ..StagedMedia::new("file", "application/pdf", vec![0; 2048])
            },
            &say,
            POINT
        )
    );
    assert_eq!(
        format!("File · {}", media_text::display_size(1, POINT)),
        composer_staging::label(
            &StagedMedia::new("file", "application/pdf", vec![0; 1]),
            &say,
            POINT
        )
    );
}

/// A voice note recorded here is named by its LENGTH on its chip, "Voice message · 0:42" (S2.7,
/// S10) — its size says nothing to somebody deciding whether to send it — while a sound file picked
/// from disk keeps its name and size.
#[test]
fn a_recorded_note_is_named_by_its_length() {
    let say = common::english();
    let note = voice_notes::staged(vec![0; 64 * 1024], Duration::from_secs(42)).unwrap();
    assert_eq!(
        "Voice message · 0:42",
        composer_staging::label(&note, &say, POINT)
    );
    assert_eq!(
        "Voice message · 5:00",
        composer_staging::label(
            &voice_notes::staged(vec![0; 4096], voice_notes::LONGEST).unwrap(),
            &say,
            POINT
        )
    );
    assert_eq!(
        format!("song.m4a · {}", media_text::display_size(2048, POINT)),
        composer_staging::label(
            &StagedMedia {
                duration_ms: Some(3000),
                name: Some("song.m4a".to_string()),
                ..StagedMedia::new("audio", "audio/mp4", vec![0; 2048])
            },
            &say,
            POINT
        )
    );
    let russian = common::catalogues().for_language("ru");
    assert_ne!(
        "Voice message · 0:42",
        composer_staging::label(&note, &russian, POINT)
    );
    assert!(composer_staging::label(&note, &russian, POINT).ends_with("0:42"));
}

/// "Delete this recording?" may be answered after other items came or went: the note is taken out
/// by REFERENCE — not by its old place, and not an equal copy of it — and a note already gone is
/// reported gone.
#[test]
fn an_item_is_removed_by_reference() {
    let staging = ComposerStaging::new();
    let note = StagedMedia {
        duration_ms: Some(12_000),
        ..StagedMedia::new("audio", "audio/mp4", vec![0; 2048])
    };
    let first = Rc::new(note.clone());
    let twin = Rc::new(note);
    assert_eq!(first, twin);
    staging.add(photo());
    staging.add(Rc::clone(&twin));
    staging.add(Rc::clone(&first));
    staging.remove_at(0);

    assert!(staging.remove(&first));

    assert_eq!(1, staging.len());
    assert!(Rc::ptr_eq(&twin, &staging.items()[0]));
    assert!(!staging.remove(&first));
    assert_eq!(1, staging.len());
}

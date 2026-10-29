//! `TranscodeAttemptsTests`: the loop around the transcoder — which way of asking is tried after
//! which, and what ends the asking. The attempts here are strings and the "transcoder" a closure,
//! because the order is the whole of what is decided.
//!
//! Time is the fake clock's (`fc_core::testing::drive` moves it to the next deadline only when
//! nothing else can move); the C#'s `Task.FromException` and a lambda that throws are both an
//! attempt whose future answers `Err(AttemptFailure)` at once — a Rust closure cannot throw.

use std::cell::RefCell;
use std::future::{Future, ready};
use std::rc::Rc;
use std::time::Duration;

use fc_core::rt::Cancel;
use fc_core::testing::{FakeClock, drive};
use fc_logic::transcode_attempts::{self, AttemptFailure, Attempted, OperationCanceled};
use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use time::macros::datetime;

const LONG: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, PartialEq, Eq)]
struct Made(String);

fn made(by: &str) -> Made {
    Made(by.to_string())
}

type Lines = Rc<RefCell<Vec<String>>>;

fn lines() -> Lines {
    Rc::new(RefCell::new(Vec::new()))
}

fn clock() -> FakeClock {
    FakeClock::at(datetime!(2026-10-28 12:00 UTC))
}

/// `RunAsync`: the attempts, the attempt, where the log lines go, the ceiling and the person's
/// cancel.
async fn run<'a, F, Fut>(
    clock: &'a FakeClock,
    attempts: &'a [&'static str],
    attempt: F,
    said: Lines,
    ceiling: Duration,
    cancel: &'a Cancel,
) -> Result<Option<Made>, OperationCanceled>
where
    F: FnMut(&'a &'static str, Cancel) -> Fut + 'a,
    Fut: Future<Output = Result<Attempted<Made>, AttemptFailure>> + 'a,
{
    transcode_attempts::first_taken(
        attempts,
        attempt,
        ceiling,
        clock,
        |line| said.borrow_mut().push(line.to_string()),
        cancel,
    )
    .await
}

#[test]
fn the_first_way_that_is_taken_is_the_result_and_nothing_after_it_is_asked() {
    let clock = clock();
    let asked = lines();
    let record = asked.clone();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["high", "main"],
            move |attempt, _| {
                record.borrow_mut().push(attempt.to_string());
                ready(Ok(Attempted::Taken(made(attempt))))
            },
            lines(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(Some(made("high"))));
    assert_eq!(*asked.borrow(), ["high"]);
}

#[test]
fn a_refused_profile_is_followed_by_the_next() {
    let clock = clock();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["exact", "nearest", "main"],
            |attempt, _| {
                ready(Ok(if *attempt == "main" {
                    Attempted::Taken(made(attempt))
                } else {
                    Attempted::Refused
                }))
            },
            lines(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(Some(made("main"))));
}

/// The encoder takes the profile when it is prepared and rejects the caps once the pipeline
/// starts: the transcode itself fails. That once ended the loop, so a mono clip whose 64 000 was
/// refused late never reached 96 000 or Main.
#[test]
fn a_refusal_that_arrives_as_an_exception_mid_transcode_is_still_followed_by_the_next() {
    let clock = clock();
    let asked = lines();
    let said = lines();
    let record = asked.clone();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["64000", "96000", "main"],
            move |attempt, _| {
                record.borrow_mut().push(attempt.to_string());
                ready(if *attempt == "64000" {
                    // What the C# threw: InvalidOperationException("MF_E_INVALIDMEDIATYPE").
                    Err(AttemptFailure::new(
                        "InvalidOperationException",
                        0x8013_1509,
                    ))
                } else {
                    Ok(Attempted::Taken(made(attempt)))
                })
            },
            said.clone(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(Some(made("96000"))));
    assert_eq!(*asked.borrow(), ["64000", "96000"]);
    // Said with its number and with what was asked for — and never with the failure's own words.
    let said = said.borrow();
    assert_eq!(said.len(), 1);
    let line = &said[0];
    assert!(line.contains("InvalidOperationException 0x"), "{line}");
    assert!(line.contains("(64000)"), "{line}");
    assert!(!line.contains("MF_E_INVALIDMEDIATYPE"), "{line}");
}

/// A thrown refusal is caught whether it is thrown before the first await or after it — here, an
/// attempt whose future fails the moment it is first asked.
#[test]
fn an_attempt_that_throws_synchronously_is_a_refusal_too() {
    let clock = clock();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["a", "b"],
            |attempt, _| {
                ready(if *attempt == "a" {
                    // IOException("disk").
                    Err(AttemptFailure::new("IOException", 0x8013_1620))
                } else {
                    Ok(Attempted::Taken(made(attempt)))
                })
            },
            lines(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(Some(made("b"))));
}

/// Sideways, squashed, too fast: asking again under Main would make the same picture, minutes
/// later.
#[test]
fn a_result_that_came_out_wrong_ends_the_asking() {
    let clock = clock();
    let asked = lines();
    let record = asked.clone();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["high", "main"],
            move |attempt, _| {
                record.borrow_mut().push(attempt.to_string());
                ready(Ok(Attempted::Wrong))
            },
            lines(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(None));
    assert_eq!(*asked.borrow(), ["high"]);
}

#[test]
fn every_way_refused_is_no_transcode_and_says_so() {
    let clock = clock();
    let said = lines();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["a", "b"],
            |_, _| ready(Ok(Attempted::Refused)),
            said.clone(),
            LONG,
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(None));
    assert_eq!(*said.borrow(), ["no transcode profile was taken"]);
    assert_eq!(
        drive(
            &clock,
            run(
                &clock,
                &[],
                |_, _| ready(Ok(Attempted::Refused)),
                lines(),
                LONG,
                &Cancel::new(),
            ),
        ),
        Ok(None)
    );
}

/// The person took the file back. That is not rule C — `None` would send the original they no
/// longer want — so it comes out as a cancellation, and the attempt in hand was told to stop
/// through its own token.
#[test]
fn a_cancel_stops_the_attempt_in_hand_and_is_not_a_failure() {
    let clock = clock();
    let cancel = Cancel::new();
    let asked = lines();
    let mut pool = LocalPool::new();
    let ended = Rc::new(RefCell::new(None));
    {
        let (clock, cancel, asked, ended) =
            (clock.clone(), cancel.clone(), asked.clone(), ended.clone());
        pool.spawner()
            .spawn_local(async move {
                let result = run(
                    &clock,
                    &["high", "main"],
                    move |attempt, token| {
                        asked.borrow_mut().push(attempt.to_string());
                        async move {
                            // `Task.Delay(Timeout.Infinite, token)`: until the token says stop.
                            token.cancelled().await;
                            Err(AttemptFailure::new("TaskCanceledException", 0x8013_153B))
                        }
                    },
                    lines(),
                    LONG,
                    &cancel,
                )
                .await;
                *ended.borrow_mut() = Some(result);
            })
            .unwrap();
    }

    pool.run_until_stalled();
    assert_eq!(*asked.borrow(), ["high"]);
    assert!(ended.borrow().is_none());
    cancel.cancel();
    pool.run_until_stalled();

    // Bounded: a cancel that never reached the attempt FAILS here rather than hanging the suite.
    assert_eq!(*ended.borrow(), Some(Err(OperationCanceled)));
    assert_eq!(*asked.borrow(), ["high"]);
}

#[test]
fn an_already_cancelled_run_asks_nothing() {
    let clock = clock();
    let cancel = Cancel::new();
    cancel.cancel();
    let asked = lines();
    let record = asked.clone();

    let ended = drive(
        &clock,
        run(
            &clock,
            &["high"],
            move |attempt, _| {
                record.borrow_mut().push(attempt.to_string());
                ready(Ok(Attempted::Refused))
            },
            lines(),
            LONG,
            &cancel,
        ),
    );
    assert_eq!(ended, Err(OperationCanceled));
    assert!(asked.borrow().is_empty());
}

/// A cancel that an attempt swallowed and answered over — "refused", or even a result — is still
/// a cancel: nothing more is asked.
#[test]
fn a_cancel_an_attempt_answered_over_still_stops_the_rest() {
    let clock = clock();
    let cancel = Cancel::new();
    let asked = lines();
    let (record, raise) = (asked.clone(), cancel.clone());

    let ended = drive(
        &clock,
        run(
            &clock,
            &["high", "main"],
            move |attempt, _| {
                record.borrow_mut().push(attempt.to_string());
                raise.cancel();
                ready(Ok(Attempted::Refused))
            },
            lines(),
            LONG,
            &cancel,
        ),
    );
    assert_eq!(ended, Err(OperationCanceled));
    assert_eq!(*asked.borrow(), ["high"]);
}

/// ONE ceiling for every way of asking together: a stalled transcode is stopped through its
/// token, is a failure (rule C — not a cancellation), and the next is not tried, because it would
/// stall too.
#[test]
fn the_ceiling_stops_a_stalled_transcode_and_nothing_more_is_tried() {
    let clock = clock();
    let asked = lines();
    let said = lines();
    let record = asked.clone();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["high", "main"],
            move |attempt, token| {
                record.borrow_mut().push(attempt.to_string());
                let attempt = attempt.to_string();
                async move {
                    token.cancelled().await;
                    // Never reached in the C#: its delay threw instead.
                    if token.is_cancelled() {
                        return Err(AttemptFailure::new("TaskCanceledException", 0x8013_153B));
                    }
                    Ok(Attempted::Taken(Made(attempt)))
                }
            },
            said.clone(),
            Duration::from_millis(50),
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(None));
    assert_eq!(*asked.borrow(), ["high"]);
    assert_eq!(*said.borrow(), ["a transcode ran past its ceiling"]);
    assert_eq!(clock.slept(), [Duration::from_millis(50)]);
}

/// The ceiling is not per attempt: one that used it up and was only REFUSED leaves none for the
/// next.
#[test]
fn an_attempt_that_used_up_the_ceiling_leaves_none_for_the_next() {
    let clock = clock();
    let asked = lines();
    let record = asked.clone();
    let made_by = drive(
        &clock,
        run(
            &clock,
            &["high", "main"],
            move |attempt, token| {
                record.borrow_mut().push(attempt.to_string());
                async move {
                    // Swallowed, as a platform call might: the loop must still see the clock has
                    // run out.
                    token.cancelled().await;
                    Ok(Attempted::Refused)
                }
            },
            lines(),
            Duration::from_millis(50),
            &Cancel::new(),
        ),
    );

    assert_eq!(made_by, Ok(None));
    assert_eq!(*asked.borrow(), ["high"]);
}

/// Not in the C#, and where this port differs from it on purpose (measured on .NET 10: the C#
/// answered `null`, `null` and the result): a cancel raised while the LAST attempt runs is a
/// cancel whatever that attempt answered — refused, wrong or even a result — and never rule C.
#[test]
fn a_cancel_the_last_attempt_answered_over_is_still_a_cancel() {
    for answer in [
        Attempted::Refused,
        Attempted::Wrong,
        Attempted::Taken(made("only")),
    ] {
        let clock = clock();
        let cancel = Cancel::new();
        let said = lines();
        let raise = cancel.clone();
        let answered = answer.clone();
        let ended = drive(
            &clock,
            run(
                &clock,
                &["only"],
                move |_, _| {
                    raise.cancel();
                    ready(Ok(answered.clone()))
                },
                said.clone(),
                LONG,
                &cancel,
            ),
        );
        assert_eq!(ended, Err(OperationCanceled), "{answer:?}");
        assert!(said.borrow().is_empty(), "{answer:?}");
    }
}

/// Not in the C# (whose token has no counterpart): dropping the run drops the attempt in hand —
/// dropping a future IS cancelling it (linux/docs/port-plan.md, "Async").
#[test]
fn dropping_the_run_drops_the_attempt_in_hand() {
    struct Dropped(Rc<RefCell<bool>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            *self.0.borrow_mut() = true;
        }
    }

    let clock = clock();
    let cancel = Cancel::new();
    let dropped = Rc::new(RefCell::new(false));
    let flag = dropped.clone();
    let mut pool = LocalPool::new();
    let mut running = Box::pin(run(
        &clock,
        &["high"],
        move |_, token| {
            let guard = Dropped(flag.clone());
            async move {
                let _guard = guard;
                token.cancelled().await;
                Ok(Attempted::Refused)
            }
        },
        lines(),
        LONG,
        &cancel,
    ));
    let first = pool.run_until(futures::future::poll_fn(|cx| {
        std::task::Poll::Ready(running.as_mut().poll(cx).is_ready())
    }));
    assert!(!first);
    assert!(!*dropped.borrow());
    drop(running);
    assert!(*dropped.borrow());
}

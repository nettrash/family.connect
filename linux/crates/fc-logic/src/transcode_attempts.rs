//! The order a transcode is asked for in, and what ends the asking (`TranscodeAttempts.cs`) — the
//! loop around the encoder that needs no encoder (the shell's media preparing runs each attempt;
//! `MediaEncoding` says what the attempts are: on Linux `x264enc` High, then `openh264enc` Main).
//!
//! A REFUSAL IS A REFUSAL WHENEVER IT ARRIVES. An encoder may take a profile when it is prepared
//! and reject the caps only once the pipeline starts, which surfaces as a failure out of the
//! transcode itself. That is the same answer as "cannot transcode" given late, so the next way of
//! asking is tried — the Main profile, the next encoder — exactly as if it had been refused up
//! front. On Windows it first ended the whole loop instead, and the fallbacks the attempts exist
//! for were never reached.
//!
//! ONE CEILING FOR ALL OF THEM. Attempts that each fail late could otherwise cost their ceiling
//! apiece. When the clock runs out nothing more is tried: a transcode that stalled will stall
//! again.
//!
//! THE PERSON'S CANCEL IS NOT A FAILURE. It comes out as `Err(`[`OperationCanceled`]`)`, not as
//! `Ok(None)` — `None` is rule C, which would read the original in and stage it for somebody who
//! has just said they do not want it.
//!
//! How the C#'s tokens are spelt here (linux/docs/port-plan.md, "Async"): the person's
//! `CancellationToken` is an [`rt::Cancel`](Cancel); the linked token each attempt is handed is a
//! [`Cancel`] of its own, raised when the person cancels OR the ceiling ([`Clock::sleep`]) runs
//! out, which the attempt observes part-way (a pipeline set to NULL) and then answers. A C#
//! exception out of an attempt is an [`AttemptFailure`]. Dropping the future returned by
//! [`first_taken`] drops the attempt in hand, which is cancelling it too.
//!
//! ONE DIVERGENCE, deliberately (measured on .NET 10): the C# looks at the person's cancel only
//! before an attempt and when one throws, so a cancel that the LAST attempt swallowed and answered
//! over — refused, wrong, or even a result — came out as `null` or as the result: rule C for
//! somebody who had just said no. Here a cancel raised by the time an attempt answers is a cancel,
//! whatever it answered — what the C# says it is ("a cancel that an attempt swallowed and answered
//! over … is still a cancel").

use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::{Pin, pin};
use std::task::Poll;
use std::time::Duration;

use fc_core::rt::{Cancel, Clock, LocalBoxFuture};

/// How one way of asking for a transcode ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptEnd {
    /// It ran and came out as asked.
    Taken,
    /// This way of asking was not taken — refused, or answered with something else. The next is
    /// tried.
    Refused,
    /// It ran and came out WRONG in a way no other way of asking would change. Nothing more is
    /// tried.
    Wrong,
}

/// One attempt's end, and what it made when it was taken (`Attempted<TResult>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attempted<R> {
    /// It ran and came out as asked: what it made.
    Taken(R),
    /// This way of asking was not taken. The next is tried.
    Refused,
    /// It came out wrong in a way no other way of asking would change. Nothing more is tried.
    Wrong,
}

impl<R> Attempted<R> {
    /// How it ended, without what it made.
    pub fn end(&self) -> AttemptEnd {
        match self {
            Attempted::Taken(_) => AttemptEnd::Taken,
            Attempted::Refused => AttemptEnd::Refused,
            Attempted::Wrong => AttemptEnd::Wrong,
        }
    }
}

/// An attempt that failed outright — what the C# caught as an exception: the kind of failure and
/// its number, which is what tells one encoder refusal from another (a GStreamer error's domain
/// and code). Deliberately no message: an error's own words can quote a path or a caps string,
/// and they never reach the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptFailure {
    /// What failed, as a type name: "InvalidOperationException", "gst-stream-error-quark".
    pub kind: String,
    /// Its number.
    pub code: u32,
}

impl AttemptFailure {
    pub fn new(kind: impl Into<String>, code: u32) -> Self {
        AttemptFailure {
            kind: kind.into(),
            code,
        }
    }
}

impl fmt::Display for AttemptFailure {
    /// "InvalidOperationException 0x80131509" — the C#'s `{Type.Name} 0x{HResult:X8}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} 0x{:08X}", self.kind, self.code)
    }
}

/// The person cancelled (the C#'s `OperationCanceledException`): not a failure, and never staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationCanceled;

impl fmt::Display for OperationCanceled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the operation was cancelled")
    }
}

impl std::error::Error for OperationCanceled {}

/// The log line for a ceiling that ran out.
pub const CEILING_LINE: &str = "a transcode ran past its ceiling";

/// The log line for every way refused.
pub const NONE_TAKEN_LINE: &str = "no transcode profile was taken";

/// The first of `attempts` that is taken, `Ok(None)` when none is — every one refused, one that
/// came out wrong, or the ceiling reached — or `Err(OperationCanceled)` once `cancel` is raised.
/// `say` is told why, for the diagnostics log. Each attempt is handed a [`Cancel`] raised when
/// the person cancels or the ceiling runs out; it is awaited until it answers.
pub async fn first_taken<'a, T, R, F, Fut>(
    attempts: &'a [T],
    mut attempt: F,
    ceiling: Duration,
    clock: &dyn Clock,
    mut say: impl FnMut(&str),
    cancel: &Cancel,
) -> Result<Option<R>, OperationCanceled>
where
    T: fmt::Display,
    F: FnMut(&'a T, Cancel) -> Fut,
    Fut: Future<Output = Result<Attempted<R>, AttemptFailure>>,
{
    // The ceiling is one clock for every attempt together, started now.
    let mut clock_runs = clock.sleep(ceiling);
    let mut ran_out = false;
    let mut person = cancel.cancelled();
    for item in attempts {
        if cancel.is_cancelled() {
            return Err(OperationCanceled);
        }
        if out_of_time(&mut clock_runs, &mut ran_out).await {
            say(CEILING_LINE);
            return Ok(None);
        }
        // The linked token: raised by the person's cancel or by the clock, whichever comes first.
        let either = Cancel::new();
        let mut running = pin!(attempt(item, either.clone()));
        let ended = poll_fn(|cx| {
            if !either.is_cancelled() {
                if !ran_out && clock_runs.as_mut().poll(cx).is_ready() {
                    ran_out = true;
                    either.cancel();
                } else if Pin::new(&mut person).poll(cx).is_ready() {
                    either.cancel();
                }
            }
            running.as_mut().poll(cx)
        })
        .await;
        if cancel.is_cancelled() {
            return Err(OperationCanceled);
        }
        match ended {
            Ok(Attempted::Taken(result)) => return Ok(Some(result)),
            Ok(Attempted::Wrong) => return Ok(None),
            Ok(Attempted::Refused) => {}
            Err(failure) => {
                if out_of_time(&mut clock_runs, &mut ran_out).await {
                    say(CEILING_LINE);
                    return Ok(None);
                }
                // The number is what tells one encoder refusal from another; never the words.
                say(&format!("a transcode failed: {failure} ({item})"));
            }
        }
    }
    say(NONE_TAKEN_LINE);
    Ok(None)
}

/// Whether the clock has run out, asked of the sleep itself — which is never polled again once it
/// has ended.
async fn out_of_time(clock_runs: &mut LocalBoxFuture<'static, ()>, ran_out: &mut bool) -> bool {
    if !*ran_out {
        *ran_out = poll_fn(|cx| Poll::Ready(clock_runs.as_mut().poll(cx).is_ready())).await;
    }
    *ran_out
}

//! The words a call record is drawn with (docs/protocol.md, "Voice calls"),
//! ported from ios/FamilyConnect/Core/Calls/CallRecordText.swift.
//!
//! The server writes an English placeholder into a record's body for
//! clients that predate calls. A client that knows the `call` object never
//! shows that body: it draws its OWN wording from the outcome, the
//! duration, and which side of the call the reader was on. One function for
//! the bubble, the chat-list preview and any notification, so the three can
//! never say different things about the same call.
//!
//! The wording is decided as a [`CallRecordLine`] — which sentence, with
//! which parameters — and rendered by [`CallRecordLine::english`], so a
//! translation table can be added beside it later without touching the
//! decision.

/// The four outcomes the wire names (`CallDTO.Outcome`). Anything else is
/// an outcome this build does not know, and is still drawn as a call.
pub mod outcome {
    pub const COMPLETED: &str = "completed";
    pub const MISSED: &str = "missed";
    pub const DECLINED: &str = "declined";
    pub const FAILED: &str = "failed";
}

/// Which sentence a call record is drawn with.
///
/// `is_mine` throughout is whether the READER placed the call — a record's
/// sender is the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallRecordLine {
    /// Answered and hung up: "Voice call · 3:42", or "Voice call" when the
    /// record carries no duration.
    Completed {
        video: bool,
        duration_secs: Option<i64>,
    },
    /// I called and nobody answered: "No answer" — kind-neutral, because
    /// nobody answering is the whole of the news.
    NoAnswer,
    /// They called and I never answered: "Missed voice call".
    Missed { video: bool },
    /// They said no to my call: "Voice call declined".
    DeclinedByThem { video: bool },
    /// I said no to theirs: "Declined voice call".
    DeclinedByMe { video: bool },
    /// The media never came up, or died: "Call failed", with "· 1:01" when
    /// it was ever up. Kind-neutral on purpose — a call that failed is a
    /// failure first.
    Failed { duration_secs: Option<i64> },
    /// An outcome this build does not know: still a call, and the render
    /// floor says never draw nothing — "Voice call" / "Video call". (Android
    /// draws "Call failed" here instead.)
    Unknown { video: bool },
}

impl CallRecordLine {
    /// Decide the sentence for a record — `CallRecordText.label`'s switch.
    pub fn new(outcome: &str, duration_secs: Option<i64>, video: bool, is_mine: bool) -> Self {
        match outcome {
            outcome::COMPLETED => CallRecordLine::Completed {
                video,
                duration_secs,
            },
            outcome::MISSED if is_mine => CallRecordLine::NoAnswer,
            outcome::MISSED => CallRecordLine::Missed { video },
            outcome::DECLINED if is_mine => CallRecordLine::DeclinedByThem { video },
            outcome::DECLINED => CallRecordLine::DeclinedByMe { video },
            outcome::FAILED => CallRecordLine::Failed { duration_secs },
            _ => CallRecordLine::Unknown { video },
        }
    }

    /// The sentence in English — the Apple string catalog's source strings.
    pub fn english(&self) -> String {
        let kind = |video: bool| if video { "Video call" } else { "Voice call" };
        match *self {
            CallRecordLine::Completed {
                video,
                duration_secs: Some(secs),
            } => format!("{} · {}", kind(video), duration(secs)),
            CallRecordLine::Completed {
                video,
                duration_secs: None,
            }
            | CallRecordLine::Unknown { video } => kind(video).to_string(),
            CallRecordLine::NoAnswer => "No answer".to_string(),
            CallRecordLine::Missed { video: false } => "Missed voice call".to_string(),
            CallRecordLine::Missed { video: true } => "Missed video call".to_string(),
            CallRecordLine::DeclinedByThem { video } => format!("{} declined", kind(video)),
            CallRecordLine::DeclinedByMe { video: false } => "Declined voice call".to_string(),
            CallRecordLine::DeclinedByMe { video: true } => "Declined video call".to_string(),
            CallRecordLine::Failed {
                duration_secs: Some(secs),
            } => format!("Call failed · {}", duration(secs)),
            CallRecordLine::Failed {
                duration_secs: None,
            } => "Call failed".to_string(),
        }
    }
}

/// The one line a record is drawn with, in English —
/// `CallRecordText.label(outcome:durationSecs:video:isMine:)`.
pub fn label(outcome: &str, duration_secs: Option<i64>, video: bool, is_mine: bool) -> String {
    CallRecordLine::new(outcome, duration_secs, video, is_mine).english()
}

/// `3:42`, or `1:03:42` past an hour — the same shape the in-call timer
/// counts in. A negative duration is drawn as none at all.
pub fn duration(seconds: i64) -> String {
    let whole = seconds.max(0);
    let hours = whole / 3600;
    let minutes = (whole % 3600) / 60;
    let secs = whole % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_wording() {
        let completed = |mine| label("completed", Some(222), false, mine);
        assert_eq!(completed(true), "Voice call · 3:42");
        assert_eq!(completed(false), "Voice call · 3:42");

        assert_eq!(label("missed", None, false, true), "No answer");
        assert_eq!(label("missed", None, false, false), "Missed voice call");

        assert_eq!(label("declined", None, false, true), "Voice call declined");
        assert_eq!(label("declined", None, false, false), "Declined voice call");

        assert_eq!(label("failed", None, false, true), "Call failed");
        assert_eq!(
            label("failed", Some(61), false, false),
            "Call failed · 1:01"
        );
        // The render floor: an outcome this build does not know is still a call.
        assert_eq!(label("hologram", None, false, false), "Voice call");
    }

    #[test]
    fn video_record_wording() {
        assert_eq!(
            label("completed", Some(222), true, true),
            "Video call · 3:42"
        );
        assert_eq!(
            label("completed", Some(222), true, false),
            "Video call · 3:42"
        );
        assert_eq!(label("completed", None, true, true), "Video call");

        assert_eq!(label("missed", None, true, true), "No answer");
        assert_eq!(label("missed", None, true, false), "Missed video call");

        assert_eq!(label("declined", None, true, true), "Video call declined");
        assert_eq!(label("declined", None, true, false), "Declined video call");

        // Failure wording is kind-neutral on purpose.
        assert_eq!(label("failed", None, true, true), "Call failed");
        // The render floor keeps the kind.
        assert_eq!(label("hologram", None, true, false), "Video call");
    }

    #[test]
    fn durations() {
        assert_eq!(duration(0), "0:00");
        assert_eq!(duration(7), "0:07");
        assert_eq!(duration(222), "3:42");
        assert_eq!(duration(3661), "1:01:01");
        assert_eq!(duration(-5), "0:00");
    }

    /// The decision, apart from its words — what a translation table and an
    /// icon would be keyed by.
    #[test]
    fn the_line_is_decided_before_it_is_worded() {
        assert_eq!(
            CallRecordLine::new("completed", None, false, true),
            CallRecordLine::Completed {
                video: false,
                duration_secs: None
            }
        );
        assert_eq!(
            CallRecordLine::new("missed", Some(3), true, true),
            CallRecordLine::NoAnswer
        );
        assert_eq!(
            CallRecordLine::new("declined", None, true, true),
            CallRecordLine::DeclinedByThem { video: true }
        );
        assert_eq!(
            CallRecordLine::new("failed", Some(12), true, false),
            CallRecordLine::Failed {
                duration_secs: Some(12)
            }
        );
        assert_eq!(
            CallRecordLine::new("video", Some(12), true, false),
            CallRecordLine::Unknown { video: true }
        );
        // Outcomes are the wire's exact lowercase words.
        assert_eq!(label("Completed", Some(9), false, true), "Voice call");
        // Past Swift's `%d` there is nothing to truncate.
        assert_eq!(duration(i64::MAX), "2562047788015215:30:07");
    }
}

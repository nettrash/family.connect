//! "Voice message not sent" (`NotSent.cs`; docs/audio-video-messages-2026-10-04.md, S2.8 and S4):
//! a recording stopped by anything but the person waits in its own row, with the reply it was
//! recorded under and any caption, to be sent or deleted.
//!
//! INTERRUPTIONS STOP AND KEEP, AND NEVER SEND OR DISCARD (decision 12) — a call, the window hidden,
//! minimised or closed, the session locking or the computer sleeping, leaving the chat, the
//! recorder failing. Under a second there is nothing worth keeping, and it goes without a word. A
//! sign-out takes everything recorded and not sent with it.
//!
//! FIVE MINUTES STOPS INTO REVIEW, NEVER INTO A SEND (S2.5), and says so: nothing is ever sent by a
//! limit running out. ONLY THE PERSON'S SEND SENDS — the slot's arrow while a recording runs — and
//! only a second or more of it: a shorter one is "too short", never a blip in the family chat.
//!
//! A NOTE IN REVIEW BECOMES "NOT SENT" ONLY WHEN THE PERSON LEAVES THE CHAT — or the window really
//! closes, which takes the chat with it — taking the words in the field as its caption and leaving
//! the field empty, so a recording nobody finished deciding about can never ride out with the next
//! text. Everything else keeps it where it is.

use std::time::Duration;

use fc_core::text::string_catalog::StringCatalog;

use crate::voice_notes;

/// What ended a recording that was running — the person, the five-minute limit, or something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingEnd {
    /// The person's Stop — the row's Stop button, Esc, the slot in row 3, a second Ctrl+Shift+R.
    Stopped,
    /// The person's Send: the slot's arrow in row 2, which stops the recording and sends it (S2.5).
    Sent,
    /// Five minutes: the recording's own limit.
    Capped,
    /// The person's Cancel — after its question, where it asks one.
    Deleted,
    /// Leaving the chat: another chat, the rail's Board, Family or Settings, a notification opening
    /// another chat.
    LeftChat,
    /// A call rang, started or was placed — in any phase.
    Call,
    /// The window minimised, or hidden to the top bar by its close button with Keep running on.
    WindowHidden,
    /// The window really closing: Quit, or the close button with Keep running off.
    WindowClosed,
    /// The session locked or went away, the screen saver started, or the computer is going to
    /// sleep.
    SessionLocked,
    /// The recorder failed: the microphone gone or taken, the disk full.
    RecorderFailed,
    /// The session ended — signed out, expired, the family left, the account deleted.
    SignedOut,
}

impl RecordingEnd {
    /// Every ending, in declaration order (the C#'s `Enum.GetValues<RecordingEnd>()`).
    pub const ALL: [RecordingEnd; 11] = [
        RecordingEnd::Stopped,
        RecordingEnd::Sent,
        RecordingEnd::Capped,
        RecordingEnd::Deleted,
        RecordingEnd::LeftChat,
        RecordingEnd::Call,
        RecordingEnd::WindowHidden,
        RecordingEnd::WindowClosed,
        RecordingEnd::SessionLocked,
        RecordingEnd::RecorderFailed,
        RecordingEnd::SignedOut,
    ];
}

/// Where a recording goes when it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingFate {
    /// Staged beside the composer, to listen to, caption and send — or throw away.
    Review,
    /// Kept on this device as a voice message that was not sent (S2.8), never sent by anything but
    /// its own row.
    Park,
    /// Gone.
    Discard,
    /// Sent now, through the outbox, with the reply the composer is primed with — the slot's Send
    /// and nothing else.
    Send,
}

/// A recording's end: where it goes, and what the composer says about it, if anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingOutcome {
    pub fate: RecordingFate,
    pub sentence: Option<String>,
}

impl RecordingOutcome {
    pub fn new(fate: RecordingFate, sentence: Option<&str>) -> Self {
        RecordingOutcome {
            fate,
            sentence: sentence.map(str::to_string),
        }
    }
}

/// What a recording takes from the composer as it is kept: the caption and the reply it goes with,
/// and which of the two the composer loses with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerTake {
    pub caption: Option<String>,
    pub reply_to: Option<i64>,
    pub clears_words: bool,
    pub clears_reply: bool,
}

impl ComposerTake {
    /// It takes nothing.
    pub const NOTHING: ComposerTake = ComposerTake {
        caption: None,
        reply_to: None,
        clears_words: false,
        clears_reply: false,
    };
}

/// Where a recording that has just ended goes, and what is said about it. `recorded`: how long it
/// ran, by the recorder's own clock. `bytes`: how much could be read back — `None` when nothing
/// could.
pub fn ended(
    why: RecordingEnd,
    recorded: Duration,
    bytes: Option<usize>,
    say: &dyn StringCatalog,
) -> RecordingOutcome {
    let readable = bytes.is_some_and(|length| length > voice_notes::NOTHING_AT_OR_BELOW_BYTES);
    let worth_keeping = readable && recorded >= voice_notes::SHORTEST;
    // A second or more that cannot be read back is a recording lost, and the person is told rather
    // than left to find nothing; under a second there was nothing to lose.
    let lost = if recorded >= voice_notes::SHORTEST {
        Some(say.get("The recording stopped unexpectedly.").to_string())
    } else {
        None
    };
    let outcome = |fate, sentence: Option<String>| RecordingOutcome { fate, sentence };
    match why {
        RecordingEnd::Deleted | RecordingEnd::SignedOut => outcome(RecordingFate::Discard, None),
        RecordingEnd::Stopped | RecordingEnd::Sent => {
            if worth_keeping {
                let fate = if why == RecordingEnd::Sent {
                    RecordingFate::Send
                } else {
                    RecordingFate::Review
                };
                outcome(fate, None)
            } else {
                let sentence =
                    lost.unwrap_or_else(|| say.get("That recording was too short.").to_string());
                outcome(RecordingFate::Discard, Some(sentence))
            }
        }
        RecordingEnd::Capped => {
            if worth_keeping {
                outcome(
                    RecordingFate::Review,
                    Some(say.get("Recording stopped at five minutes.").to_string()),
                )
            } else {
                outcome(RecordingFate::Discard, lost)
            }
        }
        RecordingEnd::RecorderFailed => outcome(
            if worth_keeping {
                RecordingFate::Park
            } else {
                RecordingFate::Discard
            },
            Some(say.get("The recording stopped unexpectedly.").to_string()),
        ),
        RecordingEnd::LeftChat
        | RecordingEnd::Call
        | RecordingEnd::WindowHidden
        | RecordingEnd::WindowClosed
        | RecordingEnd::SessionLocked => {
            if worth_keeping {
                outcome(RecordingFate::Park, None)
            } else {
                outcome(RecordingFate::Discard, lost)
            }
        }
    }
}

/// Whether this ending also turns a voice note still in REVIEW into a not-sent one: leaving the
/// chat, and the window really closing. A call, a hidden window, a locked session and a failed
/// recorder leave it where it is.
pub fn parks_review(why: RecordingEnd) -> bool {
    matches!(why, RecordingEnd::LeftChat | RecordingEnd::WindowClosed)
}

/// What a RUNNING recording takes as an interruption parks it: the reply the composer is primed
/// with — which leaves the composer only when nothing else in it would have carried it — and never
/// the words, which stay the chat's own. An edit has no reply, and gives nothing.
///
/// `words`: the composer's text. `staged`: something is staged beside the words.
pub fn for_recording(
    words: &str,
    reply_to: Option<i64>,
    editing: bool,
    staged: bool,
) -> ComposerTake {
    let Some(reply) = reply_to.filter(|_| !editing) else {
        return ComposerTake::NOTHING;
    };
    // `string.IsNullOrWhiteSpace`: .NET's white space and Rust's `char::is_whitespace` are the same
    // set (Unicode's White_Space).
    let nothing_else = words.trim().is_empty() && !staged;
    ComposerTake {
        caption: None,
        reply_to: Some(reply),
        clears_words: false,
        clears_reply: nothing_else,
    }
}

/// What a voice note in REVIEW takes as the person leaves the chat (S2.8): the words in the field as
/// its caption and the reply with them, and the composer is left empty — `takeComposer`'s caption
/// and reply together. An edit's words are the edit's, never a caption.
pub fn for_review(words: &str, reply_to: Option<i64>, editing: bool) -> ComposerTake {
    if editing {
        return ComposerTake::NOTHING;
    }
    let caption = if words.trim().is_empty() {
        None
    } else {
        Some(words.trim_end().to_string())
    };
    ComposerTake {
        clears_words: caption.is_some(),
        caption,
        reply_to,
        clears_reply: reply_to.is_some(),
    }
}

/// What a screen reader is told once a recording has gone where it went (S6, S2.5) — "Voice message
/// sent", "Ready to review, 0:42", "Recording deleted" — and nothing for what an interruption kept
/// or for an ending whose sentence the notice line already says and announces
/// ([`RecordingOutcome::sentence`]).
///
/// `went`: where it really went — a send that could not be written down lands in review instead.
/// `duration_ms`: its length, as it is kept.
pub fn said(
    why: RecordingEnd,
    went: RecordingFate,
    duration_ms: i32,
    say: &dyn StringCatalog,
) -> Option<String> {
    match (why, went) {
        (RecordingEnd::Sent, RecordingFate::Send) => {
            Some(say.get("Voice message sent").to_string())
        }
        (RecordingEnd::Stopped, RecordingFate::Review) => Some(say.format(
            "Ready to review, %@",
            &[&fc_text::media::time_label(f64::from(duration_ms) / 1000.0)],
        )),
        (RecordingEnd::Deleted, RecordingFate::Discard) => {
            Some(say.get("Recording deleted").to_string())
        }
        _ => None,
    }
}

/// Whether this ending gives the keyboard back to the text field (S2.4): the person's own Send, Stop
/// and Delete — so a second Enter cannot open the microphone again, and a note in review gets its
/// caption where it is typed. An interruption leaves focus where it was.
pub fn gives_focus_back(why: RecordingEnd) -> bool {
    matches!(
        why,
        RecordingEnd::Sent | RecordingEnd::Stopped | RecordingEnd::Deleted
    )
}

/// What follows a not-sent row's own Send or ✕, whose button goes with its row (S2.8, S6): the
/// keyboard goes back to the text field — never left nowhere, which loses a screen reader's place
/// (WCAG 2.4.3) — and what happened is said: "Voice message sent", "Recording deleted". `sent`: the
/// row's Send took it, rather than its ✕. Answers (focus the field, what to say).
pub fn row_ended(sent: bool, duration_ms: i32, say: &dyn StringCatalog) -> (bool, String) {
    let (why, went) = if sent {
        (RecordingEnd::Sent, RecordingFate::Send)
    } else {
        (RecordingEnd::Deleted, RecordingFate::Discard)
    };
    (
        gives_focus_back(why),
        said(why, went, duration_ms, say).unwrap_or_default(),
    )
}

/// "Voice message not sent · 0:42" — the row's own line.
pub fn line(duration_ms: i32, say: &dyn StringCatalog) -> String {
    say.format(
        "Voice message not sent · %@",
        &[&fc_text::media::time_label(f64::from(duration_ms) / 1000.0)],
    )
}

//! `NotSentTests`: "Voice message not sent" (docs/audio-video-messages-2026-10-04.md, S2.8) and
//! the voice columns of S4's table, row by row — what each ending does to a recording that is
//! running, and to one in review. The two C# theories over `Interruptions` loop over its five rows.

mod common;

use std::time::Duration;

use fc_logic::not_sent::{self, ComposerTake, RecordingEnd, RecordingFate, RecordingOutcome};
use fc_logic::voice_notes;

const READABLE: Option<usize> = Some(8 * 1024);

const LONG: Duration = Duration::from_secs(42);

/// Every way something other than the person stops a recording, but a failure and a sign-out.
const INTERRUPTIONS: [RecordingEnd; 5] = [
    RecordingEnd::LeftChat,
    RecordingEnd::Call,
    RecordingEnd::WindowHidden,
    RecordingEnd::WindowClosed,
    RecordingEnd::SessionLocked,
];

fn outcome(fate: RecordingFate, sentence: Option<&str>) -> RecordingOutcome {
    RecordingOutcome::new(fate, sentence)
}

fn take(caption: Option<&str>, reply_to: Option<i64>, words: bool, reply: bool) -> ComposerTake {
    ComposerTake {
        caption: caption.map(str::to_string),
        reply_to,
        clears_words: words,
        clears_reply: reply,
    }
}

#[test]
fn their_stop_goes_to_review() {
    let say = common::english();
    assert_eq!(
        outcome(RecordingFate::Review, None),
        not_sent::ended(RecordingEnd::Stopped, LONG, READABLE, &say)
    );
    assert_eq!(
        outcome(RecordingFate::Review, None),
        not_sent::ended(
            RecordingEnd::Stopped,
            Duration::from_secs(1),
            READABLE,
            &say
        )
    );
}

#[test]
fn their_stop_under_a_second_is_too_short() {
    let say = common::english();
    let too_short = outcome(
        RecordingFate::Discard,
        Some("That recording was too short."),
    );
    assert_eq!(
        too_short,
        not_sent::ended(
            RecordingEnd::Stopped,
            Duration::from_millis(999),
            READABLE,
            &say
        )
    );
    assert_eq!(
        too_short,
        not_sent::ended(
            RecordingEnd::Stopped,
            Duration::from_millis(400),
            None,
            &say
        )
    );
}

/// The slot's Send in row 2 SENDS — the one ending that does (S2.5) — with a second or more;
/// shorter is "too short" and gone, never a blip in the family chat; and what cannot be read back is
/// said to have stopped, never sent empty.
#[test]
fn their_send_sends_a_second_or_more() {
    let say = common::english();
    assert_eq!(
        outcome(RecordingFate::Send, None),
        not_sent::ended(RecordingEnd::Sent, LONG, READABLE, &say)
    );
    assert_eq!(
        outcome(RecordingFate::Send, None),
        not_sent::ended(RecordingEnd::Sent, Duration::from_secs(1), READABLE, &say)
    );
    let too_short = outcome(
        RecordingFate::Discard,
        Some("That recording was too short."),
    );
    assert_eq!(
        too_short,
        not_sent::ended(
            RecordingEnd::Sent,
            Duration::from_millis(999),
            READABLE,
            &say
        )
    );
    assert_eq!(
        too_short,
        not_sent::ended(RecordingEnd::Sent, Duration::from_millis(150), None, &say)
    );
    assert_eq!(
        outcome(
            RecordingFate::Discard,
            Some("The recording stopped unexpectedly.")
        ),
        not_sent::ended(RecordingEnd::Sent, LONG, None, &say)
    );
    // A Send keeps nothing back for review, and a note in review is not parked by one.
    assert!(!not_sent::parks_review(RecordingEnd::Sent));
}

/// What a screen reader is told once a recording went where it went (S6): its Send sent, its Stop
/// in review with the length, its Delete gone — and nothing for an interruption, whose row is right
/// there, or for an ending that did not go where it was meant to (a send kept in review is said by
/// the notice line instead).
#[test]
fn what_they_did_is_said() {
    let say = common::english();
    let said = |why, went, ms| not_sent::said(why, went, ms, &say);
    assert_eq!(
        Some("Voice message sent".to_string()),
        said(RecordingEnd::Sent, RecordingFate::Send, 42_000)
    );
    assert_eq!(
        Some("Ready to review, 0:42".to_string()),
        said(RecordingEnd::Stopped, RecordingFate::Review, 42_000)
    );
    assert_eq!(
        Some("Ready to review, 0:43".to_string()),
        said(RecordingEnd::Stopped, RecordingFate::Review, 42_500)
    );
    assert_eq!(
        Some("Recording deleted".to_string()),
        said(RecordingEnd::Deleted, RecordingFate::Discard, 3_000)
    );
    assert_eq!(
        None,
        said(RecordingEnd::Sent, RecordingFate::Review, 42_000)
    );
    assert_eq!(None, said(RecordingEnd::Sent, RecordingFate::Discard, 400));
    assert_eq!(
        None,
        said(RecordingEnd::Stopped, RecordingFate::Park, 42_000)
    );
    assert_eq!(
        None,
        said(RecordingEnd::Stopped, RecordingFate::Discard, 400)
    );
    assert_eq!(
        None,
        said(RecordingEnd::Capped, RecordingFate::Review, 300_000)
    );
    // "Recording deleted" only for what really went: a Delete whose recording was kept anywhere is
    // never said gone.
    assert_eq!(
        None,
        said(RecordingEnd::Deleted, RecordingFate::Park, 42_000)
    );
    assert_eq!(
        None,
        said(RecordingEnd::Deleted, RecordingFate::Review, 42_000)
    );
    for why in [
        RecordingEnd::LeftChat,
        RecordingEnd::Call,
        RecordingEnd::WindowHidden,
        RecordingEnd::SessionLocked,
    ] {
        assert_eq!(None, said(why, RecordingFate::Park, 42_000));
    }
    let russian = common::catalogues().for_language("ru");
    assert_ne!(
        Some("Voice message sent".to_string()),
        not_sent::said(RecordingEnd::Sent, RecordingFate::Send, 42_000, &russian)
    );
}

/// The person's own Send, Stop and Delete give the keyboard back to the field (S2.4) — so a second
/// Enter cannot open the microphone again — and an interruption leaves focus where it was.
#[test]
fn their_own_ending_gives_the_keyboard_back_to_the_field() {
    assert!(not_sent::gives_focus_back(RecordingEnd::Sent));
    assert!(not_sent::gives_focus_back(RecordingEnd::Stopped));
    assert!(not_sent::gives_focus_back(RecordingEnd::Deleted));
    for why in [
        RecordingEnd::Capped,
        RecordingEnd::LeftChat,
        RecordingEnd::Call,
        RecordingEnd::WindowHidden,
        RecordingEnd::WindowClosed,
        RecordingEnd::SessionLocked,
        RecordingEnd::RecorderFailed,
        RecordingEnd::SignedOut,
    ] {
        assert!(!not_sent::gives_focus_back(why), "{why:?}");
    }
}

/// Nothing but the person's Send sends: no interruption, no limit, no Stop, no Delete (S1.7,
/// decision 12).
#[test]
fn nothing_but_their_send_sends() {
    let say = common::english();
    for why in RecordingEnd::ALL
        .into_iter()
        .filter(|why| *why != RecordingEnd::Sent)
    {
        assert_ne!(
            RecordingFate::Send,
            not_sent::ended(why, LONG, READABLE, &say).fate
        );
        assert_ne!(
            RecordingFate::Send,
            not_sent::ended(why, voice_notes::LONGEST, READABLE, &say).fate
        );
    }
}

/// Forty seconds that cannot be read back are not "too short": they were lost, and the person is
/// told so.
#[test]
fn a_recording_that_cannot_be_read_back_is_said_to_have_stopped() {
    let say = common::english();
    let lost = outcome(
        RecordingFate::Discard,
        Some("The recording stopped unexpectedly."),
    );
    assert_eq!(
        lost,
        not_sent::ended(RecordingEnd::Stopped, LONG, None, &say)
    );
    assert_eq!(
        lost,
        not_sent::ended(
            RecordingEnd::Stopped,
            LONG,
            Some(voice_notes::NOTHING_AT_OR_BELOW_BYTES),
            &say
        )
    );
    assert_eq!(lost, not_sent::ended(RecordingEnd::Call, LONG, None, &say));
    assert_eq!(
        lost,
        not_sent::ended(RecordingEnd::Capped, voice_notes::LONGEST, None, &say)
    );
}

/// Five minutes stops into REVIEW and says so (S2.5) — never a send, and never a not-sent row.
#[test]
fn five_minutes_stops_into_review_and_says_so() {
    let say = common::english();
    assert_eq!(
        outcome(
            RecordingFate::Review,
            Some("Recording stopped at five minutes.")
        ),
        not_sent::ended(RecordingEnd::Capped, voice_notes::LONGEST, READABLE, &say)
    );
    assert!(voice_notes::is_done(voice_notes::LONGEST));
}

/// S4: a call, leaving the chat, a hidden, minimised or closed window and a locked session stop and
/// KEEP. (A theory over `Interruptions`.)
#[test]
fn an_interruption_stops_and_keeps() {
    let say = common::english();
    for why in INTERRUPTIONS {
        assert_eq!(
            outcome(RecordingFate::Park, None),
            not_sent::ended(why, LONG, READABLE, &say),
            "{why:?}"
        );
        assert_eq!(
            outcome(RecordingFate::Park, None),
            not_sent::ended(why, Duration::from_secs(1), READABLE, &say),
            "{why:?}"
        );
    }
}

/// S4: "Under 1.0 s, 'not sent' below means deleted instead — there is nothing worth keeping" — and
/// nothing is said. (A theory over `Interruptions`.)
#[test]
fn an_interruption_under_a_second_deletes_without_a_word() {
    let say = common::english();
    for why in INTERRUPTIONS {
        assert_eq!(
            outcome(RecordingFate::Discard, None),
            not_sent::ended(why, Duration::from_millis(999), READABLE, &say),
            "{why:?}"
        );
        assert_eq!(
            outcome(RecordingFate::Discard, None),
            not_sent::ended(why, Duration::ZERO, None, &say),
            "{why:?}"
        );
    }
}

/// S4: the recorder failing keeps what is readable as not sent, with the sentence — and says it
/// even when nothing is.
#[test]
fn a_failed_recorder_keeps_what_is_readable_and_says_so() {
    let say = common::english();
    let sentence = Some("The recording stopped unexpectedly.");
    assert_eq!(
        outcome(RecordingFate::Park, sentence),
        not_sent::ended(RecordingEnd::RecorderFailed, LONG, READABLE, &say)
    );
    assert_eq!(
        outcome(RecordingFate::Discard, sentence),
        not_sent::ended(RecordingEnd::RecorderFailed, LONG, None, &say)
    );
    assert_eq!(
        outcome(RecordingFate::Discard, sentence),
        not_sent::ended(
            RecordingEnd::RecorderFailed,
            Duration::from_millis(300),
            READABLE,
            &say
        )
    );
}

/// S4's last row: a sign-out takes everything recorded and not sent with it. The person's Cancel
/// deletes too.
#[test]
fn a_sign_out_and_their_cancel_discard() {
    let say = common::english();
    assert_eq!(
        outcome(RecordingFate::Discard, None),
        not_sent::ended(RecordingEnd::SignedOut, LONG, READABLE, &say)
    );
    assert_eq!(
        outcome(RecordingFate::Discard, None),
        not_sent::ended(RecordingEnd::Deleted, LONG, READABLE, &say)
    );
    assert!(!not_sent::parks_review(RecordingEnd::SignedOut));
}

/// A note in REVIEW becomes not sent only when the person leaves the chat or the window really
/// closes; a call, a hidden window, a locked session and a failed recorder keep it in review (S4's
/// "Review / not sent" column).
#[test]
fn only_leaving_the_chat_or_closing_the_window_parks_a_note_in_review() {
    assert!(not_sent::parks_review(RecordingEnd::LeftChat));
    assert!(not_sent::parks_review(RecordingEnd::WindowClosed));
    for kept in [
        RecordingEnd::Call,
        RecordingEnd::WindowHidden,
        RecordingEnd::SessionLocked,
        RecordingEnd::RecorderFailed,
        RecordingEnd::Stopped,
        RecordingEnd::Capped,
        RecordingEnd::Deleted,
    ] {
        assert!(!not_sent::parks_review(kept), "{kept:?}");
    }
}

/// A note in review takes the words as its caption and the reply with them, and leaves the composer
/// empty (S2.8's `takeComposer`); trailing white space is not part of a caption, as it is not part
/// of a sent body.
#[test]
fn a_note_in_review_takes_the_words_and_the_reply() {
    assert_eq!(
        take(Some("Happy birthday 🎂"), Some(7), true, true),
        not_sent::for_review("Happy birthday 🎂  \n", Some(7), false)
    );
    assert_eq!(
        take(Some("  indented"), None, true, false),
        not_sent::for_review("  indented", None, false)
    );
    assert_eq!(
        take(None, Some(7), false, true),
        not_sent::for_review("   ", Some(7), false)
    );
    assert_eq!(ComposerTake::NOTHING, not_sent::for_review("", None, false));
}

/// An edit's words are the edit's: never a caption, and an edit has no reply to give.
#[test]
fn an_edit_gives_nothing() {
    assert_eq!(
        ComposerTake::NOTHING,
        not_sent::for_review("the corrected words", Some(7), true)
    );
    assert_eq!(
        ComposerTake::NOTHING,
        not_sent::for_recording("the corrected words", Some(7), true, false)
    );
}

/// A RUNNING recording takes the reply it was recorded under, and never the words; the reply leaves
/// the composer only when nothing else there would carry it.
#[test]
fn a_running_recording_takes_the_reply_and_leaves_the_words() {
    assert_eq!(
        take(None, Some(7), false, true),
        not_sent::for_recording("", Some(7), false, false)
    );
    assert_eq!(
        take(None, Some(7), false, true),
        not_sent::for_recording(" \n", Some(7), false, false)
    );
    assert_eq!(
        take(None, Some(7), false, false),
        not_sent::for_recording("look at this", Some(7), false, false)
    );
    assert_eq!(
        take(None, Some(7), false, false),
        not_sent::for_recording("", Some(7), false, true)
    );
    assert_eq!(
        ComposerTake::NOTHING,
        not_sent::for_recording("look at this", None, false, false)
    );
}

#[test]
fn the_row_says_how_long_the_recording_is() {
    let say = common::english();
    assert_eq!(
        "Voice message not sent · 0:42",
        not_sent::line(42_000, &say)
    );
    assert_eq!("Voice message not sent · 0:01", not_sent::line(1_000, &say));
    assert_eq!(
        "Voice message not sent · 0:43",
        not_sent::line(42_500, &say)
    );
    assert_eq!(
        "Voice message not sent · 5:00",
        not_sent::line(300_000, &say)
    );
}

/// The row reads in the reader's language, its time where the translation puts it.
#[test]
fn the_row_is_translated() {
    let russian = common::catalogues().for_language("ru");
    assert_ne!(
        "Voice message not sent · 0:42",
        not_sent::line(42_000, &russian)
    );
    assert!(not_sent::line(42_000, &russian).contains("0:42"));
}

/// A NOT-SENT ROW'S OWN BUTTONS GO WITH IT (S2.8, S6): its Send and its ✕ each give the keyboard
/// back to the field — never nowhere, which loses a screen reader's place (WCAG 2.4.3) — and say
/// what happened.
#[test]
fn a_rows_send_or_delete_gives_the_keyboard_back_and_says_so() {
    let say = common::english();
    assert_eq!(
        (true, "Voice message sent".to_string()),
        not_sent::row_ended(true, 42_000, &say)
    );
    assert_eq!(
        (true, "Recording deleted".to_string()),
        not_sent::row_ended(false, 42_000, &say)
    );
    let russian = common::catalogues().for_language("ru");
    assert_ne!(
        "Voice message sent",
        not_sent::row_ended(true, 42_000, &russian).1
    );
}

//! The text of a recording, on request (docs/protocol.md, "Transcripts on
//! request"; issue #62).
//!
//! A member presses "Show text" under a voice note, an audio file or a
//! video, and the recording's sound goes to the server's transcription
//! deployment: its STORED bytes where the server sends them as they are, and
//! otherwise sound this device takes out of the file it holds and sends with
//! the request ([`crate::transcript_sound`]). The answer comes back to
//! whoever asked — nobody else is sent anything. Three decisions live here,
//! so that every port can be held to the same answers:
//!
//! - [`offers_show_text`]: whether the action is drawn at all — the
//!   server's capability, the server's allowed rule as far as a client can
//!   know it ([`may_ask`]), and whether there is a [`Source`] for the sound:
//!   the stored bytes ([`stored_bytes_qualify`]), or this device
//!   ([`device_may_supply`]) for a video, an Ogg file or an audio file over
//!   the server's ceiling.
//! - [`after_refusal`]: what each refusal means to the person who asked.
//! - [`Transcripts`]: what this device keeps of the answers it was given —
//!   per attachment, so reopening a chat shows them without asking again.

use std::collections::HashMap;

use crate::assistant_consent;
use crate::i18n::t;
use crate::media;

/// `assistant.transcribe_max_bytes` when a server leaves it out — its
/// default, and its ceiling: 25 MiB, the provider's own limit.
pub const DEFAULT_MAX_BYTES: i64 = 26_214_400;

/// The stored types the server sends to the provider as they are — the
/// server's `STORED_AUDIO` (server/src/handlers_transcript.rs). `audio/ogg`
/// is accepted on upload and is NOT here: the provider refuses Ogg, so its
/// text needs sound the device supplies ([`Source::Device`]).
pub const STORED_TYPES: [&str; 4] = ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav"];

/// What the server says it can do, from the `assistant` object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Server<'a> {
    /// `assistant.transcribe`.
    pub transcribe: bool,
    /// `assistant.transcribe_max_bytes`, present only while `transcribe`.
    pub max_bytes: Option<i64>,
    /// `assistant.processor` — who receives the sound, which the consent
    /// screen must name. A server that names nobody is offered nothing.
    pub processor: Option<&'a str>,
}

impl Server<'_> {
    /// The most bytes the server sends, as it said or by its default.
    pub fn max_bytes(&self) -> i64 {
        self.max_bytes
            .filter(|bytes| *bytes > 0)
            .unwrap_or(DEFAULT_MAX_BYTES)
    }
}

/// Who asks about whose message, and where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asking<'a> {
    /// `family` | `direct` | `ai` — a thread's replies live in the chat of
    /// their root, so a family-chat thread is `family`.
    pub chat_kind: &'a str,
    pub sender_id: i64,
    pub my_user_id: i64,
    /// The assistant's account, when the server has one.
    pub assistant_user_id: Option<i64>,
    /// The family's `ai_transcripts`, the owner's switch.
    pub family_allows: bool,
}

/// One attachment, as its metadata describes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recording<'a> {
    pub id: i64,
    pub kind: &'a str,
    pub mime: Option<&'a str>,
    pub size: Option<i64>,
}

/// The server's allowed rule, as far as a client can know it.
///
/// A member may ask about a message they sent themselves, in any chat they
/// are in, direct chats included; about another member's message only in
/// the family chat (its threads included), and only while the family's
/// `ai_transcripts` is on; never about another member's message in a direct
/// chat; never about the assistant's own. The server also asks whether the
/// SENDER has agreed to the assistant, which no client can see — it answers
/// `transcript_not_allowed` then, and that is drawn as "Not available for
/// this message.".
pub fn may_ask(asking: &Asking) -> bool {
    if asking.my_user_id <= 0 {
        return false;
    }
    if asking.assistant_user_id == Some(asking.sender_id) {
        return false;
    }
    if asking.sender_id == asking.my_user_id {
        return true;
    }
    asking.chat_kind == "family" && asking.family_allows
}

/// Whether the server sends this attachment's STORED bytes as they are: a
/// voice note or audio file, of a type the provider reads, within the
/// server's ceiling. A size the metadata does not carry is the server's to
/// judge. An id that is not the server's yet (a bubble still being sent)
/// has nothing to ask about.
pub fn stored_bytes_qualify(recording: &Recording, max_bytes: i64) -> bool {
    recording.id > 0
        && recording.kind == "audio"
        && recording
            .mime
            .map(media::essence)
            .is_some_and(|mime| STORED_TYPES.contains(&mime.as_str()))
        && recording.size.is_none_or(|size| size <= max_bytes)
}

/// Where the sound for a recording's text comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The server sends its stored copy — the request has no body, and its
    /// answer is kept and handed to later askers.
    Stored,
    /// This device takes the sound out of the file it holds and sends it
    /// with the request ([`crate::transcript_sound`]); the answer is the
    /// asker's alone, and only this device keeps it.
    Device,
}

/// Whether this device can be the source of a recording's sound: a video
/// or a sound file the server holds — whatever its type and size, because
/// what the file holds is only known once it is read, and a recording
/// whose sound cannot be made is told so after the press
/// ([`Failure::TooLong`], [`Failure::Unreadable`]) rather than guessed at
/// before it. A voice note, an audio file and a video are what the server
/// takes supplied sound for; a photo or a file is neither.
pub fn device_may_supply(recording: &Recording) -> bool {
    recording.id > 0 && matches!(recording.kind, "audio" | "video")
}

/// The source of a recording's sound — the server's stored copy wherever it
/// will do, because that answer costs one provider call for the whole
/// family; this device otherwise; None for an attachment with no sound to
/// ask about.
pub fn source(recording: &Recording, max_bytes: i64) -> Option<Source> {
    if stored_bytes_qualify(recording, max_bytes) {
        Some(Source::Stored)
    } else if device_may_supply(recording) {
        Some(Source::Device)
    } else {
        None
    }
}

/// Is "Show text" drawn under this recording, for this member?
pub fn offers_show_text(server: &Server, asking: &Asking, recording: &Recording) -> bool {
    server.transcribe
        && assistant_consent::is_available(server.processor)
        && may_ask(asking)
        && source(recording, server.max_bytes()).is_some()
}

/// Must this member be asked before the sound goes? The asker is the one
/// sending a recording to the provider, so their own consent to the
/// assistant is required — even when the server already keeps an answer.
pub fn asks_for_consent(processor: Option<&str>, agreed: bool) -> bool {
    assistant_consent::is_available(processor) && !agreed
}

/// The text the device was given.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transcript {
    /// Always present; `""` is silence — an answer, not a failure.
    pub text: String,
    /// As the provider spells it (`ru`, or `russian`), when it named one.
    pub language: Option<String>,
}

impl Transcript {
    /// Nothing was said: drawn as "No speech".
    pub fn is_silence(&self) -> bool {
        self.text.trim().is_empty()
    }
}

/// Why there is no text to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// `internal`, a timeout, no connection, or anything this client has
    /// not learned: transient, and offered again.
    TryAgain,
    /// `transcript_refused`: the provider's filter. Terminal.
    Refused,
    /// `not_transcribable`, `transcript_not_allowed`,
    /// `transcripts_unavailable`: asking again will not change it.
    NotAvailable,
    /// The sound this device could make of the recording is over the
    /// server's `transcribe_max_bytes`, even re-encoded. Never sent.
    TooLong,
    /// This device could not take the sound out of the file: it has none,
    /// it is in a format this device cannot read, or it needed re-encoding
    /// and this device has no AAC encoder. Never sent.
    Unreadable,
}

impl Failure {
    pub fn sentence(self) -> &'static str {
        match self {
            Failure::TryAgain => t("Couldn't get the text. Try again."),
            Failure::Refused => t("The assistant's provider refused this recording."),
            Failure::NotAvailable => t("Not available for this message."),
            Failure::TooLong => t("This recording is too long to turn into text."),
            Failure::Unreadable => t("Couldn't read the sound in this file."),
        }
    }

    /// Whether asking again could change the answer.
    pub fn may_retry(self) -> bool {
        self == Failure::TryAgain
    }
}

/// What a refusal leads to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// `assistant_consent_required`: nothing was sent, and the answer is
    /// the consent screen — after which the member asks again.
    AskConsent,
    /// A line under the player.
    Fail(Failure),
}

/// What the protocol's error `code` means to the person who asked — `None`
/// for a failure with no code at all (no connection, a timeout, a proxy's
/// bare status), which is worth another try.
pub fn after_refusal(code: Option<&str>) -> Next {
    match code {
        Some("assistant_consent_required") => Next::AskConsent,
        Some("transcript_refused") => Next::Fail(Failure::Refused),
        Some("not_transcribable" | "transcript_not_allowed" | "transcripts_unavailable") => {
            Next::Fail(Failure::NotAvailable)
        }
        _ => Next::Fail(Failure::TryAgain),
    }
}

/// Whether a refusal of the STORED copy sends this device's sound instead:
/// only `not_transcribable` — the server's own reading of the file (its
/// stored type, its size) disagreeing with what the metadata said — and
/// only for an attachment this device could supply sound for. The protocol
/// allows it ("a client that sent no body may send the sound track
/// instead"), and every client does it, so the same recording gets the same
/// answer on every device. Any other refusal is the answer.
pub fn falls_back_to_device(code: Option<&str>, recording: &Recording) -> bool {
    code == Some("not_transcribable") && device_may_supply(recording)
}

/// Where one recording's text stands on this device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// The request is out.
    Asking,
    /// The text, under the player.
    Shown(Transcript),
    /// Folded away by "Hide text" — still held, so showing it again asks
    /// nobody.
    Hidden(Transcript),
    Failed(Failure),
}

impl State {
    /// The text held, shown or not.
    pub fn transcript(&self) -> Option<&Transcript> {
        match self {
            State::Shown(transcript) | State::Hidden(transcript) => Some(transcript),
            _ => None,
        }
    }
}

/// What pressing "Show text" has to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Send the request: nothing is held, or the last try failed.
    Send,
    /// The device holds the text, and it is shown again.
    Held,
    /// A request for it is already out.
    Busy,
}

/// The answers this device was given, by attachment id.
///
/// An answer is the same whoever asks again, so it is kept once per
/// attachment and shown on every later press. A failure is kept too, so
/// that the line under the player outlives the bubble being drawn again;
/// pressing again after one that may be retried asks again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Transcripts {
    by_attachment: HashMap<i64, State>,
}

impl Transcripts {
    pub fn get(&self, attachment_id: i64) -> Option<&State> {
        self.by_attachment.get(&attachment_id)
    }

    /// "Show text" pressed.
    pub fn ask(&mut self, attachment_id: i64) -> Ask {
        match self.by_attachment.get(&attachment_id) {
            Some(State::Asking) => Ask::Busy,
            Some(State::Shown(_)) => Ask::Held,
            Some(State::Hidden(transcript)) => {
                let transcript = transcript.clone();
                self.by_attachment
                    .insert(attachment_id, State::Shown(transcript));
                Ask::Held
            }
            Some(State::Failed(_)) | None => {
                self.by_attachment.insert(attachment_id, State::Asking);
                Ask::Send
            }
        }
    }

    /// The server's answer.
    pub fn answered(&mut self, attachment_id: i64, transcript: Transcript) {
        self.by_attachment
            .insert(attachment_id, State::Shown(transcript));
    }

    pub fn failed(&mut self, attachment_id: i64, failure: Failure) {
        self.by_attachment
            .insert(attachment_id, State::Failed(failure));
    }

    /// Back to nothing asked — the consent screen was the answer.
    pub fn forget(&mut self, attachment_id: i64) {
        self.by_attachment.remove(&attachment_id);
    }

    /// "Hide text" pressed: the text folds away and is kept.
    pub fn hide(&mut self, attachment_id: i64) {
        if let Some(State::Shown(transcript)) = self.by_attachment.get(&attachment_id) {
            let transcript = transcript.clone();
            self.by_attachment
                .insert(attachment_id, State::Hidden(transcript));
        }
    }

    /// What is held for these attachments — one bubble's share.
    pub fn of(&self, attachment_ids: impl IntoIterator<Item = i64>) -> HashMap<i64, State> {
        attachment_ids
            .into_iter()
            .filter_map(|id| self.get(id).map(|state| (id, state.clone())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: i64 = 1;
    const ANNA: i64 = 7;
    const AI: i64 = 2;

    fn asking(chat_kind: &str, sender_id: i64, family_allows: bool) -> Asking<'_> {
        Asking {
            chat_kind,
            sender_id,
            my_user_id: ME,
            assistant_user_id: Some(AI),
            family_allows,
        }
    }

    fn voice_note(mime: &str, size: Option<i64>) -> Recording<'_> {
        Recording {
            id: 34,
            kind: "audio",
            mime: Some(mime),
            size,
        }
    }

    const SERVER: Server<'static> = Server {
        transcribe: true,
        max_bytes: Some(DEFAULT_MAX_BYTES),
        processor: Some("Microsoft — Azure OpenAI"),
    };

    /// The allowed rule, every chat by every sender, both ways of the
    /// owner's switch — the table the server's `allowed()` answers.
    #[test]
    fn the_allowed_rule() {
        // (chat, sender, switch, may ask)
        for (chat, sender, switch, may) in [
            // Your own recording: anywhere, switch or not.
            ("family", ME, false, true),
            ("family", ME, true, true),
            ("direct", ME, false, true),
            ("ai", ME, false, true),
            // Another member's: the family chat (and its threads), with
            // the switch on — and nowhere else.
            ("family", ANNA, true, true),
            ("family", ANNA, false, false),
            ("direct", ANNA, true, false),
            ("direct", ANNA, false, false),
            ("ai", ANNA, true, false),
            // The assistant's own: never.
            ("family", AI, true, false),
            ("ai", AI, true, false),
            // A chat kind from a newer server is not the family chat.
            ("group", ANNA, true, false),
        ] {
            assert_eq!(
                may_ask(&asking(chat, sender, switch)),
                may,
                "{chat} from {sender}, switch {switch}"
            );
        }
    }

    /// Only the server's `not_transcribable` of a stored copy sends the
    /// device's sound — never any other refusal, and never for an
    /// attachment with no sound to make.
    #[test]
    fn only_not_transcribable_falls_back_to_the_device() {
        let voice = voice_note("audio/mp4", Some(1_000));
        assert!(falls_back_to_device(Some("not_transcribable"), &voice));
        for code in [
            Some("transcript_not_allowed"),
            Some("transcripts_unavailable"),
            Some("transcript_refused"),
            Some("assistant_consent_required"),
            Some("internal"),
            None,
        ] {
            assert!(!falls_back_to_device(code, &voice), "{code:?}");
        }
        let photo = Recording {
            id: 34,
            kind: "photo",
            mime: Some("image/jpeg"),
            size: Some(1_000),
        };
        assert!(!falls_back_to_device(Some("not_transcribable"), &photo));
    }

    /// A tab that does not yet know who it is asks about nothing.
    #[test]
    fn nobody_signed_in_asks_about_nothing() {
        let mut nobody = asking("family", 0, true);
        nobody.my_user_id = 0;
        assert!(!may_ask(&nobody));
    }

    #[test]
    fn only_stored_types_the_provider_reads_qualify() {
        for mime in [
            "audio/mp4",
            "audio/m4a",
            "audio/mpeg",
            "audio/wav",
            "AUDIO/MP4; codecs=mp4a.40.2",
        ] {
            assert!(
                stored_bytes_qualify(&voice_note(mime, Some(1000)), DEFAULT_MAX_BYTES),
                "{mime}"
            );
        }
        // Ogg needs sound the device supplies.
        for mime in ["audio/ogg", "audio/webm", "video/mp4", ""] {
            assert!(
                !stored_bytes_qualify(&voice_note(mime, Some(1000)), DEFAULT_MAX_BYTES),
                "{mime}"
            );
        }
        let no_type = Recording {
            mime: None,
            ..voice_note("", None)
        };
        assert!(!stored_bytes_qualify(&no_type, DEFAULT_MAX_BYTES));
    }

    #[test]
    fn a_video_or_a_photo_is_not_offered_from_stored_bytes() {
        for kind in ["video", "photo", "file", "location"] {
            let recording = Recording {
                kind,
                ..voice_note("audio/mp4", Some(10))
            };
            assert!(
                !stored_bytes_qualify(&recording, DEFAULT_MAX_BYTES),
                "{kind}"
            );
        }
    }

    #[test]
    fn the_ceiling_is_the_servers_and_inclusive() {
        let max = 1_000;
        assert!(stored_bytes_qualify(
            &voice_note("audio/mp4", Some(max)),
            max
        ));
        assert!(!stored_bytes_qualify(
            &voice_note("audio/mp4", Some(max + 1)),
            max
        ));
        // No size in the metadata: the server judges it.
        assert!(stored_bytes_qualify(&voice_note("audio/mp4", None), max));
        // Absent, zero or nonsense from the server is its default.
        for said in [None, Some(0), Some(-5)] {
            let server = Server {
                max_bytes: said,
                ..SERVER
            };
            assert_eq!(server.max_bytes(), DEFAULT_MAX_BYTES, "{said:?}");
        }
        assert_eq!(
            Server {
                max_bytes: Some(5_000),
                ..SERVER
            }
            .max_bytes(),
            5_000
        );
    }

    #[test]
    fn a_bubble_still_being_sent_has_nothing_to_ask_about() {
        for id in [0, -3] {
            let recording = Recording {
                id,
                ..voice_note("audio/mp4", Some(10))
            };
            assert!(!stored_bytes_qualify(&recording, DEFAULT_MAX_BYTES));
        }
    }

    #[test]
    fn show_text_needs_the_server_a_named_processor_the_rule_and_the_bytes() {
        let mine = asking("direct", ME, false);
        let note = voice_note("audio/mp4", Some(10_000));
        assert!(offers_show_text(&SERVER, &mine, &note));
        // No transcription deployment.
        let none = Server {
            transcribe: false,
            ..SERVER
        };
        assert!(!offers_show_text(&none, &mine, &note));
        // Nobody named to send it to: no assistant is offered at all.
        for processor in [None, Some(""), Some("  ")] {
            let unnamed = Server {
                processor,
                ..SERVER
            };
            assert!(!offers_show_text(&unnamed, &mine, &note), "{processor:?}");
        }
        // The rule says no.
        assert!(!offers_show_text(
            &SERVER,
            &asking("direct", ANNA, true),
            &note
        ));
        // Bytes the server does not send as they are are still offered:
        // this device supplies the sound.
        assert!(offers_show_text(
            &SERVER,
            &mine,
            &voice_note("audio/ogg", Some(10))
        ));
        let small = Server {
            max_bytes: Some(9_999),
            ..SERVER
        };
        assert!(offers_show_text(&small, &mine, &note));
        // …and so is a video — but never a photo or a file, and never a
        // bubble still being sent.
        let video = Recording {
            kind: "video",
            ..voice_note("video/quicktime", Some(80_000_000))
        };
        assert!(offers_show_text(&SERVER, &mine, &video));
        for kind in ["photo", "file", "location"] {
            let other = Recording {
                kind,
                ..voice_note("image/jpeg", Some(10))
            };
            assert!(!offers_show_text(&SERVER, &mine, &other), "{kind}");
        }
        let sending = Recording { id: -4, ..video };
        assert!(!offers_show_text(&SERVER, &mine, &sending));
        // The rule holds for a video as for a voice note.
        assert!(!offers_show_text(
            &SERVER,
            &asking("direct", ANNA, true),
            &video
        ));
        assert!(offers_show_text(
            &SERVER,
            &asking("family", ANNA, true),
            &video
        ));
        assert!(!offers_show_text(
            &SERVER,
            &asking("family", ANNA, false),
            &video
        ));
    }

    /// THE SOURCE: the server's stored copy wherever it will do — one
    /// provider call for the whole family — and this device otherwise.
    #[test]
    fn the_stored_copy_wherever_it_will_do_and_the_device_otherwise() {
        let max = 1_000;
        // (kind, type, size, source)
        for (kind, mime, size, wanted) in [
            ("audio", Some("audio/mp4"), Some(max), Some(Source::Stored)),
            ("audio", Some("audio/mpeg"), None, Some(Source::Stored)),
            ("audio", Some("audio/wav"), Some(10), Some(Source::Stored)),
            // Over the ceiling, Ogg, or a type the metadata does not give.
            (
                "audio",
                Some("audio/mp4"),
                Some(max + 1),
                Some(Source::Device),
            ),
            ("audio", Some("audio/ogg"), Some(10), Some(Source::Device)),
            ("audio", None, Some(10), Some(Source::Device)),
            // Every video, whatever its size: the server never cuts a
            // video's sound out.
            ("video", Some("video/mp4"), Some(10), Some(Source::Device)),
            (
                "video",
                Some("video/quicktime"),
                Some(max * 90),
                Some(Source::Device),
            ),
            ("video", None, None, Some(Source::Device)),
            // Nothing to hear.
            ("photo", Some("image/jpeg"), Some(10), None),
            ("file", Some("audio/mp4"), Some(10), None),
            ("location", None, None, None),
        ] {
            let recording = Recording {
                id: 34,
                kind,
                mime,
                size,
            };
            assert_eq!(source(&recording, max), wanted, "{kind} {mime:?} {size:?}");
            assert_eq!(
                device_may_supply(&recording),
                matches!(kind, "audio" | "video"),
                "{kind}"
            );
        }
        // A bubble still being sent: neither.
        let sending = Recording {
            id: -2,
            kind: "video",
            mime: Some("video/mp4"),
            size: Some(10),
        };
        assert_eq!(source(&sending, max), None);
    }

    #[test]
    fn the_asker_is_asked_for_consent_before_anything_goes() {
        assert!(asks_for_consent(Some("Azure OpenAI"), false));
        assert!(!asks_for_consent(Some("Azure OpenAI"), true));
        assert!(!asks_for_consent(None, false));
    }

    #[test]
    fn each_refusal_means_what_the_protocol_says() {
        assert_eq!(
            after_refusal(Some("assistant_consent_required")),
            Next::AskConsent
        );
        assert_eq!(
            after_refusal(Some("transcript_refused")),
            Next::Fail(Failure::Refused)
        );
        for code in [
            "not_transcribable",
            "transcript_not_allowed",
            "transcripts_unavailable",
        ] {
            assert_eq!(
                after_refusal(Some(code)),
                Next::Fail(Failure::NotAvailable),
                "{code}"
            );
        }
        // Transient, unknown, or no code at all: worth another try.
        for code in [Some("internal"), Some("something_new"), None] {
            assert_eq!(
                after_refusal(code),
                Next::Fail(Failure::TryAgain),
                "{code:?}"
            );
        }
        assert!(Failure::TryAgain.may_retry());
        for terminal in [
            Failure::Refused,
            Failure::NotAvailable,
            Failure::TooLong,
            Failure::Unreadable,
        ] {
            assert!(!terminal.may_retry(), "{terminal:?}");
        }
    }

    #[test]
    fn the_sentences_are_the_catalogues() {
        assert_eq!(
            Failure::TryAgain.sentence(),
            "Couldn't get the text. Try again."
        );
        assert_eq!(
            Failure::Refused.sentence(),
            "The assistant's provider refused this recording."
        );
        assert_eq!(
            Failure::NotAvailable.sentence(),
            "Not available for this message."
        );
        assert_eq!(
            Failure::TooLong.sentence(),
            "This recording is too long to turn into text."
        );
        assert_eq!(
            Failure::Unreadable.sentence(),
            "Couldn't read the sound in this file."
        );
    }

    #[test]
    fn silence_is_an_answer() {
        for text in ["", " ", "\n\t"] {
            let transcript = Transcript {
                text: text.into(),
                language: None,
            };
            assert!(transcript.is_silence(), "{text:?}");
        }
        assert!(!Transcript {
            text: "hello".into(),
            language: None
        }
        .is_silence());
    }

    fn said(text: &str) -> Transcript {
        Transcript {
            text: text.into(),
            language: Some("en".into()),
        }
    }

    /// Asked once; shown, hidden and shown again without asking again.
    #[test]
    fn the_device_keeps_what_it_was_given() {
        let mut held = Transcripts::default();
        assert_eq!(held.get(34), None);
        assert_eq!(held.ask(34), Ask::Send);
        assert_eq!(held.get(34), Some(&State::Asking));
        // A second press while it is out sends nothing more.
        assert_eq!(held.ask(34), Ask::Busy);
        held.answered(34, said("dinner at seven"));
        assert_eq!(held.get(34), Some(&State::Shown(said("dinner at seven"))));
        held.hide(34);
        assert_eq!(held.get(34), Some(&State::Hidden(said("dinner at seven"))));
        assert_eq!(
            held.get(34).and_then(State::transcript),
            Some(&said("dinner at seven"))
        );
        // Showing it again asks nobody.
        assert_eq!(held.ask(34), Ask::Held);
        assert_eq!(held.get(34), Some(&State::Shown(said("dinner at seven"))));
        assert_eq!(held.ask(34), Ask::Held);
        // Hiding what is not shown does nothing.
        held.hide(99);
        assert_eq!(held.get(99), None);
    }

    #[test]
    fn a_failure_is_kept_and_another_press_asks_again() {
        let mut held = Transcripts::default();
        held.ask(34);
        held.failed(34, Failure::TryAgain);
        assert_eq!(held.get(34), Some(&State::Failed(Failure::TryAgain)));
        assert_eq!(held.get(34).and_then(State::transcript), None);
        assert_eq!(held.ask(34), Ask::Send);
        held.answered(34, said(""));
        assert!(held
            .get(34)
            .and_then(State::transcript)
            .is_some_and(Transcript::is_silence));
    }

    #[test]
    fn the_consent_question_leaves_nothing_behind() {
        let mut held = Transcripts::default();
        held.ask(34);
        held.forget(34);
        assert_eq!(held.get(34), None);
        assert_eq!(held.ask(34), Ask::Send);
    }

    #[test]
    fn one_bubble_reads_only_its_own_attachments() {
        let mut held = Transcripts::default();
        held.answered(34, said("one"));
        held.answered(35, said("two"));
        held.ask(36);
        let share = held.of([34, 36, 40]);
        assert_eq!(share.len(), 2);
        assert_eq!(share[&34], State::Shown(said("one")));
        assert_eq!(share[&36], State::Asking);
    }
}

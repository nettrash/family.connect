//! `TranscriptRulesTests`: who may ask for the text of which recording, and what a failed ask says
//! (docs/protocol.md, "Transcripts on request"). The rule's vectors are the server's own
//! `allowed()` cases, minus the one check a client cannot make (whether the other sender has
//! agreed to the assistant), which the server answers with the same refusal.
//!
//! The rule itself is `fc_text::transcript::may_ask`, called; every case here answers the same
//! there. The one place it differs from the C# — a reader not known yet, in the family chat, with
//! the owner's switch on — is pinned by the last test, which is Linux's own.

mod common;

use fc_core::protocol::{ApiError, AssistantDto, AttachmentDto, FamilyDto, error_codes};
use fc_core::text::string_catalog::StringCatalog;
use fc_logic::transcript_sound::{self, TranscriptForm};
use fc_logic::transcripts::{TranscriptLook, TranscriptPhase, transcript_rules};

const ME: i64 = 7;
const ANNA: i64 = 9;
const ASSISTANT: i64 = 1;
const MIB25: i64 = 26_214_400;

/// `new AssistantDto(Assistant, "Assistant", "@ai", Processor: "Microsoft — Azure OpenAI",
/// Transcribe: true, TranscribeMaxBytes: MiB25)`.
fn transcribes() -> AssistantDto {
    AssistantDto {
        user_id: ASSISTANT,
        display_name: "Assistant".to_string(),
        mention: "@ai".to_string(),
        processor: Some("Microsoft — Azure OpenAI".to_string()),
        transcribe: true,
        transcribe_max_bytes: Some(MIB25),
        ..AssistantDto::default()
    }
}

fn switch_on() -> FamilyDto {
    FamilyDto {
        ai_transcripts: true,
        ..FamilyDto::new(3, "The Smiths")
    }
}

fn switch_off() -> FamilyDto {
    FamilyDto::new(3, "The Smiths")
}

/// `Voice(mime = "audio/mp4", size = 48_000, kind = "audio")`: `new AttachmentDto(34, kind, mime,
/// size, DurationMs: 8_400)`.
fn voice_as(mime: Option<&str>, size: Option<i64>, kind: &str) -> AttachmentDto {
    AttachmentDto {
        mime: mime.map(str::to_string),
        size,
        duration_ms: Some(8_400),
        ..AttachmentDto::new(34, kind)
    }
}

fn voice() -> AttachmentDto {
    voice_as(Some("audio/mp4"), Some(48_000), "audio")
}

fn voice_mime(mime: &str) -> AttachmentDto {
    voice_as(Some(mime), Some(48_000), "audio")
}

fn voice_size(size: Option<i64>) -> AttachmentDto {
    voice_as(Some("audio/mp4"), size, "audio")
}

fn may_ask(kind: Option<&str>, sender: i64, reader: i64, family: Option<&FamilyDto>) -> bool {
    transcript_rules::may_ask(kind, sender, reader, Some(ASSISTANT), family)
}

fn offers(
    assistant: Option<&AssistantDto>,
    family: &FamilyDto,
    kind: &str,
    sender: i64,
    attachment: &AttachmentDto,
) -> bool {
    transcript_rules::offers(
        assistant,
        Some(family),
        Some(kind),
        sender,
        ME,
        attachment,
        false,
        false,
    )
}

fn offers_extracting(
    assistant: &AssistantDto,
    family: &FamilyDto,
    kind: &str,
    sender: i64,
    attachment: &AttachmentDto,
) -> bool {
    transcript_rules::offers(
        Some(assistant),
        Some(family),
        Some(kind),
        sender,
        ME,
        attachment,
        false,
        true,
    )
}

/// YOUR OWN VOICE, WHEREVER YOU SENT IT: the family chat, a direct chat, your own assistant chat —
/// switch or no switch. (The C#'s theory, one row per chat kind.)
#[test]
fn your_own_recording_needs_no_switch() {
    for kind in ["family", "direct", "ai"] {
        assert!(may_ask(Some(kind), ME, ME, Some(&switch_off())), "{kind}");
        assert!(may_ask(Some(kind), ME, ME, Some(&switch_on())), "{kind}");
        assert!(may_ask(Some(kind), ME, ME, None), "{kind}");
    }
}

/// SOMEBODY ELSE'S: the family chat (and its threads, which are the same chat) while the owner
/// allows it; the other person's in a direct chat at any setting, since the owner's switch does not
/// reach one and the server asks the sender's consent (amended 2026-10-21).
#[test]
fn another_members_recording_needs_the_family_chat_and_the_owners_switch() {
    assert!(may_ask(Some("family"), ANNA, ME, Some(&switch_on())));
    assert!(!may_ask(Some("family"), ANNA, ME, Some(&switch_off())));
    assert!(!may_ask(Some("family"), ANNA, ME, None));
    assert!(may_ask(Some("direct"), ANNA, ME, Some(&switch_on())));
    assert!(may_ask(Some("direct"), ANNA, ME, Some(&switch_off())));
    assert!(may_ask(Some("direct"), ANNA, ME, None));
    assert!(!may_ask(Some("ai"), ANNA, ME, Some(&switch_on())));
    assert!(!may_ask(None, ANNA, ME, Some(&switch_on())));
}

/// The assistant's own messages are never asked about — it sends no sound, and has no consent to
/// give.
#[test]
fn the_assistants_messages_never() {
    assert!(!may_ask(Some("family"), ASSISTANT, ME, Some(&switch_on())));
    assert!(!may_ask(Some("ai"), ASSISTANT, ME, Some(&switch_on())));
}

/// A reader not known yet is nobody's own message — never a way round the switch.
#[test]
fn an_unknown_reader_owns_nothing() {
    assert!(!may_ask(Some("direct"), 0, 0, Some(&switch_on())));
    assert!(!may_ask(Some("family"), ANNA, 0, Some(&switch_off())));
    assert!(!may_ask(Some("direct"), ANNA, 0, Some(&switch_on())));
}

/// THE STORED FORM: a recording of a type the provider reads, as the server spells it, within the
/// ceiling — and nothing else (a video, Ogg, an oversized file and a size nobody stated go as the
/// device's own sound).
#[test]
fn only_a_recording_the_server_can_send_as_it_is_qualifies() {
    let assistant = transcribes();
    for mime in ["audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav"] {
        assert!(
            transcript_rules::sends_stored_copy(&voice_mime(mime), &assistant),
            "{mime}"
        );
    }
    assert!(!transcript_rules::sends_stored_copy(
        &voice_mime("audio/ogg"),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_mime("audio/webm"),
        &assistant
    ));
    // Compared as the server compares: exactly.
    assert!(!transcript_rules::sends_stored_copy(
        &voice_mime("Audio/MP4"),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_as(Some("video/mp4"), Some(48_000), "video"),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_as(Some("audio/mp4"), Some(48_000), "file"),
        &assistant
    ));
    assert!(transcript_rules::sends_stored_copy(
        &voice_size(Some(MIB25)),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_size(Some(MIB25 + 1)),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_size(None),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_size(Some(0)),
        &assistant
    ));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_as(None, Some(48_000), "audio"),
        &assistant
    ));
}

/// The server's own ceiling where it names a lower one; never above 25 MiB, whatever it says.
#[test]
fn the_ceiling_is_the_servers_and_never_more_than_the_providers() {
    let lower = AssistantDto {
        transcribe_max_bytes: Some(1_000_000),
        ..transcribes()
    };
    assert_eq!(1_000_000, transcript_rules::max_bytes(&lower));
    assert!(!transcript_rules::sends_stored_copy(
        &voice_size(Some(1_000_001)),
        &lower
    ));
    for said in [Some(100_000_000), None, Some(0)] {
        let assistant = AssistantDto {
            transcribe_max_bytes: said,
            ..transcribes()
        };
        assert_eq!(MIB25, transcript_rules::max_bytes(&assistant), "{said:?}");
    }
}

/// THE WHOLE DECISION: a server that transcribes and can name its processor, the rule, and the
/// stored form — every one of them needed; text already held is shown whatever they say.
#[test]
fn show_text_is_offered_only_where_every_part_says_yes() {
    let assistant = transcribes();
    assert!(offers(
        Some(&assistant),
        &switch_off(),
        "direct",
        ME,
        &voice()
    ));
    assert!(offers(
        Some(&assistant),
        &switch_on(),
        "family",
        ANNA,
        &voice()
    ));

    // No transcription deployment, an older server, no assistant, no processor to name.
    let deaf = AssistantDto {
        transcribe: false,
        ..transcribes()
    };
    assert!(!offers(Some(&deaf), &switch_on(), "family", ME, &voice()));
    assert!(!offers(None, &switch_on(), "family", ME, &voice()));
    let nameless = AssistantDto {
        processor: None,
        ..transcribes()
    };
    assert!(!offers(
        Some(&nameless),
        &switch_on(),
        "family",
        ME,
        &voice()
    ));
    let blank = AssistantDto {
        processor: Some("  ".to_string()),
        ..transcribes()
    };
    assert!(!offers(Some(&blank), &switch_on(), "family", ME, &voice()));
    // The rule.
    assert!(!offers(
        Some(&assistant),
        &switch_off(),
        "family",
        ANNA,
        &voice()
    ));
    assert!(offers(
        Some(&assistant),
        &switch_off(),
        "direct",
        ANNA,
        &voice()
    ));
    assert!(!offers(
        Some(&assistant),
        &switch_on(),
        "ai",
        ASSISTANT,
        &voice()
    ));
    // The form, on a device that cannot make the sound itself: the stored copy or nothing.
    assert!(!offers(
        Some(&assistant),
        &switch_on(),
        "family",
        ME,
        &voice_mime("audio/ogg")
    ));
    assert!(!offers(
        Some(&assistant),
        &switch_on(),
        "family",
        ME,
        &voice_as(Some("video/mp4"), Some(48_000), "video")
    ));

    // What this device was already given is shown, and showing it sends nothing.
    assert!(transcript_rules::offers(
        None,
        Some(&switch_off()),
        Some("direct"),
        ANNA,
        ME,
        &voice_mime("audio/ogg"),
        true,
        false
    ));
}

/// THE FORM: the server's stored copy wherever it can send one; otherwise — on a device that makes
/// the sound itself — the device's own, for a video, an Ogg file, an oversized or unsized
/// recording; and nothing for what is no recording.
#[test]
fn the_form_is_the_stored_copy_wherever_the_server_can_send_it() {
    let assistant = transcribes();
    assert_eq!(
        TranscriptForm::Stored,
        transcript_rules::form(&voice(), &assistant, true)
    );
    assert_eq!(
        TranscriptForm::Stored,
        transcript_rules::form(&voice(), &assistant, false)
    );

    for attachment in [
        voice_mime("audio/ogg"),
        voice_mime("audio/webm"),
        voice_size(Some(MIB25 + 1)),
        voice_size(None),
        voice_as(Some("video/mp4"), Some(80_000_000), "video"),
        voice_as(Some("video/quicktime"), Some(48_000), "video"),
    ] {
        assert_eq!(
            TranscriptForm::Supplied,
            transcript_rules::form(&attachment, &assistant, true)
        );
        // A device that cannot make the sound offers none of these.
        assert_eq!(
            TranscriptForm::None,
            transcript_rules::form(&attachment, &assistant, false)
        );
    }

    for kind in ["photo", "file", "location"] {
        assert_eq!(
            TranscriptForm::None,
            transcript_rules::form(
                &voice_as(Some("audio/mp4"), Some(48_000), kind),
                &assistant,
                true
            ),
            "{kind}"
        );
    }
}

/// NEVER HIDDEN FOR A REASON OF THE SOUND: a recording whose stated length is already too long for
/// the ceiling is still offered — as on iOS and the web — and the press says "This recording is too
/// long to turn into text." without downloading a byte (the transcript model). A hidden action
/// cannot say why it is missing.
#[test]
fn a_video_too_long_to_fit_is_still_offered() {
    let assistant = transcribes();
    let video = voice_as(Some("video/mp4"), Some(900_000_000), "video");
    let lasting = |duration_ms: Option<i32>| AttachmentDto {
        duration_ms,
        ..video.clone()
    };
    assert_eq!(
        TranscriptForm::Supplied,
        transcript_rules::form(&lasting(Some(50 * 60_000)), &assistant, true)
    );
    assert_eq!(
        TranscriptForm::Supplied,
        transcript_rules::form(&lasting(Some(55 * 60_000)), &assistant, true)
    );
    assert_eq!(
        TranscriptForm::Supplied,
        transcript_rules::form(&lasting(None), &assistant, true)
    );
    let lower = AssistantDto {
        transcribe_max_bytes: Some(1_000_000),
        ..transcribes()
    };
    assert_eq!(
        TranscriptForm::Supplied,
        transcript_rules::form(&lasting(Some(5 * 60_000)), &lower, true)
    );
    assert!(offers_extracting(
        &assistant,
        &switch_on(),
        "family",
        ME,
        &lasting(Some(55 * 60_000))
    ));
}

/// WHAT THE DEVICE COULD NOT DO IS SAID IN ITS OWN WORDS — the catalogue's sentences the other
/// three clients draw — and never as "Not available for this message.", which reads as a refusal.
/// Both are final.
#[test]
fn the_devices_own_failures_say_what_they_were() {
    let say = common::english();
    assert_eq!(
        "This recording is too long to turn into text.",
        transcript_rules::failure_sentence(&transcript_sound::too_long(), &say, &common::days())
    );
    assert_eq!(
        "Couldn't read the sound in this file.",
        transcript_rules::failure_sentence(&transcript_sound::unreadable(), &say, &common::days())
    );
    assert!(!transcript_rules::may_retry(&transcript_sound::too_long()));
    assert!(!transcript_rules::may_retry(&transcript_sound::unreadable()));
}

/// "Show text" ON VIDEOS AND UNSUPPORTED AUDIO, under exactly the same rule as a voice note: your
/// own anywhere; another member's in the family chat with the owner's switch on; the other
/// person's in a direct chat; never the assistant's.
#[test]
fn videos_and_unsupported_audio_follow_the_same_rule() {
    let assistant = transcribes();
    let deaf = AssistantDto {
        transcribe: false,
        ..transcribes()
    };
    for attachment in [
        voice_as(Some("video/mp4"), Some(48_000), "video"),
        voice_mime("audio/ogg"),
        voice_size(Some(MIB25 + 1)),
    ] {
        assert!(offers_extracting(
            &assistant,
            &switch_off(),
            "direct",
            ME,
            &attachment
        ));
        assert!(offers_extracting(
            &assistant,
            &switch_on(),
            "family",
            ANNA,
            &attachment
        ));
        assert!(!offers_extracting(
            &assistant,
            &switch_off(),
            "family",
            ANNA,
            &attachment
        ));
        assert!(offers_extracting(
            &assistant,
            &switch_off(),
            "direct",
            ANNA,
            &attachment
        ));
        assert!(!offers_extracting(
            &assistant,
            &switch_on(),
            "ai",
            ASSISTANT,
            &attachment
        ));
        assert!(!offers_extracting(
            &deaf,
            &switch_on(),
            "family",
            ME,
            &attachment
        ));
    }
}

/// What a failed ask says: the provider's refusal, final; "not available", final, for every refusal
/// of this recording or this member; and "try again" for anything that says nothing about the
/// request. (The C#'s theory, one row per code.)
#[test]
fn a_failure_says_what_it_was() {
    let say = common::english();
    for (code, status, said, retry) in [
        (
            error_codes::TRANSCRIPT_REFUSED,
            400,
            "The assistant's provider refused this recording.",
            false,
        ),
        (
            error_codes::NOT_TRANSCRIBABLE,
            400,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::TRANSCRIPT_NOT_ALLOWED,
            403,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::TRANSCRIPTS_UNAVAILABLE,
            403,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::MESSAGE_NOT_FOUND,
            404,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::ATTACHMENT_NOT_FOUND,
            404,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::BLOCKED,
            409,
            "Not available for this message.",
            false,
        ),
        (
            error_codes::INTERNAL,
            500,
            "Couldn't get the text. Try again.",
            true,
        ),
        (
            error_codes::TOO_MANY_REQUESTS,
            429,
            "Couldn't get the text. Try again.",
            true,
        ),
        (
            error_codes::ASSISTANT_CONSENT_REQUIRED,
            403,
            "Couldn't get the text. Try again.",
            true,
        ),
    ] {
        let error = ApiError::new(code, "x", status);
        assert_eq!(
            said,
            transcript_rules::failure_sentence(&error, &say, &common::days()),
            "{code}"
        );
        assert_eq!(retry, transcript_rules::may_retry(&error), "{code}");
    }
}

/// No connection and the deadline are the same "try again" — the server finishes and keeps it
/// meanwhile.
#[test]
fn no_answer_at_all_may_be_asked_again() {
    let gone = ApiError::transport("TaskCanceledException");
    assert_eq!(
        "Couldn't get the text. Try again.",
        transcript_rules::failure_sentence(&gone, &common::english(), &common::days())
    );
    assert!(transcript_rules::may_retry(&gone));
}

/// SILENCE IS AN ANSWER, drawn as "No speech" — never as a failure.
#[test]
fn silence_is_drawn_as_no_speech() {
    let say = common::english();
    assert_eq!("No speech", transcript_rules::shown(Some(""), &say));
    assert_eq!("No speech", transcript_rules::shown(Some("   "), &say));
    assert_eq!("No speech", transcript_rules::shown(None, &say));
    assert_eq!(
        "Back at six",
        transcript_rules::shown(Some("Back at six"), &say)
    );
    assert!(TranscriptLook::new(TranscriptPhase::Open, Some(""), None).no_speech());
    assert!(!TranscriptLook::new(TranscriptPhase::Open, Some("Back at six"), None).no_speech());
    assert!(!TranscriptLook::closed().no_speech());
}

/// Every sentence this draws is one the apps' catalogue already says in nine languages.
#[test]
fn the_sentences_are_the_catalogues_in_every_language() {
    let russian = common::catalogues().for_language("ru");
    assert_ne!("No speech", transcript_rules::shown(Some(""), &russian));
    assert_ne!(
        "The assistant's provider refused this recording.",
        transcript_rules::failure_sentence(
            &ApiError::new(error_codes::TRANSCRIPT_REFUSED, "x", 400),
            &russian,
            &common::days()
        )
    );
    assert_ne!(
        "Not available for this message.",
        transcript_rules::failure_sentence(
            &ApiError::new(error_codes::NOT_TRANSCRIBABLE, "x", 400),
            &russian,
            &common::days()
        )
    );
    assert_ne!(
        "Couldn't get the text. Try again.",
        transcript_rules::failure_sentence(&ApiError::transport("x"), &russian, &common::days())
    );
    assert_ne!(
        "This recording is too long to turn into text.",
        transcript_rules::failure_sentence(
            &transcript_sound::too_long(),
            &russian,
            &common::days()
        )
    );
    assert_ne!(
        "Couldn't read the sound in this file.",
        transcript_rules::failure_sentence(
            &transcript_sound::unreadable(),
            &russian,
            &common::days()
        )
    );
    for key in [
        "Show text",
        "Hide text",
        "Getting the text…",
        "Text of the recording",
        "Voice and video as text",
    ] {
        assert_ne!(key, russian.get(key));
    }
}

/// A GROUP FOLLOWS THE DIRECT CHAT'S RULE (docs/protocol.md, "Transcripts on request", amended
/// 2026-10-22; D4): another member's recording there needs that sender's consent alone, which the
/// server asks — the owner's switch, the family chat's, does not reach it either way — and "another
/// member" needs a reader who is known. Your own, as everywhere.
#[test]
fn a_group_follows_the_direct_chats_rule() {
    assert!(may_ask(Some("group"), ANNA, ME, Some(&switch_on())));
    assert!(may_ask(Some("group"), ANNA, ME, Some(&switch_off())));
    assert!(may_ask(Some("group"), ANNA, ME, None));
    assert!(may_ask(Some("group"), ME, ME, Some(&switch_off())));
    assert!(!may_ask(Some("group"), ANNA, 0, Some(&switch_on())));
    assert!(!may_ask(Some("group"), ASSISTANT, ME, Some(&switch_on())));
}

/// LINUX'S OWN, where it follows `fc_text::transcript::may_ask` rather than the C#: a reader not
/// known yet owns nothing and is offered nothing — not even another member's recording in the
/// family chat with the owner's switch on, which the C# allowed (its "other person" check covered
/// direct chats and groups only). The C#'s own guard is kept beside it: a sender that is nobody.
#[test]
fn an_unknown_reader_is_offered_nothing_even_with_the_switch_on() {
    assert!(!may_ask(Some("family"), ANNA, 0, Some(&switch_on())));
    assert!(!may_ask(Some("family"), 0, ME, Some(&switch_on())));
    assert!(!may_ask(Some("direct"), -4, ME, None));
}

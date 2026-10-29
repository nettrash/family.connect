//! `AssistantConsentTests`: nothing a member writes reaches the model before that member has said
//! yes (docs/protocol.md, "Consenting to the assistant").
//!
//! The vectors are the ones `AssistantConsentTests.swift`, `fc_text::assistant_consent`, Android's
//! `AssistantConsentTest.kt` and the server's own `model_surface` carry: a disagreement between
//! them is either a message refused after it was typed or one sent having asked nothing.

mod common;

use fc_core::text::string_catalog::EnglishCatalog;
use fc_logic::assistant_consent::{
    disclosure, is_available, is_required, is_withheld_from_an_unnamed_assistant, reaches_the_model,
};

use common::consent::PROCESSOR;

fn lines(family_history: bool, family_vision: bool, transcribe: bool) -> Vec<String> {
    disclosure(
        PROCESSOR,
        family_history,
        family_vision,
        &EnglishCatalog::new(),
        transcribe,
        &[],
    )
}

#[test]
fn the_assistants_own_chat_always_reaches_the_model() {
    for body in ["hello", "", "/draw a cat", "no mention here"] {
        assert!(reaches_the_model(Some("ai"), Some(body)), "{body}");
    }
}

#[test]
fn the_family_chat_reaches_it_only_on_a_mention() {
    assert!(reaches_the_model(
        Some("family"),
        Some("@ai when is dinner?")
    ));
    assert!(reaches_the_model(Some("family"), Some("hey @AI")));
    // A picture request in the family chat IS a mention; a bare one asks nobody.
    assert!(reaches_the_model(Some("family"), Some("@ai /draw a cat")));
    assert!(!reaches_the_model(Some("family"), Some("/draw a cat")));
    assert!(!reaches_the_model(Some("family"), Some("dinner at 7?")));
    assert!(!reaches_the_model(
        Some("family"),
        Some("write to anna@ai.example")
    ));
    assert!(!reaches_the_model(Some("family"), Some("@aiden said so")));
}

#[test]
fn nowhere_else_reaches_it() {
    for kind in [Some("direct"), Some("unknown"), None] {
        assert!(!reaches_the_model(kind, Some("@ai hello")));
    }
}

#[test]
fn asked_once_and_not_again() {
    assert!(is_required(
        Some("ai"),
        Some("hello"),
        Some(PROCESSOR),
        None
    ));
    assert!(!is_required(
        Some("ai"),
        Some("hello"),
        Some(PROCESSOR),
        Some("2026-09-19T19:34:43Z")
    ));
    assert!(!is_required(
        Some("family"),
        Some("dinner at 7?"),
        Some(PROCESSOR),
        None
    ));
    assert!(!is_required(
        Some("direct"),
        Some("@ai hello"),
        Some(PROCESSOR),
        None
    ));
}

#[test]
fn a_server_that_names_nobody_offers_no_assistant() {
    assert!(!is_available(None));
    assert!(!is_available(Some("")));
    assert!(!is_available(Some("   ")));
    assert!(is_available(Some(PROCESSOR)));
    assert!(!is_required(Some("ai"), Some("hello"), None, None));
}

/// A server that HAS an assistant but names nobody: consent cannot be asked for, so the message is
/// held back rather than sent.
#[test]
fn an_unnamed_assistant_withholds_the_message() {
    assert!(is_withheld_from_an_unnamed_assistant(
        Some("ai"),
        Some("hello"),
        true,
        None
    ));
    assert!(is_withheld_from_an_unnamed_assistant(
        Some("family"),
        Some("@ai hi"),
        true,
        Some("")
    ));
    assert!(!is_withheld_from_an_unnamed_assistant(
        Some("ai"),
        Some("hello"),
        true,
        Some(PROCESSOR)
    ));
}

/// And the case that must NOT be swallowed.
#[test]
fn where_there_is_no_assistant_at_all_the_words_are_just_words() {
    assert!(!is_withheld_from_an_unnamed_assistant(
        Some("family"),
        Some("@ai hi"),
        false,
        None
    ));
}

#[test]
fn the_disclosure_names_the_processor_and_follows_the_switches() {
    let with_history = lines(true, true, false);
    assert!(with_history.iter().any(|line| line.contains(PROCESSOR)));
    assert!(
        with_history
            .iter()
            .any(|line| line.contains("30") && line.contains("200"))
    );

    let without = lines(false, false, false);
    assert!(
        !without
            .iter()
            .any(|line| line.contains("30") || line.contains("200"))
    );

    // Photos are mentioned only where a photo could go.
    let with_pictures = lines(false, true, false);
    assert_eq!(without.len() + 1, with_pictures.len());
}

/// A RECORDING'S SOUND is a new kind of thing leaving the server, so it is a line of its own — said
/// only where this server can turn recordings into text, naming who receives it (docs/protocol.md,
/// "What a client must say before it asks", amended 2026-10-02).
#[test]
fn the_sound_of_a_recording_is_said_where_it_can_go() {
    let without = lines(false, false, false);
    let with = lines(false, false, true);
    assert_eq!(without.len() + 2, with.len());
    assert!(with.contains(&format!(
        "If you ask for the text of a voice note, audio file or video, its sound is sent to {PROCESSOR}."
    )));
    // And, since 2026-10-21, what a direct-chat partner may ask of the member's own recordings.
    assert!(with.contains(&format!(
        "In a direct chat or a group, the others in it can ask for the text of your voice notes, audio files and videos, and their sound is then sent to {PROCESSOR}."
    )));
    assert!(!without.iter().any(|line| line.contains("voice note")));
    // Its own line, after what a photo does and before where the answer lands and how to stop.
    let both = lines(true, true, true);
    let photo = both
        .iter()
        .position(|line| line.starts_with("A photo"))
        .expect("a photo line");
    let sound = both
        .iter()
        .position(|line| line.contains("its sound is sent"))
        .expect("a sound line");
    let partner = both
        .iter()
        .position(|line| line.starts_with("In a direct chat or a group"))
        .expect("a partner line");
    assert_eq!(photo + 1, sound);
    assert_eq!(sound + 1, partner);
    assert_eq!(both.len() - 3, partner);
}

/// A GROUP IS THE FAMILY CHAT'S SURFACE (docs/protocol.md, "Groups"): only an `@ai` there reaches
/// the model, so only that asks — and the disclosure says "the family chat or a group" in both of
/// its family-chat lines.
#[test]
fn a_group_reaches_the_model_only_by_a_mention() {
    assert!(reaches_the_model(
        Some("group"),
        Some("@ai when is dinner?")
    ));
    assert!(!reaches_the_model(Some("group"), Some("when is dinner?")));
    assert!(is_required(
        Some("group"),
        Some("@ai hi"),
        Some(PROCESSOR),
        None
    ));
    assert!(!is_required(
        Some("group"),
        Some("hi"),
        Some(PROCESSOR),
        None
    ));
    assert!(is_withheld_from_an_unnamed_assistant(
        Some("group"),
        Some("@ai hi"),
        true,
        None
    ));

    assert!(
        lines(true, false, false)
            .iter()
            .any(|line| line.starts_with(
                "In the family chat or a group, only a message that says @ai is sent — and with it"
            ))
    );
    assert!(lines(false, false, false).iter().any(|line| {
        line.starts_with(
            "In the family chat or a group, only a message that says @ai is sent, and nothing else",
        )
    }));
}

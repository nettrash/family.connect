//! Nothing a member writes reaches the model before that member has said
//! yes (docs/protocol.md, "Consenting to the assistant").
//!
//! Ported by value from `ios/FamilyConnect/Models/AssistantConsent.swift`,
//! and the same two jobs: deciding which drafts have to be asked about,
//! and saying what the person must be told before they answer. The first
//! mirrors `model_surface` in `server/src/handlers_chat.rs` — a
//! disagreement there is either a message refused after it was typed or
//! one sent having asked nothing.

use crate::i18n::{t, t1};
use crate::{assistant, lookups};

/// Would a message with this body, in this chat, be sent to the model?
///
/// The member's own `ai` chat, where everything goes, and the family chat,
/// where only an `@ai` does. `/draw` needs no case of its own: in the
/// family chat it is `@ai /draw`, a mention, and in the assistant's own
/// chat it is that chat.
pub fn reaches_the_model(chat_kind: &str, body: &str) -> bool {
    match chat_kind {
        "ai" => true,
        "family" => assistant::mentions(body),
        _ => false,
    }
}

/// Is there an assistant this client may offer at all?
///
/// A server that names no processor has one this client must not use: the
/// disclosure would have a hole exactly where the person needs to read,
/// and "some third party" is not something anybody can weigh.
pub fn is_available(processor: Option<&str>) -> bool {
    processor.is_some_and(|processor| !processor.trim().is_empty())
}

/// Must this member be asked before this message is sent?
pub fn is_required(chat_kind: &str, body: &str, processor: Option<&str>, agreed: bool) -> bool {
    is_available(processor) && !agreed && reaches_the_model(chat_kind, body)
}

/// Must this member be asked before an event's BACKDROP is drawn?
///
/// The backdrop is drawn from the event's title — words the author wrote —
/// and the title goes to `processor`'s images deployment, and on a refusal
/// to its text deployment as well, exactly as a `/draw` does; without the
/// author's consent the server answers `assistant_consent_required` and
/// sends nothing (docs/protocol.md, "Board" and "Consenting to the
/// assistant", both amended 2026-09-30). So it is asked the question a
/// `/draw` in the assistant's own chat is, and there is no body to test:
/// the title always reaches the model. A server that names nobody offers
/// no backdrop at all (`assistant_pictures::server_draws`), so there is
/// nothing here to withhold.
pub fn is_required_for_backdrop(processor: Option<&str>, agreed: bool) -> bool {
    is_required("ai", "", processor, agreed)
}

/// Would this message reach a model whose owner the server will not name,
/// so this client must hold it back entirely?
///
/// `has_assistant` is what keeps this from swallowing ordinary words: on a
/// server with no assistant, `@ai` in the family chat is three characters
/// that reach nobody, and refusing to send them would break a conversation
/// to protect nothing.
pub fn is_withheld_from_an_unnamed_assistant(
    chat_kind: &str,
    body: &str,
    has_assistant: bool,
    processor: Option<&str>,
) -> bool {
    has_assistant && !is_available(processor) && reaches_the_model(chat_kind, body)
}

/// What the person is told BEFORE they answer, in the order it is shown.
///
/// All of it on the screen where they answer and not only in a policy
/// behind a link. The two family-chat lines depend on the owner's
/// `ai_history`: with it on a mention takes the chat's recent history with
/// it, and with it off it takes nothing but itself — saying the wrong one
/// would be worse than saying neither. `transcribe` is the server's
/// `assistant.transcribe`: where a recording's text can be asked for, the
/// person is told its sound goes too (docs/protocol.md, "Transcripts on
/// request"). `lookups` is the server's `assistant.lookups`: where the
/// assistant may look things up, the person is told which providers a query
/// it writes may go to — with the same `ai_history` split as the family
/// lines — before either agree button (docs/protocol.md, "Consenting to
/// the assistant", amended 2026-10-03). Nobody named, no line: a client
/// that cannot name the providers does not ask.
pub fn disclosure(
    processor: &str,
    family_history: bool,
    family_vision: bool,
    transcribe: bool,
    lookups: &[String],
) -> Vec<String> {
    let mut lines = vec![t1(
        "What you write to the assistant leaves this family's server and is sent to %@.",
        processor,
    )];
    lines.push(if family_history {
        t1(
            "In the family chat only a message that says %@ is sent — and with it the last 30 days of that chat, up to 200 messages, including what other people wrote, their names and the times.",
            assistant::TOKEN,
        )
    } else {
        t1(
            "In the family chat only a message that says %@ is sent, and nothing else from that chat goes with it.",
            assistant::TOKEN,
        )
    });
    if family_vision {
        lines.push(
            t("A photo is sent only when you attach one to a message for the assistant, and only while your family allows it.")
                .to_string(),
        );
    }
    if transcribe {
        lines.push(t1(
            "If you ask for the text of a voice note, audio file or video, its sound is sent to %@.",
            processor,
        ));
    }
    let providers = lookups::providers(lookups);
    if !providers.is_empty() {
        lines.push(lookups::disclosure_line(
            &lookups::names(&providers),
            family_history,
        ));
    }
    lines.push(
        t("The answer comes back as a message in that chat, where everyone in the chat can read it.")
            .to_string(),
    );
    lines.push(
        t("You can stop this at any time in Settings. What has already been sent cannot be taken back.")
            .to_string(),
    );
    lines
}

/// The one line a composer shows where the assistant cannot be used at all
/// because the server named nobody.
pub fn unnamed_processor_notice() -> &'static str {
    t("This server hasn't said which service answers, so nothing can be sent to the assistant here.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_assistants_own_chat_always_reaches_the_model() {
        for body in ["hello", "", "/draw a cat", "no mention here"] {
            assert!(reaches_the_model("ai", body), "{body:?}");
        }
    }

    #[test]
    fn the_family_chat_reaches_it_only_on_a_mention() {
        assert!(reaches_the_model("family", "@ai when is dinner?"));
        assert!(reaches_the_model("family", "hey @AI"));
        // A picture request in the family chat IS a mention; a bare one
        // asks nobody.
        assert!(reaches_the_model("family", "@ai /draw a cat"));
        assert!(!reaches_the_model("family", "/draw a cat"));
        assert!(!reaches_the_model("family", "dinner at 7?"));
        assert!(!reaches_the_model("family", "write to anna@ai.example"));
        assert!(!reaches_the_model("family", "@aiden said so"));
    }

    #[test]
    fn nowhere_else_reaches_it() {
        for kind in ["direct", "unknown", ""] {
            assert!(!reaches_the_model(kind, "@ai hello"), "{kind}");
        }
    }

    #[test]
    fn asked_once_and_not_again() {
        assert!(is_required("ai", "hello", Some("Azure OpenAI"), false));
        assert!(!is_required("ai", "hello", Some("Azure OpenAI"), true));
        assert!(!is_required(
            "family",
            "dinner at 7?",
            Some("Azure OpenAI"),
            false
        ));
        assert!(!is_required(
            "direct",
            "@ai hello",
            Some("Azure OpenAI"),
            false
        ));
    }

    #[test]
    fn a_server_that_names_nobody_offers_no_assistant() {
        assert!(!is_available(None));
        assert!(!is_available(Some("")));
        assert!(!is_available(Some("   ")));
        assert!(is_available(Some("Azure OpenAI")));
        assert!(!is_required("ai", "hello", None, false));
    }

    #[test]
    fn an_unnamed_assistant_withholds_the_message() {
        assert!(is_withheld_from_an_unnamed_assistant(
            "ai", "hello", true, None
        ));
        assert!(is_withheld_from_an_unnamed_assistant(
            "family",
            "@ai hello",
            true,
            Some("")
        ));
        assert!(!is_withheld_from_an_unnamed_assistant(
            "ai",
            "hello",
            true,
            Some("Azure OpenAI")
        ));
    }

    /// An event's backdrop asks exactly what a `/draw` in the assistant's
    /// chat asks: before the author has agreed, whatever the title says.
    #[test]
    fn a_backdrop_is_asked_about_as_a_draw_is() {
        // (processor, agreed, asked)
        for (processor, agreed, asked) in [
            (Some("Azure OpenAI"), false, true),
            (Some("Azure OpenAI"), true, false),
            // Nobody named: no question to ask — and no backdrop offered.
            (None, false, false),
            (Some("  "), false, false),
        ] {
            assert_eq!(
                is_required_for_backdrop(processor, agreed),
                asked,
                "{processor:?} agreed={agreed}"
            );
            assert_eq!(
                is_required_for_backdrop(processor, agreed),
                is_required("ai", "/draw a birthday cake", processor, agreed),
                "the same question as a /draw: {processor:?} agreed={agreed}"
            );
        }
    }

    /// The case that must NOT be swallowed.
    #[test]
    fn where_there_is_no_assistant_at_all_the_words_are_just_words() {
        assert!(!is_withheld_from_an_unnamed_assistant(
            "family",
            "@ai hello",
            false,
            None
        ));
    }

    #[test]
    fn the_disclosure_names_the_processor_and_follows_the_switches() {
        let with_history = disclosure("Azure OpenAI", true, true, false, &[]);
        assert!(with_history
            .iter()
            .any(|line| line.contains("Azure OpenAI")));
        assert!(with_history
            .iter()
            .any(|line| line.contains("30") && line.contains("200")));

        let without = disclosure("Azure OpenAI", false, false, false, &[]);
        assert!(
            !without
                .iter()
                .any(|line| line.contains("30") || line.contains("200")),
            "with history off a mention takes nothing but itself: {without:?}"
        );
        assert_eq!(
            disclosure("Azure OpenAI", false, true, false, &[]).len(),
            without.len() + 1,
            "photos are mentioned only where a photo could go"
        );
    }

    /// Where the server can turn a recording into text, the person is told
    /// before they agree that a recording's sound goes to the processor —
    /// and not told it where nothing of the kind can happen.
    #[test]
    fn the_disclosure_names_recordings_only_where_they_can_go() {
        let line = "If you ask for the text of a voice note, audio file or video, its sound is sent to Azure OpenAI.";
        let with = disclosure("Azure OpenAI", true, false, true, &[]);
        assert!(with.iter().any(|said| said == line), "{with:?}");
        let without = disclosure("Azure OpenAI", true, false, false, &[]);
        assert!(!without.iter().any(|said| said.contains("voice note")));
        assert_eq!(with.len(), without.len() + 1);
    }

    /// Where the server can look things up, the person is told which
    /// providers a query may go to, in the line that matches the family's
    /// history switch, before the answer line and the "stop" line; where it
    /// cannot, nothing is said about lookups at all.
    #[test]
    fn the_disclosure_names_the_lookup_providers_only_where_there_are_some() {
        let sources: Vec<String> = ["Brave Search", "Open-Meteo", "Wikipedia"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        let with = disclosure("Azure OpenAI", true, false, false, &sources);
        let without = disclosure("Azure OpenAI", true, false, false, &[]);
        assert_eq!(with.len(), without.len() + 1);
        let line = with
            .iter()
            .position(|said| said.contains("Brave Search, Open-Meteo and Wikipedia"))
            .expect("the providers are named");
        assert!(with[line].contains("possibly from recent messages too"));
        assert_eq!(
            line,
            with.len() - 3,
            "before the answer line and the stop line: {with:?}"
        );
        assert!(!without.iter().any(|said| said.contains("lookups")));
        let quiet = disclosure("Azure OpenAI", false, false, false, &sources);
        assert!(quiet
            .iter()
            .any(|said| said
                .contains("from your question to Brave Search, Open-Meteo and Wikipedia,")));
        assert!(!quiet
            .iter()
            .any(|said| said.contains("recent messages too")));
        // Blank names are nobody.
        let blank = vec!["  ".to_string()];
        assert_eq!(
            disclosure("Azure OpenAI", true, false, false, &blank),
            without
        );
    }
}

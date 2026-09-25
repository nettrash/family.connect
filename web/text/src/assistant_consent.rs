//! Nothing a member writes reaches the model before that member has said
//! yes (docs/protocol.md, "Consenting to the assistant").
//!
//! Ported by value from `ios/FamilyConnect/Models/AssistantConsent.swift`,
//! and the same two jobs: deciding which drafts have to be asked about,
//! and saying what the person must be told before they answer. The first
//! mirrors `model_surface` in `server/src/handlers_chat.rs` — a
//! disagreement there is either a message refused after it was typed or
//! one sent having asked nothing.

use crate::assistant;
use crate::i18n::{t, t1};

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
/// would be worse than saying neither.
pub fn disclosure(processor: &str, family_history: bool, family_vision: bool) -> Vec<String> {
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
        let with_history = disclosure("Azure OpenAI", true, true);
        assert!(with_history
            .iter()
            .any(|line| line.contains("Azure OpenAI")));
        assert!(with_history
            .iter()
            .any(|line| line.contains("30") && line.contains("200")));

        let without = disclosure("Azure OpenAI", false, false);
        assert!(
            !without
                .iter()
                .any(|line| line.contains("30") || line.contains("200")),
            "with history off a mention takes nothing but itself: {without:?}"
        );
        assert_eq!(
            disclosure("Azure OpenAI", false, true).len(),
            without.len() + 1,
            "photos are mentioned only where a photo could go"
        );
    }
}

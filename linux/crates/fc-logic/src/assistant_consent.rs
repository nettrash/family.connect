//! Nothing a member writes reaches the model before that member has said yes (docs/protocol.md,
//! "Consenting to the assistant") — `AssistantConsent.cs`.
//!
//! Ported by value from `ios/FamilyConnect/Models/AssistantConsent.swift`, and the same two jobs:
//! deciding which drafts have to be asked about, and saying what the person must be told before
//! they answer. The first MIRRORS `model_surface` on the server — a disagreement there is either a
//! message refused after it was typed, or one sent having asked nothing — and is
//! `fc_text::assistant_consent`'s, called rather than copied. The second ([`disclosure`]) is said
//! from the catalogue the caller hands in (`say`), as the C#'s is, and not from
//! `fc_text::assistant_consent::disclosure`, whose wording reads the web's thread-local language.

use fc_core::text::string_catalog::StringCatalog;

/// Would a message with this body, in this chat, be sent to the model? The member's own `ai` chat,
/// where everything goes, and the family chat or a group, where only an `@ai` does — a group is the
/// family chat's surface with fewer people (docs/protocol.md, "Groups"). `/draw` needs no case of
/// its own: in the family chat it is `@ai /draw`, a mention, and in the assistant's own chat it is
/// that chat.
pub fn reaches_the_model(chat_kind: Option<&str>, body: Option<&str>) -> bool {
    fc_text::assistant_consent::reaches_the_model(chat_kind.unwrap_or(""), body.unwrap_or(""))
}

/// Is there an assistant this client may offer at all? A server that names no processor has one it
/// must not use: the disclosure would have a hole exactly where the person needs to read, and "some
/// third party" is not something anybody can weigh.
pub fn is_available(processor: Option<&str>) -> bool {
    fc_text::assistant_consent::is_available(processor)
}

/// Must this member be asked before this message is sent? `agreed_at` is `/me`'s
/// `assistant_consent_at`; a blank one is no answer (`string.IsNullOrWhiteSpace`).
pub fn is_required(
    chat_kind: Option<&str>,
    body: Option<&str>,
    processor: Option<&str>,
    agreed_at: Option<&str>,
) -> bool {
    is_available(processor) && is_blank(agreed_at) && reaches_the_model(chat_kind, body)
}

/// Would this message reach a model whose owner the server will not name, so this client must hold
/// it back entirely?
///
/// `has_assistant` is what keeps this from swallowing ordinary words: on a server with no
/// assistant, `@ai` in the family chat is three characters that reach nobody, and refusing to send
/// them would break a conversation to protect nothing.
pub fn is_withheld_from_an_unnamed_assistant(
    chat_kind: Option<&str>,
    body: Option<&str>,
    has_assistant: bool,
    processor: Option<&str>,
) -> bool {
    has_assistant && !is_available(processor) && reaches_the_model(chat_kind, body)
}

/// What the person is told BEFORE they answer, in the order it is shown — all of it on the screen
/// where they answer, not only in a policy behind a link (App Store Review Guideline 5.1.1(i), and
/// protocol.md, "What a client must say before it asks").
///
/// The two family-chat lines depend on the owner's `ai_history`: with it on a mention takes the
/// chat's recent history with it, and with it off it takes nothing but itself. Saying the wrong one
/// of those would be worse than saying neither. Both say "the family chat or a group" since
/// 2026-10-22: the owner's switches apply to a group as they do to the family chat (docs/protocol.md,
/// "What a client must say before it asks").
///
/// `transcribe` is the server's `assistant.transcribe`: where a member can ask for the text of a
/// recording, its SOUND goes to the same provider — a new kind of thing leaving the server, so it
/// gets a line of its own (docs/protocol.md, "What a client must say before it asks", amended
/// 2026-10-02).
///
/// `lookups` is the server's `assistant.lookups` as [`crate::lookups::providers`] reads it: where
/// the assistant may look things up, the providers that would receive a query it writes are named
/// in one more line (docs/protocol.md, "What a client must say before it asks", amended 2026-10-03)
/// — said whether or not the owner has turned lookups on, because the line says it happens only
/// when they do. Empty: no line.
pub fn disclosure(
    processor: &str,
    family_history: bool,
    family_vision: bool,
    say: &dyn StringCatalog,
    transcribe: bool,
    lookups: &[String],
) -> Vec<String> {
    let token = fc_text::assistant::TOKEN;
    let mut lines = vec![
        say.format(
            "What you write to the assistant leaves this family's server and is sent to %@.",
            &[&processor],
        ),
        if family_history {
            say.format(
                "In the family chat or a group, only a message that says %@ is sent — and with it the last 30 days of that chat, up to 200 messages, including what other people wrote, their names and the times.",
                &[&token],
            )
        } else {
            say.format(
                "In the family chat or a group, only a message that says %@ is sent, and nothing else from that chat goes with it.",
                &[&token],
            )
        },
    ];
    if family_vision {
        lines.push(
            say.get("A photo is sent only when you attach one to a message for the assistant, and only while your family allows it.")
                .to_string(),
        );
    }
    if transcribe {
        lines.push(say.format(
            "If you ask for the text of a voice note, audio file or video, its sound is sent to %@.",
            &[&processor],
        ));
        lines.push(say.format(
            "In a direct chat or a group, the others in it can ask for the text of your voice notes, audio files and videos, and their sound is then sent to %@.",
            &[&processor],
        ));
    }
    if !lookups.is_empty() {
        lines.push(crate::lookups::consent_line(lookups, family_history, say));
    }
    lines.push(
        say.get("The answer comes back as a message in that chat, where everyone in the chat can read it.")
            .to_string(),
    );
    lines.push(
        say.get("You can stop this at any time in Settings. What has already been sent cannot be taken back.")
            .to_string(),
    );
    lines
}

/// `string.IsNullOrWhiteSpace` — .NET's white space and Rust's `char::is_whitespace` are the same
/// set (Unicode White_Space).
fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|text| text.trim().is_empty())
}

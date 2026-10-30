//! Looking things up for the assistant, from a client's side (docs/protocol.md, "Looking things
//! up"; issue #72) — `Lookups.cs`: which providers there are to name, what the consent screen, the
//! owner's switch and Settings say about them, the two writes an answer turns into, and which
//! answers keep their links out of preview cards.
//!
//! THREE KEYS, AND THIS CLIENT HOLDS NONE OF THEM. A lookup happens only where the server has a
//! source (`assistant.lookups`), the owner has turned `ai_lookups` on, and the asking member has
//! given BOTH consents — and the server decides all three. A client's part is to say who would
//! receive what, to ask, and to send the answer; it never assumes one. Without the lookup consent
//! nothing is refused: the question is answered as before.
//!
//! A CLIENT THAT CANNOT NAME THE PROVIDERS DOES NOT ASK (docs/protocol.md, "What a client must say
//! before it asks"). [`providers`] is `None` for an absent, empty or blank list, and every surface
//! here keys off it.
//!
//! The sentences come from the catalogue the caller hands in (`say`), as the C#'s do — not from
//! `fc_text::lookups`, whose wording reads the web's thread-local language: the names are joined by
//! the reader's own translation of "%@ and %@" and "%@, %@ and %@" (、と, 和), which only `say` knows
//! for the language the window is in.

use fc_core::protocol::{ApiClient, ApiError, AssistantDto, MessageDto};
use fc_core::text::string_catalog::StringCatalog;

use crate::app_session::SessionState;
use crate::assistant_consent;
use crate::bubble_rules;

/// What a member answered on the assistant question, where the server can look things up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConsentAnswer {
    /// Nothing agreed; nothing is sent to the server either.
    NotNow,
    /// The assistant, and not the lookups: `/me/assistant-consent` only.
    Agree,
    /// Both: `/me/assistant-consent` (when not already given), then `/me/assistant-lookup-consent`.
    AgreeWithLookups,
}

/// The providers to name, in the server's order, or `None` where there is nothing to name — an
/// absent array (an older server, or one with no source), an empty one (which the protocol
/// forbids, and which names nobody), or one of blank names. Blank names inside a list are dropped
/// rather than named as nothing; the rest are trimmed (`fc_text::lookups::providers`).
pub fn providers(assistant: Option<&AssistantDto>) -> Option<Vec<String>> {
    let names = assistant?.lookups.as_deref()?;
    let named = fc_text::lookups::providers(names);
    (!named.is_empty()).then_some(named)
}

/// Whether this server offers lookups at all: providers to name, on an assistant this client may
/// offer — one whose processor is named, since the lookup consent stands on top of the assistant's.
pub fn offered(assistant: Option<&AssistantDto>) -> bool {
    assistant_consent::is_available(assistant.and_then(|assistant| assistant.processor.as_deref()))
        && providers(assistant).is_some()
}

/// The providers as one phrase for a `%@`: one on its own, two as "A and B", three as "A, B and
/// C" — and the languages that join them differently (、と, 和) do so in their own translation of
/// those two keys. More than three (the server sends at most three) folds the head into the first
/// slot, so nobody goes unnamed.
pub fn names(names: &[String], say: &dyn StringCatalog) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [one, two] => say.format("%@ and %@", &[one, two]),
        [head @ .., second_last, last] => {
            say.format("%@, %@ and %@", &[&head.join(", "), second_last, last])
        }
    }
}

/// The consent screen's line about lookups — the one more line protocol.md asks for. Which of the
/// two follows the owner's `ai_history`, the way the two family-chat lines already do: with it on,
/// a mention's query may be shaped by recent messages too, and saying otherwise would be asking
/// permission for something other than what happens.
pub fn consent_line(
    names_given: &[String],
    family_history: bool,
    say: &dyn StringCatalog,
) -> String {
    let named = names(names_given, say);
    if family_history {
        say.format(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question — in the family chat or a group, possibly from recent messages too — to %@, and its answer then lists its sources.",
            &[&named],
        )
    } else {
        say.format(
            "If your family's owner turns on lookups, the assistant may send a short search query or place name it writes from your question to %@, and its answer then lists its sources.",
            &[&named],
        )
    }
}

/// What a member who has ALREADY agreed to the assistant reads before allowing lookups too: the
/// lookup line, and the way back out. The rest of the assistant's disclosure they have read and
/// agreed to.
pub fn disclosure(
    names_given: &[String],
    family_history: bool,
    say: &dyn StringCatalog,
) -> Vec<String> {
    vec![
        consent_line(names_given, family_history, say),
        say.get("You can stop this at any time in Settings. What has already been sent cannot be taken back.")
            .to_string(),
    ]
}

/// The footnote under the owner's `ai_lookups` switch, naming the providers.
pub fn switch_footnote(names_given: &[String], say: &dyn StringCatalog) -> String {
    say.format(
        "With this on, the assistant can look things up when a question needs it — the weather, the news, a fact it isn't sure of — in %@. Only a short search query or place name the assistant writes from the question is sent to them, never the conversation itself, and only when the member asking has agreed to it. Answers then list their sources. It is off unless you turn it on.",
        &[&names(names_given, say)],
    )
}

/// The member's footnote in Settings, before and after they allowed lookups.
pub fn settings_footnote(names_given: &[String], allowed: bool, say: &dyn StringCatalog) -> String {
    let named = names(names_given, say);
    if allowed {
        say.format(
            "The assistant may send a short search query or place name it writes from your questions to %@. Stopping takes effect at once; what has already been sent cannot be taken back.",
            &[&named],
        )
    } else {
        say.format(
            "Until you allow lookups, the assistant answers you from what it already knows, and nothing from your questions is sent to %@.",
            &[&named],
        )
    }
}

/// Which answer a pressed button is. With lookups offered, the first button is "Agree With
/// Lookups" and the second "Agree Without Lookups"; without them, the first is "I Agree" and there
/// is no second. Anything else — Not Now, Escape, a dismissed sheet — is
/// [`ConsentAnswer::NotNow`].
pub fn answer_for(first: bool, second: bool, lookups_offered: bool) -> ConsentAnswer {
    match (first, second) {
        (true, _) if lookups_offered => ConsentAnswer::AgreeWithLookups,
        (true, _) => ConsentAnswer::Agree,
        (false, true) if lookups_offered => ConsentAnswer::Agree,
        _ => ConsentAnswer::NotNow,
    }
}

/// Has this member allowed lookups, as the server last said?
pub fn allowed(state: &SessionState) -> bool {
    !is_blank(state.assistant_lookup_consent_at.as_deref())
}

/// Whether Settings offers this member the lookup agreement: a server that names its providers,
/// and a member who has agreed to the assistant — the lookup consent can only be granted on top of
/// it, and the assistant question itself already asks about lookups for a member who has not.
pub fn in_settings(state: &SessionState) -> bool {
    offered(state.assistant.as_ref()) && !is_blank(state.assistant_consent_at.as_deref())
}

/// Turn an answer into the writes it means, in the order the server needs them: the assistant
/// consent first — unless `assistant_agreed` says it is already held — and the lookup consent only
/// on top of it, never after a failed first write. `None` when everything asked for was written
/// (or nothing was asked for); the first refusal otherwise. The caller re-reads `/me` either way: a
/// first write that landed before a second failed is the truth.
pub async fn record(
    api: &ApiClient,
    answer: ConsentAnswer,
    assistant_agreed: bool,
) -> Option<ApiError> {
    if answer == ConsentAnswer::NotNow {
        return None;
    }
    if !assistant_agreed && let Err(error) = api.set_assistant_consent(true).await.into_result() {
        return Some(error);
    }
    if answer == ConsentAnswer::AgreeWithLookups
        && let Err(error) = api.set_assistant_lookup_consent(true).await.into_result()
    {
        return Some(error);
    }
    None
}

/// May this bubble draw a preview card under its first web link? Everything may, as before,
/// EXCEPT an assistant answer that is still being written, and one that ends with the server's
/// sources footer (design decision 7).
///
/// A finished lookup answer's links are its sources and its providers' credits — the server
/// removed every other link the model wrote before adding the footer — so a card under any of them
/// would have every device showing the answer contact a cited page.
///
/// An answer still STREAMING is the model's raw words: the server filters links out of the
/// finished body, not out of the deltas, so a link a web page talked the model into writing is in
/// the stream until the finished row replaces it. A card fetched for it in the meantime is exactly
/// the request the filter exists to prevent. The card waits for the finished row — and an answer
/// that stopped part-way, whose words are never replaced, never gets one.
///
/// `still_writing`: streamed text not yet replaced by the finished row (`AssistantAnswers::is_writing`).
pub fn may_preview(
    message: &MessageDto,
    body: &str,
    me: i64,
    assistant_chat: bool,
    assistant_user_id: Option<i64>,
    still_writing: bool,
) -> bool {
    if !bubble_rules::is_assistant(message, me, assistant_chat, assistant_user_id) {
        return true;
    }
    !still_writing && !fc_core::text::sources_footer::ends_with_footer(Some(body))
}

/// `string.IsNullOrWhiteSpace`.
fn is_blank(text: Option<&str>) -> bool {
    text.is_none_or(|text| text.trim().is_empty())
}

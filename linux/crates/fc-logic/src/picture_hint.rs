//! The line said while a member is asking for a picture (`PictureHint.cs`): the images
//! deployment's filter refuses most descriptions that name a real person, a public figure or a
//! brand, and nothing else would tell them which word was the problem (docs/protocol.md, "A refused
//! description is reworded once"). The server rewords a refused description once on its own; this
//! line is so that it seldom has to.
//!
//! Only where a picture is really being asked for, on a server that really draws — a hint about a
//! refusal that cannot happen is noise.

use fc_core::protocol::AssistantDto;
use fc_core::text::string_catalog::StringCatalog;

use crate::assistant_consent;

/// What the line says.
pub fn sentence(say: &dyn StringCatalog) -> String {
    say.get(
        "Describe people and things in general words — real names and brands are often refused.",
    )
    .to_string()
}

/// Under the composer: a draft that has begun a `/draw` request — the token first, past leading
/// white space, and followed by white space, before any description is typed — that the server will
/// answer with a picture. In the assistant's own chat that is the draft; in the family chat only an
/// `@ai` reaches the assistant at all ([`assistant_consent::reaches_the_model`]). Never while a
/// message is being edited: an edit cannot become a request.
///
/// Where the server draws is where "Ask for a picture" is offered: an assistant with `images` and a
/// NAMED processor ([`crate::join_memory::AssistantButtons`]). The family chat's `@ai /draw` is
/// drawn by the same deployment, so it is refused by the same filter and gets the same line. The
/// grammar is `fc_text::assistant::starts_picture_request` (`AssistantText.BeginsPictureRequest`).
pub fn for_composer(
    chat_kind: Option<&str>,
    draft: Option<&str>,
    assistant: Option<&AssistantDto>,
    editing: bool,
) -> bool {
    !editing
        && assistant.is_some_and(|assistant| {
            assistant.images && assistant_consent::is_available(assistant.processor.as_deref())
        })
        && assistant_consent::reaches_the_model(chat_kind, draft)
        && fc_text::assistant::starts_picture_request(draft.unwrap_or(""))
}

/// "Draw a backdrop" on an event — and the line beside it, since the title IS the description: the
/// author's own event on a server that can draw at all (docs/protocol.md, "Board";
/// `pictures_unavailable` otherwise).
///
/// And, like "Ask for a picture", only where the server NAMES who draws: the title is the author's
/// words going to the model, which needs their consent ([`crate::backdrop_consent`]), and a consent
/// question with a hole where the recipient goes cannot be asked.
pub fn offers_backdrop(editable: bool, assistant: Option<&AssistantDto>) -> bool {
    editable
        && assistant.is_some_and(|assistant| {
            assistant.images && assistant_consent::is_available(assistant.processor.as_deref())
        })
}

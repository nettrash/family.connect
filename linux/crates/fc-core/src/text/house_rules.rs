//! The owner's house rules (`HouseRules.cs`): the join policy, the member limit and the
//! assistant's switches.
//!
//! The member limit is `fc_text::account` — `cap_state` (C# `Cap`), `cap_footer`, `clamp_cap`,
//! `seed_cap` — and the nine family languages are `fc_text::account::LANGUAGES`; the photo budget
//! is `fc_text::assistant_pictures::MAX_PER_QUESTION`. What is here is what the web client keeps
//! in its family pane (`web/src/views/family.rs`) rather than in `fc_text`: the policies' words,
//! and why a switch says what it says.
//!
//! Two inputs are the protocol's records in the C#, and plain values here so that this module
//! needs nothing from `protocol`: [`pictures_note`] takes the family's `ai_vision` and
//! `ai_history` (the C# reads them off a `FamilyDto`), and [`assistant_failure`] takes the
//! refusal's error code (the C# reads `ApiError.Code`).

use super::string_catalog::StringCatalog;

use crate::protocol::error_codes::NOT_FAMILY_OWNER;

/// The three policies the protocol allows, with the key each is said by.
pub const POLICIES: [(&str, &str); 3] = [
    ("open", "Join immediately"),
    ("approval", "Need approval"),
    ("closed", "Nobody"),
];

/// What the policy picker says under the chosen policy; an unknown or absent one reads as open.
pub fn policy_caption(policy: Option<&str>, say: &dyn StringCatalog) -> String {
    match policy {
        Some("approval") => say
            .get("With approval, join requests wait here until you approve them.")
            .to_string(),
        Some("closed") => say
            .get("The invite code stops working — nobody new can join. Requests already waiting are unaffected, and you can still approve them.")
            .to_string(),
        _ => say
            .get("Anyone with the invite code joins straight away.")
            .to_string(),
    }
}

/// Why a switch that rides on vision does nothing, or `None` when it does: the server cannot look
/// at pictures; vision is off, and the server refuses the switch while it is; or the history it
/// draws from is not sent (`when_history_off`, which differs per switch).
pub fn pictures_note(
    server_vision: bool,
    ai_vision: bool,
    ai_history: bool,
    when_history_off: &str,
    say: &dyn StringCatalog,
) -> Option<String> {
    if !server_vision {
        Some(
            say.get("Not available here: the assistant on this server can't look at pictures.")
                .to_string(),
        )
    } else if !ai_vision {
        Some(
            say.get(
                "Turn on Can be shown photos first — the server refuses this while that is off.",
            )
            .to_string(),
        )
    } else if !ai_history {
        Some(when_history_off.to_string())
    } else {
        None
    }
}

/// What a refused change to the assistant's switches is told as, by its error code.
pub fn assistant_failure(code: &str, say: &dyn StringCatalog) -> String {
    if code == NOT_FAMILY_OWNER {
        say.get("Only the family owner can change this.")
            .to_string()
    } else {
        say.get("Couldn't save that. Try again.").to_string()
    }
}

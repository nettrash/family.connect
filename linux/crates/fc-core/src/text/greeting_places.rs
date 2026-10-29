//! The owner's places for the daily greeting's weather (`GreetingPlaces.cs`; docs/protocol.md,
//! "Today's weather, for places the owner chose"; issue #72) — the server's own rules for a name
//! (`validate_greeting_places`), applied BEFORE the request so that nothing the owner can type is
//! refused.
//!
//! `fc_text::greeting_places` holds the field: `typed` (what a field keeps as somebody types),
//! `fold` (`split_whitespace().join(" ")`), and `places` / `write` (the rows as a request). What it
//! does not have is the per-name question a field asks while it is being edited — how long a name
//! is as the server counts it, and why the server would refuse it — which is here.
//!
//! RUST'S CHARACTERS: "whitespace", "control character" and "80 characters" are the server's
//! words, so they are `char::is_whitespace`, `char::is_control` and `chars().count()` — Unicode
//! scalars, never UTF-16 units, so a name of astral characters is not cut at half the length the
//! server allows.

use fc_text::greeting_places::{MAX_CHARS, fold};

/// Why a name would be refused (`GreetingPlaces.Problem`; its `None` is `Option::None` here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    Empty,
    Control,
    TooLong,
}

/// How long a name is as the server counts it: Unicode scalars, not UTF-16 units.
pub fn length(name: &str) -> usize {
    name.chars().count()
}

/// Why the server would refuse this name, as typed — it is folded first, as the server folds it —
/// or `None` when it would be kept.
pub fn judge(raw: &str) -> Option<Problem> {
    let name = fold(raw);
    if name.is_empty() {
        Some(Problem::Empty)
    } else if name.chars().any(char::is_control) {
        Some(Problem::Control)
    } else if length(&name) > MAX_CHARS {
        Some(Problem::TooLong)
    } else {
        None
    }
}

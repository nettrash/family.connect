//! Member mentions (`Mentions.cs`; docs/protocol.md, "Mentioning a member") — the grammar, the
//! resolution, the tokens a bubble highlights and the composer's `@` strip are all
//! `fc_text::mentions`, called. What the C# adds on top is here: the most names one message
//! carries, and a text cut into its plain stretches and the names it says.

use fc_text::mentions::{Member, tokens};

/// The most names one message or note carries (the web composer's `MAX_MENTIONS`).
pub const MAX_PER_MESSAGE: usize = 20;

/// A text cut into its plain stretches and the names it says — what a bubble or a sticker draws a
/// name bold from. A text naming nobody is one plain run. Each run is its text and, for a name,
/// whose it is; every cut is on a grapheme boundary, because `fc_text::mentions::tokens` grows
/// each token to the clusters it touches. (The C# turns the tokens' UTF-8 offsets into UTF-16
/// ones; a Rust string is cut at the UTF-8 offsets themselves.)
pub fn runs(text: &str, mentions: &[Member<'_>]) -> Vec<(String, Option<i64>)> {
    let found = tokens(text, mentions);
    if found.is_empty() {
        return vec![(text.to_string(), None)];
    }
    let mut runs = Vec::new();
    let mut at = 0;
    for token in found {
        if token.range.start > at {
            runs.push((text[at..token.range.start].to_string(), None));
        }
        runs.push((
            text[token.range.clone()].to_string(),
            Some(token.member.user_id),
        ));
        at = token.range.end;
    }
    if at < text.len() {
        runs.push((text[at..].to_string(), None));
    }
    runs
}

//! Recognising what a message body ASKS the assistant for: `@ai`
//! (docs/protocol.md, "Mentioning the assistant in the family chat") and
//! `/draw` (protocol.md, "Pictures"). Two grammars in one file because the
//! second is defined in terms of the first — a family-chat picture request
//! is `@ai` and then the token.
//!
//! A WIRE CONTRACT, not a rendering detail. The SERVER decides from the same
//! grammar whether a family-chat message reaches the assistant at all and
//! whether a request goes to an image model instead (server/src/mentions.rs
//! `mentions_assistant`, `draw_prompt`), and this file decides where the
//! web client's highlight goes. Ported by value from
//! ios/FamilyConnect/Models/AssistantMention.swift; its test tables are
//! carried below vector for vector, in order.
//!
//! The `@ai` rule, in full:
//!
//! - the token is the three characters `@ai`, matched case-insensitively
//!   but only over ASCII, so `@AI` and `@Ai` are mentions;
//! - the `@` must start the body or follow a byte that is not an ASCII
//!   letter, digit or `_` — which is what stops `anna@ai.example`;
//! - the `i` must end the body or be followed by a byte that is not an ASCII
//!   letter, digit or `_` — which is what stops `@aiden`.
//!
//! The boundary test is ASCII-only on purpose (Swift's header has the long
//! version): a Unicode "is this a letter" would refuse `@ai` written against
//! Japanese or Russian with no space, and would make every port depend on
//! its own Unicode tables agreeing.
//!
//! ONE deliberate difference from the Swift, in the server's favour. Swift
//! keeps an `@ai` match only when both of its ends fall on a `Character`
//! boundary, so `@ai\u{301}`, `@ai\u{FE0F}` and `\u{600}@ai` — mentions to
//! the server, which reads bytes and nothing else, and to Android — are
//! plain text on Apple. That is the exact disagreement MemberMentions.swift
//! already fixed for member names ("The END may not be…"), left unfixed
//! here. This port takes the match by the server's byte rule and widens the
//! HIGHLIGHT outward to the grapheme clusters it touches, which is the only
//! way such a token can be drawn.

use std::ops::Range;

use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

/// The token itself, so nothing spells it twice.
pub const TOKEN: &str = "@ai";

/// The picture token, so nothing spells it twice.
pub const DRAW_TOKEN: &str = "/draw";

/// Does this body address the assistant? The server's
/// `mentions_assistant`, answer for answer.
pub fn mentions(body: &str) -> bool {
    raw_mentions(body).next().is_some()
}

/// Every `@ai` in `body`, as byte ranges on grapheme boundaries, in order.
///
/// ALL of them, not just the first: the server only needs to know whether
/// there is one, but a bubble highlighting the first and leaving a second
/// plain would look like a typo.
///
/// Normally exactly the three bytes of the token. When a combining mark, a
/// variation selector or a ZWJ follows it — or a prepended mark such as
/// U+0600 precedes it — the byte after (or at) the token sits inside a
/// grapheme cluster, and the range is widened to that whole cluster; see
/// the module header for why the match itself still stands. Two matches
/// can never share a cluster: that would need an `@` to extend the cluster
/// before it, and `@` is never a combining character.
pub fn ranges(body: &str) -> Vec<Range<usize>> {
    raw_mentions(body)
        .map(|start| widen_to_clusters(body, start..start + TOKEN.len()))
        .collect()
}

/// ASCII letters, digits and `_` are what a token may NOT sit against.
/// Anything else — punctuation, whitespace, the lead or continuation byte
/// of a multi-byte character — is a boundary.
///
/// Mirrors `AssistantMention.isBoundary(_: UInt8)` and the server's
/// `is_boundary`: a BYTE test, deliberately. A UTF-8 lead or continuation
/// byte is never an ASCII letter, digit or `_`, so "the adjacent byte is not
/// one of those" and "the adjacent character is not one of those" are the
/// same statement, and no Unicode table is consulted.
pub fn is_boundary(byte: u8) -> bool {
    !(byte.is_ascii_alphanumeric() || byte == b'_')
}

/// The start of every `@ai` the server would accept, in order.
///
/// The scan is over BYTES and indexes nothing but the byte slice, so it
/// cannot slice a `&str` in the wrong place: `@` and the token are ASCII,
/// so every start it yields is a char boundary, and so is `start + 3`.
fn raw_mentions(body: &str) -> impl Iterator<Item = usize> + '_ {
    let bytes = body.as_bytes();
    (0..bytes.len().saturating_sub(TOKEN.len() - 1)).filter(move |&index| mention_at(bytes, index))
}

/// Is there an `@ai` starting exactly at `index`, boundaries and all?
fn mention_at(bytes: &[u8], index: usize) -> bool {
    let end = index + TOKEN.len();
    end <= bytes.len()
        && bytes[index..end].eq_ignore_ascii_case(TOKEN.as_bytes())
        // `eq_ignore_ascii_case` folds A–Z only, and `@` has no case, so this
        // is exactly Swift's `byte | 0x20 == a` on the two letters.
        && (index == 0 || is_boundary(bytes[index - 1]))
        && (end == bytes.len() || is_boundary(bytes[end]))
}

// MARK: - `/draw` (docs/protocol.md, "Pictures")

/// The picture this body asks for, or `None` because it asks for none.
///
/// The SECOND wire-contract grammar, and a contract for the reason the
/// first one is: the server decides from it whether a request goes to an
/// entirely different provider, and each client highlights exactly what the
/// server will act on. The rule, in full:
///
/// - the token is the five characters `/draw`, matched case-insensitively
///   but only over ASCII, so `/DRAW` and `/Draw` ask too;
/// - it must be the FIRST thing in the body, ignoring leading whitespace
///   and ONE leading `@ai` and the whitespace after it — `@ai /draw a cat`
///   asks, `hey @ai /draw a cat` does not;
/// - it must be followed by whitespace, so `/drawer` and `/draw,a cat` are
///   words and `/draw` alone is an ordinary message;
/// - what follows, trimmed, is the PROMPT and must not be empty.
///
/// The same function answers for the family chat and for the `ai` chat, as
/// the server's does: in an `ai` chat the leading `@ai` is simply never
/// typed. Whether the family chat acts on it at all is the caller's
/// question — the server only looks for `/draw` on a message that
/// [`mentions`] the assistant.
///
/// Returns a slice of the caller's body, as the server does: what comes
/// back is the whole of what will leave the server on this request.
pub fn draw_prompt(body: &str) -> Option<&str> {
    draw_scan(body).map(|scan| scan.prompt)
}

/// Does this body ask for a picture? The twin of [`mentions`], and it must
/// answer exactly what [`draw_prompt`] does or a `/draw` button would type a
/// second token into a body that already asks.
pub fn asks_for_picture(body: &str) -> bool {
    draw_scan(body).is_some()
}

/// The token itself, as a byte range into `body`, for the bubble that draws
/// it emphasised. Present exactly when [`draw_prompt`] answers a prompt —
/// both come off ONE scan, because a highlight the server would not act on
/// is the failure this grammar exists to prevent.
///
/// Always on grapheme boundaries: the token starts the body or follows
/// whitespace or a `@ai`, and it is followed by whitespace, and neither side
/// of that can be the inside of a cluster.
pub fn draw_token_range(body: &str) -> Option<Range<usize>> {
    draw_scan(body).map(|scan| scan.token)
}

/// The one answer both public questions are asked of.
struct DrawScan<'a> {
    /// Where the five characters sit in the caller's own string.
    token: Range<usize>,
    /// The words after them, trimmed, never empty.
    prompt: &'a str,
}

/// Read a body once and answer both halves of the picture grammar.
///
/// Walked in `char`s — Unicode scalars — never in grapheme clusters, which
/// is the whole of why Swift's version is written the way it is: in
/// `/draw\u{301} a cat` a combining mark sits where the token wants
/// whitespace, and a walk by clusters steps over it onto the space. Rust's
/// `char` is Swift's `Unicode.Scalar` and the server's `char`, so this walk
/// and the server's are the same walk.
///
/// The token is compared as BYTES and nothing is sliced until it has
/// matched: five leading bytes that match an ASCII token are five whole
/// characters, so the slice after them is on a char boundary exactly when
/// the comparison passed. The NOT_DRAWS table pins the shapes that took the
/// server down before it learned this.
fn draw_scan(body: &str) -> Option<DrawScan<'_>> {
    let mut index = skipping_whitespace(body, 0);
    // ONE leading mention, and only a leading one — the position is checked
    // rather than trusted, which is what makes `look @ai /draw a cat` an
    // ordinary message. `index` is 0 or follows whitespace, so the byte
    // before it is always a boundary and `mention_at` checks the rest.
    if mention_at(body.as_bytes(), index) {
        index = skipping_whitespace(body, index + TOKEN.len());
    }
    let bytes = body.as_bytes();
    let end = index + DRAW_TOKEN.len();
    if end > bytes.len() || !bytes[index..end].eq_ignore_ascii_case(DRAW_TOKEN.as_bytes()) {
        return None;
    }
    // End of body is not a request, and anything that is not whitespace
    // makes a longer word.
    let after = &body[end..];
    match after.chars().next() {
        Some(first) if is_whitespace(first) => {
            let prompt = after.trim_matches(is_whitespace);
            (!prompt.is_empty()).then_some(DrawScan {
                token: index..end,
                prompt,
            })
        }
        _ => None,
    }
}

/// The first byte offset at or after `from` that is not whitespace.
/// `from` must be a char boundary; the answer then is one too.
fn skipping_whitespace(body: &str, from: usize) -> usize {
    let rest = &body[from..];
    from + (rest.len() - rest.trim_start_matches(is_whitespace).len())
}

/// Unicode `White_Space` — the SERVER's definition of whitespace, and
/// therefore this grammar's.
///
/// Mirrors Swift's `Unicode.Scalar.Properties.isWhitespace`, which IS that
/// property, and is exactly what Rust's `char::is_whitespace` answers. It
/// is deliberately NOT any of the other three answers this project has met:
/// Swift's `Character.isWhitespace` (the same property read off the first
/// scalar of a CLUSTER — wrong for a scalar walk), Foundation's
/// `CharacterSet.whitespacesAndNewlines` (adds U+200B ZERO WIDTH SPACE),
/// and Kotlin's `Char.isWhitespace()` (Java's: adds U+001C–U+001F, drops
/// U+0085).
fn is_whitespace(c: char) -> bool {
    c.is_whitespace()
}

// MARK: - Composer doors

/// The draft the composer's "ask the assistant" button leaves behind:
/// `@ai ` appended, or the draft untouched when it already mentions the
/// assistant.
///
/// Ported from the button's action on both Apple composers
/// (`insertAssistantMention` in ConversationView.swift and
/// MacConversationView.swift), which append rather than insert at the
/// caret: SwiftUI publishes no selection, and moving somebody's cursor is
/// worse than adding to the end of what they wrote.
///
/// "Ends in a space" is Swift's `hasSuffix(" ")`, which compares the LAST
/// CHARACTER — a space that a combining mark rides on, or that a prepended
/// mark such as U+0600 binds to, is not " " to Swift, so the separating
/// space is added there too.
pub fn with_assistant_mention(draft: &str) -> String {
    if mentions(draft) {
        return draft.to_string();
    }
    if draft.is_empty() {
        format!("{TOKEN} ")
    } else if draft.graphemes(true).next_back() == Some(" ") {
        format!("{draft}{TOKEN} ")
    } else {
        format!("{draft} {TOKEN} ")
    }
}

/// The draft the `ai` chat's "ask for a picture" button leaves behind:
/// `/draw ` in FRONT of the words already typed, or the draft untouched
/// when it already asks for a picture — pressing it twice must not produce
/// `/draw /draw`, which the server would read as a request to draw the word.
///
/// Ported from `insertDrawToken` on both Apple composers, which offer the
/// button in the assistant's own chat only. The typed words are trimmed the
/// way that code trims them — Foundation's `whitespacesAndNewlines`, see
/// [`super::composer::trimmed_for_send`] — so a body of nothing but a
/// U+200B becomes `/draw ` there, as it does on Apple.
pub fn with_draw_token(draft: &str) -> String {
    if asks_for_picture(draft) {
        return draft.to_string();
    }
    let rest = super::composer::trim_foundation_whitespace(draft);
    if rest.is_empty() {
        format!("{DRAW_TOKEN} ")
    } else {
        format!("{DRAW_TOKEN} {rest}")
    }
}

// MARK: - Grapheme clusters

/// `range` grown outward to the nearest grapheme-cluster boundaries — the
/// smallest span a highlight can actually be drawn over.
///
/// Both ends must already be char boundaries; every caller hands in the
/// ends of an ASCII-delimited byte match, which are.
pub(crate) fn widen_to_clusters(text: &str, range: Range<usize>) -> Range<usize> {
    cluster_start(text, range.start)..cluster_end(text, range.end)
}

/// The grapheme boundary at or before `index`.
fn cluster_start(text: &str, index: usize) -> usize {
    let mut cursor = GraphemeCursor::new(index, text.len(), true);
    // The whole text is one chunk starting at 0, so the cursor always has
    // the context it needs and never asks for more; the `unwrap_or`s are
    // the answers "the start" and "the end", which a complete chunk never
    // reaches.
    if cursor.is_boundary(text, 0).unwrap_or(true) {
        return index;
    }
    cursor.prev_boundary(text, 0).ok().flatten().unwrap_or(0)
}

/// The grapheme boundary at or after `index` — Swift's `clusterEnd`.
fn cluster_end(text: &str, index: usize) -> usize {
    let mut cursor = GraphemeCursor::new(index, text.len(), true);
    if cursor.is_boundary(text, 0).unwrap_or(true) {
        return index;
    }
    cursor
        .next_boundary(text, 0)
        .ok()
        .flatten()
        .unwrap_or(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared table — `AssistantMentionTests.mentions`, the server's
    /// `MENTIONS` and Android's `mentions`, in the same order.
    const MENTIONS: &[&str] = &[
        "@ai",
        "@AI",
        "@Ai",
        "@ai what is the weather",
        "hey @ai what is the weather",
        "hey @ai, what is the weather",
        "ask @ai.",
        "(@ai)",
        "\n@ai\n",
        "@@ai",
        "какая погода @ai",
        // No space after the token, which is normal in Japanese — the
        // ASCII-only boundary is what makes this a mention.
        "@aiこんにちは",
        "-@ai",
        "@ai?",
        "1+@ai",
    ];

    const NOT_MENTIONS: &[&str] = &[
        "",
        "@",
        "@a",
        "@aiden",
        "@ai_bot",
        "@ai2",
        "@aI3",
        "anna@ai.example",
        "x@ai",
        "1@ai",
        "_@ai",
        "ai",
        "email me at bob@aim.com",
        "@artificial intelligence",
    ];

    #[test]
    fn mention_vectors() {
        for body in MENTIONS {
            assert!(mentions(body), "should be a mention: {body:?}");
        }
    }

    #[test]
    fn non_mention_vectors() {
        for body in NOT_MENTIONS {
            assert!(!mentions(body), "should NOT be a mention: {body:?}");
        }
    }

    #[test]
    fn range_covers_the_token() {
        let body = "hey @ai there";
        let found = ranges(body);
        assert_eq!(found.len(), 1);
        assert_eq!(&body[found[0].clone()], "@ai");
        // The server's `the_range_points_at_the_token_itself`, as offsets.
        assert_eq!(found, vec![4..7]);
    }

    /// Multi-byte characters before the token are where a byte-versus-
    /// character mix-up shows, by slicing in the wrong place or panicking.
    #[test]
    fn multibyte_prefix() {
        for body in ["Привет @ai", "こんにちは @ai です", "🇷🇸 @ai"] {
            let found = ranges(body);
            assert_eq!(found.len(), 1, "one mention in: {body:?}");
            assert_eq!(
                &body[found[0].clone()],
                "@ai",
                "sliced the token in: {body:?}"
            );
        }
    }

    #[test]
    fn every_mention() {
        let body = "@ai and also @AI, but not @aiden";
        let found: Vec<&str> = ranges(body).into_iter().map(|r| &body[r]).collect();
        assert_eq!(found, ["@ai", "@AI"]);
    }

    /// The near miss must not stop the scan; `@@ai` must still be found by
    /// the second `@`.
    #[test]
    fn scan_continues_past_a_near_miss() {
        assert!(mentions("@aiden asked @ai"));
        let found = ranges("@@ai");
        assert_eq!(found.len(), 1);
        assert_eq!(&"@@ai"[found[0].clone()], "@ai");
        // The server's offsets for the same two shapes.
        assert_eq!(found, vec![1..4]);
        assert_eq!(ranges("@aiden asked @ai"), vec![13..16]);
    }

    // MARK: - `/draw`

    /// `AssistantMentionTests.draws` and the server's `DRAWS`, prompt and
    /// all, in the same order. The PROMPT is asserted rather than a bool: it
    /// is the whole of what leaves the server.
    const DRAWS: &[(&str, &str)] = &[
        ("/draw a cat", "a cat"),
        ("/DRAW a cat", "a cat"),
        ("/Draw a cat", "a cat"),
        ("   /draw a cat  ", "a cat"),
        ("/draw\na cat", "a cat"),
        // The family chat: one leading mention, and the words after the
        // token are still the whole of what leaves.
        ("@ai /draw a cat", "a cat"),
        ("@AI    /draw a cat in a hat", "a cat in a hat"),
        ("@ai\n/draw a cat", "a cat"),
        // A prompt that itself contains the tokens is still just words.
        ("/draw @ai holding a sign", "@ai holding a sign"),
        ("/draw /draw", "/draw"),
        // The PROMPT in the scripts this app is translated into.
        (
            "/draw \u{43A}\u{43E}\u{442} \u{432} \u{448}\u{43B}\u{44F}\u{43F}\u{435}",
            "\u{43A}\u{43E}\u{442} \u{432} \u{448}\u{43B}\u{44F}\u{43F}\u{435}",
        ),
        ("/draw \u{732B}", "\u{732B}"),
        ("@ai /draw \u{1F408} on a mat", "\u{1F408} on a mat"),
        // U+0085 NEXT LINE **is** whitespace here and on the server.
        ("/draw\u{85}a cat", "a cat"),
        // U+200B ZERO WIDTH SPACE is **not**, so it stays in the prompt —
        // and a body of the token and one of them IS a picture request.
        ("/draw \u{200B}cat", "\u{200B}cat"),
        ("/draw \u{200B}", "\u{200B}"),
    ];

    const NOT_DRAWS: &[&str] = &[
        "",
        "/draw",
        "  /draw  ",
        "/drawer",
        "/draws a cat",
        "/draw,a cat",
        "draw a cat",
        // The token has to be FIRST.
        "hey @ai /draw a cat",
        "what does /draw do?",
        "please /draw a cat",
        // A mention that is not at the start does not license it either.
        "look @ai /draw a cat",
        // Two mentions: the second is not a leading one.
        "@ai @ai /draw a cat",
        // NON-ASCII BODIES — the shapes that used to panic the server at a
        // five-byte slice.
        "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "\u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}, \u{43A}\u{430}\u{43A} \u{434}\u{435}\u{43B}\u{430}?",
        "@ai \u{41F}\u{440}\u{438}\u{432}\u{435}\u{442}",
        "\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}",
        "\u{4F60}\u{597D}\u{4E16}\u{754C}",
        "\u{417}\u{434}\u{440}\u{430}\u{432}\u{43E}",
        // `/dra` then a three-byte character.
        "/dra\u{20AC} a cat",
        // A BARE PREFIX OF THE TOKEN, in each script that trips a byte index.
        "/d",
        "/dr",
        "/dra",
        "/dr\u{438}",
        "/dr\u{3042}",
        "/dr\u{1F408}",
        "\u{41F}",
        "\u{41F}\u{440}",
        "\u{41F}\u{440}\u{438}",
        "\u{41F}\u{440}\u{438}\u{432}",
        "\u{3053}",
        "\u{3053}\u{3093}",
        "\u{1F408}",
        "\u{1F408}\u{1F408}",
        "\u{1F3A8} \u{43D}\u{430}\u{440}\u{438}\u{441}\u{443}\u{439} \u{43A}\u{43E}\u{442}\u{430}",
        "@ai \u{1F408}",
        // A HOMOGLYPH: Cyrillic `\u{430}` where the token wants ASCII `a`.
        "/dr\u{430}w a cat",
        // U+001C–U+001F are **not** whitespace (Java says they are).
        "/draw\u{1C}a cat",
        "/draw\u{1D}a cat",
        "/draw\u{1E}a cat",
        "/draw\u{1F}a cat",
        // A COMBINING MARK on the token's last letter: a longer word.
        "/draw\u{301} a cat",
        "@ai /draw\u{301} a cat",
    ];

    #[test]
    fn draw_vectors() {
        for (body, prompt) in DRAWS {
            assert_eq!(
                draw_prompt(body),
                Some(*prompt),
                "{body:?} should ask for: {prompt:?}"
            );
        }
    }

    #[test]
    fn non_draw_vectors() {
        for body in NOT_DRAWS {
            assert_eq!(
                draw_prompt(body),
                None,
                "should NOT ask for a picture: {body:?}"
            );
        }
    }

    #[test]
    fn asks_for_picture_agrees_with_the_prompt() {
        for (body, _) in DRAWS {
            assert!(asks_for_picture(body), "should ask: {body:?}");
        }
        for body in NOT_DRAWS {
            assert!(!asks_for_picture(body), "should not ask: {body:?}");
        }
    }

    /// The two grammars are independent, and the family chat needs both.
    #[test]
    fn draw_in_the_family_chat() {
        assert!(mentions("@ai /draw a cat"));
        assert_eq!(draw_prompt("@ai /draw a cat"), Some("a cat"));
        // And one in a PRIVATE thread mentions nobody.
        assert!(!mentions("/draw a cat"));
    }

    /// The highlight sits exactly on the five characters the server acts on.
    #[test]
    fn draw_range_covers_the_token() {
        for body in [
            "/draw a cat",
            "   /draw a cat",
            "@ai /draw a cat",
            "/DRAW a cat",
        ] {
            let range = draw_token_range(body).unwrap_or_else(|| panic!("no range in: {body:?}"));
            assert_eq!(
                body[range].to_lowercase(),
                "/draw",
                "sliced the token in: {body:?}"
            );
        }
        // Nothing to highlight where the server would act on nothing.
        for body in NOT_DRAWS {
            assert_eq!(draw_token_range(body), None, "no range in: {body:?}");
        }
    }

    #[test]
    fn draw_with_multibyte_text() {
        assert_eq!(draw_prompt("/draw кот в шляпе"), Some("кот в шляпе"));
        assert_eq!(draw_prompt("/draw 猫"), Some("猫"));
        assert_eq!(draw_prompt("@ai /draw 🐈 on a mat"), Some("🐈 on a mat"));
        // A body whose FIFTH byte lands inside a multi-byte character.
        assert_eq!(draw_prompt("/dra€ a cat"), None);
        assert_eq!(draw_prompt("/dra"), None);
        assert_eq!(draw_prompt("🐈"), None);
        // A leading `@aiden` is not a leading mention.
        assert_eq!(draw_prompt("@aiden /draw a cat"), None);
    }

    /// The whitespace table, code point by code point — pinned against the
    /// predicate the grammar actually walks with, not against the standard
    /// library in general.
    #[test]
    fn whitespace_is_the_unicode_property() {
        let whitespace = [
            '\u{9}', '\u{A}', '\u{B}', '\u{C}', '\u{D}', '\u{20}', '\u{85}', '\u{A0}', '\u{1680}',
            '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
            '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{2028}', '\u{2029}', '\u{202F}',
            '\u{205F}', '\u{3000}',
        ];
        for c in whitespace {
            assert!(is_whitespace(c), "U+{:04X} is whitespace", c as u32);
        }
        // The five Java calls whitespace and Unicode does not, plus the one
        // Foundation calls whitespace and Unicode does not.
        for c in ['\u{1C}', '\u{1D}', '\u{1E}', '\u{1F}', '\u{200B}'] {
            assert!(!is_whitespace(c), "U+{:04X} is NOT whitespace", c as u32);
        }
        // And the one Foundation gets wrong, spelled out: the composer's
        // Foundation-shaped trim DOES take U+200B, and this grammar must not.
        assert!(super::super::composer::is_foundation_whitespace('\u{200B}'));
        assert!(!is_whitespace('\u{200B}'));
        // And the table is the whole property: nothing else in the BMP or
        // beyond answers yes.
        let all: Vec<char> = (0..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|c| is_whitespace(*c))
            .collect();
        assert_eq!(all, whitespace);
    }

    /// Every PREFIX of every vector — cut at every char boundary, which is
    /// every place Swift's `body.indices` cuts and more — is answered rather
    /// than mis-sliced or panicked on.
    #[test]
    fn no_prefix_is_misread() {
        let bodies = DRAWS
            .iter()
            .map(|(body, _)| *body)
            .chain(NOT_DRAWS.iter().copied())
            .chain(MENTIONS.iter().copied())
            .chain(NOT_MENTIONS.iter().copied());
        for body in bodies {
            let cuts = body
                .char_indices()
                .map(|(index, _)| index)
                .chain([body.len()]);
            for index in cuts {
                let head = &body[..index];
                if let Some(range) = draw_token_range(head) {
                    assert_eq!(
                        head[range].to_ascii_lowercase(),
                        DRAW_TOKEN,
                        "sliced something other than the token out of: {head:?}"
                    );
                }
                for range in ranges(head) {
                    assert!(head[range].to_ascii_lowercase().contains(TOKEN), "{head:?}");
                }
            }
        }
    }

    // MARK: - Where this port follows the server and not the Swift

    /// The server reads bytes and nothing else, so a mark riding on the
    /// token's `i`, or a prepended mark bound to its `@`, leaves it a
    /// mention — which the server answers. Swift drops these (see the module
    /// header); Android and the server keep them. The highlight takes the
    /// whole cluster.
    #[test]
    fn a_mention_inside_a_grapheme_cluster_is_the_servers_mention() {
        for (body, drawn) in [
            ("@ai\u{301} what now?", "@ai\u{301}"),
            ("@ai\u{FE0F}", "@ai\u{FE0F}"),
            ("@ai\u{200D}\u{1F469} hi", "@ai\u{200D}"),
            ("\u{600}@ai", "\u{600}@ai"),
        ] {
            assert!(mentions(body), "{body:?}");
            let found = ranges(body);
            assert_eq!(found.len(), 1, "{body:?}");
            assert_eq!(&body[found[0].clone()], drawn, "{body:?}");
        }
        // And the ASCII boundary still decides: a Cyrillic letter is a
        // boundary, an ASCII one is not.
        assert!(mentions("@aiЖ"));
        assert!(!mentions("@aib"));
    }

    // MARK: - Composer doors

    #[test]
    fn the_mention_button_appends_once() {
        assert_eq!(with_assistant_mention(""), "@ai ");
        assert_eq!(with_assistant_mention("what is"), "what is @ai ");
        assert_eq!(with_assistant_mention("what is "), "what is @ai ");
        // Already asked: untouched, whatever the case.
        assert_eq!(with_assistant_mention("hey @AI"), "hey @AI");
        // `@aiden` is not a mention, so the button still adds one.
        assert_eq!(with_assistant_mention("@aiden"), "@aiden @ai ");
        // A space a mark rides on is not " " to Swift's `hasSuffix`.
        assert_eq!(with_assistant_mention("a \u{301}"), "a \u{301} @ai ");
        assert_eq!(with_assistant_mention("\u{600} "), "\u{600}  @ai ");
    }

    #[test]
    fn the_picture_button_puts_the_token_first_once() {
        assert_eq!(with_draw_token(""), "/draw ");
        assert_eq!(with_draw_token("  a cat \n"), "/draw a cat");
        assert_eq!(with_draw_token("/draw a cat"), "/draw a cat");
        // `/draw` alone asks for nothing, so the button still adds a token.
        assert_eq!(with_draw_token("/draw"), "/draw /draw");
        // Foundation's trim takes a ZERO WIDTH SPACE; the grammar does not.
        assert_eq!(with_draw_token("\u{200B}"), "/draw ");
        // What the button builds is a request the grammar reads back.
        let built = with_draw_token("кот");
        assert_eq!(built, "/draw кот");
        assert_eq!(draw_prompt(&built), Some("кот"));
    }
}

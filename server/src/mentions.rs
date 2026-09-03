//! Recognising what a message body ASKS the assistant for: `@ai` (docs/
//! protocol.md, "Mentioning the assistant in the family chat") and `/draw`
//! (protocol.md, "Pictures").
//!
//! This grammar is a WIRE CONTRACT, not an implementation detail. The server
//! decides from it whether a family-chat message reaches the assistant at
//! all, and each client draws the same token as a highlight in the bubble
//! and offers it from the composer. A client that highlighted something the
//! server then ignored — or the reverse — would be a family watching a
//! question go unanswered with no way to tell why. It is mirrored by value
//! in `ios/FamilyConnect/Models/AssistantMention.swift` and
//! `android/…/ui/chat/AssistantMention.kt`, with the same vectors pinned on
//! all three.
//!
//! The rule, in full:
//!
//! - the token is the three characters `@ai`, matched case-insensitively but
//!   **only over ASCII** — `@AI` and `@Ai` are mentions;
//! - the `@` must start the body or follow a character that is not an ASCII
//!   letter, digit or `_`, which is what keeps `anna@ai.example` from being
//!   a mention;
//! - the `i` must end the body or be followed by a character that is not an
//!   ASCII letter, digit or `_`, which is what keeps `@aiden` from being one.
//!
//! **The boundary test is deliberately ASCII-only.** Using Unicode
//! alphanumerics instead would refuse `@ai` written against Japanese or
//! Russian text with no space after it — languages this app is translated
//! into and where a space would not normally be typed — and it would make
//! the three implementations depend on three different Unicode tables
//! agreeing, which is precisely how the `EmojiOnly` classifier got its own
//! hand-written whitespace table. The ambiguity the boundary exists to
//! resolve (`@aiden`) only arises in ASCII anyway.

/// The token itself, so nothing spells it twice.
pub const MENTION: &str = "@ai";

/// Does this body address the assistant?
pub fn mentions_assistant(body: &str) -> bool {
    first_mention(body).is_some()
}

/// The byte range of the FIRST `@ai` in `body`, if any.
///
/// Returned as a range rather than a bool so the same scan can serve a
/// highlight; the server only asks the question, but the clients mirroring
/// this file need the position and must not answer it a second way.
pub fn first_mention(body: &str) -> Option<(usize, usize)> {
    // The scan is over BYTES, and that is exact rather than a shortcut: a
    // UTF-8 continuation or lead byte is never an ASCII letter, digit or
    // `_`, so "the adjacent byte is not one of those" and "the adjacent
    // CHARACTER is not one of those" are the same statement. `@` and the
    // token are ASCII, so every index handled here is a char boundary.
    let bytes = body.as_bytes();
    let mut index = 0usize;
    while let Some(offset) = body[index..].find('@') {
        let start = index + offset;
        let end = start + MENTION.len();
        if end <= bytes.len()
            && bytes[start + 1].eq_ignore_ascii_case(&b'a')
            && bytes[start + 2].eq_ignore_ascii_case(&b'i')
            && is_boundary(if start == 0 {
                None
            } else {
                Some(bytes[start - 1])
            })
            && is_boundary(bytes.get(end).copied())
        {
            return Some((start, end));
        }
        // Step past this `@` only, never past the whole token: `@@ai` must
        // still be found by the second one.
        index = start + 1;
    }
    None
}

/// The picture token, so nothing spells it twice.
pub const DRAW: &str = "/draw";

/// The picture this body asks for, or `None` because it asks for none.
///
/// The SECOND wire-contract grammar in this file, and it is a contract for
/// the same reason the first one is: the server decides from it whether a
/// request goes to a different provider entirely, and each client highlights
/// exactly what the server will act on. Mirrored by value in
/// `ios/FamilyConnect/Models/AssistantMention.swift` and Android's
/// `AssistantMention.kt`, with the same vectors pinned on all three.
///
/// The rule, in full:
///
/// - the token is the five characters `/draw`, matched case-insensitively
///   over ASCII, so `/DRAW` and `/Draw` ask too;
/// - it must be the FIRST thing in the body, ignoring leading whitespace
///   and — this is what makes the family chat work — one leading `@ai` and
///   the whitespace after it. `@ai /draw a cat` asks for a picture;
///   `hey @ai /draw a cat` does not, and neither does `what does /draw do?`;
/// - it must be followed by whitespace or the end of the body, so `/drawer`
///   is a word;
/// - what follows, trimmed, is the PROMPT and must not be empty. `/draw`
///   alone is an ordinary message.
///
/// First-and-nowhere-else is the same decision the `@ai` boundary rules
/// made. A token that could hide anywhere in a sentence would turn a family
/// DISCUSSING this feature into a family generating pictures of it, and it
/// would put "does this leave the server, and to whom" somewhere a reader
/// cannot see at a glance.
///
/// Returns a slice of the caller's body rather than a `String`: what comes
/// back is the whole of what will leave this server, and borrowing it makes
/// that visible at the call site instead of hiding it behind a copy.
pub fn draw_prompt(body: &str) -> Option<&str> {
    let rest = body.trim_start();
    // One leading mention, and only a LEADING one: `first_mention` finds the
    // token anywhere, so the position is checked rather than trusted.
    let rest = match first_mention(rest) {
        Some((0, end)) => rest[end..].trim_start(),
        _ => rest,
    };
    if rest.len() < DRAW.len() {
        return None;
    }
    let (token, after) = rest.split_at(DRAW.len());
    if !token.eq_ignore_ascii_case(DRAW) {
        return None;
    }
    // End of body, or whitespace. Anything else is a longer word: `/drawer`
    // is not a request, and neither is `/draw,`.
    match after.chars().next() {
        None => None,
        Some(c) if c.is_whitespace() => {
            let prompt = after.trim();
            (!prompt.is_empty()).then_some(prompt)
        }
        Some(_) => None,
    }
}

/// Is what sits on one side of the token a boundary? `None` is either end
/// of the body, which always is.
fn is_boundary(probe: Option<u8>) -> bool {
    match probe {
        None => true,
        Some(byte) => !(byte.is_ascii_alphanumeric() || byte == b'_'),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared vectors. `ios/FamilyConnectTests/AssistantMentionTests.swift`
    /// and Android's `AssistantMentionTest.kt` carry this same table — a
    /// change here is a change in three places or it is a bug.
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

    /// The picture vectors, mirrored the same way the mention ones are:
    /// `ios/FamilyConnectTests/AssistantMentionTests.swift` and Android's
    /// `AssistantMentionTest.kt` carry this table too, prompt and all, and a
    /// change here is a change in three places or it is a bug.
    ///
    /// Each pair is (body, the prompt that must come out of it) — asserting
    /// the PROMPT rather than a bool is the point: what comes back is the
    /// whole of what will leave the server, so a test that only checked "is
    /// this a draw?" would not notice the token travelling with it.
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
    ];

    const NOT_DRAWS: &[&str] = &[
        "",
        "/draw",
        "  /draw  ",
        "/drawer",
        "/draws a cat",
        "/draw,a cat",
        "draw a cat",
        // The token has to be FIRST. This is the rule that keeps a family
        // DISCUSSING the feature from generating pictures of it.
        "hey @ai /draw a cat",
        "what does /draw do?",
        "please /draw a cat",
        // A mention that is not at the start does not license the token
        // either, for the same reason.
        "look @ai /draw a cat",
        // Two mentions: the second is not a leading one.
        "@ai @ai /draw a cat",
    ];

    #[test]
    fn a_picture_request_is_the_token_first_and_the_words_after_it() {
        for (body, prompt) in DRAWS {
            assert_eq!(
                draw_prompt(body),
                Some(*prompt),
                "{body:?} asks for a picture of {prompt:?}"
            );
        }
    }

    #[test]
    fn everything_else_is_an_ordinary_message() {
        for body in NOT_DRAWS {
            assert_eq!(draw_prompt(body), None, "{body:?} is not a picture request");
        }
    }

    /// The two grammars are independent, and the family chat needs both to
    /// be true at once: the server only looks for `/draw` on a message that
    /// already mentioned the assistant.
    #[test]
    fn a_family_chat_picture_request_is_also_a_mention() {
        assert!(mentions_assistant("@ai /draw a cat"));
        assert_eq!(draw_prompt("@ai /draw a cat"), Some("a cat"));
        // And a picture request in a PRIVATE thread mentions nobody, which
        // is why the private path never consults the mention grammar.
        assert!(!mentions_assistant("/draw a cat"));
    }

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
    fn every_mention_vector_is_a_mention() {
        for body in MENTIONS {
            assert!(mentions_assistant(body), "should be a mention: {body:?}");
        }
    }

    #[test]
    fn no_non_mention_vector_is_a_mention() {
        for body in NOT_MENTIONS {
            assert!(
                !mentions_assistant(body),
                "should NOT be a mention: {body:?}"
            );
        }
    }

    #[test]
    fn the_range_points_at_the_token_itself() {
        assert_eq!(first_mention("hey @ai there"), Some((4, 7)));
        // The first `@` fails the token test; the second one carries it.
        assert_eq!(first_mention("@@ai"), Some((1, 4)));
        // A near miss must not stop the scan finding a real one later.
        assert_eq!(first_mention("@aiden asked @ai"), Some((13, 16)));
    }

    #[test]
    fn a_mention_after_multibyte_text_is_found_at_the_right_byte_offset() {
        // The scan works in BYTES, so a body with multi-byte characters
        // before the token must still report a range that slices cleanly.
        let body = "Привет @ai";
        let (start, end) = first_mention(body).expect("mentioned");
        assert_eq!(&body[start..end], "@ai");
    }
}

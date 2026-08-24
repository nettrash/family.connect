//! Recognising `@ai` in a message body (docs/protocol.md, "Mentioning the
//! assistant in the family chat").
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

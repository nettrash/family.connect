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
    // The token is compared as BYTES, and nothing is sliced until it has
    // matched. `str::split_at` panics when its index is not a char
    // boundary, and the fifth byte of a body that opens in Cyrillic,
    // Japanese or Chinese usually is not one — "Привет" is three two-byte
    // letters, so byte 5 lands inside the third. Every message in an `ai`
    // chat and every `@ai` mention reaches this function, so that was a
    // panic on "hello" in four of the nine languages the apps ship in.
    //
    // Five leading bytes that match an ASCII token ARE five leading
    // characters, so the slice below is in bounds and on a boundary
    // exactly when the comparison passed. The NOT_DRAWS table pins the
    // shapes that used to trap.
    let bytes = rest.as_bytes();
    if bytes.len() < DRAW.len() || !bytes[..DRAW.len()].eq_ignore_ascii_case(DRAW.as_bytes()) {
        return None;
    }
    let after = &rest[DRAW.len()..];
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
    /// change here is a change in three places or it is a bug, and
    /// `the_three_ports_carry_the_same_vectors` below is what says so out
    /// loud rather than leaving it to whoever reads all three files.
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
    /// change here is a change in three places or it is a bug — enforced by
    /// `the_three_ports_carry_the_same_vectors`, which compares all three
    /// tables character by character and in order.
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
        // The PROMPT in the scripts this app is translated into. The token
        // is ASCII, so nothing about it changes; what these pin is that the
        // words after it come back whole rather than sliced at a byte.
        ("/draw кот в шляпе", "кот в шляпе"),
        ("/draw 猫", "猫"),
        ("@ai /draw 🐈 on a mat", "🐈 on a mat"),
        // WHITESPACE IS UNICODE `White_Space`, and these two are the exact
        // code points where the three ports' own libraries disagree about
        // it — which is why each one hand-rolls the predicate rather than
        // asking its standard library (`AssistantMention.kt`'s table and
        // `AssistantMention.swift`'s say the same thing at more length).
        //
        // U+0085 NEXT LINE **is** whitespace. Rust and Swift agree; Java's
        // `Character.isWhitespace` — which is what Kotlin's
        // `Char.isWhitespace()` is built on — says it is not, so Android
        // used to read this body as `/draw` followed by a word and answer
        // "not a picture request" to a request the server acts on.
        ("/draw\u{85}a cat", "a cat"),
        // U+200B ZERO WIDTH SPACE is **not** whitespace, so it stays in the
        // prompt. Rust and Kotlin agree; Foundation's
        // `CharacterSet.whitespacesAndNewlines` — what Apple's side used to
        // trim with — contains it, so iOS trimmed a character out of the
        // prompt the server keeps, and read `/draw \u{200B}` as an empty
        // prompt where the server reads a picture request.
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
        // NON-ASCII BODIES, and they are the reason the token is compared
        // as bytes before anything is sliced. `str::split_at` PANICS when
        // its index is not a char boundary, and the fifth byte of a body
        // that opens in Cyrillic, Japanese or Chinese usually is not one —
        // "Привет" is three two-byte letters, so byte 5 lands inside the
        // third. Every one of these took the request down before the byte
        // comparison went in, and this app ships in ru, ja, zh-Hans and sr:
        // an assistant chat in any of them was one "hello" from a 500.
        "Привет",
        "Привет, как дела?",
        "@ai Привет",
        "こんにちは",
        "你好世界",
        "Здраво",
        // The same trap one byte further in: `/dra` then a three-byte
        // character, so the index falls inside the euro sign.
        "/dra€ a cat",
        // A BARE PREFIX OF THE TOKEN, in each script that trips the byte
        // index. The first three are shorter than the token and stop at the
        // length guard; the rest are five bytes or more with no character
        // boundary at five, which is the only reason the guard is not
        // enough on its own.
        "/d",
        "/dr",
        "/dra",
        "/drи",
        "/drあ",
        "/dr🐈",
        // Cyrillic, Japanese and emoji bodies of every length around the
        // token's five bytes. "Пр" is four bytes, "При" six with byte five
        // inside the third letter, "こん" six with byte five inside the
        // second; an emoji is four bytes on its own and eight in a pair,
        // where byte five falls inside the second.
        "П",
        "Пр",
        "При",
        "Прив",
        "こ",
        "こん",
        "🐈",
        "🐈🐈",
        "🎨 нарисуй кота",
        "@ai 🐈",
        // A HOMOGLYPH: Cyrillic `а` (U+0430) where the token wants ASCII
        // `a`. Six bytes, and it must read as an ordinary message — the
        // token is ASCII, and matching it loosely would be a different
        // grammar from the one the clients highlight.
        "/drаw a cat",
        // U+001C FILE SEPARATOR is **not** whitespace, so this is
        // `/draw` followed by a longer word. Java's
        // `Character.isWhitespace` says it is whitespace, which is what
        // made Android read a picture request here where the server reads
        // none — the mirror image of the U+0085 vector in `DRAWS`.
        "/draw\u{1c}a cat",
        "/draw\u{1d}a cat",
        "/draw\u{1e}a cat",
        "/draw\u{1f}a cat",
        // A COMBINING MARK on the token's last letter. Rust and Kotlin both
        // step one character past `/draw` and find U+0301, which is not
        // whitespace, so this is a longer word. Swift counts in grapheme
        // CLUSTERS, where `w` and its accent are one `Character` — so
        // stepping five of them landed past the accent, on the space, and
        // Apple's side used to call this a picture request.
        "/draw\u{301} a cat",
        "@ai /draw\u{301} a cat",
    ];

    // -- The mirror, made mechanical --------------------------------------

    /// The three tables above say, in a comment, that "a change here is a
    /// change in three places or it is a bug". A comment cannot fail a
    /// build, and this drift is invisible in every other way: each port's
    /// own suite goes green against its own copy, so a vector added here
    /// and nowhere else looks fully tested from any single vantage point.
    /// Two vectors reached the server table and only one client before
    /// somebody read all three files side by side.
    ///
    /// So the sentence is enforced instead of asserted. It reads the two
    /// client test files off disk — they are in this repository, checked
    /// out beside this crate, and their absence is a failure rather than a
    /// skip — and compares the vectors CHARACTER BY CHARACTER, in order.
    /// Order matters as much as membership: these tables are read by
    /// people, and a vector that moved is a vector whose comment now
    /// explains the wrong line.
    #[test]
    fn the_three_ports_carry_the_same_vectors() {
        const SWIFT: &str =
            include_str!("../../ios/FamilyConnectTests/AssistantMentionTests.swift");
        const KOTLIN: &str = include_str!(concat!(
            "../../android/app/src/test/java/me/nettrash/familyconnect/",
            "ui/chat/AssistantMentionTest.kt"
        ));

        // (name, ours, Swift's opening line, Kotlin's opening line)
        let tables: [(&str, Vec<&str>, &str, &str); 3] = [
            (
                "MENTIONS",
                MENTIONS.to_vec(),
                "static let mentions = [",
                "private val mentions = listOf(",
            ),
            (
                // Flattened: each entry is a (body, prompt) pair on all
                // three ports, so the flat sequence compares them both.
                "DRAWS",
                DRAWS.iter().flat_map(|(a, b)| [*a, *b]).collect(),
                "static let draws = [",
                "private val draws = listOf(",
            ),
            (
                "NOT_DRAWS",
                NOT_DRAWS.to_vec(),
                "static let notDraws = [",
                "private val notDraws = listOf(",
            ),
        ];

        for (name, ours, swift_start, kotlin_start) in tables {
            let theirs = swift_vectors(SWIFT, swift_start);
            assert_eq!(
                ours, theirs,
                "{name} has drifted between this file and \
                 ios/FamilyConnectTests/AssistantMentionTests.swift"
            );
            let theirs = kotlin_vectors(KOTLIN, kotlin_start);
            assert_eq!(
                ours, theirs,
                "{name} has drifted between this file and android's \
                 AssistantMentionTest.kt"
            );
        }
    }

    /// The text between `start` and the line that closes the literal, which
    /// on both clients is `]` or `)` at the table's own indentation.
    ///
    /// Panics rather than returning an option: a table this cannot find is
    /// a table that has been renamed or reformatted, and answering "no
    /// vectors, all equal" to that would be the exact failure this test
    /// exists to prevent.
    fn table_block<'a>(source: &'a str, start: &str, close: &str) -> &'a str {
        let from = source
            .find(start)
            .unwrap_or_else(|| panic!("{start:?} is no longer in the client's test file"))
            + start.len();
        let rest = &source[from..];
        let to = rest
            .find(close)
            .unwrap_or_else(|| panic!("{start:?} is never closed by {close:?}"));
        &rest[..to]
    }

    /// Every double-quoted literal in a block, in order, as RAW source —
    /// comments skipped, because these tables are more comment than code.
    ///
    /// A hand-rolled scan rather than a regular expression: a `//` inside a
    /// string literal is text (the tables contain `what does /draw do?`),
    /// and a `"` inside a comment is not a literal. Only a scanner that
    /// knows which of the two it is in can tell those apart.
    fn raw_literals(block: &str) -> Vec<String> {
        let mut found = Vec::new();
        let bytes: Vec<char> = block.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                '/' if bytes.get(i + 1) == Some(&'/') => {
                    while i < bytes.len() && bytes[i] != '\n' {
                        i += 1;
                    }
                }
                '"' => {
                    let mut literal = String::new();
                    i += 1;
                    while i < bytes.len() && bytes[i] != '"' {
                        if bytes[i] == '\\' && i + 1 < bytes.len() {
                            literal.push(bytes[i]);
                            literal.push(bytes[i + 1]);
                            i += 2;
                            continue;
                        }
                        literal.push(bytes[i]);
                        i += 1;
                    }
                    i += 1;
                    found.push(literal);
                }
                _ => i += 1,
            }
        }
        found
    }

    /// Swift's vectors, with `\u{…}` and the two escapes these tables use
    /// resolved to the characters they stand for.
    fn swift_vectors(source: &str, start: &str) -> Vec<String> {
        raw_literals(table_block(source, start, "\n    ]"))
            .iter()
            .map(|literal| {
                let mut out = String::new();
                let chars: Vec<char> = literal.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] != '\\' {
                        out.push(chars[i]);
                        i += 1;
                        continue;
                    }
                    match chars.get(i + 1) {
                        Some('u') => {
                            let close = chars[i..]
                                .iter()
                                .position(|c| *c == '}')
                                .expect("a \\u{…} escape closes");
                            let hex: String = chars[i + 3..i + close].iter().collect();
                            out.push(
                                char::from_u32(
                                    u32::from_str_radix(&hex, 16).expect("hex in \\u{…}"),
                                )
                                .expect("a scalar in \\u{…}"),
                            );
                            i += close + 1;
                        }
                        Some('n') => {
                            out.push('\n');
                            i += 2;
                        }
                        Some(other) => {
                            out.push(*other);
                            i += 2;
                        }
                        None => panic!("a literal ends in a backslash"),
                    }
                }
                out
            })
            .collect()
    }

    /// Kotlin's vectors. Its `\uXXXX` is a UTF-16 CODE UNIT, so an emoji is
    /// written as a surrogate PAIR — which is why this collects units and
    /// decodes at the end rather than a scalar at a time.
    fn kotlin_vectors(source: &str, start: &str) -> Vec<String> {
        raw_literals(table_block(source, start, "\n    )"))
            .iter()
            .map(|literal| {
                let mut units: Vec<u16> = Vec::new();
                let chars: Vec<char> = literal.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] != '\\' {
                        let mut buffer = [0u16; 2];
                        units.extend_from_slice(chars[i].encode_utf16(&mut buffer));
                        i += 1;
                        continue;
                    }
                    match chars.get(i + 1) {
                        Some('u') => {
                            let hex: String = chars[i + 2..i + 6].iter().collect();
                            units.push(u16::from_str_radix(&hex, 16).expect("hex in \\uXXXX"));
                            i += 6;
                        }
                        Some('n') => {
                            units.push(b'\n' as u16);
                            i += 2;
                        }
                        Some(other) => {
                            let mut buffer = [0u16; 2];
                            units.extend_from_slice(other.encode_utf16(&mut buffer));
                            i += 2;
                        }
                        None => panic!("a literal ends in a backslash"),
                    }
                }
                String::from_utf16(&units).expect("well-formed UTF-16 in the Kotlin table")
            })
            .collect()
    }

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

    /// EVERY PREFIX of every vector, and the point is that none of them
    /// panics.
    ///
    /// `draw_prompt` runs on the request path — on every message in an `ai`
    /// thread and every family-chat message that mentions the assistant —
    /// so a panic here is a 500 on a sentence, not a wrong answer. It got
    /// one from a five-BYTE slice taken without asking whether byte five
    /// was a character boundary, and "Привет" is the sentence that found
    /// it. A prefix walk is the shape of test that would have caught it
    /// without anyone having to think of the right six letters: it cuts
    /// each vector at every character boundary and asks for an answer,
    /// which is exactly the family of inputs a fixed-width slice mishandles.
    #[test]
    fn no_prefix_of_any_vector_panics() {
        let bodies = DRAWS
            .iter()
            .map(|(body, _)| *body)
            .chain(NOT_DRAWS.iter().copied())
            .chain(MENTIONS.iter().copied())
            .chain(NOT_MENTIONS.iter().copied());
        for body in bodies {
            for (index, _) in body.char_indices() {
                let head = &body[..index];
                // The answer itself is not the subject — returning one at
                // all is. Both grammars are walked, because both index the
                // same string and the byte scan in `first_mention` is the
                // other place a bad index could be taken.
                let _ = draw_prompt(head);
                let _ = mentions_assistant(head);
            }
            let _ = draw_prompt(body);
            let _ = mentions_assistant(body);
        }
    }

    /// The whitespace table, code point by code point.
    ///
    /// Whitespace is where the three ports part company: this is Rust's
    /// `char::is_whitespace` (Unicode `White_Space`), Kotlin's
    /// `Char.isWhitespace()` is Java's union of `isWhitespace` and
    /// `isSpaceChar`, and Apple's side had Foundation's
    /// `CharacterSet.whitespacesAndNewlines`. Three tables, three answers,
    /// one grammar — so both clients now hand-roll THIS one, and this test
    /// is what they are pinned against.
    ///
    /// The six code points that separated them are called out by name; the
    /// rest of the table is here so a future port can be checked against it
    /// in full.
    #[test]
    fn whitespace_is_the_unicode_property_and_nothing_else() {
        const WHITESPACE: &[char] = &[
            '\u{9}', '\u{a}', '\u{b}', '\u{c}', '\u{d}', '\u{20}', '\u{85}', '\u{a0}', '\u{1680}',
            '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
            '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}', '\u{2028}', '\u{2029}', '\u{202f}',
            '\u{205f}', '\u{3000}',
        ];
        for c in WHITESPACE {
            assert!(c.is_whitespace(), "U+{:04X} is whitespace", *c as u32);
        }
        // The five Java calls whitespace and Unicode does not, plus the one
        // Foundation calls whitespace and Unicode does not.
        for c in ['\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', '\u{200b}'] {
            assert!(!c.is_whitespace(), "U+{:04X} is NOT whitespace", c as u32);
        }
        // And U+0085, which Java alone leaves out, is in.
        assert!('\u{85}'.is_whitespace());
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

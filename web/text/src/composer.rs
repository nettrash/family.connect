//! The draft, and the one limit the wire puts on it — ported from
//! ios/FamilyConnect/Core/ComposerText.swift.
//!
//! A message body is 4000 characters (docs/protocol.md, "Limits"), and a
//! client that does not count them lets a pasted wall of text fill the
//! field, look as though it worked, and come back from `POST
//! /chats/{id}/messages` as `message_too_long` at Send, minutes later. The
//! ceiling is enforced where the text ARRIVES, and said out loud when it
//! bites.
//!
//! Counted in UNICODE SCALARS — Rust's `char` — which is what the server
//! counts (`body.chars().count()` over a trimmed body). Untrimmed here, as
//! on Apple: at most a few characters STRICTER than the server, and erring
//! the other way is how "you are within the limit" turns into a refused
//! send. (Android trims before counting, with Kotlin's whitespace.)
//!
//! Cutting, though, happens only between grapheme clusters — Swift's
//! `Character`s. A cut between scalars lands inside a cluster and leaves an
//! emoji's debris in the field: a skin tone with no hand, a flag with half
//! its letters.

use unicode_segmentation::UnicodeSegmentation;

/// docs/protocol.md, "Limits": a message body, 4000 characters. A
/// self-hosted server may be configured lower and does not say so; this is
/// what the client holds itself to.
pub const BODY_LIMIT: usize = 4000;

/// What became of words pasted into a draft.
///
/// Three outcomes rather than a bool, because they are three different
/// things to say: nothing, "the end was left out", and "there was no room
/// at all". A paste that quietly drops half of what was on the clipboard is
/// the failure this replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Paste {
    /// All of it fit. The new draft.
    Appended(String),
    /// Some of it fit. The new draft, with the tail dropped.
    Truncated(String),
    /// None of it fit — the draft is already at the ceiling.
    Full,
}

impl Paste {
    /// The sentence this outcome owes the writer, if any.
    pub fn notice(&self) -> Option<Notice> {
        match self {
            Paste::Appended(_) => None,
            Paste::Truncated(_) => Some(Notice::Truncated),
            Paste::Full => Some(Notice::Full),
        }
    }
}

/// A sentence the composer says when the ceiling bites. An enum rather than
/// a string so a translation table can be keyed by it; [`Notice::english`]
/// is the source-language rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Notice {
    /// A draft that got past every door was cut back ([`clamping`]).
    Clamped,
    /// Part of a paste was left out ([`Paste::Truncated`]).
    Truncated,
    /// None of a paste fit ([`Paste::Full`]).
    Full,
}

impl Notice {
    /// The words the Apple composers show, in English.
    ///
    /// The limit is written `4,000`: Apple's `String(localized:)` formats an
    /// interpolated integer for the reader's locale, and an English one
    /// groups thousands. (Android's `%1$d` prints `4000`.)
    pub fn english(self) -> String {
        let limit = grouped(BODY_LIMIT);
        match self {
            Notice::Clamped => format!("A message can be at most {limit} characters."),
            Notice::Truncated => {
                format!("A message can be at most {limit} characters. The rest wasn't pasted.")
            }
            Notice::Full => format!("The message is already at the {limit}-character limit."),
        }
    }
}

/// The length the server will measure: Unicode scalars.
pub fn length(text: &str) -> usize {
    text.chars().count()
}

/// Append pasted words to a draft, within the ceiling.
pub fn appending(addition: &str, draft: &str) -> Paste {
    let used = length(draft);
    // Over the ceiling is still full, never negative room.
    if used >= BODY_LIMIT {
        return Paste::Full;
    }
    let room = BODY_LIMIT - used;
    if length(addition) <= room {
        return Paste::Appended(format!("{draft}{addition}"));
    }
    let head = prefix(addition, room);
    // A single grapheme cluster wider than the room left has no character
    // at all that fits, and half of one is not a character.
    if head.is_empty() {
        return Paste::Full;
    }
    Paste::Truncated(format!("{draft}{head}"))
}

/// A draft cut down to the ceiling, or `None` when it is already inside it.
///
/// The backstop for the doors that cannot be intercepted — whatever put the
/// text in the field, what can always be seen is the draft it left behind.
/// A slice of the caller's draft: the cut is only ever a shortening.
pub fn clamping(draft: &str) -> Option<&str> {
    (length(draft) > BODY_LIMIT).then(|| prefix(draft, BODY_LIMIT))
}

/// The longest whole-Character prefix whose scalars fit in `room`.
///
/// `text` is segmented on its own, as Swift's `for character in text` does,
/// so the cut can only land where a Character of `text` ends.
fn prefix(text: &str, room: usize) -> &str {
    let mut used = 0;
    let mut end = 0;
    for (offset, character) in text.grapheme_indices(true) {
        let width = character.chars().count();
        if used + width > room {
            break;
        }
        used += width;
        end = offset + character.len();
    }
    &text[..end]
}

// MARK: - Sending

/// What the Apple clients SEND for a draft: trimmed at both ends, or `None`
/// when nothing is left — the composers' `canSend` and the coordinator's
/// send both ask exactly this before anything leaves.
///
/// Trimmed the way they trim, with Foundation's
/// `CharacterSet.whitespacesAndNewlines`, scalar by scalar — see
/// [`is_foundation_whitespace`]. That is NOT the server's trim: a body of
/// nothing but U+200B ZERO WIDTH SPACE is one the server would accept and
/// the Apple composers refuse to send, and a ZERO WIDTH SPACE at either end
/// of a body is gone before the server sees it.
pub fn trimmed_for_send(draft: &str) -> Option<&str> {
    let body = trim_foundation_whitespace(draft);
    (!body.is_empty()).then_some(body)
}

/// Foundation's `trimmingCharacters(in: .whitespacesAndNewlines)`: scalars
/// in the set are taken off both ends, one scalar at a time — a space under
/// a combining mark goes and leaves the mark, as it does on Apple.
pub(crate) fn trim_foundation_whitespace(text: &str) -> &str {
    text.trim_matches(is_foundation_whitespace)
}

/// Mirrors Foundation's `CharacterSet.whitespacesAndNewlines`, which is
/// Unicode `White_Space` — Rust's `char::is_whitespace` — plus U+200B ZERO
/// WIDTH SPACE, a space separator in Unicode 3 that Foundation never let
/// go of. Established by walking every scalar through both on macOS 26
/// (Swift 6.4): U+200B is the one difference.
///
/// The picture grammar deliberately does NOT use this set (see
/// `assistant`): the server keeps U+200B in a prompt.
pub fn is_foundation_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{200B}'
}

/// `n` with thousands grouped by commas — `4,000`.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - The unit

    /// The server counts scalars; one family emoji is one Character and
    /// seven scalars.
    #[test]
    fn length_is_scalars() {
        let family = "👨‍👩‍👧‍👦";
        assert_eq!(family.graphemes(true).count(), 1);
        assert_eq!(length(family), 7);
        assert_eq!(length("hello"), 5);
        assert_eq!(length(""), 0);
    }

    #[test]
    fn ceiling() {
        assert_eq!(BODY_LIMIT, 4000);
    }

    // MARK: - Pasting into a draft

    #[test]
    fn fitting_paste_is_appended() {
        assert_eq!(
            appending(" world", "hello"),
            Paste::Appended("hello world".into())
        );
        assert_eq!(appending("hello", ""), Paste::Appended("hello".into()));
    }

    #[test]
    fn oversized_paste_is_truncated() {
        let draft = "a".repeat(3990);
        let wall = "b".repeat(5000);
        let Paste::Truncated(result) = appending(&wall, &draft) else {
            panic!("expected the paste to be truncated");
        };
        assert_eq!(length(&result), BODY_LIMIT);
        assert!(result.starts_with(&draft));
        assert!(result.ends_with(&"b".repeat(10)));
    }

    #[test]
    fn full_draft_refuses() {
        let draft = "a".repeat(BODY_LIMIT);
        assert_eq!(appending("more", &draft), Paste::Full);
        // ...and over the ceiling is still full, never negative room.
        let over = "a".repeat(BODY_LIMIT + 10);
        assert_eq!(appending("more", &over), Paste::Full);
    }

    #[test]
    fn cuts_fall_on_character_boundaries() {
        // Two scalars of room, and a family emoji is seven.
        let draft = "a".repeat(BODY_LIMIT - 2);
        assert_eq!(appending("👨‍👩‍👧‍👦", &draft), Paste::Full);
        // Room for one plain character before it, and not for the emoji.
        let Paste::Truncated(result) = appending("x👨‍👩‍👧‍👦", &draft) else {
            panic!("expected the paste to be truncated");
        };
        assert!(result.ends_with('x'));
        assert_eq!(length(&result), BODY_LIMIT - 1);
    }

    // MARK: - The door nobody owns

    #[test]
    fn clamping_is_the_backstop() {
        assert_eq!(clamping("short"), None);
        assert_eq!(clamping(&"a".repeat(BODY_LIMIT)), None);
        let over = "a".repeat(BODY_LIMIT + 1);
        let clamped = clamping(&over).expect("clamped");
        assert_eq!(length(clamped), BODY_LIMIT);
    }

    #[test]
    fn clamping_falls_on_character_boundaries() {
        let over = "a".repeat(BODY_LIMIT - 2) + "👨‍👩‍👧‍👦";
        let clamped = clamping(&over).expect("clamped");
        assert!(!clamped.contains('👨'));
        assert_eq!(length(clamped), BODY_LIMIT - 2);
    }

    // MARK: - Beyond the Swift suite

    /// Every cluster shape a cut could split — combining marks, a flag, a
    /// skin tone, Cyrillic, CJK — cut at every room from nothing to all of
    /// it, and the cut always lands between Characters.
    #[test]
    fn every_room_cuts_between_characters() {
        let text = "Анна\u{301}🇷🇸👋🏽王芳e\u{301}\u{302}x";
        let boundaries: Vec<usize> = text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain([text.len()])
            .collect();
        for room in 0..=length(text) {
            let head = prefix(text, room);
            assert!(boundaries.contains(&head.len()), "room {room}: {head:?}");
            assert!(length(head) <= room);
        }
    }

    /// The notices, as the Apple composers word them.
    #[test]
    fn notices_say_what_apple_says() {
        assert_eq!(
            Notice::Clamped.english(),
            "A message can be at most 4,000 characters."
        );
        assert_eq!(
            Notice::Truncated.english(),
            "A message can be at most 4,000 characters. The rest wasn't pasted."
        );
        assert_eq!(
            Notice::Full.english(),
            "The message is already at the 4,000-character limit."
        );
        assert_eq!(appending("x", "").notice(), None);
        assert_eq!(
            appending("x", &"a".repeat(BODY_LIMIT)).notice(),
            Some(Notice::Full)
        );
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }

    /// Foundation's set is White_Space and U+200B, and the send trim is
    /// scalar by scalar.
    #[test]
    fn the_send_trim_is_foundations() {
        assert_eq!(trimmed_for_send("  hi \n"), Some("hi"));
        assert_eq!(trimmed_for_send(" \u{85}\u{3000}\t"), None);
        assert_eq!(
            trimmed_for_send("\u{200B}"),
            None,
            "Foundation takes a ZERO WIDTH SPACE"
        );
        assert_eq!(trimmed_for_send("\u{200B}hi\u{200B}\u{85}"), Some("hi"));
        // The Java-only separators are not whitespace to Foundation either.
        assert_eq!(trimmed_for_send("\u{1C}"), Some("\u{1C}"));
        // A space under a combining mark goes, and leaves the mark.
        assert_eq!(trimmed_for_send(" \u{301}a"), Some("\u{301}a"));
        assert_eq!(trimmed_for_send("a \u{301}"), Some("a \u{301}"));
        let set: Vec<char> = (0..=0x10FFFF)
            .filter_map(char::from_u32)
            .filter(|c| is_foundation_whitespace(*c) != c.is_whitespace())
            .collect();
        assert_eq!(set, ['\u{200B}']);
    }
}

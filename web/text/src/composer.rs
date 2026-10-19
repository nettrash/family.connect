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
use crate::i18n::t1;

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
    /// The words the Apple composers show, in the reader's language.
    ///
    /// The limit is written `4,000`: Apple's `String(localized:)` formats an
    /// interpolated integer for the reader's locale, and an English one
    /// groups thousands. (Android's `%1$d` prints `4000`.) It is filled into
    /// the key's `%lld` already grouped.
    pub fn said(self) -> String {
        let limit = grouped(BODY_LIMIT);
        match self {
            Notice::Clamped => t1("A message can be at most %lld characters.", &limit),
            Notice::Truncated => t1(
                "A message can be at most %lld characters. The rest wasn't pasted.",
                &limit,
            ),
            Notice::Full => t1(
                "The message is already at the %lld-character limit.",
                &limit,
            ),
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

/// Return pressed with these modifiers held makes a new line in the field
/// (issue #95); without any it is the field's own Enter — Send. Shift, Ctrl,
/// Alt/Option or Cmd/Meta: whichever habit somebody brings, no setting. The
/// Mac's `NewLineKeys.returnMakesLine` and Windows' `NewLineKeys.EnterMakesLine`
/// answer the same.
pub fn return_makes_line(shift: bool, ctrl: bool, alt: bool, meta: bool) -> bool {
    shift || ctrl || alt || meta
}

// MARK: - Capital letters (issue #97)
//
// docs/capitals-and-rich-markdown-2026-10-17.md, "The rule": on a computer
// no keyboard capitalises a sentence, so the desktop clients do it
// themselves — the web on devices with a fine pointer and hover, in the
// multi-line fields. A phone's keyboard does its own and the field only asks
// (`autocapitalize="sentences"`). The Mac (`Capitals.swift`) and Windows
// (`Capitals.cs`) hold the same table.

/// The marks that may open a word before its first letter: `"(Hi`.
const OPENING_MARKS: [char; 7] = ['"', '\'', '(', '[', '«', '“', '‘'];

/// The marks that may close a sentence after its stop: `(Done.)`.
const CLOSING_MARKS: [char; 7] = ['"', '\'', ')', ']', '»', '”', '’'];

/// A line break as any field may hold one: a line feed, or the lone
/// carriage return a WinUI `TextBox` keeps.
fn is_line_break(c: char) -> bool {
    c == '\n' || c == '\r'
}

/// The capital the rule would put in place of `typed`: `typed` must be a
/// lower-case letter whose upper case — the invariant one, never a locale's
/// — is exactly one character, and a different one. `ß` (upper case `SS`)
/// has none.
pub fn capital_of(typed: char) -> Option<char> {
    if !typed.is_lowercase() {
        return None;
    }
    let mut upper = typed.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(capital), None) if capital != typed => Some(capital),
        _ => None,
    }
}

/// Whether `typed`, the one character the keyboard just produced, is put in
/// as its capital. `before` is the field's text before the caret (the
/// selection's start).
///
/// YES only when all four hold:
///
/// 1. `typed` has a capital ([`capital_of`]);
/// 2. a word is starting: with any opening marks typed just before it set
///    aside, `before` is empty or ends in a space, a tab or a line break;
/// 3. a sentence is starting: `before`, without those marks and without
///    trailing spaces and tabs, is empty, ends in a line break, is nothing
///    on its line but a list, heading or quote marker, or ends in `.`, `!`
///    or `?` (closing marks after it allowed) — where a `.` after a `.` (an
///    ellipsis) or ending an abbreviation (`e.g.`, `U.S.`) does not count;
/// 4. the caret is not in code: an even number of fence lines before it,
///    and an even number of backticks on its own line.
pub fn capitalizes(before: &str, typed: char) -> bool {
    if capital_of(typed).is_none() {
        return false;
    }
    let core = before.trim_end_matches(OPENING_MARKS);
    // 2. The word is just starting.
    if !core.is_empty() && !core.ends_with([' ', '\t', '\n', '\r']) {
        return false;
    }
    // 3. So is a sentence.
    if !starts_sentence(core.trim_end_matches([' ', '\t'])) {
        return false;
    }
    // 4. And the caret is not in code.
    !in_code(before)
}

/// Rule 3 over `before` with the opening marks and trailing blanks gone.
fn starts_sentence(text: &str) -> bool {
    if text.is_empty() || text.ends_with(is_line_break) {
        return true;
    }
    let line = text.rsplit(is_line_break).next().unwrap_or(text);
    if is_marker(line.trim_start_matches([' ', '\t'])) {
        return true;
    }
    let stopped = text.trim_end_matches(CLOSING_MARKS);
    let Some(stop) = stopped.chars().next_back() else {
        return false;
    };
    match stop {
        '!' | '?' => true,
        '.' => {
            let head = &stopped[..stopped.len() - 1];
            // An ellipsis trails off rather than ending anything.
            if head.ends_with('.') {
                return false;
            }
            !is_abbreviation(stopped)
        }
        _ => false,
    }
}

/// A list, heading or quote marker and nothing else: `-`, `*`, `+`, `>`, one
/// to six `#`, or digits followed by `.` or `)`.
fn is_marker(line: &str) -> bool {
    match line {
        "-" | "*" | "+" | ">" => return true,
        _ => {}
    }
    if !line.is_empty() && line.len() <= 6 && line.bytes().all(|b| b == b'#') {
        return true;
    }
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && matches!(&line[digits..], "." | ")")
}

/// Whether the `.` that ends `text` ends an abbreviation: the word back to
/// the previous space holds another `.`, and every piece between the dots
/// is one or two ASCII letters (`e.g.`, `i.e.`, `a.m.`, `U.S.`) — which
/// `example.com.` is not.
fn is_abbreviation(text: &str) -> bool {
    let word = text.rsplit([' ', '\t', '\n', '\r']).next().unwrap_or(text);
    let pieces = &word[..word.len() - 1];
    pieces.contains('.')
        && pieces.split('.').all(|piece| {
            (1..=2).contains(&piece.len()) && piece.bytes().all(|b| b.is_ascii_alphabetic())
        })
}

/// Rule 4: the caret is inside a fence (an odd number of lines before it
/// start with three backticks) or a code span (an odd number of backticks
/// on its own line, outside a fence). Emoji as you type (#101) skips code
/// by the same test.
pub(crate) fn in_code(before: &str) -> bool {
    let fences = before
        .split(is_line_break)
        .filter(|line| line.starts_with("```"))
        .count();
    if fences % 2 == 1 {
        return true;
    }
    let line = before.rsplit(is_line_break).next().unwrap_or(before);
    line.matches('`').count() % 2 == 1
}

/// A capital the rule just put in, remembered so the Backspace pressed
/// straight after it can take it back (docs/capitals-and-rich-markdown,
/// "Taking it back").
///
/// The UI keeps one of these from the moment it capitalises until anything
/// else happens — another key, a click, a paste, a selection — and then
/// drops it. Offsets are BYTE offsets into the field's text; a browser's
/// `selectionStart` counts UTF-16 units, so the web converts at the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakeBack {
    /// Where the capital starts.
    pub at: usize,
    /// The capital that was put in.
    pub capital: char,
    /// What was typed: the lower-case letter a Backspace puts back.
    pub typed: char,
}

impl TakeBack {
    /// Remember that `typed` went in at `at` as its capital, or `None` when
    /// it has none.
    pub fn new(at: usize, typed: char) -> Option<TakeBack> {
        Some(TakeBack {
            at,
            capital: capital_of(typed)?,
            typed,
        })
    }

    /// A Backspace with the caret at `caret` and nothing selected: the text
    /// with the lower-case letter back, and where the caret goes (right
    /// behind it) — or `None` when the caret is not right behind the capital
    /// or the capital is no longer there, and the Backspace is an ordinary
    /// one.
    pub fn backspace(&self, text: &str, caret: usize) -> Option<(String, usize)> {
        let end = self.at + self.capital.len_utf8();
        if caret != end || text.get(self.at..end)? != self.capital.encode_utf8(&mut [0; 4]) {
            return None;
        }
        let mut undone = String::with_capacity(text.len() + 4);
        undone.push_str(&text[..self.at]);
        undone.push(self.typed);
        undone.push_str(&text[end..]);
        Some((undone, self.at + self.typed.len_utf8()))
    }
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

    // MARK: - Capital letters (#97)

    /// docs/capitals-and-rich-markdown-2026-10-17.md, "The table every
    /// port's tests hold", verbatim (`⏎` is a line feed, `⇥` a tab).
    #[test]
    fn capitals_the_design_table() {
        let table: &[(&str, char, bool)] = &[
            ("", 'h', true),
            ("Hi. ", 'h', true),
            // The word is not starting.
            ("Hi.", 'h', false),
            ("hi ", 'h', false),
            ("Really? ", 'y', true),
            ("Wow! ", 's', true),
            // An ellipsis.
            ("Wait... ", 'w', false),
            // An abbreviation.
            ("See e.g. ", 't', false),
            ("It's at example.com. ", 't', true),
            ("Line\n", 'n', true),
            ("He said \"stop.\" ", 't', true),
            ("(Done.) ", 'n', true),
            ("\"", 'h', true),
            ("Hi. (", 's', true),
            ("- ", 'm', true),
            ("List:\n1. ", 'b', true),
            ("12) ", 'x', true),
            ("# ", 't', true),
            ("> ", 'q', true),
            // Inside a fence.
            ("```\n", 'l', false),
            // The fence closed.
            ("```\ncode\n```\n", 'n', true),
            // Inside a code span.
            ("Use `x. ", 'y', false),
            // Already upper case.
            ("Hi. ", 'H', false),
            // Not a letter.
            ("Hi. ", '1', false),
            // Its upper case is two letters.
            ("", 'ß', false),
            // Two spaces.
            ("Hi.  ", 'h', true),
            // A tab.
            ("Hi.\t", 'h', true),
            ("", 'é', true),
            ("Привет. ", 'к', true),
            ("Ok.\n\n", 'n', true),
            ("@ann ", 'h', false),
        ];
        for &(before, typed, expected) in table {
            assert_eq!(
                capitalizes(before, typed),
                expected,
                "{before:?} + {typed:?}"
            );
        }
    }

    /// The edges of each condition, beyond the table.
    #[test]
    fn capitals_edges() {
        // Every opening and closing mark.
        for mark in OPENING_MARKS {
            assert!(capitalizes(&format!("Hi. {mark}"), 'a'), "{mark}");
            assert!(capitalizes(&format!("{mark}{mark}"), 'a'), "{mark}");
        }
        for mark in CLOSING_MARKS {
            assert!(capitalizes(&format!("Done.{mark} "), 'a'), "{mark}");
            assert!(capitalizes(&format!("Done!{mark}{mark} "), 'a'), "{mark}");
        }
        // A WinUI field's lone carriage return is a line break.
        assert!(capitalizes("Line\r", 'n'));
        assert!(!capitalizes("```\r", 'n'));
        assert!(capitalizes("```\rcode\r```\r", 'n'));
        // Markers: indented, every heading depth, and their near misses.
        assert!(capitalizes("  - ", 'a'));
        assert!(capitalizes("###### ", 'a'));
        assert!(!capitalizes("####### ", 'a'));
        assert!(!capitalizes("-- ", 'a'));
        assert!(!capitalizes("1.2 ", 'a'));
        // A marker is the whole line: a dash after a sentence is not one.
        assert!(!capitalizes("Hi. - ", 'a'));
        // Abbreviations: the pieces must be one or two ASCII letters.
        assert!(!capitalizes("a.m. ", 'a'));
        assert!(!capitalizes("in the U.S. ", 'a'));
        assert!(capitalizes("v1.2. ", 'a'));
        assert!(capitalizes("abc.d. ", 'a'));
        assert!(capitalizes("é.g. ", 'a'));
        // A single-letter word with one dot is a sentence's end.
        assert!(capitalizes("Plan B. ", 'a'));
        // `?` and `!` are never an ellipsis.
        assert!(capitalizes("What?! ", 'a'));
        // Other ends are not sentence ends.
        assert!(!capitalizes("Hi, ", 'a'));
        assert!(!capitalizes("Note: ", 'a'));
        // A fence line counts only at the start of a line; a closed code
        // span is not code.
        assert!(capitalizes("x ```\n", 'a'));
        assert!(capitalizes("Use `x`. ", 'a'));
        // Letters with no single capital, or none at all.
        for typed in ['ŉ', 'ǰ', 'ﬀ', 'ª', '日', ' ', '.'] {
            assert!(!capitalizes("", typed), "{typed:?}");
        }
        assert_eq!(capital_of('a'), Some('A'));
        assert_eq!(capital_of('ж'), Some('Ж'));
        assert_eq!(capital_of('ß'), None);
        assert_eq!(capital_of('A'), None);
        // Outside the Basic Multilingual Plane: Deseret.
        assert_eq!(capital_of('\u{10428}'), Some('\u{10400}'));
        assert!(capitalizes("Hi. ", '\u{10428}'));
    }

    /// A Backspace straight after the capital puts the lower-case letter
    /// back; anywhere else it is an ordinary Backspace.
    #[test]
    fn capitals_take_back() {
        let take = TakeBack::new(4, 'h').expect("h has a capital");
        assert_eq!(take.backspace("Hi. H", 5), Some(("Hi. h".to_string(), 5)));
        // Not right behind it, or the capital has gone.
        assert_eq!(take.backspace("Hi. H", 4), None);
        assert_eq!(take.backspace("Hi. Ho", 6), None);
        assert_eq!(take.backspace("Hi. x", 5), None);
        assert_eq!(take.backspace("Hi.", 5), None);
        // Text after the caret is kept.
        assert_eq!(
            take.backspace("Hi. Hello", 5),
            Some(("Hi. hello".to_string(), 5))
        );
        // Multi-byte letters, by byte offset.
        let cyrillic = TakeBack::new("Привет. ".len(), 'к').unwrap();
        let text = "Привет. К";
        assert_eq!(
            cyrillic.backspace(text, text.len()),
            Some(("Привет. к".to_string(), "Привет. к".len()))
        );
        // A caret inside a character is never a byte offset off a boundary.
        assert_eq!(cyrillic.backspace(text, text.len() - 1), None);
        assert_eq!(TakeBack::new(0, 'ß'), None);
    }

    // MARK: - New lines (#95)

    #[test]
    fn return_alone_sends_and_any_modifier_makes_a_line() {
        assert!(!return_makes_line(false, false, false, false));
        assert!(return_makes_line(true, false, false, false));
        assert!(return_makes_line(false, true, false, false));
        assert!(return_makes_line(false, false, true, false));
        assert!(return_makes_line(false, false, false, true));
        assert!(return_makes_line(true, true, true, true));
    }

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
            Notice::Clamped.said(),
            "A message can be at most 4,000 characters."
        );
        assert_eq!(
            Notice::Truncated.said(),
            "A message can be at most 4,000 characters. The rest wasn't pasted."
        );
        assert_eq!(
            Notice::Full.said(),
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

//
//  AssistantMention.swift
//  FamilyConnect
//
//  Recognising what a message body ASKS the assistant for: `@ai`
//  (docs/protocol.md, "Mentioning the assistant in the family chat") and
//  `/draw` (protocol.md, "Pictures"). Two grammars, one file, because the
//  second one is defined in terms of the first — a family-chat picture
//  request is `@ai` and then the token.
//
//  This grammar is a WIRE CONTRACT, not a rendering detail. The SERVER
//  decides from it whether a family-chat message reaches the assistant at
//  all, and this file decides where the highlight goes. If the two
//  disagree, a family watches a highlighted question go unanswered with no
//  way to tell why — or, worse, an ordinary message quietly leaves the
//  building. Mirrored by value in `server/src/mentions.rs` and
//  `android/…/ui/chat/AssistantMention.kt`, with the same vectors pinned on
//  all three. Change it in three places or it is a bug.
//
//  The rule, in full:
//
//  - the token is the three characters `@ai`, matched case-insensitively
//    but only over ASCII, so `@AI` and `@Ai` are mentions;
//  - the `@` must start the body or follow a character that is not an ASCII
//    letter, digit or `_` — which is what stops `anna@ai.example`;
//  - the `i` must end the body or be followed by a character that is not an
//    ASCII letter, digit or `_` — which is what stops `@aiden`.
//
//  The boundary test is ASCII-only on purpose. A Unicode one would refuse
//  `@ai` written against Japanese or Russian with no space after it —
//  languages this app is translated into, where a space would not normally
//  be typed — and it would make three implementations depend on three
//  Unicode tables agreeing, which is exactly the trap `EmojiOnly` had to
//  hand-roll its own whitespace table to escape. The ambiguity the boundary
//  exists to resolve (`@aiden`) only arises in ASCII anyway.
//

import Foundation

nonisolated enum AssistantMention {
    /// The token itself, so nothing spells it twice.
    static let token = "@ai"

    /// Does this body address the assistant?
    static func mentions(_ body: String) -> Bool {
        !ranges(in: body).isEmpty
    }

    /// Every `@ai` in `body`, as ranges into that same string.
    ///
    /// ALL of them, not just the first: the server only needs to know
    /// whether there is one, but a bubble highlighting the first and
    /// leaving a second plain would look like a typo.
    ///
    /// The scan works in UTF-8 BYTES, and that is exact rather than a
    /// shortcut — a continuation or lead byte is never an ASCII letter,
    /// digit or `_`, so "the adjacent byte is not one of those" and "the
    /// adjacent character is not one of those" are the same statement.
    /// `@ai` is ASCII, so every index handled here is a character boundary.
    static func ranges(in body: String) -> [Range<String.Index>] {
        let utf8 = body.utf8
        let bytes = Array(utf8)
        guard bytes.count >= 3 else { return [] }
        var found: [Range<String.Index>] = []
        for index in 0...(bytes.count - 3) {
            guard bytes[index] == UInt8(ascii: "@"),
                bytes[index + 1] | 0x20 == UInt8(ascii: "a"),
                bytes[index + 2] | 0x20 == UInt8(ascii: "i"),
                index == 0 || isBoundary(bytes[index - 1]),
                index + 3 == bytes.count || isBoundary(bytes[index + 3])
            else { continue }
            guard
                let lowerUTF8 = utf8.index(
                    utf8.startIndex, offsetBy: index, limitedBy: utf8.endIndex),
                let upperUTF8 = utf8.index(
                    utf8.startIndex, offsetBy: index + 3, limitedBy: utf8.endIndex),
                let lower = lowerUTF8.samePosition(in: body),
                let upper = upperUTF8.samePosition(in: body)
            else { continue }
            found.append(lower..<upper)
        }
        return found
    }

    /// ASCII letters, digits and `_` are what a token may NOT sit against.
    /// Anything else — punctuation, whitespace, the lead or continuation
    /// byte of a multi-byte character — is a boundary.
    private static func isBoundary(_ byte: UInt8) -> Bool {
        let isDigit = byte >= 0x30 && byte <= 0x39
        let isUpper = byte >= 0x41 && byte <= 0x5A
        let isLower = byte >= 0x61 && byte <= 0x7A
        return !(isDigit || isUpper || isLower || byte == UInt8(ascii: "_"))
    }

    // MARK: - `/draw` (docs/protocol.md, "Pictures")

    /// The picture token, so nothing spells it twice.
    ///
    /// The SECOND wire-contract grammar in this file, and a contract for
    /// the reason the first one is: the server decides from it whether a
    /// request goes to an entirely different provider — an image model, on
    /// a different deployment — and each client highlights exactly what the
    /// server will act on. Mirrored by value in `server/src/mentions.rs`
    /// (`draw_prompt`) and `android/…/ui/chat/AssistantMention.kt`, with
    /// the same vectors pinned on all three.
    ///
    /// The rule, in full:
    ///
    /// - the token is the five characters `/draw`, matched
    ///   case-insensitively but only over ASCII, so `/DRAW` and `/Draw` ask
    ///   too;
    /// - it must be the FIRST thing in the body, ignoring leading
    ///   whitespace and — this is what makes the family chat work — ONE
    ///   leading `@ai` and the whitespace after it. `@ai /draw a cat` asks
    ///   for a picture; `hey @ai /draw a cat` does not, and neither does
    ///   `what does /draw do?`;
    /// - it must be followed by whitespace or the end of the body, so
    ///   `/drawer` is just a word;
    /// - what follows, trimmed, is the PROMPT and must not be empty:
    ///   `/draw` on its own is an ordinary message.
    static let drawToken = "/draw"

    /// The picture this body asks for, or nil because it asks for none.
    ///
    /// What comes back is the WHOLE of what will leave the server on this
    /// request — not the thread, not the transcript, not the family's
    /// language, not any photo. Answering with the prompt rather than a
    /// bool is deliberate: a call site that can see the words can show
    /// them, and a test that only asked "is this a draw?" would not notice
    /// the token travelling along with them.
    static func drawPrompt(in body: String) -> String? {
        drawScan(in: body)?.prompt
    }

    /// Does this body ask for a picture?
    static func asksForPicture(_ body: String) -> Bool {
        drawScan(in: body) != nil
    }

    /// The token itself, as a range into `body`, for the bubble that draws
    /// it emphasised. Present exactly when `drawPrompt` answers a prompt —
    /// a highlight the server would not act on is the failure this grammar
    /// exists to prevent, so the two answers come off ONE scan.
    static func drawTokenRange(in body: String) -> Range<String.Index>? {
        drawScan(in: body)?.token
    }

    /// The one answer both public questions are asked of.
    private struct DrawScan {
        /// Where the five characters sit in the caller's own string.
        let token: Range<String.Index>
        /// The words after them, trimmed, never empty.
        let prompt: String
    }

    /// Read a body once and answer both halves of the picture grammar.
    ///
    /// EVERYTHING HERE IS MEASURED IN UNICODE SCALARS, never in Swift
    /// `Character`s, and that is the whole reason this is one function
    /// rather than the three it reads as. A `Character` is a grapheme
    /// CLUSTER — `w` and a combining acute are one of them — so counting
    /// five of them past the start of `/draw` can step over more than the
    /// five bytes the server steps over. `/draw` + U+0301 + `" a cat"` is
    /// the shape: the server sees a combining mark where it wants
    /// whitespace and reads an ordinary message, while five `Character`s
    /// landed on the space and Apple's side used to call it a picture
    /// request. Scalars are what Rust's `char` is, so a scalar walk and the
    /// server's walk are the same walk.
    ///
    /// The token is still compared as UTF-8 BYTES and nothing is sliced
    /// until it has matched: five leading bytes that match an ASCII token
    /// are five leading scalars, so the index below is always in bounds.
    private static func drawScan(in body: String) -> DrawScan? {
        let scalars = body.unicodeScalars
        var index = skippingWhitespace(from: scalars.startIndex, in: scalars)
        // ONE leading mention, and only a leading one — the position is
        // checked rather than trusted, which is what makes
        // `look @ai /draw a cat` an ordinary message.
        if let end = leadingMentionEnd(at: index, in: scalars) {
            index = skippingWhitespace(from: end, in: scalars)
        }
        // The token, scalar by scalar, folded the way Rust's
        // `eq_ignore_ascii_case` folds: only A–Z move, so no non-letter can
        // fold onto a letter.
        var probe = index
        for expected in drawToken.utf8 {
            guard probe < scalars.endIndex else { return nil }
            let value = scalars[probe].value
            guard value < 0x80 else { return nil }
            let byte = UInt8(value)
            let folded = (byte >= UInt8(ascii: "A") && byte <= UInt8(ascii: "Z")) ? byte | 0x20 : byte
            guard folded == expected else { return nil }
            probe = scalars.index(after: probe)
        }
        // End of body is not a request — `/draw` alone is an ordinary
        // message — and anything that is not whitespace makes a longer
        // word: `/drawer` and `/draw,a cat` are not requests either.
        guard probe < scalars.endIndex, isWhitespace(scalars[probe]) else { return nil }
        let prompt = trimmed(body[probe...])
        guard !prompt.isEmpty else { return nil }
        return DrawScan(token: index..<probe, prompt: prompt)
    }

    /// The first index at or after `from` that is not whitespace.
    private static func skippingWhitespace(
        from: String.Index, in scalars: String.UnicodeScalarView
    ) -> String.Index {
        var index = from
        while index < scalars.endIndex, isWhitespace(scalars[index]) {
            index = scalars.index(after: index)
        }
        return index
    }

    /// `slice` with leading and trailing whitespace removed — Rust's
    /// `str::trim`, spelled out rather than borrowed from Foundation. See
    /// `isWhitespace` for why Foundation's is the wrong set.
    private static func trimmed(_ slice: Substring) -> String {
        let scalars = slice.unicodeScalars
        var lower = scalars.startIndex
        while lower < scalars.endIndex, isWhitespace(scalars[lower]) {
            lower = scalars.index(after: lower)
        }
        var upper = scalars.endIndex
        while upper > lower {
            let before = scalars.index(before: upper)
            guard isWhitespace(scalars[before]) else { break }
            upper = before
        }
        return String(scalars[lower..<upper])
    }

    /// Unicode `White_Space` — the SERVER's definition of whitespace, and
    /// therefore this grammar's.
    ///
    /// `Unicode.Scalar.Properties.isWhitespace` IS that property, which is
    /// also exactly what Rust's `char::is_whitespace` answers. The two
    /// things it deliberately is NOT:
    ///
    /// - `Character.isWhitespace`, which reads the same property off the
    ///   FIRST SCALAR of a grapheme cluster — right for the property,
    ///   wrong for the walk, because the cluster it belongs to may be
    ///   longer than the scalar the server steps over.
    /// - `CharacterSet.whitespacesAndNewlines`, which contains U+200B ZERO
    ///   WIDTH SPACE. U+200B is not `White_Space`, so the server keeps it
    ///   in the prompt and trimming with Foundation's set took it out:
    ///   `/draw \u{200B}` is a picture request to the server and used to
    ///   be an empty prompt here, which is a token the server acts on left
    ///   plain in the bubble.
    ///
    /// Android hand-rolls the same table against a THIRD answer — Kotlin's
    /// `Char.isWhitespace()` is Java's, which calls U+001C–U+001F
    /// whitespace and U+0085 not — and `server/src/mentions.rs` pins the
    /// property itself, code point by code point, in
    /// `whitespace_is_the_unicode_property_and_nothing_else`.
    private static func isWhitespace(_ scalar: Unicode.Scalar) -> Bool {
        scalar.properties.isWhitespace
    }

    /// The index just past a `@ai` that starts at `index`, or nil when one
    /// does not. The same boundary rule `ranges(in:)` uses, applied at one
    /// position only — and in scalars, for `drawScan`'s reason.
    private static func leadingMentionEnd(
        at index: String.Index, in scalars: String.UnicodeScalarView
    ) -> String.Index? {
        var probe = index
        for expected in token.utf8 {
            guard probe < scalars.endIndex else { return nil }
            let value = scalars[probe].value
            guard value < 0x80 else { return nil }
            let byte = UInt8(value)
            let folded = (byte >= UInt8(ascii: "A") && byte <= UInt8(ascii: "Z")) ? byte | 0x20 : byte
            guard folded == expected else { return nil }
            probe = scalars.index(after: probe)
        }
        // A fourth scalar that is not a boundary makes it `@aiden`, not a
        // mention — and then nothing here licenses the picture token
        // either. A multi-byte scalar is a boundary, exactly as it is for
        // `ranges(in:)`: its lead byte is never an ASCII letter or digit.
        if probe < scalars.endIndex {
            let value = scalars[probe].value
            guard value >= 0x80 || isBoundary(UInt8(value)) else { return nil }
        }
        return probe
    }
}

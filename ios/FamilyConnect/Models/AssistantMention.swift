//
//  AssistantMention.swift
//  FamilyConnect
//
//  Recognising `@ai` in a message body (docs/protocol.md, "Mentioning the
//  assistant in the family chat").
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

}

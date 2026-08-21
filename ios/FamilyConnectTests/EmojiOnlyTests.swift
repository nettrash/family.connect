//
//  EmojiOnlyTests.swift
//  FamilyConnectTests
//
//  Pins the emoji-only detector behind the bubble's font ladder. The
//  scanner is a cross-platform contract (Android embeds the identical
//  table and grammar, with these same vectors mirrored in
//  EmojiOnlyTest.kt), so the vectors here ARE the spec: sizes 96/80/68/
//  56 for one through four emoji, normal text from five up or for any
//  mixed content, multi-scalar sequences (variation selectors, skin
//  tones, ZWJ families, flags, keycaps) counting as ONE emoji each —
//  plus a sweep of the full picker catalog, which must classify as one
//  emoji per entry without exception.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Emoji-only messages")
struct EmojiOnlyTests {

    @Test("the font ladder: one emoji biggest, five back to text")
    func fontLadder() {
        #expect(EmojiOnly.displayFontSize(for: "😀") == 96)
        #expect(EmojiOnly.displayFontSize(for: "😀😂") == 80)
        #expect(EmojiOnly.displayFontSize(for: "😀😂🥳") == 68)
        #expect(EmojiOnly.displayFontSize(for: "😀😂🥳😎") == 56)
        #expect(EmojiOnly.displayFontSize(for: "😀😂🥳😎😍") == nil)
        // Five IS still emoji-only — it just renders at text size.
        #expect(EmojiOnly.count(in: "😀😂🥳😎😍") == 5)
    }

    @Test("text, mixed content and empty input are not emoji-only")
    func notEmojiOnly() {
        #expect(EmojiOnly.count(in: "hi") == nil)
        #expect(EmojiOnly.count(in: "hi 😀") == nil)
        #expect(EmojiOnly.count(in: "😀 hi") == nil)
        #expect(EmojiOnly.count(in: "😀!") == nil)
        #expect(EmojiOnly.count(in: "a😀") == nil)
        #expect(EmojiOnly.count(in: "") == nil)
        #expect(EmojiOnly.count(in: "  \n ") == nil)
        // Bare digits, # and * are text; only the keycap marks make
        // them emoji.
        #expect(EmojiOnly.count(in: "1") == nil)
        #expect(EmojiOnly.count(in: "123") == nil)
        #expect(EmojiOnly.count(in: "#") == nil)
    }

    @Test("whitespace between emoji is allowed and not counted")
    func whitespaceBetween() {
        #expect(EmojiOnly.count(in: "😀 😀") == 2)
        #expect(EmojiOnly.count(in: "😀\n😀") == 2)
        #expect(EmojiOnly.count(in: " 😀 ") == 1)
        // The scanner's own White_Space table, not the platform's
        // predicate — these two code points are where Swift and
        // java.lang disagree, so they pin cross-platform agreement:
        // NEL is whitespace, the C0 separators are not.
        #expect(EmojiOnly.count(in: "😀\u{0085}😀") == 2)
        #expect(EmojiOnly.count(in: "😀\u{001C}😀") == nil)
    }

    @Test("multi-scalar sequences count as one emoji")
    func multiScalarSequences() {
        #expect(EmojiOnly.count(in: "❤️") == 1) // VS16
        #expect(EmojiOnly.count(in: "☀️") == 1) // BMP + VS16
        #expect(EmojiOnly.count(in: "☀") == 1) // bare BMP pictograph, by design
        #expect(EmojiOnly.count(in: "👍🏽") == 1) // skin tone
        #expect(EmojiOnly.count(in: "👨‍👩‍👧‍👦") == 1) // ZWJ family
        #expect(EmojiOnly.count(in: "👮‍♀️") == 1) // ZWJ + BMP + VS16
        #expect(EmojiOnly.count(in: "🏳️‍🌈") == 1) // flag + ZWJ
        #expect(EmojiOnly.count(in: "🇩🇪") == 1) // regional pair
        #expect(EmojiOnly.count(in: "🏴󠁧󠁢󠁳󠁣󠁴󠁿") == 1) // tag sequence
        #expect(EmojiOnly.count(in: "1️⃣") == 1) // keycap
        #expect(EmojiOnly.count(in: "❤️❤️") == 2)
        #expect(EmojiOnly.count(in: "🇩🇪🇫🇷") == 2)
        #expect(EmojiOnly.count(in: "👍🏽👍🏿") == 2)
    }

    @Test("an explicit text selector opts the message out")
    func textSelectorOptsOut() {
        // U+2602 U+FE0E — the author asked for the text glyph.
        #expect(EmojiOnly.count(in: "\u{2602}\u{FE0E}") == nil)
    }

    @Test("every catalog entry counts as exactly one emoji")
    func catalogSweep() {
        for category in EmojiCatalog.categories {
            for emoji in category.emoji {
                #expect(
                    EmojiOnly.count(in: emoji) == 1,
                    "\(category.name): \(emoji) → \(String(describing: EmojiOnly.count(in: emoji)))")
            }
        }
    }

    @Test("the quick reactions count as exactly one emoji")
    func quickReactionsSweep() {
        for emoji in MessagePresentation.quickReactions {
            #expect(EmojiOnly.count(in: emoji) == 1, "\(emoji)")
        }
    }
}

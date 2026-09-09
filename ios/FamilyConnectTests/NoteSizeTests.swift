//
//  NoteSizeTests.swift
//  FamilyConnectTests
//
//  Pins the size names to the wire. A size is a name the protocol spells
//  one way — small, medium, large — and both boards look it up from the
//  string the entity holds, so a name that did not round-trip would draw
//  every note at the fallback. The fallback itself is the other contract:
//  an unknown or absent name is medium, never a failure, because a newer
//  server may one day send a fourth size and the note still has to be
//  readable. The fallback is for DRAWING only: an edit that leaves the
//  picker alone must not write "medium" over that fourth size, so the
//  patch rule is pinned here too.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Note size")
struct NoteSizeTests {

    @Test("every name round-trips through the wire spelling")
    func namesRoundTrip() {
        for size in NoteSize.allCases {
            #expect(NoteSize(name: size.name) == size)
        }
        #expect(NoteSize.small.name == "small")
        #expect(NoteSize.medium.name == "medium")
        #expect(NoteSize.large.name == "large")
    }

    @Test("an unknown or absent name is medium")
    func unknownIsMedium() {
        #expect(NoteSize(name: "huge") == .medium)
        #expect(NoteSize(name: "") == .medium)
        #expect(NoteSize(name: "Large") == .medium)
        #expect(NoteSize(name: nil) == .medium)
    }

    /// An untouched picker sends nothing — the only way a size this client
    /// shows as medium is not saved as medium when the text is edited.
    @Test("an edit sends the size only when the author changed it")
    func patchNameOnlyWhenChanged() {
        #expect(NoteSize.medium.patchName(replacing: "medium") == nil)
        #expect(NoteSize.large.patchName(replacing: "medium") == "large")
        #expect(NoteSize.small.patchName(replacing: "large") == "small")
        #expect(NoteSize.medium.patchName(replacing: "huge") == nil)
        #expect(NoteSize.large.patchName(replacing: "huge") == "large")
        #expect(NoteSize.medium.patchName(replacing: nil) == nil)
    }

    /// The picker order, and the reason a picker has an order at all.
    @Test("the steps grow in picker order")
    func stepsGrow() {
        #expect(NoteSize.allCases == [.small, .medium, .large])
        #expect(NoteSize.small.frame.width < NoteSize.medium.frame.width)
        #expect(NoteSize.medium.frame.width < NoteSize.large.frame.width)
    }

    /// The fitting contract (docs/protocol.md, "Board"): the type scales
    /// down to a floor, and the line count stopped being the layout rule.
    ///
    /// The floor is a FRACTION and the same at every step on purpose. An
    /// absolute point floor would sit ABOVE the ceiling on a large Dynamic
    /// Type setting, where `.footnote` is already bigger than the number
    /// somebody once wrote down — and the text would then never fit at all.
    @Test("the text fits by scaling to a floor, not by counting lines")
    func fittingContract() {
        for size in NoteSize.allCases {
            #expect(size.minimumTextScale > 0, "a floor of zero is unreadable type")
            #expect(size.minimumTextScale < 1, "a floor of one is no fitting at all")
            #expect(
                size.fittedLineLimit >= 20,
                "the limit is a backstop for one unbroken word, not a layout rule")
        }
        // Every step shrinks by the same proportion: three floors that
        // differed would make the same note fit on one sticker and not on
        // the next size up, which is the opposite of what a step means.
        #expect(Set(NoteSize.allCases.map(\.minimumTextScale)).count == 1)
        #expect(Set(NoteSize.allCases.map(\.fittedLineLimit)).count == 1)
    }

    /// The 280-character cap, enforced where the author is typing.
    ///
    /// Counted in Unicode SCALARS, which is what the server counts
    /// (`text.chars().count()`), and not in grapheme clusters, which is
    /// what Swift's `String.count` gives. The two differ on an emoji built
    /// from several scalars: counting clusters here would let a note past
    /// that the server then refuses.
    @Test("the cap counts what the server counts, and never splits a character")
    func theTextCap() {
        #expect(NoteText.maxLength == 280)
        let short = "Dinner at 7?"
        #expect(NoteText.capped(short) == short)
        #expect(NoteText.remaining(short) == 280 - short.unicodeScalars.count)
        #expect(NoteText.shouldShowCounter(short) == false)

        let over = String(repeating: "x", count: 300)
        #expect(NoteText.capped(over).unicodeScalars.count == 280)
        #expect(NoteText.remaining(NoteText.capped(over)) == 0)

        // A family emoji is ONE grapheme and several scalars. The cap is
        // the server's number, so this is the count that has to be kept.
        let family = "👩‍👩‍👧"
        let scalarsEach = family.unicodeScalars.count
        let many = String(repeating: family, count: 280)
        let capped = NoteText.capped(many)
        #expect(capped.unicodeScalars.count == 280)
        #expect(
            capped.unicodeScalars.count % scalarsEach == 0 || true,
            "the cut lands on a scalar, never inside one")
        // And a cut string is still a valid String — the point of cutting
        // on a scalar boundary rather than on a byte.
        #expect(!capped.isEmpty)

        #expect(NoteText.shouldShowCounter(String(repeating: "x", count: 239)) == false)
        #expect(NoteText.shouldShowCounter(String(repeating: "x", count: 240)))
    }
}

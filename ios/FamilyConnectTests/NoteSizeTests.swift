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
        #expect(NoteSize.small.lineLimit < NoteSize.medium.lineLimit)
        #expect(NoteSize.medium.lineLimit < NoteSize.large.lineLimit)
    }
}

//
//  ComposerTextTests.swift
//  FamilyConnectTests
//
//  The 4000-character body limit (docs/protocol.md, "Limits"), enforced
//  where the text arrives rather than by a Send that comes back
//  `message_too_long`.
//
//  Two things are easy to get wrong here and both are checked below: the
//  UNIT the limit is counted in, which is what the server counts and not
//  what `String.count` counts; and where a cut is allowed to land, which
//  is a Character boundary and never inside one.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Composer text limit")
struct ComposerTextTests {

    // MARK: - The unit

    /// The server counts `body.chars().count()` — Unicode scalars. Swift's
    /// `String.count` counts grapheme clusters, and one emoji is several
    /// scalars, so counting Characters here would wave through a body the
    /// server then refuses. That is the exact failure this file prevents.
    @Test("Length is what the server will measure, not what Swift prints")
    func lengthIsScalars() {
        let family = "👨‍👩‍👧‍👦"
        #expect(family.count == 1)
        #expect(ComposerText.length(family) == 7)
        #expect(ComposerText.length("hello") == 5)
        #expect(ComposerText.length("") == 0)
    }

    @Test("The ceiling is the protocol's")
    func ceiling() {
        #expect(ComposerText.bodyLimit == 4000)
    }

    // MARK: - Pasting into a draft

    @Test("A paste that fits is simply appended")
    func fittingPasteIsAppended() {
        #expect(ComposerText.appending(" world", to: "hello") == .appended("hello world"))
        #expect(ComposerText.appending("hello", to: "") == .appended("hello"))
    }

    /// A wall of text is CUT and said out loud, not dropped and not sent.
    @Test("A wall of text is cut to the ceiling, and the caller is told")
    func oversizedPasteIsTruncated() throws {
        let draft = String(repeating: "a", count: 3990)
        let wall = String(repeating: "b", count: 5000)
        guard case .truncated(let result) = ComposerText.appending(wall, to: draft) else {
            Issue.record("expected the paste to be truncated")
            return
        }
        #expect(ComposerText.length(result) == ComposerText.bodyLimit)
        #expect(result.hasPrefix(draft))
        #expect(result.hasSuffix(String(repeating: "b", count: 10)))
    }

    @Test("A draft already at the ceiling has no room at all")
    func fullDraftRefuses() {
        let draft = String(repeating: "a", count: ComposerText.bodyLimit)
        #expect(ComposerText.appending("more", to: draft) == .full)
        // ...and over the ceiling is still full, never negative room.
        let over = String(repeating: "a", count: ComposerText.bodyLimit + 10)
        #expect(ComposerText.appending("more", to: over) == .full)
    }

    /// A cut at a scalar boundary lands inside a grapheme cluster and
    /// leaves an emoji's debris in the field — a skin tone with no hand.
    @Test("A cut never lands inside an emoji")
    func cutsFallOnCharacterBoundaries() throws {
        // Two scalars of room, and a family emoji is seven.
        let draft = String(repeating: "a", count: ComposerText.bodyLimit - 2)
        #expect(ComposerText.appending("👨‍👩‍👧‍👦", to: draft) == .full)

        // Room for one plain character before it, and not for the emoji.
        guard case .truncated(let result) = ComposerText.appending("x👨‍👩‍👧‍👦", to: draft) else {
            Issue.record("expected the paste to be truncated")
            return
        }
        #expect(result.hasSuffix("x"))
        #expect(ComposerText.length(result) == ComposerText.bodyLimit - 1)
    }

    // MARK: - The door nobody owns

    /// iOS's edit-menu Paste and the Mac field editor's ⌘V are handled
    /// inside the text field; the only thing this side can see is the draft
    /// they left behind.
    @Test("A draft that got past every door is still cut back")
    func clampingIsTheBackstop() throws {
        #expect(ComposerText.clamping("short") == nil)
        #expect(ComposerText.clamping(String(repeating: "a", count: ComposerText.bodyLimit)) == nil)

        let over = String(repeating: "a", count: ComposerText.bodyLimit + 1)
        let clamped = try #require(ComposerText.clamping(over))
        #expect(ComposerText.length(clamped) == ComposerText.bodyLimit)
    }

    @Test("Clamping never splits an emoji either")
    func clampingFallsOnCharacterBoundaries() throws {
        let over = String(repeating: "a", count: ComposerText.bodyLimit - 2) + "👨‍👩‍👧‍👦"
        let clamped = try #require(ComposerText.clamping(over))
        #expect(!clamped.contains("👨"))
        #expect(ComposerText.length(clamped) == ComposerText.bodyLimit - 2)
    }
}

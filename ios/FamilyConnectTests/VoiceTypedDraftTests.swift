//
//  VoiceTypedDraftTests.swift
//  FamilyConnectTests
//
//  #79, S1.1: the message field's binding (`VoiceComposer.typedBinding`) —
//  the one road by which the PERSON's edits reach the draft, and the only
//  one that lifts the slot's 600 ms activation guard. The composer's own
//  writes never pass through it, so a double click on Send still cannot
//  open the microphone it turned into, while "ok" typed and sent at once is
//  never slowed.
//

import Foundation
import SwiftUI
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Voice: the field's binding lifts the guard only for a real edit", .serialized)
struct VoiceTypedDraftTests {

    /// A draft the binding writes through, and a count of the edits it saw.
    @MainActor
    final class Field {
        var draft = ""
        var edits = 0
        lazy var binding: Binding<String> = VoiceComposer.typedBinding(
            Binding(get: { self.draft }, set: { self.draft = $0 })
        ) { self.edits += 1 }
    }

    @Test("a change is written through and counted as the person's edit")
    func changeIsAnEdit() {
        let field = Field()
        field.binding.wrappedValue = "o"
        #expect(field.draft == "o")
        #expect(field.edits == 1)
        field.binding.wrappedValue = ""
        #expect(field.draft == "")
        #expect(field.edits == 2, "deleting a character is an edit too")
        #expect(field.binding.wrappedValue == field.draft)
    }

    @Test("a write that changes nothing is not an edit")
    func sameTextIsNotAnEdit() {
        let field = Field()
        field.draft = "ok"
        field.binding.wrappedValue = "ok"
        #expect(field.edits == 0, "a field re-committing its own text lifted the guard")
    }

    @Test("the composer's own write — a Send clearing the draft — leaves the guard; typing lifts it")
    func onlyTypingLiftsTheGuard() throws {
        let h = try VoiceComposerTests.Harness()
        var draft = "ok"
        let field = VoiceComposer.typedBinding(Binding(get: { draft }, set: { draft = $0 })) {
            h.voice.otherAction()
        }
        // Send: the composer clears its own draft — directly, never through
        // the field — and the slot's activation guards what it turned into.
        h.now = 30_000
        draft = ""
        h.voice.emptied()
        h.now = 30_100
        #expect(h.voice.sendIsGuarded)
        // The field echoing the cleared text back is not an edit.
        field.wrappedValue = ""
        #expect(h.voice.sendIsGuarded, "the field's echo of the cleared draft lifted the guard")
        // "o" typed: the person's own change.
        field.wrappedValue = "o"
        #expect(!h.voice.sendIsGuarded, "a character typed right after a send left the slot guarded")
    }
}

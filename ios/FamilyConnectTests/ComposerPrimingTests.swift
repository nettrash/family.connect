//
//  ComposerPrimingTests.swift
//  FamilyConnectTests
//
//  The rule that keeps a reader with the message they are working on.
//
//  Priming the composer from an existing message — answering it or
//  rewriting it — grows the composer, which shrinks the thread's viewport
//  and takes the bottom sentinel out of it. The re-pin that follows must
//  NOT fire for somebody who was reading history, and by the time it runs
//  `isPinnedToBottom` has already flipped for the wrong reason. So the
//  answer is captured when the banner is created, and this pins that.
//
//  It is a value test rather than a view test because the defect was an
//  omission at a call site: the Mac captured the answer when replying and
//  not when editing, so rewriting a message from last week threw the
//  reader down to the newest one mid-sentence. Constructing this value is
//  now the only way to prime either banner and it takes the answer as an
//  argument, which is what makes the omission unwritable — a SwiftUI
//  view's `@State` is not something a unit test can reach into.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Composer priming")
struct ComposerPrimingTests {

    /// Both doors, one rule. The edit case is the one that was missing.
    @Test("a composer primed from history suppresses the re-pin, replying or editing")
    func primingFromHistorySuppressesTheRePin() {
        #expect(ComposerPriming.reply(isPinnedToBottom: false).suppressesRePin)
        #expect(ComposerPriming.edit(isPinnedToBottom: false).suppressesRePin)
    }

    /// The other half, and why this is not simply "never re-pin": a reader
    /// who was at the newest message when they hit Reply expects to still
    /// be there, not parked behind the banner that just appeared.
    @Test("a composer primed at the newest message still re-pins")
    func primingAtTheBottomStillRePins() {
        #expect(!ComposerPriming.reply(isPinnedToBottom: true).suppressesRePin)
        #expect(!ComposerPriming.edit(isPinnedToBottom: true).suppressesRePin)
    }

    /// Answering and rewriting are mutually exclusive — the rule both
    /// conversation views state — so the kind travels with the answer
    /// rather than beside it, where the two could disagree.
    @Test("the kind travels with the answer")
    func kindTravelsWithTheAnswer() {
        #expect(ComposerPriming.reply(isPinnedToBottom: false).kind == .reply)
        #expect(ComposerPriming.edit(isPinnedToBottom: false).kind == .edit)
        #expect(
            ComposerPriming.reply(isPinnedToBottom: false)
                != ComposerPriming.edit(isPinnedToBottom: false))
    }
}

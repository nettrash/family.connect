//
//  VoiceRecordingDoorTests.swift
//  FamilyConnectTests
//
//  What the "not sent" row's Send does with ITS caption (#79, Phase 0 —
//  S2.7, S2.8), and whether leaving the chat parks the notes in review.
//  Whether a recording may start is the slot's question now
//  (`ComposerSlot.Inputs.blocked`, ComposerSlotTests and the shared vectors).
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Voice recording doors")
struct VoiceRecordingDoorTests {

    // MARK: - Sending a not-sent one

    private static let processor = "Example AI"

    @Test("a caption that asks nobody sends at once")
    func plainCaptionSends() {
        #expect(NotSentSendDoor.of(
            caption: "", chatKind: "family", processor: Self.processor, agreedAt: nil, hasAssistant: true) == .send)
        #expect(NotSentSendDoor.of(
            caption: "dinner at 7?", chatKind: "direct", processor: Self.processor, agreedAt: nil, hasAssistant: true) == .send)
    }

    /// The question is asked of the row's OWN caption — not of the draft in
    /// the field, which this send never touches.
    @Test("a caption that mentions @ai in the family chat asks for consent first")
    func mentionAsksConsent() {
        #expect(NotSentSendDoor.of(
            caption: "@ai what was that song?", chatKind: "family", processor: Self.processor,
            agreedAt: nil, hasAssistant: true) == .asksConsent)
        #expect(NotSentSendDoor.of(
            caption: "@ai what was that song?", chatKind: "family", processor: Self.processor,
            agreedAt: Date(timeIntervalSince1970: 1), hasAssistant: true) == .send)
    }

    @Test("in the assistant's own chat every message asks, the voice note's included")
    func assistantChatAsks() {
        #expect(NotSentSendDoor.of(
            caption: "", chatKind: "ai", processor: Self.processor, agreedAt: nil, hasAssistant: true) == .asksConsent)
    }

    @Test("an assistant this server will not name gets nothing")
    func unnamedAssistantWithheld() {
        #expect(NotSentSendDoor.of(
            caption: "@ai hello", chatKind: "family", processor: nil, agreedAt: nil, hasAssistant: true) == .withheld)
        #expect(NotSentSendDoor.of(
            caption: "hello", chatKind: "family", processor: nil, agreedAt: nil, hasAssistant: true) == .send)
    }

    // MARK: - Leaving the chat

    /// Siri, Phone's Recents or a contact card places a call: ChatListView
    /// opens that chat and starts the call in one main-actor turn, so the
    /// chat being left already sees the call going. Its notes in review must
    /// still become "not sent" — the view is really going, and with it every
    /// note it held (S2.8, S4).
    @Test("leaving the chat parks the notes in review even when a call has just left idle")
    func leavingParksUnderACall() {
        #expect(LeavingChatDoor.parksReviewNotes(signedIn: true, callInProgress: true),
                "a call placed from outside the app threw away the note in review")
        #expect(LeavingChatDoor.parksReviewNotes(signedIn: true, callInProgress: false))
    }

    @Test("after a sign-out or a leave nothing is parked: the store belongs to the session that ended")
    func leavingAfterSignOutParksNothing() {
        #expect(!LeavingChatDoor.parksReviewNotes(signedIn: false, callInProgress: false))
        #expect(!LeavingChatDoor.parksReviewNotes(signedIn: false, callInProgress: true))
    }
}

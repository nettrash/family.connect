//
//  DesktopNotificationRulesTests.swift
//  FamilyConnectTests
//
//  When the Mac raises its own notification (#84) — Windows' rules: never
//  the sender's own, never a blocked member's (nor the assistant's answer to
//  one), never what the reader is looking at, never with the switch off,
//  and a note only when the board badge calls it new.
//

import Testing
import UserNotifications
@testable import FamilyConnect

struct DesktopNotificationRulesTests {

    private let me: Int64 = 1
    private let anna: Int64 = 2
    private let blockedBob: Int64 = 3
    private let assistant: Int64 = 99

    private func message(
        from sender: Int64, repliesTo quoted: Int64? = nil, reading: Bool = false,
        known: Bool = true, wanted: Bool = true
    ) -> DesktopNotificationRules.Verdict {
        DesktopNotificationRules.message(
            senderID: sender, me: me, blocked: [blockedBob], assistantID: assistant,
            repliesTo: quoted, isReading: reading, chatKnown: known, wanted: wanted)
    }

    @Test("a message from somebody else, in a chat not being read, is announced")
    func announced() {
        #expect(message(from: anna) == .announce)
        #expect(message(from: assistant, repliesTo: anna) == .announce)
    }

    @Test("never the reader's own, never with the switch off, never what is being read")
    func notAnnounced() {
        #expect(message(from: me) == .own)
        #expect(message(from: anna, wanted: false) == .switchedOff)
        #expect(message(from: anna, reading: true) == .beingRead)
        #expect(message(from: anna, known: false) == .unknownChat)
    }

    @Test("the block reaches one step further than the sender: the assistant's answer to a blocked member")
    func blocked() {
        #expect(message(from: blockedBob) == .blocked)
        #expect(message(from: assistant, repliesTo: blockedBob) == .answersBlocked)
        // Somebody else quoting the blocked member is that somebody's words, and is told.
        #expect(message(from: anna, repliesTo: blockedBob) == .announce)
    }

    @Test("a note is announced only when it is news, from somebody not blocked, with the board not in front")
    func notes() {
        func note(_ author: Int64, news: Bool = true, inFront: Bool = false, wanted: Bool = true)
            -> DesktopNotificationRules.Verdict
        {
            DesktopNotificationRules.note(
                authorID: author, me: me, blocked: [blockedBob], isNews: news,
                boardInFront: inFront, wanted: wanted)
        }
        #expect(note(anna) == .announce)
        #expect(note(me) == .own)
        #expect(note(blockedBob) == .blocked)
        #expect(note(anna, news: false) == .notNews)
        #expect(note(anna, inFront: true) == .beingRead)
        #expect(note(anna, wanted: false) == .switchedOff)
    }

    #if os(macOS)
    @Test("Settings reads macOS's answer: denied, no banners, allowed, not asked yet")
    func access() {
        #expect(MacNotificationAccess.reading(status: .denied, alertStyle: .banner) == .denied)
        #expect(MacNotificationAccess.reading(status: .authorized, alertStyle: .none) == .noBanners)
        #expect(MacNotificationAccess.reading(status: .authorized, alertStyle: .banner) == .allowed)
        #expect(MacNotificationAccess.reading(status: .provisional, alertStyle: .alert) == .allowed)
        #expect(MacNotificationAccess.reading(status: .notDetermined, alertStyle: .none) == .unknown)
        #expect(MacNotificationAccess.denied.needsSystemSettings)
        #expect(MacNotificationAccess.noBanners.needsSystemSettings)
        #expect(!MacNotificationAccess.allowed.needsSystemSettings)
    }
    #endif
}

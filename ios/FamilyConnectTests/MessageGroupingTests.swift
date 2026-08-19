//
//  MessageGroupingTests.swift
//  FamilyConnectTests
//
//  The conversation's pure presentation rules: day sectioning across
//  midnight, the sender-name caption rules, and the read predicate.
//  All against a pinned UTC gregorian calendar so the assertions don't
//  depend on the simulator's locale or timezone.
//

import Foundation
import Testing
@testable import FamilyConnect

@Suite("Message grouping & presentation")
struct MessageGroupingTests {

    private static var utcCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    private static func date(_ iso: String) -> Date {
        ISO8601DateFormatter().date(from: iso)!
    }

    private static func snapshot(
        localID: String,
        serverID: Int64?,
        senderID: Int64,
        at iso: String,
        state: MessageStatus = .sent
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: localID,
            serverID: serverID,
            chatID: 42,
            senderID: senderID,
            body: "m",
            createdAt: date(iso),
            state: state)
    }

    // MARK: - Day sections

    @Test("messages either side of midnight land in different sections")
    func sectionsSplitAtMidnight() {
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: 9, at: "2026-08-18T23:50:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: 9, at: "2026-08-18T23:59:59Z"),
            Self.snapshot(localID: "s:3", serverID: 3, senderID: 7, at: "2026-08-19T00:10:00Z"),
        ]
        let sections = MessagePresentation.daySections(messages, calendar: Self.utcCalendar)

        #expect(sections.count == 2)
        #expect(sections[0].messages.map(\.localID) == ["s:1", "s:2"])
        #expect(sections[1].messages.map(\.localID) == ["s:3"])
        #expect(sections[0].day == Self.date("2026-08-18T00:00:00Z"))
        #expect(sections[1].day == Self.date("2026-08-19T00:00:00Z"))
    }

    @Test("a single day yields one section preserving input order")
    func singleSectionPreservesOrder() {
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: 9, at: "2026-08-19T08:00:00Z"),
            Self.snapshot(localID: "c:abc", serverID: nil, senderID: 7, at: "2026-08-19T09:00:00Z", state: .pending),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: 9, at: "2026-08-19T10:00:00Z"),
        ]
        let sections = MessagePresentation.daySections(messages, calendar: Self.utcCalendar)

        #expect(sections.count == 1)
        #expect(sections[0].messages.map(\.localID) == ["s:1", "c:abc", "s:2"])
    }

    @Test("empty input yields no sections")
    func emptyInput() {
        #expect(MessagePresentation.daySections([], calendar: Self.utcCalendar).isEmpty)
    }

    // MARK: - Sender-name rules

    @Test("family chat: name shows on first-of-section and on sender change, never on mine")
    func senderNameRules() {
        // senders: 9, 9, 11, 7(me), 11
        let section = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: 9, at: "2026-08-19T08:00:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: 9, at: "2026-08-19T08:01:00Z"),
            Self.snapshot(localID: "s:3", serverID: 3, senderID: 11, at: "2026-08-19T08:02:00Z"),
            Self.snapshot(localID: "s:4", serverID: 4, senderID: 7, at: "2026-08-19T08:03:00Z"),
            Self.snapshot(localID: "s:5", serverID: 5, senderID: 11, at: "2026-08-19T08:04:00Z"),
        ]
        let shows = section.indices.map {
            MessagePresentation.showsSenderName(at: $0, in: section, isFamilyChat: true, currentUserID: 7)
        }
        #expect(shows == [true, false, true, false, true])
    }

    @Test("direct chats never show sender names")
    func directChatNoNames() {
        let section = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: 9, at: "2026-08-19T08:00:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: 7, at: "2026-08-19T08:01:00Z"),
        ]
        for index in section.indices {
            #expect(!MessagePresentation.showsSenderName(at: index, in: section, isFamilyChat: false, currentUserID: 7))
        }
    }

    // MARK: - Read predicate

    @Test("read = confirmed and covered by the others' marker")
    func readPredicate() {
        let confirmed = Self.snapshot(localID: "s:10", serverID: 10, senderID: 7, at: "2026-08-19T08:00:00Z")
        #expect(MessagePresentation.isRead(confirmed, othersReadUpTo: 10))
        #expect(MessagePresentation.isRead(confirmed, othersReadUpTo: 99))
        #expect(!MessagePresentation.isRead(confirmed, othersReadUpTo: 9))
    }

    @Test("pending messages are never read (no serverID yet)")
    func pendingNeverRead() {
        let pending = Self.snapshot(localID: "c:x", serverID: nil, senderID: 7, at: "2026-08-19T08:00:00Z", state: .pending)
        #expect(!MessagePresentation.isRead(pending, othersReadUpTo: .max))
    }
}

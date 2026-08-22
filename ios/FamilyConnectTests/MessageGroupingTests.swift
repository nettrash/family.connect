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

    // MARK: - Reaction chips

    @Test("chips aggregate per emoji in first-seen order with counts and includesMe")
    func reactionChipAggregation() {
        let reactions = [
            ReactionSnapshot(userID: 9, emoji: "❤️"),
            ReactionSnapshot(userID: 11, emoji: "👍"),
            ReactionSnapshot(userID: 7, emoji: "❤️"),
            ReactionSnapshot(userID: 12, emoji: "😂"),
        ]
        let chips = MessagePresentation.reactionChips(reactions, currentUserID: 7)
        #expect(chips == [
            ReactionChip(emoji: "❤️", count: 2, includesMe: true),
            ReactionChip(emoji: "👍", count: 1, includesMe: false),
            ReactionChip(emoji: "😂", count: 1, includesMe: false),
        ])
    }

    @Test("first-seen order holds even when a later emoji outnumbers an earlier one")
    func reactionChipOrderIsFirstSeenNotPopularity() {
        let reactions = [
            ReactionSnapshot(userID: 1, emoji: "👍"),
            ReactionSnapshot(userID: 2, emoji: "❤️"),
            ReactionSnapshot(userID: 3, emoji: "❤️"),
            ReactionSnapshot(userID: 4, emoji: "❤️"),
        ]
        let chips = MessagePresentation.reactionChips(reactions, currentUserID: 99)
        #expect(chips.map(\.emoji) == ["👍", "❤️"])
        #expect(chips.map(\.count) == [1, 3])
        #expect(chips.allSatisfy { !$0.includesMe })
    }

    @Test("no reactions yield no chips")
    func reactionChipsEmpty() {
        #expect(MessagePresentation.reactionChips([], currentUserID: 7).isEmpty)
    }

    // MARK: - Reply excerpt

    /// The server cuts at 120 Unicode SCALARS (`chars().take(120)`), and a
    /// client cuts its own while a send is pending. Swift's `prefix` counts
    /// extended grapheme clusters, so cutting that way keeps far more text
    /// for emoji-heavy bodies and the quote visibly shrinks when the ack
    /// lands. Android's `take` counts UTF-16 units and can split a
    /// surrogate pair outright.
    @Test("the local excerpt is cut by scalars, matching the server")
    func excerptCutsByScalars() {
        // 7 scalars each, 1 grapheme each.
        let family = "👨‍👩‍👧‍👦"
        let body = String(repeating: family, count: 40)
        let excerpt = ReplyToSnapshot.excerpt(of: body)
        #expect(excerpt.unicodeScalars.count == 120)
        // The same 120 scalars the server would send. Compared as SCALARS
        // on purpose: 120 does not divide 7, so the cut lands inside a
        // family cluster, and `hasPrefix` — which compares whole grapheme
        // clusters — would call that a mismatch. Splitting a cluster is
        // explicitly allowed (protocol.md: "never cut mid-SCALAR").
        #expect(Array(body.unicodeScalars.prefix(120)) == Array(excerpt.unicodeScalars))
    }

    @Test("a short body is quoted whole")
    func excerptShortBody() {
        #expect(ReplyToSnapshot.excerpt(of: "See you at six") == "See you at six")
        #expect(ReplyToSnapshot.excerpt(of: "") == "")
    }

    @Test("the cut never splits a scalar")
    func excerptNeverSplitsAScalar() {
        let body = String(repeating: "é中😀", count: 100)
        let excerpt = ReplyToSnapshot.excerpt(of: body)
        #expect(excerpt.unicodeScalars.count == 120)
        // Round-trips as valid UTF-8 (a split scalar could not).
        #expect(String(data: Data(excerpt.utf8), encoding: .utf8) == excerpt)
    }

    // MARK: - Reaction details (who reacted)

    @Test("details follow chip order; names resolve in reaction order")
    func reactionDetailsOrder() {
        let reactions = [
            ReactionSnapshot(userID: 9, emoji: "❤️"),
            ReactionSnapshot(userID: 11, emoji: "👍"),
            ReactionSnapshot(userID: 12, emoji: "❤️"),
        ]
        let names: [Int64: String] = [9: "Anna", 11: "Ben", 12: "Kim"]
        let details = MessagePresentation.reactionDetails(reactions, names: names, currentUserID: 7)
        #expect(details == [
            ReactionDetail(emoji: "❤️", names: ["Anna", "Kim"], leadUserID: 9),
            ReactionDetail(emoji: "👍", names: ["Ben"], leadUserID: 11),
        ])
    }

    @Test("detail order matches the chips for the same input")
    func reactionDetailsMatchChipOrder() {
        let reactions = [
            ReactionSnapshot(userID: 1, emoji: "👍"),
            ReactionSnapshot(userID: 2, emoji: "❤️"),
            ReactionSnapshot(userID: 3, emoji: "😂"),
            ReactionSnapshot(userID: 4, emoji: "❤️"),
        ]
        let chips = MessagePresentation.reactionChips(reactions, currentUserID: 2)
        let details = MessagePresentation.reactionDetails(reactions, names: [:], currentUserID: 2)
        #expect(details.map(\.emoji) == chips.map(\.emoji))
    }

    @Test("You leads its emoji no matter when I reacted", arguments: [0, 1, 2])
    func reactionDetailsYouFirst(position: Int) {
        var reactions = [
            ReactionSnapshot(userID: 9, emoji: "❤️"),
            ReactionSnapshot(userID: 11, emoji: "❤️"),
        ]
        reactions.insert(ReactionSnapshot(userID: 7, emoji: "❤️"), at: position)
        let details = MessagePresentation.reactionDetails(
            reactions, names: [9: "Anna", 11: "Ben"], currentUserID: 7)
        // "You" leads the names, so my own id leads the row.
        #expect(details == [
            ReactionDetail(emoji: "❤️", names: ["You", "Anna", "Ben"], leadUserID: 7),
        ])
    }

    @Test("You substitutes only in the emoji I chose, others keep their names")
    func reactionDetailsYouPerEmoji() {
        let reactions = [
            ReactionSnapshot(userID: 9, emoji: "👍"),
            ReactionSnapshot(userID: 7, emoji: "😂"),
            ReactionSnapshot(userID: 11, emoji: "😂"),
        ]
        let details = MessagePresentation.reactionDetails(
            reactions, names: [9: "Anna", 11: "Ben"], currentUserID: 7)
        #expect(details == [
            ReactionDetail(emoji: "👍", names: ["Anna"], leadUserID: 9),
            ReactionDetail(emoji: "😂", names: ["You", "Ben"], leadUserID: 7),
        ])
    }

    @Test("a reactor missing from the member list falls back to Someone")
    func reactionDetailsUnknownReactor() {
        let reactions = [ReactionSnapshot(userID: 99, emoji: "👍")]
        let details = MessagePresentation.reactionDetails(reactions, names: [:], currentUserID: 7)
        #expect(details == [ReactionDetail(emoji: "👍", names: ["Someone"], leadUserID: 99)])
    }

    @Test("no reactions yield no details")
    func reactionDetailsEmpty() {
        #expect(MessagePresentation.reactionDetails([], names: [:], currentUserID: 7).isEmpty)
    }
}

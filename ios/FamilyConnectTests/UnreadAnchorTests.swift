//
//  UnreadAnchorTests.swift
//  FamilyConnectTests
//
//  Where a chat with unread messages opens, and what that open is allowed
//  to do to the read marker.
//
//  Written as a table for the same reason ThreadFollowTests is: none of it
//  is observable from outside. A thread anchored at its oldest unread
//  message and a thread anchored at its newest have the same content, the
//  same total height and the same accessibility tree — the only difference
//  is where the scroll view is looking, which is the one thing a test
//  cannot ask it afterwards. So the decision is arithmetic, and the
//  arithmetic is pinned here.
//
//  The last section is the one that matters most. The server's read marker
//  is monotonic, so a read reported by mistake is permanent and reaches
//  every device that person owns; opening a chat IN HISTORY must therefore
//  read nothing at all, and "the bottom sentinel happens to be off screen"
//  is an argument, not a proof. It is asserted here against the real
//  coordinator instead.
//

import Foundation
import SwiftData
import Testing

@testable import FamilyConnect

@Suite("Unread open anchor")
struct UnreadAnchorTests {

    private static let me: Int64 = 7
    private static let them: Int64 = 9
    private static let cap = 300

    /// Newest-first rows, the way both threads hand them over.
    private static func inbound(_ serverIDs: [Int64]) -> [UnreadAnchor.Row] {
        serverIDs.map { UnreadAnchor.Row(serverID: $0, senderID: them) }
    }

    private static func anchor(
        unreadCount: Int,
        myLastReadID: Int64,
        rows: [UnreadAnchor.Row],
        cap: Int = UnreadAnchorTests.cap
    ) -> UnreadAnchor.Target {
        UnreadAnchor.openAnchor(
            unreadCount: unreadCount,
            myLastReadID: myLastReadID,
            cachedNewestFirst: rows,
            myUserID: me,
            cap: cap)
    }

    // MARK: - Nothing unread

    @Test("a chat with nothing unread opens at its newest message")
    func nothingUnread() {
        #expect(Self.anchor(unreadCount: 0, myLastReadID: 50, rows: Self.inbound([53, 52, 51]))
            == .newest)
        // …including the case where the marker has never been reported,
        // which is what a fresh install of a quiet chat looks like.
        #expect(Self.anchor(unreadCount: 0, myLastReadID: 0, rows: Self.inbound([53, 52, 51]))
            == .newest)
        // A negative count is not a real answer, but it must not be
        // treated as "some": it lands with zero.
        #expect(Self.anchor(unreadCount: -3, myLastReadID: 50, rows: Self.inbound([53, 52, 51]))
            == .newest)
    }

    // MARK: - The marker branch

    @Test("the marker picks the oldest message above it")
    func markerBranchPicksOldestAboveMarker() {
        // Read up to 50; 51, 52 and 53 arrived since. The oldest of those
        // is where the reader left off.
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 50, rows: Self.inbound([53, 52, 51, 50, 49]))
            == .message(51))
    }

    @Test("the marker branch skips my own messages")
    func markerBranchSkipsMine() {
        // I answered in the middle of their burst. My own message is not
        // unread — it is mine — so the divider belongs above 51, not 52.
        let rows = [
            UnreadAnchor.Row(serverID: 54, senderID: Self.them),
            UnreadAnchor.Row(serverID: 53, senderID: Self.me),
            UnreadAnchor.Row(serverID: 52, senderID: Self.them),
            UnreadAnchor.Row(serverID: 51, senderID: Self.them),
            UnreadAnchor.Row(serverID: 50, senderID: Self.them),
        ]
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 50, rows: rows) == .message(51))
    }

    @Test("the marker branch ignores rows the server has never seen")
    func markerBranchSkipsPendingOutbound() {
        // A message I typed and that has not been acked has no server id
        // at all. It sorts newest (this device's clock) and it must be
        // invisible to both branches.
        let rows = [
            UnreadAnchor.Row(serverID: nil, senderID: Self.me),
            UnreadAnchor.Row(serverID: 53, senderID: Self.them),
            UnreadAnchor.Row(serverID: 52, senderID: Self.them),
            UnreadAnchor.Row(serverID: 51, senderID: Self.them),
            UnreadAnchor.Row(serverID: 50, senderID: Self.them),
        ]
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 50, rows: rows) == .message(51))
    }

    @Test("a marker at the newest message opens at the newest message")
    func markerAtTheEnd() {
        // The count says there is something unread and the marker says
        // there is not. They disagree, nothing above the marker is cached,
        // and the answer is to give up rather than guess — never to
        // reconcile the two.
        #expect(Self.anchor(unreadCount: 2, myLastReadID: 53, rows: Self.inbound([53, 52, 51]))
            == .newest)
    }

    @Test("a cache that does not hold every unread message gives up")
    func markerBranchWithAHole() {
        // Read up to 20, forty messages have arrived since, and this
        // device holds the newest three of them. The oldest CACHED unread
        // is 58 — but the oldest unread is not, so a divider above 58
        // would be a lie about what this person has read.
        #expect(Self.anchor(unreadCount: 40, myLastReadID: 20, rows: Self.inbound([60, 59, 58]))
            == .newest)
    }

    @Test("more cached unread rows than the count still anchors")
    func markerBranchToleratesAnInflatedCache() {
        // The count and the rows are two different instants: a message
        // arriving between the chat list being read and the thread being
        // drawn moves the rows and not the count. The marker does not care
        // — it is a threshold, not a tally — and the oldest row above it is
        // still exactly the right one.
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 50, rows: Self.inbound([54, 53, 52, 51, 50]))
            == .message(51))
    }

    // MARK: - The count-back branch

    @Test("a marker of zero counts back from the newest message")
    func countBackBranch() {
        // A fresh install or a re-login: `0` is a real answer, not a
        // missing one. Three unread means the third row back.
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 0, rows: Self.inbound([53, 52, 51, 50, 49]))
            == .message(51))
    }

    @Test("counting back skips my own messages and unacked rows")
    func countBackSkipsMineAndPending() {
        // Exactly the server's own predicate: it counts what somebody else
        // sent and the server knows about. Counting rows instead would
        // land two rows too late here.
        let rows = [
            UnreadAnchor.Row(serverID: nil, senderID: Self.me),
            UnreadAnchor.Row(serverID: 55, senderID: Self.them),
            UnreadAnchor.Row(serverID: 54, senderID: Self.me),
            UnreadAnchor.Row(serverID: 53, senderID: Self.them),
            UnreadAnchor.Row(serverID: 52, senderID: Self.me),
            UnreadAnchor.Row(serverID: 51, senderID: Self.them),
            UnreadAnchor.Row(serverID: 50, senderID: Self.them),
        ]
        #expect(Self.anchor(unreadCount: 3, myLastReadID: 0, rows: rows) == .message(51))
    }

    @Test("counting back stops at the first unread when there is only one")
    func countBackOfOne() {
        #expect(Self.anchor(unreadCount: 1, myLastReadID: 0, rows: Self.inbound([53, 52, 51]))
            == .message(53))
    }

    @Test("fewer cached rows than the count gives up")
    func countBackShortCache() {
        #expect(Self.anchor(unreadCount: 5, myLastReadID: 0, rows: Self.inbound([53, 52, 51]))
            == .newest)
        // The same thing said a different way: a cache of nothing.
        #expect(Self.anchor(unreadCount: 5, myLastReadID: 0, rows: []) == .newest)
        // And a cache of only MY messages, which the count never included.
        let mine = [
            UnreadAnchor.Row(serverID: 53, senderID: Self.me),
            UnreadAnchor.Row(serverID: 52, senderID: Self.me),
        ]
        #expect(Self.anchor(unreadCount: 2, myLastReadID: 0, rows: mine) == .newest)
    }

    // MARK: - The cap

    @Test("a target beyond the render cap gives up rather than widening to it")
    func beyondTheCap() {
        // The cap is a HANG fix, not a preference: the stack that renders
        // the window is non-lazy, so reaching a message this far back means
        // laying out everything after it in one pass. Above it, the chat
        // opens at its newest message — the way the quote jump gives up.
        let rows = Self.inbound(Array((1...400).reversed().map(Int64.init)))
        #expect(Self.anchor(unreadCount: 400, myLastReadID: 0, rows: rows) == .newest)
    }

    @Test("the cap is the row count PLUS the margin above it")
    func theCapIncludesTheMargin() {
        let rows = Self.inbound(Array((1...400).reversed().map(Int64.init)))
        // The last distance that fits: one row for the target, plus the
        // margin that keeps it clear of the top sentinel.
        let deepest = Self.cap - UnreadAnchor.margin
        #expect(UnreadAnchor.rowsToRender(distanceFromNewest: deepest - 1) == Self.cap)
        #expect(Self.anchor(unreadCount: deepest, myLastReadID: 0, rows: rows)
            == .message(Int64(400 - deepest + 1)))
        // One row further back is one row too many. Landing it flush
        // against the top sentinel would fire a history page whose own
        // restore scroll fights the anchoring scroll.
        #expect(Self.anchor(unreadCount: deepest + 1, myLastReadID: 0, rows: rows) == .newest)
    }

    @Test("the marker branch is capped too")
    func markerBranchRespectsTheCap() {
        let rows = Self.inbound(Array((1...400).reversed().map(Int64.init)))
        #expect(Self.anchor(unreadCount: 400, myLastReadID: 0, rows: rows, cap: 40) == .newest)
        #expect(Self.anchor(unreadCount: 380, myLastReadID: 20, rows: rows, cap: 40) == .newest)
        // …and a shallow one still resolves at the same small cap.
        #expect(Self.anchor(unreadCount: 2, myLastReadID: 398, rows: rows, cap: 40)
            == .message(399))
    }

    // MARK: - The divider's placement

    private static func snapshot(
        localID: String, serverID: Int64?, senderID: Int64, at iso: String
    ) -> MessageSnapshot {
        MessageSnapshot(
            localID: localID,
            serverID: serverID,
            chatID: 42,
            senderID: senderID,
            body: "m",
            createdAt: ISO8601DateFormatter().date(from: iso)!,
            state: .sent)
    }

    private static var utcCalendar: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    @Test("the divider lands above the row the anchor names")
    func dividerPlacement() {
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: Self.them, at: "2026-08-19T10:00:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: Self.them, at: "2026-08-19T10:05:00Z"),
            Self.snapshot(localID: "s:3", serverID: 3, senderID: Self.them, at: "2026-08-19T10:10:00Z"),
        ]
        let sections = MessagePresentation.daySections(
            messages, firstUnreadID: 2, calendar: Self.utcCalendar)

        #expect(sections.count == 1)
        #expect(sections[0].unreadDividerAbove == "s:2")
    }

    @Test("nothing unread puts a divider nowhere")
    func noDividerWithoutAnAnchor() {
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: Self.them, at: "2026-08-19T10:00:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: Self.them, at: "2026-08-19T10:05:00Z"),
        ]
        let sections = MessagePresentation.daySections(messages, calendar: Self.utcCalendar)
        #expect(sections.allSatisfy { $0.unreadDividerAbove == nil })
    }

    @Test("the divider goes into the section its row is in, and only that one")
    func dividerLandsInOneSection() {
        // Three days; the boundary is on the middle one. A divider drawn
        // in two sections is two boundaries, and a reader cannot be at
        // both.
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: Self.them, at: "2026-08-17T10:00:00Z"),
            Self.snapshot(localID: "s:2", serverID: 2, senderID: Self.them, at: "2026-08-18T10:00:00Z"),
            Self.snapshot(localID: "s:3", serverID: 3, senderID: Self.them, at: "2026-08-18T11:00:00Z"),
            Self.snapshot(localID: "s:4", serverID: 4, senderID: Self.them, at: "2026-08-19T10:00:00Z"),
        ]
        let sections = MessagePresentation.daySections(
            messages, firstUnreadID: 3, calendar: Self.utcCalendar)

        #expect(sections.count == 3)
        #expect(sections.map { $0.unreadDividerAbove } == [nil, "s:3", nil])
    }

    @Test("a divider whose row is outside the window is drawn nowhere")
    func dividerOutsideTheWindow() {
        // What a long session eventually does: the render window is a
        // suffix and it SLIDES once it reaches the cap, so the divider's
        // row leaves with the rows around it. The boundary is not
        // re-anchored to a different row — it simply stops being drawn.
        let messages = [
            Self.snapshot(localID: "s:8", serverID: 8, senderID: Self.them, at: "2026-08-19T10:00:00Z"),
            Self.snapshot(localID: "s:9", serverID: 9, senderID: Self.them, at: "2026-08-19T10:05:00Z"),
        ]
        let sections = MessagePresentation.daySections(
            messages, firstUnreadID: 2, calendar: Self.utcCalendar)
        #expect(sections.allSatisfy { $0.unreadDividerAbove == nil })
    }

    @Test("a divider can land above a bubble with no text in it yet")
    func dividerAboveAStreamingAssistantRow() {
        // The assistant's reply counts as unread and its row is inserted
        // EMPTY and streamed into afterwards, so this is an ordinary state
        // and not a broken one.
        let messages = [
            Self.snapshot(localID: "s:1", serverID: 1, senderID: Self.me, at: "2026-08-19T10:00:00Z"),
            MessageSnapshot(
                localID: "s:2", serverID: 2, chatID: 42, senderID: 999, body: "",
                createdAt: ISO8601DateFormatter().date(from: "2026-08-19T10:01:00Z")!,
                state: .sent),
        ]
        let sections = MessagePresentation.daySections(
            messages, firstUnreadID: 2, calendar: Self.utcCalendar)
        #expect(sections[0].unreadDividerAbove == "s:2")
    }

    // MARK: - The way back down

    @Test("the jump-to-newest button is up exactly when the newest message is not")
    func jumpButtonRule() {
        #expect(ThreadFollow.showsJumpToNewest(isAtNewest: false, hasSettled: true))
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: true, hasSettled: true))
        // Not during the opening window, in either direction: the phone
        // starts `isAtNewest` false and the Mac starts it optimistically
        // true, so without this the button blinks through every open — and
        // after an anchored open it would blink at the one moment it is
        // genuinely needed.
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: false, hasSettled: false))
        #expect(!ThreadFollow.showsJumpToNewest(isAtNewest: true, hasSettled: false))
    }
}

/// The half of this feature that is not arithmetic: what an anchored open
/// is allowed to do to the read marker, asserted against the real
/// coordinator.
@MainActor
@Suite("Anchored open reads nothing")
struct AnchoredOpenReadTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    @MainActor
    private struct Harness {
        /// Retained: the coordinator holds only the mainContext, and
        /// SwiftData traps if the container backing a context deallocates.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        func readPosts() -> [RecordedRequest] {
            StubURLProtocol.requests(host: host)
                .filter { $0.method == "POST" && $0.url.path().hasSuffix("/read") }
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(host: String) throws -> Harness {
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        coordinator.ackTimeout = 0.2
        let chat = ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    private func dto(id: Int64, senderID: Int64 = 9) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: nil,
            body: "hello", createdAt: Self.serverDate, attachment: nil)
    }

    @Test("a chat opened at its oldest unread message reads nothing and keeps its count")
    func anchoredOpenReadsNothing() async throws {
        let harness = try makeHarness(host: "unread-anchored-open.test")
        defer { harness.tearDown() }

        // Five unread, and a marker that says where this person stopped.
        harness.chat(42)?.myLastReadID = 50
        try harness.context.save()
        for id in Int64(51)...Int64(55) {
            harness.coordinator.handle(frame: .message(dto(id: id)))
        }
        #expect(harness.chat(42)?.unreadCount == 5)

        // Opening: the view claims the chat before its layout has settled,
        // so it publishes "not at the newest message" — which is what BOTH
        // platforms now publish for the whole opening window, whatever
        // their `isPinnedToBottom` happens to start at.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)

        // …and once the anchored scroll has landed, the view publishes
        // AGAIN — from `hasSettled` on both platforms — with the same
        // answer, because the bottom sentinel is still far below the
        // viewport. The identical arguments are the point: settling is not
        // arriving. This is the claim the whole feature rests on, and it is
        // why the anchored open needs no new call site into `markRead` —
        // nothing on this path is allowed to reach it.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)

        #expect(harness.chat(42)?.unreadCount == 5, "the badge, the bold row and the tray notification all survive the open")
        #expect(harness.chat(42)?.myLastReadID == 50, "the monotonic marker must not have moved")
        #expect(harness.readPosts().isEmpty, "nothing may have reached the server")

        // The reader scrolls down — or presses the jump-to-newest button,
        // which is the same thing — and THAT reads the chat, doing
        // everything reading a chat has always done.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 55)
        #expect(harness.readPosts().count == 1)
    }

    @Test("a message arriving during an anchored open is not read either")
    func arrivalDuringAnAnchoredOpenStaysUnread() async throws {
        let harness = try makeHarness(host: "unread-anchored-arrival.test")
        defer { harness.tearDown() }

        harness.chat(42)?.myLastReadID = 50
        try harness.context.save()
        for id in Int64(51)...Int64(53) {
            harness.coordinator.handle(frame: .message(dto(id: id)))
        }
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)

        // The anchored open deliberately does not follow arrivals: all
        // three pin hooks that would have are gated on it, so the reader
        // stays where they were put and the sentinel goes on reporting
        // false. What lands now is genuinely unseen, and the count has to
        // say so — this is the product consequence nettrash chose, stated
        // as an assertion. (The gating itself is view code and is not what
        // this test can reach; what it pins is that the coordinator does
        // the right thing given the answer those hooks preserve.)
        harness.coordinator.handle(frame: .message(dto(id: 54)))

        #expect(harness.chat(42)?.unreadCount == 4)
        #expect(harness.chat(42)?.myLastReadID == 50)
        #expect(harness.readPosts().isEmpty)
    }
}

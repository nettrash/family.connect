//
//  UnreadBadgeTests.swift
//  FamilyConnectTests
//
//  The app-icon badge, and the two questions the feature itself did not
//  answer: what the icon says between process start and the first save,
//  and what happens to it when the chat is read on a DIFFERENT device.
//
//  Nothing here can read a badge back off an icon — `setBadgeCount` is
//  async and permission-gated, `dockTile.badgeLabel` belongs to a running
//  NSApplication, and Notification Center cannot be populated from a unit
//  test. So every test below asserts one of the three PURE seams the
//  feature was split along instead:
//
//    - `UnreadBadge.total`            — the arithmetic,
//    - `ChatSyncCoordinator.storedUnreadTotal` — the number the launch
//      publishes, which is the store's alone and needs no network,
//    - `ChatNotifier.isRead`          — whether a delivered banner has
//      been passed by a read marker,
//
//  plus the two pieces of state a resync writes: `myLastReadID`, applied
//  monotonically, and `lastResyncReadElsewhere`, which is the trigger the
//  banner teardown fires on.
//
//  Harnessed like UnreadRulesTests: an in-memory ModelContainer, a stubbed
//  URLSession routed per fake host (suites run in parallel — every host
//  here is unique and unregistered in a defer), and a real-but-never-
//  started ChatSocket.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Unread badge")
struct UnreadBadgeTests {

    // MARK: - Harness

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

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(
        host: String,
        chats: [ChatEntity] = [],
        handler: @escaping StubURLProtocol.Handler
    ) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        for chat in chats { container.mainContext.insert(chat) }
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    /// The two responses every resync makes before it reaches GET /chats,
    /// spelled once. Anna, id 7, in family 3.
    private static let meJSON = """
    {"user": {"id": 7, "username": "anna", "display_name": "Anna"},
     "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-19T17:00:00Z"},
     "role": "member"}
    """

    private static let familyJSON = """
    {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                "created_at": "2026-08-19T17:00:00Z"},
     "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}]}
    """

    /// One family chat, its `unread_count` and its `last_read_message_id`
    /// as the server would answer them — the two halves of the same row.
    private static func chatsJSON(
        unread: Int,
        lastMessageID: Int64,
        lastRead: Int64?
    ) -> String {
        let marker = lastRead.map { ", \"last_read_message_id\": \($0)" } ?? ""
        return """
        {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                             "peer_user_id": null},
                    "last_message": {"id": \(lastMessageID), "chat_id": 42, "sender_id": 9,
                                     "client_msg_id": null, "body": "hello",
                                     "created_at": "2026-08-19T17:05:00Z"},
                    "unread_count": \(unread)\(marker)}]}
        """
    }

    private func resyncHandler(
        unread: Int,
        lastMessageID: Int64,
        lastRead: Int64?
    ) -> StubURLProtocol.Handler {
        // Every body is built HERE and captured, not composed inside the
        // handler: the handler is @Sendable and runs off this actor.
        let me = Self.meJSON
        let family = Self.familyJSON
        let chats = Self.chatsJSON(unread: unread, lastMessageID: lastMessageID, lastRead: lastRead)
        return { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, me)
            case "/api/v1/families/mine":
                return .json(200, family)
            case "/api/v1/chats":
                return .json(200, chats)
            default:
                // The per-chat catch-up loop; nothing to catch up on.
                return .json(200, #"{"messages": []}"#)
            }
        }
    }

    // MARK: - The arithmetic

    @Test("the badge is the sum of the per-chat counts")
    func totalIsTheSumOfTheChats() {
        #expect(UnreadBadge.total(unreadCounts: []) == 0)
        #expect(UnreadBadge.total(unreadCounts: [0, 0]) == 0)
        #expect(UnreadBadge.total(unreadCounts: [3]) == 3)
        #expect(UnreadBadge.total(unreadCounts: [3, 0, 2]) == 5)
    }

    /// The clamp is per chat and not on the sum, so one broken row cannot
    /// take real unread messages in another chat down with it — the icon
    /// would then say nothing while the chat list still showed a capsule,
    /// which is the exact contradiction this whole file exists to stop.
    @Test("one impossible count cannot cancel out another chat's real one")
    func oneBrokenCountCannotCancelAnother() {
        #expect(UnreadBadge.total(unreadCounts: [-5, 3]) == 3)
        #expect(UnreadBadge.total(unreadCounts: [-5]) == 0)
    }

    // MARK: - The launch gap

    /// What the icon shows between process start and the first save. Until
    /// this seed existed nothing in the app had touched it: iOS was still
    /// showing whatever the last APNs push left there, and the Mac —
    /// `dockTile.badgeLabel` being per-process — was showing nothing beside
    /// a sidebar full of capsules.
    @Test("the launch seed is the store's own total, with nothing on the wire")
    func theLaunchSeedComesFromTheStoreAlone() throws {
        let harness = try makeHarness(
            host: "badge-launch-seed.test",
            chats: [
                ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths", unreadCount: 2),
                ChatEntity(chatID: 43, kind: "direct", pinRank: 1, title: "Kid", unreadCount: 1),
                ChatEntity(chatID: 44, kind: "direct", pinRank: 1, title: "Gran", unreadCount: 0),
            ],
            handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        #expect(harness.coordinator.storedUnreadTotal() == 3)
        // The one case a resync can never help with: this is the answer on
        // a launch with no network at all.
        harness.coordinator.refreshUnreadBadge()
        #expect(
            StubURLProtocol.requests(host: harness.host).isEmpty,
            "the seed must not need the server to say anything")
    }

    @Test("an empty store seeds nothing rather than refusing to answer")
    func anEmptyStoreSeedsZero() throws {
        let harness = try makeHarness(host: "badge-empty-store.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // Zero because the store is genuinely empty — a signed-out app, or
        // a fresh install — which is a real answer and the right one. What
        // it must never be is a CLEAR of a number the store disagrees with.
        #expect(harness.coordinator.storedUnreadTotal() == 0)
    }

    /// A cold launch from a push tap is the same seed, and it must not be
    /// mistaken for reading anything: the store still says what it said,
    /// and the badge with it, until the resync and then the reader arrive.
    @Test("a launch does not read anything, so the seed keeps the count")
    func aLaunchIsNotAReadAndKeepsTheCount() throws {
        let harness = try makeHarness(
            host: "badge-cold-launch.test",
            chats: [ChatEntity(
                chatID: 42, kind: "family", pinRank: 0, title: "The Smiths",
                unreadCount: 4, maxServerMessageID: 53, myLastReadID: 49)],
            handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        harness.coordinator.refreshUnreadBadge()

        #expect(harness.coordinator.storedUnreadTotal() == 4)
        #expect(harness.chat(42)?.unreadCount == 4)
        #expect(harness.chat(42)?.myLastReadID == 49, "launching is not reading")
    }

    // MARK: - The marker, applied monotonically

    @Test("a resync learns a read that happened on another device")
    func resyncAdoptsTheMarkerFromAnotherDevice() async throws {
        let harness = try makeHarness(
            host: "badge-marker-forward.test",
            chats: [ChatEntity(
                chatID: 42, kind: "family", pinRank: 0, title: "The Smiths",
                unreadCount: 3, maxServerMessageID: 53, myLastReadID: 50)],
            handler: resyncHandler(unread: 0, lastMessageID: 53, lastRead: 53))
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        // The marker is the person's, not the device's: somebody read this
        // chat on their phone and this Mac has just found out.
        #expect(harness.chat(42)?.myLastReadID == 53)
        // The count — and so the icon — follows `unread_count`, which comes
        // off the same row of the same query.
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.coordinator.storedUnreadTotal() == 0)
        // …and that is the trigger the banner teardown fires on.
        #expect(harness.coordinator.lastResyncReadElsewhere == [42: 53])
    }

    /// The response was built before it was sent. One still in flight while
    /// its owner keeps reading would walk the marker backwards — re-arming
    /// `markRead`'s throttle to re-report a read the server already has,
    /// and telling the teardown a chat it just settled is unread again.
    @Test("a response in flight while the reader reads cannot walk the marker backwards")
    func aStaleResponseCannotWalkTheMarkerBackwards() async throws {
        let harness = try makeHarness(
            host: "badge-marker-backward.test",
            chats: [ChatEntity(
                chatID: 42, kind: "family", pinRank: 0, title: "The Smiths",
                unreadCount: 0, maxServerMessageID: 99, myLastReadID: 99)],
            handler: resyncHandler(unread: 0, lastMessageID: 99, lastRead: 53))
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chat(42)?.myLastReadID == 99)
        // Nothing moved, so nothing claims a chat was read elsewhere and no
        // banner is taken down on that evidence.
        #expect(harness.coordinator.lastResyncReadElsewhere.isEmpty)
    }

    /// `0` is a real answer — "has never reported reading anything here" —
    /// and it is the one every device gets for a chat nobody has opened.
    /// Applied monotonically it lands exactly where an absent field does.
    @Test("zero is an answer, not an erasure")
    func zeroIsAnAnswerNotAnErasure() async throws {
        let harness = try makeHarness(
            host: "badge-marker-zero.test",
            chats: [ChatEntity(
                chatID: 42, kind: "family", pinRank: 0, title: "The Smiths",
                unreadCount: 0, maxServerMessageID: 53, myLastReadID: 53)],
            handler: resyncHandler(unread: 0, lastMessageID: 53, lastRead: 0))
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chat(42)?.myLastReadID == 53)
        #expect(harness.coordinator.lastResyncReadElsewhere.isEmpty)
    }

    /// The field is Optional in the DTO for one reason only: a server
    /// binary older than it omits it, and this client has to go on reading
    /// its chat list. Absent must land where 0 lands — on the stored value.
    @Test("a server that does not send the field leaves the marker alone")
    func anAbsentFieldLeavesTheMarkerAlone() async throws {
        let harness = try makeHarness(
            host: "badge-marker-absent.test",
            chats: [ChatEntity(
                chatID: 42, kind: "family", pinRank: 0, title: "The Smiths",
                unreadCount: 2, maxServerMessageID: 53, myLastReadID: 50)],
            handler: resyncHandler(unread: 2, lastMessageID: 53, lastRead: nil))
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        #expect(harness.chat(42)?.myLastReadID == 50)
        #expect(harness.chat(42)?.unreadCount == 2)
        #expect(harness.coordinator.storedUnreadTotal() == 2)
        #expect(harness.coordinator.lastResyncReadElsewhere.isEmpty)
    }

    /// The chat this device has never seen before: there is nothing to
    /// have moved FORWARD from, so a first sync must not present itself as
    /// somebody having just read something.
    @Test("a chat arriving for the first time adopts the marker")
    func aFirstSyncAdoptsTheMarker() async throws {
        let harness = try makeHarness(
            host: "badge-marker-firstsync.test",
            handler: resyncHandler(unread: 2, lastMessageID: 55, lastRead: 53))
        defer { harness.tearDown() }

        await harness.coordinator.resync()

        // Adopted, so this device does not re-report a read the server
        // already has the moment somebody opens the chat…
        #expect(harness.chat(42)?.myLastReadID == 53)
        #expect(harness.chat(42)?.unreadCount == 2)
        // …and the two messages above the marker keep their banners: the
        // teardown is a threshold, so 54 and 55 are untouched by it.
        #expect(harness.coordinator.lastResyncReadElsewhere == [42: 53])
        #expect(ChatNotifier.isRead(
            by: harness.coordinator.lastResyncReadElsewhere,
            threadIdentifier: ChatNotifier.threadIdentifier(chatID: 42),
            userInfo: ChatNotifier.userInfo(chatID: 42, messageID: 54)) == false)
    }

    // MARK: - What the marker takes down

    @Test("a banner at or below the marker is stale; one above it is not")
    func theMarkerIsAThresholdAndNotAReference() {
        let markers: [Int64: Int64] = [42: 53]
        func isRead(messageID: Int64, chatID: Int64 = 42) -> Bool {
            ChatNotifier.isRead(
                by: markers,
                threadIdentifier: ChatNotifier.threadIdentifier(chatID: chatID),
                userInfo: ChatNotifier.userInfo(chatID: chatID, messageID: messageID))
        }

        #expect(isRead(messageID: 12))
        // At the marker, not merely below it: it names the newest message
        // that has been read, and retention may already have swept it — so
        // it is compared against and never fetched.
        #expect(isRead(messageID: 53))
        // The ordinary case on a Mac, which raises its own banners off the
        // socket the whole time it runs. Dismissing a chat's notifications
        // wholesale would destroy this one.
        #expect(isRead(messageID: 54) == false)
        // Another chat's banner is not this marker's business.
        #expect(isRead(messageID: 12, chatID: 43) == false)
    }

    @Test("no markers, nothing dismissed")
    func noMarkersDismissNothing() {
        #expect(ChatNotifier.isRead(
            by: [:],
            threadIdentifier: ChatNotifier.threadIdentifier(chatID: 42),
            userInfo: ChatNotifier.userInfo(chatID: 42, messageID: 1)) == false)
    }

    /// The APNs shape: the chat and the message ride as custom keys next to
    /// the `aps` dictionary, as JSON numbers, and PushRoute is the only
    /// thing in the app that parses them.
    @Test("an APNs-delivered banner is matched by its custom keys")
    func anAPNsBannerIsMatchedByItsCustomKeys() {
        let payload: [AnyHashable: Any] = [
            "kind": "message", "chat_id": 42, "message_id": 50,
        ]
        #expect(ChatNotifier.isRead(by: [42: 53], threadIdentifier: "", userInfo: payload))
        #expect(ChatNotifier.isRead(by: [42: 49], threadIdentifier: "", userInfo: payload) == false)
    }

    /// …and the thread key is the fallback, so a banner whose chat id did
    /// not survive its payload is still matched by the grouping key the
    /// server and this app both spell `chat-<id>`.
    @Test("a banner with no chat id is matched by its thread key")
    func aBannerWithoutAChatIDIsMatchedByItsThread() {
        let payload: [AnyHashable: Any] = ["kind": "message", "message_id": 50]
        #expect(ChatNotifier.isRead(by: [42: 53], threadIdentifier: "chat-42", userInfo: payload))
        #expect(ChatNotifier.isRead(by: [43: 53], threadIdentifier: "chat-42", userInfo: payload) == false)
    }

    /// Nothing this app raises and nothing this server sends for a chat
    /// lacks a message id, so this is a case that does not arise — but if
    /// it ever did, a banner left up costs a glance and one taken down
    /// costs the message.
    @Test("a banner with no message id is left alone")
    func aBannerWithNoMessageIDIsLeftAlone() {
        #expect(ChatNotifier.isRead(
            by: [42: 53],
            threadIdentifier: "chat-42",
            userInfo: ["kind": "message", "chat_id": 42]) == false)
        // A board note or a join request is not in a chat and no read
        // marker has anything to say about it.
        #expect(ChatNotifier.isRead(
            by: [42: 53],
            threadIdentifier: "chat-42",
            userInfo: ["kind": "board_note", "message_id": 1]) == false)
    }

    @Test("the thread key round-trips, and nothing else parses as one")
    func theThreadKeyRoundTrips() {
        #expect(ChatNotifier.chatID(threadIdentifier: ChatNotifier.threadIdentifier(chatID: 42)) == 42)
        #expect(ChatNotifier.chatID(threadIdentifier: "chat-") == nil)
        #expect(ChatNotifier.chatID(threadIdentifier: "chat-x") == nil)
        #expect(ChatNotifier.chatID(threadIdentifier: "board") == nil)
        #expect(ChatNotifier.chatID(threadIdentifier: "") == nil)
    }

    @Test("the message id comes off a message push and nothing else")
    func messageIDIsReadOffMessagePushesOnly() {
        #expect(PushRoute.messageID(userInfo: ["kind": "message", "message_id": 1338]) == 1338)
        // FCM's data map is strings-only; routing has always tolerated it.
        #expect(PushRoute.messageID(userInfo: ["kind": "message", "message_id": "1338"]) == 1338)
        #expect(PushRoute.messageID(userInfo: ["kind": "board_note", "message_id": 1338]) == nil)
        #expect(PushRoute.messageID(userInfo: ["kind": "message"]) == nil)
        #expect(PushRoute.messageID(userInfo: [:]) == nil)
    }
}

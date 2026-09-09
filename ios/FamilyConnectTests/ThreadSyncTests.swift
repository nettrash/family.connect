//
//  ThreadSyncTests.swift
//  FamilyConnectTests
//
//  How the chain lives in the store (docs/protocol.md, "Threads").
//
//  The live rule is the one that can go wrong in both directions: the
//  root's own frame is never re-sent, so a client holding the root counts
//  each reply as it ARRIVES — and only then. Counting a page's replies
//  double-counts (the root's recomputed count already includes them);
//  counting a re-delivery double-counts (the REST echo and the socket frame
//  are two copies of one reply); counting nothing leaves "1 reply" under a
//  message the family has answered five times.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Thread sync")
struct ThreadSyncTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func message(serverID: Int64) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.serverID == serverID })
            return (try? context.fetch(descriptor))?.first
        }

        func message(localID: String) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.localID == localID })
            return (try? context.fetch(descriptor))?.first
        }

        func messages() -> [MessageEntity] {
            (try? context.fetch(FetchDescriptor<MessageEntity>())) ?? []
        }

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(host: String, handler: @escaping StubURLProtocol.Handler = { _ in .json(200, #"{"messages": []}"#) }) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        let chat = ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, context: container.mainContext, host: host)
    }

    private func dto(
        id: Int64,
        senderID: Int64 = 9,
        clientMsgID: String? = nil,
        body: String = "Dinner at 7?",
        replyTo: Int64? = nil,
        threadRootID: Int64? = nil,
        replyCount: Int64? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: clientMsgID,
            body: body, createdAt: Self.serverDate,
            replyTo: replyTo.map { ReplyToDTO(messageID: $0, senderID: 7, excerpt: "Dinner at 7?") },
            threadRootID: threadRootID, replyCount: replyCount)
    }

    /// The live half: a reply on the socket raises the cached root by one,
    /// and its second copy — the REST echo, a resync overlap — does not.
    @Test("a live reply raises the cached root once")
    func liveReplyBumpsOnce() throws {
        let harness = try makeHarness(host: "thread-sync-live.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)

        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: true, live: true)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
        #expect(harness.message(serverID: 101)?.threadRootID == 100)

        // The same reply again, on either path, is the same reply.
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: true, live: true)
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 1, "two copies of one reply are one reply")
    }

    /// A page is not live: the replies it carries are already inside the
    /// count on the root's own copy, which is the truth and overwrites.
    @Test("a page never bumps, and a server copy of the root overwrites")
    func pagesOverwriteRatherThanCount() throws {
        let harness = try makeHarness(host: "thread-sync-page.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100, replyCount: 2), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 2)

        // Two replies from a history page: first sight, but not live.
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: false)
        _ = harness.coordinator.upsert(dto(id: 102, replyTo: 100, threadRootID: 100), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 2, "the root's count already held them")

        // A live one on top, then a fresh copy of the root with the truth.
        _ = harness.coordinator.upsert(dto(id: 103, replyTo: 100, threadRootID: 100), bumpUnread: true, live: true)
        #expect(harness.message(serverID: 100)?.replyCount == 3)
        _ = harness.coordinator.upsert(dto(id: 100, replyCount: 5), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 5, "the server's recomputed count wins")
        // And ABSENT on a later copy means nobody has answered: retention
        // took the replies, and the next copy of the root says so.
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 0)
    }

    /// A reply for a root this device does not hold has nothing to raise
    /// and must not fail; the root, when it comes, carries its own count.
    @Test("a reply ahead of its root is harmless")
    func replyBeforeRoot() throws {
        let harness = try makeHarness(host: "thread-sync-orphan.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: true, live: true)
        #expect(harness.message(serverID: 101)?.threadRootID == 100)
        #expect(harness.message(serverID: 100) == nil)
        _ = harness.coordinator.upsert(dto(id: 100, replyCount: 1), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
    }

    /// The reader's own reply: the pending row carries the root it derived
    /// from the quoted message it holds — so the thread surface shows it at
    /// once — and its ack raises the root exactly once.
    @Test("an own reply is counted on its ack, and knows its root before it")
    func ownReplyCountsOnAck() throws {
        let harness = try makeHarness(host: "thread-sync-own.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: false)

        // Quoting the REPLY: the root is still the top of the chain.
        let localID = try #require(harness.coordinator.enqueue(
            body: "Pizza then?", in: 42,
            replyTo: ReplyToDTO(messageID: 101, senderID: 9, excerpt: "Works for me")))
        let pending = try #require(harness.message(localID: localID))
        #expect(pending.threadRootID == 100, "derived from the quoted row this client holds")
        #expect(harness.message(serverID: 100)?.replyCount == 0, "not counted until it is sent")

        let clientMsgID = try #require(pending.clientMsgID)
        // The REST echo of this device's own send — live.
        _ = harness.coordinator.upsert(
            dto(id: 102, senderID: 7, clientMsgID: clientMsgID, replyTo: 101, threadRootID: 100),
            bumpUnread: false, live: true)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
        #expect(pending.serverID == 102)
        #expect(pending.threadRootID == 100)
        // The socket's copy of the same send, on the sender's own connection.
        _ = harness.coordinator.upsert(
            dto(id: 102, senderID: 7, clientMsgID: clientMsgID, replyTo: 101, threadRootID: 100),
            bumpUnread: false, live: true)
        #expect(harness.message(serverID: 100)?.replyCount == 1, "acked once, counted once")
    }

    /// Quoting a message this client does not hold: no root can be derived,
    /// and the pending row says so rather than guessing the quote is it.
    @Test("a pending reply to an unknown message derives no root")
    func unknownQuoteDerivesNoRoot() throws {
        let harness = try makeHarness(host: "thread-sync-unknown.test")
        defer { harness.tearDown() }
        let localID = try #require(harness.coordinator.enqueue(
            body: "Pizza then?", in: 42,
            replyTo: ReplyToDTO(messageID: 555, senderID: 9, excerpt: "Works for me")))
        #expect(harness.message(localID: localID)?.threadRootID == nil)
    }

    /// The thread read: page after page until a short one, through the
    /// ordinary upsert — so a reply the page carries lands in the store
    /// under the root it names, and the root's own count is the page's.
    @Test("loadThread pages until a short page and fills the store")
    func loadThreadPages() async throws {
        let harness = try makeHarness(host: "thread-sync-load.test") { request in
            let query = request.url.query ?? ""
            if query.contains("after_id=101") {
                return .json(200, """
                    {"messages": [
                      {"id": 102, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-102",
                       "body": "Pizza then?", "created_at": "2026-08-19T17:06:00Z",
                       "reply_to": {"message_id": 101, "sender_id": 9, "excerpt": "Works for me"},
                       "thread_root_id": 100}
                    ]}
                    """)
            }
            return .json(200, """
                {"messages": [
                  {"id": 100, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-100",
                   "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:00Z", "reply_count": 2},
                  {"id": 101, "chat_id": 42, "sender_id": 9, "client_msg_id": "c-101",
                   "body": "Works for me", "created_at": "2026-08-19T17:04:00Z",
                   "reply_to": {"message_id": 100, "sender_id": 7, "excerpt": "Dinner at 7?"},
                   "thread_root_id": 100}
                ]}
                """)
        }
        defer { harness.tearDown() }

        let completed = await harness.coordinator.loadThread(chatID: 42, rootID: 100, limit: 2)
        #expect(completed)
        let sent = StubURLProtocol.requests(host: harness.host)
        #expect(sent.count == 2, "a full page asks for the next; a short one stops")
        #expect(harness.messages().count == 3)
        #expect(harness.message(serverID: 100)?.replyCount == 2, "the page's count, not the page's replies")
        #expect(harness.message(serverID: 102)?.threadRootID == 100)
    }

    /// The thread read reconciling THIS device's own pending reply: the page
    /// is root-first and the root's copy already counts the reply, so the
    /// fold must not count it again — it is not a live arrival.
    @Test("a thread page reconciling a pending own reply does not double count")
    func threadPageReconcilingOwnReplyDoesNotDoubleCount() async throws {
        var pendingClientMsgID = ""
        let harness = try makeHarness(host: "thread-sync-fold.test") { _ in
            .json(200, """
                {"messages": [
                  {"id": 100, "chat_id": 42, "sender_id": 9, "client_msg_id": "c-100",
                   "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:00Z", "reply_count": 1},
                  {"id": 102, "chat_id": 42, "sender_id": 7, "client_msg_id": "\(pendingClientMsgID)",
                   "body": "Pizza then?", "created_at": "2026-08-19T17:06:00Z",
                   "reply_to": {"message_id": 100, "sender_id": 9, "excerpt": "Dinner at 7?"},
                   "thread_root_id": 100}
                ]}
                """)
        }
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        let localID = try #require(harness.coordinator.enqueue(
            body: "Pizza then?", in: 42,
            replyTo: ReplyToDTO(messageID: 100, senderID: 9, excerpt: "Dinner at 7?")))
        pendingClientMsgID = try #require(harness.message(localID: localID)?.clientMsgID)

        #expect(await harness.coordinator.loadThread(chatID: 42, rootID: 100))

        #expect(harness.message(serverID: 100)?.replyCount == 1, "the page's count, not the page's count plus the fold")
        #expect(harness.message(localID: localID)?.serverID == 102)
    }

    /// The `after_id` catch-up stands in for the frames missed while away:
    /// it delivers only what is newer than everything held, so a cached
    /// root's count cannot yet include the reply, and it counts — exactly
    /// the call `runCatchUp` makes.
    @Test("a catch-up reply counts as live")
    func catchUpReplyCountsAsLive() throws {
        let harness = try makeHarness(host: "thread-sync-catchup.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        _ = harness.coordinator.upsert(dto(id: 101, replyTo: 100, threadRootID: 100), bumpUnread: false, live: true)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
        // A history page — older than what is held — does not.
        _ = harness.coordinator.upsert(dto(id: 99, replyTo: 100, threadRootID: 100), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
    }

    /// The thread read is no part of catch-up: the rows it fetches outside
    /// the window must not move the two paging cursors, or the next history
    /// page — or the next catch-up — would skip everything in between.
    @Test("loadThread leaves the paging cursors alone")
    func loadThreadLeavesCursorsAlone() async throws {
        let harness = try makeHarness(host: "thread-sync-cursors.test") { _ in
            .json(200, """
                {"messages": [
                  {"id": 100, "chat_id": 42, "sender_id": 9, "client_msg_id": "c-100",
                   "body": "Dinner at 7?", "created_at": "2026-08-19T17:03:00Z", "reply_count": 1},
                  {"id": 300, "chat_id": 42, "sender_id": 7, "client_msg_id": "c-300",
                   "body": "Still on?", "created_at": "2026-08-19T18:03:00Z",
                   "reply_to": {"message_id": 100, "sender_id": 9, "excerpt": "Dinner at 7?"},
                   "thread_root_id": 100}
                ]}
                """)
        }
        defer { harness.tearDown() }
        // The contiguous window: 200…210.
        for id in Int64(200)...210 { _ = harness.coordinator.upsert(dto(id: id), bumpUnread: false) }
        let chat = try #require(harness.chat(42))
        #expect(chat.oldestLoadedMessageID == 200)
        #expect(chat.maxServerMessageID == 210)

        #expect(await harness.coordinator.loadThread(chatID: 42, rootID: 100))

        #expect(harness.message(serverID: 100) != nil)
        #expect(harness.message(serverID: 300) != nil)
        #expect(chat.oldestLoadedMessageID == 200, "a root older than the window is not the window's edge")
        #expect(chat.maxServerMessageID == 210, "a reply newer than the window is not the catch-up cursor")
        // Once a page reaches the root, it is the window's edge.
        _ = harness.coordinator.upsert(dto(id: 100, replyCount: 1), bumpUnread: false)
        #expect(chat.oldestLoadedMessageID == 100)
    }

    /// A failed read is reported, and what was cached stays drawable.
    @Test("a failed thread read answers false and keeps the cache")
    func loadThreadFailure() async throws {
        let harness = try makeHarness(host: "thread-sync-fail.test") { _ in .json(500, #"{"error": "boom"}"#) }
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100, replyCount: 1), bumpUnread: false)
        let completed = await harness.coordinator.loadThread(chatID: 42, rootID: 100)
        #expect(!completed)
        #expect(harness.message(serverID: 100)?.replyCount == 1)
    }
}

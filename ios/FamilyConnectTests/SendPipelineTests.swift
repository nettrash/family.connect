//
//  SendPipelineTests.swift
//  FamilyConnectTests
//
//  The optimistic-send pipeline and the dedup matrix, against an
//  in-memory ModelContainer and a stubbed URLSession. The coordinator's
//  ChatSocket is real but never started, so socket sends throw
//  notConnected and `deliver` falls straight through to REST — which is
//  exactly the deterministic path these tests want; the ack path is
//  driven by injecting frames through `handle(frame:)` directly.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Send pipeline")
struct SendPipelineTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    // MARK: - Harness

    @MainActor
    private struct Harness {
        /// Retained here: the coordinator holds only the mainContext, and
        /// SwiftData traps if the container backing a context deallocates.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

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

    private func makeHarness(host: String, handler: @escaping StubURLProtocol.Handler) throws -> Harness {
        StubURLProtocol.register(host: host, handler: handler)
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
        return Harness(container: container, coordinator: coordinator, context: container.mainContext, host: host)
    }

    /// A 201 responder that echoes back the client_msg_id it was sent —
    /// what the real server does — assigning `serverID`.
    private static func echoingHandler(serverID: Int64) -> StubURLProtocol.Handler {
        { request in
            guard request.method == "POST", request.url.path().hasSuffix("/messages") else {
                return .empty(204)
            }
            let clientMsgID = request.bodyJSON()?["client_msg_id"] as? String ?? "?"
            let body = request.bodyJSON()?["body"] as? String ?? ""
            return .json(201, """
            {"message": {"id": \(serverID), "chat_id": 42, "sender_id": 7,
             "client_msg_id": "\(clientMsgID)", "body": "\(body)",
             "created_at": "2026-08-19T17:05:00Z"}}
            """)
        }
    }

    private func dto(id: Int64, senderID: Int64, clientMsgID: String?, body: String = "hello") -> MessageDTO {
        MessageDTO(id: id, chatID: 42, senderID: senderID, clientMsgID: clientMsgID, body: body, createdAt: Self.serverDate)
    }

    // MARK: - Optimistic insert

    @Test("enqueue inserts an optimistic pending row and updates the chat preview")
    func optimisticFields() throws {
        let harness = try makeHarness(host: "send-optimistic.test", handler: Self.echoingHandler(serverID: 1))
        defer { harness.tearDown() }

        let localID = harness.coordinator.enqueue(body: "  Dinner at 7?  ", in: 42)

        let row = try #require(harness.messages().first)
        #expect(row.localID == localID)
        #expect(row.localID.hasPrefix("c:"))
        #expect(row.serverID == nil)
        #expect(row.clientMsgID != nil)
        #expect(row.localID == "c:\(row.clientMsgID ?? "")")
        #expect(row.senderID == 7)
        #expect(row.body == "Dinner at 7?") // trimmed
        #expect(row.state == .pending)

        let chat = try #require(harness.chat(42))
        #expect(chat.lastMessagePreview == "Dinner at 7?")
        #expect(chat.lastMessageSenderID == 7)
    }

    @Test("an empty/whitespace body is refused")
    func emptyBodyRefused() throws {
        let harness = try makeHarness(host: "send-empty.test", handler: Self.echoingHandler(serverID: 1))
        defer { harness.tearDown() }

        #expect(harness.coordinator.enqueue(body: "   \n ", in: 42) == nil)
        #expect(harness.messages().isEmpty)
    }

    // MARK: - Ack reconciliation

    @Test("the ack reconciles the pending row in place — stable localID, server time, sent")
    func ackReconciliation() throws {
        let harness = try makeHarness(host: "send-ack.test", handler: Self.echoingHandler(serverID: 1))
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "hello", in: 42))
        let clientMsgID = try #require(harness.messages().first?.clientMsgID)

        harness.coordinator.handle(frame: .ack(
            clientMsgID: clientMsgID,
            message: dto(id: 1338, senderID: 7, clientMsgID: clientMsgID)))

        let rows = harness.messages()
        #expect(rows.count == 1)
        let row = try #require(rows.first)
        #expect(row.localID == localID)          // identity never changes
        #expect(row.serverID == 1338)
        #expect(row.createdAt == Self.serverDate) // rewritten to server time
        #expect(row.state == .sent)

        let chat = try #require(harness.chat(42))
        #expect(chat.maxServerMessageID == 1338)
        #expect(chat.unreadCount == 0)           // own message never bumps
    }

    @Test("the echoed message frame after the ack does not duplicate")
    func echoAfterAckNoDuplicate() throws {
        let harness = try makeHarness(host: "send-echo.test", handler: Self.echoingHandler(serverID: 1))
        defer { harness.tearDown() }

        _ = harness.coordinator.enqueue(body: "hello", in: 42)
        let clientMsgID = try #require(harness.messages().first?.clientMsgID)
        let message = dto(id: 1338, senderID: 7, clientMsgID: clientMsgID)

        harness.coordinator.handle(frame: .ack(clientMsgID: clientMsgID, message: message))
        harness.coordinator.handle(frame: .message(message))
        // And a resync re-delivery on top:
        _ = harness.coordinator.upsert(message, bumpUnread: false)

        #expect(harness.messages().count == 1)
    }

    // MARK: - REST fallback

    @Test("with no socket, deliver falls back to REST and the row settles sent")
    func restFallback() async throws {
        let harness = try makeHarness(host: "send-rest.test", handler: Self.echoingHandler(serverID: 2001))
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "over REST", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.localID == localID)
        #expect(row.serverID == 2001)
        #expect(row.state == .sent)
        #expect(row.createdAt == Self.serverDate)
    }

    @Test("a REST fallback refused with a TERMINAL code marks the row failed")
    func failedMarking() async throws {
        let harness = try makeHarness(host: "send-fail.test", handler: { _ in
            .json(400, #"{"error": {"code": "validation", "message": "nope"}}"#)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "doomed", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.state == .failed)
        #expect(row.serverID == nil)
        #expect(row.nextAttemptAt == nil, "a refusal is not retried")
    }

    // MARK: - Transient failures are not refusals

    /// The rule the whole outbox rests on (docs/protocol.md, "Sending on
    /// an unreliable network"): a 5xx says the request was not answered,
    /// not that it was refused, so the row stays queued and comes back.
    @Test("a 5xx keeps the row pending and schedules another attempt")
    func transientKeepsPending() async throws {
        let harness = try makeHarness(host: "send-5xx.test", handler: { _ in
            .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "unknown outcome", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.state == .pending, "a 5xx is an unknown outcome, never a red bubble")
        #expect(row.sendAttempts == 1)
        #expect(row.nextAttemptAt != nil, "and it is scheduled to come back")
    }

    /// A transport failure — no HTTP answer at all — is the same case.
    @Test("a transport failure keeps the row pending")
    func transportKeepsPending() async throws {
        let harness = try makeHarness(host: "send-transport.test", handler: { _ in
            .failure(URLError(.networkConnectionLost))
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "in a tunnel", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.state == .pending)
        #expect(row.nextAttemptAt != nil)
    }

    /// Bounded, and it gives up VISIBLY: a red bubble is this app
    /// promising that nothing else is coming.
    @Test("a transient failure that never clears eventually goes red")
    func transientEventuallyFails() async throws {
        let harness = try makeHarness(host: "send-exhaust.test", handler: { _ in
            .json(503, #"{"error": {"code": "internal", "message": "still down"}}"#)
        })
        defer { harness.tearDown() }
        harness.coordinator.maxSendAttempts = 3
        harness.coordinator.sendBackoff = ReconnectBackoff(base: 0.01, cap: 0.02)

        let localID = try #require(harness.coordinator.enqueue(body: "doomed", in: 42))
        for _ in 0..<3 { await harness.coordinator.deliver(localID: localID) }

        let row = try #require(harness.messages().first)
        #expect(row.state == .failed)
        #expect(row.sendAttempts == 3)
        #expect(row.nextAttemptAt == nil)
    }

    /// nginx answers its rate limit with 429 and an HTML body, so the
    /// status alone has to carry the meaning — and `Retry-After` is the
    /// server saying how long its bucket needs.
    @Test("a 429 is transient and its Retry-After sets the next attempt")
    func throttleHonoursRetryAfter() async throws {
        let harness = try makeHarness(host: "send-429.test", handler: { _ in
            StubResponse(status: 429,
                         headers: ["Retry-After": "30", "Content-Type": "text/html"],
                         body: Data("<html>429</html>".utf8))
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "too eager", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.state == .pending, "a throttle is not a refusal")
        let wait = try #require(row.nextAttemptAt).timeIntervalSinceNow
        #expect(wait > 25, "the server's own Retry-After wins over our backoff: \(wait)")
    }

    /// A person asking again is a fresh budget: the automatic attempts
    /// were spent on a network that has probably changed since.
    @Test("tap-to-retry resets the automatic attempt count")
    func retryResetsTheBudget() async throws {
        let counter = Counter()
        let harness = try makeHarness(host: "send-retrybudget.test", handler: { request in
            guard request.method == "POST", request.url.path().hasSuffix("/messages") else {
                return .empty(204)
            }
            if counter.increment() == 1 {
                return .json(400, #"{"error": {"code": "validation", "message": "nope"}}"#)
            }
            let clientMsgID = request.bodyJSON()?["client_msg_id"] as? String ?? "?"
            return .json(201, """
            {"message": {"id": 4004, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "\(clientMsgID)", "body": "x",
             "created_at": "2026-08-19T17:05:00Z"}}
            """)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "x", in: 42))
        await harness.coordinator.deliver(localID: localID)
        #expect(harness.messages().first?.state == .failed)
        #expect(harness.messages().first?.sendAttempts == 1)

        harness.coordinator.retry(localID: localID)
        // `retry` spawns the delivery; wait for it rather than leaving it
        // running past the end of the test, where its SwiftData fetch would
        // outlive this harness's container.
        for _ in 0..<100 where harness.messages().first?.state != .sent {
            try? await Task.sleep(nanoseconds: 20_000_000)
        }
        #expect(harness.messages().first?.state == .sent)
        #expect(harness.messages().first?.sendAttempts == 0, "a person asking again is a fresh budget")
    }

    // MARK: - The outbox is not a step of the read pipeline

    /// The regression this whole change exists for: the flush used to be
    /// the tail of `resync()`, behind two early returns, so the network
    /// that stranded a message was exactly the one that stopped it being
    /// re-sent.
    @Test("resync flushes the outbox even when GET /me fails")
    func outboxFlushesWhenTheReadPipelineDies() async throws {
        let sent = Counter()
        let harness = try makeHarness(host: "send-resync.test", handler: { request in
            if request.method == "POST", request.url.path().hasSuffix("/messages") {
                _ = sent.increment()
                let clientMsgID = request.bodyJSON()?["client_msg_id"] as? String ?? "?"
                return .json(201, """
                {"message": {"id": 900, "chat_id": 42, "sender_id": 7,
                 "client_msg_id": "\(clientMsgID)", "body": "stranded",
                 "created_at": "2026-08-19T17:05:00Z"}}
                """)
            }
            // Every read fails, /me first.
            return .json(500, #"{"error": {"code": "internal", "message": "down"}}"#)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "stranded", in: 42))
        let row = try #require(harness.messages().first)
        row.state = .pending
        row.serverID = nil
        // Older than the sweep's floor for a row that was never attempted.
        row.createdAt = Date().addingTimeInterval(-120)
        try harness.context.save()

        await harness.coordinator.resync()

        #expect(sent.value >= 1, "the outbox must be flushed before the reads can fail")
        #expect(harness.messages().first(where: { $0.localID == localID })?.state == .sent)
    }

    /// And it does not fire early: a row scheduled for later is left alone.
    @Test("the sweep respects a scheduled next attempt")
    func sweepRespectsSchedule() async throws {
        let sent = Counter()
        let harness = try makeHarness(host: "send-sched.test", handler: { request in
            if request.method == "POST", request.url.path().hasSuffix("/messages") {
                _ = sent.increment()
                let clientMsgID = request.bodyJSON()?["client_msg_id"] as? String ?? "?"
                return .json(201, """
                {"message": {"id": 901, "chat_id": 42, "sender_id": 7,
                 "client_msg_id": "\(clientMsgID)", "body": "later",
                 "created_at": "2026-08-19T17:05:00Z"}}
                """)
            }
            return .empty(204)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.enqueue(body: "later", in: 42)
        let row = try #require(harness.messages().first)
        row.state = .pending
        row.serverID = nil
        row.createdAt = Date().addingTimeInterval(-120)
        row.nextAttemptAt = Date().addingTimeInterval(60)
        try harness.context.save()
        let before = sent.value

        await harness.coordinator.sweepOutbox()
        #expect(sent.value == before, "a row that is not due yet is not re-sent")

        row.nextAttemptAt = Date().addingTimeInterval(-1)
        try harness.context.save()
        await harness.coordinator.sweepOutbox()
        #expect(sent.value > before, "and one that is due is")
    }

    @Test("retry re-sends the SAME client_msg_id and keeps the row identity")
    func retrySameClientMsgID() async throws {
        let counter = Counter()
        let harness = try makeHarness(host: "send-retry.test", handler: { request in
            guard request.method == "POST", request.url.path().hasSuffix("/messages") else {
                return .empty(204)
            }
            if counter.increment() == 1 {
                return .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
            }
            let clientMsgID = request.bodyJSON()?["client_msg_id"] as? String ?? "?"
            return .json(200, """
            {"message": {"id": 3003, "chat_id": 42, "sender_id": 7,
             "client_msg_id": "\(clientMsgID)", "body": "second try",
             "created_at": "2026-08-19T17:05:00Z"}}
            """)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "second try", in: 42))
        let originalClientMsgID = try #require(harness.messages().first?.clientMsgID)

        await harness.coordinator.deliver(localID: localID) // → queued, not refused
        #expect(harness.messages().first?.state == .pending)

        await harness.coordinator.deliver(localID: localID) // the retry

        let posts = StubURLProtocol.requests(host: harness.host)
            .filter { $0.method == "POST" && $0.url.path().hasSuffix("/messages") }
        #expect(posts.count == 2)
        #expect(posts[0].bodyJSON()?["client_msg_id"] as? String == originalClientMsgID)
        #expect(posts[1].bodyJSON()?["client_msg_id"] as? String == originalClientMsgID)

        let rows = harness.messages()
        #expect(rows.count == 1)
        #expect(rows.first?.localID == localID)
        #expect(rows.first?.state == .sent)
        #expect(rows.first?.serverID == 3003)
    }

    // MARK: - Unread bump rules

    @Test("live messages bump unread unless the newest one is in front of the reader")
    func unreadBumpRules() async throws {
        let harness = try makeHarness(host: "send-unread.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // Someone else, chat not on screen → bump.
        harness.coordinator.handle(frame: .message(dto(id: 10, senderID: 9, clientMsgID: "a-1")))
        #expect(harness.chat(42)?.unreadCount == 1)

        harness.coordinator.handle(frame: .message(dto(id: 11, senderID: 9, clientMsgID: "a-2")))
        #expect(harness.chat(42)?.unreadCount == 2)

        // Opening the chat is not reading it: the view claims the chat
        // before its layout has settled, so it publishes "not at the newest
        // message" and the badge survives. This used to clear it, which is
        // how a chat became read by being pushed onto the stack.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: false, isFrontmost: true)
        #expect(harness.chat(42)?.unreadCount == 2)
        #expect(harness.chat(42)?.myLastReadID == 0)

        // Settling at the bottom with the app in front IS reading it.
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 11)

        // New message while it is genuinely on screen → read, not unread.
        harness.coordinator.handle(frame: .message(dto(id: 12, senderID: 9, clientMsgID: "a-3")))
        await harness.coordinator.pendingReadPost?.value
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 12)

        // Own echo (other device) never bumps.
        harness.coordinator.releasePresence(chatID: 42)
        harness.coordinator.handle(frame: .message(dto(id: 13, senderID: 7, clientMsgID: "a-4")))
        #expect(harness.chat(42)?.unreadCount == 0)
    }

    @Test("resync-style upserts (bumpUnread: false) never touch the badge")
    func resyncUpsertsDoNotBump() throws {
        let harness = try makeHarness(host: "send-nobump.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 20, senderID: 9, clientMsgID: "b-1"), bumpUnread: false)
        _ = harness.coordinator.upsert(dto(id: 21, senderID: 9, clientMsgID: "b-2"), bumpUnread: false)

        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.maxServerMessageID == 21)
    }

    @Test("a server error frame answering a send marks the row failed")
    func errorFrameMarksFailed() throws {
        let harness = try makeHarness(host: "send-errframe.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.enqueue(body: "rejected", in: 42)
        let clientMsgID = try #require(harness.messages().first?.clientMsgID)

        harness.coordinator.handle(frame: .error(
            code: "not_chat_member", message: "nope", clientMsgID: clientMsgID, callID: nil))

        #expect(harness.messages().first?.state == .failed)
    }
}

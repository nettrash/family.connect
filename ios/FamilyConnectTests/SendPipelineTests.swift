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

    @Test("a failing REST fallback marks the row failed")
    func failedMarking() async throws {
        let harness = try makeHarness(host: "send-fail.test", handler: { _ in
            .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "doomed", in: 42))
        await harness.coordinator.deliver(localID: localID)

        let row = try #require(harness.messages().first)
        #expect(row.state == .failed)
        #expect(row.serverID == nil)
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

        await harness.coordinator.deliver(localID: localID) // → failed
        #expect(harness.messages().first?.state == .failed)

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

    @Test("live messages bump unread only for other senders in inactive chats")
    func unreadBumpRules() throws {
        let harness = try makeHarness(host: "send-unread.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // Someone else, chat not on screen → bump.
        harness.coordinator.handle(frame: .message(dto(id: 10, senderID: 9, clientMsgID: "a-1")))
        #expect(harness.chat(42)?.unreadCount == 1)

        harness.coordinator.handle(frame: .message(dto(id: 11, senderID: 9, clientMsgID: "a-2")))
        #expect(harness.chat(42)?.unreadCount == 2)

        // Opening the chat clears the badge (and advances the marker).
        harness.coordinator.activeChatID = 42
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 11)

        // New message while on screen → read, not unread.
        harness.coordinator.handle(frame: .message(dto(id: 12, senderID: 9, clientMsgID: "a-3")))
        #expect(harness.chat(42)?.unreadCount == 0)
        #expect(harness.chat(42)?.myLastReadID == 12)

        // Own echo (other device) never bumps.
        harness.coordinator.activeChatID = nil
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
            code: "not_chat_member", message: "nope", clientMsgID: clientMsgID))

        #expect(harness.messages().first?.state == .failed)
    }
}

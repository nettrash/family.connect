//
//  ReactionSyncTests.swift
//  FamilyConnectTests
//
//  The reaction pipeline against an in-memory ModelContainer and a
//  stubbed URLSession, harnessed like SendPipelineTests: the coordinator's
//  ChatSocket is real but never started, live frames are injected through
//  `handle(frame:)` directly, and REST is routed per-host through
//  StubURLProtocol. What matters here is the seq guard (ONE shared apply
//  path plus the embedded-fields guard in upsert), the per-chat cursor
//  that advances even for states we drop, and the optimistic
//  toggle/revert dance.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Reaction sync")
struct ReactionSyncTests {

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

        func message(localID: String) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.localID == localID })
            return (try? context.fetch(descriptor))?.first
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

    private func dto(
        id: Int64,
        senderID: Int64 = 9,
        clientMsgID: String? = nil,
        body: String = "hello",
        reactions: [ReactionDTO]? = nil,
        reactionSeq: Int64? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: clientMsgID,
            body: body, createdAt: Self.serverDate,
            reactions: reactions, reactionSeq: reactionSeq)
    }

    // MARK: - Seq guard

    @Test("a stale seq is a no-op on both apply paths, and absent fields never wipe")
    func staleSeqIsNoOp() throws {
        let harness = try makeHarness(host: "reaction-stale.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        harness.coordinator.applyReactionState(
            messageServerID: 100, seq: 10, reactions: [ReactionDTO(userID: 9, emoji: "❤️")])

        // Stale full-state apply (an out-of-order frame): dropped.
        harness.coordinator.applyReactionState(
            messageServerID: 100, seq: 9, reactions: [ReactionDTO(userID: 9, emoji: "👍")])
        var row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 10)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])

        // Stale embedded fields (a resync page fetched before the live
        // frame landed): the upsert guard drops them too.
        _ = harness.coordinator.upsert(
            dto(id: 100, reactions: [ReactionDTO(userID: 9, emoji: "👍")], reactionSeq: 5),
            bumpUnread: false)
        row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 10)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])

        // A message re-delivery WITHOUT reaction fields (a chat-list
        // preview shape): absence means "no data", never "clear".
        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 10)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])
    }

    @Test("newer embedded fields on a fetched message apply through the upsert guard")
    func embeddedFieldsApply() throws {
        let harness = try makeHarness(host: "reaction-embedded.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, reactions: [ReactionDTO(userID: 9, emoji: "😂")], reactionSeq: 4),
            bumpUnread: false)

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 4)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "😂")])
    }

    // MARK: - Live frames

    @Test("a reaction frame applies full state and MAX-advances the chat cursor")
    func frameAppliesAndBumpsCursor() throws {
        let harness = try makeHarness(host: "reaction-frame.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        harness.coordinator.handle(frame: .reaction(ReactionPayload(
            chatID: 42, messageID: 100, reactionSeq: 10,
            reactions: [ReactionDTO(userID: 9, emoji: "❤️")])))

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 10)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])
        #expect(harness.chat(42)?.maxReactionSeq == 10)

        // The full-state replacement: a later frame REPLACES, not merges.
        harness.coordinator.handle(frame: .reaction(ReactionPayload(
            chatID: 42, messageID: 100, reactionSeq: 11, reactions: [])))
        #expect(harness.message(localID: "s:100")?.reactionList == [])
        #expect(harness.chat(42)?.maxReactionSeq == 11)
    }

    @Test("a frame for a message we don't hold still advances the cursor, silently")
    func frameForUnknownMessageBumpsCursorOnly() throws {
        let harness = try makeHarness(host: "reaction-unknown.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .reaction(ReactionPayload(
            chatID: 42, messageID: 999, reactionSeq: 12,
            reactions: [ReactionDTO(userID: 9, emoji: "👍")])))

        #expect(harness.messages().isEmpty)
        #expect(harness.chat(42)?.maxReactionSeq == 12)
    }

    @Test("a reaction to OUR message lands on its c:-keyed row via the serverID")
    func frameForOwnMessageAppliesToClientKeyedRow() throws {
        let harness = try makeHarness(host: "reaction-own.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        // Our optimistic row gains its serverID through the ack but keeps
        // its "c:" identity — the reaction must still find it.
        let localID = try #require(harness.coordinator.enqueue(body: "hello", in: 42))
        let clientMsgID = try #require(harness.messages().first?.clientMsgID)
        harness.coordinator.handle(frame: .ack(
            clientMsgID: clientMsgID,
            message: dto(id: 1338, senderID: 7, clientMsgID: clientMsgID)))

        harness.coordinator.handle(frame: .reaction(ReactionPayload(
            chatID: 42, messageID: 1338, reactionSeq: 3,
            reactions: [ReactionDTO(userID: 9, emoji: "❤️")])))

        let row = try #require(harness.message(localID: localID))
        #expect(row.reactionSeq == 3)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])
    }

    // MARK: - Toggle

    @Test("toggling a new emoji PUTs it and applies the authoritative response")
    func togglePutPath() async throws {
        let harness = try makeHarness(host: "reaction-put.test", handler: { request in
            guard request.method == "PUT", request.url.path().hasSuffix("/reaction") else {
                return .empty(204)
            }
            // The server merges ours with an existing reaction from user 9.
            return .json(200, """
            {"message_id": 100, "reaction_seq": 11,
             "reactions": [{"user_id": 9, "emoji": "👍"}, {"user_id": 7, "emoji": "❤️"}]}
            """)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, reactions: [ReactionDTO(userID: 9, emoji: "👍")], reactionSeq: 5),
            bumpUnread: false)
        await harness.coordinator.toggleReaction(localID: "s:100", emoji: "❤️")

        let puts = StubURLProtocol.requests(host: harness.host).filter { $0.method == "PUT" }
        #expect(puts.count == 1)
        #expect(puts.first?.url.path() == "/api/v1/chats/42/messages/100/reaction")
        #expect(puts.first?.bodyJSON()?["emoji"] as? String == "❤️")

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 11)
        #expect(row.reactionList == [
            ReactionSnapshot(userID: 9, emoji: "👍"),
            ReactionSnapshot(userID: 7, emoji: "❤️"),
        ])
    }

    @Test("toggling my current emoji DELETEs it")
    func toggleDeletePath() async throws {
        let harness = try makeHarness(host: "reaction-delete.test", handler: { request in
            guard request.method == "DELETE", request.url.path().hasSuffix("/reaction") else {
                return .empty(204)
            }
            return .json(200, #"{"message_id": 100, "reaction_seq": 6, "reactions": []}"#)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, reactions: [ReactionDTO(userID: 7, emoji: "❤️")], reactionSeq: 5),
            bumpUnread: false)
        await harness.coordinator.toggleReaction(localID: "s:100", emoji: "❤️")

        let deletes = StubURLProtocol.requests(host: harness.host).filter { $0.method == "DELETE" }
        #expect(deletes.count == 1)
        #expect(deletes.first?.url.path() == "/api/v1/chats/42/messages/100/reaction")

        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 6)
        #expect(row.reactionList == [])
    }

    @Test("a failed toggle reverts the optimistic change")
    func toggleRevertsOnError() async throws {
        let harness = try makeHarness(host: "reaction-revert.test", handler: { _ in
            .json(500, #"{"error": {"code": "internal", "message": "boom"}}"#)
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(
            dto(id: 100, reactions: [ReactionDTO(userID: 9, emoji: "👍")], reactionSeq: 5),
            bumpUnread: false)
        await harness.coordinator.toggleReaction(localID: "s:100", emoji: "❤️")

        // Back to the pre-toggle state: user 9's 👍 only, seq untouched.
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 5)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "👍")])
    }

    @Test("toggle refuses a message without a serverID")
    func toggleNeedsServerID() async throws {
        let harness = try makeHarness(host: "reaction-noserver.test", handler: { _ in .empty(204) })
        defer { harness.tearDown() }

        let localID = try #require(harness.coordinator.enqueue(body: "pending", in: 42))
        await harness.coordinator.toggleReaction(localID: localID, emoji: "❤️")

        #expect(StubURLProtocol.requests(host: harness.host).isEmpty)
        #expect(harness.message(localID: localID)?.reactionsJSON == nil)
    }

    // MARK: - Resync catch-up

    @Test("resync repairs a missed reaction and advances the cursor past unknown messages")
    func resyncReactionCatchUp() async throws {
        let harness = try makeHarness(host: "reaction-resync.test", handler: { request in
            switch request.url.path() {
            case "/api/v1/me":
                return .json(200, """
                {"user": {"id": 7, "username": "anna", "display_name": "Anna",
                          "created_at": "2026-08-19T17:00:00Z"},
                 "family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:00:00Z"},
                 "role": "member"}
                """)
            case "/api/v1/families/mine":
                return .json(200, """
                {"family": {"id": 3, "name": "The Smiths", "join_policy": "open",
                            "created_at": "2026-08-19T17:00:00Z"},
                 "members": [{"id": 7, "username": "anna", "display_name": "Anna", "role": "member"}]}
                """)
            case "/api/v1/chats":
                // last_message id 100 == the local cursor, so no message
                // catch-up runs; max_reaction_seq 12 > local 0 triggers
                // the reaction catch-up.
                return .json(200, """
                {"chats": [{"chat": {"id": 42, "kind": "family", "title": "The Smiths",
                                     "peer_user_id": null},
                            "last_message": {"id": 100, "chat_id": 42, "sender_id": 9,
                                             "client_msg_id": null, "body": "hello",
                                             "created_at": "2026-08-19T17:05:00Z"},
                            "unread_count": 0, "max_reaction_seq": 12}]}
                """)
            case "/api/v1/chats/42/reactions":
                // One state for a held message, one for a message this
                // client never loaded (deep history) — the latter is
                // dropped but its seq still moves the cursor.
                return .json(200, """
                {"message_reactions": [
                  {"message_id": 100, "reaction_seq": 11,
                   "reactions": [{"user_id": 9, "emoji": "❤️"}]},
                  {"message_id": 999, "reaction_seq": 12,
                   "reactions": [{"user_id": 9, "emoji": "👍"}]}]}
                """)
            default:
                return .json(404, #"{"error": {"code": "chat_not_found", "message": "?"}}"#)
            }
        })
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100), bumpUnread: false)
        await harness.coordinator.resync()

        // The held message got its missed reaction…
        let row = try #require(harness.message(localID: "s:100"))
        #expect(row.reactionSeq == 11)
        #expect(row.reactionList == [ReactionSnapshot(userID: 9, emoji: "❤️")])

        // …the unknown one was dropped without inventing a row…
        #expect(harness.message(localID: "s:999") == nil)

        // …and the cursor covers the whole page, so the next resync
        // (server max still 12) plans no reaction step.
        #expect(harness.chat(42)?.maxReactionSeq == 12)

        let reactionGets = StubURLProtocol.requests(host: harness.host)
            .filter { $0.url.path() == "/api/v1/chats/42/reactions" }
        #expect(reactionGets.count == 1)
        #expect(reactionGets.first?.url.query()?.contains("after_seq=0") == true)
    }
}

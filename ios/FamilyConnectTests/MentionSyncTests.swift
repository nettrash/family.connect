//
//  MentionSyncTests.swift
//  FamilyConnectTests
//
//  The "@" mark on a chat row, and the list on a pending row (docs/protocol.md,
//  "Mentioning a member"). The mark has the four writers the unread count
//  has, and a fifth that must NOT write it: a page.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Mention sync")
struct MentionSyncTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!

    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        func message(localID: String) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(predicate: #Predicate { $0.localID == localID })
            return (try? context.fetch(descriptor))?.first
        }

        func settle() async {
            await coordinator.pendingDelivery?.value
        }

        func tearDown() {
            StubURLProtocol.unregister(host: host)
        }
    }

    private func makeHarness(
        host: String,
        handler: @escaping StubURLProtocol.Handler = { _ in .json(200, #"{"messages": []}"#) }
    ) throws -> Harness {
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
        coordinator.ackTimeout = 0.2
        let chat = ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(container: container, coordinator: coordinator, context: container.mainContext, host: host)
    }

    private func dto(id: Int64, senderID: Int64 = 9, mentions: [MentionDTO]? = nil) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: senderID, clientMsgID: nil,
            body: "@Me are you in?", createdAt: Self.serverDate, mentions: mentions)
    }

    private func readFrame(userID: Int64, upTo: Int64) throws -> ServerFrame {
        try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "read", "chat_id": 42, "user_id": \#(userID), "last_read_message_id": \#(upTo)}
        """#.utf8))
    }

    @Test("a live frame naming me marks the row; naming somebody else does not")
    func liveFrameMarks() throws {
        let harness = try makeHarness(host: "mention-sync-live.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100, mentions: [MentionDTO(userID: 8, name: "Anna")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.unreadCount == 1)
        #expect(harness.chat(42)?.hasUnreadMention == false, "somebody else was named")
        _ = harness.coordinator.upsert(dto(id: 101, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.unreadCount == 2)
        #expect(harness.chat(42)?.hasUnreadMention == true)
    }

    @Test("my own message naming me marks nothing, and neither does a page")
    func ownAndPagesDoNot() throws {
        let harness = try makeHarness(host: "mention-sync-own.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100, senderID: 7, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.hasUnreadMention == false)
        // A history page naming me: the server's `mentioned` is the
        // authority there, not the page.
        _ = harness.coordinator.upsert(dto(id: 101, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: false)
        #expect(harness.chat(42)?.hasUnreadMention == false)
        #expect(harness.message(localID: "s:101")?.mentionList == [MentionDTO(userID: 7, name: "Me")], "the list itself is stored")
    }

    @Test("reading clears the mark, here and from another device")
    func readingClears() throws {
        let harness = try makeHarness(host: "mention-sync-read.test")
        defer { harness.tearDown() }
        _ = harness.coordinator.upsert(dto(id: 100, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.hasUnreadMention == true)
        harness.coordinator.markRead(chatID: 42)
        #expect(harness.chat(42)?.hasUnreadMention == false)
        #expect(harness.chat(42)?.unreadCount == 0)

        // Again, cleared this time by my read from another device — at
        // zero, because the mark is a filter over the unread rows.
        _ = harness.coordinator.upsert(dto(id: 101, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.hasUnreadMention == true)
        harness.coordinator.handle(frame: try readFrame(userID: 7, upTo: 100))
        #expect(harness.chat(42)?.hasUnreadMention == true, "a partial read leaves the naming message unread")
        harness.coordinator.handle(frame: try readFrame(userID: 7, upTo: 101))
        #expect(harness.chat(42)?.hasUnreadMention == false)
    }

    @Test("a live frame from a blocked member never marks the row")
    func blockedSenderNeverMarks() throws {
        let harness = try makeHarness(host: "mention-sync-blocked.test")
        defer { harness.tearDown() }
        harness.coordinator.replaceBlocks(with: [9])
        // The frame ARRIVES — a blocked member's message is not suppressed
        // in the family chat, it is drawn as a hidden row — and it counts.
        _ = harness.coordinator.upsert(
            dto(id: 100, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.unreadCount == 1, "the count is not narrowed by a block")
        #expect(
            harness.chat(42)?.hasUnreadMention == false,
            "the mark is the server's filter: an unread row from somebody NOT blocked")
        // Somebody else naming me still marks it.
        _ = harness.coordinator.upsert(
            dto(id: 101, senderID: 8, mentions: [MentionDTO(userID: 7, name: "Me")]),
            bumpUnread: true, live: true)
        #expect(harness.chat(42)?.hasUnreadMention == true)
    }

    @Test("a live frame naming me while I am looking marks nothing")
    func lookingMarksNothing() throws {
        let harness = try makeHarness(host: "mention-sync-looking.test")
        defer { harness.tearDown() }
        harness.coordinator.updatePresence(chatID: 42, isAtNewest: true, isFrontmost: true)
        #expect(harness.coordinator.isReading(42))
        _ = harness.coordinator.upsert(
            dto(id: 100, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        #expect(harness.chat(42)?.unreadCount == 0, "read as it lands")
        #expect(
            harness.chat(42)?.hasUnreadMention == false,
            "the mark is a filter over UNREAD rows, and this one never was")
    }

    @Test("GET /chats is the authority on the mark, and an absent field means no")
    func theListOverwritesTheMark() throws {
        let harness = try makeHarness(host: "mention-sync-overwrite.test")
        defer { harness.tearDown() }
        func item(mentioned: Bool?) throws -> ChatListItemDTO {
            let mentionedField = mentioned.map { ", \"mentioned\": \($0)" } ?? ""
            return try APICoding.decoder().decode(ChatListItemDTO.self, from: Data("""
            {"chat": {"id": 42, "kind": "family", "title": "The Smiths", "peer_user_id": null},
             "last_message": {"id": 100, "chat_id": 42, "sender_id": 9, "client_msg_id": null,
                              "body": "@Me?", "created_at": "2026-08-19T17:05:00Z"},
             "unread_count": 2\(mentionedField)}
            """.utf8))
        }
        _ = harness.coordinator.upsertChat(try item(mentioned: true))
        #expect(harness.chat(42)?.hasUnreadMention == true)
        // Absent means NO, and it must WIPE a stale mark — a chat read on
        // another device is exactly the case.
        _ = harness.coordinator.upsertChat(try item(mentioned: nil))
        #expect(harness.chat(42)?.hasUnreadMention == false)
        _ = harness.coordinator.upsertChat(try item(mentioned: true))
        #expect(harness.chat(42)?.hasUnreadMention == true)
        _ = harness.coordinator.upsertChat(try item(mentioned: false))
        #expect(harness.chat(42)?.hasUnreadMention == false)
    }

    @Test("a mention the server had not counted keeps its mark through the overwrite")
    func theMarkSurvivesTheRace() throws {
        let harness = try makeHarness(host: "mention-sync-race.test")
        defer { harness.tearDown() }
        // The response was built with `last_message` 100; 101 raced it.
        _ = harness.coordinator.upsert(
            dto(id: 101, mentions: [MentionDTO(userID: 7, name: "Me")]), bumpUnread: true, live: true)
        let item = try APICoding.decoder().decode(ChatListItemDTO.self, from: Data("""
        {"chat": {"id": 42, "kind": "family", "title": "The Smiths", "peer_user_id": null},
         "last_message": {"id": 100, "chat_id": 42, "sender_id": 9, "client_msg_id": null,
                          "body": "hello", "created_at": "2026-08-19T17:05:00Z"},
         "unread_count": 0}
        """.utf8))
        _ = harness.coordinator.upsertChat(
            item,
            uncountedLiveMessages: harness.coordinator.uncountedBumpsForTesting(in: item),
            uncountedMention: harness.coordinator.uncountedMentionForTesting(in: item))
        #expect(harness.chat(42)?.unreadCount == 1, "the count keeps the raced message")
        #expect(
            harness.chat(42)?.hasUnreadMention == true,
            "and so does the mark: the two cannot drift")
    }

    @Test("a location's caption names members, like every other body")
    func locationCaptionNamesMembers() throws {
        let harness = try makeHarness(host: "mention-sync-location.test")
        defer { harness.tearDown() }
        let localID = try #require(harness.coordinator.sendLocation(
            latitude: 44.8, longitude: 20.4, accuracyM: 12, label: nil,
            caption: "@Anna we are here", replyTo: nil,
            mentions: [MentionDTO(userID: 8, name: "Anna")], in: 42))
        #expect(harness.message(localID: localID)?.mentionList == [MentionDTO(userID: 8, name: "Anna")])
    }

    @Test("a poll's question names members, like every other body")
    func pollQuestionNamesMembers() throws {
        let harness = try makeHarness(host: "mention-sync-poll.test")
        defer { harness.tearDown() }
        let localID = try #require(harness.coordinator.sendPoll(
            question: "@Anna pizza or pasta?", options: ["Pizza", "Pasta"], in: 42,
            mentions: [MentionDTO(userID: 8, name: "Anna")]))
        #expect(harness.message(localID: localID)?.mentionList == [MentionDTO(userID: 8, name: "Anna")])
    }

    @Test("the Mac's own banner says what the phone's push says")
    func macBannerSaysMentionedYou() {
        #expect(
            ChatNotifier.title(
                chatKind: "family", chatTitle: "The Smiths", senderName: "Anna", namesMe: true)
                == "The Smiths — Anna mentioned you",
            "mirrors push_payload::mention_notification")
        #expect(
            ChatNotifier.title(chatKind: "family", chatTitle: "The Smiths", senderName: "Anna")
                == "The Smiths — Anna")
        #expect(
            ChatNotifier.title(chatKind: "direct", chatTitle: "Anna", senderName: "Anna", namesMe: true)
                == "Anna",
            "a direct chat has no mention title — and no mentions at all")
    }

    @Test("a pending row keeps its list, and the REST leg sends it")
    func pendingRowCarriesTheList() async throws {
        var captured: [String: Any]?
        let harness = try makeHarness(host: "mention-sync-send.test") { request in
            if request.method == "POST", request.url.path.hasSuffix("/messages") {
                captured = request.bodyJSON()
                return .json(201, """
                    {"message": {"id": 200, "chat_id": 42, "sender_id": 7,
                                 "client_msg_id": "\(captured?["client_msg_id"] as? String ?? "")",
                                 "body": "@Anna?", "created_at": "2026-08-19T17:03:12Z",
                                 "mentions": [{"user_id": 8, "name": "Anna"}]}}
                    """)
            }
            return .json(200, #"{"messages": []}"#)
        }
        defer { harness.tearDown() }
        let localID = try #require(harness.coordinator.send(
            body: "@Anna?", in: 42, mentions: [MentionDTO(userID: 8, name: "Anna")]))
        #expect(harness.message(localID: localID)?.mentionList == [MentionDTO(userID: 8, name: "Anna")])
        await harness.settle()
        let list = try #require(captured?["mentions"] as? [[String: Any]])
        #expect(list.first?["user_id"] as? Int == 8)
        #expect(harness.message(localID: localID)?.serverID == 200)
    }
}

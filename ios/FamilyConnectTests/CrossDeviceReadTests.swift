//
//  CrossDeviceReadTests.swift
//  FamilyConnectTests
//
//  A read on one of this person's devices clears the badge on the others
//  (docs/protocol.md, WebSocket "Semantics").
//
//  The read marker has always been per-USER — one `chat_reads` row per
//  (chat, user), monotonic, reported by `GET /chats` as
//  `last_read_message_id` — so this device was always entitled to the fact.
//  What it lacked was delivery: the server excluded the reader from its own
//  `read` relay, so a member who read on their laptop watched the badge on
//  their phone sit there until something else made it resync.
//
//  This client's receiving branch was written at the same time as Android's
//  and could not fire. It raised `myLastReadID` and stopped there, which is
//  the half that shows nothing: the badge is drawn from `unreadCount`, and
//  nothing recounted it. These tests pin the recount, and they pin it as a
//  RECOUNT rather than a reset — a frame from another device may name a
//  marker part-way up the chat, where `markRead`'s "set it to zero" is
//  simply wrong.
//
//  Android counterpart: ChatRepository.applyMyReadMarker.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
struct CrossDeviceReadTests {

    private static let me: Int64 = 7
    private static let them: Int64 = 11
    private static let chatID: Int64 = 42
    private static let sentAt = Date(timeIntervalSince1970: 1_700_000_000)

    /// A direct chat holding four inbound messages (ids 101-104) with all
    /// four unread, and one of my own (105) which is never unread to me.
    private func makeCoordinator() throws -> (ChatSyncCoordinator, ModelContainer) {
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            PendingMediaItemEntity.self,
            configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let coordinator = ChatSyncCoordinator(modelContainer: container)
        coordinator.currentUserIDOverride = Self.me

        let context = container.mainContext
        context.insert(ChatEntity(
            chatID: Self.chatID, kind: "direct", pinRank: 1, peerUserID: Self.them,
            title: "Gran", unreadCount: 4))
        for id in 101...104 {
            context.insert(MessageEntity(
                localID: "s:\(id)", serverID: Int64(id), chatID: Self.chatID,
                senderID: Self.them, body: "hello", createdAt: Self.sentAt, status: .sent))
        }
        context.insert(MessageEntity(
            localID: "s:105", serverID: 105, chatID: Self.chatID,
            senderID: Self.me, body: "mine", createdAt: Self.sentAt, status: .sent))
        try context.save()
        return (coordinator, container)
    }

    private func readFrame(userID: Int64, upTo: Int64) throws -> ServerFrame {
        try APICoding.decoder().decode(ServerFrame.self, from: Data(#"""
        {"type": "read", "chat_id": \#(Self.chatID),
         "user_id": \#(userID), "last_read_message_id": \#(upTo)}
        """#.utf8))
    }

    private func chat(_ container: ModelContainer) throws -> ChatEntity {
        let chats = try container.mainContext.fetch(FetchDescriptor<ChatEntity>())
        return try #require(chats.first { $0.chatID == Self.chatID })
    }

    /// The whole feature: my other device read everything, so this one's
    /// badge goes to nothing.
    @Test("my own read from another device clears this device's count")
    func myReadClearsTheCount() throws {
        let (coordinator, container) = try makeCoordinator()

        coordinator.handle(frame: try readFrame(userID: Self.me, upTo: 104))

        let chat = try chat(container)
        #expect(chat.myLastReadID == 104)
        #expect(chat.unreadCount == 0)
    }

    /// A RECOUNT and not a reset. Another device may have read part of the
    /// way up — treating any read as "all read" would hide messages this
    /// person has not seen on any device, which is the one failure a read
    /// marker must never produce.
    @Test("a partial read leaves the messages above it unread")
    func partialReadRecounts() throws {
        let (coordinator, container) = try makeCoordinator()

        coordinator.handle(frame: try readFrame(userID: Self.me, upTo: 102))

        let chat = try chat(container)
        #expect(chat.myLastReadID == 102)
        // 103 and 104 are still unread; 105 is mine and never was.
        #expect(chat.unreadCount == 2)
    }

    /// `max(stored, received)`, and the recount follows the marker that
    /// WINS. A `GET /chats` in flight while the reader is reading carries a
    /// stale marker, and applying it must not resurrect what they have read.
    @Test("a stale marker moves nothing, and cannot resurrect a read message")
    func staleMarkerIsIgnored() throws {
        let (coordinator, container) = try makeCoordinator()

        coordinator.handle(frame: try readFrame(userID: Self.me, upTo: 104))
        coordinator.handle(frame: try readFrame(userID: Self.me, upTo: 101))

        let chat = try chat(container)
        #expect(chat.myLastReadID == 104, "the marker is monotonic")
        #expect(chat.unreadCount == 0, "recounted against the marker that won, not the one that arrived")
    }

    /// Somebody ELSE's read is a fact about the roster and says nothing
    /// about what this reader has seen. Touching the count here would clear
    /// a badge because the other person looked at their phone.
    @Test("another member's read does not touch my count")
    func theirReadLeavesMyCountAlone() throws {
        let (coordinator, container) = try makeCoordinator()

        coordinator.handle(frame: try readFrame(userID: Self.them, upTo: 104))

        let chat = try chat(container)
        #expect(chat.othersReadUpTo == 104)
        #expect(chat.myLastReadID == 0)
        #expect(chat.unreadCount == 4, "their read is not mine")
    }

    /// My own messages are not unread to me, so a marker below them still
    /// leaves nothing to read.
    @Test("my own messages are never counted as unread")
    func ownMessagesDoNotCount() throws {
        let (coordinator, container) = try makeCoordinator()

        coordinator.handle(frame: try readFrame(userID: Self.me, upTo: 104))

        let chat = try chat(container)
        // 105 is above the marker and is mine.
        #expect(chat.unreadCount == 0)
    }
}

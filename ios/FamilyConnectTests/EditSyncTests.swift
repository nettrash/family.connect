//
//  EditSyncTests.swift
//  FamilyConnectTests
//
//  The edit apply guard, harnessed like ReactionSyncTests: in-memory
//  ModelContainer, stubbed URLSession, live frames injected through
//  `handle(frame:)`.
//
//  The guard is the whole point. Message deliveries are not ordered — a
//  history page fetched BEFORE an edit can arrive after the frame carrying
//  it — so an unguarded body write silently restores the old text on one
//  device and not another, and the family disagrees about what was said.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Edit sync")
struct EditSyncTests {

    private static let serverDate = ISO8601DateFormatter().date(from: "2026-08-19T17:05:00Z")!
    private static let editDate = ISO8601DateFormatter().date(from: "2026-08-19T17:30:00Z")!

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func message(serverID: Int64) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(
                predicate: #Predicate { $0.serverID == serverID })
            return (try? context.fetch(descriptor))?.first
        }

        func chat(_ chatID: Int64) -> ChatEntity? {
            let descriptor = FetchDescriptor<ChatEntity>(predicate: #Predicate { $0.chatID == chatID })
            return (try? context.fetch(descriptor))?.first
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(host: String) throws -> Harness {
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        let chat = ChatEntity(chatID: 42, kind: "family", pinRank: 0, title: "The Smiths")
        container.mainContext.insert(chat)
        try container.mainContext.save()
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    private func dto(
        id: Int64,
        body: String,
        editSeq: Int64? = nil,
        editedAt: Date? = nil,
        replyTo: ReplyToDTO? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: 9, clientMsgID: nil,
            body: body, createdAt: Self.serverDate,
            replyTo: replyTo, editedAt: editedAt, editSeq: editSeq)
    }

    // MARK: - The guard

    @Test("an edit replaces the body and marks the message edited")
    func editApplies() throws {
        let harness = try makeHarness(host: "edit-apply.test")
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 100, body: "Dinner at 8?", editSeq: 5, editedAt: Self.editDate)))

        let row = try #require(harness.message(serverID: 100))
        #expect(row.body == "Dinner at 8?")
        #expect(row.editSeq == 5)
        #expect(row.editedAt == Self.editDate)
        // And the chat's cursor moved, so a later catch-up will not replay it.
        #expect(harness.chat(42)?.maxEditSeq == 5)
    }

    /// The failure this guard exists for: a page fetched before the edit,
    /// delivered after it, must NOT put the old text back.
    @Test("a stale body never overwrites a newer one")
    func staleBodyIsDropped() throws {
        let harness = try makeHarness(host: "edit-stale.test")
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 100, body: "Dinner at 8?", editSeq: 5, editedAt: Self.editDate)))

        // A history page that predates the edit: no seq at all.
        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.body == "Dinner at 8?")

        // …and an older edit, out of order.
        _ = harness.coordinator.upsert(
            dto(id: 100, body: "Dinner at 7:30?", editSeq: 3), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.body == "Dinner at 8?")
        #expect(harness.message(serverID: 100)?.editSeq == 5)
    }

    @Test("a newer edit wins, and re-delivery of the same seq is harmless")
    func newerEditWins() throws {
        let harness = try makeHarness(host: "edit-newer.test")
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "one", editSeq: 5), bumpUnread: false)
        _ = harness.coordinator.upsert(dto(id: 100, body: "two", editSeq: 9), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.body == "two")

        // Same seq again (a re-delivered frame): idempotent, not a revert.
        _ = harness.coordinator.upsert(dto(id: 100, body: "two", editSeq: 9), bumpUnread: false)
        #expect(harness.message(serverID: 100)?.body == "two")
        #expect(harness.message(serverID: 100)?.editSeq == 9)
    }

    /// A quote is a snapshot of the body, so a reply this device already
    /// holds goes stale the moment the quoted message is rewritten.
    @Test("editing a quoted message refreshes local replies' excerpts")
    func editRefreshesQuotes() throws {
        let harness = try makeHarness(host: "edit-quotes.test")
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        _ = harness.coordinator.upsert(
            dto(
                id: 101,
                body: "Works for me",
                replyTo: ReplyToDTO(messageID: 100, senderID: 9, excerpt: "Dinner at 7?")),
            bumpUnread: false)

        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 100, body: "Dinner at 8?", editSeq: 5, editedAt: Self.editDate)))

        #expect(harness.message(serverID: 101)?.replyExcerpt == "Dinner at 8?")
    }

    /// An edit is not new mail: it must never raise an unread count, even
    /// for a chat the reader is not looking at.
    @Test("an edit does not bump unread")
    func editDoesNotBumpUnread() throws {
        let harness = try makeHarness(host: "edit-unread.test")
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        let before = harness.chat(42)?.unreadCount ?? 0

        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 100, body: "Dinner at 8?", editSeq: 5, editedAt: Self.editDate)))

        #expect(harness.chat(42)?.unreadCount == before)
    }
}

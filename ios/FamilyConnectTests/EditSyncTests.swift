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

    private func makeHarness(
        host: String,
        handler: @escaping StubURLProtocol.Handler = { _ in .empty(204) }
    ) throws -> Harness {
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
        replyTo: ReplyToDTO? = nil,
        attachments: [AttachmentDTO]? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: 9, clientMsgID: nil,
            body: body, createdAt: Self.serverDate,
            replyTo: replyTo, editedAt: editedAt, editSeq: editSeq,
            attachments: attachments)
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

    // MARK: - The chat-wide cursor

    /// protocol.md, "Best-effort delivery": **only a live frame and a
    /// catch-up page may move a chat cursor.** A state that arrives by any
    /// other route — embedded on a fetched `Message`, or in the HTTP reply
    /// to this client's own change — is applied under the per-MESSAGE guard
    /// and must leave the chat-wide watermark alone. The doc spells the rule
    /// out for reactions and polls (which `vote`, `closePoll` and
    /// `toggleReaction` already follow); it holds for the third cursor for
    /// exactly the same reason, and `edit` used to break it.
    @Test("my own edit does not advance the chat's edit cursor")
    func ownEditLeavesTheChatCursorAlone() async throws {
        // What the server answers a PATCH with: this message, at seq 100.
        let edited = """
            {"message": {"id": 100, "chat_id": 42, "sender_id": 7,
                         "body": "Dinner at 8?",
                         "created_at": "2026-08-19T17:05:00Z",
                         "edited_at": "2026-08-19T17:30:00Z",
                         "edit_seq": 100}}
            """
        let harness = try makeHarness(host: "edit-own-cursor.test") { request in
            request.method == "PATCH" ? .json(200, edited) : .empty(204)
        }
        defer { harness.tearDown() }

        _ = harness.coordinator.upsert(dto(id: 100, body: "Dinner at 7?"), bumpUnread: false)
        // Somebody ELSE rewrote a different message at seq 99 while this
        // socket was down. This device never saw the frame, so its cursor is
        // still behind it — which is the ordinary state, not a contrived one:
        // REST goes on working precisely while the socket does not.
        #expect(harness.chat(42)?.maxEditSeq == 0)

        let ok = await harness.coordinator.edit(
            messageServerID: 100, in: 42, body: "Dinner at 8?")
        #expect(ok)

        // The ROW takes the change, under its own seq guard.
        #expect(harness.message(serverID: 100)?.body == "Dinner at 8?")
        #expect(harness.message(serverID: 100)?.editSeq == 100)
        // The CHAT-WIDE cursor does not move. It used to jump straight to
        // 100, and the next resync — comparing the server's `max_edit_seq`
        // against a cursor already at 100 — then asked for nothing at all.
        // The edit at 99 was lost until that message next changed.
        #expect(harness.chat(42)?.maxEditSeq == 0)
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
    // MARK: - The one edit that changes more than the body

    /// The generated picture, arriving the only way it can.
    ///
    /// A `message_edited` may ADD attachments to a message that had none,
    /// and the assistant's own reply is the only message for which it ever
    /// does (docs/protocol.md, "Editing" and "How a picture comes back").
    /// So the whole `Message` is applied, not the body alone — which is the
    /// rule this feed has always carried whole objects FOR, and the reason
    /// protocol.md states it generally rather than only under "Pictures".
    ///
    /// A client that merged only the body is not broken by it — the
    /// attachment is on the row, so its next history page or cold start
    /// draws the picture — but late is not the same as live, and a bubble
    /// that fills in a minute after the answer arrived reads as a bug.
    @Test("an edit that adds an attachment puts the picture on the row")
    func editAddsTheGeneratedPicture() throws {
        let harness = try makeHarness(host: "edit-adds-picture.test")
        defer { harness.tearDown() }

        // Step 2 of the answer: an EMPTY row, stored and fanned out so a
        // bubble appears at once.
        harness.coordinator.handle(frame: .message(dto(id: 1339, body: "")))
        #expect(harness.message(serverID: 1339)?.attachmentList.isEmpty == true)

        // Step 4: the picture lands as an ordinary photo attachment on that
        // same message, under a real edit_seq. The body STAYS EMPTY — a
        // photo needs no caption, and the member's own words handed back to
        // them are not one.
        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 1339, body: "", editSeq: 12, editedAt: Self.editDate,
                attachments: [Self.generatedPicture])))

        let row = try #require(harness.message(serverID: 1339))
        #expect(row.body.isEmpty)
        #expect(row.editSeq == 12)
        let drawn = row.attachmentList
        #expect(drawn.count == 1)
        #expect(drawn.first?.id == 77)
        #expect(drawn.first?.kind == AttachmentDTO.Kind.photo)
        // No preview: the server generates none, here as everywhere, so
        // clients draw it from its full bytes.
        #expect(drawn.first?.hasPreview == false)
    }

    /// The same picture, reached the other way: a device that was asleep
    /// for the whole answer and catches up through `GET /chats/{id}/edits`.
    /// That path applies each row through the ordinary upsert, so it must
    /// gain the attachment too — "late, not lost" is only true if the late
    /// path actually carries it.
    @Test("the edits catch-up carries the picture too")
    func catchUpCarriesThePicture() throws {
        let harness = try makeHarness(host: "edit-catchup-picture.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(dto(id: 1339, body: "")))
        _ = harness.coordinator.upsert(
            dto(id: 1339, body: "", editSeq: 12, editedAt: Self.editDate,
                attachments: [Self.generatedPicture]),
            bumpUnread: false)

        #expect(harness.message(serverID: 1339)?.attachmentList.count == 1)
    }

    /// The guard that keeps the other direction safe: a history page
    /// fetched BEFORE the picture landed, delivered after it, carries no
    /// attachments — and an absent field must never wipe stored state.
    /// Without this rule the bubble would flicker the picture away on the
    /// next resync.
    @Test("a page that predates the picture does not wipe it")
    func stalePageKeepsThePicture() throws {
        let harness = try makeHarness(host: "edit-picture-stale.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(dto(id: 1339, body: "")))
        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 1339, body: "", editSeq: 12, editedAt: Self.editDate,
                attachments: [Self.generatedPicture])))
        // The stale copy: the empty row as it looked before the edit.
        _ = harness.coordinator.upsert(dto(id: 1339, body: ""), bumpUnread: false)

        #expect(harness.message(serverID: 1339)?.attachmentList.count == 1)
    }

    /// The chat list has to catch up with the answer as well, or the row
    /// keeps saying the assistant sent an empty message.
    @Test("the chat-list preview follows the picture in")
    func previewFollowsThePicture() throws {
        let harness = try makeHarness(host: "edit-picture-preview.test")
        defer { harness.tearDown() }

        harness.coordinator.handle(frame: .message(dto(id: 1339, body: "")))
        harness.coordinator.handle(frame: .messageEdited(
            dto(id: 1339, body: "", editSeq: 12, editedAt: Self.editDate,
                attachments: [Self.generatedPicture])))

        #expect(harness.chat(42)?.lastMessagePreview == "Photo")
    }

    /// The picture the assistant made: an ordinary photo attachment in
    /// every respect but one — it carries no preview, because the server
    /// generates none, so a client draws it from its full bytes.
    private static let generatedPicture = AttachmentDTO(
        id: 77, kind: AttachmentDTO.Kind.photo, mime: "image/png", size: 480_000,
        width: 1024, height: 1024, durationMS: nil, hasPreview: false, name: nil,
        latitude: nil, longitude: nil, accuracyM: nil)
}

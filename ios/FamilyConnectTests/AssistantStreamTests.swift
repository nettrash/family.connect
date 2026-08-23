//
//  AssistantStreamTests.swift
//  FamilyConnectTests
//
//  The assistant's reply arrives twice over: as fragments while it is being
//  written, and as one authoritative body when it is done
//  (docs/protocol.md, "The assistant").
//
//  The rule that matters is which one wins. Deltas carry no `edit_seq`, so
//  they must never be able to outlive or overwrite the finished text — and a
//  client that missed every fragment must still end up with exactly the same
//  body as one that saw them all. That is the whole reason the reply is an
//  ordinary message finished through the edit path rather than a bespoke
//  streaming object.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Assistant streaming")
struct AssistantStreamTests {

    private static let stamp = ISO8601DateFormatter().date(from: "2026-08-23T12:00:00Z")!

    @MainActor
    private struct Harness {
        /// HELD, not just used. Letting the container deallocate while the
        /// coordinator still has a context traps inside SwiftData and takes
        /// the whole test process with it — every case in the suite then
        /// reports "failed" at 0.000 s with no message, which is the only
        /// symptom you get.
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func message(_ serverID: Int64) -> MessageEntity? {
            let descriptor = FetchDescriptor<MessageEntity>(
                predicate: #Predicate { $0.serverID == serverID })
            return (try? context.fetch(descriptor))?.first
        }

        func tearDown() { StubURLProtocol.unregister(host: host) }
    }

    private func makeHarness(host: String) throws -> Harness {
        StubURLProtocol.register(host: host, handler: { _ in .empty(204) })
        let configuration = ModelConfiguration(isStoredInMemoryOnly: true)
        let container = try ModelContainer(
            for: ChatEntity.self, MessageEntity.self, MemberEntity.self, NoteEntity.self,
            configurations: configuration)
        let api = APIClient(
            serverURL: URL(string: "https://\(host)")!,
            session: StubURLProtocol.makeSession())
        let coordinator = ChatSyncCoordinator(modelContainer: container, api: api)
        coordinator.currentUserIDOverride = 7
        return Harness(
            container: container, coordinator: coordinator,
            context: container.mainContext, host: host)
    }

    /// The empty row the server creates before it starts generating; the
    /// assistant sends under its own reserved account.
    private func placeholder(
        id: Int64,
        body: String = "",
        editSeq: Int64? = nil
    ) -> MessageDTO {
        MessageDTO(
            id: id, chatID: 42, senderID: 99, clientMsgID: nil,
            body: body, createdAt: Self.stamp,
            editedAt: editSeq == nil ? nil : Self.stamp, editSeq: editSeq)
    }

    @Test("fragments accumulate into the placeholder")
    func deltasAccumulate() throws {
        let harness = try makeHarness(host: "ai-deltas.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        for fragment in ["Sure", " — the ", "answer."] {
            coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: fragment))
        }

        #expect(harness.message(1339)?.body == "Sure — the answer.")
        #expect(coordinator.streamingMessageIDs.contains(1339))
    }

    @Test("the finished body replaces whatever the fragments built")
    func theEditWins() throws {
        let harness = try makeHarness(host: "ai-edit-wins.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: "Sure — the "))

        let finished = placeholder(id: 1339, body: "Sure — the answer.", editSeq: 91)
        coordinator.handle(frame: .messageEdited(finished))

        #expect(harness.message(1339)?.body == "Sure — the answer.")
        // No longer being written, so the bubble stops showing a cursor.
        #expect(!coordinator.streamingMessageIDs.contains(1339))
    }

    /// The point of finishing through the edit path: a device that was
    /// asleep for the whole reply needs no special path to catch up.
    @Test("a client that missed every fragment still ends up correct")
    func missingEveryDeltaIsSurvivable() throws {
        let harness = try makeHarness(host: "ai-missed.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        let finished = placeholder(id: 1339, body: "Sure — the answer.", editSeq: 91)
        coordinator.handle(frame: .messageEdited(finished))

        #expect(harness.message(1339)?.body == "Sure — the answer.")
    }

    @Test("a failed reply keeps what arrived and stops streaming")
    func errorKeepsThePartialAnswer() throws {
        let harness = try makeHarness(host: "ai-error.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        coordinator.handle(frame: .message(placeholder(id: 1339)))
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 1339, text: "Half an "))
        coordinator.handle(frame: .aiError(chatID: 42, messageID: 1339))

        // A partial answer is worth more than a bubble that never resolves.
        #expect(harness.message(1339)?.body == "Half an ")
        #expect(!coordinator.streamingMessageIDs.contains(1339))
    }

    @Test("a fragment for a row this device does not have is ignored")
    func unknownMessageIsIgnored() throws {
        let harness = try makeHarness(host: "ai-unknown.test")
        defer { harness.tearDown() }
        let coordinator = harness.coordinator

        // No crash, no phantom row.
        coordinator.handle(frame: .aiDelta(chatID: 42, messageID: 4242, text: "hello"))
        #expect(harness.message(4242) == nil)
    }
}

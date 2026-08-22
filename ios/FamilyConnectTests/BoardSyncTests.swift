//
//  BoardSyncTests.swift
//  FamilyConnectTests
//
//  The board apply path: the per-note seq guard and tombstone handling,
//  harnessed like ReactionSyncTests.
//
//  Two things here are easy to get wrong and invisible when you do. An
//  out-of-order frame must not undo a newer move — two people dragging the
//  same note is the normal case, not the exotic one. And a tombstone must
//  actually remove the row: it is the only signal a note is gone, and a
//  client that ignores it shows a deleted note forever.
//

import Foundation
import SwiftData
import Testing
@testable import FamilyConnect

@MainActor
@Suite("Board sync")
struct BoardSyncTests {

    private static let stamp = ISO8601DateFormatter().date(from: "2026-08-22T12:00:00Z")!

    @MainActor
    private struct Harness {
        let container: ModelContainer
        let coordinator: ChatSyncCoordinator
        let context: ModelContext
        let host: String

        func notes() -> [NoteEntity] {
            (try? context.fetch(FetchDescriptor<NoteEntity>())) ?? []
        }

        func note(_ id: Int64) -> NoteEntity? {
            let descriptor = FetchDescriptor<NoteEntity>(predicate: #Predicate { $0.noteID == id })
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

    private func note(
        id: Int64,
        text: String = "Milk",
        color: String = "yellow",
        x: Double = 0.2,
        y: Double = 0.3,
        boardSeq: Int64
    ) -> NoteDTO {
        NoteDTO(
            id: id, authorID: 7, text: text, color: color, x: x, y: y,
            createdAt: Self.stamp, updatedAt: Self.stamp, boardSeq: boardSeq, deleted: nil)
    }

    private func tombstone(id: Int64, boardSeq: Int64) -> NoteDTO {
        NoteDTO(
            id: id, authorID: nil, text: nil, color: nil, x: nil, y: nil,
            createdAt: nil, updatedAt: nil, boardSeq: boardSeq, deleted: true)
    }

    @Test("a note is created, then updated in place")
    func createThenUpdate() throws {
        let harness = try makeHarness(host: "board-create.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 10))
        #expect(harness.notes().count == 1)

        harness.coordinator.applyNote(note(id: 1, text: "Oat milk", x: 0.8, boardSeq: 11))
        #expect(harness.notes().count == 1)
        let row = try #require(harness.note(1))
        #expect(row.text == "Oat milk")
        #expect(row.x == 0.8)
        #expect(row.boardSeq == 11)
    }

    /// Two people dragging the same note is ordinary, so an out-of-order
    /// frame must not undo the newer move.
    @Test("a stale seq never undoes a newer move")
    func staleSeqIsDropped() throws {
        let harness = try makeHarness(host: "board-stale.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, x: 0.9, boardSeq: 20))
        harness.coordinator.applyNote(note(id: 1, x: 0.1, boardSeq: 12))

        #expect(harness.note(1)?.x == 0.9)
        #expect(harness.note(1)?.boardSeq == 20)
    }

    @Test("re-delivering the same seq changes nothing")
    func sameSeqIsIdempotent() throws {
        let harness = try makeHarness(host: "board-same.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, x: 0.4, boardSeq: 20))
        harness.coordinator.applyNote(note(id: 1, x: 0.4, boardSeq: 20))

        #expect(harness.notes().count == 1)
        #expect(harness.note(1)?.x == 0.4)
    }

    /// The tombstone is the ONLY signal a note is gone.
    @Test("a tombstone removes the note")
    func tombstoneDeletes() throws {
        let harness = try makeHarness(host: "board-tomb.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 10))
        harness.coordinator.applyNote(tombstone(id: 1, boardSeq: 11))

        #expect(harness.notes().isEmpty)
    }

    /// A tombstone for something never held is not an error — a client that
    /// joined after the note was deleted simply has nothing to remove.
    @Test("a tombstone for an unknown note is harmless")
    func tombstoneForUnknownNote() throws {
        let harness = try makeHarness(host: "board-tomb2.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(tombstone(id: 99, boardSeq: 5))
        #expect(harness.notes().isEmpty)
    }

    /// The guard covers deletion too: an old tombstone arriving after the
    /// note was legitimately re-sent must not remove it.
    @Test("a stale tombstone does not delete a newer note")
    func staleTombstoneIsDropped() throws {
        let harness = try makeHarness(host: "board-tomb3.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 30))
        harness.coordinator.applyNote(tombstone(id: 1, boardSeq: 12))

        #expect(harness.notes().count == 1)
    }

    /// A live note missing its content is a server bug; drawing a blank
    /// sticker would be worse than dropping it.
    @Test("a live note with no content is ignored")
    func contentlessLiveNoteIsIgnored() throws {
        let harness = try makeHarness(host: "board-empty.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(
            NoteDTO(
                id: 1, authorID: nil, text: nil, color: nil, x: nil, y: nil,
                createdAt: nil, updatedAt: nil, boardSeq: 3, deleted: nil))

        #expect(harness.notes().isEmpty)
    }
}

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
//  Size rides the same guard as everything else, with one extra rule: a
//  DTO with no size at all (an older server) is medium, not a dropped
//  note — the field arrived after the board did.
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
            PendingMediaItemEntity.self,
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

    /// `size` nil is what an older server sends — no field at all.
    private func note(
        id: Int64,
        text: String = "Milk",
        color: String = "yellow",
        size: String? = "medium",
        x: Double = 0.2,
        y: Double = 0.3,
        boardSeq: Int64,
        /// nil is what a server from before content seqs sends — no field.
        contentSeq: Int64? = nil,
        /// nil is what a server from before fonts sends — no field.
        font: String? = nil,
        /// nil is what a server from before kinds sends — no field.
        kind: String? = nil,
        attachment: AttachmentDTO? = nil,
        startsAt: Date? = nil,
        endsAt: Date? = nil,
        place: String? = nil,
        rsvps: [RsvpDTO]? = nil,
        mentions: [MentionDTO]? = nil,
        /// nil on every kind but a list; `[]` on a list nothing has been
        /// written into yet (docs/protocol.md, "Board").
        items: [TaskItemDTO]? = nil
    ) -> NoteDTO {
        NoteDTO(
            id: id, authorID: 7, text: text, color: color, size: size, font: font,
            kind: kind, attachment: attachment,
            startsAt: startsAt, endsAt: endsAt, place: place, rsvps: rsvps,
            items: items, mentions: mentions, x: x, y: y,
            createdAt: Self.stamp, updatedAt: Self.stamp, boardSeq: boardSeq,
            contentSeq: contentSeq, deleted: nil)
    }

    private func tombstone(id: Int64, boardSeq: Int64) -> NoteDTO {
        NoteDTO(
            id: id, authorID: nil, text: nil, color: nil, size: nil, font: nil,
            kind: nil, attachment: nil, startsAt: nil, endsAt: nil, place: nil, rsvps: nil,
            items: nil, mentions: nil, x: nil, y: nil,
            createdAt: nil, updatedAt: nil, boardSeq: boardSeq, contentSeq: nil,
            deleted: true)
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
                id: 1, authorID: nil, text: nil, color: nil, size: nil, font: nil,
                kind: nil, attachment: nil, startsAt: nil, endsAt: nil, place: nil, rsvps: nil,
                items: nil, mentions: nil, x: nil, y: nil,
                createdAt: nil, updatedAt: nil, boardSeq: 3, contentSeq: nil,
                deleted: nil))

        #expect(harness.notes().isEmpty)
    }

    @Test("a note applies with its size")
    func sizeIsStored() throws {
        let harness = try makeHarness(host: "board-size.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, size: "large", boardSeq: 10))

        #expect(harness.note(1)?.size == "large")
    }

    /// An older server has no size field; the note is medium — the size
    /// every note had before the field existed — not a dropped note.
    @Test("a note without a size is medium")
    func missingSizeIsMedium() throws {
        let harness = try makeHarness(host: "board-nosize.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, size: nil, boardSeq: 10))

        #expect(harness.notes().count == 1)
        #expect(harness.note(1)?.size == "medium")
    }

    /// A picture pinned to the wall is a NOTE: it lands in the same store
    /// with its kind and the id of its picture, and a note from a server
    /// that predates kinds is a text note — which is also what an unknown
    /// kind DRAWS as (docs/protocol.md, "Board").
    @Test("a photo note keeps its kind and its picture")
    func photoNote() throws {
        let harness = try makeHarness(host: "board-photo.test")
        defer { harness.tearDown() }

        let picture = AttachmentDTO(
            id: 61, kind: "photo", mime: "image/jpeg", size: 4096, width: 1600, height: 1200,
            durationMS: nil, hasPreview: true, name: nil,
            latitude: nil, longitude: nil, accuracyM: nil)
        harness.coordinator.applyNote(
            note(id: 1, text: "", boardSeq: 10, kind: "photo", attachment: picture))
        harness.coordinator.applyNote(note(id: 2, boardSeq: 11))

        let photo = harness.note(1)
        #expect(photo?.kind == "photo")
        #expect(photo?.attachmentID == 61)
        #expect(photo?.attachmentWidth == 1600)
        #expect(photo?.text == "", "a picture needs no caption")
        // A note from before kinds, and a note that is simply text.
        #expect(harness.note(2)?.kind == "text")
        #expect(harness.note(2)?.attachmentID == nil)
    }

    /// An event is a NOTE with a when, a where and a guest list — and the
    /// guest list is `[]` on an event nobody has answered and NIL on every
    /// other kind, which is the difference a client draws on
    /// (docs/protocol.md, "Board").
    @Test("an event keeps its times, its place and its answers")
    func eventNote() throws {
        let harness = try makeHarness(host: "board-event.test")
        defer { harness.tearDown() }

        let starts = Date(timeIntervalSince1970: 1_798_736_400)
        harness.coordinator.applyNote(
            note(
                id: 1, text: "Christmas dinner", boardSeq: 10, kind: "event",
                startsAt: starts, endsAt: starts.addingTimeInterval(4 * 3600),
                place: "Gran's house",
                rsvps: [RsvpDTO(userID: 9, answer: "going"), RsvpDTO(userID: 11, answer: "maybe")]))
        harness.coordinator.applyNote(note(id: 2, boardSeq: 11))

        let event = try #require(harness.note(1))
        #expect(event.kind == "event")
        #expect(event.startsAt == starts)
        #expect(event.place == "Gran's house")
        #expect(event.rsvpList.count == 2)
        #expect(event.myAnswer(9) == "going")
        #expect(event.myAnswer(7) == nil, "this reader has not answered")
        #expect(event.answerCount("going") == 1)
        #expect(event.answerCount("maybe") == 1)
        // A text note carries none of it.
        #expect(harness.note(2)?.startsAt == nil)
        #expect(harness.note(2)?.rsvpsJSON == nil)
    }

    /// A LIST keeps its lines, their ids and their ticks — and an empty
    /// list is still a list, which is the difference `[]` and nil carry
    /// (docs/protocol.md, "Board").
    @Test("a task list keeps its lines, their ids and their ticks")
    func taskListNote() throws {
        let harness = try makeHarness(host: "board-tasks.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(
            note(
                id: 1, text: "Saturday", boardSeq: 10, kind: "tasks",
                items: [
                    TaskItemDTO(id: 11, text: "Milk", done: true, doneBy: 9),
                    TaskItemDTO(id: 12, text: "Bread", done: false, doneBy: nil),
                ]))
        harness.coordinator.applyNote(
            note(id: 2, text: "Sunday", boardSeq: 11, kind: "tasks", items: []))
        harness.coordinator.applyNote(note(id: 3, boardSeq: 12))

        let list = try #require(harness.note(1))
        #expect(list.kind == "tasks")
        #expect(list.taskList.map(\.text) == ["Milk", "Bread"])
        #expect(list.taskList.map(\.id) == [11, 12])
        #expect(list.taskList.first?.done == true)
        #expect(list.taskList.first?.doneBy == 9)
        #expect(list.tasksDone == 1)
        // An empty list is a list: the rows are there to be written into.
        #expect(harness.note(2)?.taskList.isEmpty == true)
        #expect(harness.note(2)?.itemsJSON == "[]")
        // And a note that is not a list carries none of it at all — the
        // difference a client draws the block on.
        #expect(harness.note(3)?.itemsJSON == nil)
        #expect(harness.note(3)?.tasksDone == 0)

        // A LATER frame rewrites the lines in place — which is how a tick
        // by somebody else arrives at all: the frame carries the whole
        // note, and a row that kept its old copy would draw a list nobody
        // has (docs/protocol.md, "Board").
        harness.coordinator.applyNote(
            note(
                id: 1, text: "Saturday", boardSeq: 13, kind: "tasks",
                items: [
                    TaskItemDTO(id: 11, text: "Oat milk", done: true, doneBy: 9),
                    TaskItemDTO(id: 12, text: "Bread", done: true, doneBy: 11),
                    TaskItemDTO(id: 14, text: "Eggs", done: false, doneBy: nil),
                ]))
        let after = try #require(harness.note(1))
        #expect(after.taskList.map(\.text) == ["Oat milk", "Bread", "Eggs"])
        #expect(after.tasksDone == 2)
        #expect(after.taskList.last?.id == 14)
    }

    /// A task list as the server REALLY sends one: this JSON is a
    /// transcript of a live `POST` and a live tick, not a hand-written
    /// guess — the only kind of fixture that catches a field this client
    /// spells differently from the server (docs/protocol.md, "Board").
    @Test("A task list decodes as the server sends it")
    func taskListDecoding() throws {
        let decoder = APICoding.decoder()

        let list = Data(#"""
        {"author_id": 2, "board_seq": 2, "color": "green", "content_seq": 1,
         "created_at": "2026-09-11T14:43:36.832551Z", "font": "plain", "id": 1,
         "items": [{"done": true, "done_by": 3, "id": 1, "text": "Milk"},
                   {"done": false, "id": 2, "text": "Bread"}],
         "kind": "tasks", "size": "medium", "text": "Saturday",
         "updated_at": "2026-09-11T14:43:36.841601Z", "x": 0.2, "y": 0.3}
        """#.utf8)
        let decoded = try decoder.decode(NoteDTO.self, from: list)
        #expect(decoded.kind == "tasks")
        let items = try #require(decoded.items)
        #expect(items.count == 2)
        #expect(items[0] == TaskItemDTO(id: 1, text: "Milk", done: true, doneBy: 3))
        // Not done means nobody did it, so the server sends no `done_by`
        // at all — and this client reads that as nobody.
        #expect(items[1] == TaskItemDTO(id: 2, text: "Bread", done: false, doneBy: nil))

        // An empty list is a list, and a note that is not one carries no
        // `items` at all: the difference a client draws the block on.
        let blank = Data(#"""
        {"id": 2, "board_seq": 3, "kind": "tasks", "text": "Sunday", "items": []}
        """#.utf8)
        #expect(try decoder.decode(NoteDTO.self, from: blank).items == [])
        let plain = Data(#"{"id": 3, "board_seq": 4, "text": "Milk"}"#.utf8)
        #expect(try decoder.decode(NoteDTO.self, from: plain).items == nil)
    }

    /// What a STICKER draws of a list, and what it leaves
    /// (docs/protocol.md, "Board").
    @Test("a sticker draws the first lines of a list and says how many are left")
    func taskListOnTheWall() {
        #expect(BoardTasks.onWall == 5)
        #expect(BoardTasks.drawn(of: 0) == (0, 0))
        #expect(BoardTasks.drawn(of: 3) == (3, 0))
        // A list of exactly the cap says nothing extra.
        #expect(BoardTasks.drawn(of: 5) == (5, 0))
        #expect(BoardTasks.drawn(of: 6) == (5, 1))
        #expect(BoardTasks.drawn(of: 20) == (5, 15))
    }

    /// What a save SENDS: the lines that say something, trimmed, with the
    /// ids they keep — which is what carries a tick through a rewrite
    /// (docs/protocol.md, "Board").
    @Test("a list sends the lines that say something, with their ids")
    func writtenLines() {
        let written = DraftTaskLine.written([
            DraftTaskLine(itemID: 11, text: "  Oat milk "),
            DraftTaskLine(itemID: nil, text: "Bread"),
            // Somebody who started typing and stopped: not a thing to do,
            // and a line the server would refuse.
            DraftTaskLine(itemID: nil, text: "   "),
        ])

        #expect(written.count == 2)
        #expect(written[0] == APIClient.TaskLineRequest(id: 11, text: "Oat milk"))
        #expect(written[1] == APIClient.TaskLineRequest(id: nil, text: "Bread"))
    }

    /// The same rule one field over: an older server has no font field, and
    /// the note is plain — the face every note was written in before the
    /// field existed (docs/protocol.md, "Board"). Stored as the NAME rather
    /// than an empty string: NoteFont draws an empty one plain anyway, so a
    /// blank would be invisible here and wrong in the store.
    @Test("a note without a font is plain")
    func missingFontIsPlain() throws {
        let harness = try makeHarness(host: "board-nofont.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 10))
        harness.coordinator.applyNote(note(id: 2, boardSeq: 11, font: "casual"))

        #expect(harness.note(1)?.font == "plain")
        #expect(harness.note(2)?.font == "casual")
    }

    @Test("a newer seq changes the size in place")
    func newerSeqResizes() throws {
        let harness = try makeHarness(host: "board-resize.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, size: "large", boardSeq: 10))
        harness.coordinator.applyNote(note(id: 1, size: "small", boardSeq: 11))

        #expect(harness.notes().count == 1)
        #expect(harness.note(1)?.size == "small")
        #expect(harness.note(1)?.boardSeq == 11)
    }

    /// Resizing is a mutation like a move: the same guard keeps a late
    /// frame from shrinking a note the author has since made large.
    @Test("a stale seq does not change the size")
    func staleSeqKeepsSize() throws {
        let harness = try makeHarness(host: "board-stalesize.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, size: "large", boardSeq: 20))
        harness.coordinator.applyNote(note(id: 1, size: "small", boardSeq: 12))

        #expect(harness.note(1)?.size == "large")
        #expect(harness.note(1)?.boardSeq == 20)
    }

    // --- The badge (issue #53) -----------------------------------------
    //
    // A move, a resize and a recolour all take a new board_seq, and none of
    // them is anything to READ. The rule lives in BoardBadge so the phone,
    // the Mac and Android cannot answer it differently.

    @Test("a note applies with its content seq, and a move leaves it alone")
    func contentSeqSurvivesAMove() throws {
        let harness = try makeHarness(host: "board-contentseq.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 10, contentSeq: 10))
        #expect(harness.note(1)?.contentSeq == 10)

        // The server moved board_seq and kept content_seq: a drag.
        harness.coordinator.applyNote(note(id: 1, x: 0.9, boardSeq: 11, contentSeq: 10))
        #expect(harness.note(1)?.boardSeq == 11)
        #expect(harness.note(1)?.contentSeq == 10)

        // …and a rewrite moves both.
        harness.coordinator.applyNote(
            note(id: 1, text: "Oat milk", boardSeq: 12, contentSeq: 12))
        #expect(harness.note(1)?.contentSeq == 12)
    }

    /// A server that predates the field sends none, and the row then says
    /// so with 0 — which is what sends the badge back to the note-id rule.
    @Test("a note without a content seq stores 0")
    func missingContentSeqIsZero() throws {
        let harness = try makeHarness(host: "board-nocontentseq.test")
        defer { harness.tearDown() }

        harness.coordinator.applyNote(note(id: 1, boardSeq: 10, contentSeq: nil))

        #expect(harness.notes().count == 1)
        #expect(harness.note(1)?.contentSeq == 0)
    }
}

//
//  BoardBadgeTests.swift
//  FamilyConnectTests
//
//  Issue #53: "only changes in data should be mentioned by badge, changing
//  size or positions shouldn't be."
//
//  The rule under test is the whole of the fix on this side of the wire —
//  the phone and the Mac both call it, and Android has the same one in
//  ui/board/BoardBadge.kt, so a case that matters here has a twin there.
//

import Foundation
import Testing

@testable import FamilyConnect

@Suite("Board badge")
struct BoardBadgeTests {

    private func note(
        id: Int64,
        boardSeq: Int64,
        contentSeq: Int64
    ) -> NoteEntity {
        NoteEntity(
            noteID: id, authorID: 7, text: "Milk", color: "yellow", size: "medium",
            x: 0.2, y: 0.3, createdAt: Date(), updatedAt: Date(),
            boardSeq: boardSeq, contentSeq: contentSeq)
    }

    /// The report, in one case: a wall of notes everybody has read, dragged
    /// and resized until the sync cursor is far away, still badges nobody.
    @Test("a tidied wall raises no badge")
    func geometryNeverCounts() {
        // Three notes written at 10, 11, 12 and shown to the user.
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)
        // Then dragged and resized for a while: board_seq up at 40, and not
        // one word of any of them changed.
        let notes = [
            note(id: 1, boardSeq: 38, contentSeq: 10),
            note(id: 2, boardSeq: 40, contentSeq: 11),
            note(id: 3, boardSeq: 39, contentSeq: 12),
        ]

        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 0)
    }

    /// The other half of the sentence: a change in the DATA is exactly what
    /// a badge is for — and the note-id rule could never see it, because an
    /// edited note keeps the id it always had.
    @Test("a rewrite of an old note counts")
    func rewritesCount() {
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)
        let notes = [
            note(id: 1, boardSeq: 38, contentSeq: 10),
            // Note 2 was rewritten after the user last looked.
            note(id: 2, boardSeq: 41, contentSeq: 41),
            note(id: 3, boardSeq: 39, contentSeq: 12),
        ]

        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 1)
    }

    @Test("a note pinned since the last look counts")
    func newNotesCount() {
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)
        let notes = [note(id: 4, boardSeq: 20, contentSeq: 20)]

        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 1)
    }

    /// The case that made a drag look like a new note on a device whose
    /// cache was empty: the note only ENTERED the cache because somebody
    /// moved it. It is still an old note, and its content seq says so.
    @Test("a note that arrives via somebody's drag is not new")
    func aNoteMaterialisedByAMoveIsNotNew() {
        let marks = BoardBadge.Marks(seenNoteID: 9, seenContentSeq: 30)
        // Written at 12, dragged just now at 47 — and this device is only
        // seeing it for the first time because of the drag.
        let notes = [note(id: 4, boardSeq: 47, contentSeq: 12)]

        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 0)
    }

    /// No content seq at all — a row cached before the field existed, or a
    /// server that predates it — falls back to the rule that was in force
    /// then, unchanged.
    @Test("a note with no content seq is judged by its id")
    func zeroFallsBackToTheIDRule() {
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)

        #expect(
            BoardBadge.unreadCount(
                notes: [note(id: 2, boardSeq: 40, contentSeq: 0)], marks: marks) == 0)
        #expect(
            BoardBadge.unreadCount(
                notes: [note(id: 4, boardSeq: 40, contentSeq: 0)], marks: marks) == 1)
    }

    @Test("showing the board advances both marks, and neither goes back")
    func marksAreMonotonic() {
        let marks = BoardBadge.Marks(seenNoteID: 9, seenContentSeq: 30)
        // A board whose newest note has since been deleted by its author:
        // the highest id and seq ON IT are now lower than what this device
        // has already shown.
        let notes = [note(id: 4, boardSeq: 20, contentSeq: 12)]

        let after = BoardBadge.marksAfterShowing(notes: notes, marks: marks)

        #expect(after == marks)
    }

    @Test("showing the board clears what it shows")
    func showingClearsTheBadge() {
        var marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)
        let notes = [
            note(id: 4, boardSeq: 20, contentSeq: 20),
            note(id: 2, boardSeq: 41, contentSeq: 41),
        ]
        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 2)

        marks = BoardBadge.marksAfterShowing(notes: notes, marks: marks)

        #expect(marks == BoardBadge.Marks(seenNoteID: 4, seenContentSeq: 41))
        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 0)
    }

    // --- The one-time seed after the update ----------------------------

    /// A device that has shown this board before, on the launch that first
    /// learns about content seqs. Without a seed the server's backfill
    /// (`content_seq = board_seq` for every note that already existed)
    /// would badge the whole wall — a fix for a badge that cries wolf
    /// should not start by crying wolf.
    @Test("the content mark seeds from what was already shown")
    func seedFromShownNotes() {
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 0)
        let notes = [
            note(id: 1, boardSeq: 38, contentSeq: 0),
            note(id: 2, boardSeq: 40, contentSeq: 0),
            note(id: 3, boardSeq: 39, contentSeq: 0),
        ]

        #expect(BoardBadge.contentMarkSeed(notes: notes, marks: marks) == 40)
    }

    /// Notes the user has NOT been shown are left out of the seed, so a
    /// badge that was pending when the update landed survives it.
    @Test("the seed ignores notes above the id mark")
    func seedIgnoresUnseenNotes() {
        let marks = BoardBadge.Marks(seenNoteID: 2, seenContentSeq: 0)
        let notes = [
            note(id: 1, boardSeq: 38, contentSeq: 0),
            note(id: 2, boardSeq: 39, contentSeq: 0),
            // Pinned after the last look: still worth a badge afterwards.
            note(id: 3, boardSeq: 44, contentSeq: 0),
        ]

        let seed = BoardBadge.contentMarkSeed(notes: notes, marks: marks)
        #expect(seed == 39)

        let seeded = BoardBadge.Marks(seenNoteID: 2, seenContentSeq: seed ?? 0)
        // The server now answers with its backfill: content_seq = board_seq.
        let refreshed = [
            note(id: 1, boardSeq: 38, contentSeq: 38),
            note(id: 2, boardSeq: 39, contentSeq: 39),
            note(id: 3, boardSeq: 44, contentSeq: 44),
        ]
        #expect(BoardBadge.unreadCount(notes: refreshed, marks: seeded) == 1)
    }

    /// A fresh install has shown nothing, so there is nothing to seed and
    /// the whole board is correctly new to it.
    @Test("a device that never showed the board seeds nothing")
    func noSeedWithoutAnIDMark() {
        let marks = BoardBadge.Marks.zero
        let notes = [note(id: 1, boardSeq: 38, contentSeq: 38)]

        #expect(BoardBadge.contentMarkSeed(notes: notes, marks: marks) == nil)
        #expect(BoardBadge.unreadCount(notes: notes, marks: marks) == 1)
    }

    /// Seeded once and never again: a real mark is not second-guessed.
    @Test("a mark already set is never re-seeded")
    func noSeedOverAnExistingMark() {
        let marks = BoardBadge.Marks(seenNoteID: 3, seenContentSeq: 12)
        let notes = [note(id: 1, boardSeq: 90, contentSeq: 12)]

        #expect(BoardBadge.contentMarkSeed(notes: notes, marks: marks) == nil)
    }
}

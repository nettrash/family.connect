//
//  BoardBadge.swift
//  FamilyConnect
//
//  What the board badge counts, in ONE place (docs/protocol.md, "Board").
//
//  The rule used to be "notes whose id is above the highest id you have been
//  shown", and it was wrong in the way issue #53 describes: a badge is a
//  claim that there is something to READ, and a wall somebody tidied has
//  nothing new to read on it. Dragging a note, resizing it or recolouring it
//  all take a new `board_seq` — they must, or the change feed could not
//  carry a move from one device to another — and none of them changes a word
//  of what the note says.
//
//  So the server stamps a second seq, `content_seq`, moved only by a change
//  of TEXT (and by creation, which is new text by definition), and this is
//  what the badge counts. The definition of "worth a badge" therefore lives
//  on the server, where all three clients read the same answer, instead of
//  in three local comparisons that would drift.
//
//  ZERO MEANS UNKNOWN. A row cached before this field existed, and a note
//  from a server that predates it, both arrive with no content seq at all,
//  and for those the old note-id rule still applies — it is exactly as right
//  as it ever was, and it is the only thing such a note can be judged by.
//
//  Both marks move only when the board is actually ON SCREEN. Neither is the
//  sync cursor: that advances on a background resync and would clear the
//  badge for somebody who never looked (AppSettings.boardCursor).
//
//  Both Apple surfaces use this — the phone's toolbar badge (ChatListView)
//  and the Mac's (MacChatView) — so the two cannot disagree about the same
//  board on the same account. Android counterpart: ui/board/BoardBadge.kt.
//

import Foundation

enum BoardBadge {

    /// How much of the board this device has actually shown its user.
    struct Marks: Equatable, Sendable {
        /// Highest note id shown. The fallback rule's mark, kept for notes
        /// that carry no content seq.
        var seenNoteID: Int64
        /// Highest `content_seq` shown. The rule.
        var seenContentSeq: Int64

        static let zero = Marks(seenNoteID: 0, seenContentSeq: 0)
    }

    /// One note's verdict. Pure, so both platforms and the tests share it.
    static func isUnread(noteID: Int64, contentSeq: Int64, marks: Marks) -> Bool {
        if contentSeq > 0 { return contentSeq > marks.seenContentSeq }
        // Nothing said when this text was written: fall back to the rule
        // that was in force before the field existed.
        return noteID > marks.seenNoteID
    }

    static func unreadCount(notes: [NoteEntity], marks: Marks) -> Int {
        notes.count { isUnread(noteID: $0.noteID, contentSeq: $0.contentSeq, marks: marks) }
    }

    /// The marks after the board has been on screen: everything on it has
    /// been shown. Monotonic in both fields — two surfaces marking at once
    /// (a Mac window and its main window) must not resurrect a cleared
    /// badge, and neither must a board that has just lost its newest note
    /// to somebody else's delete.
    static func marksAfterShowing(notes: [NoteEntity], marks: Marks) -> Marks {
        Marks(
            seenNoteID: max(marks.seenNoteID, notes.map(\.noteID).max() ?? 0),
            seenContentSeq: max(marks.seenContentSeq, notes.map(\.contentSeq).max() ?? 0))
    }

    /// The one-time seed for the content mark, or nil when there is nothing
    /// to seed (docs/protocol.md, "Board").
    ///
    /// The update that introduces content seqs finds a device whose content
    /// mark is 0 and whose note-id mark is not: it HAS shown this board, it
    /// just never had the new number. Left at 0, the first thing the server
    /// says would badge the whole wall — because the server's own backfill
    /// set `content_seq = board_seq` for every note that already existed,
    /// and every one of those is above 0.
    ///
    /// The cache cannot say more than `boardSeq` here: rows stored before
    /// the update carry no content seq of their own. But `board_seq` is
    /// exactly the height the backfill used, so a mark at the highest one
    /// among the notes the user has already been shown is high enough to
    /// keep an old wall quiet, and low enough to leave a note created since
    /// (its id above the id mark, its seq above these) still counted.
    static func contentMarkSeed(notes: [NoteEntity], marks: Marks) -> Int64? {
        guard marks.seenContentSeq == 0, marks.seenNoteID > 0 else { return nil }
        let seed = notes
            .filter { $0.noteID <= marks.seenNoteID }
            .map(\.boardSeq)
            .max() ?? 0
        return seed > 0 ? seed : nil
    }
}

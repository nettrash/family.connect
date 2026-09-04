/*
 * BoardBadge.kt
 * Family Connect (Android)
 *
 * What the board badge counts, in ONE place (docs/protocol.md, "Board").
 *
 * The rule used to be "notes whose id is above the highest id you have been
 * shown", and it was wrong in the way issue #53 describes: a badge is a
 * claim that there is something to READ, and a wall somebody tidied has
 * nothing new to read on it. Dragging a note, resizing it or recolouring it
 * all take a new `board_seq` — they must, or the change feed could not carry
 * a move from one device to another — and none of them changes a word of
 * what the note says.
 *
 * So the server stamps a second seq, `content_seq`, moved only by a change
 * of TEXT (and by creation, which is new text by definition), and this is
 * what the badge counts. The definition of "worth a badge" lives on the
 * server, where all three clients read the same answer, instead of in three
 * local comparisons that would drift.
 *
 * ZERO MEANS UNKNOWN. A row cached before the column existed, and a note
 * from a server that predates the field, both arrive without one, and for
 * those the old note-id rule still applies — it is exactly as right as it
 * ever was, and it is the only thing such a note can be judged by.
 *
 * Neither mark is the sync cursor: that advances on a background resync and
 * would clear the badge for somebody who never opened the board
 * (SettingsState.boardCursor).
 *
 * Shared with the phone and the Mac by CONTRACT rather than by code:
 * ios/FamilyConnect/Core/BoardBadge.swift holds the same rules and
 * BoardBadgeTest mirrors BoardBadgeTests.swift case for case. Two ports
 * counting the same wall differently is exactly the drift this file exists
 * to prevent.
 */

package me.nettrash.familyconnect.util

import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.settings.SettingsState

object BoardBadge {

    /** How much of the board this device has actually shown its user. */
    data class Marks(
        /**
         * Highest note id shown. The fallback rule's mark, kept for notes
         * that carry no content seq.
         */
        val seenNoteId: Long = 0,
        /** Highest `content_seq` shown. The rule. */
        val seenContentSeq: Long = 0,
    )

    /** One note's verdict. */
    fun isUnread(noteId: Long, contentSeq: Long, marks: Marks): Boolean =
        if (contentSeq > 0) {
            contentSeq > marks.seenContentSeq
        } else {
            // Nothing said when this text was written: fall back to the
            // rule that was in force before the field existed.
            noteId > marks.seenNoteId
        }

    fun unreadCount(notes: List<NoteMark>, marks: Marks): Int =
        notes.count { isUnread(it.noteId, it.contentSeq, marks) }

    /**
     * The marks after the board has been on screen: everything on it has
     * been shown. Monotonic in both fields — two screens marking at once
     * must not resurrect a cleared badge, and neither must a board that has
     * just lost its newest note to somebody else's delete.
     */
    fun marksAfterShowing(notes: List<NoteMark>, marks: Marks): Marks = Marks(
        seenNoteId = maxOf(marks.seenNoteId, notes.maxOfOrNull { it.noteId } ?: 0L),
        seenContentSeq = maxOf(marks.seenContentSeq, notes.maxOfOrNull { it.contentSeq } ?: 0L),
    )

    /**
     * The one-time seed for the content mark, or null when there is nothing
     * to seed (docs/protocol.md, "Board").
     *
     * The update that introduces content seqs finds a device whose content
     * mark is 0 and whose note-id mark is not: it HAS shown this board, it
     * just never had the new number. Left at 0, the first thing the server
     * says would badge the whole wall — because the server's own backfill
     * set `content_seq = board_seq` for every note that already existed,
     * and every one of those is above 0.
     *
     * The cache cannot say more than `boardSeq` here: rows stored before the
     * update carry no content seq of their own. But `board_seq` is exactly
     * the height the backfill used, so a mark at the highest one among the
     * notes the user has already been shown is high enough to keep an old
     * wall quiet, and low enough to leave a note pinned since (its id above
     * the id mark, its seq above these) still counted.
     */
    fun contentMarkSeed(notes: List<NoteMark>, marks: Marks): Long? {
        if (marks.seenContentSeq != 0L || marks.seenNoteId <= 0L) return null
        val seed = notes
            .filter { it.noteId <= marks.seenNoteId }
            .maxOfOrNull { it.boardSeq }
            ?: 0L
        return seed.takeIf { it > 0L }
    }

    /**
     * The three numbers this rule needs from a note, so the arithmetic can
     * be tested without a database and shared with anything holding notes
     * in another shape.
     */
    data class NoteMark(val noteId: Long, val boardSeq: Long, val contentSeq: Long)
}

/**
 * The cached notes, as the three numbers the badge rule needs. Written as
 * an extension so a screen or a repository hands the rule its rows without
 * either side knowing about the other's shape.
 */
fun List<NoteEntity>.marks(): List<BoardBadge.NoteMark> =
    map { BoardBadge.NoteMark(noteId = it.id, boardSeq = it.boardSeq, contentSeq = it.contentSeq) }

/**
 * The two marks as one value. Read together on purpose: a badge that used
 * one mark from before an update and one from after would be neither rule.
 */
fun SettingsState.badgeMarks(): BoardBadge.Marks = BoardBadge.Marks(
    seenNoteId = boardSeenNoteId,
    seenContentSeq = boardSeenContentSeq,
)

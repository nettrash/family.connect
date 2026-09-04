/*
 * BoardBadgeTest.kt
 * Family Connect (Android)
 *
 * Issue #53: "only changes in data should be mentioned by badge, changing
 * size or positions shouldn't be."
 *
 * Mirrors ios/FamilyConnectTests/BoardBadgeTests.swift case for case, for
 * the reason MemberCapTest mirrors its Swift twin: a family that counts a
 * different number of new notes on a phone and on a tablet has been told
 * the badge is noise.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import org.junit.Test

class BoardBadgeTest {

    private fun note(id: Long, boardSeq: Long, contentSeq: Long) =
        BoardBadge.NoteMark(noteId = id, boardSeq = boardSeq, contentSeq = contentSeq)

    /**
     * The report, in one case: a wall of notes everybody has read, dragged
     * and resized until the sync cursor is far away, still badges nobody.
     */
    @Test
    fun `a tidied wall raises no badge`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)
        val notes = listOf(
            note(id = 1, boardSeq = 38, contentSeq = 10),
            note(id = 2, boardSeq = 40, contentSeq = 11),
            note(id = 3, boardSeq = 39, contentSeq = 12),
        )

        assertThat(BoardBadge.unreadCount(notes, marks)).isEqualTo(0)
    }

    /**
     * The other half of the sentence: a change in the DATA is exactly what
     * a badge is for — and the note-id rule could never see it, because an
     * edited note keeps the id it always had.
     */
    @Test
    fun `a rewrite of an old note counts`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)
        val notes = listOf(
            note(id = 1, boardSeq = 38, contentSeq = 10),
            note(id = 2, boardSeq = 41, contentSeq = 41),
            note(id = 3, boardSeq = 39, contentSeq = 12),
        )

        assertThat(BoardBadge.unreadCount(notes, marks)).isEqualTo(1)
    }

    @Test
    fun `a note pinned since the last look counts`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)

        assertThat(
            BoardBadge.unreadCount(listOf(note(id = 4, boardSeq = 20, contentSeq = 20)), marks),
        ).isEqualTo(1)
    }

    /**
     * The case that made a drag look like a new note on a device whose
     * cache was empty: the note only ENTERED the cache because somebody
     * moved it. It is still an old note, and its content seq says so.
     */
    @Test
    fun `a note that arrives via somebody's drag is not new`() {
        val marks = BoardBadge.Marks(seenNoteId = 9, seenContentSeq = 30)

        assertThat(
            BoardBadge.unreadCount(listOf(note(id = 4, boardSeq = 47, contentSeq = 12)), marks),
        ).isEqualTo(0)
    }

    @Test
    fun `a note with no content seq is judged by its id`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)

        assertThat(
            BoardBadge.unreadCount(listOf(note(id = 2, boardSeq = 40, contentSeq = 0)), marks),
        ).isEqualTo(0)
        assertThat(
            BoardBadge.unreadCount(listOf(note(id = 4, boardSeq = 40, contentSeq = 0)), marks),
        ).isEqualTo(1)
    }

    @Test
    fun `showing the board advances both marks and neither goes back`() {
        val marks = BoardBadge.Marks(seenNoteId = 9, seenContentSeq = 30)
        // A board whose newest note has since been deleted by its author.
        val notes = listOf(note(id = 4, boardSeq = 20, contentSeq = 12))

        assertThat(BoardBadge.marksAfterShowing(notes, marks)).isEqualTo(marks)
    }

    @Test
    fun `showing the board clears what it shows`() {
        var marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)
        val notes = listOf(
            note(id = 4, boardSeq = 20, contentSeq = 20),
            note(id = 2, boardSeq = 41, contentSeq = 41),
        )
        assertThat(BoardBadge.unreadCount(notes, marks)).isEqualTo(2)

        marks = BoardBadge.marksAfterShowing(notes, marks)

        assertThat(marks).isEqualTo(BoardBadge.Marks(seenNoteId = 4, seenContentSeq = 41))
        assertThat(BoardBadge.unreadCount(notes, marks)).isEqualTo(0)
    }

    // --- The one-time seed after the update ----------------------------

    @Test
    fun `the content mark seeds from what was already shown`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 0)
        val notes = listOf(
            note(id = 1, boardSeq = 38, contentSeq = 0),
            note(id = 2, boardSeq = 40, contentSeq = 0),
            note(id = 3, boardSeq = 39, contentSeq = 0),
        )

        assertThat(BoardBadge.contentMarkSeed(notes, marks)).isEqualTo(40)
    }

    /**
     * Notes the user has NOT been shown are left out of the seed, so a
     * badge that was pending when the update landed survives it.
     */
    @Test
    fun `the seed ignores notes above the id mark`() {
        val marks = BoardBadge.Marks(seenNoteId = 2, seenContentSeq = 0)
        val notes = listOf(
            note(id = 1, boardSeq = 38, contentSeq = 0),
            note(id = 2, boardSeq = 39, contentSeq = 0),
            note(id = 3, boardSeq = 44, contentSeq = 0),
        )

        val seed = BoardBadge.contentMarkSeed(notes, marks)
        assertThat(seed).isEqualTo(39)

        // The server now answers with its backfill: content_seq = board_seq.
        val refreshed = listOf(
            note(id = 1, boardSeq = 38, contentSeq = 38),
            note(id = 2, boardSeq = 39, contentSeq = 39),
            note(id = 3, boardSeq = 44, contentSeq = 44),
        )
        val seeded = BoardBadge.Marks(seenNoteId = 2, seenContentSeq = seed ?: 0)
        assertThat(BoardBadge.unreadCount(refreshed, seeded)).isEqualTo(1)
    }

    @Test
    fun `a device that never showed the board seeds nothing`() {
        val marks = BoardBadge.Marks()
        val notes = listOf(note(id = 1, boardSeq = 38, contentSeq = 38))

        assertThat(BoardBadge.contentMarkSeed(notes, marks)).isNull()
        assertThat(BoardBadge.unreadCount(notes, marks)).isEqualTo(1)
    }

    @Test
    fun `a mark already set is never re-seeded`() {
        val marks = BoardBadge.Marks(seenNoteId = 3, seenContentSeq = 12)
        val notes = listOf(note(id = 1, boardSeq = 90, contentSeq = 12))

        assertThat(BoardBadge.contentMarkSeed(notes, marks)).isNull()
    }
}

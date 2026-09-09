/*
 * BoardRepositoryTest.kt
 * Family Connect (Android)
 *
 * The board apply path: the per-note seq guard and tombstone handling.
 * Mirrors ios/FamilyConnectTests/BoardSyncTests.swift case for case — the
 * two clients must agree about this or a dragged note ends up in different
 * places on two phones.
 *
 * Robolectric + in-memory Room, like MessageDaoTest.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.NoteDao
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.RsvpDto
import me.nettrash.familyconnect.data.net.dto.RsvpCodec
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeBoardApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.noteDto
import me.nettrash.familyconnect.testutil.noteTombstone
import me.nettrash.familyconnect.util.BoardBadge
import me.nettrash.familyconnect.util.badgeMarks
import me.nettrash.familyconnect.util.marks
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class BoardRepositoryTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var noteDao: NoteDao
    private lateinit var boardApi: FakeBoardApi
    private lateinit var settings: FakeSettingsRepository
    private lateinit var socket: FakeChatSocket

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
        noteDao = db.noteDao()
        boardApi = FakeBoardApi()
        settings = FakeSettingsRepository()
        socket = FakeChatSocket()
    }

    @After
    fun tearDown() {
        db.close()
    }

    /** The collector runs for the life of the app scope — see the note in
     *  [[kotlin-coroutines-test-gotchas]]: it belongs on backgroundScope. */
    private fun kotlinx.coroutines.test.TestScope.repository() =
        BoardRepository(boardApi, noteDao, settings, socket, backgroundScope)

    @Test
    fun `a note is created then updated in place`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 10))
        repository.applyNote(noteDto(id = 1, text = "Oat milk", x = 0.8, boardSeq = 11))
        runCurrent()

        val notes = noteDao.observeNotes().first()
        assertThat(notes).hasSize(1)
        assertThat(notes[0].text).isEqualTo("Oat milk")
        assertThat(notes[0].x).isEqualTo(0.8)
        assertThat(notes[0].boardSeq).isEqualTo(11)
    }

    /**
     * Two people dragging the same note is ordinary, so an out-of-order
     * frame must not undo the newer move.
     */
    @Test
    fun `a stale seq never undoes a newer move`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, x = 0.9, boardSeq = 20))
        val applied = repository.applyNote(noteDto(id = 1, x = 0.1, boardSeq = 12))
        runCurrent()

        assertThat(applied).isFalse()
        assertThat(noteDao.findById(1)!!.x).isEqualTo(0.9)
        assertThat(noteDao.findById(1)!!.boardSeq).isEqualTo(20)
    }

    @Test
    fun `re-delivering the same seq changes nothing`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, x = 0.4, boardSeq = 20))
        val applied = repository.applyNote(noteDto(id = 1, x = 0.4, boardSeq = 20))
        runCurrent()

        assertThat(applied).isFalse()
        assertThat(noteDao.observeNotes().first()).hasSize(1)
    }

    /** The tombstone is the ONLY signal a note is gone. */
    @Test
    fun `a tombstone removes the note`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 10))
        repository.applyNote(noteTombstone(id = 1, boardSeq = 11))
        runCurrent()

        assertThat(noteDao.observeNotes().first()).isEmpty()
    }

    @Test
    fun `a tombstone for an unknown note is harmless`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteTombstone(id = 99, boardSeq = 5))
        runCurrent()

        assertThat(noteDao.observeNotes().first()).isEmpty()
    }

    @Test
    fun `a stale tombstone does not delete a newer note`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 30))
        repository.applyNote(noteTombstone(id = 1, boardSeq = 12))
        runCurrent()

        assertThat(noteDao.observeNotes().first()).hasSize(1)
    }

    /** A live note missing its content is a server bug; a blank sticker is
     *  worse than no sticker. */
    @Test
    fun `a live note with no content is ignored`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(
            noteDto(id = 1, boardSeq = 3).copy(text = null, color = null, x = null, y = null),
        )
        runCurrent()

        assertThat(noteDao.observeNotes().first()).isEmpty()
    }

    /** A first open reads the whole wall rather than replaying every note
     *  that ever existed. */
    @Test
    fun `an empty cursor triggers a full board read`() = runTest(dispatcher) {
        val repository = repository()
        boardApi.board = me.nettrash.familyconnect.data.net.dto.BoardResponse(
            notes = listOf(noteDto(id = 1, boardSeq = 4), noteDto(id = 2, boardSeq = 5)),
            maxBoardSeq = 5,
        )

        repository.catchUpBoard(serverMaxSeq = 5)
        runCurrent()

        assertThat(noteDao.observeNotes().first()).hasSize(2)
        assertThat(settings.current.boardCursor).isEqualTo(5)
    }

    @Test
    fun `catch up pages until short and advances the cursor`() = runTest(dispatcher) {
        val repository = repository()
        settings.setBoardCursor(4)
        repository.applyNote(noteDto(id = 1, boardSeq = 4))
        runCurrent()

        boardApi.changePages = mutableListOf(
            listOf(noteDto(id = 1, text = "moved", boardSeq = 9), noteTombstone(id = 2, boardSeq = 10)),
        )
        repository.catchUpBoard(serverMaxSeq = 10)
        runCurrent()

        assertThat(noteDao.findById(1)!!.text).isEqualTo("moved")
        assertThat(settings.current.boardCursor).isEqualTo(10)
    }

    /** A board the server says nothing has happened on costs no request. */
    @Test
    fun `catch up is skipped when the server cursor is not ahead`() = runTest(dispatcher) {
        val repository = repository()
        settings.setBoardCursor(10)
        runCurrent()

        repository.catchUpBoard(serverMaxSeq = 10)
        runCurrent()

        assertThat(boardApi.changePages).isEmpty()
        assertThat(noteDao.observeNotes().first()).isEmpty()
    }

    /** A move sends ONLY x/y — sending text (or size) would make the server
     *  demand authorship the mover may not have. */
    @Test
    fun `moving a note sends only the position`() = runTest(dispatcher) {
        val repository = repository()

        repository.updateNote(id = 7, x = 0.5, y = 0.6)
        runCurrent()

        val (id, request) = boardApi.patched.single()
        assertThat(id).isEqualTo(7)
        assertThat(request.x).isEqualTo(0.5)
        assertThat(request.y).isEqualTo(0.6)
        assertThat(request.text).isNull()
        assertThat(request.color).isNull()
        assertThat(request.size).isNull()
    }

    // -- Sizes (docs/protocol.md, "A note has a size") ------------------------

    @Test
    fun `a note applies with its size`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, size = "large", boardSeq = 10))
        runCurrent()

        assertThat(noteDao.findById(1)!!.size).isEqualTo("large")
    }

    /** A server from before the field existed sends no size; every note it
     *  has is the size every note had — medium. */
    @Test
    fun `a note without a size is stored as medium`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, size = null, boardSeq = 10))
        runCurrent()

        assertThat(noteDao.findById(1)!!.size).isEqualTo("medium")
    }

    /**
     * A picture pinned to the wall is a NOTE: it lands in the same table
     * with its kind and its attachment kept verbatim, and a note from a
     * server that predates kinds is a text note — which is also what an
     * unknown kind draws as (docs/protocol.md, "Board").
     */
    @Test
    fun `a photo note keeps its kind and its picture`() = runTest(dispatcher) {
        val repository = repository()
        val picture = FakeAttachmentApi.attachment(id = 61)

        repository.applyNote(noteDto(id = 1, kind = "photo", attachment = picture, boardSeq = 10))
        repository.applyNote(noteDto(id = 2, boardSeq = 11))
        runCurrent()

        val photo = noteDao.findById(1)!!
        assertThat(photo.kind).isEqualTo("photo")
        assertThat(AttachmentsCodec.decode(photo.attachmentJson)?.single()?.id).isEqualTo(61)
        val text = noteDao.findById(2)!!
        assertThat(text.kind).isEqualTo("text")
        assertThat(text.attachmentJson).isNull()
    }

    /**
     * An event is a NOTE with a when, a where and a guest list — and the
     * guest list is "[]" on an event nobody has answered and NULL on every
     * other kind, which is the difference the card draws on
     * (docs/protocol.md, "Board").
     */
    @Test
    fun `an event keeps its times, its place and its answers`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(
            noteDto(
                id = 1, text = "Christmas dinner", kind = "event",
                startsAt = "2026-12-24T17:00:00Z", endsAt = "2026-12-24T21:00:00Z",
                place = "Gran's house",
                rsvps = listOf(RsvpDto(9, "going"), RsvpDto(11, "maybe")),
                boardSeq = 10,
            ),
        )
        repository.applyNote(noteDto(id = 2, boardSeq = 11))
        runCurrent()

        val event = noteDao.findById(1)!!
        assertThat(event.kind).isEqualTo("event")
        assertThat(event.startsAt).isEqualTo(
            java.time.Instant.parse("2026-12-24T17:00:00Z").toEpochMilli(),
        )
        assertThat(event.endsAt).isNotNull()
        assertThat(event.place).isEqualTo("Gran's house")
        assertThat(RsvpCodec.decode(event.rsvpsJson)).hasSize(2)

        // A text note carries none of it.
        val text = noteDao.findById(2)!!
        assertThat(text.startsAt).isNull()
        assertThat(text.rsvpsJson).isNull()
    }

    /** Answering is the SHARED act: it goes out, and the answer comes back. */
    @Test
    fun `answering an event records it and applies the note that comes back`() =
        runTest(dispatcher) {
            val repository = repository()

            assertThat(repository.answerNote(1, "going")).isTrue()
            runCurrent()
            assertThat(boardApi.answers).containsExactly(1L to "going")
            assertThat(RsvpCodec.decode(noteDao.findById(1)!!.rsvpsJson)).hasSize(1)

            assertThat(repository.answerNote(1, null)).isTrue()
            runCurrent()
            assertThat(boardApi.answers.last()).isEqualTo(1L to null)
            assertThat(RsvpCodec.decode(noteDao.findById(1)!!.rsvpsJson)).isEmpty()
        }

    /**
     * The same rule one field over: a server from before fonts sends none,
     * and every note it has was written in the face every note was written
     * in — plain (docs/protocol.md, "Board"). Stored as the NAME, never as
     * an empty string: `NoteFonts.resolve` would draw an empty one plain
     * anyway, so a blank would be invisible here and wrong in the store.
     */
    @Test
    fun `a note without a font is stored as plain`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, font = null, boardSeq = 10))
        repository.applyNote(noteDto(id = 2, font = "casual", boardSeq = 11))
        runCurrent()

        assertThat(noteDao.findById(1)!!.font).isEqualTo("plain")
        assertThat(noteDao.findById(2)!!.font).isEqualTo("casual")
    }

    @Test
    fun `a newer seq changes the size in place`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, size = "small", boardSeq = 10))
        repository.applyNote(noteDto(id = 1, size = "large", boardSeq = 11))
        runCurrent()

        val notes = noteDao.observeNotes().first()
        assertThat(notes).hasSize(1)
        assertThat(notes[0].size).isEqualTo("large")
        assertThat(notes[0].boardSeq).isEqualTo(11)
    }

    /** Size is the author's field: it travels with text and colour, never
     *  with a move. */
    @Test
    fun `editing a note sends size with text and colour`() = runTest(dispatcher) {
        val repository = repository()

        repository.updateNote(id = 7, text = "Louder", color = "pink", size = "large")
        runCurrent()

        val (id, request) = boardApi.patched.single()
        assertThat(id).isEqualTo(7)
        assertThat(request.text).isEqualTo("Louder")
        assertThat(request.color).isEqualTo("pink")
        assertThat(request.size).isEqualTo("large")
        assertThat(request.x).isNull()
        assertThat(request.y).isNull()
        assertThat(noteDao.findById(7)!!.size).isEqualTo("large")
    }

    @Test
    fun `creating a note sends its size`() = runTest(dispatcher) {
        val repository = repository()

        repository.addNote(
            text = "Milk", color = "yellow", size = "small", font = "serif", x = 0.1, y = 0.2,
        )
        runCurrent()

        assertThat(boardApi.created.single().size).isEqualTo("small")
        assertThat(boardApi.created.single().font).isEqualTo("serif")
        assertThat(noteDao.observeNotes().first().single().size).isEqualTo("small")
    }

    // --- content_seq: the seq a badge counts (issue #53) ---------------

    @Test
    fun `a note stores its content seq and a move leaves it alone`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 10, contentSeq = 10))
        runCurrent()
        assertThat(noteDao.findById(1)!!.contentSeq).isEqualTo(10)

        // The server moved board_seq and kept content_seq: a drag.
        repository.applyNote(noteDto(id = 1, x = 0.9, boardSeq = 11, contentSeq = 10))
        runCurrent()
        assertThat(noteDao.findById(1)!!.boardSeq).isEqualTo(11)
        assertThat(noteDao.findById(1)!!.contentSeq).isEqualTo(10)

        // …and a rewrite moves both.
        repository.applyNote(noteDto(id = 1, text = "Oat milk", boardSeq = 12, contentSeq = 12))
        runCurrent()
        assertThat(noteDao.findById(1)!!.contentSeq).isEqualTo(12)
    }

    /** A server that predates the field sends none, and the row then says
     *  so with 0 — which is what sends the badge back to the note-id
     *  rule. */
    @Test
    fun `a note without a content seq stores zero`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 10, contentSeq = null))
        runCurrent()

        assertThat(noteDao.findById(1)!!.contentSeq).isEqualTo(0)
    }

    /** The update that brings content seqs finds a device that has shown
     *  this board before: its badge mark is seeded once, from what it
     *  already holds, or the server's backfill badges the whole wall. */
    @Test
    fun `applying a note seeds the badge's content mark once`() = runTest(dispatcher) {
        // What the cache looks like just after the update: rows with no
        // content seq of their own, and a note-id mark from before it.
        noteDao.upsert(
            NoteEntity(
                id = 1, authorId = 7, text = "Milk", color = "yellow", size = "medium",
                x = 0.2, y = 0.3, createdAt = 1L, updatedAt = 1L, boardSeq = 38, contentSeq = 0,
            ),
        )
        noteDao.upsert(
            NoteEntity(
                id = 2, authorId = 7, text = "Bread", color = "yellow", size = "medium",
                x = 0.2, y = 0.3, createdAt = 1L, updatedAt = 1L, boardSeq = 40, contentSeq = 0,
            ),
        )
        settings.setBoardSeenNoteId(2)
        val repository = repository()

        // The first thing the new server says is the backfill for a note
        // nobody has touched: content_seq = board_seq.
        repository.applyNote(noteDto(id = 1, boardSeq = 38, contentSeq = 38))
        runCurrent()

        assertThat(settings.state.first().boardSeenContentSeq).isEqualTo(40)
        val marks = settings.state.first().badgeMarks()
        assertThat(BoardBadge.unreadCount(noteDao.observeNotes().first().marks(), marks))
            .isEqualTo(0)
    }

    /** A fresh install has shown nothing, so there is nothing to seed and
     *  the board is correctly new to it. */
    @Test
    fun `a device that never showed the board seeds no mark`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 1, boardSeq = 38, contentSeq = 38))
        runCurrent()

        assertThat(settings.state.first().boardSeenContentSeq).isEqualTo(0)
        val marks = settings.state.first().badgeMarks()
        assertThat(BoardBadge.unreadCount(noteDao.observeNotes().first().marks(), marks))
            .isEqualTo(1)
    }
}

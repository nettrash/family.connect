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
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.BoardResponse
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.TaskItemDto
import me.nettrash.familyconnect.data.net.dto.TaskItemsCodec
import me.nettrash.familyconnect.data.net.dto.TaskLineRequest
import me.nettrash.familyconnect.data.net.dto.NoteMentionsCodec
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
        BoardRepository(boardApi, noteDao, db.memberDao(), settings, socket, backgroundScope)

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

    /**
     * Issue #69: a device that ran a build from before kinds cached a photo
     * note as a blank text note — at the same seq the server still has. The
     * identical copy must repair it, not be refused as "not newer".
     */
    @Test
    fun `a note cached before kinds is repaired by the same seq`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(noteDto(id = 12, text = "", boardSeq = 90))
        val repaired = repository.applyNote(
            noteDto(
                id = 12, text = "", boardSeq = 90, kind = "photo",
                attachment = me.nettrash.familyconnect.data.net.dto.AttachmentDto(
                    id = 34, kind = "photo", mime = "image/jpeg", size = 1234, hasPreview = true,
                ),
            ),
        )
        runCurrent()

        assertThat(repaired).isTrue()
        assertThat(noteDao.findById(12)!!.kind).isEqualTo("photo")
        assertThat(noteDao.findById(12)!!.attachmentJson).isNotNull()
        // An OLDER copy is still refused.
        assertThat(repository.applyNote(noteDto(id = 12, text = "", boardSeq = 80))).isFalse()
        assertThat(noteDao.findById(12)!!.kind).isEqualTo("photo")
    }

    /**
     * A full read REPLACES what is held: a note it leaves out is gone —
     * deleted while this device was not listening — except one held above
     * the read's mark, which arrived after the read was taken.
     */
    @Test
    fun `a full board read removes what it no longer lists`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyNote(noteDto(id = 1, boardSeq = 10))
        repository.applyNote(noteDto(id = 3, boardSeq = 60))
        boardApi.board = me.nettrash.familyconnect.data.net.dto.BoardResponse(
            notes = listOf(noteDto(id = 2, boardSeq = 20)),
            maxBoardSeq = 50,
        )

        repository.loadBoard()
        runCurrent()

        assertThat(noteDao.observeNotes().first().map { it.id }).containsExactly(2L, 3L)
    }

    /**
     * Only a frame and a catch-up page move the board cursor: the answer to
     * this device's own create or move is evidence about that one note, and
     * REST works while the socket is down — exactly when the frames with
     * lower seqs were missed (docs/protocol.md, "Board").
     */
    @Test
    fun `the answer to my own change moves no cursor`() = runTest(dispatcher) {
        val repository = repository()
        settings.setBoardCursor(5)
        boardApi.nextSeq = 100

        repository.addNote("Milk", "yellow", "medium", "plain", 0.4, 0.3)
        runCurrent()
        val created = noteDao.observeNotes().first().single()
        repository.updateNote(created.id, x = 0.6, y = 0.6)
        runCurrent()

        assertThat(settings.current.boardCursor).isEqualTo(5)
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

    /**
     * A TOMBSTONE IS THE LAST WORD (docs/protocol.md, "Board"). Board seqs commit out of order
     * and a catch-up page carries the pre-delete copy, so a client that merely deleted the row
     * was talked out of it by the next answer that mentioned the note — and nothing but a full
     * read took it off the wall again. The web client keeps this set; so does Windows.
     */
    @Test
    fun `a note a tombstone took does not come back when an older copy arrives`() =
        runTest(dispatcher) {
            val repository = repository()

            repository.applyNote(noteDto(id = 1, boardSeq = 10))
            repository.applyNote(noteTombstone(id = 1, boardSeq = 11))
            runCurrent()
            assertThat(noteDao.observeNotes().first()).isEmpty()

            // The copy that was already travelling when the delete happened — a page, a frame,
            // or the answer to somebody else's own change. Its seq is even NEWER than the
            // tombstone's, which is what makes the seq guard alone no defence at all.
            repository.applyNote(noteDto(id = 1, boardSeq = 99, text = "back from the dead"))
            runCurrent()

            assertThat(noteDao.observeNotes().first()).isEmpty()
            assertThat(noteDao.isGone(1)).isTrue()
        }

    /** The second of the protocol's three doors to "gone": a full read that leaves it out. */
    @Test
    fun `a note a full read left out does not come back either`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyNote(noteDto(id = 1, boardSeq = 10))
        repository.applyNote(noteDto(id = 2, boardSeq = 11))
        runCurrent()

        // The wall as it now stands names only note 2 — note 1 was deleted while this device was
        // not listening, and the full read never carries tombstones.
        boardApi.board = BoardResponse(listOf(noteDto(id = 2, boardSeq = 11)), 11)
        repository.loadBoard()
        runCurrent()
        assertThat(noteDao.observeNotes().first().map { it.id }).containsExactly(2L)

        repository.applyNote(noteDto(id = 1, boardSeq = 50))
        runCurrent()

        assertThat(noteDao.observeNotes().first().map { it.id }).containsExactly(2L)
    }

    /** And the third: this client's own DELETE. */
    @Test
    fun `a note this client deleted does not come back on the frame that follows`() =
        runTest(dispatcher) {
            val repository = repository()
            repository.applyNote(noteDto(id = 1, boardSeq = 10))
            runCurrent()

            assertThat(repository.deleteNote(1)).isTrue()
            runCurrent()
            assertThat(noteDao.observeNotes().first()).isEmpty()

            // A frame that was serialised before the delete landed.
            repository.applyNote(noteDto(id = 1, boardSeq = 9))
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

    // MARK: - the members a note names (docs/protocol.md, "Board")

    private suspend fun roster() = db.memberDao().upsertAll(
        listOf(
            MemberEntity(userId = 2L, username = "anna", displayName = "Anna", role = "member"),
            MemberEntity(userId = 3L, username = "bob", displayName = "Bob", role = "member"),
            MemberEntity(
                userId = 4L, username = "gone", displayName = "Junior", role = "member",
                hasLeft = true,
            ),
        ),
    )

    @Test
    fun `a note sends the names its text says`() = runTest(dispatcher) {
        val repository = repository()
        roster()

        repository.addNote("Milk please @Anna", "yellow", "medium", "plain", 0.1, 0.2)
        runCurrent()

        assertThat(boardApi.created.last().mentions)
            .isEqualTo(listOf(MentionDto(2L, "Anna")))
    }

    @Test
    fun `a note that names nobody sends no names at all`() = runTest(dispatcher) {
        val repository = repository()
        roster()

        // Absent, not an empty list: absence is what the wire means by
        // "nobody", and it is also what a PATCH uses to clear.
        repository.addNote("Milk please", "yellow", "medium", "plain", 0.1, 0.2)
        // A name nobody here answers to is text, and a member who has LEFT
        // is not on the roster a name resolves against.
        repository.addNote("Ask @Nobody and @Junior", "yellow", "medium", "plain", 0.1, 0.2)
        runCurrent()

        assertThat(boardApi.created.map { it.mentions }).containsExactly(null, null)
    }

    @Test
    fun `an edit re-decides the names and a text edit naming nobody clears them`() =
        runTest(dispatcher) {
            val repository = repository()
            roster()

            repository.updateNote(1L, text = "Hi @Bob")
            repository.updateNote(1L, text = "Hi everybody")
            runCurrent()

            assertThat(boardApi.patched[0].second.mentions)
                .isEqualTo(listOf(MentionDto(3L, "Bob")))
            // Empty, not absent: absent from a text edit is what CLEARS on
            // the server, and an empty list says the same thing out loud.
            assertThat(boardApi.patched[1].second.mentions).isEmpty()
        }

    @Test
    fun `a move carries no names`() = runTest(dispatcher) {
        val repository = repository()
        roster()

        // Null, so the server leaves the names alone: a note that was
        // dragged says exactly what it said, and anyone may drag one —
        // sending a list here would make a move an author's act.
        repository.updateNote(1L, x = 0.4, y = 0.5)
        runCurrent()

        assertThat(boardApi.patched.last().second.mentions).isNull()
    }

    @Test
    fun `the names a note arrives with are stored with it`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(
            noteDto(id = 1, text = "Hi @Anna", boardSeq = 10, mentions = listOf(MentionDto(2L, "Anna"))),
        )
        runCurrent()
        val stored = noteDao.observeNotes().first().single()
        assertThat(NoteMentionsCodec.decode(stored.mentionsJson))
            .isEqualTo(listOf(MentionDto(2L, "Anna")))

        // A server from before note mentions sends nothing, and this
        // device then draws no names: the same answer `rsvps` gets, and a
        // server that HAS the column always sends a list, `[]` included —
        // so nothing real is lost, and nothing is invented either.
        repository.applyNote(noteDto(id = 1, text = "Hi @Anna", boardSeq = 11))
        runCurrent()
        assertThat(NoteMentionsCodec.decode(noteDao.observeNotes().first().single().mentionsJson))
            .isEmpty()
    }

    // MARK: - task lists (docs/protocol.md, "Board")

    @Test
    fun `a list is pinned with its lines, and an empty list is still a list`() =
        runTest(dispatcher) {
            val repository = repository()

            repository.addNote(
                "Saturday", "green", "medium", "plain", 0.1, 0.2,
                items = listOf(TaskLineRequest(text = "Milk"), TaskLineRequest(text = "Bread")),
            )
            repository.addNote("Sunday", "green", "medium", "plain", 0.3, 0.4, items = emptyList())
            repository.addNote("Milk", "yellow", "medium", "plain", 0.5, 0.6)
            runCurrent()

            val written = boardApi.created
            assertThat(written[0].kind).isEqualTo("tasks")
            assertThat(written[0].items?.map { it.text }).containsExactly("Milk", "Bread").inOrder()
            // `[]`, not absent: an empty list is what makes the note a
            // list, and dropping it would pin a plain sticker.
            assertThat(written[1].kind).isEqualTo("tasks")
            assertThat(written[1].items).isEmpty()
            // And a note that is not a list sends no lines at all.
            assertThat(written[2].kind).isNull()
            assertThat(written[2].items).isNull()
        }

    @Test
    fun `the lines a list arrives with are stored with it`() = runTest(dispatcher) {
        val repository = repository()

        repository.applyNote(
            noteDto(
                id = 1, text = "Saturday", boardSeq = 10, kind = "tasks",
                items = listOf(
                    TaskItemDto(id = 11, text = "Milk", done = true, doneBy = 3L),
                    TaskItemDto(id = 12, text = "Bread"),
                ),
            ),
        )
        runCurrent()
        val stored = noteDao.observeNotes().first().single()
        val items = TaskItemsCodec.decode(stored.itemsJson)
        assertThat(items.map { it.text }).containsExactly("Milk", "Bread").inOrder()
        assertThat(items[0].done).isTrue()
        assertThat(items[0].doneBy).isEqualTo(3L)

        // A LATER frame rewrites them in place — which is how somebody
        // else's tick arrives at all.
        repository.applyNote(
            noteDto(
                id = 1, text = "Saturday", boardSeq = 11, kind = "tasks",
                items = listOf(
                    TaskItemDto(id = 11, text = "Milk", done = true, doneBy = 3L),
                    TaskItemDto(id = 12, text = "Bread", done = true, doneBy = 4L),
                ),
            ),
        )
        runCurrent()
        val after = TaskItemsCodec.decode(noteDao.observeNotes().first().single().itemsJson)
        assertThat(after.count { it.done }).isEqualTo(2)
    }

    @Test
    fun `a tick asks for a state and applies the answer`() = runTest(dispatcher) {
        val repository = repository()
        repository.applyNote(
            noteDto(
                id = 1, text = "Saturday", boardSeq = 10, kind = "tasks",
                items = listOf(TaskItemDto(id = 11, text = "Milk")),
            ),
        )
        runCurrent()
        // The answer has to be NEWER than the row it replaces, as a
        // server's is: the per-note seq guard refuses anything older.
        boardApi.nextSeq = 20

        assertThat(repository.tickTask(noteId = 1, itemId = 11, done = true)).isTrue()
        runCurrent()

        // A STATE, not a toggle: what was asked for is what was sent.
        assertThat(boardApi.ticked).containsExactly(Triple(1L, 11L, true))
        val items = TaskItemsCodec.decode(noteDao.observeNotes().first().single().itemsJson)
        assertThat(items.single().done).isTrue()
    }

    @Test
    fun `an edit sends the lines it keeps, and a move sends none`() = runTest(dispatcher) {
        val repository = repository()

        repository.updateNote(
            1L, text = "Saturday",
            items = listOf(TaskLineRequest(id = 11, text = "Oat milk"), TaskLineRequest(text = "Eggs")),
        )
        repository.updateNote(1L, x = 0.4, y = 0.5)
        runCurrent()

        val kept = boardApi.patched[0].second.items
        assertThat(kept?.map { it.id }).containsExactly(11L, null).inOrder()
        assertThat(kept?.map { it.text }).containsExactly("Oat milk", "Eggs").inOrder()
        // A move leaves the lines alone, as it leaves the names alone.
        assertThat(boardApi.patched[1].second.items).isNull()
    }
}

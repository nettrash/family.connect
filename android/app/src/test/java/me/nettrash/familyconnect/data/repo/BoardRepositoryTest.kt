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
import me.nettrash.familyconnect.testutil.FakeBoardApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.noteDto
import me.nettrash.familyconnect.testutil.noteTombstone
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

        repository.addNote(text = "Milk", color = "yellow", size = "small", x = 0.1, y = 0.2)
        runCurrent()

        assertThat(boardApi.created.single().size).isEqualTo("small")
        assertThat(noteDao.observeNotes().first().single().size).isEqualTo("small")
    }
}

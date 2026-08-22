/*
 * BoardRepository.kt
 * Family Connect (Android)
 *
 * The family board (docs/protocol.md, "Board"): one wall of sticker notes
 * per family, cached locally so it draws instantly and survives a launch
 * offline.
 *
 * Every apply goes through ONE guarded path, [applyNote], for the same
 * reason reactions and edits do: a note is written only when the incoming
 * `board_seq` beats the one held, so an out-of-order frame cannot undo a
 * newer move — two people dragging the same note is the ordinary case, not
 * the exotic one.
 *
 * TOMBSTONES are not stored. The server keeps one so its change feed can
 * say "gone"; a client that has been told simply deletes its row.
 *
 * iOS counterpart: the board section of ChatSyncCoordinator.swift.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.NoteDao
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.BoardApi
import me.nettrash.familyconnect.data.net.dto.NoteDto
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.TimeFormat
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class BoardRepository @Inject constructor(
    private val boardApi: BoardApi,
    private val noteDao: NoteDao,
    private val settings: SettingsRepository,
    socket: ChatSocket,
    @param:AppScope private val scope: CoroutineScope,
) {

    init {
        scope.launch {
            socket.frames.collect { frame ->
                if (frame is ServerFrame.BoardNote) applyNote(frame.note)
            }
        }
    }

    fun observeNotes(): Flow<List<NoteEntity>> = noteDao.observeNotes()

    private suspend fun boardCursor(): Long = settings.state.first().boardCursor

    /**
     * Apply one note under the per-note seq guard. Returns whether anything
     * changed.
     */
    suspend fun applyNote(note: NoteDto): Boolean {
        val existing = noteDao.findById(note.id)
        if (existing != null && note.boardSeq <= existing.boardSeq) return false

        if (note.isTombstone) {
            // The guard covers deletion too: a stale tombstone must not
            // remove a note that has since moved.
            if (existing != null) noteDao.delete(note.id)
            return existing != null
        }

        val authorId = note.authorId
        val text = note.text
        val color = note.color
        val x = note.x
        val y = note.y
        if (authorId == null || text == null || color == null || x == null || y == null) {
            // A live note missing content is a server bug; dropping it
            // beats drawing a blank sticker.
            return false
        }
        val now = System.currentTimeMillis()
        noteDao.upsert(
            NoteEntity(
                id = note.id,
                authorId = authorId,
                text = text,
                color = color,
                x = x,
                y = y,
                createdAt = note.createdAt?.let(TimeFormat::parseTimestamp) ?: existing?.createdAt ?: now,
                updatedAt = note.updatedAt?.let(TimeFormat::parseTimestamp) ?: now,
                boardSeq = note.boardSeq,
            ),
        )
        return true
    }

    /** Full board read — the first open, and any time the cursor is 0. */
    suspend fun loadBoard(): Boolean {
        val board = boardApi.getBoard().okOrNull() ?: return false
        board.notes.forEach { applyNote(it) }
        settings.setBoardCursor(maxOf(boardCursor(), board.maxBoardSeq))
        return true
    }

    /**
     * Board catch-up: after_seq pages until a short page, tombstones
     * included. Mirrors the reaction and edit loops.
     */
    suspend fun catchUpBoard(serverMaxSeq: Long) {
        val cursor = boardCursor()
        if (cursor == 0L) {
            // Nothing applied yet: one full read beats replaying the whole
            // history of every note that ever existed.
            loadBoard()
            return
        }
        if (serverMaxSeq <= cursor) return
        var after = cursor
        while (true) {
            val page = boardApi.getBoardChanges(after, BOARD_PAGE).okOrNull()?.notes ?: return
            page.forEach { applyNote(it) }
            val pageMax = page.maxOfOrNull { it.boardSeq }
            if (pageMax != null) {
                settings.setBoardCursor(maxOf(boardCursor(), pageMax))
                after = pageMax
            }
            if (page.size < BOARD_PAGE) return
        }
    }

    suspend fun addNote(text: String, color: String, x: Double, y: Double): Boolean =
        when (val result = boardApi.createNote(text, color, x, y)) {
            is ApiResult.Ok -> {
                applyNote(result.value.note)
                settings.setBoardCursor(maxOf(boardCursor(), result.value.note.boardSeq))
                true
            }
            else -> false
        }

    /**
     * Move (anyone) or rewrite (the author) — which fields are sent is what
     * the server checks permission against, so nulls must not be sent.
     */
    suspend fun updateNote(
        id: Long,
        text: String? = null,
        color: String? = null,
        x: Double? = null,
        y: Double? = null,
    ): Boolean = when (val result = boardApi.patchNote(id, text, color, x, y)) {
        is ApiResult.Ok -> {
            applyNote(result.value.note)
            settings.setBoardCursor(maxOf(boardCursor(), result.value.note.boardSeq))
            true
        }
        else -> false
    }

    suspend fun deleteNote(id: Long): Boolean = when (boardApi.deleteNote(id)) {
        is ApiResult.Ok -> {
            noteDao.delete(id)
            true
        }
        else -> false
    }

    private companion object {
        const val BOARD_PAGE = 200
    }
}

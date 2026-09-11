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
import me.nettrash.familyconnect.data.db.MemberDao
import me.nettrash.familyconnect.data.db.NoteDao
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.BoardApi
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.NoteDto
import me.nettrash.familyconnect.data.net.dto.NoteMentionsCodec
import me.nettrash.familyconnect.data.net.dto.RsvpCodec
import me.nettrash.familyconnect.data.net.dto.TaskItemsCodec
import me.nettrash.familyconnect.data.net.dto.TaskLineRequest
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.BoardBadge
import me.nettrash.familyconnect.util.badgeMarks
import me.nettrash.familyconnect.util.MemberMention
import me.nettrash.familyconnect.util.TimeFormat
import me.nettrash.familyconnect.util.marks
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class BoardRepository @Inject constructor(
    private val boardApi: BoardApi,
    private val noteDao: NoteDao,
    /**
     * The roster a note's names are resolved against (docs/protocol.md,
     * "Board").
     */
    private val memberDao: MemberDao,
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

    /**
     * Whether this process has already decided about seeding the badge's
     * content mark. Not a "seeded" flag: the answer "there was nothing to
     * seed" is just as final, and re-asking would read the whole board on
     * every applied note.
     */
    private var contentMarkChecked = false

    fun observeNotes(): Flow<List<NoteEntity>> = noteDao.observeNotes()

    private suspend fun boardCursor(): Long = settings.state.first().boardCursor

    /**
     * Apply one note under the per-note seq guard. Returns whether anything
     * changed.
     */
    suspend fun applyNote(note: NoteDto): Boolean {
        // Before the first note carrying a content seq is written, and only
        // ever once per process: a device that had shown this board before
        // the field existed needs its badge mark seeded, or the server's
        // backfill badges the whole wall (BoardBadge.contentMarkSeed).
        seedContentMarkIfNeeded()
        val existing = noteDao.findById(note.id)
        // STRICTLY older is refused; the SAME seq is written when it would
        // change the row. An equal seq is the same server state — except to
        // a row cached before this device knew a field: a photo note or an
        // event stored by a build from before kinds is a blank text note at
        // that very seq, and refusing the identical copy left it blank for
        // good (issue #69). The comparison below keeps an unchanged copy a
        // no-op, as it always was.
        if (existing != null && note.boardSeq < existing.boardSeq) return false

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
        val entity =
            NoteEntity(
                id = note.id,
                authorId = authorId,
                text = text,
                color = color,
                // Absent from an older server means "medium", the size
                // every note had before there was one; an unknown NAME is
                // kept as-is and the screen draws it as medium, the same
                // forgiveness color gets.
                size = note.size ?: "medium",
                // Absent from an older server means "plain", the face every
                // note was written in before the field existed.
                font = note.font ?: "plain",
                // A server from before kinds sends none, and every note it
                // has is a text note — which is also what an unknown kind
                // DRAWS as (docs/protocol.md, "Board").
                kind = note.kind ?: "text",
                attachmentJson = note.attachment?.let { AttachmentsCodec.encode(listOf(it)) },
                startsAt = note.startsAt?.let(TimeFormat::parseTimestamp),
                endsAt = note.endsAt?.let(TimeFormat::parseTimestamp),
                place = note.place,
                rsvpsJson = note.rsvps?.let(RsvpCodec::encode),
                mentionsJson = note.mentions?.let(NoteMentionsCodec::encode),
                itemsJson = note.items?.let(TaskItemsCodec::encode),
                x = x,
                y = y,
                createdAt = note.createdAt?.let(TimeFormat::parseTimestamp) ?: existing?.createdAt ?: now,
                updatedAt = note.updatedAt?.let(TimeFormat::parseTimestamp) ?: now,
                boardSeq = note.boardSeq,
                // A server from before content seqs sends none, and 0 is
                // how this table spells "nobody said" — the badge then
                // judges the note by its id, as it always did.
                contentSeq = note.contentSeq ?: 0L,
            )
        if (existing != null && note.boardSeq == existing.boardSeq && entity == existing) return false
        noteDao.upsert(entity)
        return true
    }

    /**
     * One-time repair of the badge marks after the update that brought
     * content seqs (docs/protocol.md, "Board"). The flag makes it one read
     * of the notes table per process at most, and only on a device that has
     * a seed to do.
     */
    private suspend fun seedContentMarkIfNeeded() {
        if (contentMarkChecked) return
        contentMarkChecked = true
        val marks = settings.state.first().badgeMarks()
        val seed = BoardBadge.contentMarkSeed(noteDao.observeNotes().first().marks(), marks)
            ?: return
        settings.setBoardSeenContentSeq(seed)
    }

    /** Full board read — the first open, and any time the cursor is 0. */
    suspend fun loadBoard(): Boolean {
        val board = boardApi.getBoard().okOrNull() ?: return false
        // It REPLACES what is held (docs/protocol.md, "Board"): the read
        // never returns tombstones, so a note it leaves out is a note that is
        // gone, and merely applying what it did return kept every note
        // deleted while this device was not listening. A note held ABOVE the
        // read's mark arrived after the read was taken, and stays.
        noteDao.deleteNotListed(board.maxBoardSeq, board.notes.map { it.id })
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

    /**
     * The members a note's text names (docs/protocol.md, "Board").
     *
     * Resolved HERE rather than on the screen, so a note written from
     * anywhere in the app names the same people: the names are read off
     * the text against the live roster, exactly as a message's are.
     */
    private suspend fun namedMembers(text: String): List<MentionDto> {
        if (!text.contains('@')) return emptyList()
        val roster = memberDao.activeMembers().map { MentionDto(it.userId, it.displayName) }
        return MemberMention.resolve(text, roster)
    }

    suspend fun addNote(
        text: String,
        color: String,
        size: String,
        font: String,
        x: Double,
        y: Double,
        attachmentId: Long? = null,
        startsAt: String? = null,
        endsAt: String? = null,
        place: String? = null,
        /**
         * A task list's lines — what makes this a list. Empty is still a
         * list; null is not one (docs/protocol.md, "Board").
         */
        items: List<TaskLineRequest>? = null,
    ): Boolean =
        when (
            val result =
                boardApi.createNote(
                    text, color, size, font, x, y, attachmentId, startsAt, endsAt, place,
                    namedMembers(text), items,
                )
        ) {
            is ApiResult.Ok -> {
                // The answer to this device's own change moves NO cursor
                // (docs/protocol.md, "Board"): it says nothing about another
                // note's lower seq, and REST works while the socket is down —
                // exactly when the frames carrying those were missed.
                applyNote(result.value.note)
                true
            }
            else -> false
        }

    /**
     * Move (anyone) or rewrite (the author) — which fields are sent is what
     * the server checks permission against, so nulls must not be sent. A
     * MOVE sends only x/y; size travels with the author's edit, beside
     * text and color, because how loudly a note speaks is the writer's
     * call (docs/protocol.md, "Board").
     */
    /**
     * Say whether this member is coming — null retracts. ANY member may,
     * which is why it is not `updateNote` (docs/protocol.md, "Board").
     */
    suspend fun answerNote(id: Long, answer: String?): Boolean =
        when (val result = boardApi.answerNote(id, answer)) {
            is ApiResult.Ok -> {
                applyNote(result.value.note)
                true
            }
            else -> false
        }

    suspend fun updateNote(
        id: Long,
        text: String? = null,
        color: String? = null,
        size: String? = null,
        font: String? = null,
        x: Double? = null,
        y: Double? = null,
        /**
         * REPLACES a task list's lines, and the author's like its title. A
         * move sends none, which leaves them alone (docs/protocol.md,
         * "Board").
         */
        items: List<TaskLineRequest>? = null,
    ): Boolean = when (
        val result = boardApi.patchNote(
            id, text, color, size, font, x, y,
            // A text edit carries the names again — they are re-decided on
            // every one, and a text patch without them clears them. A move
            // sends none, so a dragged note keeps the names it had
            // (docs/protocol.md, "Board").
            mentions = text?.let { namedMembers(it) },
            items = items,
        )
    ) {
        is ApiResult.Ok -> {
            // Like a create's answer: applied, and moving no cursor.
            applyNote(result.value.note)
            true
        }
        else -> false
    }

    /**
     * Tick or untick one line. ANY member may, which is why this is not
     * `updateNote` — ticking is not authorship, and it is a STATE rather
     * than a toggle (docs/protocol.md, "Board").
     */
    suspend fun tickTask(noteId: Long, itemId: Long, done: Boolean): Boolean = when (
        val result = boardApi.tickTask(noteId, itemId, done)
    ) {
        is ApiResult.Ok -> {
            applyNote(result.value.note)
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

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
 * One board PER FAMILY since several families (docs/protocol.md, "Several
 * families"): every note is filed under its `family_id`, the screen shows
 * the current family's, each family is caught up on its own cursor, and a
 * write goes to the family of the note it is about.
 *
 * iOS counterpart: the board section of ChatSyncCoordinator.swift.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.GoneNoteEntity
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
import me.nettrash.familyconnect.data.settings.boardCursor
import me.nettrash.familyconnect.data.settings.familyKey
import me.nettrash.familyconnect.data.settings.localFamilyKey
import me.nettrash.familyconnect.data.settings.pathFamily
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

    /** The CURRENT family's board, following the switcher. */
    @OptIn(ExperimentalCoroutinesApi::class)
    fun observeNotes(): Flow<List<NoteEntity>> =
        settings.state.map { it.familyKey }.distinctUntilChanged().flatMapLatest { noteDao.observeNotes(it) }

    /**
     * ONE family's board, whichever is shown — for a caller that has already
     * decided which family it means and must not be moved off it by the
     * switcher between two reads (the badge's marks are per family).
     */
    fun observeNotes(familyId: Long): Flow<List<NoteEntity>> = noteDao.observeNotes(familyId)

    private suspend fun boardCursor(familyId: Long): Long = settings.state.first().boardCursor(familyId)

    /** The family shown — where a new note goes. */
    private suspend fun currentFamily(): Long = settings.state.first().familyKey

    /** How a request path names [familyId]: its id, or nothing on a server from before. */
    private suspend fun path(familyId: Long): Long? = settings.state.first().pathFamily(familyId)

    /** The family a held note is on — a write goes to ITS board. */
    private suspend fun noteFamily(noteId: Long): Long =
        noteDao.findById(noteId)?.familyId?.takeIf { it > 0L } ?: currentFamily()

    /** The path of the family a held note is on. */
    private suspend fun notePath(noteId: Long): Long? = path(noteFamily(noteId))

    /**
     * Apply one note under the per-note seq guard. Returns whether anything
     * changed.
     *
     * Filed under the note's own `family_id`; a note from a server that
     * predates the field is the family [familyHint] names — the one whose
     * board was read — or the family shown.
     */
    suspend fun applyNote(note: NoteDto, familyHint: Long? = null): Boolean {
        // Before the first note carrying a content seq is written, and only
        // ever once per process: a device that had shown this board before
        // the field existed needs its badge mark seeded, or the server's
        // backfill badges the whole wall (BoardBadge.contentMarkSeed).
        seedContentMarkIfNeeded()
        // A TOMBSTONE IS THE LAST WORD. Board seqs commit out of order and a catch-up page
        // carries the pre-delete copy, so without this an older answer crossing the tombstone on
        // the wire puts a note the family took down back on the wall (docs/protocol.md, "Board":
        // "never brought back by an older copy of itself arriving late").
        if (noteDao.isGone(note.id)) return false
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
            // The guard above covers deletion too: a stale tombstone never
            // reaches here, so it removes nothing and remembers nothing —
            // the note it names legitimately outlived it. A tombstone for a
            // note this device never HELD does get remembered, because the
            // copy it is about may still be on its way.
            noteDao.remember(GoneNoteEntity(note.id))
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
        val familyId = settings.state.first().localFamilyKey(note.familyId ?: familyHint ?: existing?.familyId)
        val entity =
            NoteEntity(
                familyId = familyId,
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
        val familyId = currentFamily()
        val marks = settings.state.first().badgeMarks(familyId)
        val seed = BoardBadge.contentMarkSeed(noteDao.observeNotes(familyId).first().marks(), marks)
            ?: return
        settings.setBoardSeenContentSeq(familyId, seed)
    }

    /** Full board read of [familyId] — the first open, and any time its cursor is 0. */
    suspend fun loadBoard(familyId: Long? = null): Boolean {
        val family = familyId ?: currentFamily()
        val board = boardApi.getBoard(path(family)).okOrNull() ?: return false
        // It REPLACES what is held (docs/protocol.md, "Board"): the read
        // never returns tombstones, so a note it leaves out is a note that is
        // gone, and merely applying what it did return kept every note
        // deleted while this device was not listening. A note held ABOVE the
        // read's mark arrived after the read was taken, and stays.
        val listed = board.notes.map { it.id }
        // Read BEFORE they are dropped: a note the read left out is a note that is gone, and it
        // is remembered as gone for the same reason a tombstone is.
        noteDao.idsNotListed(family, board.maxBoardSeq, listed).forEach {
            noteDao.remember(GoneNoteEntity(it))
        }
        noteDao.deleteNotListed(family, board.maxBoardSeq, listed)
        board.notes.forEach { applyNote(it, familyHint = family) }
        settings.setBoardCursor(family, maxOf(boardCursor(family), board.maxBoardSeq))
        return true
    }

    /**
     * Board catch-up of [familyId]: after_seq pages until a short page,
     * tombstones included, on that family's own cursor. Mirrors the reaction
     * and edit loops.
     */
    suspend fun catchUpBoard(serverMaxSeq: Long, familyId: Long? = null) {
        val family = familyId ?: currentFamily()
        val cursor = boardCursor(family)
        if (cursor == 0L) {
            // Nothing applied yet: one full read beats replaying the whole
            // history of every note that ever existed.
            loadBoard(family)
            return
        }
        if (serverMaxSeq <= cursor) return
        var after = cursor
        while (true) {
            val page = boardApi.getBoardChanges(path(family), after, BOARD_PAGE).okOrNull()?.notes ?: return
            page.forEach { applyNote(it, familyHint = family) }
            val pageMax = page.maxOfOrNull { it.boardSeq }
            if (pageMax != null) {
                settings.setBoardCursor(family, maxOf(boardCursor(family), pageMax))
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
    // Against [familyId]'s roster — the family the note is ON, which the
    // caller has already settled — never a fresh read of the family shown:
    // a switch between the two reads would name members of the wrong family
    // on the right board.
    private suspend fun namedMembers(text: String, familyId: Long): List<MentionDto> {
        if (!text.contains('@')) return emptyList()
        val roster = memberDao.activeMembers(familyId).map { MentionDto(it.userId, it.displayName) }
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
        /**
         * The family whose board this note goes on — null for the family
         * shown, read ONCE here. A caller whose write began before a network
         * round trip passes the family it began in: a photo's upload is
         * claimed by the family it named (docs/protocol.md, "Uploads name
         * their family"), and a switch while it was going up must not send
         * the note to another family's board, where the server answers
         * `attachment_not_found` and the pin is lost.
         */
        familyId: Long? = null,
    ): Boolean {
        // One read for the path, the names and the filing alike — two reads
        // of "the family shown" are two chances for a switch to land between.
        val family = familyId ?: currentFamily()
        return when (
            val result =
                boardApi.createNote(
                    path(family), text, color, size, font, x, y, attachmentId, startsAt, endsAt, place,
                    namedMembers(text, family), items,
                )
        ) {
            is ApiResult.Ok -> {
                // The answer to this device's own change moves NO cursor
                // (docs/protocol.md, "Board"): it says nothing about another
                // note's lower seq, and REST works while the socket is down —
                // exactly when the frames carrying those were missed.
                applyNote(result.value.note, familyHint = family)
                true
            }
            else -> false
        }
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
        when (val result = boardApi.answerNote(notePath(id), id, answer)) {
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
    ): Boolean {
        // The note's own family, once, for the path and the names alike.
        val family = noteFamily(id)
        return when (
            val result = boardApi.patchNote(
                path(family), id, text, color, size, font, x, y,
                // A text edit carries the names again — they are re-decided on
                // every one, and a text patch without them clears them. A move
                // sends none, so a dragged note keeps the names it had
                // (docs/protocol.md, "Board").
                mentions = text?.let { namedMembers(it, family) },
                items = items,
            )
        ) {
            is ApiResult.Ok -> {
                // Like a create's answer: applied, and moving no cursor.
                applyNote(result.value.note, familyHint = family)
                true
            }
            else -> false
        }
    }

    /**
     * Tick or untick one line. ANY member may, which is why this is not
     * `updateNote` — ticking is not authorship, and it is a STATE rather
     * than a toggle (docs/protocol.md, "Board").
     */
    suspend fun tickTask(noteId: Long, itemId: Long, done: Boolean): Boolean = when (
        val result = boardApi.tickTask(notePath(noteId), noteId, itemId, done)
    ) {
        is ApiResult.Ok -> {
            applyNote(result.value.note)
            true
        }
        else -> false
    }

    /**
     * Ask the assistant for a backdrop. The AUTHOR's, and drawn from the
     * note's own title (docs/protocol.md, "Board").
     *
     * Answers with the PICTURE — null when none arrived. A redraw replaces
     * the picture with a new attachment, and the dialog that asked holds the
     * note as it was when it opened: without the new one it would keep
     * drawing the old picture, or none, which is what made asking again look
     * like nothing happening.
     *
     * A title the provider's own filter refused is told apart from every
     * other failure (`picture_refused`), because it reads differently: the
     * same title gets the same refusal, so the author is told to put it
     * another way rather than that it merely did not draw.
     */
    suspend fun drawBackdrop(noteId: Long): BackdropOutcome = when (
        val result = boardApi.drawBackdrop(notePath(noteId), noteId)
    ) {
        is ApiResult.Ok -> {
            applyNote(result.value.note)
            result.value.note.attachment
                ?.let { BackdropOutcome.Drawn(it) }
                ?: BackdropOutcome.Failed
        }
        else -> BackdropOutcome.ofFailure(result)
    }

    suspend fun deleteNote(id: Long): Boolean = when (boardApi.deleteNote(notePath(id), id)) {
        is ApiResult.Ok -> {
            // The third of the protocol's three doors to "gone": a tombstone, a full read that
            // left it out, and this client's own DELETE. The frame that confirms it arrives
            // later and finds the id already remembered, which is the point.
            noteDao.remember(GoneNoteEntity(id))
            noteDao.delete(id)
            true
        }
        else -> false
    }

    private companion object {
        const val BOARD_PAGE = 200
    }
}

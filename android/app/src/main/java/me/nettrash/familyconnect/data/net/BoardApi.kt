/*
 * BoardApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Board endpoint table of docs/protocol.md.
 * Interface + impl split so repository tests can substitute a scripted
 * fake without an HTTP stack.
 *
 * iOS counterpart: the board methods on ios/FamilyConnect/Core/APIClient.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.BoardChangesResponse
import me.nettrash.familyconnect.data.net.dto.BoardResponse
import me.nettrash.familyconnect.data.net.dto.CreateNoteRequest
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.NoteResponse
import me.nettrash.familyconnect.data.net.dto.PatchNoteRequest
import javax.inject.Inject
import javax.inject.Singleton
import me.nettrash.familyconnect.data.net.dto.RsvpRequest
import me.nettrash.familyconnect.data.net.dto.TaskDoneRequest
import me.nettrash.familyconnect.data.net.dto.TaskLineRequest
import me.nettrash.familyconnect.ui.board.NoteKinds

interface BoardApi {
    /** The whole board, tombstones excluded. */
    suspend fun getBoard(): ApiResult<BoardResponse>

    /** The board catch-up, tombstones INCLUDED. */
    suspend fun getBoardChanges(afterSeq: Long, limit: Int): ApiResult<BoardChangesResponse>

    suspend fun createNote(
        text: String,
        color: String,
        size: String,
        font: String,
        x: Double,
        y: Double,
        /** The picture, on a photo note: the kind rides with it. */
        attachmentId: Long? = null,
        /** An event's own three. `startsAt` is what makes this an event. */
        startsAt: String? = null,
        endsAt: String? = null,
        place: String? = null,
        /** The members the text names (docs/protocol.md, "Board"). */
        mentions: List<MentionDto> = emptyList(),
        /**
         * A task list's lines — what makes this a list, the way [startsAt]
         * makes a note an event. Empty is still a list; null is not one
         * (docs/protocol.md, "Board").
         */
        items: List<TaskLineRequest>? = null,
    ): ApiResult<NoteResponse>

    /**
     * Say whether the caller is coming — `going`, `maybe`, `no`, or null to
     * retract. ANY member may (docs/protocol.md, "Board").
     */
    suspend fun answerNote(id: Long, answer: String?): ApiResult<NoteResponse>

    /**
     * Null fields are omitted, which is what decides the permission
     * applied: `size` and `font` ride with text and color as author's
     * fields, so a move must leave them null.
     */
    suspend fun patchNote(
        id: Long,
        text: String?,
        color: String?,
        size: String?,
        font: String?,
        x: Double?,
        y: Double?,
        /**
         * REPLACES the names, and rides with a text edit — null on a move,
         * which leaves them alone (docs/protocol.md, "Board").
         */
        mentions: List<MentionDto>? = null,
        /**
         * REPLACES a task list's lines, and the author's like its title: a
         * line carrying its id keeps its TICK (docs/protocol.md, "Board").
         */
        items: List<TaskLineRequest>? = null,
    ): ApiResult<NoteResponse>

    /**
     * Tick or untick one line of a task list. ANY member may; ticking is
     * not authorship, and it is a STATE rather than a toggle so two phones
     * cannot undo each other (docs/protocol.md, "Board").
     */
    suspend fun tickTask(noteId: Long, itemId: Long, done: Boolean): ApiResult<NoteResponse>

    /**
     * Ask the assistant for a picture to sit behind an EVENT, drawn from
     * the note's own title — the AUTHOR's, and nothing to send
     * (docs/protocol.md, "Board").
     */
    suspend fun drawBackdrop(noteId: Long): ApiResult<NoteResponse>

    suspend fun deleteNote(id: Long): ApiResult<Unit>
}

/**
 * What a new note SENDS, as a value — the one piece of this adapter that
 * decides anything, so it is a function a test can hold (docs/protocol.md,
 * "Board").
 *
 * What it decides is the KIND: a note carries one when it is a picture, an
 * event or a task list, and none at all when it is words on a sticker,
 * which is what an absent kind means and what a client predating kinds
 * sends. The three signals arrive as the fields that make each kind
 * meaningful — the picture, the start, the lines.
 */
internal fun newNoteRequest(
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
    mentions: List<MentionDto> = emptyList(),
    items: List<TaskLineRequest>? = null,
): CreateNoteRequest = CreateNoteRequest(
    text, color, size, x, y, font,
    kind = when {
        startsAt != null -> NoteKinds.EVENT
        attachmentId != null -> NoteKinds.PHOTO
        items != null -> NoteKinds.TASKS
        else -> null
    },
    attachmentId = attachmentId,
    startsAt = startsAt,
    endsAt = endsAt,
    place = place,
    // Absent when nobody is named, as the wire has it.
    mentions = mentions.ifEmpty { null },
    // NOT `ifEmpty { null }`: an empty list IS a list, and dropping it
    // would pin a plain sticker instead (docs/protocol.md, "Board").
    items = items,
)

@Singleton
class DefaultBoardApi @Inject constructor(
    private val client: ApiClient,
) : BoardApi {

    override suspend fun getBoard(): ApiResult<BoardResponse> =
        client.get("/families/mine/board")

    override suspend fun getBoardChanges(
        afterSeq: Long,
        limit: Int,
    ): ApiResult<BoardChangesResponse> =
        client.get("/families/mine/board/changes?after_seq=$afterSeq&limit=$limit")

    override suspend fun createNote(
        text: String,
        color: String,
        size: String,
        font: String,
        x: Double,
        y: Double,
        attachmentId: Long?,
        startsAt: String?,
        endsAt: String?,
        place: String?,
        mentions: List<MentionDto>,
        items: List<TaskLineRequest>?,
    ): ApiResult<NoteResponse> =
        client.post(
            "/families/mine/board/notes",
            newNoteRequest(
                text = text, color = color, size = size, font = font, x = x, y = y,
                attachmentId = attachmentId,
                startsAt = startsAt, endsAt = endsAt, place = place,
                mentions = mentions, items = items,
            ),
        )

    override suspend fun answerNote(id: Long, answer: String?): ApiResult<NoteResponse> =
        if (answer == null) {
            client.delete("/families/mine/board/notes/$id/rsvp")
        } else {
            client.put("/families/mine/board/notes/$id/rsvp", RsvpRequest(answer))
        }

    override suspend fun patchNote(
        id: Long,
        text: String?,
        color: String?,
        size: String?,
        font: String?,
        x: Double?,
        y: Double?,
        mentions: List<MentionDto>?,
        items: List<TaskLineRequest>?,
    ): ApiResult<NoteResponse> =
        client.patch(
            "/families/mine/board/notes/$id",
            PatchNoteRequest(
                text, color, size, x, y, font,
                mentions = mentions?.ifEmpty { null },
                // An empty list here CLEARS the lines, which is what
                // removing the last one means.
                items = items,
            ),
        )

    override suspend fun tickTask(
        noteId: Long,
        itemId: Long,
        done: Boolean,
    ): ApiResult<NoteResponse> =
        client.put(
            "/families/mine/board/notes/$noteId/tasks/$itemId",
            TaskDoneRequest(done),
        )

    override suspend fun drawBackdrop(noteId: Long): ApiResult<NoteResponse> =
        client.postEmpty("/families/mine/board/notes/$noteId/backdrop")

    override suspend fun deleteNote(id: Long): ApiResult<Unit> =
        client.delete("/families/mine/board/notes/$id")
}

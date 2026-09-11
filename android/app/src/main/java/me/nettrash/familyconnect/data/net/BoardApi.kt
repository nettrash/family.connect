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
    ): ApiResult<NoteResponse>

    suspend fun deleteNote(id: Long): ApiResult<Unit>
}

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
    ): ApiResult<NoteResponse> =
        client.post(
            "/families/mine/board/notes",
            CreateNoteRequest(
                text, color, size, x, y, font,
                kind = when {
                    startsAt != null -> NoteKinds.EVENT
                    attachmentId != null -> NoteKinds.PHOTO
                    else -> null
                },
                attachmentId = attachmentId,
                startsAt = startsAt,
                endsAt = endsAt,
                place = place,
                mentions = mentions.ifEmpty { null },
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
    ): ApiResult<NoteResponse> =
        client.patch(
            "/families/mine/board/notes/$id",
            PatchNoteRequest(
                text, color, size, x, y, font,
                mentions = mentions?.ifEmpty { null },
            ),
        )

    override suspend fun deleteNote(id: Long): ApiResult<Unit> =
        client.delete("/families/mine/board/notes/$id")
}

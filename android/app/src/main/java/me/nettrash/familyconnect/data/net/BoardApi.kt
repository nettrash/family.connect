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
import me.nettrash.familyconnect.data.net.dto.NoteResponse
import me.nettrash.familyconnect.data.net.dto.PatchNoteRequest
import javax.inject.Inject
import javax.inject.Singleton

interface BoardApi {
    /** The whole board, tombstones excluded. */
    suspend fun getBoard(): ApiResult<BoardResponse>

    /** The board catch-up, tombstones INCLUDED. */
    suspend fun getBoardChanges(afterSeq: Long, limit: Int): ApiResult<BoardChangesResponse>

    suspend fun createNote(
        text: String,
        color: String,
        size: String,
        x: Double,
        y: Double,
    ): ApiResult<NoteResponse>

    /**
     * Null fields are omitted, which is what decides the permission
     * applied: `size` rides with text and color as an author's field, so
     * a move must leave it null.
     */
    suspend fun patchNote(
        id: Long,
        text: String?,
        color: String?,
        size: String?,
        x: Double?,
        y: Double?,
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
        x: Double,
        y: Double,
    ): ApiResult<NoteResponse> =
        client.post("/families/mine/board/notes", CreateNoteRequest(text, color, size, x, y))

    override suspend fun patchNote(
        id: Long,
        text: String?,
        color: String?,
        size: String?,
        x: Double?,
        y: Double?,
    ): ApiResult<NoteResponse> =
        client.patch("/families/mine/board/notes/$id", PatchNoteRequest(text, color, size, x, y))

    override suspend fun deleteNote(id: Long): ApiResult<Unit> =
        client.delete("/families/mine/board/notes/$id")
}

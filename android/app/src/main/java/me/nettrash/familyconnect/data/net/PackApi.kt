/*
 * PackApi.kt
 * Family Connect (Android)
 *
 * Suspend wrappers over the Sticker pack endpoint table of
 * docs/protocol.md. Interface + impl split so repository tests can
 * substitute a scripted fake without an HTTP stack — the same shape as
 * BoardApi, whose sync machinery the pack borrows whole.
 *
 * `pack` and not `sticker` in every name here: in this codebase "sticker"
 * already means a board note.
 *
 * iOS counterpart: the pack methods on ios/FamilyConnect/Core/APIClient.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AddPackItemRequest
import me.nettrash.familyconnect.data.net.dto.PackChangesResponse
import me.nettrash.familyconnect.data.net.dto.PackItemResponse
import me.nettrash.familyconnect.data.net.dto.PackResponse
import javax.inject.Inject
import javax.inject.Singleton

interface PackApi {
    /** The whole pack, tombstones excluded, in the order added. */
    suspend fun getPack(): ApiResult<PackResponse>

    /** The pack catch-up, tombstones INCLUDED. */
    suspend fun getPackChanges(afterSeq: Long, limit: Int): ApiResult<PackChangesResponse>

    /**
     * Claim an upload for the pack. ANY member may. `201` and `200` are
     * both success and both carry the item: a `200` is the pack already
     * holding those bytes, and the item then names the attachment the pack
     * HAD rather than the one sent (docs/protocol.md, "Sticker pack").
     */
    suspend fun addItem(attachmentId: Long, label: String?): ApiResult<PackItemResponse>

    /** Whoever added it, or the family owner. Idempotent. */
    suspend fun removeItem(id: Long): ApiResult<Unit>
}

@Singleton
class DefaultPackApi @Inject constructor(
    private val client: ApiClient,
) : PackApi {

    override suspend fun getPack(): ApiResult<PackResponse> =
        client.get("/families/mine/pack")

    override suspend fun getPackChanges(
        afterSeq: Long,
        limit: Int,
    ): ApiResult<PackChangesResponse> =
        client.get("/families/mine/pack/changes?after_seq=$afterSeq&limit=$limit")

    override suspend fun addItem(attachmentId: Long, label: String?): ApiResult<PackItemResponse> =
        client.post(
            "/families/mine/pack",
            // An empty label is no label, and the wire's spelling of "none"
            // is absence.
            AddPackItemRequest(attachmentId, label?.trim()?.ifEmpty { null }),
        )

    override suspend fun removeItem(id: Long): ApiResult<Unit> =
        client.delete("/families/mine/pack/$id")
}

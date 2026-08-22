/*
 * AvatarApi.kt
 * Family Connect (Android)
 *
 * The protocol's only binary surface (docs/protocol.md — "Profile
 * pictures"): raw image bytes up, raw image bytes down. Everything else
 * in the app speaks JSON.
 *
 * `fetch` takes the version purely to put it in the URL — the server
 * ignores `?v=`, but a version-stamped URL means an HTTP cache can never
 * hand back the previous picture after a change.
 *
 * A user with no picture and a user the caller may not see answer the
 * same 404 by design, so callers must treat 404 as "no picture" rather
 * than as an error worth showing.
 *
 * iOS counterpart: the avatar methods on ios/FamilyConnect/Core/APIClient.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AvatarResponse
import javax.inject.Inject
import javax.inject.Singleton

interface AvatarApi {
    /** PUT /me/avatar — square JPEG bytes → the user with a bumped version. */
    suspend fun upload(jpeg: ByteArray): ApiResult<AvatarResponse>

    /** DELETE /me/avatar — idempotent; version returns to 0. */
    suspend fun delete(): ApiResult<Unit>

    /** GET /users/{id}/avatar — the stored bytes, or a 404 for no picture. */
    suspend fun fetch(userId: Long, version: Long): ApiResult<ByteArray>
}

@Singleton
class DefaultAvatarApi @Inject constructor(
    private val client: ApiClient,
) : AvatarApi {

    override suspend fun upload(jpeg: ByteArray): ApiResult<AvatarResponse> =
        client.decode(client.rawUpload("PUT", "/me/avatar", jpeg, JPEG_CONTENT_TYPE))

    override suspend fun delete(): ApiResult<Unit> =
        client.delete("/me/avatar")

    override suspend fun fetch(userId: Long, version: Long): ApiResult<ByteArray> =
        client.rawDownload("/users/$userId/avatar?v=$version")

    companion object {
        const val JPEG_CONTENT_TYPE = "image/jpeg"
    }
}

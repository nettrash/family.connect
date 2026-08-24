/*
 * AttachmentApi.kt
 * Family Connect (Android)
 *
 * Photos and videos on the wire (docs/protocol.md, "Photos, videos and files").
 *
 * Metadata rides in the QUERY STRING and the bytes are the whole body —
 * no multipart, so neither side has to assemble or parse one and the
 * server can stream straight to disk. The upload is a file, not a byte
 * array: a 100 MB video must never be held whole in memory.
 *
 * Uploading and sending are separate steps. The bytes go up first and the
 * message that follows claims them by id; an attachment nothing claims is
 * swept by the server after a day.
 *
 * iOS counterpart: the attachment methods on ios/FamilyConnect/Core/APIClient.swift
 */

package me.nettrash.familyconnect.data.net

import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentResponse
import java.io.File
import java.net.URLEncoder
import java.util.Locale
import javax.inject.Inject
import javax.inject.Singleton

interface AttachmentApi {
    /** POST /attachments — the bytes, streamed from [file]. */
    suspend fun upload(
        file: File,
        mime: String,
        kind: String,
        width: Int?,
        height: Int?,
        durationMs: Int?,
        /** Required for `kind=file`, ignored otherwise. */
        name: String? = null,
    ): ApiResult<AttachmentResponse>

    /**
     * POST /attachments?kind=location — the one upload with nothing to
     * upload.
     *
     * A location is three numbers, and they go in the query string like
     * every other piece of attachment metadata; there is no body, no file
     * to stream and delete, and nothing to fetch back afterwards
     * (docs/protocol.md, "Locations"). Its own method rather than an
     * argument to [upload], whose whole subject is a `File`.
     */
    suspend fun uploadLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        name: String?,
    ): ApiResult<AttachmentResponse>

    /** PUT /attachments/{id}/preview — the bubble's thumbnail. */
    suspend fun uploadPreview(attachmentId: Long, jpeg: ByteArray): ApiResult<Unit>

    /** GET the bytes into [destination]; 404 means "not ours to see". */
    suspend fun download(attachmentId: Long, preview: Boolean, destination: File): ApiResult<Unit>

    /** The URL + auth headers a video player streams from itself. */
    suspend fun streamUrl(attachmentId: Long): Pair<String, Map<String, String>>?
}

@Singleton
class DefaultAttachmentApi @Inject constructor(
    private val client: ApiClient,
) : AttachmentApi {

    override suspend fun upload(
        file: File,
        mime: String,
        kind: String,
        width: Int?,
        height: Int?,
        durationMs: Int?,
        name: String?,
    ): ApiResult<AttachmentResponse> {
        val query = buildList {
            add("kind=$kind")
            width?.let { add("width=$it") }
            height?.let { add("height=$it") }
            durationMs?.let { add("duration_ms=$it") }
            // Percent-encoded: a name has spaces, umlauts and & in it, and
            // this is a query string.
            name?.let { add("name=" + URLEncoder.encode(it, "UTF-8")) }
        }.joinToString("&")
        return client.decode(client.rawUploadFile("POST", "/attachments?$query", file, mime))
    }

    override suspend fun uploadLocation(
        latitude: Double,
        longitude: Double,
        accuracyM: Int?,
        name: String?,
    ): ApiResult<AttachmentResponse> {
        val query = buildList {
            add("kind=${AttachmentDto.KIND_LOCATION}")
            add("latitude=${coordinate(latitude)}")
            add("longitude=${coordinate(longitude)}")
            accuracyM?.let { add("accuracy_m=$it") }
            name?.takeIf { it.isNotEmpty() }
                ?.let { add("name=" + URLEncoder.encode(it, "UTF-8")) }
        }.joinToString("&")
        return client.decode(client.raw("POST", "/attachments?$query", null))
    }

    /**
     * A coordinate as the wire wants it: decimal degrees with a full stop
     * for the point, and enough places to be exact to a centimetre.
     *
     * `Locale.ROOT` is the whole point — a device set to a comma decimal
     * separator would otherwise send "55,7558" and the server would refuse
     * a perfectly good coordinate.
     */
    private fun coordinate(value: Double): String = String.format(Locale.ROOT, "%.7f", value)

    override suspend fun uploadPreview(attachmentId: Long, jpeg: ByteArray): ApiResult<Unit> =
        client.decode(
            client.rawUpload("PUT", "/attachments/$attachmentId/preview", jpeg, PREVIEW_MIME),
        )

    override suspend fun download(
        attachmentId: Long,
        preview: Boolean,
        destination: File,
    ): ApiResult<Unit> {
        val suffix = if (preview) "/preview" else ""
        return client.rawDownloadToFile("/attachments/$attachmentId$suffix", destination)
    }

    override suspend fun streamUrl(attachmentId: Long): Pair<String, Map<String, String>>? =
        client.authorizedUrl("/attachments/$attachmentId")

    companion object {
        const val PREVIEW_MIME = "image/jpeg"
    }
}

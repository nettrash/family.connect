/*
 * ApiClient.kt
 * Family Connect (Android)
 *
 * The one REST core. Builds OkHttp requests against `{serverUrl}/api/v1`,
 * attaches the Bearer token, encodes/decodes bodies with
 * kotlinx-serialization, and maps every outcome onto the closed
 * ApiResult type so callers never see raw exceptions:
 *
 *   Ok(value)                        — 2xx, decoded
 *   HttpError(status, code, message) — non-2xx, error body parsed per
 *                                      docs/protocol.md ("Error shape")
 *   NetworkError(cause)              — transport / decode failure
 *
 * A 401 on an *authenticated* call means the session is gone (protocol:
 * "A 401 means the session is gone"). It is broadcast on `unauthorized`
 * so SessionRepository can wipe local state exactly once, no matter which
 * request tripped it. Unauthenticated calls (login/register/probe) 401
 * routinely and must not trigger the wipe — nor must the one 401 that is
 * a wrong PASSWORD rather than a dead session, `invalid_credentials` on
 * /me/password and /me/delete. See `isSessionGone`.
 *
 * OkHttp directly, no Retrofit/Ktor: the house convention (see
 * exchange-android / Scan.Android) is one HTTP stack, and OkHttp already
 * carries the WebSocket — a second abstraction would add surface, not
 * ability.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ApiClient.swift
 */

package me.nettrash.familyconnect.data.net

import dagger.hilt.android.qualifiers.ApplicationContext
import android.content.Context
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.net.dto.ErrorBody
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.TokenStore
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import java.io.File
import java.io.IOException
import java.time.Duration
import javax.inject.Inject
import javax.inject.Singleton

sealed class ApiResult<out T> {
    data class Ok<T>(val value: T) : ApiResult<T>()
    data class HttpError(
        val status: Int,
        val code: String?,
        val message: String?,
    ) : ApiResult<Nothing>()

    data class NetworkError(val cause: Throwable) : ApiResult<Nothing>()

    /** The success value, or null for any failure. */
    fun okOrNull(): T? = (this as? Ok<T>)?.value
}

@Singleton
class ApiClient @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val client: OkHttpClient,
    val json: Json,
    private val tokenStore: TokenStore,
    private val settings: SettingsRepository,
) {

    /**
     * Fires once per authenticated 401 — the "session is gone" signal.
     * Buffered so an emit never suspends inside the request path.
     */
    val unauthorized = MutableSharedFlow<Unit>(extraBufferCapacity = 1)

    /** REST base derived from the saved server URL, or null before setup. */
    suspend fun apiBase(): String? =
        settings.state.first().serverUrl?.let(ServerUrlNormalizer::apiBase)

    // -- Typed wrappers ------------------------------------------------------

    suspend inline fun <reified T> get(
        path: String,
        auth: Boolean = true,
        overrideBase: String? = null,
    ): ApiResult<T> = decode(raw("GET", path, null, auth, overrideBase))

    suspend inline fun <reified B, reified T> post(
        path: String,
        body: B,
        auth: Boolean = true,
    ): ApiResult<T> = decode(raw("POST", path, json.encodeToString(body), auth))

    /** POST with an empty body (approve / reject / rotate / leave / logout). */
    suspend inline fun <reified T> postEmpty(path: String): ApiResult<T> =
        decode(raw("POST", path, null))

    suspend inline fun <reified B, reified T> put(
        path: String,
        body: B,
    ): ApiResult<T> = decode(raw("PUT", path, json.encodeToString(body)))

    suspend inline fun <reified B, reified T> patch(
        path: String,
        body: B,
    ): ApiResult<T> = decode(raw("PATCH", path, json.encodeToString(body)))

    suspend inline fun <reified T> delete(path: String): ApiResult<T> =
        decode(raw("DELETE", path, null))

    /** Decode a raw body; `Unit` responses (204s) skip parsing entirely. */
    inline fun <reified T> decode(result: ApiResult<String>): ApiResult<T> = when (result) {
        is ApiResult.Ok -> {
            if (T::class == Unit::class) {
                @Suppress("UNCHECKED_CAST")
                ApiResult.Ok(Unit as T)
            } else {
                try {
                    ApiResult.Ok(json.decodeFromString<T>(result.value))
                } catch (e: Exception) {
                    // A 2xx we can't decode is a server/client mismatch,
                    // not an HTTP error — surface it as transport-level.
                    ApiResult.NetworkError(e)
                }
            }
        }
        is ApiResult.HttpError -> result
        is ApiResult.NetworkError -> result
    }

    // -- Core ------------------------------------------------------------------

    /**
     * Execute one request. `overrideBase` lets the server-setup probe hit
     * a candidate URL before anything is saved to settings.
     */
    suspend fun raw(
        method: String,
        path: String,
        jsonBody: String?,
        auth: Boolean = true,
        overrideBase: String? = null,
    ): ApiResult<String> {
        val body = when {
            jsonBody != null -> jsonBody.toRequestBody(JSON_MEDIA_TYPE)
            method == "GET" || method == "DELETE" -> null
            // OkHttp requires a body for POST/PATCH even when the
            // endpoint takes none.
            else -> ByteArray(0).toRequestBody(null)
        }
        return send(method, path, body, auth, overrideBase) { it.body.string() }
    }

    /**
     * Send raw bytes — the profile-picture upload, the protocol's only
     * non-JSON request body. The response still is JSON, so it comes back
     * as text for `decode`.
     */
    suspend fun rawUpload(
        method: String,
        path: String,
        bytes: ByteArray,
        contentType: String,
    ): ApiResult<String> =
        send(method, path, bytes.toRequestBody(contentType.toMediaType()), auth = true, overrideBase = null) {
            it.body.string()
        }

    /** Fetch a binary response body (profile pictures, previews). */
    suspend fun rawDownload(path: String): ApiResult<ByteArray> =
        send("GET", path, null, auth = true, overrideBase = null) { it.body.bytes() }

    /**
     * Send a FILE as the request body — the attachment upload.
     *
     * `asRequestBody` streams from disk: a 100 MB video is never held in
     * memory in one piece, on a phone that also has to keep the chat
     * scrolling. That is the whole reason this exists next to [rawUpload],
     * which takes the bytes it is given.
     */
    suspend fun rawUploadFile(
        method: String,
        path: String,
        file: File,
        contentType: String,
    ): ApiResult<String> =
        send(
            method,
            path,
            file.asRequestBody(contentType.toMediaType()),
            auth = true,
            overrideBase = null,
            timeout = UPLOAD_TIMEOUT,
        ) { it.body.string() }

    /**
     * Stream a response body straight into [destination].
     *
     * Writes to a `.part` file and renames on completion, so a download cut
     * off half-way can never be mistaken for a cached photo — the same
     * trick the server uses on the way in (server/src/storage.rs).
     */
    suspend fun rawDownloadToFile(path: String, destination: File): ApiResult<Unit> =
        send("GET", path, null, auth = true, overrideBase = null) { response ->
            val part = File(destination.parentFile, destination.name + ".part")
            part.parentFile?.mkdirs()
            try {
                response.body.byteStream().use { input ->
                    part.outputStream().use { output -> input.copyTo(output) }
                }
                if (!part.renameTo(destination)) {
                    part.delete()
                    throw IOException("could not move the download into place")
                }
            } catch (e: Exception) {
                part.delete()
                throw e
            }
        }

    /**
     * An absolute URL plus the headers a media player needs to fetch it
     * itself. The video is streamed by the player, not downloaded by us:
     * playback starts in a second and a 90 MB clip never lands on disk.
     */
    suspend fun authorizedUrl(path: String): Pair<String, Map<String, String>>? {
        val base = apiBase() ?: return null
        val headers = tokenStore.load()
            ?.let { mapOf("Authorization" to "Bearer $it") }
            .orEmpty()
        return base + path to headers
    }

    /**
     * The one place a request is actually executed: shared so the 401
     * broadcast and the protocol error-body parsing can never diverge
     * between the JSON and the binary paths.
     *
     * `onSuccess` runs while the response is still open — it must
     * materialize the body (string/bytes), not hand back a stream.
     */
    private suspend fun <T> send(
        method: String,
        path: String,
        body: RequestBody?,
        auth: Boolean,
        overrideBase: String?,
        timeout: Duration = Duration.ZERO,
        onSuccess: (Response) -> T,
    ): ApiResult<T> = withContext(Dispatchers.IO) {
        val base = overrideBase ?: apiBase()
            ?: return@withContext ApiResult.NetworkError(
                IllegalStateException("No server URL configured"),
            )

        val builder = Request.Builder().url(base + path)
        if (auth) {
            tokenStore.load()?.let { builder.header("Authorization", "Bearer $it") }
        }
        // OkHttp sends no Accept-Language of its own — unlike URLSession on
        // the Apple clients, which is why only this side needs it. The
        // server uses it to answer assistant questions in the language the
        // device is actually being used in.
        acceptLanguage()?.let { builder.header("Accept-Language", it) }
        builder.method(method, body)

        // An upload of up to 100 MB over a home connection outlives the
        // default read/write timeouts; ZERO leaves the shared client's
        // own values in place for every other call.
        val call = if (timeout == Duration.ZERO) {
            client
        } else {
            client.newBuilder()
                .writeTimeout(timeout)
                .readTimeout(timeout)
                .callTimeout(timeout)
                .build()
        }

        try {
            call.newCall(builder.build()).execute().use { response ->
                if (response.isSuccessful) {
                    ApiResult.Ok(onSuccess(response))
                } else {
                    val parsed = try {
                        json.decodeFromString<ErrorBody>(response.body.string()).error
                    } catch (_: Exception) {
                        null
                    }
                    if (isSessionGone(response.code, parsed?.code, auth)) {
                        unauthorized.tryEmit(Unit)
                    }
                    ApiResult.HttpError(response.code, parsed?.code, parsed?.message)
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            ApiResult.NetworkError(e)
        }
    }

    companion object {
        /**
         * The one 401 that is NOT a dead session: the caller proved they
         * are signed in and then got their PASSWORD wrong.
         */
        const val INVALID_CREDENTIALS = "invalid_credentials"

        /**
         * Does this response mean the SESSION is gone — the signal that
         * wipes local state and returns the app to sign-in?
         *
         * A 401 on an authenticated call almost always does (protocol.md,
         * "Auth": "A 401 means the session is gone"), and that is how
         * every other device of a deleted account, or one whose session an
         * owner's password reset revoked, finds out on its next call.
         *
         * The exception is `invalid_credentials`, which the server sends
         * for a WRONG PASSWORD on `POST /me/password` and
         * `POST /me/delete` — the caller's session is perfectly alive and
         * they mistyped. Broadcasting that signed the user out over a
         * typo: the screens turn it into a sentence, and the wipe
         * underneath them threw the session away anyway. Everything else
         * at 401 — `unauthorized`, or no parseable body at all — still
         * counts, because "the server would not say" is not a reason to
         * keep believing in a session.
         *
         * Unauthenticated calls (login, register, the server-setup probe)
         * 401 routinely and never count.
         */
        fun isSessionGone(status: Int, errorCode: String?, auth: Boolean): Boolean =
            auth && status == 401 && errorCode != INVALID_CREDENTIALS

        val JSON_MEDIA_TYPE = "application/json; charset=utf-8".toMediaType()

        /** Long enough for a 100 MB video on a slow upstream link. */
        val UPLOAD_TIMEOUT: Duration = Duration.ofMinutes(10)
    }
    /**
     * The app's own resolved locale as an IETF tag, e.g. `ru-RU`.
     *
     * Taken from the CONFIGURATION rather than Locale.getDefault(): what
     * matters is the language the app is actually being shown in, which is
     * what the per-app language setting changes and what the member sees.
     */
    private fun acceptLanguage(): String? {
        val locales = context.resources.configuration.locales
        if (locales.isEmpty) return null
        return locales.get(0)?.toLanguageTag()?.takeIf { it.isNotBlank() && it != "und" }
    }


}
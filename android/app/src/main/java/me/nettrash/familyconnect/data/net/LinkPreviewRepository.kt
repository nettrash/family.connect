/*
 * LinkPreviewRepository.kt
 * Family Connect (Android)
 *
 * Fetches the card shown under a message's first web link.
 *
 * PRIVACY, up front: this is the one place the app talks to a host the
 * family does not own. Every device that displays a linked message
 * contacts that link — chosen deliberately (the alternative designs
 * need the server or the wire format to carry the preview), and
 * switchable off in Settings, which is why callers check the flag
 * before asking for anything. Requests are stripped down accordingly:
 * no cookies (the app's OkHttp client has no CookieJar), no redirect
 * chain beyond OkHttp's default, and a body read that stops after
 * MAX_HTML_BYTES.
 *
 * One repository for the app, so a link posted in a busy chat is
 * fetched once no matter how many bubbles render it. Results —
 * including failures, cached so a dead link is not retried on every
 * scroll — live in memory only: a preview is derived data, and
 * persisting it would outlive the message it belongs to.
 *
 * iOS counterpart: ios/FamilyConnect/Core/LinkPreviewLoader.swift.
 */

package me.nettrash.familyconnect.data.net

import android.graphics.BitmapFactory
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.ui.chat.LinkPreview
import me.nettrash.familyconnect.ui.chat.LinkPreviewParser
import okhttp3.OkHttpClient
import okhttp3.Request
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

/** What the UI knows about one link. */
sealed interface LinkPreviewState {
    data object Loading : LinkPreviewState
    data class Loaded(val preview: LinkPreview, val image: ImageBitmap?) : LinkPreviewState
    data object Unavailable : LinkPreviewState
}

@Singleton
class LinkPreviewRepository @Inject constructor(
    okHttp: OkHttpClient,
    @AppScope private val scope: CoroutineScope,
) {

    private val _states = MutableStateFlow<Map<String, LinkPreviewState>>(emptyMap())

    /** Every known link's state; bubbles read their own URL out of it. */
    val states: StateFlow<Map<String, LinkPreviewState>> = _states

    private val inFlight = mutableSetOf<String>()

    // Short timeouts and no retry: a preview is a nicety, and a slow
    // site must never hold a coroutine (or the user's attention).
    private val client = okHttp.newBuilder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .callTimeout(20, TimeUnit.SECONDS)
        .retryOnConnectionFailure(false)
        .build()

    /**
     * Ask for [url]'s preview, starting the single app-wide fetch the
     * first time it is seen. Safe to call from composition.
     */
    fun request(url: String) {
        // https only, matching iOS: plain http is blocked there by ATS,
        // so allowing it here would show a card on one platform and not
        // the other for the same message — and silently fetching a
        // cleartext URL somebody else chose is the wrong default for
        // the one request this app makes off the family's own server.
        if (!url.startsWith("https://")) {
            publish(url, LinkPreviewState.Unavailable)
            return
        }
        synchronized(inFlight) {
            if (url in _states.value || url in inFlight) return
            inFlight += url
        }
        publish(url, LinkPreviewState.Loading)
        scope.launch {
            val preview = fetchPreview(url)
            // The image is fetched BEFORE publishing, so a card appears
            // in one step; landing text first and the image later grows
            // the bubble twice.
            val image = preview?.imageUrl?.let { fetchImage(it) }
            synchronized(inFlight) { inFlight -= url }
            publish(url, if (preview == null) {
                LinkPreviewState.Unavailable
            } else {
                LinkPreviewState.Loaded(preview, image)
            })
        }
    }

    /**
     * `update` rather than `value = value + …`: fetches settle on
     * arbitrary IO threads, and a read-modify-write between two of them
     * loses one result permanently — the bubble asks exactly once, so
     * a dropped entry means a card that never appears.
     */
    private fun publish(url: String, state: LinkPreviewState) {
        _states.update { current ->
            // Drop the oldest half when the table fills instead of
            // wiping it: a wipe makes every card on screen vanish, and
            // the per-bubble LaunchedEffect never re-fires to rebuild
            // them.
            val base = if (current.size >= MAX_ENTRIES) {
                current.entries.drop(MAX_ENTRIES / 2).associate { it.key to it.value }
            } else {
                current
            }
            base + (url to state)
        }
    }

    private suspend fun fetchPreview(url: String): LinkPreview? = withContext(Dispatchers.IO) {
        runCatching {
            val request = Request.Builder()
                .url(url)
                .header("Accept", "text/html,application/xhtml+xml")
                // Bots get the metadata without the consent walls and
                // personalization a browser UA invites.
                .header("User-Agent", "FamilyConnect/1.0 (+link-preview; like WhatsApp)")
                .build()
            client.newCall(request).execute().use { response ->
                if (!response.isSuccessful) return@use null
                val contentType = response.body?.contentType()
                val isHtml = contentType == null ||
                    contentType.type == "text" ||
                    contentType.subtype.contains("html")
                if (!isHtml) return@use null
                // peekBody reads at most this many bytes and leaves the
                // rest un-downloaded.
                val html = response.peekBody(MAX_HTML_BYTES).string()
                // Redirects land on response.request.url; resolve
                // relative images and the host label against where we
                // actually ended up.
                LinkPreviewParser.parse(html, response.request.url.toString())
            }
        }.getOrNull()
    }

    private suspend fun fetchImage(url: String): ImageBitmap? = withContext(Dispatchers.IO) {
        runCatching {
            val request = Request.Builder().url(url).build()
            client.newCall(request).execute().use { response ->
                if (!response.isSuccessful) return@use null
                val bytes = response.peekBody(MAX_IMAGE_BYTES).bytes()
                decodeScaled(bytes)
            }
        }.getOrNull()
    }

    /**
     * Decode at roughly card size rather than full resolution: a
     * 4000px hero image would otherwise cost ~64MB of heap per bubble.
     * Both axes are laddered — a panorama is wide, a poster is tall,
     * and sampling on width alone leaves the tall one enormous. OOM is
     * an Error, so runCatching upstream would not hold it.
     */
    private fun decodeScaled(bytes: ByteArray): ImageBitmap? {
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
        var sample = 1
        while (bounds.outWidth / sample > TARGET_IMAGE_WIDTH * 2 ||
            bounds.outHeight / sample > TARGET_IMAGE_WIDTH * 2
        ) {
            sample *= 2
        }
        val options = BitmapFactory.Options().apply { inSampleSize = sample }
        return try {
            BitmapFactory.decodeByteArray(bytes, 0, bytes.size, options)?.asImageBitmap()
        } catch (_: OutOfMemoryError) {
            null
        }
    }

    private companion object {
        /** <head> is all that is parsed, so the rest is never fetched. */
        const val MAX_HTML_BYTES = 256L * 1024
        const val MAX_IMAGE_BYTES = 4L * 1024 * 1024
        const val TARGET_IMAGE_WIDTH = 600
        const val MAX_ENTRIES = 200
    }
}

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
 * chain beyond OkHttp's default, and a body read that stops at the end
 * of the page's <head> (or after MAX_HTML_BYTES, whichever is first).
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
import java.io.ByteArrayOutputStream
import java.io.InputStream
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
    @param:AppScope private val scope: CoroutineScope,
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
                val contentType = response.body.contentType()
                val isHtml = contentType == null ||
                    contentType.type == "text" ||
                    contentType.subtype.contains("html")
                if (!isHtml) return@use null
                val stream = response.body.byteStream()
                // Stops at the end of <head>, leaving the rest of the
                // page un-downloaded — which is what makes the 1MB cap
                // affordable. This was peekBody(MAX_HTML_BYTES), which
                // always read the flat cap and, at 256K, never reached
                // YouTube's og: tags (#50).
                val bytes = readHead(stream, MAX_HTML_BYTES)
                // Same charset rule ResponseBody.string() applied: the
                // one the response declares, UTF-8 when it declares
                // none or one this JVM does not have.
                val charset = contentType?.charset(Charsets.UTF_8) ?: Charsets.UTF_8
                val html = String(bytes, charset)
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
        /**
         * Ceiling on the page read. It is rarely reached — the read
         * stops at the end of <head> — and exists for a page that never
         * closes its head.
         *
         * It was 256K, and that is precisely why a YouTube link showed
         * no card at all (#50): youtube.com/watch ships ~700K of inline
         * player JSON ahead of its og: tags, so the first 256K contain
         * no og:title and not even a <title>, and the parser correctly
         * refused to build a card out of nothing. Measured on a real
         * watch page: <title> at 704,923, og:title at 706,842, </head>
         * at 715,108 of 1,310,787. Kept equal to
         * LinkPreviewParser.SCAN_LIMIT and to the iOS cap.
         */
        const val MAX_HTML_BYTES = 1024L * 1024
        const val MAX_IMAGE_BYTES = 4L * 1024 * 1024
        const val TARGET_IMAGE_WIDTH = 600
        const val MAX_ENTRIES = 200
    }
}

/**
 * Reads [input] up to the end of the page's `<head>`, or [cap] bytes,
 * whichever comes first.
 *
 * The head stop is what pays for the 1MB cap. Everything the parser
 * reads lives in `<head>`, so the rest of a page is bytes burned on
 * somebody's mobile data, and heads are small even when pages are not:
 * measured over ten real sites, `</head>` lands at 347B (Hacker News),
 * 6.8K (Vimeo), 9.5K (Wikipedia, in a 650K page), 13K (apple.com), 22K
 * (GitHub), 31K (amazon.com, 762K page), 90K (BBC News), 353K (Reddit,
 * 874K page), 619K (the Guardian, 1.28M page) and 715K (YouTube, 1.31M
 * page). Seven of those ten now transfer less than the flat 256K they
 * used to.
 *
 * Internal rather than private so the unit tests can drive it off a
 * ByteArrayInputStream instead of a socket.
 *
 * iOS counterpart: `limitedData(…, stoppingAtEndOfHead:)` in
 * LinkPreviewLoader.swift.
 */
internal fun readHead(input: InputStream, cap: Long): ByteArray {
    val out = ByteArrayOutputStream(minOf(cap, 64L * 1024).toInt())
    val scanner = HeadEndScanner()
    val buffer = ByteArray(16 * 1024)
    var total = 0L
    while (total < cap) {
        val want = minOf(buffer.size.toLong(), cap - total).toInt()
        val read = input.read(buffer, 0, want)
        if (read <= 0) break
        // Scan the chunk for the head end, then write the prefix in one
        // go rather than a byte at a time.
        var stop = -1
        for (index in 0 until read) {
            if (scanner.consume(buffer[index])) {
                stop = index
                break
            }
        }
        if (stop >= 0) {
            out.write(buffer, 0, stop + 1)
            break
        }
        out.write(buffer, 0, read)
        total += read
    }
    return out.toByteArray()
}

/**
 * Spots the end of a page's `<head>` in a byte stream, one byte at a
 * time, so a fetch can stop there without buffering the page first.
 *
 * Deliberately dumber than an HTML tokenizer, and matched to the
 * scanner in LinkPreviewParser: ASCII-only case folding, and a match
 * counts only when the tag NAME ends there, so `<bodyguard>` is not the
 * body. `<body` is honoured as well as `</head`, because the head end
 * tag is optional in HTML and a page that omits it would otherwise be
 * read to the cap.
 *
 * The cost of being dumb is a page that writes the literal text
 * `</head>` or `<body>` inside an inline script in its head: the fetch
 * stops early and the card is lost. That is rare enough to accept (none
 * of the ten pages measured above does it, YouTube included — its
 * inline JSON escapes every `<` as a \u003C escape), and the
 * alternative is a real tokenizer for a nicety feature.
 *
 * iOS counterpart: HeadEndDetector in LinkPreviewLoader.swift.
 */
internal class HeadEndScanner {

    /**
     * The last few folded bytes — one longer than the longest needle,
     * because the byte AFTER the name is what proves the name ended.
     */
    private val window = ByteArray(WINDOW)
    private var filled = 0

    /** True the moment [byte] completes `</head` or `<body`. */
    fun consume(byte: Byte): Boolean {
        val folded = fold(byte)
        if (filled == WINDOW) {
            System.arraycopy(window, 1, window, 0, WINDOW - 1)
            window[WINDOW - 1] = folded
        } else {
            window[filled] = folded
            filled++
        }
        // The newest byte is the boundary; the needle sits just before it.
        if (isNameCharacter(window[filled - 1])) return false
        return endsWithNeedle(HEAD) || endsWithNeedle(BODY)
    }

    private fun endsWithNeedle(needle: ByteArray): Boolean {
        // -1 for the boundary byte, which is not part of the name.
        val end = filled - 1
        if (end < needle.size) return false
        for (index in needle.indices) {
            if (window[end - needle.size + index] != needle[index]) return false
        }
        return true
    }

    private fun fold(byte: Byte): Byte =
        if (byte >= 'A'.code.toByte() && byte <= 'Z'.code.toByte()) (byte + 32).toByte() else byte

    private fun isNameCharacter(byte: Byte): Boolean {
        val value = byte.toInt() and 0xFF
        return (value in 'a'.code..'z'.code) || (value in '0'.code..'9'.code)
    }

    private companion object {
        val HEAD = "</head".toByteArray(Charsets.US_ASCII)
        val BODY = "<body".toByteArray(Charsets.US_ASCII)
        const val WINDOW = 7
    }
}

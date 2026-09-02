/*
 * AttachmentRepository.kt
 * Family Connect (Android)
 *
 * Photos and previews, fetched once and kept.
 *
 * ON DISK, unlike AvatarRepository — and that is the whole reason this is
 * a separate type. A family album is hundreds of photos; a memory-only
 * cache would either evict constantly or grow until the process is killed.
 * The bytes are immutable (an attachment's content never changes), so a
 * file cached under its id is valid forever and survives a relaunch, which
 * is what makes scrolling back through a year of photos feel instant
 * instead of re-downloading.
 *
 * Videos are NOT cached here: they stream straight from the server through
 * the player, which buffers for itself and would gain nothing from a
 * second copy on the phone.
 *
 * The fetch is app-scoped for the same reason avatars are: the caller is a
 * LaunchedEffect that dies when its row scrolls out of the LazyColumn, and
 * a fetch it owned would be cancelled half-way — leaving the key marked
 * in-flight forever, so that photo would never load again.
 *
 * iOS counterpart: ios/FamilyConnect/Core/AttachmentStore.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AttachmentApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AttachmentRepository @Inject constructor(
    @param:ApplicationContext private val context: Context,
    private val attachmentApi: AttachmentApi,
    settings: SettingsRepository,
    connectivity: ConnectivityObserver,
    @param:AppScope private val scope: CoroutineScope,
) {

    data class Key(val attachmentId: Long, val preview: Boolean)

    /** Small hot cache in front of the disk, so a visible bubble does not
     *  re-read and re-decode on every scroll frame. */
    private val hot: SnapshotStateMap<Key, ImageBitmap> = mutableStateMapOf()
    private val order = ArrayDeque<Key>()

    /** Ids the server has nothing for; not retried. */
    private val missing = HashSet<Key>()
    private val inFlight = HashMap<Key, Deferred<Unit>>()
    /** Whole-file downloads in flight, keyed by attachment id. */
    private val fileFetches = HashMap<Long, Deferred<File?>>()
    private val guard = Mutex()

    /** Bumped on every account change — see AvatarRepository's generation. */
    private var generation = 0

    /** Bumped when the network returns, so load effects re-run. */
    var retryToken by mutableIntStateOf(0)
        private set

    private val directory: File
        get() = File(context.cacheDir, CACHE_DIR).apply { mkdirs() }

    init {
        scope.launch {
            settings.state
                .map { it.myUserId }
                .distinctUntilChanged()
                .drop(1)
                .collect { clear() }
        }
        scope.launch {
            connectivity.onAvailable.collect {
                withContext(Dispatchers.Main.immediate) {
                    // Forget what failed while there was no network, or the
                    // bump below would re-run effects that short-circuit.
                    guard.withLock { missing.clear() }
                    retryToken++
                }
            }
        }
    }

    /** The decoded image if it is already in memory, else null. Safe to
     *  call from composition; [load] is what fetches. */
    fun cached(attachmentId: Long, preview: Boolean): ImageBitmap? =
        hot[Key(attachmentId, preview)]

    /**
     * Make sure the image is in memory: from the hot map, from the disk
     * cache, or from the server. Cancelling the caller does NOT cancel a
     * fetch already running.
     *
     * @param mayArriveLate the server answering "no" is not necessarily
     *   the end of it. True for a VIDEO POSTER and nothing else: a poster
     *   is the one image uploaded separately from the message that carries
     *   it (MessageRepository.sendMedia), so a reader can ask a moment
     *   before it lands — and unlike a photo, a video has no second source
     *   of pixels to fall back on. Such a key is re-asked a few times over
     *   [POSTER_RETRY_DELAYS_MS] and then settles into `missing` exactly
     *   like any other 404: a video whose poster failed for good must stop
     *   asking, not poll forever.
     */
    suspend fun load(attachmentId: Long, preview: Boolean, mayArriveLate: Boolean = false) {
        val key = Key(attachmentId, preview)
        val fetch = guard.withLock {
            when {
                hot.containsKey(key) || key in missing -> null
                else -> inFlight.getOrPut(key) { startFetch(key, generation, mayArriveLate) }
            }
        } ?: return
        fetch.join()
    }

    private fun startFetch(
        key: Key,
        startedAt: Int,
        mayArriveLate: Boolean,
    ): Deferred<Unit> = scope.async {
        try {
            // The re-check loop, not a retry of a failed request: every
            // pass here follows a server that ANSWERED, and answered "not
            // here". A key stays in `inFlight` for the whole ladder, so
            // the bubbles asking for it join this one chain instead of
            // starting their own, and a success lands in `hot` — a
            // SnapshotStateMap — which is what repaints them with no
            // token bumped and nothing else touched.
            var attempt = 0
            while (true) {
                val file = fileFor(key)
                var transportFailed = false
                val bitmap = withContext(Dispatchers.IO) {
                    // On disk from a previous run: decode it and skip the
                    // network entirely. Bytes that no longer decode mean a
                    // truncated file, which is worth one re-download.
                    if (file.isFile) {
                        decode(file) ?: run { file.delete(); null }
                    } else {
                        null
                    } ?: run {
                        when (attachmentApi.download(key.attachmentId, key.preview, file)) {
                            is ApiResult.Ok -> decode(file)
                            // A server that ANSWERED said no: no preview
                            // yet, or not ours to see.
                            is ApiResult.HttpError -> null
                            // Nobody answered. NOT final — see below.
                            is ApiResult.NetworkError -> {
                                transportFailed = true
                                null
                            }
                        }
                    }
                }

                val again = withContext(Dispatchers.Main.immediate) {
                    guard.withLock {
                        // The session ended while this was in flight.
                        if (startedAt != generation) return@withLock false
                        if (bitmap != null) {
                            hot[key] = bitmap
                            order.addLast(key)
                            evictIfNeeded()
                            false
                        } else if (!file.isFile && !transportFailed) {
                            // Only a real answer marks a key dead. Marking
                            // a TRANSPORT failure here is what blanked
                            // every photo on screen the moment the phone
                            // left the house: `load` short-circuits on
                            // `missing`, so the key could never be retried
                            // and a @Singleton cache meant not even leaving
                            // the screen recovered it.
                            //
                            // A poster gets the ladder first, and joins
                            // `missing` when it runs out — the budget is
                            // spent once per key per process, because the
                            // key is held in `inFlight` throughout.
                            if (mayArriveLate && attempt < POSTER_RETRY_DELAYS_MS.size) {
                                true
                            } else {
                                missing += key
                                false
                            }
                        } else {
                            false
                        }
                    }
                }
                if (!again) break
                delay(POSTER_RETRY_DELAYS_MS[attempt])
                attempt++
            }
        } finally {
            withContext(Dispatchers.Main.immediate) {
                guard.withLock { inFlight.remove(key) }
            }
        }
    }

    /** Logout: the next account must not see the previous one's photos —
     *  and the FILES are the part that would otherwise survive. */
    fun clear() {
        scope.launch(Dispatchers.Main.immediate) {
            guard.withLock {
                generation++
                hot.clear()
                order.clear()
                missing.clear()
            }
            withContext(Dispatchers.IO) { directory.deleteRecursively() }
        }
    }

    /**
     * A local file for a file attachment, downloading it if this device
     * does not have it yet.
     *
     * Written under its REAL NAME inside a per-attachment directory,
     * because that name is what the app opening it will show and what a
     * share hands on — a cache keyed "34.bin" would send "34.bin". The
     * directory is what keeps two files called `Invoice.pdf` apart.
     *
     * Null when the bytes could not be fetched; the caller says so.
     */
    suspend fun fileFor(attachment: AttachmentDto): File? {
        // One download per attachment however many times it is tapped: a
        // big PDF takes seconds, tapping again is the natural reaction, and
        // two writers on the same `.part` file is a corrupted download.
        val running = guard.withLock {
            fileFetches.getOrPut(attachment.id) { scope.async { downloadFile(attachment) } }
        }
        return try {
            running.await()
        } finally {
            guard.withLock { fileFetches.remove(attachment.id) }
        }
    }

    private suspend fun downloadFile(attachment: AttachmentDto): File? = withContext(Dispatchers.IO) {
        // Shadowing `directory` here would read as recursion; the folder
        // is per-attachment so two `Invoice.pdf`s stay apart.
        val folder = File(File(directory, FILES_DIR), attachment.id.toString())
        val destination = File(folder, safeFileName(attachment.name ?: attachment.fallbackFileName))
        if (destination.isFile) return@withContext destination
        folder.mkdirs()
        when (attachmentApi.download(attachment.id, preview = false, destination = destination)) {
            is ApiResult.Ok -> destination
            else -> null
        }
    }

    /**
     * A filename safe to create on this device.
     *
     * The server sanitises what goes in its header; this is about the local
     * filesystem, where a name with a slash in it silently becomes a path
     * and a leading dot hides the file from every app it is handed to.
     */
    fun safeFileName(name: String): String {
        val cleaned = name
            .replace('/', '_')
            .replace('\\', '_')
            .replace(':', '_')
            .trim()
            .trimStart('.')
        return cleaned.ifEmpty { "file" }.take(255)
    }

    private fun fileFor(key: Key): File {
        val suffix = if (key.preview) "-preview" else ""
        return File(directory, "${key.attachmentId}$suffix.jpg")
    }

    private fun decode(file: File): ImageBitmap? =
        runCatching { AvatarImage.decode(file.readBytes(), DISPLAY_PIXELS)?.asImageBitmap() }
            .getOrNull()

    private fun evictIfNeeded() {
        while (order.size > MAX_ENTRIES) {
            hot.remove(order.removeFirst())
        }
    }

    /** Internal rather than private for one line of it: the poster ladder
     *  is a budget, and the test that pins it has to name the same number
     *  the code spends (AttachmentPosterTest). */
    internal companion object {
        const val CACHE_DIR = "attachments"

        /** Sub-directory of [CACHE_DIR]; matches res/xml/file_paths.xml,
         *  which is what FileProvider is allowed to hand out. */
        const val FILES_DIR = "files"

        /**
         * Longest edge kept decoded in memory. A bubble draws at most
         * 240 dp and the full-screen viewer a phone width, so 2048 (what
         * the uploader sends) would be several times the pixels anyone
         * sees on all but the largest displays.
         */
        const val DISPLAY_PIXELS = 1440

        /** Decoded images held at once — a screenful of a photo-heavy
         *  thread, several times over. The files stay on disk regardless. */
        const val MAX_ENTRIES = 40

        /**
         * How long to wait before asking again for a video poster the
         * server did not have, and — by its length — how many times.
         *
         * Short and finite on purpose. A poster is uploaded just before
         * the message that claims it, so the window in which a reader can
         * be told "no" and be wrong is seconds wide; twelve of them cover
         * it several times over. After the last rung the key settles like
         * any other 404, which is the half that matters most: a video
         * whose poster never made it — the frame grab failed, or its
         * upload did — costs three small requests once per process and
         * then nothing at all, and keeps the play badge it always had.
         */
        val POSTER_RETRY_DELAYS_MS = longArrayOf(2_000, 10_000)
    }
}

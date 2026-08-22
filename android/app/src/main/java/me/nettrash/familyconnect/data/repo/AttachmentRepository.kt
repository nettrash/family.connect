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
                withContext(Dispatchers.Main.immediate) { retryToken++ }
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
     */
    suspend fun load(attachmentId: Long, preview: Boolean) {
        val key = Key(attachmentId, preview)
        val fetch = guard.withLock {
            when {
                hot.containsKey(key) || key in missing -> null
                else -> inFlight.getOrPut(key) { startFetch(key, generation) }
            }
        } ?: return
        fetch.join()
    }

    private fun startFetch(key: Key, startedAt: Int): Deferred<Unit> = scope.async {
        try {
            val file = fileFor(key)
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
                        else -> null
                    }
                }
            }

            withContext(Dispatchers.Main.immediate) {
                guard.withLock {
                    // The session ended while this was in flight.
                    if (startedAt != generation) return@withLock
                    if (bitmap != null) {
                        hot[key] = bitmap
                        order.addLast(key)
                        evictIfNeeded()
                    } else if (!file.isFile) {
                        // No bytes and nothing cached: a 404 (no preview
                        // yet, or not ours to see) or an undecodable body.
                        // Both are final; a transport failure leaves the
                        // key retryable through retryToken.
                        missing += key
                    }
                }
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
    suspend fun fileFor(attachment: AttachmentDto): File? = withContext(Dispatchers.IO) {
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

    private companion object {
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
    }
}

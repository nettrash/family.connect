/*
 * RichRenderer.kt
 * Family Connect (Android)
 *
 * The one way a bubble gets a drawing of a rich block (#97): the caches in
 * front, the hidden web view (RichHost) behind, and the rules that join them.
 *
 *  - Memory first, synchronously ([cached]), so a bubble scrolled back into
 *    view draws its picture in the same frame instead of flashing the source.
 *  - Then disk, then the web view — one job at a time, because the page has
 *    one stage.
 *  - One drawing per key at a time: two bubbles showing `$x^2$` share one
 *    job rather than queueing two.
 *  - A job runs in the app's scope, not the bubble's. A bubble that scrolls
 *    away before its turn does not cancel the job — the drawing is kept, and
 *    is there the moment the bubble comes back — and a cancelled caller can
 *    never leave the web view half way through a snapshot.
 *
 * App-scoped (a Hilt singleton) and handed to the tree through
 * LocalRichRenderer, the way LocalAttachments carries AttachmentRepository.
 */

package me.nettrash.familyconnect.rich

import android.content.Context
import android.util.Log
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.di.AppScope
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class RichRenderer internal constructor(
    private val engine: RichEngine,
    private val disk: RichDiskCache?,
    /** fc-rich.js's VERSION. Blocking (it may read a file); called on [io]. */
    private val versionSource: () -> String,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher = Dispatchers.IO,
    private val memory: RichMemoryCache = RichMemoryCache(),
) {

    @Inject
    constructor(
        @ApplicationContext context: Context,
        @AppScope scope: CoroutineScope,
    ) : this(
        engine = RichHost(context),
        disk = RichDiskCache(File(context.cacheDir, DISK_FOLDER)),
        versionSource = { RichAssets.version(context) },
        scope = scope,
    )

    @Volatile
    private var version: String? = null

    /** The page has one stage: jobs take turns. */
    private val turn = Mutex()

    /** The job producing each key right now. Guarded by itself. */
    private val inFlight = HashMap<String, Deferred<RichResult>>()

    /**
     * What is already in memory for [job] — a drawing, a failure from this
     * launch, or null when it has to be asked for with [draw]. Never does
     * I/O, so a composable can read it while it composes.
     */
    fun cached(job: RichJob): RichResult? {
        val v = version ?: return null
        return memory.get(job.cacheKey(v))
    }

    /** [job]'s drawing, drawing it if nothing has kept it. */
    suspend fun draw(job: RichJob): RichResult {
        // fc-rich.js would answer "empty" — no need to wake a web view to
        // hear it.
        if (job.source.isBlank()) return RichResult.Failed
        val v = version ?: withContext(io) { versionSource() }.also { version = it }
        val key = job.cacheKey(v)
        memory.get(key)?.let { return it }
        val producer = synchronized(inFlight) {
            inFlight[key] ?: scope.async(start = CoroutineStart.LAZY) { produce(key, job) }.also {
                // Registered BEFORE it starts, so its own removal (in
                // `produce`'s finally) can never run first and leave a
                // finished job in the map for good.
                inFlight[key] = it
                it.start()
            }
        }
        return producer.await()
    }

    private suspend fun produce(key: String, job: RichJob): RichResult {
        try {
            disk?.let { store -> withContext(io) { store.read(key) } }?.let {
                memory.put(key, it)
                return it
            }
            val result = turn.withLock {
                memory.get(key) ?: try {
                    engine.render(job)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // The class only: a message could quote the source.
                    Log.w(TAG, "drawing a ${job.kind} failed: ${e.javaClass.simpleName}")
                    RichResult.Failed
                }
            }
            memory.put(key, result)
            if (result is RichResult.Drawn) disk?.let { store -> withContext(io) { store.write(key, result) } }
            return result
        } finally {
            synchronized(inFlight) { inFlight.remove(key) }
        }
    }

    companion object {
        private const val TAG = "RichRenderer"

        /** Under the app's cache folder, which the system may empty. */
        const val DISK_FOLDER = "rich"
    }
}

/** What the app knows about its own copy of web/rich. */
object RichAssets {

    /**
     * `fcRich.version`, read from the bundled fc-rich.js rather than asked
     * of the page: the disk cache must be consulted on a cold start without
     * waking a web view first, and the file the page runs is this file.
     */
    fun version(context: Context): String =
        runCatching {
            context.assets.open("rich/fc-rich.js").bufferedReader().use { versionIn(it.readText()) }
        }.getOrNull() ?: UNKNOWN

    /** The VERSION constant in fc-rich.js's [text], or null. */
    fun versionIn(text: String): String? =
        Regex("""var VERSION = '([^']*)'""").find(text)?.groupValues?.get(1)

    /**
     * When the file cannot be read: still one stable key space, and a
     * drawing kept under it is never mistaken for one from a known engine.
     */
    const val UNKNOWN = "unknown"
}

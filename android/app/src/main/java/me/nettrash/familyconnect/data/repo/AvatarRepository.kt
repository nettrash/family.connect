/*
 * AvatarRepository.kt
 * Family Connect (Android)
 *
 * The profile-picture cache: (userId, avatarVersion) → decoded bitmap,
 * in memory only.
 *
 * The VERSION IS PART OF THE KEY, which is what makes the whole thing
 * safe: a changed picture is a different key, so no entry ever needs
 * invalidating and a stale face can't survive a change. `missing`
 * remembers the 404s (no picture, or not visible to us) so a family of
 * people without photos doesn't re-request on every scroll.
 *
 * Each fetch is an app-scoped `async` keyed by (user, version), and
 * callers merely await it. That is deliberate: the caller is a
 * LaunchedEffect that dies the moment its row scrolls out of the
 * LazyColumn, and a fetch owned by the caller would be cancelled
 * half-way — leaving the key marked in-flight forever, so that face
 * would never load again for the life of the process. Owned by the app
 * scope, a scrolled-away request finishes and lands in the cache instead.
 *
 * Decoding runs on the app scope's Dispatchers.Default for the same
 * reason it must not run on the caller's: the caller is the UI thread.
 *
 * Nothing is written to disk. The pictures are small, the family server
 * is usually on the local network, and a cache on disk would be one more
 * place a face outlives the session it belongs to.
 *
 * Composition reads through `cached`, which is backed by a
 * SnapshotStateMap, so an arriving picture recomposes the avatars that
 * read it. (One map, one state record: every arrival invalidates every
 * reader, not just the one that wanted it. At family scale that is a
 * handful of recompositions, which is cheaper than a state holder per
 * key.)
 *
 * iOS counterpart: ios/FamilyConnect/Core/AvatarStore.swift
 */

package me.nettrash.familyconnect.data.repo

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
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
import me.nettrash.familyconnect.data.net.AvatarApi
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class AvatarRepository @Inject constructor(
    private val avatarApi: AvatarApi,
    settings: SettingsRepository,
    connectivity: ConnectivityObserver,
    @param:AppScope private val scope: CoroutineScope,
) {

    data class Key(val userId: Long, val version: Long)

    private val images: SnapshotStateMap<Key, ImageBitmap> = mutableStateMapOf()

    /** Insertion order for eviction — SnapshotStateMap does not keep one. */
    private val order = ArrayDeque<Key>()

    /** Keys the server has no picture for; not retried. */
    private val missing = HashSet<Key>()

    /** One shared fetch per key, so N avatars of one person fetch once. */
    private val inFlight = HashMap<Key, Deferred<Unit>>()

    /**
     * Bumped on every account change. A fetch that was already running
     * when the session ended carries the generation it started under, and
     * its result is dropped rather than cached — the next account must
     * not inherit a face, even briefly.
     */
    private var generation = 0

    private val guard = Mutex()

    /**
     * Bumped when the network comes back. Composables key their load
     * effect on it, which is what retries a fetch that failed while
     * offline — nothing else would, since a row that stays on screen
     * never re-runs its effect.
     */
    var retryToken by mutableIntStateOf(0)
        private set

    init {
        // Logout, a 401 wipe, or a removal all clear the stored user id;
        // dropping the cache with it means the next account never sees
        // the previous one's faces. `drop(1)` skips the boot emission.
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

    /**
     * The picture if it is already decoded, else null. Safe to call from
     * composition — it only reads snapshot state. Kicking off the fetch
     * is [load]'s job.
     */
    fun cached(userId: Long, version: Long): ImageBitmap? =
        if (version <= 0) null else images[Key(userId, version)]

    /**
     * Fetch and decode unless the picture is already cached, known
     * missing, or already being fetched. Suspends until that is settled;
     * cancelling the caller does NOT cancel the fetch.
     */
    suspend fun load(userId: Long, version: Long) {
        if (version <= 0) return
        val key = Key(userId, version)
        val fetch = guard.withLock {
            when {
                images.containsKey(key) || key in missing -> null
                else -> inFlight.getOrPut(key) { startFetch(key, generation) }
            }
        } ?: return
        fetch.join()
    }

    /** Runs on the app scope: outlives the caller, off the main thread. */
    private fun startFetch(key: Key, startedAt: Int): Deferred<Unit> = scope.async {
        try {
            val result = avatarApi.fetch(key.userId, key.version)
            val bitmap = (result as? ApiResult.Ok)?.value
                ?.let { AvatarImage.decode(it, maxPixels = DISPLAY_PIXELS) }
                ?.asImageBitmap()

            withContext(Dispatchers.Main.immediate) {
                guard.withLock {
                    // The session ended while this was in flight.
                    if (startedAt != generation) return@withLock
                    if (bitmap != null) {
                        images[key] = bitmap
                        order.addLast(key)
                        evictIfNeeded()
                    } else if (
                        (result is ApiResult.HttpError && result.status == 404) ||
                        result is ApiResult.Ok
                    ) {
                        // 404 = no picture (or not ours to see). Ok with
                        // no bitmap = bytes that will not decode, and
                        // re-fetching them on every scroll would not help.
                        // Both are final; any other status — and every
                        // transport failure — stays retryable via
                        // retryToken.
                        missing += key
                    }
                }
            }
        } finally {
            // Not in the block above: this must run even when the fetch
            // throws or the scope is torn down, or the key would stay
            // in-flight forever and that face would never load again.
            withContext(Dispatchers.Main.immediate) {
                guard.withLock { inFlight.remove(key) }
            }
        }
    }

    /** Drop everything — a different account must not inherit faces. */
    fun clear() {
        scope.launch(Dispatchers.Main.immediate) {
            guard.withLock {
                generation++
                images.clear()
                order.clear()
                missing.clear()
                // In-flight fetches are left to finish; the generation
                // stamp is what stops their results from being cached.
            }
        }
    }

    private fun evictIfNeeded() {
        while (order.size > MAX_ENTRIES) {
            val oldest = order.removeFirst()
            images.remove(oldest)
        }
    }

    private companion object {
        /**
         * Longest edge kept in memory. The largest avatar drawn anywhere
         * is 56 dp, so 256 px is generous even at 3x — and a quarter of
         * the bytes of holding the uploaded 512 px square. Matches iOS's
         * AvatarStore.avatarMaxPixels.
         */
        const val DISPLAY_PIXELS = 256

        /**
         * Far more than a family roster; the cap is a runaway guard for a
         * long session accumulating superseded versions, not a budget
         * anyone is expected to hit. 64 × 256 px ARGB_8888 ≈ 16 MB.
         */
        const val MAX_ENTRIES = 64
    }
}

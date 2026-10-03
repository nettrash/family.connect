/*
 * SessionEpoch.kt
 * Family Connect (Android)
 *
 * Which session a late write belongs to. A request still out when the
 * session ends — a sign-out, a 401, an account deleted, a removal from the
 * family — must not write its answer into the database the wipe has just
 * emptied, where the next account on this device would find it
 * (docs/protocol.md, "Transcripts on request": "An answer belongs to the
 * account that asked").
 *
 * The in-memory caches already do this with a generation of their own
 * (AvatarRepository, AttachmentRepository), bumped when the stored user id
 * changes. That signal comes too late for Room: SessionRepository.clearSession
 * wipes the tables BEFORE it resets the settings, so a write landing between
 * the two would survive. This epoch is advanced by SessionRepository itself,
 * before every wipe, and under the same lock a guarded write takes — so a
 * write either finishes before the advance (and the wipe takes it) or sees
 * the new epoch and is dropped. Never one that lands after the wipe.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class SessionEpoch @Inject constructor() {

    private val lock = Mutex()

    @Volatile
    private var value = 0L

    /** The epoch now — taken when a request starts. */
    fun current(): Long = value

    /** The session ends: everything started before this may no longer write. */
    suspend fun advance() {
        lock.withLock { value++ }
    }

    /**
     * [block], only while [startedAt] is still the epoch, and with no
     * [advance] able to run in the middle of it; null when the session that
     * started the request is gone.
     */
    suspend fun <T> whileCurrent(startedAt: Long, block: suspend () -> T): T? =
        lock.withLock { if (startedAt == value) block() else null }
}

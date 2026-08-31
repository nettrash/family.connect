/*
 * BackoffPolicy.kt
 * Family Connect (Android)
 *
 * Pure full-jitter exponential backoff for the WebSocket reconnect loop:
 *
 *     delay = random(0 .. min(cap, base * 2^attempt))
 *
 * Full jitter (rather than plain exponential) so a houseful of clients
 * dropped by the same server restart don't reconnect in lockstep. The
 * Random is injectable so BackoffPolicyTest can pin the exact sequence
 * with a seed.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/BackoffPolicy.swift
 */

package me.nettrash.familyconnect.data.net.ws

import kotlin.math.min
import kotlin.random.Random

class BackoffPolicy(
    private val random: Random = Random.Default,
    private val baseMillis: Long = 1_000L,
    private val capMillis: Long = 30_000L,
) {

    private var attempt: Int = 0

    /** Next delay in milliseconds; each call advances the attempt counter. */
    fun nextDelayMillis(): Long {
        // Shift instead of pow, clamped so a long outage can't overflow.
        val exp = min(attempt, 20)
        val ceiling = min(capMillis, baseMillis shl exp)
        attempt += 1
        // nextLong's upper bound is exclusive; +1 makes the ceiling reachable.
        return random.nextLong(0L, ceiling + 1L)
    }

    /** Call after a successful connect so the next drop starts fresh. */
    fun reset() {
        attempt = 0
    }

    companion object {
        /**
         * How long a connection must last before its next drop is treated as
         * bad luck rather than a broken endpoint.
         *
         * Ten seconds: an accept-then-close returns in milliseconds and can
         * never reach it, while a genuine connection on a slow network earns
         * its reset well inside one heartbeat.
         */
        const val DURABLE_AFTER_MILLIS: Long = 10_000L

        /**
         * Whether a finished connection earned its reset.
         *
         * REACHING [SocketState.Open] IS NOT ENOUGH, and that distinction is
         * the point. A proxy can accept the upgrade and drop immediately; so
         * can this app's own server, which kicks a connection whose send
         * queue overflows with code 1001. Resetting on Open meant every such
         * cycle restarted at random(0..1s), so the ceiling never climbed and
         * the socket reconnected about twice a second indefinitely — each
         * reconnect firing a full resync and a push-token check.
         *
         * Judging by DURATION distinguishes the two without asking the
         * endpoint anything: a connection that carried traffic for a while
         * was real, one that died on arrival was not.
         *
         * @param openedElapsedMillis SystemClock.elapsedRealtime() when the
         *   socket opened, or null if it never did.
         */
        fun earnsReset(
            openedElapsedMillis: Long?,
            nowElapsedMillis: Long,
            durableAfterMillis: Long = DURABLE_AFTER_MILLIS,
        ): Boolean {
            if (openedElapsedMillis == null) return false
            return nowElapsedMillis - openedElapsedMillis >= durableAfterMillis
        }
    }
}

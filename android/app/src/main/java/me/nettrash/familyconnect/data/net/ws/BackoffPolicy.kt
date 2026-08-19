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
}

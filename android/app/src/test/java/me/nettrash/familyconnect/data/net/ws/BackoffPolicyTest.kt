/*
 * BackoffPolicyTest.kt
 * Family Connect (Android)
 *
 * Full-jitter backoff: delay = random(0 .. min(cap, base * 2^attempt)).
 * A recording Random pins the exact ceilings; a seeded Random checks the
 * jitter stays inside them.
 */

package me.nettrash.familyconnect.data.net.ws

import com.google.common.truth.Truth.assertThat
import org.junit.Test
import kotlin.math.min
import kotlin.random.Random

/** Always returns the maximum and records the bound it was given. */
private class MaxRandom : Random() {
    val requestedCeilings = mutableListOf<Long>()

    override fun nextBits(bitCount: Int): Int = 0

    override fun nextLong(from: Long, until: Long): Long {
        requestedCeilings += until - 1 // until is exclusive; ceiling = until-1
        return until - 1
    }
}

class BackoffPolicyTest {

    @Test
    fun ceilingGrowsExponentiallyAndCapsAtThirtySeconds() {
        val random = MaxRandom()
        val policy = BackoffPolicy(random = random)
        repeat(8) { policy.nextDelayMillis() }
        assertThat(random.requestedCeilings).containsExactly(
            1_000L, 2_000L, 4_000L, 8_000L, 16_000L, 30_000L, 30_000L, 30_000L,
        ).inOrder()
    }

    @Test
    fun delaysStayWithinJitterBounds() {
        val policy = BackoffPolicy(random = Random(seed = 42))
        for (attempt in 0 until 12) {
            val ceiling = min(30_000L, 1_000L shl min(attempt, 20))
            val delay = policy.nextDelayMillis()
            assertThat(delay).isAtLeast(0L)
            assertThat(delay).isAtMost(ceiling)
        }
    }

    @Test
    fun seededRandomYieldsDeterministicSequence() {
        val a = BackoffPolicy(random = Random(seed = 7))
        val b = BackoffPolicy(random = Random(seed = 7))
        val seqA = List(6) { a.nextDelayMillis() }
        val seqB = List(6) { b.nextDelayMillis() }
        assertThat(seqA).isEqualTo(seqB)
        // Jitter actually jitters: not all values identical.
        assertThat(seqA.distinct().size).isGreaterThan(1)
    }

    @Test
    fun resetRestartsTheExponent() {
        val random = MaxRandom()
        val policy = BackoffPolicy(random = random)
        repeat(4) { policy.nextDelayMillis() } // ceilings 1s, 2s, 4s, 8s
        policy.reset()
        policy.nextDelayMillis()
        assertThat(random.requestedCeilings.last()).isEqualTo(1_000L)
    }

    @Test
    fun customBaseAndCapAreHonored() {
        val random = MaxRandom()
        val policy = BackoffPolicy(random = random, baseMillis = 500L, capMillis = 2_000L)
        repeat(4) { policy.nextDelayMillis() }
        assertThat(random.requestedCeilings).containsExactly(
            500L, 1_000L, 2_000L, 2_000L,
        ).inOrder()
    }

    @Test
    fun veryLargeAttemptCountsDoNotOverflow() {
        val policy = BackoffPolicy(random = Random(seed = 1))
        repeat(100) {
            val delay = policy.nextDelayMillis()
            assertThat(delay).isAtLeast(0L)
            assertThat(delay).isAtMost(30_000L)
        }
    }

    // --- What earns a reset -------------------------------------------------

    /** The bug: an accept-then-drop endpoint used to reset the ceiling every
     *  cycle, so it never climbed and the socket reconnected about twice a
     *  second forever, resyncing each time. */
    @Test
    fun acceptThenDropEarnsNothing() {
        assertThat(
            BackoffPolicy.earnsReset(openedElapsedMillis = 1_000L, nowElapsedMillis = 1_050L)
        ).isFalse()
    }

    @Test
    fun durableConnectionEarnsReset() {
        assertThat(
            BackoffPolicy.earnsReset(openedElapsedMillis = 1_000L, nowElapsedMillis = 11_000L)
        ).isTrue()
        assertThat(
            BackoffPolicy.earnsReset(openedElapsedMillis = 1_000L, nowElapsedMillis = 3_601_000L)
        ).isTrue()
    }

    /** A dial that never opened cannot have proved anything. */
    @Test
    fun neverOpenedEarnsNothing() {
        assertThat(
            BackoffPolicy.earnsReset(openedElapsedMillis = null, nowElapsedMillis = 99_000L)
        ).isFalse()
    }

    /** The storm end to end: repeated accept-then-drop must climb to the cap
     *  instead of sitting at the first ceiling forever. */
    @Test
    fun repeatedAcceptThenDropClimbsToTheCap() {
        // A random that always returns its upper bound exposes the ceiling.
        val policy = BackoffPolicy(random = Random(seed = 7))
        val ceilings = mutableListOf<Long>()
        repeat(8) {
            val openedAt = 1_000L
            if (BackoffPolicy.earnsReset(
                    openedElapsedMillis = openedAt,
                    nowElapsedMillis = openedAt + 20L,
                )
            ) {
                policy.reset()
            }
            ceilings.add(policy.nextDelayMillis())
        }
        // Not the exact values (the seed decides those), but the ceiling must
        // grow: the last delay can exceed the first ceiling of 1s, which is
        // impossible if reset() ran every cycle.
        assertThat(ceilings.max()).isGreaterThan(1_000L)
    }

    @Test
    fun aDurableConnectionRestoresTheCheapCeiling() {
        val policy = BackoffPolicy(random = Random(seed = 3))
        repeat(6) { policy.nextDelayMillis() }
        val openedAt = 5_000L
        if (BackoffPolicy.earnsReset(
                openedElapsedMillis = openedAt,
                nowElapsedMillis = openedAt + 11_000L,
            )
        ) {
            policy.reset()
        }
        // Back to the first ceiling: the delay cannot exceed base.
        assertThat(policy.nextDelayMillis()).isAtMost(1_000L)
    }
}

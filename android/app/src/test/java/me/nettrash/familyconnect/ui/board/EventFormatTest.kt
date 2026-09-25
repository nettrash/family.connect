/*
 * EventFormatTest.kt
 * Family Connect (Android) — tests
 *
 * An event's when-line and its past rule (docs/protocol.md, "Board").
 *
 * The wire carries an INSTANT, and each reader sees it in their own zone —
 * which is the whole reason the protocol stores a timestamp rather than a
 * local time. What is pinned here is the SHAPE of the answer and the past
 * rule, not one locale's punctuation: a test that asserted a formatted
 * string would be asserting the JDK's locale data, not this code.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import java.time.Instant
import org.junit.Test

class EventFormatTest {

    private val start = Instant.parse("2026-12-24T17:00:00Z").toEpochMilli()
    private val sameDayEnd = Instant.parse("2026-12-24T21:00:00Z").toEpochMilli()
    private val nextDayEnd = Instant.parse("2026-12-25T01:00:00Z").toEpochMilli()

    @Test
    fun `a start alone reads as one moment, and an end adds a second`() {
        val alone = EventFormat.whenLine(start, null)
        val ranged = EventFormat.whenLine(start, sameDayEnd)
        assertThat(alone).isNotEmpty()
        assertThat(ranged).contains("–")
        assertThat(ranged.length).isGreaterThan(alone.length)
    }

    /**
     * An end on ANOTHER day carries its own date; an end the same day is a
     * time alone. A range that said "24 Dec, 17:00 – 01:00" would read as
     * eight hours backwards.
     */
    @Test
    fun `an end on another day says which day`() {
        val sameDay = EventFormat.whenLine(start, sameDayEnd)
        val nextDay = EventFormat.whenLine(start, nextDayEnd)
        assertThat(nextDay.length).isGreaterThan(sameDay.length)
    }

    @Test
    fun `past is decided by the end when there is one, and the start when there is not`() {
        val afterStart = start + 60_000
        val afterEnd = sameDayEnd + 60_000
        // Still running: started, not finished.
        assertThat(EventFormat.isPast(start, sameDayEnd, afterStart)).isFalse()
        assertThat(EventFormat.isPast(start, sameDayEnd, afterEnd)).isTrue()
        // No end: the start is all there is to go on.
        assertThat(EventFormat.isPast(start, null, afterStart)).isTrue()
        assertThat(EventFormat.isPast(start, null, start - 1)).isFalse()
    }
}

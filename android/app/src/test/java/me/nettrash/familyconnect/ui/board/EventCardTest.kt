/*
 * EventCardTest.kt
 * Family Connect (Android)
 *
 * An event is drawn as a CALENDAR ENTRY (docs/protocol.md, "Board"): the
 * date as a block — the day's number with its short month — and the time
 * beside it, which is a different string from the one the card used to
 * carry.
 *
 * The block's shape is the same on all four clients; what each does in its
 * own language is its own, which is why these assertions are about the
 * PARTS rather than about exact words.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import org.junit.Test
import java.time.Instant
import java.time.ZoneId
import java.time.ZonedDateTime

class EventCardTest {

    private val startsAt = ZonedDateTime.of(2026, 12, 24, 16, 0, 0, 0, ZoneId.systemDefault())
        .toInstant()
        .toEpochMilli()
    private val endsAt = Instant.ofEpochMilli(startsAt).plusSeconds(4 * 3600).toEpochMilli()

    @Test
    fun `the block is the day's number and its short month`() {
        val (day, month) = EventFormat.block(startsAt)

        assertThat(day).isEqualTo("24")
        // Short, not numeric and not the whole word: a block that said
        // "December" would not be a block.
        assertThat(month).isNotEmpty()
        assertThat(month.length).isAtMost(5)
        assertThat(month).doesNotContain("24")
    }

    @Test
    fun `the line beside the block is the time, and says the day again only when it differs`() {
        // Same day: two clock times, and the date is left to the block.
        val sameDay = EventFormat.clockLine(startsAt, endsAt)
        assertThat(sameDay).contains("–")
        assertThat(sameDay).doesNotContain("24")

        // No end: one time.
        assertThat(EventFormat.clockLine(startsAt, null)).doesNotContain("–")

        // Ending the next day: the end carries its own date, or "16:00 –
        // 02:00" would read as an event that went backwards.
        val nextDay = Instant.ofEpochMilli(startsAt).plusSeconds(34 * 3600).toEpochMilli()
        val across = EventFormat.clockLine(startsAt, nextDay)
        assertThat(across).contains("–")
        assertThat(across.length).isGreaterThan(sameDay.length)
    }

    @Test
    fun `a past event is still a past event`() {
        val now = Instant.ofEpochMilli(endsAt).plusSeconds(60).toEpochMilli()
        assertThat(EventFormat.isPast(startsAt, endsAt, now)).isTrue()
        assertThat(EventFormat.isPast(startsAt, endsAt, startsAt)).isFalse()
        // With no end, the start is what decides.
        assertThat(EventFormat.isPast(startsAt, null, startsAt + 1)).isTrue()
    }
}

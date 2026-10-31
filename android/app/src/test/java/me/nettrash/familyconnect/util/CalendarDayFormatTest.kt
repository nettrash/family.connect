/*
 * CalendarDayFormatTest.kt
 * Family Connect (Android)
 *
 * The day a sentence names — an unpaid family's deadline, the day an
 * allowance renews (#89; docs/protocol.md, "Paid families"): day and month
 * in the reader's own order and words, the year only when it is not this
 * one, and the calendar day decided in the zone asked for.
 *
 * Plain JVM, like BirthdayFormatTest.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import java.time.Instant
import java.time.ZoneId
import java.time.ZoneOffset
import java.util.Locale
import org.junit.Test

class CalendarDayFormatTest {

    private val now = Instant.parse("2026-10-30T12:00:00Z").toEpochMilli()

    private fun millis(iso: String): Long = Instant.parse(iso).toEpochMilli()

    @Test
    fun `this year's day has no year, in the reader's own order and words`() {
        val day = millis("2026-11-14T09:00:00Z")
        assertThat(TimeFormat.calendarDay(day, now, ZoneOffset.UTC, Locale.forLanguageTag("en-GB")))
            .isEqualTo("14 November")
        assertThat(TimeFormat.calendarDay(day, now, ZoneOffset.UTC, Locale.forLanguageTag("en-US")))
            .isEqualTo("November 14")
        assertThat(TimeFormat.calendarDay(day, now, ZoneOffset.UTC, Locale.forLanguageTag("ru")))
            .isEqualTo("14 ноября")
        assertThat(TimeFormat.calendarDay(day, now, ZoneOffset.UTC, Locale.forLanguageTag("ja")))
            .isEqualTo("11月14日")
    }

    @Test
    fun `another year's day says its year`() {
        val day = millis("2027-01-03T09:00:00Z")
        val british = TimeFormat.calendarDay(day, now, ZoneOffset.UTC, Locale.forLanguageTag("en-GB"))
        assertThat(british).contains("2027")
        assertThat(british).contains("January")
    }

    /**
     * The 1st of a UTC month is the 1st when named in UTC — and the 30th
     * west of Greenwich, which is why a renewal is named in UTC.
     */
    @Test
    fun `the zone decides which day an instant is`() {
        val renews = millis("2026-12-01T00:00:00Z")
        val london = Locale.forLanguageTag("en-GB")
        assertThat(TimeFormat.calendarDay(renews, now, ZoneOffset.UTC, london)).isEqualTo("1 December")
        assertThat(TimeFormat.calendarDay(renews, now, ZoneId.of("America/New_York"), london))
            .isEqualTo("30 November")
    }
}

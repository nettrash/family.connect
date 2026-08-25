/*
 * BirthdayFormatTest.kt
 * Family Connect (Android)
 *
 * A birthday is drawn in the reader's own words and order, and NEVER
 * with a year — there is none on the wire to draw (docs/protocol.md,
 * "Birthdays"). The pattern is derived from the locale's full date with
 * the year cut out, so the thing worth pinning is that the cut takes the
 * year's punctuation with it: Spanish's second "de", Russian's " г." and
 * Japanese's "年" are there to carry a year, and Japanese's "日" is not.
 *
 * Plain JVM — TimeFormat has no Android dependencies, which is the whole
 * reason it can be tested this directly.
 */

package me.nettrash.familyconnect.util

import com.google.common.truth.Truth.assertThat
import org.junit.Test
import java.util.Locale

class BirthdayFormatTest {

    @Test
    fun `no year survives in any of the nine languages`() {
        val tags = listOf("en", "de", "es", "fr", "ja", "ru", "sr", "sr-Latn", "zh-Hans")
        for (tag in tags) {
            val rendered = TimeFormat.birthday(3, 14, Locale.forLanguageTag(tag))
            // 2024 is the carrier year the formatter builds its date on.
            // If it reaches the screen, so does an age nobody published.
            assertThat(rendered).doesNotContain("2024")
            assertThat(rendered).doesNotContain("24")
            assertThat(rendered).isNotEmpty()
        }
    }

    @Test
    fun `the day and the month land in the reader's own order`() {
        // British English puts the day first, American English the month —
        // which is exactly why the pattern is asked for rather than written.
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("en-GB")))
            .isEqualTo("14 March")
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("en-US")))
            .isEqualTo("March 14")
    }

    @Test
    fun `the year's punctuation goes with it, and the day's stays`() {
        // Russian: "14 марта 2024 г." → the trailing " г." was the year's.
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("ru")))
            .isEqualTo("14 марта")
        // Spanish: "14 de marzo de 2024" → the SECOND "de" was the year's.
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("es")))
            .isEqualTo("14 de marzo")
        // Japanese: "2024年3月14日" → 年 goes, 日 stays. It is not
        // decoration: a bare "3月14" is a date with its last word missing.
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("ja")))
            .isEqualTo("3月14日")
        assertThat(TimeFormat.birthday(3, 14, Locale.forLanguageTag("zh-Hans")))
            .isEqualTo("3月14日")
    }

    @Test
    fun `29 February formats like any other day`() {
        // The date that only exists because there is no year to fail in.
        assertThat(TimeFormat.birthday(2, 29, Locale.forLanguageTag("en-GB")))
            .isEqualTo("29 February")
        assertThat(TimeFormat.birthday(2, 29, Locale.forLanguageTag("ja")))
            .isEqualTo("2月29日")
    }

    @Test
    fun `an impossible pair is clamped rather than thrown`() {
        // The server validates, but a roster row is not where to find out
        // it did not: a LocalDate that threw would take the list with it.
        assertThat(TimeFormat.birthday(13, 40, Locale.forLanguageTag("en-GB")))
            .isEqualTo("31 December")
        assertThat(TimeFormat.birthday(2, 31, Locale.forLanguageTag("en-GB")))
            .isEqualTo("29 February")
    }

    @Test
    fun `February has 29 days and April has 30`() {
        // What the day picker offers. February's 29th is the whole reason
        // this is not Year.of(...).atMonth(...).lengthOfMonth().
        assertThat(daysInBirthdayMonth(2)).isEqualTo(29)
        assertThat(daysInBirthdayMonth(4)).isEqualTo(30)
        assertThat(daysInBirthdayMonth(1)).isEqualTo(31)
        assertThat(daysInBirthdayMonth(12)).isEqualTo(31)
    }

    @Test
    fun `a month names itself standing alone`() {
        assertThat(TimeFormat.monthName(3, Locale.forLanguageTag("en-GB"))).isEqualTo("March")
        // Nominative, not the genitive a date uses: a picker lists the
        // month's name, it does not say "of March".
        assertThat(TimeFormat.monthName(3, Locale.forLanguageTag("ru"))).isEqualTo("март")
    }
}

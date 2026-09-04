/*
 * TimeFormat.kt
 * Family Connect (Android)
 *
 * All timestamp parsing/formatting in one place. Server timestamps are
 * RFC 3339 UTC strings and are authoritative once a message is acked —
 * the local clock only positions a message while it's in flight, so any
 * skew self-corrects the moment the ack lands.
 *
 * Formats: bubble time (HH:mm), date-separator pills (Today / Yesterday /
 * date), and chat-list relative times (HH:mm today, weekday this week,
 * date beyond).
 *
 * Pure functions over injected clock/zone parameters — TimeFormat has no
 * Android dependencies and is directly unit-testable.
 *
 * iOS counterpart: ios/FamilyConnect/Util/TimeFormat.swift
 */

package me.nettrash.familyconnect.util

import java.time.Instant
import java.time.LocalDate
import java.time.Month
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.chrono.IsoChronology
import java.time.format.DateTimeFormatter
import java.time.format.DateTimeFormatterBuilder
import java.time.format.FormatStyle
import java.time.format.TextStyle
import java.util.Locale
import java.util.concurrent.ConcurrentHashMap

/**
 * Injectable wall clock (epoch millis). A fun interface rather than a
 * bare `() -> Long` because Dagger resolves nominal types more reliably
 * than Kotlin function types, and tests read better with a named fake.
 */
fun interface Clock {
    fun now(): Long
}

object TimeFormat {

    /** RFC 3339 → epoch millis; null when the string doesn't parse. */
    fun parseTimestamp(raw: String): Long? =
        runCatching { OffsetDateTime.parse(raw).toInstant().toEpochMilli() }
            .recoverCatching { Instant.parse(raw).toEpochMilli() }
            .getOrNull()

    /** Message-bubble time: 24h "17:03". */
    fun bubbleTime(epochMillis: Long, zone: ZoneId = ZoneId.systemDefault()): String =
        Instant.ofEpochMilli(epochMillis).atZone(zone).format(HH_MM)

    /**
     * The date-separator pill's label. Today and yesterday are WORDS the
     * pill translates where it is drawn (this file has no resources);
     * any other day is the formatted date. It used to return "Today" and
     * "Yesterday" as English strings, which shipped in all nine languages.
     */
    sealed interface DayLabel {
        data object Today : DayLabel
        data object Yesterday : DayLabel
        data class Date(val text: String) : DayLabel
    }

    fun dateSeparator(
        epochMillis: Long,
        nowMillis: Long = System.currentTimeMillis(),
        zone: ZoneId = ZoneId.systemDefault(),
    ): DayLabel {
        val day = Instant.ofEpochMilli(epochMillis).atZone(zone).toLocalDate()
        val today = Instant.ofEpochMilli(nowMillis).atZone(zone).toLocalDate()
        return when (day) {
            today -> DayLabel.Today
            today.minusDays(1) -> DayLabel.Yesterday
            else -> DayLabel.Date(day.format(LONG_DATE))
        }
    }

    /** A stable, language-free key for the day a separator stands for. */
    fun dayKey(epochMillis: Long, zone: ZoneId = ZoneId.systemDefault()): String =
        toLocalDate(epochMillis, zone).toString()

    /** True when both instants fall on the same calendar day. */
    fun sameDay(aMillis: Long, bMillis: Long, zone: ZoneId = ZoneId.systemDefault()): Boolean =
        toLocalDate(aMillis, zone) == toLocalDate(bMillis, zone)

    /**
     * Chat-list relative time: "17:03" today, "Tue" within the last week,
     * "19.08.26" beyond.
     */
    fun listTime(
        epochMillis: Long,
        nowMillis: Long = System.currentTimeMillis(),
        zone: ZoneId = ZoneId.systemDefault(),
    ): String {
        val moment = Instant.ofEpochMilli(epochMillis).atZone(zone)
        val day = moment.toLocalDate()
        val today = Instant.ofEpochMilli(nowMillis).atZone(zone).toLocalDate()
        return when {
            day == today -> moment.format(HH_MM)
            day > today.minusDays(7) ->
                day.dayOfWeek.getDisplayName(TextStyle.SHORT, Locale.getDefault())
            else -> day.format(SHORT_DATE)
        }
    }

    /**
     * A birthday: day and month in the reader's own order and words —
     * "14 March", "14 марта", "3月14日" — and NEVER a year.
     *
     * There is no year to print: a birthday on the wire is two integers
     * (docs/protocol.md, "Birthdays"), and printing one would invite the
     * age this protocol deliberately cannot compute. That is why the
     * pattern is derived rather than written: only the reader's locale
     * knows whether the day or the month comes first, what sits between
     * them, and that Japanese needs a 日 after the number — so the
     * localized full-date pattern is taken and the YEAR is cut out of it,
     * along with whatever literal text was only there to attach it ("de"
     * in Spanish, " г." in Russian, "年" in Japanese).
     *
     * 29 February formats like any other day. With no year it cannot fail
     * to exist, which is exactly why the protocol allows it.
     */
    fun birthday(month: Int, day: Int, locale: Locale = Locale.getDefault()): String {
        // Clamped rather than trusted: the server validates the pair, but
        // a roster row is not the place to find out it did not — a
        // LocalDate that throws would take the whole list down with it.
        val safeMonth = month.coerceIn(1, 12)
        val safeDay = day.coerceIn(1, daysInBirthdayMonth(safeMonth))
        // 2024 is a leap year, so 29 February has somewhere to land. The
        // year is only ever a carrier here — it never reaches the output.
        val date = LocalDate.of(2024, safeMonth, safeDay)
        return date.format(DateTimeFormatter.ofPattern(dayAndMonthPattern(locale), locale))
    }

    /**
     * A month's name standing on its own, in the reader's language:
     * "March", "март", "3月". STANDALONE rather than the form used
     * inside a date, because Slavic languages inflect the two
     * differently — a picker listing "марта" would be listing the
     * genitive of a word nobody asked a question about.
     */
    fun monthName(month: Int, locale: Locale = Locale.getDefault()): String =
        Month.of(month.coerceIn(1, 12)).getDisplayName(TextStyle.FULL_STANDALONE, locale)

    /**
     * The locale's full-date pattern with every year field, and the
     * literals orphaned by removing them, taken out.
     *
     * Split out and cached because building it walks the pattern
     * character by character, and a roster redraws it once per member.
     */
    private fun dayAndMonthPattern(locale: Locale): String =
        dayAndMonthPatterns.getOrPut(locale) {
            val full = DateTimeFormatterBuilder.getLocalizedDateTimePattern(
                FormatStyle.LONG,
                null,
                IsoChronology.INSTANCE,
                locale,
            )
            val tokens = patternTokens(full)
            val kept = tokens.toMutableList()
            // Walk from the end so removing one index never shifts the
            // ones still to be examined.
            for (index in tokens.indices.reversed()) {
                if (!isYearField(tokens[index])) continue
                // The year plus the run of literals on either side of it:
                // Spanish's "de" and Russian's "г." are only there to
                // carry a year, and leaving them behind reads as a typo.
                var from = index
                var to = index
                while (from > 0 && !isField(tokens[from - 1])) from--
                while (to < tokens.size - 1 && !isField(tokens[to + 1])) to++
                repeat(to - from + 1) { kept.removeAt(from) }
            }
            // A pattern with no month or day left is a locale this cannot
            // read; ISO-ish day-then-month is a poor answer but a legible one.
            if (kept.none(::isField)) "d MMMM" else kept.joinToString("").trim()
        }

    /**
     * One pattern split into field runs ("MMMM"), quoted literals ("'de'")
     * and everything else. Only ASCII letters are pattern fields to
     * java.time, which is what leaves Japanese's 年月日 as literals — and
     * they must stay, since they are the date's punctuation.
     */
    private fun patternTokens(pattern: String): List<String> {
        val tokens = mutableListOf<String>()
        var i = 0
        while (i < pattern.length) {
            val c = pattern[i]
            when {
                c == '\'' -> {
                    var j = i + 1
                    while (j < pattern.length && pattern[j] != '\'') j++
                    // Past the closing quote, or the end for an unterminated one.
                    j = (j + 1).coerceAtMost(pattern.length)
                    tokens += pattern.substring(i, j)
                    i = j
                }
                isPatternLetter(c) -> {
                    var j = i
                    while (j < pattern.length && pattern[j] == c) j++
                    tokens += pattern.substring(i, j)
                    i = j
                }
                else -> {
                    var j = i
                    while (j < pattern.length && !isPatternLetter(pattern[j]) && pattern[j] != '\'') j++
                    tokens += pattern.substring(i, j)
                    i = j
                }
            }
        }
        return tokens
    }

    private fun isPatternLetter(c: Char): Boolean = c in 'A'..'Z' || c in 'a'..'z'

    private fun isField(token: String): Boolean = isPatternLetter(token.first())

    /** 'u' as well as 'y': some locales spell the year field either way. */
    private fun isYearField(token: String): Boolean =
        token.first() == 'y' || token.first() == 'u'

    // Concurrent because a roster row can be laid out off the main
    // thread; the value is a pure function of the key, so a racing pair
    // of builders only ever agree.
    private val dayAndMonthPatterns = ConcurrentHashMap<Locale, String>()

    private fun toLocalDate(epochMillis: Long, zone: ZoneId): LocalDate =
        Instant.ofEpochMilli(epochMillis).atZone(zone).toLocalDate()

    private val HH_MM = DateTimeFormatter.ofPattern("HH:mm")
    private val LONG_DATE = DateTimeFormatter.ofPattern("d MMMM yyyy")
    private val SHORT_DATE = DateTimeFormatter.ofPattern("dd.MM.yy")
}

/**
 * Which days a month has when there is no year to ask about.
 *
 * February gets 29: a birthday carries no year, so 29 February cannot
 * fail to exist and the server accepts it (docs/protocol.md,
 * "Birthdays"). The server is the authority on the day-vs-month rule —
 * this is here so a picker cannot offer 31 April in the first place.
 */
fun daysInBirthdayMonth(month: Int): Int = when (month) {
    2 -> 29
    4, 6, 9, 11 -> 30
    else -> 31
}

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
import java.time.OffsetDateTime
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.util.Locale

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

    /** Date-separator pill: "Today" / "Yesterday" / "19 August 2026". */
    fun dateSeparator(
        epochMillis: Long,
        nowMillis: Long = System.currentTimeMillis(),
        zone: ZoneId = ZoneId.systemDefault(),
    ): String {
        val day = Instant.ofEpochMilli(epochMillis).atZone(zone).toLocalDate()
        val today = Instant.ofEpochMilli(nowMillis).atZone(zone).toLocalDate()
        return when (day) {
            today -> "Today"
            today.minusDays(1) -> "Yesterday"
            else -> day.format(LONG_DATE)
        }
    }

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

    private fun toLocalDate(epochMillis: Long, zone: ZoneId): LocalDate =
        Instant.ofEpochMilli(epochMillis).atZone(zone).toLocalDate()

    private val HH_MM = DateTimeFormatter.ofPattern("HH:mm")
    private val LONG_DATE = DateTimeFormatter.ofPattern("d MMMM yyyy")
    private val SHORT_DATE = DateTimeFormatter.ofPattern("dd.MM.yy")
}

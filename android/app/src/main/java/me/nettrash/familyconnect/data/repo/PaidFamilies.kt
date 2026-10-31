/*
 * PaidFamilies.kt
 * Family Connect (Android)
 *
 * The rules of paid families (#89; docs/protocol.md, "Paid families") that
 * are decisions and not requests, held where a test can reach them: whether
 * "Create a family" may be offered, which unpaid banner the chat list
 * shows, when a used-up allowance starts again, and the allowance's
 * arithmetic.
 *
 * This app SELLS nothing. Google Play's payments policy forbids steering a
 * buyer to another way of paying, so everything here only reads what the
 * server says the caller already holds, and the words it leads to say no
 * more than that a family cannot be started or renewed from this app.
 *
 * Pure: no Android, no storage.
 */

package me.nettrash.familyconnect.data.repo

import java.time.Instant
import java.time.ZoneOffset
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.util.TimeFormat

object PaidFamilies {

    /** `POST /families` from an account without an active subscription (403, terminal). */
    const val SUBSCRIPTION_REQUIRED = "subscription_required"

    /** `POST /attachments` for a paid family past its storage (403, terminal, never retried). */
    const val STORAGE_ALLOWANCE_USED = "storage_allowance_used"

    /** A transcript or a board backdrop for a paid family past this month's allowance (403). */
    const val AI_ALLOWANCE_USED = "ai_allowance_used"

    /**
     * Whether "Create a family" and "New Family…" give way to the sentence
     * that a new family cannot be started from this app: the server charges
     * for families and the caller holds no active subscription. A subscriber
     * creates exactly as before — the server does not care which app sends
     * the create.
     */
    fun mustPayToStart(subscriptionsEnabled: Boolean, subscriptionActive: Boolean): Boolean =
        subscriptionsEnabled && !subscriptionActive

    /**
     * The banner over a family's chat list while it is unpaid: the family
     * will be deleted at [deleteAtMillis] because its subscription ended.
     * The owner is told it cannot be renewed from this app; a member is
     * told who the owner is, the one person who can.
     */
    data class UnpaidBanner(
        val familyName: String,
        val deleteAtMillis: Long,
        val isOwner: Boolean,
        /** The owner's display name from the roster; null for the owner's own banner. */
        val ownerName: String?,
    )

    /**
     * The banner for [family], the one the app is showing, or null for none:
     * no deadline, a deadline that does not parse, or — for a member — a
     * roster that does not name the owner yet, since a sentence with a hole
     * where the owner's name goes would be worse than a moment's wait.
     */
    fun banner(family: FamilyDto?, ownerName: String?): UnpaidBanner? {
        val deleteAt = family?.deleteAt?.let(TimeFormat::parseTimestamp) ?: return null
        val isOwner = family.role == FamilyList.OWNER
        if (!isOwner && ownerName.isNullOrBlank()) return null
        return UnpaidBanner(
            familyName = family.name,
            deleteAtMillis = deleteAt,
            isOwner = isOwner,
            ownerName = if (isOwner) null else ownerName,
        )
    }

    /**
     * The start of the UTC calendar month after [nowMillis] — when a paid
     * family's monthly allowance starts again. What the REST code
     * `ai_allowance_used` names, since it carries no date of its own.
     */
    fun nextMonthStartUtc(nowMillis: Long): Long =
        Instant.ofEpochMilli(nowMillis).atZone(ZoneOffset.UTC).toLocalDate()
            .withDayOfMonth(1)
            .plusMonths(1)
            .atStartOfDay(ZoneOffset.UTC)
            .toInstant()
            .toEpochMilli()

    /** A `renews_at` the server sent, or — absent or unreadable — the start of next month. */
    fun renewsAt(raw: String?, nowMillis: Long): Long =
        raw?.let(TimeFormat::parseTimestamp) ?: nextMonthStartUtc(nowMillis)

    /**
     * The assistant's tokens as a whole percentage: used / limit × 100,
     * rounded DOWN so a family is never told 100% before it is, and capped
     * at 100, because one answer may overshoot its allowance by its own
     * size. Null for a limit that is not a limit.
     */
    fun percentUsed(used: Long, limit: Long): Int? {
        if (limit <= 0) return null
        // Capped before multiplying, so the product stays far inside a Long
        // for any allowance an operator could configure (millions of tokens).
        return (used.coerceIn(0, limit) * 100 / limit).toInt()
    }

    /** Transcript time used, in whole minutes, rounded UP: a started minute counts. */
    fun minutesUsed(ms: Long): Long {
        val positive = ms.coerceAtLeast(0)
        return positive / MS_PER_MINUTE + if (positive % MS_PER_MINUTE > 0) 1 else 0
    }

    /** The transcript limit, in whole minutes, rounded DOWN: never more than the family has. */
    fun minutesLimit(ms: Long): Long = ms.coerceAtLeast(0) / MS_PER_MINUTE

    private const val MS_PER_MINUTE = 60_000L
}

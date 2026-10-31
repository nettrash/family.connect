/*
 * PaidFamiliesTest.kt
 * Family Connect (Android)
 *
 * The decisions of paid families (#89; docs/protocol.md, "Paid families"),
 * on the plain JVM: when Create gives way to the sentence, which unpaid
 * banner a family shows, when a used-up month starts again, and the
 * allowance's arithmetic — percent rounded down and capped, minutes used
 * rounded up and the limit rounded down.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import java.time.Instant
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import org.junit.Test

class PaidFamiliesTest {

    private fun millis(iso: String): Long = Instant.parse(iso).toEpochMilli()

    private val unpaid = FamilyDto(
        id = 3,
        name = "Ivanovs",
        joinPolicy = "open",
        role = "member",
        plan = "paid",
        deleteAt = "2026-11-14T09:00:00Z",
    )

    // -- Starting a family ------------------------------------------------------

    @Test
    fun `create gives way only on a server that charges, to somebody who does not pay`() {
        assertThat(PaidFamilies.mustPayToStart(subscriptionsEnabled = true, subscriptionActive = false)).isTrue()
        assertThat(PaidFamilies.mustPayToStart(subscriptionsEnabled = true, subscriptionActive = true)).isFalse()
        assertThat(PaidFamilies.mustPayToStart(subscriptionsEnabled = false, subscriptionActive = false)).isFalse()
        assertThat(PaidFamilies.mustPayToStart(subscriptionsEnabled = false, subscriptionActive = true)).isFalse()
    }

    // -- The unpaid banner --------------------------------------------------------

    @Test
    fun `a member's banner names the owner`() {
        val banner = PaidFamilies.banner(unpaid, ownerName = "Olga")

        assertThat(banner).isEqualTo(
            PaidFamilies.UnpaidBanner(
                familyName = "Ivanovs",
                deleteAtMillis = millis("2026-11-14T09:00:00Z"),
                isOwner = false,
                ownerName = "Olga",
            ),
        )
    }

    @Test
    fun `the owner's banner is the owner's, whoever the roster says`() {
        val banner = PaidFamilies.banner(unpaid.copy(role = "owner"), ownerName = "Olga")

        assertThat(banner?.isOwner).isTrue()
        assertThat(banner?.ownerName).isNull()
        // The owner's own banner needs no roster at all.
        assertThat(PaidFamilies.banner(unpaid.copy(role = "owner"), ownerName = null)?.isOwner).isTrue()
    }

    @Test
    fun `no deadline, no banner`() {
        assertThat(PaidFamilies.banner(unpaid.copy(deleteAt = null), ownerName = "Olga")).isNull()
        assertThat(PaidFamilies.banner(unpaid.copy(deleteAt = "not a date"), ownerName = "Olga")).isNull()
        assertThat(PaidFamilies.banner(null, ownerName = "Olga")).isNull()
    }

    /** A sentence with a hole where the owner's name goes is worse than a moment's wait for the roster. */
    @Test
    fun `a member's banner waits for the roster to name the owner`() {
        assertThat(PaidFamilies.banner(unpaid, ownerName = null)).isNull()
        assertThat(PaidFamilies.banner(unpaid, ownerName = " ")).isNull()
    }

    // -- When a month starts again -------------------------------------------------

    @Test
    fun `the allowance starts again on the 1st of next month, UTC`() {
        assertThat(PaidFamilies.nextMonthStartUtc(millis("2026-10-30T12:00:00Z")))
            .isEqualTo(millis("2026-11-01T00:00:00Z"))
        // Across a year.
        assertThat(PaidFamilies.nextMonthStartUtc(millis("2026-12-15T23:59:59Z")))
            .isEqualTo(millis("2027-01-01T00:00:00Z"))
        // At the very first instant of a month, the NEXT month's.
        assertThat(PaidFamilies.nextMonthStartUtc(millis("2026-11-01T00:00:00Z")))
            .isEqualTo(millis("2026-12-01T00:00:00Z"))
        // UTC decides, not the reader: still 31 October in UTC here.
        assertThat(PaidFamilies.nextMonthStartUtc(millis("2026-10-31T23:30:00Z")))
            .isEqualTo(millis("2026-11-01T00:00:00Z"))
    }

    @Test
    fun `a renews_at the server sent wins, and without one it is next month`() {
        val now = millis("2026-10-30T12:00:00Z")
        assertThat(PaidFamilies.renewsAt("2026-12-01T00:00:00Z", now)).isEqualTo(millis("2026-12-01T00:00:00Z"))
        assertThat(PaidFamilies.renewsAt(null, now)).isEqualTo(millis("2026-11-01T00:00:00Z"))
        assertThat(PaidFamilies.renewsAt("garbage", now)).isEqualTo(millis("2026-11-01T00:00:00Z"))
    }

    // -- The allowance's arithmetic --------------------------------------------------

    @Test
    fun `the assistant's share is a whole percentage, rounded down and capped at 100`() {
        assertThat(PaidFamilies.percentUsed(812_345, 2_000_000)).isEqualTo(40)
        assertThat(PaidFamilies.percentUsed(0, 2_000_000)).isEqualTo(0)
        // Never 100% before it is.
        assertThat(PaidFamilies.percentUsed(1_999_999, 2_000_000)).isEqualTo(99)
        assertThat(PaidFamilies.percentUsed(2_000_000, 2_000_000)).isEqualTo(100)
        // One answer may overshoot by its own size: still 100.
        assertThat(PaidFamilies.percentUsed(2_400_000, 2_000_000)).isEqualTo(100)
        assertThat(PaidFamilies.percentUsed(29, 100)).isEqualTo(29)
        assertThat(PaidFamilies.percentUsed(-5, 100)).isEqualTo(0)
        // A limit that is not a limit has no percentage.
        assertThat(PaidFamilies.percentUsed(10, 0)).isNull()
    }

    @Test
    fun `transcript minutes used round up and the limit rounds down`() {
        assertThat(PaidFamilies.minutesUsed(0)).isEqualTo(0)
        assertThat(PaidFamilies.minutesUsed(1)).isEqualTo(1)
        assertThat(PaidFamilies.minutesUsed(60_000)).isEqualTo(1)
        assertThat(PaidFamilies.minutesUsed(60_001)).isEqualTo(2)
        assertThat(PaidFamilies.minutesUsed(600_000)).isEqualTo(10)
        assertThat(PaidFamilies.minutesUsed(-1)).isEqualTo(0)

        assertThat(PaidFamilies.minutesLimit(7_200_000)).isEqualTo(120)
        assertThat(PaidFamilies.minutesLimit(7_259_999)).isEqualTo(120)
        assertThat(PaidFamilies.minutesLimit(59_999)).isEqualTo(0)
    }
}

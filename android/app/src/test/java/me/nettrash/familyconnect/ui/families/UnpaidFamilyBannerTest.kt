/*
 * UnpaidFamilyBannerTest.kt
 * Family Connect (Android)
 *
 * The unpaid banner's words (#89; docs/protocol.md, "Paid families"): a
 * member is told who the owner is, the owner that it cannot be renewed
 * from this app — and neither names another app, a store or a price. The
 * deadline is a day in the reader's own time zone.
 */

package me.nettrash.familyconnect.ui.families

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import java.time.Instant
import java.time.ZoneId
import java.time.ZoneOffset
import java.util.Locale
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.repo.PaidFamilies
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en-rGB")
class UnpaidFamilyBannerTest {

    private val context: Context = ApplicationProvider.getApplicationContext()
    private val now = Instant.parse("2026-10-30T12:00:00Z").toEpochMilli()
    private val deadline = Instant.parse("2026-11-14T09:00:00Z").toEpochMilli()

    private fun said(sentence: BannerSentence): String = context.getString(sentence.res, *sentence.args.toTypedArray())

    @Test
    fun `a member is told the day and who the owner is`() {
        val banner = PaidFamilies.UnpaidBanner("Ivanovs", deadline, isOwner = false, ownerName = "Olga")

        val sentence = unpaidBannerSentence(banner, now, ZoneOffset.UTC, Locale.UK)

        assertThat(sentence.res).isEqualTo(R.string.s_banner_member)
        assertThat(said(sentence)).isEqualTo(
            "“Ivanovs” will be deleted on 14 November because its subscription has ended. " +
                "Only Olga, the owner, can renew it.",
        )
    }

    @Test
    fun `the owner is told it cannot be renewed from this app`() {
        val banner = PaidFamilies.UnpaidBanner("Ivanovs", deadline, isOwner = true, ownerName = null)

        val sentence = unpaidBannerSentence(banner, now, ZoneOffset.UTC, Locale.UK)

        assertThat(sentence.res).isEqualTo(R.string.s_banner_owner_not_here)
        assertThat(said(sentence)).isEqualTo(
            "“Ivanovs” will be deleted on 14 November because your subscription has ended. " +
                "It can’t be renewed from this app.",
        )
    }

    /** The deadline is the reader's day: early on the 14th in UTC is still the 13th in Los Angeles. */
    @Test
    fun `the day is the reader's own`() {
        val early = Instant.parse("2026-11-14T03:00:00Z").toEpochMilli()
        val banner = PaidFamilies.UnpaidBanner("Ivanovs", early, isOwner = true, ownerName = null)

        assertThat(said(unpaidBannerSentence(banner, now, ZoneId.of("America/Los_Angeles"), Locale.UK))).contains("13 November")
        assertThat(said(unpaidBannerSentence(banner, now, ZoneOffset.UTC, Locale.UK))).contains("14 November")
    }

    @Test
    fun `neither banner names another app, a store or a price`() {
        val owner = said(unpaidBannerSentence(PaidFamilies.UnpaidBanner("Ivanovs", deadline, true, null), now))
        val member = said(unpaidBannerSentence(PaidFamilies.UnpaidBanner("Ivanovs", deadline, false, "Olga"), now))
        for (text in listOf(owner, member)) {
            for (word in listOf("iPhone", "iPad", "Mac", "Apple", "App Store", "Play", "$", "€")) {
                assertThat(text).doesNotContain(word)
            }
        }
    }
}

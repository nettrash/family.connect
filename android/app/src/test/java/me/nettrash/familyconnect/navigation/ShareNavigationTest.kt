/*
 * ShareNavigationTest.kt
 * Family Connect (Android)
 *
 * The share picker's navigation gate is the CURRENT session status run
 * through the same status → route rule the boot destination uses —
 * never the frozen boot route itself, which goes stale the moment
 * someone logs in or joins a family after boot.
 */

package me.nettrash.familyconnect.navigation

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.repo.FamilyStatus
import org.junit.Test

class ShareNavigationTest {

    @Test
    fun `only a chat-capable current status navigates to the picked chat`() {
        assertThat(shareNavigatesToChat(FamilyStatus.MEMBER)).isTrue()
        assertThat(shareNavigatesToChat(FamilyStatus.OWNER)).isTrue()

        assertThat(shareNavigatesToChat(FamilyStatus.NO_SERVER)).isFalse()
        assertThat(shareNavigatesToChat(FamilyStatus.NO_TOKEN)).isFalse()
        assertThat(shareNavigatesToChat(FamilyStatus.NONE)).isFalse()
        assertThat(shareNavigatesToChat(FamilyStatus.PENDING)).isFalse()
    }

    /** No emission yet (the live flow is still null) must not navigate. */
    @Test
    fun `an unknown status does not navigate`() {
        assertThat(shareNavigatesToChat(null)).isFalse()
    }
}

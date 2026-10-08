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
import me.nettrash.familyconnect.MainViewModel
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

    /**
     * A shared item waits while the video recorder is open (#79, S4: "a
     * notification tap or shared item waits until it closes"): the sheet is
     * a window of its own that would rise over the recorder, and a pick
     * would change the chat under a clip in REVIEW. The import itself goes on
     * — the read grants are transient — and the sheet comes when it closes.
     */
    @Test
    fun aShareWaitsWhileTheVideoRecorderIsOpen() {
        val choose = MainViewModel.ShareFlow.ChooseChat(itemCount = 1, hasText = false)
        assertThat(shownShareFlow(choose, recorderOpen = true)).isNull()
        assertThat(shownShareFlow(MainViewModel.ShareFlow.Preparing, recorderOpen = true)).isNull()
        assertThat(shownShareFlow(choose, recorderOpen = false)).isEqualTo(choose)
        assertThat(shownShareFlow(MainViewModel.ShareFlow.Preparing, recorderOpen = false))
            .isEqualTo(MainViewModel.ShareFlow.Preparing)
        assertThat(shownShareFlow(null, recorderOpen = false)).isNull()
    }
}

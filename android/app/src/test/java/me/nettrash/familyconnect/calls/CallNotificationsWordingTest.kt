/*
 * CallNotificationsWordingTest.kt
 * Family Connect (Android)
 *
 * The kind → wording choice both call notifications draw from
 * (docs/protocol.md, "Video": the ringing push carries the flag so the
 * incoming UI is a camera one). Pure resource-id choosers, pinned on the
 * plain JVM.
 */

package me.nettrash.familyconnect.calls

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.R
import org.junit.Test

class CallNotificationsWordingTest {

    @Test
    fun theRingingLineFollowsTheCallsKind() {
        assertThat(CallNotifications.incomingTextRes(video = false)).isEqualTo(R.string.s_incoming_voice_call)
        assertThat(CallNotifications.incomingTextRes(video = true)).isEqualTo(R.string.s_incoming_video_call)
    }

    @Test
    fun theOngoingLineFollowsTheCallsKind() {
        assertThat(CallNotifications.ongoingTextRes(video = false)).isEqualTo(R.string.s_ongoing_voice_call)
        assertThat(CallNotifications.ongoingTextRes(video = true)).isEqualTo(R.string.s_ongoing_video_call)
    }
}

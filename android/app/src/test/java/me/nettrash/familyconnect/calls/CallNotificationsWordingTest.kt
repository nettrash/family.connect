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
    fun theNameFallbackFollowsTheCallsKind() {
        assertThat(CallNotifications.kindTextRes(video = false)).isEqualTo(R.string.s_voice_call)
        assertThat(CallNotifications.kindTextRes(video = true)).isEqualTo(R.string.s_video_call)
    }

    @Test
    fun theOngoingLineFollowsTheCallsKind() {
        assertThat(CallNotifications.ongoingTextRes(video = false)).isEqualTo(R.string.s_ongoing_voice_call)
        assertThat(CallNotifications.ongoingTextRes(video = true)).isEqualTo(R.string.s_ongoing_video_call)
    }

    /** The two endings that read differently by side — the split iOS CallStatusText.ended makes. */
    @Test
    fun aTimeoutIsNoAnswerToTheCallerAndAMissedCallToTheCallee() {
        assertThat(CallNotifications.endedTextRes(CallEnding.TIMEOUT, outgoing = true, video = false))
            .isEqualTo(R.string.s_no_answer)
        assertThat(CallNotifications.endedTextRes(CallEnding.TIMEOUT, outgoing = true, video = true))
            .isEqualTo(R.string.s_no_answer)
        assertThat(CallNotifications.endedTextRes(CallEnding.TIMEOUT, outgoing = false, video = false))
            .isEqualTo(R.string.s_missed_voice_call)
        assertThat(CallNotifications.endedTextRes(CallEnding.TIMEOUT, outgoing = false, video = true))
            .isEqualTo(R.string.s_missed_video_call)
    }

    @Test
    fun aDeclineIsDeclinedToTheCallerAndCallEndedToTheOneWhoDeclined() {
        assertThat(CallNotifications.endedTextRes(CallEnding.DECLINE, outgoing = true, video = false))
            .isEqualTo(R.string.s_declined)
        assertThat(CallNotifications.endedTextRes(CallEnding.DECLINE, outgoing = true, video = true))
            .isEqualTo(R.string.s_declined)
        assertThat(CallNotifications.endedTextRes(CallEnding.DECLINE, outgoing = false, video = true))
            .isEqualTo(R.string.s_call_ended)
    }

    @Test
    fun theOtherEndingsReadTheSameFromEitherSide() {
        for (outgoing in listOf(true, false)) {
            assertThat(CallNotifications.endedTextRes(CallEnding.HANGUP, outgoing, video = true))
                .isEqualTo(R.string.s_call_ended)
            assertThat(CallNotifications.endedTextRes(CallEnding.CANCEL, outgoing, video = false))
                .isEqualTo(R.string.s_call_ended)
            assertThat(CallNotifications.endedTextRes(CallEnding.FAILED, outgoing, video = false))
                .isEqualTo(R.string.s_call_failed)
            assertThat(CallNotifications.endedTextRes(CallEnding.VIDEO_DISABLED, outgoing, video = true))
                .isEqualTo(R.string.s_video_calls_are_off_on_this_server)
        }
    }
}

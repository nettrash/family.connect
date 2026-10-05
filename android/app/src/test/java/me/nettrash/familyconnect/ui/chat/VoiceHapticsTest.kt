/*
 * VoiceHapticsTest.kt
 * Family Connect (Android)
 *
 * S2.9's haptics (#79, docs/audio-video-messages-2026-10-04.md): which
 * moment plays which of Android's feedback types — the plan's table, row by
 * row — and that they play on phones only, never on a tablet.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class VoiceHapticsTest {

    @Test
    fun eachMomentPlaysThePlansFeedbackType() {
        // recording starts from a tap → ToggleOn; at H → LongPress; lock and
        // cancel armed → GestureThresholdActivate; sent → Confirm; too short
        // and deleted → Reject.
        assertThat(RecordGesture.Haptic.LIGHT.feedback()).isEqualTo(HapticFeedbackType.ToggleOn)
        assertThat(RecordGesture.Haptic.MEDIUM.feedback()).isEqualTo(HapticFeedbackType.LongPress)
        assertThat(RecordGesture.Haptic.SELECTION.feedback())
            .isEqualTo(HapticFeedbackType.GestureThresholdActivate)
        assertThat(RecordGesture.Haptic.SUCCESS.feedback()).isEqualTo(HapticFeedbackType.Confirm)
        assertThat(RecordGesture.Haptic.WARNING.feedback()).isEqualTo(HapticFeedbackType.Reject)
    }

    @Test
    fun noTwoMomentsFeelTheSame() {
        val types = RecordGesture.Haptic.entries.map { it.feedback() }
        assertThat(types).containsNoDuplicates()
    }

    @Test
    fun phonesOnlyNeverATablet() {
        assertThat(voiceHapticsOn(smallestScreenWidthDp = 360)).isTrue()
        assertThat(voiceHapticsOn(smallestScreenWidthDp = 599)).isTrue()
        assertThat(voiceHapticsOn(smallestScreenWidthDp = 600)).isFalse()
        assertThat(voiceHapticsOn(smallestScreenWidthDp = 800)).isFalse()
    }
}

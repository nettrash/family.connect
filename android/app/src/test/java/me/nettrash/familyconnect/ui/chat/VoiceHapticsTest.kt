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
        // recording starts → ToggleOn; sent → Confirm; too short and deleted
        // → Reject. (The hold's LongPress and GestureThresholdActivate went
        // with it on 2026-10-06.)
        assertThat(RecordGesture.Haptic.entries).hasSize(3)
        assertThat(RecordGesture.Haptic.LIGHT.feedback()).isEqualTo(HapticFeedbackType.ToggleOn)
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

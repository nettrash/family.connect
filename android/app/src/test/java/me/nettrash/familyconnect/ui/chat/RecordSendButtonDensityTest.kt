/*
 * RecordSendButtonDensityTest.kt
 * Family Connect (Android)
 *
 * The slot's hit area is S1.1's UNITS — 48 dp on Android
 * (docs/audio-video-messages-2026-10-04.md, "Units", and S1.1) — around a
 * 44-dp face: a lift up to 24 dp from the centre is still a tap, one past it
 * is not. The pointer speaks pixels, so the button must measure in dp — on an
 * xxhdpi phone (density 3) a pixel reading would shrink the margin to a
 * third. (Until 2026-10-06 this file pinned the hold's slide distances; the
 * hold is gone, and the hit area is what is left that a density can break.)
 *
 * At xxhdpi on purpose: at Robolectric's default mdpi a pixel IS a dp, and
 * the mistake cannot be seen.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "xxhdpi")
class RecordSendButtonDensityTest {

    @get:Rule
    val compose = createComposeRule()

    private var activations = 0

    private fun show() {
        compose.setContent {
            Box(Modifier.padding(start = 60.dp, top = 60.dp)) {
                RecordSendButton(
                    slot = ComposerSlot.Slot.Microphone,
                    onActivate = { activations++ },
                    onSend = {},
                    onStopAndListen = {},
                    onDeleteRecording = {},
                    onRecordFromMenu = {},
                    focusRequester = remember { FocusRequester() },
                )
            }
        }
    }

    private val microphone get() = compose.onNodeWithContentDescription("Record voice message")

    private val density get() = RuntimeEnvironment.getApplication().resources.displayMetrics.density

    private fun liftAt(dpRightOfCentre: Float) {
        microphone.performTouchInput {
            down(center)
            moveTo(Offset(centerX + dpRightOfCentre.dp.toPx(), centerY))
            up()
        }
    }

    @Test
    fun aLiftInsideTheFortyEightDpAreaTapsAtThreeTimesDensity() {
        assertThat(density).isEqualTo(3f)
        show()

        liftAt(23.5f)

        assertThat(activations).isEqualTo(1)
    }

    @Test
    fun aLiftPastTheFortyEightDpAreaIsNoTapAtThreeTimesDensity() {
        show()

        liftAt(26f)

        assertThat(activations).isEqualTo(0)
    }
}

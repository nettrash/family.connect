/*
 * RecordSendButtonDensityTest.kt
 * Family Connect (Android)
 *
 * The hold's distances are S1.1's UNITS — dp on Android
 * (docs/audio-video-messages-2026-10-04.md, "Units", and S1.1): a press that
 * wanders 20 can no longer become a hold, 60 up locks, 100 toward the field
 * arms cancel and under 80 disarms it. The pointer speaks pixels, so the
 * button must hand the shared reducer dp — on an xxhdpi phone (density 3)
 * pixels would make the slop under 7 dp, the lock 20 dp and cancel 33 dp,
 * and a natural wobble, a small drift up or a short slide would decide the
 * recording for the person.
 *
 * At xxhdpi on purpose: at Robolectric's default mdpi a pixel IS a dp, and
 * the mistake cannot be seen. RecordSendButton drives the real reducer here.
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

    private var state = RecordGesture.HoldState()
    private val constants = RecordGesture.HoldConstants()

    /** The reducer's clock, by hand: H is a Tick this test sends itself. */
    private var now = 0L

    private val downs = mutableListOf<Offset>()

    private fun step(event: RecordGesture.HoldEvent) {
        state = RecordGesture.holdStep(state, event, constants).first
    }

    private fun show() {
        compose.setContent {
            // Away from the window's corner, so window coordinates are not
            // the button's own.
            Box(Modifier.padding(start = 60.dp, top = 30.dp)) {
                RecordSendButton(
                    slot = ComposerSlot.Slot.Microphone,
                    onMicDown = { x, y, canHold, rtl ->
                        downs += Offset(x.toFloat(), y.toFloat())
                        step(RecordGesture.HoldEvent.Down(now, x, y, canHold, rtl))
                    },
                    onMicMove = { x, y -> step(RecordGesture.HoldEvent.Move(now, x, y)) },
                    onMicUp = { _, _, _ -> },
                    onMicCancel = {},
                    onActivate = {},
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

    /** A finger down, still, until H: the hold row. */
    private fun holdAtH() {
        microphone.performTouchInput { down(center) }
        now = constants.holdThresholdMs
        step(RecordGesture.HoldEvent.Tick(now, RecordGesture.Situation()))
        assertThat(state.phase).isInstanceOf(RecordGesture.Phase.Holding::class.java)
    }

    private fun armed() = (state.phase as RecordGesture.Phase.Holding).armed

    @Test
    fun theReducerIsHandedWindowCoordinatesInDp() {
        assertThat(density).isEqualTo(3f)
        show()
        val bounds = microphone.fetchSemanticsNode().boundsInRoot

        microphone.performTouchInput { down(center) }

        val at = downs.single()
        assertThat(at.x).isWithin(0.5f).of(bounds.center.x / density)
        assertThat(at.y).isWithin(0.5f).of(bounds.center.y / density)
    }

    /** Cancel arms at 100 dp toward the field — not at 100 pixels, 33 dp on this phone. */
    @Test
    fun cancelArmsAtAHundredDpTowardTheFieldAndNotBefore() {
        show()
        holdAtH()

        microphone.performTouchInput { moveBy(Offset(-95.dp.toPx(), 0f)) }
        assertThat(armed()).isFalse()

        microphone.performTouchInput { moveBy(Offset(-10.dp.toPx(), 0f)) }
        assertThat(armed()).isTrue()

        // And under 80 disarms it again.
        microphone.performTouchInput { moveBy(Offset(30.dp.toPx(), 0f)) }
        assertThat(armed()).isFalse()
    }

    /** It locks at 60 dp up — a 55-dp drift is still a hold. */
    @Test
    fun itLocksAtSixtyDpUpAndNotBefore() {
        show()
        holdAtH()

        microphone.performTouchInput { moveBy(Offset(0f, -55.dp.toPx())) }
        assertThat(state.phase).isInstanceOf(RecordGesture.Phase.Holding::class.java)

        microphone.performTouchInput { moveBy(Offset(0f, -10.dp.toPx())) }
        assertThat(state.recording).isEqualTo(ComposerSlot.Recording.HANDS_FREE)
    }

    /** A wobble of 18 dp before H still holds; past 20 dp it can no longer. */
    @Test
    fun theTapSlopIsTwentyDp() {
        show()
        microphone.performTouchInput {
            down(center)
            moveBy(Offset(18.dp.toPx(), 0f))
        }
        assertThat((state.phase as RecordGesture.Phase.Pressed).mayHold).isTrue()

        microphone.performTouchInput { moveBy(Offset(4.dp.toPx(), 0f)) }
        assertThat((state.phase as RecordGesture.Phase.Pressed).mayHold).isFalse()
    }
}

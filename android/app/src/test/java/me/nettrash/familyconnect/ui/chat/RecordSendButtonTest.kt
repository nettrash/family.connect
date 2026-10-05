/*
 * RecordSendButtonTest.kt
 * Family Connect (Android)
 *
 * The Send slot (#79, docs/audio-video-messages-2026-10-04.md, S1.3, S6,
 * S8.4): what TalkBack is told about it in every row — "Record voice
 * message", clicked as "start recording", the dimmed reason as its state,
 * "Stop and listen first" and "Delete recording" while recording, and NO
 * long-click action, so TalkBack's double-tap-and-hold does nothing — and
 * what each kind of input does with it: a finger or a stylus is the
 * reducer's press (in window coordinates), a mouse clicks on release and
 * opens the menu with its secondary button, Enter activates, and a gesture
 * torn down mid-press is a system cancel.
 *
 * A local Robolectric Compose test, like the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.test.requestFocus
import androidx.compose.ui.test.rightClick
import androidx.compose.ui.test.click
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Slot
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class RecordSendButtonTest {

    @get:Rule
    val compose = createComposeRule()

    /** Everything the button asked for, in order. */
    private val calls = mutableListOf<String>()
    private val downs = mutableListOf<Pair<Offset, Boolean>>()
    private val ups = mutableListOf<Pair<Offset, Boolean>>()

    private fun show(slot: Slot, coachMark: Boolean = false) {
        compose.setContent {
            // Away from the window's corner, so window coordinates are not
            // the button's own.
            Box(Modifier.padding(start = 60.dp, top = 30.dp)) {
                Button(slot, coachMark)
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun Button(slot: Slot, coachMark: Boolean = false) {
        RecordSendButton(
            slot = slot,
            onMicDown = { x, y, canHold, _ ->
                calls += "down"
                downs += Offset(x.toFloat(), y.toFloat()) to canHold
            },
            onMicMove = { _, _ -> if (calls.lastOrNull() != "move") calls += "move" },
            onMicUp = { x, y, inside ->
                calls += "up"
                ups += Offset(x.toFloat(), y.toFloat()) to inside
            },
            onMicCancel = { calls += "cancel" },
            onActivate = { calls += "activate" },
            onSend = { calls += "send" },
            onStopAndListen = { calls += "stop" },
            onDeleteRecording = { calls += "delete" },
            onRecordFromMenu = { calls += "menu record" },
            focusRequester = remember { FocusRequester() },
            coachMark = coachMark,
            onDismissCoachMark = { calls += "dismiss coach mark" },
        )
    }

    private val microphone get() = compose.onNodeWithContentDescription("Record voice message")

    @Test
    fun theMicrophoneIsARecordButtonThatStartsRecordingAndHasNoLongClick() {
        show(Slot.Microphone)

        val node = microphone
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.OnLongClick))
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.CustomActions))
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
            .assertIsEnabled()
            .fetchSemanticsNode()
        assertThat(node.config[SemanticsActions.OnClick].label).isEqualTo("start recording")

        // TalkBack's double tap is the click ACTION: the reducer's Activate,
        // never the microphone's touch (whose tap is down + up, below).
        microphone.performSemanticsAction(SemanticsActions.OnClick)
        assertThat(calls).containsExactly("activate")
    }

    /** Dimmed is not disabled: it stays enabled and focusable, and says why (S1.3, S6). */
    @Test
    fun aDimmedMicrophoneSaysWhyAsItsState() {
        show(Slot.Dimmed(ComposerSlot.Dimmed.CALL))

        microphone
            .assertIsEnabled()
            // Still a keyboard stop: it must be reachable to say why.
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.RequestFocus))
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.StateDescription,
                    "You can record a message after the call.",
                ),
            )
            .performSemanticsAction(SemanticsActions.OnClick)
        assertThat(calls).containsExactly("activate")
    }

    /** While recording: "Send voice message", with Stop and Delete as actions (S6). */
    @Test
    fun whileRecordingTheSlotIsSendVoiceMessageWithStopAndDeleteActions() {
        show(Slot.SendVoice)

        val node = compose.onNodeWithContentDescription("Send voice message")
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.OnLongClick))
            .fetchSemanticsNode()
        val actions = node.config[SemanticsActions.CustomActions]
        assertThat(actions.map { it.label }).containsExactly("Stop and listen first", "Delete recording").inOrder()
        assertThat(node.config[SemanticsActions.OnClick].label).isNull()

        actions[0].action()
        actions[1].action()
        assertThat(calls).containsExactly("stop", "delete").inOrder()
    }

    @Test
    fun aRecordingBesideWordsMakesTheSlotStopRecording() {
        show(Slot.StopRecording)

        val node = compose.onNodeWithContentDescription("Stop recording").fetchSemanticsNode()
        assertThat(node.config[SemanticsActions.CustomActions].map { it.label })
            .containsExactly("Stop and listen first", "Delete recording")
    }

    @Test
    fun sendAndSaveSendAndTheDisabledRowsTakeNothing() {
        show(Slot.Send)
        compose.onNodeWithContentDescription("Send").assertIsEnabled().performClick()
        assertThat(calls).containsExactly("send")
    }

    @Test
    fun theAssistantsChatHasTodaysDisabledSend() {
        show(Slot.SendDisabled)
        compose.onNodeWithContentDescription("Send")
            .assertIsNotEnabled()
            // Today's disabled Send was no keyboard stop, and is none now.
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.RequestFocus))
        compose.onNodeWithContentDescription("Send").performTouchInput { click() }
        assertThat(calls).isEmpty()
    }

    @Test
    fun aBlankEditsSaveIsDisabledAndNeverAMicrophone() {
        show(Slot.Save(enabled = false))
        compose.onNodeWithContentDescription("Save").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Record voice message").assertDoesNotExist()
    }

    /** A finger's tap is the reducer's press: down, then up inside — in window coordinates (S2.3). */
    @Test
    fun aFingerTapIsThePressDownAndUpInWindowCoordinates() {
        show(Slot.Microphone)
        val bounds = microphone.fetchSemanticsNode().boundsInRoot

        microphone.performTouchInput {
            down(center)
            advanceEventTime(100)
            up()
        }

        assertThat(calls).containsExactly("down", "up").inOrder()
        val (at, canHold) = downs.single()
        assertThat(canHold).isTrue()
        assertThat(at.x).isWithin(1f).of(bounds.center.x)
        assertThat(at.y).isWithin(1f).of(bounds.center.y)
        assertThat(ups.single().second).isTrue()
    }

    /** A press that wanders off and lifts outside is no tap (S1.1); its slide reached the reducer. */
    @Test
    fun aSlideIsReportedAndALiftOutsideIsNotInside() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            moveBy(Offset(-200f, 0f))
            up()
        }

        assertThat(calls).containsExactly("down", "move", "up").inOrder()
        assertThat(ups.single().second).isFalse()
    }

    /** A mouse clicks on release, and can never hold (S8.4). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun aMouseClickIsAPressThatCannotHold() {
        show(Slot.Microphone)

        microphone.performMouseInput { click() }

        assertThat(calls).containsExactly("down", "up").inOrder()
        assertThat(downs.single().second).isFalse()
        assertThat(ups.single().second).isTrue()
    }

    /** The mouse's secondary button opens the microphone's menu: "Record voice message" (S1.6). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun theSecondaryButtonOpensTheMenu() {
        show(Slot.Microphone)

        microphone.performMouseInput { rightClick() }
        compose.onNodeWithText("Record voice message").performClick()

        assertThat(calls).containsExactly("menu record")
    }

    /** Enter on the focused microphone records — activation, on the key's release (S6). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun enterOnTheFocusedMicrophoneActivatesIt() {
        show(Slot.Microphone)

        microphone.requestFocus()
        microphone.performKeyInput { pressKey(Key.Enter) }

        assertThat(calls).containsExactly("activate")
    }

    /** The Send arrow and the Stop square are plain clicks on release, never the microphone's press. */
    @Test
    fun aTapOnTheSendArrowActivatesIt() {
        show(Slot.SendVoice)

        compose.onNodeWithContentDescription("Send voice message").performTouchInput { click() }

        assertThat(calls).containsExactly("activate")
    }

    /**
     * A press torn down mid-way — the activity rebuilt, the button gone — is
     * the system cancelling the touch: the reducer locks a hold (S4).
     */
    @Test
    fun aGestureTornDownMidPressIsACancel() {
        var present by mutableStateOf(true)
        compose.setContent {
            if (present) Button(Slot.Microphone)
        }

        microphone.performTouchInput { down(center) }
        present = false
        compose.waitForIdle()

        assertThat(calls).containsExactly("down", "cancel").inOrder()
    }

    /**
     * The system cancelling the touch — an alert, the notification shade —
     * reaches the button as a lift that is already consumed: the reducer's
     * SystemCancel, which locks a hold rather than taking it as a release (S2.3).
     */
    @Test
    fun aSystemCancelIsACancelNotARelease() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            cancel()
        }

        assertThat(calls).containsExactly("down", "cancel").inOrder()
        assertThat(ups).isEmpty()
    }

    /** S7.2's coach mark, above the microphone; a tap takes it away. */
    @Test
    fun theCoachMarkSaysTheHoldAndATapDismissesIt() {
        show(Slot.Microphone, coachMark = true)

        compose.onNodeWithText("You can also hold the microphone while you talk.").performClick()

        assertThat(calls).containsExactly("dismiss coach mark")
    }

    /** The slot's 44-dp face in a 48-dp hit area: a lift just past the face still taps (S1.1). */
    @Test
    fun aLiftJustOutsideTheFaceIsStillInsideTheHitArea() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            // 44 dp wide: 23 dp right of centre is past the face, inside 48 dp.
            moveTo(Offset(centerX + 23.dp.toPx(), centerY))
            up()
        }

        assertThat(ups.single().second).isTrue()
    }

    /** No tooltip and no long click to fight the hold, in any row (S6). */
    @Test
    fun noRowOffersALongClick() {
        var slot by mutableStateOf<Slot>(Slot.Microphone)
        compose.setContent { Button(slot) }
        for (each in listOf(Slot.Microphone, Slot.HeldMicrophone, Slot.SendVoice, Slot.Send)) {
            slot = each
            compose.waitForIdle()
            val label = requireNotNull(each.label)
            val node = compose.onNodeWithContentDescription(label).fetchSemanticsNode()
            assertThat(node.config.getOrNull(SemanticsActions.OnLongClick)).isNull()
        }
    }
}

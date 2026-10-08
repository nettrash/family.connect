/*
 * RecordSendButtonTest.kt
 * Family Connect (Android)
 *
 * The Send slot (#79, docs/audio-video-messages-2026-10-04.md, S1.3, S6,
 * S8.4 — revised 2026-10-06: there is no hold): what TalkBack is told about
 * it in every row — "Record voice message", clicked as "start recording",
 * the dimmed reason as its state, "Stop and listen first" and "Delete
 * recording" while recording, and NO long-click action — and what each kind
 * of input does with it: a finger, a stylus or a mouse's primary button is a
 * click on release inside, however long it was held; a LONG PRESS records
 * nothing, opens nothing and buzzes nothing while it is down; a mouse's
 * secondary button opens the menu; Enter activates; a press that went down
 * inside the activation guard is ignored whole; a gesture torn down or
 * cancelled by the system activates nothing.
 *
 * A local Robolectric Compose test, like the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.isPopup
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
import androidx.compose.ui.test.longClick
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

    /** Every haptic anything under the button played. */
    private val haptics = mutableListOf<HapticFeedbackType>()

    /** What the activation guard answers, asked as a press goes down. */
    private var guarded = false

    private fun show(slot: Slot) {
        compose.setContent {
            Box(Modifier.padding(start = 60.dp, top = 30.dp)) {
                Button(slot)
            }
        }
    }

    @androidx.compose.runtime.Composable
    private fun Button(slot: Slot) {
        val feedback = remember {
            object : HapticFeedback {
                override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
                    haptics += hapticFeedbackType
                }
            }
        }
        CompositionLocalProvider(LocalHapticFeedback provides feedback) {
            RecordSendButton(
                slot = slot,
                onActivate = { calls += "activate" },
                onSend = { calls += "send" },
                onStopAndListen = { calls += "stop" },
                onDeleteRecording = { calls += "delete" },
                onRecordFromMenu = { calls += "menu record" },
                onRecordVideo = { calls += "menu video" },
                focusRequester = remember { FocusRequester() },
                pressIgnored = {
                    calls += "asked guard"
                    guarded
                },
            )
        }
    }

    private val microphone get() = compose.onNodeWithContentDescription("Record voice message")

    /** What the button did, without its questions to the guard. */
    private val did get() = calls.filter { it != "asked guard" }

    @Test
    fun theMicrophoneIsARecordButtonThatStartsRecordingAndHasNoLongClick() {
        show(Slot.Microphone)

        val node = microphone
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Button))
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.OnLongClick))
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
            .assertIsEnabled()
            .fetchSemanticsNode()
        assertThat(node.config[SemanticsActions.OnClick].label).isEqualTo("start recording")

        // TalkBack's double tap is the click ACTION: the reducer's Activate.
        microphone.performSemanticsAction(SemanticsActions.OnClick)
        assertThat(did).containsExactly("activate")
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
        assertThat(did).containsExactly("activate")
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
        assertThat(did).containsExactly("stop", "delete").inOrder()
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
        assertThat(did).containsExactly("send")
    }

    @Test
    fun theAssistantsChatHasTodaysDisabledSend() {
        show(Slot.SendDisabled)
        compose.onNodeWithContentDescription("Send")
            .assertIsNotEnabled()
            // Today's disabled Send was no keyboard stop, and is none now.
            .assert(SemanticsMatcher.keyNotDefined(SemanticsActions.RequestFocus))
        compose.onNodeWithContentDescription("Send").performTouchInput { click() }
        assertThat(did).isEmpty()
    }

    @Test
    fun aBlankEditsSaveIsDisabledAndNeverAMicrophone() {
        show(Slot.Save(enabled = false))
        compose.onNodeWithContentDescription("Save").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Record voice message").assertDoesNotExist()
    }

    /** A finger's tap on the microphone is its activation: a hands-free recording. */
    @Test
    fun aFingerTapOnTheMicrophoneActivatesIt() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            advanceEventTime(100)
            up()
        }

        assertThat(did).containsExactly("activate")
    }

    /**
     * THE OWNER'S RULE (2026-10-06): a long press on the microphone is not a
     * gesture. Held well past the system's long-press timeout it records
     * nothing, opens no menu or popup and plays no haptic while the finger is
     * down; when it lifts inside it is the button's ordinary tap.
     */
    @Test
    fun aLongPressOnTheMicrophoneStartsNothingAndOpensNothingWhileItIsDown() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            advanceEventTime(viewConfiguration.longPressTimeoutMillis * 4)
            moveBy(Offset(1f, 1f))
        }
        compose.waitForIdle()

        assertThat(did).isEmpty()
        assertThat(haptics).isEmpty()
        compose.onAllNodes(isPopup()).assertCountEquals(0)
        compose.onNodeWithText("Record voice message").assertDoesNotExist()
        compose.onNodeWithText("Record video message").assertDoesNotExist()

        microphone.performTouchInput { up() }

        assertThat(did).containsExactly("activate")
        assertThat(haptics).isEmpty()
        compose.onAllNodes(isPopup()).assertCountEquals(0)
    }

    /** The test framework's own long click, start to finish: one tap's activation, and no menu. */
    @Test
    fun aLongClickIsOneOrdinaryTap() {
        show(Slot.Microphone)

        microphone.performTouchInput { longClick() }

        assertThat(did).containsExactly("activate")
        compose.onAllNodes(isPopup()).assertCountEquals(0)
    }

    /** A long press that slides off and lifts outside is no tap (S1.1): nothing at all. */
    @Test
    fun aPressThatLiftsOutsideDoesNothing() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            advanceEventTime(viewConfiguration.longPressTimeoutMillis * 2)
            moveBy(Offset(-200f, -200f))
            up()
        }

        assertThat(did).isEmpty()
    }

    /**
     * The activation guard is asked as the press GOES DOWN (S1.1): one that
     * went down inside it is ignored whole, however late it lifts — and one
     * that went down after it taps, whatever the guard says by the lift.
     */
    @Test
    fun aPressThatWentDownInsideTheGuardIsIgnoredWhole() {
        show(Slot.Microphone)

        guarded = true
        microphone.performTouchInput { down(center) }
        guarded = false
        microphone.performTouchInput {
            advanceEventTime(1_000)
            up()
        }
        assertThat(calls).containsExactly("asked guard")

        calls.clear()
        microphone.performTouchInput { down(center) }
        guarded = true
        microphone.performTouchInput { up() }
        assertThat(calls).containsExactly("asked guard", "activate").inOrder()
    }

    /** A mouse's primary click activates (S8.4). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun aMouseClickActivates() {
        show(Slot.Microphone)

        microphone.performMouseInput { click() }

        assertThat(did).containsExactly("activate")
    }

    /** The mouse's secondary button opens the microphone's menu — the only way to it (S1.6). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun theSecondaryButtonOpensTheMenu() {
        show(Slot.Microphone)

        microphone.performMouseInput { rightClick() }
        compose.onNodeWithText("Record video message").assertExists()
        compose.onNodeWithText("Record voice message").performClick()

        assertThat(did).containsExactly("menu record")
    }

    /** Enter on the focused microphone records — activation, on the key's release (S6). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun enterOnTheFocusedMicrophoneActivatesIt() {
        show(Slot.Microphone)

        microphone.requestFocus()
        microphone.performKeyInput { pressKey(Key.Enter) }

        assertThat(did).containsExactly("activate")
    }

    /** The Send arrow and the Stop square are clicks on release, like everything else. */
    @Test
    fun aTapOnTheSendArrowActivatesIt() {
        show(Slot.SendVoice)

        compose.onNodeWithContentDescription("Send voice message").performTouchInput { click() }

        assertThat(did).containsExactly("activate")
    }

    /** A press torn down mid-way — the button gone — activates nothing. */
    @Test
    fun aGestureTornDownMidPressDoesNothing() {
        var present by mutableStateOf(true)
        compose.setContent {
            if (present) Button(Slot.Microphone)
        }

        microphone.performTouchInput { down(center) }
        present = false
        compose.waitForIdle()

        assertThat(did).isEmpty()
    }

    /** The system cancelling the touch — an alert, the shade — reaches the button consumed: nothing. */
    @Test
    fun aSystemCancelDoesNothing() {
        show(Slot.Microphone)

        microphone.performTouchInput {
            down(center)
            cancel()
        }

        assertThat(did).isEmpty()
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

        assertThat(did).containsExactly("activate")
    }

    /** No tooltip and no long click, in any row (S6). */
    @Test
    fun noRowOffersALongClick() {
        var slot by mutableStateOf<Slot>(Slot.Microphone)
        compose.setContent { Button(slot) }
        for (each in listOf(Slot.Microphone, Slot.SendVoice, Slot.StopRecording, Slot.Send)) {
            slot = each
            compose.waitForIdle()
            val label = requireNotNull(each.label)
            val node = compose.onNodeWithContentDescription(label).fetchSemanticsNode()
            assertThat(node.config.getOrNull(SemanticsActions.OnLongClick)).isNull()
        }
    }
}

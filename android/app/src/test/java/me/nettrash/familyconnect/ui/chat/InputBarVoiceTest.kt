/*
 * InputBarVoiceTest.kt
 * Family Connect (Android)
 *
 * The composer, wired for voice (#79, docs/audio-video-messages-2026-10-04.md,
 * S1.3, S1.5, S2.4): the slot is the microphone when the composer is empty
 * and Send once a character is typed; the assistant's chat keeps today's
 * disabled Send and loses "Record voice message"; the paperclip says "Record
 * voice message" and "Take video", never "Record audio" or "Record video";
 * the recording row takes the field's place AND the buttons', the field
 * staying composed underneath, hidden from TalkBack. And (decision 41, the
 * video recorder's layout, 2026-10-06): while the recorder is open the whole
 * composer row is not drawn, not in TalkBack's tree, and takes no touch.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.click
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.isPopup
import androidx.compose.ui.test.assertCountEquals
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Recording
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@RunWith(RobolectricTestRunner::class)
class InputBarVoiceTest {

    @get:Rule
    val compose = createComposeRule()

    private val field = TextFieldState()
    private val calls = mutableListOf<String>()

    private fun inputs(assistantChat: Boolean = false, recording: Recording = Recording.NONE) =
        ComposerSlot.SlotInputs(
            recorderOpen = false,
            recording = recording,
            editing = false,
            draftBlank = true,
            staged = false,
            assistantChat = assistantChat,
            canRecord = true,
            call = false,
            busy = false,
            notSent = false,
        )

    private fun show(
        hold: () -> RecordGesture.HoldState = { RecordGesture.HoldState() },
        assistantChat: Boolean = false,
        announcement: ChatViewModel.VoiceAnnouncement? = null,
        recorderOpen: () -> Boolean = { false },
    ) {
        compose.setContent {
            val state = hold()
            ComposerUnderRecorder(recorderOpen = recorderOpen()) {
            InputBar(
                state = field,
                onSend = { calls += "send" },
                replyDraft = null,
                replyAuthorName = "",
                onCancelReply = {},
                focusRequester = remember { FocusRequester() },
                isEditing = false,
                onCancelEdit = {},
                mediaState = ChatViewModel.MediaSendState.Idle,
                staged = emptyList(),
                onPickMedia = {},
                onPickFile = {},
                onPasteFromClipboard = {},
                onPasteContent = { ChatViewModel.PasteResult.TEXT },
                onPasteTruncated = {},
                onTakePhoto = {},
                onTakeVideo = { calls += "take video" },
                onRecordAudio = { calls += "record voice message" },
                showsRecordVoice = !assistantChat,
                slotInputs = inputs(assistantChat = assistantChat, recording = state.recording),
                hold = state,
                announcement = announcement,
                recordingMs = if (state.recording != Recording.NONE) 2_000L else null,
                onStopRecording = { calls += "stop" },
                onDeleteRecording = { calls += "delete" },
                onOtherAction = { calls += "other action" },
                // The slot's activation is recorded too: a touch that reached
                // the microphone must show up here.
                onActivateSlot = { calls += "activate" },
                showsPoll = false,
                onStartPoll = {},
                onDiscardStaged = {},
                onDismissMediaError = {},
                onShareLocation = {},
                showsAssistantMention = false,
                showsAssistantPicture = false,
                onShowAssistantPicture = {},
                showsDraw = false,
                onAskForPicture = {},
                pictureNotice = null,
                mentionPictureNotice = null,
                showsPictureDescriptionHint = false,
                assistantProcessor = null,
                assistantConsentNeeded = false,
                assistantIsUnnamed = false,
                onReviewAssistantConsent = {},
            )
            }
        }
    }

    private fun paperclip() = compose.onNodeWithContentDescription("Attach a photo, video or file")

    /** Empty: the microphone. One character: Send — in the same place (S1.3). */
    @Test
    fun anEmptyComposerShowsTheMicrophoneAndTypingMakesItSend() {
        show()
        compose.onNodeWithContentDescription("Record voice message").assertIsEnabled()

        field.setTextAndPlaceCursorAtEnd("ok")
        compose.onNodeWithContentDescription("Send").assertIsEnabled()
        compose.onNodeWithContentDescription("Record voice message").assertDoesNotExist()

        // Blank after trimming is still empty: whitespace never turns it into Send.
        field.setTextAndPlaceCursorAtEnd("   ")
        compose.onNodeWithContentDescription("Record voice message").assertIsDisplayed()
    }

    /** The paperclip's renames (S1.5): the pair reads as a pair. */
    @Test
    fun thePaperclipSaysRecordVoiceMessageAndTakeVideo() {
        show()
        paperclip().performClick()

        compose.onNodeWithText("Record audio").assertDoesNotExist()
        compose.onNodeWithText("Record video").assertDoesNotExist()
        compose.onNodeWithText("Take video").assertIsDisplayed()
        compose.onNodeWithText("Record voice message").performClick()

        assertThat(calls).containsExactly("other action", "record voice message").inOrder()
    }

    /** The assistant's chat: today's disabled Send, and no "Record voice message" (S1.5, Decision 24). */
    @Test
    fun theAssistantsChatHasNoMicrophoneAndNoRecordItem() {
        show(assistantChat = true)
        compose.onNodeWithContentDescription("Send").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Record voice message").assertDoesNotExist()

        paperclip().performClick()
        compose.onNodeWithText("Record voice message").assertDoesNotExist()
        compose.onNodeWithText("Take video").assertIsDisplayed()
    }

    /** Hands-free: Delete, Stop, and the slot sends (S2.4). */
    @Test
    fun handsFreeTheRecordingRowHasDeleteAndStopAndTheSlotSends() {
        show(hold = { RecordGesture.HoldState(phase = RecordGesture.Phase.HandsFree(besideDraft = false)) })

        paperclip().assertDoesNotExist()
        // The field stays composed under the row, hidden from TalkBack.
        compose.onNode(hasSetTextAction())
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.HideFromAccessibility))
        compose.onNodeWithContentDescription("Delete recording").performClick()
        compose.onNodeWithContentDescription("Stop recording").performClick()
        compose.onNodeWithContentDescription("Send voice message").assertIsDisplayed()

        assertThat(calls).containsExactly("delete", "stop").inOrder()
    }

    /** The composer carries the polite live region, over the recording row as anywhere (S6). */
    @Test
    fun theComposerSaysItsAnnouncementsPolitely() {
        show(
            hold = { RecordGesture.HoldState(phase = RecordGesture.Phase.HandsFree(besideDraft = false)) },
            announcement = ChatViewModel.VoiceAnnouncement("Recording", serial = 1),
        )
        compose.onNodeWithContentDescription("Recording")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.LiveRegion,
                    androidx.compose.ui.semantics.LiveRegionMode.Polite,
                ),
            )
    }

    /**
     * THE OWNER'S RULE (2026-10-06), in the real composer: a long press on
     * the microphone opens nothing — no paperclip menu, no microphone menu,
     * no popup of any kind — and starts nothing while the finger is down;
     * lifted inside, it is one ordinary tap.
     */
    @Test
    fun aLongPressOnTheComposersMicrophoneOpensNothingAndIsOneTapWhenLifted() {
        show()
        val mic = compose.onNodeWithContentDescription("Record voice message")

        mic.performTouchInput {
            down(center)
            advanceEventTime(viewConfiguration.longPressTimeoutMillis * 4)
            moveBy(androidx.compose.ui.geometry.Offset(1f, 0f))
        }
        compose.waitForIdle()
        assertThat(calls).isEmpty()
        compose.onAllNodes(isPopup()).assertCountEquals(0)
        compose.onNodeWithText("Take video").assertDoesNotExist()
        compose.onNodeWithText("Record video message").assertDoesNotExist()

        mic.performTouchInput { up() }
        compose.waitForIdle()
        assertThat(calls).containsExactly("activate")
        compose.onAllNodes(isPopup()).assertCountEquals(0)
    }

    /**
     * Decision 41: while the video recorder is open the composer row is NOT
     * DRAWN — no paperclip, field or microphone in TalkBack's tree — and a
     * touch where the paperclip and the microphone were reaches neither. It
     * comes back whole when the recorder closes, the draft with it.
     */
    @Test
    fun whileTheVideoRecorderIsOpenTheComposerIsNotThereAndTakesNoTouch() {
        var open by mutableStateOf(false)
        field.setTextAndPlaceCursorAtEnd("")
        show(recorderOpen = { open })
        val clip = paperclip().fetchSemanticsNode().boundsInRoot.center
        val mic = compose.onNodeWithContentDescription("Record voice message").fetchSemanticsNode().boundsInRoot.center

        open = true
        compose.waitForIdle()
        paperclip().assertDoesNotExist()
        compose.onNodeWithContentDescription("Record voice message").assertDoesNotExist()
        compose.onNode(hasSetTextAction()).assertDoesNotExist()
        compose.onRoot().performTouchInput { click(clip) }
        compose.onRoot().performTouchInput { click(mic) }
        compose.onRoot().performTouchInput { longClick(mic) }
        compose.waitForIdle()
        assertThat(calls).isEmpty()
        compose.onNodeWithText("Take video").assertDoesNotExist()

        open = false
        compose.waitForIdle()
        paperclip().assertIsDisplayed()
        compose.onNodeWithContentDescription("Record voice message").assertIsDisplayed()
    }
}

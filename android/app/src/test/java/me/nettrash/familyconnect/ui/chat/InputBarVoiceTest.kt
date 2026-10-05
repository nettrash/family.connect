/*
 * InputBarVoiceTest.kt
 * Family Connect (Android)
 *
 * The composer, wired for voice (#79, docs/audio-video-messages-2026-10-04.md,
 * S1.3, S1.5, S2.3, S2.4, S2.6): the slot is the microphone when the
 * composer is empty and Send once a character is typed; the assistant's chat
 * keeps today's disabled Send and loses "Record voice message"; the paperclip
 * says "Record voice message" and "Take video", never "Record audio" or
 * "Record video"; the hold row and the recording row take the field's place
 * AND the buttons', the field staying composed underneath, hidden from
 * TalkBack; the Undo row takes the field's alone and leaves the paperclip
 * usable — whose use ends the window.
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
    ) {
        compose.setContent {
            val state = hold()
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
                onUndoVoiceMessage = { calls += "undo" },
                onOtherAction = { calls += "other action" },
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

    /**
     * Held: the hold row takes the field's place AND the buttons' (S2.3); the
     * field stays composed under it, hidden from TalkBack, so a keyboard that
     * was up stays up.
     */
    @Test
    fun whileHeldTheHoldRowReplacesTheFieldAndTheButtons() {
        show(hold = {
            RecordGesture.HoldState(phase = RecordGesture.Phase.Holding(340.0, 780.0, rtl = false, armed = false))
        })

        compose.onNodeWithText("Slide to cancel").assertIsDisplayed()
        paperclip().assertDoesNotExist()
        compose.onNode(hasSetTextAction())
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.HideFromAccessibility))
        // The pressed microphone stays under the finger (S1.3 row 2).
        compose.onNodeWithContentDescription("Send voice message").assertIsDisplayed()
    }

    /** Hands-free: Delete, Stop, and the slot sends (S2.4). */
    @Test
    fun handsFreeTheRecordingRowHasDeleteAndStopAndTheSlotSends() {
        show(hold = { RecordGesture.HoldState(phase = RecordGesture.Phase.HandsFree(besideDraft = false)) })

        paperclip().assertDoesNotExist()
        compose.onNodeWithContentDescription("Delete recording").performClick()
        compose.onNodeWithContentDescription("Stop recording").performClick()
        compose.onNodeWithContentDescription("Send voice message").assertIsDisplayed()

        assertThat(calls).containsExactly("delete", "stop").inOrder()
    }

    /**
     * The Undo row takes the FIELD's place only: the paperclip stays usable
     * beside it — and using it ends the window, by sending (S2.6).
     */
    @Test
    fun theUndoRowLeavesThePaperclipUsableAndUsingItEndsTheWindow() {
        var undo by mutableStateOf<RecordGesture.UndoNote?>(
            RecordGesture.UndoNote(untilMs = Long.MAX_VALUE, recordedMs = 12_000),
        )
        show(hold = { RecordGesture.HoldState(undo = undo) })

        compose.onNodeWithText("Sending voice message · 0:12").assertIsDisplayed()
        compose.onNode(hasSetTextAction())
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.HideFromAccessibility))
        // The composer is empty, so the slot is the microphone (S2.6).
        compose.onNodeWithContentDescription("Record voice message").assertIsDisplayed()
        compose.onNodeWithText("Undo").performClick()
        paperclip().performClick()

        assertThat(calls).containsExactly("undo", "other action").inOrder()
        undo = null
        compose.onNode(hasSetTextAction())
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.HideFromAccessibility))
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
}

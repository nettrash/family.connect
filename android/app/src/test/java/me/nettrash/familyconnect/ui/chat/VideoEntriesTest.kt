/*
 * VideoEntriesTest.kt
 * Family Connect (Android)
 *
 * The ways into the video recorder (#79, docs/audio-video-messages-2026-10-04.md,
 * S1.2, S1.4, S1.5, S1.6, S6, Decision 40) — "the door": the video button
 * inside the empty field, the paperclip's "Record video message", the
 * microphone's secondary menu and its TalkBack action. Shown only where
 * S1.2's **round available** holds, in a family or a direct chat; gone the
 * moment a character is typed; dimmed — and saying why — during a call or
 * while an attachment is busy; ignoring a tap for 600 ms after it appears.
 *
 * Robolectric Compose on the composer itself; nothing here opens a camera.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.runtime.remember
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.rightClick
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.ui.chat.ComposerSlot.Recording
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.shadows.ShadowSystemClock
import java.time.Duration

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@RunWith(RobolectricTestRunner::class)
class VideoEntriesTest {

    @get:Rule
    val compose = createComposeRule()

    private val field = TextFieldState()
    private val calls = mutableListOf<String>()

    private fun slot(
        call: Boolean = false,
        busy: Boolean = false,
        notSent: Boolean = false,
        staged: Boolean = false,
        editing: Boolean = false,
        assistantChat: Boolean = false,
    ) = ComposerSlot.SlotInputs(
        recorderOpen = false,
        recording = Recording.NONE,
        editing = editing,
        draftBlank = true,
        staged = staged,
        assistantChat = assistantChat,
        canRecord = true,
        call = call,
        busy = busy,
        notSent = notSent,
    )

    private fun entries(
        slot: ComposerSlot.SlotInputs = slot(),
        familyOrDirectChat: Boolean = true,
        undoWindow: Boolean = false,
        serverOffersRound: Boolean = true,
        hasCamera: Boolean = true,
    ) = ComposerSlot.DoorInputs(
        slot = slot,
        familyOrDirectChat = familyOrDirectChat,
        undoWindow = undoWindow,
        serverOffersRound = serverOffersRound,
        hasCamera = hasCamera,
        encoderProbePasses = true,
        recordsRoundVideo = ComposerSlot.RECORDS_ROUND_VIDEO,
    )

    private fun show(videoEntries: ComposerSlot.DoorInputs? = entries()) {
        compose.setContent {
            val inputs = videoEntries?.slot ?: slot()
            InputBar(
                state = field,
                onSend = { calls += "send" },
                replyDraft = null,
                replyAuthorName = "",
                onCancelReply = {},
                focusRequester = remember { FocusRequester() },
                isEditing = inputs.editing,
                onCancelEdit = {},
                mediaState = if (inputs.busy) ChatViewModel.MediaSendState.Preparing else ChatViewModel.MediaSendState.Idle,
                staged = emptyList(),
                onPickMedia = {},
                onPickFile = {},
                onPasteFromClipboard = {},
                onPasteContent = { ChatViewModel.PasteResult.TEXT },
                onPasteTruncated = {},
                onTakePhoto = {},
                onTakeVideo = { calls += "take video" },
                onRecordAudio = { calls += "record voice message" },
                showsRecordVoice = !inputs.assistantChat,
                slotInputs = inputs,
                videoEntries = videoEntries,
                onRecordVideo = { calls += "record video message" },
                recordingMs = null,
                onStopRecording = {},
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

    private val videoButton get() = compose.onNodeWithContentDescription("Record video message")
    private val microphone get() = compose.onNodeWithContentDescription("Record voice message")
    private fun paperclip() = compose.onNodeWithContentDescription("Attach a photo, video or file")

    /** Past the 600 ms the button ignores after it appears (S1.1). */
    private fun pastTheGuard() {
        ShadowSystemClock.advanceBy(Duration.ofMillis(ComposerSlot.ACTIVATION_GUARD_MS))
    }

    // --- The video button (S1.4) -------------------------------------------------

    @Test
    fun theVideoButtonSitsInTheEmptyFieldAndOpensTheRecorder() {
        show()
        videoButton.assertIsDisplayed()
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
        pastTheGuard()
        videoButton.performClick()
        assertThat(calls).containsExactly("record video message")
    }

    /** It appears the moment a Send empties the field; a second tap that drifts onto it must not turn the camera on. */
    @Test
    fun aTapRightAfterTheVideoButtonAppearsIsIgnored() {
        show()
        videoButton.performClick()
        assertThat(calls).isEmpty()
        pastTheGuard()
        videoButton.performClick()
        assertThat(calls).containsExactly("record video message")
    }

    /** The guard's edge, on the button's own clock: 599 ms ignored, 600 ms opens (S1.1). */
    @Test
    fun theVideoButtonsGuardIsSixHundredMillisecondsFromWhenItAppeared() {
        var now = 10_000L
        compose.setContent {
            VideoDoorButton(
                door = ComposerSlot.Door.Shown,
                onOpen = { calls += "record video message" },
                uptime = { now },
            )
        }
        now += ComposerSlot.ACTIVATION_GUARD_MS - 1
        videoButton.performClick()
        assertThat(calls).isEmpty()

        now += 1
        videoButton.performClick()
        assertThat(calls).containsExactly("record video message")
    }

    /** "Hidden as soon as a character is typed (the field needs its width)" — and back when it is empty again. */
    @Test
    fun typingTakesTheVideoButtonAway() {
        show()
        field.setTextAndPlaceCursorAtEnd("o")
        videoButton.assertDoesNotExist()
        field.setTextAndPlaceCursorAtEnd("  ")
        videoButton.assertIsDisplayed()
    }

    @Test
    fun noVideoButtonWhereRoundIsNotAvailable() {
        // A server without the keys.
        show(entries(serverOffersRound = false))
        videoButton.assertDoesNotExist()
        microphone.assertIsDisplayed()
    }

    @Test
    fun noVideoButtonOnADeviceWithoutACamera() {
        show(entries(hasCamera = false))
        videoButton.assertDoesNotExist()
    }

    @Test
    fun noVideoButtonInTheAssistantsChat() {
        show(entries(slot = slot(assistantChat = true), familyOrDirectChat = false))
        videoButton.assertDoesNotExist()
    }

    @Test
    fun noVideoButtonWhileSomethingIsStagedOrAnEditIsOpen() {
        show(entries(slot = slot(staged = true)))
        videoButton.assertDoesNotExist()
    }

    @Test
    fun noVideoButtonWhileEditing() {
        show(entries(slot = slot(editing = true)))
        videoButton.assertDoesNotExist()
    }

    @Test
    fun noVideoButtonDuringTheUndoWindow() {
        show(entries(undoWindow = true))
        videoButton.assertDoesNotExist()
    }

    /** No entries offered at all — a thread, a build that cannot record: nothing. */
    @Test
    fun noEntriesNoVideoButton() {
        show(videoEntries = null)
        videoButton.assertDoesNotExist()
        paperclip().performClick()
        compose.onNodeWithText("Record video message").assertDoesNotExist()
    }

    /** Rows 7 and 8: dimmed, not disabled — focusable, hittable, and it says why (S1.3, S6). */
    @Test
    fun duringACallTheVideoButtonIsDimmedAndSaysWhy() {
        show(entries(slot = slot(call = true)))
        videoButton
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.StateDescription,
                    "You can record a message after the call.",
                ),
            )
            .assertIsEnabled()
        pastTheGuard()
        videoButton.performClick()
        // The chat's ViewModel says the sentence; the button only asks.
        assertThat(calls).containsExactly("record video message")
    }

    @Test
    fun whileAnAttachmentIsBusyTheVideoButtonIsDimmedAndSaysWhy() {
        show(entries(slot = slot(busy = true)))
        videoButton.assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.StateDescription,
                "Wait until the current attachment is done.",
            ),
        )
    }

    /** Row 9 is about voice: the video button is live while a voice message waits unsent. */
    @Test
    fun aWaitingVoiceMessageLeavesTheVideoButtonLive() {
        show(entries(slot = slot(notSent = true)))
        videoButton.assertIsDisplayed()
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
    }

    // --- The paperclip (S1.5) ------------------------------------------------------

    @Test
    fun thePaperclipOffersRecordVideoMessageRightBelowVoice() {
        show()
        paperclip().performClick()
        compose.onNodeWithText("Record voice message").assertIsDisplayed()
        compose.onNodeWithText("Record video message").assertIsEnabled().performClick()
        assertThat(calls).containsExactly("other action", "record video message").inOrder()
    }

    @Test
    fun thePaperclipItemIsDisabledDuringACall() {
        show(entries(slot = slot(call = true)))
        paperclip().performClick()
        compose.onNodeWithText("Record video message").assertIsNotEnabled()
    }

    /** Words typed or items staged: the item still opens — a video message travels alone (S1.5). */
    @Test
    fun thePaperclipItemWorksWithWordsTyped() {
        show()
        field.setTextAndPlaceCursorAtEnd("see you")
        paperclip().performClick()
        compose.onNodeWithText("Record video message").assertIsEnabled().performClick()
        assertThat(calls).contains("record video message")
    }

    @Test
    fun thePaperclipHasNoVideoItemWithoutTheServersKeys() {
        show(entries(serverOffersRound = false))
        paperclip().performClick()
        compose.onNodeWithText("Record video message").assertDoesNotExist()
    }

    // --- The microphone's menu and action (S1.6, S6) -------------------------------

    @OptIn(ExperimentalTestApi::class)
    @Test
    fun theMicrophonesMenuOffersRecordVideoMessage() {
        show()
        microphone.performMouseInput { rightClick() }
        compose.onNodeWithText("Record video message").performClick()
        assertThat(calls).containsExactly("record video message")
    }

    @OptIn(ExperimentalTestApi::class)
    @Test
    fun theMicrophonesMenuHasNoVideoItemWithoutRound() {
        show(entries(hasCamera = false))
        microphone.performMouseInput { rightClick() }
        compose.onNodeWithText("Record voice message").assertIsDisplayed()
        compose.onNodeWithText("Record video message").assertDoesNotExist()
    }

    @Test
    fun talkBackHasRecordVideoMessageOnTheMicrophone() {
        show()
        val node = microphone.fetchSemanticsNode()
        val action = node.config[SemanticsActions.CustomActions].single()
        assertThat(action.label).isEqualTo("Record video message")
        compose.runOnIdle { action.action() }
        assertThat(calls).containsExactly("record video message")
    }

    @Test
    fun noTalkBackActionWithoutRound() {
        show(entries(serverOffersRound = false))
        microphone.assert(SemanticsMatcher.keyNotDefined(SemanticsActions.CustomActions))
    }
}

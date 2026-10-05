/*
 * VoiceRowsTest.kt
 * Family Connect (Android)
 *
 * What takes the field's place while a voice message is recorded or waits
 * (#79, docs/audio-video-messages-2026-10-04.md, S2.3, S2.4, S2.6), and the
 * review pieces around it (S2.7, S2.8, S6, S1.7):
 *
 *  - the hold row: the clock and "‹ Slide to cancel", then "Release to
 *    cancel" once armed; the lines that replace the meter;
 *  - the recording row: Delete and Stop — no Stop beside a draft, where the
 *    slot is Stop — and "30 seconds left" from 4:30;
 *  - the Undo row: Undo, "Sending voice message · 0:12", and "Sending in 5"
 *    when animations are removed;
 *  - the staged voice note's ▶ / ❚❚ and its "0:12 / 0:42", dimmed while
 *    recording; the not-sent row's ▶;
 *  - the polite live region and the notice line's two new forms.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import java.io.File
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.ParkedRecording
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class VoiceRowsTest {

    @get:Rule
    val compose = createComposeRule()

    private val row = Modifier.width(360.dp).height(44.dp)

    @Test
    fun theHoldRowSaysSlideToCancelAndThenReleaseToCancel() {
        var armed by mutableStateOf(false)
        compose.setContent {
            HoldRow(recordedMs = 7_400, level = 3, line = null, armed = armed, modifier = row)
        }
        compose.onNodeWithText("0:07").assertIsDisplayed()
        compose.onNodeWithText("Slide to cancel").assertIsDisplayed()
        compose.onNodeWithText("Release to cancel").assertDoesNotExist()

        armed = true
        compose.onNodeWithText("Release to cancel").assertIsDisplayed()
        compose.onNodeWithText("Slide to cancel").assertDoesNotExist()
    }

    /** A hold let go too soon keeps recording and says so in the meter's place (S2.3). */
    @Test
    fun theStillRecordingLineTakesTheMetersPlace() {
        compose.setContent {
            HoldRow(
                recordedMs = 600,
                level = 2,
                line = ChatViewModel.VoiceLine.STILL_RECORDING,
                armed = false,
                modifier = row,
            )
        }
        compose.onNodeWithText("Still recording. Tap Send when you're done.").assertIsDisplayed()
    }

    @Test
    fun theRecordingRowHasDeleteAndStop() {
        var deleted = 0
        var stopped = 0
        compose.setContent {
            RecordingRow(
                recordedMs = 42_000,
                level = 4,
                line = null,
                besideDraft = false,
                onDelete = { deleted++ },
                onStop = { stopped++ },
                modifier = row,
            )
        }

        compose.onNodeWithText("0:42").assertIsDisplayed()
        compose.onNodeWithContentDescription("Delete recording").performClick()
        compose.onNodeWithContentDescription("Stop recording").performClick()

        assertThat(deleted).isEqualTo(1)
        assertThat(stopped).isEqualTo(1)
    }

    /** Beside words or staged items the SLOT is Stop, so the row draws none (S2.4). */
    @Test
    fun besideADraftTheRowDrawsNoStop() {
        compose.setContent {
            RecordingRow(
                recordedMs = 3_000, level = 0, line = null, besideDraft = true,
                onDelete = {}, onStop = {}, modifier = row,
            )
        }
        compose.onNodeWithContentDescription("Delete recording").assertIsDisplayed()
        compose.onNodeWithContentDescription("Stop recording").assertDoesNotExist()
    }

    /** From 4:30: "30 seconds left" in words as well as colour (S2.5, WCAG 1.4.1). */
    @Test
    fun thirtySecondsLeftIsShownInWords() {
        compose.setContent {
            RecordingRow(
                recordedMs = 271_000,
                level = 4,
                line = ChatViewModel.VoiceLine.THIRTY_SECONDS_LEFT,
                besideDraft = false,
                onDelete = {},
                onStop = {},
                modifier = row,
            )
        }
        compose.onNodeWithText("4:31").assertIsDisplayed()
        compose.onNodeWithText("30 seconds left").assertIsDisplayed()
    }

    @Test
    fun theSilenceWarningIsShownInTheMetersPlace() {
        compose.setContent {
            RecordingRow(
                recordedMs = 3_200,
                level = 0,
                line = ChatViewModel.VoiceLine.CANT_HEAR,
                besideDraft = false,
                onDelete = {},
                onStop = {},
                modifier = row,
            )
        }
        compose.onNodeWithText("We can't hear anything. Is the microphone muted?").assertIsDisplayed()
    }

    @Test
    fun theUndoRowOffersUndoAndSaysWhatIsSending() {
        var undone = 0
        compose.setContent {
            UndoRow(recordedMs = 12_000, windowMs = 5_000, onUndo = { undone++ }, modifier = row, steady = false)
        }

        compose.onNodeWithText("Sending voice message · 0:12").assertIsDisplayed()
        compose.onNodeWithText("Sending in", substring = true).assertDoesNotExist()
        compose.onNodeWithText("Undo").performClick()

        assertThat(undone).isEqualTo(1)
    }

    /** Without animations the emptying line gives way to "Sending in 5" (S2.6, S6). */
    @Test
    fun withoutAnimationsTheUndoRowCountsDownInWords() {
        compose.setContent {
            UndoRow(recordedMs = 12_000, windowMs = 5_000, onUndo = {}, modifier = row, steady = true)
        }
        compose.onNodeWithText("Sending in 5").assertIsDisplayed()
    }

    private val voiceNote = MediaPrep.Prepared(
        file = File("voice.m4a"),
        mime = "audio/mp4",
        kind = AttachmentDto.KIND_AUDIO,
        width = null,
        height = null,
        durationMs = 42_000,
        previewJpeg = null,
        name = null,
        voiceNote = true,
    )

    /** Review (S2.7): "[▶] Voice message · 0:42 [✕]", and "[❚❚] 0:12 / 0:42" while it plays. */
    @Test
    fun aStagedVoiceNotePlaysAndSaysWhereItIs() {
        var toggles = 0
        var discarded = 0
        var playing by mutableStateOf(false)
        compose.setContent {
            StagedAttachmentChip(
                staged = voiceNote,
                onDiscard = { discarded++ },
                playing = playing,
                positionMs = 12_300,
                onTogglePlay = { toggles++ },
            )
        }
        compose.onNodeWithText("Voice message · 0:42").assertIsDisplayed()
        compose.onNodeWithContentDescription("Play").performClick()
        compose.onNodeWithContentDescription("Delete recording").performClick()
        assertThat(toggles).isEqualTo(1)
        assertThat(discarded).isEqualTo(1)

        playing = true
        compose.onNodeWithText("0:12 / 0:42").assertIsDisplayed()
        compose.onNodeWithContentDescription("Pause").assertIsDisplayed()
        compose.onNodeWithText("Voice message · 0:42").assertDoesNotExist()
    }

    /** While recording, ▶ is dimmed — not disabled: it says why (S1.7, S6). */
    @Test
    fun whileRecordingItsPlayButtonSaysWhyItWaits() {
        var toggles = 0
        var explained = 0
        compose.setContent {
            StagedAttachmentChip(
                staged = voiceNote,
                onDiscard = {},
                onTogglePlay = { toggles++ },
                playDimmed = true,
                onPlayDimmed = { explained++ },
            )
        }

        compose.onNodeWithContentDescription("Play")
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.StateDescription,
                    "You can play this after recording.",
                ),
            )
            .performClick()

        assertThat(toggles).isEqualTo(0)
        assertThat(explained).isEqualTo(1)
    }

    /** "Voice message not sent · 0:42 [▶] [Send] [✕]" — its ▶ arrives with Phase 1 (S2.8). */
    @Test
    fun theNotSentRowHasItsPlayButton() {
        var toggles = 0
        compose.setContent {
            NotSentVoiceMessageRow(
                entry = ParkedRecording(id = "a", chatId = 42, file = "a.m4a", durationMs = 42_000),
                replyAuthorName = null,
                enabled = true,
                sending = false,
                onSend = {},
                onDelete = {},
                playing = false,
                onTogglePlay = { toggles++ },
            )
        }
        compose.onNodeWithContentDescription("Play").performClick()
        assertThat(toggles).isEqualTo(1)
    }

    /** S6: a polite live region in the composer says what changed. */
    @Test
    fun theAnnouncerIsAPoliteLiveRegion() {
        compose.setContent {
            VoiceAnnouncer(ChatViewModel.VoiceAnnouncement("Recording", serial = 1))
        }
        compose.waitForIdle()
        compose.onNodeWithContentDescription("Recording")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite))
    }

    /** The notice line's sentence that is not an error, and Open Settings after a refusal for good. */
    @Test
    fun theNoticeLineSaysWhyAndOffersOpenSettings() {
        var opened = 0
        var state by mutableStateOf<ChatViewModel.MediaSendState>(
            ChatViewModel.MediaSendState.Notice("You can record a message after the call."),
        )
        compose.setContent {
            MediaStrip(state = state, onDismiss = {}, onOpenSettings = { opened++ })
        }
        compose.onNodeWithText("You can record a message after the call.").assertIsDisplayed()
        compose.onNodeWithText("Open Settings").assertDoesNotExist()

        state = ChatViewModel.MediaSendState.Failed("Family needs permission to use the microphone.")
        compose.onNodeWithText("Open Settings").assertDoesNotExist()

        state = ChatViewModel.MediaSendState.Failed(
            "Family needs permission to use the microphone.",
            opensSettings = true,
        )
        compose.onNodeWithText("Open Settings").performClick()
        assertThat(opened).isEqualTo(1)
    }

    /** A rotation rebuilds the screen; the last announcement is not said a second time (S6). */
    @Test
    fun aRebuiltScreenDoesNotSayTheLastAnnouncementAgain() {
        val restorer = StateRestorationTester(compose)
        restorer.setContent {
            VoiceAnnouncer(ChatViewModel.VoiceAnnouncement("Recording", serial = 7))
        }
        compose.onNodeWithContentDescription("Recording").assertExists()

        restorer.emulateSavedInstanceStateRestore()

        compose.onNodeWithContentDescription("Recording").assertDoesNotExist()
    }

    /**
     * A cancelled touch is the background only when the app went there — a
     * configuration change tears the gesture down from a stopped activity
     * and must LOCK the hold, not park it (S4).
     */
    @Test
    fun aConfigurationChangeIsNeverTheBackground() {
        assertThat(touchCancelIsBackground(changingConfigurations = true, started = false)).isFalse()
        assertThat(touchCancelIsBackground(changingConfigurations = false, started = false)).isTrue()
        assertThat(touchCancelIsBackground(changingConfigurations = false, started = true)).isFalse()
    }
}

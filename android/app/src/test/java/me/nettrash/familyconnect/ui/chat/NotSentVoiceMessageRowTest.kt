/*
 * NotSentVoiceMessageRowTest.kt
 * Family Connect (Android)
 *
 * "Voice message not sent · 0:42 [Send] [✕]" (#79,
 * docs/audio-video-messages-2026-10-04.md, S2.8): the row a recording
 * something else stopped waits in. What a reader — and TalkBack — gets from
 * it: its length, the reply it answers and its caption; a Send that says
 * what it sends and does nothing a second time while it goes; a ✕ that is
 * "Delete recording". And the staged chip beside it, which calls a voice
 * note by its length now that it travels with no name.
 *
 * A local Robolectric Compose test, like the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
import java.io.File
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.ParkedRecording
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class NotSentVoiceMessageRowTest {

    @get:Rule
    val compose = createComposeRule()

    private val entry = ParkedRecording(
        id = "a",
        chatId = 42,
        file = "a.m4a",
        durationMs = 42_000,
        replyTo = ReplyToDto(messageId = 501, senderId = 9, excerpt = "Are you coming?"),
        caption = "for grandma",
    )

    @Test
    fun itSaysWhatItIsWhatItAnswersAndWhatItSays() {
        compose.setContent {
            NotSentVoiceMessageRow(
                entry = entry,
                replyAuthorName = "Ben",
                enabled = true,
                sending = false,
                onSend = {},
                onDelete = {},
            )
        }

        // The design's chip: "Not sent" and the length on it; TalkBack hears the whole sentence.
        compose.onNodeWithContentDescription("Voice message not sent · 0:42").assertIsDisplayed()
        compose.onNodeWithText("Not sent").assertIsDisplayed()
        compose.onNodeWithText("0:42").assertIsDisplayed()
        compose.onNodeWithText("Replying to Ben: Are you coming?").assertIsDisplayed()
        compose.onNodeWithText("for grandma").assertIsDisplayed()
        // Its length once: the waveform inside does not say it again.
        compose.onNodeWithContentDescription("Voice message, 0:42").assertDoesNotExist()
        compose.onNodeWithContentDescription("Voice message, 0:42", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun itsSendSaysWhatItSendsAndItsCrossIsDeleteRecording() {
        var sent = 0
        var deleted = 0
        compose.setContent {
            NotSentVoiceMessageRow(
                entry = entry,
                replyAuthorName = "Ben",
                enabled = true,
                sending = false,
                onSend = { sent++ },
                onDelete = { deleted++ },
            )
        }

        compose.onNodeWithContentDescription("Send voice message").assertIsEnabled().performClick()
        compose.onNodeWithContentDescription("Delete recording").assertIsEnabled().performClick()

        assertThat(sent).isEqualTo(1)
        assertThat(deleted).isEqualTo(1)
    }

    /** While it goes, neither button does anything a second time. */
    @Test
    fun whileItIsBeingSentNeitherButtonActs() {
        compose.setContent {
            NotSentVoiceMessageRow(
                entry = entry,
                replyAuthorName = "Ben",
                enabled = true,
                sending = true,
                onSend = {},
                onDelete = {},
            )
        }

        compose.onNodeWithContentDescription("Send voice message").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Delete recording").assertIsNotEnabled()
    }

    /** Without a reply or words, nothing is drawn for them. */
    @Test
    fun aBareOneIsJustItsLength() {
        compose.setContent {
            NotSentVoiceMessageRow(
                entry = entry.copy(replyTo = null, caption = "", durationMs = 61_500),
                replyAuthorName = null,
                enabled = true,
                sending = false,
                onSend = {},
                onDelete = {},
            )
        }

        compose.onNodeWithContentDescription("Voice message not sent · 1:01").assertIsDisplayed()
        compose.onNodeWithText("1:01").assertIsDisplayed()
        compose.onNodeWithText("Replying to", substring = true).assertDoesNotExist()
    }

    /** A voice note travels with no name, so the chip calls it by its length — never "Photo". */
    @Test
    fun aStagedVoiceNoteIsCalledAVoiceMessageWithItsLength() {
        compose.setContent {
            StagedAttachmentChip(
                staged = MediaPrep.Prepared(
                    file = File("voice.m4a"),
                    mime = "audio/mp4",
                    kind = AttachmentDto.KIND_AUDIO,
                    width = null,
                    height = null,
                    durationMs = 42_000,
                    previewJpeg = null,
                    name = null,
                    voiceNote = true,
                ),
                onDiscard = {},
            )
        }

        // Its length on the chip, and to TalkBack what it is (the design's chip has no title).
        compose.onNodeWithText("0:42").assertIsDisplayed()
        compose.onNodeWithContentDescription("Voice message, 0:42").assertIsDisplayed()
        compose.onNodeWithText("Photo").assertDoesNotExist()
    }
}

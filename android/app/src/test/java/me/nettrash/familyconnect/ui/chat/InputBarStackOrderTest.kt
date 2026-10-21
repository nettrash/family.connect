/*
 * InputBarStackOrderTest.kt
 * Family Connect (Android)
 *
 * What stacks above the composer row (#106,
 * docs/composer-and-wide-layout-2026-10-20.md, 2.4): one order on every
 * client, top to bottom, the nearest to the field being what is being
 * written — the notices (assistant consent, the `/draw` hint), the voice
 * messages not sent, the reply banner, the staged attachments, the mention
 * suggestions, then the busy/failure line directly above the row.
 *
 * A local Robolectric Compose test, like the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.runtime.remember
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import com.google.common.truth.Truth.assertThat
import java.io.File
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.ParkedRecording
import me.nettrash.familyconnect.util.MemberMention
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@RunWith(RobolectricTestRunner::class)
// Tall enough that every strip gets its own height: a squeezed Column would
// stack the overflow at one y and say nothing about the order.
@Config(qualifiers = "en-w600dp-h1600dp")
class InputBarStackOrderTest {

    @get:Rule
    val compose = createComposeRule()

    @Test
    fun everythingAboveTheRowStacksInTheSharedOrder() {
        compose.setContent {
            InputBar(
                state = remember { TextFieldState() },
                onSend = {},
                replyDraft = ReplyToDto(messageId = 501, senderId = 9, excerpt = "Are you coming?"),
                replyAuthorName = "Ben",
                onCancelReply = {},
                mentionCandidates = listOf(MemberMention.Suggestion.Member(MentionDto(8, "Anna"))),
                focusRequester = remember { FocusRequester() },
                isEditing = false,
                onCancelEdit = {},
                mediaState = ChatViewModel.MediaSendState.Failed("Couldn't send that"),
                staged = listOf(
                    MediaPrep.Prepared(
                        file = File("voice.m4a"),
                        mime = "audio/mp4",
                        kind = AttachmentDto.KIND_AUDIO,
                        width = null,
                        height = null,
                        durationMs = 7_000,
                        previewJpeg = null,
                        name = null,
                        voiceNote = true,
                    ),
                ),
                onPickMedia = {},
                onPickFile = {},
                onPasteFromClipboard = {},
                onPasteContent = { ChatViewModel.PasteResult.TEXT },
                onPasteTruncated = {},
                onTakePhoto = {},
                onTakeVideo = {},
                onRecordAudio = {},
                showsPoll = false,
                onStartPoll = {},
                recordingMs = null,
                onStopRecording = {},
                notSent = listOf(
                    ParkedRecording(id = "a", chatId = 42, file = "a.m4a", durationMs = 42_000),
                ),
                onDiscardStaged = {},
                onDismissMediaError = {},
                showsAssistantMention = false,
                onShareLocation = {},
                showsAssistantPicture = false,
                onShowAssistantPicture = {},
                showsDraw = false,
                onAskForPicture = {},
                pictureNotice = null,
                mentionPictureNotice = null,
                showsPictureDescriptionHint = true,
                assistantProcessor = null,
                assistantConsentNeeded = false,
                assistantIsUnnamed = true,
                onReviewAssistantConsent = {},
            )
        }

        fun top(node: SemanticsNodeInteraction) = node.fetchSemanticsNode().boundsInRoot.top
        val tops = listOf(
            "consent" to top(compose.onNodeWithText("which service answers", substring = true)),
            "draw hint" to top(compose.onNodeWithText("Describe people and things", substring = true)),
            "not sent" to top(compose.onNodeWithContentDescription("Voice message not sent · 0:42")),
            "reply" to top(compose.onNodeWithText("Replying to Ben")),
            "staged" to top(compose.onNodeWithContentDescription("Voice message, 0:07")),
            "mentions" to top(compose.onNodeWithContentDescription("Mention Anna")),
            "failure" to top(compose.onNodeWithText("Couldn't send that")),
            "field" to top(compose.onNode(hasSetTextAction())),
        )
        assertThat(tops.sortedBy { it.second }.map { it.first })
            .containsExactlyElementsIn(tops.map { it.first })
            .inOrder()
        assertThat(tops.map { it.second }.distinct()).hasSize(tops.size)
    }
}

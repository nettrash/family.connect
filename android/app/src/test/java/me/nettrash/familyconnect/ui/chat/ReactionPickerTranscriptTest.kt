/*
 * ReactionPickerTranscriptTest.kt
 * Family Connect (Android)
 *
 * The REAL long-press popup on a recording offers "Show text" (#79, the
 * approved design). The popup is composed beside the chat's Scaffold —
 * outside the block that provides LocalTranscripts — so it must be handed the
 * screen's Transcripts; when it read the local instead, it found null there
 * and the item never appeared in the app, while the tests that built
 * MessageContextMenu directly (with onShowText passed in by hand) passed.
 *
 * Composed as the chat composes it: the app-wide playback owner provided (as
 * MainActivity does), LocalTranscripts NOT provided (as at the popup's call
 * site).
 */

package me.nettrash.familyconnect.ui.chat

import androidx.activity.ComponentActivity
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.repo.TranscriptRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTranscriptApi
import me.nettrash.familyconnect.testutil.FakeTranscriptSound
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
class ReactionPickerTranscriptTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private lateinit var db: AppDatabase
    private lateinit var scope: CoroutineScope

    private val settings = FakeSettingsRepository(
        SettingsState(
            myUserId = ME,
            assistantProcessor = "Microsoft — Azure OpenAI",
            assistantTranscribe = true,
            assistantTranscribeMaxBytes = 25_000_000,
            assistantConsentAt = "2026-10-01T10:00:00Z",
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb(Dispatchers.Main)
        scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)
    }

    @After
    fun tearDown() {
        scope.cancel()
        db.close()
    }

    private fun transcripts() = Transcripts(
        scope = scope,
        settings = settings,
        repository = TranscriptRepository(FakeTranscriptApi(), db.transcriptDao(), FakeTranscriptSound()),
        agree = { true },
    )

    private val voice = AttachmentDto(id = 77, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4096, durationMs = 42_000)
    private val circle = AttachmentDto(
        id = 91, kind = "video", mime = "video/mp4", size = 1_649_700, width = 480, height = 480,
        durationMs = 23_400, hasPreview = true, round = true,
    )

    private fun mine(attachment: AttachmentDto, body: String = "") = ChatListItem.MessageItem(
        entity = MessageEntity(
            clientMsgId = "m${attachment.id}",
            serverId = 500,
            chatId = 3,
            senderId = ME,
            body = body,
            createdAt = 1_700_000_000_000,
            status = MessageStatus.SENT,
            attachmentsJson = AttachmentsCodec.encode(listOf(attachment)),
        ),
        showSenderName = false,
        senderName = null,
        showTimestamp = false,
    )

    private fun showPopup(
        attachment: AttachmentDto,
        transcripts: Transcripts?,
        body: String = "",
        onOpenFullScreen: (() -> Unit)? = null,
    ) {
        compose.setContent {
            // What MainActivity provides for the whole app — and nothing more:
            // the chat's LocalTranscripts block does not reach the popup.
            CompositionLocalProvider(LocalPlaybackCoordinator provides PlaybackCoordinator()) {
                ReactionPickerPopup(
                    target = ReactionPickerTarget(mine(attachment, body), Rect(40f, 400f, 300f, 480f)),
                    transcripts = transcripts,
                    onOpenFullScreen = onOpenFullScreen,
                    myUserId = ME,
                    onPick = {},
                    onMore = {},
                    onReply = {},
                    onEdit = {},
                    onClosePoll = {},
                    onCopy = {},
                    onShare = {},
                    onSave = {},
                    onReport = {},
                    onToggleBlock = {},
                    blockedUserIds = emptySet(),
                    assistantUserId = null,
                    isAiChat = false,
                    onDismiss = {},
                    chatKind = "direct",
                )
            }
        }
        compose.waitForIdle()
    }

    @Test
    fun theVoiceMessagesRealMenuOffersShowTextAndSpeed() {
        showPopup(voice, transcripts())
        compose.onNodeWithText("Reply").assertExists()
        compose.onNodeWithText("Show text").assertExists()
        compose.onNodeWithContentDescription("Playback speed, 1×").assertExists()
        compose.onNodeWithText("Copy").assertDoesNotExist()
    }

    @Test
    fun theVideoMessagesRealMenuOffersShowText() {
        showPopup(circle, transcripts())
        compose.onNodeWithText("Show text").assertExists()
        compose.onNodeWithText("Playback speed").assertDoesNotExist()
    }

    @Test
    fun withNoTranscriptsThereIsNoItem() {
        showPopup(voice, transcripts = null)
        compose.onNodeWithText("Reply").assertExists()
        compose.onNodeWithText("Show text").assertDoesNotExist()
    }

    /** The menu's rows, top to bottom, of those named in [labels] that are drawn. */
    private fun rowsInOrder(vararg labels: String): List<String> = labels
        .mapNotNull { label ->
            compose.onAllNodesWithText(label, useUnmergedTree = true).fetchSemanticsNodes().firstOrNull()?.let { label to it.boundsInRoot.top }
        }
        .sortedBy { it.second }
        .map { it.first }

    /**
     * CROSS-CLIENT PARITY (#79, the approved design's menu): a voice message's
     * menu is Reply, Show text, Playback speed, Save — no Copy, no Edit, NO
     * SHARE — in the order iOS, the Mac, Windows and the web draw it.
     */
    @Test
    fun theVoiceMessagesMenuIsTheOtherClientsMenu() {
        showPopup(voice, transcripts())
        assertThat(rowsInOrder("Reply", "Show text", "Playback speed", "Save", "Share", "Copy", "Edit"))
            .containsExactly("Reply", "Show text", "Playback speed", "Save")
            .inOrder()
    }

    /** A video message's: Save BEFORE Open full screen, and no Share — as on iOS, Windows and the web. */
    @Test
    fun theVideoMessagesMenuIsTheOtherClientsMenu() {
        showPopup(circle, transcripts(), onOpenFullScreen = {})
        assertThat(
            rowsInOrder("Reply", "Show text", "Playback speed", "Save to gallery", "Open full screen", "Share", "Copy"),
        ).containsExactly("Reply", "Show text", "Save to gallery", "Open full screen").inOrder()
    }

    /**
     * A voice note sent WITH WORDS is a message with words on every other
     * client (one audio attachment and no words is the voice message): the
     * ordinary menu — Copy and Share, no Show text, no Playback speed — and
     * its Save still the system's save screen, never the gallery.
     */
    @Test
    fun aCaptionedVoiceNoteKeepsTheOrdinaryMenu() {
        showPopup(voice, transcripts(), body = "for grandma")
        compose.onNodeWithText("Copy").assertExists()
        compose.onNodeWithText("Share").assertExists()
        compose.onNodeWithText("Save").assertExists()
        compose.onNodeWithText("Save to gallery").assertDoesNotExist()
        compose.onNodeWithText("Show text").assertDoesNotExist()
        compose.onNodeWithText("Playback speed").assertDoesNotExist()
    }

    private companion object {
        const val ME = 7L
    }
}

/*
 * VoiceDesignTest.kt
 * Family Connect (Android)
 *
 * The rest of the approved design for voice and video messages (#79), on
 * Robolectric — the pieces that are not a bubble:
 *
 *  - the hold row's "‹ Slide to cancel" shimmers, and is still words when
 *    animations are removed; the Undo row's line drains, and is "Sending in 5"
 *    without animations;
 *  - the hands-free row's LIVE waveform, in pixels: the newest peak at the
 *    trailing edge, older ones running back from it, mirrored right to left;
 *  - the lock pill floating above the held microphone, 36 wide;
 *  - the long-press menu on a recording: Show text, Playback speed with its
 *    value (which steps without closing the menu), Save — and no Copy or
 *    Edit where there is no text; an ordinary message's menu as it was;
 *  - the transcript item's decision, and no Edit on a word-less voice message.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertWidthIsEqualTo
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.db.MessageEntity
import me.nettrash.familyconnect.data.db.MessageStatus
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.pixelsOf
import me.nettrash.familyconnect.ui.components.VoiceBubbleRules
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class VoiceDesignTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    // -- The rows --------------------------------------------------------------------

    @Test
    fun slideToCancelShimmersAndIsStillWordsWithoutAnimations() {
        var steady by mutableStateOf(false)
        compose.setContent { SlideToCancel(steady = steady) }
        compose.onNodeWithTag("voice-slide-shimmer", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("Slide to cancel").assertExists()

        steady = true
        compose.onNodeWithTag("voice-slide-shimmer", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("voice-slide-steady", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("Slide to cancel").assertExists()
    }

    @Test
    fun theUndoRowDrainsOrCountsDown() {
        var steady by mutableStateOf(false)
        compose.setContent {
            UndoRow(
                recordedMs = 12_000, windowMs = 5_000, onUndo = {}, steady = steady,
                modifier = Modifier.width(360.dp).height(44.dp),
            )
        }
        compose.onNodeWithTag("voice-undo-drain", useUnmergedTree = true).assertExists()
        compose.onNodeWithText("Sending in", substring = true).assertDoesNotExist()

        steady = true
        compose.onNodeWithTag("voice-undo-drain", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithText("Sending in 5").assertExists()
    }

    private val red = Color(0xFFD01010)

    private fun liveWave(levels: List<Int>, rtl: Boolean = false) {
        compose.setContent {
            MaterialTheme(colorScheme = lightColorScheme(error = red)) {
                CompositionLocalProvider(LocalLayoutDirection provides if (rtl) LayoutDirection.Rtl else LayoutDirection.Ltr) {
                    Box(Modifier.background(Color.White).testTag("live")) {
                        LiveWaveform(levels = levels, steady = false, modifier = Modifier.width(100.dp))
                    }
                }
            }
        }
    }

    /** How many inked pixels a column has. */
    private fun inked(x: Int): Int {
        val pixels = compose.pixelsOf("live")
        return (0 until pixels.height).count { y -> pixels[x, y] != Color.White.toArgb() }
    }

    @Test
    fun theLiveWaveformScrollsInFromTheTrailingEdge() {
        // Oldest first: a loud peak, then a quiet one — the newest.
        liveWave(listOf(15, 0))
        val width = compose.pixelsOf("live").width
        val density = compose.density.density
        val newest = inked(width - (1.5f * density).toInt())
        val older = inked(width - ((3 + 2 + 1.5f) * density).toInt())
        // The quiet newest bar at the very edge, the loud one just before it.
        assertThat(newest).isGreaterThan(0)
        assertThat(older).isGreaterThan(newest * 3)
        // Nothing yet at the leading side.
        assertThat(inked((1.5f * density).toInt())).isEqualTo(0)
    }

    @Test
    fun rightToLeftTheLiveWaveformComesInFromTheLeft() {
        liveWave(listOf(15), rtl = true)
        val width = compose.pixelsOf("live").width
        val density = compose.density.density
        assertThat(inked((1.5f * density).toInt())).isGreaterThan(0)
        assertThat(inked(width - (1.5f * density).toInt())).isEqualTo(0)
    }

    @Test
    fun theLockPillFloatsAboveTheHeldMicrophone() {
        compose.setContent {
            RecordSendButton(
                slot = ComposerSlot.Slot.HeldMicrophone,
                onMicDown = { _, _, _, _ -> },
                onMicMove = { _, _ -> },
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
        compose.onNodeWithTag("voice-lock-pill", useUnmergedTree = true)
            .assertExists()
            .assertWidthIsEqualTo(36.dp)
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.HideFromAccessibility))
        // Visibly larger under the finger (the design's 1.35).
        assertThat(HELD_SCALE).isEqualTo(1.35f)
    }

    // -- The menu --------------------------------------------------------------------

    private val voice = AttachmentDto(id = 77, kind = AttachmentDto.KIND_AUDIO, mime = "audio/mp4", size = 4096, durationMs = 42_000)
    private val circle = AttachmentDto(
        id = 91, kind = "video", mime = "video/mp4", size = 1_649_700, width = 480, height = 480,
        durationMs = 23_400, hasPreview = true, round = true,
    )
    private val photo = AttachmentDto(id = 5, kind = AttachmentDto.KIND_PHOTO, mime = "image/jpeg", size = 9, width = 4, height = 3)

    private fun message(attachments: List<AttachmentDto>, body: String = "", senderId: Long = 9) = MessageEntity(
        clientMsgId = "m1",
        serverId = 1,
        chatId = 42,
        senderId = senderId,
        body = body,
        createdAt = 1_700_000_000_000,
        status = MessageStatus.SENT,
        attachmentsJson = AttachmentsCodec.encode(attachments).takeIf { attachments.isNotEmpty() },
    )

    @Test
    fun aRecordingIsKnownByItsOneAttachment() {
        assertThat(RecordingMenu.kindOf(message(listOf(voice)))).isEqualTo(RecordingMenu.Kind.VOICE)
        assertThat(RecordingMenu.kindOf(message(listOf(circle)))).isEqualTo(RecordingMenu.Kind.VIDEO)
        assertThat(RecordingMenu.kindOf(message(listOf(photo)))).isNull()
        assertThat(RecordingMenu.kindOf(message(emptyList(), body = "hi"))).isNull()
        // Words beside a voice note make it a message with words, as on every other client.
        assertThat(RecordingMenu.kindOf(message(listOf(voice), body = "for grandma"))).isNull()
        // No Share on a recording (the design, and iOS); every other message keeps it.
        assertThat(RecordingMenu.offersShare(RecordingMenu.Kind.VOICE)).isFalse()
        assertThat(RecordingMenu.offersShare(RecordingMenu.Kind.VIDEO)).isFalse()
        assertThat(RecordingMenu.offersShare(null)).isTrue()
        assertThat(RecordingMenu.offersSpeed(RecordingMenu.Kind.VOICE)).isTrue()
        assertThat(RecordingMenu.offersSpeed(RecordingMenu.Kind.VIDEO)).isFalse()
        // No Copy and no Edit where there is no text; words beside a voice note keep both.
        assertThat(RecordingMenu.offersCopy(message(listOf(voice)))).isFalse()
        assertThat(canEditMessage(message(listOf(voice), senderId = 7), myUserId = 7)).isFalse()
        assertThat(RecordingMenu.offersCopy(message(listOf(voice), body = "for grandma"))).isTrue()
        assertThat(canEditMessage(message(listOf(voice), body = "for grandma", senderId = 7), myUserId = 7)).isTrue()
        // An ordinary message is as it was.
        assertThat(canEditMessage(message(emptyList(), body = "hi", senderId = 7), myUserId = 7)).isTrue()
        assertThat(canEditMessage(message(listOf(photo), senderId = 7), myUserId = 7)).isTrue()
    }

    @Test
    fun theVoiceMenuHasShowTextSpeedAndSaveAndNoCopyOrEdit() {
        var shown = 0
        var stepped = 0
        var speed by mutableStateOf("1×")
        compose.setContent {
            MessageContextMenu(
                onReply = {},
                onEdit = {},
                onClosePoll = {},
                onCopy = {},
                onShare = {},
                onSave = {},
                canEdit = canEditMessage(message(listOf(voice), senderId = 7), myUserId = 7),
                canCopy = RecordingMenu.offersCopy(message(listOf(voice))),
                canSave = true,
                onShowText = TranscriptMenuAction(R.string.s_transcript_show) { shown++ },
                playbackSpeed = speed,
                onCyclePlaybackSpeed = {
                    stepped++
                    speed = "1.5×"
                },
                saveLabel = "Save",
            )
        }
        compose.onNodeWithText("Reply").assertExists()
        compose.onNodeWithText("Show text").performClick()
        compose.onNodeWithText("Save").assertExists()
        compose.onNodeWithText("Copy").assertDoesNotExist()
        compose.onNodeWithText("Edit").assertDoesNotExist()
        compose.onNodeWithText("Save to gallery").assertDoesNotExist()
        compose.onNodeWithContentDescription("Playback speed, 1×").performClick()
        // It stays, and says the new value.
        compose.onNodeWithContentDescription("Playback speed, 1.5×").assertExists()
        assertThat(shown).isEqualTo(1)
        assertThat(stepped).isEqualTo(1)
    }

    @Test
    fun anOrdinaryMessagesMenuIsAsItWas() {
        compose.setContent {
            MessageContextMenu(
                onReply = {}, onEdit = {}, onClosePoll = {}, onCopy = {}, onShare = {}, onSave = {},
                canEdit = true, canCopy = true, canSave = true,
            )
        }
        compose.onNodeWithText("Copy").assertExists()
        compose.onNodeWithText("Edit").assertExists()
        compose.onNodeWithText("Save to gallery").assertExists()
        compose.onNodeWithText("Show text").assertDoesNotExist()
        compose.onNodeWithText("Playback speed").assertDoesNotExist()
    }

    @Test
    fun theTranscriptItemFollowsTheLinesOwnDecision() {
        fun action(held: Boolean?, status: TranscriptRequests.Status?, offered: Boolean, canAsk: Boolean = true) =
            TranscriptRules.menuAction(held, status, offered, canAsk, reveal = {}, hide = {}, ask = {})?.label
        assertThat(action(held = true, status = null, offered = true)).isEqualTo(R.string.s_transcript_hide)
        assertThat(action(held = false, status = null, offered = false)).isEqualTo(R.string.s_transcript_show)
        assertThat(action(held = null, status = null, offered = true)).isEqualTo(R.string.s_transcript_show)
        assertThat(action(held = null, status = null, offered = true, canAsk = false)).isNull()
        assertThat(action(held = null, status = null, offered = false)).isNull()
        assertThat(action(held = null, status = TranscriptRequests.Status.LOADING, offered = true)).isNull()
        assertThat(action(held = null, status = TranscriptRequests.Status.REFUSED, offered = true)).isNull()
        assertThat(action(held = null, status = TranscriptRequests.Status.FAILED, offered = true))
            .isEqualTo(R.string.s_transcript_show)
    }

    @Test
    fun theSpeedChipsWordsAreTheStrings() {
        assertThat(VoiceBubbleRules.speedLabel(1f)).isEqualTo(R.string.s_speed_1x)
        assertThat(VoiceBubbleRules.speedLabel(1.5f)).isEqualTo(R.string.s_speed_1_5x)
        assertThat(VoiceBubbleRules.speedLabel(2f)).isEqualTo(R.string.s_speed_2x)
    }
}

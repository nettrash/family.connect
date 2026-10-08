/*
 * RoundVideoBubbleTest.kt
 * Family Connect (Android)
 *
 * The circle as TalkBack and a finger meet it (#79,
 * docs/audio-video-messages-2026-10-04.md, S5.2-S5.6, S6): "Video message,
 * 0:23", "Not played" while the dot shows, Play as the click and "Open full
 * screen" as an action; a tap plays it in place with the ring and the expand
 * control; loading, failure and the sender's own circle on its way; dimmed
 * while the composer records; 200 dp in a phone's window and 240 dp from
 * 600 dp.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsEqualTo
import androidx.compose.ui.test.assertWidthIsEqualTo
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
class RoundVideoBubbleTest {

    @get:Rule
    val compose = createComposeRule()

    private val circle = AttachmentDto(
        id = 91, kind = "video", mime = "video/mp4", size = 1_649_700, width = 480, height = 480,
        durationMs = 23_400, hasPreview = true, round = true,
    )

    private class Player : VideoPlayer {
        var started = 0
        override fun setSurface(surface: android.view.Surface?) = Unit
        override fun start() {
            started++
        }

        override fun pause() = Unit
        override fun seekTo(positionMs: Int) = Unit
        override val positionMs: Int = 0
        override fun release() = Unit
    }

    /** [ready]: the stream prepares the moment it is opened. [opens]: there is one at all. */
    private class Factory(private val ready: Boolean = true, private val opens: Boolean = true) : VideoPlayerFactory {
        var opened = 0
        val players = mutableListOf<Player>()
        val events = mutableListOf<VideoPlayer.Events>()
        override suspend fun open(attachmentId: Long, events: VideoPlayer.Events): VideoPlayer? {
            opened++
            this.events += events
            if (!opens) return null
            if (ready) events.onReady()
            return Player().also { players += it }
        }
    }

    private val owner = PlaybackCoordinator()
    private var fullScreen = 0
    private var explained = 0

    private fun show(
        factory: Factory = Factory(),
        isMine: Boolean = false,
        acked: Boolean = true,
        sending: Boolean = false,
        recording: Boolean = false,
    ) {
        compose.setContent {
            CompositionLocalProvider(
                LocalPlaybackCoordinator provides owner,
                LocalVideoPlayerFactory provides factory,
                LocalRecordingGate provides RecordingGate(recording = recording, explain = { explained++ }),
            ) {
                RoundVideoBubble(
                    attachment = circle,
                    isMine = isMine,
                    acked = acked,
                    sending = sending,
                    streamUrl = { null },
                    onOpenFullScreen = { fullScreen++ },
                    onLongPress = {},
                    onDoubleTap = {},
                )
            }
        }
    }

    private fun circleNode() = compose.onNodeWithContentDescription("Video message, 0:23")

    @Test
    fun `talkback hears the length, that it is new, Play, and Open full screen`() {
        show()

        circleNode()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Not played"))
            .assert(SemanticsMatcher("click label is Play") { it.config[SemanticsActions.OnClick].label == "Play" })
            .assert(
                SemanticsMatcher("Open full screen action") { node ->
                    node.config[SemanticsActions.CustomActions].map { it.label } == listOf("Open full screen")
                },
            )
        compose.onNodeWithTag("round-video-unplayed", useUnmergedTree = true).assertExists()

        val action = circleNode().fetchSemanticsNode().config[SemanticsActions.CustomActions].single()
        compose.runOnIdle { action.action() }
        assertThat(fullScreen).isEqualTo(1)
    }

    @Test
    fun `the reader's own circle has no dot`() {
        show(isMine = true)

        circleNode().assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
        compose.onNodeWithTag("round-video-unplayed", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun `a tap plays it in place - the ring, the expand control, the dot until the end`() {
        val factory = Factory()
        show(factory)

        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.waitForIdle()

        assertThat(factory.players.single().started).isEqualTo(1)
        compose.onNodeWithTag("round-video-progress", useUnmergedTree = true).assertExists()
        // Not played until it has played to the end (S5.3).
        compose.onNodeWithTag("round-video-unplayed", useUnmergedTree = true).assertExists()
        circleNode()
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Not played"))
            .assert(SemanticsMatcher("click label is Pause") { it.config[SemanticsActions.OnClick].label == "Pause" })

        // The expand control opens the viewer, and the circle stops for it.
        compose.onNodeWithTag("round-video-expand", useUnmergedTree = true).performClick()
        compose.waitForIdle()
        assertThat(fullScreen).isEqualTo(1)
        compose.onNodeWithTag("round-video-expand", useUnmergedTree = true).assertExists() // paused, still at its frame
        circleNode().assert(
            SemanticsMatcher("click label is Play again") { it.config[SemanticsActions.OnClick].label == "Play" },
        )

        // Played on to the end: back at the poster, and the dot goes.
        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.runOnIdle { factory.events.single().onEnded() }
        compose.onNodeWithTag("round-video-unplayed", useUnmergedTree = true).assertDoesNotExist()
        // ...and says "Played", as the voice bubble and every other client's circle do.
        circleNode().assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Played"))
    }

    @Test
    fun `while it loads a ring shows, and a second tap gives up`() {
        show(Factory(ready = false))

        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.onNodeWithTag("round-video-loading", useUnmergedTree = true).assertExists()

        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.onNodeWithTag("round-video-loading", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("round-video-progress", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun `a stream that cannot be had says so, and the next tap tries again`() {
        val factory = Factory(opens = false)
        show(factory)

        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.onNodeWithText("Couldn't load the video. Tap to try again.").assertExists()

        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.waitForIdle()
        assertThat(factory.opened).isEqualTo(2)
    }

    @Test
    fun `the sender's own circle on its way says Sending and plays nothing`() {
        val factory = Factory()
        show(factory, isMine = true, acked = false, sending = true)

        compose.onNodeWithText("Sending…").assertExists()
        compose.onNodeWithTag("round-video-sending", useUnmergedTree = true).assertExists()
        circleNode().performSemanticsAction(SemanticsActions.OnClick)
        compose.waitForIdle()
        assertThat(factory.opened).isEqualTo(0)
        // Nothing to open full screen yet: the server has no copy.
        circleNode().assert(SemanticsMatcher.keyNotDefined(SemanticsActions.CustomActions))
    }

    @Test
    fun `while the composer records it waits and says why`() {
        val factory = Factory()
        show(factory, recording = true)

        circleNode()
            .assert(
                SemanticsMatcher.expectValue(
                    SemanticsProperties.StateDescription,
                    "You can play this after recording.",
                ),
            )
            .performSemanticsAction(SemanticsActions.OnClick)
        compose.waitForIdle()

        assertThat(explained).isEqualTo(1)
        assertThat(factory.opened).isEqualTo(0)
    }

    @Test
    fun `the message menu offers Open full screen on a circle and nowhere else`() {
        var opened = 0
        var round by mutableStateOf(true)
        compose.setContent {
            MessageContextMenu(
                onReply = {},
                onEdit = {},
                onOpenFullScreen = if (round) ({ opened++ }) else null,
                onClosePoll = {},
                onCopy = {},
                onShare = {},
                onSave = {},
                // Never Edit on a circle (canEditMessage says so).
                canEdit = false,
            )
        }

        compose.onNodeWithText("Open full screen").performClick()
        assertThat(opened).isEqualTo(1)

        round = false
        compose.onNodeWithText("Open full screen").assertDoesNotExist()
    }

    @Test
    fun `200 dp in a phone's window`() {
        show()
        compose.onNodeWithTag("round-video-91").assertWidthIsEqualTo(200.dp).assertHeightIsEqualTo(200.dp)
    }

    @Test
    @Config(qualifiers = "en-w800dp-h1280dp")
    fun `240 dp from 600 dp`() {
        show()
        compose.onNodeWithTag("round-video-91").assertWidthIsEqualTo(240.dp).assertHeightIsEqualTo(240.dp)
    }
}

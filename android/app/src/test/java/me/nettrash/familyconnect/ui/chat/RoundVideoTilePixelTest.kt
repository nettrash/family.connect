/*
 * RoundVideoTilePixelTest.kt
 * Family Connect (Android)
 *
 * The video message's circle as the approved design draws it (#79), in
 * PIXELS (native graphics, the window drawn into a bitmap):
 *
 *  - exactly ONE accent ring, OUTSIDE the edge, showing the progress — half
 *    way through, the right half is drawn and the left half is not, and
 *    nothing of it lies on the picture inside the edge;
 *  - the play disc fades out while it plays, and is there at rest;
 *  - the length sits in a dark capsule at the bottom centre, with the WHITE
 *    unplayed dot in it.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performSemanticsAction
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.pixelsOf
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

@RunWith(RobolectricTestRunner::class)
@Config(qualifiers = "en")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class RoundVideoTilePixelTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private val circle = AttachmentDto(
        id = 91, kind = "video", mime = "video/mp4", size = 1_649_700, width = 480, height = 480,
        durationMs = 24_000, hasPreview = false, round = true,
    )

    /** Half way through the clip, whenever asked. */
    private class Player : VideoPlayer {
        override fun setSurface(surface: android.view.Surface?) = Unit
        override fun start() = Unit
        override fun pause() = Unit
        override fun seekTo(positionMs: Int) = Unit
        override val positionMs: Int = 12_000
        override fun release() = Unit
    }

    private class Factory : VideoPlayerFactory {
        override suspend fun open(attachmentId: Long, events: VideoPlayer.Events): VideoPlayer {
            events.onReady()
            return Player()
        }
    }

    private val accent = Color(0xFF1E5BD8)

    private fun show() {
        compose.setContent {
            MaterialTheme(colorScheme = lightColorScheme(primary = accent)) {
                CompositionLocalProvider(
                    LocalPlaybackCoordinator provides PlaybackCoordinator(),
                    LocalVideoPlayerFactory provides Factory(),
                ) {
                    Box(Modifier.background(Color.White)) {
                        RoundVideoBubble(
                            attachment = circle,
                            isMine = false,
                            acked = true,
                            sending = false,
                            streamUrl = { null },
                            onOpenFullScreen = {},
                            onLongPress = {},
                            onDoubleTap = {},
                        )
                    }
                }
            }
        }
    }

    /** Whether a pixel is the accent, give or take the edge's anti-aliasing. */
    private fun isAccent(argb: Int): Boolean {
        val c = Color(argb)
        return kotlin.math.abs(c.red - accent.red) < 0.12f &&
            kotlin.math.abs(c.green - accent.green) < 0.12f &&
            kotlin.math.abs(c.blue - accent.blue) < 0.12f
    }

    /**
     * The ring the bubble draws ([outsideRing]) round a grey 200-unit circle,
     * half way through. (The bubble's own playing surface is a TextureView,
     * which Robolectric cannot draw, so the ring is drawn here on its own.)
     */
    @Test
    fun `one accent ring outside the edge shows how far it has played`() {
        compose.setContent {
            Box(
                modifier = Modifier
                    .background(Color.White)
                    .size(212.dp)
                    .outsideRing(diameter = 200.dp, color = accent) { 0.5f }
                    .testTag("ring-frame"),
                contentAlignment = Alignment.Center,
            ) {
                Box(Modifier.size(200.dp).clip(CircleShape).background(Color.Gray))
            }
        }

        val frame = compose.pixelsOf("ring-frame")
        val density = compose.density.density
        val room = 6f * density
        // The ring's centre line: 1.5 units of gap, then half of its 3.
        val ringAt = (room - 3f * density).toInt()
        val midY = frame.height / 2
        // Half way: the right half is drawn (clockwise from 12), the left half is not.
        assertThat(isAccent(frame[frame.width - 1 - ringAt, midY])).isTrue()
        assertThat(isAccent(frame[ringAt, midY])).isFalse()
        // Outside the edge only: just inside the circle it is the picture, not the ring.
        assertThat(frame[frame.width - 1 - (room + 2f * density).toInt(), midY]).isEqualTo(Color.Gray.toArgb())
        // ONE ring: along the right half's radius the accent comes in one run.
        var runs = 0
        var inRun = false
        for (x in frame.width / 2 until frame.width) {
            val lit = isAccent(frame[x, midY])
            if (lit && !inRun) runs++
            inRun = lit
        }
        assertThat(runs).isEqualTo(1)
        // At 12 o'clock the ring starts, outside the top edge.
        assertThat(isAccent(frame[frame.width / 2 + (2 * density).toInt(), ringAt])).isTrue()
    }

    @Test
    fun `while it plays the play disc has faded out`() {
        show()
        compose.onNodeWithTag("round-video-play-disc", useUnmergedTree = true).assertExists()

        compose.onNodeWithContentDescription("Video message, 0:24").performSemanticsAction(SemanticsActions.OnClick)
        compose.mainClock.advanceTimeBy(500)

        compose.onNodeWithTag("round-video-play-disc", useUnmergedTree = true).assertDoesNotExist()
        compose.onNodeWithTag("round-video-progress", useUnmergedTree = true).assertExists()
    }

    @Test
    fun `at rest - no ring, the play disc, and the white dot in the dark capsule`() {
        show()
        compose.waitForIdle()

        val frame = compose.pixelsOf("round-video-frame-91")
        val density = compose.density.density
        val ringAt = (6f * density - 3f * density).toInt()
        assertThat(isAccent(frame[frame.width - 1 - ringAt, frame.height / 2])).isFalse()
        compose.onNodeWithTag("round-video-play-disc", useUnmergedTree = true).assertExists()

        val badge = compose.pixelsOf("round-video-badge")
        val dot = compose.pixelsOf("round-video-unplayed")
        assertThat(dot[dot.width / 2, dot.height / 2]).isEqualTo(Color.White.toArgb())
        // The capsule's ground is dark: its corner-free left edge, mid-height.
        val ground = Color(badge[(3 * density).toInt(), badge.height / 2])
        assertThat(ground.red + ground.green + ground.blue).isLessThan(1.2f)
    }
}

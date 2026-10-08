/*
 * RecorderLayoutTest.kt
 * Family Connect (Android)
 *
 * The video recorder's LAYOUT (#79, decision 41, 2026-10-06). On the owner's
 * iPhone the composer row showed through the recorder and its controls sat
 * on the paperclip, ✨, the field and Send, and a thin scrim let the chat
 * compete with the camera circle. Checked here, over the real recorder and a
 * fake camera (RecorderLayerTest's harness), in bounds and in pixels:
 *
 *  - the status capsule, the circle and the control bar never intersect, and
 *    no control or caption under a round button touches another — at 320,
 *    375 and 430 dp wide, a phone on its side, a tablet and twice-size text,
 *    in PREVIEW (two status lines), RECORDING and REVIEW, with and without
 *    the reply banner;
 *  - the control bar is solid — its own colour, not the scrim over what is
 *    beneath — and the scrim is dark enough that a white chat beneath reads
 *    at most 15 % grey;
 *  - with a composer row under the layer in [ComposerUnderRecorder], not one
 *    of its pixels reaches the screen.
 *
 * Robolectric native graphics; the "camera" is an empty box, and nothing
 * records.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.Surface
import androidx.activity.ComponentActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.unit.dp
import com.google.common.truth.Truth.assertThat
import com.google.common.truth.Truth.assertWithMessage
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.RoundVideoLimits
import me.nettrash.familyconnect.testutil.pixelsOf
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "en")
class RecorderLayoutTest {

    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val folder = TemporaryFolder()

    private class Camera : CameraSession {
        var listener: CameraSession.Listener? = null
        var file: File? = null
        override val cameraCount = 2
        override fun open(listener: CameraSession.Listener) {
            this.listener = listener
        }

        override fun startRecording(file: File, limitMs: Long, rotation: Int): Boolean {
            file.writeBytes(ByteArray(8))
            this.file = file
            return true
        }

        override fun stopRecording() = Unit
        override fun switchCamera() = Unit
        override fun close() = Unit
    }

    private val camera = Camera()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var now = 1_000L

    private val recorder = VideoMessageRecorder(
        camera = camera,
        clips = ClipPreparer { file, recordedMs, _ ->
            RoundClip(
                prepared = MediaPrep.Prepared(file, "video/mp4", AttachmentDto.KIND_VIDEO, 480, 480, recordedMs.toInt(), null),
                durationMs = recordedMs,
                notRound = null,
                files = listOf(file),
            )
        },
        sink = RoundVideoSink { _, _, _ -> true },
        calls = object : CallStateSource {
            override val state = MutableStateFlow<CallState>(CallState.Idle)
            override fun requestAnswer() = Unit
        },
        limits = { RoundVideoLimits(60_000, 12_582_912) },
        teaching = object : PreviewTeaching {
            override fun taught() = false
            override fun markTaught() = Unit
        },
        uptime = { now },
        scope = scope,
        newClipFile = { folder.newFile() },
    )

    private val host = object : RecorderHost {
        override fun recordVoiceInstead() = Unit
        override fun sent(replyId: Long?) = Unit
        override fun replyDropped() = Unit
        override fun announce(text: Int) = Unit
    }

    private fun session(reply: Boolean) = VideoMessageRecorder.Session(
        chatId = 42,
        reply = if (reply) ReplyToDto(messageId = 7, senderId = 9, excerpt = "Are you coming to dinner on Sunday?") else null,
        replyAuthor = if (reply) "Grandma" else "",
        replyExcerpt = if (reply) "Are you coming to dinner on Sunday?" else "",
        voiceBlocked = false,
    )

    @After
    fun tearDown() {
        scope.cancel()
    }

    /** What the chat beneath is drawn in for the pixel checks: white, and a red composer row. */
    private val chatWhite = Color.White
    private val composerRed = Color(0xFFFF0000)

    /**
     * The recorder over a white "chat" whose bottom row is a red stand-in for
     * the composer, wrapped as the app wraps the real one.
     */
    private fun show(reply: Boolean = false, compactWindow: Boolean = true) {
        compose.runOnIdle { recorder.open(session(reply), host, VideoMessageRecorder.Permissions(true, true)) }
        compose.setContent {
            val state by recorder.state.collectAsState()
            MaterialTheme(colorScheme = lightColorScheme()) {
                Box(Modifier.fillMaxSize().background(chatWhite).testTag("window")) {
                    Column(Modifier.fillMaxSize()) {
                        Spacer(Modifier.weight(1f))
                        ComposerUnderRecorder(recorderOpen = state.isOpen) {
                            Row(Modifier.fillMaxWidth().height(56.dp).background(composerRed).testTag("composer")) {
                                Box(Modifier.size(44.dp).testTag("composer-paperclip"))
                            }
                        }
                    }
                    if (state.isOpen) {
                        RecorderLayer(
                            recorder = recorder,
                            state = state,
                            pane = null,
                            compactWindow = compactWindow,
                            onRecord = { recorder.record(Surface.ROTATION_0, holdOrientation = true) },
                            onOpenSettings = {},
                            preview = { modifier -> Box(modifier) },
                        )
                    }
                }
            }
        }
        compose.runOnIdle { camera.listener!!.firstFrame() }
        compose.waitForIdle()
    }

    private fun bounds(tag: String): Rect =
        compose.onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode().boundsInRoot

    private fun described(label: String): Rect =
        compose.onNodeWithContentDescription(label, useUnmergedTree = true).fetchSemanticsNode().boundsInRoot

    /** A caption under a round button: the text node, which TalkBack does not hear. */
    private fun caption(text: String): Rect =
        compose.onAllNodesWithText(text, useUnmergedTree = true).fetchSemanticsNodes()
            .single { node -> node.boundsInRoot.top > bounds("round-video-controls-bar").top - 1f }
            .boundsInRoot

    /**
     * The decision-41 checks for whatever the recorder shows now: [controls]
     * are the round buttons' labels, [captions] the words under them.
     */
    private fun assertNothingOverlaps(state: String, controls: List<String>, captions: List<String>) {
        val window = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val status = bounds("round-video-status")
        val circle = bounds("round-video-circle")
        val bar = bounds("round-video-controls-bar")
        val pieces = mapOf("status" to status, "circle" to circle, "bar" to bar)
        for ((a, ra) in pieces) for ((b, rb) in pieces) {
            if (a < b) assertWithMessage("$state: $a $ra overlaps $b $rb").that(ra.overlaps(rb)).isFalse()
        }
        assertWithMessage("$state: the circle has room (it is drawn)").that(circle.width).isGreaterThan(40f)
        // Fitted, never squeezed: a circle the room clamped in one direction
        // only would be drawn as an oval with its ring spilling out of it.
        assertWithMessage("$state: the circle $circle is round").that(circle.height).isWithin(1f).of(circle.width)
        val face = bounds("round-video-circle-face")
        assertWithMessage("$state: the face $face is round").that(face.height).isWithin(1f).of(face.width)
        assertWithMessage("$state: the face $face sits inside its ring $circle")
            .that(face.left >= circle.left && face.top >= circle.top && face.right <= circle.right && face.bottom <= circle.bottom)
            .isTrue()
        val inBar = controls.associate { "control $it" to described(it) } +
            ("slot" to bounds("round-video-slot")) +
            captions.associate { "caption $it" to caption(it) }
        for ((name, r) in inBar + pieces) {
            assertWithMessage("$state: $name $r is inside the window $window")
                .that(r.left >= window.left - 0.5f && r.top >= window.top - 0.5f &&
                    r.right <= window.right + 0.5f && r.bottom <= window.bottom + 0.5f)
                .isTrue()
        }
        for ((name, r) in inBar) {
            assertWithMessage("$state: $name $r sits on the bar $bar")
                .that(r.left >= bar.left - 0.5f && r.top >= bar.top - 0.5f &&
                    r.right <= bar.right + 0.5f && r.bottom <= bar.bottom + 0.5f)
                .isTrue()
        }
        val names = inBar.keys.toList()
        for (i in names.indices) for (j in i + 1 until names.size) {
            val ra = inBar.getValue(names[i])
            val rb = inBar.getValue(names[j])
            assertWithMessage("$state: ${names[i]} $ra overlaps ${names[j]} $rb").that(ra.overlaps(rb)).isFalse()
        }
    }

    /** Through PREVIEW, RECORDING and REVIEW, checking each. */
    private fun everyState(reply: Boolean = false, compactWindow: Boolean = true) {
        show(reply = reply, compactWindow = compactWindow)
        // PREVIEW says two lines: "Not recording" and the first-time line.
        assertNothingOverlaps(
            "preview",
            controls = listOf("Close", "Switch camera", "Record a voice message instead"),
            captions = listOf("Close", "Switch", "Record voice message", "Record"),
        )
        compose.onNodeWithTag("round-video-slot").performClick()
        compose.waitForIdle()
        assertNothingOverlaps("recording", controls = listOf("Delete recording"), captions = listOf("Delete", "Stop"))
        now += 23_400
        compose.onNodeWithTag("round-video-slot").performClick()
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 23_400, capReached = false, failed = false) }
        compose.waitForIdle()
        assertNothingOverlaps("review", controls = listOf("Delete", "Retake"), captions = listOf("Delete", "Retake", "Send"))
    }

    @Test
    @Config(qualifiers = "en-w320dp-h568dp")
    fun aCompactPhoneHasNoOverlaps() = everyState()

    @Test
    @Config(qualifiers = "en-w320dp-h568dp")
    fun aCompactPhoneAnsweringAMessageHasNoOverlaps() = everyState(reply = true)

    @Test
    @Config(qualifiers = "en-w375dp-h667dp")
    fun aMidSizePhoneHasNoOverlaps() = everyState(reply = true)

    @Test
    @Config(qualifiers = "en-w430dp-h932dp")
    fun aLargePhoneHasNoOverlaps() = everyState(reply = true)

    @Test
    @Config(qualifiers = "en-w568dp-h320dp-land")
    fun aSmallPhoneOnItsSideHasNoOverlaps() = everyState(reply = true)

    @Test
    @Config(qualifiers = "en-w932dp-h430dp-land")
    fun aLargePhoneOnItsSideHasNoOverlaps() = everyState()

    @Test
    @Config(qualifiers = "en-w820dp-h1180dp")
    fun aTabletHasNoOverlaps() = everyState(reply = true, compactWindow = false)

    @Test
    @Config(qualifiers = "en-w320dp-h568dp", fontScale = 2.0f)
    fun twiceSizeTextOnACompactPhoneHasNoOverlaps() = everyState(reply = true)

    @Test
    @Config(qualifiers = "en-w568dp-h320dp-land", fontScale = 2.0f)
    fun twiceSizeTextOnAPhoneOnItsSideHasNoOverlaps() = everyState()

    // -- Pixels ----------------------------------------------------------------------

    private fun channels(argb: Int): List<Int> = listOf((argb shr 16) and 0xFF, (argb shr 8) and 0xFF, argb and 0xFF)

    /** The composer row under the recorder draws nothing: no red reaches the screen anywhere. */
    @Test
    @Config(qualifiers = "en-w375dp-h667dp")
    fun noPixelOfTheComposerShowsThroughTheRecorder() {
        show()
        val pixels = compose.pixelsOf("window")
        var reddish = 0
        for (y in 0 until pixels.height) for (x in 0 until pixels.width) {
            val (r, g, b) = channels(pixels[x, y])
            if (r > 40 && r > g * 2 + 20 && r > b * 2 + 20) reddish++
        }
        // The recorder's own red Record disc is the only red there may be.
        val slot = bounds("round-video-slot")
        val slotArea = (slot.width * slot.height).toInt()
        assertWithMessage("red pixels outside the Record disc").that(reddish).isAtMost(slotArea)
        // And the composer's own place is the bar's colour, not dimmed red.
        val composer = bounds("composer-under-recorder")
        val window = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val x = (composer.left + 4).toInt()
        val y = (composer.center.y - window.top).toInt()
        assertThat(pixels[x, y]).isEqualTo(RecorderBarColor.toArgb())
    }

    /** The bar is solid — its own colour at its edge — and the scrim leaves a white chat at most 15 % grey. */
    @Test
    @Config(qualifiers = "en-w375dp-h667dp")
    fun theBarIsSolidAndTheScrimIsDark() {
        show()
        val bar = compose.pixelsOf("round-video-controls-bar")
        assertThat(bar[2, 2]).isEqualTo(RecorderBarColor.toArgb())
        assertThat(bar[bar.width - 3, bar.height - 3]).isEqualTo(RecorderBarColor.toArgb())

        val window = compose.pixelsOf("window")
        // The top-left corner: only the scrim over the white chat.
        val (r, g, b) = channels(window[2, 2])
        assertWithMessage("scrim over white, r").that(r).isAtMost(41)
        assertWithMessage("scrim over white, g").that(g).isAtMost(41)
        assertWithMessage("scrim over white, b").that(b).isAtMost(41)
        assertThat(SCRIM_ALPHA).isAtLeast(0.85f)
    }
}

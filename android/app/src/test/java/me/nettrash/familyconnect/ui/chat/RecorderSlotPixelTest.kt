/*
 * RecorderSlotPixelTest.kt
 * Family Connect (Android)
 *
 * The video recorder's controls as the approved design draws them (#79), in
 * PIXELS over the real recorder and a fake camera (RecorderLayerTest's
 * harness): ONE big slot, 64 units, that is a red disc (Record), then a red
 * disc with a white rounded square (Stop), then an accent disc (Send); every
 * control captioned under it — Close, Switch, Record; Delete, Stop; Delete,
 * Retake, Send — while TalkBack keeps hearing each button's own label.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.Surface
import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsFocused
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.activity.ComponentActivity
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.test.assertWidthIsEqualTo
import androidx.compose.ui.test.assertHeightIsEqualTo
import androidx.compose.ui.unit.dp
import me.nettrash.familyconnect.testutil.pixelsOf
import org.robolectric.annotation.GraphicsMode
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.input.key.Key
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
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.RoundVideoLimits
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@org.robolectric.annotation.Config(qualifiers = "en")
class RecorderSlotPixelTest {

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
    private var notRound: RoundRecorderRules.NotRound? = null
    private val sent = mutableListOf<RoundClip>()

    private val recorder = VideoMessageRecorder(
        camera = camera,
        clips = ClipPreparer { file, recordedMs, _ ->
            RoundClip(
                prepared = MediaPrep.Prepared(file, "video/mp4", AttachmentDto.KIND_VIDEO, 480, 480, recordedMs.toInt(), null),
                durationMs = recordedMs,
                notRound = notRound,
                files = listOf(file),
            )
        },
        sink = RoundVideoSink { _, clip, _ ->
            sent += clip
            true
        },
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

    private val session = VideoMessageRecorder.Session(
        chatId = 42,
        reply = null,
        replyAuthor = "",
        replyExcerpt = "",
        voiceBlocked = false,
    )

    @After
    fun tearDown() {
        scope.cancel()
    }

    private fun show(permissions: VideoMessageRecorder.Permissions = VideoMessageRecorder.Permissions(true, true)) {
        compose.runOnIdle { recorder.open(session, host, permissions) }
        compose.setContent {
            val state by recorder.state.collectAsState()
            if (state.isOpen) MaterialTheme(colorScheme = lightColorScheme(primary = ACCENT)) {
                RecorderLayer(
                    recorder = recorder,
                    state = state,
                    pane = null,
                    compactWindow = true,
                    onRecord = { recorder.record(Surface.ROTATION_0, holdOrientation = true) },
                    onOpenSettings = {},
                    preview = { modifier -> Box(modifier) },
                )
            }
        }
    }

    private val slot get() = compose.onNodeWithTag("round-video-slot")

    /**
     * Whether a pixel is [want], allowing for the focus highlight the slot
     * wears (focus starts on it, S3.4), which shades it a little.
     */
    private fun isNear(argb: Int, want: Color): Boolean {
        val c = Color(argb)
        return kotlin.math.abs(c.red - want.red) < 0.15f &&
            kotlin.math.abs(c.green - want.green) < 0.15f &&
            kotlin.math.abs(c.blue - want.blue) < 0.15f
    }

    /** The slot's pixel [fx], [fy] of the way across and down. */
    private fun slotAt(fx: Float, fy: Float): Int {
        val pixels = compose.pixelsOf("round-video-slot")
        return pixels[(pixels.width * fx).toInt(), (pixels.height * fy).toInt()]
    }

    @Test
    fun theSlotIsOneBigButtonRecordThenStopThenSend() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.assertWidthIsEqualTo(64.dp).assertHeightIsEqualTo(64.dp)
        // Record: a red disc, red right to its middle.
        assertWithMessage(Integer.toHexString(slotAt(0.5f, 0.5f))).that(isNear(slotAt(0.5f, 0.5f), RED)).isTrue()
        assertThat(isNear(slotAt(0.5f, 0.5f), Color.White)).isFalse()
        compose.onNodeWithText("Close").assertExists()
        compose.onNodeWithText("Switch").assertExists()
        compose.onNodeWithText("Record").assertExists()

        slot.performClick()
        // Stop: a white rounded square in the middle of the red disc.
        assertWithMessage(Integer.toHexString(slotAt(0.5f, 0.5f))).that(isNear(slotAt(0.5f, 0.5f), Color.White)).isTrue()
        assertThat(isNear(slotAt(0.2f, 0.5f), RED)).isTrue()
        compose.onNodeWithText("Stop").assertExists()
        compose.onNodeWithText("Delete").assertExists()

        now += 23_400
        slot.performClick()
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 23_400, capReached = false, failed = false) }
        compose.waitForIdle()
        // Send: the accent disc.
        assertWithMessage(Integer.toHexString(slotAt(0.2f, 0.5f))).that(isNear(slotAt(0.2f, 0.5f), ACCENT)).isTrue()
        compose.onNodeWithText("Send").assertExists()
        compose.onNodeWithText("Retake").assertExists()
        // The captions are for the eye: TalkBack hears the buttons' own labels.
        compose.onNodeWithText("Send").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.HideFromAccessibility))
        slot.assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Send video message")))
    }

    private companion object {
        val ACCENT = Color(0xFF1E5BD8)
        val RED = Color(0xFFE53935)
    }
}

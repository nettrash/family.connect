/*
 * RecorderLayerTest.kt
 * Family Connect (Android)
 *
 * The video recorder on screen (#79, docs/audio-video-messages-2026-10-04.md,
 * S3.2, S3.4, S3.6, S6): a modal region TalkBack hears as "Video message";
 * what each state's status line says; the slot as Record — dimmed until the
 * first frame — then Stop, then Send; the leading and middle controls; the
 * refusal's way to Settings; Esc in REVIEW asking first. Robolectric Compose
 * over the real recorder driven by a fake camera — the "camera" here is an
 * empty box, and nothing records.
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
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performKeyInput
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.pressKey
import androidx.compose.ui.input.key.Key
import com.google.common.truth.Truth.assertThat
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
class RecorderLayerTest {

    @get:Rule
    val compose = createComposeRule()

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
            if (state.isOpen) {
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

    /** A modal region named "Video message" (S6). */
    @Test
    fun theRecorderIsAPaneCalledVideoMessage() {
        show()
        compose.onNodeWithTag("round-video-recorder")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.PaneTitle, "Video message"))
    }

    /**
     * On a wide window the controls stay a phone's reach apart, centred under
     * the circle (the approved design caps the row), rather than running out
     * to the window's two edges.
     */
    @Test
    @org.robolectric.annotation.Config(qualifiers = "w800dp-h1280dp")
    fun onAWideWindowTheControlsStayTogetherAndCentred() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        val close = compose.onNodeWithContentDescription("Close").fetchSemanticsNode().boundsInRoot
        val record = slot.fetchSemanticsNode().boundsInRoot
        val window = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val density = compose.density.density
        assertThat((record.right - close.left) / density).isAtMost(420f)
        // Centred: as far in from the left as from the right, within a few units.
        assertThat(close.left - window.left).isWithin(24f * density).of(window.right - record.right)
    }

    @Test
    fun beforeTheFirstFrameRecordIsDimmedAndTheCameraIsStarting() {
        show()
        compose.onNodeWithText("Starting camera…").assertIsDisplayed()
        slot.assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Record")))
            .assertIsNotEnabled()
        compose.onNodeWithContentDescription("Close").assertIsDisplayed()
        compose.onNodeWithContentDescription("Switch camera").assertIsDisplayed()
        compose.onNodeWithContentDescription("Record a voice message instead").assertIsDisplayed()
    }

    /** "Not recording" is true: the microphone opens at Record (S3.4). */
    @Test
    fun previewSaysNotRecordingAndTheFirstTimeLine() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        compose.onNodeWithText("Not recording").assertIsDisplayed()
        compose.onNodeWithText("Only you can see this until you start recording.").assertIsDisplayed()
        slot.assertIsEnabled()
    }

    @Test
    fun recordTurnsTheSlotIntoStopAndTheLeadingControlIntoDelete() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.performClick()

        slot.assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Stop recording")))
        compose.onNodeWithContentDescription("Delete recording").assertIsDisplayed()
        // Android switches cameras in PREVIEW only (S3.5).
        compose.onNodeWithContentDescription("Switch camera").assertDoesNotExist()
        compose.onNodeWithText("Not recording").assertDoesNotExist()
    }

    @Test
    fun reviewShowsTheLengthAndSendsTheVideoMessage() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.performClick()
        now += 23_400
        slot.performClick()
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 23_400, capReached = false, failed = false) }

        compose.onNodeWithText("Video message · 0:23").assertIsDisplayed()
        compose.onNodeWithContentDescription("Delete").assertIsDisplayed()
        compose.onNodeWithContentDescription("Retake").assertIsDisplayed()
        slot.assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Send video message")))

        now += ComposerSlot.ACTIVATION_GUARD_MS
        slot.performClick()
        compose.runOnIdle { assertThat(sent.single().round).isTrue() }
    }

    /** S3.6: said in REVIEW, before Send. */
    @Test
    fun aClipThatCouldNotBeMadeRoundSaysSoBeforeSend() {
        notRound = RoundRecorderRules.NotRound.COULDNT_MAKE_ROUND
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.performClick()
        now += 5_000
        slot.performClick()
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 5_000, capReached = false, failed = false) }
        compose.onNodeWithText("Couldn't make it round. It will be sent as a regular video.").assertIsDisplayed()
    }

    /** Esc in REVIEW always asks (S3.4). */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun escapeInReviewAsksFirst() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.performClick()
        now += 3_000
        slot.performClick()
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 3_000, capReached = false, failed = false) }

        slot.performKeyInput { pressKey(Key.Escape) }
        compose.onNodeWithText("Delete video message?").assertIsDisplayed()
        compose.onNodeWithText("Keep").performClick()
        compose.onNodeWithText("Delete video message?").assertDoesNotExist()
        compose.onNodeWithText("Video message · 0:03").assertIsDisplayed()
    }

    /**
     * "Focus starts on the slot" (S3.4) at an ordinary open — the camera's
     * first frame arriving AFTER the layer is up, which does not change the
     * phase's kind. The slot is dimmed then, and dimmed is not disabled: it
     * keeps the focus, so Esc and Return reach the recorder from the start.
     */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun focusStartsOnTheSlotAtAnOrdinaryOpen() {
        show()
        slot.assertIsFocused()
        compose.runOnIdle { camera.listener!!.firstFrame() }
        slot.assertIsFocused().assertIsEnabled()
    }

    /** Before the first frame Esc still closes: the focus is already inside the recorder. */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun escapeBeforeTheFirstFrameCloses() {
        show()
        slot.performKeyInput { pressKey(Key.Escape) }
        compose.runOnIdle { assertThat(recorder.state.value.isOpen).isFalse() }
    }

    /** A dimmed Record does nothing when clicked — it only keeps the focus. */
    @Test
    fun aDimmedRecordDoesNothing() {
        show()
        slot.performClick()
        compose.runOnIdle {
            assertThat(recorder.state.value.phase).isEqualTo(VideoMessageRecorder.Phase.Preview())
            assertThat(camera.file).isNull()
        }
    }

    /**
     * Return/Enter is the SLOT wherever focus is (S3.4, S6): Record, then
     * Stop, then Send — never the focused Close, Delete or Retake.
     */
    @OptIn(ExperimentalTestApi::class)
    @Test
    fun returnIsTheSlotWhereverFocusIs() {
        show()
        compose.runOnIdle { camera.listener!!.firstFrame() }

        val close = compose.onNodeWithContentDescription("Close")
        close.performSemanticsAction(SemanticsActions.RequestFocus)
        close.assertIsFocused()
        close.performKeyInput { pressKey(Key.Enter) }
        compose.runOnIdle {
            assertThat(recorder.state.value.isOpen).isTrue()
            assertThat(recorder.state.value.phase).isInstanceOf(VideoMessageRecorder.Phase.Recording::class.java)
        }

        now += 5_000
        val deleteRecording = compose.onNodeWithContentDescription("Delete recording")
        deleteRecording.performSemanticsAction(SemanticsActions.RequestFocus)
        deleteRecording.performKeyInput { pressKey(Key.NumPadEnter) }
        compose.runOnIdle { camera.listener!!.finalized(camera.file, 5_000, capReached = false, failed = false) }
        compose.onNodeWithText("Video message · 0:05").assertIsDisplayed()

        now += ComposerSlot.ACTIVATION_GUARD_MS
        val retake = compose.onNodeWithContentDescription("Retake")
        retake.performSemanticsAction(SemanticsActions.RequestFocus)
        retake.performKeyInput { pressKey(Key.Enter) }
        compose.runOnIdle { assertThat(sent.single().round).isTrue() }
    }

    @Test
    fun aRefusedCameraSaysWhereToTurnItOnAndOffersVoice() {
        show(
            VideoMessageRecorder.Permissions(
                camera = false,
                microphone = true,
                cameraRefusedForGood = true,
            ),
        )
        compose.onNodeWithText("Family needs permission to use your camera. Turn it on in Settings.")
            .assertIsDisplayed()
        compose.onNodeWithText("Open Settings").assertIsDisplayed()
        compose.onNodeWithText("Record a voice message instead").assertIsDisplayed()
    }

    @Test
    fun theFirstQuestionSaysWhatItNeeds() {
        show(VideoMessageRecorder.Permissions(camera = false, microphone = false))
        compose.onNodeWithText("Video messages need the camera and the microphone.").assertIsDisplayed()
    }
}

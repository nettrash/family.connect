/*
 * VideoMessageRecorderTest.kt
 * Family Connect (Android)
 *
 * The video recorder's state machine (#79, docs/audio-video-messages-2026-10-04.md,
 * S3.2, S3.4, S3.6, S4's video columns), driven on the JVM against a FAKE
 * camera — no camera is ever opened here — a fake preparer, a fake outbox and
 * the test scheduler's clock. The recorder's coroutines run on backgroundScope
 * (its call watcher collects forever), so time is moved with advanceTimeBy and
 * runCurrent, never advanceUntilIdle.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.Surface
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.RoundVideoLimits
import me.nettrash.familyconnect.ui.chat.VideoMessageRecorder.Ask
import me.nettrash.familyconnect.ui.chat.VideoMessageRecorder.Notice
import me.nettrash.familyconnect.ui.chat.VideoMessageRecorder.Permissions
import me.nettrash.familyconnect.ui.chat.VideoMessageRecorder.Phase
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
class VideoMessageRecorderTest {

    @get:Rule
    val folder = TemporaryFolder()

    private class FakeCamera : CameraSession {
        var listener: CameraSession.Listener? = null
        var on = false
        var opens = 0
        var file: File? = null
        var limitMs = 0L
        var rotation = -1
        var stops = 0
        var switches = 0
        var starts = true
        override var cameraCount = 2

        override fun open(listener: CameraSession.Listener) {
            this.listener = listener
            on = true
            opens++
        }

        override fun startRecording(file: File, limitMs: Long, rotation: Int): Boolean {
            if (!starts) return false
            file.writeBytes(ByteArray(64))
            this.file = file
            this.limitMs = limitMs
            this.rotation = rotation
            return true
        }

        override fun stopRecording() {
            stops++
        }

        override fun switchCamera() {
            switches++
        }

        override fun close() {
            on = false
        }

        /** CameraX's Finalize, as the camera reports it. */
        fun finalized(durationMs: Long, capReached: Boolean = false, failed: Boolean = false, written: Boolean = true) {
            listener!!.finalized(if (written) file else null, durationMs, capReached, failed)
        }
    }

    private class FakeHost : RecorderHost {
        var voiceInstead = 0
        val sent = mutableListOf<Long?>()
        var dropped = 0
        val announced = mutableListOf<Int>()
        override fun recordVoiceInstead() {
            voiceInstead++
        }

        override fun sent(replyId: Long?) {
            sent += replyId
        }

        override fun replyDropped() {
            dropped++
        }

        override fun announce(text: Int) {
            announced += text
        }
    }

    private class Sent(val chatId: Long, val clip: RoundClip, val reply: ReplyToDto?)

    private val camera = FakeCamera()
    private val host = FakeHost()
    private val call = MutableStateFlow<CallState>(CallState.Idle)
    private val sends = mutableListOf<Sent>()
    private var sendSucceeds = true
    private var notRound: RoundRecorderRules.NotRound? = null
    private var preparable = true
    private val prepared = mutableListOf<File>()
    private var taught = false
    private var limits: RoundVideoLimits? = RoundVideoLimits(60_000, 12_582_912)
    private val reply = ReplyToDto(messageId = 41, senderId = 2, excerpt = "Dinner?")
    private val session = VideoMessageRecorder.Session(
        chatId = 42,
        reply = reply,
        replyAuthor = "Anna",
        replyExcerpt = "Dinner?",
        voiceBlocked = false,
    )

    private fun TestScope.recorder(playback: PlaybackCoordinator? = null): VideoMessageRecorder =
        VideoMessageRecorder(
            camera = camera,
            clips = ClipPreparer { file, recordedMs, _ ->
                if (!preparable) return@ClipPreparer null
                val out = folder.newFile()
                file.delete()
                prepared += out
                RoundClip(
                    prepared = MediaPrep.Prepared(
                        file = out,
                        mime = "video/mp4",
                        kind = AttachmentDto.KIND_VIDEO,
                        width = 480,
                        height = 480,
                        durationMs = recordedMs.toInt(),
                        previewJpeg = null,
                    ),
                    durationMs = recordedMs,
                    notRound = notRound,
                    files = listOf(out),
                )
            },
            sink = RoundVideoSink { chatId, clip, reply ->
                sends += Sent(chatId, clip, reply)
                sendSucceeds
            },
            calls = object : CallStateSource {
                override val state = call
                override fun requestAnswer() = Unit
            },
            limits = { limits },
            teaching = object : PreviewTeaching {
                override fun taught() = taught
                override fun markTaught() {
                    taught = true
                }
            },
            uptime = { testScheduler.currentTime },
            scope = backgroundScope,
            newClipFile = { folder.newFile() },
            playback = playback,
        )

    private val granted = Permissions(camera = true, microphone = true)

    /** Open with both granted, and the camera's first frame in. */
    private fun TestScope.previewing(recorder: VideoMessageRecorder = recorder()): VideoMessageRecorder {
        recorder.open(session, host, granted)
        runCurrent()
        camera.listener!!.firstFrame()
        return recorder
    }

    /** Record, and let it run [ms]. */
    private fun TestScope.recordingFor(ms: Long, recorder: VideoMessageRecorder = previewing()): VideoMessageRecorder {
        recorder.record(rotation = Surface.ROTATION_0, holdOrientation = true)
        advanceTimeBy(ms)
        runCurrent()
        return recorder
    }

    /** Stop after [ms] and let the camera finish the file. */
    private fun TestScope.reviewing(ms: Long = 23_400): VideoMessageRecorder {
        val recorder = recordingFor(ms)
        recorder.stop()
        camera.finalized(ms)
        runCurrent()
        return recorder
    }

    // --- Opening (S3.2) ----------------------------------------------------------

    @Test
    fun `both granted opens to PREVIEW with the camera on and nothing recording`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, granted)

        val state = recorder.state.value
        assertThat(state.phase).isEqualTo(Phase.Preview())
        assertThat(state.firstTime).isTrue()
        assertThat(camera.on).isTrue()
        assertThat(camera.file).isNull()
    }

    /** Record is dimmed until the first frame (S3.4). */
    @Test
    fun `Record does nothing before the first frame`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, granted)
        recorder.record(Surface.ROTATION_0, holdOrientation = false)

        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
        assertThat(camera.file).isNull()

        camera.listener!!.firstFrame()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.announcement?.text).isEqualTo(R.string.s_announce_camera_ready)
    }

    @Test
    fun `missing permissions ask once, the camera off, and the answer decides`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, Permissions(camera = false, microphone = false))
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Asking)
        assertThat(camera.on).isFalse()

        recorder.permissionsAnswered(camera = true, microphone = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
        assertThat(camera.on).isTrue()
    }

    /** The camera is off in every refusal state (S3.2). */
    @Test
    fun `a refused answer is that refusal, with the camera off`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, Permissions(camera = false, microphone = true))
        recorder.permissionsAnswered(camera = false, microphone = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.CameraRefused)
        assertThat(camera.opens).isEqualTo(0)
        recorder.close()

        recorder.open(session, host, Permissions(camera = true, microphone = false))
        recorder.permissionsAnswered(camera = true, microphone = false)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.MicrophoneRefused)
        assertThat(camera.opens).isEqualTo(0)
    }

    @Test
    fun `a refusal known before opens straight to it`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, Permissions(camera = false, microphone = false, cameraRefusedForGood = true))
        assertThat(recorder.state.value.phase).isEqualTo(Phase.CameraRefused)
        // No question to answer: a late answer changes nothing.
        recorder.permissionsAnswered(camera = true, microphone = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.CameraRefused)
    }

    /** Only ever one recorder: a second way in while it is open does nothing. */
    @Test
    fun `opening while open is ignored`() = runTest {
        val recorder = previewing()
        recorder.open(session.copy(chatId = 7), host, granted)
        assertThat(recorder.state.value.session?.chatId).isEqualTo(42)
        assertThat(camera.opens).isEqualTo(1)
    }

    // --- PREVIEW → RECORDING (S3.4) ------------------------------------------------

    @Test
    fun `Record starts the take at the limit and the angle, and holds the phone`() = runTest {
        var paused = 0
        val playback = PlaybackCoordinator().apply { started { paused++ } }
        val recorder = previewing(recorder(playback))
        recorder.currentLayout = RoundRecorderRules.Layout(320, column = false)
        recorder.record(rotation = Surface.ROTATION_90, holdOrientation = true)

        val state = recorder.state.value
        assertThat(state.phase).isEqualTo(Phase.Recording(startedAtMs = testScheduler.currentTime))
        // max_round_video_ms − 500 ms (S1.1).
        assertThat(camera.limitMs).isEqualTo(59_500)
        assertThat(camera.rotation).isEqualTo(Surface.ROTATION_90)
        assertThat(state.holdOrientation).isTrue()
        assertThat(state.lockedLayout).isEqualTo(RoundRecorderRules.Layout(320, column = false))
        assertThat(state.announcement?.text).isEqualTo(R.string.s_announce_recording_video)
        // Starting a recording pauses whatever plays (S1.7).
        assertThat(paused).isEqualTo(1)
        // The first-time line is spent once this device has recorded.
        assertThat(state.firstTime).isFalse()
        assertThat(taught).isTrue()
    }

    @Test
    fun `a tablet does not hold its orientation`() = runTest {
        val recorder = previewing()
        recorder.record(rotation = Surface.ROTATION_0, holdOrientation = false)
        assertThat(recorder.state.value.holdOrientation).isFalse()
    }

    /** A server without the keys offers no video at all; nothing records against it. */
    @Test
    fun `no limits, no recording`() = runTest {
        limits = null
        val recorder = previewing()
        recorder.record(Surface.ROTATION_0, holdOrientation = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(camera.file).isNull()
    }

    @Test
    fun `a recorder that will not start says so and stays in PREVIEW`() = runTest {
        camera.starts = false
        val recorder = previewing()
        recorder.record(Surface.ROTATION_0, holdOrientation = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.notice).isEqualTo(Notice.STOPPED_UNEXPECTEDLY)
    }

    // --- RECORDING → REVIEW (S3.4) -------------------------------------------------

    /** Record → Stop is guarded: a double tap cannot make a 0.1 s clip (S1.1). */
    @Test
    fun `Stop inside the guard is ignored`() = runTest {
        val recorder = recordingFor(599)
        recorder.stop()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Recording::class.java)
        assertThat(camera.stops).isEqualTo(0)

        advanceTimeBy(1)
        recorder.stop()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Finishing(600))
        assertThat(camera.stops).isEqualTo(1)
    }

    @Test
    fun `Stop at a second or more goes to REVIEW with the camera off`() = runTest {
        val recorder = recordingFor(23_400)
        recorder.stop()
        assertThat(recorder.state.value.holdOrientation).isFalse()
        assertThat(recorder.state.value.lockedLayout).isNull()
        camera.finalized(23_400)
        runCurrent()

        val phase = recorder.state.value.phase as Phase.Review
        assertThat(phase.clip.durationMs).isEqualTo(23_400)
        assertThat(phase.clip.round).isTrue()
        // The light goes out (S3.4).
        assertThat(camera.on).isFalse()
    }

    @Test
    fun `under a second goes back to PREVIEW, too short, the file gone`() = runTest {
        val recorder = recordingFor(900)
        recorder.stop()
        val file = camera.file!!
        camera.finalized(900)
        runCurrent()

        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.notice).isEqualTo(Notice.TOO_SHORT)
        assertThat(recorder.state.value.announcement?.text).isEqualTo(R.string.s_video_too_short)
        assertThat(file.exists()).isFalse()
        assertThat(camera.on).isTrue()
    }

    /** "10 seconds left", shown and said, from 50 s (S3.4). */
    @Test
    fun `the warning at fifty seconds`() = runTest {
        val recorder = recordingFor(49_999)
        assertThat((recorder.state.value.phase as Phase.Recording).warned).isFalse()
        advanceTimeBy(2)
        runCurrent()
        assertThat((recorder.state.value.phase as Phase.Recording).warned).isTrue()
        assertThat(recorder.state.value.announcement?.text).isEqualTo(R.string.s_ten_seconds_left)
    }

    /** The length limit stops into REVIEW, never sends (S1.7). */
    @Test
    fun `the limit stops into REVIEW with the sentence`() = runTest {
        val recorder = recordingFor(59_500)
        camera.finalized(59_500, capReached = true)
        runCurrent()

        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(recorder.state.value.notice).isEqualTo(Notice.STOPPED_AT_ONE_MINUTE)
        assertThat(sends).isEmpty()
    }

    /** A recorder that does not stop itself is stopped just after the limit. */
    @Test
    fun `the backstop stops a take CameraX did not`() = runTest {
        val recorder = recordingFor(59_500 + VideoMessageRecorder.CAP_BACKSTOP_MS - 1)
        assertThat(camera.stops).isEqualTo(0)
        advanceTimeBy(2)
        runCurrent()
        assertThat(camera.stops).isEqualTo(1)
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Finishing::class.java)
    }

    @Test
    fun `a failure keeps what is readable for REVIEW`() = runTest {
        val recorder = recordingFor(5_000)
        camera.finalized(5_000, failed = true)
        runCurrent()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(recorder.state.value.notice).isEqualTo(Notice.STOPPED_UNEXPECTEDLY)
    }

    @Test
    fun `nothing readable is the sentence and the camera back`() = runTest {
        val recorder = recordingFor(5_000)
        recorder.stop()
        camera.finalized(5_000, written = false)
        runCurrent()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.notice).isEqualTo(Notice.STOPPED_UNEXPECTEDLY)
    }

    @Test
    fun `a file the preparer cannot read goes back to PREVIEW`() = runTest {
        preparable = false
        val recorder = reviewing()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
        assertThat(recorder.state.value.notice).isEqualTo(Notice.STOPPED_UNEXPECTEDLY)
        assertThat(camera.opens).isEqualTo(2)
    }

    // --- Delete (S3.4) ----------------------------------------------------------------

    @Test
    fun `Delete under ten seconds goes back to PREVIEW at once`() = runTest {
        val recorder = recordingFor(9_999)
        recorder.delete()
        val file = camera.file!!
        camera.finalized(9_999)
        runCurrent()

        assertThat(recorder.state.value.ask).isNull()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.notice).isNull()
        assertThat(file.exists()).isFalse()
    }

    @Test
    fun `Delete from ten seconds stops first and asks - Keep is REVIEW`() = runTest {
        val recorder = recordingFor(10_000)
        recorder.delete()
        assertThat(camera.stops).isEqualTo(1)
        assertThat(recorder.state.value.ask).isEqualTo(Ask.DELETE_THEN_PREVIEW)
        camera.finalized(10_000)
        runCurrent()

        recorder.answer(delete = false)
        assertThat(recorder.state.value.ask).isNull()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
    }

    @Test
    fun `Delete from ten seconds - Delete goes back to PREVIEW, nothing kept`() = runTest {
        val recorder = recordingFor(12_000)
        recorder.delete()
        camera.finalized(12_000)
        runCurrent()
        recorder.answer(delete = true)

        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
        assertThat(prepared.single().exists()).isFalse()
        assertThat(camera.on).isTrue()
    }

    /** The question can be answered before the file is ready; it is done to it when it is. */
    @Test
    fun `Delete answered while finishing discards the file when it lands`() = runTest {
        val recorder = recordingFor(12_000)
        recorder.delete()
        recorder.answer(delete = true)
        val file = camera.file!!
        camera.finalized(12_000)
        runCurrent()

        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(file.exists()).isFalse()
        assertThat(prepared).isEmpty()
    }

    @Test
    fun `REVIEW's Delete under ten seconds closes and keeps nothing`() = runTest {
        val recorder = reviewing(ms = 8_000)
        recorder.delete()
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(prepared.single().exists()).isFalse()
        assertThat(sends).isEmpty()
    }

    @Test
    fun `REVIEW's Delete from ten seconds asks, then closes`() = runTest {
        val recorder = reviewing(ms = 10_000)
        recorder.delete()
        assertThat(recorder.state.value.ask).isEqualTo(Ask.DELETE_THEN_CLOSE)
        recorder.answer(delete = true)
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(prepared.single().exists()).isFalse()
    }

    @Test
    fun `Retake under ten seconds turns the camera back on`() = runTest {
        val recorder = reviewing(ms = 4_000)
        recorder.retake()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
        assertThat(camera.on).isTrue()
        assertThat(prepared.single().exists()).isFalse()
    }

    @Test
    fun `Retake from ten seconds asks first`() = runTest {
        val recorder = reviewing(ms = 30_000)
        recorder.retake()
        assertThat(recorder.state.value.ask).isEqualTo(Ask.RETAKE)
        assertThat(prepared.single().exists()).isTrue()
        recorder.answer(delete = false)
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
        recorder.retake()
        recorder.answer(delete = true)
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview())
    }

    // --- Back and Esc (S3.4, S8.4) -----------------------------------------------------

    @Test
    fun `Back closes PREVIEW, stops RECORDING and always asks in REVIEW`() = runTest {
        val preview = previewing()
        preview.back()
        assertThat(preview.state.value.isOpen).isFalse()
        assertThat(camera.on).isFalse()

        val recording = recordingFor(3_000, previewing())
        recording.back()
        assertThat(recording.state.value.phase).isInstanceOf(Phase.Finishing::class.java)
        camera.finalized(3_000)
        runCurrent()

        // Even a short clip: a reflex key never destroys one.
        recording.back()
        assertThat(recording.state.value.ask).isEqualTo(Ask.DELETE_THEN_CLOSE)
        assertThat(recording.state.value.isOpen).isTrue()
    }

    // --- REVIEW → sent (S3.4, S3.6) -----------------------------------------------------

    @Test
    fun `Send hands the clip and the reply to the outbox and closes`() = runTest {
        val recorder = reviewing()
        advanceTimeBy(600)
        recorder.send()
        runCurrent()

        val sent = sends.single()
        assertThat(sent.chatId).isEqualTo(42)
        assertThat(sent.clip.round).isTrue()
        assertThat(sent.reply).isEqualTo(reply)
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(host.sent).containsExactly(41L)
        assertThat(host.announced).containsExactly(R.string.s_announce_video_message_sent)
        // The bytes are the outbox's now: nothing deleted them.
        assertThat(prepared.single().exists()).isTrue()
    }

    /** Stop → Send is guarded too (S1.1). */
    @Test
    fun `Send right after Stop is ignored`() = runTest {
        val recorder = reviewing()
        recorder.send()
        runCurrent()
        assertThat(sends).isEmpty()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
    }

    @Test
    fun `a clip that could not be made round goes as a regular video`() = runTest {
        notRound = RoundRecorderRules.NotRound.COULDNT_MAKE_ROUND
        val recorder = reviewing()
        assertThat((recorder.state.value.phase as Phase.Review).clip.round).isFalse()
        advanceTimeBy(600)
        recorder.send()
        runCurrent()
        assertThat(sends.single().clip.round).isFalse()
    }

    @Test
    fun `an outbox that will not take it keeps REVIEW and says so`() = runTest {
        sendSucceeds = false
        val recorder = reviewing()
        advanceTimeBy(600)
        recorder.send()
        runCurrent()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(recorder.state.value.notice).isEqualTo(Notice.SEND_FAILED)
        assertThat(host.sent).isEmpty()
    }

    @Test
    fun `the banner's cross sends it without the reply`() = runTest {
        val recorder = reviewing()
        recorder.dropReply()
        assertThat(host.dropped).isEqualTo(1)
        advanceTimeBy(600)
        recorder.send()
        runCurrent()
        assertThat(sends.single().reply).isNull()
        assertThat(host.sent).containsExactly(null)
    }

    // --- PREVIEW's own ways out (S3.4) ---------------------------------------------------

    @Test
    fun `sixty seconds with no control used turns the camera off`() = runTest {
        val recorder = previewing()
        advanceTimeBy(30_000)
        recorder.used()
        advanceTimeBy(59_999)
        runCurrent()
        assertThat(recorder.state.value.isOpen).isTrue()

        advanceTimeBy(2)
        runCurrent()
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(camera.on).isFalse()
        assertThat(host.announced).containsExactly(R.string.s_announce_camera_turned_off)
    }

    /** The reply banner's ✕ is a control used too (S1.1): the minute starts again. */
    @Test
    fun `the banner's cross keeps PREVIEW open another minute`() = runTest {
        val recorder = previewing()
        advanceTimeBy(30_000)
        recorder.dropReply()
        assertThat(host.dropped).isEqualTo(1)
        advanceTimeBy(59_999)
        runCurrent()
        assertThat(recorder.state.value.isOpen).isTrue()

        advanceTimeBy(2)
        runCurrent()
        assertThat(recorder.state.value.isOpen).isFalse()
    }

    @Test
    fun `a voice message instead closes the camera first`() = runTest {
        val recorder = previewing()
        recorder.voiceInstead()
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(camera.on).isFalse()
        assertThat(host.voiceInstead).isEqualTo(1)
    }

    /** Row 9: while a not-sent voice message waits, it says why instead (S3.4). */
    @Test
    fun `a voice message instead is dimmed while one waits unsent`() = runTest {
        val recorder = recorder()
        recorder.open(session.copy(voiceBlocked = true), host, granted)
        recorder.voiceInstead()
        assertThat(recorder.state.value.isOpen).isTrue()
        assertThat(recorder.state.value.notice).isEqualTo(Notice.VOICE_BLOCKED)
        assertThat(host.voiceInstead).isEqualTo(0)
    }

    @Test
    fun `Switch camera in PREVIEW only`() = runTest {
        val recorder = previewing()
        assertThat(recorder.canSwitchCamera).isTrue()
        recorder.switchCamera()
        assertThat(camera.switches).isEqualTo(1)
        recorder.record(Surface.ROTATION_0, holdOrientation = true)
        recorder.switchCamera()
        assertThat(camera.switches).isEqualTo(1)
    }

    // --- S3.6 and S4: interruptions -----------------------------------------------------

    @Test
    fun `another app with the camera dims Record and offers voice`() = runTest {
        val recorder = previewing()
        camera.listener!!.unavailable()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(busy = true))
        assertThat(recorder.state.value.notice).isEqualTo(Notice.CAMERA_BUSY)
        recorder.record(Surface.ROTATION_0, holdOrientation = true)
        assertThat(camera.file).isNull()
    }

    /** The other app lets go: frames again, and Record is live again. */
    @Test
    fun `the camera coming back makes Record live again`() = runTest {
        val recorder = previewing()
        camera.listener!!.unavailable()
        camera.listener!!.firstFrame()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Preview(firstFrame = true))
        assertThat(recorder.state.value.notice).isNull()
        recorder.record(Surface.ROTATION_0, holdOrientation = true)
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Recording::class.java)
    }

    @Test
    fun `the camera taken mid-take stops into REVIEW with the sentence`() = runTest {
        val recorder = recordingFor(4_000)
        camera.listener!!.unavailable()
        assertThat(camera.stops).isEqualTo(1)
        camera.finalized(4_000)
        runCurrent()
        assertThat(recorder.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(recorder.state.value.notice).isEqualTo(Notice.CAMERA_BUSY)
    }

    @Test
    fun `the background closes PREVIEW, stops a take, keeps REVIEW`() = runTest {
        val preview = previewing()
        preview.backgrounded()
        assertThat(preview.state.value.isOpen).isFalse()

        val recording = recordingFor(4_000, previewing())
        recording.backgrounded()
        assertThat(recording.state.value.phase).isInstanceOf(Phase.Finishing::class.java)
        camera.finalized(4_000)
        runCurrent()
        recording.backgrounded()
        assertThat(recording.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(sends).isEmpty()
    }

    /** A permission question the recorder raised is not the background. */
    @Test
    fun `the background does not close the question`() = runTest {
        val recorder = recorder()
        recorder.open(session, host, Permissions(camera = false, microphone = false))
        recorder.backgrounded()
        assertThat(recorder.state.value.phase).isEqualTo(Phase.Asking)
    }

    @Test
    fun `a call closes PREVIEW, stops a take into REVIEW, and REVIEW waits under it`() = runTest {
        val preview = previewing()
        call.value = CallState.Outgoing(callId = "c", chatId = 9, peerUserId = 2)
        runCurrent()
        assertThat(preview.state.value.isOpen).isFalse()

        call.value = CallState.Idle
        runCurrent()
        val recording = recordingFor(4_000, previewing())
        call.value = CallState.Outgoing(callId = "d", chatId = 9, peerUserId = 2)
        runCurrent()
        assertThat(recording.state.value.phase).isInstanceOf(Phase.Finishing::class.java)
        camera.finalized(4_000)
        runCurrent()
        assertThat(recording.state.value.phase).isInstanceOf(Phase.Review::class.java)
        assertThat(sends).isEmpty()
    }

    @Test
    fun `sign-out deletes what was recorded`() = runTest {
        val recorder = reviewing()
        recorder.signedOut()
        assertThat(recorder.state.value.isOpen).isFalse()
        assertThat(prepared.single().exists()).isFalse()

        val recording = recordingFor(4_000, previewing())
        val file = camera.file!!
        recording.signedOut()
        camera.finalized(4_000)
        runCurrent()
        assertThat(recording.state.value.isOpen).isFalse()
        assertThat(file.exists()).isFalse()
    }
}

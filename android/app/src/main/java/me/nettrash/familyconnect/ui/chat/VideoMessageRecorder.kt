/*
 * VideoMessageRecorder.kt
 * Family Connect (Android)
 *
 * Recording a video message (#79, docs/audio-video-messages-2026-10-04.md,
 * Phase 3: S3, S4's video columns, S8.4, S8.5) — the recorder's states and
 * everything that moves between them: PREVIEW, RECORDING, REVIEW (S3.4), the
 * permission and refusal states before them (S3.2), and the interruptions
 * (S4).
 *
 * OWNED OUTSIDE THE ACTIVITY. MainActivity declares no `configChanges`, so a
 * rotation, a fold, a window resize or a dark-mode change rebuilds it in the
 * middle of a take — and a recorder that lived in the activity, or in a
 * camera bound to the activity's lifecycle, would end the take there. So this
 * is an app singleton ([AppVideoMessageRecorder]), and its camera
 * ([CameraSession], CameraX in the app) is bound to a LifecycleOwner the
 * recorder owns. A rebuilt activity only re-attaches the preview surface and
 * re-applies the orientation hold (RoundVideoRecorderLayer); the recording
 * carries on.
 *
 * Nothing here touches the camera, the file system's codecs or Android's
 * threads directly — those are [CameraSession], [ClipPreparer] and
 * [RoundVideoSink] — so the whole state machine runs on the JVM in
 * VideoMessageRecorderTest, against fakes and the test scheduler's clock. No
 * camera is ever opened by a test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.annotation.StringRes
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.calls.CallState
import me.nettrash.familyconnect.calls.CallStateSource
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.RoundVideoLimits
import me.nettrash.familyconnect.util.Uptime
import java.io.File

/** The camera the recorder drives — CameraX in the app (CameraXSession), a fake in the tests. */
interface CameraSession {

    /** What the camera says on its own. Called on the main thread. */
    interface Listener {
        /** The preview delivered its first frame: Record may be pressed (S3.4). */
        fun firstFrame()

        /** The first 2 s of the preview were near-black (S3.6). */
        fun looksBlack()

        /** The camera is in use by another app, or went away (S3.6, S4). */
        fun unavailable()

        /**
         * A recording ended — asked for, at the length limit ([capReached]),
         * or because the camera or the recorder failed ([failed]). [file] is
         * null when nothing readable was written.
         */
        fun finalized(file: File?, durationMs: Long, capReached: Boolean, failed: Boolean)
    }

    /** Turn the camera on, front first: PREVIEW. Nothing records and the microphone stays closed. */
    fun open(listener: Listener)

    /**
     * Start writing [file], stopping by itself at [limitMs], the capture
     * angle fixed at [rotation] (a `Surface.ROTATION_*`). The microphone
     * opens now, never before (S3.4). False when it could not start.
     */
    fun startRecording(file: File, limitMs: Long, rotation: Int): Boolean

    /** Stop writing; [Listener.finalized] follows. */
    fun stopRecording()

    /** The other camera, in PREVIEW (S3.5) — phones and tablets. */
    fun switchCamera()

    /** How many cameras there are to switch between. */
    val cameraCount: Int

    /** The camera off — the light goes out (REVIEW, closing). */
    fun close()
}

/**
 * What happens to the file once CameraX has written it (S8.4): a probe and,
 * only when it is not already the profile's square, the Media3 pass; `moov`
 * first; the poster. MediaPrep.prepareRoundVideo in the app.
 */
fun interface ClipPreparer {
    /** The clip as it will be sent, or null when nothing readable came out of [file]. */
    suspend fun prepare(file: File, recordedMs: Long, maxBytes: Long): RoundClip?
}

/** A finished clip, as REVIEW shows it and Send sends it. */
data class RoundClip(
    val prepared: MediaPrep.Prepared,
    /** Its length as shown: "Video message · 0:23". */
    val durationMs: Long,
    /** Why it goes as a regular video, or null: a video message (S3.6). */
    val notRound: RoundRecorderRules.NotRound?,
    /** Every file this clip made, so a discard leaves none behind. */
    val files: List<File>,
) {
    val round: Boolean get() = notRound == null
}

/** Where Send hands a clip: the outbox (MessageRepository.sendMedia) in the app. */
fun interface RoundVideoSink {
    /** True once the outbox has it — the row written before the first byte goes up. */
    suspend fun send(chatId: Long, clip: RoundClip, reply: ReplyToDto?): Boolean
}

/** The device's first-time PREVIEW line (S3.4): shown once per device. */
interface PreviewTeaching {
    fun taught(): Boolean
    fun markTaught()
}

/**
 * The chat that opened the recorder, for what only it can do: start a voice
 * message instead, take back its reply once the video carried it, say what
 * happened once the recorder has gone, and explain a dimmed control.
 */
interface RecorderHost {
    /** "Record a voice message instead" (S3.4): the camera is already off. */
    fun recordVoiceInstead()

    /** The video went out carrying [replyId] (or none): the composer's reply is spent. */
    fun sent(replyId: Long?)

    /** The banner's ✕: the reply is dropped from the composer too. */
    fun replyDropped()

    /** Said once the recorder has closed — "Video message sent", "Camera turned off" (S6). */
    fun announce(@StringRes text: Int)
}

open class VideoMessageRecorder(
    private val camera: CameraSession,
    private val clips: ClipPreparer,
    private val sink: RoundVideoSink,
    private val calls: CallStateSource,
    private val limits: () -> RoundVideoLimits?,
    private val teaching: PreviewTeaching,
    private val uptime: Uptime,
    private val scope: CoroutineScope,
    private val newClipFile: () -> File,
    /** Starting a recording pauses whatever plays (S1.7). */
    private val playback: PlaybackCoordinator? = null,
) {

    /** What the chat hands over when it opens the recorder. */
    data class Session(
        val chatId: Long,
        /** The composer's primed reply — the video carries it (S1.5, S3.3). */
        val reply: ReplyToDto?,
        /** Who wrote [reply], as the banner names them. */
        val replyAuthor: String,
        /** What the banner quotes of it — "Video message" for a circle, as the composer's does. */
        val replyExcerpt: String,
        /**
         * A not-sent voice message waits in this chat (S1.3 row 9):
         * "Record a voice message instead" is dimmed and says why.
         */
        val voiceBlocked: Boolean,
    )

    /** S3.4's states, and the ones before them (S3.2). */
    sealed interface Phase {
        data object Closed : Phase

        /** The system asks for what is missing; a neutral circle and "Video messages need…". */
        data object Asking : Phase

        data object CameraRefused : Phase
        data object MicrophoneRefused : Phase

        /**
         * The camera is on, nothing records. [firstFrame]: Record is live.
         * [busy]: another app has the camera (S3.6). [looksBlack]: the
         * first 2 s were near-black (S3.6).
         */
        data class Preview(
            val firstFrame: Boolean = false,
            val busy: Boolean = false,
            val looksBlack: Boolean = false,
        ) : Phase

        /** Recording since [startedAtMs] (this recorder's monotonic clock); [warned] from 50 s. */
        data class Recording(val startedAtMs: Long, val warned: Boolean = false) : Phase

        /** Stopped; the file is being checked and, if needed, squared. The camera is off. */
        data class Finishing(val recordedMs: Long) : Phase

        /** The clip as it will be sent (S3.4). The camera and the microphone are off. */
        data class Review(val clip: RoundClip) : Phase
    }

    /** A line the recorder shows (and says) about what just happened. */
    enum class Notice(@param:StringRes val text: Int) {
        TOO_SHORT(R.string.s_video_too_short),
        STOPPED_AT_ONE_MINUTE(R.string.s_recording_stopped_at_one_minute),
        CAMERA_BUSY(R.string.s_camera_in_use_by_another_app),
        STOPPED_UNEXPECTEDLY(R.string.e_recording_stopped_unexpectedly),
        SEND_FAILED(R.string.e_send_failed),
        VOICE_BLOCKED(R.string.e_send_or_delete_the_unsent_first),
    }

    /** "Delete video message?" — and what Delete does then. Keep leaves things where they are. */
    enum class Ask {
        /** RECORDING at 10 s or more: Delete goes back to PREVIEW; Keep to REVIEW. */
        DELETE_THEN_PREVIEW,

        /** REVIEW's Delete at 10 s or more, and Back: Delete closes the recorder. */
        DELETE_THEN_CLOSE,

        /** REVIEW's Retake at 10 s or more: Delete goes back to PREVIEW. */
        RETAKE,
    }

    /** What the recorder's polite live region says next (S6). */
    data class Announcement(@param:StringRes val text: Int, val serial: Long)

    data class State(
        val phase: Phase = Phase.Closed,
        val session: Session? = null,
        /** "Only you can see this until you start recording." — the first time on this device. */
        val firstTime: Boolean = false,
        val notice: Notice? = null,
        val ask: Ask? = null,
        /** On a phone, from Record to Stop: the screen holds its orientation (S3.5). */
        val holdOrientation: Boolean = false,
        /** The layout chosen at Record, kept until Stop (S3.3, S3.5). */
        val lockedLayout: RoundRecorderRules.Layout? = null,
        val announcement: Announcement? = null,
        /** The slot ignores activation until then (S1.1: Record → Stop → Send). */
        val guardUntilMs: Long = 0L,
    ) {
        val isOpen: Boolean get() = phase != Phase.Closed
    }

    private val _state = MutableStateFlow(State())
    val state: StateFlow<State> = _state.asStateFlow()

    private var host: RecorderHost? = null
    private var idleJob: Job? = null
    private var warningJob: Job? = null
    private var capJob: Job? = null
    private var callJob: Job? = null
    private var announcementSerial = 0L

    /** What Stop or Delete asked for, applied when the camera's file arrives. */
    private enum class AfterStop { REVIEW, TOO_SHORT, DISCARD_TO_PREVIEW, DISCARD_AND_CLOSE }

    private var afterStop = AfterStop.REVIEW
    private var stopNotice: Notice? = null

    /** The pane's layout as last drawn — what Record keeps. */
    @Volatile
    var currentLayout: RoundRecorderRules.Layout? = null

    /** Granted, and refused-for-good, as the screen read them when it opened the recorder. */
    data class Permissions(
        val camera: Boolean,
        val microphone: Boolean,
        val cameraRefusedForGood: Boolean = false,
        val microphoneRefusedForGood: Boolean = false,
    )

    // -- Opening and permission (S3.1, S3.2) ---------------------------------

    /**
     * Open over the whole window for [session] — the video button, the
     * paperclip, the microphone's menu or its TalkBack action (S3.1). The
     * caller has already checked the door (ComposerSlot.videoDoor) and
     * stopped any voice recording (one at a time, S1.7). Ignored while open.
     */
    fun open(session: Session, host: RecorderHost, permissions: Permissions) {
        if (_state.value.isOpen) return
        this.host = host
        val entry = RoundRecorderRules.entry(
            cameraGranted = permissions.camera,
            microphoneGranted = permissions.microphone,
            cameraRefusedForGood = permissions.cameraRefusedForGood,
            microphoneRefusedForGood = permissions.microphoneRefusedForGood,
        )
        _state.value = State(session = session, firstTime = !teaching.taught())
        watchCalls()
        enter(entry)
    }

    /** A call in any phase: the recorder lies under the call screen, still open (S4). */
    val callState: StateFlow<CallState> get() = calls.state

    /**
     * Permissions refused for good in this process — learnt from an answer,
     * as Android says so only after asking (S3.2). The next opening goes
     * straight to that refusal and raises no question for the other.
     */
    var cameraRefusedForGood = false
    var microphoneRefusedForGood = false

    /** The system's answer to the one question [Phase.Asking] raised. */
    fun permissionsAnswered(camera: Boolean, microphone: Boolean) {
        if (_state.value.phase != Phase.Asking) return
        enter(RoundRecorderRules.afterAnswer(camera, microphone))
    }

    private fun enter(entry: RoundRecorderRules.Entry) {
        when (entry) {
            RoundRecorderRules.Entry.PREVIEW -> startPreview()
            RoundRecorderRules.Entry.ASK -> _state.update { it.copy(phase = Phase.Asking) }
            // The camera is off in every refusal state (S3.2).
            RoundRecorderRules.Entry.CAMERA_REFUSED -> _state.update { it.copy(phase = Phase.CameraRefused) }
            RoundRecorderRules.Entry.MICROPHONE_REFUSED -> _state.update { it.copy(phase = Phase.MicrophoneRefused) }
        }
    }

    private fun startPreview(notice: Notice? = null) {
        _state.update { it.copy(phase = Phase.Preview(), notice = notice, ask = null) }
        camera.open(cameraListener)
        used()
    }

    private val cameraListener = object : CameraSession.Listener {
        override fun firstFrame() {
            val phase = _state.value.phase as? Phase.Preview ?: return
            if (phase.firstFrame) return
            // Frames again after another app had the camera: it is ours
            // again, and Record is live (S3.6's sentence goes).
            _state.update {
                it.copy(
                    phase = phase.copy(firstFrame = true, busy = false),
                    notice = it.notice.takeUnless { notice -> notice == Notice.CAMERA_BUSY },
                )
            }
            announce(R.string.s_announce_camera_ready)
        }

        override fun looksBlack() {
            val phase = _state.value.phase as? Phase.Preview ?: return
            _state.update { it.copy(phase = phase.copy(looksBlack = true)) }
        }

        override fun unavailable() {
            when (val phase = _state.value.phase) {
                // "The camera is being used by another app." in place of the
                // picture, Record dimmed, the voice message offered (S3.6).
                is Phase.Preview -> _state.update {
                    it.copy(phase = phase.copy(busy = true, firstFrame = false), notice = Notice.CAMERA_BUSY)
                }
                // A recording in progress stops into REVIEW with the same sentence.
                is Phase.Recording -> stopInto(Notice.CAMERA_BUSY)
                else -> Unit
            }
        }

        override fun finalized(file: File?, durationMs: Long, capReached: Boolean, failed: Boolean) {
            onFinalized(file, durationMs, capReached, failed)
        }
    }

    // -- PREVIEW (S3.4) -------------------------------------------------------

    /**
     * Any control used — which is also what keeps PREVIEW open: 60 s with
     * none closes it, the camera off, "Camera turned off" said (S3.4).
     */
    fun used() {
        idleJob?.cancel()
        if (_state.value.phase !is Phase.Preview) return
        idleJob = scope.launch {
            delay(ComposerSlot.PREVIEW_IDLE_CLOSE_MS)
            if (_state.value.phase is Phase.Preview) {
                val chat = host
                close()
                chat?.announce(R.string.s_announce_camera_turned_off)
            }
        }
    }

    /** "Switch camera" — PREVIEW only on Android (S3.5). */
    fun switchCamera() {
        if (_state.value.phase !is Phase.Preview) return
        used()
        camera.switchCamera()
    }

    /** Whether "Switch camera" is offered at all: phones and tablets with more than one. */
    val canSwitchCamera: Boolean get() = camera.cameraCount > 1

    /**
     * The slot's Record (S3.4): once the camera has delivered its first frame,
     * outside the guard, and not while another app has the camera. The
     * microphone opens now; the capture angle is fixed now ([rotation]); a
     * phone holds its orientation from now until Stop ([holdOrientation]).
     */
    fun record(rotation: Int, holdOrientation: Boolean) {
        val state = _state.value
        val phase = state.phase as? Phase.Preview ?: return
        val now = uptime.now()
        // A busy camera is never one with a first frame (unavailable() takes it back).
        if (now < state.guardUntilMs || !phase.firstFrame) return
        val limits = limits() ?: return
        val file = newClipFile()
        playback?.pauseAll()
        idleJob?.cancel()
        if (!camera.startRecording(file, ComposerSlot.roundCapMs(limits.maxMs), rotation)) {
            file.delete()
            _state.update { it.copy(notice = Notice.STOPPED_UNEXPECTEDLY) }
            return
        }
        afterStop = AfterStop.REVIEW
        stopNotice = null
        if (state.firstTime) teaching.markTaught()
        _state.update {
            it.copy(
                phase = Phase.Recording(startedAtMs = now),
                firstTime = false,
                notice = null,
                holdOrientation = holdOrientation,
                lockedLayout = currentLayout,
                guardUntilMs = now + ComposerSlot.ACTIVATION_GUARD_MS,
            )
        }
        announce(R.string.s_announce_recording_video)
        warningJob = scope.launch {
            delay(ComposerSlot.roundWarningMs(limits.maxMs))
            val recording = _state.value.phase as? Phase.Recording ?: return@launch
            _state.update { it.copy(phase = recording.copy(warned = true)) }
            announce(R.string.s_ten_seconds_left)
        }
        // CameraX stops itself at the limit (FileOutputOptions); this is the
        // backstop for a recorder that does not, and it still never sends.
        capJob = scope.launch {
            delay(ComposerSlot.roundCapMs(limits.maxMs) + CAP_BACKSTOP_MS)
            if (_state.value.phase is Phase.Recording) stopInto(Notice.STOPPED_AT_ONE_MINUTE)
        }
    }

    /** Where this recording stops by itself: `max_round_video_ms` − 500 ms (S1.1). */
    fun capMs(): Long = ComposerSlot.roundCapMs(limits()?.maxMs ?: ComposerSlot.DEFAULT_MAX_ROUND_VIDEO_MS)

    /** How long the recording has run, for the counter and the ring; 0 outside RECORDING. */
    fun recordedMs(): Long {
        val phase = _state.value.phase as? Phase.Recording ?: return 0L
        return (uptime.now() - phase.startedAtMs).coerceAtLeast(0L)
    }

    // -- RECORDING (S3.4) -------------------------------------------------------

    /**
     * The slot's Stop, Back, Esc (S3.4): under 1.0 s back to PREVIEW with
     * "That video was too short."; otherwise REVIEW. Not inside the guard.
     */
    fun stop() {
        val state = _state.value
        if (state.phase !is Phase.Recording) return
        if (uptime.now() < state.guardUntilMs) return
        stopInto(null)
    }

    private fun stopInto(notice: Notice?) {
        val recorded = recordedMs()
        val now = uptime.now()
        warningJob?.cancel()
        capJob?.cancel()
        afterStop = if (recorded < ComposerSlot.SHORTEST_RECORDING_MS) AfterStop.TOO_SHORT else AfterStop.REVIEW
        stopNotice = notice
        _state.update {
            it.copy(
                phase = Phase.Finishing(recorded),
                guardUntilMs = now + ComposerSlot.ACTIVATION_GUARD_MS,
                holdOrientation = false,
                lockedLayout = null,
            )
        }
        camera.stopRecording()
    }

    /**
     * RECORDING's Delete (label "Delete recording"): under 10 s at once, back
     * to PREVIEW; from 10 s it stops first and asks "Delete video message?" —
     * Keep goes to REVIEW (S3.4).
     */
    fun delete() {
        when (val phase = _state.value.phase) {
            is Phase.Recording -> {
                if (recordedMs() >= ComposerSlot.DELETE_ASKS_FROM_MS) {
                    stopInto(null)
                    _state.update { it.copy(ask = Ask.DELETE_THEN_PREVIEW) }
                } else {
                    stopInto(null)
                    afterStop = AfterStop.DISCARD_TO_PREVIEW
                }
            }
            is Phase.Review -> {
                if (phase.clip.durationMs >= ComposerSlot.DELETE_ASKS_FROM_MS) {
                    _state.update { it.copy(ask = Ask.DELETE_THEN_CLOSE) }
                } else {
                    discard(phase.clip)
                    close()
                }
            }
            else -> Unit
        }
    }

    /** REVIEW's Retake: from 10 s it asks first; the camera comes back on (S3.4). */
    fun retake() {
        val phase = _state.value.phase as? Phase.Review ?: return
        if (phase.clip.durationMs >= ComposerSlot.DELETE_ASKS_FROM_MS) {
            _state.update { it.copy(ask = Ask.RETAKE) }
        } else {
            discard(phase.clip)
            startPreview()
        }
    }

    /** "Delete video message?" answered: [delete], or Keep. Dismissing the question is Keep. */
    fun answer(delete: Boolean) {
        val ask = _state.value.ask ?: return
        _state.update { it.copy(ask = null) }
        if (!delete) return
        when (val phase = _state.value.phase) {
            // Answered before the file is ready: done to it when it is.
            is Phase.Finishing -> afterStop = when (ask) {
                Ask.DELETE_THEN_CLOSE -> AfterStop.DISCARD_AND_CLOSE
                Ask.DELETE_THEN_PREVIEW, Ask.RETAKE -> AfterStop.DISCARD_TO_PREVIEW
            }
            is Phase.Review -> {
                discard(phase.clip)
                when (ask) {
                    Ask.DELETE_THEN_CLOSE -> close()
                    Ask.DELETE_THEN_PREVIEW, Ask.RETAKE -> startPreview()
                }
            }
            else -> Unit
        }
    }

    private fun onFinalized(file: File?, durationMs: Long, capReached: Boolean, failed: Boolean) {
        // Unasked — the length limit, a failure — while still recording:
        // it is a stop like any other, into REVIEW (S3.4, S4).
        if (_state.value.phase is Phase.Recording) {
            stopInto(
                when {
                    capReached -> Notice.STOPPED_AT_ONE_MINUTE
                    failed -> Notice.STOPPED_UNEXPECTEDLY
                    else -> null
                },
            )
        }
        val finishing = _state.value.phase as? Phase.Finishing ?: run {
            file?.delete()
            return
        }
        val recorded = maxOf(finishing.recordedMs, durationMs)
        val notice = stopNotice ?: if (capReached) Notice.STOPPED_AT_ONE_MINUTE else null
        when {
            afterStop == AfterStop.DISCARD_AND_CLOSE -> {
                file?.delete()
                finish()
            }
            afterStop == AfterStop.DISCARD_TO_PREVIEW -> {
                file?.delete()
                _state.update { it.copy(phase = Phase.Preview(firstFrame = true)) }
                used()
            }
            afterStop == AfterStop.TOO_SHORT || (file != null && recorded < ComposerSlot.SHORTEST_RECORDING_MS) -> {
                file?.delete()
                _state.update { it.copy(phase = Phase.Preview(firstFrame = true), notice = Notice.TOO_SHORT, ask = null) }
                announce(R.string.s_video_too_short)
                used()
            }
            file == null || !file.exists() -> {
                // Nothing readable: the sentence, and the camera back (S4's
                // "the recorder fails" row: REVIEW if readable, else this).
                file?.delete()
                _state.update {
                    it.copy(phase = Phase.Preview(firstFrame = true), notice = Notice.STOPPED_UNEXPECTEDLY, ask = null)
                }
                used()
            }
            else -> {
                // REVIEW: the camera off — the light goes out — and the file
                // made into what will be sent.
                camera.close()
                if (notice == Notice.STOPPED_AT_ONE_MINUTE) announce(R.string.s_recording_stopped_at_one_minute)
                scope.launch { review(file, recorded, notice) }
            }
        }
    }

    private suspend fun review(file: File, recordedMs: Long, notice: Notice?) {
        val maxBytes = limits()?.maxBytes ?: Long.MAX_VALUE
        val clip = runCatching { clips.prepare(file, recordedMs, maxBytes) }.getOrNull()
        if (clip == null) {
            file.delete()
            if (_state.value.isOpen) startPreview(Notice.STOPPED_UNEXPECTEDLY)
            return
        }
        // Closed while it was being made (sign-out, a delete answered as it finished).
        if (_state.value.phase !is Phase.Finishing) {
            discard(clip)
            return
        }
        when (afterStop) {
            AfterStop.DISCARD_TO_PREVIEW -> {
                discard(clip)
                startPreview()
                return
            }
            AfterStop.DISCARD_AND_CLOSE -> {
                discard(clip)
                finish()
                return
            }
            else -> Unit
        }
        _state.update { it.copy(phase = Phase.Review(clip), notice = notice) }
    }

    // -- REVIEW (S3.4) ----------------------------------------------------------

    /**
     * REVIEW's Send: the recorder closes, the circle appears in the thread at
     * once from the local file, and the outbox uploads it (S3.4, S5.6). A
     * clip that could not be made round, or is too big, goes as a regular
     * video — the person was told in REVIEW (S3.6).
     */
    fun send() {
        val state = _state.value
        val phase = state.phase as? Phase.Review ?: return
        val session = state.session ?: return
        if (uptime.now() < state.guardUntilMs) return
        _state.update { it.copy(guardUntilMs = uptime.now() + ComposerSlot.ACTIVATION_GUARD_MS) }
        scope.launch {
            if (sink.send(session.chatId, phase.clip, session.reply)) {
                val chat = host
                finish()
                chat?.sent(session.reply?.messageId)
                chat?.announce(R.string.s_announce_video_message_sent)
            } else {
                _state.update { it.copy(notice = Notice.SEND_FAILED) }
            }
        }
    }

    // -- Leaving (S3.4, S4) -----------------------------------------------------

    /**
     * Back, Esc (S3.4, S8.4): PREVIEW and the refusals close; RECORDING
     * stops; REVIEW ALWAYS asks — a reflex key never destroys a clip.
     */
    fun back() {
        when (_state.value.phase) {
            Phase.Closed -> Unit
            Phase.Asking, Phase.CameraRefused, Phase.MicrophoneRefused, is Phase.Preview -> close()
            is Phase.Recording -> stop()
            is Phase.Finishing, is Phase.Review -> _state.update { it.copy(ask = Ask.DELETE_THEN_CLOSE) }
        }
    }

    /** "Close" — PREVIEW and the refusals. Anything else closes only through Delete, Send or a question. */
    fun close() {
        val phase = _state.value.phase
        if (phase is Phase.Recording || phase is Phase.Finishing) return
        if (phase is Phase.Review) discard(phase.clip)
        finish()
    }

    /**
     * "Record a voice message instead" (S3.4): closes the camera and starts a
     * hands-free voice recording — or, while a not-sent voice message waits,
     * says row 9's sentence instead.
     */
    fun voiceInstead() {
        val state = _state.value
        val phase = state.phase
        if (phase !is Phase.Preview && phase != Phase.CameraRefused) return
        if (state.session?.voiceBlocked == true) {
            _state.update { it.copy(notice = Notice.VOICE_BLOCKED) }
            return
        }
        val chat = host
        finish()
        chat?.recordVoiceInstead()
    }

    /**
     * The reply banner's ✕: the video goes without it, and so does the
     * composer. A control used, like any other — PREVIEW's minute starts
     * again (S1.1: "no Record and no other control used").
     */
    fun dropReply() {
        val session = _state.value.session ?: return
        if (session.reply == null) return
        used()
        _state.update { it.copy(session = session.copy(reply = null)) }
        host?.replyDropped()
    }

    /**
     * The app went to the BACKGROUND or the screen locked (MainActivity's
     * ON_STOP, never a configuration change): PREVIEW closes, RECORDING stops
     * into REVIEW, REVIEW is kept (S4). A permission question the recorder
     * raised is not the background.
     */
    fun backgrounded() {
        interrupted()
    }

    /** Sign-out: everything recorded and not sent is deleted (S4). */
    fun signedOut() {
        when (val phase = _state.value.phase) {
            is Phase.Recording, is Phase.Finishing -> {
                afterStop = AfterStop.DISCARD_TO_PREVIEW
                camera.stopRecording()
            }
            is Phase.Review -> discard(phase.clip)
            else -> Unit
        }
        finish()
    }

    private fun interrupted() {
        when (_state.value.phase) {
            is Phase.Preview, Phase.CameraRefused, Phase.MicrophoneRefused -> close()
            is Phase.Recording -> stopInto(null)
            else -> Unit
        }
    }

    /** A call in any phase (S4): PREVIEW closes, RECORDING stops into REVIEW, REVIEW waits under the call. */
    private fun watchCalls() {
        callJob?.cancel()
        callJob = scope.launch {
            calls.state.collect { call ->
                if (call is CallState.Live) {
                    when (_state.value.phase) {
                        Phase.Asking, Phase.CameraRefused, Phase.MicrophoneRefused, is Phase.Preview -> close()
                        is Phase.Recording -> stopInto(null)
                        else -> Unit
                    }
                }
            }
        }
    }

    private fun finish() {
        idleJob?.cancel()
        warningJob?.cancel()
        capJob?.cancel()
        callJob?.cancel()
        camera.close()
        host = null
        _state.value = State(guardUntilMs = _state.value.guardUntilMs)
    }

    private fun discard(clip: RoundClip) {
        clip.files.forEach { it.delete() }
    }

    private fun announce(@StringRes text: Int) {
        _state.update { it.copy(announcement = Announcement(text, ++announcementSerial)) }
    }

    companion object {
        /** How long past the limit the backstop waits for CameraX's own stop. */
        const val CAP_BACKSTOP_MS = 1_500L
    }
}

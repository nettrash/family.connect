/*
 * VoiceRecorder.kt
 * Family Connect (Android)
 *
 * Recording a voice note.
 *
 * Records straight into AAC-in-MP4 (`.m4a`), which is what the server's
 * magic-number check recognises as `audio/mp4` — so nothing is re-encoded on
 * the way out and a recording is uploadable the moment it stops.
 *
 * How a recording is started and ended is the composer's business, not this
 * file's: since #79 (docs/audio-video-messages-2026-10-04.md) the Send slot is
 * the microphone — a tap records hands-free on every client, and on a touch
 * screen holding it is a walkie-talkie — and every one of those decisions is
 * the shared reducer's (ui/chat/RecordGesture.kt). This only records.
 *
 * RECORD_AUDIO genuinely is required and genuinely must be granted at
 * runtime: this app owns the microphone while recording rather than
 * handing off to another app. CAMERA is now declared too (video calls) —
 * which is exactly why the chat's capture hand-off needs the runtime
 * grant these days: MediaStore's capture intents throw for an app that
 * DECLARES the permission without holding it. See ui/chat/CaptureGate.kt.
 *
 * MADE SAFE (#79, docs/audio-video-messages-2026-10-04.md, Phase 0). The
 * recorder is one per process, and it used to be driven from a ViewModel's
 * scope with nobody stopping it when that ViewModel went: Back in the middle
 * of a recording left the microphone open, and every later chat found the
 * recorder "already recording" and could not record at all. So:
 *
 *  - An INTERFACE, so the chat's tests drive a fake one.
 *  - An OWNER. Whoever starts a recording hands in a [VoiceRecorder.Listener],
 *    and every way a recording can end that the owner did not ask for is
 *    reported to it, with what was kept: the five-minute cap (an
 *    OnInfoListener — before, the hardware stopped at 5:00 and nothing told
 *    the composer, whose counter ran on), a recorder failure, another app
 *    taking the audio (focus lost TRANSIENTLY: an alarm, a phone call, the
 *    assistant), and a recording started somewhere else in the app — one at
 *    a time in the whole app (S1.7), so the old one is stopped and handed
 *    back first.
 *  - A MONOTONIC CLOCK for the counter. The wall clock jumps, and it kept
 *    counting past a stopped recorder.
 *  - Transient AUDIO FOCUS while recording, so whatever was playing pauses
 *    (the calls' pattern, calls/CallAudio.kt) — and so losing it says that
 *    something else now has the audio. Lost for GOOD, it is another app
 *    starting to play, which S4 lets a recording carry on through.
 *
 * iOS/macOS counterpart: ios/FamilyConnect/Core/AudioRecorder.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager
import android.media.MediaRecorder
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Log
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

interface VoiceRecorder {

    /** True while a recording runs. */
    val isRecording: Boolean

    /** Milliseconds captured so far, for the composer's counter; 0 while idle. */
    val elapsedMs: Long

    /**
     * Begin recording. Returns false when the microphone could not be
     * opened — almost always a refused permission, which the caller has
     * already asked for.
     *
     * A recording already running (another chat's) is stopped first and
     * handed back to ITS owner as [Ending.SUPERSEDED]: one at a time.
     */
    fun start(owner: Listener): Boolean

    /**
     * Stop and hand back the recording, or null if nothing usable was
     * captured. The owner asked, so the owner is not told.
     */
    fun stop(): Recording?

    /** Abandon it and delete the file. */
    fun cancel()

    /**
     * The loudest sample since the last call, 0–32 767 —
     * `MediaRecorder.getMaxAmplitude()`, itself a PEAK, and so the one measure
     * the level meter and the silence check share (#79, S2.9: the meter's
     * bars at −50…−10 dBFS, silence at 32 or less). 0 while idle, and on the
     * first call after a start.
     *
     * Every read is also one of the recording's PEAKS: what the composer's
     * 200 ms tick reads is what the note's waveform is made of, so the bars
     * the reader sees are the meter the sender watched.
     */
    fun maxAmplitude(): Int

    /**
     * What a recording left: the file, how long it ran by the recorder's own
     * clock, and its WAVEFORM — the peaks the meter read while it ran, as the
     * wire's 48 hex digits (docs/protocol.md, "A voice note's waveform"), or
     * null when the meter was never read.
     */
    data class Recording(val file: File, val durationMs: Long, val waveform: String? = null)

    /** The ways a recording ends without its owner asking. */
    enum class Ending {
        /** Five minutes: it stops into review with a notice, never sent. */
        CAP,

        /** The recorder failed or the disk filled; whatever was readable comes back. */
        FAILED,

        /**
         * Something else took the audio — an alarm, a phone call, the
         * assistant: the focus lost TRANSIENTLY. Another app starting to play
         * takes it for good, and that ends nothing (S4).
         */
        INTERRUPTED,

        /** A recording started somewhere else in the app (S1.7). */
        SUPERSEDED,
    }

    fun interface Listener {
        /** [recording] is what was kept, or null when nothing usable was captured. */
        fun onEnded(ending: Ending, recording: Recording?)
    }

    companion object {
        /** A voice message is not a podcast: S1.1's five minutes (`record::VOICE_CAP_MS`). */
        const val MAX_DURATION_MS = 5 * 60 * 1000

        /** Below this a file is a few bytes of container with no sound in it. */
        const val MIN_USEFUL_BYTES = 1024L
    }
}

@Singleton
class AndroidVoiceRecorder internal constructor(
    private val context: Context,
    /** Milliseconds on a clock that never jumps; `elapsedRealtime` outside the tests. */
    private val clock: () -> Long,
    /** A fresh MediaRecorder; the tests hand in one they can reach through its shadow. */
    private val newRecorder: () -> MediaRecorder,
) : VoiceRecorder {

    @Inject
    constructor(@ApplicationContext context: Context) : this(
        context = context,
        clock = SystemClock::elapsedRealtime,
        newRecorder = { createMediaRecorder(context) },
    )

    private var recorder: MediaRecorder? = null
    private var outputFile: File? = null
    private var startedAtMs: Long = 0
    private var owner: VoiceRecorder.Listener? = null
    private var focus: AudioFocusRequest? = null
    private val peaks = PeakLog()

    private val audio: AudioManager? get() = context.getSystemService(AudioManager::class.java)

    override val isRecording: Boolean get() = recorder != null

    override val elapsedMs: Long
        get() = if (isRecording) {
            (clock() - startedAtMs).coerceIn(0L, VoiceRecorder.MAX_DURATION_MS.toLong())
        } else {
            0L
        }

    override fun start(owner: VoiceRecorder.Listener): Boolean {
        if (recorder != null) {
            // One recording at a time in the whole app: the one running is
            // stopped and KEPT, and its owner decides what that means (a
            // chat parks it as "not sent").
            val previous = this.owner
            val kept = finish()
            previous?.onEnded(VoiceRecorder.Ending.SUPERSEDED, kept)
        }
        val created = runCatching { newRecorder() }.getOrElse { error ->
            Log.d(TAG, "could not create a recorder: ${error.message}")
            return false
        }
        // Unique, not a timestamp: the recording a supersede has just stopped
        // may still be being moved out of this directory by its owner.
        val file = runCatching {
            File.createTempFile("voice-", ".m4a", File(context.cacheDir, "recordings").apply { mkdirs() })
        }.getOrElse { error ->
            Log.d(TAG, "could not make a file to record into: ${error.message}")
            runCatching { created.release() }
            return false
        }
        return runCatching {
            created.apply {
                setAudioSource(MediaRecorder.AudioSource.MIC)
                // MPEG_4 + AAC is the "ftyp" container the server checks.
                setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
                setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
                // Mono: a voice note gains nothing from stereo and doubles
                // for free.
                setAudioChannels(1)
                // The protocol's voice-note row exactly: AAC-LC (AudioEncoder.AAC
                // is LC; HE_AAC and AAC_ELD are separate constants), mono, 44.1
                // kHz, 64 kbit/s — which is why a voice note never passes through
                // the audio rules that re-encode picked files.
                setAudioSamplingRate(44_100)
                setAudioEncodingBitRate(MediaPlan.VOICE_NOTE_BITRATE.toInt())
                setMaxDuration(VoiceRecorder.MAX_DURATION_MS)
                // The cap stops the hardware by itself; this is what tells
                // the owner, who stages the note with "Recording stopped at
                // five minutes." instead of showing a counter that runs on.
                setOnInfoListener { which, what, _ ->
                    if (which === recorder &&
                        what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED
                    ) {
                        end(VoiceRecorder.Ending.CAP)
                    }
                }
                setOnErrorListener { which, what, extra ->
                    if (which === recorder) {
                        Log.w(TAG, "the recorder failed ($what, $extra)")
                        end(VoiceRecorder.Ending.FAILED)
                    }
                }
                setOutputFile(file.absolutePath)
                prepare()
                start()
            }
            recorder = created
            outputFile = file
            startedAtMs = clock()
            peaks.clear()
            this.owner = owner
            takeFocus()
            true
        }.getOrElse { error ->
            Log.d(TAG, "could not start recording: ${error.message}")
            runCatching { created.release() }
            file.delete()
            false
        }
    }

    override fun stop(): VoiceRecorder.Recording? = finish()

    override fun maxAmplitude(): Int {
        val active = recorder ?: return 0
        val peak = runCatching { active.maxAmplitude }.getOrDefault(0)
        peaks.record(peak)
        return peak
    }

    override fun cancel() {
        val active = recorder ?: return
        val file = outputFile
        clear()
        runCatching { active.stop() }
        runCatching { active.release() }
        file?.delete()
    }

    /** The recorder ended it — the cap, a failure, focus lost to a taker: tell the owner what was kept. */
    private fun end(ending: VoiceRecorder.Ending) {
        val told = owner
        val kept = finish()
        told?.onEnded(ending, kept)
    }

    /**
     * Stop, release, and hand back what is usable — the one place a
     * recording ends, whoever ended it.
     *
     * A recording that never got any audio is a few bytes of container;
     * sending it would put an unplayable bubble in the thread.
     */
    private fun finish(): VoiceRecorder.Recording? {
        val active = recorder ?: return null
        val file = outputFile
        // Read before the state is cleared: the counter stops here.
        val duration = elapsedMs
        val waveform = peaks.waveform()
        peaks.clear()
        clear()
        // stop() throws when it is called before any frames were written —
        // a tap that started and ended in the same instant, or a recorder
        // that has already failed — and that is a discard, not a crash.
        runCatching { active.stop() }.onFailure {
            Log.d(TAG, "recording produced nothing: ${it.message}")
            runCatching { active.release() }
            file?.delete()
            return null
        }
        runCatching { active.release() }
        if (file == null || file.length() < VoiceRecorder.MIN_USEFUL_BYTES) {
            file?.delete()
            return null
        }
        return VoiceRecorder.Recording(file, duration, waveform)
    }

    private fun clear() {
        recorder = null
        outputFile = null
        owner = null
        giveBackFocus()
    }

    /**
     * Transient focus: what was playing pauses, and comes back after.
     *
     * HOW it is lost says who took it, and S4 treats the two differently:
     *  - a TRANSIENT loss is an alarm, a phone call ringing or answered, the
     *    assistant — the system's own takers ask for transient focus (the
     *    clock's alarm and Telecom's ringer both ask for
     *    AUDIOFOCUS_GAIN_TRANSIENT) — so something else has the audio now,
     *    and the recording stops and is kept (S4's "Siri, an alarm or another
     *    app takes the microphone");
     *  - a PERMANENT loss is another app starting to PLAY — media players ask
     *    for AUDIOFOCUS_GAIN, as a music app an earbud tap resumes does — and
     *    S4's "Another app starts playing" row leaves a recording alone: it
     *    records on;
     *  - a chime that only asks the others to duck stops nothing either.
     * Best effort: a refused request records anyway, as before.
     */
    private fun takeFocus() {
        val manager = audio ?: return
        val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_MEDIA)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                    .build(),
            )
            .setOnAudioFocusChangeListener(
                { change ->
                    when (change) {
                        AudioManager.AUDIOFOCUS_LOSS_TRANSIENT ->
                            if (recorder != null) end(VoiceRecorder.Ending.INTERRUPTED)
                        // AUDIOFOCUS_LOSS (another app plays) and
                        // AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK (a chime): recording on.
                        else -> Unit
                    }
                },
                // The recorder is driven from the main thread; so is this.
                Handler(Looper.getMainLooper()),
            )
            .build()
        focus = request
        runCatching { manager.requestAudioFocus(request) }
    }

    private fun giveBackFocus() {
        val request = focus ?: return
        focus = null
        runCatching { audio?.abandonAudioFocusRequest(request) }
    }

    private companion object {
        const val TAG = "VoiceRecorder"

        /**
         * The Context-taking constructor is API 31+; the no-arg one is
         * deprecated there but is the only option below it.
         */
        fun createMediaRecorder(context: Context): MediaRecorder =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                MediaRecorder(context)
            } else {
                @Suppress("DEPRECATION")
                MediaRecorder()
            }
    }
}

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
 * Tap to start, tap to stop, matching the Apple clients: hold-to-talk is a
 * phone gesture with no desktop equivalent, and the three composers should
 * not diverge on something this basic.
 *
 * RECORD_AUDIO genuinely is required and genuinely must be granted at
 * runtime: this app owns the microphone while recording rather than
 * handing off to another app. CAMERA is now declared too (video calls) —
 * which is exactly why the chat's capture hand-off needs the runtime
 * grant these days: MediaStore's capture intents throw for an app that
 * DECLARES the permission without holding it. See ui/chat/CaptureGate.kt.
 *
 * iOS/macOS counterpart: ios/FamilyConnect/Core/AudioRecorder.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.media.MediaRecorder
import android.os.Build
import android.util.Log
import dagger.hilt.android.qualifiers.ApplicationContext
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class VoiceRecorder @Inject constructor(
    @param:ApplicationContext private val context: Context,
) {

    private var recorder: MediaRecorder? = null
    private var outputFile: File? = null
    private var startedAtMs: Long = 0

    val isRecording: Boolean get() = recorder != null

    /** Milliseconds captured so far, for the composer's counter. */
    val elapsedMs: Long
        get() = if (isRecording) System.currentTimeMillis() - startedAtMs else 0

    /**
     * Begin recording. Returns false when the microphone could not be
     * opened — almost always a refused permission, which the caller has
     * already asked for.
     */
    fun start(): Boolean {
        if (isRecording) return true
        val dir = File(context.cacheDir, "recordings").apply { mkdirs() }
        val file = File(dir, "voice-${System.currentTimeMillis()}.m4a")

        // The Context-taking constructor is API 31+; the no-arg one is
        // deprecated there but is the only option below it.
        val created = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            MediaRecorder(context)
        } else {
            @Suppress("DEPRECATION")
            MediaRecorder()
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
                setAudioSamplingRate(44_100)
                setAudioEncodingBitRate(64_000)
                setMaxDuration(MAX_DURATION_MS)
                setOutputFile(file.absolutePath)
                prepare()
                start()
            }
            recorder = created
            outputFile = file
            startedAtMs = System.currentTimeMillis()
            true
        }.getOrElse { error ->
            Log.d(TAG, "could not start recording: ${error.message}")
            runCatching { created.release() }
            file.delete()
            false
        }
    }

    /**
     * Stop and hand back the file, or null if nothing usable was captured.
     *
     * A recording that never got any audio is a few bytes of container;
     * sending it would put an unplayable bubble in the thread.
     */
    fun stop(): File? {
        val active = recorder ?: return null
        val file = outputFile
        recorder = null
        outputFile = null
        // stop() throws when it is called before any frames were written —
        // a tap that started and ended in the same instant — and that is a
        // discard, not a crash.
        runCatching { active.stop() }.onFailure {
            Log.d(TAG, "recording produced nothing: ${it.message}")
            runCatching { active.release() }
            file?.delete()
            return null
        }
        runCatching { active.release() }
        if (file == null || file.length() < MIN_USEFUL_BYTES) {
            file?.delete()
            return null
        }
        return file
    }

    /** Abandon it and delete the file. */
    fun cancel() {
        val active = recorder ?: return
        val file = outputFile
        recorder = null
        outputFile = null
        runCatching { active.stop() }
        runCatching { active.release() }
        file?.delete()
    }

    private companion object {
        const val TAG = "VoiceRecorder"

        /** A voice message is not a podcast. */
        const val MAX_DURATION_MS = 5 * 60 * 1000

        const val MIN_USEFUL_BYTES = 1024L
    }
}

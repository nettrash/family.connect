/*
 * CameraXSession.kt
 * Family Connect (Android)
 *
 * The video message recorder's camera (#79, docs/audio-video-messages-2026-10-04.md,
 * S3.5, S8.4, "The recording profile for a round video"), on CameraX 1.5.1.
 *
 *  - BOUND TO ITS OWN LIFECYCLE ([RecorderLifecycle]), never the activity's:
 *    MainActivity declares no `configChanges`, so a rotation, a fold or a
 *    dark-mode change rebuilds it mid-take, and a camera bound to it would
 *    stop there. A rebuilt activity only hands in a new PreviewView
 *    ([attachPreview]) — `Preview.setSurfaceProvider` — and the recording
 *    carries on.
 *  - Preview and VideoCapture in ONE UseCaseGroup with a 1:1 ViewPort, so
 *    the preview and the file are cropped to the same centre square.
 *  - The Recorder asks for SD at 4:3 — the 640 × 480 mode front cameras
 *    offer, whose centre crop is the profile's 480 — at 500 000 bit/s, and
 *    stops by itself at `max_round_video_ms` − 500 ms (FileOutputOptions'
 *    duration limit). Whatever it writes is checked after Stop and, unless it
 *    is exactly 480 × 480 H.264 + AAC at rotation 0, squared by Media3
 *    (MediaPrep.prepareRoundVideo).
 *  - The FILE is not mirrored (`MIRROR_MODE_OFF`): it is the true view, the
 *    one a call's other side sees. PreviewView mirrors the front camera's
 *    preview itself, like a mirror (S3.5, Decision 33).
 *  - The capture angle is fixed at Record from the device's orientation
 *    (`VideoCapture.targetRotation`, S3.5) and kept until Stop.
 *  - The microphone opens with the recording (`withAudioEnabled` at Record),
 *    never in PREVIEW, so no indicator contradicts "Not recording" (S3.4).
 *
 * Nothing here can run on the JVM; VideoMessageRecorderTest drives the
 * recorder with a fake [CameraSession]. What it needs a device for —
 * whether CameraX's output is square at all, its rotation after a sideways
 * take, a fold mid-take — is the plan's Blocked 4.
 */

package me.nettrash.familyconnect.ui.chat

import android.Manifest
import android.annotation.SuppressLint
import android.content.Context
import android.content.pm.PackageManager
import android.os.Handler
import android.os.Looper
import android.util.Log
import android.util.Rational
import android.view.Surface
import androidx.camera.core.AspectRatio
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.CameraState
import androidx.camera.core.MirrorMode
import androidx.camera.core.Preview
import androidx.camera.core.UseCaseGroup
import androidx.camera.core.ViewPort
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.video.FallbackStrategy
import androidx.camera.video.FileOutputOptions
import androidx.camera.video.Quality
import androidx.camera.video.QualitySelector
import androidx.camera.video.Recorder
import androidx.camera.video.Recording
import androidx.camera.video.VideoCapture
import androidx.camera.video.VideoRecordEvent
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.lifecycle.Observer
import java.io.File

/**
 * The lifecycle CameraX is bound to: the recorder's, not any activity's.
 * RESUMED while the camera is on, CREATED otherwise — never DESTROYED, which
 * a registry cannot come back from. Main thread only, as LifecycleRegistry is.
 */
private class RecorderLifecycle : LifecycleOwner {
    private val registry = LifecycleRegistry(this)
    override val lifecycle: Lifecycle get() = registry

    fun on() {
        registry.currentState = Lifecycle.State.RESUMED
    }

    fun off() {
        if (registry.currentState != Lifecycle.State.INITIALIZED) registry.currentState = Lifecycle.State.CREATED
    }
}

class CameraXSession(context: Context) : CameraSession {

    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val owner = RecorderLifecycle()

    private var listener: CameraSession.Listener? = null
    private var provider: ProcessCameraProvider? = null
    private var camera: Camera? = null
    private var preview: Preview? = null
    private var capture: VideoCapture<Recorder>? = null
    private var recording: Recording? = null
    private var previewView: PreviewView? = null
    private var front = true
    private var open = false

    /** Whether the stream reached the PreviewView: the first frame (S3.4). */
    private val streamObserver = Observer<PreviewView.StreamState> { stream ->
        if (stream == PreviewView.StreamState.STREAMING && open) {
            listener?.firstFrame()
            main.removeCallbacks(blackCheck)
            main.postDelayed(blackCheck, RoundRecorderRules.BLACK_CHECK_AFTER_MS)
        }
    }

    /** Another app has the camera, or it went away (S3.6, S4). */
    private val stateObserver = Observer<CameraState> { state ->
        val code = state.error?.code ?: return@Observer
        if (code == CameraState.ERROR_CAMERA_IN_USE ||
            code == CameraState.ERROR_MAX_CAMERAS_IN_USE ||
            code == CameraState.ERROR_CAMERA_DISABLED ||
            code == CameraState.ERROR_CAMERA_FATAL_ERROR
        ) {
            listener?.unavailable()
        }
    }

    /**
     * S3.6: two seconds of a near-black preview — Android 12's camera toggle
     * gives "a blank camera feed" and no error — is said, never refused.
     */
    private val blackCheck = Runnable {
        val view = previewView ?: return@Runnable
        val frame = runCatching { view.bitmap }.getOrNull() ?: return@Runnable
        try {
            val stepX = (frame.width / SAMPLES).coerceAtLeast(1)
            val stepY = (frame.height / SAMPLES).coerceAtLeast(1)
            var sum = 0.0
            var count = 0
            var y = 0
            while (y < frame.height) {
                var x = 0
                while (x < frame.width) {
                    sum += RoundRecorderRules.luma(frame.getPixel(x, y))
                    count++
                    x += stepX
                }
                y += stepY
            }
            if (count > 0 && RoundRecorderRules.nearBlack(sum / count)) listener?.looksBlack()
        } finally {
            frame.recycle()
        }
    }

    override val cameraCount: Int
        get() = runCatching { provider?.availableCameraInfos?.size }.getOrNull() ?: 1

    override fun open(listener: CameraSession.Listener) {
        this.listener = listener
        open = true
        owner.on()
        val ready = provider
        if (ready != null) {
            bind(ready)
            return
        }
        val future = ProcessCameraProvider.getInstance(context)
        future.addListener(
            {
                val cameras = runCatching { future.get() }.getOrNull()
                if (cameras == null) {
                    listener.unavailable()
                    return@addListener
                }
                provider = cameras
                if (open) bind(cameras)
            },
            ContextCompat.getMainExecutor(context),
        )
    }

    /** Preview + VideoCapture in one group with a 1:1 viewport, front camera first (S3.5). */
    private fun bind(cameras: ProcessCameraProvider) {
        cameras.unbindAll()
        camera?.cameraInfo?.cameraState?.removeObserver(stateObserver)
        val selector = selector(cameras) ?: run {
            listener?.unavailable()
            return
        }
        val newPreview = Preview.Builder().build()
        val recorder = Recorder.Builder()
            .setQualitySelector(
                QualitySelector.from(Quality.SD, FallbackStrategy.higherQualityOrLowerThan(Quality.SD)),
            )
            .setAspectRatio(AspectRatio.RATIO_4_3)
            .setTargetVideoEncodingBitRate(RoundRecorderRules.VIDEO_BITRATE)
            .build()
        val newCapture = VideoCapture.Builder(recorder)
            // The file is the true view, not a mirror (S3.5).
            .setMirrorMode(MirrorMode.MIRROR_MODE_OFF)
            .build()
        val group = UseCaseGroup.Builder()
            .setViewPort(ViewPort.Builder(Rational(1, 1), Surface.ROTATION_0).build())
            .addUseCase(newPreview)
            .addUseCase(newCapture)
            .build()
        val bound = runCatching { cameras.bindToLifecycle(owner, selector, group) }.getOrElse { error ->
            Log.w(TAG, "could not bind the camera: ${error.javaClass.simpleName}")
            listener?.unavailable()
            return
        }
        camera = bound
        preview = newPreview
        capture = newCapture
        bound.cameraInfo.cameraState.observe(owner, stateObserver)
        previewView?.let { attachTo(it) }
    }

    private fun selector(cameras: ProcessCameraProvider): CameraSelector? {
        val wanted = if (front) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
        val other = if (front) CameraSelector.DEFAULT_BACK_CAMERA else CameraSelector.DEFAULT_FRONT_CAMERA
        return listOf(wanted, other).firstOrNull { runCatching { cameras.hasCamera(it) }.getOrDefault(false) }
            ?: cameras.availableCameraInfos.firstOrNull()?.cameraSelector
    }

    /**
     * The PreviewView this activity draws (or null as it goes): a rebuilt
     * activity hands in its new one, and only the surface moves — the
     * recording carries on (S4).
     */
    fun attachPreview(view: PreviewView?) {
        val old = previewView
        if (old === view) return
        old?.previewStreamState?.removeObserver(streamObserver)
        previewView = view
        if (view == null) {
            preview?.surfaceProvider = null
            return
        }
        attachTo(view)
    }

    /** Whether [view] is the one the preview draws into now. */
    fun isAttached(view: PreviewView): Boolean = previewView === view

    private fun attachTo(view: PreviewView) {
        view.previewStreamState.removeObserver(streamObserver)
        view.previewStreamState.observe(owner, streamObserver)
        preview?.surfaceProvider = view.surfaceProvider
    }

    @SuppressLint("MissingPermission")
    override fun startRecording(file: File, limitMs: Long, rotation: Int): Boolean {
        val output = capture ?: return false
        // The microphone opens HERE, with the recording (S3.4) — and only
        // with the permission the recorder already asked for (S3.2).
        val microphone = ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
        if (!microphone) return false
        // The capture angle, fixed at Record (S3.5).
        output.targetRotation = rotation
        val options = FileOutputOptions.Builder(file).setDurationLimitMillis(limitMs).build()
        recording = runCatching {
            output.output.prepareRecording(context, options)
                .withAudioEnabled()
                .start(ContextCompat.getMainExecutor(context)) { event -> onEvent(file, event) }
        }.getOrElse { error ->
            Log.w(TAG, "could not start recording: ${error.javaClass.simpleName}")
            return false
        }
        return true
    }

    private fun onEvent(file: File, event: VideoRecordEvent) {
        if (event !is VideoRecordEvent.Finalize) return
        recording = null
        val error = event.error
        val durationMs = event.recordingStats.recordedDurationNanos / 1_000_000L
        val capReached = error == VideoRecordEvent.Finalize.ERROR_DURATION_LIMIT_REACHED
        // Every error but these leaves a file that may still be readable; the
        // recorder checks the file itself after Stop.
        val nothingWritten = error == VideoRecordEvent.Finalize.ERROR_NO_VALID_DATA ||
            error == VideoRecordEvent.Finalize.ERROR_INVALID_OUTPUT_OPTIONS
        val failed = event.hasError() && !capReached
        if (failed) Log.w(TAG, "recording ended with error $error")
        listener?.finalized(
            file = if (nothingWritten || !file.exists()) null else file,
            durationMs = durationMs,
            capReached = capReached,
            failed = failed,
        )
    }

    override fun stopRecording() {
        recording?.stop()
    }

    override fun switchCamera() {
        front = !front
        provider?.let(::bind)
    }

    override fun close() {
        open = false
        main.removeCallbacks(blackCheck)
        recording?.stop()
        camera?.cameraInfo?.cameraState?.removeObserver(stateObserver)
        provider?.unbindAll()
        camera = null
        preview = null
        capture = null
        front = true
        owner.off()
    }

    private companion object {
        const val TAG = "CameraXSession"

        /** Sampled across and down a preview frame for the black check. */
        const val SAMPLES = 32
    }
}

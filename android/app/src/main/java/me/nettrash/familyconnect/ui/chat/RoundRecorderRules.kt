/*
 * RoundRecorderRules.kt
 * Family Connect (Android)
 *
 * The video recorder's arithmetic and decisions (#79,
 * docs/audio-video-messages-2026-10-04.md, S3, S8.4, "The recording profile
 * for a round video") — everything about recording a video message that is a
 * RULE rather than a camera, kept free of Android so plain JUnit pins it:
 *
 *  - where the recorder opens: straight to a refusal, to the system's
 *    question, or to PREVIEW (S3.2);
 *  - the capture angle from an OrientationEventListener's degrees (S3.5);
 *  - the circle's diameter and whether the controls stand in a column (S3.3);
 *  - whether a phone holds its orientation from Record to Stop (S3.5, S8.4);
 *  - whether the file CameraX wrote needs the Media3 square pass (S8.4);
 *  - whether a finished clip may go as a video message or must go as a
 *    regular video, and why (S3.6);
 *  - whether a preview is too dark to be a picture (S3.6).
 *
 * The recorder (VideoMessageRecorder) asks these; it decides nothing itself.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.Surface
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.RoundSend

object RoundRecorderRules {

    // --- S3.2, permission -------------------------------------------------

    /** Where the recorder opens. */
    enum class Entry {
        /** Both are granted: the camera comes on, nothing records. */
        PREVIEW,

        /** One or both have not been answered: ONE system question for what is missing. */
        ASK,

        /** "Family needs permission to use your camera…", the camera off. */
        CAMERA_REFUSED,

        /** The microphone's sentence: voice needs it too, so nothing else is offered. */
        MICROPHONE_REFUSED,
    }

    /**
     * S3.2: "Both statuses are read first: if either is already refused, the
     * recorder opens straight to that refusal and raises no prompt for the
     * other." Android can only know "already refused" once a question has
     * been answered for good (a question it will not show again), so that is
     * what [cameraRefusedForGood] and [microphoneRefusedForGood] are.
     *
     * Both refused: the MICROPHONE's refusal — the camera's offers "Record a
     * voice message instead", which a refused microphone would make a dead end.
     */
    fun entry(
        cameraGranted: Boolean,
        microphoneGranted: Boolean,
        cameraRefusedForGood: Boolean,
        microphoneRefusedForGood: Boolean,
    ): Entry = when {
        !microphoneGranted && microphoneRefusedForGood -> Entry.MICROPHONE_REFUSED
        !cameraGranted && cameraRefusedForGood -> Entry.CAMERA_REFUSED
        cameraGranted && microphoneGranted -> Entry.PREVIEW
        else -> Entry.ASK
    }

    /** What the one `RequestMultiplePermissions` answer leads to — the microphone's refusal first, as [entry]. */
    fun afterAnswer(cameraGranted: Boolean, microphoneGranted: Boolean): Entry = when {
        !microphoneGranted -> Entry.MICROPHONE_REFUSED
        !cameraGranted -> Entry.CAMERA_REFUSED
        else -> Entry.PREVIEW
    }

    // --- S3.5, the capture angle -----------------------------------------

    /**
     * `VideoCapture.targetRotation` for a device an OrientationEventListener
     * reports at [degrees] (0 = upright, clockwise), read at Record and kept
     * until Stop. [fallback] — the display's rotation — when the listener
     * has nothing to say (flat on a table: `ORIENTATION_UNKNOWN`, −1).
     */
    fun surfaceRotation(degrees: Int, fallback: Int): Int = when {
        degrees < 0 -> fallback
        degrees % 360 in 45 until 135 -> Surface.ROTATION_270
        degrees % 360 in 135 until 225 -> Surface.ROTATION_180
        degrees % 360 in 225 until 315 -> Surface.ROTATION_90
        else -> Surface.ROTATION_0
    }

    /**
     * On a PHONE the screen holds its current orientation from Record to
     * Stop (`SCREEN_ORIENTATION_LOCKED`, never forced portrait). At 600 dp
     * and wider Android 16 ignores the request (S8.5), and the layout chosen
     * at Record is what stays put instead.
     */
    fun holdsOrientation(smallestScreenWidthDp: Int): Boolean = smallestScreenWidthDp < 600

    // --- S3.3, the layout ---------------------------------------------------

    /** The recorder's circle, in dp, and whether the controls stand in a column at the trailing edge. */
    data class Layout(val diameter: Int, val column: Boolean)

    const val MAX_DIAMETER = 320
    const val MIN_DIAMETER = 160

    /** A pane shorter than this — a phone on its side — puts the controls in a column. */
    const val COLUMN_BELOW_HEIGHT = 480

    /**
     * S3.3: D = min(320, pane width − 48, pane height − 240 − the reply
     * banner), at least 160; in a pane shorter than 480, D = min(320, pane
     * height − 96, pane width − 200), at least 160, with the controls in a
     * column. All in dp.
     */
    fun layout(paneWidth: Int, paneHeight: Int, bannerHeight: Int): Layout =
        if (paneHeight < COLUMN_BELOW_HEIGHT) {
            Layout(
                diameter = minOf(MAX_DIAMETER, paneHeight - 96, paneWidth - 200).coerceAtLeast(MIN_DIAMETER),
                column = true,
            )
        } else {
            Layout(
                diameter = minOf(MAX_DIAMETER, paneWidth - 48, paneHeight - 240 - bannerHeight)
                    .coerceAtLeast(MIN_DIAMETER),
                column = false,
            )
        }

    // --- S8.4, the file after Stop -----------------------------------------

    /** The profile's square, on a side. */
    const val EDGE = 480

    /** The profile's bit rates. */
    const val VIDEO_BITRATE = 500_000
    const val AUDIO_BITRATE = 64_000

    /**
     * What a probe of the file CameraX wrote says — the STORED size, before
     * any rotation (`readVideoMetadata` folds the rotation into the size, and
     * a square hides it), the rotation from `METADATA_KEY_VIDEO_ROTATION`,
     * and the two tracks' types.
     */
    data class ClipFacts(
        val storedWidth: Int?,
        val storedHeight: Int?,
        val rotation: Int,
        val videoMime: String?,
        val audioMime: String?,
    )

    /**
     * Whether the file must take the Media3 pass (S8.4): anything but
     * EXACTLY 480 × 480, H.264 + AAC, at rotation 0. The pass crops to the
     * centre square, scales to 480 and bakes any rotation into the pixels.
     */
    fun needsSquarePass(facts: ClipFacts): Boolean =
        !(
            facts.storedWidth == EDGE &&
                facts.storedHeight == EDGE &&
                facts.rotation == 0 &&
                facts.videoMime.equals(MIME_H264, ignoreCase = true) &&
                facts.audioMime.equals(MIME_AAC, ignoreCase = true)
            )

    const val MIME_H264 = "video/avc"
    const val MIME_AAC = "audio/mp4a-latm"

    // --- S3.6, round or regular ---------------------------------------------

    /** Why a finished clip goes as a regular video, said in REVIEW before Send. */
    enum class NotRound {
        /** "Couldn't make it round. It will be sent as a regular video." */
        COULDNT_MAKE_ROUND,

        /** "Too big for a video message. It will be sent as a regular video." */
        TOO_BIG,
    }

    /**
     * Whether [prepared] — the file as it will be sent — may carry
     * `round: true`. [squared]: the file is square H.264 at rotation 0, as
     * recorded or after the pass; false when the pass failed. Anything the
     * server would refuse as a video message (RoundSend) is "couldn't make it
     * round"; a file over [maxBytes] is "too big". Null: a video message.
     */
    fun notRound(prepared: MediaPrep.Prepared, squared: Boolean, sizeBytes: Long, maxBytes: Long): NotRound? = when {
        !squared -> NotRound.COULDNT_MAKE_ROUND
        // The shape alone here — the size is the next line's, and says "too big".
        !RoundSend.accepts(listOf(prepared), caption = "", sticker = false, maxBytes = Long.MAX_VALUE) ->
            NotRound.COULDNT_MAKE_ROUND
        sizeBytes > maxBytes -> NotRound.TOO_BIG
        else -> null
    }

    // --- S3.6, a picture that stays black ----------------------------------

    /** How long the PREVIEW is watched for a black picture, from its first frame. */
    const val BLACK_CHECK_AFTER_MS = 2_000L

    /**
     * Whether a preview frame's mean luma (0–255, Rec. 601 weights) is a
     * camera that sees nothing — Android 12's camera toggle gives "a blank
     * camera feed" and no error, and so does a privacy shutter. A dark room
     * still lifts the mean well above this; Record stays usable either way.
     */
    fun nearBlack(meanLuma: Double): Boolean = meanLuma <= NEAR_BLACK_LUMA

    /** At or below this mean luma a frame is black — sensor noise on a covered lens stays under it. */
    const val NEAR_BLACK_LUMA = 8.0

    /** Rec. 601 luma of one ARGB pixel, 0–255. */
    fun luma(argb: Int): Double {
        val r = (argb shr 16) and 0xFF
        val g = (argb shr 8) and 0xFF
        val b = argb and 0xFF
        return 0.299 * r + 0.587 * g + 0.114 * b
    }
}

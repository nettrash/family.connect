/*
 * RoundRecorderRulesTest.kt
 * Family Connect (Android)
 *
 * The video recorder's rules (#79, docs/audio-video-messages-2026-10-04.md,
 * S3.2, S3.3, S3.5, S3.6, S8.4): where it opens, the capture angle, the
 * layout, the orientation hold, the post-Stop probe's decision, round or
 * regular, and a black picture. Plain JUnit — no camera, no device.
 */

package me.nettrash.familyconnect.ui.chat

import android.view.Surface
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.ui.chat.RoundRecorderRules.ClipFacts
import me.nettrash.familyconnect.ui.chat.RoundRecorderRules.Entry
import me.nettrash.familyconnect.ui.chat.RoundRecorderRules.Layout
import me.nettrash.familyconnect.ui.chat.RoundRecorderRules.NotRound
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class RoundRecorderRulesTest {

    @get:Rule
    val folder = TemporaryFolder()

    // --- S3.2 -----------------------------------------------------------------

    @Test
    fun `both granted opens to PREVIEW and nothing is asked`() {
        assertThat(RoundRecorderRules.entry(true, true, false, false)).isEqualTo(Entry.PREVIEW)
    }

    @Test
    fun `anything missing and not refused for good is ONE question`() {
        assertThat(RoundRecorderRules.entry(false, true, false, false)).isEqualTo(Entry.ASK)
        assertThat(RoundRecorderRules.entry(true, false, false, false)).isEqualTo(Entry.ASK)
        assertThat(RoundRecorderRules.entry(false, false, false, false)).isEqualTo(Entry.ASK)
    }

    /** "If either is already refused, the recorder opens straight to that refusal and raises no prompt for the other." */
    @Test
    fun `a refusal already known opens straight to it, asking nothing`() {
        // The camera refused for good, the microphone never asked: no question.
        assertThat(RoundRecorderRules.entry(false, false, true, false)).isEqualTo(Entry.CAMERA_REFUSED)
        assertThat(RoundRecorderRules.entry(false, true, true, false)).isEqualTo(Entry.CAMERA_REFUSED)
        // The microphone refused for good, the camera never asked: no question.
        assertThat(RoundRecorderRules.entry(false, false, false, true)).isEqualTo(Entry.MICROPHONE_REFUSED)
        assertThat(RoundRecorderRules.entry(true, false, false, true)).isEqualTo(Entry.MICROPHONE_REFUSED)
        // Both: the microphone's — the camera's offers a voice message the microphone would refuse.
        assertThat(RoundRecorderRules.entry(false, false, true, true)).isEqualTo(Entry.MICROPHONE_REFUSED)
    }

    /** A grant held now outweighs a refusal remembered from before (Settings turned it back on). */
    @Test
    fun `a grant held now is not a refusal`() {
        assertThat(RoundRecorderRules.entry(true, true, true, true)).isEqualTo(Entry.PREVIEW)
    }

    @Test
    fun `the answer leads to PREVIEW only with both`() {
        assertThat(RoundRecorderRules.afterAnswer(true, true)).isEqualTo(Entry.PREVIEW)
        assertThat(RoundRecorderRules.afterAnswer(false, true)).isEqualTo(Entry.CAMERA_REFUSED)
        assertThat(RoundRecorderRules.afterAnswer(true, false)).isEqualTo(Entry.MICROPHONE_REFUSED)
        assertThat(RoundRecorderRules.afterAnswer(false, false)).isEqualTo(Entry.MICROPHONE_REFUSED)
    }

    // --- S3.5 -----------------------------------------------------------------

    @Test
    fun `the capture angle follows the device at Record`() {
        val fallback = Surface.ROTATION_90
        assertThat(RoundRecorderRules.surfaceRotation(0, fallback)).isEqualTo(Surface.ROTATION_0)
        assertThat(RoundRecorderRules.surfaceRotation(44, fallback)).isEqualTo(Surface.ROTATION_0)
        assertThat(RoundRecorderRules.surfaceRotation(45, fallback)).isEqualTo(Surface.ROTATION_270)
        assertThat(RoundRecorderRules.surfaceRotation(134, fallback)).isEqualTo(Surface.ROTATION_270)
        assertThat(RoundRecorderRules.surfaceRotation(135, fallback)).isEqualTo(Surface.ROTATION_180)
        assertThat(RoundRecorderRules.surfaceRotation(224, fallback)).isEqualTo(Surface.ROTATION_180)
        assertThat(RoundRecorderRules.surfaceRotation(225, fallback)).isEqualTo(Surface.ROTATION_90)
        assertThat(RoundRecorderRules.surfaceRotation(314, fallback)).isEqualTo(Surface.ROTATION_90)
        assertThat(RoundRecorderRules.surfaceRotation(315, fallback)).isEqualTo(Surface.ROTATION_0)
        assertThat(RoundRecorderRules.surfaceRotation(359, fallback)).isEqualTo(Surface.ROTATION_0)
    }

    /** Flat on a table the listener says nothing: the display's own rotation. */
    @Test
    fun `an unknown orientation keeps the display's rotation`() {
        assertThat(RoundRecorderRules.surfaceRotation(-1, Surface.ROTATION_270)).isEqualTo(Surface.ROTATION_270)
    }

    /** Phones hold their orientation; at 600 dp Android 16 ignores the request (S8.5). */
    @Test
    fun `only a phone holds its orientation`() {
        assertThat(RoundRecorderRules.holdsOrientation(411)).isTrue()
        assertThat(RoundRecorderRules.holdsOrientation(599)).isTrue()
        assertThat(RoundRecorderRules.holdsOrientation(600)).isFalse()
        assertThat(RoundRecorderRules.holdsOrientation(840)).isFalse()
    }

    // --- S3.3 -----------------------------------------------------------------

    @Test
    fun `the diameter on a phone held upright`() {
        // 411 × 800: min(320, 411 − 48, 800 − 240) = 320.
        assertThat(RoundRecorderRules.layout(411, 800, 0)).isEqualTo(Layout(320, column = false))
        // A narrow 320-dp phone: 320 − 48 = 272.
        assertThat(RoundRecorderRules.layout(320, 640, 0)).isEqualTo(Layout(272, column = false))
        // A short pane with a reply banner: 560 − 240 − 56 = 264.
        assertThat(RoundRecorderRules.layout(411, 560, 56)).isEqualTo(Layout(264, column = false))
    }

    @Test
    fun `never under 160`() {
        assertThat(RoundRecorderRules.layout(180, 480, 56)).isEqualTo(Layout(160, column = false))
    }

    /** Shorter than 480 — a phone on its side — stands the controls in a column. */
    @Test
    fun `a phone on its side puts the controls in a column`() {
        // 800 × 380: min(320, 380 − 96, 800 − 200) = 284.
        assertThat(RoundRecorderRules.layout(800, 380, 0)).isEqualTo(Layout(284, column = true))
        assertThat(RoundRecorderRules.layout(800, 479, 56)).isEqualTo(Layout(320, column = true))
        assertThat(RoundRecorderRules.layout(300, 300, 0)).isEqualTo(Layout(160, column = true))
        assertThat(RoundRecorderRules.layout(800, 480, 0).column).isFalse()
    }

    // --- S8.4: the file after Stop ---------------------------------------------

    private val profile = ClipFacts(
        storedWidth = 480,
        storedHeight = 480,
        rotation = 0,
        videoMime = "video/avc",
        audioMime = "audio/mp4a-latm",
    )

    @Test
    fun `exactly the profile's square is kept as recorded`() {
        assertThat(RoundRecorderRules.needsSquarePass(profile)).isFalse()
        // MIME types are compared without regard to case.
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(videoMime = "VIDEO/AVC"))).isFalse()
    }

    @Test
    fun `anything else takes the Media3 pass`() {
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(storedWidth = 640))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(storedHeight = 640))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(storedWidth = 720, storedHeight = 720))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(storedWidth = null))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(videoMime = "video/hevc"))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(audioMime = "audio/opus"))).isTrue()
        assertThat(RoundRecorderRules.needsSquarePass(profile.copy(audioMime = null))).isTrue()
    }

    /** A square hides its rotation from the size: it is read, and anything but 0 is baked in. */
    @Test
    fun `a rotated square takes the pass`() {
        for (rotation in listOf(90, 180, 270)) {
            assertThat(RoundRecorderRules.needsSquarePass(profile.copy(rotation = rotation))).isTrue()
        }
    }

    // --- S3.6: round or regular --------------------------------------------------

    private fun prepared(bytes: Int = 1_000, width: Int? = 480, height: Int? = 480, durationMs: Int? = 23_400) =
        MediaPrep.Prepared(
            file = folder.newFile().apply { writeBytes(ByteArray(bytes)) },
            mime = "video/mp4",
            kind = AttachmentDto.KIND_VIDEO,
            width = width,
            height = height,
            durationMs = durationMs,
            previewJpeg = null,
        )

    @Test
    fun `a square clip under the ceiling is a video message`() {
        assertThat(RoundRecorderRules.notRound(prepared(), squared = true, sizeBytes = 1_000, maxBytes = 12_582_912))
            .isNull()
        // Exactly the ceiling is still under it.
        assertThat(RoundRecorderRules.notRound(prepared(), squared = true, sizeBytes = 1_000, maxBytes = 1_000))
            .isNull()
    }

    @Test
    fun `a failed pass is a regular video`() {
        assertThat(RoundRecorderRules.notRound(prepared(), squared = false, sizeBytes = 1_000, maxBytes = 12_582_912))
            .isEqualTo(NotRound.COULDNT_MAKE_ROUND)
    }

    @Test
    fun `over the operator's ceiling is a regular video`() {
        assertThat(RoundRecorderRules.notRound(prepared(), squared = true, sizeBytes = 1_001, maxBytes = 1_000))
            .isEqualTo(NotRound.TOO_BIG)
    }

    /** A shape the server would refuse as a video message is never sent as one. */
    @Test
    fun `a shape the server refuses is a regular video`() {
        val ceiling = 12_582_912L
        assertThat(RoundRecorderRules.notRound(prepared(width = 640), true, 1_000, ceiling))
            .isEqualTo(NotRound.COULDNT_MAKE_ROUND)
        assertThat(RoundRecorderRules.notRound(prepared(width = 721, height = 721), true, 1_000, ceiling))
            .isEqualTo(NotRound.COULDNT_MAKE_ROUND)
        assertThat(RoundRecorderRules.notRound(prepared(durationMs = 60_001), true, 1_000, ceiling))
            .isEqualTo(NotRound.COULDNT_MAKE_ROUND)
        assertThat(RoundRecorderRules.notRound(prepared(durationMs = null), true, 1_000, ceiling))
            .isEqualTo(NotRound.COULDNT_MAKE_ROUND)
    }

    // --- S3.6: a black picture -----------------------------------------------------

    @Test
    fun `a black frame is near-black and a dark room is not`() {
        assertThat(RoundRecorderRules.nearBlack(0.0)).isTrue()
        assertThat(RoundRecorderRules.nearBlack(8.0)).isTrue()
        assertThat(RoundRecorderRules.nearBlack(8.5)).isFalse()
        assertThat(RoundRecorderRules.nearBlack(40.0)).isFalse()
    }

    @Test
    fun `luma is Rec 601`() {
        assertThat(RoundRecorderRules.luma(0xFF000000.toInt())).isWithin(1e-9).of(0.0)
        assertThat(RoundRecorderRules.luma(0xFFFFFFFF.toInt())).isWithin(1e-9).of(255.0)
        assertThat(RoundRecorderRules.luma(0xFFFF0000.toInt())).isWithin(1e-9).of(0.299 * 255)
        assertThat(RoundRecorderRules.luma(0xFF00FF00.toInt())).isWithin(1e-9).of(0.587 * 255)
        assertThat(RoundRecorderRules.luma(0xFF0000FF.toInt())).isWithin(1e-9).of(0.114 * 255)
    }
}

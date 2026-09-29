/*
 * MediaPrepDeviceTest.kt
 * Family Connect (Android)
 *
 * Issue #74 on a real encoder: what MediaPrep UPLOADS for a picked video or
 * sound file, checked against docs/protocol.md, "Preparing media before
 * upload" — by reading the result back with the platform's own readers, the
 * way another member's device will.
 *
 * The JVM tests prove the decision (MediaPlanVectorsTest) and what Media3 is
 * told (MediaTranscodeTest); only this proves Media3 then DOES it: the
 * turned size, the frame cap, H.264 and AAC, `moov` first, smaller — and
 * that rules A and C send the original byte for byte, and that nothing is
 * left behind in the upload cache, cancelled or not.
 *
 * Run on an emulator or a spare device (it installs and then UNINSTALLS the
 * app, taking its data with it):
 *   ANDROID_SERIAL=emulator-5554 ./gradlew -PnoBump \
 *     connectedStandardDebugAndroidTest \
 *     -Pandroid.testInstrumentationRunnerArguments.class=me.nettrash.familyconnect.data.repo.MediaPrepDeviceTest
 * CI has no emulator lane, so CI does not.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlin.random.Random
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class MediaPrepDeviceTest {

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private val mediaPrep = MediaPrep(context, context.contentResolver)
    private val inputs = File(context.cacheDir, "device-test")
    private val uploads = File(context.cacheDir, "uploads")

    @Before
    fun setUp() {
        inputs.deleteRecursively()
        inputs.mkdirs()
        uploads.deleteRecursively()
    }

    @After
    fun tearDown() {
        inputs.deleteRecursively()
        uploads.deleteRecursively()
    }

    /**
     * The fixture the reasoning document names first: a phone clip, turned,
     * faster than 30 fps and far over the profile's bitrate.
     */
    @Test
    fun aPortrait1080p60ClipBecomes720x1280At30WithH264AacAndMoovFirst(): Unit = runBlocking {
        val source = File(inputs, "portrait.mp4")
        TestClips.video(
            source, width = 1920, height = 1080, fps = 60, frames = 120,
            bitrate = 20_000_000, rotation = 90, audioChannels = 2,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/mp4", source.length())
        Log.i(TAG, "source: $read")

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4")

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_VIDEO)
        assertThat(prepared.mime).isEqualTo("video/mp4")
        // The TURNED size, as the bubble reserves it and rule 1 computes it.
        assertThat(prepared.width).isEqualTo(720)
        assertThat(prepared.height).isEqualTo(1280)
        assertThat(prepared.file.length()).isLessThan(source.length())
        assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()

        val out = tracks(prepared.file)
        assertThat(out.video?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_VIDEO_AVC)
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)

        // Rule 2: at most 30 — 120 frames over two seconds went in.
        val rate = frameRate(prepared.file)
        Log.i(TAG, "output: ${prepared.file.length()} bytes, $rate fps, video ${out.video}, audio ${out.audio}")
        assertThat(rate).isAtMost(MediaPlan.FRAME_RATE_TOLERANCE)
        assertThat(rate).isAtLeast(25.0)

        // Rule 3: about 2 Mbit/s for 720x1280 at 30, against the 20 the source was made at.
        val seconds = checkNotNull(prepared.durationMs) / 1000.0
        val bitsPerSecond = prepared.file.length() * 8 / seconds
        assertThat(bitsPerSecond).isLessThan(5_000_000.0)

        // "High profile (Main where an encoder offers nothing else)": High wherever the
        // device's H.264 encoder offers it, which DefaultEncoderFactory asks for.
        val profile = out.video?.let { if (it.containsKey(MediaFormat.KEY_PROFILE)) it.getInteger(MediaFormat.KEY_PROFILE) else null }
        Log.i(TAG, "H.264 profile in the output: $profile; encoder offers High: ${encoderOffersHigh()}")
        if (profile != null && encoderOffersHigh()) {
            assertThat(profile).isEqualTo(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
        }

        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /** Rule A: already H.264 in an MP4, at most 720 short, at most 30 fps, not over the rate. */
    @Test
    fun aClipAlreadyWithinTheProfileIsUploadedByteForByte(): Unit = runBlocking {
        val source = File(inputs, "small.mp4")
        TestClips.video(
            source, width = 640, height = 360, fps = 30, frames = 60,
            bitrate = 300_000, noise = false, audioChannels = null,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/mp4", source.length())
        Log.i(TAG, "source: $read, plan ${MediaPlan.planVideo(read)}")
        assertThat(MediaPlan.planVideo(read)).isEqualTo(MediaPlan.VideoPlan.Keep)

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4")

        assertThat(prepared.mime).isEqualTo("video/mp4")
        assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * What this platform's reader says about a clip with an AAC track — the
     * case the reference flagged: with no stated audio rate, V is unknown and
     * rule A cannot hold. Recorded, not asserted: it is a fact about the
     * platform, and the answer decides how often Android transcodes a clip
     * that was already fine.
     */
    @Test
    fun whatThePlatformStatesForAnAacTrackIsRecorded(): Unit = runBlocking {
        val source = File(inputs, "with-audio.mp4")
        TestClips.video(
            source, width = 640, height = 360, fps = 30, frames = 60,
            bitrate = 300_000, noise = false, audioChannels = 2,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/mp4", source.length())
        Log.i(TAG, "AAC track: stated audio ${read.audioBitrate}, stated video ${read.videoBitrate}, V ${MediaPlan.sourceVideoBitrate(read)}, plan ${MediaPlan.planVideo(read)}")
        assertThat(read.audioCodec).isEqualTo("aac")
    }

    @Test
    fun aPickedWavBecomesAnAacLcM4a(): Unit = runBlocking {
        val source = File(inputs, "tone.wav")
        TestClips.wav(source, seconds = 3, channels = 2)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/wav")

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.name).isEqualTo("tone.m4a")
        assertThat(MediaPrep.Magic.honest(prepared.file, "audio/mp4")).isTrue()
        assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()
        // 1 411 200 bit/s of PCM against at most 128 000 of AAC.
        assertThat(prepared.file.length()).isLessThan(source.length() / 5)
        assertThat(checkNotNull(prepared.durationMs)).isIn(2_800..3_200)

        val out = tracks(prepared.file)
        assertThat(out.video).isNull()
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)
        Log.i(TAG, "wav -> ${prepared.file.length()} bytes, ${out.audio}")
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /** A voice note is recorded to its row directly; the audio rules are for picked files. */
    @Test
    fun aVoiceNoteIsNeverReEncoded(): Unit = runBlocking {
        val source = File(inputs, "voice.wav")
        TestClips.wav(source, seconds = 1, channels = 1)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), voiceNote = true)

        assertThat(prepared.mime).isEqualTo("audio/wav")
        assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        prepared.file.delete()
    }

    /**
     * Rule C with a sendable source: nothing can read it (so there is no size
     * to transcode to), but it IS an honest MP4 within the ceiling — and 1.1
     * would have sent it untouched. So does this.
     */
    @Test
    fun aVideoNoReaderUnderstandsGoesAsTheOriginal(): Unit = runBlocking {
        val source = File(inputs, "opaque.mp4")
        source.writeBytes(
            byteArrayOf(0, 0, 0, 16) + "ftypisom".toByteArray() + ByteArray(4) +
                Random(7).nextBytes(4_096),
        )

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4")

        assertThat(prepared.mime).isEqualTo("video/mp4")
        assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * Rule C with a source that cannot go as it is: a type the server will
     * not take, which nothing can transcode either. 1.1's answer was the
     * "could not prepare" failure, and it still is — with nothing left over.
     */
    @Test
    fun anUnsendableVideoNothingCanReadFailsAsItDidIn11(): Unit = runBlocking {
        val source = File(inputs, "opaque.webm")
        source.writeBytes(Random(9).nextBytes(4_096))

        val failure = runCatching {
            mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/webm")
        }.exceptionOrNull()

        assertThat(failure).isInstanceOf(MediaPrep.UnreadableMedia::class.java)
        assertThat(leftovers(except = null)).isEmpty()
    }

    /** A cancelled prepare leaves nothing in the upload cache, whichever step it was in. */
    @Test
    fun aCancelledTranscodeLeavesNothingBehind(): Unit = runBlocking {
        val source = File(inputs, "long.mp4")
        TestClips.video(
            source, width = 1920, height = 1080, fps = 60, frames = 240,
            bitrate = 20_000_000, audioChannels = 2,
        )
        val scope = CoroutineScope(Dispatchers.Default)
        val prepare = scope.async { mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4") }
        // Mid-transcode, not "after a while": the copy and the transcoder's
        // output both exist. A fast encoder cannot finish between two polls.
        withTimeout(60_000) {
            while (uploads.listFiles().orEmpty().size < 2) delay(10)
        }
        prepare.cancel()
        val outcome = runCatching { prepare.await() }.exceptionOrNull()
        assertThat(outcome).isInstanceOf(CancellationException::class.java)

        // Transformer.cancel() is posted to the main Looper; give it its turn.
        delay(1_000)
        assertThat(leftovers(except = null)).isEmpty()
    }

    // -- Reading the result back ---------------------------------------------------

    private class Tracks(val video: MediaFormat?, val audio: MediaFormat?)

    private fun tracks(file: File): Tracks {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(file.absolutePath)
            var video: MediaFormat? = null
            var audio: MediaFormat? = null
            for (index in 0 until extractor.trackCount) {
                val format = extractor.getTrackFormat(index)
                val mime = format.getString(MediaFormat.KEY_MIME).orEmpty()
                if (mime.startsWith("video/")) video = format
                if (mime.startsWith("audio/")) audio = format
            }
            return Tracks(video, audio)
        } finally {
            extractor.release()
        }
    }

    /** Frames over the video track's duration — the rate a player will actually show. */
    private fun frameRate(file: File): Double {
        val retriever = MediaMetadataRetriever()
        try {
            retriever.setDataSource(file.absolutePath)
            val frames = checkNotNull(
                retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT),
            ).toLong()
            val durationUs = checkNotNull(tracks(file).video).getLong(MediaFormat.KEY_DURATION)
            return frames * 1_000_000.0 / durationUs
        } finally {
            retriever.release()
        }
    }

    private fun encoderOffersHigh(): Boolean =
        MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { it.isEncoder && MediaFormat.MIMETYPE_VIDEO_AVC in it.supportedTypes }
            .any { info ->
                info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC).profileLevels
                    .any { it.profile == MediaCodecInfo.CodecProfileLevel.AVCProfileHigh }
            }

    /** Whatever a prepare left in the upload cache besides what it returned. */
    private fun leftovers(except: File?): List<String> =
        uploads.listFiles().orEmpty().filter { it != except }.map { it.name }

    private companion object {
        const val TAG = "MediaPrepDeviceTest"
    }
}

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
 * Run on an emulator or a spare device, NEVER a phone somebody uses: the
 * Gradle task installs and then UNINSTALLS the app, taking its data with it.
 *   ANDROID_SERIAL=emulator-5554 ./gradlew -PnoBump \
 *     connectedStandardDebugAndroidTest \
 *     -Pandroid.testInstrumentationRunnerArguments.class=me.nettrash.familyconnect.data.repo.MediaPrepDeviceTest
 * or, touching exactly one device and uninstalling nothing, build
 * assembleStandardDebug and assembleStandardDebugAndroidTest with -PnoBump,
 * `adb -s <serial> install -r -t` both APKs, and
 *   adb -s <serial> shell am instrument -w \
 *     -e class me.nettrash.familyconnect.data.repo.MediaPrepDeviceTest \
 *     me.nettrash.familyconnect.test/androidx.test.runner.AndroidJUnitRunner
 * (an emulator started with `-read-only` forgets all of it when it stops).
 * CI has no emulator lane, so CI does not run this.
 *
 * The emulator's H.264 encoder is software (c2.android.avc.encoder) and
 * offers no High profile, so what a PHONE's hardware encoder does with the
 * same request — and HDR, which no synthesised clip here is — is not shown
 * by a green run there. Nor is a probe that cannot see a track Media3 can
 * (PCM or Opus audio in a .mov before Android 10): on a current system image
 * the platform's extractor lists everything, so that one is pinned on the
 * JVM, in MediaTranscodeTest, by what Media3 is told.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import android.util.Log
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.SdkSuppress
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
import org.junit.Assume.assumeTrue
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
            source, width = 1920, height = 1080, fps = 60, frames = 240,
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
        // The audio-in-a-video row: AAC-LC, at most 128 000 stereo.
        assertThat(out.audio?.intOrNull(MediaFormat.KEY_AAC_PROFILE))
            .isEqualTo(MediaCodecInfo.CodecProfileLevel.AACObjectLC)
        assertThat(checkNotNull(out.audio?.intOrNull(MediaFormat.KEY_BIT_RATE)))
            .isAtMost((MediaPlan.STEREO_AUDIO_BITRATE * 1.1).toInt())

        // Rule 2: at most 30 — 120 frames over two seconds went in.
        val rate = frameRate(prepared.file)
        Log.i(TAG, "output: ${prepared.file.length()} bytes, $rate fps, video ${out.video}, audio ${out.audio}")
        assertThat(rate).isAtMost(MediaPlan.FRAME_RATE_TOLERANCE)
        assertThat(rate).isAtLeast(25.0)

        // Rule 3: 2 Mbit/s for 720x1280 at 30, against the 20 the source was made at — and
        // it has to GOVERN, which is the thing a settings test cannot show. The margin is
        // rule A's own 1.25: a variable-rate encoder aims at an average, and one that
        // lands inside it has produced a file this client would leave alone.
        val target = (MediaPlan.MAX_VIDEO_BITRATE + MediaPlan.STEREO_AUDIO_BITRATE).toDouble()
        val seconds = checkNotNull(prepared.durationMs) / 1000.0
        val bitsPerSecond = prepared.file.length() * 8 / seconds
        assertThat(bitsPerSecond).isAtMost(target * 1.25)
        // Not a clip the encoder starved either: never raised is rule B, but a tenth of
        // the target would mean the request was not what set the rate.
        assertThat(bitsPerSecond).isAtLeast(target * 0.5)

        // And said the way the protocol says it: what comes out is within the profile,
        // so preparing it again would upload it as it is (rule A) — no second generation.
        val again = MediaProbe.video(context, Uri.fromFile(prepared.file), "video/mp4", prepared.file.length())
        Log.i(TAG, "output as a source: $again")
        assertThat(MediaPlan.planVideo(again)).isEqualTo(MediaPlan.VideoPlan.Keep)

        // "High profile (Main where an encoder offers nothing else)": High wherever an
        // H.264 encoder here offers it (DefaultEncoderFactory asks), Main where none
        // does and one offers that (TranscodeRecipe.h264Fallback asks) — and only a
        // device offering neither is left with its encoder's own default.
        val profile = out.video?.intOrNull(MediaFormat.KEY_PROFILE)
        val high = encoderOffers(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
        val main = encoderOffers(MediaCodecInfo.CodecProfileLevel.AVCProfileMain)
        Log.i(TAG, "H.264 profile in the output: $profile; encoder offers High: $high, Main: $main")
        when {
            high -> assertThat(profile).isEqualTo(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh)
            main -> assertThat(profile).isEqualTo(MediaCodecInfo.CodecProfileLevel.AVCProfileMain)
        }

        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * A transcode does not invent a track: the output's tracks are left for
     * Media3 to find in the source (so that one the probe cannot see is not
     * dropped), and what it finds in a silent clip is no audio — not a track
     * of silence, which is what naming "audio" for it would have produced.
     */
    @Test
    fun aSilentClipStaysSilentThroughATranscode(): Unit = runBlocking {
        val source = File(inputs, "silent.mp4")
        TestClips.video(
            source, width = 1280, height = 720, fps = 60, frames = 120,
            bitrate = 10_000_000, audioChannels = null,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/mp4", source.length())
        assertThat(read.audioCodec).isNull()
        assertThat(MediaPlan.planVideo(read)).isInstanceOf(MediaPlan.VideoPlan.Transcode::class.java)

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4")

        assertThat(prepared.mime).isEqualTo("video/mp4")
        assertThat(prepared.file.length()).isLessThan(source.length())
        val out = tracks(prepared.file)
        assertThat(out.video?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_VIDEO_AVC)
        assertThat(out.audio).isNull()
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * A source over the ceiling can never go as itself, so it is not copied:
     * it is read through the Uri it came by, and the transcode is the only
     * thing that can be sent — rule D's "otherwise the result is used".
     */
    @Test
    fun aSourceOverTheCeilingIsTranscodedFromWhereItIs(): Unit = runBlocking {
        val source = File(inputs, "big.mp4")
        TestClips.video(
            source, width = 1920, height = 1080, fps = 60, frames = 240,
            bitrate = 20_000_000, audioChannels = 2,
        )
        val limit = source.length() / 3
        Log.i(TAG, "over the ceiling: ${source.length()} bytes against $limit")

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), limit = limit, declaredMime = "video/mp4")

        assertThat(prepared.mime).isEqualTo("video/mp4")
        assertThat(prepared.file.length()).isAtMost(limit)
        assertThat(prepared.width).isEqualTo(1280)
        assertThat(prepared.height).isEqualTo(720)
        assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()
        assertThat(tracks(prepared.file).audio).isNotNull()
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * Rule A names no ceiling, so a clip within the profile but over it is
     * "kept" — and cannot go as it is. That is rule C's: 1.1's compress,
     * which for a clip already at the profile's rate has nothing to give
     * (its default rate is higher), so the honest end is the one 1.1 had,
     * "still too large". Either way nothing is left in the upload cache, and
     * a result that did fit is a finished MP4.
     */
    @Test
    fun aClipWithinTheProfileButOverTheCeilingGoesThroughThe11Compress(): Unit = runBlocking {
        val source = File(inputs, "kept.mp4")
        TestClips.video(
            source, width = 640, height = 360, fps = 30, frames = 90,
            bitrate = 300_000, grain = 0, audioChannels = null,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/mp4", source.length())
        assertThat(MediaPlan.planVideo(read)).isEqualTo(MediaPlan.VideoPlan.Keep)
        val limit = source.length() - 1

        val outcome = runCatching {
            mediaPrep.prepareVideo(Uri.fromFile(source), limit = limit, declaredMime = "video/mp4")
        }

        val prepared = outcome.getOrNull()
        Log.i(TAG, "kept but over the ceiling: ${prepared?.file?.length() ?: outcome.exceptionOrNull()}")
        if (prepared == null) {
            assertThat(outcome.exceptionOrNull()).isInstanceOf(MediaPrep.TooLargeAfterCompression::class.java)
            assertThat(leftovers(except = null)).isEmpty()
        } else {
            assertThat(prepared.mime).isEqualTo("video/mp4")
            assertThat(prepared.file.length()).isAtMost(limit)
            assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()
            assertThat(leftovers(except = prepared.file)).isEmpty()
        }
    }

    /**
     * Mp4Faststart on a file a real muxer wrote, not boxes built by hand: the
     * platform's MediaMuxer leaves its `moov` at the END once the index
     * outgrows the little it reserves, which is the layout Media3 falls back
     * to when ITS reservation is outgrown. Moved to the front, every sample
     * must still be the bytes it was.
     *
     * Skipped, not passed, on a release whose muxer wrote this clip
     * `moov`-first: there would be nothing to move.
     */
    @Test
    fun aMoovAtTheEndOfARealFileIsMovedAndEverySampleSurvives(): Unit = runBlocking {
        val file = File(inputs, "moov-last.mp4")
        TestClips.video(
            file, width = 320, height = 180, fps = 30, frames = 600,
            bitrate = 250_000, grain = 0, audioChannels = 2,
        )
        Log.i(TAG, "real file: ${file.length()} bytes, moov first: ${Mp4Faststart.moovFirst(file)}")
        assumeTrue("the platform's muxer wrote this clip moov-first", Mp4Faststart.moovFirst(file) == false)
        val before = samples(file)
        assertThat(before).isNotEmpty()

        assertThat(Mp4Faststart.apply(file)).isTrue()

        assertThat(Mp4Faststart.moovFirst(file)).isTrue()
        val after = samples(file)
        assertThat(after.size).isEqualTo(before.size)
        assertThat(after).isEqualTo(before)
        assertThat(inputs.list()!!.toList()).containsExactly(file.name)
    }

    /** Rule A: already H.264 in an MP4, at most 720 short, at most 30 fps, not over the rate. */
    @Test
    fun aClipAlreadyWithinTheProfileIsUploadedByteForByte(): Unit = runBlocking {
        val source = File(inputs, "small.mp4")
        TestClips.video(
            source, width = 640, height = 360, fps = 30, frames = 90,
            bitrate = 300_000, grain = 0, audioChannels = null,
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
     * Rule D, as the guarantee it is: whatever the encoder does with a clip
     * that is already tiny, the upload is never bigger than the source. A
     * clean 360p clip declared as QuickTime is outside rule A by its
     * container alone, so it IS transcoded — to its own bitrate, which an
     * encoder may or may not manage. Either the result is no bigger, or it
     * was thrown away and the original went as the QuickTime it is.
     */
    @Test
    fun aClipTheEncoderCannotShrinkIsNeverUploadedBigger(): Unit = runBlocking {
        val source = File(inputs, "tiny.mov")
        TestClips.video(
            source, width = 640, height = 360, fps = 30, frames = 90,
            bitrate = 250_000, grain = 0, audioChannels = null,
        )
        val read = MediaProbe.video(context, Uri.fromFile(source), "video/quicktime", source.length())
        assertThat(MediaPlan.planVideo(read)).isInstanceOf(MediaPlan.VideoPlan.Transcode::class.java)

        val prepared = mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/quicktime")

        Log.i(TAG, "tiny: ${source.length()} bytes in, ${prepared.file.length()} out as ${prepared.mime}")
        assertThat(prepared.file.length()).isAtMost(source.length())
        if (prepared.mime == "video/quicktime") {
            assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        } else {
            assertThat(prepared.mime).isEqualTo("video/mp4")
            assertThat(Mp4Faststart.moovFirst(prepared.file)).isTrue()
        }
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
            source, width = 640, height = 360, fps = 30, frames = 90,
            bitrate = 300_000, grain = 0, audioChannels = 2,
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
        // 1 411 200 bit/s of PCM against at most 128 000 of AAC: three seconds is 48 000
        // bytes and a small index. With Media3's unused `moov` reservation left in, this
        // was 449 093 — which is what this bound is here to keep out (Mp4Faststart).
        assertThat(prepared.file.length()).isLessThan(source.length() / 8)
        assertThat(checkNotNull(prepared.durationMs)).isIn(2_800..3_200)

        val out = tracks(prepared.file)
        assertThat(out.video).isNull()
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)
        Log.i(TAG, "wav -> ${prepared.file.length()} bytes, ${out.audio}")
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * The file the lossless rule saves the most on: a hi-res download at
     * 96 kHz. An AAC encoder lists nothing above 48, and asking for AAC-LC
     * by name switches off the step where Media3 fits the rate — so the
     * encoder was handed 96 kHz and left to it. This emulator's fell back to
     * 44.1 on its own; the rate is now the closest one the encoder SAYS it
     * takes, which is what the last assertion tells apart.
     */
    @Test
    @OptIn(UnstableApi::class)
    fun aHiResWavIsResampledToARateTheAacEncoderTakes(): Unit = runBlocking {
        val source = File(inputs, "hires.wav")
        TestClips.wav(source, seconds = 3, channels = 2, sampleRate = 96_000)
        val read = MediaProbe.audio(source, "audio/wav", source.length())
        Log.i(TAG, "hi-res source: $read")
        assertThat(read.codec).isEqualTo("pcm")

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/wav")

        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.name).isEqualTo("hires.m4a")
        assertThat(prepared.file.length()).isLessThan(source.length() / 8)
        assertThat(checkNotNull(prepared.durationMs)).isIn(2_800..3_200)
        val out = checkNotNull(tracks(prepared.file).audio)
        Log.i(TAG, "hi-res wav -> ${prepared.file.length()} bytes, $out")
        assertThat(out.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        val fitted = TranscodeRecipe.encoderSampleRate(MediaFormat.MIMETYPE_AUDIO_AAC, 96_000)
        Log.i(TAG, "the AAC encoder's closest rate to 96 000: $fitted")
        assertThat(out.getInteger(MediaFormat.KEY_SAMPLE_RATE)).isEqualTo(fitted)
        assertThat(out.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /** Surround is mixed down: the target is "128 000 bit/s stereo". */
    @Test
    fun aSurroundWavBecomesAStereoM4a(): Unit = runBlocking {
        val source = File(inputs, "surround.wav")
        TestClips.wav(source, seconds = 3, channels = 6, sampleRate = 48_000)
        val read = MediaProbe.audio(source, "audio/wav", source.length())
        Log.i(TAG, "surround source: $read, plan ${MediaPlan.planAudio(read)}")
        assertThat(read.channels).isEqualTo(6)
        assertThat(MediaPlan.planAudio(read))
            .isEqualTo(MediaPlan.AudioPlan.Transcode(MediaPlan.STEREO_AUDIO_BITRATE))

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/wav")

        assertThat(prepared.mime).isEqualTo("audio/mp4")
        val out = checkNotNull(tracks(prepared.file).audio)
        assertThat(out.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(2)
        assertThat(checkNotNull(prepared.durationMs)).isIn(2_800..3_200)
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * FLAC is not a type the server takes as audio at all: re-encoded, it
     * goes as an M4A every client plays, and the name says what it now is.
     *
     * The probe may call it "flac" or "pcm": the platform's FLAC reader
     * decodes as it reads on some releases (API 36's does) and hands over
     * raw audio. Both are lossless, and both are re-encoded.
     */
    @Test
    fun aPickedFlacBecomesAnM4a(): Unit = runBlocking {
        val source = File(inputs, "song.flac")
        TestClips.flac(source, seconds = 3)
        val read = MediaProbe.audio(source, "audio/flac", source.length())
        Log.i(TAG, "flac source: $read, plan ${MediaPlan.planAudio(read)}")
        assertThat(read.codec).isAnyOf("flac", "pcm")
        assertThat(MediaPlan.planAudio(read)).isInstanceOf(MediaPlan.AudioPlan.Transcode::class.java)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/flac")

        Log.i(TAG, "flac: ${source.length()} bytes in, ${prepared.file.length()} out as ${prepared.mime}")
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.name).isEqualTo("song.m4a")
        assertThat(MediaPrep.Magic.honest(prepared.file, "audio/mp4")).isTrue()
        assertThat(tracks(prepared.file).audio?.getString(MediaFormat.KEY_MIME))
            .isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * Rule C with a source that cannot go as audio: a FLAC the probe names
     * and nothing can decode. The re-encode fails, and it goes the way 1.1
     * sent every FLAC — as a file, untouched, under its own name.
     */
    @Test
    fun aFlacThatCannotBeReEncodedGoesAsTheFileItWasIn11(): Unit = runBlocking {
        val source = File(inputs, "broken.flac")
        TestClips.flac(source, seconds = 3, broken = true)
        val read = MediaProbe.audio(source, "audio/flac", source.length())
        Log.i(TAG, "broken flac source: $read, plan ${MediaPlan.planAudio(read)}")
        assertThat(MediaPlan.planAudio(read)).isInstanceOf(MediaPlan.AudioPlan.Transcode::class.java)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/flac")

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_FILE)
        assertThat(prepared.name).isEqualTo("broken.flac")
        assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * Rule C with a source that CAN go as audio: an Ogg by its first bytes
     * that nothing can decode. 1.1 sent it as the Ogg it says it is, and so
     * does this, byte for byte.
     */
    @Test
    fun anOggThatCannotBeReEncodedGoesAsTheOriginal(): Unit = runBlocking {
        val source = File(inputs, "noise.ogg")
        source.writeBytes("OggS".toByteArray() + Random(11).nextBytes(8_192))
        val read = MediaProbe.audio(source, "audio/ogg", source.length())
        assertThat(MediaPlan.planAudio(read)).isInstanceOf(MediaPlan.AudioPlan.Transcode::class.java)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/ogg")

        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/ogg")
        assertThat(prepared.name).isEqualTo("noise.ogg")
        assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * Rule D for picked audio, as the guarantee it is. A short Ogg already
     * under the target rate is re-encoded at its OWN rate (rule B), where AAC
     * in an MP4 is no smaller than Opus in an Ogg: whatever the encoder
     * makes of it, what is uploaded is never bigger than what was picked —
     * the M4A, or the Ogg exactly as it was.
     */
    @Test
    @SdkSuppress(minSdkVersion = Build.VERSION_CODES.Q) // MediaMuxer writes Ogg from API 29.
    fun aReEncodedSoundFileThatGrewLosesToItsSource(): Unit = runBlocking {
        val source = File(inputs, "short.ogg")
        TestClips.oggOpus(source, seconds = 3, bitrate = 32_000)

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/ogg")

        Log.i(TAG, "short ogg: ${source.length()} bytes in, ${prepared.file.length()} out as ${prepared.mime}")
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.file.length()).isAtMost(source.length())
        if (prepared.mime == "audio/ogg") {
            assertThat(prepared.name).isEqualTo("short.ogg")
            assertThat(prepared.file.readBytes()).isEqualTo(source.readBytes())
        } else {
            assertThat(prepared.mime).isEqualTo("audio/mp4")
            assertThat(prepared.name).isEqualTo("short.m4a")
        }
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * A WAV over the ceiling cannot go as it is, so rule D has no source to
     * prefer: the M4A is the only thing that can be sent, and it is.
     */
    @Test
    fun aWavOverTheCeilingIsSentAsItsReEncode(): Unit = runBlocking {
        val source = File(inputs, "long.wav")
        TestClips.wav(source, seconds = 3, channels = 2)
        val limit = source.length() / 4

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), limit = limit, declaredMime = "audio/wav")

        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.file.length()).isAtMost(limit)
        assertThat(leftovers(except = prepared.file)).isEmpty()
    }

    /**
     * "Ogg audio is re-encoded wherever the platform can decode it" — and
     * Android can: an Ogg file does not play on iOS or macOS, so it leaves
     * this client as an M4A, mono at mono's 64 kbit/s.
     *
     * At 96 kbit/s ON PURPOSE. Rule D is applied as written, to Ogg too: a
     * result bigger than a sendable source is thrown away, and `audio/ogg`
     * is sendable. An Ogg at or under the target rate is re-encoded at its
     * OWN rate (rule B), where AAC in an MP4 is no smaller than Opus in an
     * Ogg — a 32 kbit/s, three-second one came back as the Ogg it was. That
     * is the protocol's reading, reported as a problem with it rather than
     * worked around here.
     */
    @Test
    @SdkSuppress(minSdkVersion = Build.VERSION_CODES.Q) // MediaMuxer writes Ogg from API 29.
    fun aPickedOggOpusBecomesAnM4aEveryClientPlays(): Unit = runBlocking {
        val source = File(inputs, "memo.ogg")
        TestClips.oggOpus(source, seconds = 5, bitrate = 96_000)
        assertThat(MediaPrep.Magic.honest(source, "audio/ogg")).isTrue()
        val read = MediaProbe.audio(source, "audio/ogg", source.length())
        Log.i(TAG, "ogg source: $read, plan ${MediaPlan.planAudio(read)}")
        assertThat(read.codec).isEqualTo("opus")

        val prepared = mediaPrep.prepareAudio(Uri.fromFile(source), declaredMime = "audio/ogg")

        Log.i(TAG, "ogg: ${source.length()} bytes in, ${prepared.file.length()} out as ${prepared.mime}")
        assertThat(prepared.kind).isEqualTo(AttachmentDto.KIND_AUDIO)
        assertThat(prepared.mime).isEqualTo("audio/mp4")
        assertThat(prepared.name).isEqualTo("memo.m4a")
        assertThat(MediaPrep.Magic.honest(prepared.file, "audio/mp4")).isTrue()
        val out = tracks(prepared.file)
        assertThat(out.audio?.getString(MediaFormat.KEY_MIME)).isEqualTo(MediaFormat.MIMETYPE_AUDIO_AAC)
        assertThat(out.audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT)).isEqualTo(1)
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
        assertThat(source.length()).isLessThan(MediaPrep.SIZE_LIMIT)
        val scope = CoroutineScope(Dispatchers.Default)
        val prepare = scope.async { mediaPrep.prepareVideo(Uri.fromFile(source), declaredMime = "video/mp4") }
        // Mid-transcode, not "after a while": the copy and the transcoder's
        // output both exist. A fast encoder cannot finish between two polls.
        withTimeout(60_000) {
            while (uploads.listFiles().orEmpty().size < 2) {
                // Said at once rather than after a minute of waiting: a source
                // over the ceiling is never copied, so there would only ever
                // be one file — which is what the first fixture here was.
                check(!prepare.isCompleted) { "the prepare finished before it could be cancelled" }
                delay(10)
            }
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

    /** Every sample of every track, in file order: what a rewrite of the container must not change. */
    private fun samples(file: File): List<Triple<Int, Long, List<Byte>>> {
        val extractor = MediaExtractor()
        try {
            extractor.setDataSource(file.absolutePath)
            for (index in 0 until extractor.trackCount) extractor.selectTrack(index)
            val buffer = java.nio.ByteBuffer.allocate(1 shl 20)
            val all = mutableListOf<Triple<Int, Long, List<Byte>>>()
            while (true) {
                val size = extractor.readSampleData(buffer, 0)
                if (size < 0) break
                val bytes = ByteArray(size).also { buffer.position(0); buffer.get(it) }
                all += Triple(extractor.sampleTrackIndex, extractor.sampleTime, bytes.toList())
                extractor.advance()
            }
            return all
        } finally {
            extractor.release()
        }
    }

    private fun MediaFormat.intOrNull(key: String): Int? = if (containsKey(key)) getInteger(key) else null

    private fun encoderOffers(profile: Int): Boolean =
        MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
            .filter { it.isEncoder && MediaFormat.MIMETYPE_VIDEO_AVC in it.supportedTypes }
            .any { info ->
                info.getCapabilitiesForType(MediaFormat.MIMETYPE_VIDEO_AVC).profileLevels
                    .any { it.profile == profile }
            }

    /** Whatever a prepare left in the upload cache besides what it returned. */
    private fun leftovers(except: File?): List<String> =
        uploads.listFiles().orEmpty().filter { it != except }.map { it.name }

    private companion object {
        const val TAG = "MediaPrepDeviceTest"
    }
}

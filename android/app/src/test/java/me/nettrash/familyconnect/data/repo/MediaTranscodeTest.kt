/*
 * MediaTranscodeTest.kt
 * Family Connect (Android)
 *
 * What Media3 is TOLD for a planned transcode — the half of issue #74 a
 * vector cannot reach. MediaPlanVectorsTest proves the numbers are the
 * reference's; this proves they are the numbers the encoder actually gets:
 * the exact size (in the displayed orientation), frames dropped only when
 * the source is faster than the target, the two bitrates, AAC-LC by name
 * at a sample rate the encoder takes, a downmix for surround, HDR
 * tone-mapped, the encoder's frame-rate hint lowered to what it will be fed
 * — and that WHICH tracks come out is left to Media3, whose reader is the
 * one that has to find them.
 *
 * Robolectric only because Presentation and DefaultEncoderFactory want an
 * android.* around them; nothing here encodes a frame. That needs a device:
 * androidTest/…/MediaPrepDeviceTest.
 */

package me.nettrash.familyconnect.data.repo

import android.media.MediaCodecInfo
import android.media.metrics.LogSessionId
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.Format
import androidx.media3.common.MimeTypes
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.ChannelMixingAudioProcessor
import androidx.media3.common.util.Size
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.FrameDropEffect
import androidx.media3.effect.Presentation
import androidx.media3.transformer.Codec
import androidx.media3.transformer.Composition
import com.google.common.truth.Truth.assertThat
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@UnstableApi
@RunWith(RobolectricTestRunner::class)
class MediaTranscodeTest {

    /** What a phone shoots: 4K60, portrait (a landscape track turned 90°), HEVC in a .mov. */
    private val phone = MediaPlan.VideoSource(
        width = 2160,
        height = 3840,
        frameRate = 59.94,
        container = "video/quicktime",
        videoCodec = "hevc",
        audioCodec = "aac",
        audioChannels = 2,
        videoBitrate = 40_000_000,
        audioBitrate = 192_000,
        sizeBytes = 150_000_000,
        durationMs = 30_000,
    )

    private fun planned(source: MediaPlan.VideoSource): TranscodeSettings {
        val plan = MediaPlan.planVideo(source)
        check(plan is MediaPlan.VideoPlan.Transcode) { "expected a transcode, got $plan" }
        return TranscodeSettings.forVideo(plan.target, source)
    }

    // -- The settings -------------------------------------------------------------

    @Test
    fun `a 4K60 portrait clip is told 720x1280, 30 fps, 2 Mbit s, AAC 128k, tone-mapped`() {
        val settings = planned(phone)

        assertThat(settings.width).isEqualTo(720)
        assertThat(settings.height).isEqualTo(1280)
        assertThat(settings.dropFramesTo).isEqualTo(30f)
        assertThat(settings.encoderFrameRate).isEqualTo(30f)
        assertThat(settings.videoBitrate).isEqualTo(2_000_000)
        assertThat(settings.audioBitrate).isEqualTo(128_000)
        assertThat(settings.toneMapToSdr).isTrue()
        assertThat(settings.audioOnly).isFalse()
    }

    /**
     * Rule B for the frame rate: a source at or under 30.5 is not dropped at
     * all — 29.97 stays 29.97, and so does the encoder's budget for it.
     */
    @Test
    fun `a clip at 29,97 or 24 or 15 fps keeps every frame`() {
        for (rate in listOf(30_000.0 / 1001.0, 24.0, 15.0, 30.5)) {
            val settings = planned(phone.copy(frameRate = rate))
            assertThat(settings.dropFramesTo).isNull()
            assertThat(settings.encoderFrameRate).isEqualTo(rate.toFloat())
        }
    }

    /**
     * An unreadable rate is "30" to the planner. Dropping to 30 is then only
     * a ceiling: Media3's dropper never adds a frame, so a clip that was
     * really 15 fps stays 15.
     */
    @Test
    fun `a clip of unknown frame rate is capped at 30 rather than assumed`() {
        val settings = planned(phone.copy(frameRate = null))
        assertThat(settings.dropFramesTo).isEqualTo(30f)
        assertThat(settings.encoderFrameRate).isEqualTo(30f)
    }

    /**
     * The probe seeing no audio track is NOT the clip having none: before
     * Android 10 the platform's extractor does not list PCM, Opus, FLAC or
     * ALAC audio in an MP4 or a .mov, and Media3's does. Telling Media3
     * "video only" on the probe's word made it drop the track — a camera's
     * .mov, uploaded silent. So the tracks are Media3's to find, and the
     * settings still carry a rate for one it finds: the profile's ceiling.
     */
    @Test
    fun `a clip the probe found no audio in keeps whatever audio Media3 finds`() {
        val source = phone.copy(audioCodec = null, audioChannels = null, audioBitrate = null)
        val settings = planned(source)
        assertThat(settings.audioBitrate).isEqualTo(128_000)
        assertThat(TranscodeRecipe.audioEncoderSettings(settings).bitrate).isEqualTo(128_000)

        val item = TranscodeRecipe.editedMediaItem(Uri.parse("file:///camera.mov"), settings)
        assertThat(item.removeAudio).isFalse()
        val sequence = TranscodeRecipe.composition(item, settings).sequences.single()
        // TRACK_TYPE_NONE is Media3's "infer them from the item": nothing is
        // removed that the file has, and nothing invented that it lacks.
        assertThat(sequence.trackTypes).containsExactly(C.TRACK_TYPE_NONE)
    }

    /**
     * What arrives at the mixer is what the DECODER produced, whatever the
     * probe read: six channels become two, and one or two pass through — a
     * count with no matrix would fail the whole transcode, which is what a
     * decoder that had already mixed 5.1 down would have met when the mixer
     * only knew 3 to 6.
     */
    @Test
    fun `surround is mixed down to the stereo the target is, and mono and stereo are left alone`() {
        val surround = planned(phone.copy(audioChannels = 6, audioBitrate = 384_000))
        assertThat(surround.audioBitrate).isEqualTo(128_000)
        val mono = planned(phone.copy(audioChannels = 1, audioBitrate = 96_000))
        assertThat(mono.audioBitrate).isEqualTo(64_000)

        fun mixed(channels: Int): AudioProcessor.AudioFormat? {
            val mixer = TranscodeRecipe.audioProcessors().single()
            assertThat(mixer).isInstanceOf(ChannelMixingAudioProcessor::class.java)
            val out = mixer.configure(AudioProcessor.AudioFormat(48_000, channels, C.ENCODING_PCM_16BIT))
            mixer.flush(AudioProcessor.StreamMetadata.DEFAULT)
            return out.takeIf { mixer.isActive }
        }
        for (channels in 3..6) assertThat(mixed(channels)?.channelCount).isEqualTo(2)
        // Not active: the samples go to the encoder as they came.
        assertThat(mixed(1)).isNull()
        assertThat(mixed(2)).isNull()
        // No default mix past six: the transcode fails, and rule C sends the original.
        val failure = runCatching { mixed(8) }.exceptionOrNull()
        assertThat(failure).isInstanceOf(AudioProcessor.UnhandledAudioFormatException::class.java)
    }

    @Test
    fun `picked audio is told AAC at the planned rate, and nothing about video`() {
        val wav = MediaPlan.AudioSource("audio/wav", "pcm", 2, null, 31_752_000, 180_000)
        val plan = MediaPlan.planAudio(wav) as MediaPlan.AudioPlan.Transcode
        val settings = TranscodeSettings.forAudio(plan.bitrate)

        assertThat(settings.audioOnly).isTrue()
        assertThat(settings.audioBitrate).isEqualTo(128_000)
        assertThat(settings.videoBitrate).isNull()
        assertThat(settings.dropFramesTo).isNull()
        assertThat(TranscodeRecipe.videoEffects(settings)).isEmpty()

        val voice = MediaPlan.AudioSource("audio/ogg", "opus", 1, 32_000, 700_000, 175_000)
        val quiet = MediaPlan.planAudio(voice) as MediaPlan.AudioPlan.Transcode
        // Rule B: never above the source's, and mono's half of the profile.
        assertThat(TranscodeSettings.forAudio(quiet.bitrate).audioBitrate).isEqualTo(32_000)
    }

    // -- The Media3 objects built from them -------------------------------------------

    /**
     * The frames are dropped BEFORE they are scaled, and scaled to EXACTLY
     * the planner's size — including rule 1's drop to even, which a
     * scale-to-fit would pay for with a black bar.
     */
    @Test
    fun `the effects are the frame cap then the exact size`() {
        val effects = TranscodeRecipe.videoEffects(planned(phone))
        assertThat(effects).hasSize(2)
        assertThat(effects[0]).isInstanceOf(FrameDropEffect::class.java)
        val presentation = effects[1] as Presentation
        // Media3 hands effects the DISPLAYED frame, so the portrait source arrives as 2160x3840.
        assertThat(presentation.configure(2160, 3840)).isEqualTo(Size(720, 1280))

        // 2560x1080 is 1706.67 wide at 720: rule 1 says 1707, then 1706 to be even.
        val wide = planned(phone.copy(width = 2560, height = 1080, frameRate = 25.0))
        val scale = TranscodeRecipe.videoEffects(wide).single() as Presentation
        assertThat(scale.configure(2560, 1080)).isEqualTo(Size(1706, 720))
    }

    @Test
    fun `the composition tone-maps HDR and leaves the tracks to Media3`() {
        val settings = planned(phone)
        val item = TranscodeRecipe.editedMediaItem(Uri.parse("file:///clip.mov"), settings)
        val composition = TranscodeRecipe.composition(item, settings)

        assertThat(composition.hdrMode).isEqualTo(Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL)
        assertThat(composition.sequences.single().trackTypes).containsExactly(C.TRACK_TYPE_NONE)
        assertThat(item.removeVideo).isFalse()
        assertThat(item.removeAudio).isFalse()
        assertThat(item.effects.videoEffects).hasSize(2)

        // A picked sound file: its cover art is a picture track, and that one IS removed.
        val song = TranscodeSettings.forAudio(128_000)
        val songItem = TranscodeRecipe.editedMediaItem(Uri.parse("file:///song.flac"), song)
        assertThat(songItem.removeVideo).isTrue()
        assertThat(songItem.removeAudio).isFalse()
        assertThat(TranscodeRecipe.composition(songItem, song).hdrMode)
            .isEqualTo(Composition.HDR_MODE_KEEP_HDR)
    }

    @Test
    fun `the encoders are asked for the planned bitrates and AAC-LC`() {
        val settings = planned(phone.copy(videoBitrate = 900_000))
        // Rule B: V is under the profile's 2 000 000, so V it is — to the bit.
        assertThat(TranscodeRecipe.videoEncoderSettings(settings).bitrate).isEqualTo(900_000)
        val audio = TranscodeRecipe.audioEncoderSettings(settings)
        assertThat(audio.bitrate).isEqualTo(128_000)
        assertThat(audio.profile).isEqualTo(MediaCodecInfo.CodecProfileLevel.AACObjectLC)
    }

    /**
     * Asking for a bitrate is what makes Media3 RE-ENCODE a source it could
     * otherwise have copied: a 720p H.264 clip at 5 Mbit/s needs no rescale,
     * only a lower rate, and without this it would come out at 5 Mbit/s.
     */
    @Test
    fun `a requested bitrate forces the re-encode`() {
        val factory = TranscodeRecipe.encoderFactory(RuntimeEnvironment.getApplication(), planned(phone))
        assertThat(factory.videoNeedsEncoding()).isTrue()
        assertThat(factory.audioNeedsEncoding()).isTrue()
    }

    /**
     * "High profile (Main where an encoder offers nothing else)". High is
     * Media3's to ask for wherever any encoder offers it; where none does it
     * asks for nothing and the encoder's default is Baseline, so Main is
     * asked for here — at the highest level it is offered at.
     */
    @Test
    fun `Main is asked for only where no encoder offers High`() {
        val baseline = MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline
        val main = MediaCodecInfo.CodecProfileLevel.AVCProfileMain
        val high = MediaCodecInfo.CodecProfileLevel.AVCProfileHigh
        val level31 = MediaCodecInfo.CodecProfileLevel.AVCLevel31
        val level4 = MediaCodecInfo.CodecProfileLevel.AVCLevel4

        val software = listOf(baseline to level4, main to level31, main to level4)
        assertThat(TranscodeRecipe.h264Fallback(listOf(software))).isEqualTo(main to level4)
        // One encoder with High is enough to leave it to Media3.
        assertThat(TranscodeRecipe.h264Fallback(listOf(software, listOf(high to level4)))).isNull()
        // Neither High nor Main: the encoder's own default, as before.
        assertThat(TranscodeRecipe.h264Fallback(listOf(listOf(baseline to level4)))).isNull()
        assertThat(TranscodeRecipe.h264Fallback(emptyList())).isNull()

        val settings = planned(phone)
        val asked = TranscodeRecipe.videoEncoderSettings(settings, main to level4)
        assertThat(asked.profile).isEqualTo(main)
        assertThat(asked.level).isEqualTo(level4)
        assertThat(asked.bitrate).isEqualTo(2_000_000)
        assertThat(TranscodeRecipe.videoEncoderSettings(settings).profile)
            .isEqualTo(androidx.media3.transformer.VideoEncoderSettings.NO_VALUE)
    }

    // -- The sample rate -----------------------------------------------------------

    /**
     * A 96 kHz FLAC is the file the lossless rule saves the most on, and an
     * AAC encoder lists nothing above 48 kHz. Asking for AAC-LC by name
     * switches off the step where Media3 fits the rate itself, so the encoder
     * is asked for a rate it takes here — rather than left to do whatever it
     * does with one it does not.
     */
    @Test
    fun `the audio encoder is asked for a sample rate it takes`() {
        val recording = Recording()
        val asked = mutableListOf<Pair<String, Int>>()
        val factory = SampleRateEncoderFactory(recording) { mime, rate ->
            asked += mime to rate
            minOf(rate, 48_000)
        }
        fun aac(rate: Int) = Format.Builder()
            .setSampleMimeType(MimeTypes.AUDIO_AAC)
            .setSampleRate(rate)
            .setChannelCount(2)
            .build()

        for (rate in listOf(96_000, 88_200, 192_000)) {
            val fitted = factory.fitted(aac(rate))
            assertThat(fitted.sampleRate).isEqualTo(48_000)
            assertThat(fitted.channelCount).isEqualTo(2)
        }
        // What nearly every file is: not rebuilt, let alone resampled.
        val cd = aac(44_100)
        assertThat(factory.fitted(cd)).isSameInstanceAs(cd)
        assertThat(asked).contains(MimeTypes.AUDIO_AAC to 96_000)
        // Nothing to fit: no rate stated.
        val unstated = Format.Builder().setSampleMimeType(MimeTypes.AUDIO_AAC).build()
        assertThat(factory.fitted(unstated)).isSameInstanceAs(unstated)

        // And it is the fitted format the real factory is handed — video untouched.
        runCatching { factory.createForAudioEncoding(aac(96_000), null) }
        assertThat(recording.audio.single().sampleRate).isEqualTo(48_000)
        val picture = video(60f)
        runCatching { factory.createForVideoEncoding(picture, null) }
        assertThat(recording.video.single()).isSameInstanceAs(picture)
    }

    /** With no encoder to ask — Robolectric has none — the rate is left as it came. */
    @Test
    fun `a platform that will not say what its encoder takes changes nothing`() {
        assertThat(TranscodeRecipe.encoderSampleRate(MimeTypes.AUDIO_AAC, 96_000)).isEqualTo(96_000)
    }

    // -- The frame-rate hint -------------------------------------------------------

    @Test
    fun `the encoder is told the rate it will be fed, never a higher one`() {
        val hint = FrameRateHintEncoderFactory(Recording(), 30f)
        fun rate(of: Float) = hint.hinted(video(of)).frameRate

        assertThat(rate(59.94f)).isEqualTo(30f)
        assertThat(rate(240f)).isEqualTo(30f)
        assertThat(rate(Format.NO_VALUE.toFloat())).isEqualTo(30f)
        assertThat(rate(24f)).isEqualTo(24f)
        assertThat(rate(30f)).isEqualTo(30f)
        assertThat(FrameRateHintEncoderFactory(Recording(), null).hinted(video(60f)).frameRate)
            .isEqualTo(60f)
    }

    @Test
    fun `the hint reaches the video encoder and nothing else changes`() {
        val recording = Recording(videoNeeds = true, audioNeeds = false)
        val hint = FrameRateHintEncoderFactory(recording, 30f)

        // The recorder refuses to build a codec; only what it was ASKED for matters.
        runCatching { hint.createForVideoEncoding(video(60f), null) }
        val audio = Format.Builder().setSampleMimeType(MimeTypes.AUDIO_AAC).setChannelCount(2).build()
        runCatching { hint.createForAudioEncoding(audio, null) }

        assertThat(recording.video.single().frameRate).isEqualTo(30f)
        assertThat(recording.video.single().width).isEqualTo(1280)
        assertThat(recording.audio.single()).isSameInstanceAs(audio)
        assertThat(hint.videoNeedsEncoding()).isTrue()
        assertThat(hint.audioNeedsEncoding()).isFalse()
    }

    // -- The sound a transcript request supplies (docs/protocol.md, "Transcripts on request") --

    /**
     * RE-ENCODE: 64 kbit/s AAC-LC, mono, and the video track removed before
     * anything reads it — not one frame is decoded.
     */
    @Test
    fun `transcript sound is re-encoded to 64 kbit s mono AAC-LC with no picture`() {
        val settings = TranscodeSettings.forTranscriptSound()
        assertThat(settings.audioOnly).isTrue()
        assertThat(settings.audioBitrate).isEqualTo(64_000)
        assertThat(settings.maxAudioChannels).isEqualTo(1)
        assertThat(TranscodeRecipe.videoEffects(settings)).isEmpty()
        val audio = TranscodeRecipe.audioEncoderSettings(settings)
        assertThat(audio.bitrate).isEqualTo(64_000)
        assertThat(audio.profile).isEqualTo(MediaCodecInfo.CodecProfileLevel.AACObjectLC)

        val item = TranscodeRecipe.editedMediaItem(Uri.parse("file:///clip.mp4"), settings)
        assertThat(item.removeVideo).isTrue()
        assertThat(item.effects.videoEffects).isEmpty()

        // Every count the decoder may produce comes out as ONE channel —
        // mono by being left alone (an identity mix is not active).
        val mixer = item.effects.audioProcessors.single()
        fun mixed(channels: Int): AudioProcessor.AudioFormat? {
            val out = mixer.configure(AudioProcessor.AudioFormat(48_000, channels, C.ENCODING_PCM_16BIT))
            mixer.flush(AudioProcessor.StreamMetadata.DEFAULT)
            return out.takeIf { mixer.isActive }
        }
        for (channels in 2..6) assertThat(mixed(channels)?.channelCount).isEqualTo(1)
        assertThat(mixed(1)).isNull()
        // And the upload profile still keeps stereo: the default is unchanged.
        assertThat(TranscodeSettings.forAudio(128_000).maxAudioChannels).isEqualTo(2)
    }

    /**
     * PASSTHROUGH: no effect and no encoder settings, so Transformer copies
     * the AAC samples (TransformerUtil.shouldTranscodeAudio): nothing it is
     * told would make it decode the sound, and the picture is removed.
     */
    @Test
    fun `transcript sound passthrough removes the picture and asks for nothing that re-encodes`() {
        val item = TranscodeRecipe.soundPassthroughItem(Uri.parse("file:///clip.mp4"))
        assertThat(item.removeVideo).isTrue()
        assertThat(item.removeAudio).isFalse()
        assertThat(item.effects.audioProcessors).isEmpty()
        assertThat(item.effects.videoEffects).isEmpty()
    }

    private fun video(frameRate: Float): Format =
        Format.Builder()
            .setSampleMimeType(MimeTypes.VIDEO_H264)
            .setWidth(1280)
            .setHeight(720)
            .setFrameRate(frameRate)
            .build()

    /** An encoder factory that only remembers what it was asked for. */
    private class Recording(
        private val videoNeeds: Boolean = false,
        private val audioNeeds: Boolean = false,
    ) : Codec.EncoderFactory {
        val video = mutableListOf<Format>()
        val audio = mutableListOf<Format>()

        override fun createForAudioEncoding(format: Format, logSessionId: LogSessionId?): Codec {
            audio += format
            throw UnsupportedOperationException("recorded")
        }

        override fun createForVideoEncoding(format: Format, logSessionId: LogSessionId?): Codec {
            video += format
            throw UnsupportedOperationException("recorded")
        }

        override fun videoNeedsEncoding() = videoNeeds

        override fun audioNeedsEncoding() = audioNeeds
    }

    // -- Video messages (#79, S8.4) ------------------------------------------------

    /**
     * A video message that came out of the camera not exactly as the profile
     * asks is CROPPED to the centre square — a 640 × 480 frame comes out
     * 480 × 480, never stretched into it — at the profile's rates.
     */
    @Test
    fun `a video message's pass crops the centre square at the profile's rates`() {
        val settings = TranscodeSettings.forRoundVideo(edge = 480, videoBitrate = 500_000, audioBitrate = 64_000, frameRate = 30)
        assertThat(settings.width).isEqualTo(480)
        assertThat(settings.height).isEqualTo(480)
        assertThat(settings.cropToFill).isTrue()
        assertThat(settings.videoBitrate).isEqualTo(500_000)
        assertThat(settings.audioBitrate).isEqualTo(64_000)
        assertThat(settings.encoderFrameRate).isEqualTo(30f)
        assertThat(settings.toneMapToSdr).isTrue()
        assertThat(settings.audioOnly).isFalse()

        // AAC MONO, as iOS, the web and Windows write it: a stereo (or any)
        // CameraX track the pass re-encodes comes out as ONE channel.
        assertThat(settings.maxAudioChannels).isEqualTo(1)
        val mixer = TranscodeRecipe.editedMediaItem(Uri.parse("file:///round.mp4"), settings)
            .effects.audioProcessors.single()
        fun mixed(channels: Int): AudioProcessor.AudioFormat? {
            val out = mixer.configure(AudioProcessor.AudioFormat(48_000, channels, C.ENCODING_PCM_16BIT))
            mixer.flush(AudioProcessor.StreamMetadata.DEFAULT)
            return out.takeIf { mixer.isActive }
        }
        for (channels in 2..6) assertThat(mixed(channels)?.channelCount).isEqualTo(1)
        assertThat(mixed(1)).isNull()

        val square = TranscodeRecipe.videoEffects(settings).last() as Presentation
        assertThat(square.configure(640, 480)).isEqualTo(Size(480, 480))
        // CROPPED, not stretched: the 4:3 frame is scaled past the square's
        // sides by its own aspect, and what spills over is cut — a stretch
        // would leave the matrix at 1 and squash every face.
        val crop = FloatArray(9).also { square.getMatrix(0).getValues(it) }
        assertThat(crop[0]).isWithin(1e-4f).of(640f / 480f)
        assertThat(crop[4]).isWithin(1e-4f).of(1f)
        // Upright already (the decoder applied the rotation): a portrait frame crops the same way.
        assertThat(square.configure(480, 640)).isEqualTo(Size(480, 480))
        val portrait = FloatArray(9).also { square.getMatrix(0).getValues(it) }
        assertThat(portrait[0]).isWithin(1e-4f).of(1f)
        assertThat(portrait[4]).isWithin(1e-4f).of(640f / 480f)
    }

    /** Everything else keeps stretching to the planner's exact size, as before. */
    @Test
    fun `only a video message crops`() {
        assertThat(planned(phone).cropToFill).isFalse()
    }
}

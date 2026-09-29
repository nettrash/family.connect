/*
 * MediaTranscodeTest.kt
 * Family Connect (Android)
 *
 * What Media3 is TOLD for a planned transcode — the half of issue #74 a
 * vector cannot reach. MediaPlanVectorsTest proves the numbers are the
 * reference's; this proves they are the numbers the encoder actually gets:
 * the exact size (in the displayed orientation), frames dropped only when
 * the source is faster than the target, the two bitrates, AAC-LC by name,
 * a downmix for surround, HDR tone-mapped, and the encoder's frame-rate
 * hint lowered to what it will be fed.
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
        assertThat(settings.hasAudio).isTrue()
        assertThat(settings.downmixFrom).isNull()
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

    @Test
    fun `a silent clip gets no audio track and no audio bitrate`() {
        val settings = planned(phone.copy(audioCodec = null, audioChannels = null, audioBitrate = null))
        assertThat(settings.hasAudio).isFalse()
        assertThat(settings.audioBitrate).isNull()
        assertThat(TranscodeRecipe.trackTypes(settings)).containsExactly(C.TRACK_TYPE_VIDEO)
    }

    @Test
    fun `surround is mixed down to the stereo the target is`() {
        val surround = planned(phone.copy(audioChannels = 6, audioBitrate = 384_000))
        assertThat(surround.downmixFrom).isEqualTo(6)
        assertThat(surround.audioBitrate).isEqualTo(128_000)
        assertThat(TranscodeRecipe.audioProcessors(surround).single())
            .isInstanceOf(ChannelMixingAudioProcessor::class.java)

        val mono = planned(phone.copy(audioChannels = 1, audioBitrate = 96_000))
        assertThat(mono.downmixFrom).isNull()
        assertThat(mono.audioBitrate).isEqualTo(64_000)
        assertThat(TranscodeRecipe.audioProcessors(mono)).isEmpty()
    }

    @Test
    fun `picked audio is told AAC at the planned rate, and nothing about video`() {
        val wav = MediaPlan.AudioSource("audio/wav", "pcm", 2, null, 31_752_000, 180_000)
        val plan = MediaPlan.planAudio(wav) as MediaPlan.AudioPlan.Transcode
        val settings = TranscodeSettings.forAudio(plan.bitrate, wav)

        assertThat(settings.audioOnly).isTrue()
        assertThat(settings.audioBitrate).isEqualTo(128_000)
        assertThat(settings.videoBitrate).isNull()
        assertThat(settings.dropFramesTo).isNull()
        assertThat(TranscodeRecipe.trackTypes(settings)).containsExactly(C.TRACK_TYPE_AUDIO)
        assertThat(TranscodeRecipe.videoEffects(settings)).isEmpty()

        val voice = MediaPlan.AudioSource("audio/ogg", "opus", 1, 32_000, 700_000, 175_000)
        val quiet = MediaPlan.planAudio(voice) as MediaPlan.AudioPlan.Transcode
        // Rule B: never above the source's, and mono's half of the profile.
        assertThat(TranscodeSettings.forAudio(quiet.bitrate, voice).audioBitrate).isEqualTo(32_000)
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
    fun `the composition tone-maps HDR and carries exactly the source's tracks`() {
        val settings = planned(phone)
        val item = TranscodeRecipe.editedMediaItem(Uri.parse("file:///clip.mov"), settings)
        val composition = TranscodeRecipe.composition(item, settings)

        assertThat(composition.hdrMode).isEqualTo(Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL)
        assertThat(composition.sequences.single().trackTypes)
            .containsExactly(C.TRACK_TYPE_VIDEO, C.TRACK_TYPE_AUDIO)
        assertThat(item.removeVideo).isFalse()
        assertThat(item.effects.videoEffects).hasSize(2)

        val song = TranscodeSettings.forAudio(
            128_000,
            MediaPlan.AudioSource("audio/flac", "flac", 2, null, 30_000_000, 180_000),
        )
        val songItem = TranscodeRecipe.editedMediaItem(Uri.parse("file:///song.flac"), song)
        assertThat(songItem.removeVideo).isTrue()
        assertThat(TranscodeRecipe.composition(songItem, song).sequences.single().trackTypes)
            .containsExactly(C.TRACK_TYPE_AUDIO)
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
}

/*
 * MediaTranscode.kt
 * Family Connect (Android)
 *
 * What Media3's Transformer is TOLD for a planned transcode
 * (docs/protocol.md, "Preparing media before upload").
 *
 * Two layers, so the part that decides can be tested without a device:
 *
 *  - [TranscodeSettings] is plain data, decided from MediaPlan's target and
 *    the source it was planned from: the exact size, whether frames are
 *    dropped and to what, the two bitrates, a downmix, tone-mapping.
 *    MediaTranscodeTest pins it on the JVM.
 *  - [TranscodeRecipe] turns those settings into Media3 objects — effects,
 *    encoder settings, the composition, the Transformer — and nothing else.
 *
 * THE THINGS MEDIA3 DOES BY DEFAULT THAT THE PROFILE DOES NOT WANT, each
 * overridden below for a reason found in Media3 1.11's own source:
 *
 *  - HDR: the default HDR_MODE_KEEP_HDR, given HDR input and an H.264
 *    request, quietly switches the OUTPUT to HEVC when the device has an
 *    HEVC HDR encoder (TransformerUtil.getOutputMimeTypeAndHdrModeAfterFallback)
 *    — which is what 1.1's compress did to an HDR clip on a Pixel, and HEVC
 *    does not play in Firefox. The profile is "8-bit SDR — HDR is
 *    tone-mapped", so the tone-map is asked for.
 *  - Transmuxing: with no effect that changes anything, Transformer copies
 *    the samples untouched, bitrate and all. Here a requested bitrate
 *    (DefaultEncoderFactory.videoNeedsEncoding) forces the re-encode — which
 *    is the point: a 720p H.264 clip at 5 Mbit/s needs no rescale, only a
 *    lower rate.
 *  - The encoder's frame-rate hint: Transformer passes the SOURCE's rate to
 *    the encoder even when frames are dropped on the way, and an encoder that
 *    budgets bits per frame from that hint would spend half the target on a
 *    60 → 30 clip. [FrameRateHintEncoderFactory] tells it the rate it will
 *    actually be fed.
 *
 * The H.264 PROFILE is left to DefaultEncoderFactory, which already does
 * what the protocol row asks: High, at the highest level the encoder
 * supports, on every API this app runs on (26+), and the encoder's own
 * default where it offers no High (adjustMediaFormatForH264EncoderSettings).
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.media.MediaCodecInfo
import android.media.metrics.LogSessionId
import android.net.Uri
import androidx.media3.common.C
import androidx.media3.common.Effect
import androidx.media3.common.Format
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.ChannelMixingAudioProcessor
import androidx.media3.common.audio.ChannelMixingMatrix
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.FrameDropEffect
import androidx.media3.effect.Presentation
import androidx.media3.transformer.AudioEncoderSettings
import androidx.media3.transformer.Codec
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.EditedMediaItemSequence
import androidx.media3.transformer.Effects
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings

/** Everything a planned transcode is told, decided before any Media3 object exists. */
data class TranscodeSettings(
    /**
     * The exact output size, in the DISPLAYED orientation — Media3's decoder
     * applies the source's rotation before any effect sees a frame
     * (VideoEncoderGraphInput.applyDecoderRotation). Null for sound alone.
     */
    val width: Int?,
    val height: Int?,
    /**
     * Drop frames down to this rate, or null to keep every one. Only ever
     * set when the source is faster than the target or its rate is unknown:
     * the dropper never adds a frame, so a 15 fps clip "capped" at 30 stays
     * 15 — rule B — but asking a 24 fps clip to be dropped to 24 would only
     * risk losing frames to timestamp jitter.
     */
    val dropFramesTo: Float?,
    /** The frame rate the video encoder budgets its bits for. */
    val encoderFrameRate: Float?,
    val videoBitrate: Int?,
    val audioBitrate: Int?,
    /**
     * The channel count of a source with more than two (3–6) — mixed down to
     * stereo, because the target is "128 000 bit/s stereo" and 5.1 AAC-LC at
     * 128 kbit/s is 21 kbit/s a channel. Null keeps the source's channels:
     * mono stays mono (and 64 kbit/s), stereo stays stereo.
     */
    val downmixFrom: Int?,
    /**
     * Whether the source has an audio track to carry over. The output has
     * exactly the tracks the source had — a transcode does not invent
     * silence (MediaPlan: no audio track, no audio target).
     */
    val hasAudio: Boolean,
    /** HDR in, 8-bit SDR out. */
    val toneMapToSdr: Boolean,
) {
    /** A picked sound file: no video track goes into the output. */
    val audioOnly: Boolean get() = width == null || height == null

    companion object {
        /** A video, from MediaPlan's [target] for [source]. */
        fun forVideo(target: MediaPlan.VideoTarget, source: MediaPlan.VideoSource): TranscodeSettings {
            val known = MediaPlan.knownFrameRate(source.frameRate)
            val rate = target.frameRate.toFloat()
            return TranscodeSettings(
                width = target.width.toInt(),
                height = target.height.toInt(),
                dropFramesTo = rate.takeIf { known == null || known > target.frameRate },
                encoderFrameRate = rate,
                videoBitrate = target.videoBitrate.toInt(),
                audioBitrate = target.audioBitrate?.toInt(),
                downmixFrom = source.audioCodec?.let { downmix(source.audioChannels) },
                hasAudio = source.audioCodec != null,
                toneMapToSdr = true,
            )
        }

        /** A picked sound file, re-encoded at MediaPlan's [bitrate]. */
        fun forAudio(bitrate: Long, source: MediaPlan.AudioSource): TranscodeSettings =
            TranscodeSettings(
                width = null,
                height = null,
                dropFramesTo = null,
                encoderFrameRate = null,
                videoBitrate = null,
                audioBitrate = bitrate.toInt(),
                downmixFrom = downmix(source.channels),
                hasAudio = true,
                toneMapToSdr = false,
            )

        /** Media3 has constant-power stereo mixes for 3 to 6 channels, and none past that. */
        private fun downmix(channels: Long?): Int? = channels?.takeIf { it in 3..6 }?.toInt()
    }
}

/** [TranscodeSettings] as Media3 objects. Building them decides nothing. */
@UnstableApi
object TranscodeRecipe {

    /** Frames dropped first — no point scaling one that is then thrown away — then the exact size. */
    fun videoEffects(settings: TranscodeSettings): List<Effect> = buildList {
        settings.dropFramesTo?.let { add(FrameDropEffect.createDefaultFrameDropEffect(it)) }
        if (settings.width != null && settings.height != null) {
            // STRETCH, not SCALE_TO_FIT: MediaPlan's size keeps the aspect to
            // within the one pixel rule 1 dropped to make a side even, and
            // fitting would pay for that pixel with a black bar.
            add(
                Presentation.createForWidthAndHeight(
                    settings.width,
                    settings.height,
                    Presentation.LAYOUT_STRETCH_TO_FIT,
                ),
            )
        }
    }

    fun audioProcessors(settings: TranscodeSettings): List<AudioProcessor> = buildList {
        if (settings.downmixFrom == null) return@buildList
        add(
            ChannelMixingAudioProcessor().apply {
                // Registered for every count the mix covers, not only the one
                // probed, because the decoder is what finally says how many
                // channels arrive — and an input with no matrix is an error.
                for (channels in 3..6) {
                    putChannelMixingMatrix(ChannelMixingMatrix.createForConstantPower(channels, 2))
                }
            },
        )
    }

    fun editedMediaItem(input: Uri, settings: TranscodeSettings): EditedMediaItem =
        EditedMediaItem.Builder(MediaItem.fromUri(input))
            // Cover art an audio file carries as a picture track is not sound.
            .setRemoveVideo(settings.audioOnly)
            .setEffects(Effects(audioProcessors(settings), videoEffects(settings)))
            .build()

    /**
     * The tracks the output has, named rather than left to Media3 to infer:
     * its "infer them from the one item" sequence is package-private in 1.11,
     * and the probe already knows what the source holds.
     */
    fun trackTypes(settings: TranscodeSettings): Set<Int> = buildSet {
        if (!settings.audioOnly) add(C.TRACK_TYPE_VIDEO)
        if (settings.hasAudio) add(C.TRACK_TYPE_AUDIO)
    }

    fun composition(item: EditedMediaItem, settings: TranscodeSettings): Composition =
        Composition.Builder(
            EditedMediaItemSequence.Builder(trackTypes(settings)).addItem(item).build(),
        )
            .setHdrMode(
                if (settings.toneMapToSdr) {
                    Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL
                } else {
                    Composition.HDR_MODE_KEEP_HDR
                },
            )
            .build()

    fun videoEncoderSettings(settings: TranscodeSettings): VideoEncoderSettings =
        VideoEncoderSettings.Builder()
            .apply { settings.videoBitrate?.let { setBitrate(it) } }
            .build()

    /** AAC-LC by name — the profile row says LC, and HE-AAC is not what every client decodes alike. */
    fun audioEncoderSettings(settings: TranscodeSettings): AudioEncoderSettings =
        AudioEncoderSettings.Builder()
            .setProfile(MediaCodecInfo.CodecProfileLevel.AACObjectLC)
            .apply { settings.audioBitrate?.let { setBitrate(it) } }
            .build()

    fun encoderFactory(context: Context, settings: TranscodeSettings): Codec.EncoderFactory =
        FrameRateHintEncoderFactory(
            DefaultEncoderFactory.Builder(context)
                .setRequestedVideoEncoderSettings(videoEncoderSettings(settings))
                .setRequestedAudioEncoderSettings(audioEncoderSettings(settings))
                .build(),
            settings.encoderFrameRate,
        )

    /**
     * H.264 and AAC, into Media3's default muxer — an MP4 that TRIES to put
     * its `moov` first (Mp4Faststart finishes the job when it could not).
     * Built on, and calling back on, the thread that builds it.
     */
    fun transformer(
        context: Context,
        settings: TranscodeSettings,
        listener: Transformer.Listener,
    ): Transformer =
        Transformer.Builder(context)
            .setVideoMimeType(MimeTypes.VIDEO_H264)
            .setAudioMimeType(MimeTypes.AUDIO_AAC)
            .setEncoderFactory(encoderFactory(context, settings))
            .addListener(listener)
            .build()
}

/**
 * Tells the video encoder the frame rate it will actually be FED.
 *
 * Transformer hands the encoder the source's rate (VideoEncoderWrapper sets
 * `inputFormat.frameRate`) even when a FrameDropEffect halves it on the way.
 * The encoder's rate control is only as good as that hint on devices that
 * budget per frame, so a 60 fps clip dropped to 30 would come out at half
 * its bitrate — smaller, and worse than the profile. Everything else is the
 * delegate's, unchanged.
 */
@UnstableApi
class FrameRateHintEncoderFactory(
    private val delegate: Codec.EncoderFactory,
    private val frameRate: Float?,
) : Codec.EncoderFactory {

    override fun createForAudioEncoding(format: Format, logSessionId: LogSessionId?): Codec =
        delegate.createForAudioEncoding(format, logSessionId)

    override fun createForVideoEncoding(format: Format, logSessionId: LogSessionId?): Codec =
        delegate.createForVideoEncoding(hinted(format), logSessionId)

    override fun audioNeedsEncoding(): Boolean = delegate.audioNeedsEncoding()

    override fun videoNeedsEncoding(): Boolean = delegate.videoNeedsEncoding()

    /** Lowered to [frameRate] when the format says more, or nothing; never raised. */
    fun hinted(format: Format): Format {
        val cap = frameRate ?: return format
        val stated = format.frameRate
        if (stated != Format.NO_VALUE.toFloat() && stated <= cap) return format
        return format.buildUpon().setFrameRate(cap).build()
    }
}

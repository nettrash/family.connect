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
 *    dropped and to what, the two bitrates, tone-mapping.
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
 *  - The AAC encoder's sample rate: asking for AAC-LC by name switches OFF
 *    the one step in DefaultEncoderFactory that fits the source's rate to
 *    what the encoder takes (createForAudioEncoding skips its fallback once
 *    an encoder offering the profile is found), so for a 96 kHz FLAC — the
 *    largest lossless files there are — the encoder is configured at
 *    96 kHz, a rate Android's AAC encoders do not list. What happens next
 *    is then the encoder's to decide. [SampleRateEncoderFactory] does that
 *    step itself.
 *
 * AND ONE THING IT DOES BY DEFAULT THAT IS KEPT: which TRACKS the output has
 * is Media3's to say, not the probe's. Media3 reads the file with its own
 * extractor; MediaProbe reads it with the platform's, which before Android
 * 10 does not list PCM, Opus, FLAC or ALAC audio in an MP4 or a .mov at all.
 * Naming the tracks from the probe made Media3 DROP an audio track it could
 * read perfectly well — a camera's .mov came out silent, and smaller, so
 * rule D kept it. A probe that sees no audio track is therefore only a probe
 * that saw none: the settings still say what a track is encoded at if there
 * turns out to be one.
 *
 * The H.264 PROFILE is DefaultEncoderFactory's wherever an encoder offers
 * High: it asks for High at the highest level that encoder supports, on
 * every API this app runs on (26+). Where NO encoder offers High it sets
 * nothing, and the encoder's own default is Baseline — the emulator's
 * software encoder came out Constrained Baseline — where the protocol row
 * says "Main where an encoder offers nothing else". [TranscodeRecipe.h264Fallback]
 * asks for Main in exactly that case.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.media.MediaCodecInfo
import android.media.metrics.LogSessionId
import android.net.Uri
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
import androidx.media3.transformer.EncoderUtil
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
    /**
     * What an audio track is encoded at. MediaPlan's target where the probe
     * saw the track; where it saw none, the profile's stereo rate — a CEILING
     * for a track only Media3 can read (see the file comment), and nothing at
     * all for a clip that really is silent: a transcode does not invent a
     * track.
     */
    val audioBitrate: Int,
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
                audioBitrate = (target.audioBitrate ?: MediaPlan.targetAudioBitrate(null, null)).toInt(),
                toneMapToSdr = true,
            )
        }

        /** A picked sound file, re-encoded at MediaPlan's [bitrate]. */
        fun forAudio(bitrate: Long): TranscodeSettings =
            TranscodeSettings(
                width = null,
                height = null,
                dropFramesTo = null,
                encoderFrameRate = null,
                videoBitrate = null,
                audioBitrate = bitrate.toInt(),
                toneMapToSdr = false,
            )
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

    /**
     * Surround (3–6 channels) mixed down to stereo, because the target is
     * "128 000 bit/s stereo" and 5.1 AAC-LC at 128 kbit/s is 21 kbit/s a
     * channel. Mono stays mono and stereo stays stereo.
     *
     * ALWAYS there, and for every count from 1 to 6, because the DECODER is
     * what finally says how many channels arrive, not the probe: an input
     * with no matrix is an error that fails the whole transcode
     * (ChannelMixingAudioProcessor.onConfigure), which is what a decoder that
     * had already mixed 5.1 down to stereo would have met, and a surround
     * track the probe never saw would have gone through unmixed. The mono and
     * stereo matrices are identities, which Media3 treats as "not active".
     * Past six there is no default mix; that transcode fails, and rule C
     * sends the original.
     */
    fun audioProcessors(): List<AudioProcessor> = listOf(
        ChannelMixingAudioProcessor().apply {
            for (channels in 1..MAX_MIXED_CHANNELS) {
                putChannelMixingMatrix(
                    ChannelMixingMatrix.createForConstantPower(channels, minOf(channels, 2)),
                )
            }
        },
    )

    fun editedMediaItem(input: Uri, settings: TranscodeSettings): EditedMediaItem =
        EditedMediaItem.Builder(MediaItem.fromUri(input))
            // Cover art an audio file carries as a picture track is not sound.
            .setRemoveVideo(settings.audioOnly)
            .setEffects(Effects(audioProcessors(), videoEffects(settings)))
            .build()

    /**
     * The one item, with the output's tracks left for Media3 to INFER from
     * what its own extractor finds in the file (see the file comment) — the
     * sequence `Transformer.start(EditedMediaItem, …)` builds, which 1.1's
     * compress used and which kept every track it could read.
     *
     * Naming the tracks instead is wrong in both directions: a type left out
     * is REMOVED from a source that has it (SequenceAssetLoader), and a type
     * named is INVENTED for a source that lacks it — silence, or black
     * frames (`forceAudioTrack`). The constructor that infers is deprecated
     * in 1.11 in favour of the one that names, and its replacement
     * (`fromSingleItem`) is package-private; this is the only public way to
     * get the behaviour together with a Composition, which the HDR mode
     * needs.
     */
    @Suppress("DEPRECATION")
    fun sequence(item: EditedMediaItem): EditedMediaItemSequence =
        EditedMediaItemSequence.Builder(item).build()

    fun composition(item: EditedMediaItem, settings: TranscodeSettings): Composition =
        Composition.Builder(sequence(item))
            .setHdrMode(
                if (settings.toneMapToSdr) {
                    Composition.HDR_MODE_TONE_MAP_HDR_TO_SDR_USING_OPEN_GL
                } else {
                    Composition.HDR_MODE_KEEP_HDR
                },
            )
            .build()

    /**
     * @param h264Fallback the profile and level to ask for where no encoder
     *   offers High ([h264Fallback]), or null to leave it to Media3.
     */
    fun videoEncoderSettings(
        settings: TranscodeSettings,
        h264Fallback: Pair<Int, Int>? = null,
    ): VideoEncoderSettings =
        VideoEncoderSettings.Builder()
            .apply { settings.videoBitrate?.let { setBitrate(it) } }
            .apply { h264Fallback?.let { (profile, level) -> setEncodingProfileLevel(profile, level) } }
            .build()

    /**
     * "High profile (Main where an encoder offers nothing else)": Main and
     * the highest level it is offered at, when NO H.264 encoder on this
     * device offers High; null otherwise — High is then Media3's to ask for
     * — and null where none offers Main either, which leaves the encoder its
     * own default as before.
     *
     * [offered] is each encoder's list of (profile, level), so the choice can
     * be pinned without a device.
     */
    fun h264Fallback(offered: List<List<Pair<Int, Int>>>): Pair<Int, Int>? {
        val all = offered.flatten()
        if (all.any { it.first == MediaCodecInfo.CodecProfileLevel.AVCProfileHigh }) return null
        val level = all
            .filter { it.first == MediaCodecInfo.CodecProfileLevel.AVCProfileMain }
            .maxOfOrNull { it.second }
            ?: return null
        return MediaCodecInfo.CodecProfileLevel.AVCProfileMain to level
    }

    /** What this device's H.264 encoders offer, for [h264Fallback]. Never throws: no answer is no request. */
    private fun h264Offered(): List<List<Pair<Int, Int>>> = runCatching {
        EncoderUtil.getSupportedEncoders(MimeTypes.VIDEO_H264).map { encoder ->
            encoder.getCapabilitiesForType(MimeTypes.VIDEO_H264).profileLevels
                .map { it.profile to it.level }
        }
    }.getOrDefault(emptyList())

    /** AAC-LC by name — the profile row says LC, and HE-AAC is not what every client decodes alike. */
    fun audioEncoderSettings(settings: TranscodeSettings): AudioEncoderSettings =
        AudioEncoderSettings.Builder()
            .setProfile(AUDIO_PROFILE)
            .setBitrate(settings.audioBitrate)
            .build()

    fun encoderFactory(context: Context, settings: TranscodeSettings): Codec.EncoderFactory =
        SampleRateEncoderFactory(
            FrameRateHintEncoderFactory(
                DefaultEncoderFactory.Builder(context)
                    .setRequestedVideoEncoderSettings(
                        videoEncoderSettings(settings, h264Fallback(h264Offered())),
                    )
                    .setRequestedAudioEncoderSettings(audioEncoderSettings(settings))
                    .build(),
                settings.encoderFrameRate,
            ),
            ::encoderSampleRate,
        )

    /**
     * The sample rate closest to [requested] that the AAC encoder Media3 is
     * about to pick will take — the first one offering AAC-LC, which is
     * DefaultEncoderFactory's own choice once a profile is asked for. With no
     * such encoder Media3 does this search itself, so [requested] is
     * returned as it came; so it is when the platform will not answer.
     */
    fun encoderSampleRate(mime: String, requested: Int): Int = runCatching {
        val encoder = EncoderUtil.getSupportedEncoders(mime)
            .firstOrNull { AUDIO_PROFILE in EncoderUtil.findSupportedEncodingProfiles(it, mime) }
            ?: return requested
        EncoderUtil.getClosestSupportedSampleRate(encoder, mime, requested)
            // What it answers for an encoder that lists no rate at all.
            .takeIf { it in 1 until Int.MAX_VALUE }
    }.getOrNull() ?: requested

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

    /** AAC-LC, the profile row's audio. */
    private const val AUDIO_PROFILE = MediaCodecInfo.CodecProfileLevel.AACObjectLC

    /** Media3 has constant-power stereo mixes for up to six channels, and none past that. */
    const val MAX_MIXED_CHANNELS = 6
}

/**
 * Asks the audio encoder for a sample rate it TAKES.
 *
 * Transformer asks for the source's own rate (AudioSampleExporter), and
 * DefaultEncoderFactory fits that to the encoder only in a fallback it skips
 * when a profile was requested — which TranscodeRecipe does, for AAC-LC. So
 * a 96 kHz source reaches an AAC encoder that lists nothing above 48 kHz,
 * and the outcome is whatever that encoder does with a rate it does not
 * list. The API 36 emulator's quietly took 44.1 kHz instead, and Media3
 * resampled to it; one that refuses fails the export, and rule C then sends
 * the lossless original — the audio rule doing nothing for exactly the
 * files it saves the most on. That is not a thing to find out per device.
 *
 * Asking for a rate the encoder lists is all it takes: Transformer compares
 * what the encoder was configured with against what it is about to feed it,
 * and resamples when they differ. A rate the encoder already takes — 44.1
 * and 48 kHz, which is nearly every file — passes through untouched.
 */
@UnstableApi
class SampleRateEncoderFactory(
    private val delegate: Codec.EncoderFactory,
    /** (MIME type, requested rate) → the rate to ask for. */
    private val supported: (String, Int) -> Int,
) : Codec.EncoderFactory {

    override fun createForAudioEncoding(format: Format, logSessionId: LogSessionId?): Codec =
        delegate.createForAudioEncoding(fitted(format), logSessionId)

    override fun createForVideoEncoding(format: Format, logSessionId: LogSessionId?): Codec =
        delegate.createForVideoEncoding(format, logSessionId)

    override fun audioNeedsEncoding(): Boolean = delegate.audioNeedsEncoding()

    override fun videoNeedsEncoding(): Boolean = delegate.videoNeedsEncoding()

    /** [format] at a rate the encoder takes; itself when it states none, or one that already is. */
    fun fitted(format: Format): Format {
        val mime = format.sampleMimeType ?: return format
        val requested = format.sampleRate
        if (requested == Format.NO_VALUE) return format
        val rate = supported(mime, requested)
        return if (rate == requested) format else format.buildUpon().setSampleRate(rate).build()
    }
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

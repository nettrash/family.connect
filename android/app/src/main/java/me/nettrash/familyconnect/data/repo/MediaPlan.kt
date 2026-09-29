/*
 * MediaPlan.kt
 * Family Connect (Android)
 *
 * What a picked video or sound file becomes before it is uploaded
 * (docs/protocol.md, "Preparing media before upload"; the reasoning is
 * docs/media-upload-2026-09-28.md, issue #74).
 *
 * The server stores what it is given and never transcodes, so the size of a
 * family's history is decided on the sending device — by four codebases that
 * have to reach the same answer for the same file. iOS compressing to 1080p
 * while this client compressed to 720p is what happens when nothing writes
 * the target down. So this file is only the DECISION, as arithmetic: what the
 * source is, as MediaProbe read it, goes in; what to do with it comes out.
 * Reading the file and encoding it are MediaProbe's and MediaTranscode's.
 *
 * A PORT, NOT A READING. The reference is `fc_text::media_plan`
 * (web/text/src/media_plan.rs), and this is held to it by the vectors that
 * `win/tools/board-oracle` prints from it — the same file sits beside the
 * iOS and Windows tests (test/resources/media-plan-vectors.json,
 * MediaPlanVectorsTest). The names are the reference's own, camel-cased, so a
 * disagreement can be looked up rather than argued about.
 *
 * Integers are Long wherever the protocol says integer (the reference's u32
 * and u64), Double only where a frame rate enters, and every function is
 * total: a number the reader could not fill in is "unknown", never a throw.
 * Pure Kotlin on purpose — no android.* here, so the vectors run on the JVM.
 *
 * iOS counterpart: ios/FamilyConnect/Core/MediaPlan.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlin.math.floor

object MediaPlan {

    // -- The profile --------------------------------------------------------

    /**
     * The target's SHORT side at most — a 720p clip is indistinguishable
     * from 1080p in a chat bubble, and roughly half the bytes.
     */
    const val MAX_SHORT_SIDE: Long = 720

    /** The frame rate a transcode is capped at. */
    const val MAX_FRAME_RATE: Double = 30.0

    /**
     * The frame rate a source may have and still count as "at most 30": a
     * reader reports a nominal 30 as 30.0003, and re-encoding for that would
     * only cost quality. A rate ABOVE this becomes [MAX_FRAME_RATE]; one at
     * or below it is kept exactly, 29.97, 25 and 24 included.
     */
    const val FRAME_RATE_TOLERANCE: Double = 30.5

    /** The video bitrate's floor and ceiling: 2 Mbit/s is the rate at 1280×720, 30 fps. */
    const val MIN_VIDEO_BITRATE: Long = 250_000
    const val MAX_VIDEO_BITRATE: Long = 2_000_000

    /** AAC-LC in a video and for a re-encoded sound file; mono gets half. */
    const val STEREO_AUDIO_BITRATE: Long = 128_000
    const val MONO_AUDIO_BITRATE: Long = 64_000

    /**
     * A voice note, which is recorded to the profile directly and never
     * passes through [planAudio]: AAC-LC, mono, 44.1 or 48 kHz. VoiceRecorder
     * reads it from here, so the recorder and the protocol row are one number.
     */
    const val VOICE_NOTE_BITRATE: Long = 64_000

    /**
     * MP3 or AAC at or below this is uploaded untouched. A second lossy
     * generation costs more than the few megabytes it saves — nettrash's 1.1
     * objection to re-encoding someone's music, kept where it is right.
     */
    const val MAX_KEPT_LOSSY_AUDIO_BITRATE: Long = 192_000

    // -- What the reader saw ------------------------------------------------

    /**
     * A video about to be sent as `kind=video`, as MediaProbe saw it.
     *
     * Codecs are the reference's lowercase names, not MIME types, so four
     * readers can agree on them: "h264", "hevc", "av1", "vp9" or "unknown";
     * for audio "aac" (any profile) and the rest. Only "h264" and "aac" are
     * ever asked about.
     */
    data class VideoSource(
        /** The DISPLAYED size, after the rotation; 0 when unreadable. */
        val width: Long,
        val height: Long,
        /** Frames per second, or null. Not finite and above zero is unknown too. */
        val frameRate: Double?,
        /** The type it would be uploaded as — "video/mp4", "video/quicktime" — lowercase. */
        val container: String,
        val videoCodec: String,
        /** Null when the file has NO audio track; a track nobody can name is "unknown". */
        val audioCodec: String?,
        /** Only exactly 1 is mono. */
        val audioChannels: Long?,
        /** The rates the container STATES, or null (0 is unknown too). */
        val videoBitrate: Long?,
        val audioBitrate: Long?,
        val sizeBytes: Long,
        val durationMs: Long?,
    )

    /** What a video is transcoded TO. */
    data class VideoTarget(
        /** Even, never larger than the source's, in the source's orientation. */
        val width: Long,
        val height: Long,
        val frameRate: Double,
        val videoBitrate: Long,
        /** Null when the source has no audio track — a transcode does not invent silence. */
        val audioBitrate: Long?,
    )

    /** What happens to a video. */
    sealed interface VideoPlan {
        /** Rule A: already within the profile, and the ORIGINAL is uploaded. */
        data object Keep : VideoPlan

        /** Rule 5: transcode to [target]. */
        data class Transcode(val target: VideoTarget) : VideoPlan

        /**
         * Rule C: there is no size to scale to — none was read, or the short
         * side is one pixel and an even target would have none — so this
         * source goes the way it went before the profile existed.
         */
        data object Fallback : VideoPlan
    }

    /**
     * A sound file the member PICKED — never a voice note.
     *
     * [codec] is "pcm" (WAV and AIFF), "flac", "alac", "aac", "mp3",
     * "vorbis", "opus" or "unknown"; [container] is a type the server
     * accepts as audio or the platform's own name for one it does not
     * ("audio/flac").
     */
    data class AudioSource(
        val container: String,
        val codec: String,
        val channels: Long?,
        /** As the file STATES it, or null (0 is unknown too). */
        val bitrate: Long?,
        val sizeBytes: Long,
        val durationMs: Long?,
    )

    /** What happens to a picked sound file. */
    sealed interface AudioPlan {
        /** Uploaded untouched. */
        data object Keep : AudioPlan

        /** Re-encoded as M4A (`audio/mp4`), AAC-LC, at [bitrate]. */
        data class Transcode(val bitrate: Long) : AudioPlan
    }

    /** What rule C sends when a transcode fails, or cannot be attempted. */
    enum class OnFailure {
        /** The original, untouched, as its kind. */
        ORIGINAL,

        /** Whatever this client did before the profile existed. Not a new rule: the old one. */
        TODAYS_PATH,
    }

    /** Which bytes rule D uploads. */
    enum class Upload { SOURCE, RESULT }

    // -- Unknowns -------------------------------------------------------------

    /**
     * A count a reader handed over, or null when it could not. 0 is null as
     * well — it is what a container writes for a rate it does not state — and
     * so is a negative, which only a confused reader produces.
     */
    private fun known(value: Long?): Long? = value?.takeIf { it > 0 }

    /** A frame rate worth believing: finite and above zero. */
    fun knownFrameRate(frameRate: Double?): Double? =
        frameRate?.takeIf { it.isFinite() && it > 0.0 }

    // -- The rules ------------------------------------------------------------

    /**
     * `size × 8 × 1000 ÷ duration_ms − audioBitrate`, in integers (the
     * division truncates). Null when it "cannot be estimated": no duration,
     * or nothing left once the audio is taken off.
     *
     * The product saturates rather than overflowing. Long saturates at
     * 2^63 − 1 where the reference's u64 saturates at 2^64 − 1; the two
     * only part company for a file over a petabyte.
     */
    fun estimatedBitrate(sizeBytes: Long, durationMs: Long?, audioBitrate: Long): Long? {
        val duration = known(durationMs) ?: return null
        val whole = saturatingTimes(sizeBytes.coerceAtLeast(0), 8 * 1000) / duration
        if (audioBitrate > whole) return null
        return (whole - audioBitrate).takeIf { it > 0 }
    }

    /**
     * `V`: the video bitrate as the container states it, or else estimated.
     * The estimate takes the audio off, so an audio track whose rate is not
     * stated leaves an unknown in the formula — and V is unknown. With no
     * audio track at all, nothing is taken off.
     */
    fun sourceVideoBitrate(source: VideoSource): Long? {
        known(source.videoBitrate)?.let { return it }
        val audio = if (source.audioCodec == null) 0L else known(source.audioBitrate) ?: return null
        return estimatedBitrate(source.sizeBytes, source.durationMs, audio)
    }

    /**
     * Rule 1: the target size for a DISPLAYED `width × height`.
     *
     * The short side becomes `min(720, short)` — never upscaled — and the
     * long side follows in proportion, rounded half up in integers:
     * `(2 × long × ts + short) ÷ (2 × short)`. Then each side drops to even
     * (dropping, not rounding up, is what keeps it from exceeding the
     * source). The target keeps the source's orientation. (0, 0) for a
     * source with no size.
     */
    fun targetSize(width: Long, height: Long): Pair<Long, Long> {
        val short = minOf(width, height)
        val long = maxOf(width, height)
        if (short <= 0) return 0L to 0L
        val targetShort = minOf(short, MAX_SHORT_SIDE)
        val targetLong = (2 * long * targetShort + short) / (2 * short)
        val evenShort = targetShort - targetShort % 2
        val evenLong = targetLong - targetLong % 2
        return if (width >= height) evenLong to evenShort else evenShort to evenLong
    }

    /**
     * Rule 2: 30 when the source's rate is above [FRAME_RATE_TOLERANCE] or
     * unknown, and the source's own rate otherwise — never raised, never
     * tidied.
     */
    fun targetFrameRate(frameRate: Double?): Double {
        val rate = knownFrameRate(frameRate)
        return if (rate != null && rate <= FRAME_RATE_TOLERANCE) rate else MAX_FRAME_RATE
    }

    /**
     * Rule 3, before the source cap:
     * `2 000 000 × (w × h ÷ 921 600) × (f ÷ 30)`, clamped to
     * [250 000, 2 000 000], rounded to the nearest 1 000, half up.
     *
     * THE EVALUATION ORDER IS PART OF THE RULE. Exact half-thousands exist at
     * real sizes (960×540 at 25 fps is 937 500), and written left to right
     * the formula lands a hair under some of them. So, exactly as the
     * reference: the rate in thousands is `w × h × f ÷ 13 824` — one Double
     * product of the integer pixel count and the rate, then one division.
     *
     * And HALF UP by hand: Kotlin's `round` is half-to-even, which would make
     * 404×288 at 30 (252.5 thousand) 252 000 where the other ports say 253 000.
     * `x − floor(x)` is exact for any Double this size, so the comparison
     * with 0.5 is too.
     */
    fun profileVideoBitrate(width: Long, height: Long, frameRate: Double): Long {
        val pixels = width * height
        val thousands = pixels.toDouble() * frameRate / 13_824.0
        val clamped = thousands.coerceIn(
            (MIN_VIDEO_BITRATE / 1000).toDouble(),
            (MAX_VIDEO_BITRATE / 1000).toDouble(),
        )
        val whole = floor(clamped)
        val rounded = if (clamped - whole >= 0.5) whole + 1 else whole
        return rounded.toLong() * 1000
    }

    /**
     * Rule 3 whole: the profile's rate, and no higher than `V` when V is
     * known (rule B). The cap comes after the rounding, so a capped target
     * is the source's exact rate.
     */
    fun targetVideoBitrate(width: Long, height: Long, frameRate: Double, sourceBitrate: Long?): Long {
        val profile = profileVideoBitrate(width, height, frameRate)
        val source = known(sourceBitrate) ?: return profile
        return minOf(profile, source)
    }

    /**
     * 128 000 for stereo, 64 000 for mono, never above the source's when that
     * is known. Only exactly 1 channel is mono: 5.1 takes the stereo rate,
     * and so does a count nobody could read — guessing mono would halve a
     * stereo track.
     */
    fun targetAudioBitrate(channels: Long?, sourceBitrate: Long?): Long {
        val profile = if (channels == 1L) MONO_AUDIO_BITRATE else STEREO_AUDIO_BITRATE
        val source = known(sourceBitrate) ?: return profile
        return minOf(profile, source)
    }

    /**
     * Rules 1–3 for a source, or null when it has no size. The audio's cap is
     * the rate the container STATES for it: the protocol gives no way to
     * estimate an audio track's rate inside a video.
     */
    fun videoTarget(source: VideoSource): VideoTarget? {
        if (source.width <= 0 || source.height <= 0) return null
        val (width, height) = targetSize(source.width, source.height)
        val frameRate = targetFrameRate(source.frameRate)
        return VideoTarget(
            width = width,
            height = height,
            frameRate = frameRate,
            videoBitrate = targetVideoBitrate(width, height, frameRate, sourceVideoBitrate(source)),
            audioBitrate = source.audioCodec?.let {
                targetAudioBitrate(source.audioChannels, source.audioBitrate)
            },
        )
    }

    /**
     * Rule A — leave it alone. Every condition exactly as listed: `video/mp4`,
     * H.264, AAC or no audio, short side at most 720, F known and at most
     * 30.5, V known and at most 1.25 × step 3's bitrate (`4V ≤ 5 × target`,
     * so it stays in integers). `video/quicktime` never is: Firefox will not
     * play the container.
     *
     * Not asked, because the protocol's list does not ask: the audio's
     * bitrate, even sides, or where the `moov` box is. A kept file is the
     * original, whatever those are.
     */
    fun withinProfile(source: VideoSource): Boolean {
        val target = videoTarget(source) ?: return false
        val rate = knownFrameRate(source.frameRate) ?: return false
        val bitrate = sourceVideoBitrate(source) ?: return false
        return source.container == "video/mp4" &&
            source.videoCodec == "h264" &&
            (source.audioCodec == null || source.audioCodec == "aac") &&
            minOf(source.width, source.height) <= MAX_SHORT_SIDE &&
            rate <= FRAME_RATE_TOLERANCE &&
            saturatingTimes(bitrate, 4) <= saturatingTimes(target.videoBitrate, 5)
    }

    /**
     * Rules A, 5 and C, in that order: kept when within the profile (a
     * one-pixel-wide clip can be, and it is the original that goes);
     * otherwise transcoded; and with no size to transcode to, the fallback.
     */
    fun planVideo(source: VideoSource): VideoPlan {
        val target = videoTarget(source) ?: return VideoPlan.Fallback
        if (withinProfile(source)) return VideoPlan.Keep
        if (target.width == 0L || target.height == 0L) return VideoPlan.Fallback
        return VideoPlan.Transcode(target)
    }

    /** A sound file's rate: as stated, or estimated with nothing taken off — it is the only stream. */
    fun sourceAudioBitrate(source: AudioSource): Long? =
        known(source.bitrate) ?: estimatedBitrate(source.sizeBytes, source.durationMs, 0)

    /**
     * The audio rules, in order:
     *
     * 1. Uncompressed or lossless (`pcm`, `flac`, `alac`) is re-encoded:
     *    about ten times smaller, and no first generation to lose.
     * 2. Anything in `audio/ogg` is re-encoded, whatever the codec, because
     *    it does not play on iOS or macOS. "Wherever the platform can decode
     *    it" is not this function's to know: a platform that cannot fails
     *    the transcode, and rule C sends the file as before.
     * 3. MP3 or AAC above 192 000 bit/s is re-encoded; at or below, or at a
     *    rate nobody can read or estimate, it is untouched.
     *
     * Anything else — AC-3, Opus outside Ogg, a codec nobody could name — is
     * left alone, because no rule says to touch it.
     */
    fun planAudio(source: AudioSource): AudioPlan {
        val bitrate = sourceAudioBitrate(source)
        val transcode = when {
            source.codec in LOSSLESS_CODECS -> true
            source.container == "audio/ogg" -> true
            source.codec == "mp3" || source.codec == "aac" ->
                bitrate != null && bitrate > MAX_KEPT_LOSSY_AUDIO_BITRATE
            else -> false
        }
        return if (transcode) {
            AudioPlan.Transcode(targetAudioBitrate(source.channels, bitrate))
        } else {
            AudioPlan.Keep
        }
    }

    // -- When it goes wrong, or does not help ---------------------------------

    /**
     * Whether a file could be uploaded as [kind] ("video" or "audio") just as
     * it is: an accepted type, honest bytes (MediaPrep.Magic), and within the
     * ceiling — AT MOST, because the server refuses only a body larger than
     * it. Any other kind is not one these rules are about.
     */
    fun sendable(
        kind: String,
        container: String,
        honest: Boolean,
        sizeBytes: Long,
        ceilingBytes: Long,
    ): Boolean {
        val accepted = when (kind) {
            "video" -> container in ACCEPTED_VIDEO
            "audio" -> container in ACCEPTED_AUDIO
            else -> false
        }
        return accepted && honest && sizeBytes <= ceilingBytes
    }

    /**
     * Rule C — a failure sends what would have been sent without the
     * profile. Preparing media is an optimisation; it may never turn a send
     * that would have worked into one that does not.
     */
    fun onFailure(sourceSendable: Boolean): OnFailure =
        if (sourceSendable) OnFailure.ORIGINAL else OnFailure.TODAYS_PATH

    /**
     * Rule D — a result BIGGER than its source is thrown away and the source
     * uploaded, provided the source can go as it is. Otherwise the result,
     * the only thing that can be sent. Equal is not bigger: the result is
     * in the profile, and the source may not be.
     */
    fun keepSmaller(sourceBytes: Long, sourceSendable: Boolean, resultBytes: Long): Upload =
        if (resultBytes > sourceBytes && sourceSendable) Upload.SOURCE else Upload.RESULT

    /** The server's accepted types (server models.rs; docs/protocol.md, "Audio"). */
    private val ACCEPTED_VIDEO = setOf("video/mp4", "video/quicktime")
    private val ACCEPTED_AUDIO = setOf("audio/mp4", "audio/m4a", "audio/mpeg", "audio/wav", "audio/ogg")

    private val LOSSLESS_CODECS = setOf("pcm", "flac", "alac")

    private fun saturatingTimes(value: Long, factor: Long): Long =
        if (value > Long.MAX_VALUE / factor) Long.MAX_VALUE else value * factor
}

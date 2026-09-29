/*
 * MediaProbe.kt
 * Family Connect (Android)
 *
 * What a picked video or sound file IS, in the terms MediaPlan decides in:
 * its displayed size, frame rate, codecs, the bitrates its container states,
 * its channel count, size and duration.
 *
 * READ, NEVER GUESSED. Every field the platform will not give is left
 * unknown (null or 0), and MediaPlan has one rule for unknowns that every
 * port shares — a frame rate that cannot be read is "30", a bitrate that
 * cannot be read or estimated sends a clip past rule A to a transcode, and
 * rule D keeps the original if that came out bigger. A probe that invented a
 * plausible number instead would make this client disagree with the others
 * about the same file, silently.
 *
 * The platform's own readers (MediaExtractor for the tracks,
 * MediaMetadataRetriever for what it computes from the container), not
 * Media3's: the same ones readVideoMetadata already trusts for the bubble's
 * shape, and they answer synchronously on the calling thread. What each
 * reader says is then turned into the reference's vocabulary by the pure
 * functions at the bottom, which are what MediaProbeTest pins.
 *
 * Callers run this on Dispatchers.IO: it reads the file.
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Build
import java.io.File

object MediaProbe {

    /**
     * A video as MediaPlan sees it.
     *
     * @param container the type it would be uploaded as — what the provider
     *   called it, not what the bytes are; rule A compares it exactly.
     */
    fun video(context: Context, uri: Uri, container: String, sizeBytes: Long): MediaPlan.VideoSource {
        val tracks = tracks { setDataSource(context, uri, null) }
        val facts = retrieved { setDataSource(context, uri) }
        val video = tracks.video
        val audio = tracks.audio

        val (width, height) = displaySize(
            width = facts.int(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)
                ?: video?.intOrNull(MediaFormat.KEY_WIDTH),
            height = facts.int(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)
                ?: video?.intOrNull(MediaFormat.KEY_HEIGHT),
            rotation = facts.int(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)
                ?: video?.intOrNull(MediaFormat.KEY_ROTATION),
        )
        val frameCount = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            facts.long(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)
        } else {
            null
        }
        return MediaPlan.VideoSource(
            width = width,
            height = height,
            frameRate = frameRate(
                frameCount = frameCount,
                durationUs = video?.longOrNull(MediaFormat.KEY_DURATION),
                stated = video?.numberOrNull(MediaFormat.KEY_FRAME_RATE),
            ),
            container = container,
            videoCodec = videoCodec(video?.stringOrNull(MediaFormat.KEY_MIME)),
            audioCodec = audio?.let { audioCodec(it.stringOrNull(MediaFormat.KEY_MIME)) },
            audioChannels = audio?.intOrNull(MediaFormat.KEY_CHANNEL_COUNT)?.toLong(),
            videoBitrate = video?.intOrNull(MediaFormat.KEY_BIT_RATE)?.toLong(),
            audioBitrate = audio?.intOrNull(MediaFormat.KEY_BIT_RATE)?.toLong(),
            sizeBytes = sizeBytes,
            durationMs = facts.long(MediaMetadataRetriever.METADATA_KEY_DURATION),
        )
    }

    /**
     * A picked sound file as MediaPlan sees it. A file with no audio track
     * the platform can find is "unknown", which no audio rule re-encodes —
     * outside Ogg — so it goes as it would have gone before.
     */
    fun audio(file: File, container: String, sizeBytes: Long): MediaPlan.AudioSource {
        val audio = tracks { setDataSource(file.absolutePath) }.audio
        val facts = retrieved { setDataSource(file.absolutePath) }
        return MediaPlan.AudioSource(
            container = container,
            codec = audioCodec(audio?.stringOrNull(MediaFormat.KEY_MIME)),
            channels = audio?.intOrNull(MediaFormat.KEY_CHANNEL_COUNT)?.toLong(),
            bitrate = audio?.intOrNull(MediaFormat.KEY_BIT_RATE)?.toLong(),
            sizeBytes = sizeBytes,
            durationMs = facts.long(MediaMetadataRetriever.METADATA_KEY_DURATION)
                ?: audio?.longOrNull(MediaFormat.KEY_DURATION)?.let { it / 1000 },
        )
    }

    // -- The reference's vocabulary (pure) ------------------------------------

    /** A video track's MIME type as the reference names the codec. */
    fun videoCodec(mime: String?): String = when (mime?.lowercase()) {
        MediaFormat.MIMETYPE_VIDEO_AVC -> "h264"
        MediaFormat.MIMETYPE_VIDEO_HEVC -> "hevc"
        // MediaFormat.MIMETYPE_VIDEO_AV1's value; the constant is API 29 and minSdk is 26.
        "video/av01" -> "av1"
        MediaFormat.MIMETYPE_VIDEO_VP9 -> "vp9"
        // Dolby Vision, VP8, MPEG-4 Part 2, H.263 — none of them "h264",
        // which is the only name rule A asks about.
        else -> "unknown"
    }

    /**
     * An audio track's MIME type as the reference names the codec: "aac" for
     * every AAC profile (the platform does not split them by type), "pcm" for
     * linear PCM whatever its container. A-law, µ-law and ADPCM in a WAV are
     * NOT pcm — they are already compressed, and no rule names them.
     */
    fun audioCodec(mime: String?): String = when (mime?.lowercase()) {
        MediaFormat.MIMETYPE_AUDIO_AAC -> "aac"
        MediaFormat.MIMETYPE_AUDIO_MPEG -> "mp3"
        MediaFormat.MIMETYPE_AUDIO_RAW -> "pcm"
        MediaFormat.MIMETYPE_AUDIO_FLAC -> "flac"
        // No MediaFormat constant; this is what MPEG4Extractor calls it.
        "audio/alac" -> "alac"
        MediaFormat.MIMETYPE_AUDIO_VORBIS -> "vorbis"
        MediaFormat.MIMETYPE_AUDIO_OPUS -> "opus"
        else -> "unknown"
    }

    /**
     * The size as DISPLAYED: a portrait phone clip is a landscape track with
     * a 90° turn, and rule 1 wants 1080×1920, not 1920×1080. (0, 0) when
     * either side cannot be read — MediaPlan's "no size".
     */
    fun displaySize(width: Int?, height: Int?, rotation: Int?): Pair<Long, Long> {
        if (width == null || height == null || width <= 0 || height <= 0) return 0L to 0L
        val turned = rotation != null && Math.floorMod(rotation, 180) == 90
        return if (turned) height.toLong() to width.toLong() else width.toLong() to height.toLong()
    }

    /**
     * Frames per second: the sample count over the track's duration when
     * both are known, else the rate the extractor states.
     *
     * The count first because MPEG4Extractor states an MP4's rate as an
     * INTEGER — 29.97 reads as 30, 59.94 as 60 — where AVFoundation and the
     * others read 29.97002997…; the bitrate rule 3 computes differs by the
     * difference. 1 800 frames over 60 060 000 µs is exactly 30 000 ÷ 1 001.
     */
    fun frameRate(frameCount: Long?, durationUs: Long?, stated: Double?): Double? {
        if (frameCount != null && frameCount > 0 && durationUs != null && durationUs > 0) {
            return frameCount * 1_000_000.0 / durationUs
        }
        return stated
    }

    /** `Video/MP4; codecs="avc1"` and `video/mp4` are the same type — the reference's "essence". */
    fun essence(mime: String?): String =
        mime.orEmpty().substringBefore(';').trim().lowercase()

    // -- The platform's readers -----------------------------------------------

    private class Tracks(val video: MediaFormat?, val audio: MediaFormat?)

    /**
     * The first video and first audio track. Any failure — no such file, a
     * container the platform cannot open — is no tracks, not a throw: an
     * unreadable source is MediaPlan's Fallback, which sends it as before.
     */
    private fun tracks(open: MediaExtractor.() -> Unit): Tracks {
        val extractor = MediaExtractor()
        return try {
            extractor.open()
            var video: MediaFormat? = null
            var audio: MediaFormat? = null
            for (index in 0 until extractor.trackCount) {
                val format = extractor.getTrackFormat(index)
                val mime = format.stringOrNull(MediaFormat.KEY_MIME).orEmpty()
                if (video == null && mime.startsWith("video/")) video = format
                if (audio == null && mime.startsWith("audio/")) audio = format
            }
            Tracks(video, audio)
        } catch (_: Exception) {
            Tracks(null, null)
        } finally {
            runCatching { extractor.release() }
        }
    }

    private class Retrieved(private val values: Map<Int, String>) {
        fun int(key: Int): Int? = values[key]?.trim()?.toIntOrNull()
        fun long(key: Int): Long? = values[key]?.trim()?.toLongOrNull()?.takeIf { it > 0 }
    }

    /**
     * What MediaMetadataRetriever says, read once and released. NOT `use {}`:
     * it only became AutoCloseable in API 29 and minSdk is 26 (the same note
     * as prepareAudio's).
     */
    private fun retrieved(open: MediaMetadataRetriever.() -> Unit): Retrieved {
        val retriever = MediaMetadataRetriever()
        return try {
            retriever.open()
            Retrieved(
                RETRIEVED_KEYS.mapNotNull { key ->
                    runCatching { retriever.extractMetadata(key) }.getOrNull()?.let { key to it }
                }.toMap(),
            )
        } catch (_: Exception) {
            Retrieved(emptyMap())
        } finally {
            runCatching { retriever.release() }
        }
    }

    private val RETRIEVED_KEYS = buildList {
        add(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)
        add(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)
        add(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)
        add(MediaMetadataRetriever.METADATA_KEY_DURATION)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            add(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)
        }
    }

    // MediaFormat's getters THROW for a missing key (and for the wrong type),
    // and which keys an extractor fills in varies by container and by release.

    private fun MediaFormat.stringOrNull(key: String): String? =
        runCatching { if (containsKey(key)) getString(key) else null }.getOrNull()

    private fun MediaFormat.intOrNull(key: String): Int? =
        runCatching { if (containsKey(key)) getInteger(key) else null }.getOrNull()

    private fun MediaFormat.longOrNull(key: String): Long? =
        runCatching { if (containsKey(key)) getLong(key) else null }.getOrNull()

    /**
     * A key some extractors store as an Integer and others as a Float —
     * KEY_FRAME_RATE is both, depending on the container.
     */
    private fun MediaFormat.numberOrNull(key: String): Double? {
        if (!containsKey(key)) return null
        return runCatching { getInteger(key).toDouble() }.getOrNull()
            ?: runCatching { getFloat(key).toDouble() }.getOrNull()
    }
}

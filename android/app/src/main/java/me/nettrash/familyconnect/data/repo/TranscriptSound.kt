/*
 * TranscriptSound.kt
 * Family Connect (Android)
 *
 * The sound a transcript request SUPPLIES (docs/protocol.md, "Transcripts on
 * request"): for a video, and for a recording the server will not send from
 * its own copy — Ogg, a type outside the provider's list, or one over
 * `transcribe_max_bytes` — this device takes the sound track out of the
 * file it holds (downloading it through the attachment cache first) and
 * sends it as one AAC M4A.
 *
 * Built on the #74 media code, not a pipeline of its own: MediaProbe reads
 * what the track is, MediaPrep runs the Media3 export it already runs for
 * uploads, TranscodeRecipe says what Media3 is told. No picture is decoded:
 * the video track is removed before anything is read from it.
 *
 * Three layers, so the deciding can be pinned on the JVM:
 *  - [TranscriptSoundPlan] — which ways to try, in order, and whether a
 *    result fits. Pure.
 *  - [TranscriptSoundPlan.take] — the loop over those ways, with the
 *    extraction handed in. Pure apart from what it is handed.
 *  - [DeviceTranscriptSound] — the platform: the download, the probe, the
 *    export.
 *
 * WHAT A DEVICE THAT CANNOT DO IT SHOWS, decided once for every case and
 * the same on every client: the action stays — this device cannot know what
 * is inside a file before it holds it, the server describing a video only
 * as `video/mp4`, never by its audio codec, so hiding the action would rest
 * on a guess — and the answer is one of the catalogue's two sentences for
 * what the DEVICE could not do, never "Not available for this message.",
 * which reads as a refusal. A recording whose stated length could not fit
 * even at 64 kbit/s ([TranscriptSoundPlan.knownTooLong], told before the
 * file is fetched) and sound still over the ceiling after re-encoding are
 * "This recording is too long to turn into text." ([Result.TooLong]); a
 * track the platform cannot decode and a file with no sound are "Couldn't
 * read the sound in this file." ([Result.Unreadable]). Only a download that
 * did not finish is "try again".
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import java.io.File
import javax.inject.Inject
import javax.inject.Singleton

object TranscriptSoundPlan {

    /** How the sound is taken out. */
    enum class Way {
        /** The AAC track copied into an M4A untouched: nothing decoded or encoded. */
        PASSTHROUGH,

        /** Decoded and encoded again: 64 kbit/s mono AAC-LC. */
        REENCODE,
    }

    /** What taking the sound out came to. */
    sealed interface Result {
        /** An AAC M4A within the ceiling, in the upload cache. The caller deletes it. */
        data class Ready(val file: File, val way: Way) : Result

        /** The file could not be downloaded: try again later. */
        data object NotFetched : Result

        /** The sound is over the server's ceiling even re-encoded at 64 kbit/s mono. */
        data object TooLong : Result

        /** This device could not take sound out of the file: none there, or none it can decode. */
        data object Unreadable : Result
    }

    /**
     * Whether a recording of [durationMs] — the attachment's own stated
     * length — is certainly too long for [maxBytes] even re-encoded at
     * 64 kbit/s, so it is said without downloading anything. The web's and
     * iOS's arithmetic exactly (the sound alone, no container margin), so
     * the same recording gets the same answer everywhere. A length nobody
     * stated is not "too long".
     */
    fun knownTooLong(durationMs: Long?, maxBytes: Long): Boolean {
        if (durationMs == null || durationMs <= 0 || maxBytes <= 0) return false
        return TranscodeSettings.TRANSCRIPT_SOUND_BITRATE.toLong() * durationMs / 8_000 > maxBytes
    }

    /**
     * The ways worth trying, in order, for a source whose first sound track
     * is [codec] (MediaProbe's vocabulary — "aac", "opus", "unknown" …).
     *
     * AAC is copied as it is, which is fastest and loses nothing; anything
     * else is re-encoded. A copy whose size can be ESTIMATED over the
     * ceiling — its stated [bitrate] over its [durationMs] — is skipped for
     * the re-encode, which at 64 kbit/s mono is smaller than nearly any
     * AAC a phone records. A re-encode estimated over the ceiling is not
     * tried at all. What cannot be estimated is tried, and [fits] decides
     * afterwards.
     *
     * "unknown" — no track the platform's extractor names — is still tried
     * as a re-encode: Media3 reads with its own extractor, which finds
     * tracks the platform's does not (see MediaTranscode's file comment).
     */
    fun ways(codec: String?, bitrate: Long?, durationMs: Long?, maxBytes: Long): List<Way> {
        if (maxBytes <= 0) return emptyList()
        return buildList {
            if (codec == "aac" && !overCeiling(bitrate, durationMs, maxBytes)) add(Way.PASSTHROUGH)
            if (!overCeiling(TranscodeSettings.TRANSCRIPT_SOUND_BITRATE.toLong(), durationMs, maxBytes)) {
                add(Way.REENCODE)
            }
        }
    }

    /** The server's own bound: between 1 byte and the ceiling, inclusive. */
    fun fits(sizeBytes: Long, maxBytes: Long): Boolean = maxBytes > 0 && sizeBytes in 1..maxBytes

    /**
     * Whether sound at [bitrate] for [durationMs] would be known to come
     * out over [maxBytes]. Unknown either way is not "over": it is tried.
     * The container's own boxes are allowed [CONTAINER_MARGIN_PERCENT] on top.
     */
    fun overCeiling(bitrate: Long?, durationMs: Long?, maxBytes: Long): Boolean {
        if (bitrate == null || bitrate <= 0 || durationMs == null || durationMs <= 0) return false
        return estimatedBytes(bitrate, durationMs) > maxBytes
    }

    /** Bytes of sound at [bitrate] bit/s for [durationMs], with the container's margin. */
    fun estimatedBytes(bitrate: Long, durationMs: Long): Long {
        // Whole numbers: a float's 1.02 would put an exact bound a byte off.
        val sound = bitrate * durationMs / 8_000
        return sound + sound * CONTAINER_MARGIN_PERCENT / 100
    }

    /**
     * Each way in turn until one gives a file that [fits]: a way that fails
     * (null) or comes out too large is followed by the next, and every
     * too-large result is deleted here. None left is [Result.TooLong] when
     * something came out over the ceiling — or nothing was worth trying,
     * which [ways] decides only when even the re-encode is estimated over
     * it — and [Result.Unreadable] when nothing came out at all.
     */
    suspend fun take(
        ways: List<Way>,
        maxBytes: Long,
        extract: suspend (Way) -> File?,
    ): Result {
        if (ways.isEmpty()) return Result.TooLong
        var tooLarge = false
        for (way in ways) {
            val file = extract(way) ?: continue
            if (fits(file.length(), maxBytes)) return Result.Ready(file, way)
            if (file.length() > maxBytes) tooLarge = true
            file.delete()
        }
        return if (tooLarge) Result.TooLong else Result.Unreadable
    }

    /** MP4 boxes on top of the sound itself, in percent: generous for a sound-only file. */
    const val CONTAINER_MARGIN_PERCENT = 2L
}

/** The sound of one attachment, for a transcript request. Tests script it. */
interface TranscriptSoundSource {
    suspend fun soundFor(attachment: AttachmentDto, maxBytes: Long): TranscriptSoundPlan.Result
}

@Singleton
class DeviceTranscriptSound @Inject constructor(
    private val attachments: AttachmentRepository,
    private val mediaPrep: MediaPrep,
) : TranscriptSoundSource {

    override suspend fun soundFor(attachment: AttachmentDto, maxBytes: Long): TranscriptSoundPlan.Result {
        // The attachment cache's own download: the same file "Save" and
        // "Open" use, fetched once however often it is asked for.
        val source = attachments.fileFor(attachment) ?: return TranscriptSoundPlan.Result.NotFetched
        return soundOf(mediaPrep, source, attachment.mime, attachment.durationMs?.toLong(), maxBytes)
    }

    companion object {
        /**
         * The sound of a file already on this device: probed, planned, taken
         * out. [declaredDurationMs] — the attachment's own — stands in when
         * the file does not state its length. (A function of its own so the
         * device test can run it on a fixture without a server.)
         */
        suspend fun soundOf(
            mediaPrep: MediaPrep,
            source: File,
            mime: String,
            declaredDurationMs: Long?,
            maxBytes: Long,
        ): TranscriptSoundPlan.Result {
            val read = withContext(Dispatchers.IO) {
                MediaProbe.audio(source, MediaProbe.essence(mime), source.length())
            }
            val ways = TranscriptSoundPlan.ways(
                codec = read.codec,
                bitrate = read.bitrate,
                durationMs = read.durationMs ?: declaredDurationMs,
                maxBytes = maxBytes,
            )
            return TranscriptSoundPlan.take(ways, maxBytes) { way -> mediaPrep.soundTrackOrNull(source, way) }
        }
    }
}

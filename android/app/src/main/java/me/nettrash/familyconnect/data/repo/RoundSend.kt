/*
 * RoundSend.kt
 * Family Connect (Android)
 *
 * What a VIDEO MESSAGE send must be before a row is written for it (#79,
 * docs/audio-video-messages-2026-10-04.md, "What the server checks";
 * docs/protocol.md, "Video messages"). The server refuses every other shape
 * with a 400 — `validation` beside the sticker flag or under words,
 * `invalid_attachment` for anything but one square H.264 MP4 of at most a
 * minute — and a refused send in the outbox is a red bubble the sender
 * never asked for. So the same checks run here first, and a shape the
 * server would refuse never becomes a row.
 *
 * The size is checked against `max_round_video_bytes` (RoundVideoLimits) —
 * never against a number this file invents. Without that key there is no
 * video message at all: its absence is a server without them, which would
 * IGNORE `round` and deliver a square video, so "a client must not send the
 * flag without the discovery keys" (docs/protocol.md, "Video messages").
 * The recorder reads the same key first, and a clip over it is sent as a
 * regular video after the person has been told (S3.6), so this is the
 * backstop, not the rule people meet.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

object RoundSend {

    /** The one type a video message may be (`video/quicktime` is refused). */
    const val MIME = "video/mp4"

    /** The largest edge the server takes (`width == height`, 1…720). */
    const val MAX_EDGE = 720

    /** The longest the server takes, in milliseconds (`max_round_video_ms`). */
    const val MAX_DURATION_MS = 60_000

    /**
     * Whether [prepared] under [caption] may go out with `round: true`: the
     * shape the server takes, and a file that fits under [maxBytes] — null,
     * a device that has not heard the key, is never a video message.
     *
     * The caption is judged as it will be SENT — trimmed, the body the row
     * carries — so this and the body can never disagree about whether there
     * were words.
     */
    fun accepts(
        prepared: List<MediaPrep.Prepared>,
        caption: String,
        sticker: Boolean,
        /** `max_round_video_bytes`, or null when this device has not heard it. */
        maxBytes: Long?,
    ): Boolean {
        if (maxBytes == null) return false
        if (sticker) return false
        val item = prepared.singleOrNull() ?: return false
        if (caption.trim().isNotEmpty()) return false
        if (item.kind != AttachmentDto.KIND_VIDEO || item.mime != MIME) return false
        val width = item.width ?: return false
        val height = item.height ?: return false
        if (width != height || width !in 1..MAX_EDGE) return false
        val duration = item.durationMs ?: return false
        if (duration !in 1..MAX_DURATION_MS) return false
        return item.file.length() <= maxBytes
    }
}

/*
 * AnimationSend.kt
 * Family Connect (Android)
 *
 * What an ANIMATION send must be before a row is written for it (issue #96;
 * docs/protocol.md, "Animations" → "What the server checks"), and how a
 * composer holding one splits into messages.
 *
 * The server refuses every other shape with a 400 — `validation` beside the
 * sticker or round flag, `invalid_attachment` for anything but ONE
 * `video/mp4` with a size whose short side is 1…720 and a positive length —
 * and a refused send in the outbox is a red bubble nobody asked for. So the
 * same checks run here first ([accepts]), as RoundSend does for a video
 * message. Unlike a video message an animation MAY carry a caption.
 *
 * And because the flag is refused beside more than one attachment, a composer
 * holding an animation among other items sends them all as ONE message, the
 * animation as the ordinary video it is, without the flag
 * ([sendsAsAnimation]) — the member's set is never split.
 *
 * Plain Kotlin over MediaPrep.Prepared's fields; the JVM tests pin it.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

object AnimationSend {

    /** The one type an animation may be. */
    const val MIME = "video/mp4"

    /** The largest SHORT side the server takes. */
    const val MAX_SHORT_SIDE = 720

    /**
     * Whether [prepared] may go out with `animation: true`: exactly one
     * `kind=video` MP4 with a declared size (short side 1…720) and a positive
     * length, and neither of the other two flags.
     */
    fun accepts(prepared: List<MediaPrep.Prepared>, sticker: Boolean, round: Boolean): Boolean {
        if (sticker || round) return false
        val item = prepared.singleOrNull() ?: return false
        if (item.kind != AttachmentDto.KIND_VIDEO || item.mime != MIME) return false
        val width = item.width ?: return false
        val height = item.height ?: return false
        if (width < 1 || height < 1 || minOf(width, height) > MAX_SHORT_SIDE) return false
        val duration = item.durationMs ?: return false
        return duration >= 1
    }

    /** Whether a staged set goes with `animation: true`: only one animation, alone. */
    fun sendsAsAnimation(staged: List<MediaPrep.Prepared>): Boolean = staged.singleOrNull()?.animation == true
}

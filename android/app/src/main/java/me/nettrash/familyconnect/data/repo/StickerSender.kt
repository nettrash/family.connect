/*
 * StickerSender.kt
 * Family Connect (Android)
 *
 * One tap in the sticker panel, as a send (docs/protocol.md, "Sticker
 * pack" → "Sending one").
 *
 * A sent sticker is a COPY: the message carries its OWN `kind=photo`
 * attachment with `sticker: true`, and names no pack item. So sending is
 * two things this file joins and neither repository should know about the
 * other for: the pack's cached bytes ([PackRepository.stagedCopy]), handed
 * to the ordinary media send ([MessageRepository.sendMedia]) with the flag.
 *
 * NOTHING HERE PREPARES THE PICTURE. The bytes that go up are the bytes the
 * pack holds, with no preview — the photo path's downscale and JPEG
 * re-encode are simply never on this road.
 *
 * And because it IS the ordinary media send, it is queued before the first
 * byte goes out: a sticker tapped with no network draws at once, waits in
 * the outbox, and goes when the network returns — even if the item was
 * removed from the pack in between, which the server deliberately does not
 * check.
 *
 * iOS counterpart: the sticker send on ChatSyncCoordinator.
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.flow.first
import me.nettrash.familyconnect.data.db.PackItemEntity
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class StickerSender @Inject constructor(
    private val pack: PackRepository,
    private val messages: MessageRepository,
    private val settings: SettingsRepository,
) {

    /** What became of the tap. Only [QUEUED] leaves a bubble; the others are said. */
    enum class Result {
        /** In the outbox: drawn now, delivered when the network allows. */
        QUEUED,

        /** This device does not hold the bytes and could not fetch them. */
        NO_BYTES,

        /**
         * Over the family's per-item ceiling, which "binds a sticker MESSAGE
         * too" (docs/protocol.md, "Limits"). Only reachable when an operator
         * lowered the ceiling under an item the pack already held — the
         * server would answer `invalid_attachment`, and a sentence now is
         * better than a red bubble that can never be retried into working.
         */
        TOO_LARGE,
    }

    /**
     * Send [item] to [chatId] as its own message — no caption, no
     * confirmation. It may be a reply, which is how one answers something.
     *
     * Everything after [Result.QUEUED] is the outbox's, and reports itself
     * on the bubble like any other send.
     */
    suspend fun send(item: PackItemEntity, chatId: Long, replyTo: ReplyToDto? = null): Result {
        val ceiling = settings.state.first().packLimits?.maxItemBytes
        val size = item.attachment?.size ?: 0L
        if (ceiling != null && size > ceiling) return Result.TOO_LARGE
        val prepared = pack.stagedCopy(item) ?: return Result.NO_BYTES
        val queued = messages.sendMedia(
            prepared = listOf(prepared),
            caption = "",
            chatId = chatId,
            replyTo = replyTo,
            sticker = true,
        )
        if (queued == null) {
            // Not staged, so the copy is nobody's: it must not sit in the
            // cache waiting for a sweep that does not know about it.
            prepared.file.delete()
            return Result.NO_BYTES
        }
        // Only now, and only on this device: which stickers somebody uses
        // is never on the wire.
        pack.noteUsed(item.id)
        return Result.QUEUED
    }
}

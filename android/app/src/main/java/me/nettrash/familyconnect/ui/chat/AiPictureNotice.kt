package me.nettrash.familyconnect.ui.chat

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

/**
 * What the composer must say, out loud, about the attachments a member has
 * staged in their own assistant chat (docs/protocol.md, "Pictures").
 *
 * The protocol asks for this in as many words: *"A client must say what it
 * is about to do, at the moment it matters. The switch lives on a settings
 * screen that somebody read once; the photograph is chosen in a composer,
 * later, by someone who may not have been the one who read it."* So the
 * notice hangs off the STAGED items rather than off the button that staged
 * them — the picker, a paste, a drop and the camera all end up in the same
 * list, and a notice attached to one door would be missing from the other
 * three.
 *
 * It is a plain value computed from kinds alone, with no Android in it, so
 * the rule can be pinned by an ordinary unit test rather than inferred from
 * a screenshot. Everything it reports is a decision the SERVER has already
 * made and that this client only mirrors — see the constants below, which
 * are the protocol's fixed limits and not this client's preferences.
 */
sealed interface AiPictureNotice {

    /**
     * Both locks are open, so these pixels will leave the server.
     *
     * [shown] is how many photographs the model will actually be given —
     * at most [MAX_PHOTOS], in the sender's order. It can be ZERO, when
     * everything staged is too large or in an encoding no deployment
     * reads; the strip then says the assistant will not be shown it, and
     * the counts below say why. The other three are the "left out"
     * explanation, and they are separate because they are left out for
     * different reasons and a member can act on some of them (send fewer,
     * send a smaller one) but not on others (a video is never a picture,
     * at any count).
     */
    data class WillShow(
        val shown: Int,
        /** Photographs past the cap, which stay on this server. */
        val extraPhotos: Int,
        /** Videos, files, audio and locations, which never travel at all. */
        val otherAttachments: Int,
        /**
         * Photographs the server will leave out for what they ARE rather
         * than for how many there are: over [MAX_BYTES] once the preview
         * has been preferred, or in an encoding no chat deployment reads
         * ([ACCEPTED_MIME_TYPES]).
         *
         * A fourth count rather than folded into [extraPhotos], because a
         * member can act on one of them and not on the other — "send
         * fewer" fixes a cap and fixes nothing here.
         */
        val unreadablePhotos: Int,
    ) : AiPictureNotice

    /**
     * A photograph is staged, and it will NOT be shown: either this
     * server has no deployment that can see, or the family's owner has
     * not turned `ai_vision` on.
     *
     * Not an error and not a refusal — the message sends exactly as it
     * would anywhere else, and the assistant is simply told a photo was
     * attached ("it becomes `[photo]` like an older one, and the
     * assistant answers that it was not shown it"). Saying so is the
     * difference between a silent no-op and an explained one; the wording
     * deliberately does not say WHICH lock is shut, because a member who
     * is not the owner cannot open either and naming the configuration
     * would be a way to probe it.
     */
    data object WillNotShow : AiPictureNotice

    companion object {
        /**
         * Photographs shown to the assistant with one question. FIXED by
         * the protocol — not a client preference, and not negotiable per
         * server (docs/protocol.md, limits table).
         */
        const val MAX_PHOTOS = 4

        /**
         * The largest ONE photograph may be, after the preview has been
         * preferred over the original: 5 MiB, exactly. Fixed by the
         * protocol in the same way [MAX_PHOTOS] is.
         *
         * A rule this client states and does not apply would be worse than
         * no rule: the strip would promise, in as many words, that a
         * photograph the server is about to leave out "leaves this server
         * for the model your server talks to".
         */
        const val MAX_BYTES = 5L * 1024 * 1024

        /** The only two encodings a chat deployment reads. */
        val ACCEPTED_MIME_TYPES = listOf("image/jpeg", "image/png")

        /**
         * The media type this item will TRAVEL as.
         *
         * The server prefers the PREVIEW, and a preview is a JPEG by
         * definition (docs/protocol.md, "Photos, videos, audio, files and
         * locations") — so the encoding to judge by is not always the one
         * that was uploaded. This is the whole reason the HEIC rule almost
         * never fires here.
         */
        fun wireMime(mime: String, hasPreview: Boolean): String =
            if (hasPreview) "image/jpeg" else mime

        /**
         * The bytes this item will TRAVEL as, by the same rule.
         *
         * [originalBytes] is a lambda so the file is not measured when
         * there is a preview — which is every photograph this app makes,
         * and the strip is recomputed on every staging change.
         */
        fun wireBytes(previewBytes: Int?, originalBytes: () -> Long): Long =
            previewBytes?.toLong() ?: originalBytes()

        /**
         * Will the model be SHOWN this one, or only told it is here?
         *
         * Kind first, because it is the rule that turns most people away;
         * then the encoding and the size, both judged on what will be on
         * the wire rather than on what was picked.
         */
        fun isShownToModel(kind: String, mime: String, bytes: Long): Boolean =
            kind == AttachmentDto.KIND_PHOTO &&
                mime.lowercase() in ACCEPTED_MIME_TYPES &&
                bytes <= MAX_BYTES

        /**
         * The notice for a set of staged attachments, or null when there
         * is nothing to say.
         *
         * Takes THREE FACTS per item rather than the staged objects
         * themselves: a signature that cannot see a file handle cannot
         * accidentally start depending on one, and these three are the
         * whole of what the server's rule reads (docs/protocol.md,
         * "Pictures").
         *
         * @param staged the staged attachments, in the order the message
         *   will carry them — which is the order the server reads them in,
         *   so the first [MAX_PHOTOS] readable photographs here are exactly
         *   the ones it will show. Each carries the type and the length it
         *   will have ON THE WIRE (see [wireMime] and [wireBytes]).
         * @param allowed whether BOTH locks are open — this server can
         *   see, and this family allows it.
         */
        fun of(staged: List<StagedPicture>, allowed: Boolean): AiPictureNotice? {
            val photos = staged.filter { it.kind == AttachmentDto.KIND_PHOTO }
            // Nothing photographic staged: nothing about pictures to say.
            // A video alone in an assistant chat gets no notice, because
            // the sentence would be about a disclosure that is not on the
            // table — this app does not narrate every attachment.
            if (photos.isEmpty()) return null
            if (!allowed) return WillNotShow
            // The server's own rule, in the server's own order: what can
            // travel at all, and then the first four of those.
            val readable = photos.filter { isShownToModel(it.kind, it.mime, it.bytes) }
            return WillShow(
                shown = minOf(readable.size, MAX_PHOTOS),
                extraPhotos = (readable.size - MAX_PHOTOS).coerceAtLeast(0),
                otherAttachments = staged.size - photos.size,
                unreadablePhotos = photos.size - readable.size,
            )
        }
    }
}

/**
 * One staged item, reduced to the three facts the picture rule reads.
 *
 * [mime] and [bytes] are what the item will be ON THE WIRE — the preview's
 * type and length where a preview exists, because that is the copy the
 * server prefers (docs/protocol.md, "Pictures"). Reducing it here is what
 * keeps [AiPictureNotice] free of files, Uris and bitmaps, so the rule can
 * be pinned by an ordinary unit test.
 */
data class StagedPicture(
    val kind: String,
    val mime: String,
    val bytes: Long,
)

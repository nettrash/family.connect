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
         * server (docs/protocol.md, limits table). In the family chat the
         * same four are ONE budget across an `@ai` message and the message
         * it replies to (#56), the mention's own first — not four each.
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
         * the wire rather than on what was picked. A null [bytes] is "not
         * known here" — the honest answer for a preview on somebody
         * else's message, whose length is not on the wire — and the size
         * rule then goes unapplied rather than guessed at; the two rules
         * that ARE knowable still decide it.
         */
        fun isShownToModel(kind: String, mime: String, bytes: Long?): Boolean =
            kind == AttachmentDto.KIND_PHOTO &&
                mime.lowercase() in ACCEPTED_MIME_TYPES &&
                (bytes == null || bytes <= MAX_BYTES)

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

        /**
         * The FAMILY composer's notice for an `@ai` draft about to carry a
         * photograph, or null because there is nothing to say (#56;
         * docs/protocol.md, "Showing the assistant a picture from the
         * family chat" — "What a client's family-chat composer must say").
         *
         * Until #56 a photo in the family chat never reached the assistant
         * and this composer had nothing to say. Now a photo on an `@ai`
         * message, or on the message it replies to, goes to the model
         * under the same two locks a private question needs — and the
         * doctrine that put [of] above a staged photo puts this here for
         * the same reason: the switch was read once, on a settings screen,
         * possibly by somebody else; the photo is chosen now.
         *
         * Null is the common answer, and every reason for it is a fact the
         * sentence would otherwise lie about: either lock shut (nothing
         * leaves — and this composer does not name a lock a non-owner
         * cannot open); no mention in the draft (an ordinary attachment on
         * an ordinary message); `@ai /draw …` on a server that can draw (a
         * picture request sends the words after the token and nothing
         * else); no photograph on the draft and none on the quote.
         *
         * The counting is the SERVER's, which is what makes the "4" the
         * strip shows honest: the mention's own readable photos take their
         * places first, the quoted message's fill what is left of
         * [MAX_PHOTOS], never four each; a photo that cannot travel at all
         * spends no place and is counted as [MentionPictureNotice.unreadablePhotos].
         *
         * @param draft the composer's text as typed.
         * @param staged what is staged on the draft, reduced as [of] takes it.
         * @param quoted the attachments of the message being replied to,
         *   in the sender's order, reduced by [StagedPicture.of]; empty
         *   when the draft replies to nothing.
         * Since the owner's THIRD switch (`ai_history_photos`,
         * docs/protocol.md, "Recent photos from the family chat") the
         * same rule also says, when that switch is on and `ai_history`
         * with it, that the chat's most recent photos may go too — "up
         * to N", where N is what the draft and the quote left of the same
         * four, because the server fills history LAST. The strip cannot
         * say WHICH photos (the server decides that from the transcript
         * it builds when it answers), so it says "up to" and names none;
         * and under that switch it shows for an `@ai` draft with no photo
         * of its own, the case #56's strip was absent for, since that is
         * precisely the mention on which all four may be somebody else's.
         *
         * @param allowed whether BOTH locks are open.
         * @param serverCanDraw whether `@ai /draw` would be a picture
         *   request here rather than an ordinary mention.
         * @param familyHistory the family's `ai_history`. Without it there
         *   is no transcript, so the third switch is inert and nothing
         *   about recent photos is said, at any value of it.
         * @param familyHistoryPhotos the family's `ai_history_photos`.
         */
        fun forMention(
            draft: String,
            staged: List<StagedPicture>,
            quoted: List<StagedPicture>,
            allowed: Boolean,
            serverCanDraw: Boolean,
            familyHistory: Boolean = true,
            familyHistoryPhotos: Boolean = false,
        ): MentionPictureNotice? {
            if (!allowed) return null
            if (!AssistantMention.mentions(draft)) return null
            if (serverCanDraw && AssistantMention.drawPrompt(draft) != null) return null
            // The third switch does nothing on its own: both locks above
            // (checked already), and a transcript to take photos from.
            val recentPhotos = familyHistory && familyHistoryPhotos
            val onMention = staged.filter { it.kind == AttachmentDto.KIND_PHOTO }
            val onQuote = quoted.filter { it.kind == AttachmentDto.KIND_PHOTO }
            if (onMention.isEmpty() && onQuote.isEmpty() && !recentPhotos) return null
            // The server's rule in the server's order: what can travel at
            // all, then the mention's first and the quote's in what is
            // left (handlers_ai.rs, `vision_images` twice, one budget) —
            // and the transcript's newest fill only what remains, the same
            // four and never four more.
            val readableOnMention = onMention.count { isShownToModel(it.kind, it.mime, it.bytes) }
            val readableOnQuote = onQuote.count { isShownToModel(it.kind, it.mime, it.bytes) }
            val shownOnMention = minOf(readableOnMention, MAX_PHOTOS)
            val shownOnQuote = minOf(readableOnQuote, MAX_PHOTOS - shownOnMention)
            return MentionPictureNotice(
                shownOnMention = shownOnMention,
                shownOnQuote = shownOnQuote,
                extraPhotos = (readableOnMention - shownOnMention) + (readableOnQuote - shownOnQuote),
                otherAttachments = (staged.size - onMention.size) + (quoted.size - onQuote.size),
                unreadablePhotos = (onMention.size - readableOnMention) + (onQuote.size - readableOnQuote),
                recentUpTo = if (recentPhotos) MAX_PHOTOS - shownOnMention - shownOnQuote else null,
            )
        }
    }
}

/**
 * The family composer's strip while an `@ai` draft carries a photograph —
 * the counts, decided by [AiPictureNotice.forMention] exactly as the
 * server will decide them, and read by the strip into sentences.
 *
 * Its own type rather than a fourth case of [AiPictureNotice] because it
 * answers a different question: not "what will happen to what I staged"
 * but "what will happen to what I staged AND what I am replying to", with
 * one budget across the two and a sentence that has to say which.
 */
data class MentionPictureNotice(
    /**
     * Photographs the model will be shown off the draft itself. First,
     * because they are the ones the member chose just now.
     */
    val shownOnMention: Int,
    /**
     * Photographs it will be shown off the message being replied to —
     * whatever the draft's own left of [AiPictureNotice.MAX_PHOTOS].
     */
    val shownOnQuote: Int,
    /** Photographs past the shared budget: named to the model, not shown. */
    val extraPhotos: Int,
    /** Videos, files, audio and locations on either message: never sent. */
    val otherAttachments: Int,
    /**
     * Photographs that cannot travel at all — over [AiPictureNotice.MAX_BYTES],
     * or in an encoding no chat deployment reads: told, never shown.
     */
    val unreadablePhotos: Int,
    /**
     * How many of the chat's most recent photographs may ALSO go — what
     * the draft and the quote left of [AiPictureNotice.MAX_PHOTOS] — or
     * null when the owner's `ai_history_photos` is not in effect (off, or
     * `ai_history` off, so there is no transcript for it to widen). Zero
     * is a real value: the switch is on and the member's own photos have
     * spent the budget, so no recent photo travels and the strip may
     * still promise that no other photo in this chat does.
     */
    val recentUpTo: Int? = null,
) {
    val shown: Int get() = shownOnMention + shownOnQuote
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
    /**
     * The length on the wire, or null when this client cannot know it —
     * a preview on somebody else's message, whose length is not on the
     * wire (see [of]).
     */
    val bytes: Long?,
) {
    companion object {
        /**
         * An attachment as it sits on a message in the chat — the message
         * a member is replying to, usually somebody else's — reduced the
         * way the SERVER will read it: the preview's type (a JPEG by
         * definition) and an unknown length where a preview exists, and
         * the original's own type and stored size where none does, which
         * is exactly what the server will measure.
         */
        fun of(attachment: AttachmentDto): StagedPicture = StagedPicture(
            kind = attachment.kind,
            mime = AiPictureNotice.wireMime(attachment.mime, hasPreview = attachment.hasPreview),
            bytes = if (attachment.hasPreview) null else attachment.size,
        )
    }
}

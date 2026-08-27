/*
 * ShareIn.kt
 * Family Connect (Android)
 *
 * What an OS share should become, decided before a single byte is read —
 * the share-sheet sibling of [PastedMedia.decide], and deliberately the
 * same shape: a pure verdict over descriptions (schemes and media types),
 * so every branch is a plain unit test and nothing opens a ContentResolver
 * to make up its mind.
 *
 * The one rule that differs from the clipboard: a share is addressed to a
 * CHAT, so the verdict never lands anything itself — MainViewModel walks
 * it, copies the bytes to cache immediately (share grants are transient),
 * and parks the result in [ShareStash] until a chat is picked. Nothing
 * auto-sends: shared items arrive STAGED in the composer.
 *
 * iOS counterpart: the share-extension import behind pendingShareImport.
 */

package me.nettrash.familyconnect.data.repo

import me.nettrash.familyconnect.data.net.dto.AttachmentDto

object ShareIn {

    /** One shared stream, as the intent DESCRIBES it — no bytes. */
    data class Stream(val scheme: String?, val mime: String?)

    sealed interface Verdict {
        /**
         * Stage the streams at [indices] (in the sender's order, capped at
         * [AttachmentDto.MAX_PER_MESSAGE]); [dropped] says how many were
         * cut by the cap, so the caller can say so instead of losing them
         * silently.
         */
        data class Attach(val indices: List<Int>, val dropped: Int) : Verdict

        /**
         * Put these words in the composer — the shared-text rule: a
         * share with EXTRA_TEXT and no readable stream is TEXT, never an
         * attachment, exactly as a pasted sentence is (PastedMedia).
         */
        data class Words(val text: String) : Verdict

        /** Nothing this app can take. */
        data object Nothing : Verdict
    }

    /**
     * Attachable streams WIN over the text riding beside them, exactly as
     * an attachable clipboard item wins over the words in the same clip —
     * a "share image" from a gallery often carries a caption or a URL as
     * EXTRA_TEXT, and nobody wants that typed next to the picture.
     */
    fun decide(streams: List<Stream>, text: String?): Verdict {
        val readable = streams.withIndex()
            .filter { PastedMedia.isReadable(it.value.scheme) }
            .map { it.index }
        if (readable.isNotEmpty()) {
            val kept = readable.take(AttachmentDto.MAX_PER_MESSAGE)
            return Verdict.Attach(indices = kept, dropped = readable.size - kept.size)
        }
        val words = text?.takeIf { it.isNotBlank() }
        return if (words != null) Verdict.Words(words) else Verdict.Nothing
    }
}

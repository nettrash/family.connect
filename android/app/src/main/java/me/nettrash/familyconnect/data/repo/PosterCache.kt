/*
 * PosterCache.kt
 * Family Connect (Android)
 *
 * The narrow seam between "a video was just sent" and "this device still
 * holds its poster" (issue #54).
 *
 * A video's poster goes up in its OWN request, after the bytes and before
 * the message that claims them, and it is best-effort by design: a
 * thumbnail must never cost the send. Until #54 best-effort also meant
 * exactly once — a failed `PUT /attachments/{id}/preview` left the server
 * with `has_preview = false` and nobody able to correct it, because the
 * only device holding the pixels had already thrown them away. A video's
 * bytes are a video, so there is no second source to fall back on: every
 * recipient got a grey tile, for good.
 *
 * [MessageRepository] is where the outcome is known and [AttachmentRepository]
 * is where the pixels live; this is the one call between them. An
 * interface rather than the repository itself, for the same reason
 * `ShareImporter` is one: it keeps a Context, a disk cache and an HTTP
 * client out of every send test (see AppModule).
 *
 * iOS counterpart: the same two calls on `AttachmentStore`
 * (`seed(_:id:preview:)` and `notePosterUpload(id:landed:)`).
 */

package me.nettrash.familyconnect.data.repo

interface PosterCache {

    /**
     * Keep a poster this device just made.
     *
     * Two things at once, and both matter. The sender's own bubble draws
     * it immediately instead of downloading back bytes it produced a
     * moment ago — which is what iOS has always done — and, more to the
     * point, the bytes are still here to re-send if the upload did not
     * land.
     */
    suspend fun seedPoster(attachmentId: Long, jpeg: ByteArray)

    /**
     * Say what became of that poster's upload.
     *
     * [landed] false leaves a note for the next repair pass to find;
     * true removes one, so a poster that eventually lands stops being
     * offered. Videos only: a photo whose preview is lost still shows its
     * full bytes, so it costs bandwidth rather than the picture.
     */
    suspend fun notePosterUpload(attachmentId: Long, landed: Boolean)

    /**
     * Keep the ORIGINAL bytes of a picture this device is sending, under
     * the id the bubble will ask for.
     *
     * A STICKER's one call (docs/protocol.md, "Sticker pack"). A sticker
     * has no preview — a preview is a JPEG, which is the destruction the
     * whole design avoids — so its bubble draws from the original bytes and
     * nothing else. Seeded under the placeholder id so the sender's bubble
     * draws before the upload, and again under the server's id so it does
     * not download back bytes it sent a moment ago.
     *
     * Defaulted to nothing: a send test that is not about stickers needs no
     * disk for it.
     */
    suspend fun seedOriginal(attachmentId: Long, source: java.io.File) {}
}

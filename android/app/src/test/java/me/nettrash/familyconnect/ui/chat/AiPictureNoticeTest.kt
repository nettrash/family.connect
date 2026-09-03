/*
 * AiPictureNoticeTest.kt
 * Family Connect (Android)
 *
 * What the composer must SAY about the photographs staged in a member's
 * own assistant chat (docs/protocol.md, "Pictures").
 *
 * The rule is a privacy rule, not a formatting one — "a client must say
 * what it is about to do, at the moment it matters" — so the interesting
 * cases here are the ones where the sentence would be a lie: a picture
 * that will not actually be shown, and pictures that will be left out.
 */

package me.nettrash.familyconnect.ui.chat

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test

class AiPictureNoticeTest {

    /**
     * A staged photograph as it will be ON THE WIRE: this app re-encodes
     * every photo to JPEG and makes a 600-pixel preview, and the preview
     * is the copy the server prefers, so this is what the rule actually
     * meets in production.
     */
    private fun photo(bytes: Long = 40_000L, mime: String = "image/jpeg") =
        StagedPicture(kind = AttachmentDto.KIND_PHOTO, mime = mime, bytes = bytes)

    private val video =
        StagedPicture(kind = AttachmentDto.KIND_VIDEO, mime = "video/mp4", bytes = 900_000L)
    private val file =
        StagedPicture(kind = AttachmentDto.KIND_FILE, mime = "application/pdf", bytes = 1_000L)

    @Test
    fun `nothing staged says nothing`() {
        assertThat(AiPictureNotice.of(emptyList(), allowed = true)).isNull()
        assertThat(AiPictureNotice.of(emptyList(), allowed = false)).isNull()
    }

    /**
     * A video alone raises no notice. The sentence would be about a
     * disclosure that is not on the table: a video is never shown to the
     * assistant at any setting, so there is nothing here the member has
     * to decide about.
     */
    @Test
    fun `attachments that are not photographs say nothing on their own`() {
        assertThat(AiPictureNotice.of(listOf(video, file), allowed = true)).isNull()
        assertThat(AiPictureNotice.of(listOf(video), allowed = false)).isNull()
    }

    @Test
    fun `one photograph with both locks open is shown`() {
        val notice = AiPictureNotice.of(listOf(photo()), allowed = true)
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
    }

    /**
     * The lock the member cannot see. Either the operator configured no
     * vision deployment or the owner never turned `ai_vision` on — the
     * notice does not distinguish them, because a member who is not the
     * owner can open neither and naming the configuration would be a way
     * to probe it.
     */
    @Test
    fun `a photograph with either lock shut is not shown`() {
        assertThat(AiPictureNotice.of(listOf(photo()), allowed = false))
            .isEqualTo(AiPictureNotice.WillNotShow)
        assertThat(AiPictureNotice.of(listOf(photo(), photo(), video), allowed = false))
            .isEqualTo(AiPictureNotice.WillNotShow)
    }

    /**
     * The cap is the protocol's, fixed at four, and what is over it is
     * NAMED rather than silently dropped — the same doctrine the server
     * follows towards the model, pointed at the member instead.
     */
    @Test
    fun `past the cap the rest are named as left out`() {
        val notice = AiPictureNotice.of(List(6) { photo() }, allowed = true)
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 4, extraPhotos = 2, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
    }

    /** Exactly at the cap nothing is left out, so nothing is claimed to be. */
    @Test
    fun `exactly the cap leaves nothing out`() {
        assertThat(
            AiPictureNotice.of(List(AiPictureNotice.MAX_PHOTOS) { photo() }, allowed = true),
        ).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 4, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
            ),
        )
    }

    /**
     * The two "left out" reasons are counted apart, because a member can
     * act on one of them (send fewer photos) and not on the other (a
     * video is not a picture at any count).
     */
    @Test
    fun `photographs and everything else are counted apart`() {
        val notice = AiPictureNotice.of(
            listOf(photo(), video, photo(), file, photo(), photo(), photo()),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 4, extraPhotos = 1, otherAttachments = 2, unreadablePhotos = 0,
            ),
        )
    }

    /**
     * The non-photographs must not consume a photograph's place: the
     * server selects `kind = 'photo'` rows only and counts the first four
     * of THOSE, so a message of one photo and nine files shows the photo.
     */
    @Test
    fun `other attachments never crowd a photograph out`() {
        val notice = AiPictureNotice.of(
            listOf(file, file, file, file, file, photo()),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 1, extraPhotos = 0, otherAttachments = 5, unreadablePhotos = 0,
            ),
        )
    }

    // -- The bounds this client used to hold and never apply ---------------

    /**
     * THE 5 MiB CEILING. It was a number nobody consulted: the strip said
     * a staged photograph "leaves this server for the model your server
     * talks to" whatever its size, while the server would leave a 6 MiB
     * one out and say so to the model but not to the member.
     */
    @Test
    fun `a photograph over the ceiling is named as left out, not promised`() {
        val notice = AiPictureNotice.of(
            listOf(photo(), photo(bytes = AiPictureNotice.MAX_BYTES + 1)),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 1,
            ),
        )
    }

    /** Exactly at the ceiling still travels — the rule is "over", not "at". */
    @Test
    fun `exactly the ceiling still travels`() {
        assertThat(AiPictureNotice.of(listOf(photo(bytes = AiPictureNotice.MAX_BYTES)), allowed = true))
            .isEqualTo(
                AiPictureNotice.WillShow(
                    shown = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 0,
                ),
            )
    }

    /**
     * An encoding no chat deployment reads. It counts as left out and is
     * said out loud, exactly as the server says it to the model — and the
     * case is real for a photo that arrived without a preview.
     */
    @Test
    fun `an encoding the model cannot read is named as left out`() {
        val notice = AiPictureNotice.of(
            listOf(photo(mime = "image/heic"), photo(mime = "image/png")),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 1, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 1,
            ),
        )
    }

    /**
     * NOTHING travels: the locks are open and the strip still has to say
     * the assistant will not be shown it, or the sentence is a promise
     * about pixels that stay here.
     */
    @Test
    fun `when nothing can travel the count is zero and the reason is named`() {
        val notice = AiPictureNotice.of(
            listOf(photo(mime = "image/heic"), photo(bytes = AiPictureNotice.MAX_BYTES + 1)),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 0, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 2,
            ),
        )
    }

    /**
     * The cap counts what CAN travel, not what was staged: four readable
     * photographs plus an unreadable one leaves nothing over the cap.
     */
    @Test
    fun `the cap counts what can travel and not what was staged`() {
        val notice = AiPictureNotice.of(
            List(4) { photo() } + photo(mime = "image/gif"),
            allowed = true,
        )
        assertThat(notice).isEqualTo(
            AiPictureNotice.WillShow(
                shown = 4, extraPhotos = 0, otherAttachments = 0, unreadablePhotos = 1,
            ),
        )
    }

    /**
     * What actually travels is the PREVIEW where the client made one — so
     * that, and not the original, is what the rule is judged on. It is
     * also why the HEIC rule almost never fires on this platform: a
     * preview is a JPEG by definition.
     */
    @Test
    fun `the preview is what is judged because it is what travels`() {
        assertThat(AiPictureNotice.wireMime("image/heic", hasPreview = true))
            .isEqualTo("image/jpeg")
        assertThat(AiPictureNotice.wireMime("image/heic", hasPreview = false))
            .isEqualTo("image/heic")
        assertThat(AiPictureNotice.wireBytes(40_000) { error("not measured") })
            .isEqualTo(40_000L)
        assertThat(AiPictureNotice.wireBytes(null) { 9_000_000L })
            .isEqualTo(9_000_000L)
    }

    @Test
    fun `the fixed bounds are the protocol's own numbers`() {
        assertThat(AiPictureNotice.MAX_PHOTOS).isEqualTo(4)
        assertThat(AiPictureNotice.MAX_BYTES).isEqualTo(5L * 1024 * 1024)
        assertThat(AiPictureNotice.ACCEPTED_MIME_TYPES)
            .containsExactly("image/jpeg", "image/png").inOrder()
    }
}

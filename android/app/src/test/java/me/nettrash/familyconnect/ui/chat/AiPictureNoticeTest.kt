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

    // -- The family composer's strip (#56) --------------------------------------

    /**
     * The FAMILY composer's notice, for an `@ai` draft about to carry a
     * photograph (docs/protocol.md, "What a client's family-chat composer
     * must say"). Same doctrine as [AiPictureNotice.of], one more message
     * a photo can be on, and one budget across the two.
     */
    private fun forMention(
        draft: String = "@ai what is this?",
        staged: List<StagedPicture> = emptyList(),
        quoted: List<StagedPicture> = emptyList(),
        allowed: Boolean = true,
        serverCanDraw: Boolean = true,
    ) = AiPictureNotice.forMention(draft, staged, quoted, allowed, serverCanDraw)

    private fun mention(
        shownOnMention: Int,
        shownOnQuote: Int,
        extraPhotos: Int = 0,
        otherAttachments: Int = 0,
        unreadablePhotos: Int = 0,
    ) = MentionPictureNotice(shownOnMention, shownOnQuote, extraPhotos, otherAttachments, unreadablePhotos)

    /**
     * Either lock shut and nothing leaves — so nothing is announced, and
     * the family composer does not name a lock a non-owner cannot open.
     */
    @Test
    fun `the family strip is absent with either lock shut`() {
        assertThat(forMention(staged = listOf(photo()), allowed = false)).isNull()
        assertThat(forMention(quoted = listOf(photo()), allowed = false)).isNull()
        assertThat(forMention(staged = listOf(photo()), quoted = listOf(photo()), allowed = false)).isNull()
    }

    /** Without `@ai` the photo is an ordinary attachment on an ordinary message. */
    @Test
    fun `the family strip is absent when the draft does not mention the assistant`() {
        assertThat(forMention(draft = "what is this?", staged = listOf(photo()))).isNull()
        assertThat(forMention(draft = "", quoted = listOf(photo()))).isNull()
        // Not a mention by the server's grammar, so not one here.
        assertThat(forMention(draft = "mail anna@ai.example this", quoted = listOf(photo()))).isNull()
    }

    @Test
    fun `the family strip is absent with no photograph anywhere`() {
        assertThat(forMention()).isNull()
        assertThat(forMention(staged = listOf(video))).isNull()
        assertThat(forMention(quoted = listOf(video, file))).isNull()
    }

    /**
     * `@ai /draw …` sends the words after the token and nothing else — not
     * the photo — on a server that can draw; on one that cannot it is an
     * ordinary mention and the photo goes.
     */
    @Test
    fun `the family strip is absent for a picture request unless the server cannot draw`() {
        assertThat(forMention(draft = "@ai /draw a cat", staged = listOf(photo()))).isNull()
        assertThat(forMention(draft = "@ai /draw a cat", quoted = listOf(photo()))).isNull()
        assertThat(forMention(draft = "@ai /draw a cat", staged = listOf(photo()), serverCanDraw = false))
            .isEqualTo(mention(1, 0))
        // `/draw` anywhere but the front is just a word.
        assertThat(forMention(draft = "@ai what does /draw do?", staged = listOf(photo()))).isNotNull()
    }

    @Test
    fun `the family strip is present with a staged photo on an @ai draft`() {
        assertThat(forMention(staged = listOf(photo()))).isEqualTo(mention(1, 0))
        assertThat(forMention(draft = "@AI look", staged = listOf(photo(), photo()))).isEqualTo(mention(2, 0))
    }

    @Test
    fun `the family strip is present with a photo on the message being replied to`() {
        assertThat(forMention(quoted = listOf(photo()))).isEqualTo(mention(0, 1))
        // Other attachments on the quoted message are counted as such.
        assertThat(forMention(quoted = listOf(video, photo(), file)))
            .isEqualTo(mention(0, 1, otherAttachments = 2))
    }

    /**
     * THE SHARED BUDGET. Four across both messages, the mention's first,
     * exactly as the server calls `vision_images` twice with one budget —
     * never four each.
     */
    @Test
    fun `the four are counted across mention and quote, mention first`() {
        assertThat(forMention(staged = List(3) { photo() }, quoted = List(3) { photo() }))
            .isEqualTo(mention(3, 1, extraPhotos = 2))
        assertThat(forMention(staged = List(5) { photo() }, quoted = listOf(photo())))
            .isEqualTo(mention(4, 0, extraPhotos = 2))
        assertThat(forMention(staged = List(2) { photo() }, quoted = List(2) { photo() }))
            .isEqualTo(mention(2, 2))
        assertThat(forMention(quoted = List(6) { photo() }))
            .isEqualTo(mention(0, 4, extraPhotos = 2))
        assertThat(forMention(staged = List(3) { photo() }, quoted = List(3) { photo() })!!.shown)
            .isEqualTo(AiPictureNotice.MAX_PHOTOS)
    }

    /**
     * A photo that cannot travel spends no place and is said out loud —
     * the same two rules the server applies, on either message.
     */
    @Test
    fun `a photo that will not travel is counted as such on either message`() {
        val heic = photo(mime = "image/heic")
        val huge = photo(bytes = AiPictureNotice.MAX_BYTES + 1)
        assertThat(forMention(staged = listOf(heic))).isEqualTo(mention(0, 0, unreadablePhotos = 1))
        assertThat(forMention(quoted = listOf(huge))).isEqualTo(mention(0, 0, unreadablePhotos = 1))
        assertThat(forMention(staged = listOf(heic, photo(), photo()), quoted = listOf(huge, photo(), photo())))
            .isEqualTo(mention(2, 2, unreadablePhotos = 2))
        // A size this client cannot know is not a size that is over.
        assertThat(forMention(quoted = listOf(StagedPicture(AttachmentDto.KIND_PHOTO, "image/jpeg", bytes = null))))
            .isEqualTo(mention(0, 1))
    }

    // -- Recent photos: the owner's third switch --------------------------------

    /**
     * [forMention] with `ai_history_photos` in effect (docs/protocol.md,
     * "Recent photos from the family chat" — "What a client shows", the
     * strip's rule).
     */
    private fun recent(
        draft: String = "@ai what is this?",
        staged: List<StagedPicture> = emptyList(),
        quoted: List<StagedPicture> = emptyList(),
        allowed: Boolean = true,
        serverCanDraw: Boolean = true,
        familyHistory: Boolean = true,
    ) = AiPictureNotice.forMention(
        draft, staged, quoted, allowed, serverCanDraw,
        familyHistory = familyHistory, familyHistoryPhotos = true,
    )

    /**
     * Every #56 row above ran with the switch off and pinned a notice
     * whose [MentionPictureNotice.recentUpTo] is null — so the strip a
     * family that never turned it on reads is what #56 shipped. Explicit
     * here for the one case that could drift.
     */
    @Test
    fun `without the third switch nothing about recent photos is said`() {
        assertThat(forMention(staged = listOf(photo()))!!.recentUpTo).isNull()
        assertThat(forMention(staged = listOf(photo()), quoted = listOf(photo()))!!.recentUpTo).isNull()
        assertThat(forMention()).isNull()
    }

    /**
     * The switch does nothing on its own: both locks still have to be
     * open, and it needs a transcript — with `ai_history` off no photo
     * from the history can travel, so nothing about one is said.
     */
    @Test
    fun `the third switch is inert with a lock shut or without history`() {
        assertThat(recent(allowed = false)).isNull()
        assertThat(recent(staged = listOf(photo()), allowed = false)).isNull()
        assertThat(recent(familyHistory = false)).isNull()
        // …and with a photo of the member's own the strip is exactly #56's.
        assertThat(recent(staged = listOf(photo()), familyHistory = false)).isEqualTo(mention(1, 0))
        assertThat(recent(staged = listOf(photo()), familyHistory = false)!!.recentUpTo).isNull()
    }

    /**
     * THE CASE #56'S STRIP WAS ABSENT FOR: an `@ai` draft with no photo of
     * its own and none on its quote. Under the switch that is exactly the
     * mention on which all four may be somebody else's picture, so the
     * strip shows — with the whole budget left for history.
     */
    @Test
    fun `with the switch on a bare @ai draft shows the strip with all four left`() {
        assertThat(recent()).isEqualTo(mention(0, 0).copy(recentUpTo = AiPictureNotice.MAX_PHOTOS))
        // Still not without a mention, and still not for a picture
        // request on a server that can draw — `/draw` sends the words
        // after the token and nothing else, at every setting.
        assertThat(recent(draft = "what is this?")).isNull()
        assertThat(recent(draft = "@ai /draw a cat")).isNull()
        assertThat(recent(draft = "@ai /draw a cat", staged = listOf(photo()))).isNull()
        assertThat(recent(draft = "@ai /draw a cat", serverCanDraw = false)!!.recentUpTo).isEqualTo(4)
    }

    /**
     * THE ARITHMETIC. History fills only what the mention and the quote
     * left of the same four — N = 4, 3, 2, 1, 0 — exactly as the server
     * walks the transcript newest-first for "what is left". A photo the
     * member pointed at always beats an ambient one.
     */
    @Test
    fun `history takes what is left of the four`() {
        for (own in 0..4) {
            val notice = recent(staged = List(own) { photo() })!!
            assertThat(notice.shownOnMention).isEqualTo(own)
            assertThat(notice.recentUpTo).isEqualTo(AiPictureNotice.MAX_PHOTOS - own)
        }
        // Across the two messages, the mention's first, then the quote's.
        assertThat(recent(staged = listOf(photo()), quoted = listOf(photo()))!!.recentUpTo).isEqualTo(2)
        assertThat(recent(quoted = List(3) { photo() })!!.recentUpTo).isEqualTo(1)
        assertThat(recent(staged = List(2) { photo() }, quoted = List(2) { photo() })!!.recentUpTo).isEqualTo(0)
        // Past the budget nothing is left at all.
        val over = recent(staged = List(5) { photo() })!!
        assertThat(over.recentUpTo).isEqualTo(0)
        assertThat(over.extraPhotos).isEqualTo(1)
        // A photo that cannot travel spends no place, so history may still
        // fill it — and other attachments never did.
        val heic = recent(staged = listOf(photo(mime = "image/heic")))!!
        assertThat(heic.unreadablePhotos).isEqualTo(1)
        assertThat(heic.recentUpTo).isEqualTo(4)
        assertThat(recent(staged = listOf(photo(mime = "image/heic"), photo(), video))!!.recentUpTo).isEqualTo(3)
    }

    /**
     * An attachment on somebody's message, reduced as the server will read
     * it: a preview is a JPEG of unknown length; an original is judged by
     * its own type and stored size.
     */
    @Test
    fun `an attachment on a message is judged as it will travel`() {
        val previewed = AttachmentDto(
            id = 34, kind = AttachmentDto.KIND_PHOTO, mime = "image/heic", size = 9_000_000L,
            width = 4032, height = 3024, hasPreview = true,
        )
        assertThat(StagedPicture.of(previewed))
            .isEqualTo(StagedPicture(AttachmentDto.KIND_PHOTO, "image/jpeg", bytes = null))
        val original = previewed.copy(id = 35, hasPreview = false)
        assertThat(StagedPicture.of(original))
            .isEqualTo(StagedPicture(AttachmentDto.KIND_PHOTO, "image/heic", bytes = 9_000_000L))
        assertThat(forMention(quoted = listOf(StagedPicture.of(original)))!!.unreadablePhotos).isEqualTo(1)
        assertThat(forMention(quoted = listOf(StagedPicture.of(previewed)))!!.shownOnQuote).isEqualTo(1)
    }
}

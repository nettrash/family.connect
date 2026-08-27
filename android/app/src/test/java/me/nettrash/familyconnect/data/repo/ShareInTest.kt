/*
 * ShareInTest.kt
 * Family Connect (Android)
 *
 * What an OS share becomes, decided from descriptions alone — the same
 * kind of pure table PastedMediaTest pins for the clipboard:
 *
 *  - readable streams are extracted in the sender's order and capped at
 *    what one message may carry, with the overflow COUNTED so the flow
 *    can say so;
 *  - shared TEXT becomes composer text, never an attachment — and only
 *    when no readable stream came with it, because an attachable item
 *    wins over the words riding beside it;
 *  - a share carrying nothing this app can take says so.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import org.junit.Test

class ShareInTest {

    private fun stream(scheme: String? = "content", mime: String? = "image/jpeg") =
        ShareIn.Stream(scheme = scheme, mime = mime)

    @Test
    fun `shared streams are attached in the sender's order`() {
        val verdict = ShareIn.decide(listOf(stream(), stream(mime = "video/mp4")), text = null)
        assertThat(verdict).isEqualTo(ShareIn.Verdict.Attach(indices = listOf(0, 1), dropped = 0))
    }

    @Test
    fun `streams past the cap are dropped and counted`() {
        val verdict = ShareIn.decide(List(12) { stream() }, text = null)
        verdict as ShareIn.Verdict.Attach
        assertThat(verdict.indices).hasSize(AttachmentDto.MAX_PER_MESSAGE)
        assertThat(verdict.indices).isEqualTo((0 until AttachmentDto.MAX_PER_MESSAGE).toList())
        assertThat(verdict.dropped).isEqualTo(2)
    }

    /** A stream whose grant this app cannot open is not offered as one. */
    @Test
    fun `only readable streams count, and their original indices survive`() {
        val verdict = ShareIn.decide(
            listOf(
                stream(scheme = "https"),
                stream(),
                stream(scheme = null),
                stream(scheme = "file", mime = "application/pdf"),
            ),
            text = null,
        )
        assertThat(verdict).isEqualTo(ShareIn.Verdict.Attach(indices = listOf(1, 3), dropped = 0))
    }

    /** The text-vs-attachment rule: shared text/plain is composer TEXT. */
    @Test
    fun `shared text with no stream becomes words`() {
        assertThat(ShareIn.decide(emptyList(), "dinner at 7"))
            .isEqualTo(ShareIn.Verdict.Words("dinner at 7"))
    }

    /** An attachable stream WINS over the words riding beside it. */
    @Test
    fun `a stream beside text is the attachment and the text is dropped`() {
        val verdict = ShareIn.decide(listOf(stream()), "look at this")
        assertThat(verdict).isEqualTo(ShareIn.Verdict.Attach(indices = listOf(0), dropped = 0))
    }

    @Test
    fun `nothing shareable is nothing`() {
        assertThat(ShareIn.decide(emptyList(), null)).isEqualTo(ShareIn.Verdict.Nothing)
        assertThat(ShareIn.decide(emptyList(), "   ")).isEqualTo(ShareIn.Verdict.Nothing)
        // Unreadable streams and no text: still nothing.
        assertThat(ShareIn.decide(listOf(stream(scheme = "https")), null))
            .isEqualTo(ShareIn.Verdict.Nothing)
    }
}

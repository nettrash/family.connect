/*
 * ReportCarriedTest.kt
 * Family Connect (Android)
 *
 * WHAT THE OWNER'S INBOX SAYS A REPORTED MESSAGE CARRIED (docs/protocol.md,
 * "Reporting a member").
 *
 * A photo sent without a caption has an EMPTY body by design, and
 * "inappropriate" is very often exactly that message — so a row drawn from
 * the frozen excerpt alone is a reason word and two names. The wording is
 * the chat-list preview's own, and matches the web and Windows clients, so
 * one family's owner reads the same row whichever app they open.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.ReportedAttachmentDto
import org.junit.Test

class ReportCarriedTest {

    private fun of(kind: String, name: String? = null) =
        ReportedAttachmentDto(kind = kind, name = name)

    @Test
    fun `a caption-less photo still says what was reported`() {
        assertThat(MessageRepository.carried(listOf(of("photo")))).isEqualTo("Photo")
    }

    @Test
    fun `several photos become a count, as the chat list says it`() {
        assertThat(MessageRepository.carried(List(3) { of("photo") })).isEqualTo("3 Photos")
    }

    @Test
    fun `a video, a voice note and a place each say their kind`() {
        assertThat(MessageRepository.carried(listOf(of("video")))).isEqualTo("Video")
        assertThat(MessageRepository.carried(listOf(of("audio")))).isEqualTo("Audio")
        assertThat(MessageRepository.carried(listOf(of("location")))).isEqualTo("Location")
    }

    /**
     * A document's name is its whole identity — "attachment 34" tells a
     * moderator nothing — and one that arrived without a name still has to
     * say something.
     */
    @Test
    fun `a file says its own name, or the word for it`() {
        assertThat(MessageRepository.carried(listOf(of("file", "budget.pdf"))))
            .isEqualTo("budget.pdf")
        assertThat(MessageRepository.carried(listOf(of("file")))).isEqualTo("File")
        assertThat(MessageRepository.carried(listOf(of("file", "")))).isEqualTo("File")
    }

    /**
     * A kind added by a newer server must never leave the row blank: the
     * owner is the moderator, and this line is all a caption-less message
     * has to say for itself.
     */
    @Test
    fun `a kind this build has never heard of still draws`() {
        assertThat(MessageRepository.carried(listOf(of("hologram", "spin.h"))))
            .isEqualTo("spin.h")
        assertThat(MessageRepository.carried(listOf(of("hologram")))).isEqualTo("File")
    }

    /**
     * A report that names a PERSON carries no attachments, and neither does
     * one whose message retention has swept: there the frozen excerpt is the
     * whole row, and this line must simply not appear.
     */
    @Test
    fun `nothing carried draws no line`() {
        assertThat(MessageRepository.carried(emptyList())).isNull()
    }
}

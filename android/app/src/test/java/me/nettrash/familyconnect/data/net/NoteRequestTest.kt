/*
 * NoteRequestTest.kt
 * Family Connect (Android) — tests
 *
 * What a new note SENDS (docs/protocol.md, "Board"): the KIND is derived
 * from the fields that make each one meaningful, and the two lists are
 * sent by different rules — names absent when there are none, lines
 * present even when there are none.
 */

package me.nettrash.familyconnect.data.net

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.TaskLineRequest
import org.junit.Test

class NoteRequestTest {

    private fun request(
        mentions: List<MentionDto> = emptyList(),
        items: List<TaskLineRequest>? = null,
        attachmentId: Long? = null,
        startsAt: String? = null,
    ) = newNoteRequest(
        text = "Saturday", color = "green", size = "medium", font = "plain",
        x = 0.1, y = 0.2,
        attachmentId = attachmentId, startsAt = startsAt,
        mentions = mentions, items = items,
    )

    @Test
    fun `words on a sticker name no kind at all`() {
        val plain = request()
        assertThat(plain.kind).isNull()
        assertThat(plain.items).isNull()
        assertThat(plain.mentions).isNull()
    }

    @Test
    fun `the lines are what make a note a list, empty ones included`() {
        // An EMPTY list is a list: pinning the title and filling it in
        // later is how a list gets made, and dropping the field here
        // would pin a plain sticker instead.
        val blank = request(items = emptyList())
        assertThat(blank.kind).isEqualTo("tasks")
        assertThat(blank.items).isEmpty()

        val written = request(items = listOf(TaskLineRequest(text = "Milk")))
        assertThat(written.kind).isEqualTo("tasks")
        assertThat(written.items?.map { it.text }).containsExactly("Milk")
        // Ids are the server's: a created line carries none.
        assertThat(written.items?.single()?.id).isNull()
    }

    @Test
    fun `a picture and a start name their own kinds`() {
        assertThat(request(attachmentId = 7).kind).isEqualTo("photo")
        assertThat(request(startsAt = "2026-12-24T16:00:00Z").kind).isEqualTo("event")
        // A start wins over a picture: an event may carry a backdrop.
        assertThat(request(attachmentId = 7, startsAt = "2026-12-24T16:00:00Z").kind)
            .isEqualTo("event")
    }

    @Test
    fun `names are sent only when there are some`() {
        assertThat(request(mentions = listOf(MentionDto(2, "Anna"))).mentions)
            .containsExactly(MentionDto(2, "Anna"))
        assertThat(request(mentions = emptyList()).mentions).isNull()
    }
}

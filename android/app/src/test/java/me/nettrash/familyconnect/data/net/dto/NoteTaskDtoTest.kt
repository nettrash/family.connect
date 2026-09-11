/*
 * NoteTaskDtoTest.kt
 * Family Connect (Android) — tests
 *
 * A task list's wire shapes (docs/protocol.md, "Board").
 *
 * The JSON below is a TRANSCRIPT of a live server — a real `POST` and a
 * real tick — rather than a hand-written guess, which is the only kind of
 * fixture that can catch a field this client spells differently from the
 * server.
 */

package me.nettrash.familyconnect.data.net.dto

import com.google.common.truth.Truth.assertThat
import kotlinx.serialization.json.Json
import org.junit.Test

class NoteTaskDtoTest {

    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = false }

    @Test
    fun `a task list decodes as the server sends it`() {
        val note = json.decodeFromString<NoteDto>(
            """
            {"author_id": 2, "board_seq": 2, "color": "green", "content_seq": 1,
             "created_at": "2026-09-11T14:43:36.832551Z", "font": "plain", "id": 1,
             "items": [{"done": true, "done_by": 3, "id": 1, "text": "Milk"},
                       {"done": false, "id": 2, "text": "Bread"}],
             "kind": "tasks", "size": "medium", "text": "Saturday",
             "updated_at": "2026-09-11T14:43:36.841601Z", "x": 0.2, "y": 0.3}
            """.trimIndent(),
        )

        assertThat(note.kind).isEqualTo("tasks")
        assertThat(note.items).containsExactly(
            TaskItemDto(id = 1, text = "Milk", done = true, doneBy = 3L),
            TaskItemDto(id = 2, text = "Bread", done = false, doneBy = null),
        ).inOrder()
    }

    @Test
    fun `an empty list is a list, and another kind carries no lines`() {
        val blank = json.decodeFromString<NoteDto>(
            """{"id": 2, "board_seq": 3, "kind": "tasks", "text": "Sunday", "items": []}""",
        )
        assertThat(blank.items).isEmpty()
        val plain = json.decodeFromString<NoteDto>(
            """{"id": 3, "board_seq": 4, "text": "Milk"}""",
        )
        assertThat(plain.items).isNull()
    }

    /**
     * What the store keeps: `[]` and null are DIFFERENT notes, so the
     * codec must not collapse them (a list nobody has written into is
     * still a list).
     */
    @Test
    fun `the codec round-trips an empty list and a full one`() {
        val items = listOf(TaskItemDto(id = 1, text = "Milk", done = true, doneBy = 3L))
        assertThat(TaskItemsCodec.decode(TaskItemsCodec.encode(items))).isEqualTo(items)
        assertThat(TaskItemsCodec.encode(emptyList())).isEqualTo("[]")
        assertThat(TaskItemsCodec.decode("[]")).isEmpty()
        // Null and rubbish both read as nothing, which is what a caller
        // that has to draw something does with them.
        assertThat(TaskItemsCodec.decode(null)).isEmpty()
        assertThat(TaskItemsCodec.decode("not json")).isEmpty()
    }

    /**
     * A line the AUTHOR sends: `id` only when it has one, because absent
     * is what says "this line is new" (docs/protocol.md, "Board").
     */
    @Test
    fun `a new line sends no id at all`() {
        assertThat(json.encodeToString(TaskLineRequest(text = "Milk")))
            .isEqualTo("""{"text":"Milk"}""")
        assertThat(json.encodeToString(TaskLineRequest(id = 11, text = "Oat milk")))
            .isEqualTo("""{"id":11,"text":"Oat milk"}""")
        assertThat(json.encodeToString(TaskDoneRequest(done = true)))
            .isEqualTo("""{"done":true}""")
    }
}

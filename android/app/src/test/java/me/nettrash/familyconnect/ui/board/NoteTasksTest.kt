/*
 * NoteTasksTest.kt
 * Family Connect (Android)
 *
 * What a STICKER draws of a task list, and what a SAVE sends of one
 * (docs/protocol.md, "Board").
 *
 * The two numbers here are shared with the other three clients — five
 * lines on the wall, twenty in a list — because a list that ran to a
 * different point on the phone and on the Mac would be a different list.
 *
 * Web counterpart: `fc_text::board::wall_task_lines`.
 * Apple counterpart: `BoardTasks` in Views/NoteSize.swift.
 */

package me.nettrash.familyconnect.ui.board

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.TaskLineRequest
import org.junit.Test

class NoteTasksTest {

    @Test
    fun `a sticker draws the first lines of a list and says how many are left`() {
        assertThat(NoteTasks.ON_WALL).isEqualTo(5)
        assertThat(NoteTasks.drawn(0)).isEqualTo(0 to 0)
        assertThat(NoteTasks.drawn(3)).isEqualTo(3 to 0)
        // A list of exactly the cap says nothing extra.
        assertThat(NoteTasks.drawn(5)).isEqualTo(5 to 0)
        assertThat(NoteTasks.drawn(6)).isEqualTo(5 to 1)
        assertThat(NoteTasks.drawn(20)).isEqualTo(5 to 15)
    }

    @Test
    fun `a save sends the lines that say something, with the ids they keep`() {
        val written = NoteTasks.written(
            listOf(
                DraftTaskLine(key = 1, itemId = 11, text = "  Oat milk "),
                DraftTaskLine(key = 2, itemId = null, text = "Bread"),
                // Somebody who started typing and stopped: not a thing to
                // do, and a line the server would refuse.
                DraftTaskLine(key = 3, itemId = null, text = "   "),
            ),
        )

        assertThat(written).containsExactly(
            TaskLineRequest(id = 11, text = "Oat milk"),
            TaskLineRequest(id = null, text = "Bread"),
        ).inOrder()
    }

    @Test
    fun `the limits are the server's own`() {
        assertThat(NoteTasks.MAX_ITEMS).isEqualTo(20)
        assertThat(NoteTasks.MAX_ITEM_CHARS).isEqualTo(100)
        // Counted the way the server counts them: code points, so a family
        // writing in emoji does not lose half its allowance.
        val emoji = "👩‍👩‍👧‍👦".repeat(40)
        val cut = NoteText.cappedTo(emoji, NoteTasks.MAX_ITEM_CHARS)
        assertThat(cut.codePointCount(0, cut.length)).isEqualTo(100)
        // And a line already under the limit comes back untouched.
        assertThat(NoteText.cappedTo("Milk", NoteTasks.MAX_ITEM_CHARS)).isEqualTo("Milk")
    }
}

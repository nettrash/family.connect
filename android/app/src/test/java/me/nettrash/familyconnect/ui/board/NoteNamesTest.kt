/*
 * NoteNamesTest.kt
 * Family Connect (Android)
 *
 * The names a note says (docs/protocol.md, "Board"): on the WALL a name is
 * a highlight and nothing more, and in the note somebody has OPENED it is
 * also a door — but only where there is somebody to open it with.
 *
 * The highlight is BOLD and carries no colour of its own, which is the
 * whole reason this is pinned: a sticker's pastel is a ground like any
 * other, and the tinted mention the chat draws was unreadable on it.
 *
 * Web counterpart: `named_runs` in web/src/views/board.rs.
 * Apple counterpart: `MemberMentionTests.noteNames`.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontWeight
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.MentionDto
import org.junit.Test

class NoteNamesTest {

    private val anna = MentionDto(2L, "Anna")
    private val bob = MentionDto(3L, "Bob")

    private fun AnnotatedString.bolded(): List<String> =
        spanStyles
            .filter { it.item.fontWeight == FontWeight.Bold }
            .map { text.substring(it.start, it.end) }

    private fun AnnotatedString.doors(): List<Pair<String, String>> =
        getLinkAnnotations(0, length)
            .mapNotNull { range ->
                (range.item as? LinkAnnotation.Clickable)?.let { link ->
                    text.substring(range.start, range.end) to link.tag
                }
            }

    @Test
    fun `a name the note says is bold and nothing else is`() {
        val drawn = NoteNames.annotate("Hi @Anna, see you", listOf(anna))

        assertThat(drawn.text).isEqualTo("Hi @Anna, see you")
        assertThat(drawn.bolded()).containsExactly("@Anna")
        // No colour: the sticker's own ink, whatever the pastel under it.
        assertThat(drawn.spanStyles.map { it.item.color }.toSet())
            .containsNoneOf(androidx.compose.ui.graphics.Color.Blue, androidx.compose.ui.graphics.Color.Red)
        assertThat(drawn.spanStyles.all { it.item.color == androidx.compose.ui.graphics.Color.Unspecified })
            .isTrue()
    }

    @Test
    fun `the wall never draws a door`() {
        val drawn = NoteNames.annotate("Hi @Anna and @Bob", listOf(anna, bob))

        assertThat(drawn.bolded()).containsExactly("@Anna", "@Bob")
        // A sticker's whole face is a drag handle: a name that took the tap
        // would make the wall hard to tidy (docs/protocol.md, "Board").
        assertThat(drawn.doors()).isEmpty()
    }

    @Test
    fun `an open note opens a chat with the name somebody tapped`() {
        val opened = mutableListOf<Long>()
        val drawn = NoteNames.reader(
            text = "Hi @Anna and @Bob",
            mentions = listOf(anna, bob),
            doors = setOf(anna.userId, bob.userId),
            onOpen = { opened += it },
        )

        val doors = drawn.doors()
        assertThat(doors.map { it.first }).containsExactly("@Anna", "@Bob").inOrder()
        // The private member scheme, so a tap reaches the member and not a
        // browser.
        assertThat(doors.map { it.second }).containsExactly("fcmember://2", "fcmember://3").inOrder()
        // Still bold, still no colour — a door is not a reason to tint it.
        assertThat(drawn.bolded()).containsExactly("@Anna", "@Bob")

        drawn.getLinkAnnotations(0, drawn.length).forEach { range ->
            val link = range.item as LinkAnnotation.Clickable
            link.linkInteractionListener?.onClick(link)
        }
        assertThat(opened).containsExactly(2L, 3L).inOrder()
    }

    @Test
    fun `a name with nobody behind it is a highlight and not a door`() {
        // The reader's own name, somebody they blocked, a member who has
        // left: all named, none openable (docs/protocol.md, "Board").
        val drawn = NoteNames.reader(
            text = "@Anna asked @Bob",
            mentions = listOf(anna, bob),
            doors = setOf(bob.userId),
            onOpen = {},
        )

        assertThat(drawn.bolded()).contains("@Anna")
        assertThat(drawn.doors().map { it.first }).containsExactly("@Bob")
    }

    @Test
    fun `a note that names nobody is drawn exactly as written`() {
        val plain = "Milk, bread, and the thing @ the shop"

        assertThat(NoteNames.annotate(plain, emptyList()).text).isEqualTo(plain)
        assertThat(NoteNames.annotate(plain, emptyList()).spanStyles).isEmpty()
        // A list that names somebody the text does not say marks nothing —
        // the server refuses to store one, and an old row is not a licence
        // to bold the wrong words.
        assertThat(NoteNames.annotate(plain, listOf(anna)).bolded()).isEmpty()
        assertThat(NoteNames.annotate(plain, listOf(anna)).text).isEqualTo(plain)
    }

    @Test
    fun `a longer name wins the token it shares`() {
        // The resolve rule, read out of the drawing: `@Anna Lee` is Anna
        // Lee and not also Anna (docs/protocol.md, "Mentioning a member").
        val annaLee = MentionDto(4L, "Anna Lee")
        val drawn = NoteNames.reader(
            text = "@Anna Lee and @Anna",
            mentions = listOf(anna, annaLee),
            doors = setOf(anna.userId, annaLee.userId),
            onOpen = {},
        )

        assertThat(drawn.doors().map { it.first }).containsExactly("@Anna Lee", "@Anna").inOrder()
        assertThat(drawn.doors().map { it.second }).containsExactly("fcmember://4", "fcmember://2").inOrder()
    }
}

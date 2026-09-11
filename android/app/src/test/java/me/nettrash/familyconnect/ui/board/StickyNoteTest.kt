/*
 * StickyNoteTest.kt
 * Family Connect (Android)
 *
 * What a STICKER on the wall draws of the names its note says
 * (docs/protocol.md, "Board"): the name is a highlight — bold, in the
 * note's own ink — and nothing more. No door: a sticker's whole face is a
 * drag handle, and a name that took that tap would make the wall hard to
 * tidy.
 *
 * NoteNamesTest pins the string; this pins that the wall actually draws
 * it, which is the half a mutant can otherwise walk through.
 *
 * A local Robolectric Compose test, like NoteDialogTest.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.dto.MentionDto
import me.nettrash.familyconnect.data.net.dto.NoteMentionsCodec
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class StickyNoteTest {

    @get:Rule
    val compose = createComposeRule()

    private fun SemanticsNodeInteraction.drawn(): AnnotatedString =
        fetchSemanticsNode().config[SemanticsProperties.Text].first()

    @Test
    fun theWallDrawsANameAsAHighlightAndNotAsADoor() {
        compose.setContent {
            StickyNote(
                note = NoteEntity(
                    id = 1L,
                    authorId = 9L,
                    text = "Milk please @Anna",
                    color = "yellow",
                    x = 0.1,
                    y = 0.1,
                    createdAt = 0L,
                    updatedAt = 0L,
                    boardSeq = 1L,
                    mentionsJson = NoteMentionsCodec.encode(listOf(MentionDto(2L, "Anna"))),
                ),
                authorName = "Bob",
                isHiddenByBlock = false,
                boardWidthPx = 1080,
                boardHeightPx = 1920,
                onMoved = { _, _ -> },
                onTap = {},
            )
        }

        val drawn = compose.onNodeWithText("Milk please @Anna").drawn()
        assertThat(
            drawn.spanStyles
                .filter { it.item.fontWeight == FontWeight.Bold }
                .map { drawn.text.substring(it.start, it.end) },
        ).containsExactly("@Anna")
        assertThat(drawn.getLinkAnnotations(0, drawn.length)).isEmpty()
    }
}

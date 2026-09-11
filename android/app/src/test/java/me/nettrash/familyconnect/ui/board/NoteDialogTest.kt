/*
 * NoteDialogTest.kt
 * Family Connect (Android)
 *
 * The note dialog's color swatches moved from a raw detectTapGestures to
 * Modifier.clickable (ripple + minimum-touch-target hit expansion +
 * selection semantics). This pins what that swap must not change: tapping
 * a swatch selects that color, the selection is published to semantics,
 * and Save hands the picked color back.
 *
 * The mention strip and the reader's doors are here for the same reason:
 * the strip is the only way a name gets into a note without being typed
 * exactly right, and a name in an OPEN note is the one place on the board
 * where a tap goes somewhere other than the note (docs/protocol.md,
 * "Board").
 *
 * The size row is the same contract one field over: a segmented button
 * selects a step, the selection is published to semantics, and Save hands
 * the picked size back beside the colour.
 *
 * A local Robolectric Compose test, not an instrumented one — same
 * reasoning as the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.click
import androidx.compose.ui.test.performTouchInput
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.TaskItemDto
import me.nettrash.familyconnect.data.net.dto.MentionDto
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class NoteDialogTest {

    @get:Rule
    val compose = createComposeRule()

    private fun draft() = NoteDraft(
        noteId = null,
        text = "hi",
        color = "yellow",
        size = "medium",
        font = "plain",
        x = 0.1,
        y = 0.1,
        authorId = 1L,
    )

    @Test
    fun tappingASwatchSelectsItAndSaveReportsThatColor() {
        var saved: List<String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size, font, _ -> saved = listOf(text, color, size, font) },
                onDelete = null,
            )
        }

        // The dialog opens with the draft's color selected.
        compose.onNodeWithContentDescription("Yellow").assertIsSelected()

        compose.onNodeWithContentDescription("Green").performClick()
        compose.onNodeWithContentDescription("Green").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(listOf("hi", "green", "medium", "plain"))
    }

    @Test
    fun tappingASizeSelectsItAndSaveReportsThatSize() {
        var saved: List<String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size, font, _ -> saved = listOf(text, color, size, font) },
                onDelete = null,
            )
        }

        // The dialog opens with the draft's size selected.
        compose.onNodeWithText("Medium").assertIsSelected()

        compose.onNodeWithText("Large").performClick()
        compose.onNodeWithText("Large").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(listOf("hi", "yellow", "large", "plain"))
    }

    /**
     * A name this client does not know draws as medium, and the picker says
     * so — but Save hands the NAME back untouched, the way an unknown colour
     * is, so opening a note to fix its text does not also shrink it.
     */
    @Test
    fun anUnknownSizeOpensWithMediumSelectedAndRoundTripsUntouched() {
        var saved: List<String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft().copy(size = "enormous"),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size, font, _ -> saved = listOf(text, color, size, font) },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Medium").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(listOf("hi", "yellow", "enormous", "plain"))
    }

    /** Once the author picks a step, that step wins over the unknown name. */
    @Test
    fun pickingAStepReplacesAnUnknownSize() {
        var saved: List<String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft().copy(size = "enormous"),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size, font, _ -> saved = listOf(text, color, size, font) },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Small").performClick()
        compose.onNodeWithText("Small").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(listOf("hi", "yellow", "small", "plain"))
    }

    // MARK: - the names a note says (docs/protocol.md, "Board")

    private val roster = listOf(MentionDto(2L, "Anna"), MentionDto(3L, "Bob"))

    @Test
    fun pickingANameOffTheStripWritesItIntoTheNote() {
        var saved: List<String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft().copy(text = ""),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size, font, _ -> saved = listOf(text, color, size, font) },
                roster = roster,
                onDelete = null,
            )
        }

        // Nothing offered until an `@` is being typed.
        compose.onAllNodesWithContentDescription("Mention Anna").assertCountEquals(0)

        compose.onNodeWithText("Note").performTextInput("Milk @An")
        compose.onNodeWithContentDescription("Mention Anna").assertIsDisplayed()
        // Narrowed as the name is typed: Bob is not what `@An` could be.
        compose.onAllNodesWithContentDescription("Mention Bob").assertCountEquals(0)

        compose.onNodeWithContentDescription("Mention Anna").performClick()
        compose.onNodeWithText("Save").performClick()

        // The half-typed name is REPLACED, not appended to, and the space
        // after it is what lets the next word start a sentence rather than
        // a longer name.
        assertThat(saved).isEqualTo(listOf("Milk @Anna ", "yellow", "medium", "plain"))
    }

    @Test
    fun aNameInAnOpenNoteOpensTheChatWithThatMember() {
        val opened = mutableListOf<Long>()
        var dismissed = false
        compose.setContent {
            NoteDialog(
                draft = draft().copy(
                    // The name FIRST, so the test can put the tap on it
                    // without knowing where the line ends.
                    text = "@Anna please bring milk",
                    mentions = listOf(MentionDto(2L, "Anna")),
                    authorId = 9L,
                ),
                // A reader, not the author: the note is drawn, not edited.
                canEdit = false,
                authorName = "Bob",
                onDismiss = { dismissed = true },
                onSave = { _, _, _, _, _ -> },
                roster = roster,
                onOpenChat = { opened += it },
                onDelete = null,
            )
        }

        compose.onNodeWithText("@Anna please bring milk").performTouchInput {
            click(Offset(left + 4f, top + 4f))
        }

        assertThat(opened).containsExactly(2L)
        assertThat(dismissed).isFalse()
    }

    // MARK: - task lists (docs/protocol.md, "Board")

    private fun listDraft() = NoteDraft(
        noteId = 5L,
        text = "Saturday",
        color = "green",
        size = "medium",
        font = "plain",
        kind = NoteKinds.TASKS,
        items = listOf(
            TaskItemDto(id = 11, text = "Milk", done = true, doneBy = 3L),
            TaskItemDto(id = 12, text = "Bread", done = false),
        ),
        x = 0.1,
        y = 0.1,
        authorId = 1L,
    )

    @Test
    fun aReaderTicksALineAndCannotRewriteIt() {
        val ticked = mutableListOf<Pair<Long, Boolean>>()
        compose.setContent {
            NoteDialog(
                draft = listDraft(),
                // Not the author: the boxes are still theirs to tap.
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                onTick = { itemId, done -> ticked += itemId to done },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Milk").assertIsDisplayed()
        compose.onNodeWithText("1 of 2 done").assertIsDisplayed()
        // A reader writes nothing: there is no field beside a box.
        compose.onAllNodesWithText("Thing to do").assertCountEquals(0)
        compose.onAllNodesWithText("Add a thing").assertCountEquals(0)

        // A STATE, not a toggle: the line that is done asks for false, the
        // one that is not asks for true. Found by the line each box is
        // LABELLED with — a bare box says nothing to a screen reader.
        compose.onNodeWithContentDescription("Bread").performClick()
        compose.onNodeWithContentDescription("Milk").performClick()

        assertThat(ticked).containsExactly(12L to true, 11L to false).inOrder()
    }

    @Test
    fun theAuthorWritesTheLinesAndSaveSendsTheIdsItKeeps() {
        var saved: List<DraftTaskLine>? = null
        compose.setContent {
            NoteDialog(
                draft = listDraft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { _, _, _, _, lines -> saved = lines },
                onDelete = null,
            )
        }

        // One field per line, each holding what the line says.
        val fields = compose.onAllNodesWithText("Thing to do")
        fields.assertCountEquals(2)
        compose.onNodeWithText("Add a thing").performClick()
        compose.onAllNodesWithText("Thing to do").assertCountEquals(3)
        compose.onAllNodesWithText("Thing to do")[2].performTextInput("Eggs")

        compose.onNodeWithText("Save").performClick()

        val lines = saved ?: error("nothing was saved")
        assertThat(lines.map { it.itemId }).containsExactly(11L, 12L, null).inOrder()
        assertThat(NoteTasks.written(lines).map { it.text })
            .containsExactly("Milk", "Bread", "Eggs").inOrder()
        // The ids it kept are what carry the ticks through the rewrite.
        assertThat(NoteTasks.written(lines).map { it.id })
            .containsExactly(11L, 12L, null).inOrder()
    }
}

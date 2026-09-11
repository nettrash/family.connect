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
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
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
import me.nettrash.familyconnect.data.net.dto.RsvpDto
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
                onTick = { itemId, done, _ -> ticked += itemId to done },
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

    /**
     * A line nobody has saved has no id yet, so there is nothing to tick:
     * the box is there — a row that grew one on save would jump under the
     * finger — and it is disabled, which is what says why
     * (docs/protocol.md, "Board").
     */
    @Test
    fun aLineNobodyHasSavedCannotBeTicked() {
        val ticked = mutableListOf<Pair<Long, Boolean>>()
        compose.setContent {
            NoteDialog(
                draft = listDraft().copy(noteId = null, items = emptyList()),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                onTick = { itemId, done, _ -> ticked += itemId to done },
                onDelete = null,
            )
        }

        // A new list opens with one empty row, and its box is dead.
        compose.onNodeWithContentDescription("Done").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Done").performClick()
        assertThat(ticked).isEmpty()
    }

    /**
     * A box is lit before the round trip, so a tick the server REFUSED has
     * to go back to what the note says (docs/protocol.md, "Board" — the
     * tick is a state, and the state is the family's).
     */
    @Test
    fun aRefusedTickGoesBackToWhatTheNoteSays() {
        compose.setContent {
            NoteDialog(
                draft = listDraft(),
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                // Nothing landed: the server said no.
                onTick = { _, _, onSettled -> onSettled(false) },
                onDelete = null,
            )
        }

        compose.onNodeWithContentDescription("Bread").performClick()

        compose.onNodeWithContentDescription("Bread").assertIsOff()
        // And the line that WAS done is still drawn done.
        compose.onNodeWithContentDescription("Milk").assertIsOn()
    }

    /** And one that landed keeps its mark, for the same reason. */
    @Test
    fun aTickThatLandedKeepsItsMark() {
        compose.setContent {
            NoteDialog(
                draft = listDraft(),
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                onTick = { _, _, onSettled -> onSettled(true) },
                onDelete = null,
            )
        }

        compose.onNodeWithContentDescription("Bread").performClick()

        // The draft this dialog opened with says Bread was NOT done, so a
        // mark dropped here would show the tick undoing itself.
        compose.onNodeWithContentDescription("Bread").assertIsOn()
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

    // MARK: - events (docs/protocol.md, "Board")

    private fun eventDraft(mine: Boolean = true) = NoteDraft(
        noteId = 9L,
        text = "Christmas dinner",
        color = "blue",
        size = "medium",
        font = "plain",
        kind = NoteKinds.EVENT,
        startsAt = 1_798_128_000_000L,
        endsAt = 1_798_142_400_000L,
        place = "Gran's house",
        rsvps = listOf(
            RsvpDto(userId = 2L, answer = RsvpAnswers.GOING),
            RsvpDto(userId = 3L, answer = RsvpAnswers.MAYBE),
        ),
        x = 0.1,
        y = 0.1,
        authorId = if (mine) 1L else 7L,
    )

    /** The wide roster the dialog names guests from. */
    private val guestNames = mapOf(2L to "Anna", 3L to "Gran")

    @Test
    fun anOpenEventNamesWhoIsComingRatherThanCountingThem() {
        compose.setContent {
            NoteDialog(
                draft = eventDraft(mine = false),
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                onDelete = null,
            )
        }

        // The names, grouped by answer — the card counts, the note names.
        compose.onNodeWithText("Anna").assertIsDisplayed()
        compose.onNodeWithText("Gran").assertIsDisplayed()
        // TWO of each answer that somebody gave: the picker's own choice
        // and the group's label. Nobody said no, so "Can't" is the
        // picker's alone — that absence is the assertion.
        compose.onAllNodesWithText("Going").assertCountEquals(2)
        compose.onAllNodesWithText("Maybe").assertCountEquals(2)
        compose.onAllNodesWithText("Can't").assertCountEquals(1)
    }

    @Test
    fun anEventNobodyHasAnsweredSaysSo() {
        compose.setContent {
            NoteDialog(
                draft = eventDraft().copy(rsvps = emptyList()),
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                onDelete = null,
            )
        }

        compose.onNodeWithText("Nobody has answered yet.").assertIsDisplayed()
    }

    /**
     * The calendar copy is ANYBODY's; the backdrop is the author's, and
     * only where the server can draw at all (docs/protocol.md, "Board").
     */
    @Test
    fun theCalendarCopyIsAnybodysAndTheBackdropIsTheAuthors() {
        var asked = 0
        compose.setContent {
            NoteDialog(
                draft = eventDraft(mine = false),
                canEdit = false,
                authorName = "Bob",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                canDraw = true,
                onDrawBackdrop = { _ -> asked += 1 },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Add to Calendar").assertIsDisplayed()
        compose.onAllNodesWithText("Draw a backdrop").assertCountEquals(0)
        assertThat(asked).isEqualTo(0)
    }

    @Test
    fun theAuthorAsksForABackdropOnceAndTheButtonSaysItIsDrawing() {
        var settle: ((Boolean) -> Unit)? = null
        var asked = 0
        compose.setContent {
            NoteDialog(
                draft = eventDraft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                canDraw = true,
                // Held, not answered: what the button looks like WHILE it
                // draws is the thing worth pinning.
                onDrawBackdrop = { onSettled -> asked += 1; settle = onSettled },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Draw a backdrop").performClick()
        assertThat(asked).isEqualTo(1)
        compose.onNodeWithText("Drawing…").assertIsDisplayed()
        // Pressed again while it draws: one bill, not two.
        compose.onNodeWithText("Drawing…").performClick()
        assertThat(asked).isEqualTo(1)

        settle?.invoke(true)
        compose.waitForIdle()
        // Back to an offer — and the note this dialog was opened with had
        // no backdrop, so it is the first-time wording until it is
        // reopened on a note that has one.
        compose.onNodeWithText("Draw a backdrop").assertIsDisplayed()
    }

    @Test
    fun anEventThatAlreadyHasABackdropOffersAnother() {
        compose.setContent {
            NoteDialog(
                draft = eventDraft().copy(hasBackdrop = true),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                canDraw = true,
                onDelete = null,
            )
        }

        compose.onNodeWithText("Draw another backdrop").assertIsDisplayed()
    }

    /** A server with no picture model has nothing to hang the action on. */
    @Test
    fun aServerThatCannotDrawOffersNoBackdrop() {
        compose.setContent {
            NoteDialog(
                draft = eventDraft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { _, _, _, _, _ -> },
                names = guestNames,
                canDraw = false,
                onDelete = null,
            )
        }

        compose.onAllNodesWithText("Draw a backdrop").assertCountEquals(0)
        // The calendar copy is still there: it needs no server at all.
        compose.onNodeWithText("Add to Calendar").assertIsDisplayed()
    }
}

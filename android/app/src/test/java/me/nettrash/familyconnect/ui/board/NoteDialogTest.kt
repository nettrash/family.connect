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
 * The size row is the same contract one field over: a segmented button
 * selects a step, the selection is published to semantics, and Save hands
 * the picked size back beside the colour.
 *
 * A local Robolectric Compose test, not an instrumented one — same
 * reasoning as the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
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
        x = 0.1,
        y = 0.1,
        authorId = 1L,
    )

    @Test
    fun tappingASwatchSelectsItAndSaveReportsThatColor() {
        var saved: Triple<String, String, String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size -> saved = Triple(text, color, size) },
                onDelete = null,
            )
        }

        // The dialog opens with the draft's color selected.
        compose.onNodeWithContentDescription("Yellow").assertIsSelected()

        compose.onNodeWithContentDescription("Green").performClick()
        compose.onNodeWithContentDescription("Green").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(Triple("hi", "green", "medium"))
    }

    @Test
    fun tappingASizeSelectsItAndSaveReportsThatSize() {
        var saved: Triple<String, String, String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size -> saved = Triple(text, color, size) },
                onDelete = null,
            )
        }

        // The dialog opens with the draft's size selected.
        compose.onNodeWithText("Medium").assertIsSelected()

        compose.onNodeWithText("Large").performClick()
        compose.onNodeWithText("Large").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(Triple("hi", "yellow", "large"))
    }

    /**
     * A name this client does not know draws as medium, and the picker says
     * so — but Save hands the NAME back untouched, the way an unknown colour
     * is, so opening a note to fix its text does not also shrink it.
     */
    @Test
    fun anUnknownSizeOpensWithMediumSelectedAndRoundTripsUntouched() {
        var saved: Triple<String, String, String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft().copy(size = "enormous"),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size -> saved = Triple(text, color, size) },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Medium").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(Triple("hi", "yellow", "enormous"))
    }

    /** Once the author picks a step, that step wins over the unknown name. */
    @Test
    fun pickingAStepReplacesAnUnknownSize() {
        var saved: Triple<String, String, String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft().copy(size = "enormous"),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color, size -> saved = Triple(text, color, size) },
                onDelete = null,
            )
        }

        compose.onNodeWithText("Small").performClick()
        compose.onNodeWithText("Small").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo(Triple("hi", "yellow", "small"))
    }
}

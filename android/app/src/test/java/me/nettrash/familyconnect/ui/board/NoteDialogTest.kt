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
        x = 0.1,
        y = 0.1,
        authorId = 1L,
    )

    @Test
    fun tappingASwatchSelectsItAndSaveReportsThatColor() {
        var saved: Pair<String, String>? = null
        compose.setContent {
            NoteDialog(
                draft = draft(),
                canEdit = true,
                authorName = "You",
                onDismiss = {},
                onSave = { text, color -> saved = text to color },
                onDelete = null,
            )
        }

        // The dialog opens with the draft's color selected.
        compose.onNodeWithContentDescription("Yellow").assertIsSelected()

        compose.onNodeWithContentDescription("Green").performClick()
        compose.onNodeWithContentDescription("Green").assertIsSelected()

        compose.onNodeWithText("Save").performClick()
        assertThat(saved).isEqualTo("hi" to "green")
    }
}

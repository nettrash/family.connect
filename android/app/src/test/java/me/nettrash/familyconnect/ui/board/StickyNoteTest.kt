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
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
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

    /**
     * A PHOTO IS DRAWN WHOLE, and a BARE one's card IS the picture
     * (docs/protocol.md, "Board"): a 600x1200 photograph pinned with no
     * caption gets a card half as wide as it is tall, so every pixel of it
     * shows and the pin sits on the photograph. Issue #71: the card kept
     * its square and `ContentScale.Crop` threw away the top and the bottom
     * of the picture.
     */
    @Test
    fun aBarePhotosCardIsTheShapeOfThePhotograph() {
        compose.setContent {
            StickyNote(
                note = portrait(caption = ""),
                authorName = "Bob",
                isHiddenByBlock = false,
                boardWidthPx = 1080,
                boardHeightPx = 1920,
                onMoved = { _, _ -> },
                onTap = {},
            )
        }

        val card = compose.onNodeWithContentDescription("Note from Bob:", substring = true)
            .fetchSemanticsNode().size
        assertThat(card.width.toFloat() / card.height.toFloat()).isWithin(0.02f).of(0.5f)
    }

    /**
     * A CAPTION brings the card back — the words need paper to sit on — so
     * the card is the step's own square again, whatever shape the picture
     * is. The picture is fitted into the strip above the words.
     */
    @Test
    fun aCaptionedPhotoKeepsTheCardsOwnShape() {
        compose.setContent {
            StickyNote(
                note = portrait(caption = "Gran's garden"),
                authorName = "Bob",
                isHiddenByBlock = false,
                boardWidthPx = 1080,
                boardHeightPx = 1920,
                onMoved = { _, _ -> },
                onTap = {},
            )
        }

        val card = compose.onNodeWithContentDescription("Gran's garden", substring = true)
            .fetchSemanticsNode().size
        assertThat(card.width.toFloat() / card.height.toFloat()).isWithin(0.02f).of(1f)
    }

    private fun portrait(caption: String) = NoteEntity(
        id = 2L,
        authorId = 9L,
        text = caption,
        color = "yellow",
        x = 0.1,
        y = 0.1,
        createdAt = 0L,
        updatedAt = 0L,
        boardSeq = 1L,
        kind = "photo",
        attachmentJson = AttachmentsCodec.encode(
            listOf(
                AttachmentDto(
                    id = 7L,
                    kind = "photo",
                    mime = "image/jpeg",
                    size = 1234L,
                    width = 600,
                    height = 1200,
                    hasPreview = true,
                ),
            ),
        ),
    )
}

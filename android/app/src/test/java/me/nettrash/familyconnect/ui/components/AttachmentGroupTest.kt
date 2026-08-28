/*
 * AttachmentGroupTest.kt
 * Family Connect (Android)
 *
 * What a bubble's attachments become. Two or more photos are ONE
 * accessible button — "Album, 1 of N" — that opens on the first; one
 * photo stays the plain thumbnail, even with a file row under it, where
 * a grid cell used to sit; and the rows follow in sent order. Pinned
 * without an image cache, so every card is its placeholder — the
 * semantics are the same either way, which is rather the point.
 *
 * A local Robolectric Compose test, not an instrumented one — same
 * reasoning as the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.getBoundsInRoot
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AttachmentGroupTest {

    @get:Rule
    val compose = createComposeRule()

    /**
     * Every block also listens for a double-tap (the quick heart), so a
     * single tap fires only once the double-tap window has closed — the
     * test clock has to be walked past it before the open is observable.
     */
    private fun settleTap() = compose.mainClock.advanceTimeBy(1_000)

    private val photo1 = FakeAttachmentApi.attachment(id = 1, kind = "photo")
    private val video2 = FakeAttachmentApi.attachment(id = 2, kind = "video")
    private val photo3 = FakeAttachmentApi.attachment(id = 3, kind = "photo")
    private val file4 = FakeAttachmentApi.attachment(id = 4, kind = "file")

    @Test
    fun severalMediaAreOneAlbumButtonThatOpensOnTheFirst() {
        var opened: AttachmentDto? = null
        compose.setContent {
            AttachmentGroup(
                attachments = listOf(photo1, video2, photo3),
                onOpen = { opened = it },
            )
        }

        val album = compose.onNodeWithContentDescription("Album, 1 of 3")
        album.assertIsDisplayed()
        album.assertHasClickAction()
        // The pile is flattened: no per-photo elements and no badge text
        // leak out beside the album itself.
        compose.onAllNodesWithContentDescription("Photo").assertCountEquals(0)
        compose.onAllNodesWithContentDescription("Video").assertCountEquals(0)
        compose.onNodeWithText("3").assertDoesNotExist()

        album.performClick()
        settleTap()
        assertThat(opened).isEqualTo(photo1)
    }

    @Test
    fun aLonePhotoWithAFileRowIsStillThePlainThumbnail() {
        var opened: AttachmentDto? = null
        compose.setContent {
            AttachmentGroup(
                attachments = listOf(photo1, file4),
                onOpen = { opened = it },
            )
        }

        compose.onNodeWithContentDescription("Album, 1 of 1").assertDoesNotExist()
        compose.onNodeWithContentDescription("Photo").assertIsDisplayed().performClick()
        settleTap()
        assertThat(opened).isEqualTo(photo1)

        compose.onNodeWithContentDescription("receipts.pdf", substring = true).assertIsDisplayed().performClick()
        settleTap()
        assertThat(opened).isEqualTo(file4)
    }

    @Test
    fun rowsRideUnderTheStack() {
        compose.setContent {
            AttachmentGroup(
                attachments = listOf(file4, photo1, photo3),
                onOpen = {},
            )
        }

        val album = compose.onNodeWithContentDescription("Album, 1 of 2")
        val row = compose.onNodeWithContentDescription("receipts.pdf", substring = true)
        album.assertIsDisplayed()
        row.assertIsDisplayed()
        // Under, not merely alongside: the file was SENT first, and the
        // media still come out on top.
        assertThat(row.getBoundsInRoot().top.value)
            .isAtLeast(album.getBoundsInRoot().bottom.value)
    }
}

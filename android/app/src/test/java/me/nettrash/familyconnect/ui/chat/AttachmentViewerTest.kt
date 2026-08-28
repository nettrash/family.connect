/*
 * AttachmentViewerTest.kt
 * Family Connect (Android)
 *
 * The viewer became a pager over a message's media. This pins the part
 * a reader can see without a real image cache: opened on an album's
 * second item it says so, a lone photo says nothing, Share and Save hand
 * back the page SHOWING rather than the first, and Close still closes.
 *
 * A local Robolectric Compose test, not an instrumented one — same
 * reasoning as the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.chat

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.ui.components.AttachmentAlbum
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AttachmentViewerTest {

    @get:Rule
    val compose = createComposeRule()

    private val three = listOf(
        FakeAttachmentApi.attachment(id = 1, kind = "photo"),
        FakeAttachmentApi.attachment(id = 2, kind = "photo"),
        FakeAttachmentApi.attachment(id = 3, kind = "photo"),
    )

    @Test
    fun openedOnTheSecondOfThreeItSaysSo() {
        compose.setContent {
            AttachmentViewer(
                album = AttachmentAlbum(three, index = 1),
                streamUrl = { null },
                onShare = {},
                onSave = {},
                onDismiss = {},
            )
        }

        compose.onNodeWithText("2 of 3").assertIsDisplayed()
    }

    @Test
    fun aLonePhotoHasNoPageLabel() {
        compose.setContent {
            AttachmentViewer(
                album = AttachmentAlbum(three.take(1), index = 0),
                streamUrl = { null },
                onShare = {},
                onSave = {},
                onDismiss = {},
            )
        }

        compose.onNodeWithText("1 of 1").assertDoesNotExist()
    }

    @Test
    fun shareAndSaveActOnThePageShowing() {
        var shared: AttachmentDto? = null
        var saved: AttachmentDto? = null
        compose.setContent {
            AttachmentViewer(
                album = AttachmentAlbum(three, index = 2),
                streamUrl = { null },
                onShare = { shared = it },
                onSave = { saved = it },
                onDismiss = {},
            )
        }

        compose.onNodeWithContentDescription("Share").performClick()
        assertThat(shared).isEqualTo(three[2])

        compose.onNodeWithContentDescription("Save to gallery").performClick()
        assertThat(saved).isEqualTo(three[2])
    }

    @Test
    fun closeDismisses() {
        var dismissed = false
        compose.setContent {
            AttachmentViewer(
                album = AttachmentAlbum(three, index = 1),
                streamUrl = { null },
                onShare = {},
                onSave = {},
                onDismiss = { dismissed = true },
            )
        }

        compose.onNodeWithContentDescription("Close").performClick()
        assertThat(dismissed).isTrue()
    }
}

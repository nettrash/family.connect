/*
 * StickerPanelTest.kt
 * Family Connect (Android)
 *
 * The sticker panel's grid (docs/protocol.md, "In the panel, one tap
 * sends"): every item of the pack is one button, a tap on it is the whole
 * interaction, and what this device used recently comes first.
 *
 * The pictures are not loaded here — Robolectric has no decoder worth
 * asking — so the loader answers null and the cells are found by what a
 * screen reader would say, which is the label whoever added the item gave.
 */

package me.nettrash.familyconnect.ui.stickers

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertHasClickAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.data.db.PackItemEntity
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class StickerPanelTest {

    @get:Rule
    val compose = createComposeRule()

    private fun item(id: Long, label: String? = null, addedBy: Long = 7) = PackItemEntity(
        id = id,
        addedBy = addedBy,
        attachmentJson = """[{"id":${70 + id},"kind":"photo","mime":"image/webp","size":2048}]""",
        label = label,
        createdAt = 1,
        packSeq = 10 + id,
    )

    @Test
    fun oneTapOnAStickerIsTheWholeInteraction() {
        val picked = mutableListOf<Long>()
        compose.setContent {
            StickerGrid(
                items = listOf(item(1, "party cat"), item(2, "thumbs up")),
                recents = emptyList(),
                load = { null },
                onPick = { picked += it.id },
            )
        }

        val cell = compose.onNodeWithContentDescription("thumbs up")
        cell.assertHasClickAction()
        cell.performClick()

        // Sent by that tap: no caption asked for, nothing to confirm.
        assertThat(picked).containsExactly(2L)
    }

    @Test
    fun aStickerWithNoLabelIsStillAnnounced() {
        compose.setContent {
            StickerGrid(items = listOf(item(1)), recents = emptyList(), load = { null }, onPick = {})
        }

        compose.onNodeWithContentDescription("Sticker").assertHasClickAction()
    }

    @Test
    fun recentlyUsedComesFirstAndThePackStaysWhole() {
        val picked = mutableListOf<Long>()
        val pack = listOf(item(1, "one"), item(2, "two"), item(3, "three"))
        compose.setContent {
            StickerGrid(items = pack, recents = listOf(pack[2]), load = { null }, onPick = { picked += it.id })
        }

        compose.onNodeWithText("Recently used").assertExists()
        compose.onNodeWithText("Family stickers").assertExists()
        // In the recent row AND in its place in the pack — the pack's order
        // does not shuffle because of what somebody sent last.
        compose.onAllNodesWithContentDescription("three").assertCountEquals(2)
        compose.onAllNodesWithContentDescription("one").assertCountEquals(1)

        compose.onNodeWithTag("sticker-1").performClick()
        assertThat(picked).containsExactly(1L)
    }

    @Test
    fun withNothingRecentThereAreNoHeaders() {
        compose.setContent {
            StickerGrid(items = listOf(item(1, "one")), recents = emptyList(), load = { null }, onPick = {})
        }

        compose.onNodeWithText("Recently used").assertDoesNotExist()
        compose.onNodeWithText("Family stickers").assertDoesNotExist()
    }

    @Test
    fun aBlockedMembersItemsAreInThePanelLikeAnyOther() {
        // The grid is handed the pack and draws all of it: there is no
        // block list anywhere in its inputs, by design — a pack item is the
        // family's picture, not something a person said.
        compose.setContent {
            StickerGrid(
                items = listOf(item(1, "mine", addedBy = 7), item(2, "theirs", addedBy = 11)),
                recents = emptyList(),
                load = { null },
                onPick = {},
            )
        }

        compose.onNodeWithContentDescription("theirs").assertHasClickAction()
    }
}

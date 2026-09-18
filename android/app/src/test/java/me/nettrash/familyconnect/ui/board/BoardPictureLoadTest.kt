/*
 * BoardPictureLoadTest.kt
 * Family Connect (Android)
 *
 * WHICH BYTES THE BOARD ASKS FOR.
 *
 * The regression this pins is the one nettrash reported twice: an event's
 * backdrop drew nothing on Android. The picture was there, the note carried
 * it, the card was ready to draw it — and the sticker asked only for the
 * PREVIEW. The server generates no previews for a picture it drew itself
 * (`has_preview` stays false; server/src/handlers_ai.rs), and
 * `rememberAttachmentImage(preview = true)` answers null for a photo whose
 * flag is down WITHOUT asking the server anything, on purpose: for a photo
 * the full bytes are the fallback. The board never took it.
 *
 * So what is asserted here is the REQUEST, not the pixels: a backdrop with
 * no preview must make the board fetch the original. The chat's own blocks
 * have always done this (`AttachmentPreviewGateTest` covers their side).
 *
 * A local Robolectric Compose test with a real AttachmentRepository over
 * fakes — the harness AttachmentPreviewGateTest established, including why
 * all three of recomposition, the main looper and real time have to be
 * turned before the cache answers.
 */

package me.nettrash.familyconnect.ui.board

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import me.nettrash.familyconnect.data.db.NoteEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.net.dto.AttachmentsCodec
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.ui.components.LocalAttachments
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.shadows.ShadowLooper
import java.io.File

@RunWith(RobolectricTestRunner::class)
class BoardPictureLoadTest {

    @get:Rule
    val compose = createComposeRule()

    private val scope = CoroutineScope(Dispatchers.Main + SupervisorJob())
    private val api = FakeAttachmentApi()

    @After
    fun tearDown() {
        scope.cancel()
    }

    private companion object {
        const val TIMEOUT_MS = 5_000L
        const val POLL_MS = 10L
    }

    private fun repository() = AttachmentRepository(
        context = ApplicationProvider.getApplicationContext(),
        attachmentApi = api,
        settings = FakeSettingsRepository(),
        connectivity = FakeConnectivityObserver(),
        scope = scope,
    )

    /** What the real client leaves behind on a 200: the file, in place. */
    private fun serve(destination: File): ApiResult<Unit> {
        destination.parentFile?.mkdirs()
        destination.writeBytes(ByteArray(8))
        return ApiResult.Ok(Unit)
    }

    private fun pump(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + TIMEOUT_MS
        while (System.currentTimeMillis() < deadline) {
            compose.waitForIdle()
            ShadowLooper.idleMainLooper()
            if (condition()) return
            Thread.sleep(POLL_MS)
        }
        error("the attachment cache never answered")
    }

    private fun note(kind: String, attachment: AttachmentDto, text: String) = NoteEntity(
        id = 5L,
        authorId = 9L,
        text = text,
        color = "blue",
        x = 0.1,
        y = 0.1,
        createdAt = 0L,
        updatedAt = 0L,
        boardSeq = 1L,
        kind = kind,
        startsAt = if (kind == "event") 1_800_000_000_000L else null,
        attachmentJson = AttachmentsCodec.encode(listOf(attachment)),
    )

    private fun draw(entity: NoteEntity, repo: AttachmentRepository) {
        compose.setContent {
            CompositionLocalProvider(LocalAttachments provides repo) {
                StickyNote(
                    note = entity,
                    authorName = "Bob",
                    isHiddenByBlock = false,
                    boardWidthPx = 1080,
                    boardHeightPx = 1920,
                    onMoved = { _, _ -> },
                    onTap = {},
                )
            }
        }
    }

    /**
     * THE REGRESSION. The assistant drew the picture, so it has no preview;
     * the card must fall back to the original rather than drawing nothing.
     */
    @Test
    fun anEventsBackdropWithNoPreviewIsFetchedWhole() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val drawn = FakeAttachmentApi.attachment(id = 900, kind = "photo", hasPreview = false)

        draw(note("event", drawn, "Gran's birthday"), repo)

        pump { repo.cached(900, preview = false) != null }
        // The original, and nothing pointless: no request for a preview the
        // server never made.
        assertThat(api.downloads).containsExactly(900L to false)
    }

    /** A pinned photo with no preview is the same case, one kind over. */
    @Test
    fun aPinnedPhotoWithNoPreviewIsFetchedWhole() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val pinned = FakeAttachmentApi.attachment(id = 901, kind = "photo", hasPreview = false)

        draw(note("photo", pinned, ""), repo)

        pump { repo.cached(901, preview = false) != null }
        assertThat(api.downloads).containsExactly(901L to false)
    }

    /**
     * And with a preview to have, that is what a sticker takes: a 132dp
     * card has no use for 1600 pixels, and the full bytes are not pulled
     * down behind it.
     */
    @Test
    fun aPictureWithAPreviewIsDrawnFromThePreviewAlone() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val pinned = FakeAttachmentApi.attachment(id = 902, kind = "photo", hasPreview = true)

        draw(note("photo", pinned, "at the lake"), repo)

        pump { repo.cached(902, preview = true) != null }
        // ONE request. A fallback chain would have asked for the original on
        // the same frame — two downloads and full-size bytes on a phone for
        // every picture on the wall.
        assertThat(api.downloads).containsExactly(902L to true)
    }
}

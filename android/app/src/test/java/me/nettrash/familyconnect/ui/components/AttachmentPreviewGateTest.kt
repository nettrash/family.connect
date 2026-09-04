/*
 * AttachmentPreviewGateTest.kt
 * Family Connect (Android)
 *
 * Which attachments a bubble asks the server about, and which it decides
 * about on its own.
 *
 * `has_preview` arrives with the message and can never be corrected
 * afterwards (see AttachmentPosterTest for why), so a VIDEO stored with
 * the flag down used to be a tile that never asked anybody anything —
 * permanently grey, because a video has no full-size bytes to fall back
 * on the way a photo does. The rule now: a photo may still trust the
 * flag, a video always asks.
 *
 * A local Robolectric Compose test, not an instrumented one — same
 * reasoning as the rest of app/src/test.
 */

package me.nettrash.familyconnect.ui.components

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.AttachmentRepository
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.shadows.ShadowLooper
import java.io.File

@RunWith(RobolectricTestRunner::class)
class AttachmentPreviewGateTest {

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

    /**
     * Run until the cache has answered, or give up.
     *
     * `compose.waitUntil` is not enough here and it is worth saying why:
     * it drives the COMPOSE clock, and the fetch this waits on finishes
     * somewhere else entirely — the repository decodes on Dispatchers.IO
     * and hands the bitmap back through Dispatchers.Main.immediate, which
     * under Robolectric is a PAUSED looper that nothing has idled. So all
     * three have to be turned: recomposition, the main looper, and real
     * time for the IO thread to get its work done.
     */
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

    /** What the real client leaves behind on a 200: the file, in place. */
    private fun serve(destination: File): ApiResult<Unit> {
        destination.parentFile?.mkdirs()
        destination.writeBytes(ByteArray(8))
        return ApiResult.Ok(Unit)
    }

    /**
     * THE REGRESSION. The row says there is no poster; the server has
     * one. Before the fix this bubble made no request at all, and no
     * amount of leaving the chat, relaunching or reinstalling-short-of
     * changed that — the flag it was reading can never be corrected.
     */
    @Test
    fun aVideoWhoseStoredFlagSaysNoPosterStillAsksTheServer() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val video = FakeAttachmentApi.attachment(id = 41, kind = "video", hasPreview = false)

        compose.setContent {
            CompositionLocalProvider(LocalAttachments provides repo) {
                AttachmentGroup(attachments = listOf(video), onOpen = {})
            }
        }

        pump { repo.cached(41, preview = true) != null }
        // The poster, and only the poster: a video's own bytes are never
        // pulled into the image cache — they are streamed by the player.
        assertThat(api.downloads).containsExactly(41L to true)
    }

    /**
     * The normal path, unchanged. A photo with the flag down asks for its
     * FULL bytes and never for a preview: being wrong about a photo costs
     * nothing, so the request that would 404 is still not made.
     */
    @Test
    fun aPhotoWhoseFlagSaysNoPreviewStillAsksOnlyForTheFullBytes() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val photo = FakeAttachmentApi.attachment(id = 42, kind = "photo", hasPreview = false)

        compose.setContent {
            CompositionLocalProvider(LocalAttachments provides repo) {
                AttachmentGroup(attachments = listOf(photo), onOpen = {})
            }
        }

        pump { repo.cached(42, preview = false) != null }
        assertThat(api.downloads).containsExactly(42L to false)
    }

    /**
     * And with the flag UP nothing changed either: the preview is what a
     * bubble draws, and the full bytes are not fetched behind it.
     */
    @Test
    fun aVideoWhoseFlagSaysThereIsAPosterAsksForItExactlyOnce() {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()
        val video = FakeAttachmentApi.attachment(id = 43, kind = "video", hasPreview = true)

        compose.setContent {
            CompositionLocalProvider(LocalAttachments provides repo) {
                AttachmentGroup(attachments = listOf(video), onOpen = {})
            }
        }

        pump { repo.cached(43, preview = true) != null }
        assertThat(api.downloads).containsExactly(43L to true)
    }
}

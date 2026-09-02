/*
 * AttachmentPosterTest.kt
 * Family Connect (Android)
 *
 * A video's poster, and the flag that used to be allowed to decide
 * whether anyone ever asked for one.
 *
 * `has_preview` is a snapshot the server took when this device read the
 * message, and it is the one attachment field that changes afterwards. A
 * copy that says `false` can never correct itself — history sync is
 * `after_id` only, so it cannot see a mutation of an older row, and the
 * arrival path writes with `insertIgnore` — so a video stored with the
 * flag down was a grey tile through leaving the chat, through a relaunch,
 * for the life of the install. It had nothing to fall back on: a photo
 * shows its full bytes, a video's bytes are a video.
 *
 * These pin the two halves that have to hold together: the server is
 * asked whatever the flag says, and a poster that genuinely does not
 * exist SETTLES — three small requests once per process, then silence.
 *
 * Robolectric because decoding needs a real BitmapFactory; the bitmaps it
 * hands back are shadows, which is fine — nothing here asserts on pixels.
 *
 * iOS counterpart: ios/FamilyConnectTests/AttachmentPosterTests.swift
 */

package me.nettrash.familyconnect.data.repo

import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.testutil.FakeAttachmentApi
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class AttachmentPosterTest {

    private val dispatcher = StandardTestDispatcher()

    /**
     * NOT `backgroundScope`: the repository's re-check ladder waits on
     * `delay`, and advancing virtual time is what has to drive it —
     * `advanceUntilIdle` leaves background-scope work alone. A scope of
     * its own is also what the repository really has (@AppScope), and
     * runTest does not wait for the two endless collectors it starts.
     */
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private val api = FakeAttachmentApi()

    @Before
    fun setUp() {
        // The repository marshals its writes to Dispatchers.Main.immediate
        // so snapshot state is never mutated from two threads at once.
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
    }

    private fun repository() = AttachmentRepository(
        context = ApplicationProvider.getApplicationContext(),
        attachmentApi = api,
        settings = FakeSettingsRepository(),
        connectivity = FakeConnectivityObserver(),
        scope = repoScope,
    )

    /** What the real client leaves behind on a 200: the file, in place. */
    private fun serve(destination: File): ApiResult<Unit> {
        destination.parentFile?.mkdirs()
        // Robolectric's BitmapFactory decodes any non-empty bytes into a
        // shadow bitmap, so the content does not have to be a real JPEG.
        destination.writeBytes(ByteArray(8))
        return ApiResult.Ok(Unit)
    }

    /** The 404 the server gives for an attachment with no preview. */
    private val notFound = ApiResult.HttpError(404, "attachment_not_found", "no")

    /**
     * THE REGRESSION, at the layer that can fix it: the poster was not
     * there the first time it was asked for, and the picture must still
     * turn up — without a relaunch, without the network dropping, without
     * the reader doing anything at all.
     */
    @Test
    fun `a poster that lands late is picked up without a relaunch`() = runTest(dispatcher) {
        var answered = 0
        api.downloadHandler = { _, _, destination ->
            answered++
            if (answered == 1) notFound else serve(destination)
        }
        val repo = repository()

        repo.load(attachmentId = 34, preview = true, mayArriveLate = true)

        assertThat(repo.cached(34, preview = true)).isNotNull()
        assertThat(api.downloads).containsExactly(34L to true, 34L to true)
    }

    /**
     * The other half, and the one that keeps the fix honest: a poster
     * that will NEVER exist — the sender's frame grab failed, or its
     * upload did — must stop being asked for. The ladder is spent once
     * per key per process, whatever draws the bubble afterwards.
     */
    @Test
    fun `a video with no poster settles after a bounded number of tries`() = runTest(dispatcher) {
        api.downloadHandler = { _, _, _ -> notFound }
        val repo = repository()

        repo.load(attachmentId = 35, preview = true, mayArriveLate = true)

        val spent = api.downloads.size
        assertThat(spent)
            .isEqualTo(1 + AttachmentRepository.POSTER_RETRY_DELAYS_MS.size)

        // Scrolled back to, re-entered, redrawn: the key is settled and
        // nothing asks again.
        repeat(3) { repo.load(attachmentId = 35, preview = true, mayArriveLate = true) }
        assertThat(api.downloads).hasSize(spent)
        assertThat(repo.cached(35, preview = true)).isNull()
    }

    /**
     * A photo's preview keeps the old rule exactly: one answer, and it is
     * final. Only a poster may arrive late, and only a poster pays for the
     * re-checks.
     */
    @Test
    fun `an ordinary preview still settles on the first answer`() = runTest(dispatcher) {
        api.downloadHandler = { _, _, _ -> notFound }
        val repo = repository()

        repo.load(attachmentId = 36, preview = true)
        assertThat(api.downloads).containsExactly(36L to true)

        repeat(3) { repo.load(attachmentId = 36, preview = true) }
        assertThat(api.downloads).hasSize(1)
    }

    /**
     * The invariant the ladder must not have eaten: a server that never
     * ANSWERED says nothing about whether the bytes exist, so the key
     * stays retryable — the bug that blanked every photo on screen the
     * moment the phone left the house.
     */
    @Test
    fun `a transport failure is still not final`() = runTest(dispatcher) {
        var answered = 0
        api.downloadHandler = { _, _, destination ->
            answered++
            if (answered == 1) {
                ApiResult.NetworkError(java.io.IOException("no route"))
            } else {
                serve(destination)
            }
        }
        val repo = repository()

        repo.load(attachmentId = 37, preview = true)
        assertThat(repo.cached(37, preview = true)).isNull()

        repo.load(attachmentId = 37, preview = true)
        assertThat(repo.cached(37, preview = true)).isNotNull()
        assertThat(api.downloads).hasSize(2)
    }

    /**
     * A poster that arrives on the FIRST ask costs exactly one request —
     * the ladder is a consequence of being told "no", not a schedule.
     */
    @Test
    fun `a poster that is there costs one request`() = runTest(dispatcher) {
        api.downloadHandler = { _, _, destination -> serve(destination) }
        val repo = repository()

        repo.load(attachmentId = 38, preview = true, mayArriveLate = true)

        assertThat(repo.cached(38, preview = true)).isNotNull()
        assertThat(api.downloads).containsExactly(38L to true)
    }
}

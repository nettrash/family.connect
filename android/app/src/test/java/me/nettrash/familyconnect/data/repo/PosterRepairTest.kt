/*
 * PosterRepairTest.kt
 * Family Connect (Android)
 *
 * A video's poster is uploaded in its own request, best-effort, so that a
 * thumbnail never costs the send. Until issue #54 "best-effort" meant
 * EXACTLY ONCE: if that one `PUT /attachments/{id}/preview` failed, the
 * server kept `has_preview = false` and nothing ever repaired it — not a
 * relaunch, not a resync, not re-opening the chat. Every recipient got a
 * grey tile with a play badge, for good, and a video has no second source
 * of pixels to fall back on. That is the report behind #38, and the half
 * #38 fixed — treating `has_preview` as a hint — cannot conjure a poster
 * that was never uploaded.
 *
 * These pin the second half. The device that made the poster still holds
 * it (this repository is the [PosterCache] now, exactly as iOS's
 * AttachmentStore always was), so it finishes the job the next time the
 * network is there; and the cases where it CANNOT finish the job SETTLE
 * rather than retry for ever:
 *
 *   - nothing held locally  → no request at all, ever
 *   - the server refuses    → a fixed budget of ANSWERED attempts, then
 *                             the video is given up on for good
 *   - nobody answered       → nothing learned, nothing spent
 *   - the session is gone   → the pass stops instead of burning every
 *                             waiting poster's budget on one dead token
 *
 * Robolectric because seeding decodes; the bitmaps it hands back are
 * shadows, which is fine — nothing here asserts on pixels.
 *
 * iOS counterpart: ios/FamilyConnectTests/PosterRepairTests.swift
 */

package me.nettrash.familyconnect.data.repo

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withContext
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
class PosterRepairTest {

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private val api = FakeAttachmentApi()
    private val connectivity = FakeConnectivityObserver()

    /** Real JPEG-ness is not needed: Robolectric decodes any non-empty
     *  bytes into a shadow bitmap, and nothing here looks at pixels. */
    private val poster = ByteArray(24) { 0x7 }

    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val cache: File
        get() = File(context.cacheDir, AttachmentRepository.CACHE_DIR)

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        // Robolectric hands every case in a class the same cache dir, and
        // a marker left behind would be found by the next case's pass.
        cache.deleteRecursively()
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
    }

    private fun repository() = AttachmentRepository(
        context = context,
        attachmentApi = api,
        settings = FakeSettingsRepository(),
        connectivity = connectivity,
        scope = repoScope,
    )

    /** Put the repository in the state a failed poster upload leaves
     *  behind: the frame kept for this device's own bubble, and a note
     *  that the server never got it. */
    private suspend fun stageFailedUpload(repo: AttachmentRepository, id: Long) {
        repo.seedPoster(id, poster)
        repo.notePosterUpload(id, landed = false)
    }

    private fun previewUploads() = api.uploadedPreviews

    /**
     * Wait for work that hopped to a REAL dispatcher.
     *
     * The repository does its file IO on [Dispatchers.IO], which virtual
     * time knows nothing about, and a pass started by the connectivity
     * collector ping-pongs between that and the test dispatcher — so one
     * `advanceUntilIdle` is never enough. This drives the test dispatcher
     * and gives the real threads a moment, alternately, until the
     * condition holds or the budget is spent. Every OTHER case here calls
     * the suspend function directly and simply awaits it; only the two
     * fire-and-forget paths (the collector, and `clear`) need this.
     */
    private suspend fun TestScope.awaitReal(condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + 5_000
        while (!condition() && System.currentTimeMillis() < deadline) {
            advanceUntilIdle()
            withContext(Dispatchers.IO) { Thread.sleep(5) }
        }
        advanceUntilIdle()
    }

    // -- The repair --------------------------------------------------------

    /**
     * THE REGRESSION. One failed `uploadPreview` used to be the end of it.
     * The bytes never left this device, and this device is the only one
     * that has them.
     */
    @Test
    fun `a poster the server never got is re-sent on the next pass`() = runTest(dispatcher) {
        val repo = repository()
        stageFailedUpload(repo, 70)
        assertThat(repo.isPosterUnsent(70)).isTrue()

        repo.repairPosters()

        assertThat(previewUploads()).containsExactly(70L to poster.size)
        assertThat(repo.isPosterUnsent(70)).isFalse()
    }

    /**
     * And the trigger it hangs off: the network coming back. `onAvailable`
     * also fires for the network that is already up when the callback
     * registers, so this is once per launch plus once per recovery — the
     * only thing that starts a pass, because a repair on a timer would be
     * a poll.
     */
    @Test
    fun `the network coming back starts a pass`() = runTest(dispatcher) {
        val repo = repository()
        stageFailedUpload(repo, 71)

        connectivity.onAvailable.emit(Unit)
        awaitReal { previewUploads().isNotEmpty() }

        assertThat(previewUploads()).containsExactly(71L to poster.size)
        assertThat(repo.isPosterUnsent(71)).isFalse()
    }

    /** The pass costs nothing when nothing is owed — every normal send. */
    @Test
    fun `a poster that landed leaves nothing to repair`() = runTest(dispatcher) {
        val repo = repository()
        repo.seedPoster(72, poster)
        repo.notePosterUpload(72, landed = true)

        repo.repairPosters()

        assertThat(previewUploads()).isEmpty()
        assertThat(repo.isPosterUnsent(72)).isFalse()
    }

    /** A repair that succeeded is not walked again on the pass after it. */
    @Test
    fun `a repaired poster settles`() = runTest(dispatcher) {
        val repo = repository()
        stageFailedUpload(repo, 73)

        repeat(3) { repo.repairPosters() }

        assertThat(previewUploads()).hasSize(1)
    }

    // -- Settling when it cannot be repaired --------------------------------

    /**
     * The frame grab returned null, or the cache was purged: there are no
     * pixels here and no number of tries produces any. It must cost ZERO
     * requests — not a bounded few, none — and the note asking for them
     * must go, or every pass re-reads it.
     */
    @Test
    fun `a poster this device no longer holds is never chased`() = runTest(dispatcher) {
        val repo = repository()
        stageFailedUpload(repo, 74)
        // What a cache purge does: the bytes go, the note stays.
        assertThat(File(cache, "74-preview.jpg").delete()).isTrue()

        repo.repairPosters()

        assertThat(previewUploads()).isEmpty()
        assertThat(repo.isPosterUnsent(74)).isFalse()
    }

    /**
     * THE SETTLE. A server that keeps refusing — out of disk, say — must
     * not be asked for ever. The budget is spent in ANSWERED attempts and
     * then the video is abandoned: it keeps the play badge over a plain
     * placeholder, exactly as a video whose poster genuinely never existed
     * does on the read side.
     */
    @Test
    fun `a poster the server keeps refusing is given up on for good`() = runTest(dispatcher) {
        api.previewHandler = { _, _ -> ApiResult.HttpError(500, "internal", "no") }
        val repo = repository()
        stageFailedUpload(repo, 75)

        // One pass per reconnect. Well past the budget.
        repeat(AttachmentRepository.POSTER_REPAIR_ATTEMPTS + 4) { repo.repairPosters() }

        assertThat(previewUploads()).hasSize(AttachmentRepository.POSTER_REPAIR_ATTEMPTS)
        assertThat(repo.isPosterUnsent(75)).isFalse()
    }

    /**
     * The counterpart rule, and the one that keeps the budget honest:
     * nobody answered, so nothing was learned. Spending an attempt here is
     * what would burn a perfectly repairable poster's whole budget during
     * an outage — the same mistake [AttachmentRepository.startFetch]
     * refuses to make on the read side.
     */
    @Test
    fun `a request nobody answered does not spend the budget`() = runTest(dispatcher) {
        api.previewHandler = { _, _ -> ApiResult.NetworkError(java.io.IOException("offline")) }
        val repo = repository()
        stageFailedUpload(repo, 76)

        repeat(AttachmentRepository.POSTER_REPAIR_ATTEMPTS + 4) { repo.repairPosters() }

        assertThat(repo.isPosterUnsent(76)).isTrue()
        // Still offered every time — the budget was never touched.
        assertThat(previewUploads())
            .hasSize(AttachmentRepository.POSTER_REPAIR_ATTEMPTS + 4)
    }

    /**
     * A dead session fails every waiting poster identically, so the pass
     * stops on the first rather than spending every marker's budget on the
     * same 401. A fresh sign-in can still send them.
     */
    @Test
    fun `a dead session stops the pass instead of burning every budget`() = runTest(dispatcher) {
        api.previewHandler = { _, _ -> ApiResult.HttpError(401, "unauthorized", "no") }
        val repo = repository()
        stageFailedUpload(repo, 77)
        stageFailedUpload(repo, 78)

        repo.repairPosters()

        assertThat(previewUploads()).hasSize(1)
        assertThat(repo.isPosterUnsent(77)).isTrue()
        assertThat(repo.isPosterUnsent(78)).isTrue()
    }

    /**
     * An attachment the server does not have is as terminal as an answer
     * gets — swept, or never ours to replace. No budget, no second look.
     */
    @Test
    fun `a poster for an attachment the server never saw is dropped`() = runTest(dispatcher) {
        api.previewHandler = { _, _ -> ApiResult.HttpError(404, "attachment_not_found", "no") }
        val repo = repository()
        stageFailedUpload(repo, 79)

        repo.repairPosters()
        repo.repairPosters()

        assertThat(previewUploads()).hasSize(1)
        assertThat(repo.isPosterUnsent(79)).isFalse()
    }

    // -- What seeding buys ---------------------------------------------------

    /**
     * The other half of holding the pixels, and iOS's behaviour since it
     * had a seed at all: the sender draws its own bubble from what it just
     * made instead of downloading bytes it produced a moment ago. ONE
     * write, not two — [AttachmentRepository.load] finds the frame in
     * memory and never reaches the network.
     */
    @Test
    fun `a seeded poster is drawn without asking the server`() = runTest(dispatcher) {
        val repo = repository()
        repo.seedPoster(80, poster)

        assertThat(repo.cached(80, preview = true)).isNotNull()
        repo.load(80, preview = true, mayArriveLate = true)
        advanceUntilIdle()

        assertThat(api.downloads).isEmpty()
    }

    /** Logout takes the whole directory, notes included: the next account
     *  must not push the previous one's frames anywhere. */
    @Test
    fun `logout drops what was still owed`() = runTest(dispatcher) {
        val repo = repository()
        stageFailedUpload(repo, 81)

        repo.clear()
        awaitReal { !repo.isPosterUnsent(81) }

        assertThat(repo.isPosterUnsent(81)).isFalse()
        repo.repairPosters()
        assertThat(previewUploads()).isEmpty()
    }
}

/*
 * MainViewModelShareTest.kt
 * Family Connect (Android)
 *
 * The OS-share flow's lifecycle rules, driven with a scripted importer:
 * a prepared share opens the picker and lands in the stash; cancelling
 * the flow mid-Preparing kills the import — an import that finished its
 * copy anyway must neither deposit nor reopen the picker, and must
 * delete the files it prepared; and a newer share supersedes an
 * in-flight one, whose stale result must not clobber the newer stash.
 *
 * Robolectric only for android.net.Uri; nothing renders.
 */

package me.nettrash.familyconnect

import android.net.Uri
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.net.dto.AttachmentDto
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.MediaPrep
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.repo.ShareImporter
import me.nettrash.familyconnect.data.repo.ShareIn
import me.nettrash.familyconnect.data.repo.ShareStash
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import java.io.File

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class MainViewModelShareTest {

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
    }

    /**
     * Each prepare() call takes the next scripted entry: an optional gate
     * to wait behind, and its result. The gate is awaited NonCancellable
     * on purpose — it models the worst case, an importer whose copy
     * finished despite the cancel and so RETURNS instead of throwing.
     */
    private class ScriptedImporter : ShareImporter {
        val calls = ArrayDeque<Pair<CompletableDeferred<Unit>?, List<MediaPrep.Prepared>>>()

        override suspend fun prepare(uris: List<Uri>): List<MediaPrep.Prepared> {
            val (gate, result) = calls.removeFirst()
            if (gate != null) withContext(NonCancellable) { gate.await() }
            return result
        }
    }

    private fun tempPrepared(tag: Byte): MediaPrep.Prepared {
        val file = File.createTempFile("fc-share", ".jpg").apply { writeBytes(ByteArray(8) { tag }) }
        return MediaPrep.Prepared(
            file = file,
            mime = "image/jpeg",
            kind = AttachmentDto.KIND_PHOTO,
            width = 10,
            height = 10,
            durationMs = null,
            previewJpeg = null,
        )
    }

    private fun newViewModel(importer: ShareImporter, stash: ShareStash): MainViewModel {
        val sessionRepository = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = FakeSettingsRepository(
                SettingsState(
                    serverUrl = "https://chat.example.com",
                    familyStatus = FamilyStatus.MEMBER,
                ),
            ),
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        return MainViewModel(
            sessionRepository = sessionRepository,
            shareImporter = importer,
            shareStash = stash,
        )
    }

    private fun imageShare(n: Int = 1): Pair<List<Uri>, List<ShareIn.Stream>> {
        val uris = List(n) { Uri.parse("content://shared/$it") }
        val streams = List(n) { ShareIn.Stream(scheme = "content", mime = "image/jpeg") }
        return uris to streams
    }

    @Test
    fun `a prepared share opens the picker and lands in the stash`() = runTest(dispatcher) {
        val importer = ScriptedImporter()
        val stash = ShareStash()
        val item = tempPrepared(tag = 1)
        importer.calls += null to listOf(item)
        val viewModel = newViewModel(importer, stash)
        val (uris, streams) = imageShare()

        viewModel.onShared(uris, streams, text = null)
        advanceUntilIdle()

        assertThat(viewModel.shareFlow.value)
            .isEqualTo(MainViewModel.ShareFlow.ChooseChat(itemCount = 1, hasText = false))

        viewModel.shareChatChosen(42L)
        assertThat(viewModel.shareFlow.value).isNull()
        assertThat(stash.claim(42L)!!.items).containsExactly(item)
        assertThat(item.file.exists()).isTrue()
    }

    /**
     * Dismissing the preparing sheet must CANCEL the import, not just the
     * UI: before the fix the untracked coroutine deposited into the stash
     * when the copy finished and the chat picker popped back up for a
     * share the user had already abandoned.
     */
    @Test
    fun `cancelling mid-prepare neither deposits nor reopens the flow`() = runTest(dispatcher) {
        val importer = ScriptedImporter()
        val stash = ShareStash()
        val gate = CompletableDeferred<Unit>()
        val item = tempPrepared(tag = 2)
        importer.calls += gate to listOf(item)
        val viewModel = newViewModel(importer, stash)
        val (uris, streams) = imageShare()

        viewModel.onShared(uris, streams, text = null)
        runCurrent()
        assertThat(viewModel.shareFlow.value).isEqualTo(MainViewModel.ShareFlow.Preparing)

        // The preparing sheet was swiped away.
        viewModel.cancelShare()
        assertThat(viewModel.shareFlow.value).isNull()

        // The copy finishes anyway — the dismissed share must stay dead.
        gate.complete(Unit)
        advanceUntilIdle()

        assertThat(viewModel.shareFlow.value).isNull()
        stash.target(42L)
        assertThat(stash.claim(42L)).isNull()
        // And the files it prepared were cleaned up: nothing will ever
        // claim them.
        assertThat(item.file.exists()).isFalse()
    }

    /** A stale import must not clobber the NEWER share it was superseded by. */
    @Test
    fun `a superseded import cleans up after itself and leaves the newer share alone`() =
        runTest(dispatcher) {
            val importer = ScriptedImporter()
            val stash = ShareStash()
            val slowGate = CompletableDeferred<Unit>()
            val slowItem = tempPrepared(tag = 3)
            val quickItem = tempPrepared(tag = 4)
            importer.calls += slowGate to listOf(slowItem)
            importer.calls += null to listOf(quickItem)
            val viewModel = newViewModel(importer, stash)

            // A big video starts copying…
            val (slowUris, slowStreams) = imageShare()
            viewModel.onShared(slowUris, slowStreams, text = null)
            runCurrent()
            viewModel.cancelShare()

            // …then a different photo is shared and prepares immediately.
            val (quickUris, quickStreams) = imageShare()
            viewModel.onShared(quickUris, quickStreams, text = null)
            advanceUntilIdle()
            assertThat(viewModel.shareFlow.value)
                .isEqualTo(MainViewModel.ShareFlow.ChooseChat(itemCount = 1, hasText = false))

            // The old video's copy lands late: it must not replace the
            // photo in the stash (its deposit used to DELETE the newer
            // share's files) nor change what the picker offers.
            slowGate.complete(Unit)
            advanceUntilIdle()

            assertThat(viewModel.shareFlow.value)
                .isEqualTo(MainViewModel.ShareFlow.ChooseChat(itemCount = 1, hasText = false))
            assertThat(slowItem.file.exists()).isFalse()

            viewModel.shareChatChosen(42L)
            val claim = stash.claim(42L)
            assertThat(claim!!.items).containsExactly(quickItem)
            assertThat(quickItem.file.exists()).isTrue()
        }
}

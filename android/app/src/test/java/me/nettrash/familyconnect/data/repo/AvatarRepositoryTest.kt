/*
 * AvatarRepositoryTest.kt
 * Family Connect (Android)
 *
 * The cache's whole job is deciding when NOT to ask the server: a roster
 * of people without pictures must not re-request on every scroll, and a
 * changed picture must never be served from the previous version's entry.
 *
 * Robolectric because decoding needs a real BitmapFactory; the bitmaps it
 * hands back are shadows, which is fine — nothing here asserts on pixels.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.testutil.FakeAvatarApi
import me.nettrash.familyconnect.testutil.FakeConnectivityObserver
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class AvatarRepositoryTest {

    private val dispatcher = StandardTestDispatcher()

    @Before
    fun setUp() {
        // The repository marshals its writes to Dispatchers.Main.immediate
        // so snapshot state is never mutated from two threads at once.
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    /**
     * The repository's settings collector runs for the life of the app
     * scope and never completes, so it goes on `backgroundScope` — on the
     * test scope itself runTest would wait forever for it to finish.
     *
     * Which is also why every test here drives the clock with runCurrent()
     * rather than advanceUntilIdle(): advanceUntilIdle deliberately leaves
     * background-scope tasks alone (an endless one would hang it), so the
     * collector would never see a single emission.
     */
    private fun TestScope.repository(
        api: FakeAvatarApi = FakeAvatarApi(),
        settings: FakeSettingsRepository = FakeSettingsRepository(),
        connectivity: FakeConnectivityObserver = FakeConnectivityObserver(),
    ) = AvatarRepository(api, settings, connectivity, backgroundScope)

    @Test
    fun `a fetched picture is cached and not requested twice`() = runTest(dispatcher) {
        val api = FakeAvatarApi()
        val repo = repository(api)

        repo.load(userId = 7, version = 3)
        runCurrent()
        assertThat(repo.cached(7, 3)).isNotNull()

        repo.load(userId = 7, version = 3)
        runCurrent()
        assertThat(api.fetches).containsExactly(7L to 3L)
    }

    @Test
    fun `a new version is a different key, so it refetches`() = runTest(dispatcher) {
        val api = FakeAvatarApi()
        val repo = repository(api)

        repo.load(userId = 7, version = 3)
        runCurrent()
        repo.load(userId = 7, version = 4)
        runCurrent()

        assertThat(api.fetches).containsExactly(7L to 3L, 7L to 4L).inOrder()
        // The old face must not answer for the new version.
        assertThat(repo.cached(7, 4)).isNotNull()
    }

    @Test
    fun `version zero means no picture and never hits the network`() = runTest(dispatcher) {
        val api = FakeAvatarApi()
        val repo = repository(api)

        repo.load(userId = 7, version = 0)
        runCurrent()

        assertThat(api.fetches).isEmpty()
        assertThat(repo.cached(7, 0)).isNull()
    }

    @Test
    fun `a 404 is remembered - no picture is not retried`() = runTest(dispatcher) {
        val api = FakeAvatarApi().apply {
            onFetch = { _, _ -> ApiResult.HttpError(404, "user_not_found", "no") }
        }
        val repo = repository(api)

        repeat(3) {
            repo.load(userId = 9, version = 2)
            runCurrent()
        }

        assertThat(api.fetches).hasSize(1)
        assertThat(repo.cached(9, 2)).isNull()
    }

    @Test
    fun `a transport failure stays retryable`() = runTest(dispatcher) {
        val api = FakeAvatarApi().apply {
            onFetch = { _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        }
        val repo = repository(api)

        repo.load(userId = 9, version = 2)
        runCurrent()
        repo.load(userId = 9, version = 2)
        runCurrent()

        assertThat(api.fetches).hasSize(2)
    }

    @Test
    fun `a different account does not inherit the previous one's faces`() = runTest(dispatcher) {
        val api = FakeAvatarApi()
        val settings = FakeSettingsRepository()
        val repo = repository(api, settings)
        settings.setProfile(userId = 7, username = "anna", displayName = "Anna", avatarVersion = 3)
        runCurrent()

        repo.load(userId = 7, version = 3)
        runCurrent()
        assertThat(repo.cached(7, 3)).isNotNull()

        // Logout wipes the stored profile; the cache goes with it.
        settings.resetKeepingServerUrl()
        runCurrent()

        assertThat(repo.cached(7, 3)).isNull()
    }


    /**
     * The caller is a LaunchedEffect that dies when its row scrolls away.
     * If that cancellation took the fetch — and the in-flight marker —
     * with it, the key would stay marked forever and that face would
     * never load again for the life of the process.
     */
    @Test
    fun `cancelling the caller does not strand the fetch`() = runTest(dispatcher) {
        val gate = CompletableDeferred<Unit>()
        val api = FakeAvatarApi().apply { this.gate = gate }
        val repo = repository(api)

        val scrolledAway = launch { repo.load(userId = 7, version = 3) }
        runCurrent()
        scrolledAway.cancel()
        runCurrent()

        // The row came back before the server answered.
        gate.complete(Unit)
        runCurrent()
        assertThat(repo.cached(7, 3)).isNotNull()

        repo.load(userId = 7, version = 3)
        runCurrent()
        assertThat(api.fetches).hasSize(1)
    }

    @Test
    fun `a fetch still in flight at logout is not cached for the next account`() =
        runTest(dispatcher) {
            val gate = CompletableDeferred<Unit>()
            val api = FakeAvatarApi().apply { this.gate = gate }
            val settings = FakeSettingsRepository()
            val repo = repository(api, settings)
            settings.setProfile(userId = 7, username = "anna", displayName = "Anna", avatarVersion = 3)
            runCurrent()

            launch { repo.load(userId = 7, version = 3) }
            runCurrent()
            settings.resetKeepingServerUrl()
            runCurrent()

            // The previous session's request lands after the logout.
            gate.complete(Unit)
            runCurrent()
            assertThat(repo.cached(7, 3)).isNull()
        }

    @Test
    fun `the network coming back retries what failed while offline`() = runTest(dispatcher) {
        val api = FakeAvatarApi().apply {
            onFetch = { _, _ -> ApiResult.NetworkError(IllegalStateException("offline")) }
        }
        val connectivity = FakeConnectivityObserver(online = false)
        val repo = repository(api, connectivity = connectivity)
        val before = repo.retryToken

        repo.load(userId = 7, version = 3)
        runCurrent()
        assertThat(api.fetches).hasSize(1)

        connectivity.onAvailable.emit(Unit)
        runCurrent()

        // The token is what a composable keys its load effect on, so a
        // bump is exactly what re-runs the fetch for every visible row.
        assertThat(repo.retryToken).isGreaterThan(before)
    }
}

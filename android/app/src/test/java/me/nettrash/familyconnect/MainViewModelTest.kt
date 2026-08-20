/*
 * MainViewModelTest.kt
 * Family Connect (Android)
 *
 * Boot gate + push deep-link holding pattern: the boot snapshot loads
 * once, and a PendingRoute parsed from a notification tap is held as
 * state until the NavHost consumes it exactly once (a newer tap
 * replaces an unconsumed older one). Plain JVM, fakes only.
 */

package me.nettrash.familyconnect

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.push.PendingRoute
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import org.junit.After
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class MainViewModelTest {

    private val dispatcher = StandardTestDispatcher()

    // Foreground scope, not runTest's backgroundScope — advanceUntilIdle
    // skips background-only work (see SessionRepositoryTest).
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

    private fun newViewModel(): MainViewModel {
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
        return MainViewModel(sessionRepository)
    }

    @Test
    fun bootStateLoadsTheSnapshotOnce() = runTest(dispatcher) {
        val viewModel = newViewModel()
        assertThat(viewModel.bootState.value).isNull() // spinner phase

        runCurrent()

        assertThat(viewModel.bootState.value?.status).isEqualTo(FamilyStatus.MEMBER)
    }

    @Test
    fun pendingRouteIsHeldUntilConsumedExactlyOnce() = runTest(dispatcher) {
        val viewModel = newViewModel()
        assertThat(viewModel.pendingRoute.value).isNull()

        viewModel.onPendingRoute(PendingRoute.Chat(42L))
        assertThat(viewModel.pendingRoute.value).isEqualTo(PendingRoute.Chat(42L))

        viewModel.consumePendingRoute()
        assertThat(viewModel.pendingRoute.value).isNull()
    }

    @Test
    fun aNewerTapReplacesAnUnconsumedRoute() = runTest(dispatcher) {
        val viewModel = newViewModel()

        viewModel.onPendingRoute(PendingRoute.Chat(42L))
        viewModel.onPendingRoute(PendingRoute.JoinRequests)

        assertThat(viewModel.pendingRoute.value).isEqualTo(PendingRoute.JoinRequests)
    }
}

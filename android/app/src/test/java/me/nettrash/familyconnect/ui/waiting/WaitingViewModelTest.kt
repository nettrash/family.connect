/*
 * WaitingViewModelTest.kt
 * Family Connect (Android)
 *
 * Approval limbo: the 20 s ticker polls /me, approval flips `approved`,
 * PENDING → nothing flips `declined`. Virtual time; note advanceTimeBy
 * (never advanceUntilIdle — the ticker reschedules forever).
 */

package me.nettrash.familyconnect.ui.waiting

import androidx.lifecycle.viewModelScope
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.userDto
import org.junit.After
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class WaitingViewModelTest {

    private val dispatcher = StandardTestDispatcher()
    private val authApi = FakeAuthApi()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.PENDING,
            familyName = "The Smiths",
        ),
    )

    private val pendingMe = ApiResult.Ok(
        MeResponse(
            user = userDto(7, "anna"),
            pendingJoinRequest = PendingJoinRequestDto(3, "The Smiths", "2026-08-19T10:00:00Z"),
        ),
    )
    private val approvedMe = ApiResult.Ok(
        MeResponse(
            user = userDto(7, "anna"),
            family = FamilyDto(3, "The Smiths", "approval"),
            role = "member",
        ),
    )
    private val rejectedMe = ApiResult.Ok(MeResponse(user = userDto(7, "anna")))

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        authApi.meResult = pendingMe
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    private fun newViewModel(): WaitingViewModel {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = CoroutineScope(dispatcher),
        )
        return WaitingViewModel(sessionRepository)
    }

    /**
     * runTest wrapper that always cancels the ViewModel's scope before
     * the test ends. The poll ticker reschedules itself on the shared
     * virtual scheduler; left alive it would keep runTest's cleanup
     * advancing the clock forever.
     */
    private fun runVmTest(block: suspend TestScope.(WaitingViewModel) -> Unit) =
        runTest(dispatcher) {
            val viewModel = newViewModel()
            try {
                block(viewModel)
            } finally {
                viewModel.viewModelScope.cancel()
            }
        }

    @Test
    fun showsTheFamilyNameFromTheSnapshot() = runVmTest { viewModel ->
        runCurrent()
        assertThat(viewModel.state.value.familyName).isEqualTo("The Smiths")
        assertThat(viewModel.state.value.approved).isFalse()
        assertThat(viewModel.state.value.declined).isFalse()
    }

    @Test
    fun manualRefreshWhileStillPendingChangesNothing() = runVmTest { viewModel ->
        runCurrent()

        viewModel.refresh()
        runCurrent()

        assertThat(viewModel.state.value.approved).isFalse()
        assertThat(viewModel.state.value.declined).isFalse()
        assertThat(authApi.meCalls).isEqualTo(1)
    }

    @Test
    fun tickerPollsAndDiscoversApproval() = runVmTest { viewModel ->
        runCurrent()
        assertThat(authApi.meCalls).isEqualTo(0) // ticker hasn't fired yet

        authApi.meResult = approvedMe
        advanceTimeBy(20_001)
        runCurrent()

        assertThat(authApi.meCalls).isEqualTo(1)
        assertThat(viewModel.state.value.approved).isTrue()
    }

    @Test
    fun pendingGoneMeansDeclined() = runVmTest { viewModel ->
        runCurrent()

        authApi.meResult = rejectedMe
        viewModel.refresh()
        runCurrent()

        assertThat(viewModel.state.value.declined).isTrue()
        assertThat(viewModel.state.value.approved).isFalse()
    }

    @Test
    fun declinedStateSticksAcrossFurtherPolls() = runVmTest { viewModel ->
        runCurrent()

        authApi.meResult = rejectedMe
        viewModel.refresh()
        runCurrent()
        // Next poll still sees NONE; the flag must not flap.
        advanceTimeBy(20_001)
        runCurrent()

        assertThat(viewModel.state.value.declined).isTrue()
    }
}

/*
 * StatisticsViewModelTest.kt
 * Family Connect (Android)
 *
 * Three states and one fetch. The screen has no retry-on-resume and no
 * cache by design, so what matters here is that a failure lands as Failed
 * rather than an empty Loaded — a screen of zeroes would read as a family
 * that has never said anything.
 */

package me.nettrash.familyconnect.ui.stats

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AttachmentStatsDto
import me.nettrash.familyconnect.data.net.dto.FamilyStatsDto
import me.nettrash.familyconnect.data.net.dto.MemberStatsDto
import me.nettrash.familyconnect.data.net.dto.StatsTotalsDto
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import org.junit.After
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class StatisticsViewModelTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var api: FakeFamilyApi

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        api = FakeFamilyApi()
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    private fun stats() = FamilyStatsDto(
        totals = StatsTotalsDto(
            members = 3,
            messages = 42,
            boardNotes = 4,
            attachments = AttachmentStatsDto(count = 5, bytes = 2_000, storedBytes = 1_200),
        ),
        members = listOf(
            MemberStatsDto(userId = 7, displayName = "Olive", messages = 30),
        ),
    )

    @Test
    fun `starts loading and lands on the numbers`() = runTest(dispatcher) {
        api.statsResult = ApiResult.Ok(stats())

        val vm = StatisticsViewModel(api)
        assertThat(vm.state.value).isEqualTo(StatisticsViewModel.State.Loading)

        runCurrent()
        val loaded = vm.state.value as StatisticsViewModel.State.Loaded
        assertThat(loaded.stats.totals.messages).isEqualTo(42)
        assertThat(loaded.stats.members).hasSize(1)
    }

    /** A dead server must not look like a silent family. */
    @Test
    fun `a network failure is Failed, not an empty screen`() = runTest(dispatcher) {
        api.statsResult = ApiResult.NetworkError(java.io.IOException("offline"))

        val vm = StatisticsViewModel(api)
        runCurrent()

        assertThat(vm.state.value).isEqualTo(StatisticsViewModel.State.Failed)
    }

    /** So does a server that answers, unhappily. */
    @Test
    fun `an http failure is Failed too`() = runTest(dispatcher) {
        api.statsResult = ApiResult.HttpError(500, null, null)

        val vm = StatisticsViewModel(api)
        runCurrent()

        assertThat(vm.state.value).isEqualTo(StatisticsViewModel.State.Failed)
    }

    /** Retrying goes back to the server rather than replaying a cache. */
    @Test
    fun `load fetches again`() = runTest(dispatcher) {
        api.statsResult = ApiResult.NetworkError(java.io.IOException("offline"))
        val vm = StatisticsViewModel(api)
        runCurrent()
        assertThat(api.statsCalls).isEqualTo(1)

        api.statsResult = ApiResult.Ok(stats())
        vm.load()
        assertThat(vm.state.value).isEqualTo(StatisticsViewModel.State.Loading)
        runCurrent()

        assertThat(api.statsCalls).isEqualTo(2)
        assertThat(vm.state.value).isInstanceOf(StatisticsViewModel.State.Loaded::class.java)
    }
}

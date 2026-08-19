/*
 * ServerSetupViewModelTest.kt
 * Family Connect (Android)
 *
 * The pre-fill contract: the field starts with the URL currently in
 * effect (typed earlier or the store build's adopted default) so
 * "Use a different server" edits instead of retypes, stays blank on a
 * true first run, never clobbers input the user already started typing,
 * and saving a different URL overrides the stored one persistently.
 */

package me.nettrash.familyconnect.ui.serversetup

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import org.junit.After
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ServerSetupViewModelTest {

    private val dispatcher = StandardTestDispatcher()
    private val authApi = FakeAuthApi()

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    @Test
    fun prefillsTheFieldWithTheUrlCurrentlyInEffect() = runTest(dispatcher) {
        val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://fc.nettrash.me"))
        val viewModel = ServerSetupViewModel(authApi, settings)
        advanceUntilIdle()

        assertThat(viewModel.state.value.url).isEqualTo("https://fc.nettrash.me")
        assertThat(viewModel.state.value.isCleartext).isFalse()
    }

    @Test
    fun staysBlankOnATrueFirstRunWithoutAStoredUrl() = runTest(dispatcher) {
        val viewModel = ServerSetupViewModel(authApi, FakeSettingsRepository())
        advanceUntilIdle()

        assertThat(viewModel.state.value.url).isEmpty()
    }

    @Test
    fun prefillNeverClobbersInputTheUserAlreadyStartedTyping() = runTest(dispatcher) {
        val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://fc.nettrash.me"))
        val viewModel = ServerSetupViewModel(authApi, settings)

        // Typed before the DataStore read lands (init's launch hasn't run
        // yet on the StandardTestDispatcher).
        viewModel.onUrlChange("my-ow")
        advanceUntilIdle()

        assertThat(viewModel.state.value.url).isEqualTo("my-ow")
    }

    @Test
    fun savingADifferentUrlOverridesTheStoredOnePersistently() = runTest(dispatcher) {
        val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://fc.nettrash.me"))
        val viewModel = ServerSetupViewModel(authApi, settings)
        advanceUntilIdle()

        // A live Family Connect server answers the unauthenticated probe
        // with 401 + a parsed protocol error code.
        authApi.probeResult = ApiResult.HttpError(401, "unauthorized", "authentication required")
        viewModel.onUrlChange("my-own-box.example.com")
        viewModel.probeAndSave()
        advanceUntilIdle()

        assertThat(viewModel.state.value.saved).isTrue()
        assertThat(settings.current.serverUrl).isEqualTo("https://my-own-box.example.com")
    }
}

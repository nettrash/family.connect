/*
 * AuthViewModelTest.kt
 * Family Connect (Android)
 *
 * Validation gates (no network on invalid input), the 409 → username
 * field mapping, and the route-by-status handoff after success. The
 * SessionRepository underneath is real; only the wire is faked.
 */

package me.nettrash.familyconnect.ui.auth

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.test.setMain
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AuthResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
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
class AuthViewModelTest {

    private val dispatcher = StandardTestDispatcher()
    private val authApi = FakeAuthApi()
    private val tokenStore = FakeTokenStore()
    private val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com"))

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
    }

    @After
    fun tearDown() {
        Dispatchers.resetMain()
    }

    private fun newViewModel(): AuthViewModel {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = CoroutineScope(dispatcher),
        )
        return AuthViewModel(sessionRepository)
    }

    @Test
    fun invalidUsernameBlocksSubmitWithoutTouchingTheNetwork() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.onUsernameChange("ab") // too short
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.usernameError).isNotNull()
        assertThat(authApi.loginCalls).isEqualTo(0)
        assertThat(authApi.registerCalls).isEqualTo(0)
    }

    @Test
    fun usernameWithIllegalCharactersIsRejected() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.onUsernameChange("anna smith!") // space and !
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.usernameError).isNotNull()
        assertThat(authApi.loginCalls).isEqualTo(0)
    }

    @Test
    fun shortPasswordBlocksSubmit() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.onUsernameChange("anna")
        viewModel.onPasswordChange("seven77") // 7 chars

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.passwordError).isNotNull()
        assertThat(authApi.loginCalls).isEqualTo(0)
    }

    @Test
    fun registerRequiresADisplayName() = runTest(dispatcher) {
        val viewModel = newViewModel()
        viewModel.setMode(AuthViewModel.Mode.REGISTER)
        viewModel.onUsernameChange("anna")
        viewModel.onPasswordChange("password123")
        viewModel.onDisplayNameChange("   ")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.displayNameError).isNotNull()
        assertThat(authApi.registerCalls).isEqualTo(0)
    }

    @Test
    fun usernameTakenLandsOnTheUsernameField() = runTest(dispatcher) {
        authApi.registerResult =
            ApiResult.HttpError(409, "username_taken", "username is already in use")
        val viewModel = newViewModel()
        viewModel.setMode(AuthViewModel.Mode.REGISTER)
        viewModel.onUsernameChange("anna")
        viewModel.onDisplayNameChange("Anna")
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.usernameError).isEqualTo("Username already taken")
        assertThat(viewModel.state.value.authenticatedStatus).isNull()
    }

    @Test
    fun invalidCredentialsShowAGeneralError() = runTest(dispatcher) {
        authApi.loginResult = ApiResult.HttpError(401, "invalid_credentials", "wrong")
        val viewModel = newViewModel()
        viewModel.onUsernameChange("anna")
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.generalError).isEqualTo("Wrong username or password")
        assertThat(viewModel.state.value.usernameError).isNull()
    }

    @Test
    fun successfulLoginExposesTheReconciledStatusForRouting() = runTest(dispatcher) {
        authApi.loginResult = ApiResult.Ok(AuthResponse("tok", userDto(7, "anna")))
        authApi.meResult = ApiResult.Ok(
            MeResponse(
                user = userDto(7, "anna"),
                family = FamilyDto(3, "The Smiths", "open"),
                role = "member",
            ),
        )
        val viewModel = newViewModel()
        viewModel.onUsernameChange("anna")
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.authenticatedStatus).isEqualTo(FamilyStatus.MEMBER)
        assertThat(authApi.deviceCalls).isEqualTo(1) // device registered post-login
        assertThat(tokenStore.token).isEqualTo("tok")
    }

    @Test
    fun successfulLoginWithoutAFamilyRoutesToTheGate() = runTest(dispatcher) {
        authApi.loginResult = ApiResult.Ok(AuthResponse("tok", userDto(7, "anna")))
        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(7, "anna")))
        val viewModel = newViewModel()
        viewModel.onUsernameChange("anna")
        viewModel.onPasswordChange("password123")

        viewModel.submit()
        advanceUntilIdle()

        assertThat(viewModel.state.value.authenticatedStatus).isEqualTo(FamilyStatus.NONE)
    }
}

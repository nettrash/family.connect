/*
 * SessionRepositoryTest.kt
 * Family Connect (Android)
 *
 * Session semantics: snapshot → start-route mapping, the protocol's
 * "wipe local state but KEEP the server URL" rule, and the global 401 →
 * Expired propagation. Plain JVM — every dependency is a fake.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AuthResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.navigation.Routes
import me.nettrash.familyconnect.navigation.startDestinationFor
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.userDto
import org.junit.Test

class SessionRepositoryTest {

    private val authApi = FakeAuthApi()
    private val tokenStore = FakeTokenStore()
    private val settings = FakeSettingsRepository()
    private val wiper = RecordingWiper()
    private val unauthorized = MutableSharedFlow<Unit>(extraBufferCapacity = 1)

    private fun TestScope.newRepository(): SessionRepository {
        val repository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = wiper,
            unauthorizedEvents = unauthorized,
            scope = backgroundScope,
        )
        runCurrent() // let the 401 collector subscribe
        return repository
    }

    private val family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "approval")

    // -- Snapshot → route mapping ------------------------------------------------

    @Test
    fun snapshotStatusIsDerivedAndMapsToStartRoutes() = runTest {
        val repository = newRepository()

        // No server URL saved at all.
        assertThat(repository.snapshot().status).isEqualTo(FamilyStatus.NO_SERVER)
        assertThat(startDestinationFor(FamilyStatus.NO_SERVER)).isEqualTo(Routes.SERVER_SETUP)

        // URL but no token.
        settings.setServerUrl("https://chat.example.com")
        assertThat(repository.snapshot().status).isEqualTo(FamilyStatus.NO_TOKEN)
        assertThat(startDestinationFor(FamilyStatus.NO_TOKEN)).isEqualTo(Routes.AUTH)

        // Token, no family.
        tokenStore.save("tok")
        assertThat(repository.snapshot().status).isEqualTo(FamilyStatus.NONE)
        assertThat(startDestinationFor(FamilyStatus.NONE)).isEqualTo(Routes.FAMILY_GATE)

        settings.setFamilyStatus(FamilyStatus.PENDING)
        assertThat(startDestinationFor(repository.snapshot().status)).isEqualTo(Routes.WAITING)

        settings.setFamilyStatus(FamilyStatus.MEMBER)
        assertThat(startDestinationFor(repository.snapshot().status)).isEqualTo(Routes.CHAT_LIST)

        settings.setFamilyStatus(FamilyStatus.OWNER)
        assertThat(startDestinationFor(repository.snapshot().status)).isEqualTo(Routes.CHAT_LIST)
    }

    @Test
    fun canChatRequiresTokenAndMembership() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("tok")

        settings.setFamilyStatus(FamilyStatus.PENDING)
        assertThat(repository.snapshot().canChat).isFalse()

        settings.setFamilyStatus(FamilyStatus.MEMBER)
        assertThat(repository.snapshot().canChat).isTrue()

        tokenStore.clear()
        assertThat(repository.snapshot().canChat).isFalse()
    }

    // -- clearSession -------------------------------------------------------------

    @Test
    fun clearSessionWipesEverythingButKeepsTheServerUrl() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        settings.setProfile(7, "anna", "Anna")
        settings.setFamilyStatus(FamilyStatus.OWNER)
        settings.setFamilyName("The Smiths")
        tokenStore.save("tok")

        repository.clearSession()

        assertThat(tokenStore.token).isNull()
        assertThat(wiper.wipeCount).isEqualTo(1)
        val state = settings.state.first()
        assertThat(state.serverUrl).isEqualTo("https://chat.example.com") // kept!
        assertThat(state.myUserId).isNull()
        assertThat(state.familyName).isNull()
        assertThat(state.familyStatus).isEqualTo(FamilyStatus.NONE)
    }

    // -- 401 propagation --------------------------------------------------------------

    @Test
    fun unauthorizedEventClearsTheSessionAndEmitsExpired() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("tok")

        val event = async { repository.sessionEvents.first() }
        runCurrent()

        unauthorized.tryEmit(Unit)
        advanceUntilIdle()

        assertThat(event.await()).isEqualTo(SessionEvent.Expired)
        assertThat(tokenStore.token).isNull()
        assertThat(wiper.wipeCount).isEqualTo(1)
        assertThat(settings.state.first().serverUrl).isEqualTo("https://chat.example.com")
    }

    // -- Auth flow ------------------------------------------------------------------------

    @Test
    fun loginStoresTokenRefreshesMeAndRegistersDevice() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        authApi.loginResult = ApiResult.Ok(AuthResponse(token = "fresh-token", user = userDto(7, "anna")))
        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(7, "anna"), family = family, role = "member"))

        val result = repository.login("anna", "password123")

        assertThat(tokenStore.token).isEqualTo("fresh-token")
        assertThat(authApi.meCalls).isEqualTo(1)
        assertThat(authApi.deviceCalls).isEqualTo(1)
        val snapshot = (result as ApiResult.Ok).value
        assertThat(snapshot.status).isEqualTo(FamilyStatus.MEMBER)
        assertThat(snapshot.familyName).isEqualTo("The Smiths")
    }

    @Test
    fun registerFailurePassesTheHttpErrorThrough() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        authApi.registerResult = ApiResult.HttpError(409, "username_taken", "username is already in use")

        val result = repository.register("anna", "Anna", "password123")

        assertThat(result).isInstanceOf(ApiResult.HttpError::class.java)
        assertThat((result as ApiResult.HttpError).code).isEqualTo("username_taken")
        assertThat(tokenStore.token).isNull()
        assertThat(authApi.deviceCalls).isEqualTo(0)
    }

    // -- refreshMe reconciliation ---------------------------------------------------------------

    @Test
    fun refreshMeMapsFamilyRoleAndPendingOntoStatus() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("tok")

        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(7), family = family, role = "owner"))
        assertThat((repository.refreshMe() as ApiResult.Ok).value.status).isEqualTo(FamilyStatus.OWNER)

        authApi.meResult = ApiResult.Ok(
            MeResponse(
                user = userDto(7),
                pendingJoinRequest = PendingJoinRequestDto(3, "The Smiths", "2026-08-19T10:00:00Z"),
            ),
        )
        assertThat((repository.refreshMe() as ApiResult.Ok).value.status).isEqualTo(FamilyStatus.PENDING)
    }

    @Test
    fun pendingToNoneEmitsJoinRequestRejected() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("tok")
        settings.setFamilyStatus(FamilyStatus.PENDING)

        val event = async { repository.sessionEvents.first() }
        runCurrent()

        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(7))) // no family, no request
        repository.refreshMe()
        advanceUntilIdle()

        assertThat(event.await()).isEqualTo(SessionEvent.JoinRequestRejected)
    }

    @Test
    fun memberToNoneEmitsRemovedFromFamilyAndWipes() = runTest {
        val repository = newRepository()
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("tok")
        settings.setFamilyStatus(FamilyStatus.MEMBER)

        val event = async { repository.sessionEvents.first() }
        runCurrent()

        authApi.meResult = ApiResult.Ok(MeResponse(user = userDto(7)))
        repository.refreshMe()
        advanceUntilIdle()

        assertThat(event.await()).isEqualTo(SessionEvent.RemovedFromFamily)
        assertThat(wiper.wipeCount).isEqualTo(1)
    }
}

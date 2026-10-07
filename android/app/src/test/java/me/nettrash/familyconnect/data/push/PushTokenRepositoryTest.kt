/*
 * PushTokenRepositoryTest.kt
 * Family Connect (Android)
 *
 * Device-registration lifecycle (docs/protocol.md, "Push notifications"):
 * register while logged in, cache while logged out, re-POST only on
 * change, retry after a failed POST, and the logout split — device id
 * dies with the session, the FCM token survives it. Plain JVM, every
 * dependency a fake.
 */

package me.nettrash.familyconnect.data.push

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.DeviceResponse
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class PushTokenRepositoryTest {

    private val dispatcher = StandardTestDispatcher()

    private val authApi = FakeAuthApi()
    private val tokenStore = FakeTokenStore()
    private val settings = FakeSettingsRepository()

    /** Firebase's answer; null = no google-services.json in this build. */
    private var firebaseToken: String? = null

    /** The locale folder the app is shown in (#82). */
    private var shown = "en"

    private fun newRepository() = PushTokenRepository(
        authApi = authApi,
        settings = settings,
        tokenStore = tokenStore,
        pushTokenProvider = { firebaseToken },
        pushLanguage = { shown },
    )

    private suspend fun logIn() {
        settings.setServerUrl("https://chat.example.com")
        tokenStore.save("session-token")
    }

    // -- Registration ------------------------------------------------------------

    @Test
    fun registersTheTokenWhenLoggedIn() = runTest(dispatcher) {
        logIn()
        authApi.deviceResult = ApiResult.Ok(DeviceResponse(deviceId = 5))
        val repository = newRepository()

        repository.onNewToken("fcm-1")

        assertThat(authApi.deviceRegistrations).containsExactly("fcm-1")
        val state = settings.state.first()
        assertThat(state.pushToken).isEqualTo("fcm-1")
        assertThat(state.pushDeviceId).isEqualTo(5L)
    }

    // -- The language pushes are written in (docs/protocol.md, "The words of a push"; #82) --

    @Test
    fun theShownLanguageRidesAlongAndANewOneRePostsTheSameToken() = runTest(dispatcher) {
        logIn()
        val repository = newRepository()

        shown = "ru"
        repository.onNewToken("fcm-1")
        repository.registerCurrentToken() // same token, same language — not news
        firebaseToken = "fcm-1"
        repository.registerCurrentToken()
        assertThat(authApi.deviceLanguages).containsExactly("ru")

        // Switched to Serbian in Latin script: the same token goes again, in it.
        shown = "sr-Latn"
        repository.registerCurrentToken()
        assertThat(authApi.deviceRegistrations).containsExactly("fcm-1", "fcm-1").inOrder()
        assertThat(authApi.deviceLanguages).containsExactly("ru", "sr-Latn").inOrder()
        assertThat(settings.state.first().pushLanguage).isEqualTo("sr-Latn")
    }

    @Test
    fun aDeviceWithNoPushTokenIsNotReRegisteredForALanguage() = runTest(dispatcher) {
        logIn()
        val repository = newRepository() // no Firebase: a null token
        shown = "ru"
        repository.registerCurrentToken()
        shown = "de"
        repository.registerCurrentToken()

        // One row: a tokenless POST inserts, so a second would orphan the first.
        assertThat(authApi.deviceCalls).isEqualTo(1)
    }

    @Test
    fun theLanguageIsOneOfTheNineOrEnglish() {
        for (language in PushLanguage.ALL) {
            assertThat(PushLanguage.of(language)).isEqualTo(language)
        }
        assertThat(PushLanguage.of("zh-hans")).isEqualTo("zh-Hans")
        assertThat(PushLanguage.of(" de ")).isEqualTo("de")
        assertThat(PushLanguage.of("pt-BR")).isEqualTo("en")
        assertThat(PushLanguage.of(null)).isEqualTo("en")
    }

    @Test
    fun cachesTheTokenWhenLoggedOut() = runTest(dispatcher) {
        val repository = newRepository() // no server URL, no session token

        repository.onNewToken("fcm-1")

        assertThat(authApi.deviceCalls).isEqualTo(0)
        val state = settings.state.first()
        assertThat(state.pushToken).isEqualTo("fcm-1")
        assertThat(state.pushDeviceId).isNull()
    }

    @Test
    fun rePostsOnlyOnChange() = runTest(dispatcher) {
        logIn()
        val repository = newRepository()

        repository.onNewToken("fcm-1")
        repository.onNewToken("fcm-1") // rotation callback replayed — no-op
        repository.onNewToken("fcm-2") // real rotation — re-POST

        assertThat(authApi.deviceRegistrations).containsExactly("fcm-1", "fcm-2").inOrder()
        assertThat(settings.state.first().pushToken).isEqualTo("fcm-2")
    }

    @Test
    fun failedRegistrationIsRetriedOnTheNextAttempt() = runTest(dispatcher) {
        logIn()
        authApi.deviceResult = ApiResult.NetworkError(IllegalStateException("offline"))
        val repository = newRepository()

        repository.onNewToken("fcm-1")
        assertThat(settings.state.first().pushDeviceId).isNull() // POST failed

        // Same token again (e.g. the resync hook) — no device id stored,
        // so the "unchanged" short-circuit must NOT kick in.
        authApi.deviceResult = ApiResult.Ok(DeviceResponse(deviceId = 6))
        repository.registerCurrentToken()

        assertThat(authApi.deviceRegistrations).containsExactly("fcm-1", "fcm-1").inOrder()
        assertThat(settings.state.first().pushDeviceId).isEqualTo(6L)
    }

    // -- registerCurrentToken (login/resync hook) -----------------------------------

    @Test
    fun registerCurrentTokenPrefersFirebaseOverTheCache() = runTest(dispatcher) {
        logIn()
        settings.setPushToken("fcm-stale")
        firebaseToken = "fcm-fresh"
        val repository = newRepository()

        repository.registerCurrentToken()

        assertThat(authApi.deviceRegistrations).containsExactly("fcm-fresh")
    }

    @Test
    fun registerCurrentTokenFallsBackToTheCachedTokenAtLogin() = runTest(dispatcher) {
        // An onNewToken arrived while logged out and was cached…
        val repository = newRepository()
        repository.onNewToken("fcm-1")
        assertThat(authApi.deviceCalls).isEqualTo(0)

        // …then the user logs in on a build where Firebase can't answer
        // right now — the cache still gets the device registered.
        logIn()
        repository.registerCurrentToken()

        assertThat(authApi.deviceRegistrations).containsExactly("fcm-1")
    }

    @Test
    fun registerCurrentTokenWithoutFirebaseStillRegistersTheDeviceRow() = runTest(dispatcher) {
        // No google-services.json: provider yields null, nothing cached.
        // The protocol's v1 hook still applies — a null-token device row.
        logIn()
        val repository = newRepository()

        repository.registerCurrentToken()

        assertThat(authApi.deviceRegistrations).containsExactly(null as String?)
        assertThat(settings.state.first().pushDeviceId).isEqualTo(1L)

        // And the next resync must not re-POST another null row.
        repository.registerCurrentToken()
        assertThat(authApi.deviceCalls).isEqualTo(1)
    }

    // -- Logout (SessionRepository owns it, the split is asserted here) --------------

    @Test
    fun logoutDeletesTheDeviceRowClearsTheIdAndKeepsTheToken() = runTest(dispatcher) {
        logIn()
        settings.setPushToken("fcm-1")
        settings.setPushDeviceId(9L)
        // Foreground scope, not backgroundScope — see SessionRepositoryTest.
        val repoScope = CoroutineScope(dispatcher + SupervisorJob())
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        runCurrent()

        sessionRepository.logout()

        assertThat(authApi.deletedDeviceIds).containsExactly(9L)
        assertThat(authApi.logoutCalls).isEqualTo(1)
        val state = settings.state.first()
        assertThat(state.pushDeviceId).isNull() // account-scoped: gone
        assertThat(state.pushToken).isEqualTo("fcm-1") // device-scoped: kept
    }

    @Test
    fun logoutWithoutARegisteredDeviceSkipsTheDelete() = runTest(dispatcher) {
        logIn()
        val repoScope = CoroutineScope(dispatcher + SupervisorJob())
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        runCurrent()

        sessionRepository.logout()

        assertThat(authApi.deletedDeviceIds).isEmpty()
        assertThat(authApi.logoutCalls).isEqualTo(1)
    }
}

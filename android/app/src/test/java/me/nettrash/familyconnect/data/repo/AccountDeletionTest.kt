/*
 * AccountDeletionTest.kt
 * Family Connect (Android)
 *
 * Deleting your own account (docs/protocol.md, "Deleting an account") —
 * App Store guideline 5.1.1(v)'s in-app, self-service deletion.
 *
 * The teardown is the delicate half. It must NOT reuse the ordinary
 * logout path: that calls DELETE /devices/{id} and POST /auth/logout, and
 * by the time the 204 lands this session no longer exists, so both would
 * answer 401 — firing the unauthorized broadcast and racing a second
 * teardown against the first. What it must do instead is the app's own
 * "wipe local state and return to sign-in" primitive, cursors and cache
 * and all.
 *
 * The other half is what happens to the OTHER devices of an account that
 * has just been deleted: a REST 401 already brings them back to sign-in,
 * and a socket closed with 4401 now does too — before this it merely
 * reconnected forever behind a "Connecting…" banner.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import org.junit.After
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class AccountDeletionTest {

    private val dispatcher = StandardTestDispatcher()

    // Foreground scope, not backgroundScope — advanceUntilIdle skips
    // background-only work, which would starve the collectors.
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private val authApi = FakeAuthApi()
    private val tokenStore = FakeTokenStore("tok")
    private val wiper = RecordingWiper()
    private val unauthorized = MutableSharedFlow<Unit>(extraBufferCapacity = 1)
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            myUserId = 7,
            myUsername = "anna",
            myDisplayName = "Anna",
            familyName = "The Smiths",
            myAvatarVersion = 3,
            pushDeviceId = 4,
            // The board catch-up cursor — the one sync cursor that lives
            // outside Room, and the one a teardown can most easily forget.
            boardCursor = 88,
            boardSeenNoteId = 12,
        ),
    )

    @After
    fun tearDown() = repoScope.cancel()

    private fun TestScope.newRepository(): SessionRepository {
        val repository = SessionRepository(
            authApi = authApi,
            tokenStore = tokenStore,
            settings = settings,
            wiper = wiper,
            unauthorizedEvents = unauthorized,
            scope = repoScope,
        )
        runCurrent() // let the 401 collector subscribe
        return repository
    }

    @Test
    fun `deleting the account sends the password and wipes everything local`() =
        runTest(dispatcher) {
            val repository = newRepository()

            val result = repository.deleteAccount("hunter22")

            assertThat(result).isInstanceOf(ApiResult.Ok::class.java)
            assertThat(authApi.accountDeletions).containsExactly("hunter22")

            // Token gone, whole Room cache gone (messages, chats, roster,
            // notes — and with them every per-chat reaction and edit
            // cursor, which is what makes them cursors of the CACHE).
            assertThat(tokenStore.token).isNull()
            assertThat(wiper.wipeCount).isEqualTo(1)

            val state = settings.state.first()
            // Every stored cursor is back to zero. A survivor here means
            // the next account signed in on this phone starts its catch-up
            // from somebody else's position and silently misses history.
            assertThat(state.boardCursor).isEqualTo(0)
            assertThat(state.boardSeenNoteId).isEqualTo(0)
            assertThat(state.myUserId).isNull()
            assertThat(state.familyName).isNull()
            assertThat(state.pushDeviceId).isNull()
            assertThat(state.familyStatus).isEqualTo(FamilyStatus.NONE)
            // ...except the server URL, which is this phone's, not the
            // account's — the sign-in screen needs it.
            assertThat(state.serverUrl).isEqualTo("https://chat.example.com")
        }

    /**
     * The whole reason this does not go through [SessionRepository.logout]:
     * both of those calls would meet a session the server has already
     * deleted, answer 401, and fire the unauthorized broadcast.
     */
    @Test
    fun `deleting the account calls neither logout nor device deregistration`() =
        runTest(dispatcher) {
            val repository = newRepository()

            repository.deleteAccount("hunter22")

            assertThat(authApi.logoutCalls).isEqualTo(0)
            assertThat(authApi.deletedDeviceIds).isEmpty()
        }

    @Test
    fun `a refused deletion changes nothing at all`() = runTest(dispatcher) {
        val repository = newRepository()
        // A wrong password: `invalid_credentials`, 401 — the account is
        // still there, and so is this session.
        authApi.deleteAccountResult =
            ApiResult.HttpError(401, "invalid_credentials", "wrong password")

        val result = repository.deleteAccount("nope")

        assertThat(result).isInstanceOf(ApiResult.HttpError::class.java)
        assertThat(tokenStore.token).isEqualTo("tok")
        assertThat(wiper.wipeCount).isEqualTo(0)
        assertThat(settings.state.first().boardCursor).isEqualTo(88)
    }

    // -- The other devices ------------------------------------------------------

    @Test
    fun `a socket closed with 4401 lands the device back at sign-in`() = runTest(dispatcher) {
        val repository = newRepository()
        val event = async { repository.sessionEvents.first() }
        runCurrent()

        // What ChatSocketManager does with ChatSocket.sessionExpired.
        repository.onSessionExpired()
        advanceUntilIdle()

        assertThat(event.await()).isEqualTo(SessionEvent.Expired)
        assertThat(tokenStore.token).isNull()
        assertThat(wiper.wipeCount).isEqualTo(1)
        assertThat(settings.state.first().serverUrl).isEqualTo("https://chat.example.com")
    }

    /**
     * The reconnect loop can meet the same dead session several times
     * before the session flow stops it, and each of those must not be its
     * own wipe-and-reroute.
     */
    @Test
    fun `repeated socket expiries reroute once`() = runTest(dispatcher) {
        val repository = newRepository()

        repository.onSessionExpired()
        advanceUntilIdle()
        repository.onSessionExpired()
        repository.onSessionExpired()
        advanceUntilIdle()

        assertThat(wiper.wipeCount).isEqualTo(1)
    }
}

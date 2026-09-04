/*
 * FamilyGateViewModelTest.kt
 * Family Connect (Android)
 *
 * The gate on a server that takes no new families (docs/protocol.md,
 * "Starting a family"): it follows what `/me` said, live, and says a
 * refused creation in the app's own words.
 */

package me.nettrash.familyconnect.ui.familygate

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
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.R
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class FamilyGateViewModelTest {

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val familyApi = FakeFamilyApi()
    private val settings = FakeSettingsRepository(SettingsState(serverUrl = "https://chat.example.com"))

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private fun newViewModel(): FamilyGateViewModel {
        val sessionRepository = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        val familyRepository = FamilyRepository(
            familyApi = familyApi,
            authApi = FakeAuthApi(),
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = FakeChatSocket(),
            scope = repoScope,
        )
        return FamilyGateViewModel(
            appContext = RuntimeEnvironment.getApplication(),
            familyRepository = familyRepository,
            settings = settings,
        )
    }

    /**
     * Open until told otherwise, and back to open when told so again: a
     * server that reopens brings Create back on the next `/me`, without
     * a restart.
     */
    @Test
    fun theGateFollowsWhatTheServerSaidAboutNewFamilies() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        assertThat(viewModel.state.value.registrationEnabled).isTrue()

        settings.setFamilyRegistrationEnabled(false)
        runCurrent()
        assertThat(viewModel.state.value.registrationEnabled).isFalse()

        settings.setFamilyRegistrationEnabled(true)
        runCurrent()
        assertThat(viewModel.state.value.registrationEnabled).isTrue()
    }

    /** The grace rides the same way, and 0 — a server that never sweeps — is the default. */
    @Test
    fun theGateFollowsTheGraceAnAccountWithoutAFamilyGets() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        assertThat(viewModel.state.value.familylessAccountTtlDays).isEqualTo(0)

        settings.setFamilylessAccountTtlDays(7)
        runCurrent()
        assertThat(viewModel.state.value.familylessAccountTtlDays).isEqualTo(7)
    }

    /**
     * Reachable only when the door shut after the Create card was drawn;
     * the server's English goes unused and the app says it in its own
     * words, in the user's language.
     */
    @Test
    fun aClosedServersRefusalIsSaidInTheAppsOwnWords() = runTest(dispatcher) {
        familyApi.createResult =
            ApiResult.HttpError(403, "family_registration_disabled", "this server does not take new families")
        val viewModel = newViewModel()
        runCurrent()

        viewModel.onFamilyNameChange("The Smiths")
        viewModel.createFamily()
        runCurrent()

        val expected = RuntimeEnvironment.getApplication().getString(R.string.s_no_new_families_title)
        assertThat(viewModel.state.value.generalError).isEqualTo(expected)
        assertThat(viewModel.state.value.busy).isFalse()
        assertThat(viewModel.state.value.outcome).isNull()
    }

    /**
     * A server from before the switch never sends the key; it has no door
     * to shut, so absence must read as open (docs/protocol.md, `GET /me`).
     */
    @Test
    fun anOlderServerThatNeverSaysIsTakenToBeOpen() {
        val json = Json { ignoreUnknownKeys = true }
        val user = """{"id": 7, "username": "nora", "display_name": "Nora", "created_at": "2026-01-01T00:00:00Z"}"""
        val older = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "role": null, "pending_join_request": null}""",
        )
        assertThat(older.familyRegistrationEnabled).isTrue()

        val closed = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "role": null, "pending_join_request": null, "family_registration_enabled": false}""",
        )
        assertThat(closed.familyRegistrationEnabled).isFalse()
        assertThat(closed.familylessAccountTtlDays).isEqualTo(0)

        val week = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "role": null, "pending_join_request": null, "familyless_account_ttl_days": 7}""",
        )
        assertThat(week.familylessAccountTtlDays).isEqualTo(7)
    }
}

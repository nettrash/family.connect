/*
 * FamilyGateViewModelTest.kt
 * Family Connect (Android)
 *
 * The gate on a server that takes no new families (docs/protocol.md,
 * "Starting a family"): it follows what `/me` said, live, and says a
 * refused creation in the app's own words. The same for a server that
 * charges for new families (#89, "Paid families").
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

    // -- Paid families (docs/protocol.md, "Paid families") -------------------

    /**
     * Create gives way to the sentence on a server that charges, to a caller
     * with no active subscription — live, so a subscription bought elsewhere
     * brings Create back on the next `/me`.
     */
    @Test
    fun theGateFollowsWhetherTheCallerMustPayToStartAFamily() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        assertThat(viewModel.state.value.mustPayToStart).isFalse()

        settings.setSubscriptions(enabled = true, active = false)
        runCurrent()
        assertThat(viewModel.state.value.mustPayToStart).isTrue()

        settings.setSubscriptions(enabled = true, active = true)
        runCurrent()
        assertThat(viewModel.state.value.mustPayToStart).isFalse()

        settings.setSubscriptions(enabled = false, active = false)
        runCurrent()
        assertThat(viewModel.state.value.mustPayToStart).isFalse()
    }

    /**
     * Reachable when a subscription ended after Create was drawn: the
     * refusal is said in the app's own words, and it names no store.
     */
    @Test
    fun aCreateThatNeedsASubscriptionIsSaidInTheAppsOwnWords() = runTest(dispatcher) {
        familyApi.createResult = ApiResult.HttpError(403, "subscription_required", "a new family needs a subscription")
        val viewModel = newViewModel()
        runCurrent()

        viewModel.onFamilyNameChange("The Smiths")
        viewModel.createFamily()
        runCurrent()

        assertThat(viewModel.state.value.generalError).isEqualTo("A new family on this server needs a subscription.")
        assertThat(viewModel.state.value.busy).isFalse()
        assertThat(viewModel.state.value.outcome).isNull()
    }

    @Test
    fun meCarriesWhetherTheServerChargesAndWhatTheCallerHolds() {
        val json = Json { ignoreUnknownKeys = true }
        val user = """{"id": 7, "username": "nora", "display_name": "Nora", "created_at": "2026-01-01T00:00:00Z"}"""
        val older = json.decodeFromString<MeResponse>("""{"user": $user, "family": null}""")
        assertThat(older.subscriptionsEnabled).isFalse()
        assertThat(older.subscription).isNull()
        assertThat(older.paidFamilyLimits).isNull()
        assertThat(older.unpaidDays).isNull()

        val paid = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "subscriptions_enabled": true,
               "subscription_products": ["me.nettrash.FamilyConnect.family.monthly"],
               "app_account_token": "6f1c0000-0000-4000-8000-000000000000",
               "unpaid_days": 14,
               "paid_family_limits": {"members": 25, "storage_bytes": 10737418240, "ai_tokens": 2000000,
                                      "images": 30, "transcript_ms": 7200000},
               "subscription": {"product_id": "me.nettrash.FamilyConnect.family.yearly", "environment": "Sandbox",
                                "expires_at": "2026-11-30T10:00:00Z", "active": true, "renews": false}}""",
        )
        assertThat(paid.subscriptionsEnabled).isTrue()
        assertThat(paid.unpaidDays).isEqualTo(14)
        assertThat(paid.subscription?.active).isTrue()
        assertThat(paid.subscription?.renews).isFalse()
        assertThat(paid.subscription?.environment).isEqualTo("Sandbox")
        assertThat(paid.subscription?.expiresAt).isEqualTo("2026-11-30T10:00:00Z")
        assertThat(paid.paidFamilyLimits?.members).isEqualTo(25)
        assertThat(paid.paidFamilyLimits?.storageBytes).isEqualTo(10_737_418_240L)
        assertThat(paid.paidFamilyLimits?.aiTokens).isEqualTo(2_000_000L)
        assertThat(paid.paidFamilyLimits?.images).isEqualTo(30)
        assertThat(paid.paidFamilyLimits?.transcriptMs).isEqualTo(7_200_000L)

        // No subscription at all, and limits the operator turned off: both
        // absent rather than failing the whole read.
        val none = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "subscriptions_enabled": true, "subscription": null,
               "paid_family_limits": {"members": 25}}""",
        )
        assertThat(none.subscription).isNull()
        assertThat(none.paidFamilyLimits?.storageBytes).isNull()
        // A subscription the server half-described still reads, as not active.
        val partial = json.decodeFromString<MeResponse>(
            """{"user": $user, "family": null, "subscriptions_enabled": true, "subscription": {}}""",
        )
        assertThat(partial.subscription?.active).isFalse()
    }

    // -- Several families: the doors to another family (docs/protocol.md) ----

    @Test
    fun anotherFamilyAtTheCapIsRefusedWithTheCapNamed() = runTest(dispatcher) {
        settings.setFamilies(emptyList(), emptyList(), emptyList(), true, 5, true)
        familyApi.joinResult = ApiResult.HttpError(409, "too_many_families", null)
        val viewModel = newViewModel()
        runCurrent()

        viewModel.onInviteCodeChange("abc123")
        viewModel.join()
        runCurrent()

        assertThat(viewModel.state.value.generalError)
            .isEqualTo("You've reached the most families one account can be in (5).")
        assertThat(viewModel.state.value.outcome).isNull()
    }

    @Test
    fun startingAnotherFamilySaysWhyItCannot() = runTest(dispatcher) {
        val viewModel = newViewModel()
        runCurrent()
        viewModel.onFamilyNameChange("The Browns")

        familyApi.createResult = ApiResult.HttpError(409, "already_own_a_family", null)
        viewModel.createFamily()
        runCurrent()
        assertThat(viewModel.state.value.generalError)
            .isEqualTo("You already own a family, so you can't start another.")

        familyApi.createResult = ApiResult.HttpError(403, "member_families_disabled", null)
        viewModel.createFamily()
        runCurrent()
        assertThat(viewModel.state.value.generalError)
            .isEqualTo("This server doesn't let members start new families.")
    }
}

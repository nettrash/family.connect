/*
 * LookupConsentTest.kt
 * Family Connect (Android)
 *
 * The member's second consent — whether the assistant may send a short
 * query it writes from their words to the lookup providers
 * (docs/protocol.md, "Consenting to the assistant", amended 2026-10-03):
 * read from `GET /me` with null as an older server's answer, written only
 * through its own endpoint and only after the first consent, never inferred
 * from the first, and cleared with it when the first is withdrawn.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AssistantConsentResponse
import me.nettrash.familyconnect.data.net.dto.AssistantLookupConsentResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.userDto
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class LookupConsentTest {

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.MEMBER,
            myUserId = 7,
        ),
    )
    private val json = Json { ignoreUnknownKeys = true }

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun sessionRepository() = SessionRepository(
        authApi = authApi,
        tokenStore = FakeTokenStore("tok"),
        settings = settings,
        wiper = RecordingWiper(),
        unauthorizedEvents = MutableSharedFlow(),
        scope = repoScope,
    )

    private fun familyRepository() = FamilyRepository(
        familyApi = FakeFamilyApi(),
        authApi = authApi,
        memberDao = db.memberDao(),
        settings = settings,
        sessionRepository = sessionRepository(),
        socket = FakeChatSocket(),
        scope = repoScope,
    )

    private val family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open")

    // -- The wire ----------------------------------------------------------------

    @Test
    fun `an older server's me has no lookup consent`() {
        val me = json.decodeFromString<MeResponse>(
            """{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
               "assistant_consent_at": "2026-09-19T19:34:43Z"}""",
        )
        assertThat(me.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
        // The first consent is never stretched to cover the second.
        assertThat(me.assistantLookupConsentAt).isNull()
    }

    @Test
    fun `a current server's me carries the lookup stamp or a real null`() {
        val agreed = json.decodeFromString<MeResponse>(
            """{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
               "assistant_consent_at": "2026-09-19T19:34:43Z",
               "assistant_lookup_consent_at": "2026-10-03T09:00:00Z"}""",
        )
        assertThat(agreed.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
        val notAgreed = json.decodeFromString<MeResponse>(
            """{"user": {"id": 7, "username": "anna", "display_name": "Anna"},
               "assistant_lookup_consent_at": null}""",
        )
        assertThat(notAgreed.assistantLookupConsentAt).isNull()
    }

    @Test
    fun `the endpoint's answer decodes, null included`() {
        assertThat(
            json.decodeFromString<AssistantLookupConsentResponse>("""{"assistant_lookup_consent_at": "2026-10-03T09:00:00Z"}""")
                .assistantLookupConsentAt,
        ).isEqualTo("2026-10-03T09:00:00Z")
        assertThat(
            json.decodeFromString<AssistantLookupConsentResponse>("""{"assistant_lookup_consent_at": null}""")
                .assistantLookupConsentAt,
        ).isNull()
    }

    // -- GET /me ---------------------------------------------------------------------

    @Test
    fun `the resync records the lookup stamp`() = runTest(dispatcher) {
        authApi.meResult = ApiResult.Ok(
            MeResponse(
                user = userDto(7, "anna"),
                family = family,
                role = "member",
                assistantConsentAt = "2026-09-19T19:34:43Z",
                assistantLookupConsentAt = "2026-10-03T09:00:00Z",
            ),
        )

        sessionRepository().refreshMe()

        assertThat(settings.current.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }

    @Test
    fun `a withdrawal on another device reaches this one, null included`() = runTest(dispatcher) {
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
        authApi.meResult = ApiResult.Ok(
            MeResponse(
                user = userDto(7, "anna"),
                family = family,
                role = "member",
                assistantConsentAt = "2026-09-19T19:34:43Z",
                assistantLookupConsentAt = null,
            ),
        )

        sessionRepository().refreshMe()

        assertThat(settings.current.assistantLookupConsentAt).isNull()
        assertThat(settings.current.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
    }

    // -- Agreeing ----------------------------------------------------------------------

    @Test
    fun `agree with lookups records the assistant consent first, then the lookup consent`() = runTest(dispatcher) {
        val outcome = familyRepository().agreeToAssistant(withLookups = true)
        runCurrent()

        assertThat(authApi.consentCalls).containsExactly("assistant:true", "lookups:true").inOrder()
        assertThat(outcome).isEqualTo(FamilyRepository.AgreeOutcome(assistant = true, lookups = true))
        assertThat(settings.current.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
        assertThat(settings.current.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }

    @Test
    fun `agree without lookups never touches the lookup endpoint`() = runTest(dispatcher) {
        val outcome = familyRepository().agreeToAssistant(withLookups = false)
        runCurrent()

        assertThat(authApi.consentCalls).containsExactly("assistant:true")
        assertThat(outcome).isEqualTo(FamilyRepository.AgreeOutcome(assistant = true, lookups = false))
        assertThat(settings.current.assistantLookupConsentAt).isNull()
    }

    @Test
    fun `when the assistant consent fails the lookup consent is not even asked for`() = runTest(dispatcher) {
        authApi.assistantConsentResult = ApiResult.NetworkError(java.io.IOException("offline"))

        val outcome = familyRepository().agreeToAssistant(withLookups = true)
        runCurrent()

        // The server refuses a lookup grant without the first consent, and
        // a grant this device could not record must not be attempted.
        assertThat(authApi.consentCalls).containsExactly("assistant:true")
        assertThat(outcome).isEqualTo(FamilyRepository.AgreeOutcome(assistant = false, lookups = false))
        assertThat(settings.current.assistantLookupConsentAt).isNull()
    }

    @Test
    fun `a refused lookup grant leaves no lookup stamp, but the assistant consent stands`() = runTest(dispatcher) {
        authApi.lookupConsentResult = ApiResult.HttpError(404, "not_found", "no lookup source")

        val outcome = familyRepository().agreeToAssistant(withLookups = true)
        runCurrent()

        assertThat(outcome).isEqualTo(FamilyRepository.AgreeOutcome(assistant = true, lookups = false))
        assertThat(settings.current.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
        assertThat(settings.current.assistantLookupConsentAt).isNull()
    }

    // -- Stopping ----------------------------------------------------------------------

    @Test
    fun `stop lookups withdraws only the lookup consent`() = runTest(dispatcher) {
        settings.setAssistantConsentAt("2026-09-19T19:34:43Z")
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
        authApi.lookupConsentResult = ApiResult.Ok(AssistantLookupConsentResponse(assistantLookupConsentAt = null))

        val saved = familyRepository().setAssistantLookupConsent(false)
        runCurrent()

        assertThat(saved).isTrue()
        assertThat(authApi.consentCalls).containsExactly("lookups:false")
        assertThat(settings.current.assistantLookupConsentAt).isNull()
        assertThat(settings.current.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
    }

    @Test
    fun `a failed stop is reported and changes nothing here`() = runTest(dispatcher) {
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
        authApi.lookupConsentResult = ApiResult.NetworkError(java.io.IOException("offline"))

        val saved = familyRepository().setAssistantLookupConsent(false)
        runCurrent()

        assertThat(saved).isFalse()
        assertThat(settings.current.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }

    @Test
    fun `withdrawing the assistant consent clears the lookup consent with it`() = runTest(dispatcher) {
        settings.setAssistantConsentAt("2026-09-19T19:34:43Z")
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
        authApi.assistantConsentResult = ApiResult.Ok(AssistantConsentResponse(assistantConsentAt = null))

        familyRepository().setAssistantConsent(false)
        runCurrent()

        assertThat(settings.current.assistantConsentAt).isNull()
        assertThat(settings.current.assistantLookupConsentAt).isNull()
        // The server does that itself; nothing was sent for it.
        assertThat(authApi.consentCalls).containsExactly("assistant:false")
    }

    @Test
    fun `granting the assistant consent leaves the lookup consent alone`() = runTest(dispatcher) {
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")

        familyRepository().setAssistantConsent(true)
        runCurrent()

        assertThat(settings.current.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }

    @Test
    fun `a failed withdrawal of the assistant consent keeps both`() = runTest(dispatcher) {
        settings.setAssistantConsentAt("2026-09-19T19:34:43Z")
        settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
        authApi.assistantConsentResult = ApiResult.NetworkError(java.io.IOException("offline"))

        familyRepository().setAssistantConsent(false)
        runCurrent()

        assertThat(settings.current.assistantConsentAt).isEqualTo("2026-09-19T19:34:43Z")
        assertThat(settings.current.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
    }
}

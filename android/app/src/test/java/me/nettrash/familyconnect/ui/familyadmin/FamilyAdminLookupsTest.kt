/*
 * FamilyAdminLookupsTest.kt
 * Family Connect (Android)
 *
 * The owner's lookups switch (`ai_lookups`, docs/protocol.md, "Looking
 * things up") and the capability beside it (`assistant.lookups`): decoded
 * with the compatibility defaults, written with exactly one key, drawn
 * from the server's answer, mirrored into settings, and offered only on a
 * server that has a source to look things up in.
 */

package me.nettrash.familyconnect.ui.familyadmin

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
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AssistantDto
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.net.dto.PatchFamilyRequest
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import me.nettrash.familyconnect.testutil.memberDto
import me.nettrash.familyconnect.ui.chat.AssistantLookups
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class FamilyAdminLookupsTest {

    private companion object {
        const val ME = 7L
        val ALL_THREE = listOf("Brave Search", "Open-Meteo", "Wikipedia")
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val familyApi = FakeFamilyApi()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            myUserId = ME,
        ),
    )

    private val json = Json { ignoreUnknownKeys = true }
    private val houseJson = Json {
        ignoreUnknownKeys = true
        classDiscriminator = "type"
        encodeDefaults = false
    }

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

    private val lookingUp = AssistantDto(
        userId = 1,
        displayName = "Assistant",
        mention = "@ai",
        processor = "Microsoft — Azure OpenAI (Sweden Central)",
        lookups = ALL_THREE,
    )

    private fun mine(assistant: AssistantDto?, aiLookups: Boolean) = ApiResult.Ok(
        FamilyMineResponse(
            family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiLookups = aiLookups),
            members = listOf(memberDto(ME, "anna", role = "owner")),
            assistant = assistant,
        ),
    )

    private fun familyRepository() = FamilyRepository(
        familyApi = familyApi,
        authApi = FakeAuthApi(),
        memberDao = db.memberDao(),
        settings = settings,
        sessionRepository = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        ),
        socket = FakeChatSocket(),
        scope = repoScope,
    )

    private fun viewModel() = FamilyAdminViewModel(
        appContext = RuntimeEnvironment.getApplication(),
        familyRepository = familyRepository(),
        settings = settings,
    )

    // -- The wire ----------------------------------------------------------------

    @Test
    fun `an older server's family and assistant decode as no lookups`() {
        val family = json.decodeFromString<FamilyDto>("""{"id": 3, "name": "The Smiths", "join_policy": "open"}""")
        assertThat(family.aiLookups).isFalse()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai"}""",
        )
        assertThat(assistant.lookups).isNull()
        assertThat(AssistantLookups.showsOwnerSwitch(assistant.lookups)).isFalse()
    }

    @Test
    fun `a looking-up server's fields decode, in the server's order`() {
        val family = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "join_policy": "open", "ai_lookups": true}""",
        )
        assertThat(family.aiLookups).isTrue()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai",
               "lookups": ["SearXNG", "Open-Meteo"], "some_future_key": 1}""",
        )
        assertThat(assistant.lookups).containsExactly("SearXNG", "Open-Meteo").inOrder()
    }

    @Test
    fun `an empty list is read as no source`() {
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai", "lookups": []}""",
        )
        assertThat(AssistantLookups.showsOwnerSwitch(assistant.lookups)).isFalse()
    }

    @Test
    fun `setting the switch sends exactly that one key`() {
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiLookups(true)))
            .isEqualTo("""{"ai_lookups":true}""")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiLookups(false)))
            .isEqualTo("""{"ai_lookups":false}""")
        // …and no other switch's write carries it.
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiTranscripts(false)))
            .doesNotContain("ai_lookups")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiHistory(true)))
            .doesNotContain("ai_lookups")
    }

    // -- The mirror ----------------------------------------------------------------

    @Test
    fun `the family read mirrors the switch and the providers into settings`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = true)

        familyRepository().refreshMine()
        runCurrent()

        assertThat(settings.current.familyAiLookups).isTrue()
        assertThat(settings.current.assistantLookups).containsExactlyElementsIn(ALL_THREE).inOrder()
    }

    @Test
    fun `the switch turned off elsewhere reaches this device, false included`() = runTest(dispatcher) {
        settings.setFamilyAiLookups(true)
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = false)

        familyRepository().refreshMine()
        runCurrent()

        assertThat(settings.current.familyAiLookups).isFalse()
    }

    @Test
    fun `a server that drops its sources takes the providers away`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = true)
        val repository = familyRepository()
        repository.refreshMine()
        runCurrent()

        familyApi.mineResult = mine(assistant = lookingUp.copy(lookups = null), aiLookups = true)
        repository.refreshMine()
        runCurrent()

        assertThat(settings.current.assistantLookups).isEmpty()
    }

    @Test
    fun `a server with no assistant has no providers either`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = true)
        val repository = familyRepository()
        repository.refreshMine()
        runCurrent()

        familyApi.mineResult = mine(assistant = null, aiLookups = false)
        repository.refreshMine()
        runCurrent()

        assertThat(settings.current.assistantLookups).isEmpty()
    }

    // -- The owner's screen ----------------------------------------------------------

    @Test
    fun `the screen reads the switch and who a lookup would reach`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiLookups).isFalse()
        assertThat(viewModel.state.value.assistantLookups).containsExactlyElementsIn(ALL_THREE).inOrder()
        assertThat(AssistantLookups.showsOwnerSwitch(viewModel.state.value.assistantLookups)).isTrue()
    }

    @Test
    fun `on a server with no source the switch is not offered`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp.copy(lookups = null), aiLookups = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(AssistantLookups.showsOwnerSwitch(viewModel.state.value.assistantLookups)).isFalse()
    }

    @Test
    fun `switching it on reaches the wire and the answer is what is drawn and mirrored`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = false)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiLookups = true)),
        )

        viewModel.setAiLookups(true)
        runCurrent()

        assertThat(familyApi.aiLookupsSet).containsExactly(true)
        assertThat(viewModel.state.value.aiLookups).isTrue()
        assertThat(viewModel.state.value.busy).isFalse()
        assertThat(settings.current.familyAiLookups).isTrue()
    }

    @Test
    fun `a refused write leaves the switch where the server says it is`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = lookingUp, aiLookups = false)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.HttpError(403, "not_family_owner", "only the owner may do that")

        viewModel.setAiLookups(true)
        runCurrent()

        assertThat(viewModel.state.value.aiLookups).isFalse()
        assertThat(viewModel.state.value.error).isEqualTo("only the owner may do that")
        assertThat(settings.current.familyAiLookups).isFalse()
    }
}

/*
 * FamilyAdminTranscriptsTest.kt
 * Family Connect (Android)
 *
 * The owner's transcripts switch (`ai_transcripts`, docs/protocol.md,
 * "Transcripts on request") and the two capability fields beside it
 * (`assistant.transcribe`, `assistant.transcribe_max_bytes`): decoded with
 * the compatibility defaults, written with exactly one key, drawn from the
 * server's answer, and mirrored into settings so a bubble can decide
 * "Show text" without a round trip.
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
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class FamilyAdminTranscriptsTest {

    private companion object {
        const val ME = 7L
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

    private val transcribing = AssistantDto(
        userId = 1,
        displayName = "Assistant",
        mention = "@ai",
        processor = "Microsoft — Azure OpenAI (Sweden Central)",
        transcribe = true,
        transcribeMaxBytes = 10_000_000,
    )

    private fun mine(assistant: AssistantDto?, aiTranscripts: Boolean) = ApiResult.Ok(
        FamilyMineResponse(
            family = FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiTranscripts = aiTranscripts),
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
    fun `an older server's family and assistant decode as no transcripts`() {
        val family = json.decodeFromString<FamilyDto>("""{"id": 3, "name": "The Smiths", "join_policy": "open"}""")
        assertThat(family.aiTranscripts).isFalse()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai"}""",
        )
        assertThat(assistant.transcribe).isFalse()
        assertThat(assistant.transcribeMaxBytes).isNull()
    }

    @Test
    fun `a transcribing server's fields decode`() {
        val family = json.decodeFromString<FamilyDto>(
            """{"id": 3, "name": "The Smiths", "join_policy": "open", "ai_transcripts": true}""",
        )
        assertThat(family.aiTranscripts).isTrue()
        val assistant = json.decodeFromString<AssistantDto>(
            """{"user_id": 1, "display_name": "Assistant", "mention": "@ai", "transcribe": true, "transcribe_max_bytes": 26214400}""",
        )
        assertThat(assistant.transcribe).isTrue()
        assertThat(assistant.transcribeMaxBytes).isEqualTo(26_214_400L)
    }

    @Test
    fun `setting the switch sends exactly that one key`() {
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiTranscripts(true)))
            .isEqualTo("""{"ai_transcripts":true}""")
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiTranscripts(false)))
            .isEqualTo("""{"ai_transcripts":false}""")
        // …and no other switch's write carries it.
        assertThat(houseJson.encodeToString(PatchFamilyRequest.aiVision(false)))
            .doesNotContain("ai_transcripts")
    }

    // -- The mirror the bubbles read ----------------------------------------------

    @Test
    fun `the family read mirrors the switch and the capability into settings`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = true)

        familyRepository().refreshMine()
        runCurrent()

        assertThat(settings.current.familyAiTranscripts).isTrue()
        assertThat(settings.current.assistantTranscribe).isTrue()
        assertThat(settings.current.assistantTranscribeMaxBytes).isEqualTo(10_000_000L)
    }

    @Test
    fun `an absent ceiling means the protocol's default`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing.copy(transcribeMaxBytes = null), aiTranscripts = false)

        familyRepository().refreshMine()
        runCurrent()

        assertThat(settings.current.assistantTranscribeMaxBytes).isEqualTo(AssistantDto.DEFAULT_TRANSCRIBE_MAX_BYTES)
    }

    @Test
    fun `the switch turned off elsewhere reaches this device, false included`() = runTest(dispatcher) {
        settings.setFamilyAiTranscripts(true)
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = false)

        familyRepository().refreshMine()
        runCurrent()

        assertThat(settings.current.familyAiTranscripts).isFalse()
    }

    @Test
    fun `a server that stops transcribing takes the capability away`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = true)
        val repository = familyRepository()
        repository.refreshMine()
        runCurrent()

        familyApi.mineResult = mine(assistant = transcribing.copy(transcribe = false, transcribeMaxBytes = null), aiTranscripts = true)
        repository.refreshMine()
        runCurrent()

        assertThat(settings.current.assistantTranscribe).isFalse()
        assertThat(settings.current.assistantTranscribeMaxBytes).isEqualTo(0L)
    }

    // -- The owner's screen ----------------------------------------------------------

    @Test
    fun `the screen reads the switch, the capability and who the sound goes to`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiTranscripts).isFalse()
        assertThat(viewModel.state.value.assistantTranscribe).isTrue()
        assertThat(viewModel.state.value.assistantProcessor).isEqualTo("Microsoft — Azure OpenAI (Sweden Central)")
    }

    @Test
    fun `on a server that cannot transcribe the screen knows to say so`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing.copy(transcribe = false), aiTranscripts = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.assistantTranscribe).isFalse()
    }

    @Test
    fun `switching it on reaches the wire and the answer is what is drawn and mirrored`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = false)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiTranscripts = true)),
        )

        viewModel.setAiTranscripts(true)
        runCurrent()

        assertThat(familyApi.aiTranscriptsSet).containsExactly(true)
        assertThat(viewModel.state.value.aiTranscripts).isTrue()
        assertThat(viewModel.state.value.busy).isFalse()
        assertThat(settings.current.familyAiTranscripts).isTrue()
    }

    @Test
    fun `a refused write leaves the switch where the server says it is`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = transcribing, aiTranscripts = false)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.HttpError(403, "not_owner", "only the owner may do that")

        viewModel.setAiTranscripts(true)
        runCurrent()

        assertThat(viewModel.state.value.aiTranscripts).isFalse()
        assertThat(viewModel.state.value.error).isEqualTo("only the owner may do that")
        assertThat(settings.current.familyAiTranscripts).isFalse()
    }
}

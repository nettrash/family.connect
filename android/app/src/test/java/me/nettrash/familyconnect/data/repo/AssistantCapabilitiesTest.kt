/*
 * AssistantCapabilitiesTest.kt
 * Family Connect (Android)
 *
 * What this client remembers about pictures, and where it gets it from
 * (docs/protocol.md, "Pictures").
 *
 * Three separate answers ride in on `GET /families/mine`, and this app
 * has to keep them apart because they are opened by three different
 * people: `assistant.vision` and `assistant.images` are the OPERATOR's
 * (which deployments exist at all), `family.ai_vision` is the OWNER's,
 * and the third lock — attaching the photograph to the question — belongs
 * to the member and is deliberately not a setting anywhere. Conflating
 * the first two would put a picture affordance on a server that cannot
 * see, which is precisely the "affordance that silently does nothing"
 * the whole `assistant` object exists to prevent.
 *
 * The compatibility case is here for the same reason it is in
 * FamilySettingsDtoTest: a server that predates all of this must decode,
 * and must leave every one of these OFF.
 */

package me.nettrash.familyconnect.data.repo

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AssistantDto
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
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

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class AssistantCapabilitiesTest {

    private companion object {
        const val ME = 7L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
    private val authApi = FakeAuthApi()
    private val familyApi = FakeFamilyApi()
    private val socket = FakeChatSocket()
    private val settings = FakeSettingsRepository(
        SettingsState(
            serverUrl = "https://chat.example.com",
            familyStatus = FamilyStatus.OWNER,
            myUserId = ME,
        ),
    )

    @Before
    fun setUp() {
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
    }

    private fun TestScope.repository(): FamilyRepository {
        val sessionRepository = SessionRepository(
            authApi = authApi,
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = repoScope,
        )
        val repository = FamilyRepository(
            familyApi = familyApi,
            authApi = authApi,
            memberDao = db.memberDao(),
            settings = settings,
            sessionRepository = sessionRepository,
            socket = socket,
            scope = repoScope,
        )
        runCurrent()
        return repository
    }

    private fun mine(
        assistant: AssistantDto?,
        aiVision: Boolean,
        aiHistoryPhotos: Boolean = false,
        aiHistory: Boolean = true,
    ) = ApiResult.Ok(
        FamilyMineResponse(
            family = FamilyDto(
                id = 3,
                name = "The Smiths",
                joinPolicy = "open",
                aiHistory = aiHistory,
                aiVision = aiVision,
                aiHistoryPhotos = aiHistoryPhotos,
            ),
            members = listOf(memberDto(ME, "anna", role = "owner")),
            assistant = assistant,
        ),
    )

    private val seeing = AssistantDto(
        userId = 1,
        displayName = "Assistant",
        mention = "@ai",
        draw = "/draw",
        vision = true,
        images = true,
    )

    @Test
    fun `a server with both deployments and a family that allows it records all three`() =
        runTest(dispatcher) {
            familyApi.mineResult = mine(assistant = seeing, aiVision = true)
            repository().refreshMine()

            val state = settings.state.first()
            assertThat(state.assistantVision).isTrue()
            assertThat(state.assistantImages).isTrue()
            assertThat(state.familyAiVision).isTrue()
        }

    /**
     * The two halves are INDEPENDENT, and this is the pairing that goes
     * wrong if they are conflated: the server can see, and this family
     * has not said it may. Nothing may be offered here.
     */
    @Test
    fun `a server that can see does not mean this family allows it`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = false)
        repository().refreshMine()

        val state = settings.state.first()
        assertThat(state.assistantVision).isTrue()
        assertThat(state.familyAiVision).isFalse()
    }

    /** And the other way round: an owner may switch on what a server cannot do. */
    @Test
    fun `a family may allow what this server cannot do`() = runTest(dispatcher) {
        familyApi.mineResult = mine(
            assistant = seeing.copy(vision = false, images = false),
            aiVision = true,
        )
        repository().refreshMine()

        val state = settings.state.first()
        assertThat(state.assistantVision).isFalse()
        assertThat(state.assistantImages).isFalse()
        assertThat(state.familyAiVision).isTrue()
    }

    /**
     * A server that predates all of this: no `assistant` object at all,
     * no `ai_vision` key. Every one of these must read as OFF — the
     * default has to be the safe one in both directions, because this is
     * also what a client sees the first time it talks to a server it
     * knows nothing about.
     */
    @Test
    fun `a server that predates pictures leaves everything off`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = null, aiVision = false)
        repository().refreshMine()

        val state = settings.state.first()
        assertThat(state.assistantUserId).isNull()
        assertThat(state.assistantVision).isFalse()
        assertThat(state.assistantImages).isFalse()
        assertThat(state.familyAiVision).isFalse()
    }

    /**
     * An operator turning a deployment off must reach a device that once
     * saw it on. The capabilities are cleared WITH the assistant, so a
     * stale `true` cannot outlive the assistant it described.
     */
    @Test
    fun `an assistant that goes away takes its capabilities with it`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        repository.refreshMine()
        assertThat(settings.state.first().assistantVision).isTrue()

        familyApi.mineResult = mine(assistant = null, aiVision = true)
        repository.refreshMine()

        val state = settings.state.first()
        assertThat(state.assistantVision).isFalse()
        assertThat(state.assistantImages).isFalse()
    }

    /**
     * The owner's own switch reaches the wire, and its answer is mirrored
     * onto the settings the composer reads. Without the mirror the
     * owner's OWN device would go on hiding the affordance it just
     * enabled until the next `GET /families/mine`.
     */
    @Test
    fun `the owner's switch reaches the wire and comes back to the composer`() =
        runTest(dispatcher) {
            val repository = repository()
            familyApi.createResult = ApiResult.Ok(
                FamilyResponse(
                    FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = true),
                ),
            )

            repository.setAiVision(true)

            assertThat(familyApi.aiVisionSet).containsExactly(true)
            assertThat(settings.state.first().familyAiVision).isTrue()
        }

    /**
     * And turning it OFF is a value, not an absence: a guard that skipped
     * the false case would leave every device offering a picture the
     * server would never show.
     */
    @Test
    fun `switching it off is written too`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        repository.refreshMine()
        assertThat(settings.state.first().familyAiVision).isTrue()

        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = false),
            ),
        )
        repository.setAiVision(false)

        assertThat(familyApi.aiVisionSet).containsExactly(false)
        assertThat(settings.state.first().familyAiVision).isFalse()
    }

    // -- The third switch, and the history switch it needs ----------------------

    /**
     * All three family switches are mirrored from one read, because the
     * family composer's strip has to know all three to say what a mention
     * is about to carry (docs/protocol.md, "Recent photos from the family
     * chat"): the transcript, a pointed-at photo, and the transcript's
     * recent photos under both.
     */
    @Test
    fun `refreshMine mirrors all three family switches`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistoryPhotos = true, aiHistory = false)
        repository.refreshMine()

        val state = settings.state.first()
        assertThat(state.familyAiVision).isTrue()
        assertThat(state.familyAiHistoryPhotos).isTrue()
        assertThat(state.familyAiHistory).isFalse()

        // And back off again, `false` written rather than skipped.
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistoryPhotos = false, aiHistory = true)
        repository.refreshMine()
        assertThat(settings.state.first().familyAiHistoryPhotos).isFalse()
        assertThat(settings.state.first().familyAiHistory).isTrue()
    }

    /** A server that predates the third switch leaves it off. */
    @Test
    fun `a server that predates the third switch leaves it off`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        repository.refreshMine()
        assertThat(settings.state.first().familyAiHistoryPhotos).isFalse()
        assertThat(settings.state.first().familyAiHistory).isTrue()
    }

    @Test
    fun `the owner's third switch reaches the wire and comes back to the composer`() =
        runTest(dispatcher) {
            val repository = repository()
            familyApi.createResult = ApiResult.Ok(
                FamilyResponse(
                    FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = true, aiHistoryPhotos = true),
                ),
            )

            repository.setAiHistoryPhotos(true)

            assertThat(familyApi.aiHistoryPhotosSet).containsExactly(true)
            assertThat(settings.state.first().familyAiHistoryPhotos).isTrue()
        }

    /**
     * Turning `ai_vision` off turns the third switch off in the same
     * write on the server, whether or not this device asked — so the
     * answer to `setAiVision` is mirrored for BOTH, or the composer would
     * go on announcing recent photos the server will never send.
     */
    @Test
    fun `turning pictures off takes the third switch with it`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistoryPhotos = true)
        repository.refreshMine()
        assertThat(settings.state.first().familyAiHistoryPhotos).isTrue()

        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = false, aiHistoryPhotos = false),
            ),
        )
        repository.setAiVision(false)

        assertThat(familyApi.aiHistoryPhotosSet).isEmpty()
        assertThat(settings.state.first().familyAiVision).isFalse()
        assertThat(settings.state.first().familyAiHistoryPhotos).isFalse()
    }

    /** The history switch is mirrored too, so the strip can tell the third one is inert. */
    @Test
    fun `the history switch is mirrored from the owner's PATCH`() = runTest(dispatcher) {
        val repository = repository()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiHistory = false),
            ),
        )
        repository.setAiHistory(false)
        assertThat(familyApi.aiHistorySet).containsExactly(false)
        assertThat(settings.state.first().familyAiHistory).isFalse()
    }
}

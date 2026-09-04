/*
 * FamilyAdminPicturesTest.kt
 * Family Connect (Android)
 *
 * The owner's half of the picture rule: the `ai_vision` switch
 * (docs/protocol.md, "Pictures").
 *
 * Two things are pinned here and they matter for different reasons. The
 * VALUE, because it defaults the opposite way to its neighbour
 * `ai_history` and a state class that quietly seeded it `true` would give
 * every family a consent nobody gave. And the VISIBILITY, because on a
 * server with no vision deployment the switch must be ABSENT rather than
 * disabled: a greyed control would tell an owner their server could do
 * this and something was stopping them, which is not what a missing
 * deployment means.
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
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.AssistantDto
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
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
class FamilyAdminPicturesTest {

    private companion object {
        const val ME = 7L
    }

    private val dispatcher = StandardTestDispatcher()
    private val repoScope = CoroutineScope(dispatcher + SupervisorJob())
    private lateinit var db: AppDatabase
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
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        Dispatchers.resetMain()
        db.close()
    }

    private val seeing = AssistantDto(
        userId = 1,
        displayName = "Assistant",
        mention = "@ai",
        draw = "/draw",
        vision = true,
        images = true,
    )

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

    private fun viewModel(): FamilyAdminViewModel {
        val familyRepository = FamilyRepository(
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
            socket = socket,
            scope = repoScope,
        )
        return FamilyAdminViewModel(
            appContext = RuntimeEnvironment.getApplication(),
            familyRepository = familyRepository,
            settings = settings,
        )
    }

    /**
     * The default is FALSE, and it is the only switch on this screen that
     * defaults that way. `ai_history` widened what a model was already
     * being told; a photograph is a different thing, and the family who
     * never open this screen are exactly the family whose pictures should
     * stay where they are.
     */
    @Test
    fun `a family that never chose reads as off`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiVision).isFalse()
        // …while its neighbour defaults the other way, from the same read.
        assertThat(viewModel.state.value.aiHistory).isTrue()
    }

    @Test
    fun `a family that turned it on reads as on`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiVision).isTrue()
    }

    /**
     * No deployment, no switch. The screen reads [FamilyAdminViewModel.UiState.assistantVision]
     * and draws nothing at all when it is false — see the file header for
     * why absent rather than disabled.
     */
    @Test
    fun `a server with no vision deployment offers no switch`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing.copy(vision = false), aiVision = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.assistantVision).isFalse()
    }

    /** And a server with no assistant at all is the same answer. */
    @Test
    fun `a server with no assistant offers no switch`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = null, aiVision = false)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.assistantVision).isFalse()
    }

    @Test
    fun `switching it on reaches the wire and the answer is what is drawn`() =
        runTest(dispatcher) {
            familyApi.mineResult = mine(assistant = seeing, aiVision = false)
            val viewModel = viewModel()
            runCurrent()
            familyApi.createResult = ApiResult.Ok(
                FamilyResponse(
                    FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = true),
                ),
            )

            viewModel.setAiVision(true)
            runCurrent()

            assertThat(familyApi.aiVisionSet).containsExactly(true)
            // The SERVER's answer, not the requested value: an owner must
            // never be shown a switch that moved when the family did not.
            assertThat(viewModel.state.value.aiVision).isTrue()
            assertThat(viewModel.state.value.busy).isFalse()
        }

    /**
     * A refused change must leave the switch where it was. Otherwise the
     * owner reads consent off a control that lost the argument.
     */
    @Test
    fun `a refused change leaves the switch alone and says why`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = false)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.HttpError(403, "not_family_owner", "not the owner")

        viewModel.setAiVision(true)
        runCurrent()

        assertThat(viewModel.state.value.aiVision).isFalse()
        assertThat(viewModel.state.value.error).isEqualTo("not the owner")
        assertThat(viewModel.state.value.busy).isFalse()
    }

    // -- The THIRD switch: recent photos (docs/protocol.md, "Recent photos from the family chat")

    /**
     * FALSE by default, one notch further than `ai_vision`: a family that
     * never chose has not consented to photographs nobody pointed at
     * leaving, and a server that predates the field never sends one.
     */
    @Test
    fun `the third switch reads as off for a family that never chose`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiHistoryPhotos).isFalse()
        assertThat(viewModel.state.value.aiVision).isTrue()
    }

    @Test
    fun `the third switch reads as on when the owner turned it on`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistoryPhotos = true)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiHistoryPhotos).isTrue()
    }

    /**
     * Offered with both locks under it open; otherwise DISABLED with the
     * reason — unlike `ai_vision`'s switch, which is hidden on a server
     * that cannot see. The deployment is asked first, so an owner on such
     * a server is told that rather than "turn pictures on first", which
     * they could do to no effect. `ai_history` is not a lock: off, the
     * switch is inert and still offered, and the screen says so beside it.
     */
    @Test
    fun `the third switch is offered or withheld with its reason`() = runTest(dispatcher) {
        val rule = FamilyAdminViewModel.HistoryPhotosSwitch
        assertThat(rule.of(serverCanSee = true, familyAllowsPhotos = true))
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.OFFERED)
        assertThat(rule.of(serverCanSee = true, familyAllowsPhotos = false))
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF)
        assertThat(rule.of(serverCanSee = false, familyAllowsPhotos = true))
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT)
        assertThat(rule.of(serverCanSee = false, familyAllowsPhotos = false))
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT)
        assertThat(FamilyAdminViewModel.HistoryPhotosSwitch.OFFERED.isEnabled).isTrue()
        assertThat(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF.isEnabled).isFalse()
        assertThat(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT.isEnabled).isFalse()

        // …and the state derives it from what the server said, in every
        // combination the screen can meet.
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        val offered = viewModel()
        runCurrent()
        assertThat(offered.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.OFFERED)

        familyApi.mineResult = mine(assistant = seeing, aiVision = false)
        val visionOff = viewModel()
        runCurrent()
        assertThat(visionOff.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF)

        familyApi.mineResult = mine(assistant = seeing.copy(vision = false), aiVision = false)
        val noDeployment = viewModel()
        runCurrent()
        assertThat(noDeployment.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT)

        familyApi.mineResult = mine(assistant = null, aiVision = false)
        val noAssistant = viewModel()
        runCurrent()
        assertThat(noAssistant.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_NO_VISION_DEPLOYMENT)

        // Inert, not withheld, without a transcript.
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistory = false)
        val noHistory = viewModel()
        runCurrent()
        assertThat(noHistory.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.OFFERED)
        assertThat(noHistory.state.value.aiHistory).isFalse()
    }

    @Test
    fun `switching the third switch on reaches the wire as its own key`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(
                    id = 3, name = "The Smiths", joinPolicy = "open",
                    aiVision = true, aiHistoryPhotos = true,
                ),
            ),
        )

        viewModel.setAiHistoryPhotos(true)
        runCurrent()

        assertThat(familyApi.aiHistoryPhotosSet).containsExactly(true)
        assertThat(familyApi.aiVisionSet).isEmpty()
        assertThat(viewModel.state.value.aiHistoryPhotos).isTrue()
        assertThat(viewModel.state.value.busy).isFalse()
        // Mirrored for the composer, like `ai_vision`.
        assertThat(settings.current.familyAiHistoryPhotos).isTrue()
    }

    /**
     * Turning `ai_vision` OFF turns the third switch off in the same
     * write on the server, asked or not — and the screen draws BOTH from
     * the answer rather than leaving the third one on underneath a lock
     * that just shut.
     */
    @Test
    fun `turning pictures off takes the third switch with it`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true, aiHistoryPhotos = true)
        val viewModel = viewModel()
        runCurrent()
        assertThat(viewModel.state.value.aiHistoryPhotos).isTrue()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(
                    id = 3, name = "The Smiths", joinPolicy = "open",
                    aiVision = false, aiHistoryPhotos = false,
                ),
            ),
        )

        viewModel.setAiVision(false)
        runCurrent()

        assertThat(familyApi.aiVisionSet).containsExactly(false)
        assertThat(familyApi.aiHistoryPhotosSet).isEmpty()
        assertThat(viewModel.state.value.aiVision).isFalse()
        assertThat(viewModel.state.value.aiHistoryPhotos).isFalse()
        assertThat(viewModel.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF)
        assertThat(settings.current.familyAiHistoryPhotos).isFalse()
    }

    /** A refusal of the third switch leaves it where it was, and says why. */
    @Test
    fun `a refused third switch stays where it was`() = runTest(dispatcher) {
        familyApi.mineResult = mine(assistant = seeing, aiVision = true)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.HttpError(400, "validation", "ai_history_photos needs ai_vision")

        viewModel.setAiHistoryPhotos(true)
        runCurrent()

        assertThat(viewModel.state.value.aiHistoryPhotos).isFalse()
        assertThat(viewModel.state.value.error).isEqualTo("ai_history_photos needs ai_vision")
        assertThat(viewModel.state.value.busy).isFalse()
    }
}

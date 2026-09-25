/*
 * FamilyAdminFacesTest.kt
 * Family Connect (Android)
 *
 * The owner's half of the fifth switch: `ai_faces` (docs/protocol.md,
 * "Profile pictures of members").
 *
 * Three things are pinned. The DEFAULT, because a face is the most
 * identifying thing a photograph can carry and a family that never opened
 * this screen has consented to nothing. The DEPENDENCY, because this switch
 * is drawn under exactly the third switch's rule — withheld with the reason
 * when the server cannot see or `ai_vision` is off — and the server turns it
 * off whenever `ai_vision` goes off, so the screen must draw what the answer
 * says rather than what it last knew. And the WIRE, that turning it on
 * reaches the server and the answer is what is drawn.
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
import me.nettrash.familyconnect.data.repo.SessionRepository
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
class FamilyAdminFacesTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var settings: FakeSettingsRepository
    private lateinit var familyApi: FakeFamilyApi
    private lateinit var socket: FakeChatSocket
    private lateinit var repoScope: CoroutineScope

    private companion object {
        const val ME = 7L
    }

    /** An assistant that can see, so the switch is offered rather than withheld. */
    private val seeing = AssistantDto(
        userId = 1, displayName = "Assistant", mention = "@ai", draw = "/draw",
        vision = true, images = false,
    )

    @Before
    fun setUp() {
        Dispatchers.setMain(dispatcher)
        db = createTestDb(dispatcher)
        settings = FakeSettingsRepository()
        familyApi = FakeFamilyApi()
        socket = FakeChatSocket()
        repoScope = CoroutineScope(SupervisorJob() + dispatcher)
    }

    @After
    fun tearDown() {
        repoScope.cancel()
        db.close()
        Dispatchers.resetMain()
    }

    private fun mine(aiVision: Boolean = false, aiFaces: Boolean = false) = ApiResult.Ok(
        FamilyMineResponse(
            family = FamilyDto(
                id = 3, name = "The Smiths", joinPolicy = "open",
                aiVision = aiVision, aiFaces = aiFaces,
            ),
            members = listOf(memberDto(ME, "anna", role = "owner")),
            assistant = seeing,
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

    @Test
    fun `a family that never chose sends no face`() = runTest(dispatcher) {
        familyApi.mineResult = mine()
        val viewModel = viewModel()
        runCurrent()
        assertThat(viewModel.state.value.aiFaces).isFalse()
    }

    @Test
    fun `switching it on reaches the wire and the answer is what is drawn`() = runTest(dispatcher) {
        familyApi.mineResult = mine(aiVision = true)
        val viewModel = viewModel()
        runCurrent()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = true, aiFaces = true)),
        )

        viewModel.setAiFaces(true)
        runCurrent()

        assertThat(familyApi.aiFacesSet).containsExactly(true)
        assertThat(viewModel.state.value.aiFaces).isTrue()
    }

    /**
     * The dependency, from the answer: `ai_vision` going off takes the faces
     * down in the same write on the server, asked or not, and the screen must
     * draw that — a switch left ON here while the server had it OFF would tell
     * an owner their family's faces were still travelling.
     */
    @Test
    fun `turning pictures off carries the faces switch down with it`() = runTest(dispatcher) {
        familyApi.mineResult = mine(aiVision = true, aiFaces = true)
        val viewModel = viewModel()
        runCurrent()
        assertThat(viewModel.state.value.aiFaces).isTrue()
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(FamilyDto(id = 3, name = "The Smiths", joinPolicy = "open", aiVision = false, aiFaces = false)),
        )

        viewModel.setAiVision(false)
        runCurrent()

        assertThat(viewModel.state.value.aiVision).isFalse()
        assertThat(viewModel.state.value.aiFaces).isFalse()
        // …and the switch is now withheld, with the reason a person can act on.
        assertThat(viewModel.state.value.historyPhotosSwitch)
            .isEqualTo(FamilyAdminViewModel.HistoryPhotosSwitch.WITHHELD_VISION_OFF)
    }
}

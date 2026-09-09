/*
 * FamilyAdminGreetingTest.kt
 * Family Connect (Android)
 *
 * The family's half of the daily greeting: the `ai_greeting` switch
 * (docs/protocol.md, "The daily greeting").
 *
 * Three things are pinned, and they are the three that would be silently
 * wrong. The DEFAULT, because a family that never opens this screen must
 * never be greeted — a greeting cannot be blocked, muted or deleted, so
 * off is the only defensible starting state. The INDEPENDENCE, because
 * this is the one switch on the screen bound to none of the others, and a
 * state class that let `ai_vision` going off take it down with
 * `ai_history_photos` would quietly cancel a thing the owner chose. And
 * the OPERATOR'S HALF, because `greetings_enabled` is what tells an owner
 * whose mornings are quiet whether the server posts at all — the switch is
 * disabled with the reason rather than hidden, so the answer is theirs to
 * act on.
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
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.FamilyMineResponse
import me.nettrash.familyconnect.data.net.dto.FamilyResponse
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeChatSocket
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
class FamilyAdminGreetingTest {

    private val dispatcher = StandardTestDispatcher()
    private lateinit var db: AppDatabase
    private lateinit var settings: FakeSettingsRepository
    private lateinit var familyApi: FakeFamilyApi
    private lateinit var socket: FakeChatSocket
    private lateinit var repoScope: CoroutineScope

    private companion object {
        const val ME = 7L
    }

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

    private fun mine(
        aiGreeting: Boolean = false,
        aiVision: Boolean = false,
        aiHistoryPhotos: Boolean = false,
    ) = ApiResult.Ok(
        FamilyMineResponse(
            family = FamilyDto(
                id = 3,
                name = "The Smiths",
                joinPolicy = "open",
                aiVision = aiVision,
                aiHistoryPhotos = aiHistoryPhotos,
                aiGreeting = aiGreeting,
            ),
            members = listOf(memberDto(ME, "anna", role = "owner")),
            assistant = null,
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
     * Off unless somebody chose it. A family cannot block the assistant,
     * mute it, or delete its messages, so a greeting they did not ask for
     * would be one they could not stop.
     */
    @Test
    fun `a family that never chose is not greeted`() = runTest(dispatcher) {
        familyApi.mineResult = mine()
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiGreeting).isFalse()
    }

    @Test
    fun `a family that turned it on reads as on`() = runTest(dispatcher) {
        familyApi.mineResult = mine(aiGreeting = true)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.aiGreeting).isTrue()
    }

    @Test
    fun `switching it on reaches the wire and the answer is what is drawn`() =
        runTest(dispatcher) {
            settings.setGreetingsEnabled(true)
            familyApi.mineResult = mine()
            val viewModel = viewModel()
            runCurrent()
            familyApi.createResult = ApiResult.Ok(
                FamilyResponse(
                    FamilyDto(
                        id = 3,
                        name = "The Smiths",
                        joinPolicy = "open",
                        aiGreeting = true,
                    ),
                ),
            )

            viewModel.setAiGreeting(true)
            runCurrent()

            assertThat(familyApi.aiGreetingSet).containsExactly(true)
            // The SERVER's answer, not the requested value.
            assertThat(viewModel.state.value.aiGreeting).isTrue()
            assertThat(viewModel.state.value.busy).isFalse()
        }

    /**
     * The operator's half. It is read from settings rather than from the
     * family, because it is a fact about the SERVER — and a false here must
     * disable the switch rather than hide it, so an owner whose mornings are
     * quiet learns why instead of wondering whether the setting saved.
     */
    @Test
    fun `a server that posts no greetings reports so`() = runTest(dispatcher) {
        familyApi.mineResult = mine(aiGreeting = true)
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.greetingsEnabled).isFalse()
        // The family's own answer still reads back truthfully: the two halves
        // are independent, and the switch shows what the family chose.
        assertThat(viewModel.state.value.aiGreeting).isTrue()
    }

    @Test
    fun `a server that posts greetings reports so`() = runTest(dispatcher) {
        settings.setGreetingsEnabled(true)
        familyApi.mineResult = mine()
        val viewModel = viewModel()
        runCurrent()

        assertThat(viewModel.state.value.greetingsEnabled).isTrue()
    }

    /**
     * The independence that separates this switch from its neighbour.
     * `ai_vision` going off clears `ai_history_photos` in the same write —
     * and must not touch this one, which is about whether the assistant
     * speaks at all rather than about what it may be shown.
     */
    @Test
    fun `turning the picture switch off leaves the greeting alone`() = runTest(dispatcher) {
        settings.setGreetingsEnabled(true)
        familyApi.mineResult = mine(aiGreeting = true, aiVision = true, aiHistoryPhotos = true)
        val viewModel = viewModel()
        runCurrent()
        // What the server answers when `ai_vision` goes off: its dependent
        // goes with it, and the greeting does not.
        familyApi.createResult = ApiResult.Ok(
            FamilyResponse(
                FamilyDto(
                    id = 3,
                    name = "The Smiths",
                    joinPolicy = "open",
                    aiVision = false,
                    aiHistoryPhotos = false,
                    aiGreeting = true,
                ),
            ),
        )

        viewModel.setAiVision(false)
        runCurrent()

        assertThat(viewModel.state.value.aiHistoryPhotos).isFalse()
        assertThat(viewModel.state.value.aiGreeting).isTrue()
    }
}

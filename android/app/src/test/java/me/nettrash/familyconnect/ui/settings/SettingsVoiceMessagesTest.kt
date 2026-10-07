/*
 * SettingsVoiceMessagesTest.kt
 * Family Connect (Android)
 *
 * S9 (#79, docs/audio-video-messages-2026-10-04.md) was a "Voice messages"
 * section with one switch, "Review before sending" — the hold's Undo
 * window's "turn off". The hold went on 2026-10-06 and the section with it.
 * Rendered for real over the settings screen's own ViewModel and fakes.
 */

package me.nettrash.familyconnect.ui.settings

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableSharedFlow
import me.nettrash.familyconnect.data.db.AppDatabase
import me.nettrash.familyconnect.data.repo.AvatarSource
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.testutil.FakeAuthApi
import me.nettrash.familyconnect.testutil.FakeAvatarApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.testutil.FakeFamilyApi
import me.nettrash.familyconnect.testutil.FakeSettingsRepository
import me.nettrash.familyconnect.testutil.FakeTokenStore
import me.nettrash.familyconnect.testutil.RecordingWiper
import me.nettrash.familyconnect.testutil.createTestDb
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment

@RunWith(RobolectricTestRunner::class)
class SettingsVoiceMessagesTest {

    @get:Rule
    val compose = createComposeRule()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    private lateinit var db: AppDatabase
    private val settings = FakeSettingsRepository(
        SettingsState(serverUrl = "https://chat.example.com", familyStatus = FamilyStatus.MEMBER, myUserId = 7),
    )

    @Before
    fun setUp() {
        db = createTestDb(Dispatchers.IO)
    }

    @After
    fun tearDown() {
        scope.cancel()
        db.close()
    }

    private fun viewModel(): SettingsViewModel {
        val session = SessionRepository(
            authApi = FakeAuthApi(),
            tokenStore = FakeTokenStore("tok"),
            settings = settings,
            wiper = RecordingWiper(),
            unauthorizedEvents = MutableSharedFlow(),
            scope = scope,
        )
        return SettingsViewModel(
            appContext = RuntimeEnvironment.getApplication(),
            sessionRepository = session,
            authApi = FakeAuthApi(),
            familyRepository = FamilyRepository(
                familyApi = FakeFamilyApi(),
                authApi = FakeAuthApi(),
                memberDao = db.memberDao(),
                settings = settings,
                sessionRepository = session,
                socket = FakeChatSocket(),
                scope = scope,
            ),
            settings = settings,
            avatarApi = FakeAvatarApi(),
            avatarSource = AvatarSource { null },
        )
    }

    /**
     * S9 is gone (#79, revised 2026-10-06): with the hold removed nothing a
     * release could send is left, so "Review before sending" has nothing to
     * turn off — the "Voice messages" section and its switch are not drawn.
     */
    @Test
    fun thereIsNoVoiceMessagesSectionAndNoReviewBeforeSending() {
        val viewModel = viewModel()
        compose.setContent {
            SettingsScreen(
                onBack = {},
                onManageFamily = {},
                onOpenStatistics = {},
                onLoggedOut = {},
                viewModel = viewModel,
            )
        }
        // The screen is up: its neighbours in the list are there.
        compose.onNodeWithText("Change password").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Voice messages").assertDoesNotExist()
        compose.onNodeWithText("Review before sending").assertDoesNotExist()
        compose.onNodeWithText("When you hold the microphone", substring = true).assertDoesNotExist()
    }
}

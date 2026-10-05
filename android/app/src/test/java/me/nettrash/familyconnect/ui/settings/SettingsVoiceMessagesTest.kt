/*
 * SettingsVoiceMessagesTest.kt
 * Family Connect (Android)
 *
 * S9 (#79, docs/audio-video-messages-2026-10-04.md): a "Voice messages"
 * section with one switch, "Review before sending" — off by default, the
 * device's — and the sentence that says what it does. Rendered for real
 * over the settings screen's own ViewModel and fakes: the switch reads the
 * store and writes it.
 */

package me.nettrash.familyconnect.ui.settings

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import com.google.common.truth.Truth.assertThat
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

    @Test
    fun theVoiceMessagesSectionSwitchesReviewBeforeSending() {
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

        compose.onNodeWithText("Voice messages").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText(
            "When you hold the microphone to talk, letting go keeps the message for you to check instead of sending it.",
        ).performScrollTo().assertIsDisplayed()
        // The row is one TalkBack stop, the switch merged into it; the switch
        // itself is read in the unmerged tree.
        val row = compose.onNodeWithText("Review before sending").performScrollTo()
        val toggle = compose.onNode(isToggleable() and hasAnySiblingText("Review before sending"), useUnmergedTree = true)
        toggle.assertIsOff()

        row.performClick()
        compose.waitForIdle()
        assertThat(settings.current.reviewBeforeSending).isTrue()
        toggle.assertIsOn()

        row.performClick()
        compose.waitForIdle()
        assertThat(settings.current.reviewBeforeSending).isFalse()

        // A finger on the switch itself flips it too — the switch takes that
        // tap, not the row.
        toggle.performClick()
        compose.waitForIdle()
        assertThat(settings.current.reviewBeforeSending).isTrue()
        toggle.assertIsOn()
    }

    private fun hasAnySiblingText(text: String) =
        androidx.compose.ui.test.hasAnySibling(hasText(text)) or
            androidx.compose.ui.test.hasParent(androidx.compose.ui.test.hasAnyDescendant(hasText(text)))
}

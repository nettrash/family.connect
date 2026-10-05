/*
 * VoiceSettingsPersistenceTest.kt
 * Family Connect (Android)
 *
 * The voice messages' per-DEVICE choices (#79, docs/audio-video-messages-
 * 2026-10-04.md): "Review before sending" (S9) — "off by default, per
 * device, never on the wire" — whether this device's first held release has
 * been taught (S2.3, S7.3), and whether the coach mark has been shown (S7.2,
 * "once per device"). Against the REAL DataStore: they survive the app
 * being closed and, being about the device rather than the account, a
 * sign-out too.
 */

package me.nettrash.familyconnect.data.settings

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import com.google.common.truth.Truth.assertThat
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class VoiceSettingsPersistenceTest {

    @get:Rule
    val folder = TemporaryFolder()

    /** One "process": a repository over [file], shut down completely afterwards. */
    private fun <T> launch(file: File, block: suspend (SettingsRepository) -> T): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) { file }
        try {
            block(DataStoreSettingsRepository(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    @Test
    fun `all three are off until set, and survive the app being closed`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            val fresh = settings.state.first()
            assertThat(fresh.reviewBeforeSending).isFalse()
            assertThat(fresh.heldReleaseTaught).isFalse()
            assertThat(fresh.voiceCoachMarkShown).isFalse()
            settings.setReviewBeforeSending(true)
            settings.setHeldReleaseTaught()
            settings.setVoiceCoachMarkShown()
        }
        launch(file) { settings ->
            val later = settings.state.first()
            assertThat(later.reviewBeforeSending).isTrue()
            assertThat(later.heldReleaseTaught).isTrue()
            assertThat(later.voiceCoachMarkShown).isTrue()
        }
    }

    /** They are the device's: a sign-out keeps them, as it keeps the preview switches. */
    @Test
    fun `a sign-out keeps them`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setServerUrl("https://chat.example.com")
            settings.setReviewBeforeSending(true)
            settings.setHeldReleaseTaught()
            settings.setVoiceCoachMarkShown()

            settings.resetKeepingServerUrl()

            val after = settings.state.first()
            assertThat(after.reviewBeforeSending).isTrue()
            assertThat(after.heldReleaseTaught).isTrue()
            assertThat(after.voiceCoachMarkShown).isTrue()
        }
    }

    /** And switching Review Before Sending off is remembered too. */
    @Test
    fun `review before sending switches back off`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setReviewBeforeSending(true)
            settings.setReviewBeforeSending(false)
            assertThat(settings.state.first().reviewBeforeSending).isFalse()
        }
    }
}

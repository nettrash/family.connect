/*
 * VoiceSettingsPersistenceTest.kt
 * Family Connect (Android)
 *
 * The voice messages' per-DEVICE choices (#79, docs/audio-video-messages-
 * 2026-10-04.md), against the REAL DataStore. Until 2026-10-06 there were
 * three — "Review before sending" (S9), the first held release's lesson and
 * the coach mark — and all three went with the hold. What a test build
 * stored under their keys is never read, and the next sign-out clears it;
 * the one per-device line left, the recorder's first-time "Only you can see
 * this", still survives the app being closed and a sign-out.
 */

package me.nettrash.familyconnect.data.settings

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
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

    private val holdKeys = listOf("review_before_sending", "held_release_taught", "voice_coach_mark_shown")
        .map(::booleanPreferencesKey)

    /** One "process": a repository over [file], shut down completely afterwards. */
    private fun <T> launch(
        file: File,
        block: suspend (SettingsRepository, androidx.datastore.core.DataStore<androidx.datastore.preferences.core.Preferences>) -> T,
    ): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) { file }
        try {
            block(DataStoreSettingsRepository(store), store)
        } finally {
            job.cancelAndJoin()
        }
    }

    /** A test build's values under the hold's keys: read by nothing, and gone at the next sign-out. */
    @Test
    fun `the holds keys are never read and a sign-out clears them`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings, store ->
            store.edit { prefs -> holdKeys.forEach { prefs[it] = true } }
            settings.setServerUrl("https://chat.example.com")
            // Nothing in the state carries them any more; reading it is fine.
            assertThat(settings.state.first().serverUrl).isEqualTo("https://chat.example.com")

            settings.resetKeepingServerUrl()

            val prefs = store.data.first()
            for (key in holdKeys) assertThat(prefs.contains(key)).isFalse()
            assertThat(settings.state.first().serverUrl).isEqualTo("https://chat.example.com")
        }
    }

    /** The recorder's first-time line is still the device's: it survives closing and a sign-out. */
    @Test
    fun `the recorders first-time line survives closing and a sign-out`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings, _ ->
            assertThat(settings.state.first().roundPreviewTaught).isFalse()
            settings.setRoundPreviewTaught()
        }
        launch(file) { settings, _ ->
            assertThat(settings.state.first().roundPreviewTaught).isTrue()
            settings.resetKeepingServerUrl()
            assertThat(settings.state.first().roundPreviewTaught).isTrue()
        }
    }
}

/*
 * RoundVideoSettingsTest.kt
 * Family Connect (Android)
 *
 * What this device keeps about video messages (#79,
 * docs/audio-video-messages-2026-10-04.md, Phase 3), against the REAL
 * DataStore:
 *
 *  - the server's two limits (`max_round_video_ms`, `max_round_video_bytes`,
 *    "Discovery and limits") — a complete state-set, so a server rolled back
 *    to a build without them takes the video entries away again; one without
 *    the other is not a server that has them; and they are the ACCOUNT's
 *    server's, so a sign-out forgets them;
 *  - whether the recorder's first-time PREVIEW line has been shown (S3.4,
 *    S7.5: "the first time on this device") — the DEVICE's, so a sign-out
 *    keeps it.
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
import me.nettrash.familyconnect.data.repo.RoundVideoLimits
import me.nettrash.familyconnect.data.repo.roundVideoLimits
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class RoundVideoSettingsTest {

    @get:Rule
    val folder = TemporaryFolder()

    private val file get() = File(folder.root, "settings.preferences_pb")

    private fun <T> launch(block: suspend (SettingsRepository) -> T): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) { file }
        try {
            block(DataStoreSettingsRepository(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    @Test
    fun `no limits until a server says them, then they survive the app being closed`() {
        launch { settings ->
            assertThat(settings.state.first().roundVideoLimits).isNull()
            settings.setRoundVideoLimits(60_000, 12_582_912)
        }
        launch { settings ->
            assertThat(settings.state.first().roundVideoLimits).isEqualTo(RoundVideoLimits(60_000, 12_582_912))
        }
    }

    @Test
    fun `a server that stops saying them takes the video entries away`() {
        launch { settings ->
            settings.setRoundVideoLimits(60_000, 12_582_912)
            settings.setRoundVideoLimits(null, null)
            assertThat(settings.state.first().roundVideoLimits).isNull()
        }
    }

    @Test
    fun `one key without the other is not a server that has them`() {
        launch { settings ->
            settings.setRoundVideoLimits(60_000, null)
            assertThat(settings.state.first().roundVideoLimits).isNull()
            settings.setRoundVideoLimits(null, 12_582_912)
            assertThat(settings.state.first().roundVideoLimits).isNull()
            settings.setRoundVideoLimits(0, 12_582_912)
            assertThat(settings.state.first().roundVideoLimits).isNull()
        }
    }

    @Test
    fun `a sign-out forgets the server's limits and keeps the device's teaching`() {
        launch { settings ->
            settings.setServerUrl("https://chat.example.com")
            settings.setRoundVideoLimits(60_000, 4_194_304)
            assertThat(settings.state.first().roundPreviewTaught).isFalse()
            settings.setRoundPreviewTaught()

            settings.resetKeepingServerUrl()

            val after = settings.state.first()
            assertThat(after.roundVideoLimits).isNull()
            assertThat(after.roundPreviewTaught).isTrue()
        }
        launch { settings -> assertThat(settings.state.first().roundPreviewTaught).isTrue() }
    }
}

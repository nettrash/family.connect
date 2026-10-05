/*
 * ParkedRecordingsPersistenceTest.kt
 * Family Connect (Android)
 *
 * The "Voice message not sent" index (#79, docs/audio-video-messages-2026-10-04.md,
 * S2.8): "the file, its length, the id of its reply and its caption — so it
 * survives the app being closed, and deleted at sign-out".
 *
 * Against the REAL DataStore-backed repository and a real file, since
 * "survives the app being closed" is a claim about the disk: the fake
 * settings the other tests use would pass whatever the production class did.
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
import me.nettrash.familyconnect.data.net.dto.ReplyToDto
import me.nettrash.familyconnect.data.repo.ParkedRecording
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class ParkedRecordingsPersistenceTest {

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

    private val first = ParkedRecording(
        id = "a",
        chatId = 42,
        file = "a.m4a",
        durationMs = 42_000,
        replyTo = ReplyToDto(messageId = 501, senderId = 9, excerpt = "Are you coming?"),
        caption = "for grandma",
    )
    private val second = ParkedRecording(id = "b", chatId = 7, file = "b.m4a", durationMs = 1_000)

    @Test
    fun `not-sent recordings survive the app being closed, every field of them`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.updateParkedRecordings { it + first }
            settings.updateParkedRecordings { it + second }
        }

        val after = launch(file) { settings -> settings.state.first().parkedRecordings }

        assertThat(after).containsExactly(first, second).inOrder()
    }

    @Test
    fun `a removal rewrites the list it was given, not a stale copy`() {
        val file = File(folder.root, "settings.preferences_pb")
        val after = launch(file) { settings ->
            settings.updateParkedRecordings { listOf(first, second) }
            settings.updateParkedRecordings { entries -> entries.filterNot { it.id == "a" } }
            settings.state.first().parkedRecordings
        }

        assertThat(after).containsExactly(second)
    }

    @Test
    fun `sign-out deletes everything recorded and not sent`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setServerUrl("https://example.invalid")
            settings.updateParkedRecordings { listOf(first, second) }
            // What SessionRepository.clearSession does to the settings.
            settings.resetKeepingServerUrl()
        }

        val after = launch(file) { settings -> settings.state.first() }

        assertThat(after.parkedRecordings).isEmpty()
        // The one thing a sign-out keeps, so the test is known to have read
        // the file it wrote.
        assertThat(after.serverUrl).isEqualTo("https://example.invalid")
    }

    @Test
    fun `a corrupt index reads as empty instead of throwing in every screen`() {
        assertThat(ParkedRecording.decode("{not json")).isEmpty()
        assertThat(ParkedRecording.decode(null)).isEmpty()
        assertThat(ParkedRecording.decode(ParkedRecording.encode(listOf(first)))).containsExactly(first)
    }
}

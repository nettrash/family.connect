/*
 * PackRecentsPersistenceTest.kt
 * Family Connect (Android)
 *
 * "Recently used" in the sticker panel (docs/protocol.md, "Sticker pack":
 * "which stickers somebody used most recently is that DEVICE's own
 * business"): the last sixteen, kept PER DEVICE — so they are there after a
 * restart, not only for as long as the process lives — and gone at
 * sign-out, because they are one person's habits and the next account on
 * this phone is somebody else.
 *
 * Against the REAL DataStore-backed repository and a real file, since
 * "survives a restart" is a claim about the disk: the fake settings the
 * other tests use would pass whatever the production class did.
 */

package me.nettrash.familyconnect.data.settings

import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import me.nettrash.familyconnect.data.repo.PackRules
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class PackRecentsPersistenceTest {

    @get:Rule
    val folder = TemporaryFolder()

    /**
     * One "process": a repository over [file], handed to [block], and then
     * shut down completely — DataStore allows one live instance per file,
     * so the next launch can only open it once this one has let go.
     */
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
    fun `recently used survives a restart, in order`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            var recents = emptyList<Long>()
            for (id in listOf(5L, 9L, 2L, 9L)) recents = PackRules.used(recents, id)
            settings.setPackRecents(recents)
        }

        // A new process: nothing in memory, only what the disk kept.
        val after = launch(file) { settings -> settings.state.first().packRecents }

        // Newest first, each once.
        assertThat(after).containsExactly(9L, 2L, 5L).inOrder()
    }

    @Test
    fun `the last sixteen are kept and no more`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            var recents = emptyList<Long>()
            for (id in 1L..40L) recents = PackRules.used(recents, id)
            settings.setPackRecents(recents)
        }

        val after = launch(file) { settings -> settings.state.first().packRecents }

        assertThat(PackRules.MAX_RECENTS).isEqualTo(16)
        assertThat(after).containsExactly(*(40L downTo 25L).toList().toTypedArray()).inOrder()
    }

    @Test
    fun `sign-out clears recently used and the pack's cursor`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setServerUrl("https://example.invalid")
            settings.setPackRecents(listOf(9L, 2L, 5L))
            settings.setPackCursor(41)
            // What SessionRepository.clearSession does to the settings.
            settings.resetKeepingServerUrl()
        }

        val after = launch(file) { settings -> settings.state.first() }

        assertThat(after.packRecents).isEmpty()
        assertThat(after.packCursor).isEqualTo(0)
        // The one thing a sign-out keeps, so the test is known to have
        // read the file it wrote.
        assertThat(after.serverUrl).isEqualTo("https://example.invalid")
    }
}

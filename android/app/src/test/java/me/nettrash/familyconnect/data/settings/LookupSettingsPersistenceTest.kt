/*
 * LookupSettingsPersistenceTest.kt
 * Family Connect (Android)
 *
 * What this device keeps about "Looking things up" (docs/protocol.md):
 * the providers in the SERVER's order, the member's lookup stamp, and the
 * owner's switch — kept across a restart, cleared with the assistant, and
 * gone at sign-out, since another account on this phone may be on a
 * server with different sources or none.
 *
 * Against the REAL DataStore-backed repository and a real file, for
 * PackRecentsPersistenceTest's reason: the fake would pass whatever the
 * production class did.
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
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import java.io.File

class LookupSettingsPersistenceTest {

    @get:Rule
    val folder = TemporaryFolder()

    private fun <T> launch(file: File, block: suspend (SettingsRepository) -> T): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) { file }
        try {
            block(DataStoreSettingsRepository(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    private suspend fun SettingsRepository.assistantWith(lookups: List<String>?) = setAssistant(
        userId = 1L,
        displayName = "Assistant",
        processor = "Microsoft — Azure OpenAI (Sweden Central)",
        lookups = lookups,
    )

    @Test
    fun `the providers survive a restart, in the server's order`() {
        val file = File(folder.root, "settings.preferences_pb")
        // Not alphabetical, so a store that sorted them would show.
        launch(file) { it.assistantWith(listOf("SearXNG", "Open-Meteo", "Wikipedia")) }

        val after = launch(file) { it.state.first().assistantLookups }

        assertThat(after).containsExactly("SearXNG", "Open-Meteo", "Wikipedia").inOrder()
    }

    @Test
    fun `absent, empty and blank all store nobody`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.assistantWith(listOf("SearXNG"))
            settings.assistantWith(null)
            assertThat(settings.state.first().assistantLookups).isEmpty()
            settings.assistantWith(listOf("SearXNG"))
            settings.assistantWith(emptyList())
            assertThat(settings.state.first().assistantLookups).isEmpty()
            settings.assistantWith(listOf(" ", ""))
            assertThat(settings.state.first().assistantLookups).isEmpty()
        }
    }

    @Test
    fun `a server with no assistant takes the providers away`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.assistantWith(listOf("Open-Meteo"))
            settings.setAssistant(userId = null, displayName = null)
            assertThat(settings.state.first().assistantLookups).isEmpty()
        }
    }

    @Test
    fun `the lookup stamp and the owner's switch are kept, and false and null are written`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
            settings.setFamilyAiLookups(true)
        }
        launch(file) { settings ->
            val state = settings.state.first()
            assertThat(state.assistantLookupConsentAt).isEqualTo("2026-10-03T09:00:00Z")
            assertThat(state.familyAiLookups).isTrue()
            settings.setAssistantLookupConsentAt(null)
            settings.setFamilyAiLookups(false)
        }
        val after = launch(file) { it.state.first() }
        assertThat(after.assistantLookupConsentAt).isNull()
        assertThat(after.familyAiLookups).isFalse()
    }

    @Test
    fun `sign-out forgets all of it`() {
        val file = File(folder.root, "settings.preferences_pb")
        launch(file) { settings ->
            settings.setServerUrl("https://chat.example.com")
            settings.assistantWith(listOf("Brave Search"))
            settings.setAssistantLookupConsentAt("2026-10-03T09:00:00Z")
            settings.setFamilyAiLookups(true)
            settings.resetKeepingServerUrl()
        }
        val after = launch(file) { it.state.first() }
        assertThat(after.assistantLookups).isEmpty()
        assertThat(after.assistantLookupConsentAt).isNull()
        assertThat(after.familyAiLookups).isFalse()
    }

    @Test
    fun `an older install, with none of these keys, reads as no lookups`() {
        val state = launch(File(folder.root, "settings.preferences_pb")) { it.state.first() }
        assertThat(state.assistantLookups).isEmpty()
        assertThat(state.assistantLookupConsentAt).isNull()
        assertThat(state.familyAiLookups).isFalse()
    }
}

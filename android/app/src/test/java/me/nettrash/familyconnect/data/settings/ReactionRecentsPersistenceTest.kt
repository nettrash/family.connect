/*
 * ReactionRecentsPersistenceTest.kt
 * Family Connect (Android)
 *
 * Recent picker choices written through the real settings store.
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
import me.nettrash.familyconnect.ui.chat.EMOJI_CATALOG
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class ReactionRecentsPersistenceTest {
    @get:Rule
    val folder = TemporaryFolder()

    private val url = "https://first.example.invalid"
    private val owner = ReactionRecents.Scope(url, 1L)

    private fun <T> launch(block: suspend (DataStoreSettingsRepository) -> T): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) {
            folder.root.resolve("settings.preferences_pb")
        }
        try {
            block(DataStoreSettingsRepository(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    @Test
    fun newestChoicesSurviveRestartAndKeepAtMostTwentyFourUniqueEntries() {
        val emoji = EMOJI_CATALOG.first().emoji.take(26)
        launch { store ->
            store.setServerUrl(url)
            store.setProfile(1, "one", "One", 0)
            for (glyph in emoji) store.recordRecentReaction(owner, glyph)
            store.recordRecentReaction(owner, "❤")
            store.recordRecentReaction(owner, "❤️")
        }

        val saved = launch { it.reactionRecents.first() }
        assertThat(saved.scope).isEqualTo(owner)
        assertThat(saved.choices)
            .containsExactlyElementsIn(listOf("❤️") + emoji.takeLast(23).reversed()).inOrder()
    }

    @Test
    fun changingAccountOrServerClearsChoicesAndRejectsWritesForTheOldOwner() = launch { store ->
        store.setServerUrl(url)
        store.setProfile(1, "one", "One", 0)
        store.recordRecentReaction(owner, "🔥")

        store.setProfile(2, "two", "Two", 0)
        store.recordRecentReaction(owner, "👍")
        val second = ReactionRecents.Scope(url, 2)
        assertThat(store.reactionRecents.first()).isEqualTo(ReactionRecents(second))
        store.recordRecentReaction(second, "🧊")
        assertThat(store.reactionRecents.first().choices).containsExactly("🧊")

        val secondUrl = "https://second.example.invalid"
        store.setServerUrl(secondUrl)
        store.recordRecentReaction(second, "👍")
        assertThat(store.reactionRecents.first())
            .isEqualTo(ReactionRecents(ReactionRecents.Scope(secondUrl, 2)))
    }
}

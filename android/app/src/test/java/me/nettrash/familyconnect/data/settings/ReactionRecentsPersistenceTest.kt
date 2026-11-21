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
    private val session = ReactionRecents.scope(url, 1L, "session-one")!!

    private fun <T> launch(block: suspend (ReactionRecents) -> T): T = runBlocking {
        val job = Job()
        val store = PreferenceDataStoreFactory.create(scope = CoroutineScope(Dispatchers.IO + job)) {
            folder.root.resolve("reaction-recents.preferences_pb")
        }
        try {
            block(ReactionRecents(store))
        } finally {
            job.cancelAndJoin()
        }
    }

    @Test
    fun newestChoicesSurviveRestartAndKeepAtMostTwentyFourUniqueEntries() {
        val emoji = EMOJI_CATALOG.first().emoji.take(26)
        launch { store ->
            for (glyph in emoji) store.record(session, glyph) { session }
            store.record(session, "❤") { session }
            store.record(session, "❤️") { session }
        }

        val saved = launch { it.observe(session).first().choices }
        assertThat(saved).containsExactlyElementsIn(listOf("❤️") + emoji.takeLast(23).reversed()).inOrder()
    }

    @Test
    fun sessionsAccountsAndServersCannotReadOrOverwriteOneAnothersRecents() = launch { store ->
        store.record(session, "🔥") { session }
        for (other in listOf(
            ReactionRecents.scope(url, 1L, "session-two")!!,
            ReactionRecents.scope(url, 2L, "session-one")!!,
            ReactionRecents.scope("https://second.example.invalid", 1L, "session-one")!!,
        )) {
            assertThat(store.observe(other).first().choices).isEmpty()
            store.record(other, "👍") { session }
            store.record(session, "👍") { other }
            assertThat(store.observe(session).first().choices).containsExactly("🔥")
        }
    }

    @Test
    fun signOutClearsChoicesAndLateWritesAreIgnored() = launch { store ->
        store.record(session, "🔥") { session }
        store.clear()
        store.record(session, "👍") { null }
        assertThat(store.observe(session).first().choices).isEmpty()
        assertThat(ReactionRecents.scope(url, 1L, null)).isNull()
        assertThat(ReactionRecents.scope(url, 0L, "session-one")).isNull()
        assertThat(ReactionRecents.scope(null, 1L, "session-one")).isNull()
    }
}

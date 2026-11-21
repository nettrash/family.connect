package me.nettrash.familyconnect.data.settings

import android.content.Context
import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.emptyPreferences
import androidx.datastore.preferences.core.stringPreferencesKey
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlinx.coroutines.flow.catch
import kotlinx.coroutines.flow.map
import me.nettrash.familyconnect.util.reactionEmojiKey
import java.io.File
import java.io.IOException
import java.security.MessageDigest
import java.util.Base64
import javax.inject.Inject
import javax.inject.Singleton

/** Only the active session's choices are stored, in a file excluded from device backup. */
@Singleton
class ReactionRecents internal constructor(private val store: DataStore<Preferences>) {
    @Inject
    constructor(@ApplicationContext context: Context) : this(PreferenceDataStoreFactory.create {
        File(context.noBackupFilesDir, "reaction-recents.preferences_pb")
    })

    internal data class Snapshot(val scope: String?, val choices: List<String> = emptyList())

    internal fun observe(scope: String) = store.data
        .catch { if (it is IOException) emit(emptyPreferences()) else throw it }
        .map { prefs -> Snapshot(scope, prefs.choices(scope)) }

    internal suspend fun record(scope: String, emoji: String, currentScope: suspend () -> String?) {
        try {
            store.edit { prefs ->
                if (currentScope() != scope) return@edit
                val choices = listOf(emoji) + prefs.choices(scope)
                prefs[Scope] = scope
                prefs[Choices] = choices.distinctBy(::reactionEmojiKey).take(24).joinToString(",")
            }
        } catch (_: IOException) {
            // A local preference write must not interfere with the reaction.
        }
    }

    internal suspend fun clear() {
        try {
            store.edit { it.clear() }
        } catch (_: IOException) {
            // The session check still prevents later sessions from seeing these choices.
        }
    }

    private fun Preferences.choices(scope: String): List<String> =
        if (this[Scope] == scope) this[Choices]?.split(',').orEmpty() else emptyList()

    internal companion object {
        private val Scope = stringPreferencesKey("scope")
        private val Choices = stringPreferencesKey("choices")

        fun scope(serverUrl: String?, userId: Long?, token: String?): String? {
            if (serverUrl.isNullOrBlank() || userId == null || userId <= 0 || token.isNullOrBlank()) return null
            val bytes = "$serverUrl\u0000$userId\u0000$token".toByteArray(Charsets.UTF_8)
            return Base64.getEncoder().encodeToString(MessageDigest.getInstance("SHA-256").digest(bytes))
        }
    }
}

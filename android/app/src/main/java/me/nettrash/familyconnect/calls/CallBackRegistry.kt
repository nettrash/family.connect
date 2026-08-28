/*
 * CallBackRegistry.kt
 * Family Connect (Android)
 *
 * The Phone app's call log can call a Family call back (Android 16.1+):
 * it hands the app the UUID Telecom gave the call and nothing else, so
 * the app has to remember, per call, who that was — the chat, the
 * member, voice or video. Kept on this device only, bounded, and never
 * on the wire: the server's own call records are the family's history;
 * this is merely the index the system dialer needs.
 *
 * Behind a two-method store so the JSON round trip is pinned on the JVM
 * (CallBackRegistryTest); the app's store is SharedPreferences.
 */

package me.nettrash.familyconnect.calls

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import javax.inject.Inject
import javax.inject.Singleton
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

@Serializable
data class CallBackEntry(
    val uuid: String,
    val chatId: Long,
    val peerUserId: Long,
    val video: Boolean,
)

/** Where the registry keeps its one string. */
interface CallBackStore {
    fun read(): String?
    fun write(value: String?)
}

@Singleton
class CallBackRegistry(private val store: CallBackStore) {

    @Inject
    constructor(@ApplicationContext context: Context) : this(PreferencesStore(context))

    private val json = Json { ignoreUnknownKeys = true }

    fun all(): List<CallBackEntry> {
        val raw = store.read() ?: return emptyList()
        return runCatching { json.decodeFromString<List<CallBackEntry>>(raw) }.getOrDefault(emptyList())
    }

    /** Newest last; the oldest beyond [LIMIT] are forgotten. */
    fun remember(entry: CallBackEntry) {
        val kept = all().filter { it.uuid != entry.uuid } + entry
        store.write(json.encodeToString(kept.takeLast(LIMIT)))
    }

    fun find(uuid: String): CallBackEntry? = all().firstOrNull { it.uuid == uuid }

    /** A logout, or leaving the family: the ids mean nothing to the next session. */
    fun clear() = store.write(null)

    private class PreferencesStore(context: Context) : CallBackStore {
        private val prefs = context.getSharedPreferences("call_backs", Context.MODE_PRIVATE)
        override fun read(): String? = prefs.getString(KEY, null)
        override fun write(value: String?) {
            prefs.edit().apply { if (value == null) remove(KEY) else putString(KEY, value) }.apply()
        }
    }

    companion object {
        const val LIMIT = 100
        private const val KEY = "entries"
    }
}

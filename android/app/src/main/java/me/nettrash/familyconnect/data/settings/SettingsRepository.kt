/*
 * SettingsRepository.kt
 * Family Connect (Android)
 *
 * Small, non-secret app state persisted in Preferences DataStore: the
 * server URL, the reconciled family status, and a denormalized profile
 * snapshot (my user id / username / display name / family name) so the
 * UI can render instantly without a network round-trip.
 *
 * The session *token* deliberately does not live here — it's a secret
 * and goes through TokenStore (Keystore-wrapped) instead.
 *
 * Split into interface + DataStore-backed impl so unit tests can swap in
 * an in-memory fake without touching disk.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Settings/SettingsStore.swift
 */

package me.nettrash.familyconnect.data.settings

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map
import me.nettrash.familyconnect.data.repo.FamilyStatus
import javax.inject.Inject
import javax.inject.Singleton

/**
 * One immutable snapshot of everything in the store. `familyStatus`
 * only ever persists NONE / PENDING / MEMBER / OWNER — the derived
 * NO_SERVER / NO_TOKEN states are computed by SessionRepository from
 * `serverUrl` and TokenStore.
 */
data class SettingsState(
    val serverUrl: String? = null,
    val familyStatus: FamilyStatus = FamilyStatus.NONE,
    val myUserId: Long? = null,
    val myUsername: String? = null,
    val myDisplayName: String? = null,
    val familyName: String? = null,
    /** My own profile-picture version; 0 = no picture. */
    val myAvatarVersion: Long = 0,
    /** Last FCM registration token seen — device-scoped, survives logout. */
    val pushToken: String? = null,
    /** `device_id` from POST /devices — account-scoped, cleared on logout. */
    val pushDeviceId: Long? = null,
    /**
     * Whether a message's first web link gets a preview card. On by
     * default, but switchable because building one means THIS device
     * requests the linked page — the only routine traffic the app sends
     * anywhere but the family's own server.
     */
    val linkPreviewsEnabled: Boolean = true,
    /** Highest board_seq applied on this device; 0 = nothing yet. */
    val boardCursor: Long = 0,
    /**
     * Highest note id this device has actually SHOWN the user, for the
     * board badge.
     *
     * Deliberately not [boardCursor]. That is a SYNC cursor and advances
     * whenever a change is applied — including a background resync — so
     * using it here would clear the badge for someone who never opened the
     * board. This one moves only when the board is on screen.
     */
    val boardSeenNoteId: Long = 0,
    /**
     * The assistant's reserved account id, or null when the server has no
     * assistant configured.
     *
     * Null means BOTH "there is nobody to name" and "do not offer the
     * mention": a composer that offered `@ai` against a server without an
     * assistant would offer an affordance that silently does nothing
     * (docs/protocol.md, "Mentioning the assistant in the family chat").
     */
    /**
     * Whether a shared location draws a map. On by default; switchable
     * because drawing one asks GOOGLE for tiles — the same trade the link
     * previews make with the linked site, and the second of the only two
     * things this app fetches from anywhere but the family's own server.
     */
    val mapPreviewsEnabled: Boolean = true,
    val assistantUserId: Long? = null,
    /** What to call it — server-configured, not compiled in here. */
    val assistantName: String? = null,
    /**
     * Whether the server signals voice calls (`GET /me` → calls_enabled).
     * Account-scoped like the assistant: a different server may have them
     * off. False hides the call button rather than letting somebody
     * discover `calls_disabled` at the moment they want to talk.
     */
    val callsEnabled: Boolean = false,
)

interface SettingsRepository {
    val state: Flow<SettingsState>

    suspend fun setServerUrl(url: String)
    suspend fun setFamilyStatus(status: FamilyStatus)
    suspend fun setProfile(userId: Long, username: String, displayName: String, avatarVersion: Long)

    /**
     * Separate from setProfile because the avatar endpoints answer with
     * the new version alone — rewriting the name fields from a stale
     * cached copy would be the only way to lose a rename.
     */
    suspend fun setMyAvatarVersion(version: Long)
    suspend fun setFamilyName(name: String?)
    suspend fun setPushToken(token: String?)
    suspend fun setPushDeviceId(deviceId: Long?)
    suspend fun setLinkPreviewsEnabled(enabled: Boolean)

    /**
     * The board catch-up cursor: the highest board_seq this device has
     * APPLIED. Local-only and account-scoped, so it is wiped with the
     * session — a different family's board must never be caught up from
     * another's cursor.
     */
    suspend fun setBoardCursor(seq: Long)

    /**
     * Session teardown: wipe everything EXCEPT the server URL (protocol:
     * "the client wipes local state, keeping the server URL") and the FCM
     * push token — the token identifies the *device*, not the account, so
     * the next login can re-register /devices immediately instead of
     * waiting for Firebase to deliver the token again. The device *id* is
     * account-scoped and is dropped with everything else.
     */
    suspend fun setBoardSeenNoteId(noteId: Long)

    /**
     * Record (or clear) the assistant the server just reported. Account-
     * scoped, so it goes with the session on logout — a different server
     * may have no assistant, or a differently named one.
     */
    suspend fun setMapPreviewsEnabled(enabled: Boolean)

    suspend fun setAssistant(userId: Long?, displayName: String?)

    /** Record what `GET /me` said about voice calls on this server. */
    suspend fun setCallsEnabled(enabled: Boolean)

    suspend fun resetKeepingServerUrl()
}

@Singleton
class DataStoreSettingsRepository @Inject constructor(
    private val dataStore: DataStore<Preferences>,
) : SettingsRepository {

    private object Keys {
        val SERVER_URL = stringPreferencesKey("server_url")
        val FAMILY_STATUS = stringPreferencesKey("family_status")
        val MY_USER_ID = longPreferencesKey("my_user_id")
        val MY_USERNAME = stringPreferencesKey("my_username")
        val MY_DISPLAY_NAME = stringPreferencesKey("my_display_name")
        val MY_AVATAR_VERSION = longPreferencesKey("my_avatar_version")
        val FAMILY_NAME = stringPreferencesKey("family_name")
        val PUSH_TOKEN = stringPreferencesKey("push_token")
        val PUSH_DEVICE_ID = longPreferencesKey("push_device_id")
        // Stored inverted so a missing key reads as "on".
        val LINK_PREVIEWS_DISABLED = booleanPreferencesKey("link_previews_disabled")
        val BOARD_CURSOR = longPreferencesKey("board_cursor")
        val BOARD_SEEN_NOTE_ID = longPreferencesKey("board_seen_note_id")
        // Stored inverted so a missing key reads as "on", like the link
        // preview key above.
        val MAP_PREVIEWS_DISABLED = booleanPreferencesKey("map_previews_disabled")
        val ASSISTANT_USER_ID = longPreferencesKey("assistant_user_id")
        val ASSISTANT_NAME = stringPreferencesKey("assistant_name")
        val CALLS_ENABLED = booleanPreferencesKey("calls_enabled")
    }

    override val state: Flow<SettingsState> = dataStore.data.map { prefs ->
        SettingsState(
            serverUrl = prefs[Keys.SERVER_URL],
            familyStatus = prefs[Keys.FAMILY_STATUS]
                ?.let { raw -> FamilyStatus.entries.firstOrNull { it.name == raw } }
                ?: FamilyStatus.NONE,
            myUserId = prefs[Keys.MY_USER_ID],
            myUsername = prefs[Keys.MY_USERNAME],
            myDisplayName = prefs[Keys.MY_DISPLAY_NAME],
            familyName = prefs[Keys.FAMILY_NAME],
            myAvatarVersion = prefs[Keys.MY_AVATAR_VERSION] ?: 0,
            pushToken = prefs[Keys.PUSH_TOKEN],
            pushDeviceId = prefs[Keys.PUSH_DEVICE_ID],
            linkPreviewsEnabled = prefs[Keys.LINK_PREVIEWS_DISABLED] != true,
            boardCursor = prefs[Keys.BOARD_CURSOR] ?: 0L,
            boardSeenNoteId = prefs[Keys.BOARD_SEEN_NOTE_ID] ?: 0L,
            mapPreviewsEnabled = prefs[Keys.MAP_PREVIEWS_DISABLED] != true,
            assistantUserId = prefs[Keys.ASSISTANT_USER_ID],
            assistantName = prefs[Keys.ASSISTANT_NAME],
            callsEnabled = prefs[Keys.CALLS_ENABLED] == true,
        )
    }

    override suspend fun setServerUrl(url: String) {
        dataStore.edit { it[Keys.SERVER_URL] = url }
    }

    override suspend fun setFamilyStatus(status: FamilyStatus) {
        dataStore.edit { it[Keys.FAMILY_STATUS] = status.name }
    }

    override suspend fun setProfile(
        userId: Long,
        username: String,
        displayName: String,
        avatarVersion: Long,
    ) {
        dataStore.edit {
            it[Keys.MY_USER_ID] = userId
            it[Keys.MY_USERNAME] = username
            it[Keys.MY_DISPLAY_NAME] = displayName
            it[Keys.MY_AVATAR_VERSION] = avatarVersion
        }
    }

    override suspend fun setMyAvatarVersion(version: Long) {
        dataStore.edit { it[Keys.MY_AVATAR_VERSION] = version }
    }

    override suspend fun setFamilyName(name: String?) {
        dataStore.edit {
            if (name == null) it.remove(Keys.FAMILY_NAME) else it[Keys.FAMILY_NAME] = name
        }
    }

    override suspend fun setPushToken(token: String?) {
        dataStore.edit {
            if (token == null) it.remove(Keys.PUSH_TOKEN) else it[Keys.PUSH_TOKEN] = token
        }
    }

    override suspend fun setPushDeviceId(deviceId: Long?) {
        dataStore.edit {
            if (deviceId == null) it.remove(Keys.PUSH_DEVICE_ID) else it[Keys.PUSH_DEVICE_ID] = deviceId
        }
    }

    override suspend fun setLinkPreviewsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.LINK_PREVIEWS_DISABLED] = !enabled }
    }

    override suspend fun setBoardCursor(seq: Long) {
        dataStore.edit { it[Keys.BOARD_CURSOR] = seq }
    }

    override suspend fun setBoardSeenNoteId(noteId: Long) {
        dataStore.edit { prefs ->
            // Never goes backwards: two screens marking it at once must not
            // resurrect a badge that was already cleared.
            val current = prefs[Keys.BOARD_SEEN_NOTE_ID] ?: 0L
            if (noteId > current) prefs[Keys.BOARD_SEEN_NOTE_ID] = noteId
        }
    }

    override suspend fun setMapPreviewsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.MAP_PREVIEWS_DISABLED] = !enabled }
    }

    override suspend fun setAssistant(userId: Long?, displayName: String?) {
        dataStore.edit { prefs ->
            if (userId != null && displayName != null) {
                prefs[Keys.ASSISTANT_USER_ID] = userId
                prefs[Keys.ASSISTANT_NAME] = displayName
            } else {
                // Cleared rather than left stale: a server that turned the
                // assistant off must stop offering `@ai` on the next resync.
                prefs.remove(Keys.ASSISTANT_USER_ID)
                prefs.remove(Keys.ASSISTANT_NAME)
            }
        }
    }

    override suspend fun setCallsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.CALLS_ENABLED] = enabled }
    }

    override suspend fun resetKeepingServerUrl() {
        dataStore.edit { prefs ->
            val keepUrl = prefs[Keys.SERVER_URL]
            // Device-scoped, not account-scoped — see the interface doc.
            val keepPushToken = prefs[Keys.PUSH_TOKEN]
            // Likewise device-scoped, and a PRIVACY choice: a logout —
            // or any 401 — silently turning link previews back on would
            // start contacting third-party sites the user opted out of.
            val keepLinkPreviews = prefs[Keys.LINK_PREVIEWS_DISABLED]
            // Same reasoning as the line above, and the same consequence if
            // it is dropped: a logout — or any 401 — silently turning map
            // previews back on would resume asking Google for tiles that
            // this person opted out of.
            val keepMapPreviews = prefs[Keys.MAP_PREVIEWS_DISABLED]
            prefs.clear()
            keepUrl?.let { prefs[Keys.SERVER_URL] = it }
            keepPushToken?.let { prefs[Keys.PUSH_TOKEN] = it }
            keepLinkPreviews?.let { prefs[Keys.LINK_PREVIEWS_DISABLED] = it }
            keepMapPreviews?.let { prefs[Keys.MAP_PREVIEWS_DISABLED] = it }
        }
    }
}

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
)

interface SettingsRepository {
    val state: Flow<SettingsState>

    suspend fun setServerUrl(url: String)
    suspend fun setFamilyStatus(status: FamilyStatus)
    suspend fun setProfile(userId: Long, username: String, displayName: String)
    suspend fun setFamilyName(name: String?)
    suspend fun setPushToken(token: String?)
    suspend fun setPushDeviceId(deviceId: Long?)
    suspend fun setLinkPreviewsEnabled(enabled: Boolean)

    /**
     * Session teardown: wipe everything EXCEPT the server URL (protocol:
     * "the client wipes local state, keeping the server URL") and the FCM
     * push token — the token identifies the *device*, not the account, so
     * the next login can re-register /devices immediately instead of
     * waiting for Firebase to deliver the token again. The device *id* is
     * account-scoped and is dropped with everything else.
     */
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
        val FAMILY_NAME = stringPreferencesKey("family_name")
        val PUSH_TOKEN = stringPreferencesKey("push_token")
        val PUSH_DEVICE_ID = longPreferencesKey("push_device_id")
        // Stored inverted so a missing key reads as "on".
        val LINK_PREVIEWS_DISABLED = booleanPreferencesKey("link_previews_disabled")
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
            pushToken = prefs[Keys.PUSH_TOKEN],
            pushDeviceId = prefs[Keys.PUSH_DEVICE_ID],
            linkPreviewsEnabled = prefs[Keys.LINK_PREVIEWS_DISABLED] != true,
        )
    }

    override suspend fun setServerUrl(url: String) {
        dataStore.edit { it[Keys.SERVER_URL] = url }
    }

    override suspend fun setFamilyStatus(status: FamilyStatus) {
        dataStore.edit { it[Keys.FAMILY_STATUS] = status.name }
    }

    override suspend fun setProfile(userId: Long, username: String, displayName: String) {
        dataStore.edit {
            it[Keys.MY_USER_ID] = userId
            it[Keys.MY_USERNAME] = username
            it[Keys.MY_DISPLAY_NAME] = displayName
        }
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

    override suspend fun resetKeepingServerUrl() {
        dataStore.edit { prefs ->
            val keepUrl = prefs[Keys.SERVER_URL]
            // Device-scoped, not account-scoped — see the interface doc.
            val keepPushToken = prefs[Keys.PUSH_TOKEN]
            // Likewise device-scoped, and a PRIVACY choice: a logout —
            // or any 401 — silently turning link previews back on would
            // start contacting third-party sites the user opted out of.
            val keepLinkPreviews = prefs[Keys.LINK_PREVIEWS_DISABLED]
            prefs.clear()
            keepUrl?.let { prefs[Keys.SERVER_URL] = it }
            keepPushToken?.let { prefs[Keys.PUSH_TOKEN] = it }
            keepLinkPreviews?.let { prefs[Keys.LINK_PREVIEWS_DISABLED] = it }
        }
    }
}

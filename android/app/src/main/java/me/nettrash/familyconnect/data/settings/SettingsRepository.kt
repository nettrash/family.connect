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
)

interface SettingsRepository {
    val state: Flow<SettingsState>

    suspend fun setServerUrl(url: String)
    suspend fun setFamilyStatus(status: FamilyStatus)
    suspend fun setProfile(userId: Long, username: String, displayName: String)
    suspend fun setFamilyName(name: String?)

    /** Session teardown: wipe everything EXCEPT the server URL. */
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

    override suspend fun resetKeepingServerUrl() {
        dataStore.edit { prefs ->
            val keepUrl = prefs[Keys.SERVER_URL]
            prefs.clear()
            keepUrl?.let { prefs[Keys.SERVER_URL] = it }
        }
    }
}

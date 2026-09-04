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
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import androidx.datastore.preferences.core.stringSetPreferencesKey
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
    /**
     * Everybody this account has blocked. Account-scoped and cleared with
     * the session, then restored wholesale from the next `GET /me` — a
     * block is server state, not a device preference.
     *
     * Deliberately NOT a column on `members`: `upsertRoster` writes whole
     * rows and would reset the flag on every refresh, `wipeAll()` fires
     * when the caller leaves a family (and a block is a pair, not a
     * membership), and a blocked id may name somebody with no roster row
     * at all — somebody who left, was deleted, or shares no family.
     */
    val blockedUserIds: Set<Long> = emptySet(),
    /**
     * The operator's ceiling on a family's size. Null on a server too old
     * to report it, and the cap control then hides rather than inventing a
     * bound to draw.
     */
    val maxFamilyMembers: Int? = null,
    /**
     * The operator's published contact, absent when unset. Shown on the
     * report screen: it is the honest escalation path for the case this
     * feature is weakest at — a report ABOUT the owner never reaches the
     * owner (docs/protocol.md, "Reporting a member").
     *
     * Free text, at most 256 characters. Drawn VERBATIM and never
     * linkified: an operator may write an address, a URL or a sentence,
     * and three apps guessing differently about which it is would be worse
     * than three apps showing the same text.
     */
    val supportContact: String? = null,
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
     * Highest `content_seq` this device has actually SHOWN — what the board
     * badge counts (docs/protocol.md, "Board").
     *
     * [boardSeenNoteId] above is kept beside it, and still used, for notes
     * that carry no content seq: rows cached before the column existed and
     * notes from a server that predates the field. BoardBadge holds the
     * rule; these two are only where the numbers live.
     */
    val boardSeenContentSeq: Long = 0,
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
     * Whether this SERVER has a deployment that can look at a picture
     * (`assistant.vision`, docs/protocol.md, "Pictures").
     *
     * One of the two locks the composer reads. It says nothing about
     * whether this FAMILY allows it — that is [familyAiVision] — and a
     * client needs both before it offers a picture to the assistant.
     * False on a server that has no such deployment, which is the
     * default and is a server where this cannot happen at all.
     */
    val assistantVision: Boolean = false,
    /**
     * Whether this server can MAKE a picture (`assistant.images`). The
     * whole of the `/draw` capability check — generation has no family
     * switch, because what leaves is strictly smaller than a text
     * question: the words after the token and nothing else. Since #56 it
     * also means the assistant may draw unasked; nothing here changes for
     * that (docs/protocol.md, "Drawing without being told to").
     */
    val assistantImages: Boolean = false,
    /**
     * Whether the family's OWNER has allowed a photograph a member points
     * the assistant at to be shown to the model (`Family.ai_vision`) — in
     * their own assistant chat, and since #56 on an `@ai` message in the
     * family chat or on the message it replies to.
     *
     * FALSE by default — the deliberate opposite of `ai_history` — and
     * false for every family that existed before it. Kept here beside
     * the assistant's capabilities because the composer has to answer
     * "may I offer this?" without a round trip, and account-scoped so it
     * goes with the session: another server's family is another answer.
     */
    val familyAiVision: Boolean = false,
    /**
     * Whether a mention of the assistant in the family chat carries the
     * recent history of that chat (`Family.ai_history`). TRUE by default,
     * as on the wire; here only so the family composer can tell that the
     * third switch below is inert — without a transcript no photo from
     * it can travel (docs/protocol.md, "Recent photos from the family
     * chat").
     */
    val familyAiHistory: Boolean = true,
    /**
     * The family's THIRD switch (`Family.ai_history_photos`): whether an
     * `@ai` mention may also be shown the chat's most recent photographs
     * — pictures nobody pointed the assistant at. FALSE by default and
     * for every family that predates it; only ever true while
     * [familyAiVision] is, because the server refuses the other state.
     * Kept here for [familyAiVision]'s reason: the composer's strip has
     * to say "up to N recent photos" without a round trip.
     */
    val familyAiHistoryPhotos: Boolean = false,
    /**
     * Whether the server signals voice calls (`GET /me` → calls_enabled).
     * Account-scoped like the assistant: a different server may have them
     * off. False hides the call button rather than letting somebody
     * discover `calls_disabled` at the moment they want to talk.
     */
    val callsEnabled: Boolean = false,
    /**
     * Whether it also allows VIDEO calls (`GET /me` → video_calls_enabled,
     * docs/protocol.md, "Video"). Gates the video-call button alone —
     * false on an old server, which then correctly offers voice only.
     */
    val videoCallsEnabled: Boolean = false,
    /**
     * Whether this server takes NEW families (docs/protocol.md, "Starting
     * a family"). The family gate swaps "Create a family" for directions
     * to run one's own server when this is false; true until a `/me`
     * says otherwise, which is also the answer on an older server.
     */
    val familyRegistrationEnabled: Boolean = true,
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

    /** The badge's real mark. Monotonic, like the one above. */
    suspend fun setBoardSeenContentSeq(seq: Long)

    /**
     * Record (or clear) the assistant the server just reported. Account-
     * scoped, so it goes with the session on logout — a different server
     * may have no assistant, or a differently named one.
     */
    suspend fun setMapPreviewsEnabled(enabled: Boolean)

    suspend fun setAssistant(
        userId: Long?,
        displayName: String?,
        vision: Boolean = false,
        images: Boolean = false,
    )

    /**
     * Record the family's own picture switch, from `GET /families/mine`
     * or from the owner's own PATCH.
     *
     * Unconditional, `false` included: it is a complete state-set like
     * the block list, not a delta. An owner turning it OFF must reach
     * every device, and a guard skipping the false case would leave the
     * composer offering a picture the server would never show.
     */
    suspend fun setFamilyAiVision(enabled: Boolean)

    /** Record the family's history switch, by the same rule. */
    suspend fun setFamilyAiHistory(enabled: Boolean)

    /**
     * Record the family's third switch, by the same rule — `false`
     * included, and especially: the server turns this off whenever
     * `ai_vision` goes off, whether or not this device asked.
     */
    suspend fun setFamilyAiHistoryPhotos(enabled: Boolean)

    /** Record what `GET /me` said about voice calls on this server. */
    suspend fun setCallsEnabled(enabled: Boolean)

    /** Record what `GET /me` said about VIDEO calls on this server. */
    suspend fun setVideoCallsEnabled(enabled: Boolean)

    /** Record what `GET /me` said about this server taking new families. */
    suspend fun setFamilyRegistrationEnabled(enabled: Boolean)

    /**
     * REPLACE the caller's block list with what the server just said.
     *
     * A complete state-set and never a delta, so the empty set is a real
     * value that MUST be written: `blocked_user_ids` is "the one read in
     * this protocol where absence is not allowed to mean 'leave what you
     * hold alone'" (docs/protocol.md, `GET /me`). A guard skipping the
     * empty case is exactly the bug the protocol names — the last unblock
     * would never reach a second device.
     *
     * That is the OPPOSITE of the rule the roster follows, where an absent
     * field never wipes a stored one. Do not copy that idiom here.
     */
    suspend fun setBlockedUserIds(ids: Collection<Long>)

    /** Record what `GET /me` said the operator's family-size ceiling is. */
    suspend fun setMaxFamilyMembers(limit: Int?)

    /** Record the operator's published support contact, or clear it. */
    suspend fun setSupportContact(contact: String?)

    suspend fun resetKeepingServerUrl()
}

@Singleton
class DataStoreSettingsRepository @Inject constructor(
    private val dataStore: DataStore<Preferences>,
) : SettingsRepository {

    private object Keys {
        val SERVER_URL = stringPreferencesKey("server_url")
        /**
         * Stored as strings because Preferences DataStore has no
         * `Set<Long>` key type. Order carries no meaning, so a set rather
         * than a joined string.
         */
        val BLOCKED_USER_IDS = stringSetPreferencesKey("blocked_user_ids")
        val MAX_FAMILY_MEMBERS = intPreferencesKey("max_family_members")
        val SUPPORT_CONTACT = stringPreferencesKey("support_contact")
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
        // A separate key rather than a reused one: the two numbers come
        // from different spaces — a note id and a board seq — and a device
        // that updates has a meaningful value for the old one and none for
        // the new (BoardBadge.contentMarkSeed).
        val BOARD_SEEN_CONTENT_SEQ = longPreferencesKey("board_seen_content_seq")
        // Stored inverted so a missing key reads as "on", like the link
        // preview key above.
        val MAP_PREVIEWS_DISABLED = booleanPreferencesKey("map_previews_disabled")
        val ASSISTANT_USER_ID = longPreferencesKey("assistant_user_id")
        val ASSISTANT_NAME = stringPreferencesKey("assistant_name")
        val ASSISTANT_VISION = booleanPreferencesKey("assistant_vision")
        val ASSISTANT_IMAGES = booleanPreferencesKey("assistant_images")
        // Stored PLAIN, not inverted like the two preview keys above: this
        // one's default is already `false`, so a missing key and an
        // explicit `false` say the same thing and neither can be read as
        // permission (docs/protocol.md, "Pictures").
        val FAMILY_AI_VISION = booleanPreferencesKey("family_ai_vision")
        // Stored plain and read with the protocol's own default (true) when
        // missing — the only one of the three whose absence means "on".
        val FAMILY_AI_HISTORY = booleanPreferencesKey("family_ai_history")
        // Plain, like FAMILY_AI_VISION and for its reason: a missing key
        // and an explicit `false` say the same thing.
        val FAMILY_AI_HISTORY_PHOTOS = booleanPreferencesKey("family_ai_history_photos")
        val CALLS_ENABLED = booleanPreferencesKey("calls_enabled")
        val VIDEO_CALLS_ENABLED = booleanPreferencesKey("video_calls_enabled")
        val FAMILY_REGISTRATION_ENABLED = booleanPreferencesKey("family_registration_enabled")
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
            // `toLongOrNull` rather than `toLong`: a corrupt entry must
            // not throw inside the map every screen collects.
            maxFamilyMembers = prefs[Keys.MAX_FAMILY_MEMBERS],
            supportContact = prefs[Keys.SUPPORT_CONTACT],
            blockedUserIds = prefs[Keys.BLOCKED_USER_IDS]
                ?.mapNotNull(String::toLongOrNull)
                ?.toSet()
                .orEmpty(),
            linkPreviewsEnabled = prefs[Keys.LINK_PREVIEWS_DISABLED] != true,
            boardCursor = prefs[Keys.BOARD_CURSOR] ?: 0L,
            boardSeenNoteId = prefs[Keys.BOARD_SEEN_NOTE_ID] ?: 0L,
            boardSeenContentSeq = prefs[Keys.BOARD_SEEN_CONTENT_SEQ] ?: 0L,
            mapPreviewsEnabled = prefs[Keys.MAP_PREVIEWS_DISABLED] != true,
            assistantUserId = prefs[Keys.ASSISTANT_USER_ID],
            assistantName = prefs[Keys.ASSISTANT_NAME],
            assistantVision = prefs[Keys.ASSISTANT_VISION] == true,
            assistantImages = prefs[Keys.ASSISTANT_IMAGES] == true,
            familyAiVision = prefs[Keys.FAMILY_AI_VISION] == true,
            familyAiHistory = prefs[Keys.FAMILY_AI_HISTORY] ?: true,
            familyAiHistoryPhotos = prefs[Keys.FAMILY_AI_HISTORY_PHOTOS] == true,
            callsEnabled = prefs[Keys.CALLS_ENABLED] == true,
            videoCallsEnabled = prefs[Keys.VIDEO_CALLS_ENABLED] == true,
            familyRegistrationEnabled = prefs[Keys.FAMILY_REGISTRATION_ENABLED] != false,
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

    override suspend fun setBoardSeenContentSeq(seq: Long) {
        dataStore.edit { prefs ->
            val current = prefs[Keys.BOARD_SEEN_CONTENT_SEQ] ?: 0L
            if (seq > current) prefs[Keys.BOARD_SEEN_CONTENT_SEQ] = seq
        }
    }

    override suspend fun setMapPreviewsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.MAP_PREVIEWS_DISABLED] = !enabled }
    }

    override suspend fun setAssistant(
        userId: Long?,
        displayName: String?,
        vision: Boolean,
        images: Boolean,
    ) {
        dataStore.edit { prefs ->
            if (userId != null && displayName != null) {
                prefs[Keys.ASSISTANT_USER_ID] = userId
                prefs[Keys.ASSISTANT_NAME] = displayName
                prefs[Keys.ASSISTANT_VISION] = vision
                prefs[Keys.ASSISTANT_IMAGES] = images
            } else {
                // Cleared rather than left stale: a server that turned the
                // assistant off must stop offering `@ai` on the next resync.
                // The two capabilities go with it — an absent assistant has
                // none, and a stale `true` would offer a picture surface on
                // a server that has no assistant at all.
                prefs.remove(Keys.ASSISTANT_USER_ID)
                prefs.remove(Keys.ASSISTANT_NAME)
                prefs.remove(Keys.ASSISTANT_VISION)
                prefs.remove(Keys.ASSISTANT_IMAGES)
            }
        }
    }

    override suspend fun setFamilyAiVision(enabled: Boolean) {
        // Unconditional, false included. See the interface.
        dataStore.edit { it[Keys.FAMILY_AI_VISION] = enabled }
    }

    override suspend fun setFamilyAiHistory(enabled: Boolean) {
        dataStore.edit { it[Keys.FAMILY_AI_HISTORY] = enabled }
    }

    override suspend fun setFamilyAiHistoryPhotos(enabled: Boolean) {
        // Unconditional, false included. See the interface.
        dataStore.edit { it[Keys.FAMILY_AI_HISTORY_PHOTOS] = enabled }
    }

    override suspend fun setCallsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.CALLS_ENABLED] = enabled }
    }

    override suspend fun setVideoCallsEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.VIDEO_CALLS_ENABLED] = enabled }
    }

    override suspend fun setFamilyRegistrationEnabled(enabled: Boolean) {
        dataStore.edit { it[Keys.FAMILY_REGISTRATION_ENABLED] = enabled }
    }

    override suspend fun setBlockedUserIds(ids: Collection<Long>) {
        // Unconditional, empty set included. See the interface.
        dataStore.edit { it[Keys.BLOCKED_USER_IDS] = ids.map(Long::toString).toSet() }
    }

    override suspend fun setMaxFamilyMembers(limit: Int?) {
        dataStore.edit { prefs ->
            if (limit == null) prefs.remove(Keys.MAX_FAMILY_MEMBERS) else prefs[Keys.MAX_FAMILY_MEMBERS] = limit
        }
    }

    override suspend fun setSupportContact(contact: String?) {
        dataStore.edit { prefs ->
            if (contact.isNullOrEmpty()) prefs.remove(Keys.SUPPORT_CONTACT) else prefs[Keys.SUPPORT_CONTACT] = contact
        }
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

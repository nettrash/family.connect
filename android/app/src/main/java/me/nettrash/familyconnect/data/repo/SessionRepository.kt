/*
 * SessionRepository.kt
 * Family Connect (Android)
 *
 * Façade over TokenStore + SettingsRepository + AuthApi — the one place
 * that knows what "the session" means:
 *
 *   FamilyStatus  — NO_SERVER / NO_TOKEN are derived (no URL saved / no
 *                   token stored); NONE / PENDING / MEMBER / OWNER are
 *                   reconciled from GET /me and persisted.
 *   snapshot      — the boot read. If no server URL is stored but the
 *                   build compiled one in (DefaultServerUrl seam — the
 *                   Play Store build), it is adopted here: normalized
 *                   and persisted exactly as if the user had typed it,
 *                   no network probe, so store builds boot to AUTH
 *                   instead of SERVER_SETUP. A stored URL always wins.
 *   register/login — store token → GET /me → POST /devices (with the FCM
 *                   token when Firebase is configured, see
 *                   PushTokenRepository) → persist.
 *   logout        — best-effort DELETE /devices/{id} + POST /auth/logout,
 *                   then clearSession.
 *   deleteAccount — POST /me/delete, then clearSession DIRECTLY: the
 *                   session it would log out of is already gone.
 *   clearSession  — token + Room wipe + settings reset EXCEPT serverUrl
 *                   (protocol: on 401 "the client wipes local state,
 *                   keeping the server URL").
 *   refreshMe     — reconciles the FAMILY LIST (docs/protocol.md,
 *                   "Several families") and the current family, and emits
 *                   the session events the nav host / waiting screen react
 *                   to:
 *                     PENDING → NONE  = join request was rejected
 *                     MEMBER/OWNER → NONE = out of the last family
 *                     the current family gone, others left = back to the
 *                     chat list, now the next family's
 *                   A family that disappears takes only ITS data with it
 *                   (FamilyLocalData); the last one takes everything.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Repo/SessionRepository.swift
 */

package me.nettrash.familyconnect.data.repo

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.LocalDataWiper
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.dto.AuthResponse
import me.nettrash.familyconnect.data.net.dto.FamilyDto
import me.nettrash.familyconnect.data.net.dto.MeResponse
import me.nettrash.familyconnect.data.net.dto.PendingJoinRequestDto
import me.nettrash.familyconnect.data.push.PushLanguageProvider
import me.nettrash.familyconnect.data.push.PushTokenProvider
import me.nettrash.familyconnect.data.push.PushTokenRepository
import me.nettrash.familyconnect.data.settings.DefaultServerUrl
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.data.settings.TokenStore
import me.nettrash.familyconnect.data.settings.mirrorFamilySwitches
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.di.UnauthorizedEvents
import javax.inject.Inject
import javax.inject.Singleton

enum class FamilyStatus {
    /** No server URL saved yet — first-run. */
    NO_SERVER,

    /** Server known, but no session token — login/register. */
    NO_TOKEN,

    /** Authenticated, not in a family and no pending request. */
    NONE,

    /** Join request submitted, awaiting the owner's approval. */
    PENDING,

    MEMBER,
    OWNER,
}

sealed interface SessionEvent {
    /** The session died server-side (401) — back to login. */
    data object Expired : SessionEvent

    /** The owner removed us (or the family dissolved) — back to the gate. */
    data object RemovedFromFamily : SessionEvent

    /** Was PENDING, now nothing — the join request was declined. */
    data object JoinRequestRejected : SessionEvent

    /**
     * The family the app was SHOWING is gone — left, removed, or deleted —
     * and the account is still in others (docs/protocol.md, "Several
     * families"). The app has already switched to the first remaining one;
     * screens of the old family make way for its chat list.
     */
    data object CurrentFamilyGone : SessionEvent
}

data class SessionSnapshot(
    val status: FamilyStatus,
    val serverUrl: String?,
    val token: String?,
    val myUserId: Long?,
    val myUsername: String?,
    val myDisplayName: String?,
    val familyName: String?,
    /** My profile-picture version; 0 = none. */
    val myAvatarVersion: Long = 0,
) {
    /** The socket may connect: authenticated member of a family. */
    val canChat: Boolean
        get() = token != null && serverUrl != null &&
            (status == FamilyStatus.MEMBER || status == FamilyStatus.OWNER)

    val isOwner: Boolean get() = status == FamilyStatus.OWNER
}

@Singleton
class SessionRepository @Inject constructor(
    private val authApi: AuthApi,
    private val tokenStore: TokenStore,
    private val settings: SettingsRepository,
    private val wiper: LocalDataWiper,
    @param:UnauthorizedEvents private val unauthorizedEvents: SharedFlow<Unit>,
    @param:AppScope private val scope: CoroutineScope,
    // Kotlin default = "no default server", so tests that don't care never
    // mention it. Dagger ignores default arguments — production always
    // injects the AppModule binding backed by BuildConfig.DEFAULT_SERVER_URL.
    private val defaultServerUrl: DefaultServerUrl = DefaultServerUrl { null },
    // Same trick: the default builds a real PushTokenRepository over this
    // constructor's own collaborators with a "no Firebase" token provider,
    // so existing tests keep exercising the device-registration flow
    // unchanged. Production always injects the Hilt singleton (whose
    // provider actually asks Firebase — see AppModule).
    private val pushTokenRepository: PushTokenRepository =
        PushTokenRepository(authApi, settings, tokenStore, PushTokenProvider { null }, PushLanguageProvider { "en" }),
    // Same trick again: tests that never race a late write against a wipe
    // get a private epoch; production shares the singleton with the
    // repositories whose writes it guards (TranscriptRepository).
    private val epoch: SessionEpoch = SessionEpoch(),
    // Same trick once more: a test that never holds two families gets the
    // no-op; production drops one family's rows through Room.
    private val familyData: FamilyLocalData = FamilyLocalData.NONE,
) {

    // Bumped on every token save/clear so sessionFlow re-emits — the
    // TokenStore itself isn't observable.
    private val tokenVersion = MutableStateFlow(0)

    private val _sessionEvents = MutableSharedFlow<SessionEvent>(extraBufferCapacity = 4)
    val sessionEvents: SharedFlow<SessionEvent> = _sessionEvents

    /** Live session view — ChatSocketManager keys its connect policy off this. */
    val sessionFlow: Flow<SessionSnapshot> =
        combine(settings.state, tokenVersion) { state, _ -> snapshotFrom(state) }

    init {
        // Any authenticated 401, from any endpoint: wipe once, reroute once.
        scope.launch {
            unauthorizedEvents.collect {
                clearSession()
                _sessionEvents.emit(SessionEvent.Expired)
            }
        }
    }

    /**
     * The SOCKET says the session is gone — closed with `4401`, or an
     * upgrade refused with `401` (docs/protocol.md, "WebSocket protocol").
     *
     * It has to land in the same place a REST 401 does. Without this the
     * socket merely reconnected forever behind a "Connecting…" banner: a
     * device whose account was deleted, or whose session an owner's
     * password reset revoked, sat on the chat list looking signed in
     * until something happened to make a REST call.
     *
     * Guarded on a token still being stored so the reconnect loop's
     * repeated failures reroute once rather than once per attempt.
     */
    fun onSessionExpired() {
        scope.launch {
            if (tokenStore.load() == null) return@launch
            clearSession()
            _sessionEvents.emit(SessionEvent.Expired)
        }
    }

    /**
     * Boot read. When nothing is stored yet and the build compiled a
     * default server in, adopt it through the same normalize-and-persist
     * path a typed URL takes — from then on it IS the stored URL (kept by
     * clearSession, overridable from server setup). No network probe:
     * boot must never block on the network.
     */
    suspend fun snapshot(): SessionSnapshot {
        if (settings.state.first().serverUrl == null) {
            defaultServerUrl.get()
                ?.let(ServerUrlNormalizer::normalize)
                ?.let { settings.setServerUrl(it) }
        }
        return snapshotFrom(settings.state.first())
    }

    private fun snapshotFrom(state: SettingsState): SessionSnapshot {
        val token = tokenStore.load()
        val status = when {
            state.serverUrl == null -> FamilyStatus.NO_SERVER
            token == null -> FamilyStatus.NO_TOKEN
            else -> state.familyStatus
        }
        return SessionSnapshot(
            status = status,
            serverUrl = state.serverUrl,
            token = token,
            myUserId = state.myUserId,
            myUsername = state.myUsername,
            myDisplayName = state.myDisplayName,
            familyName = state.familyName,
            myAvatarVersion = state.myAvatarVersion,
        )
    }

    // -- Auth ------------------------------------------------------------------

    suspend fun register(
        username: String,
        displayName: String,
        password: String,
    ): ApiResult<SessionSnapshot> =
        completeAuth(authApi.register(username, displayName, password))

    suspend fun login(username: String, password: String): ApiResult<SessionSnapshot> =
        completeAuth(authApi.login(username, password))

    private suspend fun completeAuth(result: ApiResult<AuthResponse>): ApiResult<SessionSnapshot> =
        when (result) {
            is ApiResult.Ok -> {
                tokenStore.save(result.value.token)
                tokenVersion.update { it + 1 }
                val user = result.value.user
                settings.setProfile(user.id, user.username, user.displayName, user.avatarVersion)
                refreshMe()
                // Best-effort: a failed device registration must not block
                // login — PushTokenRepository retries on the next resync.
                pushTokenRepository.registerCurrentToken()
                ApiResult.Ok(snapshot())
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    suspend fun logout() {
        // Best-effort: remove the push device row FIRST (it needs the
        // session that /auth/logout is about to revoke) so a logged-out
        // phone stops getting this account's notifications. clearSession's
        // resetKeepingServerUrl drops the stored device id but keeps the
        // cached FCM token — the next login re-registers immediately.
        settings.state.first().pushDeviceId?.let { authApi.deleteDevice(it) }
        // Best-effort revoke; local teardown happens regardless.
        authApi.logout()
        clearSession()
    }

    /**
     * Delete this account, permanently (protocol.md, "Deleting an
     * account") — App Store guideline 5.1.1(v)'s in-app, self-service
     * deletion.
     *
     * The teardown deliberately does NOT go through [logout]: that calls
     * `DELETE /devices/{id}` and `POST /auth/logout`, and by the time the
     * 204 comes back this session no longer exists, so both would answer
     * 401 — which would fire the unauthorized broadcast and race a second
     * teardown against this one. The server has already deleted every
     * session and every device row of this account; there is nothing left
     * to be polite about, so this goes straight to [clearSession], which
     * is the app's "wipe local state and return to sign-in" primitive:
     * token, the whole Room cache (messages, chats, roster, board notes,
     * and with them every per-chat reaction and edit cursor) and every
     * stored preference except the server URL and the two device-scoped
     * privacy switches — the board cursor included.
     *
     * A failure changes nothing locally: a wrong password comes back as
     * `invalid_credentials` (401) and the account is still there.
     */
    suspend fun deleteAccount(password: String): ApiResult<Unit> {
        val result = authApi.deleteAccount(password)
        if (result is ApiResult.Ok) clearSession()
        return result
    }

    suspend fun clearSession() {
        tokenStore.clear()
        tokenVersion.update { it + 1 }
        wipe()
        settings.resetKeepingServerUrl()
    }

    /**
     * Every wipe of local data, with the [SessionEpoch] advanced FIRST: a
     * request still out — a transcript, above all — then finds its session
     * gone and drops its answer instead of writing it into the emptied
     * database for whoever signs in next.
     */
    private suspend fun wipe() {
        epoch.advance()
        wiper.wipeAll()
    }

    // -- Membership reconciliation ------------------------------------------------

    suspend fun refreshMe(): ApiResult<SessionSnapshot> {
        val previous = settings.state.first().familyStatus
        return when (val result = authApi.me()) {
            is ApiResult.Ok -> {
                val me = result.value
                settings.setProfile(me.user.id, me.user.username, me.user.displayName, me.user.avatarVersion)
                settings.setCallsEnabled(me.callsEnabled)
                settings.setVideoCallsEnabled(me.videoCallsEnabled)
                settings.setFamilyRegistrationEnabled(me.familyRegistrationEnabled)
                settings.setFamilylessAccountTtlDays(me.familylessAccountTtlDays)
                // The operator's half of the daily greeting — the family's
                // own switch is drawn against it (protocol.md, "The daily
                // greeting").
                settings.setGreetingsEnabled(me.greetingsEnabled)
                // Whether this member has agreed that their words may go
                // to the model. Unconditional, null included: a
                // withdrawal from another device arrives exactly here
                // (protocol.md, "Consenting to the assistant").
                settings.setAssistantConsentAt(me.assistantConsentAt)
                // …and the lookup consent beside it, by the same rule
                // (protocol.md, "Consenting to the assistant", amended
                // 2026-10-03). Absent from an older server, so null.
                settings.setAssistantLookupConsentAt(me.assistantLookupConsentAt)
                // The AUTHORITATIVE apply. `/me` is step 1 of the resync,
                // it is the only one on the login path, and it is the only
                // one a caller with NO family reaches at all — a block is a
                // pair and not a membership, so somebody outside a family
                // still holds blocks that `GET /families/mine` would answer
                // `not_in_family` about (docs/protocol.md, `GET /me`).
                //
                // Before the status/teardown logic below, which is where
                // `wipeAll()` lives.
                settings.setBlockedUserIds(me.blockedUserIds)
                settings.setMaxFamilyMembers(me.maxFamilyMembers)
                settings.setSupportContact(me.supportContact)
                val before = settings.state.first()
                val next = applyFamilies(me, before)
                if (previous == FamilyStatus.PENDING && next == FamilyStatus.NONE) {
                    // Neither family nor pending request: the request was
                    // rejected (protocol GET /me note). The waiting screen
                    // shows "declined" off this event.
                    _sessionEvents.tryEmit(SessionEvent.JoinRequestRejected)
                }
                ApiResult.Ok(snapshot())
            }
            is ApiResult.HttpError -> result // 401 handled via unauthorizedEvents
            is ApiResult.NetworkError -> result
        }
    }

    /**
     * Store what `/me` said about families and reconcile everything that
     * hangs off it; answers the new status.
     *
     * A server that predates several families sends no `families`, and its
     * one `family` (with `role`) becomes a one-entry list — so everything
     * below runs the same on both, and only [SettingsState.multiFamilyServer]
     * remembers the difference (request paths keep their old form there).
     */
    private suspend fun applyFamilies(me: MeResponse, before: SettingsState): FamilyStatus {
        val families: List<FamilyDto> = me.families
            ?: listOfNotNull(me.family?.let { it.copy(role = it.role ?: me.role) })
        val pending: List<PendingJoinRequestDto> = me.pendingJoinRequests
            ?: listOfNotNull(me.pendingJoinRequest)
        // Rows cached before several families are the home family's — the
        // only one an app from before saw. Before anything is filed under a
        // real id, so a roster refresh never meets an unclaimed twin.
        families.firstOrNull()?.let { home ->
            familyData.claimLegacy(home.id)
            settings.claimLegacyFamilyState(home.id)
        }
        settings.setFamilies(
            families = families,
            pendingJoinRequests = pending,
            invitations = me.invitations,
            memberFamiliesEnabled = me.memberFamiliesEnabled,
            maxFamiliesPerAccount = me.maxFamiliesPerAccount ?: 0,
            multiFamilyServer = me.families != null,
        )
        val current = FamilyList.current(families, before.currentFamilyId)
        settings.setCurrentFamilyId(current)
        val next = mirrorCurrent(families, current, pending)

        val remaining = families.mapTo(HashSet()) { it.id }
        val gone = before.families.map { it.id }.filter { it !in remaining }
        val wasInFamily = before.familyStatus == FamilyStatus.MEMBER || before.familyStatus == FamilyStatus.OWNER
        if (families.isEmpty() && (wasInFamily || before.families.isNotEmpty())) {
            // Out of the LAST family while we weren't looking — the chats
            // aren't ours to show any more.
            wipe()
            forgetFamilies(gone)
            _sessionEvents.tryEmit(SessionEvent.RemovedFromFamily)
        } else if (gone.isNotEmpty()) {
            // Out of SOME: each takes only its own data.
            for (familyId in gone) familyData.dropFamily(familyId, remaining)
            forgetFamilies(gone)
            if (before.currentFamilyId in gone) _sessionEvents.tryEmit(SessionEvent.CurrentFamilyGone)
        }
        return next
    }

    /**
     * The current family's status, name and switches into the single values
     * every screen reads "the family" from. Answers the status.
     */
    private suspend fun mirrorCurrent(
        families: List<FamilyDto>,
        current: Long?,
        pending: List<PendingJoinRequestDto>,
    ): FamilyStatus {
        val family = families.firstOrNull { it.id == current }
        val status = FamilyList.status(families, current, pending)
        settings.setFamilyStatus(status)
        settings.setFamilyName(family?.name ?: pending.firstOrNull()?.familyName)
        // The owner's switches ride on every Family object, `/me`'s
        // included (docs/protocol.md, "Transcripts on request"). Only from a
        // family that is there: without one the teardown takes them.
        family?.let { settings.mirrorFamilySwitches(it) }
        return status
    }

    /**
     * The cursors and badge marks of families this account is out of. With
     * none left, also the single values from before several families, which
     * must never be claimed for a family joined later (pack and board seqs
     * are server-wide).
     */
    private suspend fun forgetFamilies(familyIds: Collection<Long>) {
        for (familyId in familyIds) settings.forgetFamily(familyId)
        if (settings.state.first().families.isEmpty()) {
            settings.claimLegacyFamilyState(0L)
            settings.forgetFamily(0L)
        }
    }

    /**
     * Show [familyId] — the switcher's tap, a push's family, a family just
     * joined. Nothing when it is not one of this account's families.
     */
    suspend fun switchFamily(familyId: Long) {
        val state = settings.state.first()
        if (state.families.none { it.id == familyId }) return
        settings.setCurrentFamilyId(familyId)
        mirrorCurrent(state.families, familyId, state.pendingJoinRequests)
    }

    /**
     * A family this account just came into — created here, or an invitation
     * accepted — onto the list and shown, before any `/me` has said so. Its
     * join request and its invitation, if any, are answered by it.
     */
    suspend fun addFamily(family: FamilyDto) {
        val state = settings.state.first()
        val families = state.families.filter { it.id != family.id } + family
        val pending = state.pendingJoinRequests.filter { it.familyId != family.id }
        settings.setFamilies(
            families = families,
            pendingJoinRequests = pending,
            invitations = state.invitations.filter { it.familyId != family.id },
            memberFamiliesEnabled = state.memberFamiliesEnabled,
            maxFamiliesPerAccount = state.maxFamiliesPerAccount,
            multiFamilyServer = state.multiFamilyServer,
        )
        settings.setCurrentFamilyId(family.id)
        mirrorCurrent(families, family.id, pending)
    }

    /**
     * Re-mirror the current family after its entry changed — a roster read
     * or an owner's PATCH answered with a newer Family.
     */
    suspend fun updateFamily(family: FamilyDto) {
        val state = settings.state.first()
        val held = state.families.firstOrNull { it.id == family.id } ?: return
        // A Family read anywhere but `/me` carries no role: keep the one held.
        val families = state.families.map { if (it.id == family.id) family.copy(role = family.role ?: held.role) else it }
        settings.setFamilies(
            families = families,
            pendingJoinRequests = state.pendingJoinRequests,
            invitations = state.invitations,
            memberFamiliesEnabled = state.memberFamiliesEnabled,
            maxFamiliesPerAccount = state.maxFamiliesPerAccount,
            multiFamilyServer = state.multiFamilyServer,
        )
        if (state.currentFamilyId == family.id) mirrorCurrent(families, family.id, state.pendingJoinRequests)
    }

    /**
     * The owner of [familyId] changed (`family_owner`): the role on that
     * family's entry follows, and the status with it when it is the one
     * shown.
     */
    suspend fun setFamilyRole(familyId: Long, role: String) {
        val state = settings.state.first()
        val held = state.families.firstOrNull { it.id == familyId } ?: return
        if (held.role == role) return
        updateFamily(held.copy(role = role))
    }

    /** Live `member_left` for *my* user id — FamilyRepository calls this. */
    fun onRemovedFromFamily(familyId: Long) {
        scope.launch { leftFamily(familyId) }
    }

    /**
     * This account is out of [familyId] — it left, or was removed. Only that
     * family's chats, board, pack, roster and cursors go (docs/protocol.md,
     * "Several families"); if it was the one shown, the first remaining
     * family is shown instead. Out of the LAST family, everything goes and
     * the app returns to the gate, exactly as leaving "the family" always
     * did.
     */
    suspend fun leftFamily(familyId: Long) {
        val state = settings.state.first()
        // Already done: our own leave tears down before the `member_left`
        // that confirms it arrives. An install that has not read `/me` since
        // several families holds no list yet, and its one family is this one.
        val legacyMember = state.families.isEmpty() &&
            (state.familyStatus == FamilyStatus.MEMBER || state.familyStatus == FamilyStatus.OWNER)
        if (state.families.none { it.id == familyId } && !legacyMember) return
        val remaining = state.families.filter { it.id != familyId }
        if (remaining.isEmpty()) {
            wipe()
            settings.setFamilies(
                families = emptyList(),
                pendingJoinRequests = state.pendingJoinRequests,
                invitations = state.invitations,
                memberFamiliesEnabled = state.memberFamiliesEnabled,
                maxFamiliesPerAccount = state.maxFamiliesPerAccount,
                multiFamilyServer = state.multiFamilyServer,
            )
            forgetFamilies(state.families.map { it.id } + familyId)
            settings.setCurrentFamilyId(null)
            settings.setFamilyStatus(FamilyList.status(emptyList(), null, state.pendingJoinRequests))
            settings.setFamilyName(null)
            _sessionEvents.emit(SessionEvent.RemovedFromFamily)
            return
        }
        familyData.dropFamily(familyId, remaining.mapTo(HashSet()) { it.id })
        forgetFamilies(listOf(familyId))
        if (state.currentFamilyId == familyId || state.currentFamilyId == null) {
            val next = remaining.first().id
            settings.setCurrentFamilyId(next)
            mirrorCurrent(remaining, next, state.pendingJoinRequests)
            _sessionEvents.emit(SessionEvent.CurrentFamilyGone)
        }
    }
}

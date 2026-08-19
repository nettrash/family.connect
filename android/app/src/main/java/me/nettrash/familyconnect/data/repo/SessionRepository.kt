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
 *   register/login — store token → GET /me → POST /devices → persist.
 *   logout        — best-effort POST /auth/logout, then clearSession.
 *   clearSession  — token + Room wipe + settings reset EXCEPT serverUrl
 *                   (protocol: on 401 "the client wipes local state,
 *                   keeping the server URL").
 *   refreshMe     — reconciles status and emits the session events the
 *                   nav host / waiting screen react to:
 *                     PENDING → NONE  = join request was rejected
 *                     MEMBER/OWNER → NONE = removed from the family
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
import me.nettrash.familyconnect.data.settings.DefaultServerUrl
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.data.settings.SettingsState
import me.nettrash.familyconnect.data.settings.TokenStore
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
}

data class SessionSnapshot(
    val status: FamilyStatus,
    val serverUrl: String?,
    val token: String?,
    val myUserId: Long?,
    val myUsername: String?,
    val myDisplayName: String?,
    val familyName: String?,
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
                settings.setProfile(user.id, user.username, user.displayName)
                refreshMe()
                // Best-effort: a failed device registration must not block
                // login — v1 has no push delivery anyway.
                authApi.registerDevice()
                ApiResult.Ok(snapshot())
            }
            is ApiResult.HttpError -> result
            is ApiResult.NetworkError -> result
        }

    suspend fun logout() {
        // Best-effort revoke; local teardown happens regardless.
        authApi.logout()
        clearSession()
    }

    suspend fun clearSession() {
        tokenStore.clear()
        tokenVersion.update { it + 1 }
        wiper.wipeAll()
        settings.resetKeepingServerUrl()
    }

    // -- Membership reconciliation ------------------------------------------------

    suspend fun refreshMe(): ApiResult<SessionSnapshot> {
        val previous = settings.state.first().familyStatus
        return when (val result = authApi.me()) {
            is ApiResult.Ok -> {
                val me = result.value
                settings.setProfile(me.user.id, me.user.username, me.user.displayName)
                val next = when {
                    me.family != null ->
                        if (me.role == "owner") FamilyStatus.OWNER else FamilyStatus.MEMBER
                    me.pendingJoinRequest != null -> FamilyStatus.PENDING
                    else -> FamilyStatus.NONE
                }
                settings.setFamilyStatus(next)
                settings.setFamilyName(me.family?.name ?: me.pendingJoinRequest?.familyName)
                if (previous == FamilyStatus.PENDING && next == FamilyStatus.NONE) {
                    // Neither family nor pending request: the request was
                    // rejected (protocol GET /me note). The waiting screen
                    // shows "declined" off this event.
                    _sessionEvents.tryEmit(SessionEvent.JoinRequestRejected)
                }
                if ((previous == FamilyStatus.MEMBER || previous == FamilyStatus.OWNER) &&
                    next == FamilyStatus.NONE
                ) {
                    // Removed while we weren't looking — the chats aren't
                    // ours to show any more.
                    wiper.wipeAll()
                    _sessionEvents.tryEmit(SessionEvent.RemovedFromFamily)
                }
                ApiResult.Ok(snapshot())
            }
            is ApiResult.HttpError -> result // 401 handled via unauthorizedEvents
            is ApiResult.NetworkError -> result
        }
    }

    /** Live `member_left` for *my* user id — FamilyRepository calls this. */
    fun onRemovedFromFamily() {
        scope.launch {
            wiper.wipeAll()
            settings.setFamilyStatus(FamilyStatus.NONE)
            settings.setFamilyName(null)
            _sessionEvents.emit(SessionEvent.RemovedFromFamily)
        }
    }
}

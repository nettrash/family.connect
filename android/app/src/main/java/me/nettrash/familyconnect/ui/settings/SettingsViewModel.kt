/*
 * SettingsViewModel.kt
 * Family Connect (Android)
 *
 * Profile (picture + birthday) + family block + the three destructive
 * actions. Leave-family
 * surfaces the protocol's `owner_cannot_leave` 409 as a human message
 * (an owner with members must hand the family over — v1 has no
 * transfer, so: remove everyone or keep it). Logout is best-effort
 * server-side and unconditional locally. Account deletion is neither
 * best-effort nor reversible: the server erases the account and every
 * session it has, and this side wipes and returns to sign-in.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Settings/SettingsViewModel.swift
 */

package me.nettrash.familyconnect.ui.settings

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import me.nettrash.familyconnect.data.net.ApiClient
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.AuthApi
import me.nettrash.familyconnect.data.net.AvatarApi
import me.nettrash.familyconnect.data.net.dto.BirthdayDto
import android.net.Uri
import me.nettrash.familyconnect.data.repo.AvatarImage
import me.nettrash.familyconnect.data.repo.AvatarSource
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel @Inject constructor(
    /**
     * The application context, for `getString` only.
     *
     * A ViewModel holding a Context is usually a smell; the APPLICATION
     * context is the exception — it outlives every screen, so there is
     * nothing to leak. The alternative, carrying @StringRes ids through
     * every state field, spreads resource plumbing across code whose job
     * is state. The trade-off: a message is resolved when it is produced
     * rather than when it is drawn, so one already on screen keeps its
     * language if the system locale changes underneath it — and Android
     * recreates the activity then anyway.
     */
    @param:ApplicationContext private val appContext: Context,
    private val sessionRepository: SessionRepository,
    private val authApi: AuthApi,
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
    private val avatarApi: AvatarApi,
    private val avatarSource: AvatarSource,
) : ViewModel() {

    data class UiState(
        val displayName: String? = null,
        val username: String? = null,
        val userId: Long? = null,
        val serverUrl: String? = null,
        val familyName: String? = null,
        val isOwner: Boolean = false,
        val inviteCode: String? = null,
        val joinPolicy: String? = null,
        val error: String? = null,
        val loggedOut: Boolean = false,
        /** Whether this device may fetch link previews. */
        val linkPreviewsEnabled: Boolean = true,
        val mapPreviewsEnabled: Boolean = true,
        /** My profile-picture version; 0 = none, and the button says "Add". */
        val avatarVersion: Long = 0,
        /**
         * My own birthday — a day and a month, never a year, and null
         * until somebody sets one.
         *
         * Read out of the ROSTER rather than from a call of its own: my
         * own Member row is in `GET /families/mine` like everyone else's,
         * and that request is already made here. The write paths keep it
         * current from their own responses, since a birthday change
         * raises no frame (protocol.md, "Birthdays").
         */
        val birthday: BirthdayDto? = null,
        /** True while a settings write is in flight. */
        val busy: Boolean = false,
        /**
         * Who would inherit if the owner left right now — a PREDICTION,
         * and only ever read straight after a fresh `GET /families/mine`.
         * Null for a plain member, and null for an owner who is the LAST
         * member, which is a different dialog: leaving deletes the family
         * (docs/protocol.md, `GET /families/mine`).
         */
        val nextOwnerName: String? = null,
        /** Who the family actually went to, once it has gone. */
        val handedOverTo: String? = null,
        val uploadingAvatar: Boolean = false,
        val avatarError: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    init {
        load()
        // The flag lives in the store, so the switch follows it rather
        // than holding its own copy.
        viewModelScope.launch {
            settings.state.collect { stored ->
                _state.update {
                    it.copy(
                        linkPreviewsEnabled = stored.linkPreviewsEnabled,
                        mapPreviewsEnabled = stored.mapPreviewsEnabled,
                        avatarVersion = stored.myAvatarVersion,
                        // Followed rather than read once at [load]: a
                        // `family_owner` frame can hand this device the
                        // family while this screen is open (protocol.md,
                        // "Deleting an account"), and the owner-only
                        // entries have to appear then, not at the next
                        // visit.
                        isOwner = stored.familyStatus == FamilyStatus.OWNER,
                    )
                }
            }
        }
    }

    fun setLinkPreviewsEnabled(enabled: Boolean) {
        viewModelScope.launch { settings.setLinkPreviewsEnabled(enabled) }
    }

    fun setMapPreviewsEnabled(enabled: Boolean) {
        viewModelScope.launch { settings.setMapPreviewsEnabled(enabled) }
    }

    fun load() {
        viewModelScope.launch {
            val snapshot = sessionRepository.snapshot()
            _state.update {
                it.copy(
                    displayName = snapshot.myDisplayName,
                    username = snapshot.myUsername,
                    userId = snapshot.myUserId,
                    serverUrl = snapshot.serverUrl,
                    familyName = snapshot.familyName,
                    isOwner = snapshot.isOwner,
                )
            }
            if (snapshot.status == FamilyStatus.MEMBER || snapshot.status == FamilyStatus.OWNER) {
                familyRepository.refreshMine().okOrNull()?.let { mine ->
                    val myRow = mine.members.firstOrNull { it.id == snapshot.myUserId }
                    // Resolved against THIS response's roster rather than
                    // the stored one: the prediction and the names that
                    // explain it have to come from the same read, or the
                    // dialog can name somebody who left between them.
                    val successor = mine.nextOwnerUserId?.let { id ->
                        mine.members.firstOrNull { it.id == id }?.displayName
                    }
                    _state.update {
                        it.copy(
                            familyName = mine.family.name,
                            // Owner-only on the wire — null for members.
                            inviteCode = mine.family.inviteCode,
                            joinPolicy = mine.family.joinPolicy,
                            birthday = myRow?.birthday,
                            nextOwnerName = successor,
                        )
                    }
                }
            }
        }
    }

    /**
     * Read, downscale and upload the picked image. The server stores what
     * it is given and never transcodes (docs/protocol.md), so producing
     * something small and square is this side's job — and neither the
     * read nor the decode runs on the main thread, because the source is
     * a full-size phone photo that may still be in the cloud.
     *
     * The whole pipeline lives on viewModelScope, including the read: a
     * Uri from a cloud photo library can take seconds to materialize, and
     * the busy flag has to outlive a composable that the user scrolls or
     * navigates away from while it does.
     */
    fun setAvatar(uri: Uri) {
        if (_state.value.uploadingAvatar) return
        viewModelScope.launch {
            _state.update { it.copy(uploadingAvatar = true, avatarError = null) }
            val source = avatarSource.read(uri)
            if (source == null) {
                _state.update {
                    it.copy(uploadingAvatar = false, avatarError = appContext.getString(R.string.e_image_unreadable))
                }
                return@launch
            }
            val jpeg = withContext(Dispatchers.Default) { AvatarImage.squareJpeg(source) }
            if (jpeg == null) {
                _state.update {
                    it.copy(uploadingAvatar = false, avatarError = appContext.getString(R.string.e_image_unreadable))
                }
                return@launch
            }
            when (val result = avatarApi.upload(jpeg)) {
                is ApiResult.Ok -> {
                    settings.setMyAvatarVersion(result.value.user.avatarVersion)
                    // The roster carries my own row too — refresh so the
                    // member lists show the new picture without waiting
                    // for the next resync.
                    familyRepository.refreshMine()
                    _state.update { it.copy(uploadingAvatar = false) }
                }
                is ApiResult.HttpError -> _state.update {
                    it.copy(uploadingAvatar = false, avatarError = uploadFailure(result))
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(uploadingAvatar = false, avatarError = appContext.getString(R.string.e_unreachable))
                }
            }
        }
    }

    fun removeAvatar() {
        if (_state.value.uploadingAvatar) return
        viewModelScope.launch {
            _state.update { it.copy(uploadingAvatar = true, avatarError = null) }
            when (val result = avatarApi.delete()) {
                is ApiResult.Ok -> {
                    settings.setMyAvatarVersion(0)
                    familyRepository.refreshMine()
                    _state.update { it.copy(uploadingAvatar = false) }
                }
                is ApiResult.HttpError -> _state.update {
                    it.copy(uploadingAvatar = false, avatarError = uploadFailure(result))
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(uploadingAvatar = false, avatarError = appContext.getString(R.string.e_unreachable))
                }
            }
        }
    }

    /**
     * One sentence per way this can actually fail. A single "Couldn't
     * upload the photo." for all of them turns a five-second diagnosis
     * ("your server predates profile pictures") into a guessing game.
     * Same wording as iOS.
     */
    private fun uploadFailure(result: ApiResult.HttpError): String = when {
        result.code == "avatar_too_large" -> "That photo is too large."
        result.code == "invalid_image" -> "That file isn't a photo we can use."
        // The endpoint itself is missing: a server built before profile
        // pictures existed. The handler answers 404 only for GETs of
        // someone else's picture, so on PUT/DELETE this is always the
        // route being absent.
        result.status == 404 -> "This server doesn't support profile pictures yet — it needs updating."
        result.status >= 500 -> "The server had a problem (${result.status}). Try again."
        else -> result.message ?: "Couldn't upload the photo."
    }

    fun dismissAvatarError() {
        _state.update { it.copy(avatarError = null) }
    }

    /**
     * My own birthday. Day and month, no year — [BirthdayDto] says why.
     *
     * A `validation` refusal from the server is shown rather than
     * swallowed: the picker cannot offer 31 April, but the server owns
     * the rule and a silent failure would leave this screen claiming a
     * birthday nobody stored.
     */
    fun setBirthday(month: Int, day: Int, onSuccess: () -> Unit = {}) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setMyBirthday(month, day)) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(birthday = result.value) }
                    onSuccess()
                }
                is ApiResult.HttpError -> _state.update {
                    it.copy(error = result.message ?: appContext.getString(R.string.e_birthday_failed))
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(error = appContext.getString(R.string.e_unreachable))
                }
            }
            _state.update { it.copy(busy = false) }
        }
    }

    fun clearBirthday(onSuccess: () -> Unit = {}) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.clearMyBirthday()) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(birthday = null) }
                    onSuccess()
                }
                is ApiResult.HttpError -> _state.update {
                    it.copy(error = result.message ?: appContext.getString(R.string.e_birthday_failed))
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(error = appContext.getString(R.string.e_unreachable))
                }
            }
            _state.update { it.copy(busy = false) }
        }
    }

    /**
     * Dismiss the hand-off report. Nothing else to undo: the family state
     * is already down and the nav host has already rerouted — this only
     * takes the sentence off the screen.
     */
    fun acknowledgeHandOver() {
        _state.update { it.copy(handedOverTo = null) }
    }

    fun leaveFamily() {
        viewModelScope.launch {
            when (val result = familyRepository.leave()) {
                // An owner who leaves hands the family on and is told to
                // whom; everybody else simply leaves. Either way
                // SessionRepository has emitted RemovedFromFamily and the
                // nav host reroutes, so the name is shown on the way out.
                is ApiResult.Ok -> _state.update { it.copy(handedOverTo = result.value) }
                // No `owner_cannot_leave` branch: that error is RETIRED
                // and no endpoint raises it any more. The app used to
                // explain a rule the server had stopped enforcing
                // (docs/protocol.md, `POST /families/leave`).
                is ApiResult.HttpError -> _state.update {
                    it.copy(error = result.message ?: appContext.getString(R.string.e_leave_failed))
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    /**
     * Change my own password. The current one is required — a live session
     * is not proof of knowing it (protocol.md, "Auth"). Every OTHER device
     * of mine is signed out server-side; this one keeps its session.
     */
    fun changePassword(current: String, new: String, onSuccess: () -> Unit = {}) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = authApi.changePassword(current, new)) {
                is ApiResult.Ok -> onSuccess()
                is ApiResult.HttpError ->
                    _state.update {
                        it.copy(
                            error = if (result.status == 401) {
                                // A 401 here means the CURRENT password was
                                // wrong, not that the session died —
                                // treating it as a dead session would sign
                                // the user out over a typo.
                                appContext.getString(R.string.e_wrong_current_password)
                            } else {
                                result.message ?: appContext.getString(R.string.e_change_password_failed)
                            },
                        )
                    }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = appContext.getString(R.string.e_unreachable)) }
            }
            _state.update { it.copy(busy = false) }
        }
    }

    fun logout() {
        viewModelScope.launch {
            sessionRepository.logout()
            _state.update { it.copy(loggedOut = true) }
        }
    }

    /**
     * Delete this account — permanently, immediately, and from inside the
     * app, which is what App Store guideline 5.1.1(v) requires and what
     * protocol.md's "Deleting an account" describes.
     *
     * The password is asked for by the dialog and proved by the server;
     * a wrong one comes back as `invalid_credentials` (401) and must read
     * as a typo rather than a dead session — the account is still there.
     *
     * On success the same [UiState.loggedOut] flag the logout path raises
     * takes the app back to sign-in, because that is exactly what has
     * happened: the local state is already wiped (see
     * SessionRepository.deleteAccount) and there is no session left to
     * log out of.
     */
    fun deleteAccount(password: String) {
        if (_state.value.busy) return
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = sessionRepository.deleteAccount(password)) {
                is ApiResult.Ok -> _state.update { it.copy(busy = false, loggedOut = true) }
                is ApiResult.HttpError -> _state.update {
                    it.copy(
                        busy = false,
                        // Matched on the CODE, not the status: 401 also
                        // carries `unauthorized`, and that one is a dead
                        // session already rerouting this screen to
                        // sign-in — telling somebody their password was
                        // wrong as the app signs them out explains
                        // nothing.
                        error = if (result.code == ApiClient.INVALID_CREDENTIALS) {
                            appContext.getString(R.string.e_wrong_password)
                        } else {
                            result.message ?: appContext.getString(R.string.e_delete_account_failed)
                        },
                    )
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(busy = false, error = appContext.getString(R.string.e_unreachable))
                }
            }
        }
    }

    fun dismissError() {
        _state.update { it.copy(error = null) }
    }
}

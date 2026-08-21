/*
 * SettingsViewModel.kt
 * Family Connect (Android)
 *
 * Profile + family block + the two destructive actions. Leave-family
 * surfaces the protocol's `owner_cannot_leave` 409 as a human message
 * (an owner with members must hand the family over — v1 has no
 * transfer, so: remove everyone or keep it). Logout is best-effort
 * server-side and unconditional locally.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Settings/SettingsViewModel.swift
 */

package me.nettrash.familyconnect.ui.settings

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class SettingsViewModel @Inject constructor(
    private val sessionRepository: SessionRepository,
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
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
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    init {
        load()
        // The flag lives in the store, so the switch follows it rather
        // than holding its own copy.
        viewModelScope.launch {
            settings.state.collect { stored ->
                _state.update { it.copy(linkPreviewsEnabled = stored.linkPreviewsEnabled) }
            }
        }
    }

    fun setLinkPreviewsEnabled(enabled: Boolean) {
        viewModelScope.launch { settings.setLinkPreviewsEnabled(enabled) }
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
                    _state.update {
                        it.copy(
                            familyName = mine.family.name,
                            // Owner-only on the wire — null for members.
                            inviteCode = mine.family.inviteCode,
                            joinPolicy = mine.family.joinPolicy,
                        )
                    }
                }
            }
        }
    }

    fun leaveFamily() {
        viewModelScope.launch {
            when (val result = familyRepository.leave()) {
                // Success: SessionRepository emitted RemovedFromFamily —
                // the nav host reroutes; nothing to do here.
                is ApiResult.Ok -> Unit
                is ApiResult.HttpError -> _state.update {
                    it.copy(
                        error = if (result.code == "owner_cannot_leave") {
                            "As the owner you can only leave once every other " +
                                "member is removed — the family dissolves with you."
                        } else {
                            result.message ?: "Couldn't leave the family"
                        },
                    )
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = "Can't reach the server") }
            }
        }
    }

    fun logout() {
        viewModelScope.launch {
            sessionRepository.logout()
            _state.update { it.copy(loggedOut = true) }
        }
    }

    fun dismissError() {
        _state.update { it.copy(error = null) }
    }
}

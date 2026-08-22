/*
 * FamilyAdminViewModel.kt
 * Family Connect (Android)
 *
 * Owner console: pending join requests (approve/reject), invite-code
 * rotation, join-policy toggle, member removal. Every mutation reloads
 * the affected slice from the server — admin actions are rare enough
 * that correctness beats optimism here.
 *
 * iOS counterpart: ios/FamilyConnect/UI/FamilyAdmin/FamilyAdminViewModel.swift
 */

package me.nettrash.familyconnect.ui.familyadmin

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.db.MemberEntity
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.JoinRequestDto
import me.nettrash.familyconnect.data.repo.FamilyRepository
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.settings.SettingsRepository
import javax.inject.Inject

@HiltViewModel
class FamilyAdminViewModel @Inject constructor(
    private val familyRepository: FamilyRepository,
    private val settings: SettingsRepository,
) : ViewModel() {

    data class UiState(
        val requests: List<JoinRequestDto> = emptyList(),
        val inviteCode: String? = null,
        val joinPolicy: String = "open",
        val busy: Boolean = false,
        val error: String? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    /** Live roster straight from Room (WS frames keep it fresh). */
    val members: StateFlow<List<MemberEntity>> = familyRepository.observeMembers()
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), emptyList())

    val myUserId: StateFlow<Long?> = settings.state.map { it.myUserId }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    /**
     * Owners get the whole screen; everyone else gets the roster only.
     * The member list itself is not owner-gated on the server — only the
     * invite code, the join policy, the requests and removal are — so a
     * plain member can see who is in the family.
     */
    val isOwner: StateFlow<Boolean> = settings.state
        .map { it.familyStatus == FamilyStatus.OWNER }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), false)

    init {
        load()
    }

    fun load() {
        viewModelScope.launch {
            familyRepository.refreshMine().okOrNull()?.let { mine ->
                _state.update {
                    it.copy(
                        inviteCode = mine.family.inviteCode,
                        joinPolicy = mine.family.joinPolicy,
                    )
                }
            }
            // Join requests are an owner-only endpoint: asking as a plain
            // member is a guaranteed 403, which would paint an error over
            // a screen that is otherwise perfectly useful to them.
            if (settings.state.first().familyStatus == FamilyStatus.OWNER) {
                loadRequests()
            }
        }
    }

    private suspend fun loadRequests() {
        when (val result = familyRepository.joinRequests()) {
            is ApiResult.Ok -> _state.update { it.copy(requests = result.value.requests) }
            is ApiResult.HttpError ->
                _state.update { it.copy(error = result.message ?: "Couldn't load join requests") }
            is ApiResult.NetworkError ->
                _state.update { it.copy(error = "Can't reach the server") }
        }
    }

    fun approve(requestId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.approve(requestId) }

    fun reject(requestId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.reject(requestId) }

    fun rotateInviteCode() {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.rotateInviteCode()) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, inviteCode = result.value.inviteCode) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: "Rotation failed") }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = "Can't reach the server") }
            }
        }
    }

    fun setJoinPolicy(policy: String) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = familyRepository.setJoinPolicy(policy)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, joinPolicy = result.value.family.joinPolicy) }
                is ApiResult.HttpError ->
                    _state.update { it.copy(busy = false, error = result.message ?: "Couldn't change the policy") }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, error = "Can't reach the server") }
            }
        }
    }

    fun removeMember(userId: Long, onSuccess: () -> Unit = {}) =
        mutate(onSuccess) { familyRepository.removeMember(userId) }

    /** Runs [block]; [onSuccess] fires only when the server confirms. */
    private fun mutate(onSuccess: () -> Unit = {}, block: suspend () -> ApiResult<*>) {
        viewModelScope.launch {
            _state.update { it.copy(busy = true, error = null) }
            when (val result = block()) {
                is ApiResult.Ok -> onSuccess()
                is ApiResult.HttpError ->
                    _state.update { it.copy(error = result.message ?: "Action failed") }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(error = "Can't reach the server") }
            }
            loadRequests()
            _state.update { it.copy(busy = false) }
        }
    }
}

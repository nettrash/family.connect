/*
 * FamilyGateViewModel.kt
 * Family Connect (Android)
 *
 * Create-or-join fork after login. Join outcome depends on the family's
 * policy: "joined" (open) goes straight to the chat list, "pending"
 * (approval) parks on the waiting screen. `invalid_invite_code` and
 * `join_request_pending` map onto the code field inline.
 *
 * iOS counterpart: ios/FamilyConnect/UI/FamilyGate/FamilyGateViewModel.swift
 */

package me.nettrash.familyconnect.ui.familygate

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.FamilyRepository
import javax.inject.Inject

@HiltViewModel
class FamilyGateViewModel @Inject constructor(
    private val familyRepository: FamilyRepository,
) : ViewModel() {

    enum class Outcome { JOINED, PENDING }

    data class UiState(
        val familyName: String = "",
        val inviteCode: String = "",
        val nameError: String? = null,
        val codeError: String? = null,
        val generalError: String? = null,
        val busy: Boolean = false,
        val outcome: Outcome? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    fun onFamilyNameChange(value: String) =
        _state.update { it.copy(familyName = value, nameError = null, generalError = null) }

    fun onInviteCodeChange(value: String) =
        // Invite codes are case-insensitive server-side but always
        // displayed uppercase — normalize as the user types.
        _state.update { it.copy(inviteCode = value.uppercase(), codeError = null, generalError = null) }

    fun createFamily() {
        val name = _state.value.familyName.trim()
        if (name.isEmpty() || name.length > 64) {
            _state.update { it.copy(nameError = "1–64 characters") }
            return
        }
        viewModelScope.launch {
            _state.update { it.copy(busy = true) }
            when (val result = familyRepository.create(name)) {
                is ApiResult.Ok ->
                    _state.update { it.copy(busy = false, outcome = Outcome.JOINED) }
                is ApiResult.HttpError -> _state.update {
                    it.copy(
                        busy = false,
                        generalError = when (result.code) {
                            "already_in_family" -> "You're already in a family — pull to refresh"
                            else -> result.message ?: "Couldn't create the family"
                        },
                    )
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = "Can't reach the server") }
            }
        }
    }

    fun join() {
        val code = _state.value.inviteCode.trim()
        if (code.isEmpty()) {
            _state.update { it.copy(codeError = "Enter the invite code") }
            return
        }
        viewModelScope.launch {
            _state.update { it.copy(busy = true) }
            when (val result = familyRepository.join(code)) {
                is ApiResult.Ok -> _state.update {
                    it.copy(
                        busy = false,
                        outcome = if (result.value.status == "joined") Outcome.JOINED else Outcome.PENDING,
                    )
                }
                is ApiResult.HttpError -> _state.update {
                    when (result.code) {
                        "invalid_invite_code" ->
                            it.copy(busy = false, codeError = "Invalid invite code")
                        // A request from a previous attempt is still live —
                        // that IS the pending state, go wait on it.
                        "join_request_pending" ->
                            it.copy(busy = false, outcome = Outcome.PENDING)
                        else -> it.copy(
                            busy = false,
                            generalError = result.message ?: "Couldn't join",
                        )
                    }
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = "Can't reach the server") }
            }
        }
    }
}

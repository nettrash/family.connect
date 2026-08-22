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
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.FamilyRepository
import javax.inject.Inject

@HiltViewModel
class FamilyGateViewModel @Inject constructor(
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
                            "already_in_family" -> appContext.getString(R.string.e_already_in_family)
                            else -> result.message ?: appContext.getString(R.string.e_create_family_failed)
                        },
                    )
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    fun join() {
        val code = _state.value.inviteCode.trim()
        if (code.isEmpty()) {
            _state.update { it.copy(codeError = appContext.getString(R.string.e_enter_invite_code)) }
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
                            it.copy(busy = false, codeError = appContext.getString(R.string.e_invalid_invite_code))
                        // A request from a previous attempt is still live —
                        // that IS the pending state, go wait on it.
                        "join_request_pending" ->
                            it.copy(busy = false, outcome = Outcome.PENDING)
                        else -> it.copy(
                            busy = false,
                            generalError = result.message ?: appContext.getString(R.string.e_join_failed),
                        )
                    }
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }
}

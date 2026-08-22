/*
 * AuthViewModel.kt
 * Family Connect (Android)
 *
 * Login / Register. Client-side validation mirrors the server's rules
 * (docs/protocol.md POST /auth/register) so the common mistakes never
 * cost a round-trip: username 3–32 of [a-zA-Z0-9_.], password ≥ 8,
 * display name 1–64 on register. Server-side `username_taken` (409)
 * maps onto the username field, `invalid_credentials` onto a general
 * error.
 *
 * On success SessionRepository has already stored the token, refreshed
 * /me and registered the device — this VM just surfaces the resulting
 * status so the nav host can route by it.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Auth/AuthViewModel.swift
 */

package me.nettrash.familyconnect.ui.auth

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import me.nettrash.familyconnect.R
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionRepository
import javax.inject.Inject

@HiltViewModel
class AuthViewModel @Inject constructor(
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
) : ViewModel() {

    enum class Mode { LOGIN, REGISTER }

    data class UiState(
        val mode: Mode = Mode.LOGIN,
        val username: String = "",
        val displayName: String = "",
        val password: String = "",
        val usernameError: String? = null,
        val displayNameError: String? = null,
        val passwordError: String? = null,
        val generalError: String? = null,
        val submitting: Boolean = false,
        /** Set once auth completed — the screen routes by it. */
        val authenticatedStatus: FamilyStatus? = null,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    /**
     * Display only: the server address in effect, so the screen can show
     * which host "Use a different server" would move away from.
     */
    val serverUrl: StateFlow<String?> = sessionRepository.sessionFlow
        .map { it.serverUrl }
        .stateIn(viewModelScope, SharingStarted.WhileSubscribed(5_000), null)

    fun setMode(mode: Mode) {
        _state.update {
            it.copy(
                mode = mode,
                usernameError = null,
                displayNameError = null,
                passwordError = null,
                generalError = null,
            )
        }
    }

    fun onUsernameChange(value: String) =
        _state.update { it.copy(username = value, usernameError = null, generalError = null) }

    fun onDisplayNameChange(value: String) =
        _state.update { it.copy(displayName = value, displayNameError = null, generalError = null) }

    fun onPasswordChange(value: String) =
        _state.update { it.copy(password = value, passwordError = null, generalError = null) }

    fun submit() {
        if (!validate()) return
        val snapshot = _state.value
        viewModelScope.launch {
            _state.update { it.copy(submitting = true, generalError = null) }
            val result = when (snapshot.mode) {
                Mode.REGISTER -> sessionRepository.register(
                    username = snapshot.username.trim(),
                    displayName = snapshot.displayName.trim(),
                    password = snapshot.password,
                )
                Mode.LOGIN -> sessionRepository.login(
                    username = snapshot.username.trim(),
                    password = snapshot.password,
                )
            }
            when (result) {
                is ApiResult.Ok -> _state.update {
                    it.copy(submitting = false, authenticatedStatus = result.value.status)
                }
                is ApiResult.HttpError -> _state.update {
                    when {
                        result.code == "username_taken" || result.status == 409 ->
                            it.copy(submitting = false, usernameError = appContext.getString(R.string.e_username_taken))
                        result.code == "invalid_credentials" ->
                            it.copy(submitting = false, generalError = appContext.getString(R.string.e_wrong_credentials))
                        else -> it.copy(
                            submitting = false,
                            generalError = result.message ?: "Request failed (HTTP ${result.status})",
                        )
                    }
                }
                is ApiResult.NetworkError -> _state.update {
                    it.copy(submitting = false, generalError = appContext.getString(R.string.e_unreachable_check_connection))
                }
            }
        }
    }

    private fun validate(): Boolean {
        val s = _state.value
        var ok = true
        if (!USERNAME_REGEX.matches(s.username.trim())) {
            _state.update {
                it.copy(usernameError = "3–32 characters: letters, digits, _ or .")
            }
            ok = false
        }
        if (s.password.length < 8) {
            _state.update { it.copy(passwordError = appContext.getString(R.string.e_password_too_short)) }
            ok = false
        }
        if (s.mode == Mode.REGISTER) {
            val name = s.displayName.trim()
            if (name.isEmpty() || name.length > 64) {
                _state.update { it.copy(displayNameError = "1–64 characters") }
                ok = false
            }
        }
        return ok
    }

    companion object {
        val USERNAME_REGEX = Regex("^[a-zA-Z0-9_.]{3,32}$")
    }
}

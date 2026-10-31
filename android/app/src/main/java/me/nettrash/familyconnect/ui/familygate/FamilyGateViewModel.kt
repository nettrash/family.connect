/*
 * FamilyGateViewModel.kt
 * Family Connect (Android)
 *
 * Create-or-join fork after login. Join outcome depends on the family's
 * policy: "joined" (open) goes straight to the chat list, "pending"
 * (approval) parks on the waiting screen. `invalid_invite_code` and
 * `join_request_pending` map onto the code field inline.
 *
 * The same two doors serve an account already in a family (#88;
 * docs/protocol.md, "Several families"): "New Family…" and "Join Another
 * Family…" open this screen with one card each, and the errors several
 * families added — the cap, owning one already, a server that keeps the
 * door shut — are said here.
 *
 * On a server that charges for families (#89; docs/protocol.md, "Paid
 * families") Create is offered only to a caller whose subscription is
 * active; this app sells none.
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
import me.nettrash.familyconnect.data.repo.PaidFamilies
import me.nettrash.familyconnect.data.settings.SettingsRepository
import me.nettrash.familyconnect.ui.families.FamilyErrors
import kotlinx.coroutines.flow.first
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
    private val settings: SettingsRepository,
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
        /**
         * Whether this server takes NEW families (docs/protocol.md,
         * "Starting a family"). False swaps the Create card for directions
         * to run a server of one's own; joining stays either way.
         */
        val registrationEnabled: Boolean = true,
        /**
         * Days an account may go without a family before this server
         * removes it; 0 when it never does, and then nothing is said
         * (docs/protocol.md, "Accounts without a family").
         */
        val familylessAccountTtlDays: Int = 0,
        /**
         * The server charges for a new family and the caller holds no
         * active subscription (docs/protocol.md, "Paid families"): the
         * create card gives way to the sentence that a new family cannot be
         * started from this app. Joining stays either way.
         */
        val mustPayToStart: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    init {
        // `/me` said at login whether this server takes new families and
        // the session keeps that in settings; the gate follows it live,
        // so a refresh that flips it redraws without a restart.
        viewModelScope.launch {
            settings.state.collect { s ->
                _state.update {
                    it.copy(
                        registrationEnabled = s.familyRegistrationEnabled,
                        familylessAccountTtlDays = s.familylessAccountTtlDays,
                        mustPayToStart = PaidFamilies.mustPayToStart(s.subscriptionsEnabled, s.subscriptionActive),
                    )
                }
            }
        }
    }

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
                is ApiResult.HttpError -> {
                    val familyError = familyError(result.code)
                    _state.update {
                        it.copy(
                            busy = false,
                            generalError = familyError ?: when (result.code) {
                                "already_in_family" -> appContext.getString(R.string.e_already_in_family)
                                // Only on a server that shut its door after this
                                // screen was drawn — the gate hides Create once
                                // it knows (docs/protocol.md, "Starting a family").
                                "family_registration_disabled" ->
                                    appContext.getString(R.string.s_no_new_families_title)
                                // A subscription that ended after the card
                                // was drawn (docs/protocol.md, "Paid
                                // families"). Terminal: trying again
                                // changes nothing until something is bought.
                                PaidFamilies.SUBSCRIPTION_REQUIRED ->
                                    appContext.getString(R.string.e_subscription_required)
                                else -> result.message ?: appContext.getString(R.string.e_create_family_failed)
                            },
                        )
                    }
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }

    private fun joinFailure(state: UiState, result: ApiResult.HttpError, another: Boolean): UiState = when (result.code) {
        // From inside a family ("Join Another Family…"), `already_in_family`
        // means THAT family (docs/protocol.md, "Several families").
        "already_in_family" -> state.copy(
            busy = false,
            generalError = appContext.getString(
                if (another) R.string.e_already_in_that_family else R.string.e_already_in_family,
            ),
        )
        "invalid_invite_code" ->
            state.copy(busy = false, codeError = appContext.getString(R.string.e_invalid_invite_code))
        // A request from a previous attempt is still live —
        // that IS the pending state, go wait on it.
        "join_request_pending" ->
            state.copy(busy = false, outcome = Outcome.PENDING)
        // The one thing this endpoint tells a stranger, and
        // deliberately: telling an invited member their
        // code is invalid on the day the family filled up
        // costs a real person a real join. A CLOSED family
        // is different and needs nothing here — it answers
        // `invalid_invite_code`, byte-identical to a code
        // that never existed (docs/protocol.md,
        // `POST /families/join`).
        "family_full" ->
            state.copy(busy = false, codeError = appContext.getString(R.string.e_family_full_join))
        else -> state.copy(
            busy = false,
            generalError = result.message ?: appContext.getString(R.string.e_join_failed),
        )
    }

    /**
     * The sentence for an error several families added (the cap is named
     * from `GET /me`), or null for every other code.
     */
    private suspend fun familyError(code: String?): String? =
        FamilyErrors.message(appContext, code, settings.state.first().maxFamiliesPerAccount)

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
                is ApiResult.HttpError -> {
                    // The errors several families added — the cap, above all —
                    // before the ones a first join has always had.
                    val familyError = familyError(result.code)
                    val another = settings.state.first().families.isNotEmpty()
                    _state.update {
                        if (familyError != null) {
                            it.copy(busy = false, generalError = familyError)
                        } else {
                            joinFailure(it, result, another)
                        }
                    }
                }
                is ApiResult.NetworkError ->
                    _state.update { it.copy(busy = false, generalError = appContext.getString(R.string.e_unreachable)) }
            }
        }
    }
}

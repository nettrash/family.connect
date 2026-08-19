/*
 * WaitingViewModel.kt
 * Family Connect (Android)
 *
 * Approval limbo. There is no push in v1 and no WS access before
 * membership, so approval is discovered by polling GET /me: on every
 * ON_RESUME (screen calls refresh), on a 20 s ticker, and on manual
 * refresh. PENDING → MEMBER routes forward; PENDING → nothing means the
 * request was declined (protocol GET /me note) and we say so instead of
 * silently bouncing.
 *
 * iOS counterpart: ios/FamilyConnect/UI/Waiting/WaitingViewModel.swift
 */

package me.nettrash.familyconnect.ui.waiting

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.repo.FamilyStatus
import me.nettrash.familyconnect.data.repo.SessionEvent
import me.nettrash.familyconnect.data.repo.SessionRepository
import javax.inject.Inject

@HiltViewModel
class WaitingViewModel @Inject constructor(
    private val sessionRepository: SessionRepository,
) : ViewModel() {

    data class UiState(
        val familyName: String? = null,
        val refreshing: Boolean = false,
        val approved: Boolean = false,
        val declined: Boolean = false,
    )

    private val _state = MutableStateFlow(UiState())
    val state: StateFlow<UiState> = _state

    init {
        viewModelScope.launch {
            _state.update { it.copy(familyName = sessionRepository.snapshot().familyName) }
        }
        // Rejection can also surface through someone else's refreshMe
        // (e.g. the boot path) — listen for the event, not just our polls.
        viewModelScope.launch {
            sessionRepository.sessionEvents.collect { event ->
                if (event == SessionEvent.JoinRequestRejected) {
                    _state.update { it.copy(declined = true) }
                }
            }
        }
        viewModelScope.launch {
            while (isActive) {
                delay(POLL_INTERVAL_MS)
                refresh()
            }
        }
    }

    fun refresh() {
        viewModelScope.launch {
            _state.update { it.copy(refreshing = true) }
            val snapshot = sessionRepository.refreshMe().okOrNull()
            _state.update {
                it.copy(
                    refreshing = false,
                    familyName = snapshot?.familyName ?: it.familyName,
                    approved = it.approved ||
                        snapshot?.status == FamilyStatus.MEMBER ||
                        snapshot?.status == FamilyStatus.OWNER,
                    // We were pending; now the server knows neither a
                    // family nor a request → declined.
                    declined = it.declined || snapshot?.status == FamilyStatus.NONE,
                )
            }
        }
    }

    companion object {
        const val POLL_INTERVAL_MS = 20_000L
    }
}

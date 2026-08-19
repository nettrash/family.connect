/*
 * MainViewModel.kt
 * Family Connect (Android)
 *
 * Boot gate: loads ONE SessionSnapshot so MainActivity knows the start
 * destination, then gets out of the way. Deliberately not a live flow —
 * the NavHost must be composed exactly once with a stable start
 * destination; everything after boot navigates via events, never by
 * re-seeding the graph.
 *
 * iOS counterpart: ios/FamilyConnect/App/RootViewModel.swift
 */

package me.nettrash.familyconnect

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import dagger.hilt.android.lifecycle.HiltViewModel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import me.nettrash.familyconnect.data.repo.SessionEvent
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.repo.SessionSnapshot
import javax.inject.Inject

@HiltViewModel
class MainViewModel @Inject constructor(
    sessionRepository: SessionRepository,
) : ViewModel() {

    private val _bootState = MutableStateFlow<SessionSnapshot?>(null)

    /** null while loading → spinner; then the one-shot boot snapshot. */
    val bootState: StateFlow<SessionSnapshot?> = _bootState

    /** Expired / removed-from-family reroutes, relayed to the NavHost. */
    val sessionEvents: SharedFlow<SessionEvent> = sessionRepository.sessionEvents

    init {
        viewModelScope.launch {
            _bootState.value = sessionRepository.snapshot()
        }
    }
}

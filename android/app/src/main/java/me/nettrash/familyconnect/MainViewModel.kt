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
 * Also holds the push-tap deep link: MainActivity parses notification
 * extras (cold start AND onNewIntent) into a PendingRoute which sits
 * here as state until AppNavHost consumes it exactly once — StateFlow
 * rather than an event flow because the NavHost may not be composed yet
 * when a cold-start tap arrives (bootState is still loading).
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
import me.nettrash.familyconnect.data.push.PendingRoute
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

    private val _pendingRoute = MutableStateFlow<PendingRoute?>(null)

    /** Notification-tap deep link; AppNavHost consumes it exactly once. */
    val pendingRoute: StateFlow<PendingRoute?> = _pendingRoute

    init {
        viewModelScope.launch {
            _bootState.value = sessionRepository.snapshot()
        }
    }

    /** A newer tap wins — the user tapped it last, it's what they want. */
    fun onPendingRoute(route: PendingRoute) {
        _pendingRoute.value = route
    }

    fun consumePendingRoute() {
        _pendingRoute.value = null
    }
}

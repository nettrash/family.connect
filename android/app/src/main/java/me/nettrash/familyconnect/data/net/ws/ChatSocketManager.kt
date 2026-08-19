/*
 * ChatSocketManager.kt
 * Family Connect (Android)
 *
 * Owns the socket's lifetime. Policy:
 *
 *   connect  ⇔ app foregrounded (ProcessLifecycleOwner ON_START)
 *              AND session snapshot canChat (token present, status
 *              MEMBER/OWNER) — evaluated live off sessionFlow, so
 *              logging out disconnects and joining a family connects
 *              without anyone poking the manager.
 *   onStop   →  close(1000) + cancel the reconnect loop. Background
 *              delivery is v2 (push); holding a socket in the
 *              background just drains battery to be killed anyway.
 *
 * Reconnect loop: full-jitter BackoffPolicy between attempts, reset on
 * every successful open; a ConnectivityObserver onAvailable edge
 * short-circuits the wait. Every successful open triggers
 * SyncEngine.resync() — the socket only tells us *that* things happened,
 * REST tells us *what* (protocol: best-effort delivery).
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ChatSocketManager.swift
 * (scenePhase-driven there).
 */

package me.nettrash.familyconnect.data.net.ws

import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.repo.SessionRepository
import me.nettrash.familyconnect.data.repo.SyncEngine
import me.nettrash.familyconnect.data.settings.ServerUrlNormalizer
import me.nettrash.familyconnect.di.AppScope
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class ChatSocketManager @Inject constructor(
    private val socket: ChatSocket,
    private val sessionRepository: SessionRepository,
    private val connectivity: ConnectivityObserver,
    private val syncEngine: SyncEngine,
    @param:AppScope private val scope: CoroutineScope,
) : DefaultLifecycleObserver {

    private val foregrounded = MutableStateFlow(false)
    private var loopJob: Job? = null

    init {
        // The desire to be connected is (foreground AND canChat) — a
        // change in either direction starts/stops the loop. This is what
        // makes "connect after join" and "disconnect on logout" work.
        scope.launch {
            combine(foregrounded, sessionRepository.sessionFlow) { fg, session ->
                fg && session.canChat
            }
                .distinctUntilChanged()
                .collect { desired -> if (desired) startLoop() else stopLoop() }
        }
    }

    // ProcessLifecycleOwner callbacks — registered in FamilyConnectApp.
    override fun onStart(owner: LifecycleOwner) {
        foregrounded.value = true
    }

    override fun onStop(owner: LifecycleOwner) {
        foregrounded.value = false
    }

    @Synchronized
    private fun startLoop() {
        if (loopJob?.isActive == true) return
        loopJob = scope.launch {
            val backoff = BackoffPolicy()
            while (isActive) {
                val session = sessionRepository.snapshot()
                val token = session.token
                val serverUrl = session.serverUrl
                if (token == null || serverUrl == null || !session.canChat) {
                    // Session changed under us; the collector in init will
                    // stop/restart the loop as appropriate.
                    break
                }

                socket.connect(ServerUrlNormalizer.wsUrl(serverUrl), token)
                val settled = socket.state.first { it != SocketState.Connecting }
                if (settled == SocketState.Open) {
                    backoff.reset()
                    // The wire may have been dark for any amount of time —
                    // REST is the truth, go fetch it.
                    runCatching { syncEngine.resync() }
                    socket.state.first { it == SocketState.Disconnected }
                }
                if (!isActive) break

                // Full-jitter wait; a returning network cuts it short.
                val delayMillis = backoff.nextDelayMillis()
                withTimeoutOrNull(delayMillis) { connectivity.onAvailable.first() }
            }
        }
    }

    @Synchronized
    private fun stopLoop() {
        loopJob?.cancel()
        loopJob = null
        socket.close(1000, "backgrounded or session ended")
    }
}

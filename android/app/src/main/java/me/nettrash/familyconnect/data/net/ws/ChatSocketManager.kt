/*
 * ChatSocketManager.kt
 * Family Connect (Android)
 *
 * Owns the socket's lifetime. Policy:
 *
 *   connect  ⇔ (app foregrounded (ProcessLifecycleOwner ON_START)
 *              OR a voice call is in progress)
 *              AND session snapshot canChat (token present, status
 *              MEMBER/OWNER) — evaluated live off sessionFlow, so
 *              logging out disconnects and joining a family connects
 *              without anyone poking the manager. The rule itself is
 *              [socketDesired], pure and tested.
 *   onStop   →  close(1000) + cancel the reconnect loop — unless a call
 *              holds it. Background delivery is push; holding a socket
 *              in the background just drains battery to be killed
 *              anyway. A CALL is the exception the protocol spells out
 *              ("a client keeps its socket open for the life of a
 *              call"): its `call_end`, its candidates and — for a phone
 *              woken by the push — the offer itself all arrive over it,
 *              and CallService is the foreground work that lets it stay.
 *   4401     →  the session is gone (expired, revoked, or the account
 *              deleted): SessionRepository.onSessionExpired, which wipes
 *              and reroutes to sign-in. Reconnecting cannot help.
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

import android.os.SystemClock
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
import me.nettrash.familyconnect.calls.CallManager
import me.nettrash.familyconnect.data.net.ConnectivityObserver
import me.nettrash.familyconnect.data.push.PushTokenRepository
import me.nettrash.familyconnect.data.repo.MessageRepository
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
    private val messageRepository: MessageRepository,
    private val pushTokenRepository: PushTokenRepository,
    private val callManager: CallManager,
    @param:AppScope private val scope: CoroutineScope,
) : DefaultLifecycleObserver {

    private val foregrounded = MutableStateFlow(false)
    private var loopJob: Job? = null

    init {
        // The desire to be connected is (foreground AND canChat) — a
        // change in either direction starts/stops the loop. This is what
        // makes "connect after join" and "disconnect on logout" work.
        scope.launch {
            combine(foregrounded, callManager.isInCall, sessionRepository.sessionFlow) { fg, inCall, session ->
                socketDesired(foregrounded = fg, inCall = inCall, canChat = session.canChat)
            }
                .distinctUntilChanged()
                .collect { desired -> if (desired) startLoop() else stopLoop() }
        }
        // THE OUTBOX DOES NOT WAIT FOR A WEBSOCKET.
        //
        // `resync()` — and with it the re-send of everything queued — used
        // to run in exactly one place: inside `if (settled ==
        // SocketState.Open)` below. On a network that carries HTTP but
        // blocks the upgrade (a captive portal, a corporate proxy, a
        // carrier that drops long-lived connections) nothing was ever
        // re-sent, and a message sat under a clock forever on a phone
        // whose other traffic worked. Both edges below reach the server
        // over plain REST, which is all a re-send needs.
        scope.launch {
            foregrounded.collect { inForeground ->
                if (inForeground) runCatching { syncEngine.resync() }
            }
        }
        scope.launch {
            // A returning network is a trigger (docs/protocol.md, "Sending
            // on an unreliable network"). The flush alone here, not a full
            // resync: the socket loop is already re-dialling on this same
            // signal and will resync when it lands.
            connectivity.onAvailable.collect {
                runCatching { messageRepository.flushPending() }
            }
        }

        // A 4401 close (or a 401 upgrade) is not something to reconnect
        // through — the session is gone, exactly as a REST 401 says it is,
        // and both have to end at the sign-in screen. Without this the
        // loop below just went on retrying behind a "Connecting…" banner
        // while the device looked signed in.
        scope.launch {
            socket.sessionExpired.collect { sessionRepository.onSessionExpired() }
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
                    // NOT backoff.reset() here. Reaching Open proves the
                    // upgrade happened, not that the connection is usable: a
                    // proxy — or our own server, which kicks a connection
                    // whose send queue overflows with code 1001 — can accept
                    // and drop at once. Resetting on Open restarted the
                    // ceiling every cycle, so it never climbed and the socket
                    // reconnected roughly twice a second forever, resyncing
                    // each time. Forgiveness is judged at the drop, on how
                    // long the connection lasted.
                    val openedAt = SystemClock.elapsedRealtime()
                    // The wire may have been dark for any amount of time —
                    // REST is the truth, go fetch it.
                    runCatching { syncEngine.resync() }
                    // Push-token upkeep piggybacks the resync moment: a
                    // registration that failed at login is retried here;
                    // an unchanged token no-ops (protocol: re-POST /devices
                    // on rotation, not on every connect).
                    runCatching { pushTokenRepository.registerCurrentToken() }
                    socket.state.first { it == SocketState.Disconnected }
                    if (BackoffPolicy.earnsReset(
                            openedElapsedMillis = openedAt,
                            nowElapsedMillis = SystemClock.elapsedRealtime(),
                        )
                    ) {
                        backoff.reset()
                    }
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

    companion object {
        /**
         * Whether this device should hold a socket right now. The
         * foreground OR a call, and in either case a session that can
         * chat: a call cannot outlive a sign-out, and a phone in a pocket
         * with nothing going on holds nothing.
         */
        fun socketDesired(foregrounded: Boolean, inCall: Boolean, canChat: Boolean): Boolean =
            (foregrounded || inCall) && canChat
    }
}

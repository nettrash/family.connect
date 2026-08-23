/*
 * ChatSocket.kt
 * Family Connect (Android)
 *
 * One OkHttp WebSocket to `{base}/api/v1/ws`, authenticated via the
 * Bearer header on the upgrade request. Inbound text frames are parsed
 * with parseServerFrame (unknown types logged + dropped, per the
 * protocol's compatibility rules) and fanned out on a buffered
 * SharedFlow — capacity 64, DROP_OLDEST, because the socket is a live
 * wire, not a queue: anything a slow collector misses is repaired by the
 * next resync (protocol: "Best-effort delivery").
 *
 * Keepalive: the server WS-pings every 30 s, but OkHttp answers protocol
 * pings internally and hides them from us — so we run the app-level
 * `{"type":"ping"}` the protocol prescribes for exactly this situation,
 * every 25 s while open. Two pings without a pong in between mean the
 * wire is dead even though TCP hasn't noticed; we cancel and let the
 * manager's reconnect loop take over.
 *
 * Interface + impl split so pipeline tests can drive frames without a
 * network.
 *
 * iOS counterpart: ios/FamilyConnect/Data/Net/ChatSocket.swift
 * (URLSessionWebSocketTask there).
 */

package me.nettrash.familyconnect.data.net.ws

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import me.nettrash.familyconnect.di.AppScope
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.TimeUnit
import javax.inject.Inject
import javax.inject.Singleton

enum class SocketState {
    Disconnected,
    Connecting,
    Open,
}

interface ChatSocket {
    val frames: SharedFlow<ServerFrame>
    val state: StateFlow<SocketState>

    fun connect(wsUrl: String, token: String)
    fun close(code: Int = 1000, reason: String = "bye")

    /** Send one frame; false when the socket isn't open (caller falls back to REST). */
    fun trySend(frame: ClientFrame): Boolean
}

@Singleton
class OkHttpChatSocket @Inject constructor(
    baseClient: OkHttpClient,
    private val json: Json,
    @param:AppScope private val scope: CoroutineScope,
) : ChatSocket {

    // Same connection pool + dispatcher as the REST client; only the read
    // timeout differs — the server drops sockets idle for 75 s, so we
    // tolerate exactly that much silence before OkHttp declares failure.
    private val client: OkHttpClient = baseClient.newBuilder()
        .readTimeout(75, TimeUnit.SECONDS)
        .build()

    private val _frames = MutableSharedFlow<ServerFrame>(
        extraBufferCapacity = 64,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    override val frames: SharedFlow<ServerFrame> = _frames

    private val _state = MutableStateFlow(SocketState.Disconnected)
    override val state: StateFlow<SocketState> = _state

    private var webSocket: WebSocket? = null
    private var pingJob: Job? = null

    /** Pings sent since the last pong; 2 unanswered = dead wire. */
    @Volatile
    private var outstandingPings = 0

    @Synchronized
    override fun connect(wsUrl: String, token: String) {
        if (_state.value != SocketState.Disconnected) return
        _state.value = SocketState.Connecting
        val request = Request.Builder()
            .url(wsUrl)
            .header("Authorization", "Bearer $token")
            .build()
        webSocket = client.newWebSocket(request, listener)
    }

    @Synchronized
    override fun close(code: Int, reason: String) {
        webSocket?.close(code, reason)
        webSocket = null
        stopPinging()
        _state.value = SocketState.Disconnected
    }

    override fun trySend(frame: ClientFrame): Boolean {
        if (_state.value != SocketState.Open) return false
        val ws = webSocket ?: return false
        // encodeToString<ClientFrame> resolves the sealed-hierarchy
        // serializer so the "type" discriminator is always written.
        return ws.send(json.encodeToString<ClientFrame>(frame))
    }

    // -- Listener -------------------------------------------------------------

    /**
     * Is this callback about the socket we are actually using?
     *
     * OkHttp hands the [WebSocket] to every callback, and a socket that has
     * been replaced can still deliver a late [onFailure] or [onClosed]. Acting
     * on those clobbered a NEWER, live connection: the state went
     * `Disconnected` while frames kept arriving on the new socket — which is
     * both the "Connecting… but everything works" banner AND, because
     * [trySend] gates on the state, a silent fallback of every send onto REST.
     */
    private fun isCurrent(candidate: WebSocket): Boolean =
        synchronized(this@OkHttpChatSocket) { candidate === webSocket }

    private val listener = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            synchronized(this@OkHttpChatSocket) {
                if (webSocket !== this@OkHttpChatSocket.webSocket) return
                outstandingPings = 0
                _state.value = SocketState.Open
                startPinging()
            }
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (!isCurrent(webSocket)) return
            val frame = parseServerFrame(json, text)
            if (frame == null) {
                // Unknown type — a future protocol addition. Drop, don't die.
                Log.d(TAG, "Dropping unparseable frame: ${text.take(120)}")
                return
            }
            // ANY inbound frame proves the wire is alive, so this is what
            // makes a state that got stuck at Connecting heal itself — and
            // any frame answers a ping, not just a Pong.
            outstandingPings = 0
            synchronized(this@OkHttpChatSocket) {
                if (_state.value == SocketState.Connecting) _state.value = SocketState.Open
            }
            _frames.tryEmit(frame)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(code, null)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            if (!isCurrent(webSocket)) return
            markDisconnected()
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            Log.d(TAG, "Socket failure: ${t.message}")
            if (!isCurrent(webSocket)) return
            markDisconnected()
        }
    }

    @Synchronized
    private fun markDisconnected() {
        webSocket = null
        stopPinging()
        _state.value = SocketState.Disconnected
    }

    // -- App-level keepalive ---------------------------------------------------

    private fun startPinging() {
        pingJob?.cancel()
        pingJob = scope.launch {
            while (isActive && _state.value == SocketState.Open) {
                delay(PING_INTERVAL_MS)
                if (outstandingPings >= 2) {
                    // Two pings, no pong: half-open TCP. Cancel hard so
                    // onFailure fires and the reconnect loop wakes up.
                    synchronized(this@OkHttpChatSocket) { webSocket?.cancel() }
                    break
                }
                outstandingPings += 1
                trySend(ClientFrame.Ping)
            }
        }
    }

    private fun stopPinging() {
        pingJob?.cancel()
        pingJob = null
    }

    private companion object {
        const val TAG = "ChatSocket"
        const val PING_INTERVAL_MS = 25_000L
    }
}

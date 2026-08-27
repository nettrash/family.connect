/*
 * CallManager.kt
 * Family Connect (Android)
 *
 * The voice-call state machine (docs/protocol.md, "Voice calls"), one per
 * process, alive for as long as the app is: it is what the push service
 * hands an incoming call to, what the socket manager asks "are we on a
 * call" (a call holds the socket open), what CallService draws its
 * notification from, and what the call screen and the chat's call button
 * drive.
 *
 * What the protocol asks of a client, and where each rule lives here:
 *
 *   - `call_id` is minted by the caller (startCall), a UUID like a
 *     client_msg_id.
 *   - A frame is applied only to the call this device holds, by id —
 *     every other one is ignored in silence (onFrame). That is what makes
 *     the multi-device story work without the server tracking which of a
 *     person's devices is doing what.
 *   - A `call_offer` for a call already held is the duplicate it is — the
 *     replay on connect — and does nothing (onOffer).
 *   - Remote candidates are BUFFERED until the remote description is set
 *     (onIce / flushCandidates): a replay arrives offer-then-candidates,
 *     but a live relay promises no order.
 *   - ICE servers are fetched at the start of EVERY call, never cached.
 *   - The socket stays open for the life of the call: `isInCall` is an
 *     input to ChatSocketManager's desire to be connected.
 *   - A dead call is reported by THIS side with reason "failed", from the
 *     peer connection's own failure (Listener.onFailed) — the server
 *     never ends an active call on a socket blip.
 *   - Guards: an incoming call with no answer, or no offer, gives up after
 *     the ring timeout; an outgoing one at twice it; a connecting one
 *     when the media never comes up.
 *
 * Threading: every mutation runs under one Mutex on the app scope. WebRTC
 * calls back on its own threads; the listener hops onto the scope first.
 */

package me.nettrash.familyconnect.calls

import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.ChatApi
import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import me.nettrash.familyconnect.data.net.dto.IceServerDto
import me.nettrash.familyconnect.data.net.ws.CallEndReason
import me.nettrash.familyconnect.data.net.ws.ChatSocket
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.di.AppScope
import me.nettrash.familyconnect.util.Clock
import org.webrtc.VideoSink
import javax.inject.Inject
import javax.inject.Singleton

/**
 * The chat screen's way in. A fun interface rather than CallManager itself
 * so ChatViewModel can default it in its constructor (tests build the
 * ViewModel by hand; Dagger ignores the default and injects the manager).
 */
fun interface CallStarter {
    /**
     * False when a call could not be started — this device is already on
     * one. [video] fixes the call's KIND at placement (docs/protocol.md,
     * "Video"); it never changes mid-call.
     */
    fun startCall(chatId: Long, peerUserId: Long, video: Boolean): Boolean
}

/**
 * The activity's way in: the state the call screen follows, and the
 * notification's Answer button. An interface for the same reason as
 * [CallStarter] — MainViewModel defaults it so its tests build without a
 * manager; Dagger injects the real one.
 */
interface CallStateSource {
    val state: StateFlow<CallState>
    fun requestAnswer()

    companion object {
        /** No calls at all — what a test's MainViewModel sees. */
        val NONE: CallStateSource = object : CallStateSource {
            override val state: StateFlow<CallState> = MutableStateFlow(CallState.Idle)
            override fun requestAnswer() = Unit
        }
    }
}

@Singleton
class CallManager @Inject constructor(
    private val socket: ChatSocket,
    private val chatApi: ChatApi,
    private val mediaFactory: CallMediaClient.Factory,
    private val audio: CallAudio,
    private val clock: Clock,
    private val timings: CallTimings,
    @param:AppScope private val scope: CoroutineScope,
) : CallStarter, CallStateSource {

    private val _state = MutableStateFlow<CallState>(CallState.Idle)
    override val state: StateFlow<CallState> = _state

    /**
     * Whether a call is in any state but Idle — the socket manager's
     * input. Ended counts: the closing `call_end` may still be on its way
     * out, and two seconds of socket is nothing.
     */
    val isInCall: StateFlow<Boolean> = _state.map { it !is CallState.Idle }
        .stateIn(scope, SharingStarted.Eagerly, false)

    private val _isMuted = MutableStateFlow(false)
    val isMuted: StateFlow<Boolean> = _isMuted

    private val _isSpeaker = MutableStateFlow(false)
    val isSpeaker: StateFlow<Boolean> = _isSpeaker

    /**
     * Video calls only: whether the local camera track is live. Defaults
     * to ON when a video call starts (docs/protocol.md, "Video") — and to
     * OFF by the screen when the camera permission is denied, which still
     * answers or places the call.
     */
    private val _isCameraOn = MutableStateFlow(false)
    val isCameraOn: StateFlow<Boolean> = _isCameraOn

    private val _isFrontCamera = MutableStateFlow(true)
    val isFrontCamera: StateFlow<Boolean> = _isFrontCamera

    /**
     * The far side's PICTURE is flowing — the first decoded frame, not the
     * track's arrival (onAddTrack fires at connect time on every video
     * call, frames or not; a far side with its camera off or denied never
     * sends any). The screen keeps the avatar and status until it flips.
     */
    private val _remoteVideoActive = MutableStateFlow(false)
    val remoteVideoActive: StateFlow<Boolean> = _remoteVideoActive

    /**
     * The notification's Answer button asked for the call to be taken,
     * before the screen — which holds the microphone permission flow —
     * was up. The screen consumes it.
     */
    private val _answerRequested = MutableStateFlow(false)
    val answerRequested: StateFlow<Boolean> = _answerRequested

    private val mutex = Mutex()
    private var media: CallMediaClient? = null
    // The UI's renderers, held here so a media client created later (the
    // screen usually exists before the call is answered) still gets them.
    private var localVideoSink: VideoSink? = null
    private var remoteVideoSink: VideoSink? = null
    private var offerSdp: String? = null
    private var remoteDescriptionSet = false
    private val pendingRemoteCandidates = ArrayList<IceCandidateDto>()
    private var acceptPending = false
    private var answeredAtMillis: Long? = null
    private var guardJob: Job? = null
    private var lingerJob: Job? = null

    init {
        scope.launch {
            socket.frames.collect { frame -> mutex.withLock { onFrame(frame) } }
        }
    }

    // -- The four things a person can do ---------------------------------------

    override fun startCall(chatId: Long, peerUserId: Long, video: Boolean): Boolean {
        val current = _state.value
        if (current !is CallState.Idle && current !is CallState.Ended) return false
        scope.launch { mutex.withLock { beginOutgoing(chatId, peerUserId, video) } }
        return true
    }

    /**
     * Take the incoming call. If the offer is not here yet, it is taken the
     * moment it arrives. [cameraGranted] is the answer dialog's camera
     * verdict on a VIDEO call — granting the camera in the dialog starts
     * the call with the camera ON (parity with iOS, docs/protocol.md,
     * "Video"), denying it answers camera-off; null (a voice call, or the
     * grant already held) leaves the camera state as it stands. Applied
     * HERE, before the media opens, so the screen's while-it-rang
     * camera-off enforcement cannot swallow a grant given in the dialog.
     */
    fun accept(cameraGranted: Boolean? = null) {
        scope.launch {
            mutex.withLock {
                val current = _state.value as? CallState.Incoming ?: return@withLock
                _answerRequested.value = false
                if (cameraGranted != null && current.video) _isCameraOn.value = cameraGranted
                if (!current.hasOffer) {
                    acceptPending = true
                    return@withLock
                }
                answerOffer(current)
            }
        }
    }

    /** The notification's Answer button: remembered until the screen can act on it. */
    override fun requestAnswer() {
        if (_state.value is CallState.Incoming) _answerRequested.value = true
    }

    fun decline() {
        scope.launch {
            mutex.withLock {
                val current = _state.value as? CallState.Incoming ?: return@withLock
                socket.trySend(ClientFrame.CallEnd(current.callId, CallEndReason.DECLINE))
                finish(CallEnding.DECLINE)
            }
        }
    }

    fun hangUp() {
        scope.launch {
            mutex.withLock {
                when (val current = _state.value) {
                    is CallState.Outgoing -> {
                        socket.trySend(ClientFrame.CallEnd(current.callId, CallEndReason.CANCEL))
                        finish(CallEnding.CANCEL)
                    }
                    is CallState.Incoming -> {
                        socket.trySend(ClientFrame.CallEnd(current.callId, CallEndReason.DECLINE))
                        finish(CallEnding.DECLINE)
                    }
                    is CallState.Connecting, is CallState.Active -> {
                        socket.trySend(ClientFrame.CallEnd((current as CallState.Live).callId, CallEndReason.HANGUP))
                        finish(CallEnding.HANGUP)
                    }
                    CallState.Idle, is CallState.Ended -> Unit
                }
            }
        }
    }

    fun toggleMute() {
        val muted = !_isMuted.value
        _isMuted.value = muted
        media?.setMuted(muted)
    }

    fun toggleSpeaker() {
        val on = !_isSpeaker.value
        _isSpeaker.value = on
        audio.setSpeaker(on)
    }

    /**
     * Turn the camera on or off — the TRACK toggles, the call's kind
     * never does (docs/protocol.md, "Video"). Also how the screen answers
     * or places a video call with the camera permission denied: camera
     * off, call on.
     */
    fun setCameraEnabled(on: Boolean) {
        _isCameraOn.value = on
        media?.setCameraEnabled(on)
    }

    fun toggleCamera() = setCameraEnabled(!_isCameraOn.value)

    fun flipCamera() {
        _isFrontCamera.value = !_isFrontCamera.value
        media?.flipCamera()
    }

    /** Where the local preview draws. The UI owns the renderer; null detaches. */
    fun setLocalVideoSink(sink: VideoSink?) {
        localVideoSink = sink
        media?.setLocalVideoSink(sink)
    }

    /** Where the far side's video draws. The UI owns the renderer; null detaches. */
    fun setRemoteVideoSink(sink: VideoSink?) {
        remoteVideoSink = sink
        media?.setRemoteVideoSink(sink?.let(::firstFrameNotifyingSink))
    }

    /**
     * The remote sink the media client actually gets: the UI's own sink,
     * plus the one signal the screen draws its avatar by. remoteVideoActive
     * flips true only when a REAL frame arrives — never on track presence —
     * so a video call whose far side keeps the camera off (or denied) shows
     * the avatar over the black surface for its whole life. Fires once,
     * hops off WebRTC's render thread onto the manager's scope, and checks
     * the call is still the same one before flipping.
     */
    private fun firstFrameNotifyingSink(delegate: VideoSink): VideoSink {
        val callId = (_state.value as? CallState.Live)?.callId ?: return delegate
        val notified = AtomicBoolean(false)
        return VideoSink { frame ->
            delegate.onFrame(frame)
            if (notified.compareAndSet(false, true)) {
                scope.launch {
                    mutex.withLock {
                        val current = _state.value as? CallState.Live ?: return@withLock
                        if (current.callId == callId) _remoteVideoActive.value = true
                    }
                }
            }
        }
    }

    // -- The push that woke us ---------------------------------------------------

    /**
     * An FCM `kind: "call"` data message (docs/protocol.md, "Incoming
     * calls"). The device knows there IS a call; the offer comes over the
     * socket, replayed when it connects — or a `call_end`, if the call is
     * already over. Nothing happens if this device is on a call already.
     */
    fun onIncomingPush(callId: String, chatId: Long, fromUserId: Long, callerName: String?, video: Boolean = false) {
        scope.launch {
            mutex.withLock {
                val current = _state.value
                if (current is CallState.Live) return@withLock
                lingerJob?.cancel()
                resetCallLocals()
                _state.value = CallState.Incoming(
                    callId = callId,
                    chatId = chatId,
                    peerUserId = fromUserId,
                    video = video,
                    callerName = callerName,
                    hasOffer = false,
                )
                _isCameraOn.value = video
                startGuard(timings.ringTimeoutMillis) {
                    val still = _state.value as? CallState.Incoming ?: return@startGuard
                    if (still.callId != callId) return@startGuard
                    finish(if (still.hasOffer) CallEnding.TIMEOUT else CallEnding.NO_OFFER)
                }
            }
        }
    }

    // -- Outgoing ----------------------------------------------------------------

    private suspend fun beginOutgoing(chatId: Long, peerUserId: Long, video: Boolean) {
        val current = _state.value
        if (current !is CallState.Idle && current !is CallState.Ended) return
        lingerJob?.cancel()
        resetCallLocals()
        val callId = UUID.randomUUID().toString()
        _state.value = CallState.Outgoing(callId, chatId, peerUserId, video = video, ringing = false)
        // The camera comes up with a video call; the screen turns it off
        // when the permission is denied (the call still happens). Video
        // also defaults the SPEAKER on — begin(video) routes it, and the
        // flow must say so.
        _isCameraOn.value = video
        audio.begin(video)
        _isSpeaker.value = video
        val client = openMedia(callId, video) ?: return
        val sdp = try {
            client.createOffer()
        } catch (error: Exception) {
            finish(CallEnding.FAILED)
            return
        }
        // The call may have been cancelled while the offer was gathering.
        val still = _state.value as? CallState.Outgoing ?: return
        if (still.callId != callId) return
        // A voice offer omits the key entirely (encodeDefaults=false), so
        // it stays byte-identical to what it was before video existed.
        if (!socket.trySend(ClientFrame.CallOffer(callId, chatId, sdp, video = if (video) true else null))) {
            finish(CallEnding.FAILED)
            return
        }
        startGuard(timings.outgoingGuardMillis) {
            val ringing = _state.value as? CallState.Outgoing ?: return@startGuard
            if (ringing.callId != callId) return@startGuard
            socket.trySend(ClientFrame.CallEnd(callId, CallEndReason.CANCEL))
            finish(CallEnding.TIMEOUT)
        }
    }

    // -- Incoming ----------------------------------------------------------------

    private suspend fun answerOffer(incoming: CallState.Incoming) {
        val sdp = offerSdp ?: return
        acceptPending = false
        _state.value = CallState.Connecting(
            incoming.callId,
            incoming.chatId,
            incoming.peerUserId,
            video = incoming.video,
            incoming = true,
        )
        audio.begin(incoming.video)
        _isSpeaker.value = incoming.video
        val client = openMedia(incoming.callId, incoming.video) ?: return
        val answer = try {
            client.setRemoteDescription(SdpType.OFFER, sdp)
            remoteDescriptionSet = true
            flushCandidates()
            client.createAnswer()
        } catch (error: Exception) {
            socket.trySend(ClientFrame.CallEnd(incoming.callId, CallEndReason.FAILED))
            finish(CallEnding.FAILED)
            return
        }
        val still = _state.value as? CallState.Connecting ?: return
        if (still.callId != incoming.callId) return
        if (!socket.trySend(ClientFrame.CallAnswer(incoming.callId, answer))) {
            finish(CallEnding.FAILED)
            return
        }
        startConnectGuard(incoming.callId)
    }

    // -- Frames ------------------------------------------------------------------

    private suspend fun onFrame(frame: ServerFrame) {
        when (frame) {
            is ServerFrame.CallOffer -> onOffer(frame)
            is ServerFrame.CallRinging -> {
                val current = _state.value as? CallState.Outgoing ?: return
                if (current.callId == frame.callId) _state.value = current.copy(ringing = true)
            }
            is ServerFrame.CallAnswer -> onAnswer(frame)
            is ServerFrame.CallIce -> onIce(frame)
            is ServerFrame.CallEnd -> {
                val current = _state.value as? CallState.Live ?: return
                if (current.callId == frame.callId) finish(CallEnding.fromWire(frame.reason))
            }
            is ServerFrame.Error -> {
                val callId = frame.callId ?: return
                val current = _state.value as? CallState.Live ?: return
                if (current.callId == callId) finish(CallEnding.fromErrorCode(frame.code))
            }
            else -> Unit
        }
    }

    private suspend fun onOffer(frame: ServerFrame.CallOffer) {
        when (val current = _state.value) {
            is CallState.Incoming -> {
                if (current.callId != frame.callId) return
                // The replay after a push: the offer the push could not carry.
                // A second copy of one already held is a duplicate, and
                // does nothing.
                if (current.hasOffer) return
                offerSdp = frame.sdp
                // The OFFER is authoritative about the kind — a push that
                // could not carry the flag must not turn a video call
                // into a voice one.
                val withOffer = current.copy(hasOffer = true, video = frame.video)
                if (current.video != frame.video) _isCameraOn.value = frame.video
                _state.value = withOffer
                if (acceptPending) answerOffer(withOffer)
            }
            is CallState.Idle, is CallState.Ended -> {
                lingerJob?.cancel()
                resetCallLocals()
                offerSdp = frame.sdp
                _state.value = CallState.Incoming(
                    callId = frame.callId,
                    chatId = frame.chatId,
                    peerUserId = frame.fromUserId,
                    video = frame.video,
                    hasOffer = true,
                )
                _isCameraOn.value = frame.video
                startGuard(timings.ringTimeoutMillis) {
                    val still = _state.value as? CallState.Incoming ?: return@startGuard
                    if (still.callId == frame.callId) finish(CallEnding.TIMEOUT)
                }
            }
            // On a call already: the server refuses offers to a busy
            // callee, so this is a race at most. Not ours.
            is CallState.Outgoing, is CallState.Connecting, is CallState.Active -> Unit
        }
    }

    private suspend fun onAnswer(frame: ServerFrame.CallAnswer) {
        val current = _state.value as? CallState.Outgoing ?: return
        if (current.callId != frame.callId) return
        val client = media ?: return
        _state.value = CallState.Connecting(
            current.callId,
            current.chatId,
            current.peerUserId,
            video = current.video,
            incoming = false,
        )
        try {
            client.setRemoteDescription(SdpType.ANSWER, frame.sdp)
            remoteDescriptionSet = true
            flushCandidates()
        } catch (error: Exception) {
            socket.trySend(ClientFrame.CallEnd(current.callId, CallEndReason.FAILED))
            finish(CallEnding.FAILED)
            return
        }
        startConnectGuard(current.callId)
    }

    private fun onIce(frame: ServerFrame.CallIce) {
        val current = _state.value as? CallState.Live ?: return
        if (current.callId != frame.callId) return
        val client = media
        if (remoteDescriptionSet && client != null) {
            client.addRemoteCandidate(frame.candidate)
        } else {
            pendingRemoteCandidates += frame.candidate
        }
    }

    private fun flushCandidates() {
        val client = media ?: return
        pendingRemoteCandidates.forEach(client::addRemoteCandidate)
        pendingRemoteCandidates.clear()
    }

    // -- Media -------------------------------------------------------------------

    /** Fetch the ICE servers and open a peer connection for [callId]; null (and Ended) on failure. */
    private suspend fun openMedia(callId: String, video: Boolean): CallMediaClient? {
        val servers: List<IceServerDto> = when (val result = chatApi.iceServers()) {
            is ApiResult.Ok -> result.value.iceServers
            is ApiResult.HttpError -> {
                finish(if (result.code == "calls_disabled") CallEnding.DISABLED else CallEnding.FAILED)
                return null
            }
            is ApiResult.NetworkError -> {
                finish(CallEnding.FAILED)
                return null
            }
        }
        // The call may have ended while the request was out.
        val live = _state.value as? CallState.Live
        if (live == null || live.callId != callId) return null
        val client = try {
            mediaFactory.create(servers, video, MediaListener(callId))
        } catch (error: Exception) {
            finish(CallEnding.FAILED)
            return null
        }
        media = client
        client.setMuted(_isMuted.value)
        if (video) {
            // The camera state was decided BEFORE the media existed (the
            // default, or the screen's permission-denied override) — hand
            // it over, along with whatever renderers the screen already
            // registered.
            client.setCameraEnabled(_isCameraOn.value)
            client.setLocalVideoSink(localVideoSink)
            client.setRemoteVideoSink(remoteVideoSink?.let(::firstFrameNotifyingSink))
        }
        return client
    }

    private inner class MediaListener(private val callId: String) : CallMediaClient.Listener {
        override fun onLocalCandidate(candidate: IceCandidateDto) {
            scope.launch {
                mutex.withLock {
                    val current = _state.value as? CallState.Live ?: return@withLock
                    if (current.callId == callId) socket.trySend(ClientFrame.CallIce(callId, candidate))
                }
            }
        }

        override fun onConnected() {
            scope.launch {
                mutex.withLock {
                    val current = _state.value as? CallState.Connecting ?: return@withLock
                    if (current.callId != callId) return@withLock
                    guardJob?.cancel()
                    val now = clock.now()
                    answeredAtMillis = now
                    _state.value = CallState.Active(
                        current.callId,
                        current.chatId,
                        current.peerUserId,
                        video = current.video,
                        sinceMillis = now,
                    )
                }
            }
        }

        override fun onFailed() {
            scope.launch {
                mutex.withLock {
                    val current = _state.value as? CallState.Live ?: return@withLock
                    if (current.callId != callId) return@withLock
                    socket.trySend(ClientFrame.CallEnd(callId, CallEndReason.FAILED))
                    finish(CallEnding.FAILED)
                }
            }
        }

        override fun onRemoteVideoActive(active: Boolean) {
            // The track ARRIVING is not a picture: onAddTrack fires the
            // moment the SDP's video m-line applies, on every video call,
            // whether the far side ever captures a frame or not. The first
            // real frame — seen by firstFrameNotifyingSink — flips this
            // true; the track going away still clears it.
            if (active) return
            scope.launch {
                mutex.withLock {
                    val current = _state.value as? CallState.Live ?: return@withLock
                    if (current.callId == callId) _remoteVideoActive.value = false
                }
            }
        }
    }

    // -- Ending ------------------------------------------------------------------

    private fun startGuard(millis: Long, onExpiry: suspend () -> Unit) {
        guardJob?.cancel()
        guardJob = scope.launch {
            delay(millis)
            mutex.withLock { onExpiry() }
        }
    }

    private fun startConnectGuard(callId: String) {
        startGuard(timings.connectGuardMillis) {
            val still = _state.value as? CallState.Connecting ?: return@startGuard
            if (still.callId != callId) return@startGuard
            socket.trySend(ClientFrame.CallEnd(callId, CallEndReason.FAILED))
            finish(CallEnding.FAILED)
        }
    }

    private fun finish(reason: CallEnding) {
        val current = _state.value
        val live = current as? CallState.Live
        val duration = answeredAtMillis?.let { ((clock.now() - it) / 1000).toInt().coerceAtLeast(0) }
        guardJob?.cancel()
        guardJob = null
        media?.close()
        media = null
        audio.end()
        _isMuted.value = false
        _isSpeaker.value = false
        _answerRequested.value = false
        acceptPending = false
        _isCameraOn.value = false
        _isFrontCamera.value = true
        _remoteVideoActive.value = false
        _state.value = CallState.Ended(
            callId = live?.callId,
            chatId = live?.chatId,
            peerUserId = live?.peerUserId,
            reason = reason,
            durationSecs = duration,
            video = live?.video ?: false,
        )
        val ended = _state.value
        lingerJob?.cancel()
        lingerJob = scope.launch {
            delay(timings.lingerMillis)
            mutex.withLock {
                if (_state.value === ended) _state.value = CallState.Idle
            }
        }
    }

    private fun resetCallLocals() {
        guardJob?.cancel()
        guardJob = null
        media?.close()
        media = null
        offerSdp = null
        remoteDescriptionSet = false
        pendingRemoteCandidates.clear()
        acceptPending = false
        answeredAtMillis = null
        _isMuted.value = false
        _isSpeaker.value = false
        _isCameraOn.value = false
        _isFrontCamera.value = true
        _remoteVideoActive.value = false
        _answerRequested.value = false
    }
}

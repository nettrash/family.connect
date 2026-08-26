/*
 * WebRtcClient.kt
 * Family Connect (Android)
 *
 * The real CallMediaClient over libwebrtc (io.getstream:stream-webrtc-android,
 * see libs.versions.toml). Audio only, unified plan, one PeerConnection per
 * call. The ONLY file in the app that imports org.webrtc.
 *
 * Every SdpObserver callback is bridged into a coroutine so CallManager can
 * `await` an offer or an answer; the PeerConnection.Observer callbacks
 * that matter — a gathered candidate, the connection coming up or failing
 * — go to the Listener on WebRTC's thread, and CallManager hops off it.
 */

package me.nettrash.familyconnect.calls

import android.content.Context
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine
import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import me.nettrash.familyconnect.data.net.dto.IceServerDto
import org.webrtc.AudioSource
import org.webrtc.AudioTrack
import org.webrtc.DataChannel
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.PeerConnectionFactory
import org.webrtc.RtpReceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.audio.JavaAudioDeviceModule
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class WebRtcClientFactory @Inject constructor(
    @param:ApplicationContext private val context: Context,
) : CallMediaClient.Factory {

    // Built on first use, never at app start: initializing libwebrtc loads
    // the native library and spins up its threads, which a family that
    // never calls should not pay for.
    private val factory: PeerConnectionFactory by lazy {
        PeerConnectionFactory.initialize(
            PeerConnectionFactory.InitializationOptions.builder(context)
                .createInitializationOptions(),
        )
        val audioDeviceModule = JavaAudioDeviceModule.builder(context)
            .setUseHardwareAcousticEchoCanceler(true)
            .setUseHardwareNoiseSuppressor(true)
            .createAudioDeviceModule()
        PeerConnectionFactory.builder()
            .setAudioDeviceModule(audioDeviceModule)
            .setOptions(PeerConnectionFactory.Options())
            .createPeerConnectionFactory()
    }

    override fun create(iceServers: List<IceServerDto>, listener: CallMediaClient.Listener): CallMediaClient =
        WebRtcClient(factory, iceServers, listener)
}

class WebRtcClient(
    factory: PeerConnectionFactory,
    iceServers: List<IceServerDto>,
    private val listener: CallMediaClient.Listener,
) : CallMediaClient {

    private val audioSource: AudioSource
    private val audioTrack: AudioTrack
    private val connection: PeerConnection

    @Volatile
    private var closed = false

    // Declared BEFORE init on purpose: Kotlin initializes properties in
    // declaration order, and init hands this to createPeerConnection.
    private val observer = object : PeerConnection.Observer {
        override fun onIceCandidate(candidate: IceCandidate) {
            listener.onLocalCandidate(
                IceCandidateDto(
                    candidate = candidate.sdp,
                    sdpMid = candidate.sdpMid,
                    sdpMlineIndex = candidate.sdpMLineIndex,
                ),
            )
        }

        override fun onConnectionChange(newState: PeerConnection.PeerConnectionState) {
            when (newState) {
                PeerConnection.PeerConnectionState.CONNECTED -> listener.onConnected()
                // FAILED is final; DISCONNECTED is not — ICE keeps trying,
                // and a network change ends there before it ends CONNECTED
                // again. The protocol says the same of the socket.
                PeerConnection.PeerConnectionState.FAILED -> listener.onFailed()
                else -> Unit
            }
        }

        override fun onSignalingChange(state: PeerConnection.SignalingState) = Unit
        override fun onIceConnectionChange(state: PeerConnection.IceConnectionState) = Unit
        override fun onIceConnectionReceivingChange(receiving: Boolean) = Unit
        override fun onIceGatheringChange(state: PeerConnection.IceGatheringState) = Unit
        override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>) = Unit
        override fun onAddStream(stream: MediaStream) = Unit
        override fun onRemoveStream(stream: MediaStream) = Unit
        override fun onDataChannel(channel: DataChannel) = Unit
        override fun onRenegotiationNeeded() = Unit
        override fun onAddTrack(receiver: RtpReceiver, streams: Array<out MediaStream>) = Unit
    }

    init {
        val servers = iceServers.map { server ->
            PeerConnection.IceServer.builder(server.urls).apply {
                server.username?.let(::setUsername)
                server.credential?.let(::setPassword)
            }.createIceServer()
        }
        val config = PeerConnection.RTCConfiguration(servers).apply {
            sdpSemantics = PeerConnection.SdpSemantics.UNIFIED_PLAN
            // Keep gathering after the offer goes out: candidates trickle
            // as `call_ice` frames, and the server buffers the caller's
            // while the callee's phone is still waking up.
            continualGatheringPolicy = PeerConnection.ContinualGatheringPolicy.GATHER_CONTINUALLY
        }
        connection = checkNotNull(factory.createPeerConnection(config, observer)) {
            "PeerConnection could not be created"
        }
        audioSource = factory.createAudioSource(MediaConstraints())
        audioTrack = factory.createAudioTrack("audio0", audioSource)
        connection.addTrack(audioTrack, listOf("stream0"))
    }

    private val audioOnly = MediaConstraints().apply {
        mandatory.add(MediaConstraints.KeyValuePair("OfferToReceiveAudio", "true"))
        mandatory.add(MediaConstraints.KeyValuePair("OfferToReceiveVideo", "false"))
    }

    override suspend fun createOffer(): String = createLocal { observer -> connection.createOffer(observer, audioOnly) }

    override suspend fun createAnswer(): String = createLocal { observer -> connection.createAnswer(observer, audioOnly) }

    private suspend fun createLocal(create: (SdpObserver) -> Unit): String =
        suspendCancellableCoroutine { continuation ->
            create(
                object : SdpObserverAdapter() {
                    override fun onCreateSuccess(description: SessionDescription) {
                        connection.setLocalDescription(
                            object : SdpObserverAdapter() {
                                override fun onSetSuccess() {
                                    continuation.resume(description.description)
                                }

                                override fun onSetFailure(error: String?) {
                                    continuation.resumeWithException(IllegalStateException(error ?: "setLocalDescription failed"))
                                }
                            },
                            description,
                        )
                    }

                    override fun onCreateFailure(error: String?) {
                        continuation.resumeWithException(IllegalStateException(error ?: "create failed"))
                    }
                },
            )
        }

    override suspend fun setRemoteDescription(type: SdpType, sdp: String) {
        val description = SessionDescription(
            when (type) {
                SdpType.OFFER -> SessionDescription.Type.OFFER
                SdpType.ANSWER -> SessionDescription.Type.ANSWER
            },
            sdp,
        )
        suspendCancellableCoroutine { continuation ->
            connection.setRemoteDescription(
                object : SdpObserverAdapter() {
                    override fun onSetSuccess() = continuation.resume(Unit)
                    override fun onSetFailure(error: String?) {
                        continuation.resumeWithException(IllegalStateException(error ?: "setRemoteDescription failed"))
                    }
                },
                description,
            )
        }
    }

    override fun addRemoteCandidate(candidate: IceCandidateDto) {
        if (closed) return
        connection.addIceCandidate(
            IceCandidate(
                candidate.sdpMid ?: "",
                candidate.sdpMlineIndex ?: 0,
                candidate.candidate,
            ),
        )
    }

    override fun setMuted(muted: Boolean) {
        if (closed) return
        audioTrack.setEnabled(!muted)
    }

    override fun close() {
        if (closed) return
        closed = true
        runCatching { connection.close() }
        runCatching { connection.dispose() }
        runCatching { audioSource.dispose() }
    }

    /** The four SdpObserver callbacks, defaulted so each site overrides only what it awaits. */
    private abstract class SdpObserverAdapter : SdpObserver {
        override fun onCreateSuccess(description: SessionDescription) = Unit
        override fun onSetSuccess() = Unit
        override fun onCreateFailure(error: String?) = Unit
        override fun onSetFailure(error: String?) = Unit
    }
}

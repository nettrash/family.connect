/*
 * WebRtcClient.kt
 * Family Connect (Android)
 *
 * The real CallMediaClient over libwebrtc (io.getstream:stream-webrtc-android,
 * see libs.versions.toml). Unified plan, one PeerConnection per call —
 * audio always, plus a camera track when the call's kind is VIDEO
 * (docs/protocol.md, "Video": the kind is fixed at placement; cameras
 * toggle the TRACK, never renegotiate). The ONLY file in the app that
 * TOUCHES the native library — the seam (CallMediaClient) leaks only the
 * plain VideoSink interface, which is what keeps CallManager testable on
 * the JVM.
 *
 * Every SdpObserver callback is bridged into a coroutine so CallManager can
 * `await` an offer or an answer; the PeerConnection.Observer callbacks
 * that matter — a gathered candidate, the connection coming up or failing,
 * a remote video track arriving — go to the Listener on WebRTC's thread,
 * and CallManager hops off it.
 */

package me.nettrash.familyconnect.calls

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.util.Log
import androidx.core.content.ContextCompat
import dagger.hilt.android.qualifiers.ApplicationContext
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.suspendCancellableCoroutine
import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import me.nettrash.familyconnect.data.net.dto.IceServerDto
import org.webrtc.AudioSource
import org.webrtc.AudioTrack
import org.webrtc.Camera2Enumerator
import org.webrtc.CameraVideoCapturer
import org.webrtc.CandidatePairChangeEvent
import org.webrtc.DataChannel
import org.webrtc.DefaultVideoDecoderFactory
import org.webrtc.DefaultVideoEncoderFactory
import org.webrtc.EglBase
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.PeerConnectionFactory
import org.webrtc.RtpReceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.SurfaceTextureHelper
import org.webrtc.VideoSink
import org.webrtc.VideoSource
import org.webrtc.VideoTrack
import org.webrtc.audio.JavaAudioDeviceModule
import javax.inject.Inject
import javax.inject.Singleton

@Singleton
class WebRtcClientFactory @Inject constructor(
    @param:ApplicationContext private val context: Context,
) : CallMediaClient.Factory {

    /**
     * ONE EglBase for the whole process, owned here beside the
     * PeerConnectionFactory: the encoder/decoder factories and every
     * SurfaceViewRenderer must share ITS context, or frames decode into
     * textures no renderer can draw. Lazy for the same reason as the
     * factory below — a family that never calls should not pay for it.
     */
    private val eglBase: EglBase by lazy { EglBase.create() }

    /** What the UI initializes its SurfaceViewRenderers with. */
    val eglBaseContext: EglBase.Context get() = eglBase.eglBaseContext

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
            .setVideoEncoderFactory(DefaultVideoEncoderFactory(eglBase.eglBaseContext, true, true))
            .setVideoDecoderFactory(DefaultVideoDecoderFactory(eglBase.eglBaseContext))
            .setOptions(PeerConnectionFactory.Options())
            .createPeerConnectionFactory()
    }

    override fun create(
        iceServers: List<IceServerDto>,
        video: Boolean,
        listener: CallMediaClient.Listener,
    ): CallMediaClient = WebRtcClient(context, factory, eglBase.eglBaseContext, iceServers, video, listener)
}

class WebRtcClient(
    private val context: Context,
    factory: PeerConnectionFactory,
    eglContext: EglBase.Context,
    iceServers: List<IceServerDto>,
    private val video: Boolean,
    private val listener: CallMediaClient.Listener,
) : CallMediaClient {

    private val audioSource: AudioSource
    private val audioTrack: AudioTrack
    /** The far side's audio, once it arrives; gated by setRemoteAudioEnabled (a Telecom hold). */
    @Volatile
    private var remoteAudioTrack: AudioTrack? = null
    @Volatile
    private var remoteAudioEnabled = true
    private var videoSource: VideoSource? = null
    private var videoTrack: VideoTrack? = null
    private var videoCapturer: CameraVideoCapturer? = null
    private var surfaceTextureHelper: SurfaceTextureHelper? = null
    private val connection: PeerConnection

    @Volatile
    private var closed = false

    @Volatile
    private var capturing = false

    @Volatile
    private var localSink: VideoSink? = null

    @Volatile
    private var remoteSink: VideoSink? = null

    @Volatile
    private var remoteVideoTrack: VideoTrack? = null

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
            Log.i(TAG, "peer connection state: $newState")
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

        override fun onIceConnectionChange(state: PeerConnection.IceConnectionState) {
            Log.i(TAG, "ice connection state: $state")
        }

        override fun onIceConnectionReceivingChange(receiving: Boolean) = Unit

        override fun onIceGatheringChange(state: PeerConnection.IceGatheringState) {
            Log.i(TAG, "ice gathering: $state")
        }

        // Which KIND of pair ICE settled on — host/srflx/relay, never an
        // address. The one line that tells a direct call from a relayed
        // one in a bug report's logcat.
        override fun onSelectedCandidatePairChanged(event: CandidatePairChangeEvent) {
            Log.i(
                TAG,
                "selected pair: local=${candidateType(event.local)} remote=${candidateType(event.remote)} (${event.reason})",
            )
        }
        override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>) = Unit
        override fun onAddStream(stream: MediaStream) = Unit
        override fun onRemoveStream(stream: MediaStream) = Unit
        override fun onDataChannel(channel: DataChannel) = Unit
        override fun onRenegotiationNeeded() = Unit

        /**
         * The far side's tracks. The VIDEO one is routed to whatever sink
         * the UI registered — before or after this moment; both orders
         * happen, hence the field — and announced so the screen can drop
         * the avatar for the picture.
         */
        override fun onAddTrack(receiver: RtpReceiver, streams: Array<out MediaStream>) {
            val track = receiver.track()
            if (track is VideoTrack) {
                remoteVideoTrack = track
                remoteSink?.let(track::addSink)
                listener.onRemoteVideoActive(true)
            }
            if (track is AudioTrack) {
                remoteAudioTrack = track
                track.setEnabled(remoteAudioEnabled)
            }
        }

        override fun onRemoveTrack(receiver: RtpReceiver) {
            val track = runCatching { receiver.track() }.getOrNull()
            if (track is VideoTrack) {
                remoteVideoTrack = null
                listener.onRemoteVideoActive(false)
            }
        }
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

        if (video) {
            // The camera track exists for the LIFE of a video call — the
            // toggle enables/disables it and start/stops capture, it never
            // adds or removes the track (protocol.md, "Video": no
            // renegotiation). Front camera first; a device with no camera
            // at all still makes a video call, receive-only.
            val enumerator = Camera2Enumerator(context)
            val deviceName = enumerator.deviceNames.firstOrNull(enumerator::isFrontFacing)
                ?: enumerator.deviceNames.firstOrNull()
            if (deviceName != null) {
                val capturer = enumerator.createCapturer(deviceName, null)
                val helper = SurfaceTextureHelper.create("FcCaptureThread", eglContext)
                val source = factory.createVideoSource(capturer.isScreencast)
                capturer.initialize(helper, context, source.capturerObserver)
                val track = factory.createVideoTrack("video0", source)
                connection.addTrack(track, listOf("stream0"))
                videoCapturer = capturer
                surfaceTextureHelper = helper
                videoSource = source
                videoTrack = track
            }
        }
    }

    private val constraints = MediaConstraints().apply {
        mandatory.add(MediaConstraints.KeyValuePair("OfferToReceiveAudio", "true"))
        // The call's KIND decides this once, at creation — a video call
        // negotiates both m-lines from the start, even with the camera
        // off or its permission denied (the SDP is then receive-only).
        mandatory.add(MediaConstraints.KeyValuePair("OfferToReceiveVideo", if (video) "true" else "false"))
    }

    override suspend fun createOffer(): String = createLocal { observer -> connection.createOffer(observer, constraints) }

    override suspend fun createAnswer(): String = createLocal { observer -> connection.createAnswer(observer, constraints) }

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

    override fun setRemoteAudioEnabled(enabled: Boolean) {
        if (closed) return
        remoteAudioEnabled = enabled
        remoteAudioTrack?.setEnabled(enabled)
    }

    override fun setCameraEnabled(enabled: Boolean) {
        if (closed) return
        val track = videoTrack ?: return
        track.setEnabled(enabled)
        // The capturer follows the track: a disabled track with a running
        // camera drains the battery for frames nobody encodes.
        if (enabled) startCaptureIfAllowed() else stopCapture()
    }

    private fun startCaptureIfAllowed() {
        val capturer = videoCapturer ?: return
        if (capturing) return
        // Checked HERE as well as in the UI: a denied camera still places
        // or answers the call, camera off (protocol.md, "Video") — and
        // Camera2 throws a SecurityException without the grant, which
        // must never take the call down.
        val granted = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) ==
            PackageManager.PERMISSION_GRANTED
        if (!granted) return
        runCatching { capturer.startCapture(1280, 720, 30) }
            .onSuccess { capturing = true }
            .onFailure { Log.w(TAG, "startCapture failed", it) }
    }

    private fun stopCapture() {
        val capturer = videoCapturer ?: return
        if (!capturing) return
        capturing = false
        runCatching { capturer.stopCapture() }
    }

    override fun flipCamera() {
        if (closed) return
        videoCapturer?.switchCamera(null)
    }

    override fun setLocalVideoSink(sink: VideoSink?) {
        if (closed) return
        val track = videoTrack ?: return
        localSink?.let(track::removeSink)
        localSink = sink
        sink?.let(track::addSink)
    }

    override fun setRemoteVideoSink(sink: VideoSink?) {
        if (closed) return
        val track = remoteVideoTrack
        if (track != null) {
            remoteSink?.let(track::removeSink)
            sink?.let(track::addSink)
        }
        // Kept either way: the remote track usually arrives AFTER the
        // screen registered its renderer — onAddTrack attaches it then.
        remoteSink = sink
    }

    override fun close() {
        if (closed) return
        closed = true
        // The teardown ORDER is load-bearing — each of these out of order
        // is a known native crash, so do not "simplify":
        //   1. Detach sinks first — a renderer drawing from a track being
        //      disposed is a use-after-free. The renderers themselves are
        //      the UI's to release, never ours.
        //   2. capturer.stopCapture() BEFORE any dispose: disposing a
        //      RUNNING capturer (or its SurfaceTextureHelper) aborts in
        //      the camera thread.
        //   3. connection.close() then connection.dispose() — dispose
        //      releases the senders and their tracks, so the tracks are
        //      never disposed by hand (that is the classic double-free).
        //   4. The capturer, its SurfaceTextureHelper and the sources go
        //      AFTER the connection that consumed their frames.
        runCatching { localSink?.let { videoTrack?.removeSink(it) } }
        runCatching { remoteSink?.let { remoteVideoTrack?.removeSink(it) } }
        remoteVideoTrack = null
        remoteAudioTrack = null
        runCatching { videoCapturer?.stopCapture() }
        capturing = false
        runCatching { connection.close() }
        runCatching { connection.dispose() }
        runCatching { videoCapturer?.dispose() }
        runCatching { surfaceTextureHelper?.dispose() }
        runCatching { videoSource?.dispose() }
        runCatching { audioSource.dispose() }
    }

    private companion object {
        const val TAG = "FcCallMedia"

        /** The `typ` token of a candidate line: host, srflx, prflx or relay. */
        fun candidateType(candidate: IceCandidate): String =
            candidate.sdp.substringAfter(" typ ", "?").substringBefore(' ')
    }

    /** The four SdpObserver callbacks, defaulted so each site overrides only what it awaits. */
    private abstract class SdpObserverAdapter : SdpObserver {
        override fun onCreateSuccess(description: SessionDescription) = Unit
        override fun onSetSuccess() = Unit
        override fun onCreateFailure(error: String?) = Unit
        override fun onSetFailure(error: String?) = Unit
    }
}

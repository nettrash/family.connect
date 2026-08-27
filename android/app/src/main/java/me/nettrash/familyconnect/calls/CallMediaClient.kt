/*
 * CallMediaClient.kt
 * Family Connect (Android)
 *
 * The seam between the call state machine and WebRTC. CallManager talks
 * to THIS — offers, answers, candidates, mute — and never to org.webrtc
 * directly, so the machine is unit-tested on the plain JVM with a fake
 * while the real client (WebRtcClient) is the only file that loads the
 * native library.
 */

package me.nettrash.familyconnect.calls

import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import me.nettrash.familyconnect.data.net.dto.IceServerDto
import org.webrtc.VideoSink

enum class SdpType { OFFER, ANSWER }

interface CallMediaClient {

    /** Gather a local offer and set it as the local description. */
    suspend fun createOffer(): String

    /** After [setRemoteDescription] of an offer: answer it and set it locally. */
    suspend fun createAnswer(): String

    suspend fun setRemoteDescription(type: SdpType, sdp: String)

    /** Only meaningful once the remote description is set — CallManager buffers until then. */
    fun addRemoteCandidate(candidate: IceCandidateDto)

    fun setMuted(muted: Boolean)

    /**
     * Turn the local camera on or off (docs/protocol.md, "Video": cameras
     * toggle by enabling/disabling the TRACK — no renegotiation, no
     * frame; the far side simply sees the stream stop). Off also stops
     * the capturer, for the battery. A no-op on a voice call.
     */
    fun setCameraEnabled(enabled: Boolean)

    /** Switch between the front and the back camera. A no-op on a voice call. */
    fun flipCamera()

    /**
     * Where the local camera's frames are drawn. [VideoSink] is a plain
     * interface — no native code behind it — which is what keeps this
     * seam fake-able on the plain JVM. null detaches; the renderer
     * itself is the UI's to release, never this client's.
     */
    fun setLocalVideoSink(sink: VideoSink?)

    /** Where the far side's video is drawn, once its track arrives. null detaches. */
    fun setRemoteVideoSink(sink: VideoSink?)

    /** Tear everything down. Idempotent. */
    fun close()

    /** Callbacks arrive on WebRTC's own threads; the receiver hops off them. */
    interface Listener {
        fun onLocalCandidate(candidate: IceCandidateDto)

        /** Media is flowing. */
        fun onConnected()

        /** The connection failed for good — not a transient "disconnected". */
        fun onFailed()

        /** The far side's video track arrived (true) or went away (false). */
        fun onRemoteVideoActive(active: Boolean)
    }

    fun interface Factory {
        /**
         * [video] fixes the call's KIND (docs/protocol.md, "Video"): a
         * video client owns a camera track and offers to receive video; a
         * voice client never does.
         */
        fun create(iceServers: List<IceServerDto>, video: Boolean, listener: Listener): CallMediaClient
    }
}

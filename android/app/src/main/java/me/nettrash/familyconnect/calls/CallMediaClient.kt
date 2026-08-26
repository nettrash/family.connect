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

    /** Tear everything down. Idempotent. */
    fun close()

    /** Callbacks arrive on WebRTC's own threads; the receiver hops off them. */
    interface Listener {
        fun onLocalCandidate(candidate: IceCandidateDto)

        /** Media is flowing. */
        fun onConnected()

        /** The connection failed for good — not a transient "disconnected". */
        fun onFailed()
    }

    fun interface Factory {
        fun create(iceServers: List<IceServerDto>, listener: Listener): CallMediaClient
    }
}

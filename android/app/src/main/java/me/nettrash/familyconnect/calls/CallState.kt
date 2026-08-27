/*
 * CallState.kt
 * Family Connect (Android)
 *
 * The one voice call this device can be on (docs/protocol.md, "Voice
 * calls": one call per person), as the state machine CallManager drives
 * and every screen draws.
 *
 *   Idle ─► Outgoing(ringing=false) ─► Outgoing(ringing=true) ─► Connecting ─► Active ─► Ended ─► Idle
 *   Idle ─► Incoming(hasOffer)       ─► Connecting ─► Active ─► Ended ─► Idle
 *
 * `Ended` lingers for a moment so the screen can say WHY before it goes,
 * and is otherwise as good as Idle: a new call may start from it.
 */

package me.nettrash.familyconnect.calls

sealed interface CallState {

    data object Idle : CallState

    /** Every state that names a call: the id, the direct chat, the other person. */
    sealed interface Live : CallState {
        val callId: String
        val chatId: Long
        val peerUserId: Long

        /**
         * Whether this is a VIDEO call — fixed the moment the call was
         * placed (docs/protocol.md, "Video"): cameras toggle mid-call,
         * the kind never does.
         */
        val video: Boolean
    }

    /**
     * I am calling. [ringing] flips when the server answers `call_ringing`
     * — before that the offer is merely on its way.
     */
    data class Outgoing(
        override val callId: String,
        override val chatId: Long,
        override val peerUserId: Long,
        override val video: Boolean = false,
        val ringing: Boolean = false,
    ) : Live

    /**
     * Somebody is calling me. [hasOffer] is false while the device knows
     * of the call only from the push that woke it: the offer itself
     * arrives over the socket, replayed at registration (protocol, "Late
     * arrivals"). [callerName] is what the push carried, for the
     * notification; the roster names the caller once the app is up.
     */
    data class Incoming(
        override val callId: String,
        override val chatId: Long,
        override val peerUserId: Long,
        override val video: Boolean = false,
        val callerName: String? = null,
        val hasOffer: Boolean = false,
    ) : Live

    /** Answered on one side or the other; the media is coming up. */
    data class Connecting(
        override val callId: String,
        override val chatId: Long,
        override val peerUserId: Long,
        override val video: Boolean = false,
        val incoming: Boolean = false,
    ) : Live

    data class Active(
        override val callId: String,
        override val chatId: Long,
        override val peerUserId: Long,
        override val video: Boolean = false,
        /** Epoch millis of the moment audio came up — the timer's zero. */
        val sinceMillis: Long,
    ) : Live

    /**
     * Over. Held for a moment so the reason can be read, then [Idle].
     * [durationSecs] is present when the call was ever active.
     */
    data class Ended(
        val callId: String?,
        val chatId: Long?,
        val peerUserId: Long?,
        val reason: CallEnding,
        val durationSecs: Int? = null,
        /** What kind of call it WAS — the linger line words itself with it. */
        val video: Boolean = false,
        /**
         * Whether THIS device placed the call. The linger line reads
         * differently by side, as on iOS: a timeout is "No answer" to
         * the caller and a missed call to the callee; a decline is
         * "Declined" to the caller and just the end of the call to the
         * one who declined.
         */
        val outgoing: Boolean = false,
    ) : CallState
}

/** Why a call ended — the wire's six reasons plus the refusals and local guards. */
enum class CallEnding {
    /** An answered call, hung up by either side. */
    HANGUP,
    /** The callee said no (on any device). */
    DECLINE,
    /** The caller gave up while it rang — or their socket closed. */
    CANCEL,
    /** Nobody answered within the ring timeout. */
    TIMEOUT,
    /** The media never came up, or died. */
    FAILED,
    /** Another of my devices took the call. */
    ANSWERED_ELSEWHERE,
    /** The server refused: I am already on a call (another device). */
    BUSY,
    /** The server refused: they are on a call. */
    PEER_BUSY,
    /** The server refused: nothing of theirs can be rung. */
    PEER_UNREACHABLE,
    /** Woken by a push, but no offer ever arrived over the socket. */
    NO_OFFER,
    /** The server would not signal calls at all (`calls_disabled`). */
    DISABLED,
    /** The server refused the offer: VIDEO calls specifically are off (`video_calls_disabled`). */
    VIDEO_DISABLED,
    ;

    companion object {
        /** A `call_end` reason string → the enum; unknown strings read as FAILED. */
        fun fromWire(reason: String): CallEnding = when (reason) {
            "hangup" -> HANGUP
            "decline" -> DECLINE
            "cancel" -> CANCEL
            "timeout" -> TIMEOUT
            "answered_elsewhere" -> ANSWERED_ELSEWHERE
            else -> FAILED
        }

        /** An `error` code answering a call frame → the enum. */
        fun fromErrorCode(code: String): CallEnding = when (code) {
            "call_busy" -> BUSY
            "peer_busy" -> PEER_BUSY
            "peer_unreachable" -> PEER_UNREACHABLE
            "calls_disabled" -> DISABLED
            "video_calls_disabled" -> VIDEO_DISABLED
            else -> FAILED
        }
    }
}

/** Ring timeout and the two guards, injectable so tests run on a virtual clock. */
data class CallTimings(
    /** How long an incoming call rings with nobody answering — protocol default 45 s. */
    val ringTimeoutMillis: Long = 45_000L,
    /** The caller's own safety net, well past the server's ring timeout. */
    val outgoingGuardMillis: Long = 90_000L,
    /** How long the media may take to come up after an answer. */
    val connectGuardMillis: Long = 30_000L,
    /** How long [CallState.Ended] stays on screen before [CallState.Idle]. */
    val lingerMillis: Long = 2_000L,
)

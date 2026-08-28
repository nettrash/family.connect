/*
 * TelecomTransitions.kt
 * Family Connect (Android)
 *
 * What the Telecom session has to be told as the call's state moves —
 * the pure half of TelecomCalls, so the mapping is pinned on the JVM
 * without a CallsManager. TelecomMirror is one session's memory of what
 * it has already said: an incoming call is ANSWERED exactly once (even
 * when the session came up to find it already connecting, or already
 * active — Telecom rejects setActive on a call it still thinks is
 * ringing), media coming up is ACTIVE once, and any way out is a
 * DISCONNECT with the platform's own reason code, which is what the
 * system call log and a watch's "missed call" read.
 *
 * Kept free of android.telecom types: the disconnect reason is our own
 * enum here and becomes a DisconnectCause at the edge.
 */

package me.nettrash.familyconnect.calls

object TelecomTransitions {

    /** The platform's disconnect reasons this app ever reports. */
    enum class Cause { LOCAL, REMOTE, REJECTED, MISSED, BUSY, ERROR }

    sealed interface Command {
        /** Telecom's `answer(callType)`: the person took the call here. */
        data class Answer(val video: Boolean) : Command

        /** Telecom's `setActive()`: media is up. */
        data object SetActive : Command

        /** Telecom's `disconnect(cause)`: the session is over. */
        data class Disconnect(val cause: Cause) : Command
    }

    /**
     * The platform reason for an ended call. The log reads MISSED for a
     * ring nobody here answered, REJECTED for one declined here, BUSY for
     * a refusal, ERROR for a call that never came up; a hang-up or a
     * cancel is LOCAL, the far side's decline or timeout is REMOTE.
     */
    fun cause(ended: CallState.Ended): Cause = when (ended.reason) {
        CallEnding.HANGUP -> Cause.LOCAL
        CallEnding.CANCEL -> Cause.LOCAL
        CallEnding.ANSWERED_ELSEWHERE -> Cause.LOCAL
        CallEnding.DECLINE -> if (ended.outgoing) Cause.REMOTE else Cause.REJECTED
        CallEnding.TIMEOUT, CallEnding.NO_OFFER -> if (ended.outgoing) Cause.REMOTE else Cause.MISSED
        CallEnding.BUSY, CallEnding.PEER_BUSY -> Cause.BUSY
        CallEnding.FAILED, CallEnding.PEER_UNREACHABLE, CallEnding.DISABLED, CallEnding.VIDEO_DISABLED -> Cause.ERROR
    }
}

/**
 * One Telecom session's view of one call: feed it every state the
 * manager emits, in order, and it says what to tell Telecom — nothing,
 * or one or two commands (an incoming call found already active is
 * answered and THEN activated).
 */
class TelecomMirror(private val callId: String, incoming: Boolean) {

    // Volatile: systemAnswered() arrives on the library's callback
    // coroutine, next() on the session's collector — different threads.
    /** An outgoing call is never "answered" here; Telecom activates it directly. */
    @Volatile
    private var answered = !incoming

    @Volatile
    private var active = false

    @Volatile
    private var over = false

    /**
     * Telecom answered the call itself (a watch, a headset): it moves the
     * call to active on its own, so neither Answer nor SetActive is owed.
     */
    fun systemAnswered() {
        answered = true
        active = true
    }

    fun next(state: CallState): List<TelecomTransitions.Command> {
        if (over) return emptyList()
        val commands = mutableListOf<TelecomTransitions.Command>()
        when (state) {
            is CallState.Idle -> commands += disconnect(TelecomTransitions.Cause.LOCAL)
            is CallState.Ended -> commands += disconnect(TelecomTransitions.cause(state))
            is CallState.Live -> when {
                // The manager moved on to another call without an Ended
                // for ours: this session must not outlive its call.
                state.callId != callId -> commands += disconnect(TelecomTransitions.Cause.LOCAL)
                state is CallState.Connecting -> if (state.incoming && !answered) {
                    answered = true
                    commands += TelecomTransitions.Command.Answer(state.video)
                }
                state is CallState.Active -> {
                    if (!answered) {
                        answered = true
                        commands += TelecomTransitions.Command.Answer(state.video)
                    }
                    if (!active) {
                        active = true
                        commands += TelecomTransitions.Command.SetActive
                    }
                }
                else -> Unit
            }
        }
        return commands
    }

    private fun disconnect(cause: TelecomTransitions.Cause): TelecomTransitions.Command {
        over = true
        return TelecomTransitions.Command.Disconnect(cause)
    }
}

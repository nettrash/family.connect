/*
 * TelecomTransitionsTest.kt
 * Family Connect (Android)
 *
 * One session's mirror, pinned: an incoming call taken here answers
 * exactly once — whether the session saw the ring, found it connecting,
 * or found it already active (then answer AND activate) — media up
 * activates once, a system answer owes nothing, every way out
 * disconnects once with the reason the call log reads, a foreign call
 * is this session's end, and the ordinary moves say nothing.
 */

package me.nettrash.familyconnect.calls

import com.google.common.truth.Truth.assertThat
import me.nettrash.familyconnect.calls.TelecomTransitions.Cause
import me.nettrash.familyconnect.calls.TelecomTransitions.Command
import org.junit.Test

class TelecomTransitionsTest {

    private val id = "6a1f0c3e-0000-4000-8000-000000000001"
    private val incoming = CallState.Incoming(id, 42L, 9L, hasOffer = true)
    private val outgoing = CallState.Outgoing(id, 42L, 9L)
    private fun connecting(incoming: Boolean, video: Boolean = false) =
        CallState.Connecting(id, 42L, 9L, video = video, incoming = incoming)
    private val active = CallState.Active(id, 42L, 9L, sinceMillis = 1L)
    private fun ended(reason: CallEnding, outgoing: Boolean) =
        CallState.Ended(id, 42L, 9L, reason, outgoing = outgoing)

    @Test
    fun anIncomingCallTakenHereAnswersOnceThenActivatesOnce() {
        val mirror = TelecomMirror(id, incoming = true)
        assertThat(mirror.next(incoming.copy(hasOffer = false))).isEmpty()
        assertThat(mirror.next(incoming)).isEmpty()
        assertThat(mirror.next(connecting(incoming = true))).containsExactly(Command.Answer(false))
        assertThat(mirror.next(connecting(incoming = true))).isEmpty()
        assertThat(mirror.next(active)).containsExactly(Command.SetActive)
        assertThat(mirror.next(active.copy(sinceMillis = 2L))).isEmpty()
        assertThat(mirror.next(ended(CallEnding.HANGUP, outgoing = false))).containsExactly(Command.Disconnect(Cause.LOCAL))
        // Over is over: nothing more, whatever comes.
        assertThat(mirror.next(CallState.Idle)).isEmpty()
    }

    @Test
    fun aSessionThatComesUpLateStillAnswersFirst() {
        // Already connecting when Telecom took the call.
        val late = TelecomMirror(id, incoming = true)
        assertThat(late.next(connecting(incoming = true, video = true))).containsExactly(Command.Answer(true))
        assertThat(late.next(active)).containsExactly(Command.SetActive)
        // Already ACTIVE when Telecom took the call: answer, then activate — in that order.
        val later = TelecomMirror(id, incoming = true)
        assertThat(later.next(active)).containsExactly(Command.Answer(false), Command.SetActive).inOrder()
    }

    @Test
    fun anOutgoingCallNeverAnswers() {
        val mirror = TelecomMirror(id, incoming = false)
        assertThat(mirror.next(outgoing)).isEmpty()
        assertThat(mirror.next(outgoing.copy(ringing = true))).isEmpty()
        assertThat(mirror.next(connecting(incoming = false))).isEmpty()
        assertThat(mirror.next(active)).containsExactly(Command.SetActive)
        assertThat(mirror.next(ended(CallEnding.HANGUP, outgoing = true))).containsExactly(Command.Disconnect(Cause.LOCAL))
    }

    @Test
    fun aSystemAnswerOwesNothingMore() {
        val mirror = TelecomMirror(id, incoming = true)
        assertThat(mirror.next(incoming)).isEmpty()
        mirror.systemAnswered()
        assertThat(mirror.next(connecting(incoming = true))).isEmpty()
        assertThat(mirror.next(active)).isEmpty()
        assertThat(mirror.next(ended(CallEnding.HANGUP, outgoing = false))).containsExactly(Command.Disconnect(Cause.LOCAL))
    }

    @Test
    fun everyWayOutDisconnectsWithTheLogsReason() {
        fun cause(reason: CallEnding, outgoing: Boolean): Cause =
            (TelecomMirror(id, incoming = !outgoing).next(ended(reason, outgoing)).single() as Command.Disconnect).cause
        assertThat(cause(CallEnding.HANGUP, outgoing = true)).isEqualTo(Cause.LOCAL)
        assertThat(cause(CallEnding.CANCEL, outgoing = true)).isEqualTo(Cause.LOCAL)
        assertThat(cause(CallEnding.DECLINE, outgoing = false)).isEqualTo(Cause.REJECTED)
        assertThat(cause(CallEnding.DECLINE, outgoing = true)).isEqualTo(Cause.REMOTE)
        assertThat(cause(CallEnding.TIMEOUT, outgoing = false)).isEqualTo(Cause.MISSED)
        assertThat(cause(CallEnding.NO_OFFER, outgoing = false)).isEqualTo(Cause.MISSED)
        assertThat(cause(CallEnding.TIMEOUT, outgoing = true)).isEqualTo(Cause.REMOTE)
        assertThat(cause(CallEnding.PEER_BUSY, outgoing = true)).isEqualTo(Cause.BUSY)
        assertThat(cause(CallEnding.BUSY, outgoing = true)).isEqualTo(Cause.BUSY)
        assertThat(cause(CallEnding.FAILED, outgoing = true)).isEqualTo(Cause.ERROR)
        assertThat(cause(CallEnding.PEER_UNREACHABLE, outgoing = true)).isEqualTo(Cause.ERROR)
        assertThat(cause(CallEnding.ANSWERED_ELSEWHERE, outgoing = false)).isEqualTo(Cause.LOCAL)
        assertThat(TelecomMirror(id, incoming = false).next(CallState.Idle)).containsExactly(Command.Disconnect(Cause.LOCAL))
    }

    @Test
    fun anotherCallIsThisSessionsEnd() {
        val mirror = TelecomMirror(id, incoming = false)
        val other = CallState.Outgoing("6a1f0c3e-0000-4000-8000-000000000002", 43L, 10L)
        assertThat(mirror.next(other)).containsExactly(Command.Disconnect(Cause.LOCAL))
        assertThat(mirror.next(active)).isEmpty()
    }
}

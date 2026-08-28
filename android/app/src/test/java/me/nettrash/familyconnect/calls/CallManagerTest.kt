/*
 * CallManagerTest.kt
 * Family Connect (Android)
 *
 * The call state machine against the protocol's rules (docs/protocol.md,
 * "Voice calls"), on a virtual clock with a fake socket, a fake media
 * client and a fake audio route: the outgoing and incoming happy paths
 * (the offer before AND after accept — the push-woken case), answered
 * elsewhere, decline, cancel, both guards, candidate buffering order,
 * a foreign call_id ignored, a duplicate offer ignored, and the local
 * one-call-per-person refusal.
 *
 * The manager works on the app scope, which here is the test scheduler's
 * dispatcher — so runCurrent() drives every launch, and advanceTimeBy()
 * the guards. (advanceUntilIdle would skip backgroundScope work; nothing
 * here lives there.)
 */

package me.nettrash.familyconnect.calls

import com.google.common.truth.Truth.assertThat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.test.StandardTestDispatcher
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import me.nettrash.familyconnect.data.net.ApiResult
import me.nettrash.familyconnect.data.net.dto.IceCandidateDto
import me.nettrash.familyconnect.data.net.dto.IceServerDto
import me.nettrash.familyconnect.data.net.dto.IceServersResponse
import me.nettrash.familyconnect.data.net.ws.CallEndReason
import me.nettrash.familyconnect.data.net.ws.ClientFrame
import me.nettrash.familyconnect.data.net.ws.ServerFrame
import me.nettrash.familyconnect.testutil.FakeChatApi
import me.nettrash.familyconnect.testutil.FakeChatSocket
import me.nettrash.familyconnect.util.Clock
import org.junit.After
import org.webrtc.VideoFrame
import org.webrtc.VideoSink
import org.junit.Before
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class CallManagerTest {

    private companion object {
        const val CHAT = 42L
        const val PEER = 9L
        const val CALL = "6a1f0c3e-0000-4000-8000-000000000001"
        const val NOON = 1_756_224_000_000L
    }

    private val dispatcher = StandardTestDispatcher()
    private val scope = CoroutineScope(dispatcher + SupervisorJob())
    private val socket = FakeChatSocket()
    private val chatApi = FakeChatApi()
    private val media = FakeMediaFactory()
    private val audio = FakeCallAudio()
    private var now = NOON
    private val timings = CallTimings(
        ringTimeoutMillis = 45_000L,
        outgoingGuardMillis = 90_000L,
        connectGuardMillis = 30_000L,
        lingerMillis = 2_000L,
    )
    private lateinit var manager: CallManager

    @Before
    fun setUp() {
        socket.setOpen(true)
        chatApi.iceServersResult = ApiResult.Ok(
            IceServersResponse(iceServers = listOf(IceServerDto(urls = listOf("stun:stun.example.com:3478")))),
        )
        manager = CallManager(
            socket = socket,
            chatApi = chatApi,
            mediaFactory = media,
            audio = audio,
            clock = Clock { now },
            timings = timings,
            scope = scope,
        )
    }

    @After
    fun tearDown() {
        scope.cancel()
    }

    private fun TestScope.offer(callId: String = CALL, video: Boolean = false) {
        socket.emit(
            ServerFrame.CallOffer(callId = callId, chatId = CHAT, fromUserId = PEER, sdp = "their-offer", video = video),
        )
        runCurrent()
    }

    private fun sentEnds(): List<ClientFrame.CallEnd> = socket.sent.filterIsInstance<ClientFrame.CallEnd>()

    private fun outgoingCallId(): String = (manager.state.value as CallState.Live).callId

    // -- Outgoing ------------------------------------------------------------------

    @Test
    fun anOutgoingCallGoesOfferRingingAnswerActiveHangUp() = runTest(dispatcher) {
        assertThat(manager.startCall(CHAT, PEER, video = false)).isTrue()
        runCurrent()

        val callId = outgoingCallId()
        assertThat(manager.state.value).isEqualTo(CallState.Outgoing(callId, CHAT, PEER, ringing = false))
        // ICE servers were fetched for THIS call, the media opened with
        // them, and the offer went up with the caller-minted id.
        assertThat(chatApi.iceServersCalls).isEqualTo(1)
        assertThat(media.created.single().iceServers.single().urls).containsExactly("stun:stun.example.com:3478")
        assertThat(socket.sent).containsExactly(ClientFrame.CallOffer(callId, CHAT, "local-offer"))
        assertThat(audio.begun).isEqualTo(1)
        assertThat(manager.isInCall.value).isTrue()

        socket.emit(ServerFrame.CallRinging(callId))
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Outgoing(callId, CHAT, PEER, ringing = true))

        socket.emit(ServerFrame.CallAnswer(callId, "their-answer"))
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Connecting(callId, CHAT, PEER, incoming = false))
        assertThat(media.created.single().remote).isEqualTo(SdpType.ANSWER to "their-answer")

        // A candidate after the answer goes straight in.
        val candidate = IceCandidateDto("candidate:1", "0", 0)
        socket.emit(ServerFrame.CallIce(callId, candidate))
        runCurrent()
        assertThat(media.created.single().remoteCandidates).containsExactly(candidate)

        // And a local one goes up, tagged with the call.
        media.created.single().listener.onLocalCandidate(IceCandidateDto("candidate:mine"))
        runCurrent()
        assertThat(socket.sent).contains(ClientFrame.CallIce(callId, IceCandidateDto("candidate:mine")))

        media.created.single().listener.onConnected()
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Active(callId, CHAT, PEER, sinceMillis = NOON))

        now = NOON + 222_000L
        manager.hangUp()
        runCurrent()
        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.HANGUP))
        assertThat(manager.state.value)
            .isEqualTo(CallState.Ended(callId, CHAT, PEER, CallEnding.HANGUP, durationSecs = 222, outgoing = true))
        assertThat(media.created.single().closed).isTrue()
        assertThat(audio.ended).isEqualTo(1)

        // Ended lingers, then Idle — and the socket may let go.
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Idle)
        assertThat(manager.isInCall.value).isFalse()
    }

    // -- The platform's hand on a call (TelecomCalls) ---------------------------------

    @Test
    fun telecomRefusingAnOutgoingCallEndsItBusyWithACancel() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()

        manager.systemRefused("6a1f0c3e-0000-4000-8000-00000000dead")
        runCurrent()
        assertThat(manager.state.value).isInstanceOf(CallState.Outgoing::class.java)

        manager.systemRefused(callId)
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.CANCEL))
        val ended = manager.state.value as CallState.Ended
        assertThat(ended.reason).isEqualTo(CallEnding.BUSY)
        assertThat(ended.outgoing).isTrue()
        assertThat(audio.ended).isEqualTo(1)
    }

    @Test
    fun systemRefusedIsOnlyForAnOutgoingCall() = runTest(dispatcher) {
        offer()
        manager.systemRefused(CALL)
        runCurrent()
        assertThat(manager.state.value).isInstanceOf(CallState.Incoming::class.java)
        assertThat(sentEnds()).isEmpty()
    }

    @Test
    fun holdMutesTheMicrophoneUntilTakenBackAndTheToggleStillWins() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val client = media.created.single()

        manager.setHeld(true)
        assertThat(client.mutedNow).isTrue()
        manager.setHeld(false)
        assertThat(client.mutedNow).isFalse()

        // Muted by the person, then held and released: still muted.
        manager.toggleMute()
        manager.setHeld(true)
        manager.setHeld(false)
        assertThat(client.mutedNow).isTrue()
        assertThat(manager.isMuted.value).isTrue()

        // Held, the toggle cannot open the microphone: the cellular call
        // in front must not be heard by the family.
        manager.setHeld(true)
        manager.toggleMute()
        assertThat(manager.isMuted.value).isFalse()
        assertThat(client.mutedNow).isTrue()
        manager.setHeld(false)
        assertThat(client.mutedNow).isFalse()

        // The system unmuting (a headset button) clears both.
        manager.setMuted(false)
        assertThat(client.mutedNow).isFalse()
        assertThat(manager.isMuted.value).isFalse()
    }

    @Test
    fun theSystemsRouteDrawsTheSpeakerToggleWithoutAskingForARoute() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        assertThat(manager.isSpeaker.value).isFalse()
        manager.onSystemRoute(speaker = true)
        assertThat(manager.isSpeaker.value).isTrue()
        assertThat(audio.speakerOn).isFalse()
        manager.onSystemRoute(speaker = false)
        assertThat(manager.isSpeaker.value).isFalse()
    }

    // -- Ringback ------------------------------------------------------------------

    @Test
    fun theRingbackSoundsFromRingingToTheAnswerAndNeverAgain() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()
        assertThat(audio.ringbackStarts).isEqualTo(0)

        socket.emit(ServerFrame.CallRinging(callId))
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(1)
        assertThat(audio.isRinging).isTrue()

        // A duplicate ringing frame does not restart the cadence.
        socket.emit(ServerFrame.CallRinging(callId))
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(1)

        socket.emit(ServerFrame.CallAnswer(callId, "their-answer"))
        runCurrent()
        assertThat(audio.isRinging).isFalse()
        assertThat(audio.ringbackStops).isEqualTo(1)

        media.created.single().listener.onConnected()
        runCurrent()
        manager.hangUp()
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(1)
        assertThat(audio.ringbackStops).isEqualTo(1)
    }

    @Test
    fun cancellingWhileItRingsBackSilencesTheRingback() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        socket.emit(ServerFrame.CallRinging(outgoingCallId()))
        runCurrent()
        assertThat(audio.isRinging).isTrue()

        manager.hangUp()
        runCurrent()
        assertThat(audio.isRinging).isFalse()
        assertThat(audio.ringbackStops).isEqualTo(1)
        // The idle reset adds nothing.
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()
        assertThat(audio.ringbackStops).isEqualTo(1)
    }

    @Test
    fun aRemoteDeclineOrTimeoutSilencesTheRingback() = runTest(dispatcher) {
        for (reason in listOf("decline", "timeout")) {
            val starts = audio.ringbackStarts
            manager.startCall(CHAT, PEER, video = false)
            runCurrent()
            val callId = outgoingCallId()
            socket.emit(ServerFrame.CallRinging(callId))
            runCurrent()
            assertThat(audio.isRinging).isTrue()
            socket.emit(ServerFrame.CallEnd(callId, reason))
            runCurrent()
            assertThat(audio.isRinging).isFalse()
            assertThat(audio.ringbackStarts).isEqualTo(starts + 1)
            advanceTimeBy(timings.lingerMillis + 1)
            runCurrent()
        }
    }

    @Test
    fun theOutgoingGuardSilencesTheRingback() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        socket.emit(ServerFrame.CallRinging(outgoingCallId()))
        runCurrent()
        assertThat(audio.isRinging).isTrue()

        advanceTimeBy(timings.outgoingGuardMillis + 1)
        runCurrent()
        assertThat(audio.isRinging).isFalse()
    }

    @Test
    fun aRefusalSilencesTheRingbackAndNeverStartsItBeforeRinging() = runTest(dispatcher) {
        // Refused after ringing (a server race): silenced.
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        var callId = outgoingCallId()
        socket.emit(ServerFrame.CallRinging(callId))
        runCurrent()
        socket.emit(ServerFrame.Error(code = "peer_busy", message = "…", callId = callId))
        runCurrent()
        assertThat(audio.isRinging).isFalse()
        assertThat(audio.ringbackStarts).isEqualTo(1)
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()

        // Refused before ringing: nothing ever sounded.
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        callId = outgoingCallId()
        socket.emit(ServerFrame.Error(code = "peer_busy", message = "…", callId = callId))
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(1)
        assertThat(audio.ringbackStops).isEqualTo(1)
    }

    @Test
    fun anIncomingCallNeverRingsBack() = runTest(dispatcher) {
        offer()
        manager.accept()
        runCurrent()
        media.created.single().listener.onConnected()
        runCurrent()
        manager.hangUp()
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(0)
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()

        offer(callId = "6a1f0c3e-0000-4000-8000-000000000002")
        manager.decline()
        runCurrent()
        assertThat(audio.ringbackStarts).isEqualTo(0)
    }

    @Test
    fun cancellingWhileItRingsSendsCancel() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()

        manager.hangUp()
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.CANCEL))
        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.CANCEL)
        assertThat((manager.state.value as CallState.Ended).durationSecs).isNull()
    }

    @Test
    fun thePeerBeingBusyEndsTheCallWithoutAnEndFrame() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()

        socket.emit(ServerFrame.Error(code = "peer_busy", message = "…", callId = callId))
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.PEER_BUSY)
        // Nothing was rung; there is nothing to end.
        assertThat(sentEnds()).isEmpty()
    }

    @Test
    fun anOutgoingCallNobodyAnswersGivesUpOnItsOwnGuard() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()

        advanceTimeBy(timings.outgoingGuardMillis + 1)
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.CANCEL))
        val ended = manager.state.value as CallState.Ended
        assertThat(ended.reason).isEqualTo(CallEnding.TIMEOUT)
        // The caller's side of a timeout: the linger line says "No answer".
        assertThat(ended.outgoing).isTrue()
    }

    @Test
    fun thePeerDecliningEndsAnOutgoingCallOnTheCallersSide() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = true)
        runCurrent()
        val callId = outgoingCallId()

        socket.emit(ServerFrame.CallEnd(callId, CallEndReason.DECLINE))
        runCurrent()

        val ended = manager.state.value as CallState.Ended
        assertThat(ended.reason).isEqualTo(CallEnding.DECLINE)
        // "Declined" to the one who placed the call — the callee sees
        // "Call ended" (CallNotifications.endedTextRes).
        assertThat(ended.outgoing).isTrue()
        assertThat(ended.video).isTrue()
    }

    @Test
    fun theDirectionDoesNotLeakFromOneCallIntoTheNext() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        manager.hangUp()
        runCurrent()
        assertThat((manager.state.value as CallState.Ended).outgoing).isTrue()

        // A new call may start from Ended — an incoming one, this time.
        offer()
        manager.decline()
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).outgoing).isFalse()
    }

    @Test
    fun aSecondCallIsRefusedWhileOneIsLive() = runTest(dispatcher) {
        assertThat(manager.startCall(CHAT, PEER, video = false)).isTrue()
        runCurrent()

        // One call per person — locally as well as on the server.
        assertThat(manager.startCall(CHAT, PEER, video = false)).isFalse()
        runCurrent()
        assertThat(chatApi.iceServersCalls).isEqualTo(1)
        assertThat(media.created).hasSize(1)
    }

    @Test
    fun iceServersAreFetchedAfreshForEveryCall() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        manager.hangUp()
        runCurrent()
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()

        manager.startCall(CHAT, PEER, video = false)
        runCurrent()

        assertThat(chatApi.iceServersCalls).isEqualTo(2)
    }

    @Test
    fun aDeadMediaConnectionIsReportedAsFailed() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val callId = outgoingCallId()
        socket.emit(ServerFrame.CallAnswer(callId, "their-answer"))
        runCurrent()

        media.created.single().listener.onFailed()
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.FAILED))
        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.FAILED)
    }

    // -- Incoming ------------------------------------------------------------------

    @Test
    fun anIncomingOfferRingsAndAcceptAnswersItWithTheBufferedCandidates() = runTest(dispatcher) {
        offer()
        assertThat(manager.state.value).isEqualTo(CallState.Incoming(CALL, CHAT, PEER, callerName = null, hasOffer = true))
        assertThat(manager.isInCall.value).isTrue()

        // The caller's candidates arrive before we have anywhere to put
        // them: buffered, in order, until the remote description is set.
        val first = IceCandidateDto("candidate:1", "0", 0)
        val second = IceCandidateDto("candidate:2", "0", 0)
        socket.emit(ServerFrame.CallIce(CALL, first))
        socket.emit(ServerFrame.CallIce(CALL, second))
        runCurrent()
        assertThat(media.created).isEmpty()

        manager.accept()
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, incoming = true))
        val client = media.created.single()
        assertThat(client.remote).isEqualTo(SdpType.OFFER to "their-offer")
        assertThat(client.remoteCandidates).containsExactly(first, second).inOrder()
        assertThat(socket.sent).contains(ClientFrame.CallAnswer(CALL, "local-answer"))
        assertThat(chatApi.iceServersCalls).isEqualTo(1)

        client.listener.onConnected()
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Active(CALL, CHAT, PEER, sinceMillis = NOON))
    }

    @Test
    fun aPushWokenPhoneAnswersTheMomentTheReplayedOfferArrives() = runTest(dispatcher) {
        manager.onIncomingPush(CALL, CHAT, PEER, callerName = "Anna")
        runCurrent()
        assertThat(manager.state.value)
            .isEqualTo(CallState.Incoming(CALL, CHAT, PEER, callerName = "Anna", hasOffer = false))

        // Answered before the offer is here: nothing can be sent yet.
        manager.accept()
        runCurrent()
        assertThat(manager.state.value).isInstanceOf(CallState.Incoming::class.java)
        assertThat(socket.sent).isEmpty()

        offer()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, incoming = true))
        assertThat(socket.sent).contains(ClientFrame.CallAnswer(CALL, "local-answer"))
    }

    @Test
    fun aDuplicateOfferDoesNothing() = runTest(dispatcher) {
        offer()
        manager.accept()
        runCurrent()

        // The replay on connect, after the live frame already delivered it.
        offer()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, incoming = true))
        assertThat(media.created).hasSize(1)
        assertThat(socket.sent.filterIsInstance<ClientFrame.CallAnswer>()).hasSize(1)
    }

    @Test
    fun answeredElsewhereStopsTheRinging() = runTest(dispatcher) {
        offer()

        socket.emit(ServerFrame.CallEnd(CALL, CallEndReason.ANSWERED_ELSEWHERE))
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.ANSWERED_ELSEWHERE)
        assertThat(sentEnds()).isEmpty()
    }

    @Test
    fun decliningSendsDeclineAndEnds() = runTest(dispatcher) {
        offer()

        manager.decline()
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(CALL, CallEndReason.DECLINE))
        val ended = manager.state.value as CallState.Ended
        assertThat(ended.reason).isEqualTo(CallEnding.DECLINE)
        // I declined: my linger line is "Call ended", not "Declined".
        assertThat(ended.outgoing).isFalse()
        assertThat(media.created).isEmpty()
        assertThat(audio.begun).isEqualTo(0)
    }

    @Test
    fun theCallerCancellingEndsAnIncomingCall() = runTest(dispatcher) {
        offer()

        socket.emit(ServerFrame.CallEnd(CALL, CallEndReason.CANCEL))
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.CANCEL)
    }

    @Test
    fun anIncomingCallNobodyAnswersRingsOutOnTheGuard() = runTest(dispatcher) {
        offer()

        advanceTimeBy(timings.ringTimeoutMillis + 1)
        runCurrent()

        val ended = manager.state.value as CallState.Ended
        assertThat(ended.reason).isEqualTo(CallEnding.TIMEOUT)
        // The callee's side of a timeout: a MISSED call, not "No answer".
        assertThat(ended.outgoing).isFalse()
    }

    @Test
    fun aPushWithNoOfferBehindItGivesUpOnTheGuard() = runTest(dispatcher) {
        manager.onIncomingPush(CALL, CHAT, PEER, callerName = "Anna")
        runCurrent()

        advanceTimeBy(timings.ringTimeoutMillis + 1)
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.NO_OFFER)
    }

    @Test
    fun aPushForTheCallAlreadyRingingIsANoOp() = runTest(dispatcher) {
        offer()

        manager.onIncomingPush(CALL, CHAT, PEER, callerName = "Anna")
        runCurrent()

        // The offer held is not forgotten for a push that carried none.
        assertThat(manager.state.value).isEqualTo(CallState.Incoming(CALL, CHAT, PEER, callerName = null, hasOffer = true))
    }

    // -- The frame rule ---------------------------------------------------------------

    @Test
    fun framesForACallThisDeviceDoesNotHoldAreIgnored() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val mine = outgoingCallId()

        socket.emit(ServerFrame.CallEnd("someone-elses-call", CallEndReason.HANGUP))
        socket.emit(ServerFrame.CallAnswer("someone-elses-call", "sdp"))
        socket.emit(ServerFrame.CallIce("someone-elses-call", IceCandidateDto("candidate:x")))
        socket.emit(ServerFrame.Error(code = "peer_busy", message = "…", callId = "someone-elses-call"))
        socket.emit(ServerFrame.Error(code = "not_chat_member", message = "…", clientMsgId = "8f14e45f"))
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Outgoing(mine, CHAT, PEER, ringing = false))
        assertThat(media.created.single().remote).isNull()
        assertThat(media.created.single().remoteCandidates).isEmpty()
    }

    @Test
    fun anOfferWhileOnACallIsIgnored() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()
        val mine = outgoingCallId()

        offer("another")

        assertThat(manager.state.value).isEqualTo(CallState.Outgoing(mine, CHAT, PEER, ringing = false))
    }

    @Test
    fun aClosedSocketFailsTheCallRatherThanRingingNobody() = runTest(dispatcher) {
        socket.setOpen(false)

        manager.startCall(CHAT, PEER, video = false)
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.FAILED)
        assertThat(media.created.single().closed).isTrue()
        assertThat(audio.ended).isEqualTo(1)
    }

    @Test
    fun muteAndSpeakerReachTheMediaAndTheRouteAndResetAtTheEnd() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()

        manager.toggleMute()
        manager.toggleSpeaker()
        assertThat(media.created.single().mutedNow).isTrue()
        assertThat(audio.speakerOn).isTrue()
        assertThat(manager.isMuted.value).isTrue()
        assertThat(manager.isSpeaker.value).isTrue()

        manager.hangUp()
        runCurrent()
        assertThat(manager.isMuted.value).isFalse()
        assertThat(manager.isSpeaker.value).isFalse()
        assertThat(audio.speakerOn).isFalse()
    }

    // -- Video (docs/protocol.md, "Video") -----------------------------------------

    @Test
    fun aVideoCallThreadsItsKindToTheMediaTheFrameTheStateAndTheAudio() = runTest(dispatcher) {
        assertThat(manager.startCall(CHAT, PEER, video = true)).isTrue()
        runCurrent()

        val callId = outgoingCallId()
        assertThat(manager.state.value)
            .isEqualTo(CallState.Outgoing(callId, CHAT, PEER, video = true, ringing = false))
        // The media client is built AS a video one, and the offer frame
        // says the kind — fixed here, for the call's life.
        assertThat(media.created.single().video).isTrue()
        assertThat(socket.sent).containsExactly(ClientFrame.CallOffer(callId, CHAT, "local-offer", video = true))
        // Video defaults the SPEAKER on, and the camera comes up with it.
        assertThat(audio.lastBeginVideo).isTrue()
        assertThat(manager.isSpeaker.value).isTrue()
        assertThat(manager.isCameraOn.value).isTrue()
        assertThat(media.created.single().cameraEnabled).isTrue()
    }

    @Test
    fun aVoiceCallLeavesTheSpeakerAndTheCameraAlone() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = false)
        runCurrent()

        // The voice path is byte-stable: the existing frame assertions in
        // the tests above pin the offer WITHOUT a video key; here, none
        // of the video machinery so much as twitches.
        assertThat(audio.lastBeginVideo).isFalse()
        assertThat(manager.isSpeaker.value).isFalse()
        assertThat(manager.isCameraOn.value).isFalse()
        assertThat(media.created.single().video).isFalse()
        assertThat(media.created.single().cameraEnabled).isNull()
    }

    @Test
    fun cameraAndFlipDelegateToTheMediaClientAndResetAtTheEnd() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = true)
        runCurrent()

        manager.toggleCamera()
        assertThat(manager.isCameraOn.value).isFalse()
        assertThat(media.created.single().cameraEnabled).isFalse()
        manager.toggleCamera()
        assertThat(manager.isCameraOn.value).isTrue()
        assertThat(media.created.single().cameraEnabled).isTrue()

        manager.flipCamera()
        assertThat(manager.isFrontCamera.value).isFalse()
        assertThat(media.created.single().flips).isEqualTo(1)

        manager.hangUp()
        runCurrent()
        assertThat(manager.isCameraOn.value).isFalse()
        assertThat(manager.isFrontCamera.value).isTrue()
    }

    @Test
    fun aVideoPushRingsAVideoIncomingAndAnswersItEndToEnd() = runTest(dispatcher) {
        manager.onIncomingPush(CALL, CHAT, PEER, callerName = "Anna", video = true)
        runCurrent()
        assertThat(manager.state.value)
            .isEqualTo(CallState.Incoming(CALL, CHAT, PEER, video = true, callerName = "Anna", hasOffer = false))

        offer(video = true)
        manager.accept()
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, video = true, incoming = true))
        assertThat(media.created.single().video).isTrue()
        assertThat(audio.lastBeginVideo).isTrue()
        assertThat(manager.isSpeaker.value).isTrue()
    }

    @Test
    fun cameraOffBeforeAcceptAnswersWithTheCameraOff() = runTest(dispatcher) {
        offer(video = true)
        assertThat(manager.isCameraOn.value).isTrue()

        // The screen's camera-permission-denied path: the call is still
        // answered, the camera stays off (protocol.md, "Video").
        manager.setCameraEnabled(false)
        manager.accept()
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, video = true, incoming = true))
        assertThat(media.created.single().cameraEnabled).isFalse()
        assertThat(manager.isCameraOn.value).isFalse()
    }

    @Test
    fun remoteVideoWaitsForTheFirstFrameNotTheTrack() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = true)
        runCurrent()
        val frames = mutableListOf<VideoFrame>()
        manager.setRemoteVideoSink(VideoSink(frames::add))

        // The track ARRIVING is not a picture — onAddTrack fires at connect
        // time on every video call, frames or not (a far side with the
        // camera off or denied never sends any). The avatar stays.
        media.created.single().listener.onRemoteVideoActive(true)
        runCurrent()
        assertThat(manager.remoteVideoActive.value).isFalse()

        // The first REAL frame flips it — and still reaches the UI's sink
        // through the manager's forwarding wrapper.
        media.created.single().remoteSink!!.onFrame(fakeVideoFrame())
        runCurrent()
        assertThat(manager.remoteVideoActive.value).isTrue()
        assertThat(frames).hasSize(1)

        // The track going away still clears it.
        media.created.single().listener.onRemoteVideoActive(false)
        runCurrent()
        assertThat(manager.remoteVideoActive.value).isFalse()
    }

    @Test
    fun remoteVideoActivityClearsAtTheEnd() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = true)
        runCurrent()
        manager.setRemoteVideoSink(VideoSink {})

        media.created.single().remoteSink!!.onFrame(fakeVideoFrame())
        runCurrent()
        assertThat(manager.remoteVideoActive.value).isTrue()

        manager.hangUp()
        runCurrent()
        assertThat(manager.remoteVideoActive.value).isFalse()
    }

    @Test
    fun grantingTheCameraInTheAnswerDialogStartsTheCameraOn() = runTest(dispatcher) {
        offer(video = true)
        // The screen's no-grant-yet enforcement ran while the call rang…
        manager.setCameraEnabled(false)

        // …and the dialog's grant rides into the answer (parity with iOS).
        manager.accept(cameraGranted = true)
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, video = true, incoming = true))
        assertThat(manager.isCameraOn.value).isTrue()
        assertThat(media.created.single().cameraEnabled).isTrue()
    }

    @Test
    fun denyingTheCameraInTheAnswerDialogAnswersWithTheCameraOff() = runTest(dispatcher) {
        offer(video = true)

        manager.accept(cameraGranted = false)
        runCurrent()

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, video = true, incoming = true))
        assertThat(manager.isCameraOn.value).isFalse()
        assertThat(media.created.single().cameraEnabled).isFalse()
    }

    @Test
    fun aPushWokenAnswerCarriesTheCameraGrantToTheLateOffer() = runTest(dispatcher) {
        // The notification's Answer path: push first, dialog's grant next,
        // the offer only then — the grant must survive the wait.
        manager.onIncomingPush(CALL, CHAT, PEER, callerName = "Anna", video = true)
        runCurrent()
        manager.setCameraEnabled(false)
        manager.accept(cameraGranted = true)
        runCurrent()
        assertThat(socket.sent).isEmpty()

        offer(video = true)

        assertThat(manager.state.value).isEqualTo(CallState.Connecting(CALL, CHAT, PEER, video = true, incoming = true))
        assertThat(manager.isCameraOn.value).isTrue()
        assertThat(media.created.single().cameraEnabled).isTrue()
    }

    @Test
    fun videoCallsBeingDisabledEndsWithItsOwnReason() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER, video = true)
        runCurrent()
        val callId = outgoingCallId()

        socket.emit(ServerFrame.Error(code = "video_calls_disabled", message = "…", callId = callId))
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.VIDEO_DISABLED)
        // The server refused the offer; there is nothing to end.
        assertThat(sentEnds()).isEmpty()
    }
}

/** A 2×2 frame whose buffer does nothing — enough for the first-frame signal. */
private fun fakeVideoFrame(): VideoFrame = VideoFrame(
    object : VideoFrame.Buffer {
        override fun getWidth(): Int = 2
        override fun getHeight(): Int = 2
        override fun toI420(): VideoFrame.I420Buffer? = null
        override fun retain() = Unit
        override fun release() = Unit
        override fun cropAndScale(
            cropX: Int,
            cropY: Int,
            cropWidth: Int,
            cropHeight: Int,
            scaleWidth: Int,
            scaleHeight: Int,
        ): VideoFrame.Buffer = this
    },
    0,
    0L,
)

// -- Fakes ------------------------------------------------------------------------

private class FakeMediaFactory : CallMediaClient.Factory {
    val created = mutableListOf<FakeMediaClient>()

    override fun create(
        iceServers: List<IceServerDto>,
        video: Boolean,
        listener: CallMediaClient.Listener,
    ): CallMediaClient = FakeMediaClient(iceServers, video, listener).also(created::add)
}

private class FakeMediaClient(
    val iceServers: List<IceServerDto>,
    val video: Boolean,
    val listener: CallMediaClient.Listener,
) : CallMediaClient {
    var remote: Pair<SdpType, String>? = null
    val remoteCandidates = mutableListOf<IceCandidateDto>()
    var mutedNow = false
    var closed = false

    /** null until setCameraEnabled is ever called — a voice call never calls it. */
    var cameraEnabled: Boolean? = null
    var flips = 0
    var localSink: VideoSink? = null
    var remoteSink: VideoSink? = null

    override suspend fun createOffer(): String = "local-offer"
    override suspend fun createAnswer(): String = "local-answer"

    override suspend fun setRemoteDescription(type: SdpType, sdp: String) {
        remote = type to sdp
    }

    override fun addRemoteCandidate(candidate: IceCandidateDto) {
        remoteCandidates += candidate
    }

    override fun setMuted(muted: Boolean) {
        mutedNow = muted
    }

    override fun setCameraEnabled(enabled: Boolean) {
        cameraEnabled = enabled
    }

    override fun flipCamera() {
        flips += 1
    }

    override fun setLocalVideoSink(sink: VideoSink?) {
        localSink = sink
    }

    override fun setRemoteVideoSink(sink: VideoSink?) {
        remoteSink = sink
    }

    override fun close() {
        closed = true
    }
}

private class FakeCallAudio : CallAudio {
    var begun = 0
    var ended = 0
    var speakerOn = false

    /** What the last begin() was told about the call's kind. */
    var lastBeginVideo: Boolean? = null

    override fun begin(video: Boolean) {
        begun += 1
        lastBeginVideo = video
    }

    override fun end() {
        ended += 1
        speakerOn = false
        // The contract: end() silences the ringback too (AndroidCallAudio).
        stopRingback()
    }

    override fun setSpeaker(on: Boolean) {
        speakerOn = on
    }

    /**
     * The ringback's audible history: a start while it already rings and
     * a stop while it does not are the idempotent no-ops the real track
     * makes them, and are NOT counted — so `ringbackStarts` / `ringbackStops`
     * read as what was heard.
     */
    var ringbackStarts = 0
    var ringbackStops = 0
    val isRinging: Boolean get() = ringbackStarts > ringbackStops

    override fun startRingback() {
        if (isRinging) return
        ringbackStarts += 1
    }

    override fun stopRingback() {
        if (!isRinging) return
        ringbackStops += 1
    }
}

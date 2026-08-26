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

    private fun TestScope.offer(callId: String = CALL) {
        socket.emit(ServerFrame.CallOffer(callId = callId, chatId = CHAT, fromUserId = PEER, sdp = "their-offer"))
        runCurrent()
    }

    private fun sentEnds(): List<ClientFrame.CallEnd> = socket.sent.filterIsInstance<ClientFrame.CallEnd>()

    private fun outgoingCallId(): String = (manager.state.value as CallState.Live).callId

    // -- Outgoing ------------------------------------------------------------------

    @Test
    fun anOutgoingCallGoesOfferRingingAnswerActiveHangUp() = runTest(dispatcher) {
        assertThat(manager.startCall(CHAT, PEER)).isTrue()
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
            .isEqualTo(CallState.Ended(callId, CHAT, PEER, CallEnding.HANGUP, durationSecs = 222))
        assertThat(media.created.single().closed).isTrue()
        assertThat(audio.ended).isEqualTo(1)

        // Ended lingers, then Idle — and the socket may let go.
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()
        assertThat(manager.state.value).isEqualTo(CallState.Idle)
        assertThat(manager.isInCall.value).isFalse()
    }

    @Test
    fun cancellingWhileItRingsSendsCancel() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER)
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
        manager.startCall(CHAT, PEER)
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
        manager.startCall(CHAT, PEER)
        runCurrent()
        val callId = outgoingCallId()

        advanceTimeBy(timings.outgoingGuardMillis + 1)
        runCurrent()

        assertThat(sentEnds()).containsExactly(ClientFrame.CallEnd(callId, CallEndReason.CANCEL))
        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.TIMEOUT)
    }

    @Test
    fun aSecondCallIsRefusedWhileOneIsLive() = runTest(dispatcher) {
        assertThat(manager.startCall(CHAT, PEER)).isTrue()
        runCurrent()

        // One call per person — locally as well as on the server.
        assertThat(manager.startCall(CHAT, PEER)).isFalse()
        runCurrent()
        assertThat(chatApi.iceServersCalls).isEqualTo(1)
        assertThat(media.created).hasSize(1)
    }

    @Test
    fun iceServersAreFetchedAfreshForEveryCall() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER)
        runCurrent()
        manager.hangUp()
        runCurrent()
        advanceTimeBy(timings.lingerMillis + 1)
        runCurrent()

        manager.startCall(CHAT, PEER)
        runCurrent()

        assertThat(chatApi.iceServersCalls).isEqualTo(2)
    }

    @Test
    fun aDeadMediaConnectionIsReportedAsFailed() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER)
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
        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.DECLINE)
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

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.TIMEOUT)
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
        manager.startCall(CHAT, PEER)
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
        manager.startCall(CHAT, PEER)
        runCurrent()
        val mine = outgoingCallId()

        offer("another")

        assertThat(manager.state.value).isEqualTo(CallState.Outgoing(mine, CHAT, PEER, ringing = false))
    }

    @Test
    fun aClosedSocketFailsTheCallRatherThanRingingNobody() = runTest(dispatcher) {
        socket.setOpen(false)

        manager.startCall(CHAT, PEER)
        runCurrent()

        assertThat((manager.state.value as CallState.Ended).reason).isEqualTo(CallEnding.FAILED)
        assertThat(media.created.single().closed).isTrue()
        assertThat(audio.ended).isEqualTo(1)
    }

    @Test
    fun muteAndSpeakerReachTheMediaAndTheRouteAndResetAtTheEnd() = runTest(dispatcher) {
        manager.startCall(CHAT, PEER)
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
}

// -- Fakes ------------------------------------------------------------------------

private class FakeMediaFactory : CallMediaClient.Factory {
    val created = mutableListOf<FakeMediaClient>()

    override fun create(iceServers: List<IceServerDto>, listener: CallMediaClient.Listener): CallMediaClient =
        FakeMediaClient(iceServers, listener).also(created::add)
}

private class FakeMediaClient(
    val iceServers: List<IceServerDto>,
    val listener: CallMediaClient.Listener,
) : CallMediaClient {
    var remote: Pair<SdpType, String>? = null
    val remoteCandidates = mutableListOf<IceCandidateDto>()
    var mutedNow = false
    var closed = false

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

    override fun close() {
        closed = true
    }
}

private class FakeCallAudio : CallAudio {
    var begun = 0
    var ended = 0
    var speakerOn = false

    override fun begin() {
        begun += 1
    }

    override fun end() {
        ended += 1
        speakerOn = false
    }

    override fun setSpeaker(on: Boolean) {
        speakerOn = on
    }
}

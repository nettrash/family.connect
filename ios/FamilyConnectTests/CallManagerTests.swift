//
//  CallManagerTests.swift
//  FamilyConnectTests
//
//  The call state machine over fakes: a signalling sink that records
//  frames, a media client that never opens a microphone, and an optional
//  system bridge standing in for CallKit. Nothing here touches a socket.
//

import Foundation
import Testing
// For RTCVideoRenderer alone — the renderer fake below is two empty
// methods, no media stack involved.
import WebRTC
@testable import FamilyConnect

@MainActor
@Suite("Call manager")
struct CallManagerTests {

    // MARK: - Fakes

    @MainActor
    final class FakeSignaling: CallSignaling {
        var sent: [ClientFrame] = []
        func sendCallFrame(_ frame: ClientFrame) async throws {
            sent.append(frame)
        }
        /// `"<call id>:<reason>"` per call_end frame, in order.
        var ends: [String] {
            sent.compactMap {
                if case .callEnd(let id, let reason) = $0 { return "\(id):\(reason)" }
                return nil
            }
        }
    }

    @MainActor
    final class FakeMedia: CallMediaClient {
        weak var delegate: (any CallMediaClientDelegate)?
        var remote: [(sdp: String, isOffer: Bool)] = []
        var remoteCandidates: [IceCandidatePayload] = []
        var muted: Bool?
        var closed = false
        /// Emitted from inside createOffer/createAnswer, i.e. BEFORE the
        /// description has gone out — the buffering case.
        var candidatesDuringLocalDescription: [IceCandidatePayload] = []

        func createOffer() async throws -> String {
            for candidate in candidatesDuringLocalDescription {
                delegate?.mediaClient(self, didGatherLocalCandidate: candidate)
            }
            return "v=0 offer"
        }
        func createAnswer() async throws -> String {
            for candidate in candidatesDuringLocalDescription {
                delegate?.mediaClient(self, didGatherLocalCandidate: candidate)
            }
            return "v=0 answer"
        }
        func setRemoteDescription(sdp: String, isOffer: Bool) async throws {
            remote.append((sdp, isOffer))
        }
        func addRemoteCandidate(_ candidate: IceCandidatePayload) async throws {
            remoteCandidates.append(candidate)
        }
        func setMuted(_ muted: Bool) { self.muted = muted }
        func setSpeaker(_ enabled: Bool) {}
        func close() { closed = true }

        // Video: every camera/renderer call recorded for assertions.
        var cameraEnabled: [Bool] = []
        var flips = 0
        /// true = a renderer attached, false = detached (nil).
        var localRenderers: [Bool] = []
        var remoteRenderers: [Bool] = []
        func setCameraEnabled(_ enabled: Bool) { cameraEnabled.append(enabled) }
        func flipCamera() { flips += 1 }
        func setLocalVideoRenderer(_ renderer: (any RTCVideoRenderer)?) { localRenderers.append(renderer != nil) }
        func setRemoteVideoRenderer(_ renderer: (any RTCVideoRenderer)?) { remoteRenderers.append(renderer != nil) }

        func emit(_ state: CallMediaConnectionState) {
            delegate?.mediaClient(self, connectionStateChanged: state)
        }
        func emitLocal(_ candidate: IceCandidatePayload) {
            delegate?.mediaClient(self, didGatherLocalCandidate: candidate)
        }
        func emitRemoteVideo(_ active: Bool) {
            delegate?.mediaClient(self, remoteVideoActiveChanged: active)
        }
    }

    /// The two-method RTCVideoRenderer, so renderer plumbing is testable
    /// without a Metal view.
    final class FakeRenderer: NSObject, RTCVideoRenderer {
        func setSize(_ size: CGSize) {}
        func renderFrame(_ frame: RTCVideoFrame?) {}
    }

    /// A camera prompt that hangs until the test answers it — the
    /// undetermined-TCC alert, which on a locked phone may never resolve.
    @MainActor
    final class CameraGate {
        private var continuation: CheckedContinuation<Bool, Never>?
        private(set) var asked = false
        func request() async -> Bool {
            asked = true
            return await withCheckedContinuation { continuation = $0 }
        }
        func resolve(_ granted: Bool) {
            continuation?.resume(returning: granted)
            continuation = nil
        }
    }

    @MainActor
    final class FakeBridge: CallSystemBridge {
        weak var manager: CallManager?
        var events: [String] = []
        func reportOutgoing(callID: UUID, peerName: String, isVideo: Bool) {
            events.append(isVideo ? "outgoing:\(peerName):video" : "outgoing:\(peerName)")
        }
        func reportOutgoingConnecting(callID: UUID) { events.append("connecting") }
        func reportOutgoingConnected(callID: UUID) { events.append("connected") }
        func reportIncoming(callID: UUID, peerName: String, hasVideo: Bool) {
            events.append(hasVideo ? "incoming:\(peerName):video" : "incoming:\(peerName)")
        }
        func reportEnded(callID: UUID, reason: CallEndReason) { events.append("ended:\(reason.rawValue)") }
        func requestAnswer(callID: UUID) {
            events.append("requestAnswer")
            manager?.systemDidAnswer()
        }
        func requestEnd(callID: UUID) {
            events.append("requestEnd")
            manager?.systemDidEnd()
        }
    }

    @MainActor
    final class Harness {
        let manager = CallManager()
        let signaling = FakeSignaling()
        let media = FakeMedia()
        var bridge: FakeBridge?
        var ensureConnectedCalls = 0
        var endedCalls = 0
        var phases: [CallManager.Phase] = []
        /// The `video` handed to makeMediaClient, per creation.
        var mediaVideo: [Bool] = []

        init(bridge: Bool = false, microphone: Bool = true, camera: Bool = true) {
            manager.signaling = signaling
            manager.makeMediaClient = { [weak self, media] _, video in
                self?.mediaVideo.append(video)
                return media
            }
            manager.iceServers = { [IceServerDTO(urls: ["stun:stun.example.com:3478"])] }
            manager.requestMicrophone = { microphone }
            manager.requestCamera = { camera }
            // Determined either way by default; tests for the undecided
            // grant override this with nil.
            manager.cameraGrantState = { camera }
            manager.resolvePeer = { _ in ("Anna", 3) }
            // Long enough that no guard fires under a loaded parallel run;
            // the two guard tests lower it themselves.
            manager.ringTimeout = 5
            manager.guardSlack = 0
            manager.endedLinger = 0
            // Weak, not unowned: a guard clock can fire after a test has
            // let its harness go, and the manager's own Task keeps the
            // manager alive past the harness.
            manager.ensureConnected = { [weak self] in self?.ensureConnectedCalls += 1 }
            manager.onEnded = { [weak self] in self?.endedCalls += 1 }
            manager.onPhaseChange = { [weak self] in self?.phases.append($0) }
            if bridge {
                let b = FakeBridge()
                b.manager = manager
                manager.systemBridge = b
                self.bridge = b
            }
        }

        /// Let detached best-effort sends (call_end, call_ice) land.
        func drain() async {
            await manager.settle()
            for _ in 0..<4 { await Task.yield() }
        }

        /// Wait for a clock-driven transition without betting on how loaded
        /// the machine is: poll, bounded, rather than sleep a fixed time.
        func waitUntil(_ condition: @MainActor () -> Bool, timeout: TimeInterval = 5) async {
            let deadline = Date().addingTimeInterval(timeout)
            while !condition() && Date() < deadline {
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    private let candidateA = IceCandidatePayload(candidate: "candidate:a", sdpMid: "0", sdpMLineIndex: 0)
    private let candidateB = IceCandidatePayload(candidate: "candidate:b", sdpMid: "0", sdpMLineIndex: 0)
    private let remoteID = "6a1f0c3e-0000-4000-8000-000000000001"

    private func offer(id: String, from: Int64 = 9) -> ServerFrame {
        .callOffer(CallOfferPayload(callID: id, chatID: 42, fromUserID: from, sdp: "v=0 remote offer"))
    }

    // MARK: - Outgoing

    @Test("outgoing: offer, ringing, answer, candidates both ways, connected, hang up")
    func outgoingHappyPath() async throws {
        let h = Harness()
        h.media.candidatesDuringLocalDescription = [candidateA]

        #expect(h.manager.startCall(chatID: 42, peerUserID: 9))
        #expect(h.manager.phase == .outgoing(ringing: false))
        #expect(h.manager.direction == .outgoing)
        #expect(h.manager.peerName == "Anna")
        #expect(h.manager.peerAvatarVersion == 3)
        #expect(h.manager.chatID == 42)
        let id = try #require(h.manager.callID)
        #expect(UUID(uuidString: id) != nil)
        #expect(id == id.lowercased())

        await h.drain()
        #expect(h.ensureConnectedCalls == 1)
        // The offer FIRST, then the candidate gathered while it was being
        // built — buffered until the offer had gone out.
        #expect(h.signaling.sent.count == 2)
        #expect(h.signaling.sent[0] == .callOffer(callID: id, chatID: 42, sdp: "v=0 offer", video: false))
        #expect(h.signaling.sent[1] == .callIce(callID: id, candidate: candidateA))

        h.manager.handle(frame: .callRinging(callID: id))
        #expect(h.manager.phase == .outgoing(ringing: true))

        // A remote candidate BEFORE the answer is buffered, and applied
        // after the remote description, in order with the one after it.
        h.manager.handle(frame: .callIce(callID: id, candidate: candidateA))
        h.manager.handle(frame: .callAnswer(callID: id, sdp: "v=0 remote answer"))
        #expect(h.manager.phase == .connecting)
        h.manager.handle(frame: .callIce(callID: id, candidate: candidateB))
        await h.drain()
        #expect(h.media.remote.count == 1)
        #expect(h.media.remote.first?.sdp == "v=0 remote answer")
        #expect(h.media.remote.first?.isOffer == false)
        #expect(h.media.remoteCandidates == [candidateA, candidateB])

        // A candidate gathered mid-call goes straight out.
        h.media.emitLocal(candidateB)
        await h.drain()
        #expect(h.signaling.sent.contains(.callIce(callID: id, candidate: candidateB)))

        h.media.emit(.connected)
        guard case .active = h.manager.phase else {
            Issue.record("expected active, got \(h.manager.phase)")
            return
        }

        h.manager.toggleMute()
        #expect(h.media.muted == true)
        #expect(h.manager.isMuted)

        h.manager.hangUp()
        #expect(h.manager.phase == .ended(.hangup))
        #expect(h.media.closed)
        #expect(h.endedCalls == 1)
        await h.drain()
        #expect(h.signaling.ends == ["\(id):hangup"])
        #expect(h.manager.phase == .idle)
        #expect(h.manager.callID == nil)
        #expect(h.phases.first == .outgoing(ringing: false))
        #expect(h.phases.last == .idle)
    }

    @Test("outgoing: hanging up while it rings is a cancel")
    func outgoingCancel() async throws {
        let h = Harness()
        h.manager.startCall(chatID: 42, peerUserID: 9)
        let id = try #require(h.manager.callID)
        await h.drain()
        h.manager.handle(frame: .callRinging(callID: id))
        h.manager.hangUp()
        #expect(h.manager.phase == .ended(.cancel))
        await h.drain()
        #expect(h.signaling.ends == ["\(id):cancel"])
    }

    @Test("outgoing: a second call while one is live is refused locally")
    func busyLocally() async throws {
        let h = Harness()
        #expect(h.manager.startCall(chatID: 42, peerUserID: 9))
        let id = h.manager.callID
        #expect(!h.manager.startCall(chatID: 43, peerUserID: 10))
        #expect(h.manager.callID == id)
        await h.drain()
        #expect(h.signaling.sent.count == 1)
    }

    @Test("outgoing: the server's refusals end the call with their reason and write nothing")
    func serverRefusals() async throws {
        for (code, reason) in [("peer_busy", CallEndReason.busy), ("call_busy", .busy), ("peer_unreachable", .unreachable), ("calls_disabled", .unavailable), ("invalid_call", .unavailable), ("video_calls_disabled", .videoUnavailable)] {
            let h = Harness()
            h.manager.startCall(chatID: 42, peerUserID: 9)
            let id = try #require(h.manager.callID)
            await h.drain()
            h.manager.handle(frame: .error(code: code, message: "no", clientMsgID: nil, callID: id))
            #expect(h.manager.phase == .ended(reason), "\(code)")
            await h.drain()
            #expect(h.signaling.ends.isEmpty, "\(code): a refused offer has nothing to end")
        }
    }

    @Test("outgoing: the server's timeout / decline end the call")
    func remoteEnds() async throws {
        for (wire, reason) in [("timeout", CallEndReason.timeout), ("decline", .decline), ("failed", .failed), ("hangup", .hangup)] {
            let h = Harness()
            h.manager.startCall(chatID: 42, peerUserID: 9)
            let id = try #require(h.manager.callID)
            await h.drain()
            h.manager.handle(frame: .callEnd(callID: id, reason: wire))
            #expect(h.manager.phase == .ended(reason), Comment(rawValue: wire))
            #expect(h.media.closed)
            await h.drain()
            #expect(h.signaling.ends.isEmpty, "the other side ended it; nothing to send back")
        }
    }

    @Test("outgoing: the guard clock gives up on a server that never answers")
    func outgoingGuard() async throws {
        let h = Harness()
        h.manager.ringTimeout = 0.05
        h.manager.startCall(chatID: 42, peerUserID: 9)
        await h.drain()
        await h.waitUntil { h.endedCalls == 1 }
        #expect(h.manager.phase == .ended(.timeout) || h.manager.phase == .idle)
        #expect(h.endedCalls == 1)
        #expect(h.phases.contains(.ended(.timeout)))
    }

    @Test("outgoing: a media failure ends the call as failed and says so")
    func mediaFailure() async throws {
        let h = Harness()
        h.manager.startCall(chatID: 42, peerUserID: 9)
        let id = try #require(h.manager.callID)
        await h.drain()
        h.manager.handle(frame: .callAnswer(callID: id, sdp: "v=0 remote answer"))
        await h.drain()
        // A blip is not the end.
        h.media.emit(.disconnected)
        #expect(h.manager.phase == .connecting)
        h.media.emit(.failed)
        #expect(h.manager.phase == .ended(.failed))
        await h.drain()
        #expect(h.signaling.ends == ["\(id):failed"])
    }

    @Test("outgoing: a refused microphone places nothing")
    func microphoneDenied() async throws {
        let h = Harness(microphone: false)
        h.manager.startCall(chatID: 42, peerUserID: 9)
        await h.drain()
        #expect(h.phases.contains(.ended(.microphoneDenied)))
        #expect(h.signaling.sent.isEmpty)
    }

    // MARK: - Incoming

    @Test("incoming over the socket: offer, accept, answer, connected")
    func incomingHappyPath() async throws {
        let h = Harness()
        h.media.candidatesDuringLocalDescription = [candidateA]
        h.manager.handle(frame: offer(id: remoteID))
        #expect(h.manager.phase == .incoming)
        #expect(h.manager.direction == .incoming)
        #expect(h.manager.callID == remoteID)
        #expect(h.manager.callUUID == UUID(uuidString: remoteID))
        #expect(h.manager.peerUserID == 9)
        #expect(h.manager.peerName == "Anna")
        #expect(h.manager.chatID == 42)
        // Candidates that arrive while it rings wait for the description.
        h.manager.handle(frame: .callIce(callID: remoteID, candidate: candidateB))

        h.manager.acceptIncoming()
        #expect(h.manager.phase == .connecting)
        await h.drain()
        #expect(h.media.remote.count == 1)
        #expect(h.media.remote.first?.sdp == "v=0 remote offer")
        #expect(h.media.remote.first?.isOffer == true)
        #expect(h.media.remoteCandidates == [candidateB])
        #expect(h.signaling.sent.count == 2)
        #expect(h.signaling.sent[0] == .callAnswer(callID: remoteID, sdp: "v=0 answer"))
        #expect(h.signaling.sent[1] == .callIce(callID: remoteID, candidate: candidateA))

        h.media.emit(.connected)
        guard case .active = h.manager.phase else {
            Issue.record("expected active, got \(h.manager.phase)")
            return
        }
        h.manager.hangUp()
        #expect(h.manager.phase == .ended(.hangup))
        await h.drain()
        #expect(h.signaling.ends == ["\(remoteID):hangup"])
    }

    @Test("incoming via push: the offer arrives AFTER the accept and is answered then")
    func incomingViaPushAcceptedFirst() async throws {
        let h = Harness()
        let push = IncomingCallPush(callID: remoteID, chatID: 42, fromUserID: 9, callerName: "Anna")
        #expect(h.manager.handleIncomingPush(push))
        #expect(h.manager.phase == .incoming)
        #expect(h.manager.callID == remoteID)
        await h.drain()
        #expect(h.ensureConnectedCalls == 1, "the socket is brought up for the replay")

        h.manager.acceptIncoming()
        #expect(h.manager.phase == .connecting)
        await h.drain()
        #expect(h.signaling.sent.isEmpty, "nothing to answer yet")

        h.manager.handle(frame: offer(id: remoteID))
        await h.drain()
        #expect(h.media.remote.first?.sdp == "v=0 remote offer")
        #expect(h.signaling.sent == [.callAnswer(callID: remoteID, sdp: "v=0 answer")])

        // The same push again, or the same offer again, changes nothing.
        #expect(h.manager.handleIncomingPush(push))
        h.manager.handle(frame: offer(id: remoteID))
        await h.drain()
        #expect(h.media.remote.count == 1)
        #expect(h.signaling.sent.count == 1)
    }

    @Test("incoming via push: the offer arrives BEFORE the accept")
    func incomingViaPushOfferFirst() async throws {
        let h = Harness()
        let push = IncomingCallPush(callID: remoteID, chatID: 42, fromUserID: 9, callerName: "Anna")
        h.manager.handleIncomingPush(push)
        h.manager.handle(frame: offer(id: remoteID))
        #expect(h.manager.phase == .incoming)
        h.manager.acceptIncoming()
        await h.drain()
        #expect(h.signaling.sent == [.callAnswer(callID: remoteID, sdp: "v=0 answer")])
    }

    @Test("incoming via push while busy is refused, so the registrar can end what it reported")
    func incomingPushWhileBusy() async throws {
        let h = Harness()
        h.manager.startCall(chatID: 42, peerUserID: 9)
        let push = IncomingCallPush(callID: remoteID, chatID: 43, fromUserID: 10, callerName: "Bob")
        #expect(!h.manager.handleIncomingPush(push))
        #expect(h.manager.direction == .outgoing)
    }

    @Test("incoming: a duplicate offer is a no-op, and a foreign call's frames are ignored")
    func duplicateAndForeignFrames() async throws {
        let h = Harness()
        h.manager.handle(frame: offer(id: remoteID))
        h.manager.handle(frame: offer(id: remoteID))
        #expect(h.manager.phase == .incoming)
        #expect(h.phases.count == 1)

        let other = "6a1f0c3e-0000-4000-8000-00000000ffff"
        h.manager.handle(frame: .callAnswer(callID: other, sdp: "v=0"))
        h.manager.handle(frame: .callIce(callID: other, candidate: candidateA))
        h.manager.handle(frame: .callEnd(callID: other, reason: "hangup"))
        h.manager.handle(frame: .error(code: "peer_busy", message: "", clientMsgID: nil, callID: other))
        #expect(h.manager.phase == .incoming)
        #expect(h.manager.callID == remoteID)
        // Nor does a second, different offer replace the one ringing.
        h.manager.handle(frame: offer(id: other))
        #expect(h.manager.callID == remoteID)
    }

    @Test("incoming: decline sends decline; answered elsewhere just stops")
    func declineAndAnsweredElsewhere() async throws {
        let h = Harness()
        h.manager.handle(frame: offer(id: remoteID))
        h.manager.declineIncoming()
        #expect(h.manager.phase == .ended(.decline))
        await h.drain()
        #expect(h.signaling.ends == ["\(remoteID):decline"])
        #expect(h.manager.phase == .idle)

        let h2 = Harness()
        h2.manager.handle(frame: offer(id: remoteID))
        h2.manager.handle(frame: .callEnd(callID: remoteID, reason: "answered_elsewhere"))
        #expect(h2.manager.phase == .ended(.answeredElsewhere))
        await h2.drain()
        #expect(h2.signaling.sent.isEmpty)
        // A late frame for the call that just ended is not a new call.
        h2.manager.handle(frame: offer(id: remoteID))
        #expect(h2.manager.phase == .idle)
    }

    @Test("incoming: the ring guard gives up when the offer never comes")
    func incomingGuard() async throws {
        let h = Harness()
        h.manager.ringTimeout = 0.05
        h.manager.handleIncomingPush(IncomingCallPush(callID: remoteID, chatID: 42, fromUserID: 9, callerName: "Anna"))
        await h.waitUntil { h.endedCalls == 1 }
        #expect(h.manager.phase == .ended(.timeout) || h.manager.phase == .idle)
        #expect(h.endedCalls == 1)
        #expect(h.phases.contains(.ended(.timeout)))
    }

    @Test("incoming: the push's name is used only when the roster has no better one")
    func pushName() async throws {
        let h = Harness()
        h.manager.resolvePeer = { _ in (String(localized: "Someone"), 0) }
        h.manager.handleIncomingPush(IncomingCallPush(callID: remoteID, chatID: 42, fromUserID: 9, callerName: "Anna (push)"))
        #expect(h.manager.peerName == "Anna (push)")
    }

    // MARK: - The system bridge

    @Test("with a system bridge, answer and hang-up go through the system and come back")
    func bridgeRouting() async throws {
        let h = Harness(bridge: true)
        let bridge = try #require(h.bridge)
        h.manager.handle(frame: offer(id: remoteID))
        #expect(bridge.events == ["incoming:Anna"])

        h.manager.acceptIncoming()
        #expect(bridge.events.last == "requestAnswer")
        #expect(h.manager.phase == .connecting)
        await h.drain()
        h.media.emit(.connected)

        h.manager.hangUp()
        #expect(bridge.events.last == "requestEnd")
        #expect(h.manager.phase == .ended(.hangup))
        // The system asked for the end, so it is not told about it again.
        #expect(!bridge.events.contains("ended:hangup"))
        await h.drain()
        #expect(h.signaling.ends == ["\(remoteID):hangup"])
    }

    @Test("with a system bridge, an outgoing call is reported at each step and a remote end is reported back")
    func bridgeOutgoing() async throws {
        let h = Harness(bridge: true)
        let bridge = try #require(h.bridge)
        h.manager.startCall(chatID: 42, peerUserID: 9)
        let id = try #require(h.manager.callID)
        #expect(bridge.events == ["outgoing:Anna"])
        await h.drain()
        h.manager.handle(frame: .callAnswer(callID: id, sdp: "v=0 remote answer"))
        #expect(bridge.events.last == "connecting")
        await h.drain()
        h.media.emit(.connected)
        #expect(bridge.events.last == "connected")
        h.manager.handle(frame: .callEnd(callID: id, reason: "hangup"))
        #expect(bridge.events.last == "ended:hangup")
    }

    @Test("the system's own mute is mirrored")
    func systemMute() async throws {
        let h = Harness(bridge: true)
        h.manager.startCall(chatID: 42, peerUserID: 9)
        await h.drain()
        h.manager.systemDidSetMuted(true)
        #expect(h.manager.isMuted)
        #expect(h.media.muted == true)
    }

    // MARK: - Video calls (docs/protocol.md, "Video")

    private func videoOffer(id: String, from: Int64 = 9) -> ServerFrame {
        .callOffer(CallOfferPayload(callID: id, chatID: 42, fromUserID: from, sdp: "v=0 remote offer", video: true))
    }

    @Test("video outgoing: the flag threads to the bridge, the media client, the frame — and the camera starts on")
    func videoOutgoing() async throws {
        let h = Harness(bridge: true)
        #expect(h.manager.startCall(chatID: 42, peerUserID: 9, video: true))
        #expect(h.manager.isVideo)
        #expect(h.bridge?.events == ["outgoing:Anna:video"])
        let id = try #require(h.manager.callID)
        await h.drain()
        #expect(h.mediaVideo == [true])
        #expect(h.manager.isCameraOn)
        #expect(!h.manager.cameraDenied)
        // The camera state reaches media before the offer goes out.
        #expect(h.media.cameraEnabled == [true])
        #expect(h.signaling.sent.first == .callOffer(callID: id, chatID: 42, sdp: "v=0 offer", video: true))
    }

    @Test("video outgoing: a denied camera still places the call, camera off, with the status note")
    func videoCameraDenied() async throws {
        let h = Harness(camera: false)
        #expect(h.manager.startCall(chatID: 42, peerUserID: 9, video: true))
        let id = try #require(h.manager.callID)
        await h.drain()
        // NOT .microphoneDenied, not ended at all: the call went out.
        #expect(h.manager.phase == .outgoing(ringing: false))
        #expect(!h.manager.isCameraOn)
        #expect(h.manager.cameraDenied)
        #expect(h.media.cameraEnabled == [false])
        #expect(h.signaling.sent.first == .callOffer(callID: id, chatID: 42, sdp: "v=0 offer", video: true))
    }

    @Test("video: a refused microphone still ends the call, camera rules notwithstanding")
    func videoMicrophoneDenied() async throws {
        let h = Harness(microphone: false)
        h.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h.drain()
        #expect(h.phases.contains(.ended(.microphoneDenied)))
        #expect(h.signaling.sent.isEmpty)
    }

    @Test("video: camera and flip toggles delegate to media, and are no-ops on a voice call")
    func videoToggles() async throws {
        let h = Harness()
        h.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h.drain()
        #expect(h.media.cameraEnabled == [true])

        h.manager.toggleCamera()
        #expect(!h.manager.isCameraOn)
        #expect(h.media.cameraEnabled == [true, false])
        // Flip is gated on the camera being on.
        h.manager.flipCamera()
        #expect(h.media.flips == 0)
        h.manager.toggleCamera()
        #expect(h.manager.isCameraOn)
        h.manager.flipCamera()
        #expect(h.media.flips == 1)
        #expect(!h.manager.isFrontCamera)

        let voice = Harness()
        voice.manager.startCall(chatID: 42, peerUserID: 9)
        await voice.drain()
        voice.manager.toggleCamera()
        voice.manager.flipCamera()
        #expect(voice.media.cameraEnabled.isEmpty)
        #expect(voice.media.flips == 0)
        #expect(!voice.manager.isCameraOn)
    }

    @Test("video incoming over the socket: rings as video on the system UI, answers with video media")
    func videoIncomingOffer() async throws {
        let h = Harness(bridge: true)
        h.manager.handle(frame: videoOffer(id: remoteID))
        #expect(h.manager.isVideo)
        #expect(h.bridge?.events == ["incoming:Anna:video"])
        h.manager.acceptIncoming()
        await h.drain()
        #expect(h.mediaVideo == [true])
        #expect(h.manager.isCameraOn)
        #expect(h.signaling.sent.first == .callAnswer(callID: remoteID, sdp: "v=0 answer"))
    }

    @Test("video incoming via push: the push's flag rings video and survives to the answer")
    func videoIncomingPush() async throws {
        let h = Harness()
        let push = IncomingCallPush(callID: remoteID, chatID: 42, fromUserID: 9, callerName: "Anna", video: true)
        #expect(h.manager.handleIncomingPush(push))
        #expect(h.manager.isVideo)
        h.manager.acceptIncoming()
        h.manager.handle(frame: videoOffer(id: remoteID))
        await h.drain()
        #expect(h.mediaVideo == [true])
    }

    @Test("video: renderers set before media exists are attached once it does, and remote video tracks the delegate")
    func videoRenderers() async throws {
        let h = Harness()
        let local = FakeRenderer()
        let remote = FakeRenderer()
        h.manager.setLocalVideoRenderer(local)
        h.manager.setRemoteVideoRenderer(remote)
        h.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h.drain()
        #expect(h.media.localRenderers == [true])
        #expect(h.media.remoteRenderers == [true])

        h.media.emitRemoteVideo(true)
        #expect(h.manager.isRemoteVideoActive)
        h.media.emitRemoteVideo(false)
        #expect(!h.manager.isRemoteVideoActive)
    }

    @Test("the first-frame relay forwards everything, and reports once per attachment")
    func firstFrameRelay() async throws {
        final class Recorder: NSObject, RTCVideoRenderer {
            var sizes: [CGSize] = []
            var frames = 0
            func setSize(_ size: CGSize) { sizes.append(size) }
            func renderFrame(_ frame: RTCVideoFrame?) { frames += 1 }
        }
        let recorder = Recorder()
        var fired = 0
        let relay = RemoteFirstFrameRelay(wrapping: recorder) { fired += 1 }

        var raw: CVPixelBuffer?
        CVPixelBufferCreate(nil, 2, 2, kCVPixelFormatType_32BGRA, nil, &raw)
        let frame = RTCVideoFrame(
            buffer: RTCCVPixelBuffer(pixelBuffer: try #require(raw)),
            rotation: ._0, timeStampNs: 0)

        relay.setSize(CGSize(width: 2, height: 2))
        relay.renderFrame(nil) // a renderer reset, not a picture
        relay.renderFrame(frame)
        relay.renderFrame(frame)
        for _ in 0..<4 { await Task.yield() }
        // Everything reached the wrapped renderer; the report came once,
        // on the first REAL frame.
        #expect(recorder.sizes == [CGSize(width: 2, height: 2)])
        #expect(recorder.frames == 3)
        #expect(fired == 1)

        // A fresh attachment (window reopened mid-call) reports again.
        let second = RemoteFirstFrameRelay(wrapping: recorder) { fired += 1 }
        second.renderFrame(frame)
        for _ in 0..<4 { await Task.yield() }
        #expect(fired == 2)
    }

    @Test("video incoming: the system answer never awaits the camera prompt — answers camera-off, camera on when granted later")
    func videoSystemAnswerDoesNotAwaitCamera() async throws {
        let h = Harness()
        let gate = CameraGate()
        h.manager.cameraGrantState = { nil }
        h.manager.requestCamera = { await gate.request() }
        h.manager.handle(frame: videoOffer(id: remoteID))
        // CallKit's CXAnswerCallAction path — no in-app pre-prompt.
        h.manager.systemDidAnswer()
        #expect(h.manager.phase == .connecting)
        await h.drain()
        // The answer went out while the (unshowable) prompt still hangs.
        #expect(h.signaling.sent.first == .callAnswer(callID: remoteID, sdp: "v=0 answer"))
        #expect(!h.manager.isCameraOn)
        #expect(!h.manager.cameraDenied, "undecided is not denied")
        #expect(h.media.cameraEnabled == [false])
        #expect(gate.asked)

        // The person grants it: the camera comes on like a manual toggle.
        gate.resolve(true)
        await h.waitUntil { h.manager.isCameraOn }
        #expect(h.manager.isCameraOn)
        #expect(!h.manager.cameraDenied)
        #expect(h.media.cameraEnabled == [false, true])
    }

    @Test("video incoming: a late camera refusal keeps the camera off, with the denied note")
    func videoSystemAnswerCameraDeniedLate() async throws {
        let h = Harness()
        let gate = CameraGate()
        h.manager.cameraGrantState = { nil }
        h.manager.requestCamera = { await gate.request() }
        h.manager.handle(frame: videoOffer(id: remoteID))
        h.manager.systemDidAnswer()
        await h.drain()
        #expect(h.signaling.sent.first == .callAnswer(callID: remoteID, sdp: "v=0 answer"))
        gate.resolve(false)
        await h.waitUntil { h.manager.cameraDenied }
        #expect(!h.manager.isCameraOn)
        #expect(h.manager.cameraDenied)
        #expect(h.media.cameraEnabled == [false])
    }

    @Test("video incoming: the in-app accept prompts FIRST, then answers with the decided camera")
    func videoInAppAcceptPromptsFirst() async throws {
        let h = Harness()
        let gate = CameraGate()
        var determined: Bool?
        h.manager.cameraGrantState = { determined }
        h.manager.requestCamera = {
            let granted = await gate.request()
            determined = granted
            return granted
        }
        h.manager.handle(frame: videoOffer(id: remoteID))
        h.manager.acceptIncoming()
        // Still ringing: nothing answered while the person reads the alert.
        #expect(h.manager.phase == .incoming)
        for _ in 0..<4 { await Task.yield() }
        #expect(h.signaling.sent.isEmpty)

        gate.resolve(true)
        await h.waitUntil { !h.signaling.sent.isEmpty }
        await h.drain()
        #expect(h.manager.phase == .connecting)
        #expect(h.manager.isCameraOn)
        #expect(h.media.cameraEnabled == [true])
        #expect(h.signaling.sent.first == .callAnswer(callID: remoteID, sdp: "v=0 answer"))
    }

    @Test("toggleCamera: turning on re-asks the grant — granted turns it on, refused keeps the note")
    func toggleCameraReasksTheGrant() async throws {
        // Denied at answer time, granted when the tap re-asks.
        let h = Harness(camera: false)
        h.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h.drain()
        #expect(!h.manager.isCameraOn)
        #expect(h.manager.cameraDenied)
        h.manager.requestCamera = { true }
        h.manager.toggleCamera()
        await h.drain()
        #expect(h.manager.isCameraOn)
        #expect(!h.manager.cameraDenied)
        #expect(h.media.cameraEnabled == [false, true])

        // Still refused: the camera stays off and the note stays up —
        // no black self-tile under a "voice-only" footnote.
        let h2 = Harness(camera: false)
        h2.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h2.drain()
        h2.manager.toggleCamera()
        await h2.drain()
        #expect(!h2.manager.isCameraOn)
        #expect(h2.manager.cameraDenied)
        #expect(h2.media.cameraEnabled == [false])

        // Granted meanwhile in Settings (the Mac's mid-call grant): the
        // tap turns the camera on synchronously and clears the stale note.
        let h3 = Harness(camera: false)
        h3.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        await h3.drain()
        #expect(h3.manager.cameraDenied)
        h3.manager.cameraGrantState = { true }
        h3.manager.toggleCamera()
        #expect(h3.manager.isCameraOn)
        #expect(!h3.manager.cameraDenied)
    }

    @Test("speaker: starts ON for a video call, OFF for voice, and resets between calls")
    func speakerSeededByCallKind() async throws {
        let h = Harness()
        h.manager.startCall(chatID: 42, peerUserID: 9, video: true)
        #expect(h.manager.isSpeaker, "the .videoChat session routes to the speaker; the toggle must say so")
        h.manager.toggleSpeaker()
        #expect(!h.manager.isSpeaker)
        h.manager.hangUp()
        await h.drain()

        #expect(h.manager.startCall(chatID: 42, peerUserID: 9))
        #expect(!h.manager.isSpeaker, "a voice call starts at the receiver")
        h.manager.hangUp()
        await h.drain()

        let h2 = Harness()
        h2.manager.handle(frame: videoOffer(id: remoteID))
        #expect(h2.manager.isSpeaker, "an incoming video call rings speaker-first too")
    }

    @Test("voice: nothing changed — no video to media, no camera calls, the old byte-stable offer")
    func voiceUnchanged() async throws {
        let h = Harness(bridge: true)
        h.manager.startCall(chatID: 42, peerUserID: 9)
        let id = try #require(h.manager.callID)
        #expect(!h.manager.isVideo)
        #expect(h.bridge?.events == ["outgoing:Anna"])
        await h.drain()
        #expect(h.mediaVideo == [false])
        #expect(h.media.cameraEnabled.isEmpty)
        #expect(h.media.localRenderers.isEmpty)
        #expect(h.signaling.sent.first == .callOffer(callID: id, chatID: 42, sdp: "v=0 offer", video: false))
    }
}
